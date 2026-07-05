use den_core::DenError;
use den_llm::ModelOption;
use serde_json::Value;
use sqlx::{types::Json, FromRow, PgPool};
use uuid::Uuid;

use crate::{bears, conversation::persistence};

#[derive(Debug, Clone, FromRow)]
pub struct ModelSelectionOptionRow {
    pub handle: String,
    pub display_name: String,
    pub metadata_json: Json<Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConversationModelSelectionView {
    pub selection_mode: String,
    pub requested_model: Option<String>,
    pub selected_model: Option<String>,
    pub effective_model: String,
    pub source: String,
    pub model_options: Vec<ModelOption>,
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

fn simplify_model_option_label_for_acp(label: &str) -> String {
    label
        .trim()
        .strip_suffix(" — metadata unknown")
        .unwrap_or(label.trim())
        .split(" (")
        .next()
        .unwrap_or(label.trim())
        .trim()
        .to_string()
}

pub fn simplify_model_option_for_acp(option: &ModelOption) -> ModelOption {
    ModelOption {
        handle: option.handle.clone(),
        label: simplify_model_option_label_for_acp(&option.label),
        context_window: None,
        max_output_tokens: None,
    }
}

pub async fn list_selectable_model_options_for_acp(
    pool: &PgPool,
) -> Result<Vec<ModelOption>, DenError> {
    Ok(list_selectable_model_options(pool)
        .await?
        .iter()
        .map(simplify_model_option_for_acp)
        .collect())
}

fn resolve_selectable_model_handle(options: &[ModelOption], raw: &str) -> Result<String, DenError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(DenError::ValidationError(
            "model is required for explicit selection".to_string(),
        ));
    }
    if den_llm::model_registry::is_routing_wildcard_model_handle(trimmed) {
        return Err(DenError::ValidationError(
            "routing wildcards are not selectable models".to_string(),
        ));
    }
    let resolved = den_llm::model_registry::resolve_model_handle(trimmed);
    let available = options.iter().any(|option| {
        let handle = option.handle.as_str();
        handle == trimmed
            || resolved == Some(handle)
            || den_llm::model_registry::resolve_model_handle(handle) == resolved
    });
    if !available {
        return Err(DenError::ValidationError(
            "model must be configured as a selectable Den model".to_string(),
        ));
    }
    Ok(resolved.unwrap_or(trimmed).to_string())
}

pub async fn apply_conversation_model_selection(
    pool: &PgPool,
    conversation_id: Uuid,
    selection_mode: &str,
    requested_model: Option<&str>,
    explicit_reason: &str,
    auto_reason: &str,
) -> Result<crate::conversation::persistence::ConversationModelState, DenError> {
    let options = list_selectable_model_options(pool).await?;
    if options.is_empty() {
        return Err(DenError::System(
            "No Den model selection options are configured.".to_string(),
        ));
    }
    let selected = if selection_mode.trim() == "explicit" {
        Some(resolve_selectable_model_handle(
            &options,
            requested_model.unwrap_or(""),
        )?)
    } else {
        None
    };
    crate::conversation::persistence::set_conversation_model_state(
        pool,
        conversation_id,
        if selected.is_some() {
            "explicit"
        } else {
            "auto"
        },
        selected.as_deref(),
        selected.as_deref(),
        Some(if selected.is_some() {
            explicit_reason
        } else {
            auto_reason
        }),
    )
    .await
}

pub async fn load_conversation_model_selection_view(
    pool: &PgPool,
    bear: &bears::Bear,
    user_id: i32,
    profile: bears::BearProfile,
    default_model: &str,
    conversation_external_id: &str,
    source_client_session_id: Option<&str>,
    acp_friendly_options: bool,
) -> Result<ConversationModelSelectionView, DenError> {
    let base_model =
        bears::db::resolve_model_for_profile(pool, bear, profile, default_model).await?;
    let model_options = if acp_friendly_options {
        list_selectable_model_options_for_acp(pool).await?
    } else {
        list_selectable_model_options(pool).await?
    };
    let conversation = persistence::ensure_conversation_for_external_id(
        pool,
        bear.id,
        Some(user_id),
        conversation_external_id,
        source_client_session_id,
        None,
    )
    .await?;
    let model_state = persistence::get_conversation_model_state(pool, conversation.id).await?;
    let effective_model = persistence::resolve_conversation_selected_model(pool, conversation.id)
        .await?
        .unwrap_or(base_model);
    Ok(ConversationModelSelectionView {
        selection_mode: model_state
            .as_ref()
            .map(|row| row.selection_mode.clone())
            .unwrap_or_else(|| "auto".to_string()),
        requested_model: model_state
            .as_ref()
            .and_then(|row| row.requested_model.clone()),
        selected_model: model_state
            .as_ref()
            .and_then(|row| row.selected_model.clone()),
        source: if model_state.as_ref().map(|row| row.selection_mode.as_str()) == Some("explicit") {
            "conversation_explicit".to_string()
        } else {
            "stance_or_bear_default".to_string()
        },
        effective_model,
        model_options,
    })
}

pub async fn resolve_model_option(
    pool: &PgPool,
    handle: &str,
) -> Result<Option<ModelOption>, DenError> {
    let trimmed = handle.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if let Ok(Some(row)) = sqlx::query_as::<_, ModelSelectionOptionRow>(
        r#"
        SELECT handle, display_name, metadata_json
        FROM model_selection_options
        WHERE handle = $1
        LIMIT 1
        "#,
    )
    .bind(trimmed)
    .fetch_optional(pool)
    .await
    {
        return Ok(Some(option_from_row(row)));
    }
    Ok(den_llm::model_registry::entry_for_handle(trimmed)
        .map(|entry| entry.to_model_option_for_handle(trimmed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_model_option_for_acp_drops_context_window_details() {
        let option = ModelOption {
            handle: "openai/gpt-5".to_string(),
            label: "GPT-5 (1M ctx / 128k out)".to_string(),
            context_window: Some(1_000_000),
            max_output_tokens: Some(128_000),
        };

        let simplified = simplify_model_option_for_acp(&option);
        assert_eq!(simplified.handle, option.handle);
        assert_eq!(simplified.label, "GPT-5");
        assert_eq!(simplified.context_window, None);
        assert_eq!(simplified.max_output_tokens, None);
    }

    #[test]
    fn simplify_model_option_for_acp_drops_metadata_unknown_suffix() {
        let option = ModelOption {
            handle: "custom/model".to_string(),
            label: "Custom model — metadata unknown".to_string(),
            context_window: None,
            max_output_tokens: None,
        };

        let simplified = simplify_model_option_for_acp(&option);
        assert_eq!(simplified.label, "Custom model");
    }
}
