//! Publish generator instances after the parameter frame has left InitialYield.
use super::{EncodedVmActivation, VmRunOutcome};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::heap::ContextId;
use crate::engine::object::{CallableRef, ObjectRef, PropertyKey};
use crate::engine::value::{Value, conversion::NativeConversion};
use crate::engine::vm::{Completion, VmSuspendKind};

pub(in crate::engine::vm) struct GeneratorCreation {
    pub realm: ContextId,
    pub callable: CallableRef,
    pub asynchronous: bool,
}

pub(in crate::engine::vm) struct GeneratorPrototype {
    creation: GeneratorCreation,
    activation: Box<EncodedVmActivation>,
}

pub(in crate::engine::vm) enum CreationStep {
    Complete(Completion),
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: Box<GeneratorPrototype>,
    },
}

impl GeneratorCreation {
    pub(in crate::engine::vm) fn initial(
        self,
        runtime: &Runtime,
        outcome: VmRunOutcome,
    ) -> Result<CreationStep, RuntimeError> {
        match outcome {
            VmRunOutcome::Complete(Completion::Throw(value)) => {
                Ok(CreationStep::Complete(Completion::Throw(value)))
            }
            VmRunOutcome::Complete(Completion::Return(_)) => Err(RuntimeError::Invariant(
                "generator call completed before its initial-yield barrier",
            )),
            VmRunOutcome::Suspend { activation, .. } => self.frozen(runtime, activation),
        }
    }

    pub(in crate::engine::vm) fn frozen(
        self,
        runtime: &Runtime,
        activation: Box<EncodedVmActivation>,
    ) -> Result<CreationStep, RuntimeError> {
        if activation.kind != VmSuspendKind::Initial {
            return Err(RuntimeError::Invariant(
                "new generator activation is not suspended at start",
            ));
        }
        Ok(CreationStep::Read {
            object: self.callable.as_object().clone(),
            key: runtime.intern_property_key("prototype")?,
            resume: Box::new(GeneratorPrototype {
                creation: self,
                activation,
            }),
        })
    }
}

impl GeneratorPrototype {
    pub(in crate::engine::vm) fn resume(
        self: Box<Self>,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<CreationStep, RuntimeError> {
        let value = match completion {
            Completion::Throw(value) => {
                return Ok(CreationStep::Complete(Completion::Throw(value)));
            }
            Completion::Return(value) => value,
        };
        let prototype = if let Value::Object(prototype) = value {
            prototype
        } else {
            let realm =
                match runtime.function_realm(self.creation.realm, &self.creation.callable)? {
                    NativeConversion::Value(realm) => realm,
                    NativeConversion::Throw(value) => {
                        return Ok(CreationStep::Complete(Completion::Throw(value)));
                    }
                };
            let id = {
                let state = runtime.0.state.borrow();
                let context = state.heap.context(realm)?;
                if self.creation.asynchronous {
                    context
                        .async_generator
                        .ok_or(RuntimeError::Invariant(
                            "generator realm has no async-generator intrinsics",
                        ))?
                        .prototype
                } else {
                    context
                        .generator
                        .ok_or(RuntimeError::Invariant(
                            "generator realm has no generator intrinsics",
                        ))?
                        .prototype
                }
            };
            ObjectRef::from_borrowed_handle(runtime.clone(), id)?
        };
        let generator = if self.creation.asynchronous {
            runtime.allocate_async_generator_object(&prototype, *self.activation)?
        } else {
            runtime.allocate_generator_object(&prototype, *self.activation)?
        };
        Ok(CreationStep::Complete(Completion::Return(Value::Object(
            generator,
        ))))
    }
}

impl CreationStep {
    pub(in crate::engine::vm) fn finish(
        self,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<Completion, RuntimeError> {
        match self {
            Self::Complete(completion) => Ok(completion),
            Self::Read {
                object,
                key,
                resume,
            } => {
                let result = runtime.get_property_in_realm(realm, &object, &key)?;
                match resume.resume(runtime, result)? {
                    Self::Complete(completion) => Ok(completion),
                    Self::Read { .. } => Err(RuntimeError::Invariant(
                        "generator prototype requested a second lookup",
                    )),
                }
            }
        }
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<CreationStep>() <= 64);
