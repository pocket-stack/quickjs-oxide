//! Relocate linear IR fragments and their declaration-site metadata.
//!
//! Insertion preserves unresolved parser sentinels; a complete entry prefix
//! must have fully resolved targets. Source sites travel with each operation.

use crate::engine::api::error::{Error, ErrorKind};
use crate::engine::code::bytecode::Instruction;
use crate::engine::compiler::model::ir::function::FunctionIr;
use crate::engine::compiler::model::ir::{IrOp, SpannedIrOp};
use std::ops::Range;

pub(super) fn relocate_ir_fragment(
    operations: &mut [SpannedIrOp],
    old_range: Range<usize>,
    new_start: usize,
) -> Result<(), Error> {
    #[cfg(feature = "profiling")]
    let _phase_timer = crate::engine::api::profiling::PhaseTimer::start(
        crate::engine::api::profiling::CompilePhase::Relocation,
    );
    for operation in operations {
        let Some(target) = ir_target_mut(&mut operation.op) else {
            continue;
        };
        let Ok(old_target) = usize::try_from(*target) else {
            continue;
        };
        if old_range.contains(&old_target) {
            let relocated = new_start
                .checked_add(old_target - old_range.start)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "out of memory"))?;
            *target = u32::try_from(relocated)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?;
        }
    }
    Ok(())
}

pub(super) fn insert_hoist_fragment(
    function: &mut FunctionIr,
    at: usize,
    fragment: Vec<SpannedIrOp>,
) -> Result<(), Error> {
    if fragment.is_empty() {
        return Ok(());
    }
    if at > function.ops.len() {
        return Err(Error::internal("function hoist insertion is out of bounds"));
    }
    let shift = u32::try_from(fragment.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    for scoped in &mut function.scoped_functions {
        if scoped.authored_closure >= at {
            scoped.authored_closure = scoped
                .authored_closure
                .checked_add(fragment.len())
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        }
    }
    for annex in &mut function.program_annex_functions {
        if annex.authored_closure >= at {
            annex.authored_closure = annex
                .authored_closure
                .checked_add(fragment.len())
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        }
    }
    for operation in &mut function.ops {
        let Some(target) = ir_target_mut(&mut operation.op) else {
            continue;
        };
        // Forward edges use u32::MAX until their enclosing control construct
        // is complete. NamedEvaluation can insert a zero-effect SetName while
        // such an edge is still open; leave the sentinel for patch_jump.
        if *target != u32::MAX && usize::try_from(*target).is_ok_and(|target| target >= at) {
            *target = target
                .checked_add(shift)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        }
    }
    function.ops.splice(at..at, fragment);
    Ok(())
}

pub(super) fn prepend_hoist_prefix(
    function: &mut FunctionIr,
    mut prefix: Vec<SpannedIrOp>,
) -> Result<(), Error> {
    if prefix.is_empty() {
        return Ok(());
    }
    let shift = u32::try_from(prefix.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    for scoped in &mut function.scoped_functions {
        scoped.authored_closure = scoped
            .authored_closure
            .checked_add(prefix.len())
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    }
    for annex in &mut function.program_annex_functions {
        annex.authored_closure = annex
            .authored_closure
            .checked_add(prefix.len())
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    }
    for operation in &mut function.ops {
        let Some(target) = ir_target_mut(&mut operation.op) else {
            continue;
        };
        *target = target
            .checked_add(shift)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    }
    prefix.append(&mut function.ops);
    function.ops = prefix;
    Ok(())
}

/// Resolve a logical instruction boundary after expansion. The final offset
/// also represents the end boundary, as in the pre-existing lowering table;
/// required code validation subsequently decides whether an edge may target it.
pub(super) fn relocate_lowered_instruction(
    instruction: &mut Instruction,
    offsets: &[usize],
) -> Result<(), Error> {
    let Some(target) = instruction_target_mut(instruction) else {
        return Ok(());
    };
    #[cfg(feature = "profiling")]
    let _phase_timer = crate::engine::api::profiling::PhaseTimer::start(
        crate::engine::api::profiling::CompilePhase::Relocation,
    );
    let old =
        usize::try_from(*target).map_err(|_| Error::internal("jump target did not fit usize"))?;
    let new = offsets
        .get(old)
        .copied()
        .ok_or_else(|| Error::internal("jump target is out of bounds"))?;
    *target =
        u32::try_from(new).map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    Ok(())
}

fn instruction_target_mut(instruction: &mut Instruction) -> Option<&mut u32> {
    match instruction {
        Instruction::Goto(target)
        | Instruction::IfFalse(target)
        | Instruction::IfTrue(target)
        | Instruction::Catch(target)
        | Instruction::Gosub(target) => Some(target),
        _ => None,
    }
}

fn ir_target_mut(operation: &mut IrOp) -> Option<&mut u32> {
    match operation {
        IrOp::Bytecode(instruction) => instruction_target_mut(instruction),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{relocate_ir_fragment, relocate_lowered_instruction};
    use crate::engine::code::bytecode::Instruction;
    use crate::engine::compiler::model::ir::{IrOp, SpannedIrOp};

    #[test]
    fn moved_fragment_preserves_external_edges_and_unresolved_sentinel() {
        let mut ops = [
            Instruction::Goto(3),
            Instruction::Catch(4),
            Instruction::Gosub(5),
            Instruction::IfTrue(u32::MAX),
        ]
        .map(|instruction| SpannedIrOp {
            op: IrOp::Bytecode(instruction),
            pc_site: None,
        });
        relocate_ir_fragment(&mut ops, 3..5, 10).unwrap();
        assert!(matches!(ops[0].op, IrOp::Bytecode(Instruction::Goto(10))));
        assert!(matches!(ops[1].op, IrOp::Bytecode(Instruction::Catch(11))));
        assert!(matches!(ops[2].op, IrOp::Bytecode(Instruction::Gosub(5))));
        assert!(matches!(
            ops[3].op,
            IrOp::Bytecode(Instruction::IfTrue(u32::MAX))
        ));
    }

    #[test]
    fn lowering_relocates_handler_and_end_boundary_but_rejects_missing_target() {
        let mut catch = Instruction::Catch(1);
        relocate_lowered_instruction(&mut catch, &[0, 4, 7]).unwrap();
        assert!(matches!(catch, Instruction::Catch(4)));
        let mut end = Instruction::Goto(2);
        relocate_lowered_instruction(&mut end, &[0, 4, 7]).unwrap();
        assert!(matches!(end, Instruction::Goto(7)));
        let mut missing = Instruction::Gosub(3);
        assert_eq!(
            relocate_lowered_instruction(&mut missing, &[0, 4, 7])
                .unwrap_err()
                .message(),
            "jump target is out of bounds"
        );
        assert!(matches!(missing, Instruction::Gosub(3)));
    }
}
