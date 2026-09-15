use crate::engine::api::error::{NativeErrorKind, NativeErrorMessage};
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

pub(crate) mod constructor;
pub(crate) mod globals;
pub(crate) mod numeric;
pub(crate) mod text;

impl Runtime {
    pub(crate) fn call_primitive_constructor(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        constructor::finish(
            self,
            realm,
            constructor::PrimitiveConstructorStep::start(
                self,
                realm,
                kind,
                &invocation,
                arguments,
            )?,
        )
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
        globals::finish(
            self,
            realm,
            globals::GlobalStep::start(
                self,
                realm,
                globals::GlobalKind::Parse(kind),
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_global_number_predicate(
        &self,
        realm: ContextId,
        kind: GlobalNumberPredicateKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        globals::finish(
            self,
            realm,
            globals::GlobalStep::start(
                self,
                realm,
                globals::GlobalKind::Predicate(kind),
                &invocation,
                arguments,
            )?,
        )
    }

    fn finish_global_uri_codec(
        &self,
        realm: ContextId,
        kind: GlobalUriCodecKind,
        input: JsString,
    ) -> Result<Completion, RuntimeError> {
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

    pub(crate) fn call_global_uri_codec(
        &self,
        realm: ContextId,
        kind: GlobalUriCodecKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        globals::finish(
            self,
            realm,
            globals::GlobalStep::start(
                self,
                realm,
                globals::GlobalKind::Uri(kind),
                &invocation,
                arguments,
            )?,
        )
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
        text::finish(
            self,
            realm,
            text::ScalarTextStep::start(
                self,
                realm,
                text::ScalarTextKind::CharAt(selector),
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_string_prototype_iterator(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let arguments = NativeArguments {
            readable: Vec::new(),
            actual_arg_count: 0,
        };
        text::finish(
            self,
            realm,
            text::ScalarTextStep::start(
                self,
                realm,
                text::ScalarTextKind::Iterator,
                &invocation,
                &arguments,
            )?,
        )
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
        text::finish(
            self,
            realm,
            text::ScalarTextStep::start(
                self,
                realm,
                text::ScalarTextKind::CharCodeAt,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_string_prototype_code_point_at(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        text::finish(
            self,
            realm,
            text::ScalarTextStep::start(
                self,
                realm,
                text::ScalarTextKind::CodePointAt,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_string_prototype_concat(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        text::finish(
            self,
            realm,
            text::ScalarTextStep::start(
                self,
                realm,
                text::ScalarTextKind::Concat,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_string_prototype_well_formed(
        &self,
        realm: ContextId,
        selector: StringWellFormedKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let arguments = NativeArguments {
            readable: Vec::new(),
            actual_arg_count: 0,
        };
        text::finish(
            self,
            realm,
            text::ScalarTextStep::start(
                self,
                realm,
                text::ScalarTextKind::WellFormed(selector),
                &invocation,
                &arguments,
            )?,
        )
    }

    fn finish_branded_to_string(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        value: Value,
        radix: u32,
    ) -> Result<Completion, RuntimeError> {
        match (kind, value) {
            (PrimitiveKind::Number, value @ (Value::Int(_) | Value::Float(_))) => {
                let number = value.as_number().ok_or(RuntimeError::Invariant(
                    "Number brand extraction did not return a Number",
                ))?;
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

    pub(crate) fn call_primitive_prototype_to_string(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        numeric::finish(
            self,
            realm,
            numeric::NumericStep::start(
                self,
                realm,
                numeric::NumericKind::ToString(kind),
                &invocation,
                arguments,
            )?,
        )
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
        numeric::finish(
            self,
            realm,
            numeric::NumericStep::start(
                self,
                realm,
                numeric::NumericKind::Format(kind),
                &invocation,
                arguments,
            )?,
        )
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
        numeric::finish(
            self,
            realm,
            numeric::NumericStep::start(
                self,
                realm,
                numeric::NumericKind::BigIntAsN(kind),
                &invocation,
                arguments,
            )?,
        )
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
            SymbolRegistryKind::For => globals::finish(
                self,
                realm,
                globals::GlobalStep::start(
                    self,
                    realm,
                    globals::GlobalKind::SymbolFor,
                    &NativeInvocation::Call {
                        this_value: Value::Undefined,
                    },
                    arguments,
                )?,
            ),
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
        super::function::invoke::finish(
            self,
            realm,
            self.prepare_active_frame_probe(realm, arguments)?,
        )
    }

    #[cfg(test)]
    pub(crate) fn prepare_active_frame_probe(
        &self,
        _realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<super::function::invoke::InvokeStep, RuntimeError> {
        use super::function::invoke::InvokeStep;
        let completion = match arguments.readable.first() {
            Some(Value::Object(value))
                if matches!(arguments.readable.get(1), Some(Value::Bool(false))) =>
            {
                Ok(Completion::Throw(Value::Object(value.clone())))
            }
            Some(Value::Object(callback)) => {
                let callback = self.callable_from_value(Value::Object(callback.clone()))?;
                let active_function = self.active_function()?;
                return Ok(InvokeStep::Call(Box::new(
                    super::function::invoke::InvokeCall {
                        target: crate::engine::vm::call::DirectCallTarget::Callable(callback),
                        receiver: Value::Undefined,
                        arguments: vec![Value::Object(active_function)],
                    },
                )));
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
                let snapshot = self.0.state.borrow().active_frames.to_vec();
                self.0
                    .state
                    .borrow_mut()
                    .active_frame_probe_snapshots
                    .push(snapshot);
                Ok(Completion::Return(Value::Undefined))
            }
        }?;
        Ok(InvokeStep::Complete(completion))
    }
}
