//! Profile detail view model for bear settings.

use serde::Serialize;
use crate::{errors::CustomError, web::AppState};
use den_service::bears::{
    compose_role_context, db as bears_db, Bear, BearProfile, BearProfileBinding,
};

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
    fn from_agent(agent: BearProfileBinding, _role: BearProfile) -> Self {
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

    let role_row = BearRoleViewRow::from_agent(agent.clone(), role);

    let (memory_status_label, memory_file_count) = {
        let manager = den_runtime::memory::MemoryStoreManager::new(state.config.as_ref());
        let store = manager.store_for_bear(bear.id).await?;
        match den_runtime::memory::tools::sqlite_memory_status(&store, role.as_str()).await {
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
        role_contract,
        composed_prompt: composed.composed_prompt,
    })
}
