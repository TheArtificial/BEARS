use serde_json::Value;

use crate::runtime_conversations::{RuntimeSemanticGroup, RuntimeSemanticGroupKind};

/// One persisted `conversation_messages` row used for transcript-backed semantic grouping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptGroupingRow {
    pub message_id: Option<String>,
    pub sequence_no: Option<i64>,
    pub message_type: String,
    pub content_text: String,
    pub content_json: Value,
    pub tool_call_id: Option<String>,
}

impl TranscriptGroupingRow {
    pub fn new(
        message_type: impl Into<String>,
        content_text: impl Into<String>,
        content_json: Value,
    ) -> Self {
        Self {
            message_id: None,
            sequence_no: None,
            message_type: message_type.into(),
            content_text: content_text.into(),
            content_json,
            tool_call_id: None,
        }
    }

    pub fn with_message_id(mut self, message_id: impl Into<String>) -> Self {
        self.message_id = Some(message_id.into());
        self
    }

    pub fn with_sequence_no(mut self, sequence_no: i64) -> Self {
        self.sequence_no = Some(sequence_no);
        self
    }

    pub fn with_tool_call_id(mut self, tool_call_id: impl Into<String>) -> Self {
        self.tool_call_id = Some(tool_call_id.into());
        self
    }
}

/// Build semantic compaction groups from ordered Postgres transcript rows.
pub fn semantic_groups_from_conversation_messages(
    rows: &[TranscriptGroupingRow],
) -> Vec<RuntimeSemanticGroup> {
    let mut groups = Vec::new();
    let mut index = 0;

    while index < rows.len() {
        let row = &rows[index];
        if !row_counts_toward_grouping(row) {
            index += 1;
            continue;
        }

        if row.message_type == "compaction_marker" {
            groups.push(compaction_boundary_group(row));
            index += 1;
            continue;
        }

        if is_tool_call_request(row) {
            let (group, consumed) = tool_interaction_group(rows, index);
            groups.push(group);
            index += consumed;
            continue;
        }

        if is_tool_result_row(row) {
            groups.push(orphan_tool_result_group(row));
            index += 1;
            continue;
        }

        groups.push(single_row_group(row));
        index += 1;
    }

    groups
}

fn row_counts_toward_grouping(row: &TranscriptGroupingRow) -> bool {
    match row.message_type.as_str() {
        "user" | "assistant" => !row.content_text.trim().is_empty(),
        "tool_call" => is_tool_call_request(row),
        "tool_result" => is_tool_result_row(row),
        "compaction_marker" | "system" | "developer" | "workflow_event" => true,
        _ => false,
    }
}

fn is_tool_call_request(row: &TranscriptGroupingRow) -> bool {
    row.message_type == "tool_call"
        && row
            .content_json
            .get("event")
            .and_then(Value::as_str)
            == Some("tool_request")
}

fn is_tool_result_row(row: &TranscriptGroupingRow) -> bool {
    row.message_type == "tool_result"
        && row
            .content_json
            .get("event")
            .and_then(Value::as_str)
            == Some("tool_result")
}

fn tool_call_id_from_row(row: &TranscriptGroupingRow) -> Option<String> {
    row.tool_call_id.clone().or_else(|| {
        row.content_json
            .get("tool_call_id")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn row_identity(row: &TranscriptGroupingRow) -> Option<String> {
    row.message_id
        .clone()
        .or_else(|| row.sequence_no.map(|sequence| sequence.to_string()))
        .or_else(|| tool_call_id_from_row(row))
}

fn is_incomplete_tool_result(row: &TranscriptGroupingRow) -> bool {
    row.content_json
        .get("status")
        .and_then(Value::as_str)
        == Some("incomplete")
}

fn is_approval_interaction_row(row: &TranscriptGroupingRow) -> bool {
    if row
        .content_json
        .get("approval_request_id")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        return true;
    }
    if row.content_json.get("approval_required").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    if row
        .content_json
        .get("event")
        .and_then(Value::as_str)
        .is_some_and(|event| event.contains("approval"))
    {
        return true;
    }
    row.content_text.to_ascii_lowercase().contains("approval")
}

fn is_workflow_update_row(row: &TranscriptGroupingRow) -> bool {
    let content = row.content_text.to_ascii_lowercase();
    content.contains("workflow_state")
        || content.contains("plan_mode")
        || row
            .content_json
            .get("event")
            .and_then(Value::as_str)
            .is_some_and(|event| {
                matches!(
                    event,
                    "turn_result" | "conversation_resolved" | "workflow_state"
                )
            })
}

fn is_artifact_update_row(row: &TranscriptGroupingRow) -> bool {
    row.content_text.contains("artifact") || row.content_text.contains("file://")
}

fn classify_non_tool_row(row: &TranscriptGroupingRow) -> RuntimeSemanticGroupKind {
    match row.message_type.as_str() {
        "user" => RuntimeSemanticGroupKind::UserTurn,
        "assistant" => RuntimeSemanticGroupKind::AssistantReply,
        "system" | "developer" => RuntimeSemanticGroupKind::SystemEvent,
        "compaction_marker" => RuntimeSemanticGroupKind::PriorCompactionArtifact,
        "workflow_event" if is_approval_interaction_row(row) => {
            RuntimeSemanticGroupKind::ApprovalInteraction
        }
        "workflow_event" if is_workflow_update_row(row) => RuntimeSemanticGroupKind::WorkflowUpdate,
        "workflow_event" if is_artifact_update_row(row) => RuntimeSemanticGroupKind::ArtifactUpdate,
        "workflow_event" => RuntimeSemanticGroupKind::WorkflowUpdate,
        _ => RuntimeSemanticGroupKind::AssistantReply,
    }
}

fn group_is_protected(kind: RuntimeSemanticGroupKind, unresolved: bool) -> bool {
    unresolved
        || matches!(
            kind,
            RuntimeSemanticGroupKind::ApprovalInteraction
                | RuntimeSemanticGroupKind::WorkflowUpdate
                | RuntimeSemanticGroupKind::PriorCompactionArtifact
        )
}

fn compaction_boundary_group(row: &TranscriptGroupingRow) -> RuntimeSemanticGroup {
    RuntimeSemanticGroup {
        kind: RuntimeSemanticGroupKind::PriorCompactionArtifact,
        start_message_id: row_identity(row),
        end_message_id: row_identity(row),
        message_count: 1,
        protected: true,
    }
}

fn single_row_group(row: &TranscriptGroupingRow) -> RuntimeSemanticGroup {
    let kind = classify_non_tool_row(row);
    let protected = group_is_protected(kind.clone(), false);
    RuntimeSemanticGroup {
        kind,
        start_message_id: row_identity(row),
        end_message_id: row_identity(row),
        message_count: 1,
        protected,
    }
}

fn orphan_tool_result_group(row: &TranscriptGroupingRow) -> RuntimeSemanticGroup {
    RuntimeSemanticGroup {
        kind: RuntimeSemanticGroupKind::ToolInteraction,
        start_message_id: row_identity(row),
        end_message_id: row_identity(row),
        message_count: 1,
        protected: true,
    }
}

fn tool_interaction_group(
    rows: &[TranscriptGroupingRow],
    start_index: usize,
) -> (RuntimeSemanticGroup, usize) {
    let call_row = &rows[start_index];
    let call_id = tool_call_id_from_row(call_row);
    let approval_pending = is_approval_interaction_row(call_row);

    let mut message_count = 1;
    let mut end_row = call_row;
    let mut has_result = false;
    let mut result_incomplete = false;

    if start_index + 1 < rows.len() {
        let next = &rows[start_index + 1];
        if is_tool_result_row(next)
            && call_id
                .as_deref()
                .is_some_and(|expected| tool_call_id_from_row(next).as_deref() == Some(expected))
        {
            message_count = 2;
            end_row = next;
            has_result = true;
            result_incomplete = is_incomplete_tool_result(next);
        }
    }

    let unresolved = !has_result || result_incomplete;
    let kind = if approval_pending && unresolved {
        RuntimeSemanticGroupKind::ApprovalInteraction
    } else {
        RuntimeSemanticGroupKind::ToolInteraction
    };
    let protected = group_is_protected(kind.clone(), unresolved);

    (
        RuntimeSemanticGroup {
            kind,
            start_message_id: row_identity(call_row),
            end_message_id: row_identity(end_row),
            message_count,
            protected,
        },
        message_count,
    )
}
