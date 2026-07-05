
use super::*;

#[test]
fn governance_mode_string_round_trip_is_snake_case() {
    for mode in GovernanceMode::ALL {
        assert_eq!(GovernanceMode::from_str(mode.as_str()).unwrap(), mode);
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, format!("\"{}\"", mode.as_str()));
        assert_eq!(serde_json::from_str::<GovernanceMode>(&json).unwrap(), mode);
    }
}

#[test]
fn run_mode_projection_matches_adr_0039() {
    assert_eq!(
        GovernanceMode::AutonomousContinuation.run_mode(),
        RunMode::Autonomous
    );
    for mode in [
        GovernanceMode::Interactive,
        GovernanceMode::Grace,
        GovernanceMode::Observational,
        GovernanceMode::Frozen,
    ] {
        assert_eq!(mode.run_mode(), RunMode::Interactive);
    }
}

#[test]
fn human_present_only_for_interactive_and_observational() {
    assert!(GovernanceMode::Interactive.human_present());
    assert!(GovernanceMode::Observational.human_present());
    assert!(!GovernanceMode::Grace.human_present());
    assert!(!GovernanceMode::AutonomousContinuation.human_present());
    assert!(!GovernanceMode::Frozen.human_present());
}

#[test]
fn unknown_governance_mode_is_rejected() {
    assert!(GovernanceMode::from_str("pair").is_err());
}
