use sqlx::PgPool;

use crate::{
    core::{
        acp_sessions,
        bears::{db as bears_db, model::BearAgentRole, Bear},
        conversation_persistence,
        letta::{load_agent_conversations, LettaClient},
        runtime_contracts::{
            AcpConversationRuntime, EnsureConversationRequest, EnsureConversationResult,
            RoleProfileRegistry, RoleRuntimeBinding, RuntimeConversationBackend,
            RuntimeConversationRef, RuntimeHistoryRecord,
        },
    },
    errors::CustomError,
};

pub fn acp_missing_pair_binding_message(bear_slug: &str) -> String {
    format!(
        "ACP requires this Bear to have a provisioned `pair` role runtime binding, but none is recorded for bear `{bear_slug}`. Ask an operator to open Admin → Bears → this Bear and click `Provision missing role agents`, then retry."
    )
}

pub async fn require_pair_runtime_binding(
    pool: &PgPool,
    letta: &LettaClient,
    bear: &Bear,
) -> Result<RoleRuntimeBinding, CustomError> {
    if !letta.is_enabled() {
        return Err(CustomError::System(
            "Letta is not configured (set LETTA_BASE_URL); ACP pair role cannot run.".to_string(),
        ));
    }
    bears_db::role_runtime_binding_id(pool, bear.id, BearAgentRole::Pair)
        .await?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|binding_id| RoleRuntimeBinding {
            binding_id,
            compatibility_backend: Some("runtime:letta".to_string()),
        })
        .ok_or_else(|| CustomError::ValidationError(acp_missing_pair_binding_message(&bear.slug)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpConversationSelectionSource {
    Explicit,
    Resolved,
    Stored,
    Generated,
}

impl AcpConversationSelectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Resolved => "resolved",
            Self::Stored => "stored",
            Self::Generated => "generated",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpConversationResolution {
    pub session_selection: String,
    pub resolved_conversation: Option<RuntimeConversationRef>,
    pub upstream_target: String,
    pub should_materialize_runtime_conversation: bool,
    pub selection_source: AcpConversationSelectionSource,
    pub history_target: Option<RuntimeConversationRef>,
    pub archive_target: Option<RuntimeConversationRef>,
    pub requires_belongs_to_bear_check: bool,
}

impl AcpConversationResolution {
    pub fn from_selection(
        session_selection: String,
        selection_source: AcpConversationSelectionSource,
        binding: &RoleRuntimeBinding,
        existing_session: Option<&acp_sessions::AcpSessionRow>,
    ) -> Self {
        let resolved_conversation = if is_acp_history_target(&session_selection) {
            Some(RuntimeConversationRef {
                id: session_selection.clone(),
            })
        } else if existing_session.is_some_and(|s| s.conversation_id.trim() == session_selection) {
            normalized_durable_acp_conversation_id(
                existing_session.and_then(|s| s.resolved_conversation_id.as_deref()),
            )
            .map(|id| RuntimeConversationRef { id })
        } else {
            None
        };
        let should_materialize_runtime_conversation = session_selection.starts_with("new-")
            && selection_source == AcpConversationSelectionSource::Explicit;
        let upstream_target = if should_materialize_runtime_conversation {
            binding.binding_id.clone()
        } else {
            session_selection.clone()
        };
        let history_target = resolved_conversation
            .as_ref()
            .filter(|c| is_acp_history_target(&c.id))
            .cloned();
        let archive_target = resolved_conversation
            .as_ref()
            .filter(|c| is_acp_archive_target(&c.id))
            .cloned();
        let requires_belongs_to_bear_check = selection_source
            == AcpConversationSelectionSource::Explicit
            && session_selection.starts_with("conv-");

        Self {
            session_selection,
            resolved_conversation,
            upstream_target,
            should_materialize_runtime_conversation,
            selection_source,
            history_target,
            archive_target,
            requires_belongs_to_bear_check,
        }
    }
}

pub fn is_valid_pending_acp_conversation_id(conversation_id: &str) -> bool {
    conversation_id.starts_with("new-")
        && conversation_id.len() <= 42
        && normalize_acp_conversation_id(Some(conversation_id)).is_ok()
}

pub fn is_acp_history_target(conversation_id: &str) -> bool {
    conversation_id == "default" || conversation_id.starts_with("conv-")
}

pub fn is_acp_archive_target(conversation_id: &str) -> bool {
    conversation_id.starts_with("conv-")
}

pub fn normalized_durable_acp_conversation_id(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| is_acp_history_target(s))
        .map(str::to_string)
}

pub fn normalize_acp_conversation_id(raw: Option<&str>) -> Result<String, CustomError> {
    let s = raw.unwrap_or("default").trim();
    if s.is_empty() {
        return Ok("default".to_string());
    }
    let ok = s == "default"
        || (s.starts_with("conv-") && s.len() >= 8)
        || (s.starts_with("new-") && s.len() >= 8)
        || s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(s.to_string())
    } else {
        Err(CustomError::ValidationError(format!(
            "invalid conversation_id (expected 'default', a runtime conv- id, or a pending new- id): {s}"
        )))
    }
}

pub fn resolve_acp_prompt_conversation(
    requested_raw: Option<&str>,
    existing_session: Option<&acp_sessions::AcpSessionRow>,
    binding: &RoleRuntimeBinding,
    generated_pending_id: String,
) -> Result<AcpConversationResolution, CustomError> {
    let requested = requested_raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| normalize_acp_conversation_id(Some(s)))
        .transpose()?
        .filter(|id| id != "default");

    let (session_selection, source) = if let Some(id) = requested {
        (id, AcpConversationSelectionSource::Explicit)
    } else if let Some(id) = existing_session
        .and_then(|s| normalized_durable_acp_conversation_id(s.resolved_conversation_id.as_deref()))
    {
        (id, AcpConversationSelectionSource::Resolved)
    } else if let Some(id) = existing_session
        .map(|s| s.conversation_id.trim())
        .filter(|s| !s.is_empty())
        .filter(|s| s.starts_with("conv-") || is_valid_pending_acp_conversation_id(s))
        .map(str::to_string)
    {
        (id, AcpConversationSelectionSource::Stored)
    } else {
        (
            generated_pending_id,
            AcpConversationSelectionSource::Generated,
        )
    };

    Ok(AcpConversationResolution::from_selection(
        session_selection,
        source,
        binding,
        existing_session,
    ))
}

pub async fn ensure_acp_session_conversation_with_backend<B: RuntimeConversationBackend>(
    backend: &B,
    request: EnsureConversationRequest,
    existing_session: Option<&acp_sessions::AcpSessionRow>,
    generated_pending_id: String,
) -> Result<(AcpConversationResolution, EnsureConversationResult), CustomError> {
    let mut resolution = resolve_acp_prompt_conversation(
        request.requested_selection.as_deref(),
        existing_session,
        &request.binding,
        generated_pending_id,
    )?;
    let mut created = false;
    if resolution.should_materialize_runtime_conversation
        && resolution.resolved_conversation.is_none()
    {
        let conversation = backend.create_conversation(&request.binding).await?;
        resolution.resolved_conversation = Some(conversation.clone());
        resolution.history_target = Some(conversation.clone());
        resolution.archive_target = Some(conversation.clone());
        resolution.upstream_target = conversation.id.clone();
        created = true;
    }
    let conversation = resolution
        .resolved_conversation
        .clone()
        .unwrap_or_else(|| RuntimeConversationRef {
            id: resolution.upstream_target.clone(),
        });
    Ok((
        resolution,
        EnsureConversationResult {
            conversation,
            created,
        },
    ))
}

pub async fn ensure_acp_session_conversation(
    letta: &LettaClient,
    request: EnsureConversationRequest,
    existing_session: Option<&acp_sessions::AcpSessionRow>,
    generated_pending_id: String,
) -> Result<(AcpConversationResolution, EnsureConversationResult), CustomError> {
    let backend = LettaRuntimeConversationBackend { letta };
    ensure_acp_session_conversation_with_backend(
        &backend,
        request,
        existing_session,
        generated_pending_id,
    )
    .await
}

pub fn canonical_acp_conversation_id_for_session(
    existing_session: Option<&acp_sessions::AcpSessionRow>,
    conversation_resolution: &AcpConversationResolution,
) -> String {
    existing_session
        .map(|session| session.conversation_id.trim())
        .filter(|id| !id.is_empty())
        .filter(|id| *id == "default" || id.starts_with("conv-") || id.starts_with("new-"))
        .map(str::to_string)
        .or_else(|| {
            let id = conversation_resolution.session_selection.trim();
            (!id.is_empty() && (id == "default" || id.starts_with("conv-") || id.starts_with("new-")))
                .then(|| id.to_string())
        })
        .unwrap_or_else(|| conversation_resolution.session_selection.clone())
}

pub async fn verify_acp_conversation_belongs_to_binding_with_backend<B: RuntimeConversationBackend>(
    backend: &B,
    binding: &RoleRuntimeBinding,
    conversation_id: &str,
) -> Result<(), CustomError> {
    if conversation_id == "default" || conversation_id.starts_with("new-") {
        return Ok(());
    }
    if !conversation_id.starts_with("conv-") {
        return Err(CustomError::ValidationError(format!(
            "invalid conversation_id: {conversation_id}"
        )));
    }
    let binding_id = binding.binding_id.trim();
    if binding_id.is_empty() {
        return Err(CustomError::ValidationError(
            "this bear role is not linked to a runtime runtime binding".to_string(),
        ));
    }
    backend
        .verify_conversation_belongs_to_binding(binding, conversation_id)
        .await
}

pub async fn verify_acp_conversation_belongs_to_binding(
    letta: &LettaClient,
    binding: &RoleRuntimeBinding,
    conversation_id: &str,
) -> Result<(), CustomError> {
    let backend = LettaRuntimeConversationBackend { letta };
    verify_acp_conversation_belongs_to_binding_with_backend(&backend, binding, conversation_id)
        .await
}

pub async fn verify_acp_conversation_access(
    pool: &PgPool,
    bear_id: uuid::Uuid,
    letta: &LettaClient,
    binding: &RoleRuntimeBinding,
    conversation_id: &str,
) -> Result<(), CustomError> {
    if conversation_id == "default" || conversation_id.starts_with("new-") {
        return Ok(());
    }
    if !conversation_id.starts_with("conv-") {
        return Err(CustomError::ValidationError(format!(
            "invalid conversation_id: {conversation_id}"
        )));
    }
    if conversation_persistence::get_conversation_for_external_id(pool, bear_id, conversation_id)
        .await?
        .is_some()
    {
        return Ok(());
    }
    verify_acp_conversation_belongs_to_binding(letta, binding, conversation_id).await
}

fn letta_conversation_id_from_create_response(value: &serde_json::Value) -> Option<String> {
    value
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| s.starts_with("conv-"))
        .map(str::to_string)
}

pub struct LettaRuntimeConversationBackend<'a> {
    pub letta: &'a LettaClient,
}

impl<'a> LettaRuntimeConversationBackend<'a> {
    pub fn new(letta: &'a LettaClient) -> Self {
        Self { letta }
    }
}

#[allow(async_fn_in_trait)]
impl RuntimeConversationBackend for LettaRuntimeConversationBackend<'_> {
    async fn create_conversation(
        &self,
        binding: &RoleRuntimeBinding,
    ) -> Result<RuntimeConversationRef, CustomError> {
        let created_response = self
            .letta
            .create_conversation_for_agent(&binding.binding_id)
            .await?;
        let conv_id = letta_conversation_id_from_create_response(&created_response).ok_or_else(|| {
            CustomError::System(format!(
                "Letta create conversation response did not contain a conv-* id: {created_response}"
            ))
        })?;
        Ok(RuntimeConversationRef { id: conv_id })
    }

    async fn verify_conversation_belongs_to_binding(
        &self,
        binding: &RoleRuntimeBinding,
        conversation_id: &str,
    ) -> Result<(), CustomError> {
        if !self.letta.is_enabled() {
            return Err(CustomError::System(
                "Letta is not configured (set LETTA_BASE_URL)".to_string(),
            ));
        }
        let snap = load_agent_conversations(self.letta, binding.binding_id.trim()).await;
        let found = snap.all.iter().any(|row| row.id == conversation_id);
        if found {
            Ok(())
        } else {
            Err(CustomError::Authorization(
                "conversation not found for this bear".to_string(),
            ))
        }
    }

    async fn load_history(
        &self,
        binding: &RoleRuntimeBinding,
        conversation: &RuntimeConversationRef,
    ) -> Result<crate::core::runtime_contracts::RuntimeHistoryPage, CustomError> {
        let binding_for_conv = if conversation.id == "default" {
            Some(binding.binding_id.as_str())
        } else {
            None
        };
        let body = self
            .letta
            .list_conversation_messages(&conversation.id, binding_for_conv, 100, None, true)
            .await?;
        let messages = if let Some(array) = body.as_array() {
            array.clone()
        } else if let Some(array) = body.get("messages").and_then(serde_json::Value::as_array) {
            array.clone()
        } else if let Some(array) = body.get("data").and_then(serde_json::Value::as_array) {
            array.clone()
        } else if let Some(array) = body.get("items").and_then(serde_json::Value::as_array) {
            array.clone()
        } else {
            Vec::new()
        };
        let mut history = Vec::new();
        for raw_message in messages {
            let inner = raw_message.get("contents").unwrap_or(&raw_message);
            let role = inner
                .get("role")
                .and_then(serde_json::Value::as_str)
                .or_else(|| raw_message.get("role").and_then(serde_json::Value::as_str))
                .or_else(|| {
                    inner
                        .get("message_type")
                        .and_then(serde_json::Value::as_str)
                        .map(|message_type| match message_type {
                            "user_message" => "user",
                            "assistant_message" => "assistant",
                            _ => "system",
                        })
                })
                .unwrap_or("system")
                .to_string();
            let content = inner
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    inner
                        .get("content")
                        .and_then(|v| v.get("text"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            let message_id = raw_message
                .get("id")
                .and_then(serde_json::Value::as_str)
                .or_else(|| inner.get("id").and_then(serde_json::Value::as_str))
                .map(str::to_string);
            let created_at = raw_message
                .get("date")
                .or_else(|| raw_message.get("created_at"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            history.push(RuntimeHistoryRecord {
                message_id,
                role,
                content,
                created_at,
            });
        }
        Ok(crate::core::runtime_contracts::RuntimeHistoryPage {
            records: history,
            raw_payload: Some(body),
        })
    }
}

pub async fn load_acp_history_with_backend<B: RuntimeConversationBackend>(
    backend: &B,
    binding: &RoleRuntimeBinding,
    conversation: &RuntimeConversationRef,
) -> Result<crate::core::runtime_contracts::RuntimeHistoryPage, CustomError> {
    backend.load_history(binding, conversation).await
}

pub struct LettaAcpConversationRuntime<'a> {
    pub letta: &'a LettaClient,
}

impl RoleProfileRegistry for LettaAcpConversationRuntime<'_> {
    async fn resolve_compatibility_binding(
        &self,
        _bear_id: uuid::Uuid,
        _role: &str,
    ) -> Result<Option<RoleRuntimeBinding>, CustomError> {
        Ok(None)
    }
}

impl AcpConversationRuntime for LettaAcpConversationRuntime<'_> {
    async fn ensure_session_conversation(
        &self,
        request: EnsureConversationRequest,
    ) -> Result<EnsureConversationResult, CustomError> {
        let requested_selection = request.requested_selection.clone();
        let (resolution, result) = ensure_acp_session_conversation(
            self.letta,
            request,
            None,
            "new-acp-runtime-placeholder".to_string(),
        )
        .await?;
        let _ = resolution;
        if requested_selection.is_none() {
            Ok(EnsureConversationResult {
                conversation: RuntimeConversationRef {
                    id: "default".to_string(),
                },
                created: false,
            })
        } else {
            Ok(result)
        }
    }

    async fn load_history(
        &self,
        binding: &RoleRuntimeBinding,
        conversation: &RuntimeConversationRef,
    ) -> Result<crate::core::runtime_contracts::RuntimeHistoryPage, CustomError> {
        load_acp_history_with_backend(
            &LettaRuntimeConversationBackend { letta: self.letta },
            binding,
            conversation,
        )
        .await
    }
}
