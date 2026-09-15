//! Outlined error construction and diagnostic-only recording.
use crate::engine::api::Error;

#[cold]
#[inline(never)]
pub(super) fn internal(message: &'static str) -> Error {
    Error::internal(message)
}

#[cfg(feature = "profiling")]
#[cold]
#[inline(never)]
pub(super) fn event(name: &'static str) {
    crate::engine::api::profiling::record_owned_execution_event(name);
}

#[cfg(feature = "profiling")]
#[cold]
#[inline(never)]
pub(super) fn instruction(depth: usize) {
    crate::engine::api::profiling::record_owned_instruction(depth);
}

#[cfg(feature = "profiling")]
#[cold]
#[inline(never)]
pub(super) fn storage(event: crate::engine::api::profiling::OwnedStorageEvent) {
    crate::engine::api::profiling::record_owned_storage(event);
}

#[cfg(test)]
mod tests {
    #[test]
    fn cold_exit_keeps_only_narrow_instruction_facts() {
        assert!(std::mem::size_of::<super::super::RunExit>() <= 32);
    }
}
