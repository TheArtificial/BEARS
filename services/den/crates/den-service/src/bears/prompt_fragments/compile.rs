use den_core::DenError;

use super::{
    bundle::PromptBundle, registry::PromptFragmentRegistry, render::CompileTimePromptContext,
    render::render_compile_time_fragment,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPromptFragment {
    pub id: String,
    pub body: String,
}

pub fn render_compile_time_bundle_fragments(
    bundle: &PromptBundle,
    fragments: &PromptFragmentRegistry,
    context: &CompileTimePromptContext<'_>,
) -> Result<Vec<RenderedPromptFragment>, DenError> {
    bundle
        .fragments
        .iter()
        .map(|fragment_id| {
            let fragment = fragments.require(fragment_id)?;
            Ok(RenderedPromptFragment {
                id: fragment.frontmatter.id.clone(),
                body: render_compile_time_fragment(fragment, context)?,
            })
        })
        .collect()
}
