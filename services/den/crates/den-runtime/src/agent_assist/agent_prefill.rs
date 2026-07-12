//! Map `GET /v1/agents/{id}` JSON into Den admin "new bear" form defaults.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use super::json_fields::{model_field, pick_str};

static REPEATED_DASHES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-+").expect("dash regex"));

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentBearPrefill {
    pub suggested_slug: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub default_model: String,
}

fn suggest_slug(name: &str, agent_id: &str) -> String {
    let slugish: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else if c.is_whitespace() || matches!(c, '-' | '_') {
                '-'
            } else {
                '\0'
            }
        })
        .filter(|&c| c != '\0')
        .collect();
    let s = REPEATED_DASHES.replace_all(&slugish, "-");
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        let tail: String = agent_id
            .trim_start_matches("agent-")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(16)
            .collect();
        let tail = if tail.is_empty() {
            "import".to_string()
        } else {
            tail
        };
        format!("bear-{tail}")
    } else {
        s.chars().take(120).collect()
    }
}

impl AgentBearPrefill {
    pub fn from_agent_json(v: &Value) -> Self {
        let id = v
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();

        let name = pick_str(v, &["name"]).unwrap_or_else(|| id.clone());
        let description = pick_str(v, &["description"]).unwrap_or_default();
        let system_prompt = v
            .get("system")
            .or_else(|| v.get("system_prompt"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();

        let default_model = model_field(v).unwrap_or_default();
        let suggested_slug = suggest_slug(&name, &id);

        Self {
            suggested_slug,
            name,
            description,
            system_prompt,
            default_model,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prefill_reads_core_fields() {
        let v = json!({
            "id": "agent-abc",
            "name": "My Bot",
            "description": "desc",
            "system": "You are helpful",
            "model": "openai/gpt-4o",
        });
        let p = AgentBearPrefill::from_agent_json(&v);
        assert_eq!(p.suggested_slug, "my-bot");
        assert_eq!(p.name, "My Bot");
        assert_eq!(p.description, "desc");
        assert_eq!(p.system_prompt, "You are helpful");
        assert_eq!(p.default_model, "openai/gpt-4o");
    }

    #[test]
    fn slug_falls_back_when_name_has_no_letters() {
        let v = json!({
            "id": "agent-xyz12",
            "name": "!!!",
            "system": "x"
        });
        let p = AgentBearPrefill::from_agent_json(&v);
        assert_eq!(p.suggested_slug, "bear-xyz12");
    }
}
