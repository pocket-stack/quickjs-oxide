//! ArrayBuffer construction orders lengths, options, and new.target lookup.
use super::MAX_SAFE_INTEGER_I64;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{ObjectRef, PropertyKey},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{
            ConstructorPrototypeSource, NativeArguments, NativeInvocation,
            prototype::{ProtoSourceStep, finish as finish_source},
        },
    },
};
pub(crate) enum BufferConstructorStep {
    Complete(Completion),
    Primitive {
        value: Value,
        resume: BufferConstructorResume,
    },
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: BufferConstructorResume,
    },
    Prototype {
        new_target: Value,
        resume: BufferConstructorResume,
    },
}
pub(crate) struct BufferConstructorResume(Box<BufferConstructorResumeState>);
impl std::ops::Deref for BufferConstructorResume {
    type Target = BufferConstructorResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for BufferConstructorResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<BufferConstructorResume>() <= 8);
pub(crate) struct BufferConstructorResumeState {
    realm: ContextId,
    shared: bool,
    new_target: Value,
    options: Option<ObjectRef>,
    phase: ConstructorPhase,
}
enum ConstructorPhase {
    Length,
    Maximum(u64),
    MaximumNumber(u64),
    Prototype { length: u64, maximum: Option<u64> },
}
impl BufferConstructorStep {
    pub(crate) fn start_shared(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let Self::Primitive { value, mut resume } =
            Self::start(runtime, realm, invocation, arguments)?
        else {
            return Err(RuntimeError::Invariant(
                "buffer constructor initial step was not primitive",
            ));
        };
        resume.shared = true;
        Ok(Self::Primitive { value, resume })
    }

    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer constructor did not receive a constructor invocation",
            ));
        };
        let value = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(
                "ArrayBuffer length argument was not padded",
            ))?
            .clone();
        let options = if arguments.actual_arg_count >= 2 {
            match arguments.readable.get(1) {
                Some(Value::Object(object)) => Some(object.clone()),
                _ => None,
            }
        } else {
            None
        };
        let _ = runtime;
        Ok(Self::Primitive {
            value,
            resume: BufferConstructorResume(Box::new(BufferConstructorResumeState {
                realm,
                shared: false,
                new_target: new_target.clone(),
                options,
                phase: ConstructorPhase::Length,
            })),
        })
    }
}
impl BufferConstructorResume {
    fn lookup(mut self, length: u64, maximum: Option<u64>) -> BufferConstructorStep {
        BufferConstructorStep::Prototype {
            new_target: self.0.new_target.clone(),
            resume: {
                let updated_0 = ConstructorPhase::Prototype { length, maximum };
                self.0.phase = updated_0;
                self
            },
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<BufferConstructorStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(BufferConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            ConstructorPhase::Length => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "ArrayBuffer length conversion returned an object",
                    ));
                }
                let length = match runtime.native_to_index(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(BufferConstructorStep::Complete(Completion::Throw(value)));
                    }
                };
                if let Some(options) = &self.0.options {
                    Ok(BufferConstructorStep::Read {
                        object: options.clone(),
                        key: runtime.intern_property_key("maxByteLength")?,
                        resume: {
                            let updated_0 = ConstructorPhase::Maximum(length);
                            self.0.phase = updated_0;
                            self
                        },
                    })
                } else {
                    Ok(self.lookup(length, None))
                }
            }
            ConstructorPhase::Maximum(length) => {
                if matches!(value, Value::Undefined) {
                    Ok(self.lookup(length, None))
                } else {
                    Ok(BufferConstructorStep::Primitive {
                        value,
                        resume: {
                            let updated_0 = ConstructorPhase::MaximumNumber(length);
                            self.0.phase = updated_0;
                            self
                        },
                    })
                }
            }
            ConstructorPhase::MaximumNumber(length) => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "ArrayBuffer maximum conversion returned an object",
                    ));
                }
                let maximum = match runtime.native_to_int64(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(BufferConstructorStep::Complete(Completion::Throw(value)));
                    }
                };
                if maximum > MAX_SAFE_INTEGER_I64 || length > maximum as u64 {
                    return Ok(BufferConstructorStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Range,
                            "invalid array buffer max length",
                        )?,
                    )));
                }
                Ok(self.lookup(length, Some(maximum as u64)))
            }
            ConstructorPhase::Prototype { .. } => Err(RuntimeError::Invariant(
                "ArrayBuffer prototype request received an untyped reply",
            )),
        }
    }
    pub(crate) fn prototype(
        self,
        runtime: &Runtime,
        result: NativeConversion<ConstructorPrototypeSource>,
    ) -> Result<BufferConstructorStep, RuntimeError> {
        let prototype = match result {
            NativeConversion::Value(ConstructorPrototypeSource::Explicit(value)) => value,
            NativeConversion::Value(ConstructorPrototypeSource::Realm(realm)) => {
                if self.0.shared {
                    runtime.shared_array_buffer_default_prototype(realm)?
                } else {
                    runtime.array_buffer_default_prototype(realm)?
                }
            }
            NativeConversion::Throw(value) => {
                return Ok(BufferConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        let ConstructorPhase::Prototype { length, maximum } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer constructor received an unexpected prototype reply",
            ));
        };
        Ok(BufferConstructorStep::Complete(if self.0.shared {
            runtime.finish_shared_array_buffer_construction(
                self.0.realm,
                prototype,
                length,
                maximum,
            )?
        } else {
            runtime.finish_array_buffer_construction(self.0.realm, prototype, length, maximum)?
        }))
    }
}
pub(in crate::engine::builtins) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: BufferConstructorStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            BufferConstructorStep::Complete(result) => return Ok(result),
            BufferConstructorStep::Primitive { value, resume } => {
                let result = if matches!(value, Value::Object(_)) {
                    runtime.to_primitive(realm, value, ToPrimitiveHint::Number)?
                } else {
                    Completion::Return(value)
                };
                resume.resume(runtime, result)?
            }
            BufferConstructorStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
            BufferConstructorStep::Prototype { new_target, resume } => resume.prototype(
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
const _: () = assert!(std::mem::size_of::<BufferConstructorStep>() <= 64);
