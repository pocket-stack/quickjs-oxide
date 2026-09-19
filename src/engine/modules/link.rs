//! Resumable module linking over the existing DFS/SCC records.
use super::{ModuleBytecodeRef, ModuleDfsFrame, ModuleLinkDfs, ModuleLinkStatus};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::heap::{ContextId, RawModuleLinkRealm, RawModuleRef, RawModuleTransition};
use crate::engine::object::CallableRef;
use crate::engine::value::Value;
use crate::engine::vm::Completion;

pub(crate) enum LinkStep {
    Complete(Completion),
    Call {
        realm: ContextId,
        callable: CallableRef,
        resume: Box<LinkResume>,
    },
}
pub(crate) struct LinkResume {
    runtime: Runtime,
    root: ModuleBytecodeRef,
    dfs: ModuleLinkDfs,
    frames: Vec<ModuleDfsFrame>,
    pending: Option<ModuleDfsFrame>,
    armed: bool,
}
impl LinkStep {
    pub(crate) fn start(
        runtime: &Runtime,
        module: RawModuleRef,
        initiating_realm: ContextId,
    ) -> Result<Self, RuntimeError> {
        let root = runtime.root_module(module)?;
        runtime.preflight_module_graph_for_link(module)?;
        runtime.prepare_module_instance(module, initiating_realm)?;
        match runtime.module_record(module)?.link_status {
            ModuleLinkStatus::Linked => {
                return Ok(LinkStep::Complete(Completion::Return(Value::Undefined)));
            }
            ModuleLinkStatus::Linking => {
                return Err(RuntimeError::Invariant(
                    "module linking was re-entered by the host",
                ));
            }
            ModuleLinkStatus::Poisoned => {
                return Err(RuntimeError::Invariant(
                    "module linking previously failed inside the engine",
                ));
            }
            ModuleLinkStatus::Unlinked => {}
        }
        let mut resume = Box::new(LinkResume {
            runtime: runtime.clone(),
            root,
            dfs: ModuleLinkDfs::new(),
            frames: Vec::new(),
            pending: None,
            armed: true,
        });
        resume
            .frames
            .push(runtime.enter_module_link_dfs(module, &mut resume.dfs)?);
        resume.advance()
    }
}
impl LinkResume {
    fn advance(mut self: Box<Self>) -> Result<LinkStep, RuntimeError> {
        let runtime = self.runtime.clone();
        let dfs = &mut self.dfs;
        let frames = &mut self.frames;
        while !frames.is_empty() {
            let dependency = {
                let frame = frames.last_mut().ok_or(RuntimeError::Invariant(
                    "module link call stack unexpectedly became empty",
                ))?;
                let dependency = frame.dependencies.get(frame.next_dependency).cloned();
                if dependency.is_some() {
                    frame.next_dependency += 1;
                }
                dependency
            };
            if let Some(dependency) = dependency {
                match runtime.module_record(dependency)?.link_status {
                    ModuleLinkStatus::Linked => {}
                    ModuleLinkStatus::Linking => {
                        let dependency_ancestor = dfs
                            .entries
                            .get(&dependency.module)
                            .map(|entry| entry.ancestor)
                            .ok_or(RuntimeError::Invariant(
                                "linking dependency has no DFS entry",
                            ))?;
                        let current_id = frames.last().map(|frame| frame.module.module).ok_or(
                            RuntimeError::Invariant(
                                "module link call stack unexpectedly became empty",
                            ),
                        )?;
                        let entry = dfs
                            .entries
                            .get_mut(&current_id)
                            .ok_or(RuntimeError::Invariant("linking module lost its DFS entry"))?;
                        entry.ancestor = entry.ancestor.min(dependency_ancestor);
                    }
                    ModuleLinkStatus::Unlinked => {
                        frames.push(runtime.enter_module_link_dfs(dependency, dfs)?);
                    }
                    ModuleLinkStatus::Poisoned => {
                        return Err(RuntimeError::Invariant(
                            "module linking previously failed inside the engine",
                        ));
                    }
                }
                continue;
            }

            let frame = frames.pop().ok_or(RuntimeError::Invariant(
                "module link call stack unexpectedly became empty",
            ))?;
            let realm = runtime
                .module_record(frame.module)?
                .link_realm
                .map(|realm| match realm {
                    RawModuleLinkRealm::Cache => frame.module.cache,
                    RawModuleLinkRealm::Other(realm) => realm,
                })
                .ok_or(RuntimeError::Invariant(
                    "instantiated module has no retained link realm",
                ))?;
            runtime.validate_module_indirect_exports(frame.module, &frame.dependencies, realm)?;
            runtime.link_module_imports(frame.module, &frame.dependencies, realm)?;
            if let Some(callable) = runtime.create_module_callable(frame.module, realm)? {
                self.pending = Some(frame);
                return Ok(LinkStep::Call {
                    realm,
                    callable,
                    resume: self,
                });
            }
            Self::finish_frame(
                &runtime,
                dfs,
                frames,
                frame,
                Completion::Return(Value::Undefined),
            )?;
        }
        if !dfs.stack.is_empty() {
            return Err(RuntimeError::Invariant(
                "successful module linking retained an SCC stack",
            ));
        }
        self.armed = false;
        Ok(LinkStep::Complete(Completion::Return(Value::Undefined)))
    }
    fn finish_frame(
        runtime: &Runtime,
        dfs: &mut ModuleLinkDfs,
        frames: &mut [ModuleDfsFrame],
        frame: ModuleDfsFrame,
        completion: Completion,
    ) -> Result<(), RuntimeError> {
        match completion {
            Completion::Return(Value::Undefined) => {
                let entry = dfs
                    .entries
                    .get(&frame.module.module)
                    .copied()
                    .ok_or(RuntimeError::Invariant("linked module lost its DFS entry"))?;
                if entry.index == entry.ancestor {
                    loop {
                        let member = *dfs
                            .stack
                            .last()
                            .ok_or(RuntimeError::Invariant("module link SCC stack underflow"))?;
                        let member = RawModuleRef {
                            cache: frame.module.cache,
                            module: member,
                        };
                        if !matches!(
                            runtime.module_record(member)?.link_status,
                            ModuleLinkStatus::Linking
                        ) {
                            return Err(RuntimeError::Invariant(
                                "module link SCC contained a non-linking member",
                            ));
                        }
                        runtime
                            .transition_module_record(member, RawModuleTransition::FinishLink)?;
                        let popped = dfs.stack.pop().ok_or(RuntimeError::Invariant(
                            "module link SCC stack underflow after publication",
                        ))?;
                        if popped != member.module {
                            return Err(RuntimeError::Invariant(
                                "module link SCC stack changed during record publication",
                            ));
                        }
                        if member.module == frame.module.module {
                            break;
                        }
                    }
                }
            }
            Completion::Return(_) => {
                runtime.transition_module_record(frame.module, RawModuleTransition::PoisonLink)?;
                return Err(RuntimeError::Invariant(
                    "module link entry returned a non-undefined value",
                ));
            }
            Completion::Throw(exception) => {
                runtime.set_pending_exception(exception)?;
                return Err(RuntimeError::Exception);
            }
        }

        if matches!(
            runtime.module_record(frame.module)?.link_status,
            ModuleLinkStatus::Linking
        ) {
            let dependency_ancestor = dfs
                .entries
                .get(&frame.module.module)
                .map(|entry| entry.ancestor)
                .ok_or(RuntimeError::Invariant(
                    "linking dependency has no DFS entry",
                ))?;
            if let Some(parent) = frames.last() {
                let entry = dfs
                    .entries
                    .get_mut(&parent.module.module)
                    .ok_or(RuntimeError::Invariant("linking module lost its DFS entry"))?;
                entry.ancestor = entry.ancestor.min(dependency_ancestor);
            }
        }
        Ok(())
    }
    pub(crate) fn resume(
        mut self: Box<Self>,
        completion: Completion,
    ) -> Result<LinkStep, RuntimeError> {
        let frame = self
            .pending
            .take()
            .ok_or(RuntimeError::Invariant("module link has no pending prefix"))?;
        Self::finish_frame(
            &self.runtime,
            &mut self.dfs,
            &mut self.frames,
            frame,
            completion,
        )?;
        self.advance()
    }
}
impl Drop for LinkResume {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // An abandoned or engine-failed active prefix cannot be replayed.
        if let Some(frame) = self.pending.take() {
            let _ = self
                .runtime
                .transition_module_record(frame.module, RawModuleTransition::PoisonLink);
        }
        for id in &self.dfs.stack {
            let member = RawModuleRef {
                cache: self.root.raw.cache,
                module: *id,
            };
            if self
                .runtime
                .module_record(member)
                .is_ok_and(|record| matches!(record.link_status, ModuleLinkStatus::Linking))
            {
                let _ = self
                    .runtime
                    .transition_module_record(member, RawModuleTransition::ResetLink);
            }
        }
    }
}

pub(crate) fn resume_reply(
    runtime: &Runtime,
    result: Result<LinkStep, RuntimeError>,
) -> Result<LinkStep, RuntimeError> {
    match result {
        Err(RuntimeError::Exception) => {
            let reason = runtime
                .take_pending_exception()?
                .ok_or(RuntimeError::Invariant(
                    "module link exception has no pending value",
                ))?;
            Ok(LinkStep::Complete(Completion::Throw(reason)))
        }
        result => result,
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<LinkStep>() <= 64);
