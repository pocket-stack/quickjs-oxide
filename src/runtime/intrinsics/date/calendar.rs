//! Gregorian calendar and time-value kernels for the pinned QuickJS `Date` intrinsic.
//!
//! The algorithms in this module mirror the `Date` block beginning at
//! `quickjs.c:54786` in QuickJS 2026-06-04. Host timezone policy is deliberately
//! injected by callers: the calendar code must not silently turn local-time
//! operations into UTC operations.

const MS_PER_SECOND: i64 = 1_000;
const MS_PER_MINUTE: i64 = 60 * MS_PER_SECOND;
const MS_PER_HOUR: i64 = 60 * MS_PER_MINUTE;
const MS_PER_DAY: i64 = 24 * MS_PER_HOUR;
const TIME_CLIP_BOUND: f64 = 8.64e15;
const I64_MAX_PLUS_ONE: f64 = 9_223_372_036_854_775_808.0;

const MONTH_DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// `[year, month, date, hour, minute, second, millisecond]`.
pub(super) type DateInputFields = [f64; 7];

/// `[year, month, date, hour, minute, second, millisecond, weekday, timezone]`.
///
/// Month is zero based, date is one based, weekday uses Sunday as zero, and
/// timezone is local time minus UTC in minutes. That last sign is the opposite
/// of JavaScript's public `Date.prototype.getTimezoneOffset()` result, exactly
/// as in pinned QuickJS's internal `get_date_fields` array.
pub(super) type DateFields = [f64; 9];

/// Positive modulo used by the pinned calendar decomposition.
pub(super) fn math_mod(value: i64, modulus: i64) -> i64 {
    debug_assert!(modulus > 0);
    let remainder = value % modulus;
    remainder + i64::from(remainder < 0) * modulus
}

/// Integer division rounded toward negative infinity.
pub(super) fn floor_div(value: i64, divisor: i64) -> i64 {
    debug_assert!(divisor > 0);
    let remainder = value % divisor;
    (value - (remainder + i64::from(remainder < 0) * divisor)) / divisor
}

/// Number of days from 1970-01-01 to the start of `year`.
pub(super) fn days_from_year(year: i64) -> i64 {
    365 * (year - 1970) + floor_div(year - 1969, 4) - floor_div(year - 1901, 100)
        + floor_div(year - 1601, 400)
}

pub(super) fn days_in_year(year: i64) -> i64 {
    365 + i64::from(year % 4 == 0) - i64::from(year % 100 == 0) + i64::from(year % 400 == 0)
}

/// Returns the year containing `days` and replaces `days` with its zero-based
/// day within that year.
pub(super) fn year_from_days(days: &mut i64) -> i64 {
    let absolute_days = *days;
    let mut year = floor_div(absolute_days * 10_000, 3_652_425) + 1970;

    // The pinned approximation is already within a few iterations even at
    // TimeClip's limits. Retain its correction loop instead of substituting a
    // different civil-calendar formula.
    loop {
        let day_in_year = absolute_days - days_from_year(year);
        if day_in_year < 0 {
            year -= 1;
        } else {
            let year_length = days_in_year(year);
            if day_in_year < year_length {
                *days = day_in_year;
                return year;
            }
            year += 1;
        }
    }
}

/// Decomposes a clipped ECMAScript time value using pinned QuickJS semantics.
///
/// The provider returns the public JavaScript timezone offset (UTC minus local)
/// in minutes. It is consulted only for a finite local-time decomposition;
/// forced decomposition of `NaN` intentionally stays at UTC epoch fields, as
/// in upstream `get_date_fields`.
pub(super) fn get_date_fields<F>(
    date_value: f64,
    is_local: bool,
    force: bool,
    mut timezone_offset: F,
) -> Option<DateFields>
where
    F: FnMut(i64) -> i32,
{
    let (time, timezone_minutes) = if date_value.is_nan() {
        if !force {
            return None;
        }
        (0_i64, 0_i64)
    } else {
        // Date payloads reaching this kernel have already gone through
        // TimeClip, so the cast is within i64's exact integer range.
        let mut time = date_value as i64;
        let timezone_minutes = if is_local {
            -i64::from(timezone_offset(time))
        } else {
            0
        };
        time += timezone_minutes * MS_PER_MINUTE;
        (time, timezone_minutes)
    };

    let mut time_within_day = math_mod(time, MS_PER_DAY);
    let mut days = (time - time_within_day) / MS_PER_DAY;

    let millisecond = time_within_day % MS_PER_SECOND;
    time_within_day = (time_within_day - millisecond) / MS_PER_SECOND;
    let second = time_within_day % 60;
    time_within_day = (time_within_day - second) / 60;
    let minute = time_within_day % 60;
    let hour = (time_within_day - minute) / 60;

    let weekday = math_mod(days + 4, 7);
    let year = year_from_days(&mut days);

    let mut month = 0_usize;
    while month < 11 {
        let mut month_length = MONTH_DAYS[month];
        if month == 1 {
            month_length += days_in_year(year) - 365;
        }
        if days < month_length {
            break;
        }
        days -= month_length;
        month += 1;
    }

    Some([
        year as f64,
        month as f64,
        (days + 1) as f64,
        hour as f64,
        minute as f64,
        second as f64,
        millisecond as f64,
        weekday as f64,
        timezone_minutes as f64,
    ])
}

/// ECMAScript TimeClip as implemented by pinned QuickJS.
pub(super) fn time_clip(time: f64) -> f64 {
    if (-TIME_CLIP_BOUND..=TIME_CLIP_BOUND).contains(&time) {
        let clipped = time.trunc();
        // Upstream uses `trunc(t) + 0.0`; spell the signed-zero consequence out
        // so it cannot be optimized into retaining negative zero.
        if clipped == 0.0 { 0.0 } else { clipped }
    } else {
        f64::NAN
    }
}

/// Safe optimization barrier corresponding to upstream's volatile `temp`.
///
/// Ordinary Rust floating-point operators are non-contracting, and the opaque
/// boundary also prevents the MakeTime/MakeDate steps from being reassociated
/// or folded into a fused multiply-add by later optimization changes.
#[inline(never)]
fn fp_step(value: f64) -> f64 {
    std::hint::black_box(value)
}

/// Pinned QuickJS's `set_date_fields`: MakeDay, MakeTime, MakeDate, optional
/// local-to-UTC adjustment, then TimeClip.
///
/// `timezone_offset` has the same sign as the JavaScript
/// `getTimezoneOffset()` result (UTC minus local) and is not called for UTC.
pub(super) fn set_date_fields<F>(
    fields: &DateInputFields,
    is_local: bool,
    mut timezone_offset: F,
) -> f64
where
    F: FnMut(i64) -> i32,
{
    // 21.4.1.15 MakeDay(year, month, date).
    let year = fields[0];
    let month = fields[1];
    let date = fields[2];
    let normalized_year = fp_step(year + (month / 12.0).floor());
    let mut normalized_month = month % 12.0;
    if normalized_month < 0.0 {
        normalized_month += 12.0;
    }
    if !normalized_year.is_finite()
        || !normalized_month.is_finite()
        || normalized_year < -271_821.0
        || normalized_year > 275_760.0
    {
        return f64::NAN;
    }

    let integer_year = normalized_year as i64;
    let integer_month = normalized_month as usize;
    let mut days = days_from_year(integer_year);
    for (month_index, month_length) in MONTH_DAYS.iter().enumerate().take(integer_month) {
        days += month_length;
        if month_index == 1 {
            days += days_in_year(integer_year) - 365;
        }
    }
    let day = fp_step(fp_step(days as f64 + date) - 1.0);

    // 21.4.1.14 MakeTime(hour, minute, second, millisecond). Keep every
    // multiplication and left-to-right addition at an explicit f64 boundary.
    let mut time = fp_step(fields[3] * MS_PER_HOUR as f64);
    let minute_part = fp_step(fields[4] * MS_PER_MINUTE as f64);
    time = fp_step(time + minute_part);
    let second_part = fp_step(fields[5] * MS_PER_SECOND as f64);
    time = fp_step(time + second_part);
    time = fp_step(time + fields[6]);

    // 21.4.1.16 MakeDate(day, time). The separate day product prevents FMA.
    let day_part = fp_step(day * MS_PER_DAY as f64);
    let mut time_value = fp_step(day_part + time);
    if !time_value.is_finite() {
        return f64::NAN;
    }

    if is_local {
        // Match the pinned C comparison exactly: 2^63 maps to i64::MAX, while
        // -2^63 is representable and maps to i64::MIN.
        let offset_time = if time_value < i64::MIN as f64 {
            i64::MIN
        } else if time_value >= I64_MAX_PLUS_ONE {
            i64::MAX
        } else {
            time_value as i64
        };
        let adjustment = i64::from(timezone_offset(offset_time)) * MS_PER_MINUTE;
        time_value = fp_step(time_value + adjustment as f64);
    }

    time_clip(time_value)
}

/// Converts and validates constructor/setter fields before delegating to
/// [`set_date_fields`], including Date's special 0..99 year interpretation.
pub(super) fn set_date_fields_checked<F>(
    mut fields: DateInputFields,
    is_local: bool,
    timezone_offset: F,
) -> f64
where
    F: FnMut(i64) -> i32,
{
    for (index, field) in fields.iter_mut().enumerate() {
        if !field.is_finite() {
            return f64::NAN;
        }
        *field = field.trunc();
        if index == 0 && (0.0..100.0).contains(field) {
            *field += 1900.0;
        }
    }
    set_date_fields(&fields, is_local, timezone_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UTC: fn(i64) -> i32 = |_| 0;

    fn assert_fields(actual: DateFields, expected: DateFields) {
        assert_eq!(actual, expected);
    }

    #[test]
    fn signed_modulo_and_floor_division_match_quickjs() {
        assert_eq!(math_mod(8, 7), 1);
        assert_eq!(math_mod(-1, 7), 6);
        assert_eq!(math_mod(-8, 7), 6);
        assert_eq!(floor_div(8, 7), 1);
        assert_eq!(floor_div(-1, 7), -1);
        assert_eq!(floor_div(-8, 7), -2);
    }

    #[test]
    fn year_math_handles_epoch_leaps_and_negative_years() {
        assert_eq!(days_from_year(1970), 0);
        assert_eq!(days_from_year(1969), -365);
        assert_eq!(days_from_year(2000), 10_957);
        assert_eq!(days_in_year(2000), 366);
        assert_eq!(days_in_year(1900), 365);
        assert_eq!(days_in_year(-4), 366);
        assert_eq!(days_in_year(-100), 365);
        assert_eq!(days_in_year(-400), 366);

        let mut before_epoch = -1;
        assert_eq!(year_from_days(&mut before_epoch), 1969);
        assert_eq!(before_epoch, 364);

        let mut leap_day = days_from_year(2000) + 59;
        assert_eq!(year_from_days(&mut leap_day), 2000);
        assert_eq!(leap_day, 59);
    }

    #[test]
    fn utc_decomposition_covers_epoch_negative_time_and_leap_day() {
        assert_fields(
            get_date_fields(0.0, false, false, UTC).unwrap(),
            [1970.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0],
        );
        assert_fields(
            get_date_fields(-1.0, false, false, UTC).unwrap(),
            [1969.0, 11.0, 31.0, 23.0, 59.0, 59.0, 999.0, 3.0, 0.0],
        );

        let leap_day =
            set_date_fields_checked([2000.0, 1.0, 29.0, 12.0, 34.0, 56.0, 789.0], false, UTC);
        assert_fields(
            get_date_fields(leap_day, false, false, UTC).unwrap(),
            [2000.0, 1.0, 29.0, 12.0, 34.0, 56.0, 789.0, 2.0, 0.0],
        );
    }

    #[test]
    fn timeclip_endpoints_round_trip_to_ecmascript_calendar_limits() {
        assert_fields(
            get_date_fields(TIME_CLIP_BOUND, false, false, UTC).unwrap(),
            [275_760.0, 8.0, 13.0, 0.0, 0.0, 0.0, 0.0, 6.0, 0.0],
        );
        assert_fields(
            get_date_fields(-TIME_CLIP_BOUND, false, false, UTC).unwrap(),
            [-271_821.0, 3.0, 20.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0],
        );

        assert_eq!(
            set_date_fields_checked([275_760.0, 8.0, 13.0, 0.0, 0.0, 0.0, 0.0], false, UTC,),
            TIME_CLIP_BOUND
        );
        assert!(
            set_date_fields_checked([275_760.0, 8.0, 14.0, 0.0, 0.0, 0.0, 0.0], false, UTC,)
                .is_nan()
        );
        assert_eq!(
            set_date_fields_checked([-271_821.0, 3.0, 20.0, 0.0, 0.0, 0.0, 0.0], false, UTC,),
            -TIME_CLIP_BOUND
        );
        assert!(
            set_date_fields_checked([-271_821.0, 3.0, 19.0, 0.0, 0.0, 0.0, 0.0], false, UTC,)
                .is_nan()
        );
    }

    #[test]
    fn time_clip_truncates_and_canonicalizes_negative_zero() {
        assert_eq!(time_clip(12.9), 12.0);
        assert_eq!(time_clip(-12.9), -12.0);
        assert_eq!(time_clip(TIME_CLIP_BOUND), TIME_CLIP_BOUND);
        assert_eq!(time_clip(-TIME_CLIP_BOUND), -TIME_CLIP_BOUND);
        assert!(time_clip(TIME_CLIP_BOUND + 1.0).is_nan());
        assert!(time_clip(-TIME_CLIP_BOUND - 1.0).is_nan());
        assert!(time_clip(f64::INFINITY).is_nan());
        assert!(time_clip(f64::NEG_INFINITY).is_nan());
        assert!(time_clip(f64::NAN).is_nan());
        assert_eq!(time_clip(-0.0).to_bits(), 0.0_f64.to_bits());
        assert_eq!(time_clip(-0.9).to_bits(), 0.0_f64.to_bits());
    }

    #[test]
    fn local_operations_use_injected_timezone_with_quickjs_signs() {
        let mut get_call = None;
        let local_fields = get_date_fields(0.0, true, false, |time| {
            get_call = Some(time);
            -480
        })
        .unwrap();
        assert_eq!(get_call, Some(0));
        assert_fields(
            local_fields,
            [1970.0, 0.0, 1.0, 8.0, 0.0, 0.0, 0.0, 4.0, 480.0],
        );

        let mut set_call = None;
        let utc_value =
            set_date_fields_checked([1970.0, 0.0, 1.0, 8.0, 0.0, 0.0, 0.0], true, |time| {
                set_call = Some(time);
                -480
            });
        assert_eq!(set_call, Some(8 * MS_PER_HOUR));
        assert_eq!(utc_value, 0.0);
    }

    #[test]
    fn forced_nan_decomposition_does_not_consult_host_timezone() {
        let mut calls = 0;
        assert!(
            get_date_fields(f64::NAN, true, false, |_| {
                calls += 1;
                -480
            })
            .is_none()
        );
        assert_eq!(calls, 0);

        assert_fields(
            get_date_fields(f64::NAN, true, true, |_| {
                calls += 1;
                -480
            })
            .unwrap(),
            [1970.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0],
        );
        assert_eq!(calls, 0);
    }

    #[test]
    fn checked_fields_truncate_and_apply_the_legacy_two_digit_year_rule() {
        assert_eq!(
            set_date_fields_checked([0.9, 0.9, 1.9, 0.9, 0.9, 0.9, 0.9], false, UTC),
            -2_208_988_800_000.0
        );
        assert!(
            set_date_fields_checked([1970.0, 0.0, 1.0, 0.0, f64::NAN, 0.0, 0.0], false, UTC,)
                .is_nan()
        );
    }

    #[test]
    fn month_normalization_and_proleptic_leaps_match_quickjs() {
        let december_1969 =
            set_date_fields_checked([1970.0, -1.0, 1.0, 0.0, 0.0, 0.0, 0.0], false, UTC);
        assert_fields(
            get_date_fields(december_1969, false, false, UTC).unwrap(),
            [1969.0, 11.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        );

        let normalized_1900 =
            set_date_fields_checked([1900.0, 1.0, 29.0, 0.0, 0.0, 0.0, 0.0], false, UTC);
        assert_fields(
            get_date_fields(normalized_1900, false, false, UTC).unwrap(),
            [1900.0, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0],
        );

        let negative_year = set_date_fields(&[-1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0], false, UTC);
        assert_fields(
            get_date_fields(negative_year, false, false, UTC).unwrap(),
            [-1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 5.0, 0.0],
        );
    }

    #[test]
    fn maketime_and_makedate_preserve_specified_fp_evaluation_order() {
        // test262: built-ins/Date/UTC/fp-evaluation-order.js
        assert_eq!(
            set_date_fields_checked(
                [
                    1970.0,
                    0.0,
                    1.0,
                    80_063_993_375.0,
                    29.0,
                    1.0,
                    -288_230_376_151_711_740.0,
                ],
                false,
                UTC,
            ),
            29_312.0,
            "MakeTime must use left-to-right IEEE-754 operations"
        );
        assert_eq!(
            set_date_fields_checked(
                [
                    1970.0,
                    0.0,
                    213_503_982_336.0,
                    0.0,
                    0.0,
                    0.0,
                    -18_446_744_073_709_552_000.0,
                ],
                false,
                UTC,
            ),
            34_447_360.0,
            "MakeDate must round the day product before addition"
        );
    }

    #[test]
    fn local_offset_lookup_saturates_out_of_range_intermediate_times() {
        let mut positive_lookup = None;
        assert!(
            set_date_fields(
                &[1970.0, 0.0, 110_000_000_000.0, 0.0, 0.0, 0.0, 0.0],
                true,
                |time| {
                    positive_lookup = Some(time);
                    0
                },
            )
            .is_nan()
        );
        assert_eq!(positive_lookup, Some(i64::MAX));

        let mut negative_lookup = None;
        assert!(
            set_date_fields(
                &[1970.0, 0.0, -110_000_000_000.0, 0.0, 0.0, 0.0, 0.0],
                true,
                |time| {
                    negative_lookup = Some(time);
                    0
                },
            )
            .is_nan()
        );
        assert_eq!(negative_lookup, Some(i64::MIN));
    }
}
