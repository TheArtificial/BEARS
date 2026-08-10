//! Profile detail view model for bear settings.

use crate::{errors::CustomError, web::AppState};
use den_llm::ModelOption;
use den_service::bears::{
    compose_role_context, db as bears_db, Bear, BearProfile, BearProfileBinding,
};
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct RoleDetailView {
    pub(crate) profile: String,
    pub(crate) label: String,
    pub(crate) plain_name: &'static str,
    pub(crate) description: String,
    pub(crate) surfaces: Vec<&'static str>,
    pub(crate) capabilities: Vec<&'static str>,
    pub(crate) memory: Vec<&'static str>,
    pub(crate) actions: Vec<RoleActionLink>,
    pub(crate) runtime_family: String,
    pub(crate) provisioning_status: String,
    pub(crate) last_provisioning_error: Option<String>,
    pub(crate) memory_status_label: String,
    pub(crate) memory_file_count: usize,
    pub(crate) configured_model: String,
    pub(crate) resolved_model: String,
    pub(crate) model_source: String,
    pub(crate) model_availability_status: String,
    pub(crate) model_metadata_status: String,
    pub(crate) configured_model_custom: String,
    pub(crate) model_options: Vec<ModelOption>,
    pub(crate) all_model_options: Vec<ModelOption>,
    pub(crate) role_contract: String,
    pub(crate) composed_prompt: String,
}

#[derive(Serialize)]
pub(crate) struct RoleActionLink {
    pub(crate) label: &'static str,
    pub(crate) href: String,
}

struct BearRoleViewRow {
    provisioning_status: String,
    last_provisioning_error: Option<String>,
}

impl BearRoleViewRow {
    fn from_agent(agent: BearProfileBinding) -> Self {
        Self {
            provisioning_status: agent.provisioning_status,
            last_provisioning_error: agent.last_provisioning_error,
        }
    }
}

pub(crate) fn role_memory_label(role: BearProfile) -> &'static str {
    match role {
        BearProfile::Chat => "Conversation memory",
        BearProfile::Pair => "Pairing memory",
        BearProfile::Curate => "Review memory",
        BearProfile::Work => "Work memory",
        BearProfile::Watch => "Watch memory",
    }
}

fn role_memory_description(role: BearProfile) -> &'static str {
    match role {
        BearProfile::Chat => "Notes and local memory from chat-like conversations.",
        BearProfile::Pair => "Coding collaboration notes, logs, decisions, and summaries.",
        BearProfile::Curate => "Review, reflection, and memory integration work.",
        BearProfile::Work => "Task execution logs, decisions, and summaries.",
        BearProfile::Watch => "Event/subscription logs and summaries.",
    }
}

fn role_plain_name(role: BearProfile) -> &'static str {
    match role {
        BearProfile::Chat => "Conversational front door",
        BearProfile::Pair => "Collaborative tool/IDE partner",
        BearProfile::Curate => "Memory and integration reviewer",
        BearProfile::Work => "Approved outbound executor",
        BearProfile::Watch => "Inbound observer",
    }
}

fn role_surfaces(role: BearProfile) -> Vec<&'static str> {
    match role {
        BearProfile::Chat => vec!["Web chat", "Future chat surfaces"],
        BearProfile::Pair => vec!["ACP clients", "IDEs", "Future design/productivity tools"],
        BearProfile::Curate => vec!["Internal review and integration"],
        BearProfile::Work => vec!["Den task dispatch", "Schedules", "Approved background jobs"],
        BearProfile::Watch => vec![
            "Webhooks",
            "Polling",
            "Queues",
            "Subscriptions",
            "Event streams",
        ],
    }
}

fn role_capabilities(role: BearProfile) -> Vec<&'static str> {
    match role {
        BearProfile::Chat => vec![
            "Synchronous conversation",
            "Task intent capture",
            "Channel-safe tools",
            "Work plan updates",
        ],
        BearProfile::Pair => vec![
            "Client-mediated tool use",
            "Code/workspace context",
            "User-gated actions",
            "File/document collaboration",
        ],
        BearProfile::Curate => vec![
            "Memory review",
            "Task intent review",
            "Observation review",
            "Skill proposal review",
            "Shared memory promotion",
        ],
        BearProfile::Work => vec![
            "Approved API calls",
            "Scheduled tasks",
            "Event-triggered work",
            "Run-status reporting",
        ],
        BearProfile::Watch => vec![
            "Inbound event parsing",
            "Observation creation",
            "Subscription monitoring",
            "Event summarization",
        ],
    }
}

fn role_memory_rules(role: BearProfile) -> Vec<&'static str> {
    match role {
        BearProfile::Chat => vec![
            "Reads core/",
            "Reads and writes chat/",
            "Does not directly promote to core/",
        ],
        BearProfile::Pair => vec![
            "Reads core/",
            "Reads and writes pair/",
            "Does not directly promote to core/",
        ],
        BearProfile::Curate => vec![
            "Reads across role branches, subject to policy",
            "Writes curate/",
            "Promotes durable knowledge into core/",
            "Does not write directly to other role branches",
        ],
        BearProfile::Work => vec![
            "Reads core/",
            "Reads task definition/run context",
            "Reads and writes work/",
            "Does not read raw chat/, pair/, or watch/ directly",
        ],
        BearProfile::Watch => vec![
            "Reads core/",
            "Reads delivered event payloads",
            "Reads and writes watch/",
            "Does not write core/ directly",
            "Does not trigger outbound action directly",
        ],
    }
}

pub(crate) async fn build_role_detail_view(
    state: &AppState,
    bear: &Bear,
    role: BearProfile,
) -> Result<RoleDetailView, CustomError> {
    bears_db::ensure_bear_profile_binding_rows(state.sqlx_pool(), bear.id).await?;
    let agent = bears_db::get_bear_profile_binding(state.sqlx_pool(), bear.id, role)
        .await?
        .ok_or_else(|| CustomError::NotFound("profile runtime binding not found".to_string()))?;

    let role_row = BearRoleViewRow::from_agent(agent);

    let (memory_status_label, memory_file_count) = {
        let manager = state.memory_stores.clone();
        let store = manager.store_for_bear(bear.id).await?;
        match den_memory::tools::sqlite_memory_status(&store, role.as_str()).await {
            Ok(status) => {
                let available = status
                    .get("ok")
                    .and_then(|v| v.as_bool())
                    .or_else(|| status.get("available").and_then(|v| v.as_bool()))
                    .unwrap_or(true);
                let label = if available {
                    "Available"
                } else {
                    "Unavailable"
                }
                .to_string();
                let file_count = status
                    .get("file_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                (label, file_count)
            }
            Err(err) => (format!("Error: {err}"), 0usize),
        }
    };

    let composed = compose_role_context(
        bear,
        role,
        Some("Runtime/conversation context is injected when this role handles a specific task."),
    )?;
    let role_contract = if composed.role_contract.trim().is_empty() {
        "Legacy/manual Bear prompt; no role-aware contract is stored yet.".to_string()
    } else {
        composed.role_contract
    };

    let model_options =
        den_service::model_selection::list_selectable_model_options(state.sqlx_pool())
            .await
            .unwrap_or_else(|_| den_llm::model_registry::selectable_model_options());
    let (_, live_model_options, _) =
        crate::web::bear::create_support::all_model_catalog_options_context_for_bear(
            state, bear.id,
        )
        .await;
    let all_model_options = merge_model_options(&model_options, &live_model_options);
    let profile_settings =
        bears_db::list_profile_model_settings(state.sqlx_pool(), bear.id).await?;
    let configured_model = profile_settings
        .iter()
        .find(|row| row.profile == role.as_str())
        .and_then(|row| row.model.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_string();
    let resolved_model = if configured_model.is_empty() {
        bear.default_model.clone().unwrap_or_default()
    } else {
        configured_model.clone()
    };
    let configured_model_custom =
        if configured_model.is_empty() || model_available(&model_options, &configured_model) {
            String::new()
        } else {
            configured_model.clone()
        };

    let mut actions = Vec::new();
    match role {
        BearProfile::Chat => {
            actions.push(RoleActionLink {
                label: "Open chat",
                href: format!("/bear/{}", bear.slug),
            });
            actions.push(RoleActionLink {
                label: "All conversations",
                href: format!("/bear/{}/conversations", bear.slug),
            });
        }
        BearProfile::Pair => {
            actions.push(RoleActionLink {
                label: "Code with this Bear",
                href: format!("/bear/{}/code-token", bear.slug),
            });
        }
        _ => {}
    }
    actions.push(RoleActionLink {
        label: "Profile memory",
        href: format!("/bear/{}/memory?role={}", bear.slug, role.as_str()),
    });

    Ok(RoleDetailView {
        profile: role.as_str().to_string(),
        label: role_memory_label(role).to_string(),
        plain_name: role_plain_name(role),
        description: role_memory_description(role).to_string(),
        surfaces: role_surfaces(role),
        capabilities: role_capabilities(role),
        memory: role_memory_rules(role),
        actions,
        runtime_family: role.runtime_family().to_string(),
        provisioning_status: role_row.provisioning_status,
        last_provisioning_error: role_row.last_provisioning_error,
        memory_status_label,
        memory_file_count,
        configured_model: configured_model.clone(),
        resolved_model: resolved_model.clone(),
        model_source: if configured_model.is_empty() {
            "Bear default".to_string()
        } else {
            "Stance override".to_string()
        },
        model_availability_status: model_availability_status(&all_model_options, &resolved_model)
            .to_string(),
        model_metadata_status: model_metadata_status(&resolved_model).to_string(),
        configured_model_custom,
        model_options,
        all_model_options,
        role_contract,
        composed_prompt: composed.composed_prompt,
    })
}

fn merge_model_options(primary: &[ModelOption], secondary: &[ModelOption]) -> Vec<ModelOption> {
    let mut merged = primary.to_vec();
    for option in secondary {
        if !merged
            .iter()
            .any(|existing| existing.handle == option.handle)
        {
            merged.push(option.clone());
        }
    }
    merged.sort_by(|a, b| a.label.cmp(&b.label));
    merged
}

fn model_available(options: &[ModelOption], raw: &str) -> bool {
    let requested = raw.trim();
    if requested.is_empty() {
        return false;
    }
    let requested_resolved = den_llm::model_registry::resolve_model_handle(requested);
    options.iter().any(|model| {
        if model.handle == requested {
            return true;
        }
        let Some(resolved) = requested_resolved else {
            return false;
        };
        resolved == model.handle
            || den_llm::model_registry::resolve_model_handle(&model.handle) == Some(resolved)
    })
}

fn model_availability_status(options: &[ModelOption], raw: &str) -> &'static str {
    if raw.trim().is_empty() {
        "unset"
    } else if model_available(options, raw) {
        "available"
    } else {
        "unavailable"
    }
}

fn model_metadata_status(raw: &str) -> &'static str {
    if raw.trim().is_empty() {
        "unknown"
    } else if den_llm::model_registry::entry_for_handle(raw).is_some() {
        "known"
    } else {
        "unknown"
    }
}
