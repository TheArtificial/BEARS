//! Conversation metadata tool (`conversation_set_title`).
//!
//! Argument parsing, title normalization, and the "conversation not saved yet"
//! guards are pure and owned here; persistence flows through the
//! [`ConversationTitleOps`] seam (native Bear-conversation title store).

use serde_json::{json, Value};
use uuid::Uuid;

use crate::DenError;

use crate::tools::{
    arguments::SetConversationTitleArguments, context::DenToolInvocationContext,
    support::clean_optional,
};

// Native async fn in trait: workspace-internal, consumed via generic bounds /
// concrete impls only (never `dyn`), so Send flows through monomorphization.
#[allow(async_fn_in_trait)]
pub trait ConversationTitleOps: Send + Sync {
    /// Set the title on the Bear conversation; returns synced client-session count.
    async fn set_title(
        &self,
        bear_id: Uuid,
        conversation_id: &str,
        title: &str,
    ) -> Result<u64, DenError>;
}

pub async fn set_conversation_title(
    ops: &impl ConversationTitleOps,
    context: &DenToolInvocationContext,
    arguments: Value,
) -> Result<Value, DenError> {
    let args: SetConversationTitleArguments = serde_json::from_value(arguments)?;
    let title = args.title.trim().chars().take(120).collect::<String>();
    if title.is_empty() {
        return Err(DenError::ValidationError(
            "conversation title cannot be empty".to_string(),
        ));
    }
    let conversation_id = clean_optional(&context.conversation_id).ok_or_else(|| {
        DenError::ValidationError(
            "current conversation is not saved yet; send a message before setting its title"
                .to_string(),
        )
    })?;
    if conversation_id == "default" || conversation_id.starts_with("new-") {
        return Err(DenError::ValidationError(
            "current conversation is not saved yet; send a message before setting its title"
                .to_string(),
        ));
    }
    let synced_acp_sessions = ops
        .set_title(context.bear_id, &conversation_id, &title)
        .await?;
    Ok(json!({
        "ok": true,
        "conversation_id": conversation_id,
        "title": title,
        "synced_acp_sessions": synced_acp_sessions,
        "content": format!("Conversation title set to {title:?}."),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::arguments::DenToolChannelContext;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    #[derive(Default)]
    struct RecordingTitleOps {
        call: Mutex<Option<(Uuid, String, String)>>,
    }

    impl ConversationTitleOps for RecordingTitleOps {
        async fn set_title(
            &self,
            bear_id: Uuid,
            conversation_id: &str,
            title: &str,
        ) -> Result<u64, DenError> {
            *self.call.lock().expect("recording title op mutex") =
                Some((bear_id, conversation_id.to_string(), title.to_string()));
            Ok(2)
        }
    }

    fn test_context(conversation_id: &str) -> DenToolInvocationContext {
        DenToolInvocationContext {
            bear_id: Uuid::nil(),
            bear_slug: "test-bear".to_string(),
            binding_id: "binding-1".to_string(),
            profile: None,
            user_id: 1,
            username: None,
            membership_role: None,
            conversation_id: conversation_id.to_string(),
            session_id: "session-1".to_string(),
            work_run_id: None,
            client_session_id: Some("client-session-1".to_string()),
            conversation_selection: None,
            runtime_target: None,
            workspace_roots: Vec::new(),
            session_policy: None,
            activity: None,
            runtime: None,
            context_budget: None,
            projected_memory: None,
            recalled_memory: None,
            request_id: None,
            channel: DenToolChannelContext::default(),
        }
    }

    fn block_on_ready<F: Future>(future: F) -> F::Output {
        // ponytail: single-poll test executor; if this future ever performs real
        // async I/O, replace with the crate's runtime test harness.
        unsafe fn clone(_: *const ()) -> RawWaker {
            raw_waker()
        }
        unsafe fn wake(_: *const ()) {}
        unsafe fn wake_by_ref(_: *const ()) {}
        unsafe fn drop(_: *const ()) {}
        fn raw_waker() -> RawWaker {
            RawWaker::new(
                std::ptr::null(),
                &RawWakerVTable::new(clone, wake, wake_by_ref, drop),
            )
        }

        let waker = unsafe { Waker::from_raw(raw_waker()) };
        let mut cx = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        match Pin::new(&mut future).poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("test future unexpectedly pending"),
        }
    }

    #[test]
    fn set_conversation_title_uses_saved_conversation_and_reports_sync() {
        let ops = RecordingTitleOps::default();
        let context = test_context("conv-live-123");

        let result = block_on_ready(set_conversation_title(
            &ops,
            &context,
            json!({ "title": "  Requested ACP title  " }),
        ))
        .expect("set title result");

        assert_eq!(result["ok"], true);
        assert_eq!(result["conversation_id"], "conv-live-123");
        assert_eq!(result["title"], "Requested ACP title");
        assert_eq!(result["synced_acp_sessions"], 2);
        assert_eq!(
            *ops.call.lock().expect("recorded title call"),
            Some((
                Uuid::nil(),
                "conv-live-123".to_string(),
                "Requested ACP title".to_string(),
            ))
        );
    }

    #[test]
    fn set_conversation_title_rejects_unsaved_conversation_fallbacks() {
        for conversation_id in ["", "default", "new-local"] {
            let ops = RecordingTitleOps::default();
            let context = test_context(conversation_id);

            let error = block_on_ready(set_conversation_title(
                &ops,
                &context,
                json!({ "title": "Requested ACP title" }),
            ))
            .expect_err("unsaved conversations should be rejected");

            assert!(
                error
                    .to_string()
                    .contains("current conversation is not saved yet"),
                "{error:?}"
            );
            assert_eq!(*ops.call.lock().expect("recorded title call"), None);
        }
    }
}
