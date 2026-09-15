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
use crate::engine::heap::{BytecodeConstant, ContextId, FunctionBytecodeId};
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

/// Heap-resident certificate contains only immutable publication facts, never
/// Runtime or an external root. The function payload owns the bytecode edge.
#[derive(Debug, Clone)]
pub(crate) struct OrdinaryAuthentication {
    pub(crate) publish_generation: u64,
    pub(crate) closure_count: usize,
    pub(crate) data: Rc<PublishedFunctionData>,
}

impl PartialEq for OrdinaryAuthentication {
    fn eq(&self, other: &Self) -> bool {
        self.publish_generation == other.publish_generation
            && self.closure_count == other.closure_count
            && Rc::ptr_eq(&self.data, &other.data)
    }
}

pub(crate) struct PublishedFunctionSnapshot {
    root: std::cell::OnceCell<FunctionBytecodeRef>,
    bytecode: Option<FunctionBytecodeId>,
    runtime_identity: usize,
    data: Rc<PublishedFunctionData>,
}

impl std::ops::Deref for PublishedFunctionSnapshot {
    type Target = PublishedFunctionData;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl PublishedFunctionSnapshot {
    /// Borrow every static binding classification from this rooted owner.
    pub(crate) fn frame_layout(&self) -> crate::engine::code::function::layout::FrameLayout<'_> {
        crate::engine::code::function::layout::FrameLayout::new(
            &self.metadata,
            &self.argument_definitions,
            &self.local_definitions,
            &self.closure_variables,
        )
    }

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
            owner: self.root.get()?.clone(),
            environments: self.eval_environments.clone(),
            index,
        })
    }

    pub(crate) fn root(&self) -> Option<&FunctionBytecodeRef> {
        self.root.get()
    }

    /// Domain token is non-owning; a rooted snapshot or its frame's callee
    /// owns Runtime for every access. It avoids retaining Runtime in caches.
    pub(crate) fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.runtime_identity == Rc::as_ptr(&runtime.0) as usize
    }

    pub(crate) fn bytecode_id(&self) -> Option<FunctionBytecodeId> {
        self.bytecode
    }

    /// Cold observation boundary. Ordinary frames are already kept alive by
    /// their callee owner and therefore do not acquire this independent root
    /// until eval, suspension, or host materialization actually needs one.
    pub(crate) fn ensure_root(&self, runtime: &Runtime) -> Result<(), RuntimeError> {
        if self.bytecode.is_some() && !self.belongs_to(runtime) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        if self.root.get().is_none() {
            if let Some(id) = self.bytecode {
                let root = FunctionBytecodeRef::from_borrowed_handle(runtime.clone(), id)?;
                let _ = self.root.set(root);
            }
        }
        Ok(())
    }

    pub(crate) fn authentication(&self, closure_count: usize) -> OrdinaryAuthentication {
        OrdinaryAuthentication {
            publish_generation: self.bytecode.unwrap().publish_generation(),
            closure_count,
            data: self.data.clone(),
        }
    }

    /// Caller holds the owning function and has checked the certificate's
    /// generation and closure fact in the same immutable heap borrow.
    pub(crate) fn from_authentication(
        runtime: &Runtime,
        id: FunctionBytecodeId,
        facts: OrdinaryAuthentication,
    ) -> Self {
        Self {
            root: Default::default(),
            bytecode: Some(id),
            runtime_identity: Rc::as_ptr(&runtime.0) as usize,
            data: facts.data,
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test(realm: ContextId) -> Self {
        Self {
            root: Default::default(),
            bytecode: None,
            runtime_identity: 0,
            data: Rc::new(PublishedFunctionData {
                has_captured_locals: true,
                observes_arguments: true,
                #[cfg(feature = "stack-vm")]
                fusion: Default::default(),
                #[cfg(feature = "stack-vm")]
                property_read_ic: crate::engine::object::property_ic::PropertyReadCacheTable::new(
                    &[],
                ),
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
            }),
        }
    }
}

// Synthetic host fixtures exercise rejected internal operations. They never
// provide a production constructor or a mutable view in non-test builds.
#[cfg(test)]
impl std::ops::DerefMut for PublishedFunctionSnapshot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        assert!(
            self.bytecode.is_none(),
            "published snapshots remain immutable in tests"
        );
        Rc::get_mut(&mut self.data).expect("synthetic executable remains uniquely owned")
    }
}

#[derive(Debug)]
pub(crate) struct PublishedFunctionData {
    pub(crate) has_captured_locals: bool,
    pub(crate) observes_arguments: bool,
    #[cfg(feature = "stack-vm")]
    pub(crate) fusion: crate::engine::code::fusion::FusionPlan,
    #[cfg(feature = "stack-vm")]
    pub(crate) property_read_ic: crate::engine::object::property_ic::PropertyReadCacheTable,
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
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        self.snapshot_function_bytecode_owned(function.clone())
    }

    pub(crate) fn snapshot_function_bytecode_owned(
        &self,
        function: FunctionBytecodeRef,
    ) -> Result<PublishedFunctionSnapshot, RuntimeError> {
        let _operation = self.operation();
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        // The realm is a strong edge of the bytecode node. Validating it here
        // makes a corrupt realm edge fail before entering a VM frame.
        state.heap.context(bytecode.realm)?;
        let data = bytecode.executable.get_or_init(|| {
            let data = Rc::new(PublishedFunctionData {
                has_captured_locals: !bytecode.local_definitions.is_empty()
                    && bytecode.code.iter().any(|op| {
                        matches!(
                            op,
                            crate::engine::code::bytecode::Instruction::FClosure(_)
                                | crate::engine::code::bytecode::Instruction::Eval { .. }
                                | crate::engine::code::bytecode::Instruction::ApplyEval { .. }
                        )
                    }),
                observes_arguments: bytecode.code.iter().any(|op| {
                    matches!(
                        op,
                        crate::engine::code::bytecode::Instruction::Arguments(_)
                            | crate::engine::code::bytecode::Instruction::Rest(_)
                            | crate::engine::code::bytecode::Instruction::Eval { .. }
                            | crate::engine::code::bytecode::Instruction::ApplyEval { .. }
                    )
                }),
                #[cfg(feature = "stack-vm")]
                fusion: bytecode.fusion.clone(),
                #[cfg(feature = "stack-vm")]
                property_read_ic: crate::engine::object::property_ic::PropertyReadCacheTable::new(
                    &bytecode.code,
                ),
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
            });
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_call_buffer_capacity(
                "executable.published_data_rc",
                0,
                1,
                size_of::<PublishedFunctionData>(),
            );
            data
        });
        let data = data.clone();
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_share(
            "executable.published_data_rc",
            1,
            size_of::<PublishedFunctionData>(),
        );

        Ok(PublishedFunctionSnapshot {
            runtime_identity: Rc::as_ptr(&self.0) as usize,
            bytecode: Some(function.bytecode_id()),
            root: std::cell::OnceCell::from(function),
            data,
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
        let second = runtime.snapshot_function_bytecode(&function).unwrap();
        assert!(Rc::ptr_eq(&snapshot.data, &second.data));
        // Two frame headers share one projection but each keeps the bytecode
        // root alive independently. The cache itself owns no rooting handle.
        drop(second);
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
