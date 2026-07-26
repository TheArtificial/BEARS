use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::bearwire_events::BearWireEventRow;

const MAX_ACTIVITY_TEXT_CHARS: usize = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkActivityKind {
    AssistantMessage,
    ReasoningSummary,
    ToolCall,
    ToolResult,
    Approval,
    Lifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkActivityEntry {
    pub id: Uuid,
    pub first_sequence: i64,
    pub last_sequence: i64,
    pub kind: WorkActivityKind,
    pub text: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub created_at: OffsetDateTime,
    pub truncated: bool,
}

/// Projects the canonical BearWire event log into finalized, readable work activity.
///
/// Assistant and provider-exposed reasoning deltas are coalesced without inventing
/// hidden reasoning. Raw tool arguments/results are deliberately not copied into the
/// summary; the canonical event remains available to richer authorized diagnostics.
pub fn project_work_activity(
    rows: impl IntoIterator<Item = BearWireEventRow>,
) -> Vec<WorkActivityEntry> {
    let mut activity = Vec::new();

    for row in rows {
        let entry = match row.event_type.as_str() {
            "message.delta" => text_entry(&row, WorkActivityKind::AssistantMessage, "delta"),
            "message.reasoning.delta" if replayable_reasoning(&row) => {
                text_entry(&row, WorkActivityKind::ReasoningSummary, "delta")
            }
            "tool_call.requested" | "client.waiting" => Some(tool_entry(
                &row,
                WorkActivityKind::ToolCall,
                "Tool requested",
            )),
            "tool_call.completed"
            | "tool_call.warning"
            | "tool_call.cancelled"
            | "tool_call.failed" => Some(tool_entry(
                &row,
                WorkActivityKind::ToolResult,
                "Tool finished",
            )),
            "permission.requested" | "permission.resolved" | "permission.denied" => {
                Some(simple_entry(&row, WorkActivityKind::Approval))
            }
            event_type if event_type.starts_with("run.") => {
                Some(simple_entry(&row, WorkActivityKind::Lifecycle))
            }
            _ => None,
        };

        let Some(entry) = entry else { continue };
        if matches!(
            entry.kind,
            WorkActivityKind::AssistantMessage | WorkActivityKind::ReasoningSummary
        ) && activity.last().is_some_and(|previous: &WorkActivityEntry| {
            previous.kind == entry.kind && previous.last_sequence + 1 == entry.first_sequence
        }) {
            append_text(activity.last_mut().expect("checked above"), &entry.text);
            activity.last_mut().expect("checked above").last_sequence = entry.last_sequence;
        } else {
            activity.push(entry);
        }
    }

    activity
}

fn replayable_reasoning(row: &BearWireEventRow) -> bool {
    row.event
        .data
        .get("replay_policy")
        .and_then(serde_json::Value::as_str)
        == Some("thought")
}

fn text_entry(
    row: &BearWireEventRow,
    kind: WorkActivityKind,
    field: &str,
) -> Option<WorkActivityEntry> {
    let text = row.event.data.get(field)?.as_str()?;
    let mut entry = WorkActivityEntry {
        id: row.id,
        first_sequence: row.sequence_no,
        last_sequence: row.sequence_no,
        kind,
        text: String::new(),
        tool_call_id: None,
        tool_name: None,
        created_at: row.created_at,
        truncated: false,
    };
    append_text(&mut entry, text);
    Some(entry)
}

fn tool_entry(row: &BearWireEventRow, kind: WorkActivityKind, fallback: &str) -> WorkActivityEntry {
    let tool = row
        .event
        .data
        .pointer("/tool_call/name")
        .or_else(|| row.event.data.pointer("/tool_call/tool_call/name"))
        .and_then(|value| value.as_str());
    let summary = row
        .event
        .data
        .get("summary")
        .or_else(|| row.event.data.pointer("/tool_call/summary"))
        .and_then(|value| value.as_str());
    let tool_call_id = row
        .event
        .resource_refs
        .iter()
        .find(|resource| resource.kind == "tool_call")
        .map(|resource| resource.id.clone());
    let text = match (tool, summary) {
        (Some(tool), Some(summary)) => format!("{tool}: {summary}"),
        (Some(tool), None) => format!("{fallback}: {tool}"),
        (None, Some(summary)) => summary.to_string(),
        (None, None) => fallback.to_string(),
    };
    bounded_entry(row, kind, text, tool_call_id, tool.map(str::to_string))
}

fn simple_entry(row: &BearWireEventRow, kind: WorkActivityKind) -> WorkActivityEntry {
    let text = row
        .event
        .data
        .get("text")
        .or_else(|| row.event.data.get("message"))
        .or_else(|| row.event.data.get("reason"))
        .and_then(|value| value.as_str())
        .unwrap_or(&row.event_type)
        .to_string();
    bounded_entry(row, kind, text, None, None)
}

fn bounded_entry(
    row: &BearWireEventRow,
    kind: WorkActivityKind,
    text: String,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
) -> WorkActivityEntry {
    let mut entry = WorkActivityEntry {
        id: row.id,
        first_sequence: row.sequence_no,
        last_sequence: row.sequence_no,
        kind,
        text: String::new(),
        tool_call_id,
        tool_name,
        created_at: row.created_at,
        truncated: false,
    };
    append_text(&mut entry, &text);
    entry
}

fn append_text(entry: &mut WorkActivityEntry, text: &str) {
    let remaining = MAX_ACTIVITY_TEXT_CHARS.saturating_sub(entry.text.chars().count());
    if remaining == 0 {
        entry.truncated |= !text.is_empty();
        return;
    }
    entry.text.extend(text.chars().take(remaining));
    entry.truncated |= text.chars().count() > remaining;
}

#[cfg(test)]
mod tests {
    use bearwire_protocol::wire::{BearWireEvent, ResourceRef};
    use serde_json::json;
    use time::OffsetDateTime;

    use super::*;

    fn row(sequence: i64, event_type: &str, data: serde_json::Value) -> BearWireEventRow {
        let id = Uuid::from_u128(sequence as u128);
        let mut event =
            BearWireEvent::ephemeral(event_type, data).with_run_id(Some("run-1".into()));
        event.event_id = Some(format!("evt_{id}"));
        event.sequence = Some(sequence as u64);
        BearWireEventRow {
            id,
            sequence_no: sequence,
            session_id: "session-1".into(),
            event_type: event_type.into(),
            event,
            created_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn projects_ordered_messages_and_correlated_tools_without_raw_arguments() {
        let mut tool = row(
            5,
            "tool_call.requested",
            json!({"tool_call": {"name": "fs.read", "arguments": {"token": "secret"}}}),
        );
        tool.event
            .resource_refs
            .push(ResourceRef::new("tool_call", "call-1"));

        let activity = project_work_activity(vec![
            row(1, "message.delta", json!({"delta": "Inspecting "})),
            row(2, "message.delta", json!({"delta": "files."})),
            row(
                3,
                "message.reasoning.delta",
                json!({"delta": "private", "replay_policy": "none"}),
            ),
            row(
                4,
                "message.reasoning.delta",
                json!({"delta": "Checking inputs.", "replay_policy": "thought"}),
            ),
            tool,
            row(
                6,
                "tool_call.completed",
                json!({"summary": "read 12 lines"}),
            ),
            row(7, "run.completed", json!({"outcome": "ok"})),
        ]);

        assert_eq!(activity.len(), 5);
        assert_eq!(activity[0].text, "Inspecting files.");
        assert_eq!(activity[0].first_sequence, 1);
        assert_eq!(activity[0].last_sequence, 2);
        assert_eq!(activity[1].kind, WorkActivityKind::ReasoningSummary);
        assert_eq!(activity[1].text, "Checking inputs.");
        assert_eq!(activity[2].tool_call_id.as_deref(), Some("call-1"));
        assert!(!activity[2].text.contains("secret"));
        assert_eq!(activity[4].kind, WorkActivityKind::Lifecycle);
    }
}
