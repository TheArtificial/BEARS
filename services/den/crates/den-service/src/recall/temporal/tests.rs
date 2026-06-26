    use super::*;

    fn dt(y: i32, m: u8, d: u8, h: u8, min: u8) -> OffsetDateTime {
        Date::from_calendar_date(y, Month::try_from(m).unwrap(), d)
            .unwrap()
            .with_hms(h, min, 0)
            .unwrap()
            .assume_utc()
    }

    fn midnight(y: i32, m: u8, d: u8) -> OffsetDateTime {
        dt(y, m, d, 0, 0)
    }

    // Fixed "now": Tuesday 2026-06-16 09:30 UTC.
    fn now() -> OffsetDateTime {
        dt(2026, 6, 16, 9, 30)
    }

    #[test]
    fn no_temporal_phrase_returns_none() {
        assert!(parse_time_expression("deployment runbook for the gateway", now()).is_none());
        assert!(parse_time_expression("", now()).is_none());
    }

    #[test]
    fn yesterday_window_is_the_prior_day() {
        let q = parse_time_expression("decisions from yesterday", now()).expect("parsed");
        assert_eq!(q.from, Some(midnight(2026, 6, 15)));
        assert_eq!(q.to, Some(midnight(2026, 6, 16)));
        assert!(!q.as_of);
        assert_eq!(q.residual_query, "decisions from");
        assert_eq!(q.matched, "yesterday");
    }

    #[test]
    fn last_week_is_rolling_seven_days() {
        let q = parse_time_expression("what did we decide last week", now()).expect("parsed");
        assert_eq!(q.from, Some(dt(2026, 6, 9, 9, 30)));
        assert_eq!(q.to, Some(now()));
        assert_eq!(q.residual_query, "what did we decide");
    }

    #[test]
    fn last_n_months_rolling_window() {
        let q = parse_time_expression("incidents in the last 3 months", now()).expect("parsed");
        assert_eq!(q.from, Some(now() - Duration::days(90)));
        assert_eq!(q.to, Some(now()));
        assert_eq!(q.residual_query, "incidents in the");
    }

    #[test]
    fn as_of_sets_point_in_time_upper_bound() {
        let q = parse_time_expression("policy as of 2026-06-01", now()).expect("parsed");
        assert!(q.as_of);
        assert_eq!(q.from, None);
        assert_eq!(q.to, Some(midnight(2026, 6, 2)));
        assert_eq!(q.residual_query, "policy");
    }

    #[test]
    fn before_and_since_are_open_bounds() {
        let before = parse_time_expression("changes before 2026-01-01", now()).expect("parsed");
        assert_eq!(before.from, None);
        assert_eq!(before.to, Some(midnight(2026, 1, 1)));

        let since = parse_time_expression("changes since 2026-03-15", now()).expect("parsed");
        assert_eq!(since.from, Some(midnight(2026, 3, 15)));
        assert_eq!(since.to, None);

        let after = parse_time_expression("changes after 2026-03-15", now()).expect("parsed");
        assert_eq!(after.from, Some(midnight(2026, 3, 16)));
    }

    #[test]
    fn in_month_and_year_spans() {
        let month = parse_time_expression("retro in June 2025", now()).expect("parsed");
        assert_eq!(month.from, Some(midnight(2025, 6, 1)));
        assert_eq!(month.to, Some(midnight(2025, 7, 1)));
        assert_eq!(month.residual_query, "retro");

        let year = parse_time_expression("summary in 2024", now()).expect("parsed");
        assert_eq!(year.from, Some(midnight(2024, 1, 1)));
        assert_eq!(year.to, Some(midnight(2025, 1, 1)));
    }

    #[test]
    fn explicit_date_is_a_single_day() {
        let q = parse_time_expression("standup 2026-02-10 notes", now()).expect("parsed");
        assert_eq!(q.from, Some(midnight(2026, 2, 10)));
        assert_eq!(q.to, Some(midnight(2026, 2, 11)));
        assert_eq!(q.residual_query, "standup notes");
    }

    #[test]
    fn this_month_starts_at_the_first() {
        let q = parse_time_expression("spend this month", now()).expect("parsed");
        assert_eq!(q.from, Some(midnight(2026, 6, 1)));
        assert_eq!(q.to, Some(now()));
    }
