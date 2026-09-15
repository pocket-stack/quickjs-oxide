//! Native invocation ownership shared by synchronous consumers and owned continuations.
//! Preparing an activation executes no builtin body; arguments and the diagnostic
//! frame survive until completion, rejection or abandonment.
use super::{NativeArguments, NativeInvocation, NativeInvokeMode, NativeInvokeOutcome};
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::NativeFunctionId,
    heap::ContextId,
    object::CallableRef,
    value::Value,
    vm::{Completion, frames::ActiveFrameGuard},
};

pub(in crate::engine::vm) struct PreparedNativeCall {
    pub activation: NativeActivation,
    pub invocation: NativeInvocation,
}

pub(in crate::engine::vm) struct NativeActivation {
    pub callable: CallableRef,
    pub realm: ContextId,
    pub target: NativeFunctionId,
    pub mode: NativeInvokeMode,
    pub arguments: NativeArguments,
    active_frame: ActiveFrameGuard,
}

enum NativeCallableInput<'a> {
    Borrowed(&'a CallableRef),
    #[cfg(feature = "stack-vm")]
    Owned(CallableRef),
}

impl NativeCallableInput<'_> {
    fn as_ref(&self) -> &CallableRef {
        match self {
            Self::Borrowed(callable) => callable,
            #[cfg(feature = "stack-vm")]
            Self::Owned(callable) => callable,
        }
    }

    fn into_owned(self) -> CallableRef {
        match self {
            Self::Borrowed(callable) => callable.clone(),
            #[cfg(feature = "stack-vm")]
            Self::Owned(callable) => callable,
        }
    }
}

enum NativeArgumentInput<'a> {
    Borrowed(&'a [Value]),
    #[cfg(feature = "stack-vm")]
    Owned(Vec<Value>),
}

impl Runtime {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::engine::vm) fn prepare_native_invocation(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        invocation: NativeInvocation,
        arguments: &[Value],
        mode: NativeInvokeMode,
    ) -> Result<PreparedNativeCall, RuntimeError> {
        self.prepare_native_arguments(
            NativeCallableInput::Borrowed(callable),
            realm,
            target,
            min_readable_args,
            invocation,
            NativeArgumentInput::Borrowed(arguments),
            mode,
            false,
            #[cfg(feature = "stack-vm")]
            None,
        )
    }

    #[cfg(feature = "stack-vm")]
    #[allow(clippy::too_many_arguments)]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::engine::vm) fn prepare_native_invocation_owned(
        &self,
        callable: CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        invocation: NativeInvocation,
        arguments: Vec<Value>,
        mode: NativeInvokeMode,
    ) -> Result<PreparedNativeCall, RuntimeError> {
        self.prepare_native_arguments(
            NativeCallableInput::Owned(callable),
            realm,
            target,
            min_readable_args,
            invocation,
            NativeArgumentInput::Owned(arguments),
            mode,
            false,
            #[cfg(feature = "stack-vm")]
            None,
        )
    }

    #[cfg(feature = "stack-vm")]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::engine::vm) fn prepare_native_continuation_owned(
        &self,
        callable: CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        invocation: NativeInvocation,
        arguments: Vec<Value>,
        mode: NativeInvokeMode,
    ) -> Result<PreparedNativeCall, RuntimeError> {
        self.prepare_native_arguments(
            NativeCallableInput::Owned(callable),
            realm,
            target,
            min_readable_args,
            invocation,
            NativeArgumentInput::Owned(arguments),
            mode,
            true,
            None,
        )
    }

    /// Array iterator-next has no JavaScript arguments. Keep the same owning
    /// activation/publication ABI without entering general argv construction.
    #[cfg(feature = "stack-vm")]
    pub(in crate::engine::vm) fn prepare_array_next_owned(
        &self,
        callable: CallableRef,
        realm: ContextId,
        min_readable_args: u8,
        receiver: Value,
    ) -> Result<PreparedNativeCall, RuntimeError> {
        let target = NativeFunctionId::ArrayIteratorNext;
        let mode = NativeInvokeMode::IteratorNextRaw;
        let invocation = NativeInvocation::Call {
            this_value: receiver,
        };
        if min_readable_args != 0 {
            return self.prepare_native_continuation_owned(
                callable,
                realm,
                target,
                min_readable_args,
                invocation,
                Vec::new(),
                mode,
            );
        }
        let publication = super::super::frames::NativePublicationWitness::validate(
            self, &callable, realm, target, 0, mode,
        )?;
        let active_frame = publication.publish(0, 0, true)?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("native_activation_prepared");
        Ok(PreparedNativeCall {
            activation: NativeActivation {
                callable,
                realm,
                target,
                mode,
                arguments: NativeArguments {
                    actual_arg_count: 0,
                    readable: Vec::new(),
                },
                active_frame,
            },
            invocation,
        })
    }

    #[cfg(feature = "stack-vm")]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::engine::vm) fn prepare_native_continuation_selected(
        &self,
        callable: CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        invocation: NativeInvocation,
        arguments: Vec<Value>,
        mode: NativeInvokeMode,
        selected: Option<super::super::frames::NativeClassification>,
    ) -> Result<PreparedNativeCall, RuntimeError> {
        self.prepare_native_arguments(
            NativeCallableInput::Owned(callable),
            realm,
            target,
            min_readable_args,
            invocation,
            NativeArgumentInput::Owned(arguments),
            mode,
            true,
            selected,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_native_arguments(
        &self,
        callable_input: NativeCallableInput<'_>,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        invocation: NativeInvocation,
        arguments: NativeArgumentInput<'_>,
        mode: NativeInvokeMode,
        continuation: bool,
        #[cfg(feature = "stack-vm")] selected: Option<super::super::frames::NativeClassification>,
    ) -> Result<PreparedNativeCall, RuntimeError> {
        #[cfg(feature = "profiling")]
        let _profile_phase = crate::engine::api::profiling::PhaseTimer::start_vm("native.prepare");
        let callable = callable_input.as_ref();
        #[cfg(feature = "stack-vm")]
        let publication = match selected.as_ref() {
            Some(selected) => super::super::frames::NativePublicationWitness::from_classification(
                self,
                callable,
                realm,
                target,
                min_readable_args,
                mode,
                selected,
            )?,
            None => super::super::frames::NativePublicationWitness::validate(
                self,
                callable,
                realm,
                target,
                min_readable_args,
                mode,
            )?,
        };
        #[cfg(not(feature = "stack-vm"))]
        let publication = super::super::frames::NativePublicationWitness::validate(
            self,
            callable,
            realm,
            target,
            min_readable_args,
            mode,
        )?;

        let actual_arg_count = match &arguments {
            NativeArgumentInput::Borrowed(values) => values.len(),
            #[cfg(feature = "stack-vm")]
            NativeArgumentInput::Owned(values) => values.len(),
        };
        let available_arg_count = actual_arg_count.max(usize::from(min_readable_args));
        let (mut readable, _copied, _before) = match arguments {
            NativeArgumentInput::Borrowed(values) => {
                let mut readable = Vec::new();
                readable
                    .try_reserve_exact(available_arg_count)
                    .map_err(|_| {
                        RuntimeError::Invariant("native readable arguments allocation failed")
                    })?;
                readable.extend_from_slice(values);
                (readable, true, 0)
            }
            #[cfg(feature = "stack-vm")]
            NativeArgumentInput::Owned(mut values) => {
                let before = values.capacity();
                // All padding allocation precedes publication. Actual arity
                // and every extra argument survive this owning handoff.
                values
                    .try_reserve_exact(available_arg_count - actual_arg_count)
                    .map_err(|_| {
                        RuntimeError::Invariant("native readable arguments allocation failed")
                    })?;
                (values, false, before)
            }
        };
        readable.resize(available_arg_count, Value::Undefined);
        #[cfg(feature = "profiling")]
        {
            use crate::engine::api::profiling::{
                record_call_buffer_capacity, record_call_buffer_copies,
                record_call_buffer_initialized, record_call_buffer_moves,
                record_call_buffer_observed,
            };
            record_call_buffer_capacity(
                "native.readable",
                _before,
                readable.capacity(),
                size_of::<Value>(),
            );
            if _copied {
                record_call_buffer_copies("native.readable", &readable[..actual_arg_count]);
            } else {
                record_call_buffer_observed("native.incoming_argv", _before, size_of::<Value>());
                record_call_buffer_moves("native.readable", actual_arg_count);
            }
            record_call_buffer_initialized(
                "native.readable",
                available_arg_count - actual_arg_count,
            );
        }
        let arguments = NativeArguments {
            actual_arg_count,
            readable,
        };
        let active_frame =
            publication.publish(actual_arg_count, available_arg_count, continuation)?;

        #[cfg(all(feature = "profiling", feature = "stack-vm"))]
        crate::engine::api::profiling::record_owned_execution_event("native_activation_prepared");
        Ok(PreparedNativeCall {
            activation: NativeActivation {
                callable: callable_input.into_owned(),
                realm,
                target,
                mode,
                arguments,
                active_frame,
            },
            invocation,
        })
    }
}

impl NativeActivation {
    #[cfg(feature = "stack-vm")]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::engine::vm) fn own_continuation(&mut self) -> Result<(), RuntimeError> {
        self.active_frame.mark_native_continuation()
    }

    /// Allocate JS engine errors while this native frame and its selected realm
    /// are still visible. An existing thrown Value must not be re-materialized.
    pub(in crate::engine::vm) fn finish(
        self,
        result: Result<NativeInvokeOutcome, RuntimeError>,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        self.finish_reusing(result).0
    }

    /// Return only empty capacity after errors have been materialized and the
    /// native frame has ended. This also preserves cleanup on a failed finish.
    pub(in crate::engine::vm) fn finish_reusing(
        self,
        result: Result<NativeInvokeOutcome, RuntimeError>,
    ) -> (Result<NativeInvokeOutcome, RuntimeError>, Vec<Value>) {
        self.finish_reusing_with(result, |value| {
            NativeInvokeOutcome::Completion(Completion::Throw(value))
        })
    }

    #[cfg(feature = "stack-vm")]
    pub(in crate::engine::vm) fn finish_completion_reusing(
        self,
        result: Result<Completion, RuntimeError>,
    ) -> (Result<Completion, RuntimeError>, Vec<Value>) {
        self.finish_reusing_with(result, Completion::Throw)
    }

    fn finish_reusing_with<T>(
        self,
        result: Result<T, RuntimeError>,
        throw: impl FnOnce(Value) -> T,
    ) -> (Result<T, RuntimeError>, Vec<Value>) {
        let runtime = &self.active_frame.runtime;
        let result = (|| match result {
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind())
                    .expect("guard proved this is a JavaScript-visible native error");
                let value = runtime.new_native_error_from_error(self.realm, kind, &error)?;
                Ok(throw(value))
            }
            result => result,
        })();
        let result = self.active_frame.finish().and(result);
        // Keep the original field cleanup order: the active-frame roots and
        // callable owner are released before readable argument owners.
        drop(self.callable);
        let mut readable = self.arguments.readable;
        readable.clear();
        (result, readable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        api::{Context, Error, ErrorKind},
        vm::call::CallableExecution,
    };

    fn prepare(
        runtime: &Runtime,
        context: &mut Context,
        arguments: &[Value],
    ) -> PreparedNativeCall {
        let callable = runtime
            .callable_from_value(context.eval("Reflect.get").unwrap())
            .unwrap();
        let CallableExecution::Native {
            target,
            realm,
            min_readable_args,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("expected native")
        };
        runtime
            .prepare_native_invocation(
                &callable,
                realm,
                target,
                min_readable_args,
                NativeInvocation::Call {
                    this_value: Value::Undefined,
                },
                arguments,
                NativeInvokeMode::Ordinary,
            )
            .unwrap()
    }

    #[cfg(feature = "stack-vm")]
    #[test]
    fn borrowed_native_adaptation_shares_validation_and_preserves_input_owners() {
        use super::super::NativeInvocationAdaptation;
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for (source, construct) in [
            ("Reflect.get", false),
            ("Math.min", true),
            ("Array", false),
            ("Map", false),
            ("Map", true),
            (
                "Object.getOwnPropertyDescriptor(Map.prototype,'size').get",
                false,
            ),
            (
                "Object.getOwnPropertyDescriptor(Object.prototype,'__proto__').set",
                false,
            ),
        ] {
            let callable = runtime
                .callable_from_value(context.eval(source).unwrap())
                .unwrap();
            let CallableExecution::Native {
                target,
                realm,
                min_readable_args,
            } = runtime.bytecode_for_callable(&callable).unwrap()
            else {
                panic!("native fixture")
            };
            let input = Value::Object(runtime.new_object(None).unwrap());
            let invocation = if construct {
                NativeInvocation::Construct { new_target: input }
            } else {
                NativeInvocation::Call { this_value: input }
            };
            let prepared = runtime
                .prepare_native_invocation_owned(
                    callable,
                    realm,
                    target,
                    min_readable_args,
                    invocation,
                    Vec::new(),
                    NativeInvokeMode::Ordinary,
                )
                .unwrap();
            let borrowed = runtime
                .adapt_native_invocation_borrowed(
                    target,
                    realm,
                    &prepared.invocation,
                    &prepared.activation.arguments,
                )
                .unwrap();
            if source == "Reflect.get" {
                assert!(
                    matches!(&borrowed,NativeInvocationAdaptation::Invoke(std::borrow::Cow::Borrowed(value)) if std::ptr::eq(*value,&prepared.invocation))
                );
            }
            let owned = runtime
                .adapt_native_invocation(
                    target,
                    realm,
                    prepared.invocation.clone(),
                    &prepared.activation.arguments,
                )
                .unwrap();
            match (borrowed, owned) {
                (
                    NativeInvocationAdaptation::Invoke(borrowed),
                    NativeInvocationAdaptation::Invoke(owned),
                ) => {
                    assert_eq!(
                        std::mem::discriminant(borrowed.as_ref()),
                        std::mem::discriminant(&owned)
                    );
                    fn input(value: &NativeInvocation) -> &Value {
                        match value {
                            NativeInvocation::Call { this_value }
                            | NativeInvocation::Getter { this_value }
                            | NativeInvocation::Setter { this_value } => this_value,
                            NativeInvocation::Construct { new_target } => new_target,
                        }
                    }
                    assert_eq!(input(borrowed.as_ref()), input(&owned));
                }
                (
                    NativeInvocationAdaptation::Complete(Completion::Throw(Value::Object(a))),
                    NativeInvocationAdaptation::Complete(Completion::Throw(Value::Object(b))),
                ) => {
                    assert_eq!(
                        runtime.get_prototype_of(&a).unwrap(),
                        runtime.get_prototype_of(&b).unwrap()
                    );
                }
                _ => panic!("borrowed and owned adaptation diverged"),
            }
            let already_adapted = NativeInvocation::Getter {
                this_value: Value::Undefined,
            };
            assert!(matches!(
                runtime.adapt_native_invocation_borrowed(
                    target,
                    realm,
                    &already_adapted,
                    &prepared.activation.arguments
                ),
                Err(RuntimeError::Invariant(
                    "native invocation was adapted more than once"
                ))
            ));
            let wrong_arguments = NativeArguments {
                actual_arg_count: usize::MAX,
                readable: Vec::new(),
            };
            assert!(matches!(
                runtime.adapt_native_invocation_borrowed(
                    target,
                    realm,
                    &prepared.invocation,
                    &wrong_arguments
                ),
                Err(RuntimeError::Invariant(
                    "active native frame disagrees with handler arguments"
                ))
            ));
            prepared
                .activation
                .finish(Ok(NativeInvokeOutcome::Completion(Completion::Return(
                    Value::Undefined,
                ))))
                .unwrap();
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    #[cfg(all(feature = "stack-vm", feature = "profiling"))]
    fn native_argument_pool_reuses_nested_capacity_and_releases_all_owners() {
        let runtime = Runtime::new();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let callable = runtime
            .callable_from_value(context.eval("Reflect.get").unwrap())
            .unwrap();
        let CallableExecution::Native {
            target,
            realm,
            min_readable_args,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("expected native")
        };
        let mut slots = crate::engine::vm::stack::SlotStore::new(1024);
        let profile = crate::engine::api::profiling::CostProfile::start();
        for round in 0..20 {
            let mut pending = Vec::new();
            let mut markers = Vec::new();
            for depth in 1..=16 {
                slots.reserve_native_argument_depth(depth).unwrap();
                let mut arguments = slots.take_native_argument_buffer(4).unwrap();
                let marker = runtime.new_object(None).unwrap();
                markers.push(marker.object_id());
                arguments.push(Value::Object(marker));
                let prepared = runtime
                    .prepare_native_invocation_owned(
                        callable.clone(),
                        realm,
                        target,
                        min_readable_args,
                        NativeInvocation::Call {
                            this_value: Value::Undefined,
                        },
                        arguments,
                        NativeInvokeMode::Ordinary,
                    )
                    .unwrap();
                pending.push(prepared);
            }
            while let Some(prepared) = pending.pop() {
                let result = if round % 2 == 0 {
                    Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                        Value::Int(42),
                    )))
                } else {
                    Err(RuntimeError::Invariant("pool error path"))
                };
                let (result, empty) = prepared.activation.finish_reusing(result);
                assert!(empty.is_empty());
                assert!(empty.capacity() >= 4);
                assert_eq!(result.is_err(), round % 2 != 0);
                slots.recycle_native_argument_buffer(empty);
            }
            assert!(runtime.0.state.borrow().active_frames.is_empty());
            runtime.run_gc().unwrap();
            for marker in markers {
                assert!(runtime.0.state.borrow().heap.object(marker).is_err());
            }
        }
        let costs = profile.snapshot();
        assert_eq!(costs.call_buffers["call.native_argv"].capacity_growths, 16);
        assert_eq!(costs.call_buffers["native.readable"].capacity_growths, 0);
        assert_eq!(costs.call_buffers["native.readable"].values_copied, 0);
        drop(callable);
        drop(context);
        drop(runtime);
        assert!(weak.upgrade().is_none(), "empty pool retained Runtime");
        drop(slots);
    }

    #[test]
    #[cfg(all(feature = "stack-vm", feature = "profiling"))]
    fn owned_readable_arguments_keep_buffer_identity_arity_and_padding() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callable = runtime
            .callable_from_value(context.eval("Reflect.get").unwrap())
            .unwrap();
        let CallableExecution::Native {
            target,
            realm,
            min_readable_args,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("expected native");
        };
        let marker = runtime.new_object(None).unwrap();
        for actual in [
            vec![],
            vec![Value::Object(marker.clone())],
            vec![
                Value::Object(marker.clone()),
                Value::Int(0),
                Value::Undefined,
                Value::Object(marker.clone()),
            ],
        ] {
            let borrowed = runtime
                .prepare_native_invocation(
                    &callable,
                    realm,
                    target,
                    min_readable_args,
                    NativeInvocation::Call {
                        this_value: Value::Undefined,
                    },
                    &actual,
                    NativeInvokeMode::Ordinary,
                )
                .unwrap();
            let expected = borrowed.activation.arguments.readable.clone();
            drop(borrowed);
            let count = actual.len();
            let mut owned = Vec::with_capacity(8);
            owned.extend(actual);
            let address = owned.as_ptr();
            let profile = crate::engine::api::profiling::CostProfile::start();
            let prepared = runtime
                .prepare_native_invocation_owned(
                    callable.clone(),
                    realm,
                    target,
                    min_readable_args,
                    NativeInvocation::Call {
                        this_value: Value::Undefined,
                    },
                    owned,
                    NativeInvokeMode::Ordinary,
                )
                .unwrap();
            assert_eq!(prepared.activation.arguments.actual_arg_count, count);
            assert_eq!(prepared.activation.arguments.readable, expected);
            assert_eq!(prepared.activation.arguments.readable.as_ptr(), address);
            let costs = profile.snapshot();
            let buffer = &costs.call_buffers["native.readable"];
            assert_eq!(buffer.capacity_growths, 0);
            assert_eq!(buffer.values_copied, 0);
            assert_eq!(buffer.heap_root_copies, 0);
            assert_eq!(buffer.values_moved, count as u64);
            assert_eq!(buffer.slots_initialized, expected.len() as u64);
            let result = prepared
                .activation
                .finish(Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                    Value::Int(42),
                ))))
                .unwrap();
            assert!(matches!(
                result,
                NativeInvokeOutcome::Completion(Completion::Throw(Value::Int(42)))
            ));
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    #[cfg(feature = "stack-vm")]
    fn owning_preparation_keeps_rejection_order_and_argument_domain_errors() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callable = runtime
            .callable_from_value(context.eval("Reflect.get").unwrap())
            .unwrap();
        let CallableExecution::Native {
            target,
            realm,
            min_readable_args,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("expected native");
        };
        let foreign = Runtime::new();
        let rejected = foreign.prepare_native_invocation_owned(
            callable.clone(),
            realm,
            target,
            min_readable_args + 1,
            NativeInvocation::Call {
                this_value: Value::Undefined,
            },
            vec![],
            NativeInvokeMode::Ordinary,
        );
        assert!(matches!(
            rejected,
            Err(RuntimeError::WrongRuntime("native callable"))
        ));
        assert!(foreign.0.state.borrow().active_frames.is_empty());
        let rejected = runtime.prepare_native_invocation_owned(
            callable.clone(),
            realm,
            target,
            min_readable_args + 1,
            NativeInvocation::Call {
                this_value: Value::Undefined,
            },
            vec![],
            NativeInvokeMode::Ordinary,
        );
        assert!(matches!(
            rejected,
            Err(RuntimeError::Invariant(
                "native invocation metadata changed after snapshot"
            ))
        ));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        let arguments = vec![
            Value::Object(foreign.new_object(None).unwrap()),
            Value::Int(0),
        ];
        let mut errors = Vec::new();
        for owned in [false, true] {
            let prepared = if owned {
                runtime.prepare_native_invocation_owned(
                    callable.clone(),
                    realm,
                    target,
                    min_readable_args,
                    NativeInvocation::Call {
                        this_value: Value::Undefined,
                    },
                    arguments.clone(),
                    NativeInvokeMode::Ordinary,
                )
            } else {
                runtime.prepare_native_invocation(
                    &callable,
                    realm,
                    target,
                    min_readable_args,
                    NativeInvocation::Call {
                        this_value: Value::Undefined,
                    },
                    &arguments,
                    NativeInvokeMode::Ordinary,
                )
            }
            .unwrap();
            let result = runtime
                .dispatch_native_function(
                    &prepared.activation.callable,
                    target,
                    realm,
                    prepared.invocation,
                    &prepared.activation.arguments,
                )
                .map(NativeInvokeOutcome::Completion);
            let error = match prepared.activation.finish(result) {
                Err(error) => error,
                Ok(_) => panic!("foreign argument accepted"),
            };
            errors.push(format!("{error:?}"));
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
        assert_eq!(errors[0], errors[1]);
    }

    #[test]
    #[cfg(feature = "profiling")]
    fn readable_buffer_ledger_separates_padding_from_copied_roots() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let marker = runtime.new_object(None).unwrap();
        let profile = crate::engine::api::profiling::CostProfile::start();
        let native = prepare(
            &runtime,
            &mut context,
            &[Value::Object(marker), Value::Int(0)],
        );
        let cost = profile.snapshot();
        let buffer = &cost.call_buffers["native.readable"];
        assert_eq!(buffer.capacity_growths, 1);
        assert_eq!(buffer.values_copied, 2);
        assert_eq!(buffer.heap_root_copies, 1);
        assert_eq!(buffer.immediate_copies, 1);
        assert_eq!(buffer.primitive_rc_copies, 0);
        assert_eq!(
            buffer.slots_initialized,
            native.activation.arguments.readable.len() as u64
        );
        assert_eq!(cost.vm_phases["native.prepare"].cost.attempts, 1);
        assert_eq!(cost.vm_phases["native.prepare"].samples_ns.len(), 1);
    }

    #[test]
    fn prepared_native_activation_owns_all_arguments_without_invoking_the_body() {
        let runtime = Runtime::new();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let target = context
            .eval("var calls=0;new Proxy({}, {get(){calls++;return 1}})")
            .unwrap();
        let extra = runtime.new_object(None).unwrap();
        let extra_id = extra.object_id();
        let native = prepare(
            &runtime,
            &mut context,
            &[
                target,
                Value::Int(0),
                Value::Undefined,
                Value::Object(extra),
            ],
        );
        assert_eq!(native.activation.arguments.actual_arg_count, 4);
        assert_eq!(native.activation.arguments.readable.len(), 4);
        assert_eq!(runtime.0.state.borrow().active_frames.len(), 1);
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(extra_id).is_ok());
        assert_eq!(context.eval("calls").unwrap(), Value::Int(0));
        drop(native);
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(extra_id).is_err());
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        drop(context);
        drop(runtime);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn prepared_native_padding_and_unwind_restore_the_frame() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let native = prepare(&runtime, &mut context, &[]);
        assert_eq!(native.activation.arguments.actual_arg_count, 0);
        assert!(!native.activation.arguments.readable.is_empty());
        assert!(
            native
                .activation
                .arguments
                .readable
                .iter()
                .all(|value| *value == Value::Undefined)
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _native = native;
            panic!("abandon native activation");
        }));
        assert!(result.is_err());
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn native_activation_materializes_errors_before_leaving_its_defining_realm() {
        let runtime = Runtime::new();
        let mut caller = runtime.new_context();
        let mut defining = runtime.new_context();
        let expected_prototype = defining.eval("TypeError.prototype").unwrap();
        let native = prepare(&runtime, &mut defining, &[]);
        drop(defining);
        let result = native
            .activation
            .finish(Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "activation failure",
            ))))
            .unwrap();
        let NativeInvokeOutcome::Completion(Completion::Throw(Value::Object(error))) = result
        else {
            panic!("expected TypeError")
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap().map(Value::Object),
            Some(expected_prototype)
        );
        let stack = caller
            .get_property(&error, &runtime.intern_property_key("stack").unwrap())
            .unwrap();
        let Value::String(stack) = stack else {
            panic!("expected captured stack")
        };
        assert!(stack.to_string().contains("get (native)"), "{stack:?}");
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        let sentinel = runtime.new_object(None).unwrap();
        let native = prepare(&runtime, &mut caller, &[]);
        let result = native
            .activation
            .finish(Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                Value::Object(sentinel.clone()),
            ))))
            .unwrap();
        assert!(
            matches!(result, NativeInvokeOutcome::Completion(Completion::Throw(Value::Object(value))) if value == sentinel)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn native_activation_rejects_foreign_and_changed_metadata_before_registration() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callable = runtime
            .callable_from_value(context.eval("Reflect.get").unwrap())
            .unwrap();
        let CallableExecution::Native {
            target,
            realm,
            min_readable_args,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("expected native")
        };
        let wrong_runtime = Runtime::new();
        let result = wrong_runtime.prepare_native_invocation(
            &callable,
            realm,
            target,
            min_readable_args,
            NativeInvocation::Call {
                this_value: Value::Undefined,
            },
            &[],
            NativeInvokeMode::Ordinary,
        );
        assert!(matches!(result, Err(RuntimeError::WrongRuntime(_))));
        assert!(wrong_runtime.0.state.borrow().active_frames.is_empty());
        let result = runtime.prepare_native_invocation(
            &callable,
            realm,
            target,
            min_readable_args + 1,
            NativeInvocation::Call {
                this_value: Value::Undefined,
            },
            &[],
            NativeInvokeMode::Ordinary,
        );
        assert!(matches!(
            result,
            Err(RuntimeError::Invariant(
                "native invocation metadata changed after snapshot"
            ))
        ));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}

#[cfg(all(test, feature = "stack-vm"))]
mod continuation_publication_tests {
    use super::*;
    use crate::engine::vm::call::CallableExecution;

    #[test]
    fn native_continuation_publication_preserves_abi_hidden_flags_and_single_registration() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for (source, mode) in [
            ("Map.prototype.set", NativeInvokeMode::Ordinary),
            ("[].values().next", NativeInvokeMode::IteratorNextRaw),
        ] {
            let callable = runtime
                .callable_from_value(context.eval(source).unwrap())
                .unwrap();
            let CallableExecution::Native {
                target,
                realm,
                min_readable_args,
            } = runtime.bytecode_for_callable(&callable).unwrap()
            else {
                panic!("native");
            };
            let mut call = runtime
                .prepare_native_continuation_owned(
                    callable,
                    realm,
                    target,
                    min_readable_args,
                    NativeInvocation::Call {
                        this_value: Value::Undefined,
                    },
                    vec![Value::Int(7)],
                    mode,
                )
                .unwrap();
            {
                let state = runtime.0.state.borrow();
                let frame = state.active_frames.last().unwrap();
                assert!(frame.native_continuation);
                assert_eq!(
                    frame.flags.backtrace_hidden,
                    matches!(mode, NativeInvokeMode::IteratorNextRaw)
                );
                assert_eq!(frame.realm, realm);
            }
            assert_eq!(call.activation.arguments.actual_arg_count, 1);
            assert_eq!(
                call.activation.arguments.readable.len(),
                1usize.max(usize::from(min_readable_args))
            );
            assert!(
                runtime
                    .adapt_native_invocation_borrowed(
                        target,
                        realm,
                        &call.invocation,
                        &call.activation.arguments
                    )
                    .is_ok()
            );
            assert!(matches!(
                call.activation.own_continuation(),
                Err(RuntimeError::Invariant(
                    "native continuation was registered twice"
                ))
            ));
            drop(call);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn native_continuation_publication_rejects_stale_metadata_before_publishing() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callable = runtime
            .callable_from_value(context.eval("Math.min").unwrap())
            .unwrap();
        let CallableExecution::Native {
            target,
            realm,
            min_readable_args,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("native");
        };
        let token = runtime.0.state.borrow().next_active_frame_token;
        let result = runtime.prepare_native_continuation_owned(
            callable.clone(),
            realm,
            target,
            min_readable_args.saturating_add(1),
            NativeInvocation::Call {
                this_value: Value::Undefined,
            },
            Vec::new(),
            NativeInvokeMode::Ordinary,
        );
        assert!(matches!(
            result,
            Err(RuntimeError::Invariant(
                "native invocation metadata changed after snapshot"
            ))
        ));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        assert_eq!(runtime.0.state.borrow().next_active_frame_token, token);
        let mut legacy = runtime
            .prepare_native_invocation_owned(
                callable,
                realm,
                target,
                min_readable_args,
                NativeInvocation::Call {
                    this_value: Value::Undefined,
                },
                Vec::new(),
                NativeInvokeMode::Ordinary,
            )
            .unwrap();
        assert!(
            !runtime
                .0
                .state
                .borrow()
                .active_frames
                .last()
                .unwrap()
                .native_continuation
        );
        legacy.activation.own_continuation().unwrap();
        assert!(
            runtime
                .0
                .state
                .borrow()
                .active_frames
                .last()
                .unwrap()
                .native_continuation
        );
        drop(legacy);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}

#[cfg(test)]
mod publication_witness_tests {
    use super::*;
    use crate::engine::vm::call::CallableExecution;
    use crate::engine::vm::frames::{ActiveFrameKind, NativePublicationWitness};

    #[test]
    fn publication_witness_matches_checked_registration_and_keeps_count_guard() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for source in [
            "Map.prototype.set",
            "Math.min",
            "Reflect.get",
            "Array.prototype.push",
        ] {
            let callable = runtime
                .callable_from_value(context.eval(source).unwrap())
                .unwrap();
            let CallableExecution::Native {
                target,
                realm,
                min_readable_args,
            } = runtime.bytecode_for_callable(&callable).unwrap()
            else {
                panic!("native")
            };
            let count = usize::from(min_readable_args);
            let checked = runtime
                .push_native_active_frame(callable.as_object().clone(), realm, target, 0, count)
                .unwrap();
            let original = *runtime.0.state.borrow().active_frames.last().unwrap();
            checked.finish().unwrap();
            let witness = NativePublicationWitness::validate(
                &runtime,
                &callable,
                realm,
                target,
                min_readable_args,
                NativeInvokeMode::Ordinary,
            )
            .unwrap();
            let published = witness.publish(0, count, false).unwrap();
            let current = *runtime.0.state.borrow().active_frames.last().unwrap();
            assert_eq!(original.function, current.function);
            assert_eq!(original.realm, current.realm);
            assert_eq!(original.native_continuation, current.native_continuation);
            assert!(
                matches!(current.kind, ActiveFrameKind::Native { target: actual_target, actual_arg_count: 0, readable_arg_count } if actual_target == target && readable_arg_count == count)
            );
            published.finish().unwrap();
            let token = runtime.0.state.borrow().next_active_frame_token;
            let witness = NativePublicationWitness::validate(
                &runtime,
                &callable,
                realm,
                target,
                min_readable_args,
                NativeInvokeMode::Ordinary,
            )
            .unwrap();
            assert!(matches!(
                witness.publish(0, count + 1, false),
                Err(RuntimeError::Invariant(
                    "native active frame disagrees with its rooted callable"
                ))
            ));
            assert!(runtime.0.state.borrow().active_frames.is_empty());
            assert_eq!(runtime.0.state.borrow().next_active_frame_token, token);
        }
    }

    #[test]
    fn publication_witness_preserves_foreign_realm_metadata_and_token_error_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let other_context = runtime.new_context();
        let callable = runtime
            .callable_from_value(context.eval("Reflect.get").unwrap())
            .unwrap();
        let CallableExecution::Native {
            target,
            realm,
            min_readable_args,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("native")
        };
        let foreign = Runtime::new();
        assert!(matches!(
            NativePublicationWitness::validate(
                &foreign,
                &callable,
                other_context.realm,
                target,
                min_readable_args + 1,
                NativeInvokeMode::Ordinary
            ),
            Err(RuntimeError::WrongRuntime("native callable"))
        ));
        let token = runtime.0.state.borrow().next_active_frame_token;
        for (selected_realm, minimum) in [
            (other_context.realm, min_readable_args),
            (realm, min_readable_args + 1),
        ] {
            assert!(matches!(
                NativePublicationWitness::validate(
                    &runtime,
                    &callable,
                    selected_realm,
                    target,
                    minimum,
                    NativeInvokeMode::Ordinary
                ),
                Err(RuntimeError::Invariant(
                    "native invocation metadata changed after snapshot"
                ))
            ));
            assert!(runtime.0.state.borrow().active_frames.is_empty());
            assert_eq!(runtime.0.state.borrow().next_active_frame_token, token);
        }
        runtime.0.state.borrow_mut().next_active_frame_token = u64::MAX;
        let result = runtime.prepare_native_invocation(
            &callable,
            realm,
            target,
            min_readable_args,
            NativeInvocation::Call {
                this_value: Value::Undefined,
            },
            &[],
            NativeInvokeMode::Ordinary,
        );
        assert!(matches!(
            result,
            Err(RuntimeError::Invariant(
                "active-frame token space was exhausted"
            ))
        ));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        assert_eq!(runtime.0.state.borrow().next_active_frame_token, u64::MAX);
        runtime.0.state.borrow_mut().next_active_frame_token = token;
    }

    #[test]
    fn publication_witness_keeps_native_reentry_throw_and_following_result() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for source in [
            "(function(){var m=new Map(),n=0,o={valueOf(){n++;m.set('x',41);return m.get('x')}};return Math.min(o,99)+n})()",
            "(function(){var marker={},m=new Map(),n=0;try{Math.min({valueOf(){n++;m.set('x',41);throw marker}},0);return 0}catch(e){return e===marker&&n===1?m.get('x')+1:0}})()",
            "(function(){var n=0,a={get length(){n++;return 0},set length(v){}};Array.prototype.push.call(a,41);return a[0]+n})()",
        ] {
            assert_eq!(context.eval(source).unwrap(), Value::Int(42));
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }
}

#[cfg(all(test, feature = "stack-vm"))]
mod classified_preparation_tests {
    use super::*;
    use crate::engine::vm::{call::CallableExecution, frames::NativeClassification};

    #[test]
    fn selected_native_payload_is_bound_to_its_call_owner() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callable = runtime
            .callable_from_value(context.eval("Math.min").unwrap())
            .unwrap();
        let CallableExecution::Native {
            target,
            realm,
            min_readable_args,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("native")
        };
        let selection = NativeClassification::select(&runtime, &callable)
            .unwrap()
            .unwrap();
        let prepared = runtime
            .prepare_native_continuation_selected(
                callable.clone(),
                realm,
                target,
                min_readable_args,
                NativeInvocation::Call {
                    this_value: Value::Undefined,
                },
                vec![Value::Int(3), Value::Int(2)],
                NativeInvokeMode::Ordinary,
                Some(selection),
            )
            .unwrap();
        assert_eq!(prepared.activation.arguments.actual_arg_count, 2);
        prepared
            .activation
            .finish(Ok(NativeInvokeOutcome::Completion(Completion::Return(
                Value::Int(2),
            ))))
            .unwrap();
        let selection = NativeClassification::select(&runtime, &callable)
            .unwrap()
            .unwrap();
        let other = runtime
            .callable_from_value(context.eval("Math.max").unwrap())
            .unwrap();
        assert!(matches!(
            runtime.prepare_native_continuation_selected(
                other,
                realm,
                target,
                min_readable_args,
                NativeInvocation::Call {
                    this_value: Value::Undefined
                },
                vec![],
                NativeInvokeMode::Ordinary,
                Some(selection)
            ),
            Err(RuntimeError::Invariant(
                "native invocation metadata changed after snapshot"
            ))
        ));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}
