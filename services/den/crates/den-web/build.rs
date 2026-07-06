//! Build script for `den-web`.
//!
//! Validates/embeds the web template tree at build time so MiniJinja syntax errors
//! fail normal `cargo check`/build preflights instead of only failing production
//! template embedding. Dev runtime still path-loads templates from `config.templates_dir`,
//! default `crates/den-web/src/templates`. Assets are embedded by the `memory_serve`
//! proc-macro at compile time, so they need no build-script step here.
use std::{fs, path::Path};

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

fn main() {
    println!("cargo:rerun-if-env-changed=TEMPLATES_DIR");
    let requested_templates_dir = std::env::var("TEMPLATES_DIR").ok();
    let templates_dir = requested_templates_dir
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty() && Path::new(path).is_dir())
        .unwrap_or("src/templates");
    emit_rerun_if_changed(Path::new(templates_dir));
    minijinja_embed::embed_templates!(templates_dir);
}
