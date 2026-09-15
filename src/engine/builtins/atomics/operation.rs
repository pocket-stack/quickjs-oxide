//! Owned conversion phases surround unchanged atomic memory and waiter leaves.
use super::{
    AtomicAccess, AtomicAccessMode, AtomicAccessPreparation, atomic_to_int32_sat,
    atomic_wait_timeout,
};
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::{AtomicsNativeKind, AtomicsOperationKind},
    heap::ContextId,
    value::{Value, conversion::NativeConversion},
    vm::{Completion, ToPrimitiveHint, call::NativeArguments},
};
pub(crate) enum AtomicsStep {
    Complete(Completion),
    Primitive { value: Value, resume: AtomicsResume },
    Number { value: Value, resume: AtomicsResume },
}
enum Phase {
    Index,
    Operand,
    Replacement,
    Size,
    Timeout,
    Count,
}
pub(crate) struct AtomicsResume(Box<AtomicsResumeState>);
impl std::ops::Deref for AtomicsResume {
    type Target = AtomicsResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for AtomicsResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<AtomicsResume>() <= 8);
pub(crate) struct AtomicsResumeState {
    realm: ContextId,
    kind: AtomicsNativeKind,
    arguments: Vec<Value>,
    prepared: Option<AtomicAccessPreparation>,
    access: Option<AtomicAccess>,
    operand: [u8; 8],
    phase: Phase,
}
impl AtomicsStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: AtomicsNativeKind,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        if kind == AtomicsNativeKind::Pause {
            return Ok(Self::Complete(
                runtime.call_atomics_pause(realm, arguments)?,
            ));
        }
        let mut resume = AtomicsResume(Box::new(AtomicsResumeState {
            realm,
            kind,
            arguments: arguments.readable.clone(),
            prepared: None,
            access: None,
            operand: [0; 8],
            phase: Phase::Index,
        }));
        if kind == AtomicsNativeKind::IsLockFree {
            resume.phase = Phase::Size;
            return Ok(Self::Number {
                value: resume.argument(0, "Atomics.isLockFree size was not readable")?,
                resume,
            });
        }
        let mode = resume.mode();
        let view = resume.argument(0, "Atomics TypedArray was not readable")?;
        resume.prepared = Some(match runtime.atomics_prepare_access(realm, &view, mode)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(resume.abrupt(value)),
        });
        Ok(Self::Primitive {
            value: resume.argument(1, "Atomics index was not readable")?,
            resume,
        })
    }
}
impl AtomicsResume {
    fn mode(&self) -> AtomicAccessMode {
        match self.0.kind {
            AtomicsNativeKind::Wait => AtomicAccessMode::Wait,
            AtomicsNativeKind::Notify => AtomicAccessMode::Notify,
            _ => AtomicAccessMode::Operation,
        }
    }
    fn argument(&self, index: usize, message: &'static str) -> Result<Value, RuntimeError> {
        self.0
            .arguments
            .get(index)
            .cloned()
            .ok_or(RuntimeError::Invariant(message))
    }
    fn access(&self) -> Result<&AtomicAccess, RuntimeError> {
        self.0.access.as_ref().ok_or(RuntimeError::Invariant(
            "Atomics conversion lost its access",
        ))
    }
    fn abrupt(self, value: Value) -> AtomicsStep {
        AtomicsStep::Complete(Completion::Throw(value))
    }
    fn modify(
        self,
        runtime: &Runtime,
        operation: AtomicsOperationKind,
        replacement: Option<[u8; 8]>,
    ) -> Result<AtomicsStep, RuntimeError> {
        let access = self.access()?;
        match runtime.atomics_revalidate_after_value(self.0.realm, access)? {
            NativeConversion::Value(()) => {}
            NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
        }
        Ok(AtomicsStep::Complete(Completion::Return(
            runtime.atomics_modify(access, operation, self.0.operand, replacement)?,
        )))
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<AtomicsStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(self.abrupt(value)),
        };
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "Atomics primitive conversion returned an object",
            ));
        }
        match self.0.phase {
            Phase::Index => {
                let index = match runtime.native_to_index(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
                };
                let prepared = self.0.prepared.take().ok_or(RuntimeError::Invariant(
                    "Atomics index lost validation snapshot",
                ))?;
                self.0.access = Some(
                    match runtime.atomics_finish_access(
                        self.0.realm,
                        prepared,
                        index,
                        self.mode(),
                    )? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
                    },
                );
                if self.0.kind == AtomicsNativeKind::Operation(AtomicsOperationKind::Load) {
                    return Ok(AtomicsStep::Complete(Completion::Return(
                        runtime.atomics_load(self.access()?)?,
                    )));
                }
                if self.0.kind == AtomicsNativeKind::Notify {
                    let value = self.argument(2, "Atomics.notify count was not readable")?;
                    if matches!(value, Value::Undefined) {
                        return Ok(AtomicsStep::Complete(
                            runtime.atomics_notify_converted(self.access()?, i32::MAX)?,
                        ));
                    }
                    self.0.phase = Phase::Count;
                    return Ok(AtomicsStep::Number {
                        value,
                        resume: self,
                    });
                }
                self.0.phase = Phase::Operand;
                Ok(AtomicsStep::Primitive {
                    value: self.argument(2, "Atomics operand was not readable")?,
                    resume: self,
                })
            }
            Phase::Operand if self.0.kind == AtomicsNativeKind::Store => {
                let access = self.access()?;
                let stored = if access.snapshot.element.is_bigint() {
                    match runtime.native_to_bigint(self.0.realm, &value)? {
                        NativeConversion::Value(value) => Value::BigInt(value),
                        NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
                    }
                } else {
                    let number = match runtime.native_to_number(self.0.realm, &value)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
                    };
                    let integer = if number.is_nan() {
                        0.0
                    } else {
                        let integer = number.trunc();
                        if integer == 0.0 { 0.0 } else { integer }
                    };
                    Value::number(integer)
                };
                let bytes = match runtime.typed_array_convert_element(
                    self.0.realm,
                    access.snapshot.element,
                    &stored,
                )? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(_) => {
                        return Err(RuntimeError::Invariant(
                            "primitive Atomics.store value failed its second conversion",
                        ));
                    }
                };
                match runtime.atomics_revalidate_after_value(self.0.realm, access)? {
                    NativeConversion::Value(()) => {}
                    NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
                }
                Ok(AtomicsStep::Complete(
                    runtime.atomics_store_converted(access, stored, bytes)?,
                ))
            }
            Phase::Operand => {
                self.0.operand = match runtime.typed_array_convert_element(
                    self.0.realm,
                    self.access()?.snapshot.element,
                    &value,
                )? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
                };
                match self.0.kind {
                    AtomicsNativeKind::Operation(AtomicsOperationKind::CompareExchange) => {
                        self.0.phase = Phase::Replacement;
                        Ok(AtomicsStep::Primitive {
                            value: self.argument(3, "Atomics replacement was not readable")?,
                            resume: self,
                        })
                    }
                    AtomicsNativeKind::Operation(operation) => {
                        self.modify(runtime, operation, None)
                    }
                    AtomicsNativeKind::Wait => {
                        self.0.phase = Phase::Timeout;
                        Ok(AtomicsStep::Number {
                            value: self.argument(3, "Atomics.wait timeout was not readable")?,
                            resume: self,
                        })
                    }
                    _ => Err(RuntimeError::Invariant(
                        "Atomics operand phase kind mismatch",
                    )),
                }
            }
            Phase::Replacement => {
                let replacement = match runtime.typed_array_convert_element(
                    self.0.realm,
                    self.access()?.snapshot.element,
                    &value,
                )? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
                };
                self.modify(
                    runtime,
                    AtomicsOperationKind::CompareExchange,
                    Some(replacement),
                )
            }
            _ => Err(RuntimeError::Invariant("Atomics primitive phase mismatch")),
        }
    }
    pub(crate) fn number(
        self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<AtomicsStep, RuntimeError> {
        let number = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
        };
        Ok(AtomicsStep::Complete(match self.0.phase {
            Phase::Size => Completion::Return(Value::Bool(matches!(
                atomic_to_int32_sat(number),
                1 | 2 | 4 | 8
            ))),
            Phase::Timeout => runtime.atomics_wait_converted(
                self.0.realm,
                self.access()?,
                self.0.operand,
                atomic_wait_timeout(number),
            )?,
            Phase::Count => runtime.atomics_notify_converted(
                self.access()?,
                atomic_to_int32_sat(number).clamp(0, i32::MAX),
            )?,
            _ => return Err(RuntimeError::Invariant("Atomics number phase mismatch")),
        }))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: AtomicsStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            AtomicsStep::Complete(result) => return Ok(result),
            AtomicsStep::Primitive { value, resume } => resume.resume(
                runtime,
                runtime.to_primitive(realm, value, ToPrimitiveHint::Number)?,
            )?,
            AtomicsStep::Number { value, resume } => {
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<AtomicsStep>() <= 64);
