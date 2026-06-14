//! Derived recall index client (ADR-0038): a minimal Qdrant REST client for readiness
//! checks and collection bootstrap.
//!
//! Recall is **optional and derived** — the canonical store is SQLite. When `QDRANT_URL`
//! is unset or Qdrant is unreachable, callers degrade to keyword fallback and must never
//! fail a turn. This client therefore exposes small, idempotent operations (`readyz`,
//! `collection_exists`, `ensure_collection`) and leaves graceful-degradation policy to the
//! caller.

use std::time::Duration;

use den_core::{config::Config, DenError};
use serde_json::json;

/// Minimal Qdrant REST client scoped to the active embedding standard's recall collection.
#[derive(Clone)]
pub struct QdrantRecall {
    http: reqwest::Client,
    base_url: String,
    collection: String,
    dimensions: u32,
}

impl QdrantRecall {
    /// Build a client from config, or `None` when recall is disabled (`QDRANT_URL` empty).
    pub fn from_config(config: &Config) -> Option<Self> {
        let base_url = config
            .qdrant_url
            .as_ref()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .connect_timeout(Duration::from_secs(4))
            .build()
            .ok()?;
        Some(Self {
            http,
            base_url,
            collection: collection_name(&config.embedding_standard),
            dimensions: config.embedding_dimensions,
        })
    }

    /// The recall collection name, e.g. `den_recall_bears-embed-v1`.
    pub fn collection_name(&self) -> &str {
        &self.collection
    }

    /// The configured Qdrant base URL (no trailing slash).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `GET /readyz` — `true` when Qdrant reports it is ready to serve.
    pub async fn readyz(&self) -> Result<bool, DenError> {
        let url = format!("{}/readyz", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| DenError::System(format!("qdrant readyz ({url}): {e}")))?;
        Ok(resp.status().is_success())
    }

    /// `true` when the recall collection already exists.
    pub async fn collection_exists(&self) -> Result<bool, DenError> {
        let url = format!("{}/collections/{}", self.base_url, self.collection);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| DenError::System(format!("qdrant collection check ({url}): {e}")))?;
        Ok(resp.status().is_success())
    }

    /// Create the recall collection if missing (idempotent). Returns `true` if it was created.
    pub async fn ensure_collection(&self) -> Result<bool, DenError> {
        if self.collection_exists().await? {
            return Ok(false);
        }
        let url = format!("{}/collections/{}", self.base_url, self.collection);
        let body = json!({
            "vectors": { "size": self.dimensions, "distance": "Cosine" }
        });
        let resp = self
            .http
            .put(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| DenError::System(format!("qdrant create collection ({url}): {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(DenError::System(format!(
                "qdrant create collection HTTP {status} ({url}): {text}"
            )));
        }
        Ok(true)
    }
}

/// Recall collection name for an embedding standard, e.g. `den_recall_bears-embed-v1`.
pub fn collection_name(embedding_standard: &str) -> String {
    format!("den_recall_{embedding_standard}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_none_when_recall_disabled() {
        let cfg = Config::test_stub();
        assert!(cfg.qdrant_url.is_none());
        assert!(QdrantRecall::from_config(&cfg).is_none());
    }

    #[test]
    fn from_config_some_with_derived_collection_name() {
        let mut cfg = Config::test_stub();
        cfg.qdrant_url = Some("http://bears-qdrant:6333/".to_string());
        cfg.embedding_standard = "bears-embed-v1".into();
        cfg.embedding_dimensions = 1536;

        let recall = QdrantRecall::from_config(&cfg).expect("recall client");
        assert_eq!(recall.base_url(), "http://bears-qdrant:6333");
        assert_eq!(recall.collection_name(), "den_recall_bears-embed-v1");
        assert_eq!(recall.dimensions, 1536);
    }

    #[test]
    fn collection_name_tracks_standard() {
        assert_eq!(collection_name("bears-embed-v2"), "den_recall_bears-embed-v2");
    }
}
