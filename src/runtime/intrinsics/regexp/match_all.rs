//! `RegExp.prototype[Symbol.matchAll]` and RegExp String Iterator.

use super::super::super::*;
use super::match_protocol::advance_string_index;

impl Runtime {
    /// Rust port of pinned QuickJS `js_regexp_Symbol_matchAll`.
    ///
    /// The input conversion, species lookup, flags conversion, construction,
    /// original `lastIndex` read and matcher write deliberately retain their
    /// upstream order. The iterator caches `g` and full-Unicode mode from the
    /// original flags string rather than consulting the constructed matcher.
    pub(super) fn call_regexp_symbol_match_all(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value: regexp } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp @@matchAll did not receive a generic invocation",
            ));
        };
        let Value::Object(regexp) = regexp else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let input = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "RegExp @@matchAll input argv was not padded",
        ))?;
        let input = match self.native_to_js_string(realm, input)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let constructor = match self.regexp_species_constructor(realm, &regexp)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let flags_key = self.intern_property_key("flags")?;
        let flags = match self.get_property_in_realm(realm, &regexp, &flags_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let flags = match self.native_to_js_string(realm, &flags)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let matcher = match self.construct_internal(
            realm,
            &constructor,
            &constructor,
            &[Value::Object(regexp.clone()), Value::String(flags.clone())],
        )? {
            Completion::Return(Value::Object(value)) => value,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "RegExp matchAll species constructor returned a primitive",
                ));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let last_index_key = self.intern_property_key("lastIndex")?;
        let last_index = match self.get_property_in_realm(realm, &regexp, &last_index_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let last_index = match self.native_to_length(realm, &last_index)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if let Some(value) = self.set_property_or_throw(
            realm,
            &matcher,
            &last_index_key,
            Value::number(last_index as f64),
        )? {
            return Ok(Completion::Throw(value));
        }

        let global = flags.utf16_units().any(|unit| unit == u16::from(b'g'));
        let full_unicode = flags
            .utf16_units()
            .any(|unit| unit == u16::from(b'u') || unit == u16::from(b'v'));
        Ok(Completion::Return(Value::Object(
            self.new_regexp_string_iterator(realm, &matcher, input, global, full_unicode)?,
        )))
    }

    fn new_regexp_string_iterator(
        &self,
        realm: ContextId,
        regexp: &ObjectRef,
        string: JsString,
        global: bool,
        full_unicode: bool,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !regexp.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("RegExp String Iterator matcher"));
        }
        let prototype_id = self.regexp_realm_data(realm)?.string_iterator_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype_id)?;
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::regexp_string_iterator(
                shape,
                Vec::new(),
                regexp.object_id(),
                string,
                global,
                full_unicode,
            )) {
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

    pub(in crate::runtime) fn call_regexp_string_iterator_next(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        match self.call_regexp_string_iterator_next_raw(realm, invocation)? {
            NativeInvokeOutcome::Completion(completion) => Ok(completion),
            NativeInvokeOutcome::IteratorNextRaw { value, done } => Ok(Completion::Return(
                Value::Object(self.new_iterator_result(realm, value, done)?),
            )),
        }
    }

    /// Execute QuickJS's `JS_CFUNC_iterator_next` ABI without allocating the
    /// public iterator-result object. Exceptions leave the iterator's `done`
    /// bit unchanged so a later call retries from the matcher state produced
    /// by the failed observable operation.
    pub(in crate::runtime) fn call_regexp_string_iterator_next_raw(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp String Iterator next did not receive an iterator-next invocation",
            ));
        };
        let Value::Object(iterator) = this_value else {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "RegExp String Iterator object expected",
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
            ObjectPayload::RegExpStringIterator { .. }
        );
        if !branded {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "RegExp String Iterator object expected",
                )?,
            )));
        }
        let (regexp_id, string, global, full_unicode, done) = self
            .0
            .state
            .borrow()
            .heap
            .regexp_string_iterator_state(iterator.object_id())?;
        if done {
            return Ok(NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Undefined,
                done: true,
            });
        }

        let regexp = ObjectRef::from_borrowed_handle(self.clone(), regexp_id)?;
        let matched = match self.regexp_exec_abstract(
            realm,
            Value::Object(regexp.clone()),
            Value::String(string.clone()),
        )? {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
            }
        };
        let matched = match matched {
            Value::Null => {
                self.0
                    .state
                    .borrow_mut()
                    .heap
                    .finish_regexp_string_iterator(iterator.object_id())?;
                return Ok(NativeInvokeOutcome::IteratorNextRaw {
                    value: Value::Undefined,
                    done: true,
                });
            }
            Value::Object(value) => value,
            Value::Undefined
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => {
                return Err(RuntimeError::Invariant(
                    "RegExpExec returned neither an object nor null",
                ));
            }
        };

        if global {
            let zero = self.intern_property_key("0")?;
            let match_string = match self.get_property_in_realm(realm, &matched, &zero)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => {
                    return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
                }
            };
            let match_string = match self.native_to_js_string(realm, &match_string)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => {
                    return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
                }
            };
            if match_string.is_empty() {
                let last_index = self.intern_property_key("lastIndex")?;
                let current = match self.get_property_in_realm(realm, &regexp, &last_index)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => {
                        return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
                    }
                };
                let current = match self.native_to_length(realm, &current)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
                    }
                };
                let next = advance_string_index(&string, current, full_unicode);
                if let Some(value) = self.set_property_or_throw(
                    realm,
                    &regexp,
                    &last_index,
                    Value::number(next as f64),
                )? {
                    return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
                }
            }
        } else {
            self.0
                .state
                .borrow_mut()
                .heap
                .finish_regexp_string_iterator(iterator.object_id())?;
        }

        Ok(NativeInvokeOutcome::IteratorNextRaw {
            value: Value::Object(matched),
            done: false,
        })
    }
}
