use den_core::DenError;
use sha2::{Digest, Sha256};

use super::{PromptBundleRegistry, PromptFragmentRegistry};

const REPOSITORY_PROMPT_SOURCES: &[(&str, &str)] = &[
    (
        "fragments/base/den_baseline.md",
        include_str!("../../../../../prompts/fragments/base/den_baseline.md"),
    ),
    (
        "fragments/stances/pair.md",
        include_str!("../../../../../prompts/fragments/stances/pair.md"),
    ),
    (
        "fragments/runtime/docket_execution_active.md",
        include_str!("../../../../../prompts/fragments/runtime/docket_execution_active.md"),
    ),
];

const REPOSITORY_BUNDLE_SOURCES: &[(&str, &str)] = &[(
    "bundles/pair.yaml",
    include_str!("../../../../../prompts/bundles/pair.yaml"),
)];

pub fn repository_prompt_fragment_registry() -> Result<PromptFragmentRegistry, DenError> {
    PromptFragmentRegistry::from_embedded_sources(REPOSITORY_PROMPT_SOURCES)
}

pub fn repository_prompt_bundle_registry(
    fragments: &PromptFragmentRegistry,
) -> Result<PromptBundleRegistry, DenError> {
    PromptBundleRegistry::from_embedded_sources(REPOSITORY_BUNDLE_SOURCES, fragments)
}

pub fn repository_prompt_source_version() -> String {
    let mut hasher = Sha256::new();
    for (name, source) in REPOSITORY_PROMPT_SOURCES {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(source.as_bytes());
        hasher.update([0]);
    }
    for (name, source) in REPOSITORY_BUNDLE_SOURCES {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(source.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}
