//! `RegExp.prototype[Symbol.replace]`.

use std::rc::Rc;

use crate::heap::{RegExpFlagKind, RegExpNativeKind, RegExpObjectData};
use crate::regexp::{CompiledRegExp, ExecError, RegExpFlags, execute_with_interrupt};
use crate::value::ReplacementStringBuffer;

use super::super::super::*;
use super::super::replacement::{
    SubstitutionCaptures, SubstitutionInput, SubstitutionMatch, SubstitutionStatus,
};
use super::match_protocol::advance_string_index;

const MAX_REPLACER_ARGUMENTS: usize = 65_534;

struct CollectedReplace {
    input: JsString,
    functional: Option<CallableRef>,
    replacement: Option<JsString>,
    output: ReplacementStringBuffer,
    results: Vec<ObjectRef>,
    zero: PropertyKey,
}

struct StandardRegExpReplace {
    program: Rc<CompiledRegExp>,
    last_index: Value,
}

impl Runtime {
    /// Rust port of pinned QuickJS `js_regexp_Symbol_replace`, including its
    /// raw standard-RegExp predicate and direct matcher.
    pub(super) fn call_regexp_symbol_replace(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value: regexp } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp @@replace did not receive a generic invocation",
            ));
        };
        let Value::Object(regexp) = regexp else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let input_value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "RegExp @@replace input argv was not padded",
        ))?;
        let replace_value = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "RegExp @@replace replacement argv was not padded",
        ))?;

        // QuickJS initializes the outer StringBuffer before the first
        // observable input conversion. Its error remains latched while all
        // required generic-path result getters and callbacks continue.
        let output = ReplacementStringBuffer::new(0);
        let input = match self.native_to_js_string(realm, input_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let functional = match replace_value {
            Value::Object(object) => self.as_callable(object)?,
            _ => None,
        };
        let replacement = if functional.is_none() {
            match self.native_to_js_string(realm, replace_value)? {
                NativeConversion::Value(value) => Some(value),
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            None
        };

        if functional.is_none()
            && let Some(standard) = self.standard_regexp_replace(&regexp)?
        {
            return self.call_standard_regexp_replace(
                realm,
                &regexp,
                &input,
                replacement
                    .as_ref()
                    .expect("non-functional replacement was not converted"),
                standard,
            );
        }

        let flags_key = self.intern_property_key("flags")?;
        let flags = match self.get_property_in_realm(realm, &regexp, &flags_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let flags = match self.native_to_js_string(realm, &flags)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let global = flags.utf16_units().any(|unit| unit == u16::from(b'g'));
        let full_unicode = global
            && flags
                .utf16_units()
                .any(|unit| unit == u16::from(b'u') || unit == u16::from(b'v'));
        let last_index = self.intern_property_key("lastIndex")?;
        if global
            && let Some(value) =
                self.set_property_or_throw(realm, &regexp, &last_index, Value::Int(0))?
        {
            return Ok(Completion::Throw(value));
        }

        let zero = self.intern_property_key("0")?;
        let mut results = Vec::<ObjectRef>::new();
        loop {
            let result = match self.regexp_exec_abstract(
                realm,
                Value::Object(regexp.clone()),
                Value::String(input.clone()),
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let result = match result {
                Value::Null => break,
                Value::Object(value) => value,
                _ => {
                    return Err(RuntimeError::Invariant(
                        "RegExpExec returned neither an object nor null",
                    ));
                }
            };
            if results.try_reserve(1).is_err() {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "out of memory",
                )?));
            }
            results.push(result.clone());
            if !global {
                break;
            }

            let matched = match self.get_property_in_realm(realm, &result, &zero)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let matched = match self.native_to_js_string(realm, &matched)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if matched.is_empty() {
                let current = match self.get_property_in_realm(realm, &regexp, &last_index)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let current = match self.native_to_length(realm, &current)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let next = advance_string_index(&input, current, full_unicode);
                if let Some(value) = self.set_property_or_throw(
                    realm,
                    &regexp,
                    &last_index,
                    Value::number(next as f64),
                )? {
                    return Ok(Completion::Throw(value));
                }
            }
        }

        self.finish_regexp_symbol_replace(
            realm,
            CollectedReplace {
                input,
                functional,
                replacement,
                output,
                results,
                zero,
            },
        )
    }

    #[inline(never)]
    fn finish_regexp_symbol_replace(
        &self,
        realm: ContextId,
        collected: CollectedReplace,
    ) -> Result<Completion, RuntimeError> {
        let CollectedReplace {
            input,
            functional,
            replacement,
            mut output,
            results,
            zero,
        } = collected;
        let length_key = self.intern_property_key("length")?;
        let index_key = self.intern_property_key("index")?;
        let groups_key = self.intern_property_key("groups")?;
        let mut next_source_position = 0_usize;

        for result in results {
            let capture_count = match self.get_property_in_realm(realm, &result, &length_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let capture_count = match self.native_to_number(realm, &capture_count)? {
                NativeConversion::Value(value) => Self::to_uint32_number(value),
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };

            let matched = match self.get_property_in_realm(realm, &result, &zero)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let matched = match self.native_to_js_string(realm, &matched)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };

            let position = match self.get_property_in_realm(realm, &result, &index_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let position = match self.native_to_length(realm, &position)? {
                NativeConversion::Value(value) => value.min(input.len() as u64),
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let position = usize::try_from(position).map_err(|_| {
                RuntimeError::Invariant("RegExp replace position did not fit usize")
            })?;

            let mut captures = Vec::<Value>::new();
            if captures.try_reserve(1).is_err() {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "out of memory",
                )?));
            }
            captures.push(Value::String(matched.clone()));
            for capture_index in 1..capture_count {
                let key = self.intern_property_key(&capture_index.to_string())?;
                let capture = match self.get_property_in_realm(realm, &result, &key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let capture = if matches!(capture, Value::Undefined) {
                    Value::Undefined
                } else {
                    match self.native_to_js_string(realm, &capture)? {
                        NativeConversion::Value(value) => Value::String(value),
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                };
                if captures.try_reserve(1).is_err() {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Internal,
                        "out of memory",
                    )?));
                }
                captures.push(capture);
            }

            let groups = match self.get_property_in_realm(realm, &result, &groups_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let replacement_text = if let Some(callable) = &functional {
                let extra_arguments = if matches!(groups, Value::Undefined) {
                    2
                } else {
                    3
                };
                let position_value = Value::Int(i32::try_from(position).map_err(|_| {
                    RuntimeError::Invariant("RegExp replace position exceeded signed range")
                })?);
                if captures.try_reserve(extra_arguments).is_err() {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Internal,
                        "out of memory",
                    )?));
                }
                captures.push(position_value);
                captures.push(Value::String(input.clone()));
                if !matches!(groups, Value::Undefined) {
                    captures.push(groups);
                }
                if captures.len() > MAX_REPLACER_ARGUMENTS {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Range,
                        "too many arguments in function call (only 65534 allowed)",
                    )?));
                }
                let result =
                    match self.call_internal(realm, callable, Value::Undefined, &captures)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                match self.native_to_js_string(realm, &result)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            } else {
                let named_captures = if matches!(groups, Value::Undefined) {
                    None
                } else {
                    match self.native_to_object(realm, groups)? {
                        NativeConversion::Value(value) => Some(value),
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                };
                let mut substitution = ReplacementStringBuffer::new(0);
                let status = self.append_get_substitution(
                    realm,
                    &mut substitution,
                    SubstitutionInput {
                        matched: SubstitutionMatch::Converted(&matched),
                        input: &input,
                        position,
                        captures: Some(SubstitutionCaptures::Converted(&captures)),
                        named_captures: named_captures.as_ref(),
                        replacement: replacement
                            .as_ref()
                            .expect("non-functional replacement was not converted"),
                    },
                )?;
                if let Err(value) = status {
                    return Ok(Completion::Throw(value));
                }
                match self.finish_replacement_buffer(realm, substitution)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            };

            // Custom exec results may move backward. QuickJS still performs
            // every getter, coercion, named lookup and callback above before
            // deciding whether this replacement contributes to the output.
            if position >= next_source_position {
                output.append_range(&input, next_source_position, position);
                output.append_js_string(&replacement_text);
                next_source_position = position.saturating_add(matched.len());
            }
        }

        if next_source_position < input.len() {
            output.append_range(&input, next_source_position, input.len());
        }
        match self.finish_replacement_buffer(realm, output)? {
            NativeConversion::Value(value) => Ok(Completion::Return(Value::String(value))),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    fn standard_regexp_replace(
        &self,
        regexp: &ObjectRef,
    ) -> Result<Option<StandardRegExpReplace>, RuntimeError> {
        let last_index = self.intern_property_key("lastIndex")?;
        let exec = self.intern_property_key("exec")?;
        let flags = self.intern_property_key("flags")?;
        let global = self.intern_property_key("global")?;
        let unicode = self.intern_property_key("unicode")?;
        let state = self.0.state.borrow();
        let object = state.heap.object(regexp.object_id())?;
        let ObjectPayload::RegExp(RegExpObjectData::Compiled { program, .. }) = &object.payload
        else {
            return Ok(None);
        };
        // Pinned QuickJS's direct replacement helper cannot publish named
        // captures to GetSubstitution. Returning to the generic path makes
        // builtin exec create `groups` and preserves `$<name>` semantics.
        if program.has_named_captures() {
            return Ok(None);
        }
        let shape = state.heap.shape(object.shape)?;
        let Some(last_index_slot) = shape.find(last_index.atom()) else {
            return Ok(None);
        };
        let last_index_slot = usize::try_from(last_index_slot)
            .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
        let last_index = match object.slots.get(last_index_slot) {
            Some(PropertySlot::Data(RawValue::Int(value))) => Value::Int(*value),
            Some(PropertySlot::Data(RawValue::Float(value))) => Value::Float(*value),
            Some(
                PropertySlot::Data(_)
                | PropertySlot::VarRef(_)
                | PropertySlot::Accessor { .. }
                | PropertySlot::AutoInit(_),
            )
            | None => return Ok(None),
        };

        if !raw_regexp_data_property_matches(
            &state.heap,
            regexp.object_id(),
            exec.atom(),
            NativeFunctionId::RegExp(RegExpNativeKind::Exec),
        )? || !raw_regexp_getter_matches(
            &state.heap,
            regexp.object_id(),
            flags.atom(),
            NativeFunctionId::RegExp(RegExpNativeKind::Flags),
        )? || !raw_regexp_getter_matches(
            &state.heap,
            regexp.object_id(),
            global.atom(),
            NativeFunctionId::RegExp(RegExpNativeKind::Flag(RegExpFlagKind::Global)),
        )? || !raw_regexp_getter_matches(
            &state.heap,
            regexp.object_id(),
            unicode.atom(),
            NativeFunctionId::RegExp(RegExpNativeKind::Flag(RegExpFlagKind::Unicode)),
        )? {
            return Ok(None);
        }

        Ok(Some(StandardRegExpReplace {
            program: program.clone(),
            last_index,
        }))
    }

    #[inline(never)]
    fn call_standard_regexp_replace(
        &self,
        realm: ContextId,
        regexp: &ObjectRef,
        input: &JsString,
        replacement: &JsString,
        standard: StandardRegExpReplace,
    ) -> Result<Completion, RuntimeError> {
        // The outer @@replace buffer was initialized before input conversion.
        // QuickJS's direct helper owns a second buffer and discards the outer
        // one when this path completes.
        let mut output = ReplacementStringBuffer::new(0);
        let program = standard.program;
        let flags = program.flags();
        let global = flags.contains(RegExpFlags::GLOBAL);
        let sticky = flags.contains(RegExpFlags::STICKY);
        let mut last_index = if global {
            if let Some(value) = self.set_regexp_last_index(realm, regexp, 0)? {
                return Ok(Completion::Throw(value));
            }
            0
        } else if sticky {
            match self.native_to_length(realm, &standard.last_index)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            0
        };
        let input_units = input.utf16_units().collect::<Vec<_>>();
        let mut next_source_position = 0_usize;

        loop {
            let matched = if last_index > input_units.len() as u64 {
                None
            } else {
                match execute_with_interrupt(
                    program.as_ref(),
                    &input_units,
                    usize::try_from(last_index).expect("RegExp start was bounded by String length"),
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
                if (global || sticky)
                    && let Some(value) = self.set_regexp_last_index(realm, regexp, 0)?
                {
                    return Ok(Completion::Throw(value));
                }
                break;
            };
            let complete = matched.capture(0).ok_or(RuntimeError::Invariant(
                "successful RegExp execution omitted capture zero",
            ))?;
            if complete.start < next_source_position {
                return Err(RuntimeError::Invariant(
                    "direct RegExp matcher moved backward",
                ));
            }
            if next_source_position < complete.start {
                output.append_range(input, next_source_position, complete.start);
                if output.error().is_some() {
                    return self.complete_regexp_replacement_buffer(realm, output);
                }
            }
            if !replacement.is_empty() {
                let status = self.append_get_substitution(
                    realm,
                    &mut output,
                    SubstitutionInput {
                        matched: SubstitutionMatch::InputRange {
                            start: complete.start,
                            end: complete.end,
                        },
                        input,
                        position: complete.start,
                        captures: Some(SubstitutionCaptures::MatchRanges(matched.captures())),
                        named_captures: None,
                        replacement,
                    },
                )?;
                let status = match status {
                    Ok(status) => status,
                    Err(value) => return Ok(Completion::Throw(value)),
                };
                if matches!(status, SubstitutionStatus::BufferFailed) || output.error().is_some() {
                    return self.complete_regexp_replacement_buffer(realm, output);
                }
            }
            next_source_position = complete.end;
            if !global {
                if sticky {
                    let end = i32::try_from(complete.end).map_err(|_| {
                        RuntimeError::Invariant("RegExp match end exceeded signed String range")
                    })?;
                    if let Some(value) = self.set_regexp_last_index(realm, regexp, end)? {
                        return Ok(Completion::Throw(value));
                    }
                }
                break;
            }
            last_index = u64::try_from(complete.end)
                .map_err(|_| RuntimeError::Invariant("RegExp match end did not fit u64"))?;
            if complete.end == complete.start {
                last_index = advance_string_index(input, last_index, flags.is_unicode());
            }
        }

        if next_source_position < input.len() {
            output.append_range(input, next_source_position, input.len());
        }
        self.complete_regexp_replacement_buffer(realm, output)
    }

    fn complete_regexp_replacement_buffer(
        &self,
        realm: ContextId,
        output: ReplacementStringBuffer,
    ) -> Result<Completion, RuntimeError> {
        match self.finish_replacement_buffer(realm, output)? {
            NativeConversion::Value(value) => Ok(Completion::Return(Value::String(value))),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }
}

fn raw_regexp_data_property_matches(
    heap: &Heap,
    object: ObjectId,
    atom: Atom,
    expected: NativeFunctionId,
) -> Result<bool, RuntimeError> {
    let Some(slot) = raw_regexp_property_slot(heap, object, atom)? else {
        return Ok(false);
    };
    let PropertySlot::Data(RawValue::Object(function)) = slot else {
        return Ok(false);
    };
    raw_native_function_matches(heap, *function, expected)
}

fn raw_regexp_getter_matches(
    heap: &Heap,
    object: ObjectId,
    atom: Atom,
    expected: NativeFunctionId,
) -> Result<bool, RuntimeError> {
    let Some(slot) = raw_regexp_property_slot(heap, object, atom)? else {
        return Ok(false);
    };
    let PropertySlot::Accessor {
        get: Some(function),
        ..
    } = slot
    else {
        return Ok(false);
    };
    raw_native_function_matches(heap, *function, expected)
}

fn raw_regexp_property_slot(
    heap: &Heap,
    object: ObjectId,
    atom: Atom,
) -> Result<Option<&PropertySlot>, RuntimeError> {
    let mut cursor = Some(object);
    let mut receiver = true;
    while let Some(object) = cursor {
        let object = heap.object(object)?;
        if !receiver && regexp_chain_object_is_exotic(object) {
            return Ok(None);
        }
        let shape = heap.shape(object.shape)?;
        if let Some(index) = shape.find(atom) {
            let index = usize::try_from(index)
                .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
            return object
                .slots
                .get(index)
                .map(Some)
                .ok_or(RuntimeError::Invariant(
                    "shape property had no parallel object slot",
                ));
        }
        cursor = shape.prototype();
        receiver = false;
    }
    Ok(None)
}

fn regexp_chain_object_is_exotic(object: &ObjectData) -> bool {
    matches!(
        &object.payload,
        ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::Primitive(PrimitiveObjectData::String(_))
    )
}

fn raw_native_function_matches(
    heap: &Heap,
    object: ObjectId,
    expected: NativeFunctionId,
) -> Result<bool, RuntimeError> {
    Ok(matches!(
        &heap.object(object)?.payload,
        ObjectPayload::NativeFunction { data, .. } if data.target == expected
    ))
}
