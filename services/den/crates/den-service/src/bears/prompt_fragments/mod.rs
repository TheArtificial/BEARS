mod bundle;
mod compile;
mod frontmatter;
mod registry;
mod render;
mod repository;

pub use bundle::{PromptBundle, PromptBundleRegistry};
pub use compile::{render_compile_time_bundle_fragments, RenderedPromptFragment};
pub use frontmatter::PromptFragmentFrontmatter;
pub use registry::{PromptFragment, PromptFragmentRegistry};
pub use render::{
    render_compile_time_fragment, render_compile_time_text, render_turn_fragment, render_turn_text,
    CompileTimePromptContext,
};
pub use repository::{
    repository_prompt_bundle_registry, repository_prompt_fragment_registry,
    repository_prompt_source_version,
};

#[cfg(test)]
mod tests;
