use sqlx::PgPool;

use den_core::config::Config;
use den_http::errors::CustomError;
use den_runtime::{
    acp_sessions,
    bears::{model::BearProfile, Bear},
    conversation_persistence,
    native_runtime::NativeRuntimeConversationBackend,
    role_runtime_registry::DenNativeProfileRegistry,
    runtime_contracts::{
            EnsureConversationRequest, EnsureConversationResult, RoleRuntimeBinding,
            RuntimeConversationBackend, RuntimeConversationRef,
        },
};

// Pure conversation-id predicates now live in `den-runtime`; re-export them so this module's
// internal logic and the existing `core::acp::runtime::is_*` call sites stay unchanged.
pub use den_runtime::conversation_ids::{
    is_acp_archive_target, is_acp_history_target, is_native_runtime_conversation_id,
    is_valid_pending_acp_conversation_id, normalize_acp_conversation_id,
    normalized_durable_acp_conversation_id,
};

pub fn acp_missing_pair_binding_message(bear_slug: &str) -> String {
    format!(
        "ACP requires this Bear to have a provisioned `pair` profile runtime binding, but none is recorded for bear `{bear_slug}`. Ask an operator to open Admin → Bears → this Bear and click `Provision missing profiles`, then retry."
    )
}

pub async fn require_pair_runtime_binding(
    pool: &PgPool,
    config: &Config,
    bear: &Bear,
) -> Result<RoleRuntimeBinding, CustomError> {
    let registry = DenNativeProfileRegistry::new(pool, config);
    if let Some(binding) = registry.resolve_binding(bear.id, BearProfile::Pair).await? {
        if !binding.binding_id.trim().is_empty() {
            return Ok(binding);
        }
    }
    Ok(RoleRuntimeBinding {
        binding_id: format!("den-native:{}:pair", bear.id),
        compatibility_backend: Some("runtime:native".to_string()),
    })
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
            && selection_source == AcpConversationSelectionSource::Explicit
            && existing_session
                .and_then(|s| {
                    normalized_durable_acp_conversation_id(s.resolved_conversation_id.as_deref())
                })
                .is_none();
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
            && (session_selection.starts_with("conv-")
                || is_native_runtime_conversation_id(&session_selection));

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
        .filter(|s| {
            s.starts_with("conv-")
                || is_native_runtime_conversation_id(s)
                || is_valid_pending_acp_conversation_id(s)
        })
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
        resolution.upstream_target = conversation.id;
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

pub fn canonical_acp_conversation_id_for_session(
    existing_session: Option<&acp_sessions::AcpSessionRow>,
    conversation_resolution: &AcpConversationResolution,
) -> String {
    existing_session
        .map(|session| session.conversation_id.trim())
        .filter(|id| !id.is_empty())
        .filter(|id| {
            *id == "default"
                || id.starts_with("conv-")
                || is_native_runtime_conversation_id(id)
                || id.starts_with("new-")
        })
        .map(str::to_string)
        .or_else(|| {
            let id = conversation_resolution.session_selection.trim();
            (!id.is_empty()
                && (id == "default"
                    || id.starts_with("conv-")
                    || is_native_runtime_conversation_id(id)
                    || id.starts_with("new-")))
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
    if !conversation_id.starts_with("conv-") && !is_native_runtime_conversation_id(conversation_id)
    {
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
        .map_err(CustomError::from)
}

pub async fn load_acp_history_with_backend<B: RuntimeConversationBackend>(
    backend: &B,
    binding: &RoleRuntimeBinding,
    conversation: &RuntimeConversationRef,
) -> Result<den_protocol::RuntimeHistoryPage, CustomError> {
    backend.load_history(binding, conversation).await.map_err(CustomError::from)
}

/// Den-owned ACP conversation lifecycle entrypoint. Keeps session/bootstrap policy out of
/// prompt handlers while routing backend-specific work through `RuntimeConversationBackend`.
pub struct AcpConversationService<'a> {
    pool: &'a PgPool,
    backend: NativeRuntimeConversationBackend,
}

impl<'a> AcpConversationService<'a> {
    pub fn new(pool: &'a PgPool, _config: &Config) -> Self {
        Self {
            pool,
            backend: NativeRuntimeConversationBackend::with_pool(pool.clone()),
        }
    }

    pub async fn ensure_prompt_conversation(
        &self,
        request: EnsureConversationRequest,
        existing_session: Option<&acp_sessions::AcpSessionRow>,
        generated_pending_id: String,
    ) -> Result<(AcpConversationResolution, EnsureConversationResult), CustomError> {
        ensure_acp_session_conversation_with_backend(
            &self.backend,
            request,
            existing_session,
            generated_pending_id,
        )
        .await
    }

    pub async fn verify_conversation_access(
        &self,
        bear_id: uuid::Uuid,
        binding: &RoleRuntimeBinding,
        conversation_id: &str,
    ) -> Result<(), CustomError> {
        if conversation_id == "default" || conversation_id.starts_with("new-") {
            return Ok(());
        }
        if !conversation_id.starts_with("conv-")
            && !is_native_runtime_conversation_id(conversation_id)
        {
            return Err(CustomError::ValidationError(format!(
                "invalid conversation_id: {conversation_id}"
            )));
        }
        if is_native_runtime_conversation_id(conversation_id) {
            return Ok(());
        }
        if conversation_persistence::get_conversation_for_external_id(
            self.pool,
            bear_id,
            conversation_id,
        )
        .await?
        .is_some()
        {
            return Ok(());
        }
        verify_acp_conversation_belongs_to_binding_with_backend(
            &self.backend,
            binding,
            conversation_id,
        )
        .await
    }
}
