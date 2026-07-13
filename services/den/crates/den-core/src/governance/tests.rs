use super::*;

#[test]
fn governance_string_round_trip_is_snake_case() {
    for mode in Governance::ALL {
        assert_eq!(Governance::from_str(mode.as_str()).unwrap(), mode);
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, format!("\"{}\"", mode.as_str()));
        assert_eq!(serde_json::from_str::<Governance>(&json).unwrap(), mode);
    }
}

#[test]
fn run_mode_projection_matches_adr_0039() {
    assert_eq!(
        Governance::AutonomousContinuation.run_mode(),
        RunMode::Autonomous
    );
    for mode in [
        Governance::Interactive,
        Governance::Grace,
        Governance::Observational,
        Governance::Frozen,
    ] {
        assert_eq!(mode.run_mode(), RunMode::Interactive);
    }
}

#[test]
fn human_present_only_for_interactive_and_observational() {
    assert!(Governance::Interactive.human_present());
    assert!(Governance::Observational.human_present());
    assert!(!Governance::Grace.human_present());
    assert!(!Governance::AutonomousContinuation.human_present());
    assert!(!Governance::Frozen.human_present());
}

#[test]
fn unknown_governance_is_rejected() {
    assert!(Governance::from_str("pair").is_err());
}
