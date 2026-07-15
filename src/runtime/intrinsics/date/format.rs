//! Pure string formatting for the pinned QuickJS `Date` intrinsic.
//!
//! This is a direct Rust rendering of `get_date_string` in QuickJS 2026-06-04.
//! Locale-named methods intentionally retain QuickJS's fixed US-style output;
//! QuickJS does not consult ICU, a locale argument, or an options object here.

use std::fmt;

use super::calendar::DateFields;

const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const INVALID_DATE: &str = "Invalid Date";

/// The eight distinct formatting entry points backed by QuickJS's
/// `get_date_string` native.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DateStringKind {
    String,
    DateString,
    TimeString,
    UtcString,
    IsoString,
    LocaleString,
    LocaleDateString,
    LocaleTimeString,
}

impl DateStringKind {
    /// Whether `calendar::get_date_fields` must decompose using host local time.
    pub(super) const fn uses_local_time(self) -> bool {
        matches!(
            self,
            Self::String
                | Self::DateString
                | Self::TimeString
                | Self::LocaleString
                | Self::LocaleDateString
                | Self::LocaleTimeString
        )
    }

    const fn format(self) -> Format {
        match self {
            Self::UtcString => Format::Utc,
            Self::String | Self::DateString | Self::TimeString => Format::Local,
            Self::IsoString => Format::Iso,
            Self::LocaleString | Self::LocaleDateString | Self::LocaleTimeString => Format::Locale,
        }
    }

    const fn part(self) -> Part {
        match self {
            Self::DateString | Self::LocaleDateString => Part::Date,
            Self::TimeString | Self::LocaleTimeString => Part::Time,
            Self::String | Self::UtcString | Self::IsoString | Self::LocaleString => Part::All,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Utc,
    Local,
    Iso,
    Locale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Date,
    Time,
    All,
}

impl Part {
    const fn has_date(self) -> bool {
        matches!(self, Self::Date | Self::All)
    }

    const fn has_time(self) -> bool {
        matches!(self, Self::Time | Self::All)
    }
}

/// The one formatter-level exception produced by pinned QuickJS.
///
/// All methods except `toISOString` stringify an invalid payload as
/// `"Invalid Date"`. `toISOString` instead throws a RangeError with this exact
/// message; the runtime layer maps this typed error to that JavaScript error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct InvalidIsoDate;

impl fmt::Display for InvalidIsoDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Date value is NaN")
    }
}

impl std::error::Error for InvalidIsoDate {}

/// Formats already-decomposed fields for a pinned QuickJS Date method.
///
/// `None` represents an invalid Date payload. Callers should use
/// [`DateStringKind::uses_local_time`] when obtaining `fields` from
/// `calendar::get_date_fields`.
pub(super) fn format_date_string(
    fields: Option<&DateFields>,
    kind: DateStringKind,
) -> Result<String, InvalidIsoDate> {
    let Some(fields) = fields else {
        return if kind == DateStringKind::IsoString {
            Err(InvalidIsoDate)
        } else {
            Ok(INVALID_DATE.to_owned())
        };
    };

    let year = fields[0] as i64;
    let month = fields[1] as usize;
    let date = fields[2] as i64;
    let hour = fields[3] as i64;
    let minute = fields[4] as i64;
    let second = fields[5] as i64;
    let millisecond = fields[6] as i64;
    let weekday = fields[7] as usize;
    let timezone = fields[8] as i64;

    debug_assert!(month < MONTH_NAMES.len());
    debug_assert!(weekday < DAY_NAMES.len());

    let format = kind.format();
    let part = kind.part();
    let mut output = String::with_capacity(40);

    if part.has_date() {
        match format {
            Format::Utc => {
                output.push_str(DAY_NAMES[weekday]);
                output.push_str(", ");
                push_two_digits(&mut output, date);
                output.push(' ');
                output.push_str(MONTH_NAMES[month]);
                output.push(' ');
                push_standard_year(&mut output, year);
                output.push(' ');
            }
            Format::Local => {
                output.push_str(DAY_NAMES[weekday]);
                output.push(' ');
                output.push_str(MONTH_NAMES[month]);
                output.push(' ');
                push_two_digits(&mut output, date);
                output.push(' ');
                push_standard_year(&mut output, year);
                if part == Part::All {
                    output.push(' ');
                }
            }
            Format::Iso => {
                push_iso_year(&mut output, year);
                output.push('-');
                push_two_digits(&mut output, month as i64 + 1);
                output.push('-');
                push_two_digits(&mut output, date);
                output.push('T');
            }
            Format::Locale => {
                push_two_digits(&mut output, month as i64 + 1);
                output.push('/');
                push_two_digits(&mut output, date);
                output.push('/');
                push_standard_year(&mut output, year);
                if part == Part::All {
                    output.push_str(", ");
                }
            }
        }
    }

    if part.has_time() {
        match format {
            Format::Utc => {
                push_hms(&mut output, hour, minute, second);
                output.push_str(" GMT");
            }
            Format::Local => {
                push_hms(&mut output, hour, minute, second);
                output.push_str(" GMT");
                push_timezone(&mut output, timezone);
            }
            Format::Iso => {
                push_hms(&mut output, hour, minute, second);
                output.push('.');
                push_three_digits(&mut output, millisecond);
                output.push('Z');
            }
            Format::Locale => {
                let twelve_hour = (hour + 11) % 12 + 1;
                push_hms(&mut output, twelve_hour, minute, second);
                output.push_str(if hour < 12 { " AM" } else { " PM" });
            }
        }
    }

    Ok(output)
}

fn push_hms(output: &mut String, hour: i64, minute: i64, second: i64) {
    push_two_digits(output, hour);
    output.push(':');
    push_two_digits(output, minute);
    output.push(':');
    push_two_digits(output, second);
}

fn push_timezone(output: &mut String, timezone: i64) {
    let (sign, magnitude) = if timezone < 0 {
        ('-', timezone.unsigned_abs())
    } else {
        ('+', timezone as u64)
    };
    output.push(sign);
    push_two_digits(output, (magnitude / 60) as i64);
    push_two_digits(output, (magnitude % 60) as i64);
}

fn push_standard_year(output: &mut String, year: i64) {
    if year < 0 {
        output.push('-');
        push_at_least_digits(output, year.unsigned_abs(), 4);
    } else {
        push_at_least_digits(output, year as u64, 4);
    }
}

fn push_iso_year(output: &mut String, year: i64) {
    if (0..=9999).contains(&year) {
        push_at_least_digits(output, year as u64, 4);
    } else if year < 0 {
        output.push('-');
        push_at_least_digits(output, year.unsigned_abs(), 6);
    } else {
        output.push('+');
        push_at_least_digits(output, year as u64, 6);
    }
}

fn push_two_digits(output: &mut String, value: i64) {
    push_at_least_digits(output, value as u64, 2);
}

fn push_three_digits(output: &mut String, value: i64) {
    push_at_least_digits(output, value as u64, 3);
}

fn push_at_least_digits(output: &mut String, value: u64, width: usize) {
    use fmt::Write as _;

    write!(output, "{value:0width$}").expect("writing to String cannot fail");
}

#[cfg(test)]
mod tests {
    use super::super::calendar::{get_date_fields, set_date_fields};
    use super::*;

    const UTC: fn(i64) -> i32 = |_| 0;

    fn format_utc(time: f64, kind: DateStringKind) -> String {
        let fields = get_date_fields(time, false, false, UTC);
        format_date_string(fields.as_ref(), kind).unwrap()
    }

    #[test]
    fn kinds_select_the_same_utc_or_local_decomposition_as_quickjs_magic() {
        for kind in [DateStringKind::UtcString, DateStringKind::IsoString] {
            assert!(!kind.uses_local_time(), "{kind:?}");
        }
        for kind in [
            DateStringKind::String,
            DateStringKind::DateString,
            DateStringKind::TimeString,
            DateStringKind::LocaleString,
            DateStringKind::LocaleDateString,
            DateStringKind::LocaleTimeString,
        ] {
            assert!(kind.uses_local_time(), "{kind:?}");
        }
    }

    #[test]
    fn all_utc_forms_match_quickjs_at_the_epoch() {
        assert_eq!(
            format_utc(0.0, DateStringKind::UtcString),
            "Thu, 01 Jan 1970 00:00:00 GMT"
        );
        assert_eq!(
            format_utc(0.0, DateStringKind::IsoString),
            "1970-01-01T00:00:00.000Z"
        );
    }

    #[test]
    fn local_string_date_and_time_parts_match_quickjs() {
        let fields = [2020.0, 6.0, 1.0, 18.0, 19.0, 56.0, 789.0, 3.0, 345.0];
        assert_eq!(
            format_date_string(Some(&fields), DateStringKind::String).unwrap(),
            "Wed Jul 01 2020 18:19:56 GMT+0545"
        );
        assert_eq!(
            format_date_string(Some(&fields), DateStringKind::DateString).unwrap(),
            "Wed Jul 01 2020"
        );
        assert_eq!(
            format_date_string(Some(&fields), DateStringKind::TimeString).unwrap(),
            "18:19:56 GMT+0545"
        );
    }

    #[test]
    fn timezone_sign_hours_and_minutes_are_not_rounded_or_reversed() {
        let mut fields = [2020.0, 6.0, 1.0, 10.0, 4.0, 56.0, 0.0, 3.0, -150.0];
        assert_eq!(
            format_date_string(Some(&fields), DateStringKind::TimeString).unwrap(),
            "10:04:56 GMT-0230"
        );

        fields[8] = 0.0;
        assert_eq!(
            format_date_string(Some(&fields), DateStringKind::TimeString).unwrap(),
            "10:04:56 GMT+0000"
        );

        fields[8] = 45.0;
        assert_eq!(
            format_date_string(Some(&fields), DateStringKind::TimeString).unwrap(),
            "10:04:56 GMT+0045"
        );
    }

    #[test]
    fn locale_methods_retain_quickjs_fixed_us_style() {
        let fields = [2020.0, 6.0, 1.0, 18.0, 19.0, 56.0, 789.0, 3.0, 345.0];
        assert_eq!(
            format_date_string(Some(&fields), DateStringKind::LocaleString).unwrap(),
            "07/01/2020, 06:19:56 PM"
        );
        assert_eq!(
            format_date_string(Some(&fields), DateStringKind::LocaleDateString).unwrap(),
            "07/01/2020"
        );
        assert_eq!(
            format_date_string(Some(&fields), DateStringKind::LocaleTimeString).unwrap(),
            "06:19:56 PM"
        );

        let midnight = [2020.0, 6.0, 1.0, 0.0, 1.0, 2.0, 0.0, 3.0, 0.0];
        let noon = [2020.0, 6.0, 1.0, 12.0, 1.0, 2.0, 0.0, 3.0, 0.0];
        assert_eq!(
            format_date_string(Some(&midnight), DateStringKind::LocaleTimeString).unwrap(),
            "12:01:02 AM"
        );
        assert_eq!(
            format_date_string(Some(&noon), DateStringKind::LocaleTimeString).unwrap(),
            "12:01:02 PM"
        );
    }

    #[test]
    fn invalid_date_is_a_string_except_for_iso_range_error() {
        for kind in [
            DateStringKind::String,
            DateStringKind::DateString,
            DateStringKind::TimeString,
            DateStringKind::UtcString,
            DateStringKind::LocaleString,
            DateStringKind::LocaleDateString,
            DateStringKind::LocaleTimeString,
        ] {
            assert_eq!(
                format_date_string(None, kind).unwrap(),
                INVALID_DATE,
                "{kind:?}"
            );
        }

        let error = format_date_string(None, DateStringKind::IsoString).unwrap_err();
        assert_eq!(error, InvalidIsoDate);
        assert_eq!(error.to_string(), "Date value is NaN");
    }

    #[test]
    fn standard_and_iso_year_widths_match_quickjs() {
        let cases = [
            (-1, "-0001", "-000001"),
            (0, "0000", "0000"),
            (9999, "9999", "9999"),
            (10_000, "10000", "+010000"),
            (-10_000, "-10000", "-010000"),
            (-271_821, "-271821", "-271821"),
            (275_760, "275760", "+275760"),
        ];

        for (year, standard_year, iso_year) in cases {
            let fields = [year as f64, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 5.0, 0.0];
            assert_eq!(
                format_date_string(Some(&fields), DateStringKind::DateString).unwrap(),
                format!("Fri Jan 01 {standard_year}")
            );
            assert_eq!(
                format_date_string(Some(&fields), DateStringKind::IsoString).unwrap(),
                format!("{iso_year}-01-01T00:00:00.000Z")
            );
            assert_eq!(
                format_date_string(Some(&fields), DateStringKind::LocaleDateString).unwrap(),
                format!("01/01/{standard_year}")
            );
        }
    }

    #[test]
    fn timeclip_boundaries_format_with_extended_iso_years() {
        assert_eq!(
            format_utc(8.64e15, DateStringKind::UtcString),
            "Sat, 13 Sep 275760 00:00:00 GMT"
        );
        assert_eq!(
            format_utc(8.64e15, DateStringKind::IsoString),
            "+275760-09-13T00:00:00.000Z"
        );
        assert_eq!(
            format_utc(-8.64e15, DateStringKind::UtcString),
            "Tue, 20 Apr -271821 00:00:00 GMT"
        );
        assert_eq!(
            format_utc(-8.64e15, DateStringKind::IsoString),
            "-271821-04-20T00:00:00.000Z"
        );
    }

    #[test]
    fn formatter_consumes_calendar_fields_without_recomputing_the_calendar() {
        let time = set_date_fields(&[-1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0], false, UTC);
        let fields = get_date_fields(time, false, false, UTC).unwrap();
        assert_eq!(
            format_date_string(Some(&fields), DateStringKind::UtcString).unwrap(),
            "Fri, 01 Jan -0001 00:00:00 GMT"
        );
        assert_eq!(
            format_date_string(Some(&fields), DateStringKind::IsoString).unwrap(),
            "-000001-01-01T00:00:00.000Z"
        );
    }
}
