//! Authenticated direct ordinary calls. No wrapper dispatch or materialized argv.
use crate::engine::{
    api::{Error, runtime::Runtime, runtime_error::RuntimeError},
    code::{
        function::metadata::FunctionKind,
        rooted::FunctionBytecodeRef,
        runtime::{OrdinaryAuthentication, PublishedFunctionSnapshot},
    },
    heap::{FunctionBytecodeId, ObjectPayload, VarRefId},
    object::ObjectRef,
    value::Value,
    vm::{closure::ClosureSlots, frames::ActiveFrameGuard},
};

// Only this module can authenticate or construct this witness.
pub(in crate::engine::vm) struct OrdinaryCall {
    function: ObjectRef,
    executable: PublishedFunctionSnapshot,
    closure: ClosureSlots,
}
// Selection may read metadata but does not publish a frame or consume operands.
// Any malformed metadata error is returned only after the original domain check.
pub(in crate::engine::vm) struct OrdinarySelection<'a> {
    function: &'a ObjectRef,
    bytecode: FunctionBytecodeId,
    authentication: Option<OrdinaryAuthentication>,
    closure: std::cell::Ref<'a, std::rc::Rc<[VarRefId]>>,
}
// Only DirectSelection can create this proof: payload metadata and borrowed
// owner originate from the same heap lookup. Promotion cannot accept a caller's
// detached metadata or an unrelated object.
pub(in crate::engine::vm) struct NativeSelection<'a> {
    function: &'a ObjectRef,
    target: crate::engine::builtins::native::NativeFunctionId,
    defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    operation: crate::engine::builtins::continuation::NativeOperation,
}
impl<'a> NativeSelection<'a> {
    pub(in crate::engine::vm) fn into_parts(
        self,
    ) -> (
        &'a ObjectRef,
        crate::engine::builtins::native::NativeFunctionId,
        crate::engine::heap::ContextId,
        u8,
        crate::engine::builtins::continuation::NativeOperation,
    ) {
        (
            self.function,
            self.target,
            self.defining_realm,
            self.min_readable_args,
            self.operation,
        )
    }
}

pub(in crate::engine::vm) enum DirectSelection<'a> {
    Ordinary(OrdinarySelection<'a>),
    Native(NativeSelection<'a>),
    General,
}

impl<'a> DirectSelection<'a> {
    /// One payload inspection for ordinary, native and general callees. Any
    /// metadata error is held by the caller until operand domains are checked.
    pub(in crate::engine::vm) fn select(
        runtime: &'a Runtime,
        value: &'a Value,
    ) -> Result<Self, RuntimeError> {
        let Value::Object(function) = value else {
            return Ok(Self::General);
        };
        if !function.belongs_to(runtime) {
            return Ok(Self::General);
        }
        let mut selected_bytecode = None;
        let mut selected_authentication = None;
        let mut native = None;
        let mut failure = None;
        let closure = std::cell::Ref::filter_map(runtime.0.state.borrow(), |state| {
            let selected = (|| {
                let object = state.heap.object(function.object_id())?;
                if let ObjectPayload::NativeFunction { data, .. } = &object.payload {
                    // Unregistered native kinds retain the checked general
                    // entry, including its original preparation/error order.
                    if let Some(operation) = data.operation() {
                        let defining_realm = data.realm.ok_or(RuntimeError::Invariant(
                            "native function was called before its defining realm was attached",
                        ))?;
                        state.heap.context(defining_realm)?;
                        native = Some(NativeSelection {
                            function,
                            target: data.target,
                            defining_realm,
                            min_readable_args: data.min_readable_args,
                            operation,
                        });
                    }
                    return Ok(None);
                }
                let ObjectPayload::BytecodeFunction {
                    bytecode,
                    closure_slots,
                    authentication,
                    ..
                } = &object.payload
                else {
                    return Ok(None);
                };
                let cached = authentication.borrow();
                if let Some(facts) = cached
                    .as_ref()
                    .filter(|facts| facts.publish_generation == bytecode.publish_generation())
                {
                    if closure_slots.len() != facts.closure_count {
                        return Err(RuntimeError::Invariant(
                            "function object closure slot count does not match bytecode metadata",
                        ));
                    }
                    selected_authentication = Some(facts.clone());
                } else {
                    let data = state.heap.function_bytecode(*bytecode)?;
                    if data.metadata.function_kind != FunctionKind::Normal {
                        return Ok(None);
                    }
                    if closure_slots.len() != usize::from(data.metadata.closure_count) {
                        return Err(RuntimeError::Invariant(
                            "function object closure slot count does not match bytecode metadata",
                        ));
                    }
                }
                selected_bytecode = Some(*bytecode);
                Ok(Some(closure_slots))
            })();
            match selected {
                Ok(closure) => closure,
                Err(error) => {
                    failure = Some(error);
                    None
                }
            }
        });
        match closure {
            Ok(closure) => Ok(Self::Ordinary(OrdinarySelection {
                function,
                authentication: selected_authentication,
                bytecode: selected_bytecode
                    .ok_or(RuntimeError::Invariant("ordinary selection lost bytecode"))?,
                closure,
            })),
            Err(_) => match failure {
                Some(error) => Err(error),
                None => Ok(native.map(Self::Native).unwrap_or(Self::General)),
            },
        }
    }
}

impl OrdinaryCall {
    #[cfg(test)]
    pub(in crate::engine::vm) fn authenticate(
        runtime: &Runtime,
        value: &Value,
    ) -> Result<Option<Self>, RuntimeError> {
        match DirectSelection::select(runtime, value)? {
            DirectSelection::Ordinary(selected) => selected.authenticate(runtime).map(Some),
            _ => Ok(None),
        }
    }
    pub(in crate::engine::vm) fn function(&self) -> &ObjectRef {
        &self.function
    }
    pub(in crate::engine::vm) fn executable(&self) -> &PublishedFunctionSnapshot {
        &self.executable
    }
    pub(in crate::engine::vm) fn register(
        &self,
        runtime: &Runtime,
    ) -> Result<ActiveFrameGuard, RuntimeError> {
        runtime.push_ordinary_active_frame(self)
    }
    pub(in crate::engine::vm) fn install(
        self,
        _runtime: &Runtime,
        execution: &mut crate::engine::vm::execution::RunningExecution,
        parent: crate::engine::vm::frame::FrameId,
        count: usize,
        method: bool,
        tail: bool,
    ) -> Result<(), Error> {
        use crate::engine::vm::frame::{Frame, ReturnOwner, ReturnTarget, ReturnValue};
        let depth = execution.frames.depth() + 1;
        execution.call_storage.reserve_depth(depth)?;
        let frame = execution.frames.current_mut(parent)?;
        let caller_realm = frame.executable.realm;
        let resume = frame
            .fault_pc
            .checked_add(1)
            .ok_or_else(|| Error::internal("call resume PC overflow"))?;
        let receiver = if method {
            crate::engine::vm::stack::copy_value(execution.slots.peek(&frame.window, count + 1)?)?
        } else {
            Value::Undefined
        };
        let (flags, flag_bytes) = if self.executable.has_captured_locals {
            execution
                .call_storage
                .capture_flags(self.executable.local_definitions.len())?
        } else {
            (Vec::new(), 0)
        };
        let prepared = execution.frames.prepare_push()?;
        let mut prepared = prepared;
        let frame = prepared.current_mut(parent)?;
        let window = execution.slots.push_ordinary_frame(
            &self.executable.frame_layout(),
            &mut frame.window,
            count,
            method,
            &self.function,
            self.executable.metadata.function_name_local,
            self.executable.observes_arguments,
        )?;
        frame.resume_pc = resume;
        let (mut cold, frame_bytes) = execution.call_storage.vacant(caller_realm);
        cold.return_to = Some(ReturnTarget {
            value_use: ReturnValue::Push,
            owner: ReturnOwner::Frame(parent),
            tail,
            operation: None,
        });
        cold.entry_guard = None;
        cold.function = self.function.into();
        cold.closure_slots = self.closure;
        cold.reusable_captured_locals = flags;
        cold.input = crate::engine::vm::CallInput {
            this_value: receiver,
            new_target: Value::Undefined,
            callee_global: None,
        }
        .into();
        cold.executable = self.executable.into();
        cold.window = window.into();
        prepared.install(Frame {
            property_generation: 0,
            iterator_generation: 0,
            caller_realm,
            active_frame: crate::engine::vm::frames::ActiveFrameToken::unmaterialized(),

            fault_pc: 0,
            resume_pc: 0,
            cold,
        });
        #[cfg(feature = "profiling")]
        {
            crate::engine::api::profiling::record_owned_call_storage(frame_bytes, flag_bytes, 0);
        }
        #[cfg(not(feature = "profiling"))]
        let _ = (frame_bytes, flag_bytes);
        Ok(())
    }
}

impl OrdinarySelection<'_> {
    pub(in crate::engine::vm) fn authenticate(
        self,
        runtime: &Runtime,
    ) -> Result<OrdinaryCall, RuntimeError> {
        // Domain/slot validation has succeeded. Only now promote the selected
        // shared environment and owner, after ending the read-only heap borrow.
        let closure = std::rc::Rc::clone(&self.closure);
        drop(self.closure);
        let function = self.function.clone();
        let executable = if let Some(facts) = self.authentication {
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "ordinary_call_auth_cache_hit",
            );
            PublishedFunctionSnapshot::from_authentication(runtime, self.bytecode, facts)
        } else {
            let bytecode =
                FunctionBytecodeRef::from_borrowed_handle(runtime.clone(), self.bytecode)?;
            let snapshot = runtime.snapshot_function_bytecode_owned(bytecode)?;
            let facts = snapshot.authentication(closure.len());
            {
                let state = runtime.0.state.borrow();
                let object = state.heap.object(function.object_id())?;
                let ObjectPayload::BytecodeFunction { authentication, .. } = &object.payload else {
                    return Err(RuntimeError::Invariant(
                        "selected ordinary function changed kind",
                    ));
                };
                *authentication.borrow_mut() = Some(facts.clone());
            }
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "ordinary_call_authenticated",
            );
            PublishedFunctionSnapshot::from_authentication(runtime, self.bytecode, facts)
        };
        Ok(OrdinaryCall {
            closure: ClosureSlots::shared(function.clone(), closure),
            function,
            executable,
        })
    }
}

#[cfg(test)]
mod direct_selection_tests {
    use super::*;

    #[test]
    fn authentication_cache_is_rootless_and_rejects_a_different_publication() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let first = context.eval("(function(){ return 11 })").unwrap();
        let second = context.eval("(function(){ return 22 })").unwrap();
        let selected = OrdinaryCall::authenticate(&runtime, &first)
            .unwrap()
            .unwrap();
        assert!(selected.executable.root().is_none());
        let id = selected.executable.bytecode_id().unwrap();
        drop(selected);
        let get_facts = |value: &Value| {
            let Value::Object(object) = value else {
                unreachable!()
            };
            let state = runtime.0.state.borrow();
            let ObjectPayload::BytecodeFunction { authentication, .. } =
                &state.heap.object(object.object_id()).unwrap().payload
            else {
                unreachable!()
            };
            authentication.borrow().clone().unwrap()
        };
        let facts = get_facts(&first);
        let warmed = OrdinaryCall::authenticate(&runtime, &first)
            .unwrap()
            .unwrap();
        assert!(std::rc::Rc::ptr_eq(&facts.data, &get_facts(&first).data));
        assert_eq!(warmed.executable.bytecode_id(), Some(id));
        assert!(warmed.executable.root().is_none());
        assert!(warmed.executable.belongs_to(&runtime));
        let foreign = Runtime::new();
        assert!(!warmed.executable.belongs_to(&foreign));
        assert!(matches!(
            warmed.executable.ensure_root(&foreign),
            Err(RuntimeError::WrongRuntime("function bytecode"))
        ));
        warmed.executable.ensure_root(&runtime).unwrap();
        assert_eq!(warmed.executable.root().unwrap().bytecode_id(), id);
        drop(warmed);
        // A transplanted/stale certificate must miss even if closure arity is
        // identical. Publication identity is stronger than code pointer/shape.
        {
            let Value::Object(object) = &second else {
                unreachable!()
            };
            let state = runtime.0.state.borrow();
            let ObjectPayload::BytecodeFunction { authentication, .. } =
                &state.heap.object(object.object_id()).unwrap().payload
            else {
                unreachable!()
            };
            *authentication.borrow_mut() = Some(facts);
        }
        let refreshed = OrdinaryCall::authenticate(&runtime, &second)
            .unwrap()
            .unwrap();
        assert_ne!(refreshed.executable.bytecode_id(), Some(id));
        assert_eq!(
            get_facts(&second).publish_generation,
            refreshed
                .executable
                .bytecode_id()
                .unwrap()
                .publish_generation()
        );
        assert!(!std::rc::Rc::ptr_eq(
            &get_facts(&first).data,
            &get_facts(&second).data
        ));
    }

    #[test]
    fn heap_authentication_cache_does_not_retain_runtime() {
        let weak = {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let value = context.eval("(function(){ return 1 })").unwrap();
            drop(OrdinaryCall::authenticate(&runtime, &value).unwrap());
            std::rc::Rc::downgrade(&runtime.0)
        };
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn direct_selection_borrows_owners_and_preserves_general_fallback() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for (source, kind) in [
            ("(function(x){return x})", 0),
            ("Math.min", 1),
            ("(function(x){return x}).bind(null)", 2),
            ("new Proxy(function(){},{})", 2),
            ("new Proxy({},{})", 2),
            ("(function*(){})", 2),
            ("({})", 2),
            ("17", 2),
        ] {
            let value = context.eval(source).unwrap();
            let owners = std::rc::Rc::strong_count(&runtime.0);
            let selected = DirectSelection::select(&runtime, &value).unwrap();
            assert_eq!(std::rc::Rc::strong_count(&runtime.0), owners, "{source}");
            assert_eq!(
                match selected {
                    DirectSelection::Ordinary(_) => 0,
                    DirectSelection::Native(_) => 1,
                    DirectSelection::General => 2,
                },
                kind,
                "{source}"
            );
            assert_eq!(std::rc::Rc::strong_count(&runtime.0), owners, "{source}");
        }
        let foreign = Runtime::new();
        let value = Value::Object(foreign.new_object(None).unwrap());
        assert!(matches!(
            DirectSelection::select(&runtime, &value).unwrap(),
            DirectSelection::General
        ));
    }
}
