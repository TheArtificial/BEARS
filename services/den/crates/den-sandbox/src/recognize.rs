//! Work-surface recognition: cheap, explicit facts about a provisioned
//! workspace, recorded on the run so humans and policy can see what was
//! operated on. Heuristics are deliberately simple lookup tables — edge-case
//! cleverness is reserved for destructive decisions, which this module never
//! makes.

use crate::proc::{run_command, CommandSpec};
use crate::protocol::WorkSurface;
use std::path::Path;
use std::time::Duration;

pub async fn recognize_work_surface(path: &Path) -> WorkSurface {
    let mut surface = WorkSurface {
        writable: is_writable(path),
        ..WorkSurface::default()
    };
    recognize_git(path, &mut surface).await;
    recognize_toolchain(path, &mut surface);
    surface
}

async fn recognize_git(path: &Path, surface: &mut WorkSurface) {
    if !path.join(".git").exists() {
        return;
    }
    let args: Vec<String> = ["status", "--porcelain=v2", "--branch"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let mut spec = CommandSpec::new("git", &args);
    spec.cwd = Some(path);
    spec.timeout = Duration::from_secs(30);
    let Ok(out) = run_command(spec).await else {
        return;
    };
    if !out.success() {
        return;
    }
    surface.is_git = true;
    for line in out.stdout_lossy().lines() {
        if let Some(oid) = line.strip_prefix("# branch.oid ") {
            if oid != "(initial)" {
                surface.commit = Some(oid.to_string());
            }
        } else if let Some(head) = line.strip_prefix("# branch.head ") {
            if head != "(detached)" {
                surface.branch = Some(head.to_string());
            }
        } else if line.starts_with('?') {
            surface.untracked_present = true;
        } else if line.starts_with('1') || line.starts_with('2') || line.starts_with('u') {
            surface.dirty = true;
        }
    }
}

fn recognize_toolchain(path: &Path, surface: &mut WorkSurface) {
    let mut lang = |l: &str| {
        if !surface.language_hints.iter().any(|x| x == l) {
            surface.language_hints.push(l.to_string());
        }
    };

    surface.cargo_manifest_paths = cargo_manifest_paths(path);
    if !surface.cargo_manifest_paths.is_empty() {
        lang("rust");
        surface.package_manager_hints.push("cargo".to_string());
        surface.test_command_hints.push("cargo test".to_string());
        for manifest in &surface.cargo_manifest_paths {
            let lockfile = Path::new(manifest).with_file_name("Cargo.lock");
            if path.join(&lockfile).exists() {
                surface
                    .lockfiles
                    .push(lockfile.to_string_lossy().into_owned());
            }
        }
    }

    if path.join("package.json").exists() {
        lang("javascript");
        let node_pm = if path.join("pnpm-lock.yaml").exists() {
            surface.lockfiles.push("pnpm-lock.yaml".to_string());
            "pnpm"
        } else if path.join("yarn.lock").exists() {
            surface.lockfiles.push("yarn.lock".to_string());
            "yarn"
        } else {
            if path.join("package-lock.json").exists() {
                surface.lockfiles.push("package-lock.json".to_string());
            }
            "npm"
        };
        surface.package_manager_hints.push(node_pm.to_string());
        surface.test_command_hints.push(format!("{node_pm} test"));
    }

    if path.join("pyproject.toml").exists() {
        lang("python");
        if path.join("uv.lock").exists() {
            surface.lockfiles.push("uv.lock".to_string());
            surface.package_manager_hints.push("uv".to_string());
        } else if path.join("poetry.lock").exists() {
            surface.lockfiles.push("poetry.lock".to_string());
            surface.package_manager_hints.push("poetry".to_string());
        } else {
            surface.package_manager_hints.push("pip".to_string());
        }
        surface.test_command_hints.push("pytest".to_string());
    }

    if path.join("go.mod").exists() {
        lang("go");
        surface.package_manager_hints.push("go".to_string());
        surface.test_command_hints.push("go test ./...".to_string());
        if path.join("go.sum").exists() {
            surface.lockfiles.push("go.sum".to_string());
        }
    }

    if path.join("Makefile").exists() {
        surface.test_command_hints.push("make test".to_string());
    }
}

fn cargo_manifest_paths(root: &Path) -> Vec<String> {
    fn visit(root: &Path, relative: &Path, manifests: &mut Vec<String>) {
        let directory = root.join(relative);
        if directory.join("Cargo.toml").is_file() {
            manifests.push(relative.join("Cargo.toml").to_string_lossy().into_owned());
            return;
        }

        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        let mut directories = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter(|entry| {
                !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | "target" | "node_modules")
                )
            })
            .collect::<Vec<_>>();
        directories.sort_by_key(|entry| entry.file_name());
        for entry in directories {
            visit(root, &relative.join(entry.file_name()), manifests);
        }
    }

    // ponytail: scan stops at the first Cargo manifest in each directory tree,
    // treating it as that tree's workspace root. If nested independent Cargo
    // projects become necessary, replace recognition with explicit catalog data.
    let mut manifests = Vec::new();
    visit(root, Path::new(""), &mut manifests);
    manifests
}

fn is_writable(path: &Path) -> bool {
    // Probe rather than inspect mode bits: correct across ownership,
    // ACLs, and read-only mounts.
    let probe = path.join(".den-sandbox-write-probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "den-sandbox-recognize-{tag}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn plain_dir_has_no_git_facts() {
        let dir = tempdir("plain");
        let surface = recognize_work_surface(&dir).await;
        assert!(!surface.is_git);
        assert!(surface.writable);
        assert!(surface.language_hints.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn recognizes_rust_and_node_hints() {
        let dir = tempdir("hints");
        std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.join("Cargo.lock"), "").unwrap();
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        let surface = recognize_work_surface(&dir).await;
        assert_eq!(surface.language_hints, vec!["rust", "javascript"]);
        assert_eq!(surface.cargo_manifest_paths, vec!["Cargo.toml"]);
        assert!(surface.package_manager_hints.iter().any(|pm| pm == "pnpm"));
        assert!(surface.lockfiles.contains(&"Cargo.lock".to_string()));
        assert!(surface
            .test_command_hints
            .contains(&"cargo test".to_string()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn recognizes_nested_rust_workspace() {
        let dir = tempdir("nested-rust");
        std::fs::create_dir_all(dir.join("services/den/crates/member")).unwrap();
        std::fs::write(dir.join("services/den/Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(
            dir.join("services/den/crates/member/Cargo.toml"),
            "[package]\nname = \"member\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("services/den/Cargo.lock"), "").unwrap();

        let surface = recognize_work_surface(&dir).await;

        assert_eq!(
            surface.cargo_manifest_paths,
            vec!["services/den/Cargo.toml"]
        );
        assert_eq!(surface.language_hints, vec!["rust"]);
        assert_eq!(surface.lockfiles, vec!["services/den/Cargo.lock"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn recognizes_git_state() {
        let dir = tempdir("git");
        let run = |args: &[&str], cwd: &Path| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(cwd)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        run(&["init", "-b", "main"], &dir);
        std::fs::write(dir.join("a.txt"), "one").unwrap();
        run(&["add", "."], &dir);
        run(&["commit", "-m", "init"], &dir);
        std::fs::write(dir.join("a.txt"), "two").unwrap();
        std::fs::write(dir.join("new.txt"), "untracked").unwrap();

        let surface = recognize_work_surface(&dir).await;
        assert!(surface.is_git);
        assert_eq!(surface.branch.as_deref(), Some("main"));
        assert!(surface.commit.is_some());
        assert!(surface.dirty);
        assert!(surface.untracked_present);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
