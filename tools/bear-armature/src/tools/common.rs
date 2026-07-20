use std::process::Stdio;

use tokio::process::Command;

pub(crate) fn command_line(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    }
}

pub(crate) fn output_excerpt(raw: &str, max_chars: usize) -> String {
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

pub(crate) async fn rtk_available() -> bool {
    Command::new("rtk")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .is_ok_and(|status| status.success())
}
