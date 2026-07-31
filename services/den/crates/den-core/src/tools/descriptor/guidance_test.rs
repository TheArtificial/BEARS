use crate::{
    client_tools::{provider_tool_descriptor, ClientToolName},
    tools::{
        constants::{
            DEN_JOB_CREATE_PROVIDER, DEN_MEMORY_WRITE_ENTRY_PROVIDER,
            DEN_PROMPT_MEMORY_UPSERT_PROVIDER, DEN_SITUATION_GET_PROVIDER,
            DEN_TASK_CREATE_PROVIDER, DEN_TASK_LISTS_UPDATE_PROVIDER,
            DEN_TASK_LIST_CHECKOUT_PROVIDER, DEN_TASK_LIST_PROVIDER,
        },
        descriptor::builtin_den_tool_descriptors,
    },
};

#[test]
fn apply_patch_schema_explains_required_diff_and_edit_file_fallback() {
    let schema = provider_tool_descriptor(ClientToolName::ApplyPatch)["parameters"].clone();
    let description = schema["properties"]["patch"]["description"]
        .as_str()
        .expect("patch description");

    assert!(description.contains("Restricted patch format"));
    assert!(description.contains("--- a/path"));
    assert!(description.contains("full target content"));
    assert!(description.contains("Markdown fences"));
    assert!(description.contains("fs_edit_file"));
}

#[test]
fn session_info_descriptor_keeps_explicit_orientation_language() {
    let descriptor = builtin_den_tool_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.provider_name == DEN_SITUATION_GET_PROVIDER)
        .expect("session_info descriptor");

    assert!(
        descriptor
            .description
            .contains("Trusted Den orientation tool"),
        "unexpected description: {}",
        descriptor.description
    );
    assert!(
        descriptor.description.contains("Read-only"),
        "unexpected description: {}",
        descriptor.description
    );
}

#[test]
fn memory_write_descriptor_includes_shared_guidance() {
    let descriptor = builtin_den_tool_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.provider_name == DEN_MEMORY_WRITE_ENTRY_PROVIDER)
        .expect("memory_write_entry descriptor");

    assert!(descriptor.description.contains("Scope:"));
    assert!(descriptor.description.contains("Side effect:"));
    assert!(descriptor.description.contains("session_info"));
    assert!(descriptor
        .description
        .contains("writes role-local semantic memory"));
}

#[test]
fn task_list_update_descriptor_includes_active_work_state_guidance() {
    let descriptor = builtin_den_tool_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.provider_name == DEN_TASK_LISTS_UPDATE_PROVIDER)
        .expect("update_task_list descriptor");

    assert!(descriptor.description.contains("Scope:"));
    assert!(descriptor
        .description
        .contains("Side effect: updates active work state"));
    assert!(descriptor.description.contains("session_info"));
    assert!(descriptor
        .description
        .contains("Use durable Docket job/task tools"));
    assert!(descriptor.description.contains("user-visible"));
    assert!(descriptor.description.contains("current Pair task tree"));
    assert!(descriptor
        .description
        .contains("do not authorize autonomous execution"));
    assert!(descriptor.description.contains("checkout_task_list"));
}

#[test]
fn docket_descriptors_distinguish_pair_task_trees_from_work_jobs() {
    let descriptors = builtin_den_tool_descriptors();

    let create_job = descriptors
        .iter()
        .find(|descriptor| descriptor.provider_name == DEN_JOB_CREATE_PROVIDER)
        .expect("create_job descriptor");
    assert!(create_job.description.contains("durable Docket job"));
    assert!(create_job.description.contains("distinct objectives"));
    assert!(create_job
        .description
        .contains("small change that Pair can finish here"));
    assert!(create_job
        .description
        .contains("does not execute or dispatch it"));

    let create_task = descriptors
        .iter()
        .find(|descriptor| descriptor.provider_name == DEN_TASK_CREATE_PROVIDER)
        .expect("create_task descriptor");
    assert!(create_task.description.contains("durable/resumable plans"));
    assert!(create_task.description.contains("current Pair task tree"));
    assert!(create_task.description.contains("does not execute work"));

    let list_tasks = descriptors
        .iter()
        .find(|descriptor| descriptor.provider_name == DEN_TASK_LIST_PROVIDER)
        .expect("list_tasks descriptor");
    assert!(list_tasks.description.contains("current Pair task tree"));

    let checkout = descriptors
        .iter()
        .find(|descriptor| descriptor.provider_name == DEN_TASK_LIST_CHECKOUT_PROVIDER)
        .expect("checkout_task_list descriptor");
    assert!(checkout.description.contains("current Pair task tree"));
    assert!(checkout.description.contains("does not execute tasks"));
    assert!(!checkout
        .input_schema
        .get("required")
        .is_some_and(|required| {
            required
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item == "job_id"))
        }));
}

#[test]
fn prompt_memory_upsert_descriptor_mentions_runtime_prompt_memory() {
    let descriptor = builtin_den_tool_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.provider_name == DEN_PROMPT_MEMORY_UPSERT_PROVIDER)
        .expect("upsert_prompt_memory descriptor");

    assert!(descriptor
        .description
        .contains("editable runtime prompt memory"));
    assert!(descriptor.description.contains("semantic memory"));
}

#[test]
fn docket_work_descriptors_keep_execution_evidence_and_surfaces_explicit() {
    let descriptors = builtin_den_tool_descriptors();
    let dispatch = descriptors
        .iter()
        .find(|descriptor| descriptor.provider_name == "dispatch_work")
        .expect("dispatch_work descriptor");
    assert!(dispatch.description.contains("separate execution surface"));
    assert!(dispatch.description.contains("work run e4e4797b"));

    let work_run = descriptors
        .iter()
        .find(|descriptor| descriptor.provider_name == "get_work_run")
        .expect("get_work_run descriptor");
    assert!(work_run
        .description
        .contains("terminal result and durable evidence"));
    assert!(work_run
        .description
        .contains("not implicitly accessible to Pair"));
    assert!(work_run.description.contains("work run e4e4797b"));

    for (provider_name, expected_handle) in [
        ("create_job", "job e4e4797b"),
        (DEN_TASK_CREATE_PROVIDER, "task e4e4797b"),
        (DEN_TASK_LIST_PROVIDER, "short task handles"),
        ("list_work_runs", "short work-run handles"),
    ] {
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.provider_name == provider_name)
            .unwrap_or_else(|| panic!("{provider_name} descriptor"));
        assert!(descriptor.description.contains(expected_handle));
    }
}
