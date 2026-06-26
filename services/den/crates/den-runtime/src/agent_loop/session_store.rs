use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use den_core::profile::BearProfile;
use uuid::Uuid;

use crate::{
    agent_loop::{KeyMemoryProjectionCacheKey, StrategyProfile},
    llm::{ChatMessage, LlmApiStyle, LlmRequestTelemetry, LlmToolDefinition},
};

#[derive(Debug, Clone)]
pub struct AgentLoopSession {
    pub session_key: String,
    pub bear_id: Uuid,
    pub bear_slug: String,
    pub user_id: Option<i32>,
    pub conversation_id: String,
    pub acp_session_id: String,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<LlmToolDefinition>,
    pub model: String,
    pub bifrost_virtual_key: Option<String>,
    pub api_style: Option<LlmApiStyle>,
    pub step: u32,
    pub max_steps: u32,
    pub strategy: StrategyProfile,
    pub stream_tokens: bool,
    pub key_memory_projection_cache_key: Option<KeyMemoryProjectionCacheKey>,
    pub profile: BearProfile,
    pub overflow_retry_attempted: bool,
    pub overflow_compaction_recovered: bool,
}

impl AgentLoopSession {
    pub fn llm_telemetry(&self) -> LlmRequestTelemetry {
        LlmRequestTelemetry {
            request_id: self.request_id.clone(),
            run_id: self.run_id.clone(),
            session_id: Some(self.acp_session_id.clone()),
            conversation_id: Some(self.conversation_id.clone()),
            bear_id: Some(self.bear_id.to_string()),
            stance: Some(self.profile.as_str().to_string()),
            bifrost_virtual_key: self.bifrost_virtual_key.clone(),
        }
    }
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
            .expect("agent loop session lock")
            .insert(key, session);
    }

    pub fn get(&self, key: &str) -> Option<AgentLoopSession> {
        self.inner
            .lock()
            .expect("agent loop session lock")
            .get(key)
            .cloned()
    }

    pub fn update(&self, key: &str, update: impl FnOnce(&mut AgentLoopSession)) {
        if let Some(session) = self
            .inner
            .lock()
            .expect("agent loop session lock")
            .get_mut(key)
        {
            update(session);
        }
    }

    pub fn remove(&self, key: &str) {
        self.inner
            .lock()
            .expect("agent loop session lock")
            .remove(key);
    }

    /// Read and clear the overflow-recovery flag for ACP turn outcome mapping.
    pub fn take_overflow_compaction_recovered(&self, key: &str) -> bool {
        let mut recovered = false;
        self.update(key, |session| {
            recovered = session.overflow_compaction_recovered;
            session.overflow_compaction_recovered = false;
        });
        recovered
    }
}

pub fn agent_loop_session_key(conversation_id: &str, acp_session_id: &str) -> String {
    format!("{conversation_id}:{acp_session_id}")
}
