pub(crate) mod descriptor;
pub(crate) mod number;
pub(crate) mod primitive;

use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::builtins::native::PrimitiveKind;
use crate::engine::heap::ContextId;

use crate::engine::object::{ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol};
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
        self.property_key_from_primitive(realm, value)
    }

    /// Internal-value form of [`Runtime::native_to_property_key`].
    pub(crate) fn native_to_property_key_jsvalue(
        &self,
        realm: ContextId,
        value: crate::engine::value::JsValue,
    ) -> Result<NativeConversion<PropertyKey>, RuntimeError> {
        let value = if matches!(value, crate::engine::value::JsValue::Object(_)) {
            match self.to_primitive_jsvalue(realm, value, ToPrimitiveHint::String)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => {
                    // The callback boundary throws public roots; hand the host
                    // adapter its owned root back without a retain/release pair.
                    return Ok(NativeConversion::Throw(self.root_value(&value)?));
                }
            }
        } else {
            value
        };
        self.property_key_from_primitive_jsvalue(realm, value)
    }

    /// Internal-value form of [`Runtime::property_key_from_primitive`].
    pub(crate) fn property_key_from_primitive_jsvalue(
        &self,
        realm: ContextId,
        value: crate::engine::value::JsValue,
    ) -> Result<NativeConversion<PropertyKey>, RuntimeError> {
        use crate::engine::value::JsValue;
        if matches!(value, JsValue::Object(_)) {
            return Err(RuntimeError::Invariant(
                "property key conversion received an object",
            ));
        }
        if let Some(key) = self.immediate_numeric_property_key_jsvalue(&value) {
            return Ok(NativeConversion::Value(key));
        }
        if let JsValue::Symbol(index) = value {
            let atom = self.0.state.borrow().atoms.brand(index)?;
            return Ok(NativeConversion::Value(PropertyKey::from_borrowed_atom(
                self.clone(),
                atom,
            )?));
        }
        let string = match crate::engine::vm::to_js_string_jsvalue(self, &value) {
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

    /// Finish ToPropertyKey after the domain continuation has obtained a primitive.
    pub(crate) fn property_key_from_primitive(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<PropertyKey>, RuntimeError> {
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "property key conversion received an object",
            ));
        }
        if let Some(key) = self.immediate_numeric_property_key(&value) {
            return Ok(NativeConversion::Value(key));
        }
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

    /// Port of pinned QuickJS `js_obj_to_desc`. Field probes deliberately use
    /// its C order and inherited HasProperty/Get behavior. The release also
    /// replaces a throw from the `get`/`set` field getter with its own
    /// `invalid getter`/`invalid setter` TypeError, which is preserved here.
    pub(crate) fn native_to_property_descriptor(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<OrdinaryPropertyDescriptor>, RuntimeError> {
        use descriptor::DescriptorStep;
        let mut step = DescriptorStep::start(self, realm, value)?;
        loop {
            step = match step {
                DescriptorStep::Complete(resume) => {
                    return Ok(NativeConversion::Value(resume.take_descriptor()));
                }
                DescriptorStep::Throw(value) => return Ok(NativeConversion::Throw(value)),
                DescriptorStep::Has { mut resume } => {
                    let object = resume.take_has_object();
                    let key = resume.take_has_key();
                    resume.has(self, self.internal_has_property(realm, &object, &key)?)?
                }
                DescriptorStep::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    let receiver = resume.take_read_receiver();
                    resume.read(self, self.internal_get(realm, &object, &key, receiver)?)?
                }
            };
        }
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
        self.string_from_primitive(realm, &value)
    }

    pub(crate) fn string_from_primitive(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<JsString>, RuntimeError> {
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "ToString primitive reply contained an object",
            ));
        }
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
        let mut step = number::NumberStep::start(self, realm, value.clone())?;
        loop {
            step = match step {
                number::NumberStep::Complete(result) => return Ok(result),
                number::NumberStep::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    resume.resume(self, self.get_property_in_realm(realm, &object, &key)?)?
                }
                number::NumberStep::Call { mut resume } => {
                    let callable = resume.take_call_callable();
                    let receiver = resume.take_call_receiver();
                    let arguments = resume.take_call_arguments();
                    resume.resume(
                        self,
                        self.call_internal(realm, &callable, receiver, &arguments)?,
                    )?
                }
            };
        }
    }

    pub(crate) fn number_from_primitive(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<f64>, RuntimeError> {
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "ToNumber primitive completion received an object",
            ));
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

    pub(crate) fn number_constructor_from_primitive(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<f64>, RuntimeError> {
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "Number constructor primitive reply contained an object",
            ));
        }
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
        self.bigint_from_primitive(realm, value)
    }

    pub(crate) fn bigint_from_primitive(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<crate::engine::value::bigint::JsBigInt>, RuntimeError> {
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "ToBigInt primitive completion received an object",
            ));
        }
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

    pub(crate) fn bigint_constructor_from_primitive(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<crate::engine::value::bigint::JsBigInt>, RuntimeError> {
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "BigInt constructor primitive reply contained an object",
            ));
        }
        let value = value.clone();
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
        let number = match self.native_to_number(realm, value)? {
            NativeConversion::Value(number) => number,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        self.index_from_number(realm, number)
    }
    pub(crate) fn index_from_number(
        &self,
        realm: ContextId,
        number: f64,
    ) -> Result<NativeConversion<u64>, RuntimeError> {
        const MAX_SAFE_INTEGER: i64 = (1_i64 << 53) - 1;
        let value = Self::int64_from_number(number);
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
        Ok(NativeConversion::Value(Self::int64_from_number(number)))
    }

    pub(crate) fn int64_from_number(number: f64) -> i64 {
        if number.is_nan() {
            0
        } else if number < i64::MIN as f64 {
            i64::MIN
        } else if number >= 2_f64.powi(63) {
            i64::MAX
        } else {
            number as i64
        }
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
        match self.native_to_number(realm, value)? {
            NativeConversion::Value(number) => {
                Ok(NativeConversion::Value(Self::length_from_number(number)))
            }
            NativeConversion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    pub(crate) fn length_from_number(number: f64) -> u64 {
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;
        if number.is_nan() || number <= 0.0 {
            0
        } else if number >= MAX_SAFE_INTEGER as f64 {
            MAX_SAFE_INTEGER
        } else {
            number as u64
        }
    }

    pub(crate) fn to_primitive(
        &self,
        realm: ContextId,
        value: Value,
        hint: ToPrimitiveHint,
    ) -> Result<Completion, RuntimeError> {
        let step = primitive::PrimitiveResume::start(
            self,
            realm,
            self.unroot_value(&value)?,
            hint,
        );
        self.finish_primitive_steps(realm, step)
    }

    /// Internal-value form of [`Runtime::to_primitive`]: consumes the value.
    pub(crate) fn to_primitive_jsvalue(
        &self,
        realm: ContextId,
        value: crate::engine::value::JsValue,
        hint: ToPrimitiveHint,
    ) -> Result<Completion, RuntimeError> {
        let step = primitive::PrimitiveResume::start(self, realm, value, hint);
        self.finish_primitive_steps(realm, step)
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

    /// Internal-value form of [`Runtime::native_to_object`]: consumes the value.
    pub(crate) fn native_to_object_jsvalue(
        &self,
        realm: ContextId,
        value: crate::engine::value::JsValue,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        use crate::engine::value::JsValue;
        let (kind, value) = match value {
            JsValue::Object(object) => {
                return Ok(NativeConversion::Value(ObjectRef::from_borrowed_handle(
                    self.clone(),
                    object,
                )?));
            }
            JsValue::Undefined | JsValue::Null => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "cannot convert to object",
                )?));
            }
            value @ JsValue::Bool(_) => (PrimitiveKind::Boolean, value),
            value @ (JsValue::Int(_) | JsValue::Float(_)) => (PrimitiveKind::Number, value),
            value @ JsValue::String(_) => (PrimitiveKind::String, value),
            value @ JsValue::BigInt(_) => (PrimitiveKind::BigInt, value),
            value @ JsValue::Symbol(_) => (PrimitiveKind::Symbol, value),
        };
        let prototype = self.primitive_prototype_for_realm(realm, kind)?;
        Ok(NativeConversion::Value(
            self.new_primitive_object_jsvalue(&prototype, kind, value)?,
        ))
    }
}

pub(crate) enum NativeConversion<T> {
    Value(T),
    Throw(Value),
}
