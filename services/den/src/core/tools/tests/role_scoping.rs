use crate::core::{bears::BearAgentRole, tools::constants::*};

use super::core_helpers::names_for_role;

#[test]
fn privileged_descriptors_are_role_scoped() {
    let talk = names_for_role(BearAgentRole::Talk);
    assert!(talk.contains(DEN_TASK_WRITE_INTENT));
    assert!(talk.contains(DEN_SKILL_PROPOSE));
    assert!(!talk.contains(DEN_OBSERVATION_WRITE));
    assert!(!talk.contains(DEN_RUN_WRITE_RESULT));

    let pair = names_for_role(BearAgentRole::Pair);
    assert!(pair.contains(DEN_TASK_WRITE_INTENT));
    assert!(pair.contains(DEN_WORK_PLAN_UPDATE));
    assert!(pair.contains(DEN_WORK_PLAN_REQUEST_HANDOFF));
    assert!(pair.contains(DEN_SKILL_PROPOSE));
    assert!(!pair.contains(DEN_OBSERVATION_WRITE));
    assert!(!pair.contains(DEN_RUN_WRITE_RESULT));

    let curate = names_for_role(BearAgentRole::Curate);
    assert!(curate.contains(DEN_TASK_APPROVE_INTENT));
    assert!(curate.contains(DEN_TASK_REJECT_INTENT));
    assert!(curate.contains(DEN_CORE_WRITE_RESULT_SUMMARY));
    assert!(curate.contains(DEN_SKILL_APPROVE_PROPOSAL));
    assert!(curate.contains(DEN_SKILL_REJECT_PROPOSAL));
    assert!(curate.contains(DEN_SKILL_PROPOSE));
    assert!(!curate.contains(DEN_TASK_WRITE_INTENT));
    assert!(!curate.contains(DEN_OBSERVATION_WRITE));
    assert!(!curate.contains(DEN_RUN_WRITE_RESULT));

    let watch = names_for_role(BearAgentRole::Watch);
    assert!(watch.contains(DEN_OBSERVATION_WRITE));
    assert!(watch.contains(DEN_SKILL_PROPOSE));
    assert!(!watch.contains(DEN_WORK_PLAN_LIST));
    assert!(!watch.contains(DEN_WORK_PLAN_UPDATE));
    assert!(!watch.contains(DEN_TASK_WRITE_INTENT));
    assert!(!watch.contains(DEN_RUN_WRITE_RESULT));

    let work = names_for_role(BearAgentRole::Work);
    assert!(work.contains(DEN_RUN_WRITE_RESULT));
    assert!(work.contains(DEN_WORK_PLAN_LIST));
    assert!(work.contains(DEN_WORK_PLAN_UPDATE));
    assert!(!work.contains(DEN_WORK_PLAN_REQUEST_HANDOFF));
    assert!(work.contains(DEN_SKILL_PROPOSE));
    assert!(!work.contains(DEN_TASK_WRITE_INTENT));
    assert!(!work.contains(DEN_OBSERVATION_WRITE));
}
