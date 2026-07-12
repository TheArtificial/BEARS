#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandApprovalMode {
    FamilyAllowed,
    ExactOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandFamily {
    Cargo,
    Pytest,
    GitRead,
    JsPackageScript,
}

impl CommandFamily {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Pytest => "pytest",
            Self::GitRead => "git_read",
            Self::JsPackageScript => "js_package_script",
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Cargo => "cargo build/check/test",
            Self::Pytest => "pytest",
            Self::GitRead => "read-only git",
            Self::JsPackageScript => "package-manager scripts",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandPolicyMatch<'a> {
    pub(crate) executable: &'a str,
    pub(crate) family: CommandFamily,
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
                family: CommandFamily::Cargo,
                approval_mode: CommandApprovalMode::FamilyAllowed,
                terminal_allowed: true,
                rtk_wrap_allowed: true,
            })
        }
        ("pytest", _) => Some(CommandPolicyMatch {
            executable,
            family: CommandFamily::Pytest,
            approval_mode: CommandApprovalMode::FamilyAllowed,
            terminal_allowed: true,
            rtk_wrap_allowed: true,
        }),
        ("python" | "python3", Some("-m")) if args.get(1).copied() == Some("pytest") => {
            Some(CommandPolicyMatch {
                executable,
                family: CommandFamily::Pytest,
                approval_mode: CommandApprovalMode::FamilyAllowed,
                terminal_allowed: true,
                rtk_wrap_allowed: true,
            })
        }
        ("git", Some("status" | "diff" | "log" | "show" | "blame")) => Some(CommandPolicyMatch {
            executable,
            family: CommandFamily::GitRead,
            approval_mode: CommandApprovalMode::FamilyAllowed,
            terminal_allowed: false,
            rtk_wrap_allowed: true,
        }),
        ("npm" | "pnpm" | "yarn", Some("test" | "run" | "exec")) => Some(CommandPolicyMatch {
            executable,
            family: CommandFamily::JsPackageScript,
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
        Some(policy.family.key())
    } else {
        None
    }
}

pub(crate) fn command_workspace_scope_label(command: &str) -> Option<String> {
    let policy = command_policy_for(command)?;
    let (executable, args) = split_command(command)?;
    match policy.family {
        CommandFamily::GitRead => args
            .first()
            .map(|subcommand| format!("{} {}", policy.executable, subcommand)),
        CommandFamily::Cargo => args
            .first()
            .map(|subcommand| format!("{} {}", executable, subcommand)),
        CommandFamily::Pytest => {
            if executable == "pytest" {
                Some("pytest".to_string())
            } else if executable == "python" || executable == "python3" {
                Some(format!("{} -m pytest", executable))
            } else {
                Some(normalize_command(command))
            }
        }
        CommandFamily::JsPackageScript => Some(normalize_command(command)),
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
}
