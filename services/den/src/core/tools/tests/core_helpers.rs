use crate::core::tools::descriptor::builtin_den_tool_descriptors_for_profile;
use den_service::bears::BearProfile;
use std::collections::HashSet;

pub(super) fn names_for_profile(role: BearProfile) -> HashSet<&'static str> {
    builtin_den_tool_descriptors_for_profile(role)
        .into_iter()
        .map(|descriptor| descriptor.name)
        .collect()
}
