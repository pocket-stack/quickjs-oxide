use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::builtins::native::PrimitiveKind;
use crate::engine::heap::ContextId;

use crate::engine::object::{
    AccessorValue, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
    WellKnownSymbol,
};
use crate::engine::value::{JsString, Value};
use crate::engine::vm::{Completion, ToPrimitiveHint};

impl Runtime {
    /// Completion-aware `ToPropertyKey` used by native Object APIs. Symbols
    /// retain identity; every other value uses string-hint ToPrimitive before
    /// exact UTF-16 key interning.
    pub(crate) fn native_to_property_key(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<PropertyKey>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value, ToPrimitiveHint::String)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value
        };
        if let Value::Symbol(symbol) = value {
            if !symbol.belongs_to(self) {
                return Err(RuntimeError::WrongRuntime("property-key symbol"));
            }
            return Ok(NativeConversion::Value(PropertyKey::from_borrowed_atom(
                self.clone(),
                symbol.atom(),
            )?));
        }
        let string = match value.to_js_string() {
            Ok(string) => string,
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                return Ok(NativeConversion::Throw(
                    self.new_native_error_from_error(realm, kind, &error)?,
                ));
            }
        };
        Ok(NativeConversion::Value(
            self.intern_property_key_js_string(&string)?,
        ))
    }

    pub(crate) fn native_get_present_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        name: &str,
    ) -> Result<NativeConversion<Option<Value>>, RuntimeError> {
        let key = self.intern_property_key(name)?;
        match self.internal_has_property(realm, object, &key)? {
            NativeConversion::Value(true) => {}
            NativeConversion::Value(false) => return Ok(NativeConversion::Value(None)),
            // Pinned `js_obj_to_desc` uses the tri-state C result directly as
            // a Boolean. `-1` therefore takes the present branch and a
            // following successful Get replaces the HasProperty throw.
            NativeConversion::Throw(_) => {}
        }
        match self.get_property_in_realm(realm, object, &key)? {
            Completion::Return(value) => Ok(NativeConversion::Value(Some(value))),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    /// Port of pinned QuickJS `js_obj_to_desc`. Field probes deliberately use
    /// its C order and inherited HasProperty/Get behavior. The release also
    /// replaces a throw from the `get`/`set` field getter with its own
    /// `invalid getter`/`invalid setter` TypeError, which is preserved here.
    pub(crate) fn native_to_property_descriptor(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<OrdinaryPropertyDescriptor>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let mut descriptor = OrdinaryPropertyDescriptor::new();

        for (name, target) in [
            ("enumerable", &mut descriptor.enumerable),
            ("configurable", &mut descriptor.configurable),
        ] {
            match self.native_get_present_property(realm, &object, name)? {
                NativeConversion::Value(Some(value)) => {
                    *target = DescriptorField::Present(self.value_to_boolean(&value)?);
                }
                NativeConversion::Value(None) => {}
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        }
        match self.native_get_present_property(realm, &object, "value")? {
            NativeConversion::Value(Some(value)) => {
                descriptor.value = DescriptorField::Present(value);
            }
            NativeConversion::Value(None) => {}
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        }
        match self.native_get_present_property(realm, &object, "writable")? {
            NativeConversion::Value(Some(value)) => {
                descriptor.writable = DescriptorField::Present(self.value_to_boolean(&value)?);
            }
            NativeConversion::Value(None) => {}
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        }

        for (name, target, error_message) in [
            ("get", &mut descriptor.get, "invalid getter"),
            ("set", &mut descriptor.set, "invalid setter"),
        ] {
            let field = match self.native_get_present_property(realm, &object, name)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(_) => {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        error_message,
                    )?));
                }
            };
            let Some(value) = field else {
                continue;
            };
            let accessor = match value {
                Value::Undefined => AccessorValue::Undefined,
                Value::Object(object) => {
                    let Some(callable) = self.as_callable(&object)? else {
                        return Ok(NativeConversion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            error_message,
                        )?));
                    };
                    AccessorValue::Callable(callable)
                }
                _ => {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        error_message,
                    )?));
                }
            };
            *target = DescriptorField::Present(accessor);
        }
        if descriptor.is_mixed_descriptor() {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot have setter/getter and value or writable",
            )?));
        }
        Ok(NativeConversion::Value(descriptor))
    }

    pub(crate) fn native_to_js_string(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<JsString>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::String)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        match value.to_js_string() {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                Ok(NativeConversion::Throw(
                    self.new_native_error_from_error(realm, kind, &error)?,
                ))
            }
        }
    }

    /// QuickJS's `JS_ToStringCheckObject`: reject nullish receivers with its
    /// dedicated diagnostic before running any observable ToString steps.
    pub(crate) fn native_to_string_check_object(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<JsString>, RuntimeError> {
        if matches!(value, Value::Null | Value::Undefined) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "null or undefined are forbidden",
            )?));
        }
        self.native_to_js_string(realm, value)
    }

    pub(crate) fn native_to_dynamic_source_fragment(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<JsString>, RuntimeError> {
        self.native_to_js_string(realm, value)
    }

    pub(crate) fn native_to_number(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<f64>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::Number)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        match value.to_number() {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                Ok(NativeConversion::Throw(
                    self.new_native_error_from_error(realm, kind, &error)?,
                ))
            }
        }
    }

    /// QuickJS's `%Number%` constructor uses `ToNumeric`, then converts a
    /// BigInt result to binary64. Ordinary `ToNumber` deliberately remains
    /// stricter and continues to reject BigInt everywhere else.
    pub(crate) fn native_to_number_constructor_value(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<f64>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::Number)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        if let Value::BigInt(value) = &value {
            return Ok(NativeConversion::Value(value.to_f64()));
        }
        match value.to_number() {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                Ok(NativeConversion::Throw(
                    self.new_native_error_from_error(realm, kind, &error)?,
                ))
            }
        }
    }

    pub(crate) fn native_bigint_from_string(
        &self,
        realm: ContextId,
        value: &JsString,
    ) -> Result<NativeConversion<crate::engine::value::bigint::JsBigInt>, RuntimeError> {
        let units = value.utf16_units().collect::<Vec<_>>();
        let Ok(value) = String::from_utf16(&units) else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Syntax,
                "invalid bigint literal",
            )?));
        };
        match crate::engine::value::bigint::JsBigInt::parse_js_string(&value) {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(crate::engine::value::bigint::BigIntError::InvalidSyntax) => {
                Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Syntax,
                    "invalid bigint literal",
                )?))
            }
            Err(
                crate::engine::value::bigint::BigIntError::BigIntTooLarge
                | crate::engine::value::bigint::BigIntError::AllocationTooLarge,
            ) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "BigInt is too large to allocate",
            )?)),
            Err(error) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                &error.to_string(),
            )?)),
        }
    }

    /// QuickJS `JS_ToBigInt`: Numbers, null, undefined and Symbols are
    /// rejected, while Boolean and String inputs are accepted after ordered
    /// number-hint `ToPrimitive` for objects.
    pub(crate) fn native_to_bigint(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<crate::engine::value::bigint::JsBigInt>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::Number)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        match value {
            Value::BigInt(value) => Ok(NativeConversion::Value(value)),
            Value::Bool(value) => Ok(NativeConversion::Value(
                crate::engine::value::bigint::JsBigInt::from(i64::from(value)),
            )),
            Value::String(value) => self.native_bigint_from_string(realm, &value),
            Value::Undefined
            | Value::Null
            | Value::Int(_)
            | Value::Float(_)
            | Value::Symbol(_)
            | Value::Object(_) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot convert to bigint",
            )?)),
        }
    }

    /// BigInt constructor conversion differs from ordinary `ToBigInt` by
    /// accepting integral Number values and by using the pinned capitalized
    /// TypeError spelling for unsupported primitives.
    pub(crate) fn native_to_bigint_constructor_value(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<crate::engine::value::bigint::JsBigInt>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::Number)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        match value {
            Value::Int(value) => Ok(NativeConversion::Value(
                crate::engine::value::bigint::JsBigInt::from(value),
            )),
            Value::Bool(value) => Ok(NativeConversion::Value(
                crate::engine::value::bigint::JsBigInt::from(i64::from(value)),
            )),
            Value::BigInt(value) => Ok(NativeConversion::Value(value)),
            Value::Float(value) if !value.is_finite() => {
                Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "cannot convert NaN or Infinity to BigInt",
                )?))
            }
            Value::Float(value) if value.fract() != 0.0 => {
                Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "cannot convert to BigInt: not an integer",
                )?))
            }
            Value::Float(value) => {
                let value = crate::engine::value::bigint::JsBigInt::from_integral_f64(value)
                    .ok_or(RuntimeError::Invariant(
                        "finite integral f64 could not become a BigInt",
                    ))?;
                Ok(NativeConversion::Value(value))
            }
            Value::String(value) => self.native_bigint_from_string(realm, &value),
            Value::Undefined | Value::Null | Value::Symbol(_) | Value::Object(_) => {
                Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "cannot convert to BigInt",
                )?))
            }
        }
    }

    /// Pinned QuickJS `JS_ToIndex`: saturating ToInt64 followed by the
    /// non-negative MAX_SAFE_INTEGER range check.
    pub(crate) fn native_to_index(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<u64>, RuntimeError> {
        const MAX_SAFE_INTEGER: i64 = (1_i64 << 53) - 1;
        let value = match self.native_to_int64_sat(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !(0..=MAX_SAFE_INTEGER).contains(&value) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid array index",
            )?));
        }
        Ok(NativeConversion::Value(
            u64::try_from(value).expect("validated non-negative ToIndex value fits u64"),
        ))
    }

    /// Pinned QuickJS `JS_ToInt64Sat`: number-hint coercion followed by
    /// truncation toward zero with NaN mapped to zero and infinities/outliers
    /// saturated at the signed 64-bit bounds.
    pub(crate) fn native_to_int64_sat(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<i64>, RuntimeError> {
        let number = match self.native_to_number(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        Ok(NativeConversion::Value(if number.is_nan() {
            0
        } else if number < i64::MIN as f64 {
            i64::MIN
        } else if number >= 2_f64.powi(63) {
            i64::MAX
        } else {
            number as i64
        }))
    }

    /// Pinned QuickJS `JS_ToInt64Clamp`, including its negative offset before
    /// the final inclusive clamp.
    pub(crate) fn native_to_int64_clamp(
        &self,
        realm: ContextId,
        value: &Value,
        min: i64,
        max: i64,
        negative_offset: i64,
    ) -> Result<NativeConversion<i64>, RuntimeError> {
        let mut value = match self.native_to_int64_sat(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if value < 0 {
            value += negative_offset;
        }
        Ok(NativeConversion::Value(value.clamp(min, max)))
    }

    pub(crate) fn native_to_length(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<u64>, RuntimeError> {
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;

        let number = match self.native_to_number(realm, value)? {
            NativeConversion::Value(number) => number,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let length = if number.is_nan() || number <= 0.0 {
            0
        } else if number >= MAX_SAFE_INTEGER as f64 {
            MAX_SAFE_INTEGER
        } else {
            // This branch is finite, positive and below 2^53, so the
            // truncating cast is the exact ToIntegerOrInfinity result.
            number as u64
        };
        Ok(NativeConversion::Value(length))
    }

    pub(crate) fn to_primitive(
        &self,
        realm: ContextId,
        value: Value,
        hint: ToPrimitiveHint,
    ) -> Result<Completion, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(Completion::Return(value));
        };
        let to_primitive = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToPrimitive));
        let exotic = match self.get_property_in_realm(realm, &object, &to_primitive)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if !matches!(exotic, Value::Undefined | Value::Null) {
            let Value::Object(exotic_object) = exotic else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            let Some(exotic) = self.as_callable(&exotic_object)? else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            return match self.call_internal(
                realm,
                &exotic,
                Value::Object(object),
                &[Value::String(JsString::from_static(match hint {
                    ToPrimitiveHint::String => "string",
                    ToPrimitiveHint::Number => "number",
                    ToPrimitiveHint::Default => "default",
                }))],
            )? {
                Completion::Return(Value::Object(_)) => Ok(Completion::Throw(
                    self.new_native_error(realm, NativeErrorKind::Type, "toPrimitive")?,
                )),
                completion => Ok(completion),
            };
        }

        self.ordinary_to_primitive(realm, &object, hint)
    }

    pub(crate) fn native_to_object(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let (kind, value) = match value {
            Value::Object(object) => return Ok(NativeConversion::Value(object)),
            Value::Undefined | Value::Null => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "cannot convert to object",
                )?));
            }
            value @ Value::Bool(_) => (PrimitiveKind::Boolean, value),
            value @ (Value::Int(_) | Value::Float(_)) => (PrimitiveKind::Number, value),
            value @ Value::String(_) => (PrimitiveKind::String, value),
            value @ Value::BigInt(_) => (PrimitiveKind::BigInt, value),
            value @ Value::Symbol(_) => (PrimitiveKind::Symbol, value),
        };
        let prototype = self.primitive_prototype_for_realm(realm, kind)?;
        Ok(NativeConversion::Value(
            self.new_primitive_object(&prototype, kind, value)?,
        ))
    }
}

pub(crate) enum NativeConversion<T> {
    Value(T),
    Throw(Value),
}
