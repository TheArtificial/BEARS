use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use den_core::profile::BearProfile;
use den_docket::TaskListProjection;
use den_protocol::ContextBudgetReport;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    agent_loop::{
        CheckpointNextAction, CheckpointState, KeyMemoryProjectionCacheKey, ResolvedAgentLoopControl,
        RuntimeCheckpointRequest, StrategyProfile, ToolCallBudgetLimits, TurnBudgetPolicy,
        TurnBudgetState,
    },
    context_budget::AssembledTurnBudgetComponents,
    llm::{ChatMessage, LlmApiStyle, LlmRequestTelemetry, LlmToolDefinition},
};

#[derive(Debug, Clone)]
pub struct AgentLoopSession {
    pub session_key: String,
    pub bear_id: Uuid,
    pub bear_slug: String,
    pub user_id: Option<i32>,
    pub conversation_id: String,
    pub client_session_id: String,
    pub workspace_roots: Vec<String>,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<LlmToolDefinition>,
    pub budget_components: AssembledTurnBudgetComponents,
    pub model: String,
    pub model_context_window: Option<u32>,
    pub model_max_output_tokens: Option<u32>,
    pub bifrost_virtual_key: Option<String>,
    pub api_style: Option<LlmApiStyle>,
    pub step: u32,
    pub turn_budget: TurnBudgetPolicy,
    pub turn_budget_state: TurnBudgetState,
    pub agent_loop_control: ResolvedAgentLoopControl,
    pub checkpoint_state: CheckpointState,
    pub pending_checkpoint_request: Option<RuntimeCheckpointRequest>,
    pub pending_checkpoint_task_action: Option<CheckpointNextAction>,
    pub strategy: StrategyProfile,
    pub stream_tokens: bool,
    pub key_memory_projection_cache_key: Option<KeyMemoryProjectionCacheKey>,
    pub latest_context_budget: Option<ContextBudgetReport>,
    pub latest_projected_memory: Option<Value>,
    pub latest_recalled_memory: Option<Value>,
    pub active_activity_plan: Option<TaskListProjection>,
    pub profile: BearProfile,
    pub overflow_retry_attempted: bool,
    pub overflow_compaction_recovered: bool,
}

impl AgentLoopSession {
    pub fn llm_telemetry(&self) -> LlmRequestTelemetry {
        LlmRequestTelemetry {
            request_id: self.request_id.clone(),
            run_id: self.run_id.clone(),
            session_id: Some(self.client_session_id.clone()),
            conversation_id: Some(self.conversation_id.clone()),
            bear_id: Some(self.bear_id.to_string()),
            stance: Some(self.profile.as_str().to_string()),
            bifrost_virtual_key: self.bifrost_virtual_key.clone(),
        }
    }

    pub fn session_info_runtime_snapshot(&self) -> Value {
        let elapsed_ms = self.turn_budget_state.started_at.elapsed().as_millis() as u64;
        let remaining_wall_clock_ms = self
            .turn_budget
            .max_wall_clock_ms
            .saturating_sub(elapsed_ms);
        let next_incomplete_task = self
            .active_activity_plan
            .as_ref()
            .and_then(|plan| {
                plan.items.iter().find(|item| {
                    matches!(
                        item.status,
                        den_docket::TaskListItemStatus::Pending
                            | den_docket::TaskListItemStatus::InProgress
                    )
                })
            })
            .map(|item| item.title.clone());
        json!({
            "schema": "den.runtime_state.v1",
            "state": "active",
            "source": "native_agent_loop_session",
            "active_turn": {
                "present": true,
                "step": self.step,
                "elapsed_ms": elapsed_ms,
                "wall_clock": {
                    "elapsed_ms": elapsed_ms,
                    "remaining_ms": remaining_wall_clock_ms,
                    "limit_ms": self.turn_budget.max_wall_clock_ms,
                },
                "pending_obligations": 0,
                "pending_adapter_tools": 0,
                "pending_den_tools": pending_tool_call_count(&self.messages),
                "pending_permissions": 0,
            },
            "agent_loop_control": serde_json::to_value(&self.agent_loop_control).unwrap_or_else(|_| json!({
                "level": self.agent_loop_control.level.as_str(),
            })),
            "checkpoint_state": serde_json::to_value(&self.checkpoint_state).unwrap_or(Value::Null),
            "pending_checkpoint_request": serde_json::to_value(&self.pending_checkpoint_request).unwrap_or(Value::Null),
            "pending_checkpoint_task_action": serde_json::to_value(&self.pending_checkpoint_task_action).unwrap_or(Value::Null),
            "budgets": {
                "turn": {
                    "max_wall_clock_ms": self.turn_budget.max_wall_clock_ms,
                    "emergency_hard_steps": self.turn_budget.emergency_hard_steps,
                    "remaining_steps_before_hard_fuse": self.turn_budget.emergency_hard_steps.saturating_sub(self.step),
                    "max_consecutive_tool_failures": self.turn_budget.max_consecutive_tool_failures,
                    "max_same_tool_signature_repeats": self.turn_budget.max_same_tool_signature_repeats,
                    "budget_finalization_grace_used": self.turn_budget_state.budget_finalization_grace_used,
                },
                "tool_calls": {
                    "limits": tool_call_limits_json(self.turn_budget.tool_call_limits),
                    "usage": tool_call_usage_json(self.turn_budget_state.tool_usage),
                    "post_mutation_verification_window": self.turn_budget.post_mutation_verification_window.map(|window| json!({
                        "replenish_read": window.replenish_read,
                        "replenish_search": window.replenish_search,
                    })),
                },
            },
            "loop_guards": {
                "consecutive_tool_failures": self.turn_budget_state.consecutive_tool_failures,
                "max_consecutive_tool_failures": self.turn_budget.max_consecutive_tool_failures,
                "same_tool_signature_repeats": self.turn_budget_state.same_batch_signature_repeats,
                "max_same_tool_signature_repeats": self.turn_budget.max_same_tool_signature_repeats,
                "last_batch_signature_present": self.turn_budget_state.last_batch_signature.is_some(),
            },
            "task_focus": {
                "active": self.active_activity_plan.is_some(),
                "plan_id": self.active_activity_plan.as_ref().map(|plan| plan.id.to_string()),
                "plan_title": self.active_activity_plan.as_ref().map(|plan| plan.title.clone()),
                "plan_status": self.active_activity_plan.as_ref().map(|plan| plan.status.clone()),
                "next_incomplete_task_title": next_incomplete_task,
                "continuation_policy": if self.active_activity_plan.is_some() {
                    "continue_until_complete_or_blocked"
                } else {
                    "inactive"
                },
            },
            "docket": docket_context_json(self.active_activity_plan.as_ref()),
            "last_budget_advisory": last_system_advisory(&self.messages, "Budget advisory:"),
            "last_task_focus_advisory": last_system_advisory(&self.messages, "You are in autonomous implementation mode."),
        })
    }
}

fn pending_tool_call_count(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .rev()
        .find_map(|message| message.tool_calls.as_ref().map(Vec::len))
        .unwrap_or(0)
}

fn docket_context_json(plan: Option<&TaskListProjection>) -> Value {
    let active_task = plan
        .and_then(|plan| plan.current_item.as_ref())
        .or_else(|| {
            plan.and_then(|plan| {
                plan.items.iter().find(|item| {
                    matches!(
                        item.status,
                        den_docket::TaskListItemStatus::Pending
                            | den_docket::TaskListItemStatus::InProgress
                    )
                })
            })
        });
    json!({
        "active_job_id": plan
            .and_then(|plan| plan.source_ref.docket_job_id.clone())
            .or_else(|| plan.map(|plan| plan.id.to_string())),
        "active_run_id": Value::Null,
        "active_task_id": active_task.and_then(|item| item.source_ref.docket_task_id.clone()),
        "active_task_title": active_task.map(|item| item.title.clone()),
        "source": if plan.is_some() { "task_focus_projection" } else { "none" },
    })
}

fn last_system_advisory(messages: &[ChatMessage], prefix: &str) -> Option<Value> {
    messages.iter().rev().find_map(|message| {
        let content = message.content.as_deref()?;
        if message.role == "system" && content.starts_with(prefix) {
            Some(json!({
                "present": true,
                "summary": content,
            }))
        } else {
            None
        }
    })
}

fn tool_call_limits_json(limits: ToolCallBudgetLimits) -> Value {
    json!({
        "total": limits.total,
        "read": limits.read,
        "search": limits.search,
        "fetch": limits.fetch,
        "execute": limits.execute,
        "write": limits.write,
        "destructive": limits.destructive,
        "other": limits.other,
    })
}

fn tool_call_usage_json(usage: crate::agent_loop::ToolCallBudgetUsage) -> Value {
    json!({
        "total": usage.total,
        "read": usage.read,
        "search": usage.search,
        "fetch": usage.fetch,
        "execute": usage.execute,
        "write": usage.write,
        "destructive": usage.destructive,
        "other": usage.other,
    })
}

#[derive(Clone, Default)]
pub struct AgentLoopSessionStore {
    inner: Arc<Mutex<HashMap<String, AgentLoopSession>>>,
}

impl AgentLoopSessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, session: AgentLoopSession) {
        let key = session.session_key.clone();
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, session);
    }

    pub fn get(&self, key: &str) -> Option<AgentLoopSession> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    pub fn update(&self, key: &str, update: impl FnOnce(&mut AgentLoopSession)) {
        if let Some(session) = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(key)
        {
            update(session);
        }
    }

    pub fn remove(&self, key: &str) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key);
    }

    /// Read and clear the overflow-recovery flag for client turn outcome mapping.
    pub fn take_overflow_compaction_recovered(&self, key: &str) -> bool {
        let mut recovered = false;
        self.update(key, |session| {
            recovered = session.overflow_compaction_recovered;
            session.overflow_compaction_recovered = false;
        });
        recovered
    }
}

pub fn agent_loop_session_key(conversation_id: &str, client_session_id: &str) -> String {
    format!("{conversation_id}:{client_session_id}")
}
