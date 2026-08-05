//! Borrow-free access tokens for ArrayBuffer-family backing stores.
//!
//! Ordinary ArrayBuffer bytes live inside `RuntimeState`, while shared bytes
//! live behind a process-wide mutex. Keeping their access paths separate here
//! makes the lock order structural: a shared backing is never locked while a
//! `RefCell` borrow of the runtime state is alive.

use super::*;
use crate::heap::{BufferState, ObjectKind};
use crate::shared_memory::{SharedBufferHandle, SharedMemoryError};

/// Owned backing identity plus the wrapper state observed at validation time.
///
/// The ordinary variant owns a runtime root, so its arena identity cannot go
/// stale while a non-observable byte leaf is running. The shared variant owns
/// an `Arc` handle and therefore needs no runtime-state borrow while locking
/// its backing store.
pub(in crate::runtime) struct BufferAccessToken {
    pub state: BufferState,
    backing: BufferBackingToken,
}

impl BufferAccessToken {
    /// Whether this token owns a SharedArrayBuffer backing handle.
    pub(in crate::runtime) const fn is_shared(&self) -> bool {
        matches!(&self.backing, BufferBackingToken::Shared(_))
    }

    fn validate_range(&self, byte_offset: usize, byte_length: usize) -> Result<(), RuntimeError> {
        let byte_end = byte_offset
            .checked_add(byte_length)
            .ok_or(RuntimeError::Invariant(
                "ArrayBuffer-family range overflowed usize",
            ))?;
        if self.state.detached
            || byte_end
                > usize::try_from(self.state.byte_length).map_err(|_| {
                    RuntimeError::Invariant("ArrayBuffer-family length overflowed usize")
                })?
        {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer-family range exceeded its live backing store",
            ));
        }
        Ok(())
    }
}

enum BufferBackingToken {
    Ordinary(ObjectRef),
    Shared(SharedBufferHandle),
}

const BUFFER_COPY_SCRATCH_BYTE_LENGTH: usize = 8 * 1024;

impl Runtime {
    /// Snapshot a genuine ArrayBuffer or SharedArrayBuffer by raw heap id.
    ///
    /// `try_borrow_mut` is intentional even though most of this operation is
    /// read-only: it fails closed if a caller attempts to cross the shared
    /// mutex boundary while any runtime-state borrow remains alive.
    pub(in crate::runtime) fn snapshot_buffer_access(
        &self,
        buffer: ObjectId,
    ) -> Result<BufferAccessToken, RuntimeError> {
        self.snapshot_buffer_access_raw(buffer)?
            .ok_or(RuntimeError::Invariant(
                "ArrayBuffer-family access reached another object class",
            ))
    }

    /// Brand-test an externally rooted object without exposing heap borrows.
    pub(in crate::runtime) fn snapshot_buffer_access_if_branded(
        &self,
        object: &ObjectRef,
    ) -> Result<Option<BufferAccessToken>, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("ArrayBuffer-family object"));
        }
        self.snapshot_buffer_access_raw(object.object_id())
    }

    fn snapshot_buffer_access_raw(
        &self,
        buffer: ObjectId,
    ) -> Result<Option<BufferAccessToken>, RuntimeError> {
        enum BackingSeed {
            Ordinary,
            Shared(SharedBufferHandle),
        }

        let (state_snapshot, seed) = {
            let mut state = self.0.state.try_borrow_mut().map_err(|_| {
                RuntimeError::Invariant(
                    "ArrayBuffer-family snapshot attempted during a runtime-state borrow",
                )
            })?;
            let kind = state.heap.object(buffer)?.kind;
            if !matches!(
                kind,
                ObjectKind::ArrayBuffer | ObjectKind::SharedArrayBuffer
            ) {
                return Ok(None);
            }
            // Finish every fallible validation before retaining an ordinary
            // heap root. Nothing after `retain_object` may fail, so an error
            // cannot leak the transferred reference.
            let state_snapshot = state.heap.buffer_state(buffer)?;
            let seed = match kind {
                ObjectKind::ArrayBuffer => {
                    // Transfer one retained heap reference into the ObjectRef
                    // constructed after this RefMut has been dropped.
                    state.heap.retain_object(buffer)?;
                    BackingSeed::Ordinary
                }
                ObjectKind::SharedArrayBuffer => {
                    BackingSeed::Shared(state.heap.clone_shared_array_buffer_handle(buffer)?)
                }
                _ => unreachable!("ArrayBuffer-family kind was checked above"),
            };
            (state_snapshot, seed)
        };

        let backing = match seed {
            BackingSeed::Ordinary => {
                BufferBackingToken::Ordinary(ObjectRef::from_owned_handle(self.clone(), buffer))
            }
            BackingSeed::Shared(handle) => BufferBackingToken::Shared(handle),
        };
        Ok(Some(BufferAccessToken {
            state: state_snapshot,
            backing,
        }))
    }

    /// Run one pure read-only byte operation after all observable work.
    pub(in crate::runtime) fn with_buffer_range<R>(
        &self,
        access: &BufferAccessToken,
        byte_offset: usize,
        byte_length: usize,
        operation: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, RuntimeError> {
        match &access.backing {
            BufferBackingToken::Ordinary(buffer) => {
                let state = self.0.state.try_borrow_mut().map_err(|_| {
                    RuntimeError::Invariant(
                        "ordinary buffer read attempted during a runtime-state borrow",
                    )
                })?;
                Ok(state.heap.with_array_buffer_range(
                    buffer.object_id(),
                    byte_offset,
                    byte_length,
                    operation,
                )?)
            }
            BufferBackingToken::Shared(handle) => {
                let byte_offset = u32::try_from(byte_offset).map_err(|_| {
                    RuntimeError::Invariant("shared buffer byte offset overflowed u32")
                })?;
                let byte_length = u32::try_from(byte_length).map_err(|_| {
                    RuntimeError::Invariant("shared buffer byte length overflowed u32")
                })?;
                handle
                    .with_range(byte_offset, byte_length, operation)
                    .map_err(shared_memory_runtime_error)
            }
        }
    }

    /// Run one pure mutable byte operation after all observable work.
    pub(in crate::runtime) fn with_buffer_range_mut<R>(
        &self,
        access: &BufferAccessToken,
        byte_offset: usize,
        byte_length: usize,
        operation: impl FnOnce(&mut [u8]) -> R,
    ) -> Result<R, RuntimeError> {
        match &access.backing {
            BufferBackingToken::Ordinary(buffer) => {
                let mut state = self.0.state.try_borrow_mut().map_err(|_| {
                    RuntimeError::Invariant(
                        "ordinary buffer write attempted during a runtime-state borrow",
                    )
                })?;
                Ok(state.heap.with_array_buffer_range_mut(
                    buffer.object_id(),
                    byte_offset,
                    byte_length,
                    operation,
                )?)
            }
            BufferBackingToken::Shared(handle) => {
                let byte_offset = u32::try_from(byte_offset).map_err(|_| {
                    RuntimeError::Invariant("shared buffer byte offset overflowed u32")
                })?;
                let byte_length = u32::try_from(byte_length).map_err(|_| {
                    RuntimeError::Invariant("shared buffer byte length overflowed u32")
                })?;
                handle
                    .with_range_mut(byte_offset, byte_length, operation)
                    .map_err(shared_memory_runtime_error)
            }
        }
    }

    /// Read one fixed-width word into owned storage.
    pub(in crate::runtime) fn read_buffer_word(
        &self,
        access: &BufferAccessToken,
        byte_offset: usize,
        byte_length: usize,
    ) -> Result<[u8; 8], RuntimeError> {
        match &access.backing {
            BufferBackingToken::Ordinary(buffer) => {
                let state = self.0.state.try_borrow_mut().map_err(|_| {
                    RuntimeError::Invariant(
                        "ordinary buffer word read attempted during a runtime-state borrow",
                    )
                })?;
                Ok(state.heap.read_array_buffer_word(
                    buffer.object_id(),
                    byte_offset,
                    byte_length,
                )?)
            }
            BufferBackingToken::Shared(handle) => {
                let byte_offset = u32::try_from(byte_offset).map_err(|_| {
                    RuntimeError::Invariant("shared buffer word offset overflowed u32")
                })?;
                let byte_length = u8::try_from(byte_length).map_err(|_| {
                    RuntimeError::Invariant("shared buffer word width overflowed u8")
                })?;
                handle
                    .read_word(byte_offset, byte_length)
                    .map_err(shared_memory_runtime_error)
            }
        }
    }

    /// Write one fixed-width owned word.
    pub(in crate::runtime) fn write_buffer_word(
        &self,
        access: &BufferAccessToken,
        byte_offset: usize,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        match &access.backing {
            BufferBackingToken::Ordinary(buffer) => {
                let mut state = self.0.state.try_borrow_mut().map_err(|_| {
                    RuntimeError::Invariant(
                        "ordinary buffer word write attempted during a runtime-state borrow",
                    )
                })?;
                Ok(state
                    .heap
                    .write_array_buffer_word(buffer.object_id(), byte_offset, bytes)?)
            }
            BufferBackingToken::Shared(handle) => {
                let byte_offset = u32::try_from(byte_offset).map_err(|_| {
                    RuntimeError::Invariant("shared buffer word offset overflowed u32")
                })?;
                handle
                    .write_word(byte_offset, bytes)
                    .map_err(shared_memory_runtime_error)
            }
        }
    }

    /// Memmove one validated byte range between ArrayBuffer-family stores.
    ///
    /// Same-store ordinary and shared copies delegate to their backing's
    /// overlap-aware primitive. Mixed-family stores cannot alias, so they are
    /// copied through a fixed scratch buffer while releasing the source
    /// borrow or lock before acquiring the target one.
    pub(in crate::runtime) fn move_buffer_range(
        &self,
        source: &BufferAccessToken,
        target: &BufferAccessToken,
        source_start: usize,
        target_start: usize,
        byte_length: usize,
    ) -> Result<(), RuntimeError> {
        // Validate both complete ranges before the first byte is written. In
        // particular, a mixed AB/SAB copy must not discover an out-of-bounds
        // final chunk after earlier chunks have already mutated the target.
        source.validate_range(source_start, byte_length)?;
        target.validate_range(target_start, byte_length)?;
        match (&source.backing, &target.backing) {
            (BufferBackingToken::Ordinary(source), BufferBackingToken::Ordinary(target)) => {
                let mut state = self.0.state.try_borrow_mut().map_err(|_| {
                    RuntimeError::Invariant(
                        "ordinary buffer move attempted during a runtime-state borrow",
                    )
                })?;
                Ok(state.heap.move_array_buffer_range(
                    source.object_id(),
                    target.object_id(),
                    source_start,
                    target_start,
                    byte_length,
                )?)
            }
            (BufferBackingToken::Shared(source), BufferBackingToken::Shared(target)) => {
                let source_start = u32::try_from(source_start).map_err(|_| {
                    RuntimeError::Invariant("shared buffer move source offset overflowed u32")
                })?;
                let target_start = u32::try_from(target_start).map_err(|_| {
                    RuntimeError::Invariant("shared buffer move target offset overflowed u32")
                })?;
                let byte_length = u32::try_from(byte_length).map_err(|_| {
                    RuntimeError::Invariant("shared buffer move byte length overflowed u32")
                })?;
                target
                    .copy_range_from(source, source_start, target_start, byte_length)
                    .map_err(shared_memory_runtime_error)
            }
            _ => {
                let mut scratch = [0_u8; BUFFER_COPY_SCRATCH_BYTE_LENGTH];
                let mut copied = 0_usize;
                while copied < byte_length {
                    let chunk_length = (byte_length - copied).min(BUFFER_COPY_SCRATCH_BYTE_LENGTH);
                    self.with_buffer_range(
                        source,
                        source_start
                            .checked_add(copied)
                            .ok_or(RuntimeError::Invariant(
                                "buffer move source range overflowed usize",
                            ))?,
                        chunk_length,
                        |bytes| scratch[..chunk_length].copy_from_slice(bytes),
                    )?;
                    self.with_buffer_range_mut(
                        target,
                        target_start
                            .checked_add(copied)
                            .ok_or(RuntimeError::Invariant(
                                "buffer move target range overflowed usize",
                            ))?,
                        chunk_length,
                        |bytes| bytes.copy_from_slice(&scratch[..chunk_length]),
                    )?;
                    copied += chunk_length;
                }
                Ok(())
            }
        }
    }
}

fn shared_memory_runtime_error(error: SharedMemoryError) -> RuntimeError {
    match error {
        SharedMemoryError::InvalidLength => {
            RuntimeError::Invariant("shared buffer access has an invalid length")
        }
        SharedMemoryError::Allocation => {
            RuntimeError::Invariant("shared buffer access unexpectedly allocated")
        }
        SharedMemoryError::NotGrowable => {
            RuntimeError::Invariant("shared buffer byte access attempted to grow a fixed wrapper")
        }
        SharedMemoryError::CannotShrink => {
            RuntimeError::Invariant("shared buffer byte access attempted to shrink")
        }
        SharedMemoryError::RangeOverflow => {
            RuntimeError::Invariant("shared buffer byte range overflowed")
        }
        SharedMemoryError::OutOfBounds => {
            RuntimeError::Invariant("shared buffer byte range exceeded its wrapper length")
        }
        SharedMemoryError::InvalidWordLength => {
            RuntimeError::Invariant("shared buffer word has an invalid width")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_ordinary_buffer(runtime: &Runtime, bytes: Vec<u8>) -> ObjectRef {
        let mut state = runtime.0.state.borrow_mut();
        let shape = state.get_or_create_shape(None, &[]).unwrap();
        let object = state
            .heap
            .allocate_object(ObjectData::array_buffer_from_bytes(
                shape,
                Vec::new(),
                bytes,
                None,
            ))
            .unwrap();
        let cleanup = state.heap.release_shape(shape).unwrap();
        state.apply_cleanup(cleanup).unwrap();
        drop(state);
        ObjectRef::from_owned_handle(runtime.clone(), object)
    }

    fn new_shared_buffer(runtime: &Runtime, handle: SharedBufferHandle) -> ObjectRef {
        let mut state = runtime.0.state.borrow_mut();
        let shape = state.get_or_create_shape(None, &[]).unwrap();
        let object = state
            .heap
            .allocate_object(ObjectData::shared_array_buffer(shape, Vec::new(), handle))
            .unwrap();
        let cleanup = state.heap.release_shape(shape).unwrap();
        state.apply_cleanup(cleanup).unwrap();
        drop(state);
        ObjectRef::from_owned_handle(runtime.clone(), object)
    }

    #[test]
    fn ordinary_token_owns_a_root_and_preserves_leaf_access() {
        let runtime = Runtime::new();
        let buffer = new_ordinary_buffer(&runtime, vec![1, 2, 3, 4]);
        let access = runtime.snapshot_buffer_access(buffer.object_id()).unwrap();
        drop(buffer);

        runtime.write_buffer_word(&access, 0, &[9, 8]).unwrap();
        assert_eq!(
            runtime.read_buffer_word(&access, 0, 4).unwrap(),
            [9, 8, 3, 4, 0, 0, 0, 0]
        );
    }

    #[test]
    fn shared_token_releases_runtime_state_before_locking_bytes() {
        let runtime = Runtime::new();
        let handle = SharedBufferHandle::new(4, None).unwrap();
        let observer = handle.clone();
        let buffer = new_shared_buffer(&runtime, handle);
        let access = runtime.snapshot_buffer_access(buffer.object_id()).unwrap();

        runtime
            .with_buffer_range_mut(&access, 0, 4, |bytes| {
                bytes.copy_from_slice(&[4, 3, 2, 1]);
            })
            .unwrap();
        assert_eq!(observer.read_range(0, 4).unwrap(), [4, 3, 2, 1]);
        let mut observed = [0_u8; 2];
        runtime
            .with_buffer_range(&access, 1, 2, |bytes| observed.copy_from_slice(bytes))
            .unwrap();
        assert_eq!(observed, [3, 2]);
    }

    #[test]
    fn snapshot_fails_closed_while_runtime_state_is_borrowed() {
        let runtime = Runtime::new();
        let buffer = new_shared_buffer(&runtime, SharedBufferHandle::new(1, None).unwrap());
        let _state = runtime.0.state.borrow();
        assert!(matches!(
            runtime.snapshot_buffer_access(buffer.object_id()),
            Err(RuntimeError::Invariant(
                "ArrayBuffer-family snapshot attempted during a runtime-state borrow"
            ))
        ));
    }

    #[test]
    fn shared_move_preserves_overlap_across_distinct_wrappers() {
        let runtime = Runtime::new();
        let first_handle = SharedBufferHandle::new(8, None).unwrap();
        first_handle
            .write_range(0, &[0, 1, 2, 3, 4, 5, 6, 7])
            .unwrap();
        let second_handle = first_handle.clone();
        let first = new_shared_buffer(&runtime, first_handle.clone());
        let second = new_shared_buffer(&runtime, second_handle);
        let source = runtime.snapshot_buffer_access(first.object_id()).unwrap();
        let target = runtime.snapshot_buffer_access(second.object_id()).unwrap();

        runtime
            .move_buffer_range(&source, &target, 0, 2, 6)
            .unwrap();
        assert_eq!(
            first_handle.read_range(0, 8).unwrap(),
            [0, 1, 0, 1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn mixed_family_move_releases_each_store_before_acquiring_the_other() {
        let runtime = Runtime::new();
        let ordinary = new_ordinary_buffer(&runtime, vec![1, 2, 3, 4, 0, 0]);
        let shared_handle = SharedBufferHandle::new(6, None).unwrap();
        let shared = new_shared_buffer(&runtime, shared_handle.clone());
        let ordinary_access = runtime
            .snapshot_buffer_access(ordinary.object_id())
            .unwrap();
        let shared_access = runtime.snapshot_buffer_access(shared.object_id()).unwrap();

        runtime
            .move_buffer_range(&ordinary_access, &shared_access, 0, 1, 4)
            .unwrap();
        assert_eq!(shared_handle.read_range(0, 6).unwrap(), [0, 1, 2, 3, 4, 0]);

        runtime
            .move_buffer_range(&shared_access, &ordinary_access, 1, 2, 4)
            .unwrap();
        let mut observed = [0_u8; 6];
        runtime
            .with_buffer_range(&ordinary_access, 0, 6, |bytes| {
                observed.copy_from_slice(bytes)
            })
            .unwrap();
        assert_eq!(observed, [1, 2, 1, 2, 3, 4]);
    }

    #[test]
    fn mixed_family_move_validates_the_complete_target_before_writing() {
        let runtime = Runtime::new();
        let ordinary = new_ordinary_buffer(&runtime, vec![7; BUFFER_COPY_SCRATCH_BYTE_LENGTH + 1]);
        let shared_handle =
            SharedBufferHandle::new(BUFFER_COPY_SCRATCH_BYTE_LENGTH as u32, None).unwrap();
        let shared = new_shared_buffer(&runtime, shared_handle.clone());
        let source = runtime
            .snapshot_buffer_access(ordinary.object_id())
            .unwrap();
        let target = runtime.snapshot_buffer_access(shared.object_id()).unwrap();

        assert!(matches!(
            runtime.move_buffer_range(&source, &target, 0, 0, BUFFER_COPY_SCRATCH_BYTE_LENGTH + 1,),
            Err(RuntimeError::Invariant(
                "ArrayBuffer-family range exceeded its live backing store"
            ))
        ));
        assert_eq!(
            shared_handle
                .read_range(0, BUFFER_COPY_SCRATCH_BYTE_LENGTH as u32)
                .unwrap(),
            vec![0; BUFFER_COPY_SCRATCH_BYTE_LENGTH]
        );
    }
}
