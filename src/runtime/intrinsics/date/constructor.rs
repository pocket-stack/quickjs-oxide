//! Pinned QuickJS `Date` constructor and static native handlers.
//!
//! The observable conversion order in this file follows QuickJS 2026-06-04
//! `js_date_constructor`, `js_Date_UTC`, `js_Date_parse`, and `js_Date_now`.
//! In particular, a function call ignores every supplied argument, the
//! multi-argument forms coerce at most seven values from left to right before
//! inspecting finiteness, and parsing applies its explicit offset after the
//! calendar kernel's TimeClip without clipping the static `Date.parse` result a
//! second time.

use crate::heap::DateNativeKind;

use super::super::super::*;
use super::calendar::{
    DateInputFields, get_date_fields, set_date_fields, set_date_fields_checked, time_clip,
};
use super::format::{DateStringKind, format_date_string};
use super::parse::{ParsedDateString, parse_date_string};

const DEFAULT_DATE_FIELDS: DateInputFields = [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
const MAX_DATE_ARGUMENTS: usize = DEFAULT_DATE_FIELDS.len();
const MS_PER_MINUTE: f64 = 60_000.0;

impl Runtime {
    /// Dispatch the constructor and three static functions represented by the
    /// constructor-side portion of [`DateNativeKind`]. Prototype operations
    /// are deliberately rejected here so `date/mod.rs` remains the sole
    /// top-level Date dispatcher.
    pub(in crate::runtime) fn call_date_constructor_native(
        &self,
        realm: ContextId,
        kind: DateNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            DateNativeKind::Constructor => self.call_date_constructor(realm, invocation, arguments),
            DateNativeKind::Now | DateNativeKind::Parse | DateNativeKind::Utc => {
                let NativeInvocation::Call { .. } = invocation else {
                    return Err(RuntimeError::Invariant(
                        "Date static function did not receive a generic invocation",
                    ));
                };
                match kind {
                    DateNativeKind::Now => self.call_date_now(),
                    DateNativeKind::Parse => self.call_date_parse(realm, arguments),
                    DateNativeKind::Utc => self.call_date_utc(realm, arguments),
                    _ => unreachable!("outer match selected a Date static function"),
                }
            }
            _ => Err(RuntimeError::Invariant(
                "Date prototype native reached the constructor/static dispatcher",
            )),
        }
    }

    fn call_date_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Date constructor did not receive constructor-or-function invocation",
            ));
        };

        // `JS_CFUNC_constructor_or_func` represents an ordinary function call
        // with an undefined new.target. Pinned QuickJS sets argc to zero before
        // looking at argv, so even proxies, Symbols, and throwing conversion
        // hooks supplied as arguments are completely ignored.
        if matches!(new_target, Value::Undefined) {
            return self.call_date_as_function();
        }

        let date_value = match self.date_constructor_value(realm, arguments)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let prototype = match self.date_prototype_from_new_target(realm, new_target)? {
            NativeConversion::Value(prototype) => prototype,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let date = self.new_date_object(&prototype, date_value)?;
        Ok(Completion::Return(Value::Object(date)))
    }

    fn call_date_as_function(&self) -> Result<Completion, RuntimeError> {
        let date_value = self.date_now_millis() as f64;
        let fields = get_date_fields(date_value, true, false, |epoch_millis| {
            self.date_timezone_offset_minutes(epoch_millis)
        });
        let text = format_date_string(fields.as_ref(), DateStringKind::String).map_err(|_| {
            RuntimeError::Invariant("the host clock produced an invalid Date string")
        })?;
        Ok(Completion::Return(Value::String(JsString::try_from_utf8(
            &text,
        )?)))
    }

    fn date_constructor_value(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<NativeConversion<f64>, RuntimeError> {
        match arguments.actual_arg_count {
            0 => Ok(NativeConversion::Value(self.date_now_millis() as f64)),
            1 => {
                let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "Date constructor argv was not readable",
                ))?;
                if let Some(value) = self.genuine_date_value(argument)? {
                    return Ok(NativeConversion::Value(time_clip(value)));
                }

                // The one-argument form performs ToPrimitive with the default
                // hint exactly once. A String goes through Date.parse; every
                // other primitive goes through ToNumber.
                let primitive =
                    match self.to_primitive(realm, argument.clone(), ToPrimitiveHint::Default)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
                    };
                let value = if let Value::String(string) = primitive {
                    parsed_date_value(parse_date_string(&string), |epoch_millis| {
                        self.date_timezone_offset_minutes(epoch_millis)
                    })
                } else {
                    match self.native_to_number(realm, &primitive)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(NativeConversion::Throw(value));
                        }
                    }
                };
                Ok(NativeConversion::Value(time_clip(value)))
            }
            _ => {
                let fields = match self.date_numeric_fields(realm, arguments)? {
                    NativeConversion::Value(fields) => fields,
                    NativeConversion::Throw(value) => {
                        return Ok(NativeConversion::Throw(value));
                    }
                };
                Ok(NativeConversion::Value(set_date_fields_checked(
                    fields,
                    true,
                    |epoch_millis| self.date_timezone_offset_minutes(epoch_millis),
                )))
            }
        }
    }

    fn date_numeric_fields(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<NativeConversion<DateInputFields>, RuntimeError> {
        let count = arguments.actual_arg_count.min(MAX_DATE_ARGUMENTS);
        if arguments.readable.len() < count {
            return Err(RuntimeError::Invariant(
                "Date numeric argv was shorter than the actual argument count",
            ));
        }
        let mut fields = DEFAULT_DATE_FIELDS;
        for (field, argument) in fields.iter_mut().zip(arguments.readable.iter()).take(count) {
            *field = match self.native_to_number(realm, argument)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
        }
        Ok(NativeConversion::Value(fields))
    }

    fn call_date_now(&self) -> Result<Completion, RuntimeError> {
        Ok(Completion::Return(Value::number(
            self.date_now_millis() as f64
        )))
    }

    fn call_date_parse(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let argument = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant("Date.parse argv was not padded"))?;
        let string = match self.native_to_js_string(realm, argument)? {
            NativeConversion::Value(string) => string,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let value = parsed_date_value(parse_date_string(&string), |epoch_millis| {
            self.date_timezone_offset_minutes(epoch_millis)
        });
        Ok(Completion::Return(Value::number(value)))
    }

    fn call_date_utc(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        if arguments.actual_arg_count == 0 {
            return Ok(Completion::Return(Value::Float(f64::NAN)));
        }
        let fields = match self.date_numeric_fields(realm, arguments)? {
            NativeConversion::Value(fields) => fields,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::number(set_date_fields_checked(
            fields,
            false,
            |_| 0,
        ))))
    }

    fn genuine_date_value(&self, value: &Value) -> Result<Option<f64>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(None);
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Date argument"));
        }
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        Ok(match &object.payload {
            ObjectPayload::Date(value) => Some(*value),
            _ => None,
        })
    }

    fn date_prototype_from_new_target(
        &self,
        realm: ContextId,
        new_target: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(new_target_object) = new_target else {
            return Err(RuntimeError::Invariant(
                "Date constructor new.target was not an object",
            ));
        };
        let prototype_key = self.intern_property_key("prototype")?;
        match self.get_property_in_realm(realm, &new_target_object, &prototype_key)? {
            Completion::Return(Value::Object(prototype)) => Ok(NativeConversion::Value(prototype)),
            Completion::Return(_) => {
                // QuickJS asks for the constructor realm only after the
                // observable `.prototype` Get returned a non-object.
                let new_target_callable =
                    self.callable_from_value(Value::Object(new_target_object))?;
                let fallback_realm = self.callable_realm(&new_target_callable)?;
                let prototype = self
                    .0
                    .state
                    .borrow()
                    .heap
                    .context(fallback_realm)?
                    .date_prototype
                    .ok_or(RuntimeError::Invariant("realm has no Date prototype"))?;
                Ok(NativeConversion::Value(ObjectRef::from_borrowed_handle(
                    self.clone(),
                    prototype,
                )?))
            }
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    /// Allocate a genuine Date after the newTarget prototype lookup. Keeping
    /// TimeClip at this final write boundary prevents a future caller from
    /// publishing the deliberately un-clipped `Date.parse` result directly as
    /// a Date payload.
    fn new_date_object(
        &self,
        prototype: &ObjectRef,
        value: f64,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Date prototype"));
        }
        let value = time_clip(value);
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::date(shape, Vec::new(), value))
        {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }
}

/// Convert parser output to the static `Date.parse` result. The explicit
/// timezone subtraction intentionally occurs after `set_date_fields` and is
/// intentionally *not* followed by TimeClip, matching pinned QuickJS.
fn parsed_date_value<F>(parsed: Option<ParsedDateString>, timezone_offset: F) -> f64
where
    F: FnMut(i64) -> i32,
{
    let Some(parsed) = parsed else {
        return f64::NAN;
    };
    let mut fields = [0.0; 7];
    for (target, source) in fields.iter_mut().zip(parsed.fields) {
        *target = f64::from(source);
    }
    let value = set_date_fields(&fields, parsed.is_local, timezone_offset);
    let explicit_offset = f64::from(parsed.fields[8]) * MS_PER_MINUTE;
    value - explicit_offset
}

#[cfg(test)]
mod tests {
    use super::*;

    const UTC: fn(i64) -> i32 = |_| 0;

    #[test]
    fn date_parse_applies_explicit_offset_after_inner_time_clip_without_reclipping() {
        let parsed = ParsedDateString {
            fields: [275_760, 8, 13, 0, 0, 0, 0, 0, -60],
            is_local: false,
        };

        let value = parsed_date_value(Some(parsed), UTC);
        assert_eq!(value, 8.64e15 + 3_600_000.0);
        assert!(time_clip(value).is_nan());
    }

    #[test]
    fn date_parse_invalid_syntax_is_nan() {
        assert!(parsed_date_value(None, UTC).is_nan());
    }

    #[test]
    fn utc_defaults_and_legacy_year_adjustment_match_quickjs() {
        assert_eq!(
            set_date_fields_checked(DEFAULT_DATE_FIELDS, false, UTC),
            -2_208_988_800_000.0
        );
        let mut fields = DEFAULT_DATE_FIELDS;
        fields[0] = 99.0;
        assert_eq!(
            set_date_fields_checked(fields, false, UTC),
            915_148_800_000.0
        );
    }
}
