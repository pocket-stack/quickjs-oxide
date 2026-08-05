//! Thread-safe backing storage for `SharedArrayBuffer` wrappers.
//!
//! Pinned QuickJS allocates a growable shared buffer's maximum capacity up
//! front, shares that allocation by reference count, and stores the current
//! byte length on each wrapper. This module preserves those boundaries with a
//! safe `Arc<SharedBackingStore>` and keeps every byte access behind one
//! `Mutex`. It deliberately contains no heap or ECMAScript object edges.

use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

/// Pinned QuickJS rejects ArrayBuffer lengths greater than `INT32_MAX`.
pub const MAX_SHARED_ARRAY_BUFFER_BYTE_LENGTH: u32 = i32::MAX as u32;

const SHARED_COPY_SCRATCH_BYTE_LENGTH: usize = 8 * 1024;

/// Failure of a checked shared-backing-store operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharedMemoryError {
    InvalidLength,
    Allocation,
    NotGrowable,
    CannotShrink,
    RangeOverflow,
    OutOfBounds,
    InvalidWordLength,
}

impl fmt::Display for SharedMemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidLength => "invalid shared buffer length",
            Self::Allocation => "could not allocate shared buffer backing store",
            Self::NotGrowable => "shared buffer is not growable",
            Self::CannotShrink => "shared buffer cannot shrink",
            Self::RangeOverflow => "shared buffer range overflowed",
            Self::OutOfBounds => "shared buffer range is out of bounds",
            Self::InvalidWordLength => "shared buffer word length must be 1, 2, 4, or 8 bytes",
        };
        formatter.write_str(message)
    }
}

impl Error for SharedMemoryError {}

/// One maximum-capacity allocation shared by any number of wrappers.
///
/// The slice length is immutable and equals the maximum capacity selected by
/// the creating wrapper. Only its bytes are mutable, and all access goes
/// through `lock_bytes`.
pub struct SharedBackingStore {
    capacity: u32,
    bytes: Mutex<Box<[u8]>>,
}

impl fmt::Debug for SharedBackingStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedBackingStore")
            .field("capacity", &self.capacity())
            .finish_non_exhaustive()
    }
}

impl SharedBackingStore {
    fn allocate(capacity: u32) -> Result<Self, SharedMemoryError> {
        let capacity = usize::try_from(capacity).map_err(|_| SharedMemoryError::InvalidLength)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| SharedMemoryError::Allocation)?;
        bytes.resize(capacity, 0);
        Ok(Self {
            capacity: u32::try_from(capacity).map_err(|_| SharedMemoryError::InvalidLength)?,
            bytes: Mutex::new(bytes.into_boxed_slice()),
        })
    }

    fn lock_bytes(&self) -> MutexGuard<'_, Box<[u8]>> {
        // A panic in an internal leaf callback must not permanently make the
        // backing inaccessible. Recovering the guard is safe because byte
        // slices have no structural invariant beyond their immutable length.
        self.bytes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Maximum number of bytes committed in this backing store.
    #[must_use]
    pub const fn capacity(&self) -> u32 {
        self.capacity
    }
}

/// A safe cross-thread handle for one `SharedArrayBuffer` wrapper.
///
/// Clones share bytes but copy `byte_length` and `max_byte_length`, matching
/// pinned QuickJS's wrapper-local grow behavior. The handle contains no heap
/// identity and is therefore safe to send independently of the runtime.
#[derive(Clone)]
pub struct SharedBufferHandle {
    backing: Arc<SharedBackingStore>,
    byte_length: u32,
    max_byte_length: Option<u32>,
}

impl fmt::Debug for SharedBufferHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SharedBufferHandle")
            .field("byte_length", &self.byte_length)
            .field("max_byte_length", &self.max_byte_length)
            .field("backing_capacity", &self.backing.capacity())
            .finish()
    }
}

impl PartialEq for SharedBufferHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.backing, &other.backing)
            && self.byte_length == other.byte_length
            && self.max_byte_length == other.max_byte_length
    }
}

impl Eq for SharedBufferHandle {}

impl SharedBufferHandle {
    /// Allocate a fixed or growable zero-filled shared backing store.
    pub fn new(byte_length: u32, max_byte_length: Option<u32>) -> Result<Self, SharedMemoryError> {
        let capacity = max_byte_length.unwrap_or(byte_length);
        if byte_length > capacity || capacity > MAX_SHARED_ARRAY_BUFFER_BYTE_LENGTH {
            return Err(SharedMemoryError::InvalidLength);
        }
        Ok(Self {
            backing: Arc::new(SharedBackingStore::allocate(capacity)?),
            byte_length,
            max_byte_length,
        })
    }

    /// Current byte length visible through this wrapper.
    #[must_use]
    pub const fn byte_length(&self) -> u32 {
        self.byte_length
    }

    /// The optional wrapper-local maximum. `None` denotes a fixed wrapper.
    #[must_use]
    pub const fn max_byte_length_option(&self) -> Option<u32> {
        self.max_byte_length
    }

    /// Observable `maxByteLength`, which equals `byteLength` when fixed.
    #[must_use]
    pub const fn max_byte_length(&self) -> u32 {
        match self.max_byte_length {
            Some(maximum) => maximum,
            None => self.byte_length,
        }
    }

    /// Whether this wrapper may grow.
    #[must_use]
    pub const fn is_growable(&self) -> bool {
        self.max_byte_length.is_some()
    }

    /// Maximum capacity committed by the shared backing store.
    #[must_use]
    pub fn backing_capacity(&self) -> u32 {
        self.backing.capacity()
    }

    /// Whether two wrappers refer to the same shared bytes.
    #[must_use]
    pub fn shares_backing_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.backing, &other.backing)
    }

    /// Grow this wrapper's visible byte range without reallocating backing.
    pub fn grow(&mut self, new_byte_length: u32) -> Result<(), SharedMemoryError> {
        let Some(maximum) = self.max_byte_length else {
            return Err(SharedMemoryError::NotGrowable);
        };
        if new_byte_length < self.byte_length {
            return Err(SharedMemoryError::CannotShrink);
        }
        if new_byte_length > maximum {
            return Err(SharedMemoryError::OutOfBounds);
        }
        self.byte_length = new_byte_length;
        Ok(())
    }

    /// Create a fixed wrapper with an independent backing copy of one range.
    pub fn slice(&self, start: u32, byte_length: u32) -> Result<Self, SharedMemoryError> {
        self.checked_range(start, byte_length)?;
        let result = Self::new(byte_length, None)?;
        result.copy_range_from(self, start, 0, byte_length)?;
        Ok(result)
    }

    /// Copy a visible range into owned storage.
    pub fn read_range(
        &self,
        byte_offset: u32,
        byte_length: u32,
    ) -> Result<Vec<u8>, SharedMemoryError> {
        self.checked_range(byte_offset, byte_length)?;
        let byte_length_usize =
            usize::try_from(byte_length).map_err(|_| SharedMemoryError::RangeOverflow)?;
        let mut copy = Vec::new();
        copy.try_reserve_exact(byte_length_usize)
            .map_err(|_| SharedMemoryError::Allocation)?;
        copy.resize(byte_length_usize, 0);
        self.with_range(byte_offset, byte_length, |bytes| {
            copy.copy_from_slice(bytes);
        })?;
        Ok(copy)
    }

    /// Copy owned bytes into a visible range.
    pub fn write_range(&self, byte_offset: u32, bytes: &[u8]) -> Result<(), SharedMemoryError> {
        let byte_length =
            u32::try_from(bytes.len()).map_err(|_| SharedMemoryError::RangeOverflow)?;
        self.with_range_mut(byte_offset, byte_length, |target| {
            target.copy_from_slice(bytes)
        })
    }

    /// Copy one visible source range into this wrapper's visible range.
    ///
    /// Both wrapper-local ranges are validated before either backing is
    /// locked. Two wrappers over the same backing use one lock and
    /// `copy_within`, preserving memmove semantics for overlap. Distinct
    /// backings are copied through an 8 KiB scratch buffer, releasing the
    /// source lock before taking the target lock so opposite-direction copies
    /// cannot deadlock.
    pub fn copy_range_from(
        &self,
        source: &Self,
        source_byte_offset: u32,
        target_byte_offset: u32,
        byte_length: u32,
    ) -> Result<(), SharedMemoryError> {
        let source_range = source.checked_range(source_byte_offset, byte_length)?;
        let target_range = self.checked_range(target_byte_offset, byte_length)?;
        if byte_length == 0 {
            return Ok(());
        }

        if self.shares_backing_with(source) {
            let mut bytes = self.backing.lock_bytes();
            bytes.copy_within(source_range, target_range.start);
            return Ok(());
        }

        let mut scratch = [0_u8; SHARED_COPY_SCRATCH_BYTE_LENGTH];
        let mut copied = 0_usize;
        while copied < source_range.len() {
            let chunk_length = (source_range.len() - copied).min(scratch.len());
            let source_start = source_range.start + copied;
            let source_end = source_start + chunk_length;
            {
                let bytes = source.backing.lock_bytes();
                scratch[..chunk_length].copy_from_slice(&bytes[source_start..source_end]);
            }

            let target_start = target_range.start + copied;
            let target_end = target_start + chunk_length;
            {
                let mut bytes = self.backing.lock_bytes();
                bytes[target_start..target_end].copy_from_slice(&scratch[..chunk_length]);
            }
            copied += chunk_length;
        }
        Ok(())
    }

    /// Copy one fixed-width word out of the shared store.
    pub fn read_word(
        &self,
        byte_offset: u32,
        byte_length: u8,
    ) -> Result<[u8; 8], SharedMemoryError> {
        if !matches!(byte_length, 1 | 2 | 4 | 8) {
            return Err(SharedMemoryError::InvalidWordLength);
        }
        let mut word = [0_u8; 8];
        self.with_range(byte_offset, u32::from(byte_length), |bytes| {
            word[..usize::from(byte_length)].copy_from_slice(bytes);
        })?;
        Ok(word)
    }

    /// Copy one fixed-width word into the shared store.
    pub fn write_word(&self, byte_offset: u32, bytes: &[u8]) -> Result<(), SharedMemoryError> {
        if !matches!(bytes.len(), 1 | 2 | 4 | 8) {
            return Err(SharedMemoryError::InvalidWordLength);
        }
        self.write_range(byte_offset, bytes)
    }

    /// Run a pure byte-only operation while holding the backing lock.
    ///
    /// Callers must not enter JavaScript, borrow runtime state, allocate, or
    /// recursively access this backing from `operation`.
    pub(crate) fn with_range<R>(
        &self,
        byte_offset: u32,
        byte_length: u32,
        operation: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, SharedMemoryError> {
        let range = self.checked_range(byte_offset, byte_length)?;
        let bytes = self.backing.lock_bytes();
        Ok(operation(&bytes[range]))
    }

    /// Run a pure byte-only operation while holding the backing lock.
    ///
    /// Callers must not enter JavaScript, borrow runtime state, allocate, or
    /// recursively access this backing from `operation`.
    pub(crate) fn with_range_mut<R>(
        &self,
        byte_offset: u32,
        byte_length: u32,
        operation: impl FnOnce(&mut [u8]) -> R,
    ) -> Result<R, SharedMemoryError> {
        let range = self.checked_range(byte_offset, byte_length)?;
        let mut bytes = self.backing.lock_bytes();
        Ok(operation(&mut bytes[range]))
    }

    fn checked_range(
        &self,
        byte_offset: u32,
        byte_length: u32,
    ) -> Result<std::ops::Range<usize>, SharedMemoryError> {
        let byte_end = byte_offset
            .checked_add(byte_length)
            .ok_or(SharedMemoryError::RangeOverflow)?;
        if byte_end > self.byte_length {
            return Err(SharedMemoryError::OutOfBounds);
        }
        let start = usize::try_from(byte_offset).map_err(|_| SharedMemoryError::RangeOverflow)?;
        let end = usize::try_from(byte_end).map_err(|_| SharedMemoryError::RangeOverflow)?;
        Ok(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::{SharedBufferHandle, SharedMemoryError};
    use std::sync::{Arc, Barrier};

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn maximum_capacity_is_zero_filled_and_all_bytes_are_shared() {
        assert_send_sync::<SharedBufferHandle>();
        let mut first = SharedBufferHandle::new(2, Some(8)).unwrap();
        let second = first.clone();
        first.write_range(0, &[11, 22]).unwrap();
        assert_eq!(second.read_range(0, 2).unwrap(), [11, 22]);

        first.grow(8).unwrap();
        assert_eq!(first.read_range(0, 8).unwrap(), [11, 22, 0, 0, 0, 0, 0, 0]);
        assert_eq!(first.backing_capacity(), 8);
    }

    #[test]
    fn wrapper_growth_is_local_to_each_clone() {
        let mut longer = SharedBufferHandle::new(2, Some(6)).unwrap();
        let mut shorter = longer.clone();
        longer.grow(6).unwrap();
        shorter.grow(4).unwrap();
        assert_eq!(longer.byte_length(), 6);
        assert_eq!(shorter.byte_length(), 4);
        assert_eq!(
            shorter.read_range(4, 1),
            Err(SharedMemoryError::OutOfBounds)
        );
        assert!(longer.shares_backing_with(&shorter));
    }

    #[test]
    fn cloned_handle_crosses_threads_without_heap_state() {
        let handle = SharedBufferHandle::new(4, None).unwrap();
        let worker = handle.clone();
        std::thread::spawn(move || worker.write_word(0, &[1, 2, 3, 4]).unwrap())
            .join()
            .unwrap();
        assert_eq!(handle.read_word(0, 4).unwrap(), [1, 2, 3, 4, 0, 0, 0, 0]);
    }

    #[test]
    fn slice_owns_an_independent_fixed_backing_store() {
        let source = SharedBufferHandle::new(5, Some(8)).unwrap();
        source.write_range(0, &[1, 2, 3, 4, 5]).unwrap();
        let slice = source.slice(1, 3).unwrap();
        assert_eq!(slice.read_range(0, 3).unwrap(), [2, 3, 4]);
        assert!(!slice.shares_backing_with(&source));
        assert!(!slice.is_growable());

        slice.write_range(0, &[9]).unwrap();
        assert_eq!(source.read_range(1, 1).unwrap(), [2]);
    }

    #[test]
    fn same_backing_copy_uses_wrapper_local_ranges_and_memmove_overlap() {
        let mut longer = SharedBufferHandle::new(8, Some(12)).unwrap();
        let shorter = longer.clone();
        longer.grow(12).unwrap();
        longer
            .write_range(0, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])
            .unwrap();

        longer.copy_range_from(&shorter, 0, 2, 8).unwrap();
        assert_eq!(
            longer.read_range(0, 12).unwrap(),
            [0, 1, 0, 1, 2, 3, 4, 5, 6, 7, 10, 11]
        );
        assert_eq!(
            longer.copy_range_from(&shorter, 8, 0, 1),
            Err(SharedMemoryError::OutOfBounds)
        );
        assert_eq!(
            shorter.copy_range_from(&longer, 0, 2, 8),
            Err(SharedMemoryError::OutOfBounds)
        );
    }

    #[test]
    fn cross_backing_copy_spans_multiple_scratch_chunks() {
        const BYTE_LENGTH: usize = 3 * super::SHARED_COPY_SCRATCH_BYTE_LENGTH + 17;
        let source = SharedBufferHandle::new(u32::try_from(BYTE_LENGTH).unwrap(), None).unwrap();
        let target = SharedBufferHandle::new(u32::try_from(BYTE_LENGTH).unwrap(), None).unwrap();
        let expected = (0..BYTE_LENGTH)
            .map(|index| u8::try_from(index % 251).unwrap())
            .collect::<Vec<_>>();
        source.write_range(0, &expected).unwrap();

        target
            .copy_range_from(&source, 0, 0, u32::try_from(BYTE_LENGTH).unwrap())
            .unwrap();
        assert_eq!(
            target
                .read_range(0, u32::try_from(BYTE_LENGTH).unwrap())
                .unwrap(),
            expected
        );
    }

    #[test]
    fn opposite_direction_cross_backing_copies_do_not_deadlock() {
        const BYTE_LENGTH: u32 = 2 * super::SHARED_COPY_SCRATCH_BYTE_LENGTH as u32 + 29;
        let first = Arc::new(SharedBufferHandle::new(BYTE_LENGTH, None).unwrap());
        let second = Arc::new(SharedBufferHandle::new(BYTE_LENGTH, None).unwrap());
        first
            .write_range(0, &vec![0x5a; usize::try_from(BYTE_LENGTH).unwrap()])
            .unwrap();
        second
            .write_range(0, &vec![0xa5; usize::try_from(BYTE_LENGTH).unwrap()])
            .unwrap();
        let start = Arc::new(Barrier::new(2));

        let first_to_second = {
            let first = Arc::clone(&first);
            let second = Arc::clone(&second);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                for _ in 0..128 {
                    second.copy_range_from(&first, 0, 0, BYTE_LENGTH).unwrap();
                }
            })
        };
        let second_to_first = {
            let first = Arc::clone(&first);
            let second = Arc::clone(&second);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                for _ in 0..128 {
                    first.copy_range_from(&second, 0, 0, BYTE_LENGTH).unwrap();
                }
            })
        };

        first_to_second.join().unwrap();
        second_to_first.join().unwrap();
    }

    #[test]
    fn ranges_growth_and_word_widths_fail_closed() {
        assert_eq!(
            SharedBufferHandle::new(5, Some(4)),
            Err(SharedMemoryError::InvalidLength)
        );
        let mut fixed = SharedBufferHandle::new(4, None).unwrap();
        assert_eq!(fixed.grow(4), Err(SharedMemoryError::NotGrowable));

        let mut growable = SharedBufferHandle::new(2, Some(4)).unwrap();
        assert_eq!(growable.grow(1), Err(SharedMemoryError::CannotShrink));
        assert_eq!(growable.grow(5), Err(SharedMemoryError::OutOfBounds));
        assert_eq!(
            growable.read_range(u32::MAX, 2),
            Err(SharedMemoryError::RangeOverflow)
        );
        assert_eq!(
            growable.read_range(2, 1),
            Err(SharedMemoryError::OutOfBounds)
        );
        assert_eq!(
            growable.read_word(0, 3),
            Err(SharedMemoryError::InvalidWordLength)
        );
        assert_eq!(
            growable.write_word(0, &[1, 2, 3]),
            Err(SharedMemoryError::InvalidWordLength)
        );
    }
}
