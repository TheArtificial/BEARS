//! Build script for `den-web`.
//!
//! Validates/embeds the web template tree at build time so MiniJinja syntax errors
//! fail normal `cargo check`/build preflights instead of only failing production
//! template embedding. Dev runtime still path-loads templates from `config.templates_dir`,
//! default `crates/den-web/src/templates`. Assets are embedded by the `memory_serve`
//! proc-macro at compile time, so they need no build-script step here.
use std::{
    fs,
    path::{Path, PathBuf},
};

fn emit_rerun_if_changed(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());

    if path.is_dir() {
        let mut entries = fs::read_dir(path)
            .unwrap_or_else(|err| panic!("failed to read template path {}: {err}", path.display()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|err| {
                panic!(
                    "failed to enumerate template path {}: {err}",
                    path.display()
                )
            });
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            emit_rerun_if_changed(&entry.path());
        }
    }
}

fn collect_template_names(root: &Path) -> Vec<String> {
    fn visit(root: &Path, dir: &Path, names: &mut Vec<String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap_or_else(|err| panic!("failed to read template path {}: {err}", dir.display()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|err| {
                panic!("failed to enumerate template path {}: {err}", dir.display())
            });
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, names);
            } else if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("html" | "jinja")
            ) {
                names.push(
                    path.strip_prefix(root)
                        .expect("template must be under template root")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut names = Vec::new();
    visit(root, root, &mut names);
    names
}

fn validate_templates(templates_dir: &Path) {
    let canonical = templates_dir
        .canonicalize()
        .unwrap_or_else(|err| panic!("failed to canonicalize {}: {err}", templates_dir.display()));
    let mut env = minijinja::Environment::new();
    env.set_loader(minijinja::path_loader(PathBuf::from(&canonical)));
    // Parsing resolves filter names. Runtime supplies the real implementations;
    // build-time validation needs only signatures so syntax/inheritance compile.
    for name in [
        "hexadecimal",
        "urlencode",
        "markdown",
        "timeago",
        "humanize_time",
        "is_future",
    ] {
        env.add_filter(name, |value: minijinja::Value| value);
    }
    for template_name in collect_template_names(&canonical) {
        env.get_template(&template_name).unwrap_or_else(|err| {
            panic!("template {template_name:?} failed build-time validation: {err:#}")
        });
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=TEMPLATES_DIR");
    let requested_templates_dir = std::env::var("TEMPLATES_DIR").ok();
    let templates_dir = requested_templates_dir
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty() && Path::new(path).is_dir())
        .unwrap_or("src/templates");
    let templates_path = Path::new(templates_dir);
    emit_rerun_if_changed(templates_path);
    validate_templates(templates_path);
    minijinja_embed::embed_templates!(templates_dir);
}
