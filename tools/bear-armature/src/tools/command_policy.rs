#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandApprovalMode {
    FamilyAllowed,
    ExactOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandPolicyMatch<'a> {
    pub(crate) executable: &'a str,
    pub(crate) family_key: &'static str,
    pub(crate) family_label: &'static str,
    pub(crate) approval_mode: CommandApprovalMode,
    pub(crate) terminal_allowed: bool,
    pub(crate) rtk_wrap_allowed: bool,
}

pub(crate) fn normalize_command(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn split_command(command: &str) -> Option<(&str, Vec<&str>)> {
    let mut parts = command.split_whitespace();
    let executable = parts.next()?;
    Some((executable, parts.collect()))
}

pub(crate) fn command_policy_for(command: &str) -> Option<CommandPolicyMatch<'_>> {
    let (executable, args) = split_command(command)?;
    let first = args.first().copied();
    match (executable, first) {
        ("cargo", Some("build" | "check" | "test" | "clippy" | "fmt")) => {
            Some(CommandPolicyMatch {
                executable,
                family_key: "cargo",
                family_label: "cargo build/check/test",
                approval_mode: CommandApprovalMode::FamilyAllowed,
                terminal_allowed: true,
                rtk_wrap_allowed: true,
            })
        }
        ("pytest", _) => Some(CommandPolicyMatch {
            executable,
            family_key: "pytest",
            family_label: "pytest",
            approval_mode: CommandApprovalMode::FamilyAllowed,
            terminal_allowed: true,
            rtk_wrap_allowed: true,
        }),
        ("python" | "python3", Some("-m")) if args.get(1).copied() == Some("pytest") => {
            Some(CommandPolicyMatch {
                executable,
                family_key: "pytest",
                family_label: "pytest",
                approval_mode: CommandApprovalMode::FamilyAllowed,
                terminal_allowed: true,
                rtk_wrap_allowed: true,
            })
        }
        ("git", Some("status" | "diff" | "log" | "show" | "blame")) => Some(CommandPolicyMatch {
            executable,
            family_key: "git_read",
            family_label: "read-only git",
            approval_mode: CommandApprovalMode::FamilyAllowed,
            terminal_allowed: false,
            rtk_wrap_allowed: true,
        }),
        ("npm" | "pnpm" | "yarn", Some("test" | "run" | "exec")) => Some(CommandPolicyMatch {
            executable,
            family_key: "js_package_script",
            family_label: "package-manager scripts",
            approval_mode: CommandApprovalMode::ExactOnly,
            terminal_allowed: true,
            rtk_wrap_allowed: true,
        }),
        _ => None,
    }
}

pub(crate) fn command_family_key(command: &str) -> Option<&'static str> {
    let policy = command_policy_for(command)?;
    if policy.approval_mode == CommandApprovalMode::FamilyAllowed {
        Some(policy.family_key)
    } else {
        None
    }
}

pub(crate) fn command_workspace_scope_label(command: &str) -> Option<String> {
    let policy = command_policy_for(command)?;
    let (executable, args) = split_command(command)?;
    match policy.family_key {
        "git_read" => args
            .first()
            .map(|subcommand| format!("{} {}", policy.executable, subcommand)),
        "cargo" => args
            .first()
            .map(|subcommand| format!("{} {}", executable, subcommand)),
        "pytest" => {
            if executable == "pytest" {
                Some("pytest".to_string())
            } else if executable == "python" || executable == "python3" {
                Some(format!("{} -m pytest", executable))
            } else {
                Some(normalize_command(command))
            }
        }
        _ => Some(normalize_command(command)),
    }
}

pub(crate) fn terminal_command_allowed(command: &str, args: &[String]) -> bool {
    let full = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    command_policy_for(&full).is_some_and(|policy| policy.terminal_allowed)
}

pub(crate) fn process_command_preferred(command: &str, args: &[String]) -> bool {
    let full = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    if command_policy_for(&full).is_some() {
        return false;
    }
    matches!(
        command,
        "printf" | "true" | "false" | "pwd" | "uname" | "whoami" | "date" | "env"
    )
}

pub(crate) fn rtk_wrap_allowed(command: &str, args: &[String]) -> bool {
    let full = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    command_policy_for(&full).is_some_and(|policy| policy.rtk_wrap_allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_policy_allows_safe_family_only() {
        assert_eq!(command_family_key("cargo build"), Some("cargo"));
        assert_eq!(command_family_key("cargo check"), Some("cargo"));
        assert_eq!(command_family_key("cargo publish"), None);
    }

    #[test]
    fn git_policy_is_read_only() {
        assert_eq!(command_family_key("git status"), Some("git_read"));
        assert_eq!(command_family_key("git commit"), None);
        assert_eq!(
            command_workspace_scope_label("git diff -- src/main.rs").as_deref(),
            Some("git diff")
        );
    }

    #[test]
    fn cargo_and_pytest_workspace_scope_labels_drop_extra_arguments() {
        assert_eq!(
            command_workspace_scope_label("cargo test --lib foo::bar").as_deref(),
            Some("cargo test")
        );
        assert_eq!(
            command_workspace_scope_label("cargo check -p den-runtime").as_deref(),
            Some("cargo check")
        );
        assert_eq!(
            command_workspace_scope_label("pytest tests/unit/test_x.py -k foo").as_deref(),
            Some("pytest")
        );
        assert_eq!(
            command_workspace_scope_label("python -m pytest tests/unit/test_x.py -k foo")
                .as_deref(),
            Some("python -m pytest")
        );
    }

    #[test]
    fn python_pytest_maps_to_pytest_family() {
        assert_eq!(command_family_key("python -m pytest"), Some("pytest"));
        assert_eq!(command_family_key("python -m http.server"), None);
    }

    #[test]
    fn terminal_allows_safe_build_test_commands() {
        assert!(terminal_command_allowed("cargo", &["check".to_string()]));
        assert!(!terminal_command_allowed("cargo", &["publish".to_string()]));
        assert!(!terminal_command_allowed("git", &["status".to_string()]));
    }

    #[test]
    fn process_preferred_is_limited_to_quiet_non_terminal_commands() {
        assert!(process_command_preferred("printf", &["hello".to_string()]));
        assert!(!process_command_preferred("cargo", &["test".to_string()]));
        assert!(!process_command_preferred("git", &["diff".to_string()]));
    }
}
