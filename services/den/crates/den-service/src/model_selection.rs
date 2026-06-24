use den_core::DenError;
use den_llm::ModelOption;
use serde_json::Value;
use sqlx::{types::Json, FromRow, PgPool};

#[derive(Debug, Clone, FromRow)]
pub struct ModelSelectionOptionRow {
    pub handle: String,
    pub display_name: String,
    pub metadata_json: Json<Value>,
}

fn metadata_u32(metadata: &Value, key: &str) -> Option<u32> {
    metadata
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn option_from_row(row: ModelSelectionOptionRow) -> ModelOption {
    if let Some(entry) = den_llm::model_registry::entry_for_handle(&row.handle) {
        return entry.to_model_option_for_handle(&row.handle);
    }
    let context_window = metadata_u32(&row.metadata_json.0, "context_window");
    let max_output_tokens = metadata_u32(&row.metadata_json.0, "max_output_tokens");
    ModelOption {
        handle: row.handle,
        label: row.display_name,
        context_window,
        max_output_tokens,
    }
}

/// Stable Den-owned model options for user-facing selectors.
///
/// Bifrost remains the execution/availability authority, but the selector list
/// should not flicker when Bifrost's live catalog temporarily shrinks or expands.
pub async fn list_selectable_model_options(pool: &PgPool) -> Result<Vec<ModelOption>, DenError> {
    let rows = match sqlx::query_as::<_, ModelSelectionOptionRow>(
        r#"
        SELECT handle, display_name, metadata_json
        FROM model_selection_options
        WHERE selectable = TRUE
        ORDER BY COALESCE(sort_order, 100000), display_name, handle
        "#,
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "could not load model_selection_options; using static Den model registry fallback"
            );
            return Ok(den_llm::model_registry::selectable_model_options());
        }
    };

    let mut options = rows.into_iter().map(option_from_row).collect::<Vec<_>>();
    if options.is_empty() {
        options = den_llm::model_registry::selectable_model_options();
    }
    Ok(options)
}
