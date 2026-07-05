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
        block(
            "bear",
            PromptMemoryBlockScope::BearWide,
            None,
            None,
            None,
            1,
        ),
        block(
            "role",
            PromptMemoryBlockScope::RoleLocal,
            Some("pair"),
            None,
            None,
            1,
        ),
        block(
            "surface",
            PromptMemoryBlockScope::WorkSurface,
            None,
            Some("/workspace"),
            None,
            1,
        ),
        block(
            "session",
            PromptMemoryBlockScope::Session,
            None,
            None,
            Some("sess-1"),
            1,
        ),
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
        block(
            "bear",
            PromptMemoryBlockScope::BearWide,
            None,
            None,
            None,
            1,
        ),
        block(
            "role",
            PromptMemoryBlockScope::RoleLocal,
            Some("pair"),
            None,
            None,
            1,
        ),
        block(
            "surface",
            PromptMemoryBlockScope::WorkSurface,
            None,
            Some("/workspace"),
            None,
            1,
        ),
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
