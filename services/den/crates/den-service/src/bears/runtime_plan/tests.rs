use super::*;

#[test]
fn effective_merges_memory_git_remote() {
    let stored = serde_json::json!({
        "memory": { "git_remote": "https://example.com/repo.git" }
    });
    let v = effective_runtime_plan(Some(&stored)).expect("runtime plan parses");
    assert_eq!(v["memory"]["git_remote"], "https://example.com/repo.git");
    assert_eq!(v["memory"]["git_ref"], "main");
    assert_eq!(v["version"], RUNTIME_PLAN_VERSION);
}

#[test]
fn runtime_plan_rejects_unknown_json_fields() {
    let stored = serde_json::json!({
        "version": RUNTIME_PLAN_VERSION,
        "memory": {
            "git_ref": "main",
            "seed_template": "default",
            "typo_git_remote": "https://example.com/repo.git"
        }
    });

    let err = effective_runtime_plan(Some(&stored)).expect_err("unknown fields are rejected");
    assert!(err.to_string().contains("unknown field `typo_git_remote`"));
}
