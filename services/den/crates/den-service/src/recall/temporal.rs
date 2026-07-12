//! Time-expression parsing for the temporal recall leg (ADR-0038 Phase 3.5).
//!
//! Turns a natural-language query into an effective-time window (and optional point-in-time
//! `as_of`) plus the residual query text with the recognized temporal phrase removed, so the
//! vector/keyword legs match on the topical remainder while recall filters/boosts on time.
//!
//! The grammar is deliberately bounded: relative windows (`today`, `yesterday`, `last week`,
//! `last N days|weeks|months|years`, `this week|month|year`), explicit `YYYY-MM-DD` days,
//! month/year spans (`in June 2026`, `2025`), open bounds (`before`/`since`/`after <date>`), and
//! point-in-time (`as of <date>`). Unrecognized queries return `None` (no temporal constraint).

use time::{Date, Duration, Month, OffsetDateTime, Time};

/// A parsed temporal constraint over effective event time (`COALESCE(valid_from, created_at)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalQuery {
    /// Inclusive lower bound, if any.
    pub from: Option<OffsetDateTime>,
    /// Exclusive upper bound, if any.
    pub to: Option<OffsetDateTime>,
    /// Point-in-time recall: also exclude records already superseded as of `to`.
    pub as_of: bool,
    /// The query with the recognized temporal phrase removed (topical remainder).
    pub residual_query: String,
    /// The recognized phrase, for diagnostics.
    pub matched: String,
}

/// Parse a temporal expression from `query` relative to `now` (UTC). Returns `None` when no
/// recognized temporal phrase is present.
pub fn parse_time_expression(query: &str, now: OffsetDateTime) -> Option<TemporalQuery> {
    let orig: Vec<&str> = query.split_whitespace().collect();
    if orig.is_empty() {
        return None;
    }
    let lower: Vec<String> = orig
        .iter()
        .map(|t| {
            t.to_lowercase()
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
                .to_string()
        })
        .collect();
    let toks: Vec<&str> = lower.iter().map(String::as_str).collect();

    let (from, to, as_of, start, end) = match_as_of(&toks, now)
        .or_else(|| match_open_bound(&toks, now))
        .or_else(|| match_last_n(&toks, now))
        .or_else(|| match_named_relative(&toks, now))
        .or_else(|| match_this_period(&toks, now))
        .or_else(|| match_in_month_or_year(&toks, now))
        .or_else(|| match_explicit_date(&toks))?;

    let matched = orig[start..end].join(" ");
    let residual = orig
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < start || *i >= end)
        .map(|(_, t)| *t)
        .collect::<Vec<_>>()
        .join(" ");
    Some(TemporalQuery {
        from,
        to,
        as_of,
        residual_query: residual,
        matched,
    })
}

type Match = (
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
    bool,
    usize,
    usize,
);

fn start_of_day(dt: OffsetDateTime) -> OffsetDateTime {
    dt.replace_time(Time::MIDNIGHT)
}

fn day_start(date: Date) -> OffsetDateTime {
    date.midnight().assume_utc()
}

fn month_start(year: i32, month: Month) -> Option<OffsetDateTime> {
    Date::from_calendar_date(year, month, 1).ok().map(day_start)
}

fn next_month_start(year: i32, month: Month) -> Option<OffsetDateTime> {
    if month == Month::December {
        month_start(year + 1, Month::January)
    } else {
        month_start(year, month.next())
    }
}

fn parse_ymd(tok: &str) -> Option<Date> {
    let mut parts = tok.split('-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u8 = parts.next()?.parse().ok()?;
    let d: u8 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Date::from_calendar_date(y, Month::try_from(m).ok()?, d).ok()
}

fn parse_year(tok: &str) -> Option<i32> {
    if tok.len() == 4 && tok.chars().all(|c| c.is_ascii_digit()) {
        let y: i32 = tok.parse().ok()?;
        if (1970..=2200).contains(&y) {
            return Some(y);
        }
    }
    None
}

fn parse_month_name(tok: &str) -> Option<Month> {
    Some(match tok {
        "jan" | "january" => Month::January,
        "feb" | "february" => Month::February,
        "mar" | "march" => Month::March,
        "apr" | "april" => Month::April,
        "may" => Month::May,
        "jun" | "june" => Month::June,
        "jul" | "july" => Month::July,
        "aug" | "august" => Month::August,
        "sep" | "sept" | "september" => Month::September,
        "oct" | "october" => Month::October,
        "nov" | "november" => Month::November,
        "dec" | "december" => Month::December,
        _ => return None,
    })
}

/// Resolve a date token that is either `YYYY-MM-DD`, `today`, or `yesterday`.
fn resolve_date_token(tok: &str, now: OffsetDateTime) -> Option<Date> {
    match tok {
        "today" => Some(now.date()),
        "yesterday" => Some((now - Duration::days(1)).date()),
        other => parse_ymd(other),
    }
}

fn match_as_of(toks: &[&str], now: OffsetDateTime) -> Option<Match> {
    // "as of <date>"
    for i in 0..toks.len() {
        if toks[i] == "as" && toks.get(i + 1) == Some(&"of") {
            if let Some(date_tok) = toks.get(i + 2) {
                if let Some(date) = resolve_date_token(date_tok, now) {
                    // Inclusive through the named day → exclusive upper bound at next midnight.
                    let to = day_start(date) + Duration::days(1);
                    return Some((None, Some(to), true, i, i + 3));
                }
            }
        }
    }
    None
}

fn match_open_bound(toks: &[&str], now: OffsetDateTime) -> Option<Match> {
    for i in 0..toks.len() {
        // "up to <date>" behaves like "before".
        let (kind, date_idx, start) = match toks[i] {
            "before" | "until" => ("before", i + 1, i),
            "since" => ("since", i + 1, i),
            "after" => ("after", i + 1, i),
            "up" if toks.get(i + 1) == Some(&"to") => ("before", i + 2, i),
            _ => continue,
        };
        let Some(date_tok) = toks.get(date_idx) else {
            continue;
        };
        let Some(date) = resolve_date_token(date_tok, now) else {
            continue;
        };
        let end = date_idx + 1;
        return Some(match kind {
            "before" => (None, Some(day_start(date)), false, start, end),
            "since" => (Some(day_start(date)), None, false, start, end),
            // "after" excludes the named day itself.
            _ => (
                Some(day_start(date) + Duration::days(1)),
                None,
                false,
                start,
                end,
            ),
        });
    }
    None
}

fn unit_window(n: i64, unit: &str, now: OffsetDateTime) -> Option<OffsetDateTime> {
    let dur = match unit {
        "day" | "days" => Duration::days(n),
        "week" | "weeks" => Duration::weeks(n),
        // Rolling approximations for open-ended natural windows.
        "month" | "months" => Duration::days(n * 30),
        "year" | "years" => Duration::days(n * 365),
        _ => return None,
    };
    Some(now - dur)
}

fn match_last_n(toks: &[&str], now: OffsetDateTime) -> Option<Match> {
    for i in 0..toks.len() {
        if !matches!(toks[i], "last" | "past" | "previous") {
            continue;
        }
        // "last 3 weeks"
        if let (Some(num), Some(unit)) = (toks.get(i + 1), toks.get(i + 2)) {
            if let Ok(n) = num.parse::<i64>() {
                if n > 0 {
                    if let Some(from) = unit_window(n, unit, now) {
                        return Some((Some(from), Some(now), false, i, i + 3));
                    }
                }
            }
        }
        // "last week" (singular, n = 1)
        if let Some(unit) = toks.get(i + 1) {
            if let Some(from) = unit_window(1, unit, now) {
                return Some((Some(from), Some(now), false, i, i + 2));
            }
        }
    }
    None
}

fn match_named_relative(toks: &[&str], now: OffsetDateTime) -> Option<Match> {
    for (i, tok) in toks.iter().enumerate() {
        match *tok {
            "today" => {
                let from = start_of_day(now);
                return Some((Some(from), Some(from + Duration::days(1)), false, i, i + 1));
            }
            "yesterday" => {
                let from = start_of_day(now) - Duration::days(1);
                return Some((Some(from), Some(from + Duration::days(1)), false, i, i + 1));
            }
            _ => {}
        }
    }
    None
}

fn match_this_period(toks: &[&str], now: OffsetDateTime) -> Option<Match> {
    for i in 0..toks.len() {
        if toks[i] != "this" {
            continue;
        }
        let Some(period) = toks.get(i + 1) else {
            continue;
        };
        let from = match *period {
            "week" => {
                let weekday = i64::from(now.weekday().number_days_from_monday());
                start_of_day(now) - Duration::days(weekday)
            }
            "month" => month_start(now.year(), now.month())?,
            "year" => month_start(now.year(), Month::January)?,
            _ => continue,
        };
        return Some((Some(from), Some(now), false, i, i + 2));
    }
    None
}

fn match_in_month_or_year(toks: &[&str], now: OffsetDateTime) -> Option<Match> {
    for i in 0..toks.len() {
        // "in June 2026" / "in June" / "in 2025"
        let base = if toks[i] == "in" { i + 1 } else { i };
        let consume_in = usize::from(toks[i] == "in");

        if let Some(month_tok) = toks.get(base) {
            if let Some(month) = parse_month_name(month_tok) {
                let (year, end_idx) = match toks.get(base + 1).and_then(|t| parse_year(t)) {
                    Some(y) => (y, base + 2),
                    None => (now.year(), base + 1),
                };
                let from = month_start(year, month)?;
                let to = next_month_start(year, month)?;
                return Some((
                    Some(from),
                    Some(to),
                    false,
                    i,
                    end_idx.max(i + consume_in + 1),
                ));
            }
            if let Some(year) = parse_year(month_tok) {
                // Only treat a bare year as temporal when introduced by "in" (avoid eating IDs).
                if toks[i] == "in" {
                    let from = month_start(year, Month::January)?;
                    let to = month_start(year + 1, Month::January)?;
                    return Some((Some(from), Some(to), false, i, base + 1));
                }
            }
        }
    }
    None
}

fn match_explicit_date(toks: &[&str]) -> Option<Match> {
    for (i, tok) in toks.iter().enumerate() {
        if let Some(date) = parse_ymd(tok) {
            let from = day_start(date);
            return Some((Some(from), Some(from + Duration::days(1)), false, i, i + 1));
        }
    }
    None
}

#[cfg(test)]
mod tests {
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
}
