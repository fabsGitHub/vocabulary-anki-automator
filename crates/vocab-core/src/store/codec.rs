//! Text encodings shared by every table.
//!
//! Writes are canonical, one format per type, chosen so that SQL text
//! comparison agrees with value order. Reads are lenient, so rows written by
//! hand, by another tool or by SQLite's own date functions still load.

use std::{fmt::Display, ops::RangeInclusive};

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, SecondsFormat, Utc};

use super::{StoreError, StoreResult};
use crate::time;

/// The years the stored forms can hold. Only these have the fixed-width
/// `YYYY` that the ordering relies on: chrono writes any other year with a
/// sign and more digits (`+10000-01-01T...`), which sorts before every other
/// row and does not parse back, so a single such row would break every
/// listing that reaches it. Writing one is refused instead.
const STORABLE_YEARS: RangeInclusive<i32> = 0..=9999;

/// Compares above every stored instant: stored text starts with a digit, and
/// `:` is the character after `9`.
const ABOVE_EVERY_TIMESTAMP: &str = ":";

/// The stored form of an instant: RFC 3339 in UTC with a `Z` and exactly
/// three fractional digits. The fixed width is what makes `ORDER BY` and
/// `<` on the text chronological. Fails for a year outside 0000–9999.
pub(super) fn format_timestamp(at: DateTime<Utc>) -> StoreResult<String> {
    require_storable_year(at.year(), at)?;
    Ok(at.to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// The stored form of an exclusive upper bound, for `column < ?`.
///
/// Stored instants are whole milliseconds, so `stored < before` holds exactly
/// when `stored` is below `before` rounded *up* to the next millisecond.
/// Formatting `before` itself would round it down and wrongly exclude a row
/// stored at that millisecond, such as a card made due "now" and queried
/// with the same `now`.
///
/// A bound past year 9999 (`DateTime::<Utc>::MAX_UTC` as "no bound") is above
/// every storable instant. One before year 0 is below every one, which its
/// text, starting with `-`, already is.
pub(super) fn format_exclusive_bound(before: DateTime<Utc>) -> String {
    let below_milli = before.timestamp_subsec_nanos() % 1_000_000;
    let rounded_up = if below_milli == 0 {
        before
    } else {
        before
            .checked_add_signed(Duration::nanoseconds(i64::from(1_000_000 - below_milli)))
            .unwrap_or(before)
    };
    if rounded_up.year() > *STORABLE_YEARS.end() {
        return ABOVE_EVERY_TIMESTAMP.to_owned();
    }
    rounded_up.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Parses a stored instant. Accepts any RFC 3339 form (with or without
/// fractional seconds, any offset, `T` or a space as separator) and, taken as
/// UTC, the offset-less `YYYY-MM-DD HH:MM:SS[.fff]` that SQLite's
/// `datetime()` produces.
pub(super) fn parse_timestamp(text: &str) -> StoreResult<DateTime<Utc>> {
    let text = text.trim();
    if let Ok(at) = DateTime::parse_from_rfc3339(text) {
        return Ok(at.with_timezone(&Utc));
    }
    ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"]
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
        .map(|naive| naive.and_utc())
        .ok_or_else(|| StoreError::Invalid(format!("not an RFC 3339 timestamp: {text:?}")))
}

pub(super) fn parse_optional_timestamp(text: Option<String>) -> StoreResult<Option<DateTime<Utc>>> {
    text.as_deref().map(parse_timestamp).transpose()
}

/// The stored form of a local calendar date, `YYYY-MM-DD`. Fails for a year
/// outside 0000–9999.
pub(super) fn format_date(date: NaiveDate) -> StoreResult<String> {
    require_storable_year(date.year(), date)?;
    Ok(time::format_date(date))
}

fn require_storable_year(year: i32, value: impl Display) -> StoreResult<()> {
    if STORABLE_YEARS.contains(&year) {
        return Ok(());
    }
    Err(StoreError::Invalid(format!(
        "{value} is outside the years 0000 to 9999 that the database can store"
    )))
}

pub(super) fn parse_date(text: &str) -> StoreResult<NaiveDate> {
    time::parse_date(text.trim())
        .ok_or_else(|| StoreError::Invalid(format!("not a YYYY-MM-DD date: {text:?}")))
}

/// Tags are a JSON array of strings, so a tag may contain any character.
pub(super) fn encode_tags(tags: &[String]) -> StoreResult<String> {
    Ok(serde_json::to_string(tags)?)
}

pub(super) fn decode_tags(text: &str) -> StoreResult<Vec<String>> {
    Ok(serde_json::from_str(text)?)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, milli: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap() + Duration::milliseconds(milli.into())
    }

    #[test]
    fn timestamps_are_written_with_exactly_three_fractional_digits() {
        let cases = [
            (utc(2026, 10, 31, 12, 0, 0, 0), "2026-10-31T12:00:00.000Z"),
            (utc(2026, 10, 31, 12, 0, 0, 7), "2026-10-31T12:00:00.007Z"),
            (
                utc(2026, 10, 31, 12, 0, 0, 7) + Duration::microseconds(999),
                "2026-10-31T12:00:00.007Z",
            ),
            (utc(1999, 1, 2, 3, 4, 5, 600), "1999-01-02T03:04:05.600Z"),
        ];
        for (at, expected) in cases {
            assert_eq!(format_timestamp(at).unwrap(), expected);
        }
    }

    #[test]
    fn stored_text_order_is_chronological_where_shorter_forms_are_not() {
        let whole = utc(2026, 10, 31, 12, 0, 0, 0);
        let later = utc(2026, 10, 31, 12, 0, 0, 500);
        assert!(format_timestamp(whole).unwrap() < format_timestamp(later).unwrap());
        // The trap the fixed width avoids: without fractional digits, "Z"
        // sorts after ".", putting 12:00:00 after 12:00:00.500.
        assert!(
            whole.to_rfc3339_opts(SecondsFormat::AutoSi, true)
                > later.to_rfc3339_opts(SecondsFormat::AutoSi, true)
        );
    }

    #[test]
    fn exclusive_bounds_round_up_to_the_next_millisecond() {
        let at = utc(2026, 10, 31, 12, 0, 0, 500);
        let cases = [
            (at, "2026-10-31T12:00:00.500Z"),
            (at + Duration::nanoseconds(1), "2026-10-31T12:00:00.501Z"),
            (at + Duration::microseconds(300), "2026-10-31T12:00:00.501Z"),
            (
                utc(2026, 10, 31, 23, 59, 59, 999) + Duration::microseconds(1),
                "2026-11-01T00:00:00.000Z",
            ),
        ];
        for (before, expected) in cases {
            assert_eq!(format_exclusive_bound(before), expected, "{before:?}");
        }
    }

    #[test]
    fn exclusive_bounds_outside_the_storable_years_compare_past_every_stored_instant() {
        let first = format_timestamp(utc(0, 1, 1, 0, 0, 0, 0)).unwrap();
        let last = format_timestamp(utc(9999, 12, 31, 23, 59, 59, 999)).unwrap();
        for before in [
            utc(10000, 1, 1, 0, 0, 0, 0),
            utc(9999, 12, 31, 23, 59, 59, 999) + Duration::nanoseconds(1),
            DateTime::<Utc>::MAX_UTC,
        ] {
            assert!(format_exclusive_bound(before) > last, "{before:?}");
        }
        for before in [utc(-1, 12, 31, 23, 59, 59, 999), DateTime::<Utc>::MIN_UTC] {
            assert!(format_exclusive_bound(before) < first, "{before:?}");
        }
        // The last storable millisecond itself still gets an exact bound.
        assert_eq!(
            format_exclusive_bound(utc(9999, 12, 31, 23, 59, 59, 999)),
            last
        );
    }

    #[test]
    fn years_without_a_four_digit_form_are_refused_on_write() {
        // The edges of the range still round-trip.
        for at in [utc(0, 1, 1, 0, 0, 0, 0), utc(9999, 12, 31, 23, 59, 59, 999)] {
            assert_eq!(parse_timestamp(&format_timestamp(at).unwrap()).unwrap(), at);
            let date = at.date_naive();
            assert_eq!(parse_date(&format_date(date).unwrap()).unwrap(), date);
        }
        for at in [
            utc(10000, 1, 1, 0, 0, 0, 0),
            utc(-1, 12, 31, 23, 59, 59, 999),
            DateTime::<Utc>::MAX_UTC,
        ] {
            assert!(
                matches!(format_timestamp(at), Err(StoreError::Invalid(message)) if message.contains("0000 to 9999")),
                "{at:?}"
            );
            assert!(
                matches!(format_date(at.date_naive()), Err(StoreError::Invalid(_))),
                "{at:?}"
            );
        }
    }

    #[test]
    fn timestamps_are_read_leniently() {
        let noon = utc(2026, 10, 31, 12, 0, 0, 0);
        let cases = [
            ("2026-10-31T12:00:00.000Z", noon),
            ("2026-10-31T12:00:00Z", noon),
            ("2026-10-31T12:00:00+00:00", noon),
            ("2026-10-31T13:00:00.000+01:00", noon),
            ("2026-10-31t12:00:00z", noon),
            ("2026-10-31 12:00:00Z", noon),
            ("2026-10-31 12:00:00", noon),
            ("2026-10-31T12:00:00", noon),
            (" 2026-10-31T12:00:00Z ", noon),
            ("2026-10-31T12:00:00.25Z", utc(2026, 10, 31, 12, 0, 0, 250)),
            ("2026-10-31 12:00:00.250", utc(2026, 10, 31, 12, 0, 0, 250)),
            (
                "2026-10-31T12:00:00.123456789Z",
                noon + Duration::nanoseconds(123_456_789),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_timestamp(text).unwrap(), expected, "{text:?}");
        }
    }

    #[test]
    fn unparseable_timestamps_are_invalid() {
        for text in [
            "",
            "yesterday",
            "2026-10-31",
            "2026-13-01T00:00:00Z",
            "12:00:00",
        ] {
            assert!(
                matches!(parse_timestamp(text), Err(StoreError::Invalid(_))),
                "{text:?} should be rejected"
            );
        }
        assert_eq!(parse_optional_timestamp(None).unwrap(), None);
    }

    #[test]
    fn dates_round_trip_and_reject_other_formats() {
        let date = NaiveDate::from_ymd_opt(2026, 2, 3).unwrap();
        assert_eq!(format_date(date).unwrap(), "2026-02-03");
        assert_eq!(parse_date("2026-02-03").unwrap(), date);
        for text in ["03.02.2026", "2026-02-30", ""] {
            assert!(
                matches!(parse_date(text), Err(StoreError::Invalid(_))),
                "{text:?}"
            );
        }
    }

    #[test]
    fn tags_are_a_json_array() {
        let tags = vec!["hsk-1".to_string(), "with \"quotes\", commas".to_string()];
        let text = encode_tags(&tags).unwrap();
        assert_eq!(text, r#"["hsk-1","with \"quotes\", commas"]"#);
        assert_eq!(decode_tags(&text).unwrap(), tags);
        assert_eq!(encode_tags(&[]).unwrap(), "[]");
        assert!(matches!(
            decode_tags("hsk-1, travel"),
            Err(StoreError::Json(_))
        ));
    }
}
