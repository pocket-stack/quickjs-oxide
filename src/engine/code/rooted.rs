//! Runtime-rooted immutable function bytecode and compiler drafts.
//!
//! QuickJS publishes `JSFunctionBytecode` as a runtime GC node: its constant
//! pool, child bytecode, atoms, and realm are owned by that node.  This module
//! preserves the same boundary without exposing heap identities or mutable
//! bytecode to safe callers.
//!
//! [`FunctionBytecodeRef`] is the public owning root.  Heap payloads store raw
//! [`FunctionBytecodeId`] edges, while cloning or dropping a public root updates
//! the runtime's external reference count.  Compilation first produces an
//! [`UnlinkedFunction`]; the runtime consumes that draft transactionally and
//! publishes an immutable heap node.  A draft cannot contain runtime-owned
//! objects or symbols, so it cannot accidentally join two runtime domains
//! before publication.

use std::fmt;
use std::hash::{Hash, Hasher};

use crate::engine::api::runtime::Runtime;
use crate::engine::heap::{FunctionBytecodeId, HeapError};

/// A public owning root for immutable runtime-local function bytecode.
///
/// The fields and constructors are not public.  Only runtime publication and
/// checked raw-edge promotion may create a root, which prevents a bytecode ID
/// from one runtime being paired with another runtime domain.
pub struct FunctionBytecodeRef {
    runtime: Runtime,
    id: FunctionBytecodeId,
}

impl FunctionBytecodeRef {
    /// Consume one function-bytecode reference already owned by the caller.
    ///
    /// The runtime uses this after transactional publication; it deliberately
    /// does not retain the newly allocated node a second time.
    #[must_use]
    pub(crate) const fn from_owned_handle(runtime: Runtime, id: FunctionBytecodeId) -> Self {
        Self { runtime, id }
    }

    /// Promote a borrowed raw heap edge to a public owning root.
    pub(crate) fn from_borrowed_handle(
        runtime: Runtime,
        id: FunctionBytecodeId,
    ) -> Result<Self, HeapError> {
        runtime.retain_function_bytecode_handle(id)?;
        Ok(Self { runtime, id })
    }

    /// Duplicate this root while preserving a checked internal path for the
    /// runtime and tests.  Public [`Clone`] treats failure as an invariant or
    /// resource-exhaustion violation because a live root cannot be stale.
    pub(crate) fn try_clone(&self) -> Result<Self, HeapError> {
        self.runtime.retain_function_bytecode_handle(self.id)?;
        Ok(Self {
            runtime: self.runtime.clone(),
            id: self.id,
        })
    }

    /// Return the runtime which owns this bytecode root.
    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Return whether this bytecode belongs to `runtime`.
    #[must_use]
    pub fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.runtime.is_same_runtime(runtime)
    }

    /// Return whether two bytecode roots belong to the same runtime domain.
    #[must_use]
    pub fn is_same_runtime(&self, other: &Self) -> bool {
        self.runtime.is_same_runtime(&other.runtime)
    }

    /// Stable identity of the owning runtime domain.
    #[must_use]
    pub fn domain_id(&self) -> u64 {
        self.runtime.domain_id()
    }

    /// Raw identity for runtime, heap, and executor internals.
    #[must_use]
    pub(crate) const fn bytecode_id(&self) -> FunctionBytecodeId {
        self.id
    }
}

impl Clone for FunctionBytecodeRef {
    fn clone(&self) -> Self {
        self.try_clone().unwrap_or_else(|_| {
            panic!("attempted to clone stale function bytecode or overflow its reference count")
        })
    }
}

impl Drop for FunctionBytecodeRef {
    fn drop(&mut self) {
        self.runtime.release_function_bytecode_handle(self.id);
    }
}

impl PartialEq for FunctionBytecodeRef {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.runtime.is_same_runtime(&other.runtime)
    }
}

impl Eq for FunctionBytecodeRef {}

impl Hash for FunctionBytecodeRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.runtime.domain_id().hash(state);
        self.id.hash(state);
    }
}

impl fmt::Debug for FunctionBytecodeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FunctionBytecodeRef")
            .field("domain_id", &self.runtime.domain_id())
            .field("id", &self.id)
            .finish()
    }
}
#[cfg(test)]
mod tests {
    use crate::engine::code::bytecode::Instruction;
    use crate::engine::code::function::metadata::{
        ClosureSource, ClosureVariable, ClosureVariableKind, EvalEnvironment, EvalScope,
        EvalScopeKind, EvalVariableEnvironment, FunctionMetadata,
    };
    use crate::engine::code::function::{
        UnlinkedConstant, UnlinkedConstantError, UnlinkedFunction, UnlinkedVariableDefinition,
    };
    use crate::engine::value::bigint::JsBigInt;
    use crate::engine::value::{JsString, Value};

    #[test]
    fn detached_constant_pool_accepts_every_runtime_independent_primitive() {
        let values = [
            Value::Undefined,
            Value::Null,
            Value::Bool(true),
            Value::Int(42),
            Value::Float(-0.0),
            Value::BigInt(JsBigInt::one()),
            Value::String(JsString::try_from_utf16([0xd800, 0x61]).unwrap()),
        ];

        for value in values {
            let constant = UnlinkedConstant::primitive(value).unwrap();
            assert!(constant.as_primitive().is_some());
            assert!(constant.as_child().is_none());
            let (primitive, atom_string, child) = constant.into_parts();
            assert!(primitive.is_some());
            assert!(!atom_string);
            assert!(child.is_none());
        }
    }

    #[test]
    fn child_draft_is_distinct_from_a_primitive_constant() {
        let child = UnlinkedFunction::fixture(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata::default(),
        );
        let constant = UnlinkedConstant::child(child);

        assert!(constant.as_primitive().is_none());
        assert_eq!(constant.as_child().unwrap().code().len(), 2);
        let (primitive, atom_string, child) = constant.into_parts();
        assert!(primitive.is_none());
        assert!(!atom_string);
        assert!(child.is_some());
    }

    #[test]
    fn compiler_atom_string_keeps_a_structural_publication_marker() {
        let constant = UnlinkedConstant::atom_string(JsString::from_static("literal"));
        assert!(!constant.is_empty_atom_string());
        assert!(matches!(
            constant.as_primitive(),
            Some(crate::engine::value::PrimitiveValue::String(_))
        ));
        assert!(constant.as_child().is_none());
        let (primitive, atom_string, child) = constant.into_parts();
        assert!(matches!(
            primitive,
            Some(crate::engine::value::PrimitiveValue::String(_))
        ));
        assert!(atom_string);
        assert!(child.is_none());
    }

    #[test]
    fn empty_atom_string_is_the_only_narrow_ordinary_leaf_exception() {
        let atom = UnlinkedConstant::atom_string(JsString::from_static(""));
        let primitive =
            UnlinkedConstant::primitive(Value::String(JsString::from_static(""))).unwrap();

        assert!(atom.is_empty_atom_string());
        assert!(!atom.is_plain_primitive());
        assert!(!primitive.is_empty_atom_string());
        assert!(primitive.is_plain_primitive());
    }

    #[test]
    fn regexp_literal_constant_is_a_runtime_independent_rc_leaf() {
        let pattern = JsString::from_static("a");
        let flags = JsString::from_static("g");
        let program = std::rc::Rc::new(crate::regexp::compile(&pattern, &flags).unwrap());
        let weak = std::rc::Rc::downgrade(&program);
        let constant = UnlinkedConstant::regexp(pattern.clone(), program);

        assert!(constant.as_primitive().is_none());
        assert!(constant.as_child().is_none());
        let (borrowed_pattern, _) = constant.as_regexp().unwrap();
        assert_eq!(borrowed_pattern, &pattern);

        let (owned_pattern, owned_program) = constant.into_regexp().unwrap();
        assert_eq!(owned_pattern, pattern);
        assert!(weak.upgrade().is_some());
        drop(owned_program);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn runtime_bound_rejection_has_stable_diagnostics() {
        assert_eq!(
            UnlinkedConstantError::RuntimeBoundObject.to_string(),
            "an object cannot enter a runtime-independent constant pool"
        );
        assert_eq!(
            UnlinkedConstantError::RuntimeBoundSymbol.to_string(),
            "a symbol cannot enter a runtime-independent constant pool"
        );
    }

    #[test]
    fn publication_consumes_code_constants_and_metadata_together() {
        let function = UnlinkedFunction::fixture(
            vec![Instruction::PushConst(0), Instruction::Return],
            vec![UnlinkedConstant::primitive(Value::Int(7)).unwrap()],
            FunctionMetadata::default(),
        );
        assert_eq!(function.code().len(), 2);
        assert_eq!(function.constants().len(), 1);
        let _metadata = function.metadata();

        let parts = function.into_parts();
        assert_eq!(parts.code.len(), 2);
        assert_eq!(parts.constants.len(), 1);
        assert!(parts.closure_variables.is_empty());
        assert!(parts.eval_environments.is_empty());
    }

    #[test]
    fn eval_environments_stay_attached_across_the_publication_boundary() {
        let environment = EvalEnvironment {
            scopes: vec![EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            }]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::StrictLocal(0),
            caller_strict: true,
            super_call_allowed: false,
            super_allowed: false,
        };
        let function = UnlinkedFunction::fixture(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                strict: true,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![environment.clone()]);

        assert_eq!(
            function.eval_environments(),
            std::slice::from_ref(&environment)
        );
        assert_eq!(function.into_parts().eval_environments, vec![environment]);
    }

    #[test]
    fn closure_descriptors_stay_attached_to_the_child_draft() {
        let descriptor = ClosureVariable {
            source: ClosureSource::ParentArgument(0),
            name: crate::engine::code::function::metadata::ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: crate::engine::code::function::metadata::ClosureVariableKind::Normal,
        };
        let function = UnlinkedFunction::fixture_with_closure_variables(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![descriptor],
        );

        assert_eq!(function.closure_variables(), &[descriptor]);
        assert_eq!(function.into_parts().closure_variables, vec![descriptor]);
    }

    #[test]
    fn constructor_accepts_authored_definitions_and_names_the_function_slot() {
        let name = JsString::from_static("selfName");
        let function = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                argument_count: 1,
                defined_argument_count: 1,
                local_count: 2,
                function_name_local: Some(1),
                strict: true,
                ..FunctionMetadata::default()
            },
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("argument"),
            ))],
            vec![
                UnlinkedVariableDefinition::lexical(Some(JsString::from_static("lexical")), false),
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("wrong"))),
            ],
            Vec::new(),
        )
        .with_name(Some(name.clone()));

        assert_eq!(function.argument_definitions().len(), 1);
        assert_eq!(function.local_definitions().len(), 2);
        assert!(function.local_definitions()[0].is_lexical);
        assert_eq!(
            function.local_definitions()[0].kind,
            ClosureVariableKind::Normal
        );
        assert_eq!(function.local_definitions()[1].name.as_ref(), Some(&name));
        assert!(!function.local_definitions()[1].is_lexical);
        assert!(function.local_definitions()[1].is_const);
        assert_eq!(
            function.local_definitions()[1].kind,
            ClosureVariableKind::FunctionName
        );
    }

    #[test]
    fn constructors_authenticate_the_eval_variable_object_slot() {
        let function = UnlinkedFunction::fixture(
            vec![Instruction::VariableEnvironment, Instruction::PutLocal(0)],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                eval_variable_object_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );

        let [definition] = function.local_definitions() else {
            panic!("eval variable-object function did not have one local")
        };
        assert_eq!(
            definition.name.as_ref().map(JsString::to_utf8_lossy),
            Some("<var>".to_owned())
        );
        assert!(!definition.is_lexical);
        assert!(!definition.is_const);
        assert_eq!(definition.kind, ClosureVariableKind::EvalVariableObject);
    }
}
