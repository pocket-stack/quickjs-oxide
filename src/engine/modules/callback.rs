//! Module completion reactions request child bodies and selected Promise settlers.
use super::{ModuleBytecodeRef, ModuleEvaluationState, body::BodyStep};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::builtins::native::{
    DynamicImportHandlerKind, ModuleEvaluationKind, NativeFunctionId,
};
use crate::engine::heap::{
    ContextId, InternalCallableData, ModuleId, RawModuleRef, RawModuleTransition, RawValue,
};
use crate::engine::object::CallableRef;
use crate::engine::value::Value;
use crate::engine::vm::{
    Completion,
    call::{NativeArguments, NativeInvocation},
};
use std::collections::VecDeque;

pub(crate) enum CallbackStep {
    Complete(Completion),
    Call {
        callable: CallableRef,
        value: Value,
        resume: Box<CallbackResume>,
    },
    Body {
        step: Box<BodyStep>,
        resume: Box<CallbackResume>,
    },
    Nested {
        step: Box<CallbackStep>,
        resume: Box<CallbackResume>,
    },
}
enum FulfillPhase {
    RootSettled,
    Iterate,
    Body {
        module: RawModuleRef,
        asynchronous: bool,
    },
}
enum Mode {
    Fulfill {
        ready: VecDeque<RawModuleRef>,
        phase: FulfillPhase,
    },
    Reject {
        reason: Value,
        raw: RawValue,
        pending: Vec<ModuleId>,
        parents: Vec<ModuleId>,
    },
    DynamicSettled,
}
pub(crate) struct CallbackResume {
    runtime: Runtime,
    realm: ContextId,
    root: ModuleBytecodeRef,
    mode: Mode,
}
impl CallbackStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        target: NativeFunctionId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        match target {
            NativeFunctionId::ModuleEvaluation(kind) => {
                Self::module(runtime, realm, kind, invocation, arguments)
            }
            NativeFunctionId::DynamicImportHandler(kind) => {
                Self::dynamic(runtime, realm, kind, invocation, arguments)
            }
            _ => Err(RuntimeError::Invariant("unregistered module callback")),
        }
    }
    fn module(
        runtime: &Runtime,
        realm: ContextId,
        target_kind: ModuleEvaluationKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "module evaluation callback received a constructor invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "module evaluation callback argv was not padded",
            ))?;
        let active = runtime.active_function()?;
        let internal = runtime
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "module evaluation callback has no internal state",
            ))?;
        let InternalCallableData::ModuleEvaluation { module, kind } = internal else {
            return Err(RuntimeError::Invariant(
                "module evaluation callback has the wrong internal state",
            ));
        };
        if kind != target_kind {
            return Err(RuntimeError::Invariant(
                "module evaluation callback target disagrees with its capture",
            ));
        }
        match target_kind {
            ModuleEvaluationKind::Fulfill => Self::fulfill(runtime, realm, module),
            ModuleEvaluationKind::Reject => Self::reject(runtime, realm, module, argument),
        }
    }
    fn dynamic(
        runtime: &Runtime,
        realm: ContextId,
        target_kind: DynamicImportHandlerKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "dynamic import handler received a constructor invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "dynamic import handler argv was not padded",
            ))?;
        let active = runtime.active_function()?;
        let internal = runtime
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "dynamic import handler has no internal state",
            ))?;
        let InternalCallableData::DynamicImportHandler {
            module,
            resolve,
            reject,
            kind,
        } = internal
        else {
            return Err(RuntimeError::Invariant(
                "dynamic import handler has the wrong internal state",
            ));
        };
        if kind != target_kind || module.cache != realm {
            return Err(RuntimeError::Invariant(
                "dynamic import handler target disagrees with its capture",
            ));
        }

        let (target, value) = match target_kind {
            DynamicImportHandlerKind::Reject => (reject, argument),
            DynamicImportHandlerKind::Fulfill => {
                match runtime.get_module_namespace_raw(module, realm) {
                    Ok(namespace) => (resolve, Value::Object(namespace)),
                    Err(error) => (reject, runtime.dynamic_import_error_reason(realm, error)?),
                }
            }
        };
        let callable = runtime.dynamic_import_settler(target)?;
        Ok(Self::Call {
            callable,
            value,
            resume: Box::new(CallbackResume {
                runtime: runtime.clone(),
                realm,
                root: runtime.root_module(module)?,
                mode: Mode::DynamicSettled,
            }),
        })
    }
    fn fulfill(
        runtime: &Runtime,
        realm: ContextId,
        module: RawModuleRef,
    ) -> Result<Self, RuntimeError> {
        match runtime.module_record(module)?.evaluation {
            ModuleEvaluationState::Errored(_) => {
                return Ok(Self::Complete(Completion::Return(Value::Undefined)));
            }
            ModuleEvaluationState::EvaluatingAsync => {}
            _ => {
                return Err(RuntimeError::Invariant(
                    "async module fulfillment reached an inactive module",
                ));
            }
        }
        let resume = Box::new(CallbackResume {
            runtime: runtime.clone(),
            realm,
            root: runtime.root_module(module)?,
            mode: Mode::Fulfill {
                ready: VecDeque::new(),
                phase: FulfillPhase::RootSettled,
            },
        });
        runtime.transition_module_record(module, RawModuleTransition::FinishAsyncEvaluation)?;
        if let Some(callable) =
            runtime.module_evaluation_settler(module, ModuleEvaluationKind::Fulfill)?
        {
            return Ok(Self::Call {
                callable,
                value: Value::Undefined,
                resume,
            });
        }
        resume.resume(Completion::Return(Value::Undefined))
    }
    fn reject(
        runtime: &Runtime,
        realm: ContextId,
        module: RawModuleRef,
        reason: Value,
    ) -> Result<Self, RuntimeError> {
        runtime.validate_value_domain(&reason, "async module rejection")?;
        let raw = runtime.raw_property_value(&reason)?;
        Box::new(CallbackResume {
            runtime: runtime.clone(),
            realm,
            root: runtime.root_module(module)?,
            mode: Mode::Reject {
                reason,
                raw,
                pending: vec![module.module],
                parents: Vec::new(),
            },
        })
        .advance()
    }
    pub(super) fn finish(
        self,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<Completion, RuntimeError> {
        #[cfg(feature = "stack-vm")]
        {
            crate::engine::vm::execute_root(
                runtime.clone(),
                realm,
                crate::engine::vm::RootOperation::ModuleCallback(self),
            )
            .map_err(RuntimeError::Engine)
        }
        #[cfg(not(feature = "stack-vm"))]
        {
            let mut step = self;
            let mut parents = Vec::new();
            loop {
                step = match step {
                    Self::Complete(result) => match parents.pop() {
                        Some(resume) => {
                            let resume: Box<CallbackResume> = resume;
                            resume.resume(result)?
                        }
                        None => return Ok(result),
                    },
                    Self::Call {
                        callable,
                        value,
                        resume,
                    } => resume.resume(runtime.call_internal(
                        realm,
                        &callable,
                        Value::Undefined,
                        &[value],
                    )?)?,
                    Self::Body { step, resume } => resume.resume(step.finish(runtime, realm)?)?,
                    Self::Nested { step, resume } => {
                        parents.push(resume);
                        *step
                    }
                };
            }
        }
    }
}
impl CallbackResume {
    pub(crate) fn resume(
        mut self: Box<Self>,
        completion: Completion,
    ) -> Result<CallbackStep, RuntimeError> {
        match &mut self.mode {
            Mode::DynamicSettled => {
                return match completion {
                    Completion::Return(_) => {
                        Ok(CallbackStep::Complete(Completion::Return(Value::Undefined)))
                    }
                    Completion::Throw(_) => Err(RuntimeError::Invariant(
                        "intrinsic dynamic import resolving function threw",
                    )),
                };
            }
            Mode::Reject {
                pending, parents, ..
            } => pending.extend(std::mem::take(parents).into_iter().rev()),
            Mode::Fulfill { ready, phase } => match std::mem::replace(phase, FulfillPhase::Iterate)
            {
                FulfillPhase::RootSettled => {
                    *ready = self
                        .runtime
                        .gather_available_module_ancestors(self.root.raw)?
                        .into()
                }
                FulfillPhase::Iterate => {} // selected settlement calls deliberately ignore their completion
                FulfillPhase::Body {
                    module,
                    asynchronous,
                } => {
                    if asynchronous {
                        return self.advance();
                    }
                    match completion {
                        Completion::Return(Value::Undefined) => {
                            self.runtime.transition_module_record(
                                module,
                                RawModuleTransition::FinishAsyncEvaluation,
                            )?;
                            if let Some(callable) = self
                                .runtime
                                .module_evaluation_settler(module, ModuleEvaluationKind::Fulfill)?
                            {
                                return Ok(CallbackStep::Call {
                                    callable,
                                    value: Value::Undefined,
                                    resume: self,
                                });
                            }
                        }
                        Completion::Throw(reason) => {
                            let step =
                                CallbackStep::reject(&self.runtime, self.realm, module, reason)?;
                            return Ok(CallbackStep::Nested {
                                step: Box::new(step),
                                resume: self,
                            });
                        }
                        Completion::Return(_) => {
                            return Err(RuntimeError::Invariant(
                                "module evaluation returned a non-undefined value",
                            ));
                        }
                    }
                }
            },
        }
        self.advance()
    }
    fn advance(mut self: Box<Self>) -> Result<CallbackStep, RuntimeError> {
        match &mut self.mode {
            Mode::DynamicSettled => Err(RuntimeError::Invariant(
                "dynamic import settlement was advanced twice",
            )),
            Mode::Fulfill { ready, phase } => {
                while let Some(module) = ready.pop_front() {
                    let record = self.runtime.module_record(module)?;
                    if matches!(record.evaluation, ModuleEvaluationState::Errored(_)) {
                        continue;
                    }
                    let asynchronous = record.has_top_level_await;
                    let step = BodyStep::start(&self.runtime, self.realm, module, asynchronous)?;
                    *phase = FulfillPhase::Body {
                        module,
                        asynchronous,
                    };
                    return Ok(CallbackStep::Body {
                        step: Box::new(step),
                        resume: self,
                    });
                }
                Ok(CallbackStep::Complete(Completion::Return(Value::Undefined)))
            }
            Mode::Reject {
                reason,
                raw,
                pending,
                parents,
            } => {
                while let Some(id) = pending.pop() {
                    let current = RawModuleRef {
                        cache: self.root.raw.cache,
                        module: id,
                    };
                    let record = self.runtime.module_record(current)?;
                    match record.evaluation {
                        ModuleEvaluationState::Errored(_) => continue,
                        ModuleEvaluationState::EvaluatingAsync => {}
                        _ => {
                            return Err(RuntimeError::Invariant(
                                "async module rejection reached an inactive ancestor",
                            ));
                        }
                    }
                    let next_parents = record.async_parent_modules;
                    let mut state = self.runtime.0.state.borrow_mut();
                    let retained_atoms = match raw {
                        RawValue::Symbol(atom) => {
                            Runtime::retain_module_atoms(&mut state, vec![*atom])?
                        }
                        _ => Vec::new(),
                    };
                    if let Err(error) = state
                        .heap
                        .publish_loaded_module_async_error(current, raw.clone())
                    {
                        state.release_atoms(retained_atoms)?;
                        return Err(error.into());
                    }
                    drop(state);
                    // Publish this node, settle it, then visit parents in reference order.
                    if let Some(callable) = self
                        .runtime
                        .module_evaluation_settler(current, ModuleEvaluationKind::Reject)?
                    {
                        *parents = next_parents;
                        return Ok(CallbackStep::Call {
                            callable,
                            value: reason.clone(),
                            resume: self,
                        });
                    }
                    pending.extend(next_parents.into_iter().rev());
                }
                Ok(CallbackStep::Complete(Completion::Return(Value::Undefined)))
            }
        }
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<CallbackStep>() <= 64);
