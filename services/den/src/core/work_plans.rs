//! Compatibility shim — the canonical home is [`crate::core::docket`].
//!
//! The work-plan **types** are re-exported here unchanged so existing
//! `crate::core::work_plans::…` paths keep compiling. The former free-function
//! DB operations (`create_or_update_work_plan`, `list_visible_work_plans`,
//! `get_visible_work_plan`) are now methods on
//! [`crate::core::docket::PgDocketService`] (the `DocketService` public face).
//! New code should use `crate::core::docket` directly.

pub use crate::core::docket::*;
