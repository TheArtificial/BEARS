use serde::Serialize;

// Prompt-memory domain types now live in `den-tools` (the tool-executor crate);
// re-exported here so existing `crate::prompt_memory_blocks::*` paths keep
// resolving. The runtime prompt-compilation logic below stays in `den`.
pub use den_tools::prompt_memory::{
    PromptMemoryBlock, PromptMemoryBlockScope, PromptMemoryBlockState, PromptMemoryBlockType,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptMemoryCompilationInput<'a> {
    pub role: &'a str,
    pub work_surfaces: &'a [String],
    pub session_id: &'a str,
    pub max_blocks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptMemoryCompilation {
    pub included_blocks: Vec<PromptMemoryBlock>,
    pub omitted_block_ids: Vec<String>,
}

fn block_matches_scope(block: &PromptMemoryBlock, input: &PromptMemoryCompilationInput<'_>) -> bool {
    if block.state != PromptMemoryBlockState::Active {
        return false;
    }
    match block.scope {
        PromptMemoryBlockScope::BearWide => true,
        PromptMemoryBlockScope::RoleLocal => block.role.as_deref() == Some(input.role),
        PromptMemoryBlockScope::WorkSurface => block
            .work_surface
            .as_ref()
            .map(|surface| input.work_surfaces.iter().any(|candidate| candidate == surface))
            .unwrap_or(false),
        PromptMemoryBlockScope::Session => block.session_id.as_deref() == Some(input.session_id),
    }
}

fn scope_rank(scope: PromptMemoryBlockScope) -> i32 {
    match scope {
        PromptMemoryBlockScope::Session => 4,
        PromptMemoryBlockScope::WorkSurface => 3,
        PromptMemoryBlockScope::RoleLocal => 2,
        PromptMemoryBlockScope::BearWide => 1,
    }
}

pub fn compile_prompt_memory_blocks(
    blocks: &[PromptMemoryBlock],
    input: PromptMemoryCompilationInput<'_>,
) -> PromptMemoryCompilation {
    let mut eligible = blocks
        .iter()
        .filter(|block| block_matches_scope(block, &input))
        .cloned()
        .collect::<Vec<_>>();
    eligible.sort_by(|a, b| {
        scope_rank(b.scope)
            .cmp(&scope_rank(a.scope))
            .then_with(|| b.priority.cmp(&a.priority))
            .then_with(|| a.title.cmp(&b.title))
    });
    let included_blocks = eligible.iter().take(input.max_blocks).cloned().collect::<Vec<_>>();
    let omitted_block_ids = eligible
        .iter()
        .skip(input.max_blocks)
        .map(|block| block.id.clone())
        .collect::<Vec<_>>();
    PromptMemoryCompilation {
        included_blocks,
        omitted_block_ids,
    }
}

pub fn render_prompt_memory_block_context(compilation: &PromptMemoryCompilation) -> String {
    if compilation.included_blocks.is_empty() {
        return "No prompt memory blocks are active for this runtime context.".to_string();
    }
    let rendered = compilation
        .included_blocks
        .iter()
        .map(|block| {
            format!(
                "[{}::{:?}::{:?}] {}",
                block.id, block.scope, block.block_type, block.body
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    if compilation.omitted_block_ids.is_empty() {
        format!(
            "Active Den-owned prompt memory blocks for this runtime context: {}",
            rendered
        )
    } else {
        format!(
            "Active Den-owned prompt memory blocks for this runtime context: {} Omitted lower-priority blocks due to prompt budgeting: {}.",
            rendered,
            compilation.omitted_block_ids.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(
        id: &str,
        scope: PromptMemoryBlockScope,
        role: Option<&str>,
        work_surface: Option<&str>,
        session_id: Option<&str>,
        priority: i32,
    ) -> PromptMemoryBlock {
        PromptMemoryBlock {
            id: id.to_string(),
            block_type: PromptMemoryBlockType::RoleGuidance,
            scope,
            state: PromptMemoryBlockState::Active,
            role: role.map(str::to_string),
            work_surface: work_surface.map(str::to_string),
            session_id: session_id.map(str::to_string),
            title: id.to_string(),
            body: format!("body:{id}"),
            priority,
        }
    }

    #[test]
    fn prompt_memory_compilation_prefers_more_specific_scopes() {
        let surfaces = vec!["/workspace".to_string()];
        let blocks = vec![
            block("bear", PromptMemoryBlockScope::BearWide, None, None, None, 1),
            block("role", PromptMemoryBlockScope::RoleLocal, Some("pair"), None, None, 1),
            block("surface", PromptMemoryBlockScope::WorkSurface, None, Some("/workspace"), None, 1),
            block("session", PromptMemoryBlockScope::Session, None, None, Some("sess-1"), 1),
        ];
        let compiled = compile_prompt_memory_blocks(
            &blocks,
            PromptMemoryCompilationInput {
                role: "pair",
                work_surfaces: &surfaces,
                session_id: "sess-1",
                max_blocks: 4,
            },
        );
        let ids = compiled
            .included_blocks
            .iter()
            .map(|block| block.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["session", "surface", "role", "bear"]);
    }

    #[test]
    fn prompt_memory_compilation_omits_lower_priority_blocks_when_budgeted() {
        let surfaces = vec!["/workspace".to_string()];
        let blocks = vec![
            block("bear", PromptMemoryBlockScope::BearWide, None, None, None, 1),
            block("role", PromptMemoryBlockScope::RoleLocal, Some("pair"), None, None, 1),
            block("surface", PromptMemoryBlockScope::WorkSurface, None, Some("/workspace"), None, 1),
        ];
        let compiled = compile_prompt_memory_blocks(
            &blocks,
            PromptMemoryCompilationInput {
                role: "pair",
                work_surfaces: &surfaces,
                session_id: "sess-1",
                max_blocks: 2,
            },
        );
        let ids = compiled
            .included_blocks
            .iter()
            .map(|block| block.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["surface", "role"]);
        assert_eq!(compiled.omitted_block_ids, vec!["bear"]);
    }
}
