//! Synchronous generator intrinsics and resumable activation plumbing.
//!
//! QuickJS keeps `%GeneratorPrototype%` and
//! `%GeneratorFunction.prototype%` as realm roots even though the hidden
//! `GeneratorFunction` constructor is not installed on the global object.
//! This module owns that exact graph; the execution state machine is kept here
//! as the generator milestone grows instead of expanding `runtime.rs`.

use super::*;
use crate::heap::{GeneratorState, ObjectData, ObjectPayload};
use crate::runtime::vm_host::{EncodedGeneratorActivation, GeneratorVmRunOutcome, RuntimeVmHost};
use crate::vm::{VmResume, VmSuspension};

impl Runtime {
    pub(super) fn initialize_generator_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        iterator_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let generator_prototype = self.new_object(Some(iterator_prototype))?;
        for (kind, name) in [
            (GeneratorResumeKind::Next, "next"),
            (GeneratorResumeKind::Return, "return"),
            (GeneratorResumeKind::Throw, "throw"),
        ] {
            self.define_native_builtin_auto_init(
                &generator_prototype,
                realm,
                NativeFunctionId::GeneratorPrototypeResume(kind),
                name,
                1,
                1,
            )?;
        }
        self.define_generator_to_string_tag(&generator_prototype, "Generator")?;

        let generator_function_prototype = self.new_object(Some(function_prototype))?;
        self.define_generator_to_string_tag(&generator_function_prototype, "GeneratorFunction")?;

        let function_constructor = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .function_constructor
            .ok_or(RuntimeError::Invariant(
                "Generator initialization requires the Function constructor",
            ))?;
        let function_constructor =
            ObjectRef::from_borrowed_handle(self.clone(), function_constructor)?;
        let constructor = self.new_native_builtin(
            &function_constructor,
            realm,
            NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Generator),
            1,
            "GeneratorFunction",
            1,
        )?;

        // JS_NewCConstructor keeps the hidden constructor's own `prototype`
        // fully fixed. JS_NEW_CTOR_READONLY only changes the reciprocal
        // `%GeneratorFunction.prototype%.constructor` descriptor below.
        self.define_function_data_property(
            constructor.as_object(),
            "prototype",
            Value::Object(generator_function_prototype.clone()),
            false,
            false,
        )?;
        self.define_function_data_property(
            &generator_function_prototype,
            "constructor",
            Value::Object(constructor.as_object().clone()),
            false,
            true,
        )?;

        // JS_SetConstructor2 then creates the second reciprocal pair.  Its
        // `constructor` value is `%GeneratorFunction.prototype%`, not the
        // hidden dynamic constructor.
        self.define_function_data_property(
            &generator_function_prototype,
            "prototype",
            Value::Object(generator_prototype.clone()),
            false,
            true,
        )?;
        self.define_function_data_property(
            &generator_prototype,
            "constructor",
            Value::Object(generator_function_prototype.clone()),
            false,
            true,
        )?;

        self.0.state.borrow_mut().heap.attach_generator_intrinsics(
            realm,
            GeneratorRealmData {
                prototype: generator_prototype.object_id(),
                function_prototype: generator_function_prototype.object_id(),
            },
        )?;
        Ok(())
    }

    fn define_generator_to_string_tag(
        &self,
        object: &ObjectRef,
        value: &'static str,
    ) -> Result<(), RuntimeError> {
        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static(value))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Generator intrinsic toStringTag definition was rejected",
            ));
        }
        Ok(())
    }

    /// Finish QuickJS's generator-function call sequence after the bytecode
    /// frame has reached `OP_initial_yield` and its active-frame record has
    /// been popped. The public `.prototype` lookup intentionally happens now:
    /// pinned QuickJS initializes parameters first, then performs
    /// `js_create_from_ctor`, whose getter may throw or re-enter JavaScript.
    pub(super) fn finish_generator_function_call(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
        host: RuntimeVmHost,
        suspension: VmSuspension,
    ) -> Result<Completion, RuntimeError> {
        let prototype = match self.generator_instance_prototype(caller_realm, callable)? {
            NativeConversion::Value(prototype) => prototype,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let activation = host.encode_generator_activation(suspension)?;
        if activation.state != GeneratorState::SuspendedStart {
            return Err(RuntimeError::Invariant(
                "new generator activation is not suspended at start",
            ));
        }
        let generator = self.allocate_generator_object(&prototype, activation)?;
        Ok(Completion::Return(Value::Object(generator)))
    }

    fn generator_instance_prototype(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype =
            match self.get_property_in_realm(caller_realm, callable.as_object(), &prototype_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
        if let Value::Object(prototype) = prototype {
            return Ok(NativeConversion::Value(prototype));
        }
        let realm = self.callable_realm(callable)?;
        let prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .generator
            .ok_or(RuntimeError::Invariant(
                "generator callable realm has no Generator intrinsics",
            ))?
            .prototype;
        Ok(NativeConversion::Value(ObjectRef::from_borrowed_handle(
            self.clone(),
            prototype,
        )?))
    }

    fn allocate_generator_object(
        &self,
        prototype: &ObjectRef,
        activation: EncodedGeneratorActivation,
    ) -> Result<ObjectRef, RuntimeError> {
        let atoms = activation.atoms();
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let mut retained_atoms = Vec::with_capacity(atoms.len());
        for atom in atoms {
            if let Err(error) = state.atoms.retain(atom) {
                state.release_atoms(retained_atoms)?;
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
            retained_atoms.push(atom);
        }
        let object = match state.heap.allocate_object(ObjectData::generator(
            shape,
            Vec::new(),
            activation.data.clone(),
        )) {
            Ok(object) => object,
            Err(error) => {
                state.release_atoms(retained_atoms)?;
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        drop(activation);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    pub(super) fn call_generator_prototype_resume(
        &self,
        realm: ContextId,
        kind: GeneratorResumeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match self.call_generator_prototype_resume_raw(realm, kind, invocation, arguments)? {
            NativeInvokeOutcome::Completion(completion) => Ok(completion),
            NativeInvokeOutcome::IteratorNextRaw { value, done } => Ok(Completion::Return(
                Value::Object(self.new_iterator_result(realm, value, done)?),
            )),
        }
    }

    pub(super) fn call_generator_prototype_resume_raw(
        &self,
        realm: ContextId,
        kind: GeneratorResumeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Generator resume did not receive an iterator-next invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Generator resume has no readable argument slot",
            ))?;
        let Value::Object(generator) = this_value else {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(realm, NativeErrorKind::Type, "not a generator")?,
            )));
        };
        let snapshot = {
            let state = self.0.state.borrow();
            if !matches!(
                state.heap.object(generator.object_id())?.payload,
                ObjectPayload::Generator { .. }
            ) {
                None
            } else {
                Some(state.heap.generator_snapshot(generator.object_id())?)
            }
        };
        let Some((previous_state, activation)) = snapshot else {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(realm, NativeErrorKind::Type, "not a generator")?,
            )));
        };
        match previous_state {
            GeneratorState::Completed => {
                if activation.is_some() {
                    return Err(RuntimeError::Invariant(
                        "completed generator retained an activation",
                    ));
                }
                return Ok(Self::completed_generator_outcome(kind, argument));
            }
            GeneratorState::Executing => {
                if activation.is_some() {
                    return Err(RuntimeError::Invariant(
                        "executing generator retained a dormant activation",
                    ));
                }
                return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                    self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "cannot invoke a running generator",
                    )?,
                )));
            }
            GeneratorState::SuspendedStart
            | GeneratorState::SuspendedYield
            | GeneratorState::SuspendedYieldStar => {}
        }
        let activation = activation.ok_or(RuntimeError::Invariant(
            "suspended generator has no activation",
        ))?;

        // Root the cloned snapshot before detaching the generator object's raw
        // occurrences. `begin_generator_resume` may otherwise finalize its
        // function, VarRefs, values, or callee realm immediately.
        let rooted = RuntimeVmHost::decode_generator_activation(
            self.clone(),
            previous_state,
            realm,
            &activation,
        )?;
        {
            let mut state = self.0.state.borrow_mut();
            let (began_state, _moved, cleanup) =
                match state.heap.begin_generator_resume(generator.object_id()) {
                    Ok(result) => result,
                    Err(error) => {
                        let _ = state.heap.complete_generator(generator.object_id());
                        return Err(error.into());
                    }
                };
            let changed = began_state != previous_state;
            let cleanup_result = state.apply_cleanup(cleanup);
            if changed {
                let _ = state.heap.complete_generator(generator.object_id());
                cleanup_result?;
                return Err(RuntimeError::Invariant(
                    "generator activation changed between snapshot and resume",
                ));
            }
            if let Err(error) = cleanup_result {
                let _ = state.heap.complete_generator(generator.object_id());
                return Err(error);
            }
        }

        if previous_state == GeneratorState::SuspendedStart && kind != GeneratorResumeKind::Next {
            self.complete_executing_generator(&generator)?;
            drop(rooted);
            return Ok(Self::completed_generator_outcome(kind, argument));
        }

        let resume = match previous_state {
            GeneratorState::SuspendedStart => None,
            GeneratorState::SuspendedYield | GeneratorState::SuspendedYieldStar => {
                Some(match kind {
                    GeneratorResumeKind::Next => VmResume::Next(argument),
                    GeneratorResumeKind::Return => VmResume::Return(argument),
                    GeneratorResumeKind::Throw => VmResume::Throw(argument),
                })
            }
            GeneratorState::Executing | GeneratorState::Completed => unreachable!(),
        };
        let outcome = match rooted.run(self, resume) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.complete_executing_generator(&generator)?;
                return Err(error);
            }
        };
        match outcome {
            GeneratorVmRunOutcome::Complete(completion) => {
                self.complete_executing_generator(&generator)?;
                Ok(match completion {
                    Completion::Return(value) => {
                        NativeInvokeOutcome::IteratorNextRaw { value, done: true }
                    }
                    Completion::Throw(value) => {
                        NativeInvokeOutcome::Completion(Completion::Throw(value))
                    }
                })
            }
            GeneratorVmRunOutcome::Suspend {
                yielded,
                activation,
            } => {
                let state = activation.state;
                if state == GeneratorState::SuspendedYieldStar
                    && !matches!(yielded, Value::Object(_))
                {
                    self.complete_executing_generator(&generator)?;
                    return Err(RuntimeError::Invariant(
                        "yield* suspension did not retain an iterator-result object",
                    ));
                }
                if let Err(error) = self.store_generator_suspension(&generator, &activation) {
                    let _ = self.complete_executing_generator(&generator);
                    return Err(error);
                }
                drop(activation);
                match state {
                    GeneratorState::SuspendedYield => Ok(NativeInvokeOutcome::IteratorNextRaw {
                        value: yielded,
                        done: false,
                    }),
                    GeneratorState::SuspendedYieldStar => {
                        // QuickJS's JS_CFUNC_iterator_next ABI sets pdone=2:
                        // both ordinary calls and direct IteratorNext callers
                        // receive the delegate's exact result object.
                        Ok(NativeInvokeOutcome::Completion(Completion::Return(yielded)))
                    }
                    GeneratorState::SuspendedStart
                    | GeneratorState::Executing
                    | GeneratorState::Completed => Err(RuntimeError::Invariant(
                        "resumed generator stopped in an invalid state",
                    )),
                }
            }
        }
    }

    fn completed_generator_outcome(
        kind: GeneratorResumeKind,
        argument: Value,
    ) -> NativeInvokeOutcome {
        match kind {
            GeneratorResumeKind::Next => NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Undefined,
                done: true,
            },
            GeneratorResumeKind::Return => NativeInvokeOutcome::IteratorNextRaw {
                value: argument,
                done: true,
            },
            GeneratorResumeKind::Throw => {
                NativeInvokeOutcome::Completion(Completion::Throw(argument))
            }
        }
    }

    fn store_generator_suspension(
        &self,
        generator: &ObjectRef,
        activation: &EncodedGeneratorActivation,
    ) -> Result<(), RuntimeError> {
        let atoms = activation.atoms();
        let mut state = self.0.state.borrow_mut();
        let mut retained_atoms = Vec::with_capacity(atoms.len());
        for atom in atoms {
            if let Err(error) = state.atoms.retain(atom) {
                state.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
            retained_atoms.push(atom);
        }
        if let Err(error) = state.heap.suspend_generator(
            generator.object_id(),
            activation.state,
            activation.data.clone(),
        ) {
            state.release_atoms(retained_atoms)?;
            return Err(error.into());
        }
        Ok(())
    }

    fn complete_executing_generator(&self, generator: &ObjectRef) -> Result<(), RuntimeError> {
        self.0
            .state
            .borrow_mut()
            .heap
            .complete_generator(generator.object_id())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_activation_is_dormant_between_start_yield_and_completion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(generator) = context
            .eval(
                "globalThis.__generator = (function* () { \
                     let resumed = yield 1; return resumed + 1; \
                 })(); __generator",
            )
            .unwrap()
        else {
            panic!("generator call did not return an object");
        };
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .generator_snapshot(generator.object_id())
                .unwrap()
                .0,
            GeneratorState::SuspendedStart
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());

        assert_eq!(
            context
                .eval("let r1 = __generator.next(99); r1.value === 1 && r1.done === false")
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .generator_snapshot(generator.object_id())
                .unwrap()
                .0,
            GeneratorState::SuspendedYield
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());

        assert_eq!(
            context
                .eval("let r2 = __generator.next(41); r2.value === 42 && r2.done === true")
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .generator_snapshot(generator.object_id())
                .unwrap()
                .0,
            GeneratorState::Completed
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn generator_abrupt_resume_and_reentry_match_quickjs_states() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    "let entered = 0; \
                     let returned = (function*(){ entered++; yield 1 })(); \
                     let rr = returned.return(42); \
                     let marker = {}; \
                     let thrown = (function*(){ entered++; yield 1 })(); \
                     let caught; try { thrown.throw(marker) } catch (e) { caught = e } \
                     let active; active = (function*(){ \
                         let reentry = false; \
                         try { active.next() } catch (e) { reentry = e instanceof TypeError } \
                         yield reentry; throw marker; \
                     })(); \
                     let first = active.next(); \
                     let abrupt; try { active.next() } catch (e) { abrupt = e } \
                     entered === 0 && rr.value === 42 && rr.done && \
                     caught === marker && thrown.next().done && \
                     first.value === true && !first.done && abrupt === marker && active.next().done",
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn generator_nan_argument_does_not_fake_an_activation_change() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    "let result = (function* (argument) { yield 1 })(NaN).next(); \
                     result.value === 1 && result.done === false",
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn thrown_resume_backtrace_stays_on_the_suspended_yield() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval_with_filename(
                    concat!(
                        "let error = Error(\"boom\"); delete error.stack;\n",
                        "let iterator = (function* probe() {\n",
                        "  yield 1;\n",
                        "  yield 2;\n",
                        "})();\n",
                        "iterator.next();\n",
                        "let stack;\n",
                        "try {\n",
                        "  iterator.throw(error);\n",
                        "} catch (caught) {\n",
                        "  stack = caught.stack;\n",
                        "}\n",
                        "stack;",
                    ),
                    "generator-stack.js",
                )
                .unwrap(),
            Value::String(JsString::from_static(concat!(
                "    at probe (generator-stack.js:3:3)\n",
                "    at throw (native)\n",
                "    at <eval> (generator-stack.js:9:17)\n",
            ),))
        );
    }

    #[test]
    fn yield_star_preserves_delegate_result_identity() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    "let result = { value: 42, done: false }; \
                     let delegate = { \
                         [Symbol.iterator]() { return this }, \
                         next() { return result }, \
                         return(value) { return { value, done: true } } \
                     }; \
                     let iterator = (function*(){ return yield* delegate })(); \
                     let same = iterator.next() === result; \
                     let closed = iterator.return(9); \
                     same && closed.value === 9 && closed.done",
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn generator_snapshot_roots_captures_private_bindings_and_prototype_choice() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    "function outer() { \
                         let captured = 1; \
                         return function* (argument) { \
                             let local = 2; \
                             let bump = () => ++local; \
                             class C { \
                                 #field = 40; \
                                 #method() { return 2 } \
                                 value() { return this.#field + this.#method() } \
                             } \
                             let instance = new C; \
                             yield argument + captured + local; \
                             return bump() + captured + instance.value(); \
                         }; \
                     } \
                     let iterator = outer()(38); \
                     let roots = iterator.next().value === 41 && iterator.next().value === 46; \
                     let probe = function*(){}; \
                     let intrinsic = Object.getPrototypeOf(probe.prototype); \
                     let custom = {}; probe.prototype = custom; \
                     let explicit = Object.getPrototypeOf(probe()) === custom; \
                     probe.prototype = null; \
                     roots && explicit && Object.getPrototypeOf(probe()) === intrinsic",
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn dormant_generator_roots_survive_gc_and_release_after_completion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(generator) = context
            .eval(
                "globalThis.__gcGenerator = (function(){ \
                     let capturedOuter = { value: 2 }; \
                     return (function*(){ \
                         let directObject = { value: 1 }; \
                         let directSymbol = Symbol('kept'); \
                         let capturedLocal = { value: 3 }; \
                         let readCaptured = () => capturedOuter.value + capturedLocal.value; \
                         class C { \
                             #field = 30; \
                             #method() { return 5 } \
                             value() { return this.#field + this.#method() } \
                         } \
                         let instance = new C; \
                         yield 0; \
                         return directObject.value + \
                             (directSymbol.description === 'kept' ? 4 : 0) + \
                             readCaptured() + instance.value(); \
                     })(); \
                 })(); __gcGenerator",
            )
            .unwrap()
        else {
            panic!("GC probe did not return a generator");
        };
        assert_eq!(
            context
                .eval(
                    "let firstGcResult = __gcGenerator.next(); \
                     firstGcResult.value === 0 && !firstGcResult.done",
                )
                .unwrap(),
            Value::Bool(true)
        );

        runtime.run_gc().unwrap();
        assert_eq!(
            context
                .eval(
                    "let finalGcResult = __gcGenerator.next(); \
                     finalGcResult.value === 45 && finalGcResult.done",
                )
                .unwrap(),
            Value::Bool(true)
        );
        context.eval("__gcGenerator = undefined").unwrap();
        drop(generator);
        assert!(runtime.run_gc().unwrap().cleanup.finalized_objects >= 1);
        assert_eq!(runtime.heap_counts().zombies, 0);
    }

    #[test]
    fn dormant_generator_does_not_keep_its_creation_caller_realm_alive() {
        let runtime = Runtime::new();
        let mut defining = runtime.new_context();
        let mut caller = runtime.new_context();
        let Value::Object(function_object) = defining
            .eval("(function* crossRealmGenerator() { yield 1; return 2; })")
            .unwrap()
        else {
            panic!("cross-realm generator function was not an object");
        };
        let function_callable = runtime.as_callable(&function_object).unwrap().unwrap();
        let Value::Object(generator) = caller
            .call(&function_callable, Value::Undefined, &[])
            .unwrap()
        else {
            panic!("cross-realm generator call did not return an object");
        };
        assert_eq!(runtime.heap_counts().context_nodes, 2);

        drop(caller);
        runtime.run_gc().unwrap();
        assert_eq!(
            runtime.heap_counts().context_nodes,
            1,
            "a dormant generator must not retain the realm which called it"
        );

        let next_key = runtime.intern_property_key("next").unwrap();
        let Value::Object(next_object) = defining.get_property(&generator, &next_key).unwrap()
        else {
            panic!("cross-realm generator next method was not an object");
        };
        let next_callable = runtime.as_callable(&next_object).unwrap().unwrap();
        let value_key = runtime.intern_property_key("value").unwrap();
        for expected in [1, 2] {
            let Value::Object(result) = defining
                .call(&next_callable, Value::Object(generator.clone()), &[])
                .unwrap()
            else {
                panic!("cross-realm generator next result was not an object");
            };
            assert_eq!(
                defining.get_property(&result, &value_key).unwrap(),
                Value::Int(expected)
            );
        }
        assert_eq!(runtime.heap_counts().context_nodes, 1);

        drop(next_callable);
        drop(next_object);
        drop(function_callable);
        drop(function_object);
        drop(generator);
        drop(defining);
        runtime.run_gc().unwrap();
        assert_eq!(runtime.heap_counts().live, 0);
    }

    #[test]
    fn prototype_getter_runs_after_parameters_and_its_throw_aborts_creation() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context
            .eval(
                "globalThis.__order = 0; globalThis.__marker = {}; \
                 (function* (value = (__order = 1, 0)) { __order = 2; yield value })",
            )
            .unwrap()
        else {
            panic!("generator source did not return a function");
        };
        let callable = runtime.as_callable(&function).unwrap().unwrap();
        let Value::Object(getter) = context
            .eval(
                "(function(){ \
                     if (__order !== 1) throw 'prototype getter ran before parameters'; \
                     throw __marker; \
                 })",
            )
            .unwrap()
        else {
            panic!("getter source did not return a function");
        };
        let marker = context.eval("__marker").unwrap();
        let prototype = runtime.intern_property_key("prototype").unwrap();

        // Generator functions expose a non-configurable data property, so
        // ordinary source cannot install this accessor before Proxy support.
        // The internal replacement isolates the observable call ordering and
        // abrupt-completion path exercised by js_create_from_ctor.
        runtime
            .store_property_slot(
                &function,
                &prototype,
                PropertyFlags::accessor(false, false),
                PropertySlot::Accessor {
                    get: Some(getter.object_id()),
                    set: None,
                },
            )
            .unwrap();
        assert_eq!(
            runtime
                .call_internal(context.realm, &callable, Value::Undefined, &[])
                .unwrap(),
            Completion::Throw(marker)
        );
        assert_eq!(context.eval("__order").unwrap(), Value::Int(1));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn generator_intrinsic_reciprocal_descriptors_match_quickjs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    "let GFP = Object.getPrototypeOf(function*(){}); \
                     let GF = GFP.constructor; \
                     let GP = GFP.prototype; \
                     let fixed = Object.getOwnPropertyDescriptor(GF, 'prototype'); \
                     let back = Object.getOwnPropertyDescriptor(GFP, 'constructor'); \
                     let bridge = Object.getOwnPropertyDescriptor(GFP, 'prototype'); \
                     let publicBack = Object.getOwnPropertyDescriptor(GP, 'constructor'); \
                     fixed.value === GFP && !fixed.writable && !fixed.enumerable && !fixed.configurable && \
                     back.value === GF && !back.writable && !back.enumerable && back.configurable && \
                     bridge.value === GP && !bridge.writable && !bridge.enumerable && bridge.configurable && \
                     publicBack.value === GFP && !publicBack.writable && !publicBack.enumerable && publicBack.configurable",
                )
                .unwrap(),
            Value::Bool(true)
        );
    }
}
