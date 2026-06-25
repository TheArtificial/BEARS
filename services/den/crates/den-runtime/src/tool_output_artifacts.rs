use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use den_core::DenError;

#[derive(Debug, Clone)]
pub struct ToolOutputArtifactInput {
    pub bear_id: Uuid,
    pub user_id: Option<i32>,
    pub session_id: String,
    pub conversation_id: Option<String>,
    pub run_id: Option<String>,
    pub tool_call_id: String,
    pub tool_name: Option<String>,
    pub source: &'static str,
    pub content_text: Option<String>,
    pub content_json: Option<Value>,
    pub metadata: Value,
}

#[derive(Debug, Clone)]
pub struct ToolOutputArtifactRecord {
    pub id: Uuid,
    pub artifact_ref: String,
}

pub async fn create_tool_output_artifact(
    pool: &PgPool,
    input: ToolOutputArtifactInput,
) -> Result<ToolOutputArtifactRecord, DenError> {
    let content_bytes = input
        .content_text
        .as_ref()
        .map(|text| text.len() as i64)
        .or_else(|| input.content_json.as_ref().map(|value| value.to_string().len() as i64))
        .unwrap_or(0);
    let metadata = if input.metadata.is_object() {
        input.metadata
    } else {
        json!({ "value": input.metadata })
    };
    let id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO tool_output_artifacts (
            bear_id, user_id, session_id, conversation_id, run_id, tool_call_id,
            tool_name, source, content_text, content_json, metadata, content_bytes
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        RETURNING id
        "#,
    )
    .bind(input.bear_id)
    .bind(input.user_id)
    .bind(&input.session_id)
    .bind(&input.conversation_id)
    .bind(&input.run_id)
    .bind(&input.tool_call_id)
    .bind(&input.tool_name)
    .bind(input.source)
    .bind(&input.content_text)
    .bind(&input.content_json)
    .bind(&metadata)
    .bind(content_bytes)
    .fetch_one(pool)
    .await
    .map_err(|err| DenError::Database(format!("insert tool output artifact: {err}")))?;
    Ok(ToolOutputArtifactRecord {
        id,
        artifact_ref: format!("tool-output://{id}"),
    })
}
