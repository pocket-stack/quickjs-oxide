//! Immutable closure indices. The callee, not this Rc slice, owns the cells.
use crate::engine::heap::{
    VarRefId,
    roots::{VarRefRoot, VarRefView},
};
use crate::engine::object::ObjectRef;
use std::rc::Rc;

pub(crate) struct ClosureSlots(Environment);

enum Environment {
    Shared {
        owner: ObjectRef,
        ids: Rc<[VarRefId]>,
    },
    // Materialized/synthetic external entry keeps its independent ownership.
    Rooted(Vec<VarRefRoot>),
}
impl Default for ClosureSlots {
    fn default() -> Self {
        Self(Environment::Rooted(Vec::new()))
    }
}
impl From<Vec<VarRefRoot>> for ClosureSlots {
    fn from(roots: Vec<VarRefRoot>) -> Self {
        Self(Environment::Rooted(roots))
    }
}
impl ClosureSlots {
    pub(super) fn shared(owner: ObjectRef, ids: Rc<[VarRefId]>) -> Self {
        Self(Environment::Shared { owner, ids })
    }
    pub(crate) fn len(&self) -> usize {
        match &self.0 {
            Environment::Shared { ids, .. } => ids.len(),
            Environment::Rooted(roots) => roots.len(),
        }
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub(crate) fn borrowed_cell(
        &self,
        index: usize,
    ) -> Option<(&crate::engine::api::Runtime, VarRefId)> {
        match &self.0 {
            Environment::Shared { owner, ids } => ids.get(index).map(|id| (owner.runtime(), *id)),
            Environment::Rooted(roots) => roots.get(index).map(|root| (&root.runtime, root.id())),
        }
    }
    pub(crate) fn get(&self, index: usize) -> Option<VarRefView<'_>> {
        VarRefView::from_closure(self, index)
    }
}
#[cfg(test)]
impl ClosureSlots {
    pub(super) fn test_roots_mut(&mut self) -> &mut Vec<VarRefRoot> {
        match &mut self.0 {
            Environment::Rooted(roots) => roots,
            _ => panic!("published closure is immutable"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        api::{Runtime, Value},
        object::CallableRef,
        vm::call::CallableExecution,
    };
    #[test]
    fn shared_environment_does_not_retain_each_cell_and_outlives_external_callee() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context
            .eval("(()=>{let a=1,b=2;return ()=>a+b})()")
            .unwrap()
        else {
            panic!()
        };
        let callable = CallableRef::from_validated_object(function);
        let ids = {
            let state = runtime.0.state.borrow();
            let crate::engine::heap::ObjectPayload::BytecodeFunction { closure_slots, .. } = &state
                .heap
                .object(callable.as_object().object_id())
                .unwrap()
                .payload
            else {
                panic!()
            };
            closure_slots.clone()
        };
        assert_eq!(ids.len(), 2);
        let counts: Vec<_> = ids
            .iter()
            .map(|id| {
                runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .var_ref_strong_count(*id)
                    .unwrap()
            })
            .collect();
        let CallableExecution::Bytecode {
            bytecode,
            closure_slots,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!()
        };
        for (id, count) in ids.iter().zip(&counts) {
            assert_eq!(
                runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .var_ref_strong_count(*id)
                    .unwrap(),
                *count
            );
        }
        drop(callable);
        drop(bytecode);
        assert_eq!(
            runtime
                .read_var_ref(&closure_slots.get(0).unwrap())
                .unwrap(),
            Value::Int(1)
        );
        runtime
            .write_var_ref(&closure_slots.get(0).unwrap(), Value::Int(9))
            .unwrap();
        assert_eq!(
            runtime
                .read_var_ref(&closure_slots.get(0).unwrap())
                .unwrap(),
            Value::Int(9)
        );
        let escaped = closure_slots.get(0).unwrap().clone();
        drop(closure_slots);
        assert_eq!(runtime.read_var_ref(&escaped).unwrap(), Value::Int(9));
        assert!(runtime.0.state.borrow().heap.var_ref(ids[1]).is_err());
        drop(escaped);
        assert!(runtime.0.state.borrow().heap.var_ref(ids[0]).is_err());
    }
}
