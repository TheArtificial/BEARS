use den_core::DenError;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    {
        bears::{managed_blocks::get_compiled_bear_config, model::BearProfile, provision::profile_config_hash, Bear},
        memory::{
            has_work_surface_canonical_anchor, head_record_for_logical_path,
            list_profile_local_head_records, memory_sequence_high_water, record_visible,
            AccessContext, MemoryRecordRow, MemoryStoreManager,
        },
    },
};
use den_core::tools::support::truncate_chars;
use den_core::tools::work_surface::{
    work_surface_anchor_paths, work_surface_candidate_slug_from_hints,
    work_surface_projection_status, WorkSurfaceProjectionStatus, WorkSurfaceSessionHints,
};
use crate::llm::model_registry;

const TIER1_SHARED_PATHS: &[&str] = &[
    "core/bear-overview.md",
    "core/bear-glossary.md",
    "core/shared-conventions.md",
];

const TIER4_SITUATION_PATH: &str = "core/situation/briefing.md";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMemoryProjectionCacheKey {
    pub bear_id: Uuid,
    pub profile: BearProfile,
    pub conversation_id: String,
    pub primary_surface_slug: Option<String>,
    pub sequence_high_water: i64,
    pub compiled_config_token: String,
}

#[derive(Debug, Clone)]
pub struct KeyMemoryProjectionResult {
    pub rendered_text: String,
    pub diagnostic: Value,
    pub cache_key: KeyMemoryProjectionCacheKey,
}

#[derive(Clone)]
pub struct KeyMemoryProjectionInput<'a> {
    pub pool: &'a PgPool,
    pub stores: &'a MemoryStoreManager,
    pub bear: &'a Bear,
    pub profile: BearProfile,
    pub conversation_id: &'a str,
    pub session_hints: WorkSurfaceSessionHints,
    pub work_surface_status_override: Option<&'a str>,
    pub native_runtime: bool,
    /// Mandatory access gate (ADR-0042 §7): records carrying access-bearing relations are
    /// only projected when this context grants them. An empty context is fail-closed.
    pub access: AccessContext,
}

struct TierBudget {
    max_records: usize,
    per_record_cap: usize,
    tier_soft_cap: usize,
}

struct ProjectionBudget {
    global_cap: usize,
    tiers: [TierBudget; 4],
}



fn projection_budget_for_profile_and_model(
    role: BearProfile,
    context_window: Option<u32>,
) -> ProjectionBudget {
    let base_global_cap = match role {
        BearProfile::Pair | BearProfile::Chat | BearProfile::Work => 8_000,
        BearProfile::Curate => 6_000,
        BearProfile::Watch => 4_000,
    };
    let global_cap = match context_window {
        Some(ctx) if ctx >= 1_000_000 => base_global_cap * 2,
        Some(ctx) if ctx >= 200_000 => base_global_cap + (base_global_cap / 2),
        _ => base_global_cap,
    };
    ProjectionBudget {
        global_cap,
        tiers: [
            TierBudget {
                max_records: 4,
                per_record_cap: 1_500,
                tier_soft_cap: 3_000,
            },
            TierBudget {
                max_records: 6,
                per_record_cap: 1_200,
                tier_soft_cap: 3_500,
            },
            TierBudget {
                max_records: 4,
                per_record_cap: 800,
                tier_soft_cap: 2_000,
            },
            TierBudget {
                max_records: 1,
                per_record_cap: 1_000,
                tier_soft_cap: 1_000,
            },
        ],
    }
}

struct BudgetTracker {
    global_remaining: usize,
    tier_remaining_records: usize,
    tier_char_remaining: usize,
    per_record_cap: usize,
}

impl BudgetTracker {
    fn new(budget: &ProjectionBudget, tier_index: usize) -> Self {
        Self {
            global_remaining: budget.global_cap,
            tier_remaining_records: budget.tiers[tier_index].max_records,
            tier_char_remaining: budget.tiers[tier_index].tier_soft_cap,
            per_record_cap: budget.tiers[tier_index].per_record_cap,
        }
    }

    fn try_take_record(&mut self, content: &str) -> Option<String> {
        if self.tier_remaining_records == 0 || self.global_remaining == 0 || self.tier_char_remaining == 0 {
            return None;
        }
        let cap = self
            .per_record_cap
            .min(self.tier_char_remaining)
            .min(self.global_remaining);
        let (text, truncated) = truncate_chars(content, cap);
        if text.trim().is_empty() {
            return None;
        }
        let used = text.chars().count() + if truncated { 3 } else { 0 };
        let rendered = if truncated {
            format!("{text}...")
        } else {
            text
        };
        self.tier_remaining_records -= 1;
        self.tier_char_remaining = self.tier_char_remaining.saturating_sub(used);
        self.global_remaining = self.global_remaining.saturating_sub(used);
        Some(rendered)
    }
}

fn format_record_block(record: &MemoryRecordRow, body: &str) -> String {
    let path = record
        .logical_path
        .as_deref()
        .unwrap_or("<unmapped>");
    format!("### {path}\n{body}")
}

pub(crate) async fn compiled_prompt_cache_token(
    pool: &PgPool,
    bear: &Bear,
    role: BearProfile,
    _native_runtime: bool,
) -> Result<String, DenError> {
    if bear.context_profile.is_none() {
        return Ok(format!("legacy:{}:{}", bear.id, bear.provisioning_version));
    }
    if let Some(compiled) = get_compiled_bear_config(pool, bear.id).await? {
        return Ok(compiled.config_hash);
    }
    let hash = profile_config_hash(pool, bear, role).await?;
    Ok(hash
        .get("compiled_config_hash")
        .and_then(Value::as_str)
        .unwrap_or("missing")
        .to_string())
}

pub async fn project_key_memory(input: KeyMemoryProjectionInput<'_>) -> Result<KeyMemoryProjectionResult, DenError> {
    let store = input.stores.store_for_bear(input.bear.id).await?;
    let sequence_high_water = memory_sequence_high_water(&store).await?;
    let compiled_config_token =
        compiled_prompt_cache_token(input.pool, input.bear, input.profile, input.native_runtime)
            .await?;
    let status = work_surface_projection_status(
        &input.session_hints,
        input.work_surface_status_override,
    );
    let primary_slug = work_surface_candidate_slug_from_hints(&input.session_hints);
    let cache_key = KeyMemoryProjectionCacheKey {
        bear_id: input.bear.id,
        profile: input.profile,
        conversation_id: input.conversation_id.to_string(),
        primary_surface_slug: primary_slug.clone(),
        sequence_high_water,
        compiled_config_token,
    };

    let model_metadata = input
        .bear
        .default_model
        .as_deref()
        .and_then(model_registry::entry_for_handle);
    let budget = projection_budget_for_profile_and_model(
        input.profile,
        model_metadata.map(|entry| entry.context_window),
    );
    let mut included = Vec::<Value>::new();
    let mut omitted_budget = Vec::<String>::new();
    let mut omitted_no_surface = Vec::<String>::new();
    let mut omitted_by_access = Vec::<String>::new();
    let mut sections = Vec::<String>::new();

    // Tier 1 — shared identity anchors
    {
        let mut tracker = BudgetTracker::new(&budget, 0);
        let mut blocks = Vec::new();
        for path in TIER1_SHARED_PATHS {
            if tracker.tier_remaining_records == 0 {
                omitted_budget.push(path.to_string());
                continue;
            }
            let Some(record) = head_record_for_logical_path(&store, path).await? else {
                continue;
            };
            if !record_visible(&store, &record.memory_id, &input.access).await? {
                omitted_by_access.push(path.to_string());
                continue;
            }
            match tracker.try_take_record(&record.content_text) {
                Some(body) => {
                    included.push(json!({
                        "tier": 1,
                        "memory_id": record.memory_id,
                        "logical_path": path,
                    }));
                    blocks.push(format_record_block(&record, &body));
                }
                None => omitted_budget.push(path.to_string()),
            }
        }
        if !blocks.is_empty() {
            sections.push(format!("## Shared anchors\n\n{}", blocks.join("\n\n")));
        }
    }

    // Tier 2 — work-surface anchors
    let tier2_active = match status {
        WorkSurfaceProjectionStatus::Unresolved | WorkSurfaceProjectionStatus::Ambiguous => false,
        WorkSurfaceProjectionStatus::Candidate => {
            if let Some(ref slug) = primary_slug {
                has_work_surface_canonical_anchor(&store, slug).await?
            } else {
                false
            }
        }
        WorkSurfaceProjectionStatus::Resolved | WorkSurfaceProjectionStatus::Confirmed => {
            primary_slug.is_some()
        }
    };
    if let Some(slug) = primary_slug.as_deref() {
        if matches!(
            status,
            WorkSurfaceProjectionStatus::Unresolved | WorkSurfaceProjectionStatus::Ambiguous
        ) {
            omitted_no_surface.push(format!("tier2:status={}", status.as_str()));
        } else if matches!(status, WorkSurfaceProjectionStatus::Candidate) && !tier2_active {
            omitted_no_surface.push(format!("tier2:slug={slug}:anchor_required"));
        } else if tier2_active {
            let (canonical_paths, _) = work_surface_anchor_paths(input.profile, slug);
            let mut tracker = BudgetTracker::new(&budget, 1);
            let mut blocks = Vec::new();
            for path in canonical_paths {
                if tracker.tier_remaining_records == 0 {
                    omitted_budget.push(path.clone());
                    continue;
                }
                let Some(record) = head_record_for_logical_path(&store, &path).await? else {
                    continue;
                };
                if !record_visible(&store, &record.memory_id, &input.access).await? {
                    omitted_by_access.push(path.clone());
                    continue;
                }
                match tracker.try_take_record(&record.content_text) {
                    Some(body) => {
                        included.push(json!({
                            "tier": 2,
                            "memory_id": record.memory_id,
                            "logical_path": path,
                            "work_surface_slug": slug,
                        }));
                        blocks.push(format_record_block(&record, &body));
                    }
                    None => omitted_budget.push(path),
                }
            }
            if !blocks.is_empty() {
                sections.push(format!("## Work surface: {slug}\n\n{}", blocks.join("\n\n")));
            }
        }
    }

    // Tier 3 — role-local highlights
    {
        let mut tracker = BudgetTracker::new(&budget, 2);
        let mut blocks = Vec::new();
        let surface_ref = if tier2_active {
            primary_slug.as_deref()
        } else {
            None
        };
        let records = if let Some(surface) = surface_ref {
            let mut rows = list_profile_local_head_records(&store, input.profile.as_str(), Some(surface), 8)
                .await?;
            if rows.len() < budget.tiers[2].max_records {
                let remaining = (budget.tiers[2].max_records - rows.len()) as i64;
                let global = list_profile_local_head_records(&store, input.profile.as_str(), None, remaining)
                    .await?;
                rows.extend(global);
            }
            rows
        } else {
            list_profile_local_head_records(&store, input.profile.as_str(), None, budget.tiers[2].max_records as i64)
                .await?
        };
        for record in records {
            if tracker.tier_remaining_records == 0 {
                if let Some(path) = record.logical_path.clone() {
                    omitted_budget.push(path);
                }
                continue;
            }
            if !record_visible(&store, &record.memory_id, &input.access).await? {
                if let Some(path) = record.logical_path.clone() {
                    omitted_by_access.push(path);
                }
                continue;
            }
            match tracker.try_take_record(&record.content_text) {
                Some(body) => {
                    included.push(json!({
                        "tier": 3,
                        "memory_id": record.memory_id,
                        "logical_path": record.logical_path,
                        "work_surface_ref": record.work_surface_ref,
                    }));
                    blocks.push(format_record_block(&record, &body));
                }
                None => {
                    if let Some(path) = record.logical_path.clone() {
                        omitted_budget.push(path);
                    }
                }
            }
        }
        if !blocks.is_empty() {
            sections.push(format!(
                "## Role highlights ({})\n\n{}",
                input.profile.as_str(),
                blocks.join("\n\n")
            ));
        }
    }

    // Tier 4 — situation briefing
    {
        let mut tracker = BudgetTracker::new(&budget, 3);
        if let Some(record) = head_record_for_logical_path(&store, TIER4_SITUATION_PATH).await? {
            if !record_visible(&store, &record.memory_id, &input.access).await? {
                omitted_by_access.push(TIER4_SITUATION_PATH.to_string());
            } else if let Some(body) = tracker.try_take_record(&record.content_text) {
                included.push(json!({
                    "tier": 4,
                    "memory_id": record.memory_id,
                    "logical_path": TIER4_SITUATION_PATH,
                }));
                sections.push(format!("## Situation\n\n{}", format_record_block(&record, &body)));
            } else {
                omitted_budget.push(TIER4_SITUATION_PATH.to_string());
            }
        }
    }

    let rendered_text = if sections.is_empty() {
        String::new()
    } else {
        format!("# Projected memory\n\n{}", sections.join("\n\n"))
    };
    let diagnostic = json!({
        "source": "key_memory_projection",
        "work_surface_status": status.as_str(),
        "primary_work_surface_slug": primary_slug,
        "tier2_active": tier2_active,
        "sequence_high_water": sequence_high_water,
        "included": included,
        "omitted_by_budget": omitted_budget,
        "omitted_because_no_surface": omitted_no_surface,
        "omitted_by_access": omitted_by_access,
        "global_char_cap": budget.global_cap,
        "model_metadata": model_metadata.map(|entry| json!({
            "key": entry.key,
            "context_window": entry.context_window,
            "max_output_tokens": entry.max_output_tokens,
        })),
    });
    Ok(KeyMemoryProjectionResult {
        rendered_text,
        diagnostic,
        cache_key,
    })
}

pub fn render_key_memory_projection_block(result: &KeyMemoryProjectionResult) -> Option<String> {
    if result.rendered_text.trim().is_empty() {
        None
    } else {
        Some(result.rendered_text.clone())
    }
}
