use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{tools::descriptor::DenToolDescriptor, BearProfile};

#[derive(Debug, Deserialize)]
pub struct CapabilitySearchArguments {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct CapabilityDescribeArguments {
    pub r#ref: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityEntry {
    pub r#ref: String,
    pub kind: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub provider: String,
    pub execution_locality: String,
    pub authority: String,
    pub lifetime: String,
    pub surface: String,
    pub risk: String,
    pub good_for: Vec<String>,
    pub not_good_for: Vec<String>,
    pub execution_options: Vec<String>,
    pub tool: Option<CapabilityToolRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityToolRef {
    pub canonical_name: String,
    pub provider_name: String,
    pub schema_ref: String,
}

impl CapabilityEntry {
    fn searchable_text(&self) -> String {
        format!(
            "{} {} {} {} {} {} {} {} {} {} {}",
            self.r#ref,
            self.kind,
            self.summary,
            self.tags.join(" "),
            self.provider,
            self.execution_locality,
            self.authority,
            self.lifetime,
            self.surface,
            self.good_for.join(" "),
            self.not_good_for.join(" ")
        )
        .to_ascii_lowercase()
    }
}

pub fn tool_descriptor_to_capability(descriptor: DenToolDescriptor) -> CapabilityEntry {
    let risk = risk_for_tool(&descriptor);
    CapabilityEntry {
        r#ref: format!("tool:{}", descriptor.name),
        kind: "tool".to_string(),
        summary: descriptor.description.to_string(),
        tags: tags_for_tool(&descriptor),
        provider: descriptor.provider.to_string(),
        execution_locality: locality_for_tool(&descriptor).to_string(),
        authority: authority_for_tool(&descriptor).to_string(),
        lifetime: lifetime_for_tool(&descriptor).to_string(),
        surface: descriptor.scope.to_string(),
        risk: risk.clone(),
        good_for: good_for_tool(&descriptor),
        not_good_for: not_good_for_tool(&descriptor, &risk),
        execution_options: execution_options_for_tool(&descriptor, &risk),
        tool: Some(CapabilityToolRef {
            canonical_name: descriptor.name.to_string(),
            provider_name: descriptor.provider_name.clone(),
            schema_ref: format!("tool-schema:{}", descriptor.name),
        }),
    }
}

pub fn code_mode_capability(role: BearProfile) -> CapabilityEntry {
    CapabilityEntry {
        r#ref: "executor:code_mode.den".to_string(),
        kind: "executor".to_string(),
        summary: "Compose allowed Den capabilities with small scripts for loops, batching, filtering, aggregation, joins, retries, or large intermediate state.".to_string(),
        tags: vec![
            "executor.code".to_string(),
            "composition".to_string(),
            "batching".to_string(),
            format!("role.{}", role.as_str()),
        ],
        provider: "den".to_string(),
        execution_locality: "den-managed sandbox".to_string(),
        authority: "current Bear/session policy; only explicitly mediated capabilities".to_string(),
        lifetime: "not yet executable; catalog guidance only".to_string(),
        surface: "allowed Den capability calls".to_string(),
        risk: "depends_on_invoked_capabilities".to_string(),
        good_for: vec![
            "more than a few related calls".to_string(),
            "loops or batching".to_string(),
            "parsing/filtering/aggregation/joins/transforms".to_string(),
            "retries or conditional execution".to_string(),
            "large intermediate outputs with compact final results".to_string(),
        ],
        not_good_for: vec![
            "single simple reads or actions".to_string(),
            "risky actions needing individual review".to_string(),
            "direct access to armature-local files, local commands, or adapter/browser session state unless mediated".to_string(),
        ],
        execution_options: vec!["discover now; executable Code Mode lands later".to_string()],
        tool: None,
    }
}

pub fn search_capabilities(entries: &[CapabilityEntry], args: CapabilitySearchArguments) -> Value {
    let limit = args.limit.unwrap_or(12).clamp(1, 50);
    let query_terms: Vec<String> = args
        .query
        .unwrap_or_default()
        .split_whitespace()
        .map(|term| term.to_ascii_lowercase())
        .collect();
    let tag_filter = args.tag.map(|tag| tag.to_ascii_lowercase());
    let kind_filter = args.kind.map(|kind| kind.to_ascii_lowercase());

    let mut matches = Vec::new();
    for entry in entries {
        if let Some(kind) = &kind_filter {
            if entry.kind.to_ascii_lowercase() != *kind {
                continue;
            }
        }
        if let Some(tag) = &tag_filter {
            if !entry
                .tags
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(tag))
            {
                continue;
            }
        }
        let haystack = entry.searchable_text();
        if !query_terms.iter().all(|term| haystack.contains(term)) {
            continue;
        }
        matches.push(entry);
    }

    let results: Vec<Value> = matches
        .into_iter()
        .take(limit)
        .map(compact_result)
        .collect();
    json!({
        "results": results,
        "notes": [
            "Search is lexical/tag-based in this first slice; use capability_describe for full details.",
            "Discovery does not grant invocation authority.",
            "Check execution_locality/surface/authority before assuming similarly named capabilities operate on the same state."
        ]
    })
}

pub fn describe_capability(entries: &[CapabilityEntry], reference: &str) -> Option<Value> {
    let entry = entries.iter().find(|entry| {
        entry.r#ref == reference
            || entry.tool.as_ref().is_some_and(|tool| {
                tool.canonical_name == reference || tool.provider_name == reference
            })
    })?;
    Some(json!({
        "capability": entry,
        "notes": [
            "Prefer direct invocation for simple one-off actions.",
            "Prefer Code Mode for loops, batching, transforms, joins, retries, or large intermediate state when an executor is available.",
            "Local/session-bound providers must be mediated explicitly; this Den-managed entry does not imply armature-local access."
        ]
    }))
}

fn compact_result(entry: &CapabilityEntry) -> Value {
    json!({
        "ref": entry.r#ref,
        "kind": entry.kind,
        "summary": entry.summary,
        "tags": entry.tags,
        "provider": entry.provider,
        "execution_locality": entry.execution_locality,
        "authority": entry.authority,
        "lifetime": entry.lifetime,
        "surface": entry.surface,
        "risk": entry.risk,
        "execution_options": entry.execution_options,
    })
}

fn risk_for_tool(descriptor: &DenToolDescriptor) -> String {
    if descriptor.approval_policy != "never" {
        return "approval_required".to_string();
    }
    if descriptor
        .permissions
        .iter()
        .any(|permission| permission.ends_with(".write") || permission.contains("write"))
    {
        "mutating".to_string()
    } else if descriptor.domain == "web" {
        "external_network".to_string()
    } else {
        "read_only".to_string()
    }
}

fn tags_for_tool(descriptor: &DenToolDescriptor) -> Vec<String> {
    let mut tags = vec![
        descriptor.domain.to_string(),
        descriptor.scope.to_string(),
        format!("provider.{}", descriptor.provider),
        format!("locality.{}", descriptor.execution_target),
    ];
    tags.extend(descriptor.permissions.iter().map(|item| item.to_string()));
    tags.sort();
    tags.dedup();
    tags
}

fn locality_for_tool(descriptor: &DenToolDescriptor) -> &str {
    match descriptor.execution_target {
        "den" => "den-managed runtime",
        "adapter" => "session-local adapter",
        "client" => "client-provided session",
        _ => descriptor.execution_target,
    }
}

fn authority_for_tool(descriptor: &DenToolDescriptor) -> &str {
    match descriptor.provider {
        "den" => "current Bear/user/session policy",
        "armature" => "user local adapter session",
        "mcp" => "connected MCP server policy",
        _ => "provider policy",
    }
}

fn lifetime_for_tool(descriptor: &DenToolDescriptor) -> &str {
    match descriptor.provider {
        "den" => "durable while Den service and role policy expose it",
        "armature" | "adapter" | "mcp" => "current session/provider connection",
        _ => "provider-defined",
    }
}

fn good_for_tool(descriptor: &DenToolDescriptor) -> Vec<String> {
    vec![format!(
        "{} via {}",
        descriptor.label, descriptor.provider_name
    )]
}

fn not_good_for_tool(descriptor: &DenToolDescriptor, risk: &str) -> Vec<String> {
    let mut notes = vec!["assuming access to a different surface with a similar name".to_string()];
    if risk != "read_only" {
        notes.push("unreviewed bulk execution; consider direct review or Code Mode only with explicit approval".to_string());
    }
    if descriptor.provider == "den" {
        notes.push(
            "direct armature-local filesystem, process, browser, or MCP server state".to_string(),
        );
    }
    notes
}

fn execution_options_for_tool(_descriptor: &DenToolDescriptor, risk: &str) -> Vec<String> {
    let mut options = vec!["direct invocation".to_string()];
    if risk == "read_only" || risk == "external_network" {
        options.push("Code Mode when batching/composition is available".to_string());
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::descriptor::builtin_den_tool_descriptors_for_profile;

    #[test]
    fn search_finds_memory_tool_by_tag_and_query() {
        let mut entries: Vec<_> = builtin_den_tool_descriptors_for_profile(BearProfile::Pair)
            .into_iter()
            .map(tool_descriptor_to_capability)
            .collect();
        entries.push(code_mode_capability(BearProfile::Pair));
        let result = search_capabilities(
            &entries,
            CapabilitySearchArguments {
                query: Some("memory search".to_string()),
                tag: Some("memory.search".to_string()),
                kind: Some("tool".to_string()),
                limit: Some(10),
            },
        );
        let refs: Vec<_> = result["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["ref"].as_str())
            .collect();
        assert!(refs.contains(&"tool:den.memory.search"));
    }

    #[test]
    fn describe_accepts_provider_name() {
        let entries: Vec<_> = builtin_den_tool_descriptors_for_profile(BearProfile::Pair)
            .into_iter()
            .map(tool_descriptor_to_capability)
            .collect();
        let result = describe_capability(&entries, "memory_search").unwrap();
        assert_eq!(result["capability"]["ref"], "tool:den.memory.search");
        assert_eq!(
            result["capability"]["execution_locality"],
            "den-managed runtime"
        );
    }

    #[test]
    fn code_mode_entry_names_locality_limits() {
        let entry = code_mode_capability(BearProfile::Pair);
        assert_eq!(entry.r#ref, "executor:code_mode.den");
        assert!(entry
            .not_good_for
            .iter()
            .any(|note| note.contains("armature-local")));
    }
}
