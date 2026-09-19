//! TypedArray.set retains cached range validation across live source callbacks.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::TypedArrayElementKind,
    heap::ContextId,
    object::{ObjectRef, PropertyKey},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum TypedSetStep {
    Complete(Completion),
    Primitive {
        value: Value,
        resume: TypedSetResume,
    },
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: TypedSetResume,
    },
    Element {
        element: TypedArrayElementKind,
        value: Value,
        resume: TypedSetResume,
    },
}
pub(crate) struct TypedSetResume(Box<TypedSetResumeState>);
impl std::ops::Deref for TypedSetResume {
    type Target = TypedSetResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TypedSetResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<TypedSetResume>() <= 8);
pub(crate) struct TypedSetResumeState {
    realm: ContextId,
    phase: SetPhase,
}
struct SetState {
    target: ObjectRef,
    source: ObjectRef,
    target_length: u32,
    offset: u64,
    length: u64,
    index: u64,
}
enum SetPhase {
    Offset { target: ObjectRef, source: Value },
    Length(SetState),
    LengthNumber(SetState),
    Read(SetState),
    Write(SetState),
}
impl TypedSetStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.set received a constructor invocation",
            ));
        };
        let target = match runtime.require_typed_array(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let value = arguments
            .readable
            .get(1)
            .ok_or(RuntimeError::Invariant(
                "TypedArray.set offset argv was not padded",
            ))?
            .clone();
        let source = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(
                "TypedArray.set source argv was not padded",
            ))?
            .clone();
        Ok(Self::Primitive {
            value,
            resume: TypedSetResume(Box::new(TypedSetResumeState {
                realm,
                phase: SetPhase::Offset { target, source },
            })),
        })
    }
}
impl TypedSetResume {
    fn next(
        runtime: &Runtime,
        realm: ContextId,
        state: SetState,
    ) -> Result<TypedSetStep, RuntimeError> {
        if state.index == state.length {
            return Ok(TypedSetStep::Complete(Completion::Return(Value::Undefined)));
        }
        Ok(TypedSetStep::Read {
            object: state.source.clone(),
            key: runtime.property_key_for_index(state.index)?,
            resume: Self(Box::new(TypedSetResumeState {
                realm,
                phase: SetPhase::Read(state),
            })),
        })
    }
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<TypedSetStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(TypedSetStep::Complete(Completion::Throw(value)));
            }
        };
        let realm = self.0.realm;
        match self.0.phase {
            SetPhase::Offset { target, source } => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "TypedArray.set offset conversion returned an object",
                    ));
                }
                let offset = match runtime.native_to_int64_sat(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(TypedSetStep::Complete(Completion::Throw(value)));
                    }
                };
                if offset < 0 {
                    return Ok(TypedSetStep::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(
                            realm,
                            NativeErrorKind::Range,
                            "invalid offset",
                        )?,
                    )));
                }
                let offset = offset as u64;
                let target_length = match runtime.typed_array_validated_length(realm, &target)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(TypedSetStep::Complete(Completion::Throw(value)));
                    }
                };
                if let Value::Object(source_object) = &source
                    && let Some(snapshot) =
                        runtime.typed_array_snapshot_if_branded(source_object)?
                {
                    // Genuine typed elements are Number/BigInt primitives; the
                    // original overlap-preserving copy has no JS callbacks.
                    return Ok(TypedSetStep::Complete(
                        runtime.set_typed_array_from_typed_array(
                            realm,
                            &target,
                            target_length,
                            offset,
                            source_object,
                            snapshot,
                        )?,
                    ));
                }
                let source = match runtime.native_to_object(realm, source)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(TypedSetStep::Complete(Completion::Throw(value)));
                    }
                };
                let key =
                    runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)?;
                Ok(TypedSetStep::Read {
                    object: source.clone(),
                    key,
                    resume: Self(Box::new(TypedSetResumeState {
                        realm,
                        phase: SetPhase::Length(SetState {
                            target,
                            source,
                            target_length,
                            offset,
                            length: 0,
                            index: 0,
                        }),
                    })),
                })
            }
            SetPhase::Length(state) => Ok(TypedSetStep::Primitive {
                value,
                resume: Self(Box::new(TypedSetResumeState {
                    realm,
                    phase: SetPhase::LengthNumber(state),
                })),
            }),
            SetPhase::LengthNumber(mut state) => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "TypedArray.set source length conversion returned an object",
                    ));
                }
                state.length = match runtime.native_to_length(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(TypedSetStep::Complete(Completion::Throw(value)));
                    }
                };
                if state
                    .offset
                    .checked_add(state.length)
                    .is_none_or(|end| end > u64::from(state.target_length))
                {
                    return Ok(TypedSetStep::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(realm, NativeErrorKind::Range, "out of bound")?,
                    )));
                }
                Self::next(runtime, realm, state)
            }
            SetPhase::Read(state) => Ok(TypedSetStep::Element {
                element: runtime.typed_array_snapshot(&state.target)?.element,
                value,
                resume: Self(Box::new(TypedSetResumeState {
                    realm,
                    phase: SetPhase::Write(state),
                })),
            }),
            SetPhase::Write(_) => Err(RuntimeError::Invariant(
                "TypedArray.set element write received an untyped reply",
            )),
        }
    }
    pub(crate) fn element(
        self,
        runtime: &Runtime,
        result: NativeConversion<[u8; 8]>,
    ) -> Result<TypedSetStep, RuntimeError> {
        let bytes = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(TypedSetStep::Complete(Completion::Throw(value)));
            }
        };
        let SetPhase::Write(mut state) = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "TypedArray.set received an unexpected element reply",
            ));
        };
        let _ = runtime.typed_array_write_converted_index(
            &state.target,
            state.offset + state.index,
            &bytes,
        )?;
        state.index += 1;
        Self::next(runtime, self.0.realm, state)
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: TypedSetStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            TypedSetStep::Complete(result) => return Ok(result),
            TypedSetStep::Primitive { value, resume } => {
                let result = if matches!(value, Value::Object(_)) {
                    runtime.to_primitive(realm, value, ToPrimitiveHint::Number)?
                } else {
                    Completion::Return(value)
                };
                resume.resume(runtime, result)?
            }
            TypedSetStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
            TypedSetStep::Element {
                element,
                value,
                resume,
            } => resume.element(
                runtime,
                super::element::ElementStep::start(runtime, realm, element, value)?
                    .finish_sync(runtime, realm)?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TypedSetStep>() <= 64);
