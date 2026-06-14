//! Build script for `den-web`.
//!
//! In production builds (the `production` feature) it embeds the web template tree
//! so the MiniJinja environment can `load_templates!` without disk access. In dev
//! builds the embed is a no-op (templates are path-loaded from `config.templates_dir`,
//! default `crates/den-web/src/templates`). Assets are embedded by the `memory_serve`
//! proc-macro at compile time, so they need no build-script step here.
fn main() {
    println!("cargo:rerun-if-env-changed=TEMPLATES_DIR");
    let production_enabled = std::env::var_os("CARGO_FEATURE_PRODUCTION").is_some();
    if production_enabled {
        let templates_dir =
            std::env::var("TEMPLATES_DIR").unwrap_or_else(|_| "src/templates".to_string());
        minijinja_embed::embed_templates!(&templates_dir);
    }
}
