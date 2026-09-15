//! `%TypedArray%.prototype.sort` and `toSorted`.
//!
//! Pinned QuickJS deliberately does not reuse the generic Array sorter.  A
//! custom comparator receives values decoded from an immutable machine-word
//! snapshot, then a successful sort writes the ordered raw words back into
//! the receiver's final live range.  This preserves NaN payloads and signed
//! zero while making callback-driven resize and detach behavior match the C
//! implementation.

use crate::engine::builtins::array::rqsort::{SortAction, SortMachine};
use crate::engine::builtins::array::{QuickJsSortAccessor, quickjs_rqsort_with};
use crate::engine::builtins::buffer_access::BufferAccessToken;
use std::cmp::Ordering;

use super::{TypedArrayState, typed_array_absolute_byte_offset, typed_array_decode};
use crate::engine::{
    api::{
        error::{Error, ErrorKind},
        runtime::Runtime,
        runtime_error::RuntimeError,
    },
    builtins::native::TypedArrayElementKind,
    heap::ContextId,
    object::{CallableRef, ObjectRef},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};

#[cfg(test)]
mod tests;

struct TypedArrayInPlaceSort<'a> {
    runtime: &'a Runtime,
    access: BufferAccessToken,
    element: TypedArrayElementKind,
    start: usize,
    width: usize,
}

impl TypedArrayInPlaceSort<'_> {
    fn offset(&self, index: usize) -> Result<usize, RuntimeError> {
        index
            .checked_mul(self.width)
            .and_then(|offset| self.start.checked_add(offset))
            .ok_or(RuntimeError::Invariant(
                "TypedArray in-place sort offset overflowed usize",
            ))
    }
}

impl QuickJsSortAccessor for TypedArrayInPlaceSort<'_> {
    type Error = RuntimeError;

    fn compare(&mut self, left: usize, right: usize) -> Result<Ordering, RuntimeError> {
        let left_offset = self.offset(left)?;
        let right_offset = self.offset(right)?;
        let range_start = left_offset.min(right_offset);
        let range_length = left_offset
            .max(right_offset)
            .checked_sub(range_start)
            .and_then(|length| length.checked_add(self.width))
            .ok_or(RuntimeError::Invariant(
                "TypedArray in-place sort comparison range overflowed usize",
            ))?;
        self.runtime
            .with_buffer_range(&self.access, range_start, range_length, |bytes| {
                let left =
                    typed_array_sort_slice_word(bytes, left_offset - range_start, self.width)?;
                let right =
                    typed_array_sort_slice_word(bytes, right_offset - range_start, self.width)?;
                Ok(compare_typed_array_words(self.element, &left, &right))
            })?
    }

    fn swap(&mut self, left: usize, right: usize) -> Result<(), RuntimeError> {
        if left == right {
            return Ok(());
        }
        let left_offset = self.offset(left)?;
        let right_offset = self.offset(right)?;
        let range_start = left_offset.min(right_offset);
        let range_length = left_offset
            .max(right_offset)
            .checked_sub(range_start)
            .and_then(|length| length.checked_add(self.width))
            .ok_or(RuntimeError::Invariant(
                "TypedArray in-place sort swap range overflowed usize",
            ))?;
        self.runtime
            .with_buffer_range_mut(&self.access, range_start, range_length, |bytes| {
                let left = left_offset - range_start;
                let right = right_offset - range_start;
                let left_end = left.checked_add(self.width).ok_or(RuntimeError::Invariant(
                    "TypedArray in-place sort left word overflowed usize",
                ))?;
                let right_end = right
                    .checked_add(self.width)
                    .ok_or(RuntimeError::Invariant(
                        "TypedArray in-place sort right word overflowed usize",
                    ))?;
                if left_end > bytes.len() || right_end > bytes.len() {
                    return Err(RuntimeError::Invariant(
                        "TypedArray in-place sort word was out of bounds",
                    ));
                }
                for byte in 0..self.width {
                    bytes.swap(left + byte, right + byte);
                }
                Ok(())
            })?
    }
}

impl Runtime {
    pub(crate) fn call_typed_array_sort(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            TypedSortStep::start(self, realm, false, &invocation, arguments)?,
        )
    }

    pub(crate) fn call_typed_array_to_sorted(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            TypedSortStep::start(self, realm, true, &invocation, arguments)?,
        )
    }

    fn sort_typed_array_words_default(
        &self,
        target: &ObjectRef,
        length: u32,
    ) -> Result<(), RuntimeError> {
        if length < 2 {
            return Ok(());
        }
        let initial = self.typed_array_state(target)?;
        if initial.out_of_bounds || initial.length < length {
            return Err(RuntimeError::Invariant(
                "validated TypedArray changed before sort snapshot",
            ));
        }
        let width = usize::from(initial.snapshot.element.byte_length());
        let count = usize::try_from(length)
            .map_err(|_| RuntimeError::Invariant("TypedArray sort length overflowed usize"))?;
        let access = self.snapshot_buffer_access(initial.snapshot.buffer)?;
        let mut accessor = TypedArrayInPlaceSort {
            runtime: self,
            access,
            element: initial.snapshot.element,
            start: typed_array_absolute_byte_offset(initial.snapshot, 0)?,
            width,
        };
        quickjs_rqsort_with(count, &mut accessor)
    }

    fn snapshot_custom_typed_array_sort(
        &self,
        state: TypedArrayState,
        length: u32,
    ) -> Result<(Vec<u8>, Vec<u32>), RuntimeError> {
        let count = usize::try_from(length)
            .map_err(|_| RuntimeError::Invariant("TypedArray sort length overflowed usize"))?;
        let width = usize::from(state.snapshot.element.byte_length());
        let byte_count = count.checked_mul(width).ok_or(RuntimeError::Invariant(
            "TypedArray sort snapshot byte length overflowed usize",
        ))?;

        // Match QuickJS allocation order and shape: first one exact-width raw
        // element copy, then one uint32 index vector for stable rqsort.
        let mut raw_bytes = Vec::new();
        raw_bytes.try_reserve_exact(byte_count).map_err(|_| {
            RuntimeError::Engine(Error::new(ErrorKind::JsInternal, "out of memory"))
        })?;
        raw_bytes.resize(byte_count, 0);
        let start = typed_array_absolute_byte_offset(state.snapshot, 0)?;
        let access = self.snapshot_buffer_access(state.snapshot.buffer)?;
        self.with_buffer_range(&access, start, byte_count, |source| {
            raw_bytes.copy_from_slice(source)
        })?;

        let mut indices = Vec::new();
        indices.try_reserve_exact(count).map_err(|_| {
            RuntimeError::Engine(Error::new(ErrorKind::JsInternal, "out of memory"))
        })?;
        indices.extend(0..length);
        Ok((raw_bytes, indices))
    }

    fn write_custom_typed_array_sort(
        &self,
        target: &ObjectRef,
        raw_bytes: &[u8],
        indices: &[u32],
        width: usize,
    ) -> Result<(), RuntimeError> {
        let current = self.typed_array_state(target)?;
        if current.out_of_bounds {
            return Ok(());
        }
        if usize::from(current.snapshot.element.byte_length()) != width {
            return Err(RuntimeError::Invariant(
                "TypedArray sort element width changed during comparison",
            ));
        }
        let count = indices.len().min(
            usize::try_from(current.length)
                .map_err(|_| RuntimeError::Invariant("TypedArray sort length overflowed usize"))?,
        );
        let start = typed_array_absolute_byte_offset(current.snapshot, 0)?;
        let byte_length = count.checked_mul(width).ok_or(RuntimeError::Invariant(
            "TypedArray sort write byte length overflowed usize",
        ))?;
        let access = self.snapshot_buffer_access(current.snapshot.buffer)?;
        self.with_buffer_range_mut(&access, start, byte_length, |target| {
            for (destination, source_index) in target
                .chunks_exact_mut(width)
                .zip(indices[..count].iter().copied())
            {
                let source_start = usize::try_from(source_index)
                    .ok()
                    .and_then(|index| index.checked_mul(width))
                    .ok_or(RuntimeError::Invariant(
                        "TypedArray sort source offset overflowed usize",
                    ))?;
                let source_end = source_start
                    .checked_add(width)
                    .ok_or(RuntimeError::Invariant(
                        "TypedArray sort source end overflowed usize",
                    ))?;
                let source =
                    raw_bytes
                        .get(source_start..source_end)
                        .ok_or(RuntimeError::Invariant(
                            "TypedArray sort source word was out of bounds",
                        ))?;
                destination.copy_from_slice(source);
            }
            Ok(())
        })?
    }
}

fn typed_array_sort_slice_word(
    bytes: &[u8],
    byte_offset: usize,
    width: usize,
) -> Result<[u8; 8], RuntimeError> {
    if !matches!(width, 1 | 2 | 4 | 8) {
        return Err(RuntimeError::Invariant(
            "TypedArray sort backing has an invalid element width",
        ));
    }
    let end = byte_offset
        .checked_add(width)
        .ok_or(RuntimeError::Invariant(
            "TypedArray sort backing word end overflowed usize",
        ))?;
    let source = bytes.get(byte_offset..end).ok_or(RuntimeError::Invariant(
        "TypedArray sort backing word was out of bounds",
    ))?;
    let mut word = [0_u8; 8];
    word[..width].copy_from_slice(source);
    Ok(word)
}

fn custom_typed_array_sort_word(
    raw_bytes: &[u8],
    width: usize,
    index: u32,
) -> Result<[u8; 8], RuntimeError> {
    if !matches!(width, 1 | 2 | 4 | 8) {
        return Err(RuntimeError::Invariant(
            "TypedArray sort snapshot has an invalid element width",
        ));
    }
    let start = usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_mul(width))
        .ok_or(RuntimeError::Invariant(
            "TypedArray sort snapshot word offset overflowed usize",
        ))?;
    let end = start.checked_add(width).ok_or(RuntimeError::Invariant(
        "TypedArray sort snapshot word end overflowed usize",
    ))?;
    let source = raw_bytes.get(start..end).ok_or(RuntimeError::Invariant(
        "TypedArray sort snapshot word was out of bounds",
    ))?;
    let mut word = [0_u8; 8];
    word[..width].copy_from_slice(source);
    Ok(word)
}

fn compare_typed_array_words(
    element: TypedArrayElementKind,
    left: &[u8; 8],
    right: &[u8; 8],
) -> Ordering {
    match element {
        TypedArrayElementKind::Int8 => (left[0] as i8).cmp(&(right[0] as i8)),
        TypedArrayElementKind::Uint8 | TypedArrayElementKind::Uint8Clamped => {
            left[0].cmp(&right[0])
        }
        TypedArrayElementKind::Int16 => word_i16(left).cmp(&word_i16(right)),
        TypedArrayElementKind::Uint16 => word_u16(left).cmp(&word_u16(right)),
        TypedArrayElementKind::Int32 => word_i32(left).cmp(&word_i32(right)),
        TypedArrayElementKind::Uint32 => word_u32(left).cmp(&word_u32(right)),
        TypedArrayElementKind::BigInt64 => word_i64(left).cmp(&word_i64(right)),
        TypedArrayElementKind::BigUint64 => word_u64(left).cmp(&word_u64(right)),
        TypedArrayElementKind::Float16 => compare_typed_array_floats(
            crate::engine::value::number::from_float16_bits(word_u16(left)),
            crate::engine::value::number::from_float16_bits(word_u16(right)),
        ),
        TypedArrayElementKind::Float32 => compare_typed_array_floats(
            f64::from(f32::from_bits(word_u32(left))),
            f64::from(f32::from_bits(word_u32(right))),
        ),
        TypedArrayElementKind::Float64 => compare_typed_array_floats(
            f64::from_bits(word_u64(left)),
            f64::from_bits(word_u64(right)),
        ),
    }
}

fn compare_typed_array_floats(left: f64, right: f64) -> Ordering {
    if left.is_nan() {
        return if right.is_nan() {
            Ordering::Equal
        } else {
            Ordering::Greater
        };
    }
    if right.is_nan() {
        return Ordering::Less;
    }
    if left < right {
        return Ordering::Less;
    }
    if left > right {
        return Ordering::Greater;
    }
    if left != 0.0 {
        return Ordering::Equal;
    }
    match (left.is_sign_negative(), right.is_sign_negative()) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

fn word_u16(word: &[u8; 8]) -> u16 {
    u16::from_ne_bytes([word[0], word[1]])
}

fn word_i16(word: &[u8; 8]) -> i16 {
    i16::from_ne_bytes([word[0], word[1]])
}

fn word_u32(word: &[u8; 8]) -> u32 {
    u32::from_ne_bytes([word[0], word[1], word[2], word[3]])
}

fn word_i32(word: &[u8; 8]) -> i32 {
    i32::from_ne_bytes([word[0], word[1], word[2], word[3]])
}

fn word_u64(word: &[u8; 8]) -> u64 {
    u64::from_ne_bytes(*word)
}

fn word_i64(word: &[u8; 8]) -> i64 {
    i64::from_ne_bytes(*word)
}

pub(crate) enum TypedSortStep {
    Complete(Completion),
    Call {
        callable: CallableRef,
        arguments: Vec<Value>,
        resume: TypedSortResume,
    },
    Number {
        value: Value,
        resume: TypedSortResume,
    },
}
enum TypedSortPhase {
    Call,
    Number,
}
pub(crate) struct TypedSortResume(Box<TypedSortResumeState>);
impl std::ops::Deref for TypedSortResume {
    type Target = TypedSortResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TypedSortResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<TypedSortResume>() <= 8);
pub(crate) struct TypedSortResumeState {
    target: ObjectRef,
    comparator: CallableRef,
    raw_bytes: Vec<u8>,
    indices: Vec<u32>,
    element: TypedArrayElementKind,
    width: usize,
    // Keep rqsort's fixed partition stack outside each copied domain reply.
    machine: Box<SortMachine>,
    left_index: u32,
    right_index: u32,
    phase: TypedSortPhase,
}
impl TypedSortStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        copying: bool,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let comparator = if !copying {
            match runtime.native_sort_comparator(realm, arguments)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => {
                    return Ok(Self::Complete(Completion::Throw(value)));
                }
            }
        } else {
            None
        };
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray sort requires generic invocation",
            ));
        };
        let source = match runtime.require_typed_array(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let (target, length, comparator) = if copying {
            let source_state = runtime.typed_array_state(&source)?;
            let target = match runtime.typed_array_copy_to_default(
                realm,
                &source,
                source_state.snapshot.element,
                u64::from(source_state.length),
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => {
                    return Ok(Self::Complete(Completion::Throw(value)));
                }
            };
            let comparator = match runtime.native_sort_comparator(realm, arguments)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => {
                    return Ok(Self::Complete(Completion::Throw(value)));
                }
            };
            let length = runtime.typed_array_state(&target)?.length;
            (target, length, comparator)
        } else {
            let length = match runtime.typed_array_validated_length(realm, &source)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => {
                    return Ok(Self::Complete(Completion::Throw(value)));
                }
            };
            (source, length, comparator)
        };
        if length < 2 {
            return Ok(Self::Complete(Completion::Return(Value::Object(target))));
        }
        let Some(comparator) = comparator else {
            runtime.sort_typed_array_words_default(&target, length)?;
            return Ok(Self::Complete(Completion::Return(Value::Object(target))));
        };
        let initial = runtime.typed_array_state(&target)?;
        if initial.out_of_bounds || initial.length < length {
            return Err(RuntimeError::Invariant(
                "validated TypedArray changed before sort snapshot",
            ));
        }
        let (raw_bytes, indices) = runtime.snapshot_custom_typed_array_sort(initial, length)?;
        let width = usize::from(initial.snapshot.element.byte_length());
        TypedSortResume(Box::new(TypedSortResumeState {
            target,
            comparator,
            raw_bytes,
            indices,
            element: initial.snapshot.element,
            width,
            machine: Box::new(SortMachine::new(length as usize)),
            left_index: 0,
            right_index: 0,
            phase: TypedSortPhase::Call,
        }))
        .next(runtime, None)
    }
}
impl TypedSortResume {
    fn next(
        mut self,
        runtime: &Runtime,
        mut reply: Option<Ordering>,
    ) -> Result<TypedSortStep, RuntimeError> {
        loop {
            match self.0.machine.advance(reply.take()) {
                SortAction::Complete => {
                    // Reacquire the final buffer token only after all callback-driven resize/detach mutations.
                    runtime.write_custom_typed_array_sort(
                        &self.0.target,
                        &self.0.raw_bytes,
                        &self.0.indices,
                        self.0.width,
                    )?;
                    return Ok(TypedSortStep::Complete(Completion::Return(Value::Object(
                        self.0.target,
                    ))));
                }
                SortAction::Swap(left, right) => self.0.indices.swap(left, right),
                SortAction::Compare(left, right) => {
                    self.0.left_index = *self.0.indices.get(left).ok_or(
                        RuntimeError::Invariant("TypedArray sort left index out of bounds"),
                    )?;
                    self.0.right_index = *self.0.indices.get(right).ok_or(
                        RuntimeError::Invariant("TypedArray sort right index out of bounds"),
                    )?;
                    let left_value = typed_array_decode(
                        self.0.element,
                        custom_typed_array_sort_word(
                            &self.0.raw_bytes,
                            self.0.width,
                            self.0.left_index,
                        )?,
                    );
                    let right_value = typed_array_decode(
                        self.0.element,
                        custom_typed_array_sort_word(
                            &self.0.raw_bytes,
                            self.0.width,
                            self.0.right_index,
                        )?,
                    );
                    self.0.phase = TypedSortPhase::Call;
                    return Ok(TypedSortStep::Call {
                        callable: self.0.comparator.clone(),
                        arguments: vec![left_value, right_value],
                        resume: self,
                    });
                }
            }
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<TypedSortStep, RuntimeError> {
        if !matches!(self.0.phase, TypedSortPhase::Call) {
            return Err(RuntimeError::Invariant(
                "TypedArray sort call reply mismatch",
            ));
        }
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(TypedSortStep::Complete(Completion::Throw(value)));
            }
        };
        if let Value::Int(value) = value {
            return self.compared(runtime, f64::from(value));
        }
        self.0.phase = TypedSortPhase::Number;
        Ok(TypedSortStep::Number {
            value,
            resume: self,
        })
    }
    pub(crate) fn number(
        self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<TypedSortStep, RuntimeError> {
        if !matches!(self.0.phase, TypedSortPhase::Number) {
            return Err(RuntimeError::Invariant(
                "TypedArray sort number reply mismatch",
            ));
        }
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(TypedSortStep::Complete(Completion::Throw(value)));
            }
        };
        self.compared(runtime, value)
    }
    fn compared(self, runtime: &Runtime, number: f64) -> Result<TypedSortStep, RuntimeError> {
        let order = if number > 0.0 {
            Ordering::Greater
        } else if number < 0.0 {
            Ordering::Less
        } else {
            self.0.left_index.cmp(&self.0.right_index)
        };
        self.next(runtime, Some(order))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: TypedSortStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            TypedSortStep::Complete(result) => return Ok(result),
            TypedSortStep::Call {
                callable,
                arguments,
                resume,
            } => resume.resume(
                runtime,
                runtime.call_internal(realm, &callable, Value::Undefined, &arguments)?,
            )?,
            TypedSortStep::Number { value, resume } => {
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TypedSortStep>() <= 64);
