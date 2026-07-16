//! Builtin and abstract RegExp execution.

use std::rc::Rc;

use crate::heap::{RegExpNativeKind, RegExpObjectData};
use crate::regexp::{CompiledRegExp, ExecError, RegExpFlags, RegExpMatch, execute_with_interrupt};

use super::super::super::*;

impl Runtime {
    pub(super) fn call_regexp_exec_native(
        &self,
        realm: ContextId,
        kind: RegExpNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp exec/test did not receive a generic invocation",
            ));
        };
        let input = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "RegExp exec/test input argv was not padded",
        ))?;
        match kind {
            RegExpNativeKind::Exec => self.builtin_regexp_exec(realm, &this_value, input),
            RegExpNativeKind::Test => {
                match self.regexp_exec_abstract(realm, this_value, input.clone())? {
                    Completion::Return(value) => Ok(Completion::Return(Value::Bool(!matches!(
                        value,
                        Value::Null
                    )))),
                    Completion::Throw(value) => Ok(Completion::Throw(value)),
                }
            }
            RegExpNativeKind::Constructor
            | RegExpNativeKind::Species
            | RegExpNativeKind::Compile
            | RegExpNativeKind::Source
            | RegExpNativeKind::Flags
            | RegExpNativeKind::Flag(_)
            | RegExpNativeKind::ToString
            | RegExpNativeKind::Replace
            | RegExpNativeKind::Match
            | RegExpNativeKind::MatchAll
            | RegExpNativeKind::Search
            | RegExpNativeKind::Split => Err(RuntimeError::Invariant(
                "non-exec RegExp selector reached exec dispatch",
            )),
        }
    }

    pub(super) fn regexp_exec_abstract(
        &self,
        realm: ContextId,
        regexp: Value,
        input: Value,
    ) -> Result<Completion, RuntimeError> {
        let exec_key = self.intern_property_key("exec")?;
        // JS_RegExpExec starts with the ordinary Get(R, "exec") for every
        // receiver. In particular, nullish receivers expose the pinned
        // property-read diagnostic rather than a generic object check.
        let method = match regexp {
            Value::Null | Value::Undefined => {
                let base = if matches!(regexp, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    &format!("cannot read property 'exec' of {base}"),
                )?));
            }
            _ => self.get_value_property_in_realm(realm, regexp.clone(), &exec_key)?,
        };
        let method = match method {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if self.regexp_value_is_callable(&method)? {
            let callable = self.callable_from_value(method)?;
            let result = self.call_internal(realm, &callable, regexp, &[input])?;
            return match result {
                Completion::Return(value @ (Value::Object(_) | Value::Null)) => {
                    Ok(Completion::Return(value))
                }
                Completion::Return(_) => Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "RegExp exec method must return an object or null",
                )?)),
                Completion::Throw(value) => Ok(Completion::Throw(value)),
            };
        }
        self.builtin_regexp_exec(realm, &regexp, &input)
    }

    fn builtin_regexp_exec(
        &self,
        realm: ContextId,
        this_value: &Value,
        input_value: &Value,
    ) -> Result<Completion, RuntimeError> {
        // Brand validation precedes input conversion.
        let Value::Object(object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "RegExp object expected",
            )?));
        };
        let compiled = {
            let state = self.0.state.borrow();
            match &state.heap.object(object.object_id())?.payload {
                ObjectPayload::RegExp(RegExpObjectData::Compiled { program, .. }) => {
                    Some((program.clone(), program.flags()))
                }
                ObjectPayload::RegExp(RegExpObjectData::Uninitialized) => {
                    return Err(RuntimeError::Invariant(
                        "observable RegExp object was not initialized",
                    ));
                }
                ObjectPayload::Ordinary
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::Date(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. }
                | ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. } => None,
            }
        };
        let Some((program, flags)) = compiled else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "RegExp object expected",
            )?));
        };
        let input = match self.native_to_js_string(realm, input_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        // Even a non-global RegExp observes ToLength(lastIndex); only after
        // that conversion does QuickJS force its local starting position to 0.
        let last_index = match self.regexp_last_index(realm, object)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let updates_last_index =
            flags.contains(RegExpFlags::GLOBAL) || flags.contains(RegExpFlags::STICKY);
        let start = if updates_last_index { last_index } else { 0 };
        let input_units = input.utf16_units().collect::<Vec<_>>();

        let matched = if start > input_units.len() as u64 {
            None
        } else {
            match execute_with_interrupt(
                program.as_ref(),
                &input_units,
                usize::try_from(start).expect("RegExp start was bounded by String length"),
                // The runtime interrupt callback is not exposed at this layer
                // yet.  Keep the executor boundary interrupt-aware now so a
                // later host hook is a closure substitution rather than a
                // semantic rewrite of builtin exec.
                || false,
            ) {
                Ok(value) => value,
                Err(ExecError::OutOfMemory) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Internal,
                        "out of memory in regexp execution",
                    )?));
                }
                Err(ExecError::Interrupted) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Internal,
                        "interrupted",
                    )?));
                }
                Err(ExecError::InvalidProgram(_)) => {
                    return Err(RuntimeError::Invariant(
                        "compiled RegExp program failed executor validation",
                    ));
                }
                Err(ExecError::StartOutOfBounds { .. }) => {
                    return Err(RuntimeError::Invariant(
                        "bounded RegExp start was rejected by executor",
                    ));
                }
            }
        };

        let Some(matched) = matched else {
            if updates_last_index
                && let Some(exception) = self.set_regexp_last_index(realm, object, 0)?
            {
                return Ok(Completion::Throw(exception));
            }
            return Ok(Completion::Return(Value::Null));
        };

        let complete = matched.capture(0).ok_or(RuntimeError::Invariant(
            "successful RegExp execution omitted capture zero",
        ))?;
        if updates_last_index {
            let end = i32::try_from(complete.end).map_err(|_| {
                RuntimeError::Invariant("RegExp match end exceeded signed String range")
            })?;
            // This write happens before any result/indices allocation.
            if let Some(exception) = self.set_regexp_last_index(realm, object, end)? {
                return Ok(Completion::Throw(exception));
            }
        }

        self.build_regexp_result(realm, input, program, matched)
            .map(Completion::Return)
    }

    fn regexp_last_index(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<u64>, RuntimeError> {
        let key = self.intern_property_key("lastIndex")?;
        let descriptor = self
            .get_own_property(object, &key)?
            .ok_or(RuntimeError::Invariant(
                "genuine RegExp object had no lastIndex property",
            ))?;
        let CompleteOrdinaryPropertyDescriptor::Data { value, .. } = descriptor else {
            return Err(RuntimeError::Invariant(
                "RegExp lastIndex became an accessor",
            ));
        };
        self.native_to_length(realm, &value)
    }

    pub(super) fn set_regexp_last_index(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        value: i32,
    ) -> Result<Option<Value>, RuntimeError> {
        let key = self.intern_property_key("lastIndex")?;
        self.set_property_or_throw(realm, object, &key, Value::Int(value))
    }

    fn build_regexp_result(
        &self,
        realm: ContextId,
        input: JsString,
        program: Rc<CompiledRegExp>,
        matched: RegExpMatch,
    ) -> Result<Value, RuntimeError> {
        let mut captures = Vec::with_capacity(matched.captures().len());
        for range in matched.captures() {
            captures.push(match range {
                Some(range) => Value::String(input.sub_string(range.start, range.end)),
                None => Value::Undefined,
            });
        }
        let result = self.new_array_from_values(realm, captures)?;
        let complete = matched.capture(0).ok_or(RuntimeError::Invariant(
            "successful RegExp result omitted capture zero",
        ))?;
        self.define_regexp_result_property(
            &result,
            "index",
            Value::Int(i32::try_from(complete.start).map_err(|_| {
                RuntimeError::Invariant("RegExp match start exceeded signed String range")
            })?),
        )?;
        self.define_regexp_result_property(&result, "input", Value::String(input.clone()))?;
        // Named captures remain outside R1a, but QuickJS always publishes the
        // property even when its value is undefined.
        self.define_regexp_result_property(&result, "groups", Value::Undefined)?;

        if program.flags().contains(RegExpFlags::HAS_INDICES) {
            let mut values = Vec::with_capacity(matched.captures().len());
            for range in matched.captures() {
                values.push(match range {
                    Some(range) => {
                        let start = i32::try_from(range.start).map_err(|_| {
                            RuntimeError::Invariant(
                                "RegExp capture start exceeded signed String range",
                            )
                        })?;
                        let end = i32::try_from(range.end).map_err(|_| {
                            RuntimeError::Invariant(
                                "RegExp capture end exceeded signed String range",
                            )
                        })?;
                        Value::Object(self.new_array_from_values(
                            realm,
                            vec![Value::Int(start), Value::Int(end)],
                        )?)
                    }
                    None => Value::Undefined,
                });
            }
            let indices = self.new_array_from_values(realm, values)?;
            self.define_regexp_result_property(&indices, "groups", Value::Undefined)?;
            self.define_regexp_result_property(&result, "indices", Value::Object(indices))?;
        }
        Ok(Value::Object(result))
    }

    fn define_regexp_result_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        if !self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "fresh RegExp result property definition was rejected",
            ));
        }
        Ok(())
    }

    fn regexp_value_is_callable(&self, value: &Value) -> Result<bool, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(false);
        };
        let state = self.0.state.borrow();
        Ok(matches!(
            state.heap.object(object.object_id())?.payload,
            ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
        ))
    }
}
