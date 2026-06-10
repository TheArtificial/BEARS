use crate::{
    core::{
        agent_loop::{context::repair_tool_call_message_chain, AgentLoopSession},
        llm::LlmClient,
        native_runtime::openai_byte_stream_to_event_stream,
        runtime_contracts::RuntimeEventStream,
    },
    errors::CustomError,
};

pub async fn run_agent_step_stream(
    llm: &LlmClient,
    session: &AgentLoopSession,
) -> Result<RuntimeEventStream, CustomError> {
    let messages = repair_tool_call_message_chain(session.messages.clone());
    let request = crate::core::llm::ChatCompletionRequest {
        model: session.model.clone(),
        messages,
        tools: session.tools.clone(),
        stream: true,
        tool_choice: None,
        temperature: None,
        max_tokens: None,
    };
    let byte_stream = llm.chat_completions_byte_stream(&request).await?;
    Ok(openai_byte_stream_to_event_stream(byte_stream))
}
