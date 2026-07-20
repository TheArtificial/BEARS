use den_core::DenError;
use time::OffsetDateTime;

pub(crate) fn now_rfc3339() -> Result<String, DenError> {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|err| DenError::System(format!("timestamp format failed: {err}")))
}
