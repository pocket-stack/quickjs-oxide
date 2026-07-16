//! Pinned QuickJS 2026-06-04 `Date.prototype` native handlers.
//!
//! The prototype object itself is deliberately ordinary. Every branded method
//! therefore checks for a genuine `ObjectPayload::Date` receiver instead of
//! accepting the realm's `Date.prototype` object.

use super::super::super::*;
use super::calendar::{DateFields, DateInputFields, get_date_fields, set_date_fields, time_clip};
use super::format::{DateStringKind as DateFormatKind, format_date_string};

fn date_format_kind(method: DateStringMethod) -> DateFormatKind {
    match method {
        DateStringMethod::ToString => DateFormatKind::String,
        DateStringMethod::ToDateString => DateFormatKind::DateString,
        DateStringMethod::ToTimeString => DateFormatKind::TimeString,
        DateStringMethod::ToUtcString => DateFormatKind::UtcString,
        DateStringMethod::ToIsoString => DateFormatKind::IsoString,
        DateStringMethod::ToLocaleString => DateFormatKind::LocaleString,
        DateStringMethod::ToLocaleDateString => DateFormatKind::LocaleDateString,
        DateStringMethod::ToLocaleTimeString => DateFormatKind::LocaleTimeString,
    }
}

fn date_input_fields(fields: &DateFields) -> DateInputFields {
    [
        fields[0], fields[1], fields[2], fields[3], fields[4], fields[5], fields[6],
    ]
}

fn date_argument(arguments: &NativeArguments, index: usize) -> Result<&Value, RuntimeError> {
    arguments.readable.get(index).ok_or(RuntimeError::Invariant(
        "Date native argument vector was not padded to readable arity",
    ))
}

impl Runtime {
    /// Dispatch every `Date.prototype` callable in the typed Date native
    /// family. Constructor and static selectors are rejected here so the
    /// parent Date module remains the single owner of that routing boundary.
    pub(in crate::runtime) fn call_date_prototype_native(
        &self,
        realm: ContextId,
        kind: DateNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        if matches!(
            kind,
            DateNativeKind::Constructor
                | DateNativeKind::Now
                | DateNativeKind::Parse
                | DateNativeKind::Utc
        ) {
            return Err(RuntimeError::Invariant(
                "Date static or constructor selector reached Date.prototype dispatch",
            ));
        }

        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Date.prototype native did not receive a generic invocation",
            ));
        };

        match kind {
            DateNativeKind::TimeValue => self.call_date_time_value(realm, &this_value),
            DateNativeKind::String(method) => self.call_date_string(realm, &this_value, method),
            DateNativeKind::ToPrimitive => {
                self.call_date_to_primitive(realm, this_value, arguments)
            }
            DateNativeKind::TimezoneOffset => self.call_date_timezone_offset(realm, &this_value),
            DateNativeKind::GetField(field) => self.call_date_get_field(realm, &this_value, field),
            DateNativeKind::SetTime => self.call_date_set_time(realm, &this_value, arguments),
            DateNativeKind::SetField(field) => {
                self.call_date_set_field(realm, &this_value, field, arguments)
            }
            DateNativeKind::SetYear => self.call_date_set_year(realm, &this_value, arguments),
            DateNativeKind::ToJson => self.call_date_to_json(realm, this_value),
            DateNativeKind::Constructor
            | DateNativeKind::Now
            | DateNativeKind::Parse
            | DateNativeKind::Utc => unreachable!("rejected before invocation adaptation"),
        }
    }

    fn date_this_time_value(
        &self,
        realm: ContextId,
        this_value: &Value,
    ) -> Result<NativeConversion<(ObjectRef, f64)>, RuntimeError> {
        let Value::Object(object) = this_value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a Date object",
            )?));
        };
        let value = {
            let state = self.0.state.borrow();
            match &state.heap.object(object.object_id())?.payload {
                ObjectPayload::Date(value) => Some(*value),
                ObjectPayload::Ordinary
                | ObjectPayload::RegExp(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. } => None,
            }
        };
        let Some(value) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a Date object",
            )?));
        };
        Ok(NativeConversion::Value((object.clone(), value)))
    }

    fn set_date_this_time_value(
        &self,
        object: &ObjectRef,
        value: f64,
    ) -> Result<Completion, RuntimeError> {
        self.0
            .state
            .borrow_mut()
            .heap
            .set_date_value(object.object_id(), value)?;
        Ok(Completion::Return(Value::number(value)))
    }

    fn call_date_time_value(
        &self,
        realm: ContextId,
        this_value: &Value,
    ) -> Result<Completion, RuntimeError> {
        let (_, value) = match self.date_this_time_value(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::number(value)))
    }

    /// `toGMTString` is not a ninth formatter native. The installer must
    /// materialize `toUTCString` once and store that exact callable in both
    /// properties, preserving `toGMTString === toUTCString` and the shared
    /// function object's `name === "toUTCString"`.
    fn call_date_string(
        &self,
        realm: ContextId,
        this_value: &Value,
        method: DateStringMethod,
    ) -> Result<Completion, RuntimeError> {
        let (_, value) = match self.date_this_time_value(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let kind = date_format_kind(method);
        let fields = get_date_fields(value, kind.uses_local_time(), false, |instant| {
            self.date_timezone_offset_minutes(instant)
        });
        let output = match format_date_string(fields.as_ref(), kind) {
            Ok(output) => output,
            Err(_) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "Date value is NaN",
                )?));
            }
        };
        Ok(Completion::Return(Value::String(JsString::try_from_utf8(
            &output,
        )?)))
    }

    fn call_date_get_field(
        &self,
        realm: ContextId,
        this_value: &Value,
        field: DateGetFieldKind,
    ) -> Result<Completion, RuntimeError> {
        let (_, value) = match self.date_this_time_value(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let Some(fields) = get_date_fields(value, field.uses_local_time(), false, |instant| {
            self.date_timezone_offset_minutes(instant)
        }) else {
            return Ok(Completion::Return(Value::number(f64::NAN)));
        };
        let mut value = fields[usize::from(field.field_index())];
        if field.is_legacy_year() {
            value -= 1900.0;
        }
        Ok(Completion::Return(Value::number(value)))
    }

    fn call_date_timezone_offset(
        &self,
        realm: ContextId,
        this_value: &Value,
    ) -> Result<Completion, RuntimeError> {
        let (_, value) = match self.date_this_time_value(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if value.is_nan() {
            return Ok(Completion::Return(Value::number(f64::NAN)));
        }
        let offset = self.date_timezone_offset_minutes(value.trunc() as i64);
        Ok(Completion::Return(Value::number(f64::from(offset))))
    }

    fn call_date_set_time(
        &self,
        realm: ContextId,
        this_value: &Value,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        // QuickJS performs the brand check before observing argument coercion.
        let (object, _) = match self.date_this_time_value(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let value = match self.native_to_number(realm, date_argument(arguments, 0)?)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.set_date_this_time_value(&object, time_clip(value))
    }

    fn call_date_set_field(
        &self,
        realm: ContextId,
        this_value: &Value,
        field: DateSetFieldKind,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let (object, old_value) = match self.date_this_time_value(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let first_field = usize::from(field.first_field());
        let end_field = usize::from(field.end_field());
        let recovered_fields = get_date_fields(
            old_value,
            field.uses_local_time(),
            first_field == 0,
            |instant| self.date_timezone_offset_minutes(instant),
        );
        let had_fields = recovered_fields.is_some();
        let mut fields = recovered_fields.unwrap_or([0.0; 9]);
        let mut all_finite = had_fields;

        // Match QuickJS's `min(argc, end - first)`: padded undefined values
        // are not converted for a zero-argument generic setter, and extra
        // arguments beyond the setter's field window remain completely
        // unobserved. A non-finite earlier value does not suppress later
        // conversions.
        let conversion_count = arguments
            .actual_arg_count
            .min(end_field.saturating_sub(first_field));
        for index in 0..conversion_count {
            let value = match self.native_to_number(realm, date_argument(arguments, index)?)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if !value.is_finite() {
                all_finite = false;
            }
            fields[first_field + index] = value.trunc();
        }

        // A non-full-year setter cannot recover an invalid Date. Argument
        // coercions above are still observable, but the Date handler itself
        // performs no write on this path, exactly like upstream's early
        // `return JS_NAN`.
        if !had_fields {
            return Ok(Completion::Return(Value::number(f64::NAN)));
        }

        let new_value = if all_finite && arguments.actual_arg_count > 0 {
            let input = date_input_fields(&fields);
            set_date_fields(&input, field.uses_local_time(), |instant| {
                self.date_timezone_offset_minutes(instant)
            })
        } else {
            f64::NAN
        };
        self.set_date_this_time_value(&object, new_value)
    }

    fn call_date_set_year(
        &self,
        realm: ContextId,
        this_value: &Value,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        // The first brand check precedes coercion. Unlike the generic setters,
        // QuickJS then re-enters set_date_field after coercion, so any user
        // side effect which changed this Date is observed by the decomposition
        // below.
        let (object, _) = match self.date_this_time_value(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let mut year = match self.native_to_number(realm, date_argument(arguments, 0)?)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if year.is_finite() {
            year = year.trunc();
            if (0.0..100.0).contains(&year) {
                year += 1900.0;
            }
        }

        let current_value = self.0.state.borrow().heap.date_value(object.object_id())?;
        let mut fields = get_date_fields(current_value, true, true, |instant| {
            self.date_timezone_offset_minutes(instant)
        })
        .ok_or(RuntimeError::Invariant(
            "forced Date decomposition unexpectedly rejected a time value",
        ))?;
        fields[0] = year;
        let new_value = if year.is_finite() {
            let input = date_input_fields(&fields);
            set_date_fields(&input, true, |instant| {
                self.date_timezone_offset_minutes(instant)
            })
        } else {
            f64::NAN
        };
        self.set_date_this_time_value(&object, new_value)
    }

    fn call_date_to_primitive(
        &self,
        realm: ContextId,
        this_value: Value,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let Value::Object(object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let hint = match date_argument(arguments, 0)? {
            Value::String(value)
                if value == &JsString::from_static("number")
                    || value == &JsString::from_static("integer") =>
            {
                ToPrimitiveHint::Number
            }
            Value::String(value)
                if value == &JsString::from_static("string")
                    || value == &JsString::from_static("default") =>
            {
                ToPrimitiveHint::String
            }
            _ => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "invalid hint",
                )?));
            }
        };
        self.ordinary_to_primitive(realm, &object, hint)
    }

    fn call_date_to_json(
        &self,
        realm: ContextId,
        this_value: Value,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.native_to_object(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let primitive = match self.to_primitive(
            realm,
            Value::Object(object.clone()),
            ToPrimitiveHint::Number,
        )? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if primitive
            .as_number()
            .is_some_and(|number| !number.is_finite())
        {
            return Ok(Completion::Return(Value::Null));
        }

        let to_iso_string = self.intern_property_key("toISOString")?;
        let method = match self.get_property_in_realm(realm, &object, &to_iso_string)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let callable = match method {
            Value::Object(method) => self.as_callable(&method)?,
            Value::Undefined
            | Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => None,
        };
        let Some(callable) = callable else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "object needs toISOString method",
            )?));
        };
        self.call_internal(realm, &callable, Value::Object(object), &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(source: &str) -> Value {
        let runtime = Runtime::new();
        runtime
            .new_context()
            .eval(source)
            .unwrap_or_else(|error| panic!("Date prototype test failed: {error:?}"))
    }

    #[test]
    fn genuine_date_brand_and_utc_formatting_are_observable() {
        assert_eq!(
            eval("new Date(0).toISOString()"),
            Value::String(JsString::from_static("1970-01-01T00:00:00.000Z"))
        );
        assert_eq!(
            eval("(function(){try{return Date.prototype.valueOf()}catch(e){return e.message}})()"),
            Value::String(JsString::from_static("not a Date object"))
        );
        assert_eq!(
            eval("Object.prototype.toString.call(Date.prototype)"),
            Value::String(JsString::from_static("[object Object]"))
        );
    }

    #[test]
    fn only_full_year_setters_recover_an_invalid_date() {
        assert_eq!(
            eval(
                r#"
                (function(){
                    var d=new Date(NaN);
                    var month=d.setUTCMonth(1);
                    var year=d.setUTCFullYear(2000);
                    return (month!==month)+'|'+year+'|'+d.toISOString()
                })()
                "#
            ),
            Value::String(JsString::from_static(
                "true|946684800000|2000-01-01T00:00:00.000Z"
            ))
        );
    }

    #[test]
    fn setters_convert_the_full_field_window_but_ignore_extra_arguments() {
        assert_eq!(
            eval(
                r#"
                (function(){
                    var log='';
                    function V(s,v){this.s=s;this.v=v}
                    V.prototype.valueOf=function(){log+=this.s;return this.v};
                    var d=new Date(0);
                    var result=d.setUTCMinutes(
                        new V('a',NaN),new V('b',1),new V('c',2),new V('x',3));
                    return log+'|'+(result!==result)+'|'+(d.getTime()!==d.getTime())
                })()
                "#
            ),
            Value::String(JsString::from_static("abc|true|true"))
        );
    }

    #[test]
    fn to_primitive_is_forced_ordinary_and_to_json_is_generic() {
        assert_eq!(
            eval(
                r#"
                (function(){
                    var object={};
                    object.valueOf=function(){return 7};
                    object.toString=function(){return 'string-result'};
                    var primitive=Date.prototype[Symbol.toPrimitive];
                    var number=primitive.call(object,'number');
                    var integer=primitive.call(object,'integer');
                    var string=primitive.call(object,'default');
                    var invalid={};
                    invalid.valueOf=function(){return Infinity};
                    invalid.toISOString=function(){return 'unreachable'};
                    var finite={};
                    finite.valueOf=function(){return 1};
                    finite.toISOString=function(){return this===finite?'json-result':'bad-this'};
                    return number+'|'+integer+'|'+string+'|'+
                        Date.prototype.toJSON.call(invalid)+'|'+
                        Date.prototype.toJSON.call(finite)
                })()
                "#
            ),
            Value::String(JsString::from_static("7|7|string-result|null|json-result"))
        );
    }
}
