use crate::engine::api::error::{Error, NativeErrorKind, NativeErrorMessage};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{
    BigIntAsNKind, GlobalNumberPredicateKind, GlobalUriCodecKind, NumberFormatKind,
    NumberParseKind, NumberPredicateKind, PrimitiveKind, StringCharAtKind, StringWellFormedKind,
    SymbolRegistryKind,
};
use crate::engine::heap::{ContextId, ObjectPayload, PrimitiveObjectData};
use crate::engine::object::SymbolRef;
use crate::engine::object::access::raw_string_property_one_level;
use crate::engine::value::conversion::NativeConversion;

use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation, NativeInvokeOutcome};

impl Runtime {
    pub(crate) fn call_primitive_constructor(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "primitive constructor readable argv was not padded to one",
        ))?;
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "primitive constructor did not receive constructor-or-function invocation",
            ));
        };
        if kind == PrimitiveKind::Symbol {
            // Like QuickJS's constructor-or-function C entry, Symbol keeps its
            // constructor bit but rejects a real new.target before ToString.
            if !matches!(new_target, Value::Undefined) {
                return Ok(Completion::Throw(
                    self.new_not_constructor_error(realm, &new_target)?,
                ));
            }
            let description =
                if arguments.actual_arg_count == 0 || matches!(argument, Value::Undefined) {
                    None
                } else {
                    match self.native_to_js_string(realm, argument)? {
                        NativeConversion::Value(value) => Some(value),
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                };
            return Ok(Completion::Return(Value::Symbol(
                self.new_symbol(description)?,
            )));
        }
        if kind == PrimitiveKind::BigInt {
            // BigInt deliberately keeps QuickJS's constructor-or-function
            // cproto bit, but its body rejects any real new.target before
            // touching the argument.
            if !matches!(new_target, Value::Undefined) {
                return Ok(Completion::Throw(
                    self.new_not_constructor_error(realm, &new_target)?,
                ));
            }
            let value = match self.native_to_bigint_constructor_value(realm, argument)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            return Ok(Completion::Return(Value::BigInt(value)));
        }
        let value = match kind {
            PrimitiveKind::Boolean => Value::Bool(self.value_to_boolean(argument)?),
            PrimitiveKind::Number if arguments.actual_arg_count == 0 => Value::Int(0),
            PrimitiveKind::Number => {
                let value = match self.native_to_number_constructor_value(realm, argument)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                Value::number(value)
            }
            PrimitiveKind::String if arguments.actual_arg_count == 0 => {
                Value::String(JsString::from_static(""))
            }
            PrimitiveKind::String => {
                let value = if matches!(new_target, Value::Undefined)
                    && let Value::Symbol(symbol) = argument
                {
                    self.symbol_descriptive_string(symbol)?
                } else {
                    match self.native_to_js_string(realm, argument)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                };
                Value::String(value)
            }
            PrimitiveKind::Symbol | PrimitiveKind::BigInt => {
                return Err(RuntimeError::Invariant(
                    "unimplemented primitive constructor reached native dispatch",
                ));
            }
        };
        if matches!(new_target, Value::Undefined) {
            return Ok(Completion::Return(value));
        }
        let prototype =
            match self.prototype_from_constructor_value(realm, &new_target, |fallback_realm| {
                self.primitive_prototype_for_realm(fallback_realm, kind)
            })? {
                NativeConversion::Value(prototype) => prototype,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        Ok(Completion::Return(Value::Object(
            self.new_primitive_object(&prototype, kind, value)?,
        )))
    }

    pub(crate) fn new_not_constructor_error(
        &self,
        realm: ContextId,
        target: &Value,
    ) -> Result<Value, RuntimeError> {
        let name = if let Value::Object(object) = target
            && self.as_callable(object)?.is_some()
        {
            let name = self.intern_property_key("name")?;
            raw_string_property_one_level(&self.0.state.borrow(), object.object_id(), name.atom())?
                .filter(JsString::is_flat)
        } else {
            None
        };
        let mut message = NativeErrorMessage::new();
        if let Some(name) = name {
            name.push_c_string_to(&mut message);
            message.push_utf8(" is not a constructor");
        } else {
            message.push_utf8("not a constructor");
        }
        self.new_native_error_from_message(realm, NativeErrorKind::Type, message)
    }

    pub(crate) fn call_global_number_parse(
        &self,
        realm: ContextId,
        kind: NumberParseKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "global numeric parser did not receive a generic call",
            ));
        };
        let input = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "global numeric parser argv was not padded",
        ))?;
        let input = match self.native_to_js_string(realm, input)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = match kind {
            NumberParseKind::ParseFloat => crate::engine::value::number_parse::parse_float(&input),
            NumberParseKind::ParseInt => {
                let radix = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                    "parseInt radix argv was not padded",
                ))?;
                let radix = match self.native_to_number(realm, radix)? {
                    NativeConversion::Value(value) => crate::engine::value::number::to_int32(value),
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                crate::engine::value::number_parse::parse_int(&input, radix)
            }
        };
        Ok(Completion::Return(Value::number(result)))
    }

    pub(crate) fn call_global_number_predicate(
        &self,
        realm: ContextId,
        kind: GlobalNumberPredicateKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "global numeric predicate did not receive a generic call",
            ));
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "global numeric predicate argv was not padded",
        ))?;
        let number = match self.native_to_number(realm, argument)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = match kind {
            GlobalNumberPredicateKind::IsNaN => number.is_nan(),
            GlobalNumberPredicateKind::IsFinite => number.is_finite(),
        };
        Ok(Completion::Return(Value::Bool(result)))
    }

    pub(crate) fn call_global_uri_codec(
        &self,
        realm: ContextId,
        kind: GlobalUriCodecKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "global URI codec did not receive a generic call",
            ));
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "global URI codec argv was not padded",
        ))?;
        let input = match self.native_to_js_string(realm, argument)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = match kind {
            GlobalUriCodecKind::DecodeUri => crate::engine::builtins::uri::decode(&input, false),
            GlobalUriCodecKind::DecodeUriComponent => {
                crate::engine::builtins::uri::decode(&input, true)
            }
            GlobalUriCodecKind::EncodeUri => crate::engine::builtins::uri::encode(&input, false),
            GlobalUriCodecKind::EncodeUriComponent => {
                crate::engine::builtins::uri::encode(&input, true)
            }
            GlobalUriCodecKind::Escape => crate::engine::builtins::uri::escape(&input),
            GlobalUriCodecKind::Unescape => crate::engine::builtins::uri::unescape(&input),
        };
        match result {
            Ok(value) => Ok(Completion::Return(Value::String(value))),
            Err(crate::engine::builtins::uri::UriCodecError::String(error)) => Err(error.into()),
            Err(error) => Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Uri,
                error.message(),
            )?)),
        }
    }

    pub(crate) fn primitive_this_value(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        this_value: Value,
    ) -> Result<NativeConversion<Value>, RuntimeError> {
        let direct = matches!(
            (&this_value, kind),
            (Value::Int(_) | Value::Float(_), PrimitiveKind::Number)
                | (Value::String(_), PrimitiveKind::String)
                | (Value::Bool(_), PrimitiveKind::Boolean)
                | (Value::Symbol(_), PrimitiveKind::Symbol)
                | (Value::BigInt(_), PrimitiveKind::BigInt)
        );
        if direct {
            return Ok(NativeConversion::Value(this_value));
        }
        if let Value::Object(object) = &this_value {
            let payload = {
                let state = self.0.state.borrow();
                match &state.heap.object(object.object_id())?.payload {
                    ObjectPayload::Primitive(PrimitiveObjectData::Number(value))
                        if kind == PrimitiveKind::Number =>
                    {
                        Some(Ok(Value::number(*value)))
                    }
                    ObjectPayload::Primitive(PrimitiveObjectData::String(value))
                        if kind == PrimitiveKind::String =>
                    {
                        Some(Ok(Value::String(value.clone())))
                    }
                    ObjectPayload::Primitive(PrimitiveObjectData::Boolean(value))
                        if kind == PrimitiveKind::Boolean =>
                    {
                        Some(Ok(Value::Bool(*value)))
                    }
                    ObjectPayload::Primitive(PrimitiveObjectData::Symbol(atom))
                        if kind == PrimitiveKind::Symbol =>
                    {
                        // Promote the wrapper's raw owning atom only after the
                        // immutable heap borrow above has ended.
                        Some(Err(*atom))
                    }
                    ObjectPayload::Primitive(PrimitiveObjectData::BigInt(value))
                        if kind == PrimitiveKind::BigInt =>
                    {
                        Some(Ok(Value::BigInt(value.clone())))
                    }
                    ObjectPayload::Ordinary
                    | ObjectPayload::ArrayBuffer(_)
                    | ObjectPayload::SharedArrayBuffer(_)
                    | ObjectPayload::DataView(_)
                    | ObjectPayload::TypedArray(_)
                    | ObjectPayload::Proxy(_)
                    | ObjectPayload::AsyncFunctionState(_)
                    | ObjectPayload::RawJson
                    | ObjectPayload::Promise(_)
                    | ObjectPayload::Date(_)
                    | ObjectPayload::RegExp(_)
                    | ObjectPayload::Array { .. }
                    | ObjectPayload::Arguments { .. }
                    | ObjectPayload::ArrayIterator { .. }
                    | ObjectPayload::IteratorHelper(_)
                    | ObjectPayload::IteratorWrap(_)
                    | ObjectPayload::AsyncFromSyncIterator(_)
                    | ObjectPayload::IteratorConcat(_)
                    | ObjectPayload::Map { .. }
                    | ObjectPayload::MapIterator { .. }
                    | ObjectPayload::Set { .. }
                    | ObjectPayload::WeakMap { .. }
                    | ObjectPayload::WeakSet { .. }
                    | ObjectPayload::WeakRef { .. }
                    | ObjectPayload::FinalizationRegistry(_)
                    | ObjectPayload::SetIterator { .. }
                    | ObjectPayload::ForInIterator(_)
                    | ObjectPayload::Primitive(_)
                    | ObjectPayload::GlobalObject { .. }
                    | ObjectPayload::Error
                    | ObjectPayload::StringIterator { .. }
                    | ObjectPayload::RegExpStringIterator { .. }
                    | ObjectPayload::NativeFunction { .. }
                    | ObjectPayload::BoundFunction { .. }
                    | ObjectPayload::BytecodeFunction { .. }
                    | ObjectPayload::Generator { .. }
                    | ObjectPayload::AsyncGenerator(_) => None,
                }
            };
            if let Some(payload) = payload {
                let payload = match payload {
                    Ok(value) => value,
                    Err(atom) => Value::Symbol(SymbolRef::from_borrowed_atom(self.clone(), atom)?),
                };
                return Ok(NativeConversion::Value(payload));
            }
        }
        let message = match kind {
            PrimitiveKind::Number => "not a number",
            PrimitiveKind::String => "not a string",
            PrimitiveKind::Boolean => "not a boolean",
            PrimitiveKind::Symbol => "not a symbol",
            PrimitiveKind::BigInt => "not a BigInt",
        };
        Ok(NativeConversion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            message,
        )?))
    }

    pub(crate) fn call_string_prototype_char_at(
        &self,
        realm: ContextId,
        selector: StringCharAtKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String character method did not receive a generic invocation",
            ));
        };
        let string = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String character method argv was not padded",
        ))?;
        let mut index = match self.native_to_number(realm, argument)? {
            NativeConversion::Value(value) => crate::engine::value::number::to_int32_sat(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = i32::try_from(string.len()).map_err(|_| {
            RuntimeError::Invariant("String length exceeded QuickJS's signed index range")
        })?;
        if selector == StringCharAtKind::At && index < 0 {
            index += length;
        }
        if index < 0 || index >= length {
            return Ok(Completion::Return(match selector {
                StringCharAtKind::At => Value::Undefined,
                StringCharAtKind::CharAt => Value::String(JsString::from_static("")),
            }));
        }
        let index =
            usize::try_from(index).expect("validated non-negative String index always fits usize");
        let unit = string.code_unit_at(index).ok_or(RuntimeError::Invariant(
            "validated String character index did not name a code unit",
        ))?;
        Ok(Completion::Return(Value::String(JsString::from_code_unit(
            unit,
        ))))
    }

    pub(crate) fn call_string_prototype_iterator(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String.prototype iterator did not receive a generic invocation",
            ));
        };
        let string = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Object(
            self.new_string_iterator(realm, string)?,
        )))
    }

    pub(crate) fn call_string_iterator_next(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        match self.call_string_iterator_next_raw(realm, invocation)? {
            NativeInvokeOutcome::Completion(completion) => Ok(completion),
            NativeInvokeOutcome::IteratorNextRaw { value, done } => Ok(Completion::Return(
                Value::Object(self.new_iterator_result(realm, value, done)?),
            )),
        }
    }

    /// Execute the QuickJS `JS_CFUNC_iterator_next` half of String Iterator
    /// without materializing the public iterator-result object. The ordinary
    /// JavaScript call adapter above wraps this outcome; the VM's direct-native
    /// `ForOfNext` path consumes it as-is.
    pub(crate) fn call_string_iterator_next_raw(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String Iterator next did not receive an iterator-next invocation",
            ));
        };
        let Value::Object(iterator) = this_value else {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "String Iterator object expected",
                )?,
            )));
        };
        let branded = matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(iterator.object_id())?
                .payload,
            ObjectPayload::StringIterator { .. }
        );
        if !branded {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "String Iterator object expected",
                )?,
            )));
        }
        let value = self
            .0
            .state
            .borrow_mut()
            .heap
            .string_iterator_next(iterator.object_id())?;
        let (value, done) = match value {
            Some(value) => (Value::String(value), false),
            None => (Value::Undefined, true),
        };
        Ok(NativeInvokeOutcome::IteratorNextRaw { value, done })
    }

    pub(crate) fn call_string_prototype_char_code_at(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String charCodeAt did not receive a generic invocation",
            ));
        };
        let string = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String charCodeAt argv was not padded",
        ))?;
        let index = match self.native_to_number(realm, argument)? {
            NativeConversion::Value(value) => crate::engine::value::number::to_int32_sat(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let Some(unit) = usize::try_from(index)
            .ok()
            .and_then(|index| string.code_unit_at(index))
        else {
            return Ok(Completion::Return(Value::Float(f64::NAN)));
        };
        Ok(Completion::Return(Value::Int(i32::from(unit))))
    }

    pub(crate) fn call_string_prototype_code_point_at(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String codePointAt did not receive a generic invocation",
            ));
        };
        let string = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String codePointAt argv was not padded",
        ))?;
        let index = match self.native_to_number(realm, argument)? {
            NativeConversion::Value(value) => crate::engine::value::number::to_int32_sat(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let Some(code_point) = usize::try_from(index)
            .ok()
            .and_then(|index| string.code_point_at(index))
        else {
            return Ok(Completion::Return(Value::Undefined));
        };
        Ok(Completion::Return(Value::Int(
            i32::try_from(code_point).expect("a Unicode code point always fits i32"),
        )))
    }

    pub(crate) fn call_string_prototype_concat(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String concat did not receive a generic invocation",
            ));
        };
        let receiver = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if arguments.actual_arg_count == 0 {
            return Ok(Completion::Return(Value::String(receiver)));
        }

        let mut result = receiver;
        for argument in &arguments.readable[..arguments.actual_arg_count] {
            let chunk = match argument {
                // QuickJS `JS_ConcatString` accepts an existing rope without
                // routing it back through `JS_ToString`/linearization.
                Value::String(value) => value.clone(),
                _ => match self.native_to_js_string(realm, argument)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                },
            };
            result = result.try_concat(&chunk).map_err(Error::from)?;
        }
        Ok(Completion::Return(Value::String(result)))
    }

    pub(crate) fn call_string_prototype_well_formed(
        &self,
        realm: ContextId,
        selector: StringWellFormedKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String well-formed method did not receive a generic invocation",
            ));
        };
        let string = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(match selector {
            StringWellFormedKind::IsWellFormed => Value::Bool(string.is_well_formed()),
            StringWellFormedKind::ToWellFormed => Value::String(string.to_well_formed()),
        }))
    }

    pub(crate) fn call_primitive_prototype_to_string(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "primitive toString did not receive a generic invocation",
            ));
        };
        let value = match self.primitive_this_value(realm, kind, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        match (kind, value) {
            (PrimitiveKind::Number, value @ (Value::Int(_) | Value::Float(_))) => {
                let number = value.as_number().ok_or(RuntimeError::Invariant(
                    "Number brand extraction did not return a Number",
                ))?;
                let radix_argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "Number.prototype.toString argv was not padded",
                ))?;
                let radix = if matches!(radix_argument, Value::Undefined) {
                    10
                } else {
                    let radix = match self.native_to_number(realm, radix_argument)? {
                        NativeConversion::Value(value) => {
                            crate::engine::value::number::to_int32_sat(value)
                        }
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if !(2..=36).contains(&radix) {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Range,
                            "radix must be between 2 and 36",
                        )?));
                    }
                    u32::try_from(radix).expect("a Number radix between 2 and 36 always fits u32")
                };
                let formatted = crate::engine::value::number::to_string_radix(number, radix)
                    .map_err(|error| match error {
                        crate::engine::value::number::NumberFormatError::InvalidRadix => {
                            RuntimeError::Invariant(
                                "validated Number radix was rejected by the formatter",
                            )
                        }
                        crate::engine::value::number::NumberFormatError::InvalidDigits => {
                            RuntimeError::Invariant(
                                "Number radix formatting reported a digit-count error",
                            )
                        }
                    })?;
                Ok(Completion::Return(Value::String(JsString::try_from_utf8(
                    &formatted,
                )?)))
            }
            (PrimitiveKind::String, Value::String(value)) => {
                Ok(Completion::Return(Value::String(value)))
            }
            (PrimitiveKind::Boolean, Value::Bool(value)) => Ok(Completion::Return(Value::String(
                JsString::from_static(if value { "true" } else { "false" }),
            ))),
            (PrimitiveKind::Symbol, Value::Symbol(value)) => Ok(Completion::Return(Value::String(
                self.symbol_descriptive_string(&value)?,
            ))),
            (PrimitiveKind::BigInt, Value::BigInt(value)) => {
                let radix_argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "BigInt.prototype.toString argv was not padded",
                ))?;
                let radix = if matches!(radix_argument, Value::Undefined) {
                    10
                } else {
                    let radix = match self.native_to_number(realm, radix_argument)? {
                        NativeConversion::Value(value) => {
                            crate::engine::value::number::to_int32_sat(value)
                        }
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if !(2..=36).contains(&radix) {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Range,
                            "radix must be between 2 and 36",
                        )?));
                    }
                    u32::try_from(radix).expect("a BigInt radix between 2 and 36 fits u32")
                };
                if value.exceeds_allocation_limit()
                    && (value.is_negative() || !radix.is_power_of_two())
                {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Range,
                        "BigInt is too large to allocate",
                    )?));
                }
                let text = value
                    .to_string_radix(radix)
                    .map_err(|_| RuntimeError::Invariant("validated BigInt radix was rejected"))?;
                Ok(Completion::Return(Value::String(JsString::try_from_utf8(
                    &text,
                )?)))
            }
            _ => Err(RuntimeError::Invariant(
                "unimplemented primitive toString reached native dispatch",
            )),
        }
    }

    pub(crate) fn finish_number_format(
        &self,
        realm: ContextId,
        result: Result<String, crate::engine::value::number::NumberFormatError>,
    ) -> Result<Completion, RuntimeError> {
        match result {
            Ok(value) => Ok(Completion::Return(Value::String(JsString::try_from_utf8(
                &value,
            )?))),
            Err(crate::engine::value::number::NumberFormatError::InvalidDigits) => {
                Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "invalid number of digits",
                )?))
            }
            Err(crate::engine::value::number::NumberFormatError::InvalidRadix) => {
                Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "radix must be between 2 and 36",
                )?))
            }
        }
    }

    pub(crate) fn call_number_prototype_format(
        &self,
        realm: ContextId,
        kind: NumberFormatKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Number prototype formatter did not receive a generic invocation",
            ));
        };
        // QuickJS performs the receiver brand check before touching any
        // argument, including user-code coercion on the digit/radix value.
        let value = match self.primitive_this_value(realm, PrimitiveKind::Number, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let number = value.as_number().ok_or(RuntimeError::Invariant(
            "Number formatter brand extraction did not return a Number",
        ))?;

        let result = match kind {
            NumberFormatKind::LocaleString => {
                crate::engine::value::number::to_string_radix(number, 10)
            }
            NumberFormatKind::Fixed => {
                let digits = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "Number.prototype.toFixed argv was not padded",
                ))?;
                let digits = match self.native_to_number(realm, digits)? {
                    NativeConversion::Value(value) => {
                        crate::engine::value::number::to_int32_sat(value)
                    }
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                crate::engine::value::number::to_fixed(number, digits)
            }
            NumberFormatKind::Exponential => {
                let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "Number.prototype.toExponential argv was not padded",
                ))?;
                // The pinned C implementation runs ToInt32Sat even for
                // undefined, then records undefined as the FREE-format case.
                let converted = match self.native_to_number(realm, argument)? {
                    NativeConversion::Value(value) => {
                        crate::engine::value::number::to_int32_sat(value)
                    }
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let digits = (!matches!(argument, Value::Undefined)).then_some(converted);
                crate::engine::value::number::to_exponential(number, digits)
            }
            NumberFormatKind::Precision => {
                let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "Number.prototype.toPrecision argv was not padded",
                ))?;
                let precision = if matches!(argument, Value::Undefined) {
                    None
                } else {
                    match self.native_to_number(realm, argument)? {
                        NativeConversion::Value(value) => {
                            Some(crate::engine::value::number::to_int32_sat(value))
                        }
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                };
                crate::engine::value::number::to_precision(number, precision)
            }
        };
        self.finish_number_format(realm, result)
    }

    pub(crate) fn call_number_predicate(
        &self,
        kind: NumberPredicateKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Number predicate did not receive a generic invocation",
            ));
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Number predicate argv was not padded",
        ))?;
        let result = argument.as_number().is_some_and(|number| match kind {
            NumberPredicateKind::IsNaN => number.is_nan(),
            NumberPredicateKind::IsFinite => number.is_finite(),
            NumberPredicateKind::IsInteger => number.is_finite() && number.fract() == 0.0,
            NumberPredicateKind::IsSafeInteger => {
                number.is_finite()
                    && number.fract() == 0.0
                    && number.abs() <= 9_007_199_254_740_991.0
            }
        });
        Ok(Completion::Return(Value::Bool(result)))
    }

    pub(crate) fn call_bigint_as_n(
        &self,
        realm: ContextId,
        kind: BigIntAsNKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "BigInt truncation method did not receive a generic call",
            ));
        };
        let bits = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "BigInt truncation bits argument was not padded",
        ))?;
        let bits = match self.native_to_index(realm, bits)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let value = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "BigInt truncation value argument was not padded",
        ))?;
        let value = match self.native_to_bigint(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let value = match kind {
            BigIntAsNKind::AsUintN => value.as_uint_n(bits),
            BigIntAsNKind::AsIntN => value.as_int_n(bits),
        };
        match value {
            Ok(value) => Ok(Completion::Return(Value::BigInt(value))),
            Err(_) => Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "BigInt is too large to allocate",
            )?)),
        }
    }

    pub(crate) fn call_symbol_registry(
        &self,
        realm: ContextId,
        kind: SymbolRegistryKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Symbol registry method did not receive a generic call",
            ));
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Symbol registry argv was not padded",
        ))?;
        match kind {
            SymbolRegistryKind::For => {
                let key = match self.native_to_js_string(realm, argument)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                Ok(Completion::Return(Value::Symbol(self.symbol_for(&key)?)))
            }
            SymbolRegistryKind::KeyFor => {
                let Value::Symbol(symbol) = argument else {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a symbol",
                    )?));
                };
                Ok(Completion::Return(
                    self.symbol_key_for(symbol)?
                        .map_or(Value::Undefined, Value::String),
                ))
            }
        }
    }

    pub(crate) fn call_symbol_prototype_description(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Symbol.prototype.description received the wrong native invocation",
            ));
        };
        let value = match self.primitive_this_value(realm, PrimitiveKind::Symbol, this_value)? {
            NativeConversion::Value(Value::Symbol(value)) => value,
            NativeConversion::Value(_) => {
                return Err(RuntimeError::Invariant(
                    "Symbol brand extraction did not return a Symbol",
                ));
            }
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(
            self.symbol_description(&value)?
                .map_or(Value::Undefined, Value::String),
        ))
    }

    pub(crate) fn call_primitive_prototype_value_of(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "primitive valueOf did not receive a generic invocation",
            ));
        };
        match self.primitive_this_value(realm, kind, this_value)? {
            NativeConversion::Value(value) => Ok(Completion::Return(value)),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    #[cfg(test)]
    pub(crate) fn call_active_frame_probe(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match arguments.readable.first() {
            Some(Value::Object(value))
                if matches!(arguments.readable.get(1), Some(Value::Bool(false))) =>
            {
                Ok(Completion::Throw(Value::Object(value.clone())))
            }
            Some(Value::Object(callback)) => {
                let callback = self.callable_from_value(Value::Object(callback.clone()))?;
                let active_function = self.active_function()?;
                self.call_internal(
                    realm,
                    &callback,
                    Value::Undefined,
                    &[Value::Object(active_function)],
                )
            }
            Some(Value::Bool(false)) => Ok(Completion::Throw(Value::String(
                JsString::from_static("active frame probe throw"),
            ))),
            Some(Value::Bool(true)) => {
                Err(RuntimeError::Invariant("active frame probe engine error"))
            }
            Some(_) => Err(RuntimeError::Invariant(
                "active frame probe received an unsupported command",
            )),
            None => {
                let snapshot = self.0.state.borrow().active_frames.clone();
                self.0
                    .state
                    .borrow_mut()
                    .active_frame_probe_snapshots
                    .push(snapshot);
                Ok(Completion::Return(Value::Undefined))
            }
        }
    }
}
