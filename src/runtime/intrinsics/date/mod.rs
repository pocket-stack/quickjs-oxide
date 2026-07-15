//! Date kernels and host boundary, ported from pinned QuickJS 2026-06-04.
//!
//! This module intentionally lands before the observable intrinsic table so
//! the calendar, parser, formatter, payload, and host boundary can be reviewed
//! independently. Remove this allowance when the Date native dispatch is
//! connected in the next milestone.

#![allow(dead_code)]

mod calendar;
mod format;
mod host;
mod parse;

pub(in crate::runtime) use host::{DateHost, SystemDateHost};

use super::*;

impl Runtime {
    pub(super) fn date_now_millis(&self) -> i64 {
        self.0.date_host.now_millis()
    }

    pub(super) fn date_timezone_offset_minutes(&self, epoch_millis: i64) -> i32 {
        self.0.date_host.timezone_offset_minutes(epoch_millis)
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

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
    fn runtime_owns_an_injectable_date_host() {
        let runtime = Runtime::new_with_date_host(Rc::new(FixedDateHost {
            now_millis: 42,
            timezone_offset_minutes: -480,
        }));

        assert_eq!(runtime.date_now_millis(), 42);
        assert_eq!(runtime.date_timezone_offset_minutes(i64::MAX), -480);
    }
}
