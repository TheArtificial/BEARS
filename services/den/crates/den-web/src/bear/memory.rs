//! Bear memory + entity admin views at `/bear/{slug}/memory…` and `/bear/{slug}/entities…`.
//!
//! Read surface for members; delete / review-request gated to bear admins. Canonical memory
//! lives in per-Bear SQLite (ADR-0031); the recall index (Qdrant + Postgres `recall_passages`)
//! and the entity layer (ADR-0042) are derived/auxiliary and may be empty. Five use-cases:
//! a dashboard ("how much memory"), a recent feed, search, library browse, and per-entry +
//! per-entity detail pages that cross-link via the `memory_links` relation view.

use axum::{
    extract::{Multipart, Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use axum_extra::extract::Form;
use axum_extra::routing::RouterExt;
use minijinja::context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path as FsPath, PathBuf};
use uuid::Uuid;

use crate::{
    errors::CustomError,
    web::{self, AppState},
};

use super::settings::{bear_nav_context, load_session_bear, session_user};
use den_memory::{
    self as store, bear_memory_admin_stats, count_records_by_kind, count_records_by_profile,
    get_memory_record_detail, head_entry_count, import_legacy_memory_bundle, list_path_summaries,
    list_recent_memory_records, list_relations_for_entity, list_relations_for_source,
    search_memory_records, LegacyMemoryImportOptions, MemoryRecordRow, MemoryStoreManager, PathSummary,
};
use den_service::bears::{db as bears_db, BearProfile};
use den_service::recall::{registry as recall_registry, semantic_search_for_bear};
use den_service::{
    memory_proposals::{self, CreateMemoryProposal},
    pair_reflection,
};

use super::member::{email_verify_redirect, load_bear_member, viewer_can_manage_bear};

pub fn router() -> Router<AppState> {
    Router::new()
        .route_with_tsr("/bear/{slug}/memory", get(dashboard_view))
        .route_with_tsr(
            "/bear/{slug}/memory/import-legacy",
            post(import_legacy_memory_post),
        )
        .route_with_tsr("/bear/{slug}/memory/recent", get(recent_view))
        .route_with_tsr("/bear/{slug}/memory/search", get(search_view))
        .route_with_tsr(
            "/bear/{slug}/memory/browse",
            get(browse_view).post(browse_delete_post),
        )
        .route_with_tsr("/bear/{slug}/memory/records/{memory_id}", get(record_view))
        .route_with_tsr(
            "/bear/{slug}/memory/proposals/{proposal_id}",
            get(proposal_get).post(proposal_post),
        )
        .route_with_tsr("/bear/{slug}/entities", get(entities_view))
        .route_with_tsr("/bear/{slug}/entities/{entity_id}", get(entity_detail_view))
}

// ---------------------------------------------------------------------------
// Query / form types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DashboardQuery {
    #[serde(default)]
    import_notice: Option<String>,
    #[serde(default)]
    import_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrowseQuery {
    #[serde(default)]
    deleted: Option<usize>,
    #[serde(default)]
    review_requested: Option<usize>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EntitiesQuery {
    #[serde(default)]
    r#type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MemoryDeleteForm {
    role: String,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    confirm: String,
    #[serde(default)]
    review_title: Option<String>,
    #[serde(default)]
    review_summary: Option<String>,
    #[serde(default)]
    review_rationale: Option<String>,
    #[serde(default)]
    suggested_action: Option<String>,
    #[serde(default)]
    sensitivity: Option<String>,
    #[serde(default)]
    requires_human: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MemoryProposalResolutionForm {
    status: String,
    #[serde(default)]
    review_notes: Option<String>,
    #[serde(default)]
    decision_summary: Option<String>,
}

// ---------------------------------------------------------------------------
// View-model rows
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct RecordListItem {
    memory_id: String,
    kind: String,
    scope_label: String,
    logical_path: Option<String>,
    created_at: String,
    sequence_no: i64,
    snippet: String,
    score: Option<f32>,
}

#[derive(Debug, Serialize)]
struct PathGroup {
    label: String,
    paths: Vec<PathSummary>,
}

#[derive(Debug, Serialize)]
struct HistoryItem {
    memory_id: String,
    sequence_no: i64,
    kind: String,
    created_at: String,
    is_current: bool,
}

#[derive(Debug, Serialize)]
struct LinkedEntity {
    entity_id: String,
    display_name: String,
    entity_type: String,
    relation_label: String,
    class_label: String,
}

#[derive(Debug, Serialize)]
struct EntityListItem {
    entity_id: String,
    display_name: String,
    entity_type: String,
    resolution: String,
    trust: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct LinkedRecord {
    src_memory_id: String,
    relation_label: String,
    class_label: String,
    author_profile: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct EntityDetail {
    entity_id: String,
    sequence_no: i64,
    entity_type: String,
    display_name: String,
    resolution: String,
    trust: String,
    canonical_ref: Option<String>,
    superseded_by_entity_id: Option<String>,
    metadata_json: Value,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct HandleItem {
    handle_type: String,
    handle_value: String,
    source: Option<String>,
    trust: String,
    state: String,
}

#[derive(Debug, Serialize)]
struct CountBucket {
    label: String,
    count: i64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn snippet(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        collapsed
    } else {
        let head: String = collapsed.chars().take(max).collect();
        format!("{head}…")
    }
}

fn scope_label(scope_profile: Option<&str>) -> String {
    match scope_profile {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => "core".to_string(),
    }
}

/// Strip the `den.memory.relation.` prefix to a short, human label.
fn relation_label(relation: &str) -> String {
    relation
        .rsplit('.')
        .next()
        .unwrap_or(relation)
        .replace('_', " ")
}

fn class_label(class: &store::RelationClass) -> &'static str {
    match class {
        store::RelationClass::AccessBearing => "access-bearing",
        store::RelationClass::Descriptive => "descriptive",
    }
}

fn record_list_item(row: MemoryRecordRow, score: Option<f32>) -> RecordListItem {
    RecordListItem {
        scope_label: scope_label(row.scope_profile.as_deref()),
        snippet: snippet(&row.content_text, 240),
        memory_id: row.memory_id,
        kind: row.kind,
        logical_path: row.logical_path,
        created_at: row.created_at,
        sequence_no: row.sequence_no,
        score,
    }
}

/// First path segment (`core`, `pair`, `work`, …) used to group the library browse view.
fn path_group_label(logical_path: &str) -> String {
    logical_path
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("other")
        .to_string()
}

// ---------------------------------------------------------------------------
// Dashboard — "how much memory"
// ---------------------------------------------------------------------------

const LETTA_IMPORT_MAX_UPLOAD_BYTES: usize = 128 * 1024 * 1024;

fn dashboard_redirect_with_query(slug: &str, key: &str, message: &str) -> Response {
    let encoded = urlencoding::encode(message);
    Redirect::to(&format!("/bear/{slug}/memory?{key}={encoded}")).into_response()
}

fn looks_like_git_bundle(bytes: &[u8]) -> bool {
    let first_line = bytes.split(|b| *b == b'\n').next().unwrap_or(&[]);
    if first_line.is_empty() {
        return false;
    }
    String::from_utf8_lossy(first_line)
        .to_ascii_lowercase()
        .contains("git bundle")
}

fn import_dir_for_bear(config: &den_core::config::Config, bear_id: Uuid) -> PathBuf {
    FsPath::new(&config.bear_sqlite_data_dir)
        .join("imports")
        .join(bear_id.to_string())
}

async fn import_staged_bundle(
    state: &AppState,
    bear_id: Uuid,
    bundle_path: &FsPath,
) -> Result<den_memory::LegacyMemoryImportReport, CustomError> {
    let stores = MemoryStoreManager::new(state.config.as_ref());
    let store = stores.store_for_bear(bear_id).await?;
    let record_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM memory_records WHERE bear_id = ?")
            .bind(bear_id.to_string())
            .fetch_one(store.pool())
            .await?;
    if record_count > 0 {
        return Err(CustomError::ValidationError(
            "Legacy memory import is disabled for Bears that already have memory records."
                .to_string(),
        ));
    }

    let report = import_legacy_memory_bundle(
        &store,
        bundle_path,
        &LegacyMemoryImportOptions {
            dry_run: false,
            include_workflow_artifacts: false,
            import_history: false,
        },
    )
    .await?;

    let report_path = bundle_path.with_extension("report.json");
    if let Ok(report_json) = serde_json::to_string_pretty(&report) {
        if let Err(err) = std::fs::write(&report_path, report_json) {
            tracing::warn!(bear_id = %bear_id, error = %err, path = %report_path.display(), "failed to write legacy import report next to staged bundle");
        }
    }

    Ok(report)
}

async fn dashboard_view(
    Path(slug): Path<String>,
    Query(query): Query<DashboardQuery>,
    State(state): State<AppState>,
    auth_session: crate::auth_backend::AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let id = bear.id;
    let config = state.config.as_ref();
    let manager = MemoryStoreManager::new(config);

    let stats = bear_memory_admin_stats(&manager, config, id).await.ok();
    let legacy_import_locked = stats.as_ref().map(|s| s.record_count > 0).unwrap_or(true);
    let head_count = head_entry_count(&manager, id).await.unwrap_or(0);
    let by_kind = count_records_by_kind(&manager, id)
        .await
        .unwrap_or_default();
    let by_profile = count_records_by_profile(&manager, id)
        .await
        .unwrap_or_default();

    // Derived recall coverage (Postgres registry). Only meaningful when recall is configured.
    let recall = if config.qdrant_url.is_some() {
        let (passages, memories) =
            recall_registry::passage_stats(state.sqlx_pool(), id, &config.embedding_standard)
                .await
                .unwrap_or((0, 0));
        Some(json!({ "enabled": true, "passages": passages, "memories": memories }))
    } else {
        None
    };

    // Entity layer summary (ADR-0042). Populated as the Bear resolves entities (Phase 6+);
    // empty for most Bears today.
    let entity_summary = entity_summary(&manager, id)
        .await
        .unwrap_or_else(|_| json!({ "total": 0, "by_type": [], "by_resolution": [] }));

    let recent: Vec<RecordListItem> = list_recent_memory_records(&manager, id, 8)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| record_list_item(r, None))
        .collect();

    let proposals = memory_proposals::list_for_bear(state.sqlx_pool(), id, None, 10)
        .await
        .unwrap_or_default();
    let pair_reflection_runs = pair_reflection::list_recent_for_bear(state.sqlx_pool(), id, 8)
        .await
        .unwrap_or_default();

    web::render_template(
        &state,
        "bear/memory/dashboard.html",
        auth_session,
        context! {
            stats,
            head_count,
            by_kind,
            by_profile,
            recall,
            entity_summary,
            recent,
            proposals,
            pair_reflection_runs,
            import_notice => query.import_notice.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            import_error => query.import_error.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            legacy_import_locked,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "memory"),
        },
    )
    .await
}

async fn entity_summary(manager: &MemoryStoreManager, bear_id: Uuid) -> Result<Value, CustomError> {
    let store = manager.store_for_bear(bear_id).await?;
    let entities = store::list_entities(&store, None, 500).await?;
    let total = entities.len();
    let mut by_type: std::collections::BTreeMap<String, i64> = Default::default();
    let mut by_resolution: std::collections::BTreeMap<String, i64> = Default::default();
    for e in &entities {
        *by_type.entry(e.entity_type.clone()).or_default() += 1;
        *by_resolution
            .entry(e.resolution.as_str().to_string())
            .or_default() += 1;
    }
    let to_buckets = |m: std::collections::BTreeMap<String, i64>| -> Vec<CountBucket> {
        let mut v: Vec<CountBucket> = m
            .into_iter()
            .map(|(label, count)| CountBucket { label, count })
            .collect();
        v.sort_by(|a, b| b.count.cmp(&a.count).then(a.label.cmp(&b.label)));
        v
    };
    Ok(json!({
        "total": total,
        "by_type": to_buckets(by_type),
        "by_resolution": to_buckets(by_resolution),
    }))
}

// ---------------------------------------------------------------------------
// Recent additions feed
// ---------------------------------------------------------------------------

async fn recent_view(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: crate::auth_backend::AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let records: Vec<RecordListItem> = list_recent_memory_records(&manager, bear.id, 50)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| record_list_item(r, None))
        .collect();
    web::render_template(
        &state,
        "bear/memory/recent.html",
        auth_session,
        context! {
            records,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "memory"),
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// Search (keyword always; semantic when recall is configured)
// ---------------------------------------------------------------------------

async fn search_view(
    Path(slug): Path<String>,
    Query(query): Query<SearchQuery>,
    State(state): State<AppState>,
    auth_session: crate::auth_backend::AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let config = state.config.as_ref();
    let manager = MemoryStoreManager::new(config);
    let q = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let semantic_available = config.qdrant_url.is_some() && !config.llm_api_url.trim().is_empty();
    let want_semantic = query.mode.as_deref() == Some("semantic") && semantic_available;

    let mut mode_used = "keyword";
    let mut notice: Option<String> = None;
    let mut results: Vec<RecordListItem> = Vec::new();

    if let Some(q) = q {
        if want_semantic {
            match semantic_search_for_bear(config, bear.id, q, 25).await {
                Ok(projection) if !projection.passages.is_empty() => {
                    mode_used = "semantic";
                    results = projection
                        .passages
                        .into_iter()
                        .map(|p| RecordListItem {
                            memory_id: p.memory_id,
                            kind: p.kind.unwrap_or_else(|| "memory".to_string()),
                            scope_label: String::new(),
                            logical_path: p.logical_path,
                            created_at: String::new(),
                            sequence_no: 0,
                            snippet: snippet(&p.text, 240),
                            score: Some(p.score),
                        })
                        .collect();
                }
                Ok(_) => {
                    notice = Some(
                        "Semantic search returned no matches; showing keyword results.".to_string(),
                    );
                }
                Err(err) => {
                    tracing::warn!(bear_id = %bear.id, error = %err, "semantic search failed; keyword fallback");
                    notice = Some(
                        "Semantic search is unavailable; showing keyword results.".to_string(),
                    );
                }
            }
        }
        if results.is_empty() {
            results = search_memory_records(&manager, bear.id, q, 50)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|r| record_list_item(r, None))
                .collect();
        }
    }

    let result_count = results.len();
    web::render_template(
        &state,
        "bear/memory/search.html",
        auth_session,
        context! {
            q => q.unwrap_or(""),
            results,
            result_count,
            mode_used,
            semantic_available,
            notice,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "memory"),
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// Browse the library
// ---------------------------------------------------------------------------

async fn browse_view(
    Path(slug): Path<String>,
    Query(query): Query<BrowseQuery>,
    State(state): State<AppState>,
    auth_session: crate::auth_backend::AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let summaries = list_path_summaries(&manager, bear.id)
        .await
        .unwrap_or_default();

    // Group by first path segment, preserving a stable, meaningful order.
    let mut groups: Vec<PathGroup> = Vec::new();
    for summary in summaries {
        let label = path_group_label(&summary.logical_path);
        if let Some(group) = groups.iter_mut().find(|g| g.label == label) {
            group.paths.push(summary);
        } else {
            groups.push(PathGroup {
                label,
                paths: vec![summary],
            });
        }
    }
    groups.sort_by(|a, b| {
        group_rank(&a.label)
            .cmp(&group_rank(&b.label))
            .then(a.label.cmp(&b.label))
    });

    web::render_template(
        &state,
        "bear/memory/browse.html",
        auth_session,
        context! {
            groups,
            delete_notice => query.deleted,
            review_notice => query.review_requested,
            delete_error => query.error.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "memory"),
        },
    )
    .await
}

/// Canonical-first ordering for the browse groups (`core` shared memory before role branches).
fn group_rank(label: &str) -> usize {
    match label {
        "core" => 0,
        "pair" => 1,
        "work" => 2,
        "curate" => 3,
        "talk" => 4,
        "watch" => 5,
        _ => 9,
    }
}

async fn import_legacy_memory_post(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: crate::auth_backend::AuthSession,
    mut multipart: Multipart,
) -> Result<Response, CustomError> {
    let user = session_user(&auth_session).await?;
    if let Some(r) = email_verify_redirect(state.sqlx_pool(), user.id).await? {
        return Ok(r.into_response());
    }
    let bear = load_bear_member(state.sqlx_pool(), user.id, &slug).await?;
    if !viewer_can_manage_bear(state.sqlx_pool(), user, bear.id).await? {
        return Err(CustomError::Authorization(
            "bear admin role required".to_string(),
        ));
    }

    let mut bundle_bytes: Option<Vec<u8>> = None;
    while let Some(mut field) = match multipart.next_field().await {
        Ok(field) => field,
        Err(err) => {
            tracing::warn!(bear_id = %bear.id, error = %err, "invalid legacy bundle multipart upload");
            return Ok(dashboard_redirect_with_query(
                &bear.slug,
                "import_error",
                "Invalid multipart upload.",
            ));
        }
    } {
        if field.name() != Some("bundle") {
            continue;
        }
        let mut data = Vec::new();
        while let Some(chunk) = match field.chunk().await {
            Ok(chunk) => chunk,
            Err(err) => {
                tracing::warn!(bear_id = %bear.id, error = %err, "failed reading legacy bundle upload field");
                return Ok(dashboard_redirect_with_query(
                    &bear.slug,
                    "import_error",
                    "Failed reading uploaded bundle.",
                ));
            }
        } {
            if data.len() + chunk.len() > LETTA_IMPORT_MAX_UPLOAD_BYTES {
                return Ok(dashboard_redirect_with_query(
                    &bear.slug,
                    "import_error",
                    "Bundle exceeds the 128 MiB upload limit.",
                ));
            }
            data.extend_from_slice(&chunk);
        }
        bundle_bytes = Some(data);
        break;
    }

    let bundle_bytes = match bundle_bytes {
        Some(bytes) if !bytes.is_empty() => bytes,
        _ => {
            return Ok(dashboard_redirect_with_query(
                &bear.slug,
                "import_error",
                "Please select a bundle file.",
            ));
        }
    };

    if !looks_like_git_bundle(&bundle_bytes) {
        return Ok(dashboard_redirect_with_query(
            &bear.slug,
            "import_error",
            "Upload must be a git bundle.",
        ));
    }

    let stores = MemoryStoreManager::new(state.config.as_ref());
    let store = stores.store_for_bear(bear.id).await?;
    let record_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM memory_records WHERE bear_id = ?")
            .bind(bear.id.to_string())
            .fetch_one(store.pool())
            .await?;
    if record_count > 0 {
        return Ok(dashboard_redirect_with_query(
            &bear.slug,
            "import_error",
            "Legacy memory import is disabled for Bears that already have memory records.",
        ));
    }

    let import_dir = import_dir_for_bear(state.config.as_ref(), bear.id);
    std::fs::create_dir_all(&import_dir).map_err(|err| {
        CustomError::System(format!("failed to create legacy import directory: {err}"))
    })?;

    let file_path = import_dir.join(format!("legacy-memory-{}.bundle", Uuid::new_v4()));
    std::fs::write(&file_path, &bundle_bytes)
        .map_err(|err| CustomError::System(format!("failed to stage legacy bundle: {err}")))?;

    let report = match import_staged_bundle(&state, bear.id, &file_path).await {
        Ok(report) => report,
        Err(err) => {
            tracing::warn!(bear_id = %bear.id, error = %err, path = %file_path.display(), "legacy bundle import failed after staging");
            if let Err(delete_err) = std::fs::remove_file(&file_path) {
                tracing::warn!(bear_id = %bear.id, error = %delete_err, path = %file_path.display(), "failed to discard staged legacy bundle after import failure");
            }
            return Ok(dashboard_redirect_with_query(
                &bear.slug,
                "import_error",
                "Bundle upload succeeded, but importing into SQLite failed. The failed upload was discarded; check Den logs for the importer error.",
            ));
        }
    };

    Ok(dashboard_redirect_with_query(
        &bear.slug,
        "import_notice",
        &format!(
            "Imported {} memory paths (skipped {}, quarantined {}) from the uploaded legacy bundle.",
            report.imported_count, report.skipped_count, report.quarantined_count
        ),
    ))
}

async fn browse_delete_post(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    auth_session: crate::auth_backend::AuthSession,
    Form(form): Form<MemoryDeleteForm>,
) -> Result<Response, CustomError> {
    let user = session_user(&auth_session).await?;
    if let Some(r) = email_verify_redirect(state.sqlx_pool(), user.id).await? {
        return Ok(r.into_response());
    }
    let bear = load_bear_member(state.sqlx_pool(), user.id, &slug).await?;
    if !viewer_can_manage_bear(state.sqlx_pool(), user, bear.id).await? {
        return Err(CustomError::Authorization(
            "bear admin role required".to_string(),
        ));
    }
    let role = form
        .role
        .parse::<BearProfile>()
        .map_err(CustomError::ValidationError)?;
    let action = form.action.as_deref().unwrap_or("delete").trim();
    let confirm = form.confirm.trim();
    let mut paths = form
        .paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let browse_base = format!("/bear/{}/memory/browse", bear.slug);
    if paths.is_empty() {
        return Ok(Redirect::to(&format!(
            "{browse_base}?error={}",
            urlencoding::encode("Select at least one memory path.")
        ))
        .into_response());
    }
    if action == "request_review" {
        let title = form
            .review_title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Review selected memory");
        let summary = form
            .review_summary
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Selected memory paths were marked for Reflection/curate review from the Bear memory UI.");
        let proposal = memory_proposals::create(
            state.sqlx_pool(),
            CreateMemoryProposal {
                bear_id: bear.id,
                source_profile: role,
                source_agent_id: bears_db::profile_binding_id(state.sqlx_pool(), bear.id, role)
                    .await?,
                source_paths: paths,
                source_refs: serde_json::json!([]),
                suggested_action: form
                    .suggested_action
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("unspecified"),
                target_ref: None,
                title,
                summary,
                rationale: form
                    .review_rationale
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or(""),
                proposed_content: None,
                proposed_patch: None,
                refs: serde_json::json!({}),
                sensitivity: form
                    .sensitivity
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("normal"),
                requires_human: form.requires_human.as_deref() == Some("on"),
                project_to_conversation: true,
            },
        )
        .await?;
        return Ok(Redirect::to(&format!(
            "{browse_base}?review_requested=1&path={}",
            urlencoding::encode(
                proposal
                    .source_paths
                    .first()
                    .map(String::as_str)
                    .unwrap_or("")
            )
        ))
        .into_response());
    }
    if confirm != role.as_str() && confirm != bear.slug {
        return Ok(Redirect::to(&format!(
            "{browse_base}?error={}",
            urlencoding::encode("Type the profile name or Bear slug to confirm deletion.")
        ))
        .into_response());
    }
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let store = manager.store_for_bear(bear.id).await?;
    let mut deleted = 0usize;
    for path in &paths {
        let result = sqlx::query(
            "DELETE FROM memory_records WHERE bear_id = ? AND scope_profile = ? AND logical_path = ?",
        )
        .bind(bear.id.to_string())
        .bind(role.as_str())
        .bind(path)
        .execute(store.pool())
        .await
        .map_err(|err| CustomError::System(format!("delete memory records failed: {err}")))?;
        if result.rows_affected() > 0 {
            deleted += 1;
        }
    }
    Ok(Redirect::to(&format!("{browse_base}?deleted={deleted}")).into_response())
}

// ---------------------------------------------------------------------------
// Single memory entry: content + history + referenced entities + recall status
// ---------------------------------------------------------------------------

async fn record_view(
    Path((slug, memory_id)): Path<(String, String)>,
    State(state): State<AppState>,
    auth_session: crate::auth_backend::AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let config = state.config.as_ref();
    let manager = MemoryStoreManager::new(config);
    let record = get_memory_record_detail(&manager, bear.id, &memory_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("memory record not found".to_string()))?;

    let store = manager.store_for_bear(bear.id).await?;

    // History: all versions at this logical path (newest first). The newest is the head today
    // (supersession is latest-at-path by sequence; `supersedes_memory_id` is not yet written).
    let mut history: Vec<HistoryItem> = Vec::new();
    if let Some(path) = record.logical_path.as_deref() {
        let versions = store::list_records_for_logical_path(&store, path, 50)
            .await
            .unwrap_or_default();
        for (idx, v) in versions.iter().enumerate() {
            history.push(HistoryItem {
                is_current: idx == 0,
                memory_id: v.memory_id.clone(),
                sequence_no: v.sequence_no,
                kind: v.kind.clone(),
                created_at: v.created_at.clone(),
            });
        }
    }
    let is_head = history
        .first()
        .map(|h| h.memory_id == memory_id)
        .unwrap_or(true);

    // Referenced entities (descriptive + access-bearing) via the relation view.
    let mut entities: Vec<LinkedEntity> = Vec::new();
    for rel in list_relations_for_source(&store, &memory_id, 50)
        .await
        .unwrap_or_default()
    {
        let (display_name, entity_type) = match store::get_entity(&store, &rel.entity_id).await {
            Ok(Some(e)) => (
                e.display_name.unwrap_or_else(|| e.entity_id.clone()),
                e.entity_type,
            ),
            _ => (rel.entity_id.clone(), "unknown".to_string()),
        };
        entities.push(LinkedEntity {
            entity_id: rel.entity_id,
            display_name,
            entity_type,
            relation_label: relation_label(&rel.relation),
            class_label: class_label(&rel.class).to_string(),
        });
    }

    // Recall coverage for this record.
    let recall = if config.qdrant_url.is_some() {
        let passages = recall_registry::list_passages(
            state.sqlx_pool(),
            bear.id,
            &memory_id,
            &config.embedding_standard,
        )
        .await
        .map(|p| p.len())
        .unwrap_or(0);
        Some(json!({ "enabled": true, "passages": passages }))
    } else {
        None
    };

    web::render_template(
        &state,
        "bear/memory/record.html",
        auth_session,
        context! {
            record,
            is_head,
            history,
            entities,
            recall,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "memory"),
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

async fn entities_view(
    Path(slug): Path<String>,
    Query(query): Query<EntitiesQuery>,
    State(state): State<AppState>,
    auth_session: crate::auth_backend::AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let store = manager.store_for_bear(bear.id).await?;
    let type_filter = query
        .r#type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let rows = store::list_entities(&store, type_filter, 200)
        .await
        .unwrap_or_default();

    // Distinct types present, for the filter chips.
    let mut types: Vec<String> = store::list_entities(&store, None, 500)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.entity_type)
        .collect();
    types.sort();
    types.dedup();

    let entities: Vec<EntityListItem> = rows
        .into_iter()
        .map(|e| EntityListItem {
            display_name: e.display_name.unwrap_or_else(|| e.entity_id.clone()),
            entity_id: e.entity_id,
            entity_type: e.entity_type,
            resolution: e.resolution.as_str().to_string(),
            trust: e.trust.as_str().to_string(),
            created_at: e.created_at,
        })
        .collect();

    web::render_template(
        &state,
        "bear/memory/entities.html",
        auth_session,
        context! {
            entities,
            types,
            type_filter => type_filter.unwrap_or(""),
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "entities"),
        },
    )
    .await
}

async fn entity_detail_view(
    Path((slug, entity_id)): Path<(String, String)>,
    State(state): State<AppState>,
    auth_session: crate::auth_backend::AuthSession,
) -> Result<Response, CustomError> {
    let (bear, can_manage_bear) = match load_session_bear(&state, &auth_session, &slug).await? {
        Ok(v) => v,
        Err(r) => return Ok(r.into_response()),
    };
    let manager = MemoryStoreManager::new(state.config.as_ref());
    let store = manager.store_for_bear(bear.id).await?;
    let row = store::get_entity(&store, &entity_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("entity not found".to_string()))?;
    let entity = EntityDetail {
        display_name: row
            .display_name
            .clone()
            .unwrap_or_else(|| row.entity_id.clone()),
        entity_id: row.entity_id,
        sequence_no: row.sequence_no,
        entity_type: row.entity_type,
        resolution: row.resolution.as_str().to_string(),
        trust: row.trust.as_str().to_string(),
        canonical_ref: row.canonical_ref,
        superseded_by_entity_id: row.superseded_by_entity_id,
        metadata_json: row.metadata_json,
        created_at: row.created_at,
    };
    let handles: Vec<HandleItem> = store::list_handles(&store, &entity_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|h| HandleItem {
            handle_type: h.handle_type,
            handle_value: h.handle_value,
            source: h.source,
            trust: h.trust.as_str().to_string(),
            state: h.state,
        })
        .collect();

    let related: Vec<LinkedRecord> = list_relations_for_entity(&store, &entity_id, 100)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|rel| LinkedRecord {
            src_memory_id: rel.src_memory_id,
            relation_label: relation_label(&rel.relation),
            class_label: class_label(&rel.class).to_string(),
            author_profile: rel.author_profile,
            created_at: rel.created_at,
        })
        .collect();

    web::render_template(
        &state,
        "bear/memory/entity.html",
        auth_session,
        context! {
            entity,
            handles,
            related,
            can_manage_bear,
            native_runtime => true,
            ..bear_nav_context(&bear, "entities"),
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// Memory proposals (review/governance)
// ---------------------------------------------------------------------------

async fn proposal_get(
    Path((slug, proposal_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    auth_session: crate::auth_backend::AuthSession,
) -> Result<Response, CustomError> {
    let user = session_user(&auth_session).await?;
    if let Some(r) = email_verify_redirect(state.sqlx_pool(), user.id).await? {
        return Ok(r.into_response());
    }
    let bear = load_bear_member(state.sqlx_pool(), user.id, &slug).await?;
    let can_manage_bear = viewer_can_manage_bear(state.sqlx_pool(), user, bear.id).await?;
    let proposal = memory_proposals::get_for_bear(state.sqlx_pool(), bear.id, proposal_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("memory proposal not found".to_string()))?;
    web::render_template(
        &state,
        "bear/memory_proposal.html",
        auth_session,
        context! {
            bear,
            proposal,
            can_manage_bear,
            errors => None::<String>,
            ..bear_nav_context(&bear, "memory"),
        },
    )
    .await
}

async fn proposal_post(
    Path((slug, proposal_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    auth_session: crate::auth_backend::AuthSession,
    Form(form): Form<MemoryProposalResolutionForm>,
) -> Result<Response, CustomError> {
    let user = session_user(&auth_session).await?;
    if let Some(r) = email_verify_redirect(state.sqlx_pool(), user.id).await? {
        return Ok(r.into_response());
    }
    let bear = load_bear_member(state.sqlx_pool(), user.id, &slug).await?;
    if !viewer_can_manage_bear(state.sqlx_pool(), user, bear.id).await? {
        return Err(CustomError::Authorization(
            "bear admin role required".to_string(),
        ));
    }
    let status = form.status.trim();
    if !matches!(
        status,
        "rejected" | "retained_local" | "deferred" | "superseded" | "needs_human_review"
    ) {
        return Err(CustomError::ValidationError(
            "invalid memory proposal status".to_string(),
        ));
    }
    memory_proposals::resolve_for_bear(
        state.sqlx_pool(),
        memory_proposals::ProposalResolutionParams {
            bear_id: bear.id,
            proposal_id,
            reviewer_profile: BearProfile::Curate,
            reviewer_agent_id: None,
            status,
            review_notes: form.review_notes.as_deref(),
            decision_summary: form.decision_summary.as_deref(),
            result_path: None,
            result_commit: None,
            project_to_conversation: true,
        },
    )
    .await?;
    Ok(Redirect::to(&format!(
        "/bear/{}/memory/proposals/{proposal_id}",
        bear.slug
    ))
    .into_response())
}

#[cfg(test)]
mod tests;
