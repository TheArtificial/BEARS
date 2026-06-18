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

// `persistence_non_acp_integration_tests` exercises the conversation→memory-curate
// projection path, which still lives in `den`'s `reflection` subsystem; it is relocated to
// the `den` crate as a bridge test until `reflection` lands in `den-runtime` (Stage E).

#[cfg(test)]
mod persistence_idempotency_integration_tests {
    include!("persistence_idempotency_integration_tests.rs");
}
