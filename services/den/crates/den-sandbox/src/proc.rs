//! Bounded subprocess execution: piped stdio, byte-capped capture, hard
//! timeout with kill. Ported from bear-armature's `tools/process.rs` harness —
//! there is no shared den-crate utility for this yet, so this crate carries
//! its own copy rather than depending on the armature.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum ProcError {
    #[error("failed to spawn {program}: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("io error while running {program}: {source}")]
    Io {
        program: String,
        #[source]
        source: std::io::Error,
    },
}

/// Which end of the stream to keep when output exceeds the byte cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureWindow {
    /// Keep the first N bytes (default; matches the armature harness).
    Head,
    /// Keep the last N bytes (log tails).
    Tail,
}

pub struct CommandSpec<'a> {
    pub program: &'a str,
    pub args: &'a [String],
    pub cwd: Option<&'a Path>,
    pub env: &'a [(String, String)],
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub window: CaptureWindow,
}

impl<'a> CommandSpec<'a> {
    pub fn new(program: &'a str, args: &'a [String]) -> Self {
        Self {
            program,
            args,
            cwd: None,
            env: &[],
            timeout: Duration::from_secs(60),
            max_output_bytes: 256 * 1024,
            window: CaptureWindow::Head,
        }
    }
}

#[derive(Debug)]
pub struct CommandOutput {
    pub exit_code: Option<i64>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub timed_out: bool,
    pub elapsed: Duration,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        !self.timed_out && self.exit_code == Some(0)
    }

    pub fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// Run a command to completion with bounded output and a hard timeout.
/// On timeout the child is killed and whatever output was captured is returned
/// with `timed_out: true`.
pub async fn run_command(spec: CommandSpec<'_>) -> Result<CommandOutput, ProcError> {
    let started = std::time::Instant::now();
    let mut command = Command::new(spec.program);
    command
        .args(spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = spec.cwd {
        command.current_dir(cwd);
    }
    for (key, value) in spec.env {
        command.env(key, value);
    }
    let mut child = command.spawn().map_err(|source| ProcError::Spawn {
        program: spec.program.to_string(),
        source,
    })?;

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let limit = spec.max_output_bytes;
    let window = spec.window;
    let stdout_task = tokio::spawn(read_limited(stdout, limit, window));
    let stderr_task = tokio::spawn(read_limited(stderr, limit, window));

    let (exit_code, timed_out) = match tokio::time::timeout(spec.timeout, child.wait()).await {
        Ok(status) => {
            let status = status.map_err(|source| ProcError::Io {
                program: spec.program.to_string(),
                source,
            })?;
            (status.code().map(i64::from), false)
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            (None, true)
        }
    };

    let (stdout, stdout_truncated) = join_capture(stdout_task, spec.program).await?;
    let (stderr, stderr_truncated) = join_capture(stderr_task, spec.program).await?;

    Ok(CommandOutput {
        exit_code,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        timed_out,
        elapsed: started.elapsed(),
    })
}

async fn join_capture(
    task: tokio::task::JoinHandle<Result<(Vec<u8>, bool), std::io::Error>>,
    program: &str,
) -> Result<(Vec<u8>, bool), ProcError> {
    match task.await {
        Ok(Ok(capture)) => Ok(capture),
        Ok(Err(source)) => Err(ProcError::Io {
            program: program.to_string(),
            source,
        }),
        Err(join_err) => Err(ProcError::Io {
            program: program.to_string(),
            source: std::io::Error::other(join_err),
        }),
    }
}

async fn read_limited<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
    window: CaptureWindow,
) -> Result<(Vec<u8>, bool), std::io::Error> {
    let mut bytes: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    let mut truncated = false;
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        match window {
            CaptureWindow::Head => {
                let remaining = limit.saturating_sub(bytes.len());
                if remaining == 0 {
                    truncated = true;
                    continue;
                }
                let take = remaining.min(n);
                bytes.extend_from_slice(&buf[..take]);
                if take < n {
                    truncated = true;
                }
            }
            CaptureWindow::Tail => {
                bytes.extend_from_slice(&buf[..n]);
                if bytes.len() > limit {
                    let drop = bytes.len() - limit;
                    bytes.drain(..drop);
                    truncated = true;
                }
            }
        }
    }
    Ok((bytes, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[tokio::test]
    async fn captures_exit_code_and_output() {
        let a = args(&["hello"]);
        let out = run_command(CommandSpec::new("echo", &a)).await.unwrap();
        assert!(out.success());
        assert_eq!(out.stdout_lossy().trim(), "hello");
    }

    #[tokio::test]
    async fn kills_on_timeout() {
        let a = args(&["5"]);
        let mut spec = CommandSpec::new("sleep", &a);
        spec.timeout = Duration::from_millis(100);
        let out = run_command(spec).await.unwrap();
        assert!(out.timed_out);
        assert_eq!(out.exit_code, None);
    }

    #[tokio::test]
    async fn head_window_keeps_prefix() {
        let a = args(&["-c", "printf 'aaaabbbb'"]);
        let mut spec = CommandSpec::new("sh", &a);
        spec.max_output_bytes = 4;
        let out = run_command(spec).await.unwrap();
        assert_eq!(out.stdout, b"aaaa");
        assert!(out.stdout_truncated);
    }

    #[tokio::test]
    async fn tail_window_keeps_suffix() {
        let a = args(&["-c", "printf 'aaaabbbb'"]);
        let mut spec = CommandSpec::new("sh", &a);
        spec.max_output_bytes = 4;
        spec.window = CaptureWindow::Tail;
        let out = run_command(spec).await.unwrap();
        assert_eq!(out.stdout, b"bbbb");
        assert!(out.stdout_truncated);
    }
}
