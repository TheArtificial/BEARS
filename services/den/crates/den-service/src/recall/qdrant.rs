//! Minimal Qdrant REST client for the derived recall index (ADR-0038): readiness,
//! collection bootstrap, and point upsert/delete/count.
//!
//! Recall is **optional and derived** — the canonical store is SQLite. When `QDRANT_URL`
//! is unset or Qdrant is unreachable, callers degrade to keyword fallback and must never
//! fail a turn.

use std::time::Duration;

use den_core::{config::Config, DenError};
use serde::Serialize;
use serde_json::{json, Value};

/// A single Qdrant point: a deterministic id, its vector, and a filterable payload.
#[derive(Debug, Clone, Serialize)]
pub struct QdrantPoint {
    pub id: String,
    pub vector: Vec<f32>,
    pub payload: Value,
}

/// A scored search result: the point id, its similarity score, and the stored payload.
#[derive(Debug, Clone)]
pub struct RecallHit {
    pub id: String,
    pub score: f32,
    pub payload: Value,
}

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

    /// Upsert points into the recall collection (waits for the write to be applied).
    pub async fn upsert_points(&self, points: &[QdrantPoint]) -> Result<(), DenError> {
        if points.is_empty() {
            return Ok(());
        }
        let url = format!(
            "{}/collections/{}/points?wait=true",
            self.base_url, self.collection
        );
        let body = json!({ "points": points });
        let resp = self
            .http
            .put(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| DenError::System(format!("qdrant upsert ({url}): {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(DenError::System(format!(
                "qdrant upsert HTTP {status} ({url}): {text}"
            )));
        }
        Ok(())
    }

    /// Delete points by id (waits for the write to be applied).
    pub async fn delete_points(&self, point_ids: &[String]) -> Result<(), DenError> {
        if point_ids.is_empty() {
            return Ok(());
        }
        let url = format!(
            "{}/collections/{}/points/delete?wait=true",
            self.base_url, self.collection
        );
        let body = json!({ "points": point_ids });
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| DenError::System(format!("qdrant delete ({url}): {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(DenError::System(format!(
                "qdrant delete HTTP {status} ({url}): {text}"
            )));
        }
        Ok(())
    }

    /// Vector search with an optional payload filter (ADR-0038 Phase 2 recall query).
    ///
    /// Returns the top `limit` hits (id, score, payload) for the given query vector. A `null`
    /// filter searches the whole collection; callers always scope by `bear_id`.
    pub async fn search(
        &self,
        vector: &[f32],
        filter: Value,
        limit: u64,
    ) -> Result<Vec<RecallHit>, DenError> {
        let url = format!(
            "{}/collections/{}/points/search",
            self.base_url, self.collection
        );
        let mut body = json!({
            "vector": vector,
            "limit": limit,
            "with_payload": true,
        });
        if !filter.is_null() {
            body["filter"] = filter;
        }
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| DenError::System(format!("qdrant search ({url}): {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| DenError::System(format!("qdrant search body ({url}): {e}")))?;
        if !status.is_success() {
            return Err(DenError::System(format!(
                "qdrant search HTTP {status} ({url}): {text}"
            )));
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|e| DenError::System(format!("qdrant search parse ({url}): {e}")))?;
        let result = value
            .get("result")
            .and_then(Value::as_array)
            .ok_or_else(|| DenError::System(format!("qdrant search missing result array: {text}")))?;
        let hits = result
            .iter()
            .filter_map(|item| {
                let id = match item.get("id") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Number(n)) => n.to_string(),
                    _ => return None,
                };
                let score = item.get("score").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                let payload = item.get("payload").cloned().unwrap_or(Value::Null);
                Some(RecallHit { id, score, payload })
            })
            .collect();
        Ok(hits)
    }

    /// Count points matching a Qdrant filter (used for inspection/diagnostics + tests).
    pub async fn count_with_filter(&self, filter: Value) -> Result<u64, DenError> {
        let url = format!(
            "{}/collections/{}/points/count",
            self.base_url, self.collection
        );
        let body = json!({ "filter": filter, "exact": true });
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| DenError::System(format!("qdrant count ({url}): {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| DenError::System(format!("qdrant count body ({url}): {e}")))?;
        if !status.is_success() {
            return Err(DenError::System(format!(
                "qdrant count HTTP {status} ({url}): {text}"
            )));
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|e| DenError::System(format!("qdrant count parse ({url}): {e}")))?;
        let count = value
            .get("result")
            .and_then(|r| r.get("count"))
            .and_then(Value::as_u64)
            .ok_or_else(|| DenError::System(format!("qdrant count missing result.count: {text}")))?;
        Ok(count)
    }
}

/// Recall collection name for an embedding standard, e.g. `den_recall_bears-embed-v1`.
pub fn collection_name(embedding_standard: &str) -> String {
    format!("den_recall_{embedding_standard}")
}

#[cfg(test)]
mod tests;

