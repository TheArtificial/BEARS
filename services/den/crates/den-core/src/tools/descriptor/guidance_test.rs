use crate::tools::{
    constants::{
        DEN_JOB_CREATE_PROVIDER, DEN_MEMORY_WRITE_ENTRY_PROVIDER,
        DEN_PROMPT_MEMORY_UPSERT_PROVIDER, DEN_SITUATION_GET_PROVIDER, DEN_TASK_CREATE_PROVIDER,
        DEN_TASK_LISTS_UPDATE_PROVIDER, DEN_TASK_LIST_CHECKOUT_PROVIDER, DEN_TASK_LIST_PROVIDER,
    },
    descriptor::builtin_den_tool_descriptors,
};

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
    assert!(descriptor
        .description
        .contains("current conversation's implied Docket objective"));
    assert!(descriptor
        .description
        .contains("do not authorize autonomous execution"));
    assert!(descriptor.description.contains("checkout_task_list"));
}

#[test]
fn docket_descriptors_distinguish_conversation_objectives_from_explicit_jobs() {
    let descriptors = builtin_den_tool_descriptors();

    let create_job = descriptors
        .iter()
        .find(|descriptor| descriptor.provider_name == DEN_JOB_CREATE_PROVIDER)
        .expect("create_job descriptor");
    assert!(create_job.description.contains("Explicit Jobs"));
    assert!(create_job.description.contains("distinct objectives"));
    assert!(create_job
        .description
        .contains("implied conversation objective"));
    assert!(create_job.description.contains("Does not execute the job"));

    let create_task = descriptors
        .iter()
        .find(|descriptor| descriptor.provider_name == DEN_TASK_CREATE_PROVIDER)
        .expect("create_task descriptor");
    assert!(create_task.description.contains("durable/resumable plans"));
    assert!(create_task
        .description
        .contains("current conversation's implied Docket objective"));
    assert!(create_task.description.contains("does not execute work"));

    let list_tasks = descriptors
        .iter()
        .find(|descriptor| descriptor.provider_name == DEN_TASK_LIST_PROVIDER)
        .expect("list_tasks descriptor");
    assert!(list_tasks
        .description
        .contains("current conversation's implied Docket objective"));

    let checkout = descriptors
        .iter()
        .find(|descriptor| descriptor.provider_name == DEN_TASK_LIST_CHECKOUT_PROVIDER)
        .expect("checkout_task_list descriptor");
    assert!(checkout
        .description
        .contains("current conversation's implied Docket objective"));
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
