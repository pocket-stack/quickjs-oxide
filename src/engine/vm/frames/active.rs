//! Incremental physical-native budgets owned by the activation stack.
use super::{ActiveFrameKind, ActiveFrameRecord, ActiveFrameToken};
use crate::engine::vm::native_stack::{native_stack_family, native_stack_weight};
use std::ops::{Deref, DerefMut};
use std::{cell::Cell, rc::Rc};

#[derive(Default)]
pub(crate) struct ActiveFrames {
    records: Vec<ActiveFrameRecord>,
    depth: Rc<Cell<usize>>,
    native_cost: usize,
    families: [usize; 14],
}
impl Clone for ActiveFrames {
    fn clone(&self) -> Self {
        Self {
            records: self.records.clone(),
            depth: Rc::new(Cell::new(self.records.len())),
            native_cost: self.native_cost,
            families: self.families,
        }
    }
}
impl ActiveFrames {
    pub(crate) fn with_depth(depth: Rc<Cell<usize>>) -> Self {
        debug_assert_eq!(depth.get(), 0);
        Self {
            depth,
            ..Self::default()
        }
    }
    pub(crate) fn native_cost(&self) -> usize {
        self.native_cost
    }
    pub(crate) fn native_family_depth(&self, family: usize) -> usize {
        self.families[family]
    }
    fn charge(&mut self, record: ActiveFrameRecord, add: bool) {
        if record.native_continuation {
            return;
        }
        let ActiveFrameKind::Native { target, .. } = record.kind else {
            return;
        };
        let weight = native_stack_weight(target);
        if add {
            self.native_cost += weight;
        } else {
            self.native_cost -= weight;
        }
        if let Some((family, _)) = native_stack_family(target) {
            if add {
                self.families[family] += 1;
            } else {
                self.families[family] -= 1;
            }
        }
    }
    pub(crate) fn push(&mut self, record: ActiveFrameRecord) {
        // Allocation precedes accounting so unwinding cannot leave a charge.
        self.records.push(record);
        self.depth.set(self.records.len());
        self.charge(record, true);
    }
    pub(crate) fn pop(&mut self) -> Option<ActiveFrameRecord> {
        let record = self.records.pop()?;
        self.depth.set(self.records.len());
        self.charge(record, false);
        Some(record)
    }
    pub(crate) fn truncate(&mut self, depth: usize) {
        while self.records.len() > depth {
            self.pop();
        }
    }
    pub(super) fn mark_native_continuation(&mut self, depth: usize, token: ActiveFrameToken) {
        let record = self.records[depth];
        assert_eq!(record.token, token);
        self.charge(record, false);
        self.records[depth].native_continuation = true;
    }
}
impl Deref for ActiveFrames {
    type Target = [ActiveFrameRecord];
    fn deref(&self) -> &Self::Target {
        &self.records
    }
}
impl DerefMut for ActiveFrames {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.records
    }
}
impl ActiveFrames {
    pub(crate) fn retire(&mut self, token: ActiveFrameToken, depth: usize) {
        // Guards carry their installation index. Exception suffix destruction
        // touches only the removed records, not surviving ancestors.
        if self
            .records
            .get(depth)
            .is_some_and(|frame| frame.token == token)
        {
            self.truncate(depth);
        } else if let Ok(position) = self
            .records
            .binary_search_by_key(&token.0, |frame| frame.token.0)
        {
            // Corrupt/out-of-order identities preserve the old cleanup result.
            self.truncate(position);
        } else {
            self.truncate(depth);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        api::{Runtime, Value},
        builtins::native::{NativeFunctionId as N, RegExpNativeKind as R},
        vm::frames::ActiveFrameFlags,
    };
    #[test]
    fn native_budget_matches_scan_across_mark_pop_and_exception_suffixes() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context.eval("function f(){}; f").unwrap() else {
            panic!()
        };
        let shared_depth = Rc::new(Cell::new(0));
        let mut frames = ActiveFrames::with_depth(shared_depth.clone());
        let verify = |frames: &ActiveFrames| {
            assert_eq!(shared_depth.get(), frames.len());
            let mut cost = 0;
            let mut families = [0; 14];
            for frame in frames.iter().filter(|f| !f.native_continuation) {
                if let ActiveFrameKind::Native { target, .. } = frame.kind {
                    cost += native_stack_weight(target);
                    if let Some((family, _)) = native_stack_family(target) {
                        families[family] += 1;
                    }
                }
            }
            assert_eq!(frames.native_cost, cost);
            assert_eq!(frames.families, families);
        };
        for (i, target) in [
            N::FunctionPrototypeCall,
            N::ObjectAssign,
            N::ObjectFromEntries,
            N::RegExp(R::Search),
            N::ArrayPrototypeSort,
            N::ObjectHasOwn,
        ]
        .into_iter()
        .enumerate()
        {
            frames.push(ActiveFrameRecord {
                token: ActiveFrameToken(i as u64 + 1),
                function: function.object_id(),
                realm: context.realm,
                flags: ActiveFrameFlags::default(),
                kind: ActiveFrameKind::Native {
                    target,
                    actual_arg_count: 0,
                    readable_arg_count: 0,
                },
                native_continuation: false,
            });
            verify(&frames);
        }
        let mut independent = frames.clone();
        independent.pop();
        assert_eq!(shared_depth.get(), frames.len());
        assert_eq!(independent.depth.get(), independent.len());
        frames.mark_native_continuation(2, ActiveFrameToken(3));
        verify(&frames);
        frames.pop();
        verify(&frames);
        frames.retire(ActiveFrameToken(3), 2);
        verify(&frames);
        frames.retire(ActiveFrameToken(1), 0);
        verify(&frames);
        assert!(frames.is_empty());
    }
}
