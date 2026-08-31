//! Postgres-backed tests for managed work surfaces. Skip when no database is
//! reachable (same convention as den-docket's integration tests).

use sqlx::PgPool;
use uuid::Uuid;

use super::*;

const TEST_SECRET_KEY: &str = "work-surface-test-secret-key";

/// `build_managed_config`'s version hashes the whole table, so tests that
/// mutate surfaces/catalog rows serialize to keep version assertions stable.
static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn test_pool() -> Option<PgPool> {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .ok()?;
    sqlx::migrate!("../../migrations").run(&pool).await.ok()?;
    Some(pool)
}

async fn create_user(pool: &PgPool) -> i32 {
    let suffix = Uuid::new_v4().simple().to_string();
    let username = format!("wsuser{}", &suffix[..12]);
    sqlx::query_scalar(
        "INSERT INTO users (email, username, display_name, passhash)
         VALUES ($1, $2, $3, 'x') RETURNING id",
    )
    .bind(format!("{username}@example.invalid"))
    .bind(&username)
    .bind(&username)
    .fetch_one(pool)
    .await
    .expect("create user")
}

async fn create_bear(pool: &PgPool) -> Uuid {
    let suffix = Uuid::new_v4().simple().to_string();
    sqlx::query_scalar("INSERT INTO bears (slug, name) VALUES ($1, $2) RETURNING id")
        .bind(format!("ws-bear-{}", &suffix[..12]))
        .bind("Surface Test Bear")
        .fetch_one(pool)
        .await
        .expect("create bear")
}

fn new_surface(name: &str) -> NewWorkSurface {
    NewWorkSurface {
        name: name.to_string(),
        description: Some("test surface".to_string()),
        upstream_url: "https://example.invalid/repo.git".to_string(),
        default_ref: "main".to_string(),
        default_image: Some("base".to_string()),
        allowed_outbound_hosts: Vec::new(),
        credential: None,
    }
}

fn unique_name() -> String {
    let suffix = Uuid::new_v4().simple().to_string();
    format!("surface-{}", &suffix[..12])
}

#[tokio::test]
async fn creator_becomes_owner_and_may_manage() {
    let _guard = DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let user = create_user(&pool).await;
    let other = create_user(&pool).await;

    let surface = create_surface(&pool, user, new_surface(&unique_name()), TEST_SECRET_KEY)
        .await
        .expect("create surface");

    assert!(user_may_manage_surface(&pool, user, surface.id)
        .await
        .expect("check owner"));
    assert!(!user_may_manage_surface(&pool, other, surface.id)
        .await
        .expect("check non-manager"));

    let managers = list_managers(&pool, surface.id).await.expect("managers");
    assert_eq!(managers.len(), 1);
    assert_eq!(managers[0].user_id, user);
    assert_eq!(managers[0].role, SURFACE_ROLE_OWNER);

    // Granted managers may manage; the last owner cannot be revoked.
    grant_manager(&pool, surface.id, other, SURFACE_ROLE_MANAGER, user)
        .await
        .expect("grant manager");
    assert!(user_may_manage_surface(&pool, other, surface.id)
        .await
        .expect("check granted manager"));
    let err = revoke_manager(&pool, surface.id, user).await.unwrap_err();
    assert!(err.to_string().contains("last owner"), "got: {err}");
    revoke_manager(&pool, surface.id, other)
        .await
        .expect("revoke manager");
}

#[tokio::test]
async fn rejects_invalid_names() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let user = create_user(&pool).await;
    for bad in ["../evil", "a/b", "", "-leading", "UPPER"] {
        let err = create_surface(&pool, user, new_surface(bad), TEST_SECRET_KEY)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("invalid surface name"),
            "name {bad:?} got: {err}"
        );
    }
}

#[tokio::test]
async fn bear_assignment_gates_use() {
    let _guard = DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let user = create_user(&pool).await;
    let bear = create_bear(&pool).await;
    let surface = create_surface(&pool, user, new_surface(&unique_name()), TEST_SECRET_KEY)
        .await
        .expect("create surface");

    assert!(!bear_may_use_surface(&pool, bear, surface.id)
        .await
        .expect("unassigned"));
    assign_bear(&pool, surface.id, bear, user)
        .await
        .expect("assign");
    assert!(bear_may_use_surface(&pool, bear, surface.id)
        .await
        .expect("assigned"));

    let for_bears = list_surfaces_for_bears(&pool, &[bear]).await.expect("list");
    assert!(for_bears
        .iter()
        .any(|s| s.id == surface.id && s.bear_id == bear));

    unassign_bear(&pool, surface.id, bear)
        .await
        .expect("unassign");
    assert!(!bear_may_use_surface(&pool, bear, surface.id)
        .await
        .expect("after unassign"));
}

#[tokio::test]
async fn credential_roundtrips_through_managed_config() {
    let _guard = DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let user = create_user(&pool).await;
    let name = unique_name();
    let mut surface = new_surface(&name);
    surface.credential = Some((
        CREDENTIAL_KIND_HTTPS_TOKEN.to_string(),
        "ghp_secret_token_value".to_string(),
    ));
    let row = create_surface(&pool, user, surface, TEST_SECRET_KEY)
        .await
        .expect("create surface");
    // Listings expose the kind, never the ciphertext (the row shape has no
    // credential field beyond the kind).
    assert_eq!(row.credential_kind.as_deref(), Some("https_token"));

    let config = build_managed_config(&pool, TEST_SECRET_KEY)
        .await
        .expect("build config");
    let synced = config
        .surfaces
        .iter()
        .find(|s| s.name == name)
        .expect("surface in config");
    match &synced.credential {
        Some(ManagedCredential::HttpsToken { token }) => {
            assert_eq!(token, "ghp_secret_token_value");
        }
        other => panic!("expected https token credential, got {other:?}"),
    }

    // Version is stable across identical content and changes on rotation.
    let version_before = config.version.clone().expect("version");
    let again = build_managed_config(&pool, TEST_SECRET_KEY)
        .await
        .expect("rebuild config");
    assert_eq!(again.version.as_ref(), Some(&version_before));
    set_credential(
        &pool,
        row.id,
        CREDENTIAL_KIND_HTTPS_TOKEN,
        "ghp_rotated_value",
        TEST_SECRET_KEY,
    )
    .await
    .expect("rotate");
    let rotated = build_managed_config(&pool, TEST_SECRET_KEY)
        .await
        .expect("config after rotation");
    assert_ne!(rotated.version.as_ref(), Some(&version_before));

    clear_credential(&pool, row.id).await.expect("clear");
    let cleared = surface_by_id(&pool, row.id)
        .await
        .expect("fetch")
        .expect("exists");
    assert!(cleared.credential_kind.is_none());
}

#[tokio::test]
async fn catalog_single_default_invariant_and_seed() {
    let _guard = DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let user = create_user(&pool).await;

    // Migration seed is present.
    let images = list_catalog_images(&pool).await.expect("list images");
    assert!(images.iter().any(|i| i.name == "base"));

    let suffix = Uuid::new_v4().simple().to_string();
    let name = format!("img-{}", &suffix[..12]);
    let created = create_catalog_image(&pool, &name, "example.invalid/img:latest", None, user)
        .await
        .expect("create image");
    set_default_catalog_image(&pool, created.id)
        .await
        .expect("set default");
    let images = list_catalog_images(&pool).await.expect("list again");
    assert_eq!(images.iter().filter(|i| i.is_default).count(), 1);
    assert!(images.iter().any(|i| i.id == created.id && i.is_default));

    // Restore a stable default for other tests, then clean up.
    let base = images.iter().find(|i| i.name == "base").expect("base");
    set_default_catalog_image(&pool, base.id)
        .await
        .expect("restore default");
    delete_catalog_image(&pool, created.id)
        .await
        .expect("delete image");
}

#[tokio::test]
async fn surface_update_and_delete() {
    let _guard = DB_LOCK.lock().await;
    let Some(pool) = test_pool().await else {
        return;
    };
    let user = create_user(&pool).await;
    let surface = create_surface(&pool, user, new_surface(&unique_name()), TEST_SECRET_KEY)
        .await
        .expect("create surface");

    let updated = update_surface(
        &pool,
        surface.id,
        WorkSurfaceUpdate {
            description: Some(None),
            upstream_url: Some("https://example.invalid/other.git".to_string()),
            default_ref: Some("trunk".to_string()),
            default_image: Some(Some("rust".to_string())),
            allowed_outbound_hosts: None,
            github_app_installation_id: None,
            github_app_write_enabled: None,
        },
    )
    .await
    .expect("update");
    assert_eq!(updated.name, surface.name, "name is immutable");
    assert!(updated.description.is_none());
    assert_eq!(updated.upstream_url, "https://example.invalid/other.git");
    assert_eq!(updated.default_ref, "trunk");
    assert_eq!(updated.default_image.as_deref(), Some("rust"));

    let github_app = update_surface(
        &pool,
        surface.id,
        WorkSurfaceUpdate {
            github_app_installation_id: Some(Some(12345)),
            github_app_write_enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .expect("enable GitHub App publishing");
    assert_eq!(github_app.github_app_installation_id, Some(12345));
    assert!(github_app.github_app_write_enabled);

    delete_surface(&pool, surface.id).await.expect("delete");
    assert!(surface_by_id(&pool, surface.id)
        .await
        .expect("fetch")
        .is_none());
}
