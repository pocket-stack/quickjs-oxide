//! Pinned QuickJS 2026-06-04 `Date` intrinsic.
//!
//! Calendar, parsing, formatting, and host policy stay in narrow modules;
//! constructor and prototype handlers meet at one typed native-dispatch edge.

mod calendar;
mod constructor;
mod format;
mod host;
mod parse;
mod prototype;

pub(in crate::runtime) use host::{DateHost, SystemDateHost};

use super::*;

impl Runtime {
    /// Install the complete pinned `js_date_funcs` and
    /// `js_date_proto_funcs` tables. `%Date.prototype%` is an ordinary object
    /// without a Date payload, exactly as in upstream.
    pub(in crate::runtime) fn initialize_date_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        date_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        // Source-table order matters even though OrdinaryOwnPropertyKeys later
        // groups the symbol after all String keys.
        for (kind, name) in [
            (DateNativeKind::TimeValue, "valueOf"),
            (
                DateNativeKind::String(DateStringMethod::ToString),
                "toString",
            ),
        ] {
            self.define_native_builtin_auto_init(
                date_prototype,
                realm,
                NativeFunctionId::Date(kind),
                name,
                kind.length(),
                kind.length(),
            )?;
        }

        let to_primitive = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToPrimitive));
        let to_primitive_kind = DateNativeKind::ToPrimitive;
        self.define_native_builtin_auto_init_with_key(
            date_prototype,
            realm,
            &to_primitive,
            NativeFunctionId::Date(to_primitive_kind),
            "[Symbol.toPrimitive]",
            to_primitive_kind.length(),
            to_primitive_kind.length(),
            PropertyFlags::data(false, false, true),
        )?;

        let utc_string_kind = DateNativeKind::String(DateStringMethod::ToUtcString);
        self.define_native_builtin_auto_init(
            date_prototype,
            realm,
            NativeFunctionId::Date(utc_string_kind),
            "toUTCString",
            utc_string_kind.length(),
            utc_string_kind.length(),
        )?;
        // QuickJS explicitly materializes aliases instead of putting an
        // AutoInit alias in the shape. Preserve both identity and the aliased
        // function's original `name`.
        let utc_string_key = self.intern_property_key("toUTCString")?;
        let utc_string = match self.get_property_in_realm(realm, date_prototype, &utc_string_key)? {
            Completion::Return(value @ Value::Object(_)) => value,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "Date.prototype.toUTCString did not materialize as an object",
                ));
            }
            Completion::Throw(_) => {
                return Err(RuntimeError::Invariant(
                    "Date.prototype.toUTCString materialization threw",
                ));
            }
        };
        self.define_function_data_property(date_prototype, "toGMTString", utc_string, true, true)?;

        for (kind, name) in [
            (
                DateNativeKind::String(DateStringMethod::ToIsoString),
                "toISOString",
            ),
            (
                DateNativeKind::String(DateStringMethod::ToDateString),
                "toDateString",
            ),
            (
                DateNativeKind::String(DateStringMethod::ToTimeString),
                "toTimeString",
            ),
            (
                DateNativeKind::String(DateStringMethod::ToLocaleString),
                "toLocaleString",
            ),
            (
                DateNativeKind::String(DateStringMethod::ToLocaleDateString),
                "toLocaleDateString",
            ),
            (
                DateNativeKind::String(DateStringMethod::ToLocaleTimeString),
                "toLocaleTimeString",
            ),
            (DateNativeKind::TimezoneOffset, "getTimezoneOffset"),
            (DateNativeKind::TimeValue, "getTime"),
        ] {
            self.define_native_builtin_auto_init(
                date_prototype,
                realm,
                NativeFunctionId::Date(kind),
                name,
                kind.length(),
                kind.length(),
            )?;
        }
        for kind in DateGetFieldKind::ALL {
            let target = DateNativeKind::GetField(kind);
            self.define_native_builtin_auto_init(
                date_prototype,
                realm,
                NativeFunctionId::Date(target),
                kind.name(),
                target.length(),
                target.length(),
            )?;
        }

        let set_time = DateNativeKind::SetTime;
        self.define_native_builtin_auto_init(
            date_prototype,
            realm,
            NativeFunctionId::Date(set_time),
            "setTime",
            set_time.length(),
            set_time.length(),
        )?;
        for kind in DateSetFieldKind::ALL.into_iter().take(12) {
            let target = DateNativeKind::SetField(kind);
            self.define_native_builtin_auto_init(
                date_prototype,
                realm,
                NativeFunctionId::Date(target),
                kind.name(),
                target.length(),
                target.length(),
            )?;
        }
        let set_year = DateNativeKind::SetYear;
        self.define_native_builtin_auto_init(
            date_prototype,
            realm,
            NativeFunctionId::Date(set_year),
            "setYear",
            set_year.length(),
            set_year.length(),
        )?;
        for kind in [DateSetFieldKind::FullYear, DateSetFieldKind::UtcFullYear] {
            let target = DateNativeKind::SetField(kind);
            self.define_native_builtin_auto_init(
                date_prototype,
                realm,
                NativeFunctionId::Date(target),
                kind.name(),
                target.length(),
                target.length(),
            )?;
        }
        let to_json = DateNativeKind::ToJson;
        self.define_native_builtin_auto_init(
            date_prototype,
            realm,
            NativeFunctionId::Date(to_json),
            "toJSON",
            to_json.length(),
            to_json.length(),
        )?;

        let constructor_kind = DateNativeKind::Constructor;
        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::Date(constructor_kind),
            constructor_kind.length(),
            "Date",
            i32::from(constructor_kind.length()),
        )?;
        for (kind, name) in [
            (DateNativeKind::Now, "now"),
            (DateNativeKind::Parse, "parse"),
            (DateNativeKind::Utc, "UTC"),
        ] {
            self.define_native_builtin_auto_init(
                constructor.as_object(),
                realm,
                NativeFunctionId::Date(kind),
                name,
                kind.length(),
                kind.length(),
            )?;
        }
        self.define_function_data_property(
            global_object,
            "Date",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, date_prototype)
    }

    pub(in crate::runtime) fn call_date_native(
        &self,
        realm: ContextId,
        kind: DateNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            DateNativeKind::Constructor
            | DateNativeKind::Now
            | DateNativeKind::Parse
            | DateNativeKind::Utc => {
                self.call_date_constructor_native(realm, kind, invocation, arguments)
            }
            _ => self.call_date_prototype_native(realm, kind, invocation, arguments),
        }
    }

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
