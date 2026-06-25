use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

const RTK_REDUCER_TIMEOUT_MS: u64 = 5_000;
const RTK_REDUCER_INPUT_MAX_CHARS: usize = 128 * 1024;
const RTK_REDUCER_OUTPUT_MAX_CHARS: usize = 24 * 1024;

pub(crate) struct RtkReduction {
    pub(crate) output: String,
    pub(crate) input_truncated: bool,
    pub(crate) output_truncated: bool,
}

impl RtkReduction {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "reducer": "rtk",
            "mode": "summary",
            "reduced": true,
            "input_truncated": self.input_truncated,
            "output_truncated": self.output_truncated,
        })
    }
}

pub(crate) fn rtk_reducer_enabled(env_name: &str) -> bool {
    std::env::var(env_name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn output_excerpt(raw: &str, max_chars: usize) -> String {
    if raw.chars().count() <= max_chars {
        raw.to_string()
    } else {
        let omitted = raw.chars().count().saturating_sub(max_chars);
        format!(
            "{}\n... truncated, omitted {omitted} characters",
            raw.chars().take(max_chars).collect::<String>()
        )
    }
}

pub(crate) async fn reduce_with_rtk_summary(env_name: &str, input: String) -> Option<RtkReduction> {
    if !rtk_reducer_enabled(env_name) || input.trim().is_empty() {
        return None;
    }
    let input_truncated = input.chars().count() > RTK_REDUCER_INPUT_MAX_CHARS;
    let input = if input_truncated {
        input
            .chars()
            .take(RTK_REDUCER_INPUT_MAX_CHARS)
            .collect::<String>()
    } else {
        input
    };
    let mut child = Command::new("rtk")
        .arg("summary")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| {
            eprintln!("bear-armature: RTK reducer unavailable env={env_name} error={err}");
            err
        })
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(input.as_bytes()).await.is_err() {
            eprintln!("bear-armature: RTK reducer stdin write failed env={env_name}");
            let _ = child.kill().await;
            return None;
        }
    }
    let output = match tokio::time::timeout(
        Duration::from_millis(RTK_REDUCER_TIMEOUT_MS),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            eprintln!("bear-armature: RTK reducer wait failed env={env_name} error={err}");
            return None;
        }
        Err(_) => {
            eprintln!("bear-armature: RTK reducer timed out env={env_name}");
            return None;
        }
    };
    if !output.status.success() {
        eprintln!(
            "bear-armature: RTK reducer exited nonzero env={} status={}",
            env_name, output.status
        );
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        eprintln!("bear-armature: RTK reducer returned empty output env={env_name}");
        return None;
    }
    let output_truncated = text.chars().count() > RTK_REDUCER_OUTPUT_MAX_CHARS;
    let output = if output_truncated {
        output_excerpt(&text, RTK_REDUCER_OUTPUT_MAX_CHARS)
    } else {
        text
    };
    eprintln!(
        "bear-armature: RTK reducer applied env={} input_truncated={} output_truncated={}",
        env_name, input_truncated, output_truncated
    );
    Some(RtkReduction {
        output,
        input_truncated,
        output_truncated,
    })
}
