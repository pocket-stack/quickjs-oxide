use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{NativeCProto, NativeFunctionId};
use crate::engine::code::function::metadata::EvalKind;
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::heap::runtime::DeferredRefOp;
use crate::source::LineColumn;

use crate::engine::heap::{ContextId, FunctionBytecodeId, ObjectId, ObjectPayload};
use crate::engine::object::ObjectRef;
use crate::engine::value::JsString;
use crate::engine::vm::BytecodePc;

impl Runtime {
    pub(crate) fn push_active_collection_record(
        &self,
        record: ActiveCollectionRecord,
    ) -> ActiveCollectionRecordGuard {
        let depth = {
            let mut state = self.0.state.borrow_mut();
            let depth = state.active_collection_records.len();
            state.active_collection_records.push(record);
            depth
        };
        ActiveCollectionRecordGuard {
            runtime: self.clone(),
            record,
            depth,
            active: true,
        }
    }

    pub(crate) fn push_active_frame(
        &self,
        function_root: ObjectRef,
        bytecode_root: Option<FunctionBytecodeRef>,
        realm: ContextId,
        flags: ActiveFrameFlags,
        kind: ActiveFrameKind,
        native_iterator_next_fast_path: bool,
    ) -> Result<ActiveFrameGuard, RuntimeError> {
        if !function_root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("active-frame function"));
        }
        if bytecode_root
            .as_ref()
            .is_some_and(|root| !root.belongs_to(self))
        {
            return Err(RuntimeError::WrongRuntime("active-frame bytecode"));
        }

        let (token, depth) = {
            let mut state = self.0.state.borrow_mut();
            state.heap.context(realm)?;
            let object = state.heap.object(function_root.object_id())?;
            match (kind, &object.payload, bytecode_root.as_ref()) {
                (
                    ActiveFrameKind::Bytecode { bytecode, .. },
                    ObjectPayload::BytecodeFunction {
                        bytecode: object_bytecode,
                        ..
                    },
                    Some(root),
                ) if *object_bytecode == bytecode && root.bytecode_id() == bytecode => {
                    if state.heap.function_bytecode(bytecode)?.realm != realm {
                        return Err(RuntimeError::Invariant(
                            "bytecode active frame realm disagrees with its bytecode",
                        ));
                    }
                }
                (
                    ActiveFrameKind::Native {
                        target,
                        actual_arg_count,
                        readable_arg_count,
                    },
                    ObjectPayload::NativeFunction { data, .. },
                    None,
                ) if data.target == target
                    && data.realm.is_some_and(|defining_realm| {
                        target.uses_calling_realm()
                            || defining_realm == realm
                            || (native_iterator_next_fast_path
                                && target.descriptor().cproto == NativeCProto::IteratorNext)
                    })
                    && readable_arg_count
                        == actual_arg_count.max(usize::from(data.min_readable_args)) => {}
                (ActiveFrameKind::Bytecode { .. }, _, _) => {
                    return Err(RuntimeError::Invariant(
                        "bytecode active frame disagrees with its rooted callable",
                    ));
                }
                (ActiveFrameKind::Native { .. }, _, _) => {
                    return Err(RuntimeError::Invariant(
                        "native active frame disagrees with its rooted callable",
                    ));
                }
            }

            let token = ActiveFrameToken(state.next_active_frame_token);
            state.next_active_frame_token =
                state
                    .next_active_frame_token
                    .checked_add(1)
                    .ok_or(RuntimeError::Invariant(
                        "active-frame token space was exhausted",
                    ))?;
            let depth = state.active_frames.len();
            state.active_frames.push(ActiveFrameRecord {
                token,
                function: function_root.object_id(),
                realm,
                flags,
                kind,
            });
            (token, depth)
        };

        Ok(ActiveFrameGuard {
            runtime: self.clone(),
            token,
            depth,
            active: true,
            _function_root: function_root,
            _bytecode_root: bytecode_root,
        })
    }

    pub(crate) fn push_bytecode_active_frame(
        &self,
        function_root: ObjectRef,
        bytecode_root: FunctionBytecodeRef,
        realm: ContextId,
        strict: bool,
    ) -> Result<ActiveFrameGuard, RuntimeError> {
        let bytecode = bytecode_root.bytecode_id();
        self.push_active_frame(
            function_root,
            Some(bytecode_root),
            realm,
            ActiveFrameFlags {
                strict,
                ..ActiveFrameFlags::default()
            },
            ActiveFrameKind::Bytecode { bytecode, pc: None },
            false,
        )
    }

    pub(crate) fn push_native_active_frame(
        &self,
        function_root: ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        actual_arg_count: usize,
        readable_arg_count: usize,
    ) -> Result<ActiveFrameGuard, RuntimeError> {
        self.push_active_frame(
            function_root,
            None,
            realm,
            ActiveFrameFlags::default(),
            ActiveFrameKind::Native {
                target,
                actual_arg_count,
                readable_arg_count,
            },
            false,
        )
    }

    pub(crate) fn push_native_iterator_next_active_frame(
        &self,
        function_root: ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        actual_arg_count: usize,
        readable_arg_count: usize,
    ) -> Result<ActiveFrameGuard, RuntimeError> {
        self.push_active_frame(
            function_root,
            None,
            realm,
            ActiveFrameFlags {
                backtrace_hidden: true,
                ..ActiveFrameFlags::default()
            },
            ActiveFrameKind::Native {
                target,
                actual_arg_count,
                readable_arg_count,
            },
            true,
        )
    }

    pub(crate) fn update_active_bytecode_pc(
        &self,
        token: ActiveFrameToken,
        pc: BytecodePc,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let frame = state
            .active_frames
            .last_mut()
            .ok_or(RuntimeError::Invariant(
                "bytecode PC update ran without an active frame",
            ))?;
        if frame.token != token {
            return Err(RuntimeError::Invariant(
                "bytecode PC update did not target the top active frame",
            ));
        }
        let ActiveFrameKind::Bytecode { pc: frame_pc, .. } = &mut frame.kind else {
            return Err(RuntimeError::Invariant(
                "bytecode PC update targeted a native active frame",
            ));
        };
        *frame_pc = Some(pc);
        Ok(())
    }

    /// Return the debug name of the active Script or Module, mirroring
    /// QuickJS `JS_GetScriptOrModuleName(ctx, 0)`.
    ///
    /// Synthetic direct and indirect eval roots inherit the first enclosing
    /// non-eval bytecode name. A visible native frame is a hard host boundary;
    /// internal hidden native frames are absent from the observable stack and
    /// are skipped. Backtrace barriers deliberately do not participate in
    /// this lookup.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn active_script_or_module_name(&self) -> Result<Option<JsString>, RuntimeError> {
        let state = self.0.state.borrow();
        for frame in state.active_frames.iter().rev() {
            if frame.flags.backtrace_hidden {
                continue;
            }
            let ActiveFrameKind::Bytecode { bytecode, .. } = frame.kind else {
                return Ok(None);
            };
            let bytecode = state.heap.function_bytecode(bytecode)?;
            if bytecode.metadata.eval_kind != EvalKind::None {
                continue;
            }
            let Some(debug) = &bytecode.debug else {
                return Ok(None);
            };
            return Ok(Some(state.atoms.to_js_string(debug.filename)?));
        }
        Ok(None)
    }

    /// QuickJS `JS_EVAL_FLAG_BACKTRACE_BARRIER` temporarily marks the frame
    /// which existed before eval begins. New eval/nested frames remain visible
    /// and stack traversal stops before printing this caller frame.
    pub(crate) fn install_backtrace_barrier(
        &self,
        enabled: bool,
    ) -> Result<BacktraceBarrierGuard, RuntimeError> {
        if !enabled {
            return Ok(BacktraceBarrierGuard {
                runtime: self.clone(),
                token: None,
                previous: false,
                active: true,
            });
        }
        let (token, previous) = {
            let mut state = self.0.state.borrow_mut();
            let Some(frame) = state.active_frames.last_mut() else {
                return Ok(BacktraceBarrierGuard {
                    runtime: self.clone(),
                    token: None,
                    previous: false,
                    active: true,
                });
            };
            let previous = frame.flags.backtrace_barrier;
            frame.flags.backtrace_barrier = true;
            (frame.token, previous)
        };
        Ok(BacktraceBarrierGuard {
            runtime: self.clone(),
            token: Some(token),
            previous,
            active: true,
        })
    }

    pub(crate) fn restore_backtrace_barrier(
        &self,
        token: ActiveFrameToken,
        previous: bool,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let frame = state
            .active_frames
            .iter_mut()
            .find(|frame| frame.token == token)
            .ok_or(RuntimeError::Invariant(
                "backtrace-barrier caller frame disappeared during eval",
            ))?;
        frame.flags.backtrace_barrier = previous;
        Ok(())
    }

    pub(crate) fn restore_backtrace_barrier_fallback(
        &self,
        token: ActiveFrameToken,
        previous: bool,
    ) {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            if let Some(frame) = state
                .active_frames
                .iter_mut()
                .find(|frame| frame.token == token)
            {
                frame.flags.backtrace_barrier = previous;
            }
        } else {
            self.0
                .deferred_references
                .push_front(DeferredRefOp::BacktraceBarrierRestore { token, previous });
        }
    }

    pub(crate) fn pop_active_collection_record(
        &self,
        record: ActiveCollectionRecord,
        depth: usize,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        if state.active_collection_records.len() == depth + 1
            && state.active_collection_records.last() == Some(&record)
        {
            state.active_collection_records.pop();
            return Ok(());
        }

        state.active_collection_records.truncate(depth);
        Err(RuntimeError::Invariant(
            "active collection record stack was not restored in LIFO order",
        ))
    }

    pub(crate) fn pop_active_collection_record_fallback(&self, depth: usize) {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            state.active_collection_records.truncate(depth);
        } else {
            self.0
                .deferred_references
                .push_front(DeferredRefOp::ActiveCollectionRecordsTruncate { depth });
        }
    }

    pub(crate) fn pop_active_frame(
        &self,
        token: ActiveFrameToken,
        depth: usize,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        if state.active_frames.len() == depth + 1
            && state.active_frames.last().map(|frame| frame.token) == Some(token)
        {
            state.active_frames.pop();
            return Ok(());
        }

        if let Some(position) = state
            .active_frames
            .iter()
            .rposition(|frame| frame.token == token)
        {
            state.active_frames.truncate(position);
        } else if state.active_frames.len() > depth {
            state.active_frames.truncate(depth);
        }
        Err(RuntimeError::Invariant(
            "active frame stack was not restored in LIFO order",
        ))
    }

    pub(crate) fn pop_active_frame_fallback(&self, token: ActiveFrameToken, depth: usize) {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            if let Some(position) = state
                .active_frames
                .iter()
                .rposition(|frame| frame.token == token)
            {
                state.active_frames.truncate(position);
            } else if state.active_frames.len() > depth {
                state.active_frames.truncate(depth);
            }
        } else {
            self.0
                .deferred_references
                .push_front(DeferredRefOp::ActiveFramePop { token, depth });
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActiveCollectionRecord {
    Map { object: ObjectId, index: usize },
    Set { object: ObjectId, index: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActiveFrameRecord {
    pub(crate) token: ActiveFrameToken,
    pub(crate) function: ObjectId,
    pub(crate) realm: ContextId,
    pub(crate) flags: ActiveFrameFlags,
    pub(crate) kind: ActiveFrameKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActiveFrameToken(pub(in crate::engine::vm) u64);

/// Flags which belong to a QuickJS stack frame rather than to the callable
/// heap object. Raw IteratorNext dispatch keeps a rooted validation frame but
/// hides it from JavaScript backtraces because QuickJS calls that native
/// function pointer without pushing a visible `JSStackFrame`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ActiveFrameFlags {
    pub(crate) strict: bool,
    pub(crate) is_async: bool,
    pub(crate) backtrace_barrier: bool,
    pub(crate) backtrace_hidden: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActiveFrameKind {
    Bytecode {
        bytecode: FunctionBytecodeId,
        pc: Option<BytecodePc>,
    },
    Native {
        target: NativeFunctionId,
        actual_arg_count: usize,
        readable_arg_count: usize,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct ExplicitBacktraceLocation {
    pub(crate) filename: JsString,
    pub(crate) position: LineColumn,
}

/// Stack-owned root set and LIFO token for one active execution frame.
///
/// Normal execution calls [`Self::finish`] so token/order corruption becomes
/// an engine error. `Drop` is a no-fail fallback for unwinding paths and keeps
/// stale diagnostic frames from escaping their invocation.
pub(crate) struct ActiveFrameGuard {
    pub(crate) runtime: Runtime,
    pub(crate) token: ActiveFrameToken,
    pub(crate) depth: usize,
    pub(crate) active: bool,
    pub(crate) _function_root: ObjectRef,
    pub(crate) _bytecode_root: Option<FunctionBytecodeRef>,
}

/// LIFO scope for one Map/Set record exposed to QuickJS-style diagnostics.
///
/// Normal execution calls [`Self::finish`] so stack corruption becomes an
/// engine error. `Drop` truncates the scope suffix during unwinding, ensuring
/// a panicking callback cannot leave a stale record visible to later prints.
pub(crate) struct ActiveCollectionRecordGuard {
    pub(crate) runtime: Runtime,
    pub(crate) record: ActiveCollectionRecord,
    pub(crate) depth: usize,
    pub(crate) active: bool,
}

pub(crate) struct BacktraceBarrierGuard {
    pub(crate) runtime: Runtime,
    pub(crate) token: Option<ActiveFrameToken>,
    pub(crate) previous: bool,
    pub(crate) active: bool,
}

impl ActiveFrameGuard {
    pub(crate) const fn token(&self) -> ActiveFrameToken {
        self.token
    }

    pub(crate) fn finish(mut self) -> Result<(), RuntimeError> {
        let result = self.runtime.pop_active_frame(self.token, self.depth);
        self.active = false;
        result
    }
}

impl Drop for ActiveFrameGuard {
    fn drop(&mut self) {
        if self.active {
            self.runtime
                .pop_active_frame_fallback(self.token, self.depth);
            self.active = false;
        }
    }
}

impl ActiveCollectionRecordGuard {
    pub(crate) fn finish(mut self) -> Result<(), RuntimeError> {
        let result = self
            .runtime
            .pop_active_collection_record(self.record, self.depth);
        self.active = false;
        result
    }
}

impl Drop for ActiveCollectionRecordGuard {
    fn drop(&mut self) {
        if self.active {
            self.runtime
                .pop_active_collection_record_fallback(self.depth);
            self.active = false;
        }
    }
}

impl BacktraceBarrierGuard {
    pub(crate) fn finish(mut self) -> Result<(), RuntimeError> {
        if let Some(token) = self.token {
            self.runtime
                .restore_backtrace_barrier(token, self.previous)?;
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for BacktraceBarrierGuard {
    fn drop(&mut self) {
        if self.active {
            if let Some(token) = self.token {
                self.runtime
                    .restore_backtrace_barrier_fallback(token, self.previous);
            }
            self.active = false;
        }
    }
}
