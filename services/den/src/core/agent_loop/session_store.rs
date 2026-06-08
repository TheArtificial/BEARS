use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use uuid::Uuid;

use crate::core::{
    agent_loop::StrategyProfile,
    llm::{ChatMessage, LlmToolDefinition},
};

#[derive(Debug, Clone)]
pub struct AgentLoopSession {
    pub session_key: String,
    pub bear_id: Uuid,
    pub conversation_id: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<LlmToolDefinition>,
    pub model: String,
    pub step: u32,
    pub max_steps: u32,
    pub strategy: StrategyProfile,
    pub stream_tokens: bool,
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
        self.inner.lock().expect("agent loop session lock").insert(key, session);
    }

    pub fn get(&self, key: &str) -> Option<AgentLoopSession> {
        self.inner
            .lock()
            .expect("agent loop session lock")
            .get(key)
            .cloned()
    }

    pub fn update(&self, key: &str, update: impl FnOnce(&mut AgentLoopSession)) {
        if let Some(session) = self.inner.lock().expect("agent loop session lock").get_mut(key) {
            update(session);
        }
    }

    pub fn remove(&self, key: &str) {
        self.inner.lock().expect("agent loop session lock").remove(key);
    }
}

pub fn agent_loop_session_key(conversation_id: &str, acp_session_id: &str) -> String {
    format!("{conversation_id}:{acp_session_id}")
}
