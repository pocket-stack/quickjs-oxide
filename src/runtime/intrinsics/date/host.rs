//! Host clock and time-zone boundary for the pinned QuickJS `Date` intrinsic.
//!
//! QuickJS keeps its calendar algorithms inside the engine but delegates two
//! values to the host: the current Unix time and the UTC offset in effect at a
//! particular Unix instant.  Keeping that boundary typed lets unit tests use a
//! deterministic provider without replacing any of QuickJS's Date semantics
//! with a third-party calendar implementation.

use std::cell::RefCell;
use std::env;
use std::ffi::{OsStr, OsString};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tz::TimeZone;

/// Runtime-wide host services used by Date algorithms.
pub(in crate::runtime) trait DateHost: std::fmt::Debug {
    /// Milliseconds since the Unix epoch, with pre-epoch fractions rounded in
    /// the same direction as POSIX `gettimeofday` plus QuickJS's integer math.
    fn now_millis(&self) -> i64;

    /// QuickJS/ECMAScript offset convention: UTC minus local time, in minutes.
    fn timezone_offset_minutes(&self, epoch_millis: i64) -> i32;
}

/// Production provider backed by `SystemTime` and the host TZif/POSIX rules.
#[derive(Debug, Default)]
pub(in crate::runtime) struct SystemDateHost {
    cached_time_zone: RefCell<Option<CachedTimeZone>>,
}

#[derive(Debug)]
struct CachedTimeZone {
    environment: Option<OsString>,
    time_zone: TimeZone,
}

impl DateHost for SystemDateHost {
    fn now_millis(&self) -> i64 {
        system_time_millis(SystemTime::now())
    }

    fn timezone_offset_minutes(&self, epoch_millis: i64) -> i32 {
        let unix_seconds = epoch_millis / 1_000;
        let environment = env::var_os("TZ");

        // With no explicit TZ, libc's localtime path consults the host's
        // current local-zone configuration. Reload `/etc/localtime` instead
        // of freezing its first snapshot for the Runtime's whole lifetime, so
        // replacing the host configuration remains observable like QuickJS.
        if environment.is_none() {
            return timezone_offset_at(
                &load_host_time_zone(None).unwrap_or_else(|_| TimeZone::utc()),
                unix_seconds,
            );
        }

        let mut cached = self.cached_time_zone.borrow_mut();
        if cached
            .as_ref()
            .is_none_or(|cached| cached.environment != environment)
        {
            let time_zone = load_host_time_zone(environment.as_deref())
                // QuickJS's Windows path also falls back to zero when the host
                // conversion fails. The POSIX path assumes localtime_r works.
                .unwrap_or_else(|_| TimeZone::utc());
            *cached = Some(CachedTimeZone {
                environment: environment.clone(),
                time_zone,
            });
        }
        timezone_offset_at(
            &cached
                .as_ref()
                .expect("Date timezone cache was initialized above")
                .time_zone,
            unix_seconds,
        )
    }
}

fn timezone_offset_at(time_zone: &TimeZone, unix_seconds: i64) -> i32 {
    time_zone
        .find_local_time_type(unix_seconds)
        .ok()
        .map(|local| -local.ut_offset() / 60)
        // Retain the host-conversion failure fallback at the typed boundary;
        // ordinary TZif/POSIX lookups do not take this path.
        .unwrap_or(0)
}

fn load_host_time_zone(environment: Option<&OsStr>) -> Result<TimeZone, tz::Error> {
    match environment {
        Some(value) if value.is_empty() => Ok(TimeZone::utc()),
        Some(value) => value
            .to_str()
            .map_or_else(TimeZone::local, TimeZone::from_posix_tz),
        None => TimeZone::local(),
    }
}

fn system_time_millis(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration_millis_saturating(duration),
        Err(error) => {
            let duration = error.duration();
            let whole = duration_millis_saturating(duration);
            let fractional = i64::from(duration.subsec_nanos() % 1_000_000 != 0);
            whole.saturating_add(fractional).saturating_neg()
        }
    }
}

fn duration_millis_saturating(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FixedDateHost {
        now_millis: i64,
        timezone_offset_minutes: i32,
    }

    impl DateHost for FixedDateHost {
        fn now_millis(&self) -> i64 {
            self.now_millis
        }

        fn timezone_offset_minutes(&self, _epoch_millis: i64) -> i32 {
            self.timezone_offset_minutes
        }
    }

    #[test]
    fn pre_epoch_submillisecond_time_rounds_like_gettimeofday() {
        assert_eq!(system_time_millis(UNIX_EPOCH), 0);
        assert_eq!(
            system_time_millis(UNIX_EPOCH.checked_sub(Duration::from_nanos(1)).unwrap()),
            -1
        );
        assert_eq!(
            system_time_millis(
                UNIX_EPOCH
                    .checked_sub(Duration::from_micros(999_999))
                    .unwrap()
            ),
            -1_000
        );
        assert_eq!(
            system_time_millis(UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap()),
            -1_000
        );
        assert_eq!(
            system_time_millis(UNIX_EPOCH.checked_add(Duration::from_micros(999)).unwrap()),
            0
        );
    }

    #[test]
    fn fixed_provider_keeps_clock_and_offset_injectable() {
        let host = FixedDateHost {
            now_millis: -123,
            timezone_offset_minutes: 480,
        };
        assert_eq!(host.now_millis(), -123);
        assert_eq!(host.timezone_offset_minutes(i64::MAX), 480);
    }

    #[test]
    fn explicit_timezone_loading_covers_empty_and_posix_forms() {
        let offset = |time_zone: &TimeZone, unix_seconds| {
            -time_zone
                .find_local_time_type(unix_seconds)
                .unwrap()
                .ut_offset()
                / 60
        };
        assert_eq!(
            offset(&load_host_time_zone(Some(OsStr::new(""))).unwrap(), 0),
            0
        );
        assert_eq!(
            offset(&load_host_time_zone(Some(OsStr::new("UTC0"))).unwrap(), 0),
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn tz_rules_use_quickjs_offset_sign_and_historical_instants() {
        let shanghai = TimeZone::from_posix_tz("Asia/Shanghai").unwrap();
        let new_york = TimeZone::from_posix_tz("America/New_York").unwrap();

        let js_offset = |time_zone: &TimeZone, unix_seconds| {
            -time_zone
                .find_local_time_type(unix_seconds)
                .unwrap()
                .ut_offset()
                / 60
        };

        assert_eq!(js_offset(&shanghai, 0), -480);
        assert_eq!(js_offset(&new_york, 1_609_459_200), 300);
        assert_eq!(js_offset(&new_york, 1_625_097_600), 240);
    }

    #[cfg(unix)]
    #[test]
    fn explicit_iana_timezone_uses_the_system_zoneinfo_database() {
        let shanghai = load_host_time_zone(Some(OsStr::new("Asia/Shanghai"))).unwrap();
        let offset = -shanghai.find_local_time_type(0).unwrap().ut_offset() / 60;
        assert_eq!(offset, -480);
    }

    #[test]
    fn millisecond_to_second_conversion_truncates_toward_zero() {
        assert_eq!(-999_i64 / 1_000, 0);
        assert_eq!(-1_001_i64 / 1_000, -1);
        assert_eq!(999_i64 / 1_000, 0);
        assert_eq!(1_001_i64 / 1_000, 1);
    }
}
