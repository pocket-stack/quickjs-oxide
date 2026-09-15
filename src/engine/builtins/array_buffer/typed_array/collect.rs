//! TypedArray iterable collection keeps the pinned ordinary-call and no-close policy.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::TypedArrayElementKind,
    heap::ContextId,
    object::{CallableRef, ObjectRef, PropertyKey, WellKnownSymbol},
    value::{Value, conversion::NativeConversion},
    vm::Completion,
};

pub(crate) enum TypedIteratorMethodStep {
    Complete(NativeConversion<Option<CallableRef>>),
    Read {
        receiver: Value,
        key: PropertyKey,
        resume: TypedIteratorMethodResume,
    },
}
pub(crate) struct TypedIteratorMethodResume {
    realm: ContextId,
}
impl TypedIteratorMethodStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        source: Value,
    ) -> Result<Self, RuntimeError> {
        if matches!(source, Value::Null | Value::Undefined) {
            return Ok(Self::Complete(NativeConversion::Throw(
                runtime.new_native_error(realm, NativeErrorKind::Type, "cannot get iterator")?,
            )));
        }
        Ok(Self::Read {
            receiver: source,
            key: PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator)),
            resume: TypedIteratorMethodResume { realm },
        })
    }
}
impl TypedIteratorMethodResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<TypedIteratorMethodStep, RuntimeError> {
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(TypedIteratorMethodStep::Complete(NativeConversion::Throw(
                    value,
                )));
            }
        };
        if matches!(value, Value::Null | Value::Undefined) {
            return Ok(TypedIteratorMethodStep::Complete(NativeConversion::Value(
                None,
            )));
        }
        let callable = match value {
            Value::Object(object) => runtime.as_callable(&object)?,
            _ => None,
        };
        Ok(TypedIteratorMethodStep::Complete(match callable {
            Some(value) => NativeConversion::Value(Some(value)),
            None => NativeConversion::Throw(runtime.new_native_error(
                self.realm,
                NativeErrorKind::Type,
                "value is not iterable",
            )?),
        }))
    }
}
pub(crate) fn finish_method(
    runtime: &Runtime,
    realm: ContextId,
    mut step: TypedIteratorMethodStep,
) -> Result<NativeConversion<Option<CallableRef>>, RuntimeError> {
    loop {
        step = match step {
            TypedIteratorMethodStep::Complete(result) => return Ok(result),
            TypedIteratorMethodStep::Read {
                receiver,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_value_property_in_realm(realm, receiver, &key)?,
            )?,
        };
    }
}

pub(crate) enum TypedCollectStep {
    Complete(NativeConversion<Vec<Value>>),
    Call {
        callable: CallableRef,
        receiver: Value,
        resume: TypedCollectResume,
    },
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: TypedCollectResume,
    },
}
enum Phase {
    Factory,
    NextMethod,
    NextResult,
    Done,
    Value,
}
pub(crate) struct TypedCollectResume(Box<TypedCollectResumeState>);
impl std::ops::Deref for TypedCollectResume {
    type Target = TypedCollectResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TypedCollectResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<TypedCollectResume>() <= 8);
pub(crate) struct TypedCollectResumeState {
    realm: ContextId,
    _method: CallableRef,
    iterator: Option<ObjectRef>,
    next: Option<CallableRef>,
    iteration: Option<ObjectRef>,
    done_key: Option<PropertyKey>,
    value_key: Option<PropertyKey>,
    maximum: u64,
    values: Vec<Value>,
    phase: Phase,
}
impl TypedCollectStep {
    pub(crate) fn start(
        realm: ContextId,
        source: Value,
        method: CallableRef,
        element: TypedArrayElementKind,
    ) -> Self {
        Self::Call {
            callable: method.clone(),
            receiver: source,
            resume: TypedCollectResume(Box::new(TypedCollectResumeState {
                realm,
                _method: method,
                iterator: None,
                next: None,
                iteration: None,
                done_key: None,
                value_key: None,
                maximum: super::MAX_ARRAY_BUFFER_LENGTH / u64::from(element.byte_length()),
                values: Vec::new(),
                phase: Phase::Factory,
            })),
        }
    }
}
impl TypedCollectResume {
    fn abrupt(self, value: Value) -> TypedCollectStep {
        TypedCollectStep::Complete(NativeConversion::Throw(value))
    }
    fn fail(self, runtime: &Runtime, message: &str) -> Result<TypedCollectStep, RuntimeError> {
        let error = runtime.new_native_error(self.0.realm, NativeErrorKind::Type, message)?;
        Ok(self.abrupt(error))
    }
    fn next(mut self) -> Result<TypedCollectStep, RuntimeError> {
        self.0.iteration = None;
        self.0.phase = Phase::NextResult;
        Ok(TypedCollectStep::Call {
            callable: self
                .0
                .next
                .as_ref()
                .ok_or(RuntimeError::Invariant(
                    "TypedArray iterator lost cached next",
                ))?
                .clone(),
            receiver: Value::Object(
                self.0
                    .iterator
                    .as_ref()
                    .ok_or(RuntimeError::Invariant(
                        "TypedArray collection lost iterator",
                    ))?
                    .clone(),
            ),
            resume: self,
        })
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<TypedCollectStep, RuntimeError> {
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(self.abrupt(value)),
        };
        match self.0.phase {
            Phase::Factory => {
                let Value::Object(iterator) = value else {
                    return self.fail(runtime, "not an object");
                };
                self.0.iterator = Some(iterator.clone());
                self.0.phase = Phase::NextMethod;
                Ok(TypedCollectStep::Read {
                    object: iterator,
                    key: runtime.intern_property_key("next")?,
                    resume: self,
                })
            }
            Phase::NextMethod => {
                let next = match value {
                    Value::Object(object) => runtime.as_callable(&object)?,
                    _ => None,
                };
                let Some(next) = next else {
                    return self.fail(runtime, "not a function");
                };
                self.0.next = Some(next);
                self.0.done_key = Some(runtime.intern_property_key("done")?);
                self.0.value_key = Some(runtime.intern_property_key("value")?);
                self.next()
            }
            Phase::NextResult => {
                let Value::Object(iteration) = value else {
                    return self.fail(runtime, "iterator must return an object");
                };
                self.0.iteration = Some(iteration.clone());
                self.0.phase = Phase::Done;
                Ok(TypedCollectStep::Read {
                    object: iteration,
                    key: self
                        .0
                        .done_key
                        .as_ref()
                        .ok_or(RuntimeError::Invariant("TypedArray iterator lost done key"))?
                        .clone(),
                    resume: self,
                })
            }
            Phase::Done => {
                if runtime.value_to_boolean(&value)? {
                    return Ok(TypedCollectStep::Complete(NativeConversion::Value(
                        self.0.values,
                    )));
                }
                // This limit is observed before Get(value), unlike the shared
                // generic next consumer. No failure path closes the iterator.
                if self.0.values.len() as u64 == self.0.maximum {
                    let error = runtime.typed_array_invalid_length(self.0.realm)?;
                    return Ok(self.abrupt(error));
                }
                self.0.phase = Phase::Value;
                Ok(TypedCollectStep::Read {
                    object: self
                        .0
                        .iteration
                        .as_ref()
                        .ok_or(RuntimeError::Invariant("TypedArray iterator lost result"))?
                        .clone(),
                    key: self
                        .0
                        .value_key
                        .as_ref()
                        .ok_or(RuntimeError::Invariant(
                            "TypedArray iterator lost value key",
                        ))?
                        .clone(),
                    resume: self,
                })
            }
            Phase::Value => {
                if self.0.values.try_reserve(1).is_err() {
                    let error = runtime.new_native_error(
                        self.0.realm,
                        NativeErrorKind::Internal,
                        "out of memory",
                    )?;
                    return Ok(self.abrupt(error));
                }
                self.0.values.push(value);
                self.next()
            }
        }
    }
}
pub(crate) fn finish_collect(
    runtime: &Runtime,
    realm: ContextId,
    mut step: TypedCollectStep,
) -> Result<NativeConversion<Vec<Value>>, RuntimeError> {
    loop {
        step = match step {
            TypedCollectStep::Complete(result) => return Ok(result),
            TypedCollectStep::Call {
                callable,
                receiver,
                resume,
            } => resume.resume(
                runtime,
                runtime.call_internal(realm, &callable, receiver, &[])?,
            )?,
            TypedCollectStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TypedCollectStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TypedIteratorMethodStep>() <= 64);
