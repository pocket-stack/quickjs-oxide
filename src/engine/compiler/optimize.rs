//! Bounded local rewrites and their QuickJS source-observation projection.
//!
//! These passes preserve instruction slots. The late-throw source projection
//! must run before branch folding: it simulates QuickJS's ordered label walks,
//! including its ten-goto cycle workaround, rather than a fixed-point analysis.
//! No optional un-converged facts are consumed by required stack validation.

use crate::engine::api::error::{Error, ErrorKind};
use crate::engine::code::bytecode::Instruction;
use crate::source::SourceOffset;

/// QuickJS `resolve_labels` folds this deliberately narrow constant set before
/// `compute_stack_size`. Keep instruction slots stable with Nops so existing
/// IR-index jump remapping and debug PCs remain valid.
pub(super) fn fold_quickjs_constant_branches(code: &mut [Instruction]) {
    // Most functions contain no foldable constant/conditional pair. Materialize
    // the original entry map only when the first such pair needs it, before any
    // rewrite; subsequent folds must use those same original structural entries.
    let mut entries = None;

    for pc in 0..code.len().saturating_sub(1) {
        let truthy = match code[pc] {
            Instruction::Undefined | Instruction::Null | Instruction::PushFalse => false,
            Instruction::PushTrue => true,
            Instruction::PushI32(value) => value != 0,
            Instruction::PushAtomValueIndex(_) => true,
            _ => continue,
        };
        let effects = code[pc].potential_effects();
        // Preserve QuickJS's existing elimination of tagged integer-atom String
        // materialization. No other allocation or observable JS effect is waived.
        if effects.may_call_js
            || effects.javascript_exception
                != crate::engine::code::instruction::JsExceptionEffect::None
            || (effects.may_allocate && !matches!(code[pc], Instruction::PushAtomValueIndex(_)))
        {
            continue;
        }
        let (branch_on_true, target) = match code[pc + 1] {
            Instruction::IfFalse(target) => (false, target),
            Instruction::IfTrue(target) => (true, target),
            _ => continue,
        };
        // A hostile or hand-built control-flow edge may enter the conditional
        // without executing its adjacent constant. Preserve that independent
        // entry, including when it was named by a branch folded earlier here.
        let entries =
            entries.get_or_insert_with(|| crate::engine::compiler::flow::block_entries(code));
        if entries[pc + 1] {
            continue;
        }
        code[pc] = if truthy == branch_on_true {
            Instruction::Goto(target)
        } else {
            Instruction::Nop
        };
        code[pc + 1] = Instruction::Nop;
    }
}

pub(super) fn apply_quickjs_late_throw_sites(
    code: &[Instruction],
    pc_sites: &mut [Option<SourceOffset>],
) -> Result<(), Error> {
    if code.len() != pc_sites.len() {
        return Err(Error::internal(
            "lowered instructions and source markers have different lengths",
        ));
    }
    // With neither labels nor late throws, this projection cannot modify any
    // source marker or encounter a label-validation error. Keep all functions
    // containing control targets on the full path even without a late throw:
    // that path also authenticates malformed targets before stack verification.
    if !code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::Goto(_)
                | Instruction::IfFalse(_)
                | Instruction::IfTrue(_)
                | Instruction::Catch(_)
                | Instruction::Gosub(_)
                | Instruction::ThrowReadOnly(_)
                | Instruction::ThrowRedeclaration(_)
        )
    }) {
        return Ok(());
    }
    // Maintenance invariant: every new label-bearing instruction or
    // resolve-labels peephole must update this projection and add a pinned
    // fault-stack oracle before that control-flow slice is enabled.
    let label_target = |instruction: &Instruction| -> Result<Option<usize>, Error> {
        let (Instruction::Goto(target)
        | Instruction::IfFalse(target)
        | Instruction::IfTrue(target)
        | Instruction::Catch(target)
        | Instruction::Gosub(target)) = instruction
        else {
            return Ok(None);
        };
        usize::try_from(*target)
            .map(Some)
            .map_err(|_| Error::internal("jump target did not fit usize"))
    };
    let branch_target = |instruction: &Instruction| -> Result<Option<usize>, Error> {
        let (Instruction::Goto(target)
        | Instruction::IfFalse(target)
        | Instruction::IfTrue(target)) = instruction
        else {
            return Ok(None);
        };
        usize::try_from(*target)
            .map(Some)
            .map_err(|_| Error::internal("jump target did not fit usize"))
    };

    // `resolve_scope_var` introduces terminal OP_throw_error only after
    // parsing. QuickJS then performs two relevant linear rewrites.
    // `resolve_variables` first drops source after parser-authored terminals,
    // updating label reference counts for jumps in that dead range.
    // `resolve_labels` recognizes every newly introduced throw as terminal and
    // repeats the walk with one shared, cumulatively updated reference table.
    // Project both passes once for the whole function: per-throw simulation is
    // not equivalent when an earlier throw removes a forward branch reference.
    let mut label_references = vec![0_usize; code.len()];
    let mut has_physical_label = vec![false; code.len()];
    for instruction in code {
        let Some(target) = label_target(instruction)? else {
            continue;
        };
        let references = label_references
            .get_mut(target)
            .ok_or_else(|| Error::internal("jump target is out of bounds"))?;
        *references = references
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "out of memory"))?;
        has_physical_label[target] = true;
    }

    let mut survives_first_pass = vec![false; code.len()];
    let mut marker_before_label = vec![None; code.len()];
    let mut index = 0_usize;
    while index < code.len() {
        survives_first_pass[index] = true;
        let parser_terminal = matches!(
            code[index],
            Instruction::Goto(_)
                | Instruction::Return
                | Instruction::ReturnUndefined
                | Instruction::Throw
                | Instruction::Ret
        );
        if !parser_terminal {
            index += 1;
            continue;
        }

        let mut dead_index = index + 1;
        let mut final_dead_marker = None;
        while dead_index < code.len() {
            // An upstream OP_label precedes the marker attached to our direct
            // target instruction. A still-referenced label ends this dead
            // range before that authored marker is observed.
            if label_references[dead_index] > 0 {
                break;
            }
            if pc_sites[dead_index].is_some() {
                final_dead_marker = pc_sites[dead_index];
            }
            if let Some(target) = label_target(&code[dead_index])? {
                label_references[target] = label_references[target]
                    .checked_sub(1)
                    .ok_or_else(|| Error::internal("jump label reference count underflow"))?;
            }
            dead_index += 1;
        }
        if dead_index == code.len() {
            break;
        }
        marker_before_label[dead_index] = final_dead_marker;
        index = dead_index;
    }

    let follow_jump_target =
        |initial_target: usize, references: &mut [usize]| -> Result<usize, Error> {
            let initial = initial_target;
            let initial_references = references
                .get_mut(initial)
                .ok_or_else(|| Error::internal("jump target is out of bounds"))?;
            *initial_references = initial_references
                .checked_sub(1)
                .ok_or_else(|| Error::internal("jump label reference count underflow"))?;

            let mut target = initial;
            let mut followed_ten_gotos = true;
            for _ in 0..10 {
                if !survives_first_pass
                    .get(target)
                    .copied()
                    .ok_or_else(|| Error::internal("jump target is out of bounds"))?
                {
                    return Err(Error::internal(
                        "jump target did not survive variable resolution",
                    ));
                }
                let Some(next_target) = branch_target(&code[target])? else {
                    followed_ten_gotos = false;
                    break;
                };
                if !matches!(code[target], Instruction::Goto(_)) {
                    followed_ten_gotos = false;
                    break;
                }
                target = next_target;
            }
            // Preserve QuickJS's cycle workaround after ten chained gotos.
            if followed_ten_gotos {
                target = initial;
            }
            let final_references = references
                .get_mut(target)
                .ok_or_else(|| Error::internal("jump target is out of bounds"))?;
            *final_references = final_references
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "out of memory"))?;
            Ok(target)
        };

    let mut current_site = None;
    let mut late_throw_sites = Vec::new();
    index = 0;
    while index < code.len() {
        if !survives_first_pass[index] {
            index += 1;
            continue;
        }
        if marker_before_label[index].is_some() {
            current_site = marker_before_label[index];
        }
        if pc_sites[index].is_some() {
            current_site = pc_sites[index];
        }

        // `resolve_labels` folds the same adjacent constant-condition forms
        // as `fold_quickjs_constant_branches`. A non-taken branch releases its
        // forward label before a following late throw is visited; a taken one
        // becomes a terminal Goto whose target reference remains live.
        let constant_truthy = match code[index] {
            Instruction::Undefined | Instruction::Null | Instruction::PushFalse => Some(false),
            Instruction::PushTrue => Some(true),
            Instruction::PushI32(value) => Some(value != 0),
            Instruction::PushAtomValueIndex(_) => Some(true),
            _ => None,
        };
        let mut folded_goto = false;
        let mut terminal_tail = index + 1;
        if let Some(truthy) = constant_truthy
            && let Some(conditional_index) = index.checked_add(1)
            && conditional_index < code.len()
            && survives_first_pass[conditional_index]
            // `code_match` skips source markers but never crosses a physical
            // OP_label, even after earlier rewrites reduce its refcount to
            // zero. Direct-target IR therefore needs an immutable label bit;
            // the mutable reference count alone is not an adjacency test.
            && !has_physical_label[conditional_index]
        {
            let branch = match code[conditional_index] {
                Instruction::IfFalse(target) => Some((false, target)),
                Instruction::IfTrue(target) => Some((true, target)),
                _ => None,
            };
            if let Some((branch_on_true, target)) = branch {
                if marker_before_label[conditional_index].is_some() {
                    current_site = marker_before_label[conditional_index];
                }
                if pc_sites[conditional_index].is_some() {
                    current_site = pc_sites[conditional_index];
                }
                let target = usize::try_from(target)
                    .map_err(|_| Error::internal("jump target did not fit usize"))?;
                terminal_tail = conditional_index + 1;
                if truthy == branch_on_true {
                    follow_jump_target(target, &mut label_references)?;
                    folded_goto = true;
                } else {
                    label_references[target] = label_references[target]
                        .checked_sub(1)
                        .ok_or_else(|| Error::internal("jump label reference count underflow"))?;
                    index = terminal_tail;
                    continue;
                }
            }
        }

        let mut followed_target = None;
        if !folded_goto && let Some(target) = branch_target(&code[index])? {
            followed_target = Some(follow_jump_target(target, &mut label_references)?);
        }

        // QuickJS also folds `if_x(l1); goto(l2); label(l1)` to the opposite
        // conditional targeting `l2`. The Goto is consumed, its existing l2
        // reference is reused by the conditional, and l1 loses the reference
        // transferred above. This must happen before either branch's late
        // readonly throw updates the shared label table.
        if !folded_goto
            && matches!(
                code[index],
                Instruction::IfFalse(_) | Instruction::IfTrue(_)
            )
            && let Some(effective_target) = followed_target
        {
            let mut goto_index = index + 1;
            while goto_index < code.len() && !survives_first_pass[goto_index] {
                goto_index += 1;
            }
            if goto_index < code.len()
                && !has_physical_label[goto_index]
                && matches!(code[goto_index], Instruction::Goto(_))
            {
                let mut after_goto = goto_index + 1;
                while after_goto < code.len() && !survives_first_pass[after_goto] {
                    after_goto += 1;
                }
                let has_effective_label = after_goto < code.len()
                    && ((has_physical_label[after_goto] && after_goto == effective_target)
                        || (matches!(code[after_goto], Instruction::Goto(_))
                            && branch_target(&code[after_goto])? == Some(effective_target)));
                if has_effective_label {
                    if pc_sites[goto_index].is_some() {
                        current_site = pc_sites[goto_index];
                    }
                    label_references[effective_target] = label_references[effective_target]
                        .checked_sub(1)
                        .ok_or_else(|| Error::internal("jump label reference count underflow"))?;
                    index = after_goto;
                    continue;
                }
            }
        }

        let terminal = folded_goto
            || matches!(
                code[index],
                Instruction::Goto(_)
                    | Instruction::Return
                    | Instruction::ReturnUndefined
                    | Instruction::Throw
                    | Instruction::Ret
                    | Instruction::ThrowReadOnly(_)
                    | Instruction::ThrowRedeclaration(_)
            );
        if !terminal {
            index += 1;
            continue;
        }

        let terminal_index = index;
        let mut dead_index = terminal_tail;
        while dead_index < code.len() {
            if !survives_first_pass[dead_index] {
                dead_index += 1;
                continue;
            }
            // The first pass emits its final removed marker before the label,
            // so the second pass observes it even when another live reference
            // makes that label the stopping point.
            if marker_before_label[dead_index].is_some() {
                current_site = marker_before_label[dead_index];
            }
            if label_references[dead_index] > 0 {
                break;
            }
            if pc_sites[dead_index].is_some() {
                current_site = pc_sites[dead_index];
            }
            if let Some(target) = label_target(&code[dead_index])? {
                label_references[target] = label_references[target]
                    .checked_sub(1)
                    .ok_or_else(|| Error::internal("jump label reference count underflow"))?;
            }
            dead_index += 1;
        }
        if matches!(
            code[terminal_index],
            Instruction::ThrowReadOnly(_) | Instruction::ThrowRedeclaration(_)
        ) {
            late_throw_sites.push((terminal_index, current_site));
        }
        index = dead_index;
    }

    for (index, site) in late_throw_sites {
        pc_sites[index] = site;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{apply_quickjs_late_throw_sites, fold_quickjs_constant_branches};
    use crate::engine::code::bytecode::Instruction;

    #[test]
    fn lazy_branch_entries_preserve_targets_of_earlier_removed_branches() {
        use Instruction::*;
        // The first rewrite removes the only branch to PC 4. The second
        // candidate must still see the original entry map and remain intact.
        let mut code = [
            Nop,
            PushFalse,
            IfTrue(4),
            PushTrue,
            IfTrue(6),
            Nop,
            ReturnUndefined,
        ];
        fold_quickjs_constant_branches(&mut code);
        assert!(matches!(code[1], Nop));
        assert!(matches!(code[2], Nop));
        assert!(matches!(code[3], PushTrue));
        assert!(matches!(code[4], IfTrue(6)));
    }

    #[test]
    fn late_throw_projection_keeps_straight_line_sites_and_validation_order() {
        use crate::source::SourceOffset;
        use Instruction::*;
        let start = Some(SourceOffset::try_from_usize(3).unwrap());
        let dead = Some(SourceOffset::try_from_usize(29).unwrap());
        let code = [PushI32(1), Return, Nop];
        let original = [start, None, dead];
        let mut sites = original;
        apply_quickjs_late_throw_sites(&code, &mut sites).unwrap();
        assert_eq!(sites, original);
        assert_eq!(
            apply_quickjs_late_throw_sites(&code, &mut [])
                .unwrap_err()
                .message(),
            "lowered instructions and source markers have different lengths"
        );
        // No late throw is present, but unreachable invalid targets must still
        // fail in the projection before later publication validators run.
        for target in [Goto(9), IfTrue(9), IfFalse(9), Catch(9), Gosub(9)] {
            let code = [ReturnUndefined, target];
            assert_eq!(
                apply_quickjs_late_throw_sites(&code, &mut [start, dead])
                    .unwrap_err()
                    .message(),
                "jump target is out of bounds"
            );
        }
        // A late throw with no labels still requires the ordered dead-source
        // projection: it inherits the final source marker after its terminal.
        let mut sites = [start, dead];
        apply_quickjs_late_throw_sites(&[ThrowReadOnly(0), Nop], &mut sites).unwrap();
        assert_eq!(sites, [dead, dead]);
    }
}
