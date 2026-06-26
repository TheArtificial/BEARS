    use super::*;

    /// Axum panics on path conflicts at merge time. Merging the memory router alongside the
    /// settings and management routers guards against regressions like the old
    /// `/memory/browse` redirect colliding with the real browse page.
    #[test]
    fn bear_routers_merge_without_conflict() {
        // Mirrors `lib.rs`: `management::router()` already merges `settings::router()`.
        let _router: Router<AppState> = Router::new()
            .merge(router())
            .merge(crate::bear::management::router());
    }

    /// Compile every new memory/entity template via the path loader to catch MiniJinja
    /// syntax errors at test time rather than first render in production.
    #[test]
    fn memory_templates_compile() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/templates");
        let mut env = minijinja::Environment::new();
        env.set_loader(minijinja::path_loader(dir));
        for name in [
            "bear/memory/_memory_nav.html",
            "bear/memory/dashboard.html",
            "bear/memory/recent.html",
            "bear/memory/search.html",
            "bear/memory/browse.html",
            "bear/memory/record.html",
            "bear/memory/entities.html",
            "bear/memory/entity.html",
        ] {
            env.get_template(name)
                .unwrap_or_else(|e| panic!("template {name} failed to compile: {e}"));
        }
    }
