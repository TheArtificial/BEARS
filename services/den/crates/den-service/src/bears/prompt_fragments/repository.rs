use den_core::DenError;
use sha2::{Digest, Sha256};

use super::registry::PromptFragmentRegistry;

const REPOSITORY_PROMPT_SOURCES: &[(&str, &str)] = &[(
    "fragments/base/den_baseline.md",
    include_str!("../../../../../prompts/fragments/base/den_baseline.md"),
)];

pub fn repository_prompt_fragment_registry() -> Result<PromptFragmentRegistry, DenError> {
    PromptFragmentRegistry::from_embedded_sources(REPOSITORY_PROMPT_SOURCES)
}

pub fn repository_prompt_source_version() -> String {
    let mut hasher = Sha256::new();
    for (name, source) in REPOSITORY_PROMPT_SOURCES {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(source.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}
