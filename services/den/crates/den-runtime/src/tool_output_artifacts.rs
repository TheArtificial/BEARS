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

#[derive(Debug, Clone)]
pub struct ToolOutputArtifactRead {
    pub id: Uuid,
    pub artifact_ref: String,
    pub tool_call_id: String,
    pub tool_name: Option<String>,
    pub source: String,
    pub content: String,
    pub offset: usize,
    pub limit_chars: usize,
    pub total_chars: usize,
    pub truncated: bool,
    pub metadata: Value,
}

fn artifact_id_from_ref(artifact_ref: &str) -> Result<Uuid, DenError> {
    let raw = artifact_ref
        .strip_prefix("tool-output://")
        .unwrap_or(artifact_ref)
        .trim();
    Uuid::parse_str(raw)
        .map_err(|err| DenError::ValidationError(format!("invalid artifact_ref: {err}")))
}

pub async fn create_tool_output_artifact(
    pool: &PgPool,
    input: ToolOutputArtifactInput,
) -> Result<ToolOutputArtifactRecord, DenError> {
    let content_bytes = input
        .content_text
        .as_ref()
        .map(|text| text.len() as i64)
        .or_else(|| {
            input
                .content_json
                .as_ref()
                .map(|value| value.to_string().len() as i64)
        })
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

pub async fn read_tool_output_artifact(
    pool: &PgPool,
    bear_id: Uuid,
    session_id: &str,
    artifact_ref: &str,
    offset: usize,
    limit_chars: usize,
) -> Result<ToolOutputArtifactRead, DenError> {
    let id = artifact_id_from_ref(artifact_ref)?;
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            String,
            Option<String>,
            Option<Value>,
            Value,
        ),
    >(
        r#"
        SELECT id, tool_call_id, tool_name, source, content_text, content_json, metadata
        FROM tool_output_artifacts
        WHERE id = $1 AND bear_id = $2 AND session_id = $3
        "#,
    )
    .bind(id)
    .bind(bear_id)
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| DenError::Database(format!("read tool output artifact: {err}")))?
    .ok_or_else(|| {
        DenError::NotFound("tool output artifact not found for this session".to_string())
    })?;

    let content = row
        .4
        .or_else(|| {
            row.5.map(|value| {
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
            })
        })
        .unwrap_or_default();
    let total_chars = content.chars().count();
    let limit_chars = limit_chars.clamp(1, 24_000);
    let start = offset.min(total_chars);
    let slice = content
        .chars()
        .skip(start)
        .take(limit_chars)
        .collect::<String>();
    let truncated = start + slice.chars().count() < total_chars;
    Ok(ToolOutputArtifactRead {
        id: row.0,
        artifact_ref: format!("tool-output://{}", row.0),
        tool_call_id: row.1,
        tool_name: row.2,
        source: row.3,
        content: slice,
        offset: start,
        limit_chars,
        total_chars,
        truncated,
        metadata: row.6,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_output_artifact_ref() {
        let id = Uuid::new_v4();
        assert_eq!(
            artifact_id_from_ref(&format!("tool-output://{id}")).unwrap(),
            id
        );
        assert_eq!(artifact_id_from_ref(&id.to_string()).unwrap(), id);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn writes_and_reads_tool_output_artifact_slice(pool: PgPool) {
        let suffix = Uuid::new_v4();
        let bear_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO bears (slug, name, description) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(format!("artifact-bear-{}", suffix.simple()))
        .bind("Artifact Bear")
        .bind("artifact test bear")
        .fetch_one(&pool)
        .await
        .unwrap();
        let artifact = create_tool_output_artifact(
            &pool,
            ToolOutputArtifactInput {
                bear_id,
                user_id: None,
                session_id: "session-1".to_string(),
                conversation_id: Some("conv-1".to_string()),
                run_id: Some("run-1".to_string()),
                tool_call_id: "call-1".to_string(),
                tool_name: Some("search".to_string()),
                source: "den_hosted",
                content_text: Some("abcdef".to_string()),
                content_json: None,
                metadata: json!({ "test": true }),
            },
        )
        .await
        .unwrap();

        let read =
            read_tool_output_artifact(&pool, bear_id, "session-1", &artifact.artifact_ref, 2, 3)
                .await
                .unwrap();

        assert_eq!(read.content, "cde");
        assert_eq!(read.total_chars, 6);
        assert!(read.truncated);
        assert_eq!(read.tool_call_id, "call-1");
    }
}
