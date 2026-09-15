//! Map callbacks preserve active record guards and callback-created insertion order.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{CallableRef, ObjectRef},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
        frames::{ActiveCollectionRecord, ActiveCollectionRecordGuard},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum CallbackKind {
    Each,
    Insert { computed: bool },
}
pub(crate) enum CallbackStep {
    Complete(Completion),
    Call { resume: CallbackResume },
}
pub(crate) struct CallbackResume(Box<CallbackResumeState>);
impl std::ops::Deref for CallbackResume {
    type Target = CallbackResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for CallbackResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<CallbackResume>() <= 8);
pub(crate) struct CallbackResumeState {
    pending_effect: CallbackStepPending,
    phase: Phase,
    map: ObjectRef,
}
enum Phase {
    Each {
        callback: CallableRef,
        receiver: Value,
        index: usize,
        record: Option<ActiveCollectionRecordGuard>,
    },
    Insert(Value),
}
impl CallbackStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: CallbackKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let map = match runtime.map_receiver(realm, invocation.clone(), false)? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        if let CallbackKind::Insert { computed } = kind {
            let key = Runtime::normalized_map_key(arguments.readable.first().cloned().ok_or(
                RuntimeError::Invariant("Map getOrInsert key argv was not padded"),
            )?);
            let second = arguments
                .readable
                .get(1)
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "Map getOrInsert value argv was not padded",
                ))?;
            let callback = if computed {
                match callable(runtime, realm, &second)? {
                    NativeConversion::Value(value) => Some(value),
                    NativeConversion::Throw(value) => {
                        return Ok(Self::Complete(Completion::Throw(value)));
                    }
                }
            } else {
                None
            };
            if let Some((_, value)) = runtime.find_map_record(&map, &key)? {
                return Ok(Self::Complete(Completion::Return(
                    runtime.root_raw_value(&value)?,
                )));
            }
            if let Some(callable) = callback {
                return Ok(Self::request_call(
                    callable,
                    Value::Undefined,
                    vec![key.clone()],
                    CallbackResume(Box::new(CallbackResumeState {
                        pending_effect: CallbackStepPending::default(),
                        map,
                        phase: Phase::Insert(key),
                    })),
                ));
            }
            runtime.set_map_record(&map, key, second.clone())?;
            return Ok(Self::Complete(Completion::Return(second)));
        }
        let value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Map.prototype.forEach callback argv was not padded",
        ))?;
        let callback = match callable(runtime, realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        CallbackResume(Box::new(CallbackResumeState {
            pending_effect: CallbackStepPending::default(),
            map,
            phase: Phase::Each {
                callback,
                receiver: arguments
                    .readable
                    .get(1)
                    .cloned()
                    .unwrap_or(Value::Undefined),
                index: 0,
                record: None,
            },
        }))
        .next(runtime)
    }
}
fn callable(
    runtime: &Runtime,
    realm: ContextId,
    value: &Value,
) -> Result<NativeConversion<CallableRef>, RuntimeError> {
    let result = match value {
        Value::Object(object) => runtime.as_callable(object)?,
        _ => None,
    };
    Ok(match result {
        Some(value) => NativeConversion::Value(value),
        None => NativeConversion::Throw(runtime.new_native_error(
            realm,
            NativeErrorKind::Type,
            "not a function",
        )?),
    })
}
impl CallbackResume {
    fn next(mut self, runtime: &Runtime) -> Result<CallbackStep, RuntimeError> {
        let Phase::Each {
            callback,
            receiver,
            index,
            record,
        } = &mut self.0.phase
        else {
            return Err(RuntimeError::Invariant("Map callback next phase mismatch"));
        };
        let entry = {
            runtime
                .0
                .state
                .borrow()
                .heap
                .map_records(self.0.map.object_id())?
                .next_at_or_after(*index)
                .map(|(id, entry)| (id, entry.key.clone(), entry.value.clone()))
        };
        let Some((record_index, key, value)) = entry else {
            return Ok(CallbackStep::Complete(Completion::Return(Value::Undefined)));
        };
        *index = record_index.checked_add(1).ok_or(RuntimeError::Invariant(
            "Map forEach record index overflowed",
        ))?;
        let key = runtime.root_raw_value(&key)?;
        let value = runtime.root_raw_value(&value)?;
        *record = Some(
            runtime.push_active_collection_record(ActiveCollectionRecord::Map {
                object: self.0.map.object_id(),
                index: record_index,
            }),
        );
        Ok(CallbackStep::request_call(
            callback.clone(),
            receiver.clone(),
            vec![value, key, Value::Object(self.0.map.clone())],
            self,
        ))
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<CallbackStep, RuntimeError> {
        if let Phase::Each { record, .. } = &mut self.0.phase
            && let Some(record) = record.take()
        {
            record.finish()?;
        }
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(CallbackStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Insert(key) => {
                runtime.delete_map_record(&self.0.map, &key)?;
                runtime.set_map_record(&self.0.map, key, value.clone())?;
                Ok(CallbackStep::Complete(Completion::Return(value)))
            }
            Phase::Each { .. } => self.next(runtime),
        }
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: CallbackStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            CallbackStep::Complete(result) => return Ok(result),
            CallbackStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &arguments)?,
                )?
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abandoned_each_requests_release_records_and_collection_roots_in_lifo_order() {
        let runtime = Runtime::new();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let Value::Object(map) = context.eval("new Map([[{}, {}]])").unwrap() else {
            panic!("map expected")
        };
        let Value::Object(set) = context.eval("new Set([{}])").unwrap() else {
            panic!("set expected")
        };
        let map_id = map.object_id();
        let set_id = set.object_id();
        let arguments = NativeArguments {
            actual_arg_count: 1,
            readable: vec![context.eval("(function () {})").unwrap()],
        };
        let map_step = CallbackStep::start(
            &runtime,
            context.realm,
            CallbackKind::Each,
            &NativeInvocation::Call {
                this_value: Value::Object(map),
            },
            &arguments,
        )
        .unwrap();
        let set_step = crate::engine::builtins::set::callback::EachStep::start(
            &runtime,
            context.realm,
            &NativeInvocation::Call {
                this_value: Value::Object(set),
            },
            &arguments,
        )
        .unwrap();
        assert!(matches!(map_step, CallbackStep::Call { .. }));
        assert!(matches!(
            set_step,
            crate::engine::builtins::set::callback::EachStep::Call { .. }
        ));
        drop(arguments);
        runtime.run_gc().unwrap();
        {
            let state = runtime.0.state.borrow();
            assert_eq!(state.active_collection_records.len(), 2);
            assert!(state.heap.object(map_id).is_ok());
            assert!(state.heap.object(set_id).is_ok());
        }
        drop(set_step);
        runtime.run_gc().unwrap();
        {
            let state = runtime.0.state.borrow();
            assert_eq!(state.active_collection_records.len(), 1);
            assert!(state.heap.object(map_id).is_ok());
            assert!(state.heap.object(set_id).is_err());
        }
        drop(map_step);
        runtime.run_gc().unwrap();
        {
            let state = runtime.0.state.borrow();
            assert!(state.active_collection_records.is_empty());
            assert!(state.heap.object(map_id).is_err());
        }
        drop(context);
        drop(runtime);
        assert!(weak.upgrade().is_none());
    }
}

#[derive(Default)]
struct CallbackStepPending {
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
}
impl CallbackStep {
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: CallbackResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
}
impl CallbackResume {
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("CallbackStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("CallbackStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("CallbackStep Call arguments")
    }
}
const _: () = assert!(std::mem::size_of::<CallbackStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<CallbackStep>() <= 64);
