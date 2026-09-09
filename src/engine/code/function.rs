//! Runtime-independent compilation products.
#[cfg(test)]
mod fixtures;
pub mod metadata;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::debug::Pc2LineTable;

use crate::engine::value::{JsString, PrimitiveValue};
use crate::regexp::CompiledRegExp;

use metadata::*;
use std::{error::Error, fmt, rc::Rc};

/// Why a value cannot enter a runtime-independent constant pool draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnlinkedConstantError {
    /// Objects are runtime heap identities and must be linked through raw edges.
    RuntimeBoundObject,
    /// Symbols are runtime atom identities and need a dedicated linked form.
    RuntimeBoundSymbol,
    /// A template site must contain one or more aligned cooked/raw segments.
    InvalidTemplateObject,
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
            Self::InvalidTemplateObject => formatter.write_str(
                "a template object requires equal non-empty cooked and raw segment lists",
            ),
        }
    }
}

impl Error for UnlinkedConstantError {}

/// Private representation keeps runtime independence structural.
///
/// Making the enum itself crate-visible would let any module construct
/// `Primitive(PrimitiveValue::Object(_))` and bypass the checked constructor.
#[derive(Debug)]
enum UnlinkedConstantKind {
    Primitive(PrimitiveValue),
    /// Source String lowered through QuickJS `emit_push_const(..., as_atom=1)`.
    /// Publication canonicalizes non-immediate atoms in the runtime domain.
    AtomString(PrimitiveValue),
    /// Runtime-independent RegExp literal source and its compile-once matcher
    /// program. Both payloads are reference-counted leaves with no heap or
    /// atom identity, so the draft remains safe to publish into any runtime.
    RegExp {
        pattern: JsString,
        program: Rc<CompiledRegExp>,
    },
    /// Runtime-independent template-site data. Runtime publication replaces
    /// this pair with one realm-local frozen cooked Array whose `raw` property
    /// points at the corresponding frozen raw Array.
    TemplateObject {
        cooked: Box<[Option<JsString>]>,
        raw: Box<[JsString]>,
    },
    Child(Box<UnlinkedFunction>),
}

type TemplateObjectPayload = (Box<[Option<JsString>]>, Box<[JsString]>);

/// One constant in a compiler draft, before runtime publication.
///
/// Primitive constants may contain undefined, null, booleans, numbers,
/// BigInts, and strings. RegExp constants own only pure-Rust String/Rc leaves.
/// Symbols are ECMAScript primitives but remain
/// runtime-owned atom identities, so they intentionally use a future dedicated
/// linked representation instead of this draft variant.
#[derive(Debug)]
pub struct UnlinkedConstant(UnlinkedConstantKind);

impl UnlinkedConstant {
    /// Construct a runtime-independent primitive constant.
    ///
    /// # Errors
    ///
    /// Rejects object and symbol roots so an unlinked function cannot retain or
    /// combine runtime domains.
    pub fn primitive<T: TryInto<PrimitiveValue>>(value: T) -> Result<Self, UnlinkedConstantError>
    where
        UnlinkedConstantError: From<T::Error>,
    {
        let primitive = value.try_into().map_err(UnlinkedConstantError::from)?;
        Ok(Self(UnlinkedConstantKind::Primitive(primitive)))
    }

    /// Mark one compiler-produced String value for runtime atom
    /// canonicalization. Keeping this distinct from an ordinary primitive
    /// constant preserves QuickJS's non-atom constant-pool paths.
    #[must_use]
    pub fn atom_string(value: crate::engine::value::JsString) -> Self {
        Self(UnlinkedConstantKind::AtomString(PrimitiveValue::String(
            value,
        )))
    }

    /// Store one compile-once RegExp literal payload.
    #[must_use]
    pub fn regexp(pattern: JsString, program: Rc<CompiledRegExp>) -> Self {
        Self(UnlinkedConstantKind::RegExp { pattern, program })
    }

    /// Store one checked runtime-independent template-site payload.
    pub fn template_object(
        cooked: Vec<Option<JsString>>,
        raw: Vec<JsString>,
    ) -> Result<Self, UnlinkedConstantError> {
        if cooked.is_empty() || cooked.len() != raw.len() {
            return Err(UnlinkedConstantError::InvalidTemplateObject);
        }
        Ok(Self(UnlinkedConstantKind::TemplateObject {
            cooked: cooked.into_boxed_slice(),
            raw: raw.into_boxed_slice(),
        }))
    }

    /// Store one recursively compiled child-function draft.
    #[must_use]
    pub fn child(function: UnlinkedFunction) -> Self {
        Self(UnlinkedConstantKind::Child(Box::new(function)))
    }

    /// Borrow the primitive value, or return `None` for another constant kind.
    #[must_use]
    pub fn as_primitive(&self) -> Option<&PrimitiveValue> {
        match &self.0 {
            UnlinkedConstantKind::Primitive(value) | UnlinkedConstantKind::AtomString(value) => {
                Some(value)
            }
            UnlinkedConstantKind::RegExp { .. }
            | UnlinkedConstantKind::TemplateObject { .. }
            | UnlinkedConstantKind::Child(_) => None,
        }
    }

    /// Return whether this is an ordinary primitive constant rather than an
    /// atom-canonicalized String or another compiler-owned constant kind.
    #[must_use]
    pub const fn is_plain_primitive(&self) -> bool {
        matches!(self.0, UnlinkedConstantKind::Primitive(_))
    }

    /// Return whether this is exactly QuickJS's canonical empty atom String.
    /// The trusted ordinary-leaf boundary permits this one sealed
    /// representation without admitting arbitrary atom-backed constants.
    #[must_use]
    pub fn is_empty_atom_string(&self) -> bool {
        matches!(
            &self.0,
            UnlinkedConstantKind::AtomString(PrimitiveValue::String(value)) if value.is_empty()
        )
    }

    /// Borrow a RegExp literal payload, or return `None` for other constants.
    #[must_use]
    pub fn as_regexp(&self) -> Option<(&JsString, &Rc<CompiledRegExp>)> {
        match &self.0 {
            UnlinkedConstantKind::RegExp { pattern, program } => Some((pattern, program)),
            UnlinkedConstantKind::Primitive(_)
            | UnlinkedConstantKind::AtomString(_)
            | UnlinkedConstantKind::TemplateObject { .. }
            | UnlinkedConstantKind::Child(_) => None,
        }
    }

    /// Consume a RegExp literal payload without cloning its matcher program.
    /// Non-RegExp constants are returned unchanged to preserve their private
    /// representation invariant for the ordinary publication path.
    pub fn into_regexp(self) -> Result<(JsString, Rc<CompiledRegExp>), Self> {
        match self.0 {
            UnlinkedConstantKind::RegExp { pattern, program } => Ok((pattern, program)),
            other => Err(Self(other)),
        }
    }

    /// Consume a template-site payload without exposing the private constant
    /// representation to the publisher.
    pub fn into_template_object(self) -> Result<TemplateObjectPayload, Self> {
        match self.0 {
            UnlinkedConstantKind::TemplateObject { cooked, raw } => Ok((cooked, raw)),
            other => Err(Self(other)),
        }
    }

    /// Borrow a template-site payload for publication verification.
    #[must_use]
    pub fn as_template_object(&self) -> Option<(&[Option<JsString>], &[JsString])> {
        match &self.0 {
            UnlinkedConstantKind::TemplateObject { cooked, raw } => Some((cooked, raw)),
            UnlinkedConstantKind::Primitive(_)
            | UnlinkedConstantKind::AtomString(_)
            | UnlinkedConstantKind::RegExp { .. }
            | UnlinkedConstantKind::Child(_) => None,
        }
    }

    /// Borrow the child draft, or return `None` for another constant kind.
    #[must_use]
    pub fn as_child(&self) -> Option<&UnlinkedFunction> {
        match &self.0 {
            UnlinkedConstantKind::Primitive(_)
            | UnlinkedConstantKind::AtomString(_)
            | UnlinkedConstantKind::RegExp { .. }
            | UnlinkedConstantKind::TemplateObject { .. } => None,
            UnlinkedConstantKind::Child(function) => Some(function),
        }
    }

    /// Consume this constant for transactional runtime publication.
    ///
    /// RegExp payloads are first consumed by [`Self::into_regexp`]. For the
    /// remaining kinds, exactly one optional tuple field is `Some`; the middle
    /// flag marks the atom-string representation of a primitive payload.
    /// Returning output-only parts keeps the private invariant from being
    /// bypassed by callers.
    #[must_use]
    pub fn into_parts(self) -> (Option<PrimitiveValue>, bool, Option<UnlinkedFunction>) {
        match self.0 {
            UnlinkedConstantKind::Primitive(value) => (Some(value), false, None),
            UnlinkedConstantKind::AtomString(value) => (Some(value), true, None),
            UnlinkedConstantKind::RegExp { .. } => {
                unreachable!("RegExp constants must use into_regexp before ordinary publication")
            }
            UnlinkedConstantKind::TemplateObject { .. } => unreachable!(
                "template constants must use into_template_object before ordinary publication"
            ),
            UnlinkedConstantKind::Child(function) => (None, false, Some(*function)),
        }
    }
}

impl TryFrom<PrimitiveValue> for UnlinkedConstant {
    type Error = UnlinkedConstantError;

    fn try_from(value: PrimitiveValue) -> Result<Self, Self::Error> {
        Self::primitive(value)
    }
}

/// Mutable compiler output which has not entered a runtime domain yet.
///
/// The draft owns its vectors, but its fields stay private.  Publication
/// consumes the whole value through [`Self::into_parts`], verifies it, interns
/// runtime atoms, recursively publishes child functions, retains all outgoing
/// edges transactionally, and only then returns an engine `FunctionBytecodeRef`.
/// Published bytecode therefore has no mutation path back to this draft.
#[derive(Debug)]
pub struct UnlinkedFunction {
    code: Vec<Instruction>,
    constants: Vec<UnlinkedConstant>,
    metadata: FunctionMetadata,
    parameter_environment: Option<ParameterEnvironmentLayout>,
    func_name: Option<crate::engine::value::JsString>,
    argument_definitions: Vec<UnlinkedVariableDefinition>,
    local_definitions: Vec<UnlinkedVariableDefinition>,
    closure_variables: Vec<ClosureVariable>,
    eval_environments: Vec<EvalEnvironment<JsString>>,
    debug: Option<UnlinkedFunctionDebug>,
}

/// Runtime-independent form of QuickJS's `JSVarDef`/argument metadata.
///
/// Names remain optional because synthetic locals do not have an observable
/// identifier. Publication interns every present name and transfers the
/// resulting atom ownership to immutable function bytecode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnlinkedVariableDefinition {
    pub name: Option<crate::engine::value::JsString>,
    pub is_lexical: bool,
    pub is_const: bool,
    /// The lexical binding belongs to a nested scope evaluated wholly before
    /// the formal-parameter/body boundary.  Parameter-environment cells use
    /// their dedicated layout and leave this flag clear.
    pub is_parameter_initializer: bool,
    pub kind: ClosureVariableKind,
}

impl UnlinkedVariableDefinition {
    /// Construct an ordinary mutable argument/local definition.
    #[must_use]
    pub const fn ordinary(name: Option<crate::engine::value::JsString>) -> Self {
        Self {
            name,
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        }
    }

    /// Construct a block-scoped mutable or immutable definition.
    #[must_use]
    pub const fn lexical(name: Option<crate::engine::value::JsString>, is_const: bool) -> Self {
        Self {
            name,
            is_lexical: true,
            is_const,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        }
    }

    /// Retain compiler-authored parameter-initializer scope provenance across
    /// the publication boundary.
    #[must_use]
    pub const fn with_parameter_initializer(mut self, value: bool) -> Self {
        self.is_parameter_initializer = value;
        self
    }

    fn function_name(name: Option<crate::engine::value::JsString>, is_const: bool) -> Self {
        Self {
            name,
            is_lexical: false,
            is_const,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::FunctionName,
        }
    }

    #[cfg(test)]
    fn eval_variable_object() -> Self {
        Self {
            name: Some(crate::engine::value::JsString::from_static("<var>")),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::EvalVariableObject,
        }
    }

    fn arg_eval_variable_object() -> Self {
        Self {
            name: Some(crate::engine::value::JsString::from_static("<arg_var>")),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::ArgEvalVariableObject,
        }
    }

    pub fn with_object() -> Self {
        Self {
            name: Some(crate::engine::value::JsString::from_static("<with>")),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::WithObject,
        }
    }
}

/// Runtime-independent debug payload produced by the compiler.
///
/// Publication interns the filename separately for every function. Function
/// source is an independent byte copy so a nested closure does not retain its
/// complete enclosing script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnlinkedFunctionDebug {
    pub filename: crate::engine::value::JsString,
    pub pc2line: Option<Pc2LineTable>,
    pub source: Option<Box<[u8]>>,
}

/// Owned pieces crossing the one-way publication boundary.
pub struct UnlinkedFunctionParts {
    pub code: Vec<Instruction>,
    pub constants: Vec<UnlinkedConstant>,
    pub metadata: FunctionMetadata,
    pub parameter_environment: Option<ParameterEnvironmentLayout>,
    pub func_name: Option<crate::engine::value::JsString>,
    pub argument_definitions: Vec<UnlinkedVariableDefinition>,
    pub local_definitions: Vec<UnlinkedVariableDefinition>,
    pub closure_variables: Vec<ClosureVariable>,
    pub eval_environments: Vec<EvalEnvironment<JsString>>,
    pub debug: Option<UnlinkedFunctionDebug>,
}

impl UnlinkedFunction {
    /// Assemble a compiler draft with producer-authored binding definitions.
    /// Publication validates counts, flags and captured binding sources.
    #[must_use]
    pub fn new(
        code: Vec<Instruction>,
        constants: Vec<UnlinkedConstant>,
        metadata: FunctionMetadata,
        argument_definitions: Vec<UnlinkedVariableDefinition>,
        local_definitions: Vec<UnlinkedVariableDefinition>,
        closure_variables: Vec<ClosureVariable>,
    ) -> Self {
        Self {
            code,
            constants,
            metadata,
            parameter_environment: None,
            func_name: None,
            argument_definitions,
            local_definitions,
            closure_variables,
            eval_environments: Vec::new(),
            debug: None,
        }
    }

    /// Attach the compiler-produced lexical-environment descriptors used by
    /// syntactic direct-eval instructions in this function.
    #[must_use]
    pub fn with_eval_environments(
        mut self,
        eval_environments: Vec<EvalEnvironment<JsString>>,
    ) -> Self {
        self.eval_environments = eval_environments;
        self
    }

    /// Attach the exact parentless parameter-scope ABI. An empty layout is
    /// retained rather than folded into `None` because environment existence
    /// is observable independently from authored BoundName count.
    #[must_use]
    pub fn with_parameter_environment(
        mut self,
        layout: Option<ParameterEnvironmentLayout>,
    ) -> Self {
        self.parameter_environment = layout;
        if let Some(index) = self
            .parameter_environment
            .as_ref()
            .and_then(|layout| layout.arg_eval_variable_object_local)
            && let Some(definition) = self.local_definitions.get_mut(usize::from(index))
        {
            *definition = UnlinkedVariableDefinition::arg_eval_variable_object();
        }
        self
    }

    /// Attach the source-level intrinsic name of a function expression.
    #[must_use]
    pub fn with_name(mut self, name: Option<crate::engine::value::JsString>) -> Self {
        self.func_name = name.clone();
        if let Some(index) = self.metadata.function_name_local {
            if let Some(definition) = self.local_definitions.get_mut(usize::from(index)) {
                *definition = UnlinkedVariableDefinition::function_name(name, self.metadata.strict);
            }
        }
        self
    }

    /// Attach compiler-produced source metadata before publication.
    #[must_use]
    pub fn with_debug(mut self, debug: UnlinkedFunctionDebug) -> Self {
        self.debug = Some(debug);
        self
    }

    #[must_use]
    pub fn code(&self) -> &[Instruction] {
        &self.code
    }

    #[must_use]
    pub fn constants(&self) -> &[UnlinkedConstant] {
        &self.constants
    }

    #[must_use]
    pub const fn metadata(&self) -> &FunctionMetadata {
        &self.metadata
    }

    #[must_use]
    pub const fn parameter_environment(&self) -> Option<&ParameterEnvironmentLayout> {
        self.parameter_environment.as_ref()
    }

    #[must_use]
    pub fn func_name(&self) -> Option<&crate::engine::value::JsString> {
        self.func_name.as_ref()
    }

    #[must_use]
    pub fn closure_variables(&self) -> &[ClosureVariable] {
        &self.closure_variables
    }

    #[must_use]
    pub fn argument_definitions(&self) -> &[UnlinkedVariableDefinition] {
        &self.argument_definitions
    }

    #[must_use]
    pub fn local_definitions(&self) -> &[UnlinkedVariableDefinition] {
        &self.local_definitions
    }

    #[must_use]
    pub fn eval_environments(&self) -> &[EvalEnvironment<JsString>] {
        &self.eval_environments
    }

    #[must_use]
    pub const fn debug(&self) -> Option<&UnlinkedFunctionDebug> {
        self.debug.as_ref()
    }

    /// Consume the draft at the immutable publication boundary.
    #[must_use]
    pub fn into_parts(self) -> UnlinkedFunctionParts {
        UnlinkedFunctionParts {
            code: self.code,
            constants: self.constants,
            metadata: self.metadata,
            parameter_environment: self.parameter_environment,
            func_name: self.func_name,
            argument_definitions: self.argument_definitions,
            local_definitions: self.local_definitions,
            closure_variables: self.closure_variables,
            eval_environments: self.eval_environments,
            debug: self.debug,
        }
    }
}

impl From<std::convert::Infallible> for UnlinkedConstantError {
    fn from(value: std::convert::Infallible) -> Self {
        match value {}
    }
}
