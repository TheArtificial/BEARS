//! Build script for `den-http`.
//!
//! In production builds (the `production` feature), embed the email template group
//! so `email::template_environment` can `load_templates!(.., "email")` without disk
//! access. In dev builds this is a no-op (templates are path-loaded at runtime).
fn main() {
    let production_enabled = std::env::var_os("CARGO_FEATURE_PRODUCTION").is_some();
    if production_enabled {
        let email_templates_dir =
            std::env::var("EMAIL_TEMPLATES_DIR").unwrap_or_else(|_| "src/email/templates".to_string());
        minijinja_embed::embed_templates!(&email_templates_dir, &[][..], "email");
    }
}
