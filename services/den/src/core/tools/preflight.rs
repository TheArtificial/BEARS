//! Tool preflight types/functions now live in `den-tools::preflight`; this is a
//! re-export shim so existing `crate::core::tools::preflight::*` callers keep
//! resolving. See `docs/roadmap/DEN_CRATE_SPLIT_PLAN.md` (Phase B).

pub(crate) use den_tools::preflight::*;
