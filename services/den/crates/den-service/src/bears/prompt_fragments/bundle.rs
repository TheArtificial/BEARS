use std::collections::BTreeMap;

use den_core::DenError;
use serde::Deserialize;

use super::registry::PromptFragmentRegistry;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PromptBundle {
    pub id: String,
    pub fragments: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PromptBundleRegistry {
    bundles: BTreeMap<String, PromptBundle>,
}

impl PromptBundleRegistry {
    pub fn from_embedded_sources(
        sources: &[(&str, &str)],
        fragments: &PromptFragmentRegistry,
    ) -> Result<Self, DenError> {
        let mut bundles = BTreeMap::new();
        for (source_name, source) in sources {
            let bundle: PromptBundle = serde_yml::from_str(source).map_err(|err| {
                DenError::Parsing(format!("prompt bundle {source_name} failed to parse: {err}"))
            })?;
            bundle.validate(source_name, fragments)?;
            let key = bundle.id.clone();
            if bundles.insert(key.clone(), bundle).is_some() {
                return Err(DenError::ValidationError(format!(
                    "duplicate prompt bundle id {key:?}"
                )));
            }
        }
        Ok(Self { bundles })
    }

    pub fn get(&self, id: &str) -> Option<&PromptBundle> {
        self.bundles.get(id)
    }

    pub fn require(&self, id: &str) -> Result<&PromptBundle, DenError> {
        self.get(id)
            .ok_or_else(|| DenError::NotFound(format!("prompt bundle {id:?} not found")))
    }
}

impl PromptBundle {
    fn validate(&self, source_name: &str, fragments: &PromptFragmentRegistry) -> Result<(), DenError> {
        if self.id.trim().is_empty() {
            return Err(DenError::ValidationError(format!(
                "prompt bundle {source_name} has an empty id"
            )));
        }
        if self.fragments.is_empty() {
            return Err(DenError::ValidationError(format!(
                "prompt bundle {source_name} must reference at least one fragment"
            )));
        }
        for fragment_id in &self.fragments {
            fragments.require(fragment_id).map_err(|_| {
                DenError::ValidationError(format!(
                    "prompt bundle {source_name} references missing fragment {fragment_id:?}"
                ))
            })?;
        }
        Ok(())
    }
}
