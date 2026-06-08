use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScopeType {
    RoleLocal,
    Shared,
}

impl MemoryScopeType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RoleLocal => "role_local",
            Self::Shared => "shared",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "role_local" => Some(Self::RoleLocal),
            "shared" => Some(Self::Shared),
            _ => None,
        }
    }
}

/// Stable anchor path projection over SQLite rows (replaces MemFS file tree UX).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalMemoryPath {
    pub scope_type: MemoryScopeType,
    pub scope_role: Option<String>,
    pub work_surface_ref: Option<String>,
    pub kind: String,
    pub entity_ref: Option<String>,
}

impl LogicalMemoryPath {
    pub fn role_local(role: &str, kind: &str) -> Self {
        Self {
            scope_type: MemoryScopeType::RoleLocal,
            scope_role: Some(role.to_string()),
            work_surface_ref: None,
            kind: kind.to_string(),
            entity_ref: None,
        }
    }

    pub fn shared_core(kind: &str) -> Self {
        Self {
            scope_type: MemoryScopeType::Shared,
            scope_role: None,
            work_surface_ref: None,
            kind: kind.to_string(),
            entity_ref: None,
        }
    }

    /// Encode to the legacy logical path string used by memory tools.
    pub fn to_logical_path(&self) -> String {
        match (&self.scope_type, &self.scope_role, &self.work_surface_ref) {
            (MemoryScopeType::Shared, None, Some(ws)) => {
                format!("core/work_surfaces/{ws}/{}.md", self.kind)
            }
            (MemoryScopeType::Shared, None, None) if self.kind == "overview" => {
                "core/bear-overview.md".to_string()
            }
            (MemoryScopeType::Shared, None, None) => format!("core/{}.md", self.kind),
            (MemoryScopeType::RoleLocal, Some(role), Some(ws)) => {
                format!("{role}/work_surfaces/{ws}/{}.md", self.kind)
            }
            (MemoryScopeType::RoleLocal, Some(role), None) => {
                format!("{role}/{}.md", self.kind)
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
                scope_role: None,
                work_surface_ref: Some(ws),
                kind,
                entity_ref: None,
            };
        }
        if trimmed.starts_with("core/") {
            let kind = trimmed
                .trim_start_matches("core/")
                .trim_end_matches(".md")
                .to_string();
            return Self::shared_core(&kind);
        }
        if let Some((role, rest)) = trimmed.split_once('/') {
            if rest.starts_with("work_surfaces/") {
                let sub = rest.trim_start_matches("work_surfaces/");
                let mut parts = sub.split('/');
                let ws = parts.next().unwrap_or("unknown").to_string();
                let file = parts.next().unwrap_or("index.md");
                let kind = file.trim_end_matches(".md").to_string();
                return Self {
                    scope_type: MemoryScopeType::RoleLocal,
                    scope_role: Some(role.to_string()),
                    work_surface_ref: Some(ws),
                    kind,
                    entity_ref: None,
                };
            }
            let kind = rest.trim_end_matches(".md").to_string();
            return Self::role_local(role, &kind);
        }
        Self::shared_core("note")
    }
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
}
