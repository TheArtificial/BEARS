mod frontmatter;
mod registry;
mod render;
mod repository;

pub use frontmatter::PromptFragmentFrontmatter;
pub use registry::{PromptFragment, PromptFragmentRegistry};
pub use render::{render_compile_time_fragment, render_compile_time_text, CompileTimePromptContext};
pub use repository::{repository_prompt_fragment_registry, repository_prompt_source_version};

#[cfg(test)]
mod tests;
