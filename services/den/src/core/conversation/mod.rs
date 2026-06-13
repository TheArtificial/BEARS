//! Conversation subsystem: canonical message persistence, typed write payloads,
//! event projection, and archived-conversation reads.

pub mod archived;
pub mod events;
pub mod message_types;
pub mod persistence;

#[cfg(test)]
mod events_tests {
    include!("events_tests.rs");
}

#[cfg(test)]
mod persistence_non_acp_integration_tests {
    include!("persistence_non_acp_integration_tests.rs");
}

#[cfg(test)]
mod persistence_idempotency_integration_tests {
    include!("persistence_idempotency_integration_tests.rs");
}
