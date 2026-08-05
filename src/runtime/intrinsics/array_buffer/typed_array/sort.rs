//! `%TypedArray%.prototype.sort` and `toSorted`.
//!
//! Pinned QuickJS deliberately does not reuse the generic Array sorter.  A
//! custom comparator receives values decoded from an immutable machine-word
//! snapshot, then a successful sort writes the ordered raw words back into
//! the receiver's final live range.  This preserves NaN payloads and signed
//! zero while making callback-driven resize and detach behavior match the C
//! implementation.

use std::cmp::Ordering;

use crate::runtime::buffer_access::BufferAccessToken;
use crate::runtime::intrinsics::array::{
    QuickJsSortAccessor, quickjs_rqsort_by, quickjs_rqsort_with,
};

use super::*;

#[cfg(test)]
mod tests;

enum TypedArraySortAbort {
    Throw(Value),
    Runtime(RuntimeError),
}

#[derive(Clone, Copy)]
struct CustomTypedArraySortRaw<'a> {
    bytes: &'a [u8],
    element: TypedArrayElementKind,
    width: usize,
}

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
    pub(super) fn call_typed_array_sort(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        // QuickJS validates sort's comparefn before touching the receiver.
        let comparator = match self.native_sort_comparator(realm, arguments)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.sort received a constructor invocation",
            ));
        };
        let target = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = match self.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        match self.sort_typed_array_words(realm, &target, length, comparator.as_ref())? {
            NativeConversion::Value(()) => Ok(Completion::Return(Value::Object(target))),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    pub(super) fn call_typed_array_to_sorted(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.toSorted received a constructor invocation",
            ));
        };

        // Unlike sort, QuickJS brands and copies first.  In particular an
        // invalid comparefn cannot mask a detached or out-of-bounds source.
        let source = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let source_state = self.typed_array_state(&source)?;
        let target = match self.typed_array_copy_to_default(
            realm,
            &source,
            source_state.snapshot.element,
            u64::from(source_state.length),
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let comparator = match self.native_sort_comparator(realm, arguments)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let target_length = self.typed_array_state(&target)?.length;
        match self.sort_typed_array_words(realm, &target, target_length, comparator.as_ref())? {
            NativeConversion::Value(()) => Ok(Completion::Return(Value::Object(target))),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    fn sort_typed_array_words(
        &self,
        realm: ContextId,
        target: &ObjectRef,
        length: u32,
        comparator: Option<&CallableRef>,
    ) -> Result<NativeConversion<()>, RuntimeError> {
        if length < 2 {
            return Ok(NativeConversion::Value(()));
        }
        let initial = self.typed_array_state(target)?;
        if initial.out_of_bounds || initial.length < length {
            return Err(RuntimeError::Invariant(
                "validated TypedArray changed before sort snapshot",
            ));
        }
        if comparator.is_none() {
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
            quickjs_rqsort_with(count, &mut accessor)?;
            return Ok(NativeConversion::Value(()));
        }

        let (raw_bytes, mut indices) = self.snapshot_custom_typed_array_sort(initial, length)?;
        let comparator = comparator.ok_or(RuntimeError::Invariant(
            "custom TypedArray sort lost its comparator",
        ))?;
        let width = usize::from(initial.snapshot.element.byte_length());
        let raw = CustomTypedArraySortRaw {
            bytes: &raw_bytes,
            element: initial.snapshot.element,
            width,
        };

        let result = quickjs_rqsort_by(&mut indices, |indices, left, right| {
            let ordering = match self
                .compare_typed_array_sort_indices(realm, comparator, raw, indices, left, right)
            {
                Ok(NativeConversion::Value(value)) => value,
                Ok(NativeConversion::Throw(value)) => {
                    return Err(TypedArraySortAbort::Throw(value));
                }
                Err(error) => return Err(TypedArraySortAbort::Runtime(error)),
            };
            Ok(ordering)
        });
        match result {
            Ok(()) => {}
            Err(TypedArraySortAbort::Throw(value)) => {
                return Ok(NativeConversion::Throw(value));
            }
            Err(TypedArraySortAbort::Runtime(error)) => return Err(error),
        }

        // A comparator may mutate, shrink, grow, transiently invalidate, or
        // detach the receiver.  QuickJS only consults its final live count:
        // detach/final OOB means no write, shrink clips the prefix, and grow
        // never extends the old snapshot.
        self.write_custom_typed_array_sort(target, &raw_bytes, &indices, width)?;
        Ok(NativeConversion::Value(()))
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

    fn compare_typed_array_sort_indices(
        &self,
        realm: ContextId,
        comparator: &CallableRef,
        raw: CustomTypedArraySortRaw<'_>,
        indices: &[u32],
        left: usize,
        right: usize,
    ) -> Result<NativeConversion<Ordering>, RuntimeError> {
        // TypedArray sort must call even for raw-identical values.  Array's
        // representation-equality shortcut is intentionally not shared.
        let left_index = *indices.get(left).ok_or(RuntimeError::Invariant(
            "TypedArray sort left index was out of bounds",
        ))?;
        let right_index = *indices.get(right).ok_or(RuntimeError::Invariant(
            "TypedArray sort right index was out of bounds",
        ))?;
        let left_value = typed_array_decode(
            raw.element,
            custom_typed_array_sort_word(raw.bytes, raw.width, left_index)?,
        );
        let right_value = typed_array_decode(
            raw.element,
            custom_typed_array_sort_word(raw.bytes, raw.width, right_index)?,
        );
        let result = match self.call_internal(
            realm,
            comparator,
            Value::Undefined,
            &[left_value, right_value],
        )? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let number = if let Value::Int(value) = result {
            f64::from(value)
        } else {
            match self.native_to_number(realm, &result)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => {
                    return Ok(NativeConversion::Throw(value));
                }
            }
        };
        let ordering = if number > 0.0 {
            Ordering::Greater
        } else if number < 0.0 {
            Ordering::Less
        } else {
            left_index.cmp(&right_index)
        };
        Ok(NativeConversion::Value(ordering))
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
            crate::number::from_float16_bits(word_u16(left)),
            crate::number::from_float16_bits(word_u16(right)),
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
