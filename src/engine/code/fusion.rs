//! Authenticated execution spans over canonical, verified instruction PCs.
//!
//! The published bytecode and its source/relocation tables are never rewritten.
//! A span is entered only at its first instruction, contains no external entry,
//! and falls back to that original instruction before changing any VM state.
//! Legacy execution and serialization continue to consume canonical opcodes.
use super::bytecode::Instruction;
use super::function::metadata::{ClosureVariableKind, VariableDefinition};
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub(crate) struct FusionPlan(Option<Rc<[u8]>>);

#[derive(Clone, Copy)]
pub(crate) struct UpdateLocal {
    pub increment: bool,
    pub postfix: bool,
    pub discard: bool,
    pub instructions: usize,
}

impl FusionPlan {
    pub(crate) fn build(code: &[Instruction], locals: &[VariableDefinition]) -> Self {
        #[cfg(feature = "profiling")]
        let _timer = crate::engine::api::profiling::PhaseTimer::start(
            crate::engine::api::profiling::CompilePhase::Fusion,
        );
        // Allocate nothing for the common small leaf without a candidate.
        if !code.windows(2).enumerate().any(|(pc, pair)| {
            if method_call_count(&code[pc..]).is_some() {
                return true;
            }
            if matches!(
                pair,
                [
                    Instruction::Add,
                    Instruction::SetLocal(_) | Instruction::SetLocalCheck(_)
                ]
            ) && matches!(code.get(pc + 2), Some(Instruction::Drop))
            {
                return true;
            }
            matches!(
                pair,
                [
                    Instruction::Add,
                    Instruction::PutLocal(_) | Instruction::PutLocalCheck(_)
                ] | [
                    Instruction::GetLocal(_) | Instruction::GetLocalCheck(_),
                    Instruction::Inc
                        | Instruction::Dec
                        | Instruction::PostInc
                        | Instruction::PostDec
                ] | [
                    Instruction::Lt
                        | Instruction::Lte
                        | Instruction::Gt
                        | Instruction::Gte
                        | Instruction::Eq
                        | Instruction::Neq
                        | Instruction::StrictEq
                        | Instruction::StrictNeq,
                    Instruction::IfTrue(_) | Instruction::IfFalse(_)
                ]
            )
        }) {
            return Self::default();
        }
        let mut entries = vec![false; code.len()];
        for (pc, instruction) in code.iter().enumerate() {
            let control = instruction.control_effect();
            if let Some(target) = control.target() {
                if let Some(entry) = entries.get_mut(target as usize) {
                    *entry = true;
                }
            }
            if control.ends_block() {
                if let Some(entry) = entries.get_mut(pc + 1) {
                    *entry = true;
                }
            }
        }
        let mut flags = vec![0; code.len()];
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_compiler_storage(
            crate::engine::api::profiling::CompilePhase::Fusion,
            (entries.capacity() + flags.capacity()) as u64,
        );
        let mut any = false;
        for pc in 0..code.len() {
            let rest = &code[pc..];
            let update = match rest {
                [
                    Instruction::GetLocal(index) | Instruction::GetLocalCheck(index),
                    operation,
                    store,
                    ..,
                ] if locals
                    .get(usize::from(*index))
                    .is_some_and(|d| !d.is_const && d.kind == ClosureVariableKind::Normal) =>
                {
                    let postfix = matches!(operation, Instruction::PostInc | Instruction::PostDec);
                    let increment = matches!(operation, Instruction::Inc | Instruction::PostInc);
                    let operation_valid = matches!(
                        operation,
                        Instruction::Inc
                            | Instruction::Dec
                            | Instruction::PostInc
                            | Instruction::PostDec
                    );
                    let valid_store = match store {
                        Instruction::PutLocal(i) | Instruction::PutLocalCheck(i) => *i == *index,
                        Instruction::SetLocal(i) | Instruction::SetLocalCheck(i) => {
                            *i == *index && !postfix
                        }
                        _ => false,
                    };
                    if operation_valid && valid_store {
                        let keeps = postfix
                            || matches!(
                                store,
                                Instruction::SetLocal(_) | Instruction::SetLocalCheck(_)
                            );
                        let drop = keeps
                            && matches!(rest.get(3), Some(Instruction::Drop))
                            && !entries[pc + 3];
                        Some((
                            16 | u8::from(increment)
                                | (u8::from(postfix) << 1)
                                | (u8::from(!keeps || drop) << 2)
                                | (u8::from(drop) << 3),
                            if drop { 4 } else { 3 },
                        ))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let local_add = match rest {
                [
                    Instruction::GetLocal(left) | Instruction::GetLocalCheck(left),
                    Instruction::GetLocal(right) | Instruction::GetLocalCheck(right),
                    Instruction::Add,
                    store,
                    ..,
                ] if [left, right].iter().all(|index| {
                    locals
                        .get(usize::from(**index))
                        .is_some_and(|d| d.kind == ClosureVariableKind::Normal)
                }) && locals.get(usize::from(*left)).is_some_and(|d| !d.is_const) =>
                {
                    match store {
                        Instruction::PutLocal(index) | Instruction::PutLocalCheck(index)
                            if index == left =>
                        {
                            Some((128, 4))
                        }
                        Instruction::SetLocal(index) | Instruction::SetLocalCheck(index)
                            if index == left && matches!(rest.get(4), Some(Instruction::Drop)) =>
                        {
                            Some((129, 5))
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            let method = method_call_count(rest).map(|count| (160 + count as u8, count + 2));
            let candidate = method.or(local_add).or(update).or_else(|| match rest {
                [
                    Instruction::Lt
                    | Instruction::Lte
                    | Instruction::Gt
                    | Instruction::Gte
                    | Instruction::Eq
                    | Instruction::Neq
                    | Instruction::StrictEq
                    | Instruction::StrictNeq,
                    Instruction::IfTrue(_) | Instruction::IfFalse(_),
                    ..,
                ] => Some((32, 2)),
                [
                    Instruction::Add,
                    Instruction::PutLocal(index) | Instruction::PutLocalCheck(index),
                    ..,
                ] if locals
                    .get(usize::from(*index))
                    .is_some_and(|d| !d.is_const && d.kind == ClosureVariableKind::Normal) =>
                {
                    Some((64, 2))
                }
                [
                    Instruction::Add,
                    Instruction::SetLocal(index) | Instruction::SetLocalCheck(index),
                    Instruction::Drop,
                    ..,
                ] if locals
                    .get(usize::from(*index))
                    .is_some_and(|d| !d.is_const && d.kind == ClosureVariableKind::Normal) =>
                {
                    Some((65, 3))
                }
                _ => None,
            });
            if let Some((flag, length)) = candidate {
                if !entries[pc + 1..pc + length].iter().any(|v| *v) {
                    flags[pc] = flag;
                    any = true;
                }
            }
        }
        Self(any.then(|| flags.into()))
    }

    #[inline]
    fn flag(&self, pc: usize) -> u8 {
        self.0
            .as_ref()
            .and_then(|flags| flags.get(pc))
            .copied()
            .unwrap_or(0)
    }
    #[inline]
    pub(crate) fn update(&self, pc: usize) -> Option<UpdateLocal> {
        let flag = self.flag(pc);
        (flag & 16 != 0).then_some(UpdateLocal {
            increment: flag & 1 != 0,
            postfix: flag & 2 != 0,
            discard: flag & 4 != 0,
            instructions: if flag & 8 != 0 { 4 } else { 3 },
        })
    }
    #[inline]
    pub(crate) fn compare_branch(&self, pc: usize) -> bool {
        self.flag(pc) == 32
    }
    #[inline]
    pub(crate) fn add_store(&self, pc: usize) -> bool {
        matches!(self.flag(pc), 64 | 65)
    }
    /// Only literals and direct binding reads may join a completed own read.
    /// Runtime guards retain canonical evaluation for TDZ/captured bindings.
    pub(crate) fn method_call(&self, pc: usize) -> Option<usize> {
        let flag = self.flag(pc);
        (160..=167).contains(&flag).then(|| usize::from(flag - 160))
    }
    /// Full borrowed-local addition begins before either operand copy.
    pub(crate) fn local_add_span(&self, pc: usize) -> Option<usize> {
        match self.flag(pc) {
            128 => Some(4),
            129 => Some(5),
            _ => None,
        }
    }
    pub(crate) fn add_store_span(&self, pc: usize) -> usize {
        if self.flag(pc) == 65 { 3 } else { 2 }
    }
}

fn method_call_count(rest: &[Instruction]) -> Option<usize> {
    if !matches!(rest.first(), Some(Instruction::GetField2(_))) {
        return None;
    }
    for count in 0..=7 {
        match rest.get(count + 1)? {
            Instruction::CallMethod(arguments) if usize::from(*arguments) == count => {
                return Some(count);
            }
            Instruction::GetLocal(_)
            | Instruction::GetLocalCheck(_)
            | Instruction::GetArg(_)
            | Instruction::PushI32(_)
            | Instruction::Undefined
            | Instruction::Null
            | Instruction::PushTrue
            | Instruction::PushFalse => {}
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn local(constant: bool) -> VariableDefinition {
        VariableDefinition {
            name: None,
            is_lexical: true,
            is_const: constant,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        }
    }
    #[test]
    fn local_add_span_rejects_intermediate_entries_and_other_targets() {
        use Instruction::*;
        let code = [
            GetLocalCheck(0),
            GetLocalCheck(1),
            Add,
            PutLocalCheck(0),
            ReturnUndefined,
        ];
        assert_eq!(
            FusionPlan::build(&code, &[local(false), local(false)]).local_add_span(0),
            Some(4)
        );
        assert_eq!(
            FusionPlan::build(&code, &[local(true), local(false)]).local_add_span(0),
            None
        );
        let code = [
            GetLocal(0),
            GetLocal(1),
            Add,
            SetLocal(0),
            Drop,
            ReturnUndefined,
        ];
        assert_eq!(
            FusionPlan::build(&code, &[local(false), local(false)]).local_add_span(0),
            Some(5)
        );
        for target in 1..5 {
            let mut code = code.to_vec();
            code.push(Goto(target));
            assert_eq!(
                FusionPlan::build(&code, &[local(false), local(false)]).local_add_span(0),
                None
            );
        }
        let code = [GetLocal(0), GetLocal(1), Add, PutLocal(1), ReturnUndefined];
        assert_eq!(
            FusionPlan::build(&code, &[local(false), local(false)]).local_add_span(0),
            None
        );
    }

    #[test]
    fn method_call_spans_reject_effectful_arguments_and_interior_entry() {
        use Instruction::*;
        let code = [GetField2(0), PushI32(1), PushFalse, CallMethod(2), Return];
        assert_eq!(FusionPlan::build(&code, &[]).method_call(0), Some(2));
        let code = [GetField2(0), GetLocal(0), CallMethod(1), Return];
        assert_eq!(
            FusionPlan::build(&code, &[local(false)]).method_call(0),
            Some(1)
        );
        let code = [GetField2(0), PushI32(1), CallMethod(1), Goto(1), Return];
        assert_eq!(FusionPlan::build(&code, &[]).method_call(0), None);
        let code = [GetField2(0), PushI32(1), TailCallMethod(1)];
        assert_eq!(FusionPlan::build(&code, &[]).method_call(0), None);
    }
    #[test]
    fn add_store_requires_mutable_normal_target_and_no_interior_entry() {
        use Instruction::*;
        let code = [Add, PutLocal(0), ReturnUndefined];
        assert!(FusionPlan::build(&code, &[local(false)]).add_store(0));
        assert!(!FusionPlan::build(&code, &[local(true)]).add_store(0));
        let code = [Add, SetLocalCheck(0), Drop, ReturnUndefined];
        let plan = FusionPlan::build(&code, &[local(false)]);
        assert!(plan.add_store(0));
        assert_eq!(plan.add_store_span(0), 3);
        let code = [Add, SetLocalCheck(0), Drop, Goto(2), ReturnUndefined];
        assert!(!FusionPlan::build(&code, &[local(false)]).add_store(0));
        let code = [Add, PutLocal(0), Goto(1), ReturnUndefined];
        assert!(!FusionPlan::build(&code, &[local(false)]).add_store(0));
    }
    #[test]
    fn finite_update_forms_preserve_their_logical_extent() {
        use Instruction::*;
        for (operation, store, postfix, discard) in [
            (Inc, SetLocal(0), false, false),
            (PostInc, PutLocal(0), true, false),
            (Dec, PutLocal(0), false, true),
        ] {
            let code = [GetLocalCheck(0), operation, store, Return];
            let plan = FusionPlan::build(&code, &[local(false)]);
            let update = plan.update(0).unwrap();
            assert_eq!(
                (update.instructions, update.postfix, update.discard),
                (3, postfix, discard)
            );
            assert!(plan.update(1).is_none());
            assert!(FusionPlan::build(&code, &[local(true)]).update(0).is_none());
        }
        let code = [GetLocal(0), PostDec, PutLocal(0), Drop, ReturnUndefined];
        let update = FusionPlan::build(&code, &[local(false)]).update(0).unwrap();
        assert_eq!((update.instructions, update.discard), (4, true));
    }
    #[test]
    fn all_interior_control_targets_prevent_fusion() {
        use Instruction::*;
        for target in [1, 2] {
            for entry in [Goto(target), IfTrue(target), Catch(target), Gosub(target)] {
                let code = [GetLocal(0), Inc, PutLocal(0), entry, ReturnUndefined];
                assert!(
                    FusionPlan::build(&code, &[local(false)])
                        .update(0)
                        .is_none()
                );
            }
        }
        let code = [Lt, IfFalse(3), Goto(1), ReturnUndefined];
        assert!(!FusionPlan::build(&code, &[]).compare_branch(0));
        let code = [Lt, IfFalse(3), Nop, ReturnUndefined];
        assert!(FusionPlan::build(&code, &[]).compare_branch(0));
    }
    #[test]
    fn branch_to_drop_keeps_the_drop_outside_the_span() {
        use Instruction::*;
        let code = [GetLocal(0), PostInc, PutLocal(0), Drop, Goto(3)];
        let update = FusionPlan::build(&code, &[local(false)]).update(0).unwrap();
        assert_eq!((update.instructions, update.discard), (3, false));
    }
}
