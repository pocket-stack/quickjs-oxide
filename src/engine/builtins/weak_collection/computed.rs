//! WeakMap computed insertion roots its key until the callback result is committed.
use super::WeakCollectionKind;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::{ContextId, WeakCollectionKey},
    object::{CallableRef, ObjectRef},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum ComputedStep {
    Complete(Completion),
    Call {
        callable: CallableRef,
        arguments: Vec<Value>,
        resume: ComputedResume,
    },
}
pub(crate) struct ComputedResume(Box<ComputedResumeState>);
impl std::ops::Deref for ComputedResume {
    type Target = ComputedResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ComputedResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ComputedResume>() <= 8);
pub(crate) struct ComputedResumeState {
    map: ObjectRef,
    key: WeakCollectionKey,
    _key_owner: Value,
}
impl ComputedStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let map = match runtime.weak_collection_receiver(
            realm,
            invocation.clone(),
            WeakCollectionKind::Map,
        )? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let key_value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant("WeakMap key argv was not padded"))?;
        let callback_value = arguments
            .readable
            .get(1)
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "WeakMap computed value argv was not padded",
            ))?;
        let callback = match callback_value {
            Value::Object(ref object) => runtime.as_callable(object)?,
            _ => None,
        };
        let Some(callable) = callback else {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error(realm, NativeErrorKind::Type, "not a function")?,
            )));
        };
        let Some(key) = runtime.weak_collection_key(&key_value, "WeakMap key")? else {
            return Ok(Self::Complete(
                runtime.invalid_weak_key(realm, WeakCollectionKind::Map)?,
            ));
        };
        if let Some(value) = runtime.find_weak_map_record(&map, key)? {
            return Ok(Self::Complete(Completion::Return(
                runtime.root_raw_value(&value)?,
            )));
        }
        Ok(Self::Call {
            callable,
            arguments: vec![key_value.clone()],
            resume: ComputedResume(Box::new(ComputedResumeState {
                map,
                key,
                _key_owner: key_value,
            })),
        })
    }
}
impl ComputedResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<ComputedStep, RuntimeError> {
        match reply {
            Completion::Throw(value) => Ok(ComputedStep::Complete(Completion::Throw(value))),
            Completion::Return(value) => {
                runtime.delete_weak_map_record(&self.0.map, self.0.key)?;
                runtime.set_weak_map_record(&self.0.map, self.0.key, value.clone())?;
                Ok(ComputedStep::Complete(Completion::Return(value)))
            }
        }
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ComputedStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            ComputedStep::Complete(result) => return Ok(result),
            ComputedStep::Call {
                callable,
                arguments,
                resume,
            } => resume.resume(
                runtime,
                runtime.call_internal(realm, &callable, Value::Undefined, &arguments)?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ComputedStep>() <= 64);
