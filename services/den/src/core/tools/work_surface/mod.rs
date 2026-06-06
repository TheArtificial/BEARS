use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    core::{
        bears::BearAgentRole,
        memory_manager_head::{append_markdown_section, fetch_memfs_role_memory_file, write_memfs_core_update, MemfsCoreUpdateRequest},
        tools::{memfs::memfs_http_client, session::DenToolInvocationContext, support::validate_bounded_text},
    },
    config::Config,
    errors::CustomError,
};

use super::support::clean_optional;

pub(crate) fn infer_work_surface_hint(
    context: &DenToolInvocationContext,
    role: BearAgentRole,
) -> Value {
    let mut candidates = Vec::new();
    if let Some(runtime_target) = context.runtime_target.as_deref().and_then(clean_optional) {
        candidates.push(json!({
            "kind": "runtime_target",
            "value": runtime_target,
            "confidence": "medium"
        }));
    }
    if let Some(selection) = context
        .conversation_selection
        .as_deref()
        .and_then(clean_optional)
    {
        candidates.push(json!({
            "kind": "conversation_selection",
            "value": selection,
            "confidence": "low"
        }));
    }
    for root in context
        .workspace_roots
        .iter()
        .filter(|root| !root.trim().is_empty())
    {
        candidates.push(json!({
            "kind": "workspace_root",
            "value": root,
            "confidence": "medium"
        }));
    }
    let active_work_surface_roles = matches!(role, BearAgentRole::Pair | BearAgentRole::Work);
    json!({
        "workplace": {
            "role": role.as_str(),
            "memory_surface": format!("{}/", role.as_str()),
        },
        "work_surface": {
            "mode": if active_work_surface_roles { "active" } else { "reference_only" },
            "status": if candidates.is_empty() { "unresolved" } else { "candidate" },
            "note": if candidates.is_empty() {
                if active_work_surface_roles {
                    "No trusted work-surface hint is available yet from this session. Use workspace roots, runtime target, user references, and memory anchors to resolve what the agent may be acting on."
                } else {
                    "No trusted work-surface references are available yet from this session. Use Bear memory anchors and explicit user references to identify relevant work surfaces."
                }
            } else if active_work_surface_roles {
                "Trusted session metadata provides work-surface reference candidates for what the agent may be acting on. Treat these as hints, not canonical identity, until confirmed by anchors or explicit user intent."
            } else {
                "Trusted session metadata provides work-surface reference candidates that may help the agent answer about relevant Bear work surfaces. Treat these as hints, not canonical identity, until confirmed by anchors or explicit user intent."
            },
            "reference_candidates": candidates,
        }
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct MemoryCreateWorkSurfaceScaffoldArguments {
    pub(crate) work_surface_slug: String,
    pub(crate) work_surface_name: String,
    pub(crate) overview: String,
    #[serde(default)]
    pub(crate) glossary: Option<String>,
    #[serde(default)]
    pub(crate) current_understanding: Option<String>,
}

pub(crate) fn normalize_work_surface_slug(value: &str) -> Result<String, CustomError> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Err(CustomError::ValidationError(
            "work_surface_slug must not be empty".to_string(),
        ));
    }
    let normalized: String = trimmed
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    let collapsed = normalized
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        return Err(CustomError::ValidationError(
            "work_surface_slug must include at least one letter or digit".to_string(),
        ));
    }
    if collapsed.len() > 80 {
        return Err(CustomError::ValidationError(
            "work_surface_slug must be 80 characters or fewer after normalization".to_string(),
        ));
    }
    Ok(collapsed)
}

pub(crate) fn work_surface_candidate_slug(context: &DenToolInvocationContext) -> Option<String> {
    let mut raw_candidates = Vec::new();
    if let Some(value) = context.runtime_target.as_deref().and_then(clean_optional) {
        raw_candidates.push(value);
    }
    if let Some(value) = context
        .conversation_selection
        .as_deref()
        .and_then(clean_optional)
    {
        raw_candidates.push(value);
    }
    raw_candidates.extend(
        context
            .workspace_roots
            .iter()
            .filter_map(|value| clean_optional(value)),
    );

    for raw in raw_candidates {
        let lowered = raw.to_ascii_lowercase();
        for segment in raw.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_') {
            let trimmed =
                segment.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
            if trimmed.is_empty() {
                continue;
            }
            let candidate = normalize_work_surface_slug(trimmed).ok();
            if let Some(candidate) = candidate {
                if candidate.len() >= 3
                    && !matches!(
                        candidate.as_str(),
                        "workspace" | "pair" | "core" | "main" | "src" | "repo"
                    )
                {
                    return Some(candidate);
                }
            }
        }
        if let Some(rest) = lowered.strip_prefix("repo:") {
            if let Ok(candidate) = normalize_work_surface_slug(rest) {
                return Some(candidate);
            }
        }
    }
    None
}

fn work_surface_scaffold_paths(
    role: BearAgentRole,
    slug: &str,
) -> (String, String, String, Option<String>, String) {
    (
        format!("core/work_surfaces/{slug}/index.md"),
        format!("core/work_surfaces/{slug}/overview.md"),
        format!("core/work_surfaces/{slug}/glossary.md"),
        match role {
            BearAgentRole::Pair | BearAgentRole::Work => Some(format!(
                "{}/work_surfaces/{slug}/current-understanding.md",
                role.as_str()
            )),
            _ => None,
        },
        "core/work_surfaces/index.md".to_string(),
    )
}

pub(crate) fn work_surface_index_file_body() -> &'static str {
    "# Work Surfaces\n\nThis index lists the Bear's registered Work Surfaces.\n"
}

pub(crate) fn work_surface_entry_body(slug: &str, name: &str) -> String {
    format!("- [{name}](./{slug}/index.md)")
}

pub(crate) fn work_surface_scaffold_requests(
    role: BearAgentRole,
    slug: &str,
    name: &str,
    overview: &str,
    glossary: Option<&str>,
    current_understanding: Option<&str>,
) -> Vec<MemfsCoreUpdateRequest> {
    let (index_path, overview_path, glossary_path, current_understanding_path, registry_path) =
        work_surface_scaffold_paths(role, slug);
    let glossary_body =
        glossary.unwrap_or("Glossary terms for this work surface will be added here.");
    let understanding_body = current_understanding.unwrap_or(match role {
        BearAgentRole::Work => {
            "Current work understanding for this work surface will be maintained here."
        }
        _ => "Current pair understanding for this work surface will be maintained here.",
    });
    let mut requests = vec![
        MemfsCoreUpdateRequest {
            target_path: registry_path,
            mode: "create_file".to_string(),
            title: Some("Work Surfaces".to_string()),
            body: Some(work_surface_index_file_body().to_string()),
            old_text: None,
            new_text: None,
            proposal_id: None,
            source_paths: vec![],
        },
        MemfsCoreUpdateRequest {
            target_path: "core/work_surfaces/index.md".to_string(),
            mode: "append_section".to_string(),
            title: Some(name.to_string()),
            body: Some(work_surface_entry_body(slug, name)),
            old_text: None,
            new_text: None,
            proposal_id: None,
            source_paths: vec![],
        },
        MemfsCoreUpdateRequest {
            target_path: index_path,
            mode: "create_file".to_string(),
            title: Some(name.to_string()),
            body: Some(format!(
                "# {name}\n\n- Slug: `{slug}`\n- Overview: [overview](./overview.md)\n- Glossary: [glossary](./glossary.md)\n"
            )),
            old_text: None,
            new_text: None,
            proposal_id: None,
            source_paths: vec![],
        },
        MemfsCoreUpdateRequest {
            target_path: overview_path,
            mode: "create_file".to_string(),
            title: Some(format!("{name} overview")),
            body: Some(overview.trim().to_string()),
            old_text: None,
            new_text: None,
            proposal_id: None,
            source_paths: vec![],
        },
        MemfsCoreUpdateRequest {
            target_path: glossary_path,
            mode: "create_file".to_string(),
            title: Some(format!("{name} glossary")),
            body: Some(glossary_body.trim().to_string()),
            old_text: None,
            new_text: None,
            proposal_id: None,
            source_paths: vec![],
        },
    ];
    if let Some(current_understanding_path) = current_understanding_path {
        requests.push(MemfsCoreUpdateRequest {
            target_path: current_understanding_path,
            mode: "create_file".to_string(),
            title: Some(format!("{name} current understanding")),
            body: Some(understanding_body.trim().to_string()),
            old_text: None,
            new_text: None,
            proposal_id: None,
            source_paths: vec![],
        });
    }
    requests
}

pub(crate) fn work_surface_anchor_paths(
    role: BearAgentRole,
    slug: &str,
) -> (Vec<String>, Vec<String>) {
    let canonical = vec![
        format!("core/work_surfaces/{slug}/index.md"),
        format!("core/work_surfaces/{slug}/overview.md"),
        format!("core/work_surfaces/{slug}/glossary.md"),
        format!("core/work_surfaces/{slug}/architecture.md"),
        format!("core/work_surfaces/{slug}/decisions.md"),
        format!("core/work_surfaces/{slug}/conventions.md"),
    ];
    let role_local = match role {
        BearAgentRole::Pair | BearAgentRole::Work => vec![
            format!(
                "{}/work_surfaces/{slug}/current-understanding.md",
                role.as_str()
            ),
            format!("{}/work_surfaces/{slug}/recent-findings.md", role.as_str()),
            format!("{}/work_surfaces/{slug}/open-questions.md", role.as_str()),
        ],
        _ => Vec::new(),
    };
    (canonical, role_local)
}

pub(crate) fn collect_memory_tree_paths(files: &Value, out: &mut Vec<String>) {
    match files {
        Value::String(path) => out.push(path.clone()),
        Value::Array(items) => {
            for item in items {
                collect_memory_tree_paths(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(path) = map.get("path").and_then(|v| v.as_str()) {
                out.push(path.to_string());
            }
            for value in map.values() {
                collect_memory_tree_paths(value, out);
            }
        }
        _ => {}
    }
}

pub(crate) async fn create_work_surface_scaffold(
    config: &Config,
    context: &DenToolInvocationContext,
    role: BearAgentRole,
    arguments: Value,
) -> Result<Value, CustomError> {
    if role != BearAgentRole::Pair {
        return Err(CustomError::Authorization(
            "den.memory.create_work_surface_scaffold is currently available only to the pair role"
                .to_string(),
        ));
    }
    let args: MemoryCreateWorkSurfaceScaffoldArguments = serde_json::from_value(arguments)?;
    let work_surface_slug = normalize_work_surface_slug(&args.work_surface_slug)?;
    let work_surface_name =
        validate_bounded_text("work_surface_name", &args.work_surface_name, 1, 200)?;
    let overview = validate_bounded_text("overview", &args.overview, 1, 20_000)?;
    let glossary = args
        .glossary
        .as_deref()
        .map(|value| validate_bounded_text("glossary", value, 1, 20_000))
        .transpose()?;
    let current_understanding = args
        .current_understanding
        .as_deref()
        .map(|value| validate_bounded_text("current_understanding", value, 1, 20_000))
        .transpose()?;
    let http = memfs_http_client("MemFS work-surface scaffold client build failed")?;
    let mut responses = Vec::new();
    for request in work_surface_scaffold_requests(
        role,
        &work_surface_slug,
        &work_surface_name,
        &overview,
        glossary.as_deref(),
        current_understanding.as_deref(),
    ) {
        if request.target_path == "core/work_surfaces/index.md" && request.mode == "append_section"
        {
            let registry = fetch_memfs_role_memory_file(
                &http,
                &config.letta_memfs_service_url,
                context.bear_id,
                role.as_str(),
                "core/work_surfaces/index.md",
            )
            .await?;
            let existing = registry.map(|file| file.content).unwrap_or_default();
            let updated = append_markdown_section(
                &existing,
                &format!("## {work_surface_name}"),
                &work_surface_entry_body(&work_surface_slug, &work_surface_name),
            );
            let existing_is_empty = existing.trim().is_empty();
            let replace_request = MemfsCoreUpdateRequest {
                target_path: "core/work_surfaces/index.md".to_string(),
                mode: if existing_is_empty {
                    "create_file".to_string()
                } else {
                    "replace_text".to_string()
                },
                title: Some("Work Surfaces".to_string()),
                body: if existing_is_empty {
                    Some(updated.clone())
                } else {
                    None
                },
                old_text: if existing_is_empty {
                    None
                } else {
                    Some(existing)
                },
                new_text: if existing_is_empty {
                    None
                } else {
                    Some(updated)
                },
                proposal_id: None,
                source_paths: vec![],
            };
            let response = write_memfs_core_update(
                &http,
                &config.letta_memfs_service_url,
                context.bear_id,
                &replace_request,
            )
            .await?;
            if let Some(response) = response {
                responses.push(response);
            }
            continue;
        }
        let response = write_memfs_core_update(
            &http,
            &config.letta_memfs_service_url,
            context.bear_id,
            &request,
        )
        .await?;
        if let Some(response) = response {
            responses.push(response);
        }
    }
    let (index_path, overview_path, glossary_path, current_understanding_path, registry_path) =
        work_surface_scaffold_paths(role, &work_surface_slug);
    Ok(json!({
        "ok": true,
        "bear_id": context.bear_id,
        "work_surface": {
            "slug": work_surface_slug,
            "name": work_surface_name,
            "paths": {
                "registry": registry_path,
                "index": index_path,
                "overview": overview_path,
                "glossary": glossary_path,
                "current_understanding": current_understanding_path,
            }
        },
        "updates": responses,
    }))
}

pub(crate) fn build_work_surface_orientation_payload(
    role: BearAgentRole,
    hint_payload: &Value,
    files: &[String],
    candidate_slug: Option<String>,
) -> Value {
    let mut sorted_files = files.to_vec();
    sorted_files.sort();
    sorted_files.dedup();
    let slug = candidate_slug;
    let (canonical_paths, role_local_paths) = slug
        .as_deref()
        .map(|slug| work_surface_anchor_paths(role, slug))
        .unwrap_or_else(|| (Vec::new(), Vec::new()));
    let existing_canonical = canonical_paths
        .iter()
        .filter(|path| sorted_files.contains(path))
        .cloned()
        .collect::<Vec<_>>();
    let existing_role_local = role_local_paths
        .iter()
        .filter(|path| sorted_files.contains(path))
        .cloned()
        .collect::<Vec<_>>();
    let missing_expected_paths = canonical_paths
        .iter()
        .chain(role_local_paths.iter())
        .filter(|path| !sorted_files.contains(path))
        .cloned()
        .collect::<Vec<_>>();
    let active_work_surface_roles = matches!(role, BearAgentRole::Pair | BearAgentRole::Work);
    let status = if slug.is_none() {
        "unresolved"
    } else if existing_canonical.is_empty() && existing_role_local.is_empty() {
        "candidate_without_anchors"
    } else {
        "oriented"
    };
    let mut recommended_read_order = Vec::new();
    recommended_read_order.extend(existing_canonical.iter().cloned());
    recommended_read_order.extend(existing_role_local.iter().cloned());
    json!({
        "workplace": hint_payload["workplace"].clone(),
        "work_surface": {
            "mode": if active_work_surface_roles { "active" } else { "reference_only" },
            "status": status,
            "slug": slug,
            "confidence": if status == "oriented" { "medium" } else if status == "candidate_without_anchors" { "low" } else { "unknown" },
            "basis": hint_payload["work_surface"]["reference_candidates"].clone(),
            "note": match status {
                "oriented" if active_work_surface_roles => "Trusted hints and existing memory anchors provide a usable work-surface orientation for what the agent may be acting on.",
                "oriented" => "Trusted hints and existing canonical memory anchors provide a usable orientation for answering about this Bear work surface.",
                "candidate_without_anchors" if active_work_surface_roles => "Trusted hints suggest a work surface the agent may be acting on, but no canonical or role-local anchors were found yet.",
                "candidate_without_anchors" => "Trusted hints suggest a relevant Bear work surface, but no canonical anchors were found yet.",
                _ if active_work_surface_roles => "A current work surface the agent may be acting on could not be resolved from trusted hints alone.",
                _ => "A relevant Bear work surface could not be resolved from trusted hints alone.",
            }
        },
        "canonical_paths": existing_canonical,
        "role_local_paths": existing_role_local,
        "recommended_read_order": recommended_read_order,
        "missing_expected_paths": missing_expected_paths,
        "notes": if active_work_surface_roles {
            json!([
                "Use canonical work-surface anchors before broader Bear memory search when available.",
                "Use role-local work-surface memory as supporting working memory for what the agent is acting on.",
                "Treat the resolved slug as a working orientation hint unless explicit user intent or stronger anchors disagree."
            ])
        } else {
            json!([
                "Use canonical work-surface anchors before broader Bear memory search when available.",
                "This role is orienting about Bear work surfaces rather than claiming role-local ownership of one.",
                "Treat the resolved slug as a working orientation hint unless explicit user intent or stronger anchors disagree."
            ])
        }
    })
}

#[cfg(test)]
mod test;
