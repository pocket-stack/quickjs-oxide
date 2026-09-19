//! Module evaluation retains its DFS/SCC state while body and settlement requests run.
use super::body::BodyStep;
use super::{
    ModuleBytecodeRef, ModuleDfsFrame, ModuleEvaluationDfs, ModuleEvaluationState,
    ModuleEvaluationVisit, ModuleLinkStatus,
};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::builtins::promise::RootedPromiseCapability;
use crate::engine::heap::{ContextId, RawModuleRef, RawModuleTransition};
use crate::engine::object::{CallableRef, ObjectRef};
use crate::engine::value::Value;
use crate::engine::vm::Completion;

pub(crate) enum EvaluationStep {
    Complete(Completion),
    Body {
        step: Box<BodyStep>,
        resume: Box<EvaluationResume>,
    },
    Call {
        callable: CallableRef,
        value: Value,
        resume: Box<EvaluationResume>,
    },
}
enum GraphProgress {
    Done,
    Body(BodyStep),
}
pub(crate) struct EvaluationResume {
    runtime: Runtime,
    root: ModuleBytecodeRef,
    realm: ContextId,
    capability: RootedPromiseCapability,
    dfs: ModuleEvaluationDfs,
    frames: Vec<ModuleDfsFrame>,
    pending: Option<ModuleDfsFrame>,
    armed: bool,
    settling: bool,
}
impl EvaluationStep {
    pub(crate) fn start(
        runtime: &Runtime,
        requested_module: RawModuleRef,
        initiating_realm: ContextId,
    ) -> Result<Self, RuntimeError> {
        let requested_record = runtime.module_record(requested_module)?;
        let module =
            match requested_record.evaluation {
                ModuleEvaluationState::EvaluatingAsync
                | ModuleEvaluationState::Evaluated
                | ModuleEvaluationState::Errored(_) => RawModuleRef {
                    cache: requested_module.cache,
                    module: requested_record.evaluation_cycle_root.ok_or(
                        RuntimeError::Invariant("completed module evaluation has no cycle root"),
                    )?,
                },
                ModuleEvaluationState::Unevaluated => requested_module,
                ModuleEvaluationState::Evaluating => {
                    return Err(RuntimeError::Invariant(
                        "module evaluation Promise was requested during evaluation",
                    ));
                }
                ModuleEvaluationState::Poisoned => {
                    return Err(RuntimeError::Invariant(
                        "module evaluation previously failed inside the engine",
                    ));
                }
            };
        let record = runtime.module_record(module)?;
        if let Some(promise) = record.evaluation_promise {
            return match record.evaluation {
                ModuleEvaluationState::EvaluatingAsync
                | ModuleEvaluationState::Evaluated
                | ModuleEvaluationState::Errored(_) => {
                    ObjectRef::from_borrowed_handle(runtime.clone(), promise)
                        .map(|promise| Self::Complete(Completion::Return(Value::Object(promise))))
                        .map_err(Into::into)
                }
                ModuleEvaluationState::Unevaluated => Err(RuntimeError::Invariant(
                    "module retained an unsettled Promise before evaluation",
                )),
                ModuleEvaluationState::Evaluating => Err(RuntimeError::Invariant(
                    "module cycle-root Promise was requested during evaluation",
                )),
                ModuleEvaluationState::Poisoned => Err(RuntimeError::Invariant(
                    "module cycle-root evaluation previously failed inside the engine",
                )),
            };
        }
        if matches!(record.evaluation, ModuleEvaluationState::Evaluating) {
            return Err(RuntimeError::Invariant(
                "module cycle-root Promise was requested during evaluation",
            ));
        }
        if matches!(record.evaluation, ModuleEvaluationState::Poisoned) {
            return Err(RuntimeError::Invariant(
                "module cycle-root evaluation previously failed inside the engine",
            ));
        }
        if !matches!(record.link_status, ModuleLinkStatus::Linked) {
            return Err(RuntimeError::Invariant(
                "module evaluation Promise was requested before linking",
            ));
        }
        let capability = runtime.new_default_promise_capability(initiating_realm)?;
        let promise = capability.promise.clone();
        runtime
            .0
            .state
            .borrow_mut()
            .heap
            .publish_loaded_module_evaluation_capability(
                module,
                promise.object_id(),
                capability.resolve.as_object().object_id(),
                capability.reject.as_object().object_id(),
            )?;

        let root = runtime.root_module(module)?;
        let mut resume = Box::new(EvaluationResume {
            runtime: runtime.clone(),
            root,
            realm: initiating_realm,
            capability,
            dfs: ModuleEvaluationDfs::new(),
            frames: Vec::new(),
            pending: None,
            armed: false,
            settling: false,
        });
        match record.evaluation {
            ModuleEvaluationState::Unevaluated => {
                resume.armed = true;
                resume
                    .frames
                    .push(runtime.enter_module_evaluation_dfs(module, &mut resume.dfs)?);
                resume.advance()
            }
            ModuleEvaluationState::Evaluated => resume.settle(true, Value::Undefined),
            ModuleEvaluationState::Errored(reason) => {
                resume.settle(false, runtime.root_raw_value(&reason)?)
            }
            ModuleEvaluationState::EvaluatingAsync => {
                Ok(Self::Complete(Completion::Return(Value::Object(promise))))
            }
            ModuleEvaluationState::Evaluating => Err(RuntimeError::Invariant(
                "module evaluation Promise was requested during evaluation",
            )),
            ModuleEvaluationState::Poisoned => Err(RuntimeError::Invariant(
                "module evaluation previously failed inside the engine",
            )),
        }
    }
    pub(super) fn finish(
        self,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        let completion = crate::engine::vm::execute_root(
            runtime.clone(),
            realm,
            crate::engine::vm::RootOperation::ModuleEvaluation(self),
        )
        .map_err(RuntimeError::Engine)?;

        match completion {
            Completion::Return(Value::Object(promise)) => Ok(promise),
            _ => Err(RuntimeError::Invariant(
                "module evaluation did not return a Promise",
            )),
        }
    }
}
impl EvaluationResume {
    fn settle(
        mut self: Box<Self>,
        success: bool,
        value: Value,
    ) -> Result<EvaluationStep, RuntimeError> {
        self.armed = false;
        self.settling = true;
        let callable = if success {
            self.capability.resolve.clone()
        } else {
            self.capability.reject.clone()
        };
        Ok(EvaluationStep::Call {
            callable,
            value,
            resume: self,
        })
    }
    pub(crate) fn resume(
        mut self: Box<Self>,
        completion: Completion,
    ) -> Result<EvaluationStep, RuntimeError> {
        if self.settling {
            return match completion {
                Completion::Return(_) => Ok(EvaluationStep::Complete(Completion::Return(
                    Value::Object(self.capability.promise.clone()),
                ))),
                Completion::Throw(_) => Err(RuntimeError::Invariant(
                    "intrinsic module Promise resolving function threw",
                )),
            };
        }
        let frame = self.pending.take().ok_or(RuntimeError::Invariant(
            "module evaluation has no pending body",
        ))?;
        let result = Self::finish_frame(
            &self.runtime,
            &mut self.dfs,
            &mut self.frames,
            frame,
            completion,
        );
        if let Err(error) = result {
            return self.fail(error);
        }
        self.advance()
    }
    fn fail(mut self: Box<Self>, error: RuntimeError) -> Result<EvaluationStep, RuntimeError> {
        if matches!(error, RuntimeError::Exception) {
            let exception = self.dfs.exception.take().ok_or(RuntimeError::Invariant(
                "module evaluation exception had no cached value",
            ))?;
            self.runtime.cache_module_evaluation_exception(
                self.root.raw.cache,
                self.root.raw.module,
                &self.dfs.stack,
                &exception,
            )?;
            let reason = self
                .runtime
                .take_pending_exception()?
                .ok_or(RuntimeError::Invariant(
                    "module evaluation failed without a pending exception",
                ))?;
            return self.settle(false, reason);
        }
        Err(error)
    }
    fn advance(mut self: Box<Self>) -> Result<EvaluationStep, RuntimeError> {
        match self.advance_graph() {
            Ok(GraphProgress::Body(step)) => Ok(EvaluationStep::Body {
                step: Box::new(step),
                resume: self,
            }),
            Err(error) => self.fail(error),
            Ok(GraphProgress::Done) => {
                if !self.dfs.stack.is_empty() || self.dfs.exception.is_some() {
                    return Err(RuntimeError::Invariant(
                        "successful module evaluation retained DFS state",
                    ));
                }
                self.armed = false;
                match self.runtime.module_record(self.root.raw)?.evaluation {
                    ModuleEvaluationState::EvaluatingAsync => Ok(EvaluationStep::Complete(
                        Completion::Return(Value::Object(self.capability.promise.clone())),
                    )),
                    ModuleEvaluationState::Evaluated => self.settle(true, Value::Undefined),
                    ModuleEvaluationState::Errored(reason) => {
                        let reason = self.runtime.root_raw_value(&reason)?;
                        self.settle(false, reason)
                    }
                    ModuleEvaluationState::Unevaluated | ModuleEvaluationState::Evaluating => {
                        Err(RuntimeError::Invariant(
                            "successful module evaluation retained an active root state",
                        ))
                    }
                    ModuleEvaluationState::Poisoned => Err(RuntimeError::Invariant(
                        "module evaluation poisoned after a successful graph traversal",
                    )),
                }
            }
        }
    }
    fn advance_graph(&mut self) -> Result<GraphProgress, RuntimeError> {
        let runtime = &self.runtime;
        let dfs = &mut self.dfs;
        let frames = &mut self.frames;
        while !frames.is_empty() {
            let dependency = {
                let frame = frames.last_mut().ok_or(RuntimeError::Invariant(
                    "module evaluation call stack unexpectedly became empty",
                ))?;
                let dependency = frame.dependencies.get(frame.next_dependency).cloned();
                if dependency.is_some() {
                    frame.next_dependency += 1;
                }
                dependency
            };
            if let Some(dependency) = dependency {
                let dependency_state = {
                    let record = runtime.module_record(dependency)?;
                    match &record.evaluation {
                        ModuleEvaluationState::Unevaluated => ModuleEvaluationVisit::Unevaluated,
                        ModuleEvaluationState::Evaluating => ModuleEvaluationVisit::Evaluating,
                        ModuleEvaluationState::EvaluatingAsync => {
                            ModuleEvaluationVisit::EvaluatingAsync
                        }
                        ModuleEvaluationState::Evaluated => ModuleEvaluationVisit::Evaluated,
                        ModuleEvaluationState::Errored(exception) => {
                            ModuleEvaluationVisit::Errored(runtime.root_raw_value(exception)?)
                        }
                        ModuleEvaluationState::Poisoned => ModuleEvaluationVisit::Poisoned,
                    }
                };
                let async_dependency = match dependency_state {
                    ModuleEvaluationVisit::Evaluated => {
                        let cycle_root = runtime
                            .module_record(dependency)?
                            .evaluation_cycle_root
                            .ok_or(RuntimeError::Invariant(
                            "completed dependency has no cycle root",
                        ))?;
                        Some(RawModuleRef {
                            cache: dependency.cache,
                            module: cycle_root,
                        })
                    }
                    ModuleEvaluationVisit::EvaluatingAsync => {
                        let cycle_root = runtime
                            .module_record(dependency)?
                            .evaluation_cycle_root
                            .ok_or(RuntimeError::Invariant(
                            "async dependency has no cycle root",
                        ))?;
                        Some(RawModuleRef {
                            cache: dependency.cache,
                            module: cycle_root,
                        })
                    }
                    ModuleEvaluationVisit::Evaluating => {
                        let dependency_ancestor = dfs
                            .entries
                            .get(&dependency.module)
                            .map(|entry| entry.ancestor)
                            .ok_or(RuntimeError::Invariant(
                                "evaluating dependency has no DFS entry",
                            ))?;
                        let current_id = frames.last().map(|frame| frame.module.module).ok_or(
                            RuntimeError::Invariant(
                                "module evaluation call stack unexpectedly became empty",
                            ),
                        )?;
                        let entry =
                            dfs.entries
                                .get_mut(&current_id)
                                .ok_or(RuntimeError::Invariant(
                                    "evaluating module lost its DFS entry",
                                ))?;
                        entry.ancestor = entry.ancestor.min(dependency_ancestor);
                        Some(dependency)
                    }
                    ModuleEvaluationVisit::Unevaluated => {
                        // Revisit this exact dependency after its child frame
                        // returns. InnerModuleEvaluation must then canonicalize
                        // its cycle root and register any async blocker; merely
                        // advancing past the edge loses that post-child phase.
                        let parent = frames.last_mut().ok_or(RuntimeError::Invariant(
                            "module evaluation call stack unexpectedly became empty",
                        ))?;
                        parent.next_dependency = parent.next_dependency.checked_sub(1).ok_or(
                            RuntimeError::Invariant(
                                "module dependency cursor underflow before child evaluation",
                            ),
                        )?;
                        frames.push(runtime.enter_module_evaluation_dfs(dependency, dfs)?);
                        continue;
                    }
                    ModuleEvaluationVisit::Errored(exception) => {
                        if dfs.exception.replace(exception).is_some() {
                            return Err(RuntimeError::Invariant(
                                "module evaluation recorded more than one exception",
                            ));
                        }
                        return Err(RuntimeError::Exception);
                    }
                    ModuleEvaluationVisit::Poisoned => {
                        return Err(RuntimeError::Invariant(
                            "module evaluation previously failed inside the engine",
                        ));
                    }
                };
                if let Some(async_dependency) = async_dependency {
                    let dependency_record = runtime.module_record(async_dependency)?;
                    if matches!(
                        dependency_record.evaluation,
                        ModuleEvaluationState::Errored(_)
                    ) {
                        let ModuleEvaluationState::Errored(exception) =
                            dependency_record.evaluation
                        else {
                            unreachable!();
                        };
                        let exception = runtime.root_raw_value(&exception)?;
                        if dfs.exception.replace(exception).is_some() {
                            return Err(RuntimeError::Invariant(
                                "module evaluation recorded more than one exception",
                            ));
                        }
                        return Err(RuntimeError::Exception);
                    }
                    if dependency_record.async_evaluation_order.is_some() {
                        let parent = frames.last().map(|frame| frame.module).ok_or(
                            RuntimeError::Invariant(
                                "module evaluation call stack unexpectedly became empty",
                            ),
                        )?;
                        runtime
                            .0
                            .state
                            .borrow_mut()
                            .heap
                            .add_loaded_module_async_dependency(async_dependency, parent)?;
                    }
                }
                continue;
            }

            let frame = frames.pop().ok_or(RuntimeError::Invariant(
                "module evaluation call stack unexpectedly became empty",
            ))?;
            let record = runtime.module_record(frame.module)?;
            if record.pending_async_dependencies != 0 || record.has_top_level_await {
                let order = runtime.next_module_async_evaluation_order()?;
                runtime.transition_module_record(
                    frame.module,
                    RawModuleTransition::BeginAsyncEvaluation { order },
                )?;
            }
            if record.pending_async_dependencies != 0 {
                Self::finish_frame(
                    runtime,
                    dfs,
                    frames,
                    frame,
                    Completion::Return(Value::Undefined),
                )?;
                continue;
            }
            let step = BodyStep::start(
                runtime,
                self.realm,
                frame.module,
                record.has_top_level_await,
            )?;
            match step {
                BodyStep::Complete(completion) => {
                    Self::finish_frame(runtime, dfs, frames, frame, completion)?
                }
                step => {
                    self.pending = Some(frame);
                    return Ok(GraphProgress::Body(step));
                }
            }
        }
        Ok(GraphProgress::Done)
    }
    fn finish_frame(
        runtime: &Runtime,
        dfs: &mut ModuleEvaluationDfs,
        frames: &mut [ModuleDfsFrame],
        frame: ModuleDfsFrame,
        completion: Completion,
    ) -> Result<(), RuntimeError> {
        match completion {
            Completion::Return(Value::Undefined) => {
                let entry = dfs.entries.get(&frame.module.module).copied().ok_or(
                    RuntimeError::Invariant("evaluated module lost its DFS entry"),
                )?;
                if entry.index == entry.ancestor {
                    loop {
                        let member = *dfs.stack.last().ok_or(RuntimeError::Invariant(
                            "module evaluation SCC stack underflow",
                        ))?;
                        let member = RawModuleRef {
                            cache: frame.module.cache,
                            module: member,
                        };
                        let is_evaluating = matches!(
                            runtime.module_record(member)?.evaluation,
                            ModuleEvaluationState::Evaluating
                        );
                        if !is_evaluating {
                            return Err(RuntimeError::Invariant(
                                "module evaluation SCC contained a non-evaluating member",
                            ));
                        }
                        runtime.transition_module_record(
                            member,
                            RawModuleTransition::FinishEvaluation {
                                cycle_root: frame.module.module,
                            },
                        )?;
                        let popped = dfs.stack.pop().ok_or(RuntimeError::Invariant(
                            "module evaluation SCC stack underflow after publication",
                        ))?;
                        if popped != member.module {
                            return Err(RuntimeError::Invariant(
                                "module evaluation SCC stack changed during record publication",
                            ));
                        }
                        if member.module == frame.module.module {
                            break;
                        }
                    }
                }
            }
            Completion::Return(_) => {
                runtime.transition_module_record(
                    frame.module,
                    RawModuleTransition::PoisonEvaluation,
                )?;
                return Err(RuntimeError::Invariant(
                    "module evaluation returned a non-undefined value",
                ));
            }
            Completion::Throw(exception) => {
                if dfs.exception.replace(exception).is_some() {
                    return Err(RuntimeError::Invariant(
                        "module evaluation recorded more than one exception",
                    ));
                }
                return Err(RuntimeError::Exception);
            }
        }

        let still_evaluating = matches!(
            runtime.module_record(frame.module)?.evaluation,
            ModuleEvaluationState::Evaluating
        );
        if still_evaluating {
            let dependency_ancestor = dfs
                .entries
                .get(&frame.module.module)
                .map(|entry| entry.ancestor)
                .ok_or(RuntimeError::Invariant(
                    "evaluating dependency has no DFS entry",
                ))?;
            if let Some(parent) = frames.last() {
                let entry =
                    dfs.entries
                        .get_mut(&parent.module.module)
                        .ok_or(RuntimeError::Invariant(
                            "evaluating module lost its DFS entry",
                        ))?;
                entry.ancestor = entry.ancestor.min(dependency_ancestor);
            }
        }
        Ok(())
    }
}
impl Drop for EvaluationResume {
    fn drop(&mut self) {
        if self.armed {
            let _ = self
                .runtime
                .poison_active_module_evaluations(self.root.raw, &self.dfs.stack);
        }
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<EvaluationStep>() <= 64);
