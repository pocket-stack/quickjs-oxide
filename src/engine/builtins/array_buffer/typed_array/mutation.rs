//! In-place `%TypedArray%.prototype` mutation algorithms.
//!
//! Pinned QuickJS operates directly on the branded backing view after one
//! initial length snapshot and a post-coercion bounds revalidation. Keeping
//! these methods together makes those resize/detach rules explicit without
//! growing the runtime facade or the shared constructor/indexed-property owner.

use super::typed_array_absolute_byte_offset;
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::{NativeFunctionId, TypedArrayNativeKind};
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::TypedArrayElementKind,
    heap::ContextId,
    object::ObjectRef,
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{NativeArguments, NativeInvocation},
    },
};

#[cfg(test)]
mod tests;

impl Runtime {
    pub(crate) fn call_typed_array_copy_within(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            TypedMutationStep::start(
                self,
                realm,
                TypedMutationKind::CopyWithin,
                &invocation,
                arguments,
            )?,
        )
    }
    fn finish_typed_copy_within(
        &self,
        realm: ContextId,
        target: ObjectRef,
        initial_length: i64,
        to: i64,
        from: i64,
        final_index: i64,
    ) -> Result<Completion, RuntimeError> {
        let current = self.typed_array_state(&target)?;
        if current.out_of_bounds {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "out of bound",
            )?));
        }
        // QuickJS caps the initial copy count by the live space remaining
        // after hostile coercions resize a length-tracking backing store.
        let live_space = i64::from(current.length) - to.max(from);
        let count = (final_index - from)
            .min(initial_length - to)
            .min(live_space);
        if count > 0 {
            let to = u64::try_from(to).map_err(|_| {
                RuntimeError::Invariant("TypedArray.copyWithin target was negative")
            })?;
            let from = u64::try_from(from).map_err(|_| {
                RuntimeError::Invariant("TypedArray.copyWithin source was negative")
            })?;
            let count = usize::try_from(count).map_err(|_| {
                RuntimeError::Invariant("TypedArray.copyWithin count overflowed usize")
            })?;
            let width = usize::from(current.snapshot.element.byte_length());
            let source_start = typed_array_absolute_byte_offset(current.snapshot, from)?;
            let target_start = typed_array_absolute_byte_offset(current.snapshot, to)?;
            let byte_count = count.checked_mul(width).ok_or(RuntimeError::Invariant(
                "TypedArray.copyWithin byte count overflowed usize",
            ))?;
            let access = self.snapshot_buffer_access(current.snapshot.buffer)?;
            self.move_buffer_range(&access, &access, source_start, target_start, byte_count)?;
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    pub(crate) fn call_typed_array_fill(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            TypedMutationStep::start(self, realm, TypedMutationKind::Fill, &invocation, arguments)?,
        )
    }
    fn finish_typed_fill(
        &self,
        realm: ContextId,
        target: ObjectRef,
        element: TypedArrayElementKind,
        converted: [u8; 8],
        start: i64,
        end: i64,
    ) -> Result<Completion, RuntimeError> {
        let current = self.typed_array_state(&target)?;
        if current.out_of_bounds {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "out of bound",
            )?));
        }
        let end = end.min(i64::from(current.length));
        if start < end {
            let count = usize::try_from(end - start)
                .map_err(|_| RuntimeError::Invariant("TypedArray.fill count overflowed usize"))?;
            let start = u64::try_from(start)
                .map_err(|_| RuntimeError::Invariant("TypedArray.fill start was negative"))?;
            let width = usize::from(element.byte_length());
            let byte_start = typed_array_absolute_byte_offset(current.snapshot, start)?;
            let byte_count = count.checked_mul(width).ok_or(RuntimeError::Invariant(
                "TypedArray.fill byte count overflowed usize",
            ))?;
            let access = self.snapshot_buffer_access(current.snapshot.buffer)?;
            self.with_buffer_range_mut(&access, byte_start, byte_count, |bytes| {
                for target in bytes.chunks_exact_mut(width) {
                    target.copy_from_slice(&converted[..width]);
                }
            })?;
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    pub(crate) fn call_typed_array_reverse(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.reverse received a constructor invocation",
            ));
        };
        let target = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let current = self.typed_array_state(&target)?;
        if current.out_of_bounds {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached or resized",
            )?));
        }
        if current.length > 1 {
            let count = usize::try_from(current.length).map_err(|_| {
                RuntimeError::Invariant("TypedArray.reverse length overflowed usize")
            })?;
            let width = usize::from(current.snapshot.element.byte_length());
            let start = typed_array_absolute_byte_offset(current.snapshot, 0)?;
            let byte_count = count.checked_mul(width).ok_or(RuntimeError::Invariant(
                "TypedArray.reverse byte count overflowed usize",
            ))?;
            let access = self.snapshot_buffer_access(current.snapshot.buffer)?;
            self.with_buffer_range_mut(&access, start, byte_count, |bytes| {
                for left in 0..(count / 2) {
                    let right = count - 1 - left;
                    for byte in 0..width {
                        bytes.swap(left * width + byte, right * width + byte);
                    }
                }
            })?;
        }
        Ok(Completion::Return(Value::Object(target)))
    }
}
#[derive(Clone, Copy)]
pub(crate) enum TypedMutationKind {
    CopyWithin,
    Fill,
}
impl TypedMutationKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        Some(match target {
            NativeFunctionId::TypedArray(TypedArrayNativeKind::CopyWithin) => Self::CopyWithin,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Fill) => Self::Fill,
            _ => return None,
        })
    }
}
pub(crate) enum TypedMutationStep {
    Complete(Completion),
    Primitive {
        value: Value,
        resume: TypedMutationResume,
    },
    Element {
        element: TypedArrayElementKind,
        value: Value,
        resume: TypedMutationResume,
    },
}
pub(crate) struct TypedMutationResume(Box<TypedMutationResumeState>);
impl std::ops::Deref for TypedMutationResume {
    type Target = TypedMutationResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TypedMutationResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<TypedMutationResume>() <= 8);
pub(crate) struct TypedMutationResumeState {
    realm: ContextId,
    target: ObjectRef,
    length: i64,
    phase: Phase,
}
enum Phase {
    To {
        from: Value,
        end: Option<Value>,
    },
    From {
        to: i64,
        end: Option<Value>,
    },
    CopyEnd {
        to: i64,
        from: i64,
    },
    FillValue {
        element: TypedArrayElementKind,
        start: Option<Value>,
        end: Option<Value>,
    },
    FillStart {
        element: TypedArrayElementKind,
        bytes: [u8; 8],
        end: Option<Value>,
    },
    FillEnd {
        element: TypedArrayElementKind,
        bytes: [u8; 8],
        start: i64,
    },
}
impl TypedMutationStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: TypedMutationKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray mutation received a constructor invocation",
            ));
        };
        let target = match runtime.require_typed_array(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let length = match runtime.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => i64::from(value),
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let first = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(
                "TypedArray mutation first argv was not padded",
            ))?
            .clone();
        let end = if arguments.actual_arg_count > 2
            && !matches!(arguments.readable.get(2), Some(Value::Undefined))
        {
            Some(
                arguments
                    .readable
                    .get(2)
                    .ok_or(RuntimeError::Invariant(
                        "TypedArray mutation end argv was missing",
                    ))?
                    .clone(),
            )
        } else {
            None
        };
        Ok(match kind {
            TypedMutationKind::CopyWithin => Self::Primitive {
                value: first,
                resume: TypedMutationResume(Box::new(TypedMutationResumeState {
                    realm,
                    target,
                    length,
                    phase: Phase::To {
                        from: arguments
                            .readable
                            .get(1)
                            .ok_or(RuntimeError::Invariant(
                                "TypedArray.copyWithin start argv was not padded",
                            ))?
                            .clone(),
                        end,
                    },
                })),
            },
            TypedMutationKind::Fill => {
                let element = runtime.typed_array_snapshot(&target)?.element;
                let start = if arguments.actual_arg_count > 1 {
                    Some(
                        arguments
                            .readable
                            .get(1)
                            .ok_or(RuntimeError::Invariant(
                                "TypedArray.fill start argv was missing",
                            ))?
                            .clone(),
                    )
                } else {
                    None
                };
                Self::Element {
                    element,
                    value: first,
                    resume: TypedMutationResume(Box::new(TypedMutationResumeState {
                        realm,
                        target,
                        length,
                        phase: Phase::FillValue {
                            element,
                            start,
                            end,
                        },
                    })),
                }
            }
        })
    }
}
impl TypedMutationResume {
    pub(crate) fn element(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<[u8; 8]>,
    ) -> Result<TypedMutationStep, RuntimeError> {
        let bytes = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(TypedMutationStep::Complete(Completion::Throw(value)));
            }
        };
        let Phase::FillValue {
            element,
            start,
            end,
        } = self.0.phase
        else {
            return Err(RuntimeError::Invariant(
                "TypedArray mutation element reply in wrong phase",
            ));
        };
        let resume = {
            let updated_0 = Phase::FillStart {
                element,
                bytes,
                end,
            };
            self.0.phase = updated_0;
            self
        };
        if let Some(value) = start {
            Ok(TypedMutationStep::Primitive { value, resume })
        } else {
            resume.resume(runtime, Completion::Return(Value::Int(0)))
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<TypedMutationStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(TypedMutationStep::Complete(Completion::Throw(value)));
            }
        };
        let index = match runtime.native_to_int64_clamp(
            self.0.realm,
            &value,
            0,
            self.0.length,
            self.0.length,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(TypedMutationStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::To { from, end } => Ok(TypedMutationStep::Primitive {
                value: from,
                resume: {
                    let updated_0 = Phase::From { to: index, end };
                    self.0.phase = updated_0;
                    self
                },
            }),
            Phase::From { to, end } => {
                if let Some(value) = end {
                    Ok(TypedMutationStep::Primitive {
                        value,
                        resume: {
                            let updated_0 = Phase::CopyEnd { to, from: index };
                            self.0.phase = updated_0;
                            self
                        },
                    })
                } else {
                    Ok(TypedMutationStep::Complete(
                        runtime.finish_typed_copy_within(
                            self.0.realm,
                            self.0.target,
                            self.0.length,
                            to,
                            index,
                            self.0.length,
                        )?,
                    ))
                }
            }
            Phase::CopyEnd { to, from } => Ok(TypedMutationStep::Complete(
                runtime.finish_typed_copy_within(
                    self.0.realm,
                    self.0.target,
                    self.0.length,
                    to,
                    from,
                    index,
                )?,
            )),
            Phase::FillStart {
                element,
                bytes,
                end,
            } => {
                if let Some(value) = end {
                    Ok(TypedMutationStep::Primitive {
                        value,
                        resume: {
                            let updated_0 = Phase::FillEnd {
                                element,
                                bytes,
                                start: index,
                            };
                            self.0.phase = updated_0;
                            self
                        },
                    })
                } else {
                    Ok(TypedMutationStep::Complete(runtime.finish_typed_fill(
                        self.0.realm,
                        self.0.target,
                        element,
                        bytes,
                        index,
                        self.0.length,
                    )?))
                }
            }
            Phase::FillEnd {
                element,
                bytes,
                start,
            } => Ok(TypedMutationStep::Complete(runtime.finish_typed_fill(
                self.0.realm,
                self.0.target,
                element,
                bytes,
                start,
                index,
            )?)),
            Phase::FillValue { .. } => Err(RuntimeError::Invariant(
                "TypedArray mutation primitive reply in element phase",
            )),
        }
    }
}
fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: TypedMutationStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            TypedMutationStep::Complete(result) => return Ok(result),
            TypedMutationStep::Primitive { value, resume } => {
                let result = if matches!(value, Value::Object(_)) {
                    runtime.to_primitive(realm, value, ToPrimitiveHint::Number)?
                } else {
                    Completion::Return(value)
                };
                resume.resume(runtime, result)?
            }
            TypedMutationStep::Element {
                element,
                value,
                resume,
            } => resume.element(
                runtime,
                runtime.typed_array_convert_element(realm, element, &value)?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TypedMutationStep>() <= 64);
