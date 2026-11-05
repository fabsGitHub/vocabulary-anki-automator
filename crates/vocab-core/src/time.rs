//! Injectable clock (Milestone G+: "use an injectable clock in tests, not
//! real wall-clock time").
//!
//! Everything that depends on "now" or on the user's local calendar date
//! (FSRS due dates, the once-per-day onboarding prompt, daily sessions,
//! streaks) takes a `&dyn Clock` instead of calling `Utc::now()` itself.

use std::sync::Mutex;

use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate, TimeZone, Utc};

pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> DateTime<Utc>;

    /// The UTC offset used to turn instants into the user's local calendar
    /// date. Sampled at "now", so a day that contains a DST switch is off by
    /// up to an hour at its edges, which is acceptable for daily scheduling.
    fn local_offset(&self) -> FixedOffset;

    /// The user's local calendar date at `now()`.
    fn today(&self) -> NaiveDate {
        self.now().with_timezone(&self.local_offset()).date_naive()
    }
}

/// The real clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn local_offset(&self) -> FixedOffset {
        *Local::now().offset()
    }
}

/// A clock for tests: fixed until explicitly moved.
#[derive(Debug)]
pub struct FixedClock {
    now: Mutex<DateTime<Utc>>,
    offset: FixedOffset,
}

impl FixedClock {
    pub fn new(now: DateTime<Utc>, offset: FixedOffset) -> Self {
        Self {
            now: Mutex::new(now),
            offset,
        }
    }

    /// A fixed clock in UTC.
    pub fn utc(now: DateTime<Utc>) -> Self {
        Self::new(now, FixedOffset::east_opt(0).expect("zero offset is valid"))
    }

    pub fn set(&self, now: DateTime<Utc>) {
        *self.now.lock().expect("clock mutex poisoned") = now;
    }

    pub fn advance(&self, by: Duration) {
        let mut now = self.now.lock().expect("clock mutex poisoned");
        *now += by;
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("clock mutex poisoned")
    }

    fn local_offset(&self) -> FixedOffset {
        self.offset
    }
}

/// `[start, end)` of a local calendar day, as UTC instants.
pub fn local_day_bounds(date: NaiveDate, offset: FixedOffset) -> (DateTime<Utc>, DateTime<Utc>) {
    let start_naive = date.and_hms_opt(0, 0, 0).expect("midnight is valid");
    let start = offset
        .from_local_datetime(&start_naive)
        .single()
        .expect("fixed offsets have no ambiguous local times")
        .with_timezone(&Utc);
    (start, start + Duration::days(1))
}

/// Formats a local date the way the store keeps it (`YYYY-MM-DD`).
pub fn format_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Parses a `YYYY-MM-DD` date as stored by [`format_date`].
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_respects_the_local_offset() {
        // 23:30 UTC on Oct 31 is already Nov 1 in Berlin (UTC+1 in winter).
        let now = Utc.with_ymd_and_hms(2026, 10, 31, 23, 30, 0).unwrap();
        let berlin = FixedClock::new(now, FixedOffset::east_opt(3600).unwrap());
        assert_eq!(
            berlin.today(),
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        );
        let utc = FixedClock::utc(now);
        assert_eq!(utc.today(), NaiveDate::from_ymd_opt(2026, 10, 31).unwrap());
    }

    #[test]
    fn fixed_clock_advances() {
        let clock = FixedClock::utc(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
        clock.advance(Duration::days(2));
        assert_eq!(clock.today(), NaiveDate::from_ymd_opt(2026, 1, 3).unwrap());
    }

    #[test]
    fn day_bounds_are_half_open_and_offset_aware() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let (start, end) = local_day_bounds(date, FixedOffset::east_opt(3600).unwrap());
        assert_eq!(start, Utc.with_ymd_and_hms(2026, 10, 31, 23, 0, 0).unwrap());
        assert_eq!(end, Utc.with_ymd_and_hms(2026, 11, 1, 23, 0, 0).unwrap());
    }

    /// docs/architecture.md § 2: with the offset sampled when the clock is
    /// read, the local date is right on both sides of a DST switch, and the
    /// end of that day is an hour early until the switch has happened.
    #[test]
    fn a_dst_switch_moves_the_end_of_the_day_only_until_it_happens() {
        // Berlin, 2026-10-25: CEST (UTC+2) until 01:00 UTC, then CET
        // (UTC+1). That local day ends at 23:00 UTC.
        let (cest, cet) = (
            FixedOffset::east_opt(2 * 3600).unwrap(),
            FixedOffset::east_opt(3600).unwrap(),
        );
        let switch = Utc.with_ymd_and_hms(2026, 10, 25, 1, 0, 0).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        for (now, offset) in [(switch - Duration::milliseconds(1), cest), (switch, cet)] {
            assert_eq!(FixedClock::new(now, offset).today(), date, "{now}");
        }
        let midnight = Utc.with_ymd_and_hms(2026, 10, 25, 23, 0, 0).unwrap();
        assert_eq!(
            local_day_bounds(date, cest).1,
            midnight - Duration::hours(1)
        );
        assert_eq!(local_day_bounds(date, cet).1, midnight);
    }

    #[test]
    fn date_format_round_trips() {
        let date = NaiveDate::from_ymd_opt(2026, 2, 3).unwrap();
        assert_eq!(format_date(date), "2026-02-03");
        assert_eq!(parse_date("2026-02-03"), Some(date));
        assert_eq!(parse_date("03.02.2026"), None);
    }
}
