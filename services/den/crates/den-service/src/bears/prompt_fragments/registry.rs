use std::collections::BTreeMap;

use den_core::DenError;

use super::frontmatter::{parse_frontmatter, PromptFragmentFrontmatter};

#[derive(Debug, Clone)]
pub struct PromptFragment {
    pub source_name: String,
    pub frontmatter: PromptFragmentFrontmatter,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct PromptFragmentRegistry {
    fragments: BTreeMap<String, PromptFragment>,
}

impl PromptFragmentRegistry {
    pub fn from_embedded_sources(sources: &[(&str, &str)]) -> Result<Self, DenError> {
        let mut fragments = BTreeMap::new();
        for (source_name, source) in sources {
            let (frontmatter, body) = parse_frontmatter(source_name, source)?;
            let key = frontmatter.id.clone();
            if fragments
                .insert(
                    key.clone(),
                    PromptFragment {
                        source_name: (*source_name).to_string(),
                        frontmatter,
                        body,
                    },
                )
                .is_some()
            {
                return Err(DenError::ValidationError(format!(
                    "duplicate prompt fragment id {key:?}"
                )));
            }
        }
        Ok(Self { fragments })
    }

    pub fn get(&self, id: &str) -> Option<&PromptFragment> {
        self.fragments.get(id)
    }

    pub fn require(&self, id: &str) -> Result<&PromptFragment, DenError> {
        self.get(id)
            .ok_or_else(|| DenError::NotFound(format!("prompt fragment {id:?} not found")))
    }

    pub fn iter(&self) -> impl Iterator<Item = &PromptFragment> {
        self.fragments.values()
    }
}
