use crate::core::tools::constants::*;
use den_runtime::bears::BearProfile;

use super::core_helpers::names_for_profile;

#[test]
fn privileged_descriptors_are_role_scoped() {
    let chat = names_for_profile(BearProfile::Chat);
    assert!(chat.contains(DEN_TASK_WRITE_INTENT));
    assert!(chat.contains(DEN_SKILL_PROPOSE));
    assert!(!chat.contains(DEN_OBSERVATION_WRITE));
    assert!(!chat.contains(DEN_RUN_WRITE_RESULT));

    let pair = names_for_profile(BearProfile::Pair);
    assert!(pair.contains(DEN_TASK_WRITE_INTENT));
    assert!(pair.contains(DEN_WORK_PLAN_UPDATE));
    assert!(pair.contains(DEN_WORK_PLAN_REQUEST_HANDOFF));
    assert!(pair.contains(DEN_SKILL_PROPOSE));
    assert!(!pair.contains(DEN_OBSERVATION_WRITE));
    assert!(!pair.contains(DEN_RUN_WRITE_RESULT));

    let curate = names_for_profile(BearProfile::Curate);
    assert!(curate.contains(DEN_TASK_APPROVE_INTENT));
    assert!(curate.contains(DEN_TASK_REJECT_INTENT));
    assert!(curate.contains(DEN_CORE_WRITE_RESULT_SUMMARY));
    assert!(curate.contains(DEN_SKILL_APPROVE_PROPOSAL));
    assert!(curate.contains(DEN_SKILL_REJECT_PROPOSAL));
    assert!(curate.contains(DEN_SKILL_PROPOSE));
    assert!(curate.contains(DEN_ENTITY_BROWSE));
    assert!(curate.contains(DEN_ENTITY_RESOLVE));
    assert!(!curate.contains(DEN_ENTITY_LINK_MEMORY));
    assert!(curate.contains(DEN_ENTITY_MERGE));
    assert!(curate.contains(DEN_ENTITY_SPLIT));
    assert!(!curate.contains(DEN_TASK_WRITE_INTENT));
    assert!(!curate.contains(DEN_OBSERVATION_WRITE));
    assert!(!curate.contains(DEN_RUN_WRITE_RESULT));

    let watch = names_for_profile(BearProfile::Watch);
    assert!(watch.contains(DEN_MEMORY_STATUS));
    assert!(watch.contains(DEN_MEMORY_SEARCH));
    assert!(watch.contains(DEN_MEMORY_READ));
    assert!(watch.contains(DEN_ENTITY_BROWSE));
    assert!(watch.contains(DEN_ENTITY_RESOLVE));
    assert!(watch.contains(DEN_ENTITY_LINK_MEMORY));
    assert!(!watch.contains(DEN_ENTITY_MERGE));
    assert!(!watch.contains(DEN_ENTITY_SPLIT));
    assert!(watch.contains(DEN_OBSERVATION_WRITE));
    assert!(watch.contains(DEN_SKILL_PROPOSE));
    assert!(!watch.contains(DEN_WORK_PLAN_LIST));
    assert!(!watch.contains(DEN_WORK_PLAN_UPDATE));
    assert!(!watch.contains(DEN_TASK_WRITE_INTENT));
    assert!(!watch.contains(DEN_RUN_WRITE_RESULT));

    let work = names_for_profile(BearProfile::Work);
    assert!(work.contains(DEN_MEMORY_STATUS));
    assert!(work.contains(DEN_MEMORY_SEARCH));
    assert!(work.contains(DEN_MEMORY_READ));
    assert!(work.contains(DEN_ENTITY_BROWSE));
    assert!(work.contains(DEN_ENTITY_RESOLVE));
    assert!(work.contains(DEN_ENTITY_LINK_MEMORY));
    assert!(!work.contains(DEN_ENTITY_MERGE));
    assert!(!work.contains(DEN_ENTITY_SPLIT));
    assert!(work.contains(DEN_RUN_WRITE_RESULT));
    assert!(work.contains(DEN_WORK_PLAN_LIST));
    assert!(work.contains(DEN_WORK_PLAN_UPDATE));
    assert!(!work.contains(DEN_WORK_PLAN_REQUEST_HANDOFF));
    assert!(work.contains(DEN_SKILL_PROPOSE));
    assert!(!work.contains(DEN_TASK_WRITE_INTENT));
    assert!(!work.contains(DEN_OBSERVATION_WRITE));
}
