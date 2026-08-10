use den_core::DenError;
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientSessionWorkSurfaceResolutionStatus {
    Resolved,
    Confirmed,
}

impl ClientSessionWorkSurfaceResolutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Resolved => "resolved",
            Self::Confirmed => "confirmed",
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClientSessionWorkSurfaceResolution {
    pub client_session_id: Uuid,
    pub work_surface_id: Uuid,
    pub status: String,
    pub evidence: Value,
    pub resolved_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub async fn find(
    pool: &PgPool,
    client_session_id: Uuid,
) -> Result<Option<ClientSessionWorkSurfaceResolution>, DenError> {
    sqlx::query_as!(
        ClientSessionWorkSurfaceResolution,
        r#"
        SELECT client_session_id, work_surface_id, status, evidence, resolved_at, updated_at
        FROM client_session_work_surface_resolutions
        WHERE client_session_id = $1
        "#,
        client_session_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub async fn clear_resolved(pool: &PgPool, client_session_id: Uuid) -> Result<(), DenError> {
    sqlx::query!(
        r#"
        DELETE FROM client_session_work_surface_resolutions
        WHERE client_session_id = $1 AND status = 'resolved'
        "#,
        client_session_id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert(
    pool: &PgPool,
    client_session_id: Uuid,
    work_surface_id: Uuid,
    status: ClientSessionWorkSurfaceResolutionStatus,
    evidence: Value,
) -> Result<(), DenError> {
    sqlx::query!(
        r#"
        INSERT INTO client_session_work_surface_resolutions (
            client_session_id, work_surface_id, status, evidence
        ) VALUES ($1, $2, $3, $4)
        ON CONFLICT (client_session_id) DO UPDATE
        SET work_surface_id = EXCLUDED.work_surface_id,
            status = EXCLUDED.status,
            evidence = EXCLUDED.evidence,
            resolved_at = NOW(),
            updated_at = NOW()
        "#,
        client_session_id,
        work_surface_id,
        status.as_str(),
        evidence,
    )
    .execute(pool)
    .await?;
    Ok(())
}
