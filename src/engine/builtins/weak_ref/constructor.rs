//! Weak constructors retain validated inputs across new.target prototype lookup.
use super::WeakIntrinsicKind;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::{ContextId, WeakCollectionKey},
    object::{CallableRef, ObjectRef},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{
            ConstructorPrototypeSource, NativeArguments, NativeInvocation,
            prototype::{ProtoSourceStep, finish as finish_source},
        },
    },
};
pub(crate) enum WeakConstructorStep {
    Complete(Completion),
    Prototype {
        new_target: Value,
        resume: WeakConstructorResume,
    },
}
pub(crate) struct WeakConstructorResume(Box<WeakConstructorResumeState>);
impl std::ops::Deref for WeakConstructorResume {
    type Target = WeakConstructorResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for WeakConstructorResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<WeakConstructorResume>() <= 8);
pub(crate) struct WeakConstructorResumeState {
    realm: ContextId,
    input: Input,
}
enum Input {
    WeakRef {
        _value: Value,
        key: WeakCollectionKey,
    },
    FinalizationRegistry(CallableRef),
}
impl WeakConstructorStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: WeakIntrinsicKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(match kind {
                WeakIntrinsicKind::WeakRef => {
                    "WeakRef constructor received the wrong native invocation"
                }
                WeakIntrinsicKind::FinalizationRegistry => {
                    "FinalizationRegistry constructor received the wrong native invocation"
                }
            }));
        };
        if matches!(new_target, Value::Undefined) {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "constructor requires 'new'",
                )?,
            )));
        }
        let value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(match kind {
                WeakIntrinsicKind::WeakRef => "WeakRef target argv was not padded",
                WeakIntrinsicKind::FinalizationRegistry => {
                    "FinalizationRegistry callback argv was not padded"
                }
            }))?;
        let input = match kind {
            WeakIntrinsicKind::WeakRef => {
                let Some(key) = runtime.weak_target_key(&value, "WeakRef target")? else {
                    return runtime
                        .invalid_weak_target(realm, "invalid target")
                        .map(Self::Complete);
                };
                Input::WeakRef { _value: value, key }
            }
            WeakIntrinsicKind::FinalizationRegistry => {
                let callable = match value {
                    Value::Object(object) => runtime.as_callable(&object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    return runtime
                        .invalid_weak_target(realm, "argument must be a function")
                        .map(Self::Complete);
                };
                Input::FinalizationRegistry(callable)
            }
        };
        Ok(Self::Prototype {
            new_target: new_target.clone(),
            resume: WeakConstructorResume(Box::new(WeakConstructorResumeState { realm, input })),
        })
    }
}
impl WeakConstructorResume {
    pub(crate) fn prototype(
        self,
        runtime: &Runtime,
        reply: NativeConversion<ConstructorPrototypeSource>,
    ) -> Result<WeakConstructorStep, RuntimeError> {
        let prototype = match reply {
            NativeConversion::Throw(value) => {
                return Ok(WeakConstructorStep::Complete(Completion::Throw(value)));
            }
            NativeConversion::Value(ConstructorPrototypeSource::Explicit(prototype)) => prototype,
            NativeConversion::Value(ConstructorPrototypeSource::Realm(realm)) => {
                let kind = match &self.0.input {
                    Input::WeakRef { .. } => WeakIntrinsicKind::WeakRef,
                    Input::FinalizationRegistry(_) => WeakIntrinsicKind::FinalizationRegistry,
                };
                ObjectRef::from_borrowed_handle(
                    runtime.clone(),
                    runtime.weak_intrinsic_prototype(realm, kind)?,
                )?
            }
        };
        let object = match self.0.input {
            Input::WeakRef { _value, key } => runtime.new_weak_ref_object(&prototype, key)?,
            Input::FinalizationRegistry(callback) => {
                runtime.new_finalization_registry_object(&prototype, &callback, self.0.realm)?
            }
        };
        Ok(WeakConstructorStep::Complete(Completion::Return(
            Value::Object(object),
        )))
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: WeakConstructorStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            WeakConstructorStep::Complete(result) => return Ok(result),
            WeakConstructorStep::Prototype { new_target, resume } => resume.prototype(
                runtime,
                finish_source(
                    runtime,
                    realm,
                    ProtoSourceStep::start(runtime, realm, new_target)?,
                )?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<WeakConstructorStep>() <= 64);
