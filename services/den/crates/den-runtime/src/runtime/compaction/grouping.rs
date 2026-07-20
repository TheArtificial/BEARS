use den_service::conversation::persistence::{
    PersistedToolRequestPayload, PersistedToolResultPayload,
};
use serde_json::Value;

use crate::runtime_conversations::{RuntimeSemanticGroup, RuntimeSemanticGroupKind};

const APPROVAL_NEEDLE: &str = "approval";
const WORKFLOW_STATE_NEEDLE: &str = "workflow_state";
const PLAN_MODE_NEEDLE: &str = "plan_mode";
const ARTIFACT_NEEDLE: &str = "artifact";
const FILE_URL_NEEDLE: &str = "file://";

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

/// Build the one-row semantic group used by native runtime-message fallback inputs.
pub fn semantic_group_from_runtime_message(message: &Value) -> RuntimeSemanticGroup {
    let role = runtime_message_role(message);
    let content = runtime_message_content(message);
    let tool_call_id = runtime_message_tool_call_id(message);
    let has_tool_identity = tool_call_id.is_some() || message.get("tool_name").is_some();
    let kind = classify_runtime_message_parts(role, content, message, has_tool_identity);
    let protected = runtime_message_group_is_protected(&kind);

    RuntimeSemanticGroup {
        kind,
        start_message_id: tool_call_id.clone(),
        end_message_id: tool_call_id,
        message_count: 1,
        protected,
    }
}

fn runtime_message_role(message: &Value) -> &str {
    message
        .get("role")
        .or_else(|| message.get("message_type"))
        .and_then(Value::as_str)
        .unwrap_or("message")
}

fn runtime_message_content(message: &Value) -> &str {
    message
        .get("content")
        .and_then(Value::as_str)
        .or_else(|| message.get("text").and_then(Value::as_str))
        .unwrap_or_default()
}

fn runtime_message_tool_call_id(message: &Value) -> Option<String> {
    message
        .get("tool_call_id")
        .or_else(|| message.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn classify_runtime_message_parts(
    role: &str,
    content: &str,
    message: &Value,
    has_tool_identity: bool,
) -> RuntimeSemanticGroupKind {
    if role.eq_ignore_ascii_case("tool") || has_tool_identity {
        RuntimeSemanticGroupKind::ToolInteraction
    } else if message.get("approval_request_id").is_some()
        || message.get("approvals").is_some()
        || content_mentions_approval(content)
    {
        RuntimeSemanticGroupKind::ApprovalInteraction
    } else if content_mentions_workflow_update(content) {
        RuntimeSemanticGroupKind::WorkflowUpdate
    } else if content_mentions_artifact_update(content) {
        RuntimeSemanticGroupKind::ArtifactUpdate
    } else if role.eq_ignore_ascii_case("user") {
        RuntimeSemanticGroupKind::UserTurn
    } else if role.eq_ignore_ascii_case("assistant") {
        RuntimeSemanticGroupKind::AssistantReply
    } else if role.eq_ignore_ascii_case("system") {
        RuntimeSemanticGroupKind::SystemEvent
    } else {
        RuntimeSemanticGroupKind::AssistantReply
    }
}

fn runtime_message_group_is_protected(kind: &RuntimeSemanticGroupKind) -> bool {
    matches!(
        *kind,
        RuntimeSemanticGroupKind::ToolInteraction
            | RuntimeSemanticGroupKind::ApprovalInteraction
            | RuntimeSemanticGroupKind::WorkflowUpdate
    )
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

fn tool_request_payload(row: &TranscriptGroupingRow) -> Option<PersistedToolRequestPayload> {
    (row.message_type == "tool_call")
        .then(|| PersistedToolRequestPayload::try_from(&row.content_json).ok())
        .flatten()
}

fn tool_result_payload(row: &TranscriptGroupingRow) -> Option<PersistedToolResultPayload> {
    (row.message_type == "tool_result")
        .then(|| PersistedToolResultPayload::try_from(&row.content_json).ok())
        .flatten()
}

fn is_tool_call_request(row: &TranscriptGroupingRow) -> bool {
    tool_request_payload(row).is_some()
}

fn is_tool_result_row(row: &TranscriptGroupingRow) -> bool {
    tool_result_payload(row).is_some()
}

fn tool_call_id_from_row(row: &TranscriptGroupingRow) -> Option<String> {
    row.tool_call_id
        .clone()
        .or_else(|| tool_request_payload(row).map(|payload| payload.tool_call_id))
        .or_else(|| tool_result_payload(row).and_then(|payload| payload.tool_call_id))
        .filter(|value| !value.is_empty())
}

fn row_identity(row: &TranscriptGroupingRow) -> Option<String> {
    row.message_id
        .clone()
        .or_else(|| row.sequence_no.map(|sequence| sequence.to_string()))
        .or_else(|| tool_call_id_from_row(row))
}

fn is_incomplete_tool_result_payload(payload: &PersistedToolResultPayload) -> bool {
    payload.status == den_core::tools::result_compaction::ToolResultStatus::Incomplete
}

fn is_approval_interaction_row(row: &TranscriptGroupingRow) -> bool {
    if let Some(payload) = tool_request_payload(row) {
        if payload
            .approval_request_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || payload.approval_required
        {
            return true;
        }
    }
    if row
        .content_json
        .get("event")
        .and_then(Value::as_str)
        .is_some_and(|event| event.contains(APPROVAL_NEEDLE))
    {
        return true;
    }
    content_mentions_approval(&row.content_text)
}

fn content_mentions_approval(content: &str) -> bool {
    content.to_ascii_lowercase().contains(APPROVAL_NEEDLE)
}

fn content_mentions_workflow_update(content: &str) -> bool {
    let content = content.to_ascii_lowercase();
    content.contains(WORKFLOW_STATE_NEEDLE) || content.contains(PLAN_MODE_NEEDLE)
}

fn content_mentions_artifact_update(content: &str) -> bool {
    content.contains(ARTIFACT_NEEDLE) || content.contains(FILE_URL_NEEDLE)
}

fn is_workflow_update_row(row: &TranscriptGroupingRow) -> bool {
    content_mentions_workflow_update(&row.content_text)
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
    content_mentions_artifact_update(&row.content_text)
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

fn group_is_protected(kind: &RuntimeSemanticGroupKind, unresolved: bool) -> bool {
    unresolved
        || matches!(
            *kind,
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
    let protected = group_is_protected(&kind, false);
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
        if let Some(result_payload) = tool_result_payload(next) {
            let result_call_id = result_payload.tool_call_id.as_deref();
            if call_id
                .as_deref()
                .is_some_and(|expected| result_call_id == Some(expected))
            {
                message_count = 2;
                end_row = next;
                has_result = true;
                result_incomplete = is_incomplete_tool_result_payload(&result_payload);
            }
        }
    }

    let unresolved = !has_result || result_incomplete;
    let kind = if approval_pending && unresolved {
        RuntimeSemanticGroupKind::ApprovalInteraction
    } else {
        RuntimeSemanticGroupKind::ToolInteraction
    };
    let protected = group_is_protected(&kind, unresolved);

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
