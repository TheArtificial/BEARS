use serde_json::json;

use crate::db::validate_primary_output_evidence;

fn accepted_evidence() -> serde_json::Value {
    json!({
        "primary_output": {
            "kind": "git_commit",
            "artifact_ref": "git:0123456789abcdef0123456789abcdef01234567",
            "immutable_identity": "0123456789abcdef0123456789abcdef01234567"
        },
        "validation": {
            "primary_output_ref": "git:0123456789abcdef0123456789abcdef01234567",
            "immutable_identity": "0123456789abcdef0123456789abcdef01234567",
            "command": "cargo test -p den-docket",
            "result": "passed",
            "execution_provenance": "local test"
        }
    })
}

#[test]
fn accepts_validation_bound_to_primary_output_identity() {
    assert!(validate_primary_output_evidence(Some(&accepted_evidence())).is_ok());
}

#[test]
fn rejects_missing_primary_output_or_validation() {
    assert!(validate_primary_output_evidence(None).is_err());
    assert!(validate_primary_output_evidence(Some(&json!({
        "primary_output": {
            "kind": "git_commit",
            "artifact_ref": "git:abc",
            "immutable_identity": "abc"
        }
    })))
    .is_err());
}

#[test]
fn rejects_validation_for_a_different_output_identity() {
    let mut evidence = accepted_evidence();
    evidence["validation"]["immutable_identity"] = json!("different-commit");
    assert!(validate_primary_output_evidence(Some(&evidence)).is_err());
}
