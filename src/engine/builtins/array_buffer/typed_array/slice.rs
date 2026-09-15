//! Copying and view-producing `%TypedArray%.prototype` algorithms.
//!
//! Pinned QuickJS gives `slice` and `subarray` deliberately different
//! validation points. `slice` snapshots a valid range and copies into a
//! species result, while `subarray` keeps the durable raw view metadata even
//! when a detached or resized source currently reports length zero.

use super::{TypedArraySnapshot, typed_array_absolute_byte_offset};
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::{NativeFunctionId, TypedArrayNativeKind};
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
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
    pub(crate) fn call_typed_array_slice(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            TypedSliceStep::start(self, realm, TypedSliceKind::Slice, &invocation, arguments)?,
        )
    }

    fn finish_typed_slice(
        &self,
        realm: ContextId,
        source: ObjectRef,
        target: ObjectRef,
        start: i64,
        requested_count: u64,
    ) -> Result<Completion, RuntimeError> {
        let source_snapshot = self.typed_array_snapshot(&source)?;
        // QuickJS deliberately skips both post-species validations for an
        // originally empty range.
        if requested_count != 0 {
            let current_source_length = match self.typed_array_validated_length(realm, &source)? {
                NativeConversion::Value(value) => u64::from(value),
                NativeConversion::Throw(value) => {
                    return Ok(Completion::Throw(value));
                }
            };
            match self.typed_array_validated_length(realm, &target)? {
                NativeConversion::Value(_) => {}
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }

            let start = u64::try_from(start)
                .map_err(|_| RuntimeError::Invariant("TypedArray.slice start was negative"))?;
            let live_count = requested_count.min(current_source_length.saturating_sub(start));
            if live_count != 0 {
                let target_snapshot = self.typed_array_snapshot(&target)?;
                if source_snapshot.element == target_snapshot.element {
                    self.typed_array_slice_raw_copy(
                        source_snapshot,
                        target_snapshot,
                        start,
                        live_count,
                    )?;
                } else {
                    for index in 0..live_count {
                        let value = self
                            .typed_array_read_index(&source, start + index)?
                            .unwrap_or(Value::Undefined);
                        match self.typed_array_set_index(realm, &target, index, &value)? {
                            NativeConversion::Value(()) => {}
                            NativeConversion::Throw(value) => {
                                return Ok(Completion::Throw(value));
                            }
                        }
                    }
                }
            }
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    pub(crate) fn call_typed_array_subarray(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            TypedSliceStep::start(
                self,
                realm,
                TypedSliceKind::Subarray,
                &invocation,
                arguments,
            )?,
        )
    }

    fn typed_array_slice_raw_copy(
        &self,
        source: TypedArraySnapshot,
        target: TypedArraySnapshot,
        source_index: u64,
        count: u64,
    ) -> Result<(), RuntimeError> {
        let width = usize::from(source.element.byte_length());
        let source_start = typed_array_absolute_byte_offset(source, source_index)?;
        let target_start = typed_array_absolute_byte_offset(target, 0)?;
        let count = usize::try_from(count)
            .map_err(|_| RuntimeError::Invariant("TypedArray.slice count overflowed usize"))?;
        let byte_count = count.checked_mul(width).ok_or(RuntimeError::Invariant(
            "TypedArray.slice byte count overflowed usize",
        ))?;
        let source_end = source_start
            .checked_add(byte_count)
            .ok_or(RuntimeError::Invariant(
                "TypedArray.slice source range overflowed usize",
            ))?;
        let source_access = self.snapshot_buffer_access(source.buffer)?;
        let target_access = self.snapshot_buffer_access(target.buffer)?;

        // `slice_memcpy` copies overlapping ranges in increasing byte order.
        // With same-class aligned views, one element word at a time is
        // equivalent and lets earlier writes feed later reads as mandated.
        // Always using that path also covers distinct SharedArrayBuffer
        // wrappers which alias one backing store without exposing backing
        // identity through the runtime heap.
        for index in 0..count {
            let relative = index.checked_mul(width).ok_or(RuntimeError::Invariant(
                "TypedArray.slice overlap offset overflowed usize",
            ))?;
            let bytes = self.read_buffer_word(&source_access, source_start + relative, width)?;
            self.write_buffer_word(&target_access, target_start + relative, &bytes[..width])?;
        }
        debug_assert_eq!(source_start + byte_count, source_end);
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub(crate) enum TypedSliceKind {
    Slice,
    Subarray,
}
impl TypedSliceKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        Some(match target {
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Slice) => Self::Slice,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Subarray) => Self::Subarray,
            _ => return None,
        })
    }
}
pub(crate) enum TypedSliceStep {
    Complete(Completion),
    Primitive { resume: TypedSliceResume },
    Species { resume: TypedSliceResume },
    SpeciesView { resume: TypedSliceResume },
}
pub(crate) struct TypedSliceResume(Box<TypedSliceResumeState>);
impl std::ops::Deref for TypedSliceResume {
    type Target = TypedSliceResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TypedSliceResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<TypedSliceResume>() <= 8);
pub(crate) struct TypedSliceResumeState {
    pending_effect: TypedSliceStepPending,
    realm: ContextId,
    source: ObjectRef,
    length: i64,
    kind: TypedSliceKind,
    phase: Phase,
}
enum Phase {
    Start(Value),
    End { start: i64, offset: u64 },
    Species { start: i64, count: u64 },
}
impl TypedSliceStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: TypedSliceKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray slice received a constructor invocation",
            ));
        };
        let source = match runtime.require_typed_array(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let length = match kind {
            TypedSliceKind::Slice => match runtime.typed_array_validated_length(realm, &source)? {
                NativeConversion::Value(value) => i64::from(value),
                NativeConversion::Throw(value) => {
                    return Ok(Self::Complete(Completion::Throw(value)));
                }
            },
            TypedSliceKind::Subarray => i64::from(runtime.typed_array_state(&source)?.length),
        };
        // A branded view's raw metadata is immutable; its backing state is reread after every conversion.
        runtime.typed_array_snapshot(&source)?;
        let end = arguments
            .readable
            .get(1)
            .ok_or(RuntimeError::Invariant(
                "TypedArray slice end argv was not padded",
            ))?
            .clone();
        Ok(Self::request_primitive(
            arguments
                .readable
                .first()
                .ok_or(RuntimeError::Invariant(
                    "TypedArray slice start argv was not padded",
                ))?
                .clone(),
            TypedSliceResume(Box::new(TypedSliceResumeState {
                pending_effect: TypedSliceStepPending::default(),
                realm,
                source,
                length,
                kind,
                phase: Phase::Start(end),
            })),
        ))
    }
}
impl TypedSliceResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<TypedSliceStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(TypedSliceStep::Complete(Completion::Throw(value)));
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
                return Ok(TypedSliceStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Start(end) => {
                let offset = if matches!(self.0.kind, TypedSliceKind::Subarray) {
                    let snapshot = runtime.typed_array_snapshot(&self.0.source)?;
                    u64::from(snapshot.byte_offset)
                        .checked_add(
                            u64::try_from(index)
                                .map_err(|_| {
                                    RuntimeError::Invariant(
                                        "TypedArray.subarray start was negative",
                                    )
                                })?
                                .checked_mul(u64::from(snapshot.element.byte_length()))
                                .ok_or(RuntimeError::Invariant(
                                    "TypedArray.subarray relative offset overflowed u64",
                                ))?,
                        )
                        .ok_or(RuntimeError::Invariant(
                            "TypedArray.subarray byteOffset overflowed u64",
                        ))?
                } else {
                    0
                };
                let next = {
                    let updated_0 = Phase::End {
                        start: index,
                        offset,
                    };
                    self.0.phase = updated_0;
                    self
                };
                if matches!(end, Value::Undefined) {
                    let length = next.length;
                    return next.select(runtime, index, offset, length, true);
                }
                Ok(TypedSliceStep::request_primitive(end, next))
            }
            Phase::End { start, offset } => self.select(runtime, start, offset, index, false),
            Phase::Species { .. } => Err(RuntimeError::Invariant(
                "TypedArray slice primitive reply in species phase",
            )),
        }
    }
    fn select(
        mut self,
        runtime: &Runtime,
        start: i64,
        offset: u64,
        end: i64,
        end_undefined: bool,
    ) -> Result<TypedSliceStep, RuntimeError> {
        let count = u64::try_from((end - start).max(0))
            .map_err(|_| RuntimeError::Invariant("TypedArray.slice count was negative"))?;
        let snapshot = runtime.typed_array_snapshot(&self.0.source)?;
        let source = self.0.source.clone();
        let kind = self.0.kind;
        let resume = {
            let updated_0 = Phase::Species { start, count };
            self.0.phase = updated_0;
            self
        };
        Ok(match kind {
            TypedSliceKind::Slice => {
                TypedSliceStep::request_species(source, snapshot.element, count, resume)
            }
            TypedSliceKind::Subarray => TypedSliceStep::request_species_view(
                source,
                snapshot.element,
                ObjectRef::from_borrowed_handle(runtime.clone(), snapshot.buffer)?,
                offset,
                if end_undefined && snapshot.fixed_byte_length.is_none() {
                    None
                } else {
                    Some(count)
                },
                resume,
            ),
        })
    }
    pub(crate) fn species(
        self,
        runtime: &Runtime,
        result: NativeConversion<ObjectRef>,
    ) -> Result<TypedSliceStep, RuntimeError> {
        let target = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(TypedSliceStep::Complete(Completion::Throw(value)));
            }
        };
        let Phase::Species { start, count } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "TypedArray slice species reply in wrong phase",
            ));
        };
        Ok(TypedSliceStep::Complete(match self.0.kind {
            TypedSliceKind::Slice => {
                runtime.finish_typed_slice(self.0.realm, self.0.source, target, start, count)?
            }
            TypedSliceKind::Subarray => Completion::Return(Value::Object(target)),
        }))
    }
}
fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: TypedSliceStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            TypedSliceStep::Complete(result) => return Ok(result),
            TypedSliceStep::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                {
                    let result = if matches!(value, Value::Object(_)) {
                        runtime.to_primitive(realm, value, ToPrimitiveHint::Number)?
                    } else {
                        Completion::Return(value)
                    };
                    resume.resume(runtime, result)?
                }
            }
            TypedSliceStep::Species { mut resume } => {
                let source = resume.take_species_source();
                let element = resume.take_species_element();
                let length = resume.take_species_length();
                resume.species(
                    runtime,
                    runtime.typed_array_species_create(realm, &source, element, length)?,
                )?
            }
            TypedSliceStep::SpeciesView { mut resume } => {
                let source = resume.take_species_view_source();
                let element = resume.take_species_view_element();
                let buffer = resume.take_species_view_buffer();
                let offset = resume.take_species_view_offset();
                let length = resume.take_species_view_length();
                resume.species(
                    runtime,
                    runtime.typed_array_species_create_subarray(
                        realm, &source, element, &buffer, offset, length,
                    )?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct TypedSliceStepPending {
    primitive_value: Option<Value>,
    species_source: Option<ObjectRef>,
    species_element: Option<TypedArrayElementKind>,
    species_length: Option<u64>,
    species_view_source: Option<ObjectRef>,
    species_view_element: Option<TypedArrayElementKind>,
    species_view_buffer: Option<ObjectRef>,
    species_view_offset: Option<u64>,
    species_view_length: Option<Option<u64>>,
}
impl TypedSliceStep {
    pub(crate) fn request_primitive(value: Value, mut resume: TypedSliceResume) -> Self {
        resume.0.pending_effect.primitive_value = Some(value);
        Self::Primitive { resume }
    }
    pub(crate) fn request_species(
        source: ObjectRef,
        element: TypedArrayElementKind,
        length: u64,
        mut resume: TypedSliceResume,
    ) -> Self {
        resume.0.pending_effect.species_source = Some(source);
        resume.0.pending_effect.species_element = Some(element);
        resume.0.pending_effect.species_length = Some(length);
        Self::Species { resume }
    }
    pub(crate) fn request_species_view(
        source: ObjectRef,
        element: TypedArrayElementKind,
        buffer: ObjectRef,
        offset: u64,
        length: Option<u64>,
        mut resume: TypedSliceResume,
    ) -> Self {
        resume.0.pending_effect.species_view_source = Some(source);
        resume.0.pending_effect.species_view_element = Some(element);
        resume.0.pending_effect.species_view_buffer = Some(buffer);
        resume.0.pending_effect.species_view_offset = Some(offset);
        resume.0.pending_effect.species_view_length = Some(length);
        Self::SpeciesView { resume }
    }
}
impl TypedSliceResume {
    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .pending_effect
            .primitive_value
            .take()
            .expect("TypedSliceStep Primitive value")
    }
    pub(crate) fn take_species_source(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .species_source
            .take()
            .expect("TypedSliceStep Species source")
    }
    pub(crate) fn take_species_element(&mut self) -> TypedArrayElementKind {
        self.0
            .pending_effect
            .species_element
            .take()
            .expect("TypedSliceStep Species element")
    }
    pub(crate) fn take_species_length(&mut self) -> u64 {
        self.0
            .pending_effect
            .species_length
            .take()
            .expect("TypedSliceStep Species length")
    }
    pub(crate) fn take_species_view_source(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .species_view_source
            .take()
            .expect("TypedSliceStep SpeciesView source")
    }
    pub(crate) fn take_species_view_element(&mut self) -> TypedArrayElementKind {
        self.0
            .pending_effect
            .species_view_element
            .take()
            .expect("TypedSliceStep SpeciesView element")
    }
    pub(crate) fn take_species_view_buffer(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .species_view_buffer
            .take()
            .expect("TypedSliceStep SpeciesView buffer")
    }
    pub(crate) fn take_species_view_offset(&mut self) -> u64 {
        self.0
            .pending_effect
            .species_view_offset
            .take()
            .expect("TypedSliceStep SpeciesView offset")
    }
    pub(crate) fn take_species_view_length(&mut self) -> Option<u64> {
        self.0
            .pending_effect
            .species_view_length
            .take()
            .expect("TypedSliceStep SpeciesView length")
    }
}
const _: () = assert!(std::mem::size_of::<TypedSliceStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TypedSliceStep>() <= 64);
