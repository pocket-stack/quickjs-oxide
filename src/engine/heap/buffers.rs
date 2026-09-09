//! Borrow-contained ArrayBuffer storage operations and SharedArrayBuffer backing handles.
//!
//! The runtime performs observable coercions and view validation before entering
//! these operations. Backing bytes stay inside the heap borrow; resize and
//! transfer preserve the source allocation when allocation fails.

use super::{ArrayBufferState, Heap, HeapError, ObjectId, ObjectPayload};
use crate::engine::heap::shared_memory::{SharedBufferHandle, SharedMemoryError};

impl Heap {
    /// Snapshot either ArrayBuffer class without exposing its backing store.
    ///
    /// This is the common view-validation leaf for future DataView and
    /// TypedArray shared-buffer support. SharedArrayBuffers are never detached.
    pub(crate) fn buffer_state(&self, id: ObjectId) -> Result<ArrayBufferState, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::ArrayBuffer(data) => {
                let byte_length = u32::try_from(data.bytes.len()).map_err(|_| {
                    HeapError::Invariant("ArrayBuffer byte length exceeds the supported range")
                })?;
                Ok(ArrayBufferState {
                    byte_length,
                    max_byte_length: data.max_byte_length,
                    detached: data.detached,
                })
            }
            ObjectPayload::SharedArrayBuffer(data) => Ok(ArrayBufferState {
                byte_length: data.handle.byte_length(),
                max_byte_length: data.handle.max_byte_length_option(),
                detached: false,
            }),
            _ => Err(HeapError::Invariant(
                "buffer state reached another object class",
            )),
        }
    }

    /// Clone the sendable backing handle of one genuine SharedArrayBuffer.
    /// The returned wrapper has independent length metadata and no heap edge.
    pub(crate) fn clone_shared_array_buffer_handle(
        &self,
        id: ObjectId,
    ) -> Result<SharedBufferHandle, HeapError> {
        let ObjectPayload::SharedArrayBuffer(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "SharedArrayBuffer handle clone reached another object class",
            ));
        };
        Ok(data.handle.clone())
    }

    /// Grow only the selected SharedArrayBuffer wrapper's visible length.
    pub(crate) fn grow_shared_array_buffer(
        &mut self,
        id: ObjectId,
        new_byte_length: u32,
    ) -> Result<(), HeapError> {
        let ObjectPayload::SharedArrayBuffer(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "SharedArrayBuffer grow reached another object class",
            ));
        };
        data.handle
            .grow(new_byte_length)
            .map_err(shared_memory_heap_error)
    }

    /// Borrow one validated live ArrayBuffer range only for a non-observable
    /// leaf operation. The callback boundary prevents backing bytes from
    /// escaping into runtime code which could re-enter or mutate the heap.
    pub(crate) fn with_array_buffer_range<R>(
        &self,
        id: ObjectId,
        byte_offset: usize,
        byte_length: usize,
        operation: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, HeapError> {
        let ObjectPayload::ArrayBuffer(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "ArrayBuffer range read reached another object class",
            ));
        };
        let byte_end = byte_offset
            .checked_add(byte_length)
            .ok_or(HeapError::Invariant(
                "ArrayBuffer range read overflowed usize",
            ))?;
        if data.detached || byte_end > data.bytes.len() {
            return Err(HeapError::Invariant(
                "ArrayBuffer range read exceeded the live backing store",
            ));
        }
        Ok(operation(&data.bytes[byte_offset..byte_end]))
    }

    /// Mutably borrow one validated live ArrayBuffer range only for a
    /// non-observable leaf operation. All coercions, property access, and view
    /// validation must finish before this callback is entered.
    pub(crate) fn with_array_buffer_range_mut<R>(
        &mut self,
        id: ObjectId,
        byte_offset: usize,
        byte_length: usize,
        operation: impl FnOnce(&mut [u8]) -> R,
    ) -> Result<R, HeapError> {
        let ObjectPayload::ArrayBuffer(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "ArrayBuffer range write reached another object class",
            ));
        };
        let byte_end = byte_offset
            .checked_add(byte_length)
            .ok_or(HeapError::Invariant(
                "ArrayBuffer range write overflowed usize",
            ))?;
        if data.detached || byte_end > data.bytes.len() {
            return Err(HeapError::Invariant(
                "ArrayBuffer range write exceeded the live backing store",
            ));
        }
        Ok(operation(&mut data.bytes[byte_offset..byte_end]))
    }

    /// Copy one 1-, 2-, 4-, or 8-byte ArrayBuffer word into owned storage.
    ///
    /// This API deliberately returns a fixed-size value so runtime numeric
    /// conversion cannot retain a heap borrow across subsequent coercions.
    pub(crate) fn read_array_buffer_word(
        &self,
        id: ObjectId,
        byte_offset: usize,
        byte_length: usize,
    ) -> Result<[u8; 8], HeapError> {
        if !matches!(byte_length, 1 | 2 | 4 | 8) {
            return Err(HeapError::Invariant(
                "ArrayBuffer word read has an unsupported byte length",
            ));
        }
        let ObjectPayload::ArrayBuffer(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "ArrayBuffer word read reached another object class",
            ));
        };
        let byte_end = byte_offset
            .checked_add(byte_length)
            .ok_or(HeapError::Invariant(
                "ArrayBuffer word read range overflowed usize",
            ))?;
        if data.detached || byte_end > data.bytes.len() {
            return Err(HeapError::Invariant(
                "ArrayBuffer word read exceeded the live backing store",
            ));
        }
        let mut word = [0_u8; 8];
        word[..byte_length].copy_from_slice(&data.bytes[byte_offset..byte_end]);
        Ok(word)
    }

    /// Copy one owned 1-, 2-, 4-, or 8-byte word into an ArrayBuffer.
    pub(crate) fn write_array_buffer_word(
        &mut self,
        id: ObjectId,
        byte_offset: usize,
        bytes: &[u8],
    ) -> Result<(), HeapError> {
        if !matches!(bytes.len(), 1 | 2 | 4 | 8) {
            return Err(HeapError::Invariant(
                "ArrayBuffer word write has an unsupported byte length",
            ));
        }
        let ObjectPayload::ArrayBuffer(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "ArrayBuffer word write reached another object class",
            ));
        };
        let byte_end = byte_offset
            .checked_add(bytes.len())
            .ok_or(HeapError::Invariant(
                "ArrayBuffer word write range overflowed usize",
            ))?;
        if data.detached || byte_end > data.bytes.len() {
            return Err(HeapError::Invariant(
                "ArrayBuffer word write exceeded the live backing store",
            ));
        }
        data.bytes[byte_offset..byte_end].copy_from_slice(bytes);
        Ok(())
    }

    /// Resize the owned bytes of one attached ArrayBuffer after the runtime
    /// has performed all observable coercions and range checks.
    pub(crate) fn resize_array_buffer_bytes(
        &mut self,
        id: ObjectId,
        new_length: usize,
    ) -> Result<bool, HeapError> {
        let ObjectPayload::ArrayBuffer(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "ArrayBuffer resize reached another object class",
            ));
        };
        if data.detached {
            return Err(HeapError::Invariant(
                "ArrayBuffer resize reached a detached backing store",
            ));
        }
        if new_length < data.bytes.len() {
            let Some(replacement) = try_shrink_array_buffer_allocation(&data.bytes, new_length)
            else {
                return Ok(false);
            };
            data.bytes = replacement;
        } else if new_length > data.bytes.len() {
            if data
                .bytes
                .try_reserve_exact(new_length - data.bytes.len())
                .is_err()
            {
                return Ok(false);
            }
            data.bytes.resize(new_length, 0);
        }
        Ok(true)
    }

    /// Copy bytes into the prefix of one attached ArrayBuffer. Bounds and
    /// class checks remain inside the heap mutation boundary.
    #[cfg(test)]
    pub(crate) fn write_array_buffer_prefix(
        &mut self,
        id: ObjectId,
        bytes: &[u8],
    ) -> Result<(), HeapError> {
        let ObjectPayload::ArrayBuffer(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "ArrayBuffer write reached another object class",
            ));
        };
        if data.detached || bytes.len() > data.bytes.len() {
            return Err(HeapError::Invariant(
                "ArrayBuffer write exceeded the live backing store",
            ));
        }
        data.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    /// Copy one live ArrayBuffer range into another buffer's prefix without
    /// allocating a second range-sized temporary backing store.
    pub(crate) fn copy_array_buffer_range(
        &mut self,
        source: ObjectId,
        target: ObjectId,
        source_start: usize,
        byte_count: usize,
    ) -> Result<(), HeapError> {
        if source == target {
            return Err(HeapError::Invariant(
                "ArrayBuffer range copy source and target are identical",
            ));
        }
        {
            let ObjectPayload::ArrayBuffer(source_data) = &self.object(source)?.payload else {
                return Err(HeapError::Invariant(
                    "ArrayBuffer range copy source has another object class",
                ));
            };
            let ObjectPayload::ArrayBuffer(target_data) = &self.object(target)?.payload else {
                return Err(HeapError::Invariant(
                    "ArrayBuffer range copy target has another object class",
                ));
            };
            let source_end = source_start
                .checked_add(byte_count)
                .ok_or(HeapError::Invariant(
                    "ArrayBuffer range copy overflowed usize",
                ))?;
            if source_data.detached
                || target_data.detached
                || source_end > source_data.bytes.len()
                || byte_count > target_data.bytes.len()
            {
                return Err(HeapError::Invariant(
                    "ArrayBuffer range copy exceeded a live backing store",
                ));
            }
        }

        const COPY_CHUNK_BYTES: usize = 8 * 1024;
        let mut scratch = [0_u8; COPY_CHUNK_BYTES];
        let mut copied = 0usize;
        while copied < byte_count {
            let chunk_length = (byte_count - copied).min(COPY_CHUNK_BYTES);
            {
                let ObjectPayload::ArrayBuffer(source_data) = &self.object(source)?.payload else {
                    unreachable!("ArrayBuffer range copy source was validated");
                };
                let chunk_start = source_start + copied;
                scratch[..chunk_length]
                    .copy_from_slice(&source_data.bytes[chunk_start..chunk_start + chunk_length]);
            }
            {
                let ObjectPayload::ArrayBuffer(target_data) = &mut self.object_mut(target)?.payload
                else {
                    unreachable!("ArrayBuffer range copy target was validated");
                };
                target_data.bytes[copied..copied + chunk_length]
                    .copy_from_slice(&scratch[..chunk_length]);
            }
            copied += chunk_length;
        }
        Ok(())
    }

    /// Memmove one live byte range between ArrayBuffer backing stores.
    ///
    /// Unlike constructor/slice copying, TypedArray `set` may name overlapping
    /// ranges in the same store and may start at a non-zero target offset.
    /// `copy_within` supplies the allocation-free memmove path for that case.
    pub(crate) fn move_array_buffer_range(
        &mut self,
        source: ObjectId,
        target: ObjectId,
        source_start: usize,
        target_start: usize,
        byte_count: usize,
    ) -> Result<(), HeapError> {
        let source_end = source_start
            .checked_add(byte_count)
            .ok_or(HeapError::Invariant(
                "ArrayBuffer range move source overflowed usize",
            ))?;
        let target_end = target_start
            .checked_add(byte_count)
            .ok_or(HeapError::Invariant(
                "ArrayBuffer range move target overflowed usize",
            ))?;
        {
            let ObjectPayload::ArrayBuffer(source_data) = &self.object(source)?.payload else {
                return Err(HeapError::Invariant(
                    "ArrayBuffer range move source has another object class",
                ));
            };
            let ObjectPayload::ArrayBuffer(target_data) = &self.object(target)?.payload else {
                return Err(HeapError::Invariant(
                    "ArrayBuffer range move target has another object class",
                ));
            };
            if source_data.detached
                || target_data.detached
                || source_end > source_data.bytes.len()
                || target_end > target_data.bytes.len()
            {
                return Err(HeapError::Invariant(
                    "ArrayBuffer range move exceeded a live backing store",
                ));
            }
        }
        if source == target {
            let ObjectPayload::ArrayBuffer(data) = &mut self.object_mut(source)?.payload else {
                unreachable!("ArrayBuffer range move source was validated");
            };
            data.bytes
                .copy_within(source_start..source_end, target_start);
            return Ok(());
        }

        const COPY_CHUNK_BYTES: usize = 8 * 1024;
        let mut scratch = [0_u8; COPY_CHUNK_BYTES];
        let mut copied = 0usize;
        while copied < byte_count {
            let chunk_length = (byte_count - copied).min(COPY_CHUNK_BYTES);
            {
                let ObjectPayload::ArrayBuffer(source_data) = &self.object(source)?.payload else {
                    unreachable!("ArrayBuffer range move source was validated");
                };
                let chunk_start = source_start + copied;
                scratch[..chunk_length]
                    .copy_from_slice(&source_data.bytes[chunk_start..chunk_start + chunk_length]);
            }
            {
                let ObjectPayload::ArrayBuffer(target_data) = &mut self.object_mut(target)?.payload
                else {
                    unreachable!("ArrayBuffer range move target was validated");
                };
                let chunk_start = target_start + copied;
                target_data.bytes[chunk_start..chunk_start + chunk_length]
                    .copy_from_slice(&scratch[..chunk_length]);
            }
            copied += chunk_length;
        }
        Ok(())
    }

    /// Fill a live ArrayBuffer range with repeated fixed-width machine words.
    #[cfg(test)]
    pub(crate) fn fill_array_buffer_words(
        &mut self,
        buffer: ObjectId,
        start: usize,
        word: &[u8],
        count: usize,
    ) -> Result<(), HeapError> {
        if !matches!(word.len(), 1 | 2 | 4 | 8) {
            return Err(HeapError::Invariant(
                "ArrayBuffer fill word has an invalid width",
            ));
        }
        let byte_count = word.len().checked_mul(count).ok_or(HeapError::Invariant(
            "ArrayBuffer fill byte count overflowed usize",
        ))?;
        let end = start.checked_add(byte_count).ok_or(HeapError::Invariant(
            "ArrayBuffer fill range overflowed usize",
        ))?;
        let ObjectPayload::ArrayBuffer(data) = &mut self.object_mut(buffer)?.payload else {
            return Err(HeapError::Invariant(
                "ArrayBuffer fill reached another object class",
            ));
        };
        if data.detached || end > data.bytes.len() {
            return Err(HeapError::Invariant(
                "ArrayBuffer fill exceeded a live backing store",
            ));
        }
        for target in data.bytes[start..end].chunks_exact_mut(word.len()) {
            target.copy_from_slice(word);
        }
        Ok(())
    }

    /// Reverse fixed-width words in one live ArrayBuffer range in place.
    #[cfg(test)]
    pub(crate) fn reverse_array_buffer_words(
        &mut self,
        buffer: ObjectId,
        start: usize,
        word_width: usize,
        count: usize,
    ) -> Result<(), HeapError> {
        if !matches!(word_width, 1 | 2 | 4 | 8) {
            return Err(HeapError::Invariant(
                "ArrayBuffer reverse word has an invalid width",
            ));
        }
        let byte_count = word_width.checked_mul(count).ok_or(HeapError::Invariant(
            "ArrayBuffer reverse byte count overflowed usize",
        ))?;
        let end = start.checked_add(byte_count).ok_or(HeapError::Invariant(
            "ArrayBuffer reverse range overflowed usize",
        ))?;
        let ObjectPayload::ArrayBuffer(data) = &mut self.object_mut(buffer)?.payload else {
            return Err(HeapError::Invariant(
                "ArrayBuffer reverse reached another object class",
            ));
        };
        if data.detached || end > data.bytes.len() {
            return Err(HeapError::Invariant(
                "ArrayBuffer reverse exceeded a live backing store",
            ));
        }
        let words = &mut data.bytes[start..end];
        for left in 0..count / 2 {
            let right = count - left - 1;
            let left_start = left * word_width;
            let right_start = right * word_width;
            for byte in 0..word_width {
                words.swap(left_start + byte, right_start + byte);
            }
        }
        Ok(())
    }

    /// Resize and move one owned backing allocation into an already-created
    /// empty ArrayBuffer. Allocation failure leaves the source attached and
    /// byte-for-byte unchanged.
    pub(crate) fn transfer_array_buffer_bytes(
        &mut self,
        source: ObjectId,
        target: ObjectId,
        new_length: usize,
    ) -> Result<bool, HeapError> {
        if source == target {
            return Err(HeapError::Invariant(
                "ArrayBuffer transfer source and target are identical",
            ));
        }
        {
            let ObjectPayload::ArrayBuffer(source_data) = &self.object(source)?.payload else {
                return Err(HeapError::Invariant(
                    "ArrayBuffer transfer source has another object class",
                ));
            };
            let ObjectPayload::ArrayBuffer(target_data) = &self.object(target)?.payload else {
                return Err(HeapError::Invariant(
                    "ArrayBuffer transfer target has another object class",
                ));
            };
            if source_data.detached
                || target_data.detached
                || !target_data.bytes.is_empty()
                || target_data
                    .max_byte_length
                    .is_some_and(|maximum| new_length > maximum as usize)
            {
                return Err(HeapError::Invariant(
                    "ArrayBuffer transfer reached invalid backing-store state",
                ));
            }
        }

        if new_length == 0 {
            let ObjectPayload::ArrayBuffer(source_data) = &mut self.object_mut(source)?.payload
            else {
                unreachable!("ArrayBuffer transfer source was validated");
            };
            source_data.bytes = Vec::new();
            source_data.detached = true;
            return Ok(true);
        }

        {
            let ObjectPayload::ArrayBuffer(source_data) = &mut self.object_mut(source)?.payload
            else {
                unreachable!("ArrayBuffer transfer source was validated");
            };
            if new_length < source_data.bytes.len() {
                let Some(replacement) =
                    try_shrink_array_buffer_allocation(&source_data.bytes, new_length)
                else {
                    return Ok(false);
                };
                source_data.bytes = replacement;
            } else if new_length > source_data.bytes.len() {
                if source_data
                    .bytes
                    .try_reserve_exact(new_length - source_data.bytes.len())
                    .is_err()
                {
                    return Ok(false);
                }
                source_data.bytes.resize(new_length, 0);
            }
        }
        let bytes = {
            let ObjectPayload::ArrayBuffer(source_data) = &mut self.object_mut(source)?.payload
            else {
                unreachable!("ArrayBuffer transfer source was validated");
            };
            source_data.detached = true;
            std::mem::take(&mut source_data.bytes)
        };
        let ObjectPayload::ArrayBuffer(target_data) = &mut self
            .object_mut(target)
            .expect("validated ArrayBuffer transfer target disappeared")
            .payload
        else {
            unreachable!("ArrayBuffer transfer target was validated");
        };
        target_data.bytes = bytes;
        Ok(true)
    }

    /// Release one ordinary ArrayBuffer backing store and retain its
    /// fixed/resizable metadata. Repeated detach is an idempotent no-op.
    pub(crate) fn detach_array_buffer(&mut self, id: ObjectId) -> Result<(), HeapError> {
        let ObjectPayload::ArrayBuffer(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "ArrayBuffer detach reached another object class",
            ));
        };
        if !data.detached {
            data.bytes = Vec::new();
            data.detached = true;
        }
        Ok(())
    }
}

fn shared_memory_heap_error(error: SharedMemoryError) -> HeapError {
    let message = match error {
        SharedMemoryError::InvalidLength => "SharedArrayBuffer has an invalid length",
        SharedMemoryError::Allocation => "SharedArrayBuffer backing allocation failed",
        SharedMemoryError::NotGrowable => "SharedArrayBuffer wrapper is not growable",
        SharedMemoryError::CannotShrink => "SharedArrayBuffer wrapper cannot shrink",
        SharedMemoryError::RangeOverflow => "SharedArrayBuffer range overflowed",
        SharedMemoryError::OutOfBounds => "SharedArrayBuffer range is out of bounds",
        SharedMemoryError::InvalidWordLength => {
            "SharedArrayBuffer word has an unsupported byte length"
        }
    };
    HeapError::Invariant(message)
}

/// Build a smaller ArrayBuffer allocation without mutating the live source.
///
/// QuickJS uses `realloc` for this transition. Constructing the replacement
/// first gives the same failure atomicity: allocation failure leaves the
/// original bytes and capacity untouched, while success drops the oversized
/// allocation only after its preserved prefix has been copied.
fn try_shrink_array_buffer_allocation(bytes: &[u8], new_length: usize) -> Option<Vec<u8>> {
    debug_assert!(new_length < bytes.len());
    let mut replacement = Vec::new();
    replacement.try_reserve_exact(new_length).ok()?;
    replacement.resize(new_length, 0);
    replacement.copy_from_slice(&bytes[..new_length]);
    Some(replacement)
}
