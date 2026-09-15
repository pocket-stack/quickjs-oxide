//! Required stack-state validation and entry facts used during lowering.
//!
//! The code verifier remains authoritative for reachable normal, exception,
//! private-reference and resume states. This adapter never replaces it with
//! a syntactic stack-height estimate or a budget-limited dataflow result.

use crate::engine::api::error::{Error, ErrorKind};
use crate::engine::code::bytecode::{Instruction, verify_parts};
use crate::engine::compiler::MAX_BYTECODE_STACK;

pub(super) fn verify_lowered_max_stack(
    code: &[Instruction],
    constant_count: usize,
) -> Result<u16, Error> {
    verify_parts(code, constant_count, MAX_BYTECODE_STACK as u16)
        .map(|verified| verified.max_stack)
        .map_err(|error| {
            if matches!(
                error.message(),
                "declared maximum stack is smaller than required"
                    | "bytecode stack exceeds u16::MAX"
            ) {
                Error::new(ErrorKind::JsInternal, "stack overflow")
            } else {
                error
            }
        })
}

/// Structural basic-block starts, including unreachable continuations. These
/// bits delimit local rewrites; they do not assert reachability, initialized
/// locals, or a valid resume shape. `verify_parts` proves those stack facts.
pub(super) fn block_entries(code: &[Instruction]) -> Vec<bool> {
    #[cfg(feature = "profiling")]
    let _phase_timer = crate::engine::api::profiling::PhaseTimer::start(
        crate::engine::api::profiling::CompilePhase::Blocks,
    );
    let mut entries = vec![false; code.len()];
    if let Some(entry) = entries.first_mut() {
        *entry = true;
    }
    for (pc, instruction) in code.iter().enumerate() {
        let control = instruction.control_effect();
        if let Some(target) = control.target()
            && let Ok(target) = usize::try_from(target)
            && let Some(entry) = entries.get_mut(target)
        {
            *entry = true;
        }
        if control.ends_block()
            && let Some(entry) = entries.get_mut(pc + 1)
        {
            *entry = true;
        }
    }
    entries
}
