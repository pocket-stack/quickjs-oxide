//! Partial owned-buffer snapshots at compilation phase boundaries.
//!
//! This measures actual Vec capacities owned by the current function arena.
//! It does not infer allocator peaks or count referenced strings/boxed operands,
//! hash-table buckets, source storage, or temporary verifier/worklist storage.

use crate::engine::api::profiling::{CompilePhase, record_compiler_storage};
use crate::engine::compiler::model::ir::function::FunctionIr;

pub(super) fn arena_bytes<T>(values: &Vec<T>) -> u64 {
    values.capacity().saturating_mul(size_of::<T>()) as u64
}

pub(super) fn sample_ir_storage<'a>(
    phase: CompilePhase,
    arena_inline_bytes: u64,
    functions: impl Iterator<Item = &'a FunctionIr>,
) {
    if !crate::engine::api::profiling::cost_profile_active() {
        return;
    }
    let mut bytes = arena_inline_bytes;
    for function in functions {
        for size in [
            arena_bytes(&function.ops),
            arena_bytes(&function.constants),
            arena_bytes(&function.bindings),
            arena_bytes(&function.scopes),
            arena_bytes(&function.locals),
            arena_bytes(&function.parameters),
            arena_bytes(&function.closure_variables),
            arena_bytes(&function.eval_environments),
        ] {
            bytes = bytes.saturating_add(size);
        }
        for scope in &function.scopes {
            bytes = bytes.saturating_add(arena_bytes(&scope.bindings));
        }
    }
    record_compiler_storage(phase, bytes);
}
