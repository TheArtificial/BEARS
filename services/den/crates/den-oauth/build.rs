//! Build script for `den-oauth`.
//!
//! In production builds (the `production` feature) it embeds the OAuth consent /
//! authorize template group (keyed `"api"` to match `templates::init_template_env`'s
//! `load_templates!(.., "api")`) so rendering needs no disk access. In dev builds
//! the embed is a no-op (templates are path-loaded from `crates/den-oauth/src/templates`).
fn main() {
    println!("cargo:rerun-if-env-changed=API_TEMPLATES_DIR");
    let production_enabled = std::env::var_os("CARGO_FEATURE_PRODUCTION").is_some();
    if production_enabled {
        let api_templates_dir =
            std::env::var("API_TEMPLATES_DIR").unwrap_or_else(|_| "src/templates".to_string());
        minijinja_embed::embed_templates!(&api_templates_dir, &[][..], "api");
    }
}
