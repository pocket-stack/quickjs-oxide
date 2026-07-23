//! Native cproto adaptation and exhaustive builtin dispatch.

use super::*;

impl Runtime {
    /// Tail-forward an ordinary `Function.prototype.call` invocation without
    /// retaining an otherwise redundant native frame around the target call.
    /// This preserves QuickJS's observable forwarding while using a Rust-side
    /// trampoline; upstream retains a thin C frame. Recursive `.call(...)`
    /// chains remain governed by the eventual target family's stack budget.
    pub(super) fn forward_function_prototype_call(
        &self,
        realm: ContextId,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<NativeConversion<(CallableRef, Value)>, RuntimeError> {
        let Value::Object(target) = this_value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(target) = self.as_callable(&target)? else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let this_argument = arguments.first().cloned().unwrap_or(Value::Undefined);
        Ok(NativeConversion::Value((target, this_argument)))
    }

    pub(super) fn concatenate_bound_arguments(
        &self,
        realm: ContextId,
        bound_arguments: &[Value],
        call_arguments: &[Value],
    ) -> Result<NativeConversion<Vec<Value>>, RuntimeError> {
        const MAX_CALL_ARGUMENTS: usize = 65_534;

        let Some(total) = bound_arguments.len().checked_add(call_arguments.len()) else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "stack overflow",
            )?));
        };
        if total > MAX_CALL_ARGUMENTS {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "stack overflow",
            )?));
        }
        let mut arguments = Vec::with_capacity(total);
        arguments.extend_from_slice(bound_arguments);
        arguments.extend_from_slice(call_arguments);
        Ok(NativeConversion::Value(arguments))
    }

    pub(super) fn call_internal(
        &self,
        mut caller_realm: ContextId,
        callable: &CallableRef,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        self.0.state.borrow().heap.context(caller_realm)?;
        self.validate_value_domain(&this_value, "call this value")?;
        for argument in arguments {
            self.validate_value_domain(argument, "call argument")?;
        }
        let mut callable = callable.clone();
        let mut this_value = this_value;
        let mut arguments = arguments.to_vec();
        // Function.prototype.call consumes one argument per forwarded logical
        // frame. Advance a window into the owned argv instead of repeatedly
        // allocating and copying every shrinking suffix.
        let mut argument_start = 0_usize;
        let mut forwarded_call_frames = Vec::new();
        let result = (|| loop {
            match self.bytecode_for_callable(&callable)? {
                CallableExecution::Bytecode {
                    bytecode,
                    closure_slots,
                } => {
                    return self.execute_bytecode_callable(
                        caller_realm,
                        &callable,
                        this_value,
                        Value::Undefined,
                        &arguments[argument_start..],
                        bytecode,
                        closure_slots,
                    );
                }
                CallableExecution::Native {
                    target,
                    realm,
                    min_readable_args,
                } => {
                    if target == NativeFunctionId::FunctionPrototypeCall {
                        if self.native_call_would_overflow(target) {
                            return Ok(Completion::Throw(self.new_native_error(
                                caller_realm,
                                NativeErrorKind::Internal,
                                "stack overflow",
                            )?));
                        }
                        forwarded_call_frames.push(self.push_native_active_frame(
                            callable.as_object().clone(),
                            realm,
                            target,
                            arguments.len() - argument_start,
                            (arguments.len() - argument_start).max(usize::from(min_readable_args)),
                        )?);
                        match self.forward_function_prototype_call(
                            realm,
                            this_value,
                            &arguments[argument_start..],
                        )? {
                            NativeConversion::Value((target, next_this)) => {
                                caller_realm = realm;
                                callable = target;
                                this_value = next_this;
                                if argument_start < arguments.len() {
                                    argument_start += 1;
                                }
                                continue;
                            }
                            NativeConversion::Throw(value) => {
                                return Ok(Completion::Throw(value));
                            }
                        }
                    }
                    if self.native_call_would_overflow(target) {
                        return Ok(Completion::Throw(self.new_native_error(
                            caller_realm,
                            NativeErrorKind::Internal,
                            "stack overflow",
                        )?));
                    }
                    return self.call_native_function(
                        &callable,
                        realm,
                        target,
                        min_readable_args,
                        this_value,
                        &arguments[argument_start..],
                    );
                }
                CallableExecution::Bound {
                    target,
                    this_value: bound_this,
                    arguments: bound_arguments,
                } => {
                    arguments = match self.concatenate_bound_arguments(
                        caller_realm,
                        &bound_arguments,
                        &arguments[argument_start..],
                    )? {
                        NativeConversion::Value(arguments) => arguments,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    argument_start = 0;
                    callable = target;
                    this_value = bound_this;
                }
            }
        })();
        let mut frame_error = None;
        while let Some(frame) = forwarded_call_frames.pop() {
            if let Err(error) = frame.finish()
                && frame_error.is_none()
            {
                frame_error = Some(error);
            }
        }
        frame_error.map_or(result, Err)
    }

    fn call_function_prototype_call(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.call did not receive a generic invocation",
            ));
        };
        let actual_arguments = &arguments.readable[..arguments.actual_arg_count];
        match self.forward_function_prototype_call(realm, this_value, actual_arguments)? {
            NativeConversion::Value((target, this_argument)) => {
                let forwarded = if actual_arguments.is_empty() {
                    &[]
                } else {
                    &actual_arguments[1..]
                };
                self.call_internal(realm, &target, this_argument, forwarded)
            }
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    /// Validate the active native frame and adapt the public call shape to the
    /// target's typed C-function protocol. Both ordinary calls and the raw
    /// iterator-next fast path pass through this single boundary.
    fn adapt_native_invocation(
        &self,
        target: NativeFunctionId,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<NativeInvocationAdaptation, RuntimeError> {
        let frame =
            self.0
                .state
                .borrow()
                .active_frames
                .last()
                .copied()
                .ok_or(RuntimeError::Invariant(
                    "native handler ran without an active frame",
                ))?;
        let ActiveFrameKind::Native {
            target: frame_target,
            actual_arg_count,
            readable_arg_count,
        } = frame.kind
        else {
            return Err(RuntimeError::Invariant(
                "native handler was not the top active frame",
            ));
        };
        if frame.realm != realm
            || frame_target != target
            || actual_arg_count != arguments.actual_arg_count
            || readable_arg_count != arguments.readable.len()
        {
            return Err(RuntimeError::Invariant(
                "active native frame disagrees with handler arguments",
            ));
        }
        // Some handlers do not inspect their adapted this/new-target input,
        // but keeping it rooted for the full dispatch is part of the ABI.
        let invocation = match (target.descriptor().cproto, invocation) {
            (
                NativeCProto::Generic
                | NativeCProto::GenericMagic
                | NativeCProto::UnaryF64
                | NativeCProto::BinaryF64,
                NativeInvocation::Call { this_value },
            ) => NativeInvocation::Call { this_value },
            (
                NativeCProto::Generic
                | NativeCProto::GenericMagic
                | NativeCProto::UnaryF64
                | NativeCProto::BinaryF64,
                NativeInvocation::Construct { new_target },
            ) => {
                // QuickJS's generic and floating-point ABIs receive
                // new.target in their receiver slot when an embedding
                // independently enables the constructor bit on the native
                // function object. Floating-point argument conversion stays
                // in the handler so abrupt completions keep their defining
                // realm and left-to-right order.
                NativeInvocation::Call {
                    this_value: new_target,
                }
            }
            (
                NativeCProto::Constructor | NativeCProto::ConstructorMagic,
                NativeInvocation::Construct { new_target },
            ) => NativeInvocation::Construct { new_target },
            (
                NativeCProto::Constructor | NativeCProto::ConstructorMagic,
                NativeInvocation::Call { .. },
            ) => {
                let exception =
                    self.new_native_error(realm, NativeErrorKind::Type, "must be called with new")?;
                return Ok(NativeInvocationAdaptation::Complete(Completion::Throw(
                    exception,
                )));
            }
            (
                NativeCProto::ConstructorOrFunction | NativeCProto::ConstructorOrFunctionMagic,
                NativeInvocation::Call { .. },
            ) => NativeInvocation::Construct {
                new_target: Value::Undefined,
            },
            (
                NativeCProto::ConstructorOrFunction | NativeCProto::ConstructorOrFunctionMagic,
                NativeInvocation::Construct { new_target },
            ) => NativeInvocation::Construct { new_target },
            (
                NativeCProto::Getter | NativeCProto::GetterMagic,
                NativeInvocation::Call { this_value },
            ) => NativeInvocation::Getter { this_value },
            (
                NativeCProto::Getter | NativeCProto::GetterMagic,
                NativeInvocation::Construct { new_target },
            ) => NativeInvocation::Getter {
                this_value: new_target,
            },
            (
                NativeCProto::Setter | NativeCProto::SetterMagic,
                NativeInvocation::Call { this_value },
            ) => NativeInvocation::Setter { this_value },
            (
                NativeCProto::Setter | NativeCProto::SetterMagic,
                NativeInvocation::Construct { new_target },
            ) => NativeInvocation::Setter {
                this_value: new_target,
            },
            (NativeCProto::IteratorNext, NativeInvocation::Call { this_value }) => {
                NativeInvocation::Call { this_value }
            }
            (NativeCProto::IteratorNext, NativeInvocation::Construct { new_target }) => {
                // Iterator-next functions are non-constructors by default.
                // If an embedder independently enables [[Construct]], QuickJS
                // passes new.target through the same native receiver slot.
                NativeInvocation::Call {
                    this_value: new_target,
                }
            }
            (_, NativeInvocation::Getter { .. } | NativeInvocation::Setter { .. }) => {
                return Err(RuntimeError::Invariant(
                    "native invocation was adapted more than once",
                ));
            }
        };
        Ok(NativeInvocationAdaptation::Invoke(invocation))
    }

    pub(in crate::runtime) fn dispatch_native_iterator_next_raw(
        &self,
        target: NativeFunctionId,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        let invocation = match self.adapt_native_invocation(target, realm, invocation, arguments)? {
            NativeInvocationAdaptation::Invoke(invocation) => invocation,
            NativeInvocationAdaptation::Complete(completion) => {
                return Ok(NativeInvokeOutcome::Completion(completion));
            }
        };
        match target {
            NativeFunctionId::StringIteratorNext => {
                self.call_string_iterator_next_raw(realm, invocation)
            }
            NativeFunctionId::ArrayIteratorNext => {
                self.call_array_iterator_next_raw(realm, invocation)
            }
            NativeFunctionId::MapIteratorNext => self.call_map_iterator_next_raw(realm, invocation),
            NativeFunctionId::SetIteratorNext => self.call_set_iterator_next_raw(realm, invocation),
            NativeFunctionId::RegExpStringIteratorNext => {
                self.call_regexp_string_iterator_next_raw(realm, invocation)
            }
            NativeFunctionId::GeneratorPrototypeResume(kind) => {
                self.call_generator_prototype_resume_raw(realm, kind, invocation, arguments)
            }
            _ => Err(RuntimeError::Invariant(
                "IteratorNext cproto has no raw native dispatcher",
            )),
        }
    }

    pub(in crate::runtime) fn dispatch_native_function(
        &self,
        target: NativeFunctionId,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let invocation = match self.adapt_native_invocation(target, realm, invocation, arguments)? {
            NativeInvocationAdaptation::Invoke(invocation) => invocation,
            NativeInvocationAdaptation::Complete(completion) => return Ok(completion),
        };
        match target {
            NativeFunctionId::FunctionPrototype => Ok(Completion::Return(Value::Undefined)),
            NativeFunctionId::FunctionConstructor(kind) => {
                self.call_function_constructor(realm, kind, invocation, arguments)
            }
            NativeFunctionId::GeneratorPrototypeResume(kind) => {
                self.call_generator_prototype_resume(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ArrayConstructor => {
                self.call_array_constructor(realm, invocation, arguments)
            }
            NativeFunctionId::ArrayIsArray => self.call_array_is_array(invocation, arguments),
            NativeFunctionId::ArrayFrom => self.call_array_from(realm, invocation, arguments),
            NativeFunctionId::ArrayOf => self.call_array_of(realm, invocation, arguments),
            NativeFunctionId::ArraySpeciesGetter => self.call_array_species_getter(invocation),
            NativeFunctionId::ArrayPrototypeAt => {
                self.call_array_prototype_at(realm, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeWith => {
                self.call_array_prototype_with(realm, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeConcat => {
                self.call_array_prototype_concat(realm, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeIteration(kind) => {
                self.call_array_prototype_iteration(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeReduce(kind) => {
                self.call_array_prototype_reduce(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeFill => {
                self.call_array_prototype_fill(realm, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeFind(kind) => {
                self.call_array_prototype_find(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeCopyWithin => {
                self.call_array_prototype_copy_within(realm, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeFlatten(kind) => {
                self.call_array_prototype_flatten(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeSearch(kind) => {
                self.call_array_prototype_search(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeJoin(kind) => {
                self.call_array_prototype_join(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeToString => {
                self.call_array_prototype_to_string(realm, invocation)
            }
            NativeFunctionId::ArrayPrototypePop(kind) => {
                self.call_array_prototype_pop(realm, kind, invocation)
            }
            NativeFunctionId::ArrayPrototypePush(kind) => {
                self.call_array_prototype_push(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeReverse => {
                self.call_array_prototype_reverse(realm, invocation)
            }
            NativeFunctionId::ArrayPrototypeToReversed => {
                self.call_array_prototype_to_reversed(realm, invocation)
            }
            NativeFunctionId::ArrayPrototypeSort => {
                self.call_array_prototype_sort(realm, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeToSorted => {
                self.call_array_prototype_to_sorted(realm, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeSlice(kind) => {
                self.call_array_prototype_slice(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeToSpliced => {
                self.call_array_prototype_to_spliced(realm, invocation, arguments)
            }
            NativeFunctionId::ArrayPrototypeIterator(kind) => {
                self.call_array_prototype_iterator(realm, kind, invocation)
            }
            NativeFunctionId::ArrayIteratorNext => self.call_array_iterator_next(realm, invocation),
            NativeFunctionId::Map(kind) => self.call_map_native(realm, kind, invocation, arguments),
            NativeFunctionId::MapIteratorNext => self.call_map_iterator_next(realm, invocation),
            NativeFunctionId::Set(kind) => self.call_set_native(realm, kind, invocation, arguments),
            NativeFunctionId::SetIteratorNext => self.call_set_iterator_next(realm, invocation),
            NativeFunctionId::ThrowTypeError => {
                self.call_throw_type_error(realm, invocation, arguments)
            }
            NativeFunctionId::FunctionPrototypeCall => {
                self.call_function_prototype_call(realm, invocation, arguments)
            }
            NativeFunctionId::FunctionPrototypeApply => {
                self.call_function_prototype_apply(realm, invocation, arguments)
            }
            NativeFunctionId::FunctionPrototypeBind => {
                self.call_function_prototype_bind(realm, invocation, arguments)
            }
            NativeFunctionId::FunctionPrototypeToString => {
                self.call_function_prototype_to_string(realm, invocation)
            }
            NativeFunctionId::FunctionPrototypeHasInstance => {
                self.call_function_prototype_has_instance(realm, invocation, arguments)
            }
            NativeFunctionId::FunctionPrototypeFileName => {
                self.call_function_prototype_file_name(invocation)
            }
            NativeFunctionId::FunctionPrototypePosition(selector) => {
                self.call_function_prototype_position(invocation, selector)
            }
            NativeFunctionId::ObjectConstructor => {
                self.call_object_constructor(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectCreate => self.call_object_create(realm, invocation, arguments),
            NativeFunctionId::ObjectGetPrototypeOf => {
                self.call_object_get_prototype_of(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectSetPrototypeOf => {
                self.call_object_set_prototype_of(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectDefineProperty => {
                self.call_object_define_property(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectDefineProperties => {
                self.call_object_define_properties(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectGetOwnPropertyKeys(kind) => {
                self.call_object_get_own_property_keys(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ObjectGroupBy => {
                self.call_object_group_by(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectKeys(kind) => {
                self.call_object_keys(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ObjectExtensibility(kind) => {
                self.call_object_extensibility(kind, invocation, arguments)
            }
            NativeFunctionId::ObjectGetOwnPropertyDescriptor => {
                self.call_object_get_own_property_descriptor(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectGetOwnPropertyDescriptors => {
                self.call_object_get_own_property_descriptors(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectIs => self.call_object_is(invocation, arguments),
            NativeFunctionId::ObjectAssign => self.call_object_assign(realm, invocation, arguments),
            NativeFunctionId::ObjectIntegrity(kind) => {
                self.call_object_integrity(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ObjectFromEntries => {
                self.call_object_from_entries(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectHasOwn => {
                self.call_object_has_own(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectPrototypeToString => {
                self.call_object_prototype_to_string(realm, invocation)
            }
            NativeFunctionId::ObjectPrototypeToLocaleString => {
                self.call_object_prototype_to_locale_string(realm, invocation)
            }
            NativeFunctionId::ObjectPrototypeValueOf => {
                self.call_object_prototype_value_of(realm, invocation)
            }
            NativeFunctionId::ObjectPrototypeHasOwnProperty => {
                self.call_object_prototype_has_own_property(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectPrototypeIsPrototypeOf => {
                self.call_object_prototype_is_prototype_of(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectPrototypePropertyIsEnumerable => {
                self.call_object_prototype_property_is_enumerable(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectPrototypeProtoGetter => {
                self.call_object_prototype_proto_getter(realm, invocation)
            }
            NativeFunctionId::ObjectPrototypeProtoSetter => {
                self.call_object_prototype_proto_setter(realm, invocation, arguments)
            }
            NativeFunctionId::ObjectPrototypeDefineAccessor(kind) => {
                self.call_object_prototype_define_accessor(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ObjectPrototypeLookupAccessor(kind) => {
                self.call_object_prototype_lookup_accessor(realm, kind, invocation, arguments)
            }
            NativeFunctionId::Json(kind) => {
                self.call_json_native(realm, kind, invocation, arguments)
            }
            NativeFunctionId::Reflect(kind) => {
                self.call_reflect(realm, kind, invocation, arguments)
            }
            NativeFunctionId::Date(kind) => {
                self.call_date_native(realm, kind, invocation, arguments)
            }
            NativeFunctionId::RegExp(kind) => {
                self.call_regexp_native(realm, kind, invocation, arguments)
            }
            NativeFunctionId::PrimitiveConstructor(kind) => {
                self.call_primitive_constructor(realm, kind, invocation, arguments)
            }
            NativeFunctionId::StringStatic(selector) => {
                self.call_string_static(realm, selector, invocation, arguments)
            }
            NativeFunctionId::StringCodePointRange => {
                self.call_string_code_point_range(realm, invocation, arguments)
            }
            NativeFunctionId::PrimitivePrototypeToString(kind) => {
                self.call_primitive_prototype_to_string(realm, kind, invocation, arguments)
            }
            NativeFunctionId::PrimitivePrototypeValueOf(kind) => {
                self.call_primitive_prototype_value_of(realm, kind, invocation)
            }
            NativeFunctionId::StringPrototypeCharAt(selector) => {
                self.call_string_prototype_char_at(realm, selector, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeCharCodeAt => {
                self.call_string_prototype_char_code_at(realm, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeConcat => {
                self.call_string_prototype_concat(realm, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeCodePointAt => {
                self.call_string_prototype_code_point_at(realm, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeWellFormed(selector) => {
                self.call_string_prototype_well_formed(realm, selector, invocation)
            }
            NativeFunctionId::StringPrototypeIndexOf(selector) => {
                self.call_string_prototype_index_of(realm, selector, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeIncludes(selector) => {
                self.call_string_prototype_includes(realm, selector, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeReplace(selector) => {
                self.call_string_prototype_replace(realm, selector, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeMatch => {
                self.call_string_prototype_match(realm, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeMatchAll => {
                self.call_string_prototype_match_all(realm, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeSearch => {
                self.call_string_prototype_search(realm, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeSplit => {
                self.call_string_prototype_split(realm, invocation, arguments)
            }
            NativeFunctionId::MathMinMax(kind) => {
                self.call_math_min_max(realm, kind, invocation, arguments)
            }
            NativeFunctionId::MathUnary(kind) => {
                self.call_math_unary(realm, kind, invocation, arguments)
            }
            NativeFunctionId::MathBinary(kind) => {
                self.call_math_binary(realm, kind, invocation, arguments)
            }
            NativeFunctionId::MathHypot => self.call_math_hypot(realm, invocation, arguments),
            NativeFunctionId::MathRandom => self.call_math_random(realm, invocation),
            NativeFunctionId::MathImul => self.call_math_imul(realm, invocation, arguments),
            NativeFunctionId::MathClz32 => self.call_math_clz32(realm, invocation, arguments),
            NativeFunctionId::MathSumPrecise => {
                self.call_math_sum_precise(realm, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeSubrange(selector) => {
                self.call_string_prototype_subrange(realm, selector, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeRepeat => {
                self.call_string_prototype_repeat(realm, invocation, arguments)
            }
            NativeFunctionId::StringPrototypePad(selector) => {
                self.call_string_prototype_pad(realm, selector, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeTrim(selector) => {
                self.call_string_prototype_trim(realm, selector, invocation)
            }
            NativeFunctionId::StringPrototypeCase(selector) => {
                self.call_string_prototype_case(realm, selector, invocation)
            }
            NativeFunctionId::StringPrototypeCreateHtml(selector) => {
                self.call_string_prototype_create_html(realm, selector, invocation, arguments)
            }
            NativeFunctionId::IteratorPrototypeIterator => {
                self.call_iterator_prototype_iterator(invocation)
            }
            NativeFunctionId::IteratorPrototypeToStringTagGetter => {
                self.call_iterator_prototype_to_string_tag_getter(invocation)
            }
            NativeFunctionId::IteratorPrototypeToStringTagSetter => {
                self.call_iterator_prototype_to_string_tag_setter(realm, invocation, arguments)
            }
            NativeFunctionId::StringPrototypeIterator => {
                self.call_string_prototype_iterator(realm, invocation)
            }
            NativeFunctionId::StringIteratorNext => {
                self.call_string_iterator_next(realm, invocation)
            }
            NativeFunctionId::RegExpStringIteratorNext => {
                self.call_regexp_string_iterator_next(realm, invocation)
            }
            NativeFunctionId::SymbolRegistry(kind) => {
                self.call_symbol_registry(realm, kind, invocation, arguments)
            }
            NativeFunctionId::SymbolPrototypeDescription => {
                self.call_symbol_prototype_description(realm, invocation)
            }
            NativeFunctionId::BigIntAsN(kind) => {
                self.call_bigint_as_n(realm, kind, invocation, arguments)
            }
            NativeFunctionId::GlobalEval => self.call_global_eval(realm, invocation, arguments),
            NativeFunctionId::GlobalNumberParse(kind) => {
                self.call_global_number_parse(realm, kind, invocation, arguments)
            }
            NativeFunctionId::GlobalNumberPredicate(kind) => {
                self.call_global_number_predicate(realm, kind, invocation, arguments)
            }
            NativeFunctionId::GlobalUriCodec(kind) => {
                self.call_global_uri_codec(realm, kind, invocation, arguments)
            }
            NativeFunctionId::NumberPredicate(kind) => {
                self.call_number_predicate(kind, invocation, arguments)
            }
            NativeFunctionId::NumberPrototypeFormat(kind) => {
                self.call_number_prototype_format(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ErrorConstructor(kind) => {
                self.call_error_constructor(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ErrorPrototypeToString => {
                self.call_error_prototype_to_string(realm, invocation)
            }
            NativeFunctionId::ErrorIsError => self.call_error_is_error(arguments),
            #[cfg(test)]
            NativeFunctionId::ActiveFrameProbe => self.call_active_frame_probe(realm, arguments),
            #[cfg(test)]
            NativeFunctionId::ArgumentProbe
            | NativeFunctionId::ConstructorProbe
            | NativeFunctionId::ConstructorOrFunctionProbe => {
                if matches!(arguments.readable.first(), Some(Value::Bool(false))) {
                    return Ok(Completion::Throw(Value::String(JsString::from_static(
                        "native probe throw",
                    ))));
                }
                if matches!(arguments.readable.first(), Some(Value::Bool(true))) {
                    return Err(RuntimeError::Invariant("native probe engine error"));
                }
                let padded_undefined = arguments.readable[arguments.actual_arg_count..]
                    .iter()
                    .filter(|value| matches!(value, Value::Undefined))
                    .count();
                let active_function = self.active_function()?.object_id();
                let invocation_target_is_function = match invocation {
                    NativeInvocation::Call {
                        this_value: Value::Object(object),
                    } => object.object_id() == active_function,
                    NativeInvocation::Construct {
                        new_target: Value::Object(object),
                    } => object.object_id() == active_function,
                    NativeInvocation::Getter {
                        this_value: Value::Object(object),
                    } => object.object_id() == active_function,
                    NativeInvocation::Setter {
                        this_value: Value::Object(object),
                    } => object.object_id() == active_function,
                    NativeInvocation::Call { .. }
                    | NativeInvocation::Construct { .. }
                    | NativeInvocation::Getter { .. }
                    | NativeInvocation::Setter { .. } => false,
                };
                let result = format!(
                    "{}|{}|{}|{}",
                    arguments.actual_arg_count,
                    arguments.readable.len(),
                    padded_undefined,
                    invocation_target_is_function
                );
                Ok(Completion::Return(Value::String(JsString::try_from_utf8(
                    &result,
                )?)))
            }
        }
    }
}
