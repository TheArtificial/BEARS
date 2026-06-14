//! Pure tool-support helpers now live in `den-tools::support`; this is a
//! re-export shim so existing `crate::core::tools::support::*` callers keep
//! resolving. The relocated helpers return `den_core::DenError`, which `?`
//! converts to `CustomError` at the (still web-coupled) executor boundary.
//! See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md` (Phase B).

pub(crate) use den_core::tools::support::*;
