//! Immutable execution projection. Only a runtime snapshot can pair this
//! layout with its owning bytecode root. Deref exposes shared fields for
//! readers, never mutable metadata or a constructor accepting arbitrary parts.
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::Atom;
use crate::engine::code::function::metadata::{
    ClosureVariable, EvalEnvironment, FunctionMetadata, VariableDefinition,
};
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::heap::{BytecodeConstant, ContextId};
use std::rc::Rc;

/// A rooted, immutable eval descriptor selected from its publisher's array.
/// Cloning this view shares the array; it never copies scopes or bindings.
#[derive(Clone)]
pub(crate) struct PublishedEvalEnvironment {
    owner: FunctionBytecodeRef,
    environments: Rc<[EvalEnvironment<Atom>]>,
    index: usize,
}

impl PublishedEvalEnvironment {
    pub(crate) fn same_environment(&self, other: &Self) -> bool {
        self.index == other.index && Rc::ptr_eq(&self.environments, &other.environments)
    }

    pub(crate) fn owner(&self) -> &FunctionBytecodeRef {
        &self.owner
    }
}

impl std::ops::Deref for PublishedEvalEnvironment {
    type Target = EvalEnvironment<Atom>;

    fn deref(&self) -> &Self::Target {
        &self.environments[self.index]
    }
}

pub(crate) struct PublishedFunctionSnapshot {
    root: Option<FunctionBytecodeRef>,
    data: PublishedFunctionData,
}

impl std::ops::Deref for PublishedFunctionSnapshot {
    type Target = PublishedFunctionData;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl PublishedFunctionSnapshot {
    /// One checked projection for all constant consumers. The opcode still
    /// chooses the kind-specific operation; this view owns no extra roots.
    #[inline]
    pub(crate) fn constant(&self, index: u32) -> Option<&BytecodeConstant> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.constants.get(index))
    }

    pub(crate) fn eval_environment(&self, index: u16) -> Option<PublishedEvalEnvironment> {
        let index = usize::from(index);
        self.eval_environments.get(index)?;
        Some(PublishedEvalEnvironment {
            owner: self.root.as_ref()?.clone(),
            environments: self.eval_environments.clone(),
            index,
        })
    }

    pub(crate) fn root(&self) -> Option<&FunctionBytecodeRef> {
        self.root.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test(realm: ContextId) -> Self {
        Self {
            root: None,
            data: PublishedFunctionData {
                code: Rc::from([]),
                constants: Rc::from([]),
                property_key_atoms: None,
                argument_definitions: Rc::from([]),
                local_definitions: Rc::from([]),
                closure_variables: Rc::from([]),
                eval_environments: Rc::from([]),
                arg_eval_variable_object_local: None,
                metadata: FunctionMetadata::default(),
                realm,
            },
        }
    }
}

// Synthetic host fixtures exercise rejected internal operations. They never
// provide a production constructor or a mutable view in non-test builds.
#[cfg(test)]
impl std::ops::DerefMut for PublishedFunctionSnapshot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        assert!(
            self.root.is_none(),
            "published snapshots remain immutable in tests"
        );
        &mut self.data
    }
}

pub(crate) struct PublishedFunctionData {
    pub(crate) code: Rc<[crate::engine::code::bytecode::Instruction]>,
    pub(crate) constants: Rc<[BytecodeConstant]>,
    pub(crate) property_key_atoms: Option<Rc<[Atom]>>,
    pub(crate) argument_definitions: Rc<[VariableDefinition]>,
    pub(crate) local_definitions: Rc<[VariableDefinition]>,
    pub(crate) closure_variables: Rc<[ClosureVariable]>,
    pub(crate) eval_environments: Rc<[EvalEnvironment<Atom>]>,
    /// Parameter-scope variable-object slot, carried separately from the
    /// body `<var>` slot in `FunctionMetadata`.
    pub(crate) arg_eval_variable_object_local: Option<u16>,
    pub(crate) metadata: FunctionMetadata,
    pub(crate) realm: ContextId,
}

impl Runtime {
    pub(crate) fn snapshot_function_bytecode(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<PublishedFunctionSnapshot, RuntimeError> {
        let _operation = self.operation();
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let root = function.clone();
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        // The realm is a strong edge of the bytecode node. Validating it here
        // makes a corrupt realm edge fail before entering a VM frame.
        state.heap.context(bytecode.realm)?;
        Ok(PublishedFunctionSnapshot {
            root: Some(root),
            data: PublishedFunctionData {
                code: bytecode.code.clone(),
                constants: bytecode.constants.clone(),
                property_key_atoms: bytecode.property_key_atoms.clone(),
                argument_definitions: bytecode.argument_definitions.clone(),
                local_definitions: bytecode.local_definitions.clone(),
                closure_variables: bytecode.closure_variables.clone(),
                eval_environments: bytecode.eval_environments.clone(),
                arg_eval_variable_object_local: bytecode
                    .parameter_environment
                    .as_ref()
                    .and_then(|layout| layout.arg_eval_variable_object_local),
                metadata: bytecode.metadata,
                realm: bytecode.realm,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::code::bytecode::Instruction;
    use crate::engine::code::function::UnlinkedFunction;

    fn publish(runtime: &Runtime, realm: ContextId) -> FunctionBytecodeRef {
        runtime
            .publish_unlinked_function(
                realm,
                UnlinkedFunction::fixture(
                    vec![Instruction::PushI32(42), Instruction::Return],
                    vec![],
                    FunctionMetadata {
                        max_stack: 1,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap()
    }

    #[test]
    fn snapshot_retains_its_owner_and_rejects_another_runtime() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let function = publish(&runtime, context.realm);
        let other = Runtime::new();
        assert!(matches!(
            other.snapshot_function_bytecode(&function),
            Err(RuntimeError::WrongRuntime("function bytecode"))
        ));
        let snapshot = runtime.snapshot_function_bytecode(&function).unwrap();
        let id = function.bytecode_id();
        drop(function);
        assert_eq!(snapshot.root().unwrap().bytecode_id(), id);
        assert!(matches!(
            snapshot.code.as_ref(),
            [Instruction::PushI32(42), Instruction::Return]
        ));
        assert!(runtime.0.state.borrow().heap.function_bytecode(id).is_ok());
    }

    #[test]
    fn eval_view_shares_storage_but_authenticates_the_selected_environment() {
        use crate::engine::code::function::metadata::EvalVariableEnvironment;
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let owner = publish(&runtime, context.realm);
        let id = owner.bytecode_id();
        let environment = EvalEnvironment {
            scopes: Box::new([]),
            variable_environment: EvalVariableEnvironment::Global,
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };
        let view = PublishedEvalEnvironment {
            owner,
            environments: Rc::from([environment.clone(), environment.clone()]),
            index: 0,
        };
        let shared = view.clone();
        assert!(view.same_environment(&shared));
        let mut another_index = view.clone();
        another_index.index = 1;
        assert!(!view.same_environment(&another_index));
        let mut another_owner = view.clone();
        another_owner.environments = Rc::from([environment]);
        assert!(!view.same_environment(&another_owner));
        drop(view);
        drop(another_index);
        drop(another_owner);
        assert!(runtime.0.state.borrow().heap.function_bytecode(id).is_ok());
        assert!(!shared.caller_strict);
        drop(shared);
        assert!(runtime.0.state.borrow().heap.function_bytecode(id).is_err());
    }

    #[test]
    #[should_panic(expected = "published snapshots remain immutable in tests")]
    fn synthetic_fixture_mutation_cannot_change_published_code() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let function = publish(&runtime, context.realm);
        let mut snapshot = runtime.snapshot_function_bytecode(&function).unwrap();
        snapshot.code = Rc::from([]);
    }
}
