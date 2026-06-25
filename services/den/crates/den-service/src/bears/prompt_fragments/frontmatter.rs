use den_core::DenError;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PromptFragmentFrontmatter {
    pub id: String,
    pub layer: String,
    pub templating_phase: String,
    #[serde(default)]
    pub applies_to: Vec<String>,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub vars: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

pub(crate) fn split_frontmatter<'a>(
    source_name: &str,
    source: &'a str,
) -> Result<(&'a str, &'a str), DenError> {
    let Some(rest) = source.strip_prefix("---\n") else {
        return Err(DenError::Parsing(format!(
            "prompt fragment {source_name} must start with YAML frontmatter"
        )));
    };
    let Some(end) = rest.find("\n---") else {
        return Err(DenError::Parsing(format!(
            "prompt fragment {source_name} is missing closing YAML frontmatter marker"
        )));
    };
    let yaml = &rest[..end];
    let body = rest[end + "\n---".len()..]
        .strip_prefix('\n')
        .unwrap_or(&rest[end + "\n---".len()..]);
    Ok((yaml, body))
}

pub(crate) fn parse_frontmatter(
    source_name: &str,
    source: &str,
) -> Result<(PromptFragmentFrontmatter, String), DenError> {
    let (yaml, body) = split_frontmatter(source_name, source)?;
    let frontmatter: PromptFragmentFrontmatter = serde_yml::from_str(yaml).map_err(|err| {
        DenError::Parsing(format!(
            "prompt fragment {source_name} frontmatter failed to parse: {err}"
        ))
    })?;
    frontmatter.validate(source_name)?;
    Ok((frontmatter, body.trim().to_string()))
}

impl PromptFragmentFrontmatter {
    fn validate(&self, source_name: &str) -> Result<(), DenError> {
        if self.id.trim().is_empty() {
            return Err(DenError::ValidationError(format!(
                "prompt fragment {source_name} has an empty id"
            )));
        }
        if self.layer.trim().is_empty() {
            return Err(DenError::ValidationError(format!(
                "prompt fragment {source_name} has an empty layer"
            )));
        }
        match self.templating_phase.as_str() {
            "compile" | "turn" => Ok(()),
            other => Err(DenError::ValidationError(format!(
                "prompt fragment {source_name} has unsupported templating_phase {other:?}"
            ))),
        }
    }
}
