
use super::*;

#[test]
fn effective_merges_memory_git_remote() {
    let stored = serde_json::json!({
        "memory": { "git_remote": "https://example.com/repo.git" }
    });
    let v = effective_runtime_plan(Some(&stored));
    assert_eq!(v["memory"]["git_remote"], "https://example.com/repo.git");
    assert_eq!(v["memory"]["git_ref"], "main");
    assert_eq!(v["version"], RUNTIME_PLAN_VERSION);
}
