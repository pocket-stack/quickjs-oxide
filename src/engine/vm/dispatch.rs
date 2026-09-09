use super::*;

impl VmActivation {
    // Keep cold host bridges behind one call site instead of adding large
    // Result temporaries to `execute_inner`'s interpreter
    // frame. This preserves the proven-safe recursive native/callback depth on
    // the 2 MiB libtest stack.
    #[inline(never)]
    pub(in crate::engine::vm) fn execute_cold_instruction(
        &mut self,
        instruction: &Instruction,
        host: &mut impl VmHost,
    ) -> Result<Option<Completion>, Error> {
        match instruction {
            Instruction::Arguments(kind) => match host.create_arguments(*kind)? {
                Completion::Return(arguments) => self.stack.push(arguments),
                Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
            },
            Instruction::Rest(start) => match host.create_rest(*start)? {
                Completion::Return(rest) => self.stack.push(rest),
                Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
            },
            Instruction::VariableEnvironment => match host.create_variable_environment()? {
                Completion::Return(environment) => self.stack.push(environment),
                Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
            },
            Instruction::HasEvalVariable { source, name } => {
                match host.has_eval_variable(*source, *name)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::GetEvalVariable { source, name } => {
                match host.get_eval_variable(*source, *name)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::PutEvalVariable { source, name } => {
                let value = self.pop()?;
                if let Completion::Throw(value) = host.put_eval_variable(*source, *name, value)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::DeleteEvalVariable { source, name } => {
                match host.delete_eval_variable(*source, *name)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::DefineEvalVariable { source, name } => {
                let value = self.pop()?;
                if let Completion::Throw(value) =
                    host.define_eval_variable(*source, *name, value)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::ToObject => {
                let value = self.pop()?;
                match value {
                    value @ Value::Object(_) => self.stack.push(value),
                    Value::Null | Value::Undefined => {
                        return Err(Error::new(ErrorKind::Type, "cannot convert to object"));
                    }
                    primitive => self.stack.push(host.box_primitive(primitive)?),
                }
            }
            Instruction::HasDynamicBinding { source, name } => {
                match host.has_dynamic_binding(*source, *name)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::GetDynamicBinding { source, name } => {
                match host.get_dynamic_binding(*source, *name, self.strict)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::PutDynamicBinding { source, name } => {
                let value = self.pop()?;
                if let Completion::Throw(value) =
                    host.put_dynamic_binding(*source, *name, value, self.strict)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::DeleteDynamicBinding { source, name } => {
                match host.delete_dynamic_binding(*source, *name)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::DynamicEnvironmentObject(source) => {
                match host.dynamic_environment_object(*source)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::GlobalReference(index) => match host.global_reference(*index)? {
                Completion::Return(value) => self.stack.push(value),
                Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
            },
            Instruction::GetRefValue(name) | Instruction::GetRefValueUndef(name) => {
                let environment = self.pop()?;
                let retained_environment = environment.clone();
                let strict = matches!(instruction, Instruction::GetRefValue(_)) && self.strict;
                match host.get_ref_value(environment, *name, strict)? {
                    Completion::Return(value) => {
                        self.stack.push(retained_environment);
                        self.stack.push(value);
                    }
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::PutRefValue(name) => {
                let (environment, value) = self.pop_pair()?;
                if let Completion::Throw(value) =
                    host.put_ref_value(environment, *name, value, self.strict)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Object => match host.object()? {
                Completion::Return(object) => self.stack.push(object),
                Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
            },
            Instruction::RegExp(index) => match host.create_regexp(*index)? {
                Completion::Return(object) => self.stack.push(object),
                Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
            },
            Instruction::SetNameComputed => {
                let key_index = self
                    .stack
                    .len()
                    .checked_sub(2)
                    .ok_or_else(|| Error::internal("set computed name without a key"))?;
                let value = self
                    .stack
                    .last()
                    .ok_or_else(|| Error::internal("set computed name on an empty stack"))?;
                let key = self
                    .stack
                    .get(key_index)
                    .ok_or_else(|| Error::internal("set computed name without a key"))?;
                host.set_function_name_computed(value, key)?;
            }
            Instruction::DefineMethod {
                key,
                kind,
                enumerable,
            } => {
                let (base, function) = self.pop_pair()?;
                let retained_base = base.clone();
                match host.define_method(base, *key, function, *kind, *enumerable)? {
                    Completion::Return(_) => self.stack.push(retained_base),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::DefineMethodComputed { kind, enumerable } => {
                let function = self.pop()?;
                let key = self.pop()?;
                let base = self.pop()?;
                let retained_base = base.clone();
                match host.define_method_computed(base, key, function, *kind, *enumerable)? {
                    Completion::Return(_) => self.stack.push(retained_base),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::DefineClass { name, has_heritage } => {
                let (parent, constructor) = self.pop_pair()?;
                match host.define_class(parent, constructor, *name, *has_heritage)? {
                    DefineClassOutcome::Defined {
                        constructor,
                        prototype,
                    } => {
                        self.stack.push(constructor);
                        self.stack.push(prototype);
                    }
                    DefineClassOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::SetProto => {
                let (object, prototype) = self.pop_pair()?;
                let retained_object = object.clone();
                match host.set_object_prototype(object, prototype)? {
                    Completion::Return(_) => self.stack.push(retained_object),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::CopyDataProperties => {
                let (target, source) = self.pop_pair()?;
                let retained_target = target.clone();
                match host.copy_data_properties(target, source)? {
                    Completion::Return(_) => self.stack.push(retained_target),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::CopyDataPropertiesExcluded {
                target_depth,
                source_depth,
                excluded_depth,
            } => {
                let target = self.clone_at_depth(*target_depth)?;
                let source = self.clone_at_depth(*source_depth)?;
                let excluded = self.clone_at_depth(*excluded_depth)?;
                if let Completion::Throw(value) =
                    host.copy_data_properties_excluded(target, source, excluded)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::IteratorStart => {
                let iterable = self.pop()?;
                match host.for_of_start(iterable)? {
                    ForOfStartOutcome::Record {
                        iterator,
                        next_method,
                    } => {
                        // Unlike ForOfStart this yield*-private record is made
                        // entirely of ordinary operands. In particular, it
                        // must not create an unwind region which would call
                        // the delegate's `return` method on a propagated
                        // next/return/throw completion.
                        self.stack.push(iterator);
                        self.stack.push(next_method);
                        self.stack.push(Value::Undefined);
                    }
                    ForOfStartOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::AsyncIteratorStart => {
                let iterable = self.pop()?;
                match host.for_await_of_start(iterable)? {
                    ForOfStartOutcome::Record {
                        iterator,
                        next_method,
                    } => {
                        // The async delegation record has the same explicit
                        // compiler-private shape as synchronous yield*. Its
                        // iterator is either the authored async iterator or a
                        // branded Async-from-Sync wrapper.
                        self.stack.push(iterator);
                        self.stack.push(next_method);
                        self.stack.push(Value::Undefined);
                    }
                    ForOfStartOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::ForAwaitOfStart => {
                let iterable = self.pop()?;
                match host.for_await_of_start(iterable)? {
                    ForOfStartOutcome::Record {
                        iterator,
                        next_method,
                    } => {
                        let record_base = self.stack.len();
                        self.stack.push(iterator);
                        self.stack.push(next_method);
                        self.regions.push(VmUnwindRegion::Iterator {
                            record_base,
                            enabled: true,
                            asynchronous: true,
                        });
                    }
                    ForOfStartOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::ForAwaitOfNext => {
                let (record_base, enabled, asynchronous) = match self.regions.last() {
                    Some(VmUnwindRegion::Iterator {
                        record_base,
                        enabled,
                        asynchronous,
                    }) => (*record_base, *enabled, *asynchronous),
                    _ => {
                        return Err(Error::internal(
                            "ForAwaitOfNext has no innermost iterator region",
                        ));
                    }
                };
                if !asynchronous {
                    return Err(Error::internal(
                        "ForAwaitOfNext targeted a synchronous iterator region",
                    ));
                }
                let record_end = record_base
                    .checked_add(2)
                    .ok_or_else(|| Error::internal("iterator record depth overflow"))?;
                if self.stack.len() != record_end {
                    return Err(Error::internal(
                        "ForAwaitOfNext did not reach its iterator record",
                    ));
                }
                if !enabled {
                    return Err(Error::internal(
                        "ForAwaitOfNext targeted a disabled iterator region",
                    ));
                }
                let iterator = self
                    .stack
                    .get(record_base)
                    .cloned()
                    .ok_or_else(|| Error::internal("iterator record is truncated"))?;
                let next_method = self
                    .stack
                    .get(record_base + 1)
                    .cloned()
                    .ok_or_else(|| Error::internal("iterator record is truncated"))?;
                self.set_iterator_region_enabled(record_base, false, true)?;
                match host.call(next_method, iterator, Vec::new())? {
                    Completion::Return(result) => self.stack.push(result),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::IteratorGetValueDone => {
                let result = self.pop()?;
                let (record_base, enabled, asynchronous) = match self.regions.last() {
                    Some(VmUnwindRegion::Iterator {
                        record_base,
                        enabled,
                        asynchronous,
                    }) => (*record_base, *enabled, *asynchronous),
                    _ => {
                        return Err(Error::internal(
                            "IteratorGetValueDone has no innermost iterator region",
                        ));
                    }
                };
                if !asynchronous || enabled {
                    return Err(Error::internal(
                        "IteratorGetValueDone targeted a non-pending async iterator region",
                    ));
                }
                let record_end = record_base
                    .checked_add(2)
                    .ok_or_else(|| Error::internal("iterator record depth overflow"))?;
                if self.stack.len() != record_end {
                    return Err(Error::internal(
                        "IteratorGetValueDone did not reach its iterator record",
                    ));
                }
                match host.iterator_get_value_done(result)? {
                    ForOfNextOutcome::Result { value, done } => {
                        self.set_iterator_region_enabled(record_base, true, true)?;
                        self.stack.push(value);
                        self.stack.push(Value::Bool(done));
                    }
                    ForOfNextOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::IteratorNext => {
                // iter, cached-next, placeholder, argument ->
                // iter, cached-next, placeholder, result-object
                let iterator = self.clone_at_depth(3)?;
                let next_method = self.clone_at_depth(2)?;
                let argument = self.pop()?;
                match host.call(next_method, iterator, vec![argument])? {
                    Completion::Return(result) => self.stack.push(result),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::IteratorCall(kind) => {
                // QuickJS looks up the method before touching the retained
                // input value. A missing/nullish method leaves that value in
                // place and appends true; a present method replaces it with
                // the call result and appends false.
                let iterator = self.clone_at_depth(3)?;
                let method_name = match kind {
                    IteratorCallKind::ThrowWithValue => "throw",
                    IteratorCallKind::ReturnWithValue | IteratorCallKind::ReturnWithoutValue => {
                        "return"
                    }
                };
                let method = match host.get_property(
                    iterator.clone(),
                    Value::String(JsString::from_static(method_name)),
                )? {
                    Completion::Return(method) => method,
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                };
                if matches!(method, Value::Undefined | Value::Null) {
                    self.stack.push(Value::Bool(true));
                } else {
                    let arguments = match kind {
                        IteratorCallKind::ReturnWithoutValue => Vec::new(),
                        IteratorCallKind::ReturnWithValue | IteratorCallKind::ThrowWithValue => {
                            vec![self.stack.last().cloned().ok_or_else(|| {
                                Error::internal("iterator call has no input value")
                            })?]
                        }
                    };
                    match host.call(method, iterator, arguments)? {
                        Completion::Return(result) => {
                            let input = self.stack.last_mut().ok_or_else(|| {
                                Error::internal("iterator call lost its input value")
                            })?;
                            *input = result;
                            self.stack.push(Value::Bool(false));
                        }
                        Completion::Throw(value) => {
                            return Ok(Some(Completion::Throw(value)));
                        }
                    }
                }
            }
            Instruction::ForInStart => {
                let value = self.pop()?;
                match host.for_in_start(value)? {
                    ForInStartOutcome::Iterator(iterator) => self.stack.push(iterator),
                    ForInStartOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::ForInNext => {
                let iterator = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| Error::internal("for-in next without an iterator"))?;
                match host.for_in_next(iterator)? {
                    ForInNextOutcome::Result { value, done } => {
                        self.stack.push(value);
                        self.stack.push(Value::Bool(done));
                    }
                    ForInNextOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            _ => {
                return Err(Error::internal("hot instruction reached cold VM dispatch"));
            }
        }
        Ok(None)
    }

    // Calls recursively enter bytecode/native execution. Keep their argument
    // vectors and host-completion temporaries out of the interpreter loop's
    // frame so each nested JavaScript call retains only the hot dispatch
    // state on the native stack.
    #[inline(never)]
    pub(in crate::engine::vm) fn execute_call_instruction(
        &mut self,
        instruction: &Instruction,
        host: &mut impl VmHost,
    ) -> Result<Option<Completion>, Error> {
        let completion = match instruction {
            Instruction::Import => {
                // QuickJS passes `sp[-2]` and `sp[-1]` to
                // `js_dynamic_import` in authored evaluation order. Pop the
                // top options value first, then restore that raw order at the
                // host boundary.
                let options = self.pop()?;
                let specifier = self.pop()?;
                host.dynamic_import(specifier, options)?
            }
            Instruction::Call(argument_count) => {
                let arguments = self.take_call_arguments(*argument_count, 1)?;
                let function = self.pop()?;
                host.call(function, Value::Undefined, arguments)?
            }
            Instruction::TailCall(argument_count) => {
                let arguments = self.take_call_arguments(*argument_count, 1)?;
                let function = self.pop()?;
                return host.call(function, Value::Undefined, arguments).map(Some);
            }
            Instruction::Eval {
                argument_count,
                environment,
            } => {
                let arguments = self.take_call_arguments(*argument_count, 1)?;
                let function = self.pop()?;
                self.execute_eval_call(function, arguments, *environment, host)?
            }
            Instruction::CallMethod(argument_count) => {
                let arguments = self.take_call_arguments(*argument_count, 2)?;
                let function = self.pop()?;
                let receiver = self.pop()?;
                host.call(function, receiver, arguments)?
            }
            Instruction::TailCallMethod(argument_count) => {
                let arguments = self.take_call_arguments(*argument_count, 2)?;
                let function = self.pop()?;
                let receiver = self.pop()?;
                return host.call(function, receiver, arguments).map(Some);
            }
            Instruction::Construct(argument_count)
            | Instruction::ConstructSuper(argument_count) => {
                let arguments = self.take_call_arguments(*argument_count, 2)?;
                let new_target = self.pop()?;
                let function = self.pop()?;
                host.construct(function, new_target, arguments)?
            }
            Instruction::InitDerivedConstructor => {
                let active_function = self.current_function.clone().ok_or_else(|| {
                    Error::internal(
                        "derived constructor initialization has no active function object",
                    )
                })?;
                host.init_derived_constructor(active_function, self.new_target.clone())?
            }
            Instruction::Apply(kind) => {
                let argument_array = self.pop()?;
                let this_or_new_target = self.pop()?;
                let function = self.pop()?;
                host.apply(function, this_or_new_target, argument_array, *kind)?
            }
            Instruction::ApplySuper => {
                let argument_array = self.pop()?;
                let new_target = self.pop()?;
                let function = self.pop()?;
                host.apply(function, new_target, argument_array, ApplyKind::Construct)?
            }
            Instruction::ApplyEval { environment } => {
                let argument_array = self.pop()?;
                let function = self.pop()?;
                let arguments = match host.build_argument_list(argument_array)? {
                    ArgumentListOutcome::Values(arguments) => arguments,
                    ArgumentListOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                };
                self.execute_eval_call(function, arguments, *environment, host)?
            }
            _ => {
                return Err(Error::internal(
                    "non-call instruction reached VM call dispatch",
                ));
            }
        };
        match completion {
            Completion::Return(value) => {
                self.stack.push(value);
                Ok(None)
            }
            Completion::Throw(value) => Ok(Some(Completion::Throw(value))),
        }
    }

    pub(in crate::engine::vm) fn execute_eval_call(
        &mut self,
        function: Value,
        arguments: Vec<Value>,
        environment: u16,
        host: &mut impl VmHost,
    ) -> Result<Completion, Error> {
        if host.is_original_eval(&function)? {
            let input = arguments.first().cloned().unwrap_or(Value::Undefined);
            // QuickJS only enters the compiler for primitive String input.
            // Preserve that lazy boundary for sloppy `this`: non-String eval
            // returns its input without allocating a primitive wrapper or
            // touching caller bindings. Retain the complete arguments Vec
            // until direct eval returns so all spread values remain rooted.
            let this_value = if matches!(input, Value::String(_)) {
                self.normalized_this(host)?
            } else {
                self.this_value.clone()
            };
            let completion = host.direct_eval(DirectEvalInvocation {
                input,
                environment,
                this_value,
                new_target: self.new_target.clone(),
                caller_strict: self.strict,
            })?;
            drop(arguments);
            Ok(completion)
        } else {
            host.call(function, Value::Undefined, arguments)
        }
    }

    // Numeric coercion and BigInt paths instantiate several large generic
    // result temporaries in debug builds. Isolate the whole family so ordinary
    // bytecode calls do not reserve those slots in every recursive VM frame.
    #[inline(never)]
    pub(in crate::engine::vm) fn execute_numeric_instruction(
        &mut self,
        instruction: &Instruction,
        host: &mut impl VmHost,
    ) -> Result<Option<Completion>, Error> {
        match instruction {
            Instruction::Neg => {
                if let OperationOutcome::Throw(value) = self.neg(host)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Plus => {
                if let OperationOutcome::Throw(value) = self.unary_plus(host)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Inc | Instruction::Dec => {
                let increment = matches!(instruction, Instruction::Inc);
                if let OperationOutcome::Throw(value) =
                    self.update_numeric(host, increment, false)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::PostInc | Instruction::PostDec => {
                let increment = matches!(instruction, Instruction::PostInc);
                if let OperationOutcome::Throw(value) =
                    self.update_numeric(host, increment, true)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::BitNot => {
                if let OperationOutcome::Throw(value) = self.bit_not(host)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Not => {
                let value = self.pop()?;
                self.stack.push(Value::Bool(!host.to_boolean(&value)?));
            }
            Instruction::TypeOf => {
                let value = self.pop()?;
                self.stack.push(Value::String(host.type_of(&value)?));
            }
            Instruction::IsUndefinedOrNull => {
                let value = self.pop()?;
                self.stack
                    .push(Value::Bool(matches!(value, Value::Undefined | Value::Null)));
            }
            Instruction::IsUndefined => {
                let value = self.pop()?;
                self.stack
                    .push(Value::Bool(matches!(value, Value::Undefined)));
            }
            Instruction::IsNull => {
                let value = self.pop()?;
                self.stack.push(Value::Bool(matches!(value, Value::Null)));
            }
            Instruction::TypeOfIsUndefined => {
                let value = self.pop()?;
                let is_undefined = matches!(value, Value::Undefined) || host.is_html_dda(&value)?;
                self.stack.push(Value::Bool(is_undefined));
            }
            Instruction::TypeOfIsFunction => {
                let value = self.pop()?;
                let is_function = !host.is_html_dda(&value)? && host.is_callable(&value)?;
                self.stack.push(Value::Bool(is_function));
            }
            Instruction::Add => {
                if let OperationOutcome::Throw(value) = self.add(host)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Sub => {
                if let OperationOutcome::Throw(value) =
                    self.binary_numeric(host, |left, right| left - right, JsBigInt::sub)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Mul => {
                if let OperationOutcome::Throw(value) =
                    self.binary_numeric(host, |left, right| left * right, JsBigInt::mul)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Div => {
                if let OperationOutcome::Throw(value) =
                    self.binary_numeric(host, |left, right| left / right, JsBigInt::div)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Mod => {
                if let OperationOutcome::Throw(value) =
                    self.binary_numeric(host, |left, right| left % right, JsBigInt::rem)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Pow => {
                if let OperationOutcome::Throw(value) =
                    self.binary_numeric(host, crate::engine::value::number::pow, JsBigInt::pow)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Shl => {
                if let OperationOutcome::Throw(value) = self.binary_numeric(
                    host,
                    |left, right| {
                        f64::from(
                            number_to_int32(left).wrapping_shl(number_to_uint32(right) & 0x1f),
                        )
                    },
                    JsBigInt::shl,
                )? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Sar => {
                if let OperationOutcome::Throw(value) = self.binary_numeric(
                    host,
                    |left, right| {
                        f64::from(number_to_int32(left) >> (number_to_uint32(right) & 0x1f))
                    },
                    JsBigInt::shr,
                )? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Shr => {
                if let OperationOutcome::Throw(value) = self.unsigned_shift_right(host)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::BitAnd => {
                if let OperationOutcome::Throw(value) = self.binary_numeric(
                    host,
                    |left, right| f64::from(number_to_int32(left) & number_to_int32(right)),
                    JsBigInt::bit_and,
                )? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::BitXor => {
                if let OperationOutcome::Throw(value) = self.binary_numeric(
                    host,
                    |left, right| f64::from(number_to_int32(left) ^ number_to_int32(right)),
                    JsBigInt::bit_xor,
                )? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::BitOr => {
                if let OperationOutcome::Throw(value) = self.binary_numeric(
                    host,
                    |left, right| f64::from(number_to_int32(left) | number_to_int32(right)),
                    JsBigInt::bit_or,
                )? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Eq => {
                let (left, right) = self.pop_pair()?;
                match abstract_equal(host, left, right)? {
                    OperationOutcome::Value(equal) => self.stack.push(Value::Bool(equal)),
                    OperationOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::StrictEq => {
                let (left, right) = self.pop_pair()?;
                self.stack.push(Value::Bool(left.strict_equal(&right)));
            }
            Instruction::Neq => {
                let (left, right) = self.pop_pair()?;
                match abstract_equal(host, left, right)? {
                    OperationOutcome::Value(equal) => self.stack.push(Value::Bool(!equal)),
                    OperationOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::StrictNeq => {
                let (left, right) = self.pop_pair()?;
                self.stack.push(Value::Bool(!left.strict_equal(&right)));
            }
            Instruction::Lt => {
                if let OperationOutcome::Throw(value) =
                    self.compare(host, std::cmp::Ordering::is_lt)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Lte => {
                if let OperationOutcome::Throw(value) =
                    self.compare(host, std::cmp::Ordering::is_le)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Gt => {
                if let OperationOutcome::Throw(value) =
                    self.compare(host, std::cmp::Ordering::is_gt)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Gte => {
                if let OperationOutcome::Throw(value) =
                    self.compare(host, std::cmp::Ordering::is_ge)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            _ => {
                return Err(Error::internal(
                    "non-numeric instruction reached VM numeric dispatch",
                ));
            }
        }
        Ok(None)
    }

    // Keep the large non-call instruction match out of `execute_inner`'s
    // frame. A suspended JavaScript call retains `execute_inner` while its
    // callee runs, so allowing this debug-build dispatch frame to remain live
    // multiplied roughly 71 KiB by every ordinary bytecode call.
    #[inline(never)]
    pub(in crate::engine::vm) fn execute_hot_instruction(
        &mut self,
        code: &[Instruction],
        instruction: &Instruction,
        host: &mut impl VmHost,
    ) -> Result<Option<Completion>, Error> {
        match instruction {
            Instruction::Nop => {}
            Instruction::InitialYield
            | Instruction::Yield
            | Instruction::YieldStar
            | Instruction::AsyncYieldStar
            | Instruction::Await => {
                unreachable!("VM suspension dispatch was bypassed")
            }
            Instruction::IteratorStart
            | Instruction::AsyncIteratorStart
            | Instruction::IteratorNext
            | Instruction::IteratorCall(_)
            | Instruction::ForAwaitOfStart
            | Instruction::ForAwaitOfNext
            | Instruction::IteratorGetValueDone => {
                unreachable!("yield-star iterator dispatch was bypassed")
            }
            Instruction::PushI32(value) => self.stack.push(Value::Int(*value)),
            Instruction::PushAtomValueIndex(value) => self.stack.push(Value::String(
                crate::engine::value::JsString::from_fresh_decimal_u32(*value),
            )),
            Instruction::PushConst(index) => {
                self.stack.push(host.load_constant(*index)?);
            }
            Instruction::FClosure(index) => {
                self.stack.push(host.instantiate_closure(*index)?);
            }
            Instruction::ArrayFrom(element_count) => {
                let element_count = usize::from(*element_count);
                let first = self
                    .stack
                    .len()
                    .checked_sub(element_count)
                    .ok_or_else(|| Error::internal("array_from stack underflow"))?;
                let elements = self.stack.drain(first..).collect();
                match host.array_from(elements)? {
                    Completion::Return(array) => self.stack.push(array),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::Arguments(_) => {
                unreachable!("arguments-object dispatch was bypassed")
            }
            Instruction::Rest(_) => unreachable!("rest-parameter dispatch was bypassed"),
            Instruction::VariableEnvironment
            | Instruction::HasEvalVariable { .. }
            | Instruction::GetEvalVariable { .. }
            | Instruction::PutEvalVariable { .. }
            | Instruction::DeleteEvalVariable { .. }
            | Instruction::DefineEvalVariable { .. }
            | Instruction::ToObject
            | Instruction::HasDynamicBinding { .. }
            | Instruction::GetDynamicBinding { .. }
            | Instruction::PutDynamicBinding { .. }
            | Instruction::DeleteDynamicBinding { .. }
            | Instruction::DynamicEnvironmentObject(_)
            | Instruction::GlobalReference(_)
            | Instruction::GetRefValue(_)
            | Instruction::GetRefValueUndef(_)
            | Instruction::PutRefValue(_) => {
                unreachable!("eval variable-object dispatch was bypassed")
            }
            Instruction::Object => unreachable!("object literal dispatch was bypassed"),
            Instruction::RegExp(_) => {
                unreachable!("RegExp literal dispatch was bypassed")
            }
            Instruction::SetName(index) => {
                let value = self
                    .stack
                    .last()
                    .ok_or_else(|| Error::internal("set name on an empty stack"))?;
                host.set_function_name(value, *index)?;
            }
            Instruction::SetNameComputed => {
                unreachable!("computed-name literal dispatch was bypassed")
            }
            Instruction::DefineMethod { .. }
            | Instruction::DefineMethodComputed { .. }
            | Instruction::DefineClass { .. } => {
                unreachable!("object method dispatch was bypassed")
            }
            Instruction::InstallClassInstanceInitializer => {
                let initializer = self.pop()?;
                let prototype = self.pop()?;
                let constructor = self.pop()?;
                let retained_constructor = constructor.clone();
                let retained_prototype = prototype.clone();
                match host.install_class_instance_initializer(
                    constructor,
                    prototype,
                    initializer,
                )? {
                    Completion::Return(_) => {
                        self.stack.push(retained_constructor);
                        self.stack.push(retained_prototype);
                    }
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::CallClassInstanceInitializer => {
                let active_constructor = self.pop()?;
                let receiver = self.pop()?;
                let retained_receiver = receiver.clone();
                match host.call_class_instance_initializer(active_constructor, receiver)? {
                    Completion::Return(_) => self.stack.push(retained_receiver),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::RunClassStaticInitializer => {
                let initializer = self.pop()?;
                let constructor = self.pop()?;
                let retained_constructor = constructor.clone();
                match host.run_class_static_initializer(constructor, initializer)? {
                    Completion::Return(_) => self.stack.push(retained_constructor),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::CallClassStaticBlock => {
                let block = self.pop()?;
                let static_initializer = self.current_function.clone().ok_or_else(|| {
                    Error::internal("static block call has no active initializer")
                })?;
                if let Completion::Throw(value) = host.call_class_static_block(
                    static_initializer,
                    self.this_value.clone(),
                    block,
                )? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::CheckCtor => {
                if matches!(self.new_target, Value::Undefined) {
                    return Err(Error::new(
                        ErrorKind::Type,
                        "class constructors must be invoked with 'new'",
                    ));
                }
            }
            Instruction::IteratorCheckObject => {
                if !matches!(self.stack.last(), Some(Value::Object(_))) {
                    return Err(Error::new(
                        ErrorKind::Type,
                        "iterator must return an object",
                    ));
                }
            }
            Instruction::ThrowIteratorMissingThrow => {
                return Err(Error::new(
                    ErrorKind::Type,
                    "iterator does not have a throw method",
                ));
            }
            Instruction::MarkSuperCall => {}
            Instruction::ThrowReadOnly(index) => {
                return Err(host.read_only_error(*index)?);
            }
            Instruction::ThrowRedeclaration(index) => {
                return Err(host.redeclaration_error(*index)?);
            }
            Instruction::ThrowDeleteSuper => {
                self.pop()?;
                self.pop()?;
                self.pop()?;
                return Err(Error::new(
                    ErrorKind::Reference,
                    "unsupported reference to 'super'",
                ));
            }
            Instruction::Undefined => self.stack.push(Value::Undefined),
            Instruction::Null => self.stack.push(Value::Null),
            Instruction::PushFalse => self.stack.push(Value::Bool(false)),
            Instruction::PushTrue => self.stack.push(Value::Bool(true)),
            Instruction::PushThis => {
                let value = self.normalized_this(host)?;
                self.stack.push(value);
            }
            Instruction::PushActiveFunction => {
                let function = self.current_function.clone().ok_or_else(|| {
                    Error::internal("active function opcode has no active function object")
                })?;
                self.stack.push(Value::Object(function));
            }
            Instruction::PushHomeObject => {
                self.stack.push(host.home_object()?);
            }
            Instruction::PushNewTarget => self.stack.push(self.new_target.clone()),
            Instruction::GetLocal(index) => {
                self.stack.push(host.get_local(*index)?);
            }
            Instruction::PutLocal(index) => {
                let value = self.pop()?;
                host.put_local(*index, value)?;
            }
            Instruction::SetLocal(index) => {
                let value = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| Error::internal("set local on an empty stack"))?;
                host.put_local(*index, value)?;
            }
            Instruction::SetLocalUninitialized(index) => {
                host.set_local_uninitialized(*index)?;
            }
            Instruction::GetLocalCheck(index) => {
                self.stack.push(host.get_local_checked(*index)?);
            }
            Instruction::InitializeLocal(index) => {
                let value = self.pop()?;
                host.initialize_local(*index, value)?;
            }
            Instruction::InitializeDerivedLocal(index) => {
                let value = self.pop()?;
                host.initialize_derived_local(*index, value)?;
            }
            Instruction::PutLocalCheck(index) => {
                let value = self.pop()?;
                host.put_local_checked(*index, value)?;
            }
            Instruction::SetLocalCheck(index) => {
                let value = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| Error::internal("set lexical local on an empty stack"))?;
                host.put_local_checked(*index, value)?;
            }
            Instruction::GetArg(index) => {
                self.stack.push(host.get_argument(*index)?);
            }
            Instruction::PutArg(index) => {
                let value = self.pop()?;
                host.put_argument(*index, value)?;
            }
            Instruction::SetArg(index) => {
                let value = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| Error::internal("set argument on an empty stack"))?;
                host.put_argument(*index, value)?;
            }
            Instruction::GetVarRef(index) => {
                self.stack.push(host.get_var_ref(*index)?);
            }
            Instruction::PutVarRef(index) => {
                let value = self.pop()?;
                host.put_var_ref(*index, value)?;
            }
            Instruction::SetVarRef(index) => {
                let value = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| Error::internal("set VarRef on an empty stack"))?;
                host.put_var_ref(*index, value)?;
            }
            Instruction::GetVarRefCheck(index) => {
                self.stack.push(host.get_var_ref_checked(*index)?);
            }
            Instruction::PutVarRefCheck(index) => {
                let value = self.pop()?;
                host.put_var_ref_checked(*index, value)?;
            }
            Instruction::InitializeVarRef(index) => {
                let value = self.pop()?;
                host.initialize_var_ref(*index, value)?;
            }
            Instruction::InitializeModuleImportCollision(index) => {
                let value = self.pop()?;
                host.initialize_module_import_collision(*index, value)?;
            }
            Instruction::InitializeDerivedVarRef(index) => {
                let value = self.pop()?;
                host.initialize_derived_var_ref(*index, value)?;
            }
            Instruction::CloseLocal(index) => {
                host.close_local(*index)?;
            }
            Instruction::GetVar(index) | Instruction::GetVarUndef(index) => {
                let throw_if_missing = matches!(instruction, Instruction::GetVar(_));
                match host.get_global_var(*index, throw_if_missing)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::DeleteVar(index) => match host.delete_global_var(*index)? {
                Completion::Return(value) => self.stack.push(value),
                Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
            },
            Instruction::PutVar(index) | Instruction::PutVarInit(index) => {
                let value = self.pop()?;
                let initialize = matches!(instruction, Instruction::PutVarInit(_));
                if let Completion::Throw(value) =
                    host.put_global_var(*index, value, initialize, self.strict)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::InitializePrivateName(index) => {
                host.initialize_private_name(*index)?;
            }
            Instruction::InitializePrivateMethod(index) => {
                let (home_object, method) = self.pop_pair()?;
                host.initialize_private_method(*index, home_object.clone(), method)?;
                self.stack.push(home_object);
            }
            Instruction::InitializePrivateAccessor(index) => {
                let (home_object, accessor) = self.pop_pair()?;
                host.initialize_private_accessor(*index, home_object.clone(), accessor)?;
                self.stack.push(home_object);
            }
            Instruction::GetPrivateField(source) | Instruction::GetPrivateField2(source) => {
                let keep_receiver = matches!(instruction, Instruction::GetPrivateField2(_));
                let base = self.pop()?;
                let receiver = keep_receiver.then(|| base.clone());
                match host.get_private_field(*source, base)? {
                    Completion::Return(value) => {
                        if let Some(receiver) = receiver {
                            self.stack.push(receiver);
                        }
                        self.stack.push(value);
                    }
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::PutPrivateField(source) => {
                let (base, value) = self.pop_pair()?;
                if let Completion::Throw(value) = host.put_private_field(*source, base, value)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::DefinePrivateField(source) => {
                let (base, value) = self.pop_pair()?;
                let retained_base = base.clone();
                match host.define_private_field(*source, base, value)? {
                    Completion::Return(_) => self.stack.push(retained_base),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::PrivateIn(source) => {
                let base = self.pop()?;
                match host.private_in(*source, base)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::GetField(index) | Instruction::GetField2(index) => {
                let keep_receiver = matches!(instruction, Instruction::GetField2(_));
                let base = self.pop()?;
                let receiver = keep_receiver.then(|| base.clone());
                match host.get_field(base, *index)? {
                    Completion::Return(value) => {
                        if let Some(receiver) = receiver {
                            self.stack.push(receiver);
                        }
                        self.stack.push(value);
                    }
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::GetArrayEl | Instruction::GetArrayEl2 => {
                let keep_receiver = matches!(instruction, Instruction::GetArrayEl2);
                let (base, key) = self.pop_pair()?;
                let receiver = keep_receiver.then(|| base.clone());
                match host.get_property(base, key)? {
                    Completion::Return(value) => {
                        if let Some(receiver) = receiver {
                            self.stack.push(receiver);
                        }
                        self.stack.push(value);
                    }
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::GetArrayEl3 => {
                let (base, key) = self.pop_pair()?;
                let key_is_already_canonical =
                    matches!(key, Value::Int(_) | Value::String(_) | Value::Symbol(_));
                if matches!(base, Value::Null | Value::Undefined) && !key_is_already_canonical {
                    // QuickJS `get_array_el3` performs this special check
                    // before ToPropertyKey for non-fast key tags.
                    return Err(Error::new(ErrorKind::Type, "value has no property"));
                }
                if matches!(base, Value::Null | Value::Undefined) {
                    // Reuse the ordinary read path solely for its exact
                    // fast-key nullish diagnostic.
                    match host.get_property(base, key)? {
                        Completion::Return(_) => {
                            return Err(Error::internal(
                                "nullish property read unexpectedly completed",
                            ));
                        }
                        Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                    }
                }
                let key = match host.convert_property_key(key)? {
                    Completion::Return(key) => key,
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                };
                let value = match host.get_property(base.clone(), key.clone())? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                };
                self.stack.push(base);
                self.stack.push(key);
                self.stack.push(value);
            }
            Instruction::GetSuper => {
                let home_object = self.pop()?;
                self.stack.push(host.get_super(home_object)?);
            }
            Instruction::GetSuperValue | Instruction::GetSuperValueForCall => {
                let key = self.pop()?;
                let base = self.pop()?;
                let receiver = self.pop()?;
                let keep_receiver = matches!(instruction, Instruction::GetSuperValueForCall);
                let outcome = if keep_receiver {
                    // Pinned QuickJS rewrites get_super_value to the
                    // ordinary get_array_el opcode at a call site. The
                    // getter therefore observes `base`, while the
                    // preserved receiver is still used by CallMethod.
                    host.get_property(base, key)?
                } else {
                    host.get_super_property(receiver.clone(), base, key)?
                };
                match outcome {
                    Completion::Return(value) => {
                        if keep_receiver {
                            self.stack.push(receiver);
                        }
                        self.stack.push(value);
                    }
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::ToPropKey => {
                let key = self.pop()?;
                match host.convert_property_key(key)? {
                    Completion::Return(key) => self.stack.push(key),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::Insert2 => {
                let (base, value) = self.pop_pair()?;
                self.stack.push(value.clone());
                self.stack.push(base);
                self.stack.push(value);
            }
            Instruction::Insert3 => {
                let value = self.pop()?;
                let key = self.pop()?;
                let base = self.pop()?;
                self.stack.push(value.clone());
                self.stack.push(base);
                self.stack.push(key);
                self.stack.push(value);
            }
            Instruction::Dup3 => {
                let len = self.stack.len();
                let first = len
                    .checked_sub(3)
                    .ok_or_else(|| Error::internal("dup3 needs three stack values"))?;
                let first_value = self.stack[first].clone();
                let second_value = self.stack[first + 1].clone();
                let third_value = self.stack[first + 2].clone();
                self.stack.push(first_value);
                self.stack.push(second_value);
                self.stack.push(third_value);
            }
            Instruction::Insert4 => {
                let value = self.pop()?;
                let key = self.pop()?;
                let base = self.pop()?;
                let receiver = self.pop()?;
                self.stack.push(value.clone());
                self.stack.push(receiver);
                self.stack.push(base);
                self.stack.push(key);
                self.stack.push(value);
            }
            Instruction::Perm3 => {
                let new_value = self.pop()?;
                let old_value = self.pop()?;
                let base = self.pop()?;
                self.stack.push(old_value);
                self.stack.push(base);
                self.stack.push(new_value);
            }
            Instruction::Perm4 => {
                let new_value = self.pop()?;
                let old_value = self.pop()?;
                let key = self.pop()?;
                let base = self.pop()?;
                self.stack.push(old_value);
                self.stack.push(base);
                self.stack.push(key);
                self.stack.push(new_value);
            }
            Instruction::Perm5 => {
                let new_value = self.pop()?;
                let old_value = self.pop()?;
                let key = self.pop()?;
                let base = self.pop()?;
                let receiver = self.pop()?;
                self.stack.push(old_value);
                self.stack.push(receiver);
                self.stack.push(base);
                self.stack.push(key);
                self.stack.push(new_value);
            }
            Instruction::Rot4Left => {
                let key = self.pop()?;
                let base = self.pop()?;
                let receiver = self.pop()?;
                let value = self.pop()?;
                self.stack.push(receiver);
                self.stack.push(base);
                self.stack.push(key);
                self.stack.push(value);
            }
            Instruction::PutField(index) => {
                let (base, value) = self.pop_pair()?;
                if let Completion::Throw(value) =
                    host.set_field(base, *index, value, self.strict)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::PutArrayEl => {
                let value = self.pop()?;
                let key = self.pop()?;
                let base = self.pop()?;
                if let Completion::Throw(value) =
                    host.set_property(base, key, value, self.strict)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::PutSuperValue => {
                let value = self.pop()?;
                let key = self.pop()?;
                let base = self.pop()?;
                let receiver = self.pop()?;
                if let Completion::Throw(value) =
                    host.set_super_property(receiver, base, key, value, self.strict)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::DefineField(index) => {
                let (base, value) = self.pop_pair()?;
                let retained_base = base.clone();
                match host.define_field(base, *index, value)? {
                    Completion::Return(_) => self.stack.push(retained_base),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::DefineFieldComputed => {
                let value = self.pop()?;
                let key = self.pop()?;
                let base = self.pop()?;
                let retained_base = base.clone();
                match host.define_field_computed(base, key, value)? {
                    Completion::Return(_) => self.stack.push(retained_base),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::DefineArrayEl => {
                let value = self.pop()?;
                let index = self.pop()?;
                let base = self.pop()?;
                let retained_base = base.clone();
                let retained_index = index.clone();
                match host.define_array_element(base, index, value)? {
                    Completion::Return(_) => {
                        self.stack.push(retained_base);
                        self.stack.push(retained_index);
                    }
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::SetProto => unreachable!("prototype literal dispatch was bypassed"),
            Instruction::CopyDataProperties => {
                unreachable!("spread literal dispatch was bypassed")
            }
            Instruction::CopyDataPropertiesExcluded { .. } => {
                unreachable!("object-rest copy dispatch was bypassed")
            }
            Instruction::Append => {
                let iterable = self.pop()?;
                let index = self.pop()?;
                let array = self.pop()?;
                match Self::append_iterable(host, array, index, iterable)? {
                    OperationOutcome::Value((array, index)) => {
                        self.stack.push(array);
                        self.stack.push(index);
                    }
                    OperationOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::Delete => {
                let (base, key) = self.pop_pair()?;
                match host.delete_property(base, key, self.strict)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::Drop => {
                self.pop()?;
            }
            Instruction::Nip => {
                let (_, value) = self.pop_pair()?;
                self.stack.push(value);
            }
            Instruction::Swap => {
                let (left, right) = self.pop_pair()?;
                self.stack.push(right);
                self.stack.push(left);
            }
            Instruction::Dup => {
                let value = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| Error::internal("dup on an empty stack"))?;
                self.stack.push(value);
            }
            Instruction::Dup1 => {
                let index = self
                    .stack
                    .len()
                    .checked_sub(2)
                    .ok_or_else(|| Error::internal("dup1 needs two stack values"))?;
                let value = self.stack[index].clone();
                self.stack.insert(index + 1, value);
            }
            Instruction::Neg
            | Instruction::Plus
            | Instruction::Inc
            | Instruction::Dec
            | Instruction::PostInc
            | Instruction::PostDec
            | Instruction::BitNot
            | Instruction::Not
            | Instruction::TypeOf
            | Instruction::IsUndefinedOrNull
            | Instruction::IsUndefined
            | Instruction::IsNull
            | Instruction::TypeOfIsUndefined
            | Instruction::TypeOfIsFunction
            | Instruction::Add
            | Instruction::Sub
            | Instruction::Mul
            | Instruction::Div
            | Instruction::Mod
            | Instruction::Pow
            | Instruction::Shl
            | Instruction::Sar
            | Instruction::Shr
            | Instruction::BitAnd
            | Instruction::BitXor
            | Instruction::BitOr
            | Instruction::Eq
            | Instruction::StrictEq
            | Instruction::Neq
            | Instruction::StrictNeq
            | Instruction::Lt
            | Instruction::Lte
            | Instruction::Gt
            | Instruction::Gte => unreachable!("numeric dispatch was bypassed"),
            Instruction::In => {
                let (key, object) = self.pop_pair()?;
                let Value::Object(object) = object else {
                    return Err(Error::new(ErrorKind::Type, "invalid 'in' operand"));
                };
                match host.has_property(key, object)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::InstanceOf => {
                let (candidate, target) = self.pop_pair()?;
                let Value::Object(target) = target else {
                    return Err(Error::new(
                        ErrorKind::Type,
                        "invalid 'instanceof' right operand",
                    ));
                };
                match host.is_instance_of(candidate, target)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::IfFalse(target) => {
                let value = self.pop()?;
                if !host.to_boolean(&value)? {
                    self.pc = checked_target(*target, code.len())?;
                }
            }
            Instruction::IfTrue(target) => {
                let value = self.pop()?;
                if host.to_boolean(&value)? {
                    self.pc = checked_target(*target, code.len())?;
                }
            }
            Instruction::Goto(target) => {
                self.pc = checked_target(*target, code.len())?;
            }
            Instruction::Catch(target) => {
                self.regions.push(VmUnwindRegion::Catch {
                    target: checked_target(*target, code.len())?,
                    stack_depth: self.stack.len(),
                });
            }
            Instruction::DropCatch => {
                let region = self
                    .regions
                    .pop()
                    .ok_or_else(|| Error::internal("DropCatch has no active catch handler"))?;
                let VmUnwindRegion::Catch { stack_depth, .. } = region else {
                    return Err(Error::internal(
                        "DropCatch did not target the innermost unwind region",
                    ));
                };
                if self.stack.len() != stack_depth {
                    return Err(Error::internal(
                        "DropCatch did not reach its catch entry depth",
                    ));
                }
            }
            Instruction::NipCatch => {
                let region = *self
                    .regions
                    .last()
                    .ok_or_else(|| Error::internal("NipCatch has no active catch handler"))?;
                let VmUnwindRegion::Catch { stack_depth, .. } = region else {
                    return Err(Error::internal(
                        "NipCatch did not target the innermost unwind region",
                    ));
                };
                if self.stack.len() <= stack_depth {
                    return Err(Error::internal(
                        "NipCatch has no value above its catch marker",
                    ));
                }
                // A return crossing a try/finally uses NipCatch to retain
                // its pending value while removing the private handler.
                // QuickJS does not synthesize CloseLocal along that edge;
                // if the finally body overrides the return, the same frame
                // may re-enter those captured lexical slots.
                host.prepare_captured_local_reuse()?;
                self.regions.pop();
                let value = self.pop()?;
                self.stack.truncate(stack_depth);
                self.stack.push(value);
            }
            Instruction::Gosub(target) => {
                let return_pc = i32::try_from(self.pc)
                    .map_err(|_| Error::internal("gosub return PC does not fit Int"))?;
                self.stack.push(Value::Int(return_pc));
                self.pc = checked_target(*target, code.len())?;
            }
            Instruction::Ret => {
                let Value::Int(target) = self.pop()? else {
                    return Err(Error::internal("invalid ret value"));
                };
                let target =
                    usize::try_from(target).map_err(|_| Error::internal("invalid ret value"))?;
                if target >= code.len() {
                    return Err(Error::internal("invalid ret value"));
                }
                self.pc = target;
            }
            Instruction::DropGosub => {
                if !matches!(self.pop()?, Value::Int(_)) {
                    return Err(Error::internal("invalid gosub cleanup value"));
                }
            }
            Instruction::ForOfStart => {
                let iterable = self.pop()?;
                match host.for_of_start(iterable)? {
                    ForOfStartOutcome::Record {
                        iterator,
                        next_method,
                    } => {
                        let record_base = self.stack.len();
                        self.stack.push(iterator);
                        self.stack.push(next_method);
                        self.regions.push(VmUnwindRegion::Iterator {
                            record_base,
                            enabled: true,
                            asynchronous: false,
                        });
                    }
                    ForOfStartOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::ForOfNext(offset) => {
                let (record_base, enabled, asynchronous) = match self.regions.last() {
                    Some(VmUnwindRegion::Iterator {
                        record_base,
                        enabled,
                        asynchronous,
                    }) => (*record_base, *enabled, *asynchronous),
                    _ => {
                        return Err(Error::internal(
                            "ForOfNext has no innermost iterator region",
                        ));
                    }
                };
                if asynchronous {
                    return Err(Error::internal(
                        "ForOfNext targeted an asynchronous iterator region",
                    ));
                }
                let expected_depth = record_base
                    .checked_add(2)
                    .and_then(|depth| depth.checked_add(usize::from(*offset)))
                    .ok_or_else(|| Error::internal("for-of offset overflow"))?;
                if self.stack.len() != expected_depth {
                    return Err(Error::internal(
                        "ForOfNext offset does not reach its iterator record",
                    ));
                }
                if !enabled {
                    self.stack.push(Value::Undefined);
                    self.stack.push(Value::Bool(true));
                    return Ok(None);
                }
                let iterator = self
                    .stack
                    .get(record_base)
                    .cloned()
                    .ok_or_else(|| Error::internal("iterator record is truncated"))?;
                let next_method = self
                    .stack
                    .get(record_base + 1)
                    .cloned()
                    .ok_or_else(|| Error::internal("iterator record is truncated"))?;
                match host.for_of_next(iterator, next_method)? {
                    ForOfNextOutcome::Result { value, done } => {
                        if done {
                            self.disable_iterator_region(record_base)?;
                        }
                        self.stack.push(value);
                        self.stack.push(Value::Bool(done));
                    }
                    ForOfNextOutcome::Throw(value) => {
                        self.disable_iterator_region(record_base)?;
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::ForInStart | Instruction::ForInNext => {
                unreachable!("for-in dispatch was bypassed")
            }
            Instruction::IteratorClose => {
                let (iterator, enabled, asynchronous) =
                    self.take_iterator_region(false, "IteratorClose")?;
                if asynchronous && !enabled {
                    return Err(Error::internal(
                        "IteratorClose targeted a pending async iterator region",
                    ));
                }
                if enabled {
                    match host.iterator_close(iterator, false)? {
                        IteratorCloseOutcome::Closed => {}
                        IteratorCloseOutcome::Throw(value) => {
                            return Ok(Some(Completion::Throw(value)));
                        }
                    }
                }
            }
            Instruction::IteratorClosePreserve => {
                let (iterator, enabled, asynchronous) =
                    self.take_iterator_region(true, "IteratorClosePreserve")?;
                if asynchronous && !enabled {
                    return Err(Error::internal(
                        "IteratorClosePreserve targeted a pending async iterator region",
                    ));
                }
                if enabled {
                    match host.iterator_close(iterator, false)? {
                        IteratorCloseOutcome::Closed => {}
                        IteratorCloseOutcome::Throw(value) => {
                            return Ok(Some(Completion::Throw(value)));
                        }
                    }
                }
            }
            Instruction::IteratorDropPreserve => {
                let (_, enabled, asynchronous) =
                    self.take_iterator_region(true, "IteratorDropPreserve")?;
                if asynchronous && !enabled {
                    return Err(Error::internal(
                        "IteratorDropPreserve targeted a pending async iterator region",
                    ));
                }
            }
            Instruction::IteratorDetachPreserve => {
                let (iterator, enabled, _) =
                    self.take_iterator_region(true, "IteratorDetachPreserve")?;
                if !enabled {
                    return Err(Error::internal(
                        "IteratorDetachPreserve targeted a disabled iterator region",
                    ));
                }
                self.stack.push(iterator);
            }
            Instruction::Import
            | Instruction::Call(_)
            | Instruction::TailCall(_)
            | Instruction::Eval { .. }
            | Instruction::CallMethod(_)
            | Instruction::TailCallMethod(_)
            | Instruction::Construct(_)
            | Instruction::ConstructSuper(_)
            | Instruction::InitDerivedConstructor
            | Instruction::Apply(_)
            | Instruction::ApplySuper
            | Instruction::ApplyEval { .. } => {
                unreachable!("call dispatch was bypassed")
            }
            Instruction::Return => {
                return self.pop().map(|value| Some(Completion::Return(value)));
            }
            Instruction::ReturnUndefined => {
                return Ok(Some(Completion::Return(Value::Undefined)));
            }
            Instruction::ReturnDerived(index) => {
                let value = self.pop()?;
                return host.return_derived(*index, value).map(Some);
            }
            Instruction::Throw => {
                return self.pop().map(|value| Some(Completion::Throw(value)));
            }
        }
        Ok(None)
    }
}
