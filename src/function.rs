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

use std::error::Error;
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::bytecode::Instruction;
use crate::debug::Pc2LineTable;
use crate::heap::{
    ClosureVariable, ClosureVariableKind, FunctionBytecodeId, FunctionMetadata, HeapError,
};
use crate::runtime::Runtime;
use crate::value::Value;

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

/// Why a value cannot enter a runtime-independent constant pool draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnlinkedConstantError {
    /// Objects are runtime heap identities and must be linked through raw edges.
    RuntimeBoundObject,
    /// Symbols are runtime atom identities and need a dedicated linked form.
    RuntimeBoundSymbol,
}

impl fmt::Display for UnlinkedConstantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeBoundObject => {
                formatter.write_str("an object cannot enter a runtime-independent constant pool")
            }
            Self::RuntimeBoundSymbol => {
                formatter.write_str("a symbol cannot enter a runtime-independent constant pool")
            }
        }
    }
}

impl Error for UnlinkedConstantError {}

/// Private representation keeps the primitive-only invariant structural.
///
/// Making the enum itself crate-visible would let any module construct
/// `Primitive(Value::Object(_))` and bypass the checked constructor.
#[derive(Debug)]
#[allow(dead_code)] // Constructed by the function-syntax compiler slice.
enum UnlinkedConstantKind {
    Primitive(Value),
    Child(Box<UnlinkedFunction>),
}

/// One constant in a compiler draft, before runtime publication.
///
/// Primitive constants may contain undefined, null, booleans, numbers,
/// BigInts, and strings.  Symbols are ECMAScript primitives but remain
/// runtime-owned atom identities, so they intentionally use a future dedicated
/// linked representation instead of this draft variant.
#[derive(Debug)]
pub(crate) struct UnlinkedConstant(UnlinkedConstantKind);

impl UnlinkedConstant {
    /// Construct a runtime-independent primitive constant.
    ///
    /// # Errors
    ///
    /// Rejects object and symbol roots so an unlinked function cannot retain or
    /// combine runtime domains.
    pub(crate) fn primitive(value: Value) -> Result<Self, UnlinkedConstantError> {
        match value {
            Value::Object(_) => Err(UnlinkedConstantError::RuntimeBoundObject),
            Value::Symbol(_) => Err(UnlinkedConstantError::RuntimeBoundSymbol),
            primitive => Ok(Self(UnlinkedConstantKind::Primitive(primitive))),
        }
    }

    /// Store one recursively compiled child-function draft.
    #[must_use]
    #[allow(dead_code)] // The runtime publisher already supports this future compiler output.
    pub(crate) fn child(function: UnlinkedFunction) -> Self {
        Self(UnlinkedConstantKind::Child(Box::new(function)))
    }

    /// Borrow the primitive value, or return `None` for a child function.
    #[must_use]
    pub(crate) fn as_primitive(&self) -> Option<&Value> {
        match &self.0 {
            UnlinkedConstantKind::Primitive(value) => Some(value),
            UnlinkedConstantKind::Child(_) => None,
        }
    }

    /// Borrow the child draft, or return `None` for a primitive constant.
    #[must_use]
    pub(crate) fn as_child(&self) -> Option<&UnlinkedFunction> {
        match &self.0 {
            UnlinkedConstantKind::Primitive(_) => None,
            UnlinkedConstantKind::Child(function) => Some(function),
        }
    }

    /// Consume this constant for transactional runtime publication.
    ///
    /// Exactly one tuple field is `Some`.  Returning output-only options keeps
    /// the private representation invariant from being bypassed by callers.
    #[must_use]
    pub(crate) fn into_parts(self) -> (Option<Value>, Option<UnlinkedFunction>) {
        match self.0 {
            UnlinkedConstantKind::Primitive(value) => (Some(value), None),
            UnlinkedConstantKind::Child(function) => (None, Some(*function)),
        }
    }
}

impl TryFrom<Value> for UnlinkedConstant {
    type Error = UnlinkedConstantError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        Self::primitive(value)
    }
}

/// Mutable compiler output which has not entered a runtime domain yet.
///
/// The draft owns its vectors, but its fields stay private.  Publication
/// consumes the whole value through [`Self::into_parts`], verifies it, interns
/// runtime atoms, recursively publishes child functions, retains all outgoing
/// edges transactionally, and only then returns a [`FunctionBytecodeRef`].
/// Published bytecode therefore has no mutation path back to this draft.
#[derive(Debug)]
pub(crate) struct UnlinkedFunction {
    code: Vec<Instruction>,
    constants: Vec<UnlinkedConstant>,
    metadata: FunctionMetadata,
    func_name: Option<crate::value::JsString>,
    argument_definitions: Vec<UnlinkedVariableDefinition>,
    local_definitions: Vec<UnlinkedVariableDefinition>,
    closure_variables: Vec<ClosureVariable>,
    debug: Option<UnlinkedFunctionDebug>,
}

/// Runtime-independent form of QuickJS's `JSVarDef`/argument metadata.
///
/// Names remain optional because synthetic locals do not have an observable
/// identifier. Publication interns every present name and transfers the
/// resulting atom ownership to immutable function bytecode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnlinkedVariableDefinition {
    pub(crate) name: Option<crate::value::JsString>,
    pub(crate) is_lexical: bool,
    pub(crate) is_const: bool,
    pub(crate) kind: ClosureVariableKind,
}

impl UnlinkedVariableDefinition {
    /// Construct an ordinary mutable argument/local definition.
    #[must_use]
    pub(crate) const fn ordinary(name: Option<crate::value::JsString>) -> Self {
        Self {
            name,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }
    }

    /// Construct a block-scoped mutable or immutable definition.
    #[must_use]
    #[allow(dead_code)] // Consumed once the parser starts emitting lexical declarations.
    pub(crate) const fn lexical(name: Option<crate::value::JsString>, is_const: bool) -> Self {
        Self {
            name,
            is_lexical: true,
            is_const,
            kind: ClosureVariableKind::Normal,
        }
    }

    fn function_name(name: Option<crate::value::JsString>, is_const: bool) -> Self {
        Self {
            name,
            is_lexical: false,
            is_const,
            kind: ClosureVariableKind::FunctionName,
        }
    }
}

/// Runtime-independent debug payload produced by the compiler.
///
/// Publication interns the filename separately for every function. Function
/// source is an independent byte copy so a nested closure does not retain its
/// complete enclosing script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnlinkedFunctionDebug {
    pub(crate) filename: crate::value::JsString,
    pub(crate) pc2line: Option<Pc2LineTable>,
    pub(crate) source: Option<Box<[u8]>>,
}

/// Owned pieces crossing the one-way publication boundary.
pub(crate) struct UnlinkedFunctionParts {
    pub(crate) code: Vec<Instruction>,
    pub(crate) constants: Vec<UnlinkedConstant>,
    pub(crate) metadata: FunctionMetadata,
    pub(crate) func_name: Option<crate::value::JsString>,
    pub(crate) argument_definitions: Vec<UnlinkedVariableDefinition>,
    pub(crate) local_definitions: Vec<UnlinkedVariableDefinition>,
    pub(crate) closure_variables: Vec<ClosureVariable>,
    pub(crate) debug: Option<UnlinkedFunctionDebug>,
}

impl UnlinkedFunction {
    fn ordinary_definitions(
        metadata: FunctionMetadata,
    ) -> (
        Vec<UnlinkedVariableDefinition>,
        Vec<UnlinkedVariableDefinition>,
    ) {
        let arguments = (0..metadata.argument_count)
            .map(|_| UnlinkedVariableDefinition::ordinary(None))
            .collect();
        let mut locals = (0..metadata.local_count)
            .map(|_| UnlinkedVariableDefinition::ordinary(None))
            .collect::<Vec<_>>();
        if let Some(index) = metadata.function_name_local {
            if let Some(definition) = locals.get_mut(usize::from(index)) {
                *definition = UnlinkedVariableDefinition::function_name(None, metadata.strict);
            }
        }
        (arguments, locals)
    }

    /// Assemble a complete compiler draft.
    #[must_use]
    pub(crate) fn new(
        code: Vec<Instruction>,
        constants: Vec<UnlinkedConstant>,
        metadata: FunctionMetadata,
    ) -> Self {
        let (argument_definitions, local_definitions) = Self::ordinary_definitions(metadata);
        Self {
            code,
            constants,
            metadata,
            func_name: None,
            argument_definitions,
            local_definitions,
            closure_variables: Vec::new(),
            debug: None,
        }
    }

    /// Assemble a compiler draft whose function object receives captured
    /// bindings when its parent's `FClosure` opcode executes.
    ///
    /// The publisher validates the descriptor count and every parent source
    /// before any heap node is allocated. Keeping these descriptors on the
    /// child mirrors QuickJS's `JSFunctionBytecode::closure_var` ownership.
    #[must_use]
    #[allow(dead_code)] // First consumed by the pending function-syntax compiler slice.
    pub(crate) fn new_with_closure_variables(
        code: Vec<Instruction>,
        constants: Vec<UnlinkedConstant>,
        metadata: FunctionMetadata,
        closure_variables: Vec<ClosureVariable>,
    ) -> Self {
        let (argument_definitions, local_definitions) = Self::ordinary_definitions(metadata);
        Self {
            code,
            constants,
            metadata,
            func_name: None,
            argument_definitions,
            local_definitions,
            closure_variables,
            debug: None,
        }
    }

    /// Attach the source-level intrinsic name of a function expression.
    #[must_use]
    pub(crate) fn with_name(mut self, name: Option<crate::value::JsString>) -> Self {
        self.func_name = name.clone();
        if let Some(index) = self.metadata.function_name_local {
            if let Some(definition) = self.local_definitions.get_mut(usize::from(index)) {
                *definition = UnlinkedVariableDefinition::function_name(name, self.metadata.strict);
            }
        }
        self
    }

    /// Replace the compatibility definitions with compiler-produced binding
    /// metadata. Publication remains the validation boundary for counts and
    /// flag combinations.
    #[must_use]
    pub(crate) fn with_variable_definitions(
        mut self,
        argument_definitions: Vec<UnlinkedVariableDefinition>,
        mut local_definitions: Vec<UnlinkedVariableDefinition>,
    ) -> Self {
        if let Some(index) = self.metadata.function_name_local {
            if let Some(definition) = local_definitions.get_mut(usize::from(index)) {
                *definition = UnlinkedVariableDefinition::function_name(
                    self.func_name.clone(),
                    self.metadata.strict,
                );
            }
        }
        self.argument_definitions = argument_definitions;
        self.local_definitions = local_definitions;
        self
    }

    /// Attach compiler-produced source metadata before publication.
    #[must_use]
    pub(crate) fn with_debug(mut self, debug: UnlinkedFunctionDebug) -> Self {
        self.debug = Some(debug);
        self
    }

    #[must_use]
    pub(crate) fn code(&self) -> &[Instruction] {
        &self.code
    }

    #[must_use]
    pub(crate) fn constants(&self) -> &[UnlinkedConstant] {
        &self.constants
    }

    #[must_use]
    pub(crate) const fn metadata(&self) -> &FunctionMetadata {
        &self.metadata
    }

    #[must_use]
    pub(crate) fn func_name(&self) -> Option<&crate::value::JsString> {
        self.func_name.as_ref()
    }

    #[must_use]
    pub(crate) fn closure_variables(&self) -> &[ClosureVariable] {
        &self.closure_variables
    }

    #[must_use]
    pub(crate) fn argument_definitions(&self) -> &[UnlinkedVariableDefinition] {
        &self.argument_definitions
    }

    #[must_use]
    pub(crate) fn local_definitions(&self) -> &[UnlinkedVariableDefinition] {
        &self.local_definitions
    }

    #[must_use]
    pub(crate) const fn debug(&self) -> Option<&UnlinkedFunctionDebug> {
        self.debug.as_ref()
    }

    /// Consume the draft at the immutable publication boundary.
    #[must_use]
    pub(crate) fn into_parts(self) -> UnlinkedFunctionParts {
        UnlinkedFunctionParts {
            code: self.code,
            constants: self.constants,
            metadata: self.metadata,
            func_name: self.func_name,
            argument_definitions: self.argument_definitions,
            local_definitions: self.local_definitions,
            closure_variables: self.closure_variables,
            debug: self.debug,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        UnlinkedConstant, UnlinkedConstantError, UnlinkedFunction, UnlinkedVariableDefinition,
    };
    use crate::bigint::JsBigInt;
    use crate::bytecode::Instruction;
    use crate::heap::{ClosureSource, ClosureVariable, ClosureVariableKind, FunctionMetadata};
    use crate::value::{JsString, Value};

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
            let (primitive, child) = constant.into_parts();
            assert!(primitive.is_some());
            assert!(child.is_none());
        }
    }

    #[test]
    fn child_draft_is_distinct_from_a_primitive_constant() {
        let child = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata::default(),
        );
        let constant = UnlinkedConstant::child(child);

        assert!(constant.as_primitive().is_none());
        assert_eq!(constant.as_child().unwrap().code().len(), 2);
        let (primitive, child) = constant.into_parts();
        assert!(primitive.is_none());
        assert!(child.is_some());
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
        let function = UnlinkedFunction::new(
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
    }

    #[test]
    fn closure_descriptors_stay_attached_to_the_child_draft() {
        let descriptor = ClosureVariable {
            source: ClosureSource::ParentArgument(0),
            name: crate::heap::ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: crate::heap::ClosureVariableKind::Normal,
        };
        let function = UnlinkedFunction::new_with_closure_variables(
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
    fn constructors_supply_ordinary_definitions_and_normalize_the_function_name_slot() {
        let name = JsString::from_static("selfName");
        let function = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                argument_count: 1,
                local_count: 2,
                function_name_local: Some(1),
                strict: true,
                ..FunctionMetadata::default()
            },
        )
        .with_name(Some(name.clone()))
        .with_variable_definitions(
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("argument"),
            ))],
            vec![
                UnlinkedVariableDefinition::lexical(Some(JsString::from_static("lexical")), false),
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("wrong"))),
            ],
        );

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
}
