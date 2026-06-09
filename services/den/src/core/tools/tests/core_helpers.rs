use crate::core::{
    bears::BearProfile,
    tools::descriptor::builtin_den_tool_descriptors_for_role,
};
use std::collections::HashSet;

pub(super) fn names_for_role(role: BearProfile) -> HashSet<&'static str> {
    builtin_den_tool_descriptors_for_role(role)
        .into_iter()
        .map(|descriptor| descriptor.name)
        .collect()
}
