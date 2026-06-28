use serde::{Deserialize, Serialize};

use crate::descriptors;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScopeType {
    ProfileLocal,
    Shared,
}

impl MemoryScopeType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProfileLocal => "profile_local",
            Self::Shared => "shared",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "profile_local" | "role_local" => Some(Self::ProfileLocal),
            "shared" => Some(Self::Shared),
            _ => None,
        }
    }
}

/// Stable anchor path projection over SQLite rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalMemoryPath {
    pub scope_type: MemoryScopeType,
    pub scope_profile: Option<String>,
    pub work_surface_ref: Option<String>,
    pub kind: String,
}

impl LogicalMemoryPath {
    pub fn profile_local(profile: &str, kind: &str) -> Self {
        Self {
            scope_type: MemoryScopeType::ProfileLocal,
            scope_profile: Some(profile.to_string()),
            work_surface_ref: None,
            kind: kind.to_string(),
        }
    }

    pub fn shared_core(kind: &str) -> Self {
        Self {
            scope_type: MemoryScopeType::Shared,
            scope_profile: None,
            work_surface_ref: None,
            kind: kind.to_string(),
        }
    }

    /// Encode to the legacy logical path string used by memory tools.
    pub fn to_logical_path(&self) -> String {
        match (&self.scope_type, &self.scope_profile, &self.work_surface_ref) {
            (MemoryScopeType::Shared, None, Some(ws)) => {
                format!("core/work_surfaces/{ws}/{}.md", self.kind)
            }
            (MemoryScopeType::Shared, None, None) if self.kind == "overview" => {
                "core/bear-overview.md".to_string()
            }
            (MemoryScopeType::Shared, None, None) => format!("core/{}.md", self.kind),
            (MemoryScopeType::ProfileLocal, Some(profile), Some(ws)) => {
                format!("{profile}/work_surfaces/{ws}/{}.md", self.kind)
            }
            (MemoryScopeType::ProfileLocal, Some(profile), None) => {
                format!("{profile}/{}.md", self.kind)
            }
            _ => format!("memory/{}.md", self.kind),
        }
    }

    pub fn from_logical_path(path: &str) -> Self {
        let trimmed = path.trim().trim_start_matches('/');
        if trimmed.starts_with("core/work_surfaces/") {
            let rest = trimmed.trim_start_matches("core/work_surfaces/");
            let mut parts = rest.split('/');
            let ws = parts.next().unwrap_or("unknown").to_string();
            let file = parts.next().unwrap_or("index.md");
            let kind = file.trim_end_matches(".md").to_string();
            return Self {
                scope_type: MemoryScopeType::Shared,
                scope_profile: None,
                work_surface_ref: Some(ws),
                kind,
            };
        }
        if trimmed.starts_with("core/") {
            let kind = trimmed
                .trim_start_matches("core/")
                .trim_end_matches(".md")
                .to_string();
            return Self::shared_core(&kind);
        }
        if let Some((profile, rest)) = trimmed.split_once('/') {
            if rest.starts_with("work_surfaces/") {
                let sub = rest.trim_start_matches("work_surfaces/");
                let mut parts = sub.split('/');
                let ws = parts.next().unwrap_or("unknown").to_string();
                let file = parts.next().unwrap_or("index.md");
                let kind = file.trim_end_matches(".md").to_string();
                return Self {
                    scope_type: MemoryScopeType::ProfileLocal,
                    scope_profile: Some(profile.to_string()),
                    work_surface_ref: Some(ws),
                    kind,
                };
            }
            let kind = rest.trim_end_matches(".md").to_string();
            return Self::profile_local(profile, &kind);
        }
        Self::shared_core("note")
    }
}

fn sanitize_anchor_segment(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch.is_whitespace() || ch == '/' || ch == ':' {
            out.push('-');
        }
    }
    let collapsed = out
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        "unknown".to_string()
    } else {
        collapsed
    }
}

fn entity_anchor_collection(entity_type: &str) -> Option<&'static str> {
    let descriptor = descriptors::entity_type(entity_type)?;
    if !descriptor.anchor_eligible {
        return None;
    }
    match entity_type {
        "person" => Some("people"),
        "org" => Some("orgs"),
        "mission" => Some("missions"),
        "domain" => Some("domains"),
        "work_surface" => Some("work_surfaces"),
        _ => None,
    }
}

/// Stable shared-memory anchor path for resolved, salient entities (ADR-0042 Phase 5).
///
/// This helper only encodes the descriptor-owned path shape. Callers remain responsible for the
/// policy decision that an entity is resolved and salient enough to deserve a canonical anchor.
pub fn entity_anchor_path(entity_type: &str, anchor_ref: &str, kind: &str) -> Option<String> {
    let collection = entity_anchor_collection(entity_type)?;
    let anchor = sanitize_anchor_segment(anchor_ref);
    let kind = sanitize_anchor_segment(kind);
    Some(format!("core/{collection}/{anchor}/{kind}.md"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_work_surface_core_path() {
        let path = "core/work_surfaces/my-repo/architecture.md";
        let logical = LogicalMemoryPath::from_logical_path(path);
        assert_eq!(logical.work_surface_ref.as_deref(), Some("my-repo"));
        assert_eq!(logical.kind, "architecture");
        assert_eq!(logical.to_logical_path(), path);
    }

    #[test]
    fn builds_descriptor_owned_entity_anchor_paths() {
        assert_eq!(
            entity_anchor_path("person", "Ryan Dahl", "profile").as_deref(),
            Some("core/people/ryan-dahl/profile.md")
        );
        assert_eq!(
            entity_anchor_path("mission", "Cabinet:Launch", "overview").as_deref(),
            Some("core/missions/cabinet-launch/overview.md")
        );
        assert!(entity_anchor_path("event", "standup", "overview").is_none());
    }
}
