//! Source-to-bytecode compilation with late lexical-name resolution.
//!
//! QuickJS first emits scope-variable operations, then `resolve_scope_var`
//! rewrites them after every nested function and lexical scope is known.  Its
//! `get_closure_var` helper also installs relay closure slots on intervening
//! functions.  This module keeps the same boundary in typed form: parsing emits
//! `IrOp`s into a recursive `FunctionIr` arena, identifier resolution runs
//! child-first, and only then are VM instructions and recursive unlinked
//! function constants produced. The parser owns its Lexer and requests tokens
//! through fallible advances, so an error on later source cannot preempt a
//! diagnostic on the current token. Directive-prologue probes clone and seek
//! the lexer, then the committed stream is rescanned under its strict context.

pub mod lexer;
mod scope_validation;
use scope_validation::validate_scope_graph;
mod resolution;
#[cfg(test)]
use resolution::ensure_closure_variable;
use resolution::{
    ResolvedBinding, apply_quickjs_late_throw_sites, capture_binding_path, ensure_string_constant,
    find_or_create_own_binding, insert_hoist_fragment, ordered_hoisted_functions,
    prepend_hoist_prefix, push_closure_variable, resolve_identifiers,
};
mod lowering;
use crate::engine::api::error::{Error, ErrorKind, NativeErrorMessage};
#[cfg(test)]
use crate::engine::atom::AtomTable;
#[cfg(test)]
use crate::engine::code::bytecode::DetachedBytecode;
use crate::source::{SourceLocation, SourceSpan};

use crate::engine::code::bytecode::{
    ApplyKind, ArgumentsKind, DynamicEnvironmentSource, EvalVariableSource, Instruction,
    MAX_LOCAL_SLOTS, PrivateNameSource, WithObjectSource, verify_parts,
};
use crate::engine::code::bytecode_validation::quickjs_copies_defined_argument_count;
use crate::engine::code::debug::{DebugInfoMode, Pc2LineEntry, Pc2LineTable};
use crate::source::{QuickJsSourceLocator, SourceOffset};

use crate::engine::code::function::metadata::{
    ClassInitializerKind, ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName,
    ConstructorKind, EvalBinding, EvalBindingSource, EvalCallerProfile, EvalCallerVariableTarget,
    EvalEnvironment, EvalKind, EvalRootBinding, EvalScope, EvalScopeKind, EvalVariableEnvironment,
    FunctionKind as BytecodeFunctionKind, FunctionMetadata, ParameterArgumentCell,
    ParameterBodyStorage, ParameterDefaultSource, ParameterEnvironmentLayout, ParameterPatternCopy,
};
use crate::engine::code::function::{
    UnlinkedConstant, UnlinkedFunction, UnlinkedFunctionDebug, UnlinkedVariableDefinition,
};
use crate::engine::code::module::{ModuleImportAttribute, ModuleRequest, UnlinkedModule};
use crate::engine::compiler::lexer::{
    Identifier, Keyword, LexContext, LexError, LexErrorKind, Lexer, LexerOptions, LexicalGoal,
    NumberKind, NumericRadix, Punctuator, Span, TemplatePartKind, Token, TokenKind,
    quickjs_simple_lookahead_is_of,
};
use crate::engine::value::bigint::JsBigInt;
use crate::engine::value::{JsString, JsStringError, PrimitiveValue as Value};
use crate::source::text::SourceText;
#[cfg(test)]
use lowering::lower_detached_script;
use lowering::lower_unlinked_tree;
use num_bigint::BigUint;
use num_traits::ToPrimitive;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

mod arrow;
mod class;
mod destructuring;
mod function;
mod generator;
mod module;
mod object_literal;
mod optional_chain;
mod private_reference;
mod pseudo_binding;
mod template;

use optional_chain::{FinalizedOptionalChain, PendingOptionalChain};
use pseudo_binding::{
    ACTIVE_FUNCTION_LOCAL_NAME, HOME_OBJECT_LOCAL_NAME, NEW_TARGET_LOCAL_NAME, PseudoBinding,
    THIS_LOCAL_NAME, ensure_eval_visible_pseudo_bindings, find_or_create_own_pseudo_binding,
    function_owns_pseudo_binding, install_pseudo_binding_prologues,
};

/// Default filename used by the Rust convenience compile/eval APIs.
pub const DEFAULT_EVAL_FILENAME: &str = "<input>";

/// Named source compilation options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileOptions {
    pub filename: String,
}

impl CompileOptions {
    #[must_use]
    pub fn new(filename: impl Into<String>) -> Self {
        Self {
            filename: filename.into(),
        }
    }
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self::new(DEFAULT_EVAL_FILENAME)
    }
}

/// Internal compilation context for one synthetic eval root.
///
/// Direct eval imports the exact live caller bindings described by R1w.
/// Indirect eval has no external bindings and resolves against the defining
/// realm's global environment. `caller_strict` is ignored for indirect eval,
/// matching QuickJS's `JS_EVAL_TYPE_INDIRECT` path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalCompileContext {
    pub kind: EvalKind,
    pub caller_strict: bool,
    pub bindings: Box<[EvalRootBinding<JsString>]>,
    pub caller_profile: EvalCallerProfile,
    pub super_call_allowed: bool,
    pub super_allowed: bool,
    pub arguments_forbidden: bool,
}

impl EvalCompileContext {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn direct(caller_strict: bool, bindings: Vec<EvalRootBinding<JsString>>) -> Self {
        let scope_count = bindings
            .iter()
            .map(|binding| usize::from(binding.scope) + 1)
            .max()
            .unwrap_or(0);
        let mut scope_kinds = vec![EvalScopeKind::FunctionRoot; scope_count];
        for binding in &bindings {
            if binding.kind == ClosureVariableKind::WithObject {
                scope_kinds[usize::from(binding.scope)] = EvalScopeKind::With;
            } else if binding.is_catch_parameter {
                scope_kinds[usize::from(binding.scope)] = EvalScopeKind::Catch;
            }
        }
        for (scope, scope_kind) in scope_kinds.iter_mut().enumerate() {
            let has_parameter_object = bindings.iter().any(|binding| {
                usize::from(binding.scope) == scope
                    && binding.kind == ClosureVariableKind::ArgEvalVariableObject
            });
            let has_body_object = bindings.iter().any(|binding| {
                usize::from(binding.scope) == scope
                    && binding.kind == ClosureVariableKind::EvalVariableObject
            });
            if has_parameter_object && !has_body_object {
                *scope_kind = EvalScopeKind::Parameter;
            }
        }
        let variable_target = if caller_strict {
            EvalCallerVariableTarget::StrictLocal
        } else {
            bindings
                .iter()
                .position(|binding| {
                    matches!(
                        binding.kind,
                        ClosureVariableKind::EvalVariableObject
                            | ClosureVariableKind::ArgEvalVariableObject
                    )
                })
                .and_then(|index| u16::try_from(index).ok())
                .map(EvalCallerVariableTarget::ExternalBinding)
                .unwrap_or(EvalCallerVariableTarget::Global)
        };
        Self::direct_with_profile(
            caller_strict,
            bindings,
            EvalCallerProfile {
                scope_kinds: scope_kinds.into_boxed_slice(),
                variable_target,
            },
            false,
            false,
        )
    }

    pub fn direct_with_profile(
        caller_strict: bool,
        bindings: Vec<EvalRootBinding<JsString>>,
        caller_profile: EvalCallerProfile,
        super_call_allowed: bool,
        super_allowed: bool,
    ) -> Self {
        Self {
            kind: EvalKind::Direct,
            caller_strict,
            bindings: bindings.into_boxed_slice(),
            caller_profile,
            super_call_allowed,
            super_allowed,
            arguments_forbidden: false,
        }
    }

    pub fn direct_with_profile_and_arguments(
        caller_strict: bool,
        bindings: Vec<EvalRootBinding<JsString>>,
        caller_profile: EvalCallerProfile,
        super_call_allowed: bool,
        super_allowed: bool,
        arguments_forbidden: bool,
    ) -> Self {
        let mut context = Self::direct_with_profile(
            caller_strict,
            bindings,
            caller_profile,
            super_call_allowed,
            super_allowed,
        );
        context.arguments_forbidden = arguments_forbidden;
        context
    }

    pub fn indirect() -> Self {
        Self {
            kind: EvalKind::Indirect,
            caller_strict: false,
            bindings: Box::new([]),
            caller_profile: EvalCallerProfile {
                scope_kinds: Box::new([]),
                variable_target: EvalCallerVariableTarget::Global,
            },
            super_call_allowed: false,
            super_allowed: false,
            arguments_forbidden: false,
        }
    }
}

/// Compile one ECMAScript script directly to stack bytecode.
///
/// # Errors
/// Returns a syntax error for invalid source and an unsupported diagnostic for
/// grammar which has not yet reached the feature-parity implementation path.
#[cfg(test)]
pub fn compile_script(source: &str) -> Result<DetachedBytecode<Value>, Error> {
    let mut tree = Parser::parse(source, JsString::from_static(DEFAULT_EVAL_FILENAME))?;
    resolve_identifiers(&mut tree)?;
    if let Some(error) = tree.pending_unsupported.take() {
        return Err(error);
    }
    if tree.functions.len() != 1 {
        return Err(Error::unsupported(
            "nested function bytecode requires runtime publication; use Context::compile or Context::eval",
            source_span(tree.functions[1].source.span),
        ));
    }
    lower_detached_script(tree)
}

/// Compile a script into a runtime-independent draft ready for publication.
///
/// Runtime publication uses this production boundary to keep primitive
/// constants structural and to carry execution metadata into the heap node.
/// The older detached compiler result is test-only.
#[cfg_attr(not(test), allow(dead_code))]
pub fn compile_unlinked_script(source: &str) -> Result<UnlinkedFunction, Error> {
    compile_unlinked_script_with_filename(source, DEFAULT_EVAL_FILENAME, DebugInfoMode::Full)
}

pub fn compile_unlinked_script_with_filename(
    source: &str,
    filename: &str,
    debug_info: DebugInfoMode,
) -> Result<UnlinkedFunction, Error> {
    let mut tree = Parser::parse(source, JsString::try_from_utf8(filename)?)?;
    resolve_identifiers(&mut tree)?;
    if let Some(error) = tree.pending_unsupported.take() {
        return Err(error);
    }
    lower_unlinked_tree(tree, debug_info)
}

/// Reject source buffers which cannot fit QuickJS's signed debug-source
/// length before any byte-oriented carrier allocation is attempted.
pub fn validate_source_length(source_len: usize) -> Result<(), Error> {
    i32::try_from(source_len).map(|_| ()).map_err(|_| {
        Error::new(
            ErrorKind::JsInternal,
            "source is too large for QuickJS debug metadata",
        )
    })
}

/// Compile one explicitly sized Script buffer after applying QuickJS's signed
/// source-length guard, before allocating its byte-exact carrier.
pub fn compile_unlinked_script_bytes_with_filename(
    source: &[u8],
    filename: &str,
    debug_info: DebugInfoMode,
) -> Result<UnlinkedFunction, Error> {
    validate_source_length(source.len())?;
    let source = SourceText::try_from_raw_bytes(source)?;
    compile_unlinked_script_source_with_filename(&source, filename, debug_info)
}

/// Compile one byte-exact Script carrier for the public byte-oriented
/// embedding boundary.
pub fn compile_unlinked_script_source_with_filename(
    source: &SourceText,
    filename: &str,
    debug_info: DebugInfoMode,
) -> Result<UnlinkedFunction, Error> {
    let mut tree = Parser::parse_script_source(source, JsString::try_from_utf8(filename)?)?;
    resolve_identifiers(&mut tree)?;
    if let Some(error) = tree.pending_unsupported.take() {
        return Err(error);
    }
    lower_unlinked_tree(tree, debug_info)
}

/// Compile one ECMAScript module into its runtime-independent module record.
#[cfg_attr(not(test), allow(dead_code))]
pub fn compile_unlinked_module_with_filename(
    source: &str,
    filename: &str,
    debug_info: DebugInfoMode,
) -> Result<UnlinkedModule, Error> {
    compile_unlinked_module_with_name(source, JsString::try_from_utf8(filename)?, debug_info)
}

/// Compile a module whose host-resolved identity may contain any ECMAScript
/// String code unit, including lone UTF-16 surrogates.
pub fn compile_unlinked_module_with_name(
    source: &str,
    name: JsString,
    debug_info: DebugInfoMode,
) -> Result<UnlinkedModule, Error> {
    compile_unlinked_module_with_name_and_attribute_checker(source, name, debug_info, None)
        .map_err(ModuleCompileFailure::into_engine_without_checker)
}

/// Synchronous host validation performed as soon as one non-empty static
/// import-attribute clause has been parsed.
///
/// Pinned QuickJS invokes `JSModuleCheckAttributes` before consuming the
/// clause's closing brace and before parsing any following source. Returning
/// an error here therefore must stop parsing immediately.
pub trait ModuleImportAttributeChecker {
    /// Observe one source-order request after it has entered the parser-owned
    /// requested-module table.
    ///
    /// Called after decoding attributes and before checking them, while the
    /// closing brace is still current.
    fn publish_request(&mut self, request: &ModuleRequest) -> Result<(), ModuleCompileFailure>;

    fn check(&mut self, attributes: &[ModuleImportAttribute]) -> Result<(), ModuleCompileFailure>;
}

/// Abrupt completion of the checked module-compilation path.
///
/// Ordinary compiler diagnostics remain [`Error`] values. The distinct
/// host-abort arm stops parsing after a module attribute callback throws.
/// The caller retains the exact JavaScript value; the compiler never owns
/// an engine handle or converts that value into a diagnostic.
#[derive(Clone, Debug, PartialEq)]
pub enum ModuleCompileFailure {
    Engine(Error),
    Host,
}

impl From<Error> for ModuleCompileFailure {
    fn from(error: Error) -> Self {
        Self::Engine(error)
    }
}

impl From<JsStringError> for ModuleCompileFailure {
    fn from(error: JsStringError) -> Self {
        Self::Engine(error.into())
    }
}

impl ModuleCompileFailure {
    fn into_engine_without_checker(self) -> Error {
        match self {
            Self::Engine(error) => error,
            Self::Host => Error::internal(
                "module compiler produced a host throw without an attribute checker",
            ),
        }
    }
}

/// Compile a module while exposing QuickJS's parse-time attribute-check hook.
pub fn compile_unlinked_module_with_name_and_attribute_checker(
    source: &str,
    name: JsString,
    debug_info: DebugInfoMode,
    checker: Option<&mut dyn ModuleImportAttributeChecker>,
) -> Result<UnlinkedModule, ModuleCompileFailure> {
    let tree = Parser::parse_module(source, name, checker)?;
    finish_unlinked_module_tree(tree, debug_info)
}

/// Compile one explicitly sized Module buffer after applying QuickJS's signed
/// source-length guard, before allocating its byte-exact carrier.
pub fn compile_unlinked_module_bytes_with_name_and_attribute_checker(
    source: &[u8],
    name: JsString,
    debug_info: DebugInfoMode,
    checker: Option<&mut dyn ModuleImportAttributeChecker>,
) -> Result<UnlinkedModule, ModuleCompileFailure> {
    validate_source_length(source.len())?;
    let source = SourceText::try_from_raw_bytes(source)?;
    let tree = Parser::parse_module_source(&source, name, checker)?;
    finish_unlinked_module_tree(tree, debug_info)
}

fn finish_unlinked_module_tree(
    mut tree: FunctionTree,
    debug_info: DebugInfoMode,
) -> Result<UnlinkedModule, ModuleCompileFailure> {
    resolve_identifiers(&mut tree)?;
    if let Some(error) = tree.pending_unsupported.take() {
        return Err(error.into());
    }
    let name = tree.filename.clone();
    let module = tree
        .module
        .take()
        .ok_or_else(|| Error::internal("module compiler produced no module record"))?;
    let has_top_level_await = tree.functions.first().is_some_and(|function| {
        function
            .ops
            .iter()
            .any(|operation| matches!(operation.op, IrOp::Bytecode(Instruction::Await)))
    });
    let function = lower_unlinked_tree(tree, debug_info)?;
    module::finish_module(name, function, has_top_level_await, module).map_err(Into::into)
}

/// Compile one primitive-String eval as an independent synthetic root.
///
/// This deliberately does not reuse the ordinary Script root. QuickJS gives
/// eval its own body environment and, for direct eval, attaches that root to
/// the active caller's VarRefs only while publishing and executing it.
#[cfg_attr(not(test), allow(dead_code))]
pub fn compile_unlinked_eval_with_filename(
    source: &str,
    filename: &str,
    debug_info: DebugInfoMode,
    context: EvalCompileContext,
) -> Result<UnlinkedFunction, Error> {
    let mut tree = Parser::parse_eval(source, JsString::try_from_utf8(filename)?, context)?;
    resolve_identifiers(&mut tree)?;
    if let Some(error) = tree.pending_unsupported.take() {
        return Err(error);
    }
    lower_unlinked_tree(tree, debug_info)
}

/// Compile dynamic eval source whose carrier may represent lone UTF-16
/// surrogates. Public Rust `&str` compilation remains on the ordinary wrapper
/// above; only JavaScript String eval uses this reversible internal boundary.
pub fn compile_unlinked_eval_source_with_filename(
    source: &SourceText,
    filename: &str,
    debug_info: DebugInfoMode,
    context: EvalCompileContext,
) -> Result<UnlinkedFunction, Error> {
    let mut tree = Parser::parse_eval_source(source, JsString::try_from_utf8(filename)?, context)?;
    resolve_identifiers(&mut tree)?;
    if let Some(error) = tree.pending_unsupported.take() {
        return Err(error);
    }
    lower_unlinked_tree(tree, debug_info)
}

type FunctionId = usize;
/// Function-local lexical scope identity. QuickJS carries the corresponding
/// `scope_level` beside every unresolved scope opcode; keeping it typed avoids
/// accidentally resolving a child use from the parent's final parse scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ScopeId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct BindingId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ParentLink {
    function: FunctionId,
    definition_scope: ScopeId,
}

// QuickJS 2026-06-04 `JS_MAX_LOCAL_VARS` and `JS_STACK_SIZE_MAX` are both
// 65,534. Call opcodes encode one more argument count value; the resulting
// operand stack is checked against the smaller stack limit during lowering.
const MAX_LOCAL_VARIABLES: usize = MAX_LOCAL_SLOTS as usize;
const MAX_BYTECODE_STACK: usize = 65_534;
const MAX_CALL_ARGUMENTS: usize = 65_535;
// QuickJS `js_parse_program` allocates `JS_ATOM__ret_` as the first local of
// every script. Source text cannot spell this sentinel as an IdentifierName.
const EVAL_RET_LOCAL_NAME: &str = "<ret>";
// QuickJS `JS_ATOM__var_`: the null-prototype variable object used by sloppy
// direct eval. Source text cannot spell this binding identity.
const EVAL_VARIABLE_OBJECT_LOCAL_NAME: &str = "<var>";
// QuickJS `JS_ATOM__arg_var_`: the independent null-prototype variable
// object selected by sloppy direct eval while a non-simple parameter list is
// being evaluated. It remains live for the whole activation so body eval can
// consult it after the ordinary `<var>` object.
const ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME: &str = "<arg_var>";
// QuickJS `JS_ATOM__with_`: the object-environment binding owned by one
// sloppy `with` scope. Source text cannot spell this binding identity.
const WITH_OBJECT_LOCAL_NAME: &str = "<with>";
// A finally clause in script code must preserve the incoming completion value
// when it terminates normally. Keep those implementation-only save slots in
// the same explicit metadata domain as `<ret>` rather than letting an unbound
// ordinary local silently escape the scope-graph trust boundary.
const FINALLY_EVAL_RET_LOCAL_NAME: &str = "<finally-ret>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FunctionKind {
    Script,
    Module,
    Eval(EvalKind),
    Ordinary,
    /// Compiler-only object-literal concise method. Like an ordinary function
    /// it owns `this`, `arguments`, and `new.target`, but publication lowers it
    /// as a non-constructor with no `prototype` property.
    Method,
    /// Compiler-only parse/binding kind. QuickJS publishes synchronous arrow
    /// bytecode as a normal function with no prototype or constructor bit.
    Arrow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatementCompletion {
    Eval,
    Discard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatementPosition {
    ProgramBody,
    FunctionBody,
    NestedList,
    /// Sloppy `if` consequent/alternate: ordinary functions are permitted,
    /// but a label may not forward that permission to its body.
    AnnexBIfArm,
    /// Sloppy labelled statement reached from a declaration list. Ordinary
    /// functions and further labels are permitted, but other declarations are
    /// still single-statement syntax errors.
    AnnexBLabelBody,
    Single,
}

impl StatementPosition {
    const fn allows_other_declaration(self) -> bool {
        matches!(
            self,
            Self::ProgramBody | Self::FunctionBody | Self::NestedList
        )
    }

    const fn allows_labelled_annex_b(self) -> bool {
        matches!(
            self,
            Self::ProgramBody | Self::FunctionBody | Self::NestedList | Self::AnnexBLabelBody
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeKind {
    FunctionRoot,
    /// Declarative environment used while evaluating non-simple formal
    /// parameters. It is deliberately parentless: parameter initializers may
    /// see parameter cells and selected function pseudo-bindings, but never
    /// declarations from the authored body variable environment.
    Parameter,
    FunctionBody,
    ProgramBody,
    Block,
    /// QuickJS's class-body scope. It owns private-name declarations plus the
    /// compiler-only lexical cells used to retain computed field keys. It is
    /// separate from the class-name scope so heritage evaluation cannot observe
    /// names declared later in the body.
    ClassPrivate,
    If,
    For,
    Switch,
    Catch,
    With,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyntheticLocalKind {
    EvalCompletion,
    FinallySavedEvalCompletion,
}

impl SyntheticLocalKind {
    const fn name(self) -> &'static str {
        match self {
            Self::EvalCompletion => EVAL_RET_LOCAL_NAME,
            Self::FinallySavedEvalCompletion => FINALLY_EVAL_RET_LOCAL_NAME,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SyntheticLocal {
    index: u16,
    kind: SyntheticLocalKind,
}

#[derive(Debug)]
struct IrScope {
    parent: Option<ScopeId>,
    kind: ScopeKind,
    /// This nested lexical scope is entered and left while formal-parameter
    /// initialization is still running.  It is distinct from the Parameter
    /// scope itself: its bindings may be captured by closures created in an
    /// initializer, but must never be mistaken for authored body lexicals.
    is_parameter_initializer: bool,
    bindings: Vec<BindingId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingKind {
    Normal,
    Lexical {
        is_const: bool,
    },
    FunctionName {
        is_const: bool,
    },
    EvalVariableObject,
    ArgEvalVariableObject,
    WithObject,
    /// One class-private data-field identity. Staticness is declaration-only
    /// metadata used to diagnose conflicts; closure/runtime metadata needs only
    /// the authenticated `PrivateField` kind.
    PrivateField {
        is_static: bool,
    },
    /// One class-private synchronous method. Like private fields, staticness
    /// exists only while parsing the declaring class so same-spelling
    /// instance/static elements conflict before the typed closure metadata is
    /// published.
    PrivateMethod {
        is_static: bool,
    },
    /// Primary binding for a private getter. The cell contains the getter
    /// callable; a matching setter upgrades this binding to
    /// `PrivateGetterSetter` while retaining the same local identity.
    PrivateGetter {
        is_static: bool,
    },
    /// Private setter capability. The source-visible primary `#name` binding
    /// deliberately remains uninitialized for a setter-only declaration;
    /// the callable itself lives in the synthetic `#name<set>` binding with
    /// this same kind, matching pinned QuickJS.
    PrivateSetter {
        is_static: bool,
    },
    /// Primary binding for a paired private getter/setter. The getter lives in
    /// this cell and the setter lives in the sibling synthetic `<set>` cell.
    PrivateGetterSetter {
        is_static: bool,
    },
}

fn binding_kinds_compatible(left: BindingKind, right: BindingKind) -> bool {
    left == right
        || matches!(
            (left, right),
            (
                BindingKind::PrivateField { .. },
                BindingKind::PrivateField { .. }
            ) | (
                BindingKind::PrivateMethod { .. },
                BindingKind::PrivateMethod { .. }
            ) | (
                BindingKind::PrivateGetter { .. },
                BindingKind::PrivateGetter { .. }
            ) | (
                BindingKind::PrivateSetter { .. },
                BindingKind::PrivateSetter { .. }
            ) | (
                BindingKind::PrivateGetterSetter { .. },
                BindingKind::PrivateGetterSetter { .. }
            )
        )
}

const fn binding_kind_from_closure_flags(
    kind: ClosureVariableKind,
    is_lexical: bool,
    is_const: bool,
) -> Option<BindingKind> {
    match kind {
        ClosureVariableKind::Normal if is_lexical => Some(BindingKind::Lexical { is_const }),
        ClosureVariableKind::Normal if !is_const => Some(BindingKind::Normal),
        ClosureVariableKind::ModuleImportView if is_lexical && is_const => {
            Some(BindingKind::Lexical { is_const: true })
        }
        ClosureVariableKind::FunctionName if !is_lexical => {
            Some(BindingKind::FunctionName { is_const })
        }
        ClosureVariableKind::EvalVariableObject if !is_lexical && !is_const => {
            Some(BindingKind::EvalVariableObject)
        }
        ClosureVariableKind::ArgEvalVariableObject if !is_lexical && !is_const => {
            Some(BindingKind::ArgEvalVariableObject)
        }
        ClosureVariableKind::WithObject if !is_lexical && !is_const => {
            Some(BindingKind::WithObject)
        }
        ClosureVariableKind::PrivateField if is_lexical && is_const => {
            // Staticness is declaration-only conflict metadata. A closure or
            // direct-eval descriptor needs only the authenticated identity kind.
            Some(BindingKind::PrivateField { is_static: false })
        }
        ClosureVariableKind::PrivateMethod if is_lexical && is_const => {
            // Staticness is likewise declaration-only for method identities.
            Some(BindingKind::PrivateMethod { is_static: false })
        }
        ClosureVariableKind::PrivateGetter if is_lexical && is_const => {
            Some(BindingKind::PrivateGetter { is_static: false })
        }
        ClosureVariableKind::PrivateSetter if is_lexical && is_const => {
            Some(BindingKind::PrivateSetter { is_static: false })
        }
        ClosureVariableKind::PrivateGetterSetter if is_lexical && is_const => {
            Some(BindingKind::PrivateGetterSetter { is_static: false })
        }
        ClosureVariableKind::Normal
        | ClosureVariableKind::ModuleImportView
        | ClosureVariableKind::FunctionName
        | ClosureVariableKind::GlobalFunction
        | ClosureVariableKind::EvalVariableObject
        | ClosureVariableKind::ArgEvalVariableObject
        | ClosureVariableKind::WithObject
        | ClosureVariableKind::PrivateField
        | ClosureVariableKind::PrivateMethod
        | ClosureVariableKind::PrivateGetter
        | ClosureVariableKind::PrivateSetter
        | ClosureVariableKind::PrivateGetterSetter => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingStorage {
    Argument(u16),
    Local(u16),
    /// Exact slot on a synthetic direct-eval root, supplied by the active
    /// caller environment rather than by a compiler-tree parent.
    External(u16),
    /// Exact binding in the synthetic module root. Its physical VarRef slot is
    /// assigned after the complete module declaration table is known.
    Module(module::ModuleBindingId),
    Global,
}

#[derive(Debug)]
struct IrBinding {
    name: String,
    storage_scope: ScopeId,
    /// Parse scope of the first declaration. QuickJS keeps this separately as
    /// the `scope_next` origin even for function-scoped `var` storage.
    declaration_scope: ScopeId,
    storage: BindingStorage,
    kind: BindingKind,
    /// Header-time marker for a block/switch FunctionDeclaration. The child
    /// constant is attached separately after its body parses successfully.
    is_scoped_function: bool,
    /// Generator declarations are lexical declarations and never receive the
    /// sloppy Annex B duplicate-function exception. Retain the grammar kind
    /// on the binding until the child constant is attached so a later
    /// declaration in the same block can apply QuickJS's early-error rule.
    is_scoped_generator: bool,
    /// Catch parameters behave as mutable lexicals for resolution/lifetime,
    /// but `var` of the same name is explicitly permitted and resolves its
    /// initializer through this nearer cell.
    is_catch_parameter: bool,
    declaration_span: Option<Span>,
}

#[derive(Debug)]
struct IrGlobalDeclaration {
    name: String,
    is_lexical: bool,
    is_const: bool,
    /// Child-function constant for a QuickJS
    /// `JS_VAR_GLOBAL_FUNCTION_DECL`. Ordinary `var` and lexical
    /// declarations have no declaration-time value.
    function_constant: Option<u32>,
    /// Exact root `GLOBAL_DECL` slot allocated during declaration seeding.
    closure_index: Option<u16>,
}

#[derive(Clone, Copy, Debug)]
struct IrHoistedFunction {
    binding: BindingId,
    constant: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvalDeclarationTarget {
    /// A pre-existing caller cell which precedes the caller's `<var>` object.
    External { index: u16, kind: BindingKind },
    /// A novel name stored as a configurable property on the caller's `<var>`
    /// object. Repeated records deliberately redefine the property.
    Dynamic(EvalVariableSource),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvalDeclarationValue {
    Undefined,
    Function(u32),
}

#[derive(Clone, Debug)]
struct IrEvalDeclaration {
    name: String,
    target: EvalDeclarationTarget,
    value: EvalDeclarationValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvalDeclarationMode {
    Local,
    Global,
    Dynamic(EvalVariableSource),
}

#[derive(Clone, Copy, Debug)]
struct IrScopedFunction {
    binding: BindingId,
    constant: u32,
    annex_binding: Option<IrAnnexBinding>,
    authored_closure: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IrAnnexBinding {
    Static(BindingId),
    Dynamic,
}

/// A sloppy labelled FunctionDeclaration in ProgramBody. QuickJS deliberately
/// skips the lexical declaration used by block/body Annex B forms, then writes
/// the authored closure through both the synthetic root var and the current
/// global environment at the declaration's source position.
#[derive(Clone, Copy, Debug)]
struct IrProgramAnnexFunction {
    binding: BindingId,
    constant: u32,
    authored_closure: usize,
}

#[derive(Debug)]
enum IrConstant {
    Primitive(Value),
    /// Source String emitted through QuickJS's atom-value constant path.
    AtomString(JsString),
    /// QuickJS stores the literal pattern and `lre_compile` bytecode as two
    /// constants consumed by `OP_regexp`. The typed form keeps the same
    /// compile-once payload behind one verified constant index.
    RegExp {
        pattern: JsString,
        program: Rc<crate::regexp::CompiledRegExp>,
    },
    /// Runtime-independent template-site payload. Publication materializes
    /// the two frozen realm-local Arrays once and replaces this structural
    /// constant with the cooked template object identity retained by bytecode.
    TemplateObject {
        cooked: Vec<Option<JsString>>,
        raw: Vec<JsString>,
    },
    Child(FunctionId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdentifierAccess {
    Get,
    GetOrUndefined,
    Delete,
    Initialize,
    /// QuickJS `put_loc_check_init` / `put_var_ref_check_init` for the
    /// derived-constructor `this` binding. Unlike an ordinary lexical
    /// declaration initializer this access may cross an arrow/direct-eval
    /// closure boundary, but it still succeeds exactly once.
    InitializeDerivedThis,
    Put,
    /// Sloppy Annex B writes past the block lexical binding to the function
    /// root. Global code must still resolve the name dynamically so a later
    /// Program lexical can retain QuickJS's source-ordered TDZ behavior.
    AnnexBPut,
    Set,
}

/// Parser/linker form of the object-environment Reference which QuickJS keeps
/// only when an authored `with` scope can be visible from the use site.  The
/// parser uses the same stack shape for every identifier lvalue; resolution
/// later decides whether the base is a selected object or the `undefined`
/// sentinel used by the statically resolved fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdentifierReferenceAccess {
    /// Select and retain a base before evaluating a simple assignment RHS.
    Prepare,
    /// Select a base and append its current value for compound/update/call.
    Get,
    /// Select a method-call receiver and append the callee. If the selected
    /// property disappears during the repeated HasProperty action, QuickJS's
    /// `with_get_ref` supplies `undefined` even in strict code.
    Call,
    /// Consume `base, value`, perform the write, and preserve `value`.
    Set,
    /// Consume `base, old, value`, perform the write, and preserve `old`.
    PostPut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MemberReference {
    Field {
        key: u32,
        site: SourceOffset,
    },
    Computed {
        site: SourceOffset,
    },
    /// `this, frozen HomeObject prototype, raw/canonical key` reference used
    /// by QuickJS's get/put-super-value lowering.
    Super {
        site: SourceOffset,
    },
    Private {
        name: String,
        span: Span,
        scope: ScopeId,
        site: SourceOffset,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrivateFieldAccess {
    Get,
    GetKeepReceiver,
    Put,
    Define,
    In,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IdentifierReference {
    name: String,
    span: Span,
    scope: ScopeId,
    object_environment: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogicalAssignment {
    And,
    Or,
    Nullish,
}

/// Mirrors QuickJS's `PF_POW_ALLOWED`, `PF_POW_FORBIDDEN`, and zero flag.
/// The zero mode is reserved for prefix-update operands: `++x ** 2` may use
/// the updated value as the left operand, while ordinary unary expressions
/// such as `-x ** 2` are early errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PowerMode {
    Allowed,
    Forbidden,
    None,
}

/// QuickJS `PF_IN_ACCEPTED`, kept as parser state so recursive assignment RHS
/// inherits ExpressionNoIn while parentheses and selected grammar entries can
/// temporarily restore the ordinary Expression grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InMode {
    Allow,
    Disallow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ForHeadDelimiter {
    Parenthesis,
    Bracket,
    Brace,
    Template,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ForIterationKind {
    In,
    Of,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ForAssignmentDeclaration {
    Assignment,
    Var,
    Lexical,
}

#[derive(Clone, Debug)]
struct ForAssignmentTargetInfo {
    declaration: ForAssignmentDeclaration,
    var_initializer: Option<IdentifierReference>,
    is_destructuring: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallArguments {
    Fixed(u16),
    Spread,
}

#[derive(Debug)]
enum IrOp {
    Bytecode(Instruction),
    /// Parser-only `ImportMeta` marker. It resolves directly to the hidden
    /// module cell and deliberately never becomes an IdentifierReference, so
    /// assignment/update syntax cannot target it.
    ImportMeta {
        span: Span,
        scope: ScopeId,
    },
    /// Typed counterparts of QuickJS `OP_enter_scope` / `OP_leave_scope`.
    /// They remain scope identities until every declaration and child capture
    /// is known, then lowering expands entry to lexical TDZ initialization and
    /// exit to `CloseLocal` for exactly the locals captured by children.
    EnterScope(ScopeId),
    /// QuickJS places the Catch handler label after `OP_enter_scope`, leaving
    /// CatchParameter slots at their frame-default `undefined` values. Oxide
    /// starts every lexical frame slot as Uninitialized, so this handler-only
    /// preparation explicitly reproduces the skipped-entry state without
    /// weakening the binding's static lexical metadata.
    PrepareCatchScope(ScopeId),
    LeaveScope(ScopeId),
    /// Stable publication marker separating parameter BindingPattern
    /// evaluation from the authored body scope. It lowers to one Nop so the
    /// trust boundary can authenticate the segment after jump remapping.
    ParameterInitializationEnd,
    /// QuickJS's template parser does not apply the ordinary call parser's
    /// u16 argument guard.  Retain the full count until the bytecode stack
    /// limit has been checked during lowering.
    TemplateCall {
        argument_count: usize,
        method: bool,
    },
    /// Parser/linker form of QuickJS `OP_eval`. Retain the syntactic call
    /// site's scope identity until bytecode publication so String-source eval
    /// can later lower it into a verified environment descriptor. R1v's
    /// non-String shell intentionally publishes only the argument shape.
    EvalCall {
        arguments: CallArguments,
        scope: ScopeId,
        environment: Option<u16>,
    },
    PushConstant(u32),
    MakeClosure(u32),
    /// Lowering-only assignment-expression form. QuickJS has no `set_var`;
    /// this expands to `dup; put_var` before verification/publication.
    GlobalSet(u16),
    /// QuickJS has no value-preserving checked VarRef write. Keep the typed
    /// operation unresolved until lowering expands it to `dup; put_var_ref_check`.
    CapturedLexicalSet(u16),
    /// One or more QuickJS `with_*`-shaped checks against hidden sloppy-eval
    /// variable objects, followed by the statically resolved outer fallback.
    DynamicIdentifier {
        name: u32,
        access: IdentifierAccess,
        sources: Box<[DynamicEnvironmentSource]>,
        fallback: Box<IrOp>,
    },
    /// Resolved identifier Reference. `sources` are selected once before the
    /// RHS/call; `late_sources` are consulted only when no authored `with`
    /// made QuickJS enter Reference mode (notably an imported eval `<with>`).
    DynamicIdentifierReference {
        name: u32,
        access: IdentifierReferenceAccess,
        sources: Box<[DynamicEnvironmentSource]>,
        late_sources: Box<[DynamicEnvironmentSource]>,
        fallback: Box<IrOp>,
        syntactic_with: bool,
        fallback_readonly: bool,
    },
    Identifier {
        name: String,
        span: Span,
        scope: ScopeId,
        access: IdentifierAccess,
    },
    IdentifierReference {
        name: String,
        span: Span,
        scope: ScopeId,
        access: IdentifierReferenceAccess,
    },
    /// Parser/linker form of a private data-field operation. The private name
    /// remains a source spelling plus lexical scope until child-first binding
    /// resolution can select an authenticated local/closure cell. It is never
    /// represented by a normal Identifier operation or public stack Value.
    PrivateField {
        name: String,
        span: Span,
        scope: ScopeId,
        access: PrivateFieldAccess,
    },
}

#[derive(Debug)]
struct SpannedIrOp {
    op: IrOp,
    /// `Some` is the typed equivalent of QuickJS `emit_source_pos`; `None`
    /// leaves the previous source position in force.
    pc_site: Option<SourceOffset>,
}

/// Parser-only counterpart of the breakable-statement part of QuickJS
/// `BlockEnv`. Each function owns its own stack so a nested function cannot
/// target an outer statement. `drop_count` models the values which must be
/// removed when an abrupt jump crosses a control (the retained switch
/// discriminant today). Try/finally unwinding is represented by the dedicated
/// control kinds below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BreakControlKind {
    RegularStatement,
    Loop,
    /// QuickJS installs a transient `BlockEnv` with `has_iterator` while
    /// lowering every ArrayBindingPattern or ArrayAssignmentPattern. It has
    /// no break/continue target, but a generator `.return(value)` injected at
    /// a `yield` inside the pattern must still close this iterator before the
    /// frame returns.
    DestructuringIterator,
    /// QuickJS precompiles the assignment fragment of a for-of/for-await head
    /// before the right-hand side installs the loop iterator record. At
    /// runtime that fragment executes with the record active, but a generator
    /// return from the fragment abandons it without calling the outer
    /// iterator's `return` method.
    ForOfAssignmentFragment,
    /// QuickJS's `has_iterator` BlockEnv, shared by for-of and for-await. Its
    /// target depth retains the conceptual `iterator`, `next`, and private
    /// unwind marker slots. A same-loop continue keeps that record, a break
    /// reaches the shared close tail, and an edge crossing the loop closes it
    /// immediately.
    ForOf,
    /// QuickJS for-in retains one hidden enumeration object. Same-loop
    /// continue keeps it, the shared break tail drops it, and a jump crossing
    /// the loop removes it without IteratorClose.
    ForIn,
    Switch,
    /// QuickJS's catch-marker BlockEnv. It is not itself breakable, but every
    /// abrupt edge crossing it must discard the marker and call its finally
    /// subroutine (which may be the empty `Ret` used by try/catch).
    TryFinally,
    /// The BlockEnv active while parsing a finally body. A break/continue
    /// leaving it discards the pending value and gosub return address so the
    /// new abrupt completion overrides the old one.
    FinallyBody,
}

#[derive(Debug)]
struct BreakControlContext {
    kind: BreakControlKind,
    label_name: Option<String>,
    /// Parser scope active when QuickJS pushes this `BlockEnv`. Abrupt jumps
    /// leave descendant lexical scopes, but keep the matched control's own
    /// scope active until its shared tail runs.
    scope: ScopeId,
    entry_depth: usize,
    drop_count: usize,
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
    /// Parser-IR Gosub sites whose common target is known only after the catch
    /// and optional finally clauses have been parsed.
    finally_gosubs: Vec<usize>,
}

impl IrOp {
    fn stack_effect(&self) -> (usize, usize) {
        match self {
            Self::Bytecode(instruction) => instruction.stack_effect(),
            Self::ImportMeta { .. } => (0, 1),
            Self::EnterScope(_)
            | Self::PrepareCatchScope(_)
            | Self::LeaveScope(_)
            | Self::ParameterInitializationEnd => (0, 0),
            Self::TemplateCall {
                argument_count,
                method,
            } => (argument_count + usize::from(*method) + 1, 1),
            Self::EvalCall { arguments, .. } => match arguments {
                CallArguments::Fixed(argument_count) => (usize::from(*argument_count) + 1, 1),
                CallArguments::Spread => (2, 1),
            },
            Self::PushConstant(_) | Self::MakeClosure(_) => (0, 1),
            Self::GlobalSet(_) | Self::CapturedLexicalSet(_) => (1, 1),
            Self::DynamicIdentifier { access, .. } => match access {
                IdentifierAccess::Get
                | IdentifierAccess::GetOrUndefined
                | IdentifierAccess::Delete => (0, 1),
                IdentifierAccess::Initialize
                | IdentifierAccess::InitializeDerivedThis
                | IdentifierAccess::Put
                | IdentifierAccess::AnnexBPut => (1, 0),
                IdentifierAccess::Set => (1, 1),
            },
            Self::DynamicIdentifierReference { access, .. }
            | Self::IdentifierReference { access, .. } => match access {
                IdentifierReferenceAccess::Prepare => (0, 1),
                IdentifierReferenceAccess::Get | IdentifierReferenceAccess::Call => (0, 2),
                IdentifierReferenceAccess::Set => (2, 1),
                IdentifierReferenceAccess::PostPut => (3, 1),
            },
            Self::Identifier {
                access:
                    IdentifierAccess::Get | IdentifierAccess::GetOrUndefined | IdentifierAccess::Delete,
                ..
            } => (0, 1),
            Self::Identifier {
                access:
                    IdentifierAccess::Initialize
                    | IdentifierAccess::InitializeDerivedThis
                    | IdentifierAccess::Put
                    | IdentifierAccess::AnnexBPut,
                ..
            } => (1, 0),
            Self::Identifier {
                access: IdentifierAccess::Set,
                ..
            } => (1, 1),
            Self::PrivateField { access, .. } => match access {
                PrivateFieldAccess::Get | PrivateFieldAccess::In => (1, 1),
                PrivateFieldAccess::GetKeepReceiver => (1, 2),
                PrivateFieldAccess::Put => (2, 0),
                PrivateFieldAccess::Define => (2, 1),
            },
        }
    }
}

#[derive(Debug)]
struct IrParameterPatternBinding {
    name: String,
    parameter_local: u16,
    body_local: Option<u16>,
    declaration_span: Span,
}

#[derive(Debug)]
struct FunctionIr {
    /// Parent function plus the scope which was current at this function's
    /// definition. This is QuickJS `parent` + `parent_scope_level` as one
    /// invariant-preserving typed link.
    parent: Option<ParentLink>,
    kind: FunctionKind,
    /// Callable execution semantics are independent from the grammar role.
    /// A class or object generator remains a concise `Method` for bindings
    /// and HomeObject purposes while publishing generator bytecode.
    execution_kind: BytecodeFunctionKind,
    /// YieldExpression is disabled while generator formal initializers parse
    /// and becomes active only after the InitialYield boundary is installed.
    in_function_body: bool,
    /// Base/derived class constructors share the compiler's concise-method
    /// binding model but publish constructor bytecode without an ordinary
    /// function's eagerly visible `.prototype` shape. `DefineClass` owns that
    /// descriptor, while `CheckCtor` enforces construct-only invocation.
    class_constructor: bool,
    /// Whether this class constructor uses the derived [[Construct]]
    /// protocol. Keeping this separate from ordinary constructability lets
    /// arrows and direct eval inherit `super()` authority without pretending
    /// to be constructors themselves.
    derived_class_constructor: bool,
    /// Synthetic QuickJS class-element program role. This is assigned only by
    /// class lowering after the ordinary method-shaped FunctionIr is created.
    class_initializer_kind: Option<ClassInitializerKind>,
    /// Whether this authenticated instance/static aggregate installs the
    /// private-method brand for its class side before executing element code.
    class_private_brand: bool,
    /// QuickJS parser authority copied independently from HomeObject storage.
    super_call_allowed: bool,
    super_allowed: bool,
    arguments_forbidden: bool,
    source: FunctionSourceInfo,
    /// Intrinsic function name, independent of contextual `SetName` inference
    /// for anonymous definitions.
    function_name: Option<String>,
    /// Whether a named expression may lazily create QuickJS's private
    /// `JS_VAR_FUNCTION_NAME` self binding. Declarations carry an intrinsic
    /// name but resolve recursion through their authored environment.
    private_name_binding: bool,
    /// Lazily allocated private self-binding local.
    function_name_local: Option<u16>,
    /// Root local initialized by the typed arguments-object entry prologue.
    ///
    /// Like QuickJS's `arguments_var_idx`, this is selected only when source
    /// resolution (or a function-scoped `var`/function declaration) needs the
    /// implicit binding. A named physical `arguments` parameter suppresses it;
    /// a BindingPattern BoundName does not, because QuickJS reserves an
    /// anonymous argument slot and initializes the arguments object first.
    arguments_local: Option<u16>,
    /// Lazily materialized QuickJS pseudo variables captured by descendant
    /// arrows or exposed to direct eval. Arrow frames never own these locals;
    /// only concise methods can own the HomeObject cell.
    home_object_local: Option<u16>,
    /// QuickJS's hidden `this_active_func`, captured by arrows/direct eval so
    /// `super()` dynamically reads the active constructor's [[Prototype]].
    active_function_local: Option<u16>,
    this_local: Option<u16>,
    new_target_local: Option<u16>,
    /// Hidden null-prototype variable object for sloppy authored function code
    /// containing syntactic direct eval. Its identity is explicit rather than
    /// inferred from local allocation order.
    eval_variable_object_local: Option<u16>,
    /// Hidden `<arg_var>` object used by sloppy direct eval in a parentless
    /// Parameter Environment. Unlike authored parameter cells, this slot is
    /// rooted for the full activation and is projected into both parameter
    /// and body eval descriptors.
    arg_eval_variable_object_local: Option<u16>,
    /// Sloppy Parameter Environment alias of the ordinary function's
    /// unmapped arguments object. QuickJS skips this cell's ordinary TDZ reset
    /// and initializes it together with the body arguments binding.
    synthetic_parameter_arguments_local: Option<u16>,
    /// A lazily allocated HomeObject pseudo local requires the published
    /// method function to retain its object literal as HomeObject. Descendant
    /// arrows relay the local without carrying this metadata themselves.
    needs_home_object: bool,
    /// Physical call-frame argument slots. Destructuring parameters use an
    /// unnamed slot, matching QuickJS's `JS_ATOM_NULL` argument descriptor;
    /// their individual BoundNames live in root locals instead.
    parameters: Vec<Option<String>>,
    /// Every authored BoundName in formal-list order, including leaves of a
    /// BindingPattern. This is the authority for duplicate-parameter policy,
    /// `arguments` shadowing, and Annex B parameter-name checks.
    parameter_names: Vec<String>,
    /// QuickJS `defined_arg_count`, exposed as the function's public `length`.
    /// An identifier rest parameter owns a physical argument slot but is not
    /// included in this count.
    defined_argument_count: usize,
    /// QuickJS `has_simple_parameter_list`. Besides early-error policy, this
    /// selects mapped versus unmapped `arguments` for sloppy functions.
    has_simple_parameter_list: bool,
    /// Physical argument slot overwritten by the entry-time `OP_rest` result.
    rest_parameter: Option<u16>,
    /// First actual argument collected for a terminal `...BindingPattern`.
    /// Unlike an identifier rest parameter this does not reserve a physical
    /// frame slot; the fresh Array is consumed directly by destructuring.
    rest_pattern_start: Option<u16>,
    /// Independent declarative scope used by identifier default parameters.
    /// QuickJS calls this its argument scope; keeping the identity explicit
    /// lets resolution enforce the body-variable visibility barrier.
    parameter_scope: Option<ScopeId>,
    /// Every initializer-visible mutable cell owned by `parameter_scope`, in
    /// FormalParameters BoundName order. Identifier formals contribute one
    /// cell while a BindingPattern contributes one cell per leaf.
    parameter_locals: Vec<u16>,
    /// Exact whole-list pre-scan reservation for authored parameter cells.
    /// Reserving this leading local prefix before parsing any initializer
    /// prevents nested class/function compilation from interleaving scratch
    /// locals with the heap-visible Parameter Environment ABI.
    parameter_local_reservation_count: Option<usize>,
    /// Parameter-scope cell selected by each physical named argument. An
    /// anonymous BindingPattern slot has no direct cell because destructuring
    /// initializes its individual BoundNames instead.
    parameter_argument_locals: Vec<Option<u16>>,
    /// Parameter-scope BindingPattern leaves which must be copied into fresh
    /// FunctionRoot variables after every parameter expression has run.
    parameter_pattern_bindings: Vec<IrParameterPatternBinding>,
    /// Top-level formal initializers in source order. Pattern-leaf defaults
    /// create the argument scope but do not cut Function.length, so they are
    /// intentionally absent from this list.
    parameter_default_sources: Vec<ParameterDefaultSource>,
    /// At least one BindingPattern is initialized before the authored body.
    /// Without a Parameter Environment it runs in FunctionRoot; with one it
    /// runs in the parentless parameter scope and is copied out at the end.
    pattern_parameter_initialization: bool,
    locals: Vec<String>,
    scopes: Vec<IrScope>,
    bindings: Vec<IrBinding>,
    global_declarations: Vec<IrGlobalDeclaration>,
    /// Last direct function declaration attached to each ordinary
    /// function-scoped argument/local binding.
    hoisted_functions: Vec<IrHoistedFunction>,
    /// Source-ordered declaration records for sloppy direct eval targeting a
    /// caller function's variable environment.
    eval_declarations: Vec<IrEvalDeclaration>,
    eval_declarations_installed: bool,
    /// First caller lexical name which conflicts with an eval `var`/function.
    /// The eval still compiles so global declaration instantiation can run
    /// before this typed SyntaxError is thrown at bytecode entry.
    eval_redeclaration: Option<String>,
    function_hoists_installed: bool,
    /// Phase marker for the final hidden-frame entry prefix. Unlike ordinary
    /// body hoists this also applies to scripts and eval roots, so it cannot
    /// be inferred from `function_hoists_installed`.
    pseudo_binding_prologues_installed: bool,
    /// Scoped lexical function slots, including one slot per sloppy same-scope
    /// duplicate as in QuickJS `JS_VAR_FUNCTION_DECL`.
    scoped_functions: Vec<IrScopedFunction>,
    /// ProgramBody's labelled-function exception has authored closure writes
    /// but no lexical scope-entry slot.
    program_annex_functions: Vec<IrProgramAnnexFunction>,
    current_scope: ScopeId,
    var_scope: ScopeId,
    body_scope: ScopeId,
    /// QuickJS `eval_ret_idx`: the script-only hidden completion local.
    /// Keeping the typed slot separate from its unspellable debug name avoids
    /// confusing it with future source bindings or other synthetic locals.
    eval_ret_local: Option<u16>,
    /// Every local which deliberately has no source binding identity. This is
    /// validated separately from authored locals before publication.
    synthetic_locals: Vec<SyntheticLocal>,
    ops: Vec<SpannedIrOp>,
    /// Parser-only Reference marker for the final member getter. QuickJS uses
    /// `last_opcode_pos` for the same rewrite, but an explicit index prevents
    /// comma/conditional values from accidentally retaining a method receiver.
    last_member_reference: Option<usize>,
    /// Parser-only Reference marker for a final identifier read. This lets
    /// parenthesized IdentifierReferences remain assignment targets while
    /// composed values (comma, conditional, logical and binary forms) do not.
    last_identifier_reference: Option<usize>,
    /// A completed optional chain whose value has not yet been composed with
    /// an outer operation. Parentheses deliberately preserve this marker.
    last_optional_chain: Option<FinalizedOptionalChain>,
    constants: Vec<IrConstant>,
    closure_variables: Vec<ClosureVariable>,
    /// Exact flattened caller bindings imported by a synthetic direct-eval
    /// root. Entries retain their original R1w descriptor indices even though
    /// bindings are inserted into the synthetic root in outer-to-inner order
    /// so ordinary reverse lookup selects the innermost duplicate name.
    external_bindings: Vec<EvalRootBinding<JsString>>,
    /// Exact imported caller scope topology and variable target.  The root's
    /// flat external binding vector remains the closure-prefix ABI, while this
    /// profile reconstructs the original ordered suffix for nested eval.
    eval_caller_profile: EvalCallerProfile,
    /// Immutable QuickJS-shaped scope chains linked for syntactic direct-eval
    /// call sites. Multiple calls from the same parser scope share one entry.
    eval_environments: Vec<EvalEnvironment<JsString>>,
    break_controls: Vec<BreakControlContext>,
    stack_depth: usize,
    strict: bool,
}

#[derive(Clone, Debug)]
struct FunctionSourceInfo {
    span: Span,
    definition: SourceOffset,
    range: Option<Range<SourceOffset>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SuperCapabilities {
    super_call_allowed: bool,
    super_allowed: bool,
}

impl SuperCapabilities {
    const NONE: Self = Self {
        super_call_allowed: false,
        super_allowed: false,
    };
    const PROPERTY: Self = Self {
        super_call_allowed: false,
        super_allowed: true,
    };
    const CALL_AND_PROPERTY: Self = Self {
        super_call_allowed: true,
        super_allowed: true,
    };

    fn validated(self) -> Result<Self, Error> {
        if self.super_call_allowed && !self.super_allowed {
            return Err(Error::internal(
                "function permits super() without SuperProperty",
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug)]
struct FunctionIrOptions {
    function_name: Option<String>,
    private_name_binding: bool,
    class_constructor: bool,
    derived_class_constructor: bool,
    parameters: Vec<Option<String>>,
    defined_argument_count: usize,
    has_simple_parameter_list: bool,
    rest_parameter: Option<u16>,
    strict: bool,
    super_capabilities: SuperCapabilities,
}

impl FunctionIr {
    fn new(
        parent: Option<ParentLink>,
        kind: FunctionKind,
        source: FunctionSourceInfo,
        options: FunctionIrOptions,
    ) -> Result<Self, Error> {
        let super_capabilities = options.super_capabilities.validated()?;
        if options.derived_class_constructor
            && (!options.class_constructor
                || kind != FunctionKind::Method
                || super_capabilities != SuperCapabilities::CALL_AND_PROPERTY)
        {
            return Err(Error::internal("derived constructor metadata is malformed"));
        }
        let parameter_count = options.parameters.len();
        if options.defined_argument_count > options.parameters.len()
            || (options.has_simple_parameter_list
                && (options.defined_argument_count != options.parameters.len()
                    || options.rest_parameter.is_some()
                    || options.parameters.iter().any(Option::is_none)))
            || options.rest_parameter.is_some_and(|rest| {
                usize::from(rest) + 1 != options.parameters.len()
                    || options.defined_argument_count != usize::from(rest)
                    || options.has_simple_parameter_list
                    || options.parameters[usize::from(rest)].is_none()
            })
        {
            return Err(Error::internal("formal parameter metadata is malformed"));
        }
        let (locals, eval_ret_local, synthetic_locals) =
            if matches!(kind, FunctionKind::Script | FunctionKind::Eval(_)) {
                (
                    vec![EVAL_RET_LOCAL_NAME.to_owned()],
                    Some(0),
                    vec![SyntheticLocal {
                        index: 0,
                        kind: SyntheticLocalKind::EvalCompletion,
                    }],
                )
            } else {
                (Vec::new(), None, Vec::new())
            };
        // QuickJS reserves scope zero for arguments/function-scoped storage,
        // then pushes the authored body scope. Named-expression self storage
        // is a lazy local in the root, not a synthetic lexical parent scope.
        let function_root = ScopeId(0);
        let body = ScopeId(1);
        let scopes = vec![
            IrScope {
                parent: None,
                kind: ScopeKind::FunctionRoot,
                is_parameter_initializer: false,
                bindings: Vec::new(),
            },
            IrScope {
                parent: Some(function_root),
                kind: if matches!(
                    kind,
                    FunctionKind::Script | FunctionKind::Module | FunctionKind::Eval(_)
                ) {
                    ScopeKind::ProgramBody
                } else {
                    ScopeKind::FunctionBody
                },
                is_parameter_initializer: false,
                bindings: Vec::new(),
            },
        ];
        let current_scope = body;
        let var_scope = function_root;
        let ops = if matches!(
            kind,
            FunctionKind::Ordinary
                | FunctionKind::Method
                | FunctionKind::Arrow
                | FunctionKind::Eval(_)
        ) {
            vec![SpannedIrOp {
                op: IrOp::EnterScope(body),
                pc_site: None,
            }]
        } else {
            Vec::new()
        };
        let mut function = Self {
            parent,
            kind,
            execution_kind: BytecodeFunctionKind::Normal,
            in_function_body: false,
            class_constructor: options.class_constructor,
            derived_class_constructor: options.derived_class_constructor,
            class_initializer_kind: None,
            class_private_brand: false,
            super_call_allowed: super_capabilities.super_call_allowed,
            super_allowed: super_capabilities.super_allowed,
            arguments_forbidden: false,
            source,
            function_name: options.function_name,
            private_name_binding: options.private_name_binding,
            function_name_local: None,
            arguments_local: None,
            home_object_local: None,
            active_function_local: None,
            this_local: None,
            new_target_local: None,
            eval_variable_object_local: None,
            arg_eval_variable_object_local: None,
            synthetic_parameter_arguments_local: None,
            needs_home_object: false,
            parameters: options.parameters,
            parameter_names: Vec::new(),
            defined_argument_count: options.defined_argument_count,
            has_simple_parameter_list: options.has_simple_parameter_list,
            rest_parameter: options.rest_parameter,
            rest_pattern_start: None,
            parameter_scope: None,
            parameter_locals: Vec::new(),
            parameter_local_reservation_count: None,
            parameter_argument_locals: vec![None; parameter_count],
            parameter_pattern_bindings: Vec::new(),
            parameter_default_sources: Vec::new(),
            pattern_parameter_initialization: false,
            locals,
            scopes,
            bindings: Vec::new(),
            global_declarations: Vec::new(),
            hoisted_functions: Vec::new(),
            eval_declarations: Vec::new(),
            eval_declarations_installed: false,
            eval_redeclaration: None,
            function_hoists_installed: false,
            pseudo_binding_prologues_installed: false,
            scoped_functions: Vec::new(),
            program_annex_functions: Vec::new(),
            current_scope,
            var_scope,
            body_scope: body,
            eval_ret_local,
            synthetic_locals,
            ops,
            last_member_reference: None,
            last_identifier_reference: None,
            last_optional_chain: None,
            constants: Vec::new(),
            closure_variables: Vec::new(),
            external_bindings: Vec::new(),
            eval_caller_profile: EvalCallerProfile {
                scope_kinds: Box::new([]),
                variable_target: EvalCallerVariableTarget::Global,
            },
            eval_environments: Vec::new(),
            break_controls: Vec::new(),
            stack_depth: 0,
            strict: options.strict,
        };
        for (index, name) in function.parameters.clone().into_iter().enumerate() {
            let Some(name) = name else {
                continue;
            };
            let index = u16::try_from(index)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
            function.parameter_names.push(name.clone());
            function.add_binding(
                function.var_scope,
                function.var_scope,
                name,
                BindingStorage::Argument(index),
                BindingKind::Normal,
                None,
            );
        }
        Ok(function)
    }

    /// Allocate the derived constructor's hidden cells after formal parsing.
    /// Parameter-environment cells must remain the leading locals, but
    /// unresolved `super()`/`this` operations in parameter initializers do not
    /// need physical operands until the later identifier-linking pass.
    fn allocate_derived_constructor_pseudo_bindings(&mut self) -> Result<(), Error> {
        if !self.derived_class_constructor
            || !self.class_constructor
            || self.kind != FunctionKind::Method
            || self.active_function_local.is_some()
            || self.this_local.is_some()
        {
            return Err(Error::internal(
                "derived constructor pseudo bindings were allocated in an invalid phase",
            ));
        }
        if self.locals.len().saturating_add(2) > MAX_LOCAL_VARIABLES {
            return Err(Error::new(
                ErrorKind::JsInternal,
                "too many local variables",
            ));
        }
        let active_function = u16::try_from(self.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        self.locals.push(ACTIVE_FUNCTION_LOCAL_NAME.to_owned());
        self.active_function_local = Some(active_function);
        self.add_binding(
            self.var_scope,
            self.var_scope,
            ACTIVE_FUNCTION_LOCAL_NAME.to_owned(),
            BindingStorage::Local(active_function),
            BindingKind::Normal,
            None,
        );

        let this = u16::try_from(self.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        self.locals.push(THIS_LOCAL_NAME.to_owned());
        self.this_local = Some(this);
        self.add_binding(
            self.var_scope,
            self.var_scope,
            THIS_LOCAL_NAME.to_owned(),
            BindingStorage::Local(this),
            BindingKind::Lexical { is_const: false },
            None,
        );
        Ok(())
    }

    fn add_binding(
        &mut self,
        storage_scope: ScopeId,
        declaration_scope: ScopeId,
        name: String,
        storage: BindingStorage,
        kind: BindingKind,
        declaration_span: Option<Span>,
    ) -> BindingId {
        let binding = BindingId(self.bindings.len());
        self.bindings.push(IrBinding {
            name,
            storage_scope,
            declaration_scope,
            storage,
            kind,
            is_scoped_function: false,
            is_scoped_generator: false,
            is_catch_parameter: false,
            declaration_span,
        });
        self.scopes[storage_scope.0].bindings.push(binding);
        binding
    }

    fn add_synthetic_local(&mut self, kind: SyntheticLocalKind) -> Result<u16, Error> {
        if self.locals.len() >= MAX_LOCAL_VARIABLES {
            return Err(Error::new(
                ErrorKind::JsInternal,
                "too many local variables",
            ));
        }
        let index = u16::try_from(self.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        self.locals.push(kind.name().to_owned());
        self.synthetic_locals.push(SyntheticLocal { index, kind });
        Ok(index)
    }

    fn binding_in_scope(&self, scope: ScopeId, name: &str) -> Option<&IrBinding> {
        self.binding_id_in_scope(scope, name)
            .map(|binding| &self.bindings[binding.0])
    }

    fn binding_id_in_scope(&self, scope: ScopeId, name: &str) -> Option<BindingId> {
        self.scopes[scope.0]
            .bindings
            .iter()
            .rev()
            .copied()
            .find(|binding| self.bindings[binding.0].name == name)
    }

    fn binding_id_from_scope(
        &self,
        mut scope: ScopeId,
        name: &str,
    ) -> Option<(ScopeId, BindingId)> {
        loop {
            if let Some(binding) = self.binding_id_in_scope(scope, name) {
                return Some((scope, binding));
            }
            scope = self.scopes[scope.0].parent?;
        }
    }

    fn first_global_declaration_is_normal(&self, name: &str) -> bool {
        self.global_declarations
            .iter()
            .find(|declaration| declaration.name == name)
            .is_some_and(|declaration| !declaration.is_lexical)
    }

    fn binding_from_scope(&self, mut scope: ScopeId, name: &str) -> Option<ResolvedBinding> {
        loop {
            if let Some(binding) = self.binding_in_scope(scope, name) {
                return Some(ResolvedBinding {
                    storage: binding.storage,
                    kind: binding.kind,
                });
            }
            scope = self.scopes[scope.0].parent?;
        }
    }

    fn scope_is_within(&self, mut scope: ScopeId, ancestor: ScopeId) -> bool {
        loop {
            if scope == ancestor {
                return true;
            }
            let Some(parent) = self.scopes[scope.0].parent else {
                return false;
            };
            scope = parent;
        }
    }
}

fn install_eval_external_bindings(
    function: &mut FunctionIr,
    bindings: Box<[EvalRootBinding<JsString>]>,
    caller_profile: EvalCallerProfile,
    caller_strict: bool,
) -> Result<(), Error> {
    let FunctionKind::Eval(kind) = function.kind else {
        return Err(Error::internal(
            "eval caller bindings escaped a synthetic eval root",
        ));
    };
    if kind == EvalKind::Indirect && !bindings.is_empty() {
        return Err(Error::internal(
            "indirect eval root received external caller bindings",
        ));
    }
    if !function.closure_variables.is_empty() || !function.external_bindings.is_empty() {
        return Err(Error::internal(
            "eval caller bindings were installed more than once",
        ));
    }
    if bindings.iter().any(|binding| {
        let Some(&scope_kind) = caller_profile.scope_kinds.get(usize::from(binding.scope)) else {
            return true;
        };
        (binding.is_catch_parameter && scope_kind != EvalScopeKind::Catch)
            || (binding.kind == ClosureVariableKind::WithObject)
                != (scope_kind == EvalScopeKind::With)
    }) || caller_profile
        .scope_kinds
        .iter()
        .enumerate()
        .any(|(scope, kind)| {
            *kind == EvalScopeKind::With
                && bindings
                    .iter()
                    .filter(|binding| usize::from(binding.scope) == scope)
                    .count()
                    != 1
        })
    {
        return Err(Error::internal(
            "eval caller bindings disagree with their scope profile",
        ));
    }
    let has_variable_object = bindings.iter().any(|binding| {
        matches!(
            binding.kind,
            ClosureVariableKind::EvalVariableObject | ClosureVariableKind::ArgEvalVariableObject
        )
    });
    match (caller_strict, caller_profile.variable_target) {
        (false, EvalCallerVariableTarget::Global) if !has_variable_object => {}
        (true, EvalCallerVariableTarget::StrictLocal) if kind == EvalKind::Direct => {}
        (false, EvalCallerVariableTarget::ExternalBinding(index))
            if bindings.get(usize::from(index)).is_some_and(|binding| {
                matches!(
                    binding.kind,
                    ClosureVariableKind::EvalVariableObject
                        | ClosureVariableKind::ArgEvalVariableObject
                ) && !binding.is_lexical
                    && !binding.is_const
                    && !binding.is_catch_parameter
            }) => {}
        _ => {
            return Err(Error::internal(
                "eval caller variable target is not authenticated",
            ));
        }
    }

    for (index, binding) in bindings.iter().enumerate() {
        let index = u16::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
        let name = String::from_utf16(&binding.name.utf16_units().collect::<Vec<_>>())
            .map_err(|_| Error::internal("eval caller binding name is not well formed"))?;
        let name = ensure_string_constant(function, &name)?;
        let descriptor = ClosureVariable {
            source: ClosureSource::EvalEnvironment(index),
            name: ClosureVariableName::Constant(name),
            is_lexical: binding.is_lexical,
            is_const: binding.is_const,
            kind: binding.kind,
        };
        let installed = push_closure_variable(function, descriptor)?;
        if installed != index {
            return Err(Error::internal(
                "eval caller closure indices are not contiguous",
            ));
        }
    }

    // Scope bindings are searched newest-first. Install outer-to-inner so the
    // innermost exact descriptor wins for duplicate names while every closure
    // slot remains available to the specialized publication verifier. The
    // `<var>` remains unspellable source metadata, but it must still have a
    // binding identity in the synthetic root.  QuickJS relays the same hidden
    // closure VarRef when eval source itself contains a direct eval; retaining
    // it here lets that later call authenticate the exact variable target.
    for (index, binding) in bindings.iter().enumerate().rev() {
        let index = u16::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
        if binding.kind == ClosureVariableKind::EvalVariableObject
            && (binding.is_lexical
                || binding.is_const
                || binding.is_catch_parameter
                || binding.name.to_utf8_lossy() != EVAL_VARIABLE_OBJECT_LOCAL_NAME)
        {
            return Err(Error::internal(
                "eval variable object binding metadata is malformed",
            ));
        }
        if binding.kind == ClosureVariableKind::ArgEvalVariableObject
            && (binding.is_lexical
                || binding.is_const
                || binding.is_catch_parameter
                || binding.name.to_utf8_lossy() != ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME)
        {
            return Err(Error::internal(
                "argument eval variable object binding metadata is malformed",
            ));
        }
        if binding.kind == ClosureVariableKind::WithObject
            && (binding.is_lexical
                || binding.is_const
                || binding.is_catch_parameter
                || binding.name.to_utf8_lossy() != WITH_OBJECT_LOCAL_NAME)
        {
            return Err(Error::internal("with object binding metadata is malformed"));
        }
        let name = String::from_utf16(&binding.name.utf16_units().collect::<Vec<_>>())
            .map_err(|_| Error::internal("eval caller binding name is not well formed"))?;
        let kind =
            binding_kind_from_closure_flags(binding.kind, binding.is_lexical, binding.is_const)
                .ok_or_else(|| Error::internal("eval caller binding flags are inconsistent"))?;
        let installed = function.add_binding(
            function.var_scope,
            function.var_scope,
            name,
            BindingStorage::External(index),
            kind,
            None,
        );
        function.bindings[installed.0].is_catch_parameter = binding.is_catch_parameter;
    }
    function.external_bindings = bindings.into_vec();
    function.eval_caller_profile = caller_profile;
    Ok(())
}

#[derive(Debug)]
struct FunctionTree {
    functions: Vec<FunctionIr>,
    source: SourceText,
    filename: JsString,
    module: Option<module::IrModule>,
    pending_unsupported: Option<Error>,
}

#[derive(Clone, Copy, Debug)]
struct PreparedScopedFunction {
    binding: BindingId,
    create_annex_binding: bool,
}

#[derive(Clone, Copy, Debug)]
enum AnonymousFunctionDefinition {
    Function,
    Class {
        owner: FunctionId,
        /// IR insertion point immediately before the static initializer
        /// closure. NamedEvaluation is moved here so static elements observe
        /// the inferred class name.
        static_initializer_start: Option<usize>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModuleDeclarationExport {
    None,
    Named,
    Default,
}

struct Parser<'source> {
    lexer: Lexer<'source>,
    tokens: Vec<Token<'source>>,
    cursor: usize,
    current_function: FunctionId,
    in_mode: InMode,
    functions: Vec<FunctionIr>,
    module: Option<module::IrModule>,
    module_declaration_export: ModuleDeclarationExport,
    module_declaration_export_target: Option<(FunctionId, ScopeId)>,
    /// Function expression eligible for QuickJS's assignment-name inference.
    /// Operators which make the surrounding expression cease to be an
    /// AnonymousFunctionDefinition clear this marker.
    anonymous_function_definition: Option<AnonymousFunctionDefinition>,
    /// First syntactically valid construct which belongs to an unimplemented
    /// engine frontier. The parser keeps going so later grammar and early
    /// errors retain QuickJS priority over the implementation diagnostic.
    pending_unsupported: Option<Error>,
}

enum RootCompileContext {
    Script,
    Module,
    Eval(EvalCompileContext),
}

impl<'source> Parser<'source> {
    fn parse(source: &'source str, filename: JsString) -> Result<FunctionTree, Error> {
        Self::parse_root(source, None, filename, RootCompileContext::Script, None)
            .map_err(ModuleCompileFailure::into_engine_without_checker)
    }

    fn parse_module(
        source: &'source str,
        filename: JsString,
        checker: Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<FunctionTree, ModuleCompileFailure> {
        Self::parse_root(source, None, filename, RootCompileContext::Module, checker)
    }

    fn parse_module_source(
        source: &'source SourceText,
        filename: JsString,
        checker: Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<FunctionTree, ModuleCompileFailure> {
        Self::parse_root(
            source.carrier(),
            Some(source),
            filename,
            RootCompileContext::Module,
            checker,
        )
    }

    fn parse_script_source(
        source: &'source SourceText,
        filename: JsString,
    ) -> Result<FunctionTree, Error> {
        Self::parse_root(
            source.carrier(),
            Some(source),
            filename,
            RootCompileContext::Script,
            None,
        )
        .map_err(ModuleCompileFailure::into_engine_without_checker)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn parse_eval(
        source: &'source str,
        filename: JsString,
        context: EvalCompileContext,
    ) -> Result<FunctionTree, Error> {
        Self::parse_root(
            source,
            None,
            filename,
            RootCompileContext::Eval(context),
            None,
        )
        .map_err(ModuleCompileFailure::into_engine_without_checker)
    }

    fn parse_eval_source(
        source: &'source SourceText,
        filename: JsString,
        context: EvalCompileContext,
    ) -> Result<FunctionTree, Error> {
        Self::parse_root(
            source.carrier(),
            Some(source),
            filename,
            RootCompileContext::Eval(context),
            None,
        )
        .map_err(ModuleCompileFailure::into_engine_without_checker)
    }

    fn parse_root(
        source: &'source str,
        source_text: Option<&'source SourceText>,
        filename: JsString,
        context: RootCompileContext,
        mut module_attribute_checker: Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<FunctionTree, ModuleCompileFailure> {
        validate_source_length(source.len())?;
        let is_module = matches!(&context, RootCompileContext::Module);
        let (
            root_kind,
            inherited_strict,
            external_bindings,
            caller_profile,
            super_capabilities,
            arguments_forbidden,
        ) = match context {
            RootCompileContext::Script => (
                FunctionKind::Script,
                false,
                Vec::<EvalRootBinding<JsString>>::new().into_boxed_slice(),
                EvalCallerProfile {
                    scope_kinds: Box::new([]),
                    variable_target: EvalCallerVariableTarget::Global,
                },
                SuperCapabilities::NONE,
                false,
            ),
            RootCompileContext::Module => (
                FunctionKind::Module,
                true,
                Vec::<EvalRootBinding<JsString>>::new().into_boxed_slice(),
                EvalCallerProfile {
                    scope_kinds: Box::new([]),
                    variable_target: EvalCallerVariableTarget::StrictLocal,
                },
                SuperCapabilities::NONE,
                false,
            ),
            RootCompileContext::Eval(context) => {
                if !matches!(context.kind, EvalKind::Direct | EvalKind::Indirect) {
                    return Err(
                        Error::internal("eval compiler received a non-eval root kind").into(),
                    );
                }
                if context.kind == EvalKind::Indirect
                    && (!context.bindings.is_empty()
                        || !context.caller_profile.scope_kinds.is_empty()
                        || context.caller_profile.variable_target
                            != EvalCallerVariableTarget::Global
                        || context.super_call_allowed
                        || context.super_allowed
                        || context.arguments_forbidden)
                {
                    return Err(Error::internal(
                        "indirect eval compiler received a caller environment",
                    )
                    .into());
                }
                let super_capabilities = SuperCapabilities {
                    super_call_allowed: context.super_call_allowed,
                    super_allowed: context.super_allowed,
                }
                .validated()
                .map_err(|_| {
                    Error::internal("eval compiler permits super() without SuperProperty")
                })?;
                (
                    FunctionKind::Eval(context.kind),
                    context.kind == EvalKind::Direct && context.caller_strict,
                    context.bindings,
                    context.caller_profile,
                    super_capabilities,
                    context.arguments_forbidden,
                )
            }
        };
        // QuickJS enables Annex B HTML comments for every Script and Eval
        // parse, independently of strict mode. Module parsing keeps them
        // disabled (`allow_html_comments = !is_module`).
        let lexer_options = LexerOptions {
            context: LexContext {
                strict: inherited_strict,
                module: is_module,
                ..LexContext::default()
            },
            allow_html_comments: !is_module,
        };
        let mut lexer = match source_text {
            Some(source) => Lexer::with_source_text(source, lexer_options),
            None => Lexer::with_options(source, lexer_options),
        };
        let first_token = lexer.next_token().map_err(lex_error)?;
        let source_span = first_token.span;
        let mut parser = Self {
            lexer,
            tokens: vec![first_token],
            cursor: 0,
            current_function: 0,
            in_mode: InMode::Allow,
            anonymous_function_definition: None,
            pending_unsupported: None,
            module: is_module.then(module::IrModule::default),
            module_declaration_export: ModuleDeclarationExport::None,
            module_declaration_export_target: None,
            functions: vec![FunctionIr::new(
                None,
                root_kind,
                FunctionSourceInfo {
                    span: source_span,
                    definition: SourceOffset::try_from_usize(0)
                        .map_err(|error| Error::internal(error.to_string()))?,
                    range: None,
                },
                FunctionIrOptions {
                    function_name: (!is_module).then(|| "<eval>".to_owned()),
                    private_name_binding: false,
                    class_constructor: false,
                    derived_class_constructor: false,
                    parameters: Vec::new(),
                    defined_argument_count: 0,
                    has_simple_parameter_list: true,
                    rest_parameter: None,
                    strict: inherited_strict,
                    super_capabilities,
                },
            )?],
        };
        if is_module {
            // QuickJS compiles every module root as an async function. The
            // separate module record bit records whether authored evaluation
            // can actually suspend; keeping the callable kind async here lets
            // top-level AwaitExpression and `for await` reuse the ordinary
            // async-function lowering and continuation machinery.
            parser.functions[0].execution_kind = BytecodeFunctionKind::Async;
            parser.functions[0].in_function_body = true;
            parser.functions[0].eval_caller_profile = caller_profile.clone();
        }
        if matches!(root_kind, FunctionKind::Eval(_)) {
            install_eval_external_bindings(
                &mut parser.functions[0],
                external_bindings,
                caller_profile,
                inherited_strict,
            )?;
        }
        let strict =
            inherited_strict || parser.directive_prologue_has_use_strict(0, inherited_strict)?;
        parser.relex_current_with_strict(strict)?;
        parser.functions[0].strict = strict;
        parser.functions[0].arguments_forbidden = arguments_forbidden;
        if is_module {
            parser.parse_module_body(&mut module_attribute_checker)?;
        } else {
            parser.parse_script_body()?;
        }
        Ok(FunctionTree {
            functions: parser.functions,
            source: source_text
                .cloned()
                .unwrap_or_else(|| SourceText::from_utf8(source)),
            filename,
            module: parser.module,
            pending_unsupported: parser.pending_unsupported,
        })
    }

    fn parse_script_body(&mut self) -> Result<(), Error> {
        while !self.at_eof() {
            self.parse_statement_or_decl(
                StatementCompletion::Eval,
                StatementPosition::ProgramBody,
            )?;
        }

        self.emit_instruction(Instruction::GetLocal(self.eval_ret_local()?))?;
        self.emit_instruction(Instruction::Return)?;
        Ok(())
    }

    fn parse_function_body(&mut self) -> Result<(), Error> {
        while !self.is_punctuator(Punctuator::RightBrace) {
            if self.at_eof() {
                return Err(self.syntax_here("unterminated function body"));
            }
            self.parse_statement_or_decl(
                StatementCompletion::Discard,
                StatementPosition::FunctionBody,
            )?;
        }

        // QuickJS ends function bytecode with `return_undef`. It may
        // be unreachable after an explicit return, but keeps fallthrough
        // behavior structural and gives every function a terminal opcode.
        self.emit_instruction(Instruction::Undefined)?;
        if self.current_ir().derived_class_constructor {
            let this = self
                .current_ir()
                .this_local
                .ok_or_else(|| Error::internal("derived constructor has no this binding"))?;
            self.emit_instruction(Instruction::ReturnDerived(this))?;
        } else {
            self.emit_instruction(Instruction::Return)?;
        }
        Ok(())
    }

    /// QuickJS funnels program elements, function bodies, block bodies and
    /// single-statement branches through `js_parse_statement_or_decl`. Keep
    /// the same spine so completion handling, ASI and later declaration masks
    /// have one parser boundary instead of diverging script/function loops.
    fn parse_statement_or_decl(
        &mut self,
        completion: StatementCompletion,
        position: StatementPosition,
    ) -> Result<(), Error> {
        if self.consume_punctuator(Punctuator::Semicolon)? {
            return Ok(());
        }

        if self.lexical_declaration_ahead(position.allows_other_declaration())? {
            return match position {
                StatementPosition::FunctionBody => self.parse_lexical_statement(),
                StatementPosition::ProgramBody => self.parse_lexical_statement(),
                StatementPosition::NestedList => self.parse_lexical_statement(),
                StatementPosition::AnnexBIfArm
                | StatementPosition::AnnexBLabelBody
                | StatementPosition::Single => Err(self
                    .syntax_here("lexical declarations can't appear in single-statement context")),
            };
        }

        let annex_b_function_allowed = matches!(
            position,
            StatementPosition::AnnexBIfArm | StatementPosition::AnnexBLabelBody
        );
        if !position.allows_other_declaration()
            && self.restricted_function_declaration_ahead(annex_b_function_allowed)?
        {
            return Err(
                self.syntax_here("function declarations can't appear in single-statement context")
            );
        }

        if let Some(label_name) = self.label_ahead() {
            return self.parse_labeled_statement(completion, label_name, position);
        }

        match self.current().kind {
            TokenKind::Punctuator(Punctuator::LeftBrace) => self.parse_block_statement(completion),
            TokenKind::Keyword(Keyword::If) => self.parse_if_statement(completion),
            TokenKind::Keyword(Keyword::While) => self.parse_while_statement(completion, None),
            TokenKind::Keyword(Keyword::Do) => self.parse_do_while_statement(completion, None),
            TokenKind::Keyword(Keyword::For) => self.parse_for_statement(completion, None),
            TokenKind::Keyword(Keyword::Switch) => self.parse_switch_statement(completion),
            TokenKind::Keyword(Keyword::Try) => self.parse_try_statement(completion),
            TokenKind::Keyword(Keyword::With) => self.parse_with_statement(completion),
            TokenKind::Keyword(Keyword::Break) => self.parse_loop_jump_statement(false),
            TokenKind::Keyword(Keyword::Continue) => self.parse_loop_jump_statement(true),
            TokenKind::Keyword(Keyword::Function) => {
                self.parse_hoistable_function_declaration(position)
            }
            TokenKind::Identifier(_) if self.async_function_ahead() => {
                self.parse_hoistable_function_declaration(position)
            }
            TokenKind::Keyword(Keyword::Class) => {
                if position.allows_other_declaration() {
                    self.parse_class_declaration()
                } else {
                    Err(self
                        .syntax_here("class declarations can't appear in single-statement context"))
                }
            }
            TokenKind::Keyword(Keyword::Var) => self.parse_var_statement(),
            TokenKind::Keyword(Keyword::Return) => {
                if matches!(
                    self.current_ir().kind,
                    FunctionKind::Script | FunctionKind::Module | FunctionKind::Eval(_)
                ) {
                    Err(self.syntax_here("return not in a function"))
                } else {
                    self.parse_return_statement()
                }
            }
            TokenKind::Keyword(Keyword::Throw) => self.parse_throw_statement(),
            TokenKind::Keyword(Keyword::Debugger) => self.parse_debugger_statement(),
            TokenKind::Keyword(keyword @ (Keyword::Enum | Keyword::Export | Keyword::Extends)) => {
                Err(self.syntax_here(format!("unsupported keyword: {}", keyword.as_str())))
            }
            _ => self.parse_expression_statement(completion),
        }
    }

    /// QuickJS 2026-06-04 `js_parse_statement_or_decl(TOK_DEBUGGER)` has no
    /// debugger hook: it advances past the keyword, applies ordinary ASI, and
    /// emits no bytecode. In particular, an eval root keeps the last non-empty
    /// statement completion instead of replacing it with `undefined`.
    fn parse_debugger_statement(&mut self) -> Result<(), Error> {
        self.advance()?;
        self.consume_statement_terminator()
    }

    fn parse_hoistable_function_declaration(
        &mut self,
        position: StatementPosition,
    ) -> Result<(), Error> {
        if matches!(self.current_ir().kind, FunctionKind::Script)
            && position == StatementPosition::ProgramBody
        {
            self.parse_program_function_declaration()
        } else if matches!(self.current_ir().kind, FunctionKind::Module)
            && position == StatementPosition::ProgramBody
        {
            self.parse_module_function_declaration(ModuleDeclarationExport::None)
        } else if matches!(self.current_ir().kind, FunctionKind::Eval(_))
            && position == StatementPosition::ProgramBody
        {
            self.parse_eval_program_function_declaration()
        } else if matches!(
            self.current_ir().kind,
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
        ) && position == StatementPosition::FunctionBody
        {
            self.parse_function_body_declaration()
        } else if matches!(
            position,
            StatementPosition::NestedList
                | StatementPosition::AnnexBIfArm
                | StatementPosition::AnnexBLabelBody
        ) {
            self.parse_annex_b_function_declaration()
        } else {
            Err(self.syntax_here("function declarations can't appear in single-statement context"))
        }
    }

    fn parse_labeled_statement(
        &mut self,
        completion: StatementCompletion,
        label_name: String,
        position: StatementPosition,
    ) -> Result<(), Error> {
        if self
            .current_ir()
            .break_controls
            .iter()
            .any(|control| control.label_name.as_deref() == Some(label_name.as_str()))
        {
            return Err(self.syntax_here("duplicate label name"));
        }

        self.advance()?;
        self.expect_punctuator(Punctuator::Colon)?;
        match self.current().kind {
            // QuickJS passes a directly attached label into an iteration
            // statement's BlockEnv. A second label first becomes a regular
            // labeled statement, preserving the pinned release's current
            // multiple-label continue behavior.
            TokenKind::Keyword(Keyword::While) => {
                self.parse_while_statement(completion, Some(label_name))
            }
            TokenKind::Keyword(Keyword::Do) => {
                self.parse_do_while_statement(completion, Some(label_name))
            }
            TokenKind::Keyword(Keyword::For) => {
                self.parse_for_statement(completion, Some(label_name))
            }
            _ => {
                let entry_depth = self.current_ir().stack_depth;
                self.push_break_control(
                    BreakControlKind::RegularStatement,
                    Some(label_name),
                    entry_depth,
                    0,
                );
                let body_position =
                    if !self.current_ir().strict && position.allows_labelled_annex_b() {
                        StatementPosition::AnnexBLabelBody
                    } else {
                        StatementPosition::Single
                    };
                self.parse_statement_or_decl(completion, body_position)?;
                self.require_stack_depth(entry_depth, "labeled statement")?;

                let break_target = self.current_ir().ops.len();
                let control = self.pop_break_control()?;
                if !control.continue_jumps.is_empty() {
                    return Err(Error::internal(
                        "regular labeled statement received a continue jump",
                    ));
                }
                for jump in control.break_jumps {
                    self.patch_jump(jump, break_target)?;
                }
                self.finish_control_statement();
                Ok(())
            }
        }
    }

    fn parse_block_statement(&mut self, completion: StatementCompletion) -> Result<(), Error> {
        self.advance()?;
        if self.is_punctuator(Punctuator::RightBrace) {
            return self.advance();
        }
        let scope = self.push_scope(ScopeKind::Block);
        while !self.is_punctuator(Punctuator::RightBrace) {
            self.parse_statement_or_decl(completion, StatementPosition::NestedList)?;
        }
        self.advance()?;
        self.pop_scope(scope)
    }

    /// QuickJS `js_parse_statement_or_decl(TOK_WITH)`: the object expression
    /// is evaluated outside the new scope, then its `ToObject` result is stored
    /// in one unspellable local owned by that scope.  Keeping the binding typed
    /// is what lets publication reject forged dynamic-environment operands.
    fn parse_with_statement(&mut self, completion: StatementCompletion) -> Result<(), Error> {
        let with_span = self.current().span;
        if self.current_ir().strict {
            return Err(Error::syntax(
                "invalid keyword: with",
                source_span(with_span),
            ));
        }
        self.advance()?;
        self.expect_punctuator(Punctuator::LeftParen)?;
        self.parse_expression()?;
        self.expect_punctuator(Punctuator::RightParen)?;

        let scope = self.push_scope(ScopeKind::With);
        let local = {
            let function = self.current_ir_mut();
            if function.locals.len() >= MAX_LOCAL_VARIABLES {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(with_span)),
                );
            }
            let local = u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
            function.locals.push(WITH_OBJECT_LOCAL_NAME.to_owned());
            function.add_binding(
                scope,
                scope,
                WITH_OBJECT_LOCAL_NAME.to_owned(),
                BindingStorage::Local(local),
                BindingKind::WithObject,
                None,
            );
            local
        };
        self.emit_instruction(Instruction::ToObject)?;
        self.emit_instruction(Instruction::InitializeLocal(local))?;
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        self.parse_statement_or_decl(completion, StatementPosition::Single)?;
        self.pop_scope(scope)
    }

    fn parse_if_statement(&mut self, completion: StatementCompletion) -> Result<(), Error> {
        self.advance()?;
        let scope = self.push_scope(ScopeKind::If);
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        self.expect_punctuator(Punctuator::LeftParen)?;
        self.parse_expression()?;
        self.expect_punctuator(Punctuator::RightParen)?;

        let false_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        let branch_stack = self.current_ir().stack_depth;
        let branch_position = if self.current_ir().strict {
            StatementPosition::Single
        } else {
            StatementPosition::AnnexBIfArm
        };
        self.parse_statement_or_decl(completion, branch_position)?;
        let joined_stack = self.current_ir().stack_depth;

        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Else)) {
            let end_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
            self.advance()?;
            self.patch_jump(false_jump, self.current_ir().ops.len())?;
            self.current_ir_mut().stack_depth = branch_stack;
            self.parse_statement_or_decl(completion, branch_position)?;
            if self.current_ir().stack_depth != joined_stack {
                return Err(Error::internal("if branches have unequal stack depth"));
            }
            self.patch_jump(end_jump, self.current_ir().ops.len())?;
        } else {
            if joined_stack != branch_stack {
                return Err(Error::internal(
                    "if statement changed the fallthrough stack depth",
                ));
            }
            self.patch_jump(false_jump, self.current_ir().ops.len())?;
        }
        self.current_ir_mut().last_member_reference = None;
        self.current_ir_mut().last_identifier_reference = None;
        self.current_ir_mut().last_optional_chain = None;
        self.anonymous_function_definition = None;
        self.pop_scope(scope)?;
        Ok(())
    }

    fn parse_while_statement(
        &mut self,
        completion: StatementCompletion,
        label_name: Option<String>,
    ) -> Result<(), Error> {
        let entry_depth = self.current_ir().stack_depth;
        self.push_loop_control(entry_depth, label_name);
        self.advance()?;
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }

        let condition_target = self.current_ir().ops.len();
        self.expect_punctuator(Punctuator::LeftParen)?;
        self.parse_expression()?;
        self.expect_punctuator(Punctuator::RightParen)?;
        let false_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        self.require_stack_depth(entry_depth, "while condition")?;

        self.parse_statement_or_decl(completion, StatementPosition::Single)?;
        self.require_stack_depth(entry_depth, "while body")?;
        self.emit_instruction(Instruction::Goto(
            u32::try_from(condition_target)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
        ))?;

        let break_target = self.current_ir().ops.len();
        let control = self.pop_break_control()?;
        self.patch_jump(false_jump, break_target)?;
        for jump in control.continue_jumps {
            self.patch_jump(jump, condition_target)?;
        }
        for jump in control.break_jumps {
            self.patch_jump(jump, break_target)?;
        }
        self.finish_control_statement();
        Ok(())
    }

    fn parse_do_while_statement(
        &mut self,
        completion: StatementCompletion,
        label_name: Option<String>,
    ) -> Result<(), Error> {
        let entry_depth = self.current_ir().stack_depth;
        self.push_loop_control(entry_depth, label_name);
        self.advance()?;

        // QuickJS targets the reset itself, so every entered iteration starts
        // with an undefined eval completion. A continue instead targets the
        // condition below and does not repeat this reset prematurely.
        let body_target = self.current_ir().ops.len();
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        self.parse_statement_or_decl(completion, StatementPosition::Single)?;
        self.require_stack_depth(entry_depth, "do-while body")?;

        let condition_target = self.current_ir().ops.len();
        if !matches!(self.current().kind, TokenKind::Keyword(Keyword::While)) {
            // `js_parse_expect(TOK_WHILE)` formats the non-ASCII token through
            // `%c`; preserve the pinned release's observable replacement-char
            // diagnostic, including its missing closing quote.
            return Err(self.syntax_here("expecting '�"));
        }
        self.advance()?;
        self.expect_punctuator(Punctuator::LeftParen)?;
        self.parse_expression()?;
        self.expect_punctuator(Punctuator::RightParen)?;
        // Unlike an ordinary statement terminator, the trailing semicolon is
        // unconditionally optional, even before a same-line expression.
        self.consume_punctuator(Punctuator::Semicolon)?;
        self.emit_instruction(Instruction::IfTrue(
            u32::try_from(body_target)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
        ))?;
        self.require_stack_depth(entry_depth, "do-while condition")?;

        let break_target = self.current_ir().ops.len();
        let control = self.pop_break_control()?;
        for jump in control.continue_jumps {
            self.patch_jump(jump, condition_target)?;
        }
        for jump in control.break_jumps {
            self.patch_jump(jump, break_target)?;
        }
        self.finish_control_statement();
        Ok(())
    }

    fn parse_for_statement(
        &mut self,
        completion: StatementCompletion,
        label_name: Option<String>,
    ) -> Result<(), Error> {
        let entry_depth = self.current_ir().stack_depth;
        self.advance()?;
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        // QuickJS recognizes this contextual form only when the lexer has
        // classified `await` as the real keyword. In an ordinary function or
        // script the same source spelling remains an Identifier, so the
        // ordinary `for (` expectation reports the syntax error instead.
        let is_for_await = matches!(self.current().kind, TokenKind::Keyword(Keyword::Await));
        if is_for_await {
            if !matches!(
                self.current_ir().execution_kind,
                BytecodeFunctionKind::Async | BytecodeFunctionKind::AsyncGenerator
            ) {
                return Err(self.syntax_here("for await is only valid in asynchronous functions"));
            }
            self.advance()?;
        }
        // `for await` always enters the for-in/of parser. A semicolon or `in`
        // is rejected there rather than being reinterpreted as a classic for
        // head.
        let classic_head = !is_for_await && self.for_head_has_top_level_semicolon();
        self.expect_punctuator(Punctuator::LeftParen)?;
        let outer_scope = self.current_ir().current_scope;
        let scope = self.push_scope(ScopeKind::For);

        if !classic_head {
            return self.parse_for_in_of_statement(
                completion,
                label_name,
                entry_depth,
                outer_scope,
                scope,
                is_for_await,
            );
        }

        // QuickJS parses the classic initializer with PF_IN_ACCEPTED clear.
        // Keep that mode explicit even while the AllowIn operator itself
        // remains a later runtime slice.
        if !self.is_punctuator(Punctuator::Semicolon) {
            if self.lexical_declaration_ahead(true)? {
                self.parse_lexical_declarations_with_in(InMode::Disallow)?;
            } else if matches!(self.current().kind, TokenKind::Keyword(Keyword::Var)) {
                self.advance()?;
                self.parse_var_declarations_with_in(InMode::Disallow)?;
            } else {
                self.parse_expression_no_in()?;
                self.emit_instruction(Instruction::Drop)?;
            }
            self.require_stack_depth(entry_depth, "for initializer")?;
            // Detach any initializer capture before the first test while the
            // initialized value remains in the local slot for the iteration.
            self.emit_scope_closures(scope, outer_scope)?;
        }
        self.expect_punctuator(Punctuator::Semicolon)?;

        self.push_loop_control(entry_depth, label_name);
        let test_target = if self.is_punctuator(Punctuator::Semicolon) {
            None
        } else {
            let target = self.current_ir().ops.len();
            self.parse_expression()?;
            let false_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
            self.require_stack_depth(entry_depth, "for test")?;
            Some((target, false_jump))
        };
        self.expect_punctuator(Punctuator::Semicolon)?;

        let mut body_skip = None;
        let mut moved_update = None;
        let unmoved_continue_target = if self.is_punctuator(Punctuator::RightParen) {
            test_target.map(|(target, _)| target)
        } else {
            body_skip = Some(self.emit_instruction(Instruction::Goto(u32::MAX))?);
            let update_start = self.current_ir().ops.len();
            self.parse_expression()?;
            self.emit_instruction(Instruction::Drop)?;
            self.require_stack_depth(entry_depth, "for update")?;
            if let Some((test_target, _)) = test_target {
                self.emit_instruction(Instruction::Goto(
                    u32::try_from(test_target)
                        .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
                ))?;
            }
            if test_target.is_some() {
                // QuickJS's OPTIMIZE path moves the complete update chunk
                // after the body. Preserve empty Nop slots at its source
                // position and relocate only fragment-internal targets; the
                // backedge to the earlier test remains external.
                let update_end = self.current_ir().ops.len();
                let fragment = self.current_ir_mut().ops.split_off(update_start);
                for _ in 0..fragment.len() {
                    self.emit_instruction(Instruction::Nop)?;
                }
                moved_update = Some((update_start, update_end, fragment));
                None
            } else {
                Some(update_start)
            }
        };
        self.expect_punctuator(Punctuator::RightParen)?;

        let body_target = self.current_ir().ops.len();
        if let Some(body_skip) = body_skip {
            self.patch_jump(body_skip, body_target)?;
        }
        self.parse_statement_or_decl(completion, StatementPosition::Single)?;
        self.require_stack_depth(entry_depth, "for body")?;
        // Normal fallthrough closes the current head-binding cell before the
        // update creates the next iteration's cell. A `continue` targets the
        // update/test below and intentionally skips this close, preserving the
        // pinned release's observable `XXX: check continue` behavior.
        self.emit_scope_closures(scope, outer_scope)?;
        let continue_target = if let Some((old_start, old_end, mut fragment)) = moved_update {
            let target = self.current_ir().ops.len();
            relocate_ir_fragment(&mut fragment, old_start..old_end, target)?;
            self.current_ir_mut().ops.extend(fragment);
            target
        } else {
            let target = unmoved_continue_target.unwrap_or(body_target);
            self.emit_instruction(Instruction::Goto(
                u32::try_from(target)
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
            ))?;
            target
        };

        let break_target = self.current_ir().ops.len();
        let control = self.pop_break_control()?;
        if let Some((_, false_jump)) = test_target {
            self.patch_jump(false_jump, break_target)?;
        }
        for jump in control.continue_jumps {
            self.patch_jump(jump, continue_target)?;
        }
        for jump in control.break_jumps {
            self.patch_jump(jump, break_target)?;
        }
        self.finish_control_statement();
        self.pop_scope(scope)?;
        Ok(())
    }

    /// Lower QuickJS `js_parse_for_in_of`. The assignment fragment is emitted
    /// before the enumerated expression and skipped on first entry, just as
    /// upstream does; each `done == false` edge jumps back with the yielded
    /// value above the retained for-in object or three-slot iterator record.
    fn parse_for_in_of_statement(
        &mut self,
        completion: StatementCompletion,
        label_name: Option<String>,
        entry_depth: usize,
        outer_scope: ScopeId,
        scope: ScopeId,
        is_for_await: bool,
    ) -> Result<(), Error> {
        let iteration_hint = self
            .for_iteration_kind_ahead()
            .ok_or_else(|| self.syntax_here("expected 'of' or 'in' in for control expression"))?;
        if is_for_await && iteration_hint != ForIterationKind::Of {
            return Err(self.syntax_here("'for await' loop should be used with 'of'"));
        }
        let retained_slots = match iteration_hint {
            ForIterationKind::In => 1,
            ForIterationKind::Of => 3,
        };

        let expression_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let assignment_target = self.current_ir().ops.len();

        // ForInNext supplies `enum, value`; ForOfNext supplies the retained
        // three-slot iterator record plus `value` on this edge.
        self.current_ir_mut().stack_depth = entry_depth
            .checked_add(retained_slots + 1)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        if iteration_hint == ForIterationKind::Of {
            self.push_for_of_assignment_fragment_control(entry_depth)?;
        }
        let target = self.parse_for_iteration_assignment_target(iteration_hint, is_for_await)?;
        self.require_stack_depth(entry_depth + retained_slots, "for-in/of assignment target")?;
        if iteration_hint == ForIterationKind::Of {
            self.pop_for_of_assignment_fragment_control(entry_depth)?;
        }
        let body_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;

        let expression_target = self.current_ir().ops.len();
        self.patch_jump(expression_jump, expression_target)?;
        self.current_ir_mut().stack_depth = entry_depth;

        let has_initializer = if self.consume_punctuator(Punctuator::Equal)? {
            match iteration_hint {
                ForIterationKind::In => {
                    self.with_in_mode(InMode::Disallow, Self::parse_assignment)?;
                }
                ForIterationKind::Of => self.parse_assignment_allow_in()?,
            }
            if iteration_hint == ForIterationKind::In
                && target.declaration == ForAssignmentDeclaration::Var
                && !self.current_ir().strict
                && !target.is_destructuring
            {
                let initializer = target
                    .var_initializer
                    .as_ref()
                    .ok_or_else(|| Error::internal("for-in var initializer lost its binding"))?;
                self.emit_identifier_inherited(
                    initializer.name.clone(),
                    initializer.span,
                    initializer.scope,
                    IdentifierAccess::Put,
                )?;
            } else {
                self.emit_instruction(Instruction::Drop)?;
            }
            true
        } else {
            false
        };

        let iteration_kind = if self.is_for_of_keyword() {
            ForIterationKind::Of
        } else if matches!(self.current().kind, TokenKind::Keyword(Keyword::In)) {
            ForIterationKind::In
        } else {
            return Err(self.syntax_here("expected 'of' or 'in' in for control expression"));
        };
        if iteration_kind != iteration_hint {
            return Err(Error::internal("for-in/of delimiter probe drifted"));
        }
        if is_for_await && iteration_kind != ForIterationKind::Of {
            return Err(self.syntax_here("'for await' loop should be used with 'of'"));
        }
        if has_initializer
            && (iteration_kind == ForIterationKind::Of
                || target.declaration != ForAssignmentDeclaration::Var
                || self.current_ir().strict
                || target.is_destructuring)
        {
            return Err(self.syntax_here(format!(
                "a declaration in the head of a for-{} loop can't have an initializer",
                if iteration_kind == ForIterationKind::Of {
                    "of"
                } else {
                    "in"
                }
            )));
        }

        // After contextual `of`, a slash starts the right-hand side's RegExp
        // lexical goal. The literal itself remains an explicit frontier, but
        // it must not drift into the division-token diagnostic.
        self.advance_expression_start()?;
        if iteration_kind == ForIterationKind::Of {
            // For-of consumes exactly one AssignmentExpression.
            self.parse_assignment_allow_in()?;
        } else {
            // QuickJS deliberately accepts a full comma Expression for-in.
            self.parse_expression()?;
        }
        self.emit_scope_closures(scope, outer_scope)?;
        self.emit_instruction(match (iteration_kind, is_for_await) {
            (ForIterationKind::In, false) => Instruction::ForInStart,
            (ForIterationKind::Of, false) => Instruction::ForOfStart,
            (ForIterationKind::Of, true) => Instruction::ForAwaitOfStart,
            (ForIterationKind::In, true) => {
                return Err(Error::internal("for-await retained a for-in iterator"));
            }
        })?;
        let record_depth = entry_depth + retained_slots;
        self.require_stack_depth(record_depth, "for-in/of iterator start")?;
        let next_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        self.expect_punctuator(Punctuator::RightParen)?;

        let body_target = self.current_ir().ops.len();
        self.patch_jump(body_jump, body_target)?;
        self.current_ir_mut().stack_depth = record_depth;
        match iteration_kind {
            ForIterationKind::In => {
                self.push_for_in_control(entry_depth, label_name, outer_scope)?;
            }
            ForIterationKind::Of => {
                self.push_for_of_control(entry_depth, label_name, outer_scope)?;
            }
        }
        self.parse_statement_or_decl(completion, StatementPosition::Single)?;
        self.require_stack_depth(record_depth, "for-in/of body")?;
        self.emit_scope_closures(scope, outer_scope)?;

        let next_target = self.current_ir().ops.len();
        self.patch_jump(next_jump, next_target)?;
        match (iteration_kind, is_for_await) {
            (ForIterationKind::In, false) => {
                self.emit_instruction(Instruction::ForInNext)?;
            }
            (ForIterationKind::Of, false) => {
                self.emit_instruction(Instruction::ForOfNext(0))?;
            }
            (ForIterationKind::Of, true) => {
                self.emit_instruction(Instruction::ForAwaitOfNext)?;
                self.emit_instruction(Instruction::Await)?;
                self.emit_instruction(Instruction::IteratorGetValueDone)?;
            }
            (ForIterationKind::In, true) => {
                return Err(Error::internal("for-await advanced a for-in iterator"));
            }
        }
        let assignment_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        self.patch_jump(assignment_jump, assignment_target)?;

        // A completed enumeration contributes undefined above its retained
        // record. Natural exhaustion removes it and shares the break tail.
        self.emit_instruction(Instruction::Drop)?;
        let break_target = self.current_ir().ops.len();
        match iteration_kind {
            ForIterationKind::In => {
                self.emit_instruction(Instruction::Drop)?;
            }
            ForIterationKind::Of => {
                self.emit_instruction(Instruction::IteratorClose)?;
            }
        }
        self.require_stack_depth(entry_depth, "for-in/of close")?;

        let control = self.pop_break_control()?;
        let expected_control = match iteration_kind {
            ForIterationKind::In => (BreakControlKind::ForIn, 1),
            ForIterationKind::Of => (BreakControlKind::ForOf, 3),
        };
        if (control.kind, control.drop_count) != expected_control {
            return Err(Error::internal("for-in/of control stack is unbalanced"));
        }
        for jump in control.continue_jumps {
            self.patch_jump(jump, next_target)?;
        }
        for jump in control.break_jumps {
            self.patch_jump(jump, break_target)?;
        }
        self.finish_control_statement();
        self.pop_scope(scope)?;
        Ok(())
    }

    /// Parse and consume the yielded value for one supported for-in/of head.
    /// Declarations bind in the enumeration scope; ordinary references keep
    /// their base/key evaluation in the per-iteration assignment fragment.
    fn parse_for_iteration_assignment_target(
        &mut self,
        iteration_kind: ForIterationKind,
        is_for_await: bool,
    ) -> Result<ForAssignmentTargetInfo, Error> {
        if self.for_array_assignment_pattern_ahead(iteration_kind) {
            return self.parse_for_array_assignment_pattern(iteration_kind);
        }
        if self.for_object_assignment_pattern_ahead(iteration_kind) {
            return self.parse_for_object_assignment_pattern(iteration_kind);
        }

        if self.lexical_declaration_ahead(true)? {
            let is_const = matches!(self.current().kind, TokenKind::Keyword(Keyword::Const));
            self.advance()?;
            if self.is_punctuator(Punctuator::LeftBracket) {
                return self.parse_for_array_binding_pattern(
                    iteration_kind,
                    ForAssignmentDeclaration::Lexical,
                    is_const,
                );
            }
            if matches!(
                self.current().kind,
                TokenKind::Punctuator(Punctuator::LeftBrace)
            ) {
                return self.parse_for_object_binding_pattern(
                    iteration_kind,
                    ForAssignmentDeclaration::Lexical,
                    is_const,
                );
            }
            let token = self.current().clone();
            let TokenKind::Identifier(identifier) = token.kind else {
                return Err(self.syntax_here("variable name expected"));
            };
            validate_identifier_reservation(
                &identifier,
                token.span,
                self.current_ir().strict,
                IdentifierContext::Variable,
            )?;
            if identifier.value == "let" {
                return Err(Error::syntax(
                    "'let' is not a valid lexical identifier",
                    source_span(token.span),
                ));
            }
            let name = identifier.value;
            let strict = self.current_ir().strict;
            self.advance()?;
            if strict && matches!(name.as_str(), "eval" | "arguments") {
                return Err(Error::syntax(
                    "invalid variable name in strict mode",
                    source_span(self.current().span),
                ));
            }
            self.register_lexical_binding(&name, token.span, self.current().span, is_const, false)?;
            self.emit_identifier_at(
                name,
                token.span,
                IdentifierAccess::Initialize,
                source_offset(token.span)?,
            )?;
            return Ok(ForAssignmentTargetInfo {
                declaration: ForAssignmentDeclaration::Lexical,
                var_initializer: None,
                is_destructuring: false,
            });
        }

        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Var)) {
            self.advance()?;
            if self.is_punctuator(Punctuator::LeftBracket) {
                return self.parse_for_array_binding_pattern(
                    iteration_kind,
                    ForAssignmentDeclaration::Var,
                    false,
                );
            }
            if matches!(
                self.current().kind,
                TokenKind::Punctuator(Punctuator::LeftBrace)
            ) {
                return self.parse_for_object_binding_pattern(
                    iteration_kind,
                    ForAssignmentDeclaration::Var,
                    false,
                );
            }
            let token = self.current().clone();
            let TokenKind::Identifier(identifier) = token.kind else {
                return Err(self.syntax_here("variable name expected"));
            };
            validate_identifier_reservation(
                &identifier,
                token.span,
                self.current_ir().strict,
                IdentifierContext::Variable,
            )?;
            let strict = self.current_ir().strict;
            let name = identifier.value;
            self.advance()?;
            if strict && matches!(name.as_str(), "eval" | "arguments") {
                return Err(Error::syntax(
                    "invalid variable name in strict mode",
                    source_span(self.current().span),
                ));
            }
            self.register_var_binding(&name, token.span, self.current().span)?;
            let initializer = IdentifierReference {
                name: name.clone(),
                span: token.span,
                scope: self.current_ir().current_scope,
                object_environment: false,
            };
            self.emit_identifier_at(
                name,
                token.span,
                IdentifierAccess::Put,
                source_offset(token.span)?,
            )?;
            return Ok(ForAssignmentTargetInfo {
                declaration: ForAssignmentDeclaration::Var,
                var_initializer: Some(initializer),
                is_destructuring: false,
            });
        }

        let async_span = match &self.current().kind {
            TokenKind::Identifier(identifier)
                if identifier.value == "async" && !identifier.has_escape =>
            {
                Some(self.current().span)
            }
            _ => None,
        };
        // Mirror QuickJS's pre-LHS `token_is_pseudo_keyword(async)` plus
        // `peek_token(FALSE) == TOK_OF` ambiguity guard. Looking ahead from
        // the raw token boundary rejects only the bare `async of` pair;
        // `async.value` and `async[key]` proceed through ordinary member-LHS
        // parsing.
        if let Some(async_span) = async_span
            && !is_for_await
            && self.next_token_is_for_of_keyword()
        {
            return Err(Error::syntax(
                "'for of' expression cannot start with 'async'",
                source_span(async_span),
            ));
        }
        self.parse_left_hand_side_expression()?;
        if self.current_ir().last_optional_chain.is_some() {
            return Err(self.syntax_here("invalid for in/of left hand-side"));
        }
        if let Some(target) = self.take_tail_identifier_reference()? {
            self.validate_identifier_assignment_target(&target)?;
            if target.object_environment {
                let function = self.current_ir_mut();
                let Some(SpannedIrOp {
                    op:
                        IrOp::IdentifierReference {
                            access: IdentifierReferenceAccess::Prepare,
                            ..
                        },
                    ..
                }) = function.ops.pop()
                else {
                    return Err(Error::internal(
                        "for-in/of identifier target lost its prepared Reference",
                    ));
                };
                function.stack_depth = function.stack_depth.checked_sub(1).ok_or_else(|| {
                    Error::internal("for-in/of identifier Reference underflowed the stack")
                })?;
            }
            self.emit_identifier_inherited(
                target.name,
                target.span,
                target.scope,
                IdentifierAccess::Put,
            )?;
            return Ok(ForAssignmentTargetInfo {
                declaration: ForAssignmentDeclaration::Assignment,
                var_initializer: None,
                is_destructuring: false,
            });
        }
        let Some(target) = self.take_tail_member_reference()? else {
            return Err(self.syntax_here("invalid for in/of left hand-side"));
        };
        self.emit_for_of_member_put(target)?;
        Ok(ForAssignmentTargetInfo {
            declaration: ForAssignmentDeclaration::Assignment,
            var_initializer: None,
            is_destructuring: false,
        })
    }

    /// Reorder `value, base[, key]` into the ordinary property-write layout
    /// without introducing a forgeable temporary. `Insert2; Drop` is the
    /// existing typed bytecode's two-value swap; `Perm3` first rotates the
    /// computed form into position.
    fn emit_for_of_member_put(&mut self, target: MemberReference) -> Result<(), Error> {
        match target {
            MemberReference::Field { key, site } => {
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction_at(Instruction::PutField(key), site)?;
            }
            MemberReference::Computed { site } => {
                self.emit_instruction(Instruction::Perm3)?;
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction_at(Instruction::PutArrayEl, site)?;
            }
            MemberReference::Super { site } => {
                self.emit_instruction(Instruction::Rot4Left)?;
                self.emit_instruction_at(Instruction::PutSuperValue, site)?;
            }
            MemberReference::Private {
                name,
                span,
                scope,
                site,
            } => {
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_private_field_operation(
                    name,
                    span,
                    scope,
                    PrivateFieldAccess::Put,
                    site,
                )?;
            }
        }
        Ok(())
    }

    /// Lower the pinned QuickJS switch layout while keeping the discriminant
    /// on the operand stack through every case body. A new case test is placed
    /// behind the previous body's fallthrough jump; all consecutive matching
    /// clauses join the same body. The final failed test is patched either to
    /// the recorded default body or to the shared break/drop tail.
    fn parse_switch_statement(&mut self, completion: StatementCompletion) -> Result<(), Error> {
        let outer_depth = self.current_ir().stack_depth;
        self.advance()?;
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        self.expect_punctuator(Punctuator::LeftParen)?;
        self.parse_expression()?;
        self.expect_punctuator(Punctuator::RightParen)?;

        let switch_depth = outer_depth
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        self.require_stack_depth(switch_depth, "switch discriminant")?;
        let scope = self.push_scope(ScopeKind::Switch);
        self.push_break_control(BreakControlKind::Switch, None, switch_depth, 1);
        self.expect_punctuator(Punctuator::LeftBrace)?;

        let mut pending_no_match = None;
        let mut default_target = None;
        while !self.is_punctuator(Punctuator::RightBrace) {
            match self.current().kind {
                TokenKind::Keyword(Keyword::Case) => {
                    let previous_no_match = pending_no_match.take();
                    let fallthrough_jump = if previous_no_match.is_some() {
                        Some(self.emit_instruction(Instruction::Goto(u32::MAX))?)
                    } else {
                        None
                    };
                    let test_target = self.current_ir().ops.len();
                    if let Some(previous_no_match) = previous_no_match {
                        self.patch_jump(previous_no_match, test_target)?;
                    }

                    let mut matched_jumps = Vec::new();
                    loop {
                        self.advance()?;
                        self.emit_instruction(Instruction::Dup)?;
                        self.parse_expression()?;
                        self.expect_punctuator(Punctuator::Colon)?;
                        self.emit_instruction(Instruction::StrictEq)?;

                        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Case)) {
                            matched_jumps
                                .push(self.emit_instruction(Instruction::IfTrue(u32::MAX))?);
                        } else {
                            pending_no_match =
                                Some(self.emit_instruction(Instruction::IfFalse(u32::MAX))?);
                            let body_target = self.current_ir().ops.len();
                            if let Some(fallthrough_jump) = fallthrough_jump {
                                self.patch_jump(fallthrough_jump, body_target)?;
                            }
                            for matched_jump in matched_jumps {
                                self.patch_jump(matched_jump, body_target)?;
                            }
                            self.require_stack_depth(switch_depth, "switch case tests")?;
                            break;
                        }
                    }
                }
                TokenKind::Keyword(Keyword::Default) => {
                    self.advance()?;
                    self.expect_punctuator(Punctuator::Colon)?;
                    if default_target.is_some() {
                        return Err(self.syntax_here("duplicate default"));
                    }
                    if pending_no_match.is_none() {
                        pending_no_match =
                            Some(self.emit_instruction(Instruction::Goto(u32::MAX))?);
                    }
                    default_target = Some(self.current_ir().ops.len());
                }
                _ => {
                    if pending_no_match.is_none() {
                        return Err(self.syntax_here("invalid switch statement"));
                    }
                    self.parse_statement_or_decl(completion, StatementPosition::NestedList)?;
                    self.require_stack_depth(switch_depth, "switch case body")?;
                }
            }
        }
        self.advance()?;

        let no_match_target = default_target.unwrap_or(self.current_ir().ops.len());
        if let Some(pending_no_match) = pending_no_match {
            self.patch_jump(pending_no_match, no_match_target)?;
        }
        let break_target = self.current_ir().ops.len();
        let control = self.pop_break_control()?;
        if !control.continue_jumps.is_empty() {
            return Err(Error::internal("switch received a continue jump"));
        }
        for jump in control.break_jumps {
            self.patch_jump(jump, break_target)?;
        }
        self.emit_instruction(Instruction::Drop)?;
        self.require_stack_depth(outer_depth, "switch tail")?;
        self.finish_control_statement();
        self.pop_scope(scope)?;
        Ok(())
    }

    /// Lower TryStatement with the same catch-marker/finally-subroutine shape
    /// as the pinned QuickJS release. Even a catch-only statement owns an
    /// empty `Ret` subroutine: abrupt break/continue/return code can therefore
    /// be emitted while the parser is still unaware whether a source finally
    /// clause follows.
    fn parse_try_statement(&mut self, completion: StatementCompletion) -> Result<(), Error> {
        let entry_depth = self.current_ir().stack_depth;
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        self.advance()?;

        if !self.is_punctuator(Punctuator::LeftBrace) {
            return Err(self.syntax_here("expecting '{'"));
        }

        let catch_jump = self.emit_instruction(Instruction::Catch(u32::MAX))?;
        self.push_break_control(BreakControlKind::TryFinally, None, entry_depth + 1, 1);
        self.parse_block_statement(completion)?;
        self.require_stack_depth(entry_depth + 1, "try block")?;
        let try_control = self.pop_break_control()?;
        if try_control.kind != BreakControlKind::TryFinally {
            return Err(Error::internal("try block lost its finally control"));
        }
        let mut finally_gosubs = try_control.finally_gosubs;

        self.emit_instruction(Instruction::DropCatch)?;
        self.emit_instruction(Instruction::Undefined)?;
        finally_gosubs.push(self.emit_instruction(Instruction::Gosub(u32::MAX))?);
        self.emit_instruction(Instruction::Drop)?;
        let mut end_jumps = vec![self.emit_instruction(Instruction::Goto(u32::MAX))?];

        // A catch target receives the thrown value where the catch marker had
        // lived. Restore that exceptional stack shape explicitly before
        // parsing the handler's otherwise-linear IR.
        self.current_ir_mut().stack_depth = entry_depth + 1;

        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Catch)) {
            self.advance()?;
            let catch_scope = self.push_scope(ScopeKind::Catch);
            self.current_ir_mut().ops.push(SpannedIrOp {
                op: IrOp::PrepareCatchScope(catch_scope),
                pc_site: None,
            });
            // QuickJS emits the Catch-scope EnterScope before the exceptional
            // handler label. The jump therefore skips TDZ initialization while
            // still using that scope's statically allocated bindings. Oxide's
            // typed preparation below restores QuickJS's default-undefined
            // frame state before any pattern initializer can read a later name.
            let catch_target = self.current_ir().ops.len() - 1;
            self.patch_jump(catch_jump, catch_target)?;

            if self.is_punctuator(Punctuator::LeftBrace) {
                // Optional catch binding: discard the exception before the
                // catch body installs its own protection marker.
                self.emit_instruction(Instruction::Drop)?;
            } else {
                self.expect_punctuator(Punctuator::LeftParen)?;
                if self.is_punctuator(Punctuator::LeftBrace) {
                    self.parse_catch_object_binding_pattern()?;
                } else if self.is_punctuator(Punctuator::LeftBracket) {
                    self.parse_catch_array_binding_pattern()?;
                } else {
                    let token = self.current().clone();
                    let TokenKind::Identifier(identifier) = token.kind else {
                        return Err(self.syntax_here("identifier expected"));
                    };
                    if identifier.escaped_reserved_word {
                        return Err(self.syntax_here("identifier expected"));
                    }
                    validate_identifier_reservation(
                        &identifier,
                        token.span,
                        self.current_ir().strict,
                        IdentifierContext::Variable,
                    )?;
                    let invalid_strict_name = self.current_ir().strict
                        && matches!(identifier.value.as_str(), "eval" | "arguments");
                    let name = identifier.value;
                    self.advance()?;
                    if invalid_strict_name {
                        return Err(Error::syntax(
                            "invalid variable name in strict mode",
                            source_span(self.current().span),
                        ));
                    }
                    self.register_lexical_binding(
                        &name,
                        token.span,
                        self.current().span,
                        false,
                        false,
                    )?;
                    let catch_binding =
                        self.current_ir()
                            .binding_id_in_scope(catch_scope, &name)
                            .ok_or_else(|| Error::internal("catch binding was not registered"))?;
                    self.current_ir_mut().bindings[catch_binding.0].is_catch_parameter = true;
                    self.emit_identifier(name, token.span, IdentifierAccess::Initialize)?;
                }
                self.expect_punctuator(Punctuator::RightParen)?;
            }

            let catch2_jump = self.emit_instruction(Instruction::Catch(u32::MAX))?;
            self.expect_punctuator(Punctuator::LeftBrace)?;
            let catch_body_scope = if self.is_punctuator(Punctuator::RightBrace) {
                None
            } else {
                Some(self.push_scope(ScopeKind::Block))
            };
            self.push_break_control(BreakControlKind::TryFinally, None, entry_depth + 1, 1);
            while !self.is_punctuator(Punctuator::RightBrace) {
                if self.at_eof() {
                    return Err(self.syntax_here("unterminated catch block"));
                }
                self.parse_statement_or_decl(completion, StatementPosition::NestedList)?;
            }
            self.advance()?;
            self.require_stack_depth(entry_depth + 1, "catch block")?;
            let catch_control = self.pop_break_control()?;
            if catch_control.kind != BreakControlKind::TryFinally {
                return Err(Error::internal("catch block lost its finally control"));
            }
            finally_gosubs.extend(catch_control.finally_gosubs);
            if let Some(catch_body_scope) = catch_body_scope {
                self.pop_scope(catch_body_scope)?;
            }
            self.pop_scope(catch_scope)?;

            self.emit_instruction(Instruction::DropCatch)?;
            self.emit_instruction(Instruction::Undefined)?;
            finally_gosubs.push(self.emit_instruction(Instruction::Gosub(u32::MAX))?);
            self.emit_instruction(Instruction::Drop)?;
            end_jumps.push(self.emit_instruction(Instruction::Goto(u32::MAX))?);

            // A throw from the catch body bypasses its normal LeaveScope. This
            // deliberately preserves QuickJS's captured catch-cell lifetime
            // quirk rather than synthesizing exception-path CloseLocal ops.
            let catch2_target = self.current_ir().ops.len();
            self.patch_jump(catch2_jump, catch2_target)?;
            self.current_ir_mut().stack_depth = entry_depth + 1;
            finally_gosubs.push(self.emit_instruction(Instruction::Gosub(u32::MAX))?);
            self.emit_instruction(Instruction::Throw)?;
        } else if matches!(self.current().kind, TokenKind::Keyword(Keyword::Finally)) {
            // A try-finally handler retains the exception as the pending value;
            // the subroutine returns to the following rethrow.
            let catch_target = self.current_ir().ops.len();
            self.patch_jump(catch_jump, catch_target)?;
            finally_gosubs.push(self.emit_instruction(Instruction::Gosub(u32::MAX))?);
            self.emit_instruction(Instruction::Throw)?;
        } else {
            return Err(self.syntax_here("expecting catch or finally"));
        }

        let finally_target = self.current_ir().ops.len();
        for gosub in finally_gosubs {
            self.patch_jump(gosub, finally_target)?;
        }

        // Every call enters with a pending value plus the Gosub return address.
        self.current_ir_mut().stack_depth = entry_depth + 2;
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Finally)) {
            self.advance()?;
            self.push_break_control(BreakControlKind::FinallyBody, None, entry_depth + 2, 2);

            let saved_eval_ret = if matches!(completion, StatementCompletion::Eval) {
                let eval_ret = self.eval_ret_local()?;
                let saved = self
                    .current_ir_mut()
                    .add_synthetic_local(SyntheticLocalKind::FinallySavedEvalCompletion)?;
                self.emit_instruction(Instruction::GetLocal(eval_ret))?;
                self.emit_instruction(Instruction::PutLocal(saved))?;
                self.set_eval_ret_undefined()?;
                Some(saved)
            } else {
                None
            };

            if !self.is_punctuator(Punctuator::LeftBrace) {
                return Err(self.syntax_here("expecting '{'"));
            }
            self.parse_block_statement(completion)?;
            if let Some(saved) = saved_eval_ret {
                self.emit_instruction(Instruction::GetLocal(saved))?;
                self.emit_instruction(Instruction::PutLocal(self.eval_ret_local()?))?;
            }
            self.require_stack_depth(entry_depth + 2, "finally block")?;
            let finally_control = self.pop_break_control()?;
            if finally_control.kind != BreakControlKind::FinallyBody
                || !finally_control.finally_gosubs.is_empty()
            {
                return Err(Error::internal("finally body control is malformed"));
            }
        }
        self.emit_instruction(Instruction::Ret)?;

        let end_target = self.current_ir().ops.len();
        for jump in end_jumps {
            self.patch_jump(jump, end_target)?;
        }
        self.current_ir_mut().stack_depth = entry_depth;
        self.finish_control_statement();
        Ok(())
    }

    fn parse_loop_jump_statement(&mut self, is_continue: bool) -> Result<(), Error> {
        self.advance()?;

        let label_name = if self.current().line_terminator_before {
            None
        } else if let TokenKind::Identifier(identifier) = &self.current().kind
            && !identifier.escaped_reserved_word
        {
            Some(identifier.value.clone())
        } else {
            None
        };
        let target = self
            .current_ir()
            .break_controls
            .iter()
            .rposition(|control| match label_name.as_deref() {
                Some(label_name) if is_continue => {
                    matches!(
                        control.kind,
                        BreakControlKind::Loop | BreakControlKind::ForIn | BreakControlKind::ForOf
                    ) && control.label_name.as_deref() == Some(label_name)
                }
                Some(label_name) => control.label_name.as_deref() == Some(label_name),
                None if is_continue => {
                    matches!(
                        control.kind,
                        BreakControlKind::Loop | BreakControlKind::ForIn | BreakControlKind::ForOf
                    )
                }
                None => matches!(
                    control.kind,
                    BreakControlKind::Loop
                        | BreakControlKind::ForIn
                        | BreakControlKind::ForOf
                        | BreakControlKind::Switch
                ),
            });
        let Some(target) = target else {
            return Err(self.syntax_here(if label_name.is_some() {
                "break/continue label not found"
            } else if is_continue {
                "continue must be inside loop"
            } else {
                "break must be inside loop or switch"
            }));
        };
        let source_depth = self.current_ir().stack_depth;
        let current_scope = self.current_ir().current_scope;
        let (target_scope, entry_depth, crossed_controls) = {
            let controls = &self.current_ir().break_controls;
            let target_control = &controls[target];
            let crossed_controls = controls[target + 1..]
                .iter()
                .enumerate()
                .rev()
                .map(|(offset, control)| {
                    (
                        target + 1 + offset,
                        control.kind,
                        control.scope,
                        control.drop_count,
                    )
                })
                .collect::<Vec<_>>();
            (
                target_control.scope,
                target_control.entry_depth,
                crossed_controls,
            )
        };
        let mut cleanup_scope = current_scope;
        for (control_index, control_kind, control_scope, drop_count) in crossed_controls {
            self.emit_scope_closures(cleanup_scope, control_scope)?;
            cleanup_scope = control_scope;
            match control_kind {
                BreakControlKind::TryFinally => {
                    if drop_count != 1 {
                        return Err(Error::internal(
                            "try/finally control has the wrong catch-marker depth",
                        ));
                    }
                    self.emit_instruction(Instruction::DropCatch)?;
                    self.emit_instruction(Instruction::Undefined)?;
                    let gosub = self.emit_instruction(Instruction::Gosub(u32::MAX))?;
                    self.current_ir_mut().break_controls[control_index]
                        .finally_gosubs
                        .push(gosub);
                    self.emit_instruction(Instruction::Drop)?;
                }
                BreakControlKind::FinallyBody => {
                    if drop_count != 2 {
                        return Err(Error::internal(
                            "finally-body control has the wrong cleanup depth",
                        ));
                    }
                    // The typed return-address value is at TOS and must never
                    // pass through the ordinary JavaScript-value Drop path.
                    self.emit_instruction(Instruction::DropGosub)?;
                    self.emit_instruction(Instruction::Drop)?;
                }
                BreakControlKind::ForOf => {
                    if drop_count != 3 {
                        return Err(Error::internal(
                            "for-of control has the wrong iterator-record depth",
                        ));
                    }
                    self.emit_instruction(Instruction::IteratorClose)?;
                }
                BreakControlKind::DestructuringIterator => {
                    if drop_count != 3 {
                        return Err(Error::internal(
                            "destructuring control has the wrong iterator-record depth",
                        ));
                    }
                    self.emit_instruction(Instruction::IteratorClose)?;
                }
                BreakControlKind::ForOfAssignmentFragment => {
                    return Err(Error::internal(
                        "break/continue crossed a for-of assignment fragment",
                    ));
                }
                BreakControlKind::RegularStatement
                | BreakControlKind::Loop
                | BreakControlKind::ForIn
                | BreakControlKind::Switch => {
                    for _ in 0..drop_count {
                        self.emit_instruction(Instruction::Drop)?;
                    }
                }
            }
        }
        self.emit_scope_closures(cleanup_scope, target_scope)?;
        self.require_stack_depth(entry_depth, "break/continue cleanup")?;
        let jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let control = self
            .current_ir_mut()
            .break_controls
            .get_mut(target)
            .ok_or_else(|| Error::internal("break control disappeared while emitting jump"))?;
        if is_continue {
            control.continue_jumps.push(jump);
        } else {
            control.break_jumps.push(jump);
        }
        if label_name.is_some() {
            self.advance()?;
        }
        self.consume_statement_terminator()?;
        // The emitted jump is terminal, but parsing continues linearly so a
        // later case body or ordinary unreachable statement must retain the
        // enclosing control's fallthrough stack shape.
        self.current_ir_mut().stack_depth = source_depth;
        Ok(())
    }

    fn push_loop_control(&mut self, entry_depth: usize, label_name: Option<String>) {
        self.push_break_control(BreakControlKind::Loop, label_name, entry_depth, 0);
    }

    /// QuickJS restores the outer scope level on the for-in break entry while
    /// retaining its one hidden enumeration object across local continue.
    fn push_for_in_control(
        &mut self,
        entry_depth: usize,
        label_name: Option<String>,
        outer_scope: ScopeId,
    ) -> Result<(), Error> {
        let record_depth = entry_depth
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        self.current_ir_mut()
            .break_controls
            .push(BreakControlContext {
                kind: BreakControlKind::ForIn,
                label_name,
                scope: outer_scope,
                entry_depth: record_depth,
                drop_count: 1,
                break_jumps: Vec::new(),
                continue_jumps: Vec::new(),
                finally_gosubs: Vec::new(),
            });
        Ok(())
    }

    /// QuickJS changes a for-of/for-await `BlockEnv`'s scope level back to the
    /// level outside the enumeration scope. Thus a same-loop break/continue
    /// closes the current lexical head cell while retaining the three-slot
    /// iterator record for the loop's shared next/close tail.
    fn push_for_of_control(
        &mut self,
        entry_depth: usize,
        label_name: Option<String>,
        outer_scope: ScopeId,
    ) -> Result<(), Error> {
        let record_depth = entry_depth
            .checked_add(3)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        self.current_ir_mut()
            .break_controls
            .push(BreakControlContext {
                kind: BreakControlKind::ForOf,
                label_name,
                scope: outer_scope,
                entry_depth: record_depth,
                drop_count: 3,
                break_jumps: Vec::new(),
                continue_jumps: Vec::new(),
                finally_gosubs: Vec::new(),
            });
        Ok(())
    }

    fn push_destructuring_iterator_control(&mut self) -> Result<(), Error> {
        let record_depth = self.current_ir().stack_depth;
        if record_depth < 3 {
            return Err(Error::internal(
                "destructuring iterator record is below the stack base",
            ));
        }
        self.push_break_control(
            BreakControlKind::DestructuringIterator,
            None,
            record_depth,
            3,
        );
        Ok(())
    }

    fn push_for_of_assignment_fragment_control(&mut self, entry_depth: usize) -> Result<(), Error> {
        let record_depth = entry_depth
            .checked_add(3)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        self.require_stack_depth(
            record_depth
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?,
            "for-of assignment fragment entry",
        )?;
        self.push_break_control(
            BreakControlKind::ForOfAssignmentFragment,
            None,
            record_depth,
            3,
        );
        Ok(())
    }

    fn pop_for_of_assignment_fragment_control(&mut self, entry_depth: usize) -> Result<(), Error> {
        let record_depth = entry_depth
            .checked_add(3)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        let control = self.pop_break_control()?;
        if control.kind != BreakControlKind::ForOfAssignmentFragment
            || control.label_name.is_some()
            || control.entry_depth != record_depth
            || control.drop_count != 3
            || !control.break_jumps.is_empty()
            || !control.continue_jumps.is_empty()
            || !control.finally_gosubs.is_empty()
        {
            return Err(Error::internal(
                "for-of assignment fragment control stack is unbalanced",
            ));
        }
        Ok(())
    }

    fn pop_destructuring_iterator_control(&mut self) -> Result<(), Error> {
        let control = self.pop_break_control()?;
        if control.kind != BreakControlKind::DestructuringIterator
            || control.label_name.is_some()
            || control.entry_depth
                != self
                    .current_ir()
                    .stack_depth
                    .checked_add(3)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?
            || control.drop_count != 3
            || !control.break_jumps.is_empty()
            || !control.continue_jumps.is_empty()
            || !control.finally_gosubs.is_empty()
        {
            return Err(Error::internal(
                "destructuring iterator control stack is unbalanced",
            ));
        }
        Ok(())
    }

    fn push_break_control(
        &mut self,
        kind: BreakControlKind,
        label_name: Option<String>,
        entry_depth: usize,
        drop_count: usize,
    ) {
        let scope = self.current_ir().current_scope;
        self.current_ir_mut()
            .break_controls
            .push(BreakControlContext {
                kind,
                label_name,
                scope,
                entry_depth,
                drop_count,
                break_jumps: Vec::new(),
                continue_jumps: Vec::new(),
                finally_gosubs: Vec::new(),
            });
    }

    fn pop_break_control(&mut self) -> Result<BreakControlContext, Error> {
        self.current_ir_mut()
            .break_controls
            .pop()
            .ok_or_else(|| Error::internal("break control stack underflow"))
    }

    fn require_stack_depth(&self, expected: usize, construct: &str) -> Result<(), Error> {
        if self.current_ir().stack_depth == expected {
            Ok(())
        } else {
            Err(Error::internal(format!(
                "{construct} changed the enclosing stack depth"
            )))
        }
    }

    fn finish_control_statement(&mut self) {
        self.current_ir_mut().last_member_reference = None;
        self.current_ir_mut().last_identifier_reference = None;
        self.current_ir_mut().last_optional_chain = None;
        self.anonymous_function_definition = None;
    }

    fn parse_expression_statement(&mut self, completion: StatementCompletion) -> Result<(), Error> {
        // QuickJS seeds `emit_source_pos` from the first token before
        // `js_parse_expr`. A more specific marker emitted by the expression at
        // the same first opcode wins; otherwise synthetic operations (notably
        // template concat lookup) inherit this statement-entry position.
        let expression_start = self.current_ir().ops.len();
        let expression_site = source_offset(self.current().span)?;
        self.parse_expression()?;
        self.inherit_source_marker_at(expression_start, expression_site)?;
        match completion {
            StatementCompletion::Eval => {
                self.emit_instruction(Instruction::PutLocal(self.eval_ret_local()?))?;
            }
            StatementCompletion::Discard => {
                self.emit_instruction(Instruction::Drop)?;
            }
        }
        self.consume_statement_terminator()
    }

    fn set_eval_ret_undefined(&mut self) -> Result<(), Error> {
        self.emit_instruction(Instruction::Undefined)?;
        self.emit_instruction(Instruction::PutLocal(self.eval_ret_local()?))?;
        Ok(())
    }

    fn eval_ret_local(&self) -> Result<u16, Error> {
        self.current_ir()
            .eval_ret_local
            .ok_or_else(|| Error::internal("eval completion local requested outside a script"))
    }

    fn parse_return_statement(&mut self) -> Result<(), Error> {
        if self.current_ir().class_initializer_kind == Some(ClassInitializerKind::StaticBlock) {
            return Err(self.syntax_here("return in a static initializer block"));
        }
        let statement_depth = self.current_ir().stack_depth;
        let return_span = self.current().span;
        self.advance()?;
        let has_value = !(self.current().line_terminator_before
            || self.at_eof()
            || self.is_punctuator(Punctuator::Semicolon)
            || self.is_punctuator(Punctuator::RightBrace));
        if !has_value {
            self.emit_instruction(Instruction::Undefined)?;
        } else {
            self.parse_expression()?;
            // QuickJS folds `call; return` to a tail-call opcode and moves the
            // source marker to the `return` keyword. Preserve that observable
            // debug site even though this typed VM keeps two instructions.
            if let Some(SpannedIrOp {
                op:
                    IrOp::Bytecode(Instruction::Call(_) | Instruction::CallMethod(_))
                    | IrOp::TemplateCall { .. },
                pc_site,
            }) = self.current_ir_mut().ops.last_mut()
            {
                *pc_site = Some(source_offset(return_span)?);
            }
        }
        self.emit_return_completion(return_span, has_value)?;
        self.consume_statement_terminator()?;
        // Parsing continues through unreachable source. Retain the enclosing
        // statement's marker/discriminant shape just as break/continue do.
        self.current_ir_mut().stack_depth = statement_depth;
        Ok(())
    }

    /// Emit QuickJS's shared return path for an already-evaluated value at
    /// TOS. Generator resumption uses this same path when `.return(value)` is
    /// injected at a `yield`, so iterator/finally unwinding remains identical
    /// to an authored ReturnStatement.
    fn emit_return_completion(
        &mut self,
        return_span: Span,
        await_async_generator_value: bool,
    ) -> Result<(), Error> {
        if await_async_generator_value
            && self.current_ir().execution_kind == BytecodeFunctionKind::AsyncGenerator
        {
            // Pinned QuickJS performs this await before any iterator-close or
            // finally work so a rejected return value wins with the same
            // observable ordering as `emit_return`.
            self.emit_instruction_at(Instruction::Await, source_offset(return_span)?)?;
        }
        let async_iterator_return =
            if self.current_ir().execution_kind == BytecodeFunctionKind::AsyncGenerator
                && self.current_ir().break_controls.iter().any(|control| {
                    matches!(
                        control.kind,
                        BreakControlKind::DestructuringIterator | BreakControlKind::ForOf
                    )
                })
            {
                Some(self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::from_static("return"),
                )))?)
            } else {
                None
            };
        // QuickJS walks BlockEnv entries from inner to outer and interleaves
        // iterator closing with finally execution. Keeping that order is
        // observable when either an iterator `return` method or a finally body
        // throws, and is also required for the VM's nested unwind regions.
        let unwind_controls = self
            .current_ir()
            .break_controls
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, control)| {
                matches!(
                    control.kind,
                    BreakControlKind::DestructuringIterator
                        | BreakControlKind::ForOfAssignmentFragment
                        | BreakControlKind::ForOf
                        | BreakControlKind::TryFinally
                )
                .then_some((index, control.kind, control.entry_depth))
            })
            .collect::<Vec<_>>();
        for (control_index, kind, handler_depth) in unwind_controls {
            match kind {
                BreakControlKind::DestructuringIterator | BreakControlKind::ForOf => {
                    if handler_depth < 3 || handler_depth >= self.current_ir().stack_depth {
                        return Err(Error::internal(
                            "return unwind targeted an invalid iterator record",
                        ));
                    }
                    if let Some(return_name) = async_iterator_return {
                        // Pinned QuickJS hand-lowers AsyncGenerator return
                        // cleanup instead of using OP_iterator_close:
                        //
                        //   iter next ... completion
                        //     -> completion iter
                        //     -> completion iter return
                        //
                        // A nullish return method is ignored. Otherwise it is
                        // called without arguments, its immediate result is
                        // checked as an Object *before* Await, and the settled
                        // value is discarded without a second object check.
                        self.emit_instruction(Instruction::IteratorDetachPreserve)?;
                        self.current_ir_mut().stack_depth = handler_depth - 1;
                        self.emit_instruction(Instruction::GetField2(return_name))?;
                        self.emit_instruction(Instruction::Dup)?;
                        self.emit_instruction(Instruction::IsUndefinedOrNull)?;
                        let missing_return =
                            self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
                        self.emit_instruction(Instruction::CallMethod(0))?;
                        self.emit_instruction(Instruction::IteratorCheckObject)?;
                        self.emit_instruction(Instruction::Await)?;
                        let close_done = self.emit_instruction(Instruction::Goto(u32::MAX))?;

                        let missing_target = self.current_ir().ops.len();
                        self.current_ir_mut().stack_depth = handler_depth;
                        self.patch_jump(missing_return, missing_target)?;
                        // Drop the nullish method; the common tail then drops
                        // the retained iterator or settled close result.
                        self.emit_instruction(Instruction::Drop)?;
                        let close_done_target = self.current_ir().ops.len();
                        self.patch_jump(close_done, close_done_target)?;
                        self.require_stack_depth(
                            handler_depth - 1,
                            "async-generator iterator-return branch",
                        )?;
                        self.emit_instruction(Instruction::Drop)?;
                        self.current_ir_mut().stack_depth = handler_depth - 2;
                    } else {
                        self.emit_instruction(Instruction::IteratorClosePreserve)?;
                        // The generic instruction effect is value preserving,
                        // but this typed form also truncates the complete
                        // iterator record and any intermediate finally
                        // operands.
                        self.current_ir_mut().stack_depth = handler_depth - 2;
                    }
                }
                BreakControlKind::ForOfAssignmentFragment => {
                    if handler_depth < 3 || handler_depth >= self.current_ir().stack_depth {
                        return Err(Error::internal(
                            "return unwind targeted an invalid for-of assignment record",
                        ));
                    }
                    self.emit_instruction(Instruction::IteratorDropPreserve)?;
                    // As in pinned QuickJS, leaving the precompiled head
                    // assignment abandons the outer record without invoking
                    // IteratorClose.
                    self.current_ir_mut().stack_depth = handler_depth - 2;
                }
                BreakControlKind::TryFinally => {
                    // Preserve the return value while removing everything
                    // through the nearest catch marker, then call the
                    // associated finally body.
                    self.emit_instruction(Instruction::NipCatch)?;
                    if handler_depth > self.current_ir().stack_depth {
                        return Err(Error::internal(
                            "return unwind targeted a deeper catch marker",
                        ));
                    }
                    self.current_ir_mut().stack_depth = handler_depth;
                    self.require_stack_depth(handler_depth, "return catch cleanup")?;
                    let gosub = self.emit_instruction(Instruction::Gosub(u32::MAX))?;
                    self.current_ir_mut().break_controls[control_index]
                        .finally_gosubs
                        .push(gosub);
                }
                _ => unreachable!("return unwind list contains an ordinary control"),
            }
        }
        let instruction = if self.current_ir().derived_class_constructor {
            Instruction::ReturnDerived(
                self.current_ir()
                    .this_local
                    .ok_or_else(|| Error::internal("derived constructor has no this binding"))?,
            )
        } else {
            Instruction::Return
        };
        self.emit_instruction_at(instruction, source_offset(return_span)?)?;
        Ok(())
    }

    fn parse_throw_statement(&mut self) -> Result<(), Error> {
        let throw_span = self.current().span;
        self.advance()?;
        if self.current().line_terminator_before {
            return Err(Error::syntax(
                "line terminator not allowed after throw",
                source_span(self.current().span),
            ));
        }
        self.parse_expression()?;
        self.emit_instruction_at(Instruction::Throw, source_offset(throw_span)?)?;
        self.consume_statement_terminator()
    }

    fn parse_lexical_statement(&mut self) -> Result<(), Error> {
        self.parse_lexical_declarations_with_in(InMode::Allow)?;
        self.consume_statement_terminator()
    }

    fn parse_lexical_declarations_with_in(&mut self, mode: InMode) -> Result<(), Error> {
        self.with_in_mode(mode, Self::parse_lexical_declarations)
    }

    fn parse_lexical_declarations(&mut self) -> Result<(), Error> {
        let is_const = matches!(self.current().kind, TokenKind::Keyword(Keyword::Const));
        self.advance()?;

        loop {
            if self.is_punctuator(Punctuator::LeftBracket) {
                self.parse_array_binding_declaration(ForAssignmentDeclaration::Lexical, is_const)?;
            } else if self.is_punctuator(Punctuator::LeftBrace) {
                self.parse_object_binding_declaration(ForAssignmentDeclaration::Lexical, is_const)?;
            } else {
                let token = self.current().clone();
                let TokenKind::Identifier(identifier) = token.kind else {
                    return Err(self.syntax_here("variable name expected"));
                };
                validate_identifier_reservation(
                    &identifier,
                    token.span,
                    self.current_ir().strict,
                    IdentifierContext::Variable,
                )?;
                if identifier.value == "let" {
                    return Err(Error::syntax(
                        "'let' is not a valid lexical identifier",
                        source_span(token.span),
                    ));
                }
                let name = identifier.value;
                let strict = self.current_ir().strict;
                self.advance()?;
                if strict && matches!(name.as_str(), "eval" | "arguments") {
                    return Err(Error::syntax(
                        "invalid variable name in strict mode",
                        source_span(self.current().span),
                    ));
                }
                self.register_lexical_binding(
                    &name,
                    token.span,
                    self.current().span,
                    is_const,
                    false,
                )?;

                let initializer_site = if self.consume_punctuator(Punctuator::Equal)? {
                    let site = source_offset(self.tokens[self.cursor - 1].span)?;
                    self.parse_assignment()?;
                    if let Some(definition) = self.take_anonymous_function_definition() {
                        let name_constant = self.add_constant(IrConstant::Primitive(
                            Value::String(JsString::try_from_utf8(&name)?),
                        ))?;
                        self.emit_anonymous_set_name(
                            definition,
                            Instruction::SetName(name_constant),
                        )?;
                    }
                    site
                } else {
                    if is_const {
                        return Err(Error::syntax(
                            "missing initializer for const variable",
                            source_span(self.current().span),
                        ));
                    }
                    self.emit_instruction(Instruction::Undefined)?;
                    source_offset(token.span)?
                };
                self.emit_identifier_at(
                    name,
                    token.span,
                    IdentifierAccess::Initialize,
                    initializer_site,
                )?;
            }

            if !self.consume_punctuator(Punctuator::Comma)? {
                break;
            }
        }
        Ok(())
    }

    fn parse_var_statement(&mut self) -> Result<(), Error> {
        self.advance()?;
        self.parse_var_declarations_with_in(InMode::Allow)?;
        self.consume_statement_terminator()
    }

    fn parse_var_declarations_with_in(&mut self, mode: InMode) -> Result<(), Error> {
        self.with_in_mode(mode, Self::parse_var_declarations)
    }

    fn parse_var_declarations(&mut self) -> Result<(), Error> {
        loop {
            if self.is_punctuator(Punctuator::LeftBracket) {
                self.parse_array_binding_declaration(ForAssignmentDeclaration::Var, false)?;
            } else if self.is_punctuator(Punctuator::LeftBrace) {
                self.parse_object_binding_declaration(ForAssignmentDeclaration::Var, false)?;
            } else {
                let token = self.current().clone();
                let TokenKind::Identifier(identifier) = token.kind else {
                    return Err(self.syntax_here("variable name expected"));
                };
                validate_identifier_reservation(
                    &identifier,
                    token.span,
                    self.current_ir().strict,
                    IdentifierContext::Variable,
                )?;
                let strict = self.current_ir().strict;
                let name = identifier.value;
                self.advance()?;
                if strict && matches!(name.as_str(), "eval" | "arguments") {
                    return Err(Error::syntax(
                        "invalid variable name in strict mode",
                        source_span(self.current().span),
                    ));
                }
                self.register_var_binding(&name, token.span, self.current().span)?;

                let initializer_span = self.current().span;
                if self.consume_punctuator(Punctuator::Equal)? {
                    let initializer_scope = self.current_ir().current_scope;
                    let object_environment = self
                        .parser_scope_has_authored_with(self.current_function, initializer_scope)?;
                    if object_environment {
                        self.emit_at(
                            IrOp::IdentifierReference {
                                name: name.clone(),
                                span: token.span,
                                scope: initializer_scope,
                                access: IdentifierReferenceAccess::Prepare,
                            },
                            source_offset(token.span)?,
                        )?;
                    }
                    self.parse_assignment()?;
                    if let Some(definition) = self.take_anonymous_function_definition() {
                        // QuickJS emits a dummy OP_set_name after an anonymous
                        // closure and rewrites its atom when NamedEvaluation
                        // applies to this initializer. Keep that contextual name
                        // separate from the child bytecode's intrinsic func_name.
                        let name_constant = self.add_constant(IrConstant::Primitive(
                            Value::String(JsString::try_from_utf8(&name)?),
                        ))?;
                        self.emit_anonymous_set_name(
                            definition,
                            Instruction::SetName(name_constant),
                        )?;
                    }
                    if object_environment {
                        self.emit_at(
                            IrOp::IdentifierReference {
                                name,
                                span: token.span,
                                scope: initializer_scope,
                                access: IdentifierReferenceAccess::Set,
                            },
                            source_offset(initializer_span)?,
                        )?;
                        self.emit_instruction(Instruction::Drop)?;
                    } else {
                        self.emit_identifier_at(
                            name,
                            token.span,
                            IdentifierAccess::Put,
                            source_offset(initializer_span)?,
                        )?;
                    }
                }
            }

            if !self.consume_punctuator(Punctuator::Comma)? {
                break;
            }
        }
        Ok(())
    }

    fn register_var_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
        conflict_span: Span,
    ) -> Result<(), Error> {
        if matches!(self.current_ir().kind, FunctionKind::Eval(_)) {
            return self.register_eval_var_binding(name, declaration_span, conflict_span);
        }
        if matches!(self.current_ir().kind, FunctionKind::Module) {
            if let Some((_, binding)) = self
                .current_ir()
                .binding_id_from_scope(self.current_ir().current_scope, name)
            {
                let BindingStorage::Module(module_binding) =
                    self.current_ir().bindings[binding.0].storage
                else {
                    return Err(Error::internal(
                        "module declaration resolved to non-module storage",
                    ));
                };
                if self
                    .module
                    .as_ref()
                    .and_then(|module| module.binding(module_binding).ok())
                    .and_then(|binding| binding.declaration)
                    .is_some_and(|declaration| {
                        matches!(declaration, module::ModuleDeclarationOrigin::Lexical { .. })
                    })
                {
                    return Err(Error::syntax(
                        "invalid redefinition of lexical identifier",
                        source_span(conflict_span),
                    ));
                }
            }
            if let Some(binding) = self
                .current_ir()
                .binding_id_in_scope(self.current_ir().var_scope, name)
            {
                let BindingStorage::Module(module_binding) =
                    self.current_ir().bindings[binding.0].storage
                else {
                    return Err(Error::internal("module var resolved to non-module storage"));
                };
                let first_declaration = self
                    .module
                    .as_ref()
                    .and_then(|module| module.binding(module_binding).ok())
                    .is_some_and(|binding| binding.declaration.is_none());
                if first_declaration {
                    let declaration_scope = self.current_ir().current_scope;
                    let binding = self
                        .current_ir_mut()
                        .bindings
                        .get_mut(binding.0)
                        .ok_or_else(|| Error::internal("module var binding moved"))?;
                    binding.declaration_scope = declaration_scope;
                    binding.declaration_span = Some(declaration_span);
                }
                self.add_module_binding(name, module::ModuleDeclarationOrigin::Var)?;
                self.export_module_declaration(name, module_binding, declaration_span)?;
                return Ok(());
            }
            let module_binding =
                self.add_module_binding(name, module::ModuleDeclarationOrigin::Var)?;
            let function = self.current_ir_mut();
            function.add_binding(
                function.var_scope,
                function.current_scope,
                name.to_owned(),
                BindingStorage::Module(module_binding),
                BindingKind::Normal,
                Some(declaration_span),
            );
            self.export_module_declaration(name, module_binding, declaration_span)?;
            return Ok(());
        }
        let function = &mut self.functions[self.current_function];
        let selects_arguments_object =
            matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
                && name == "arguments"
                && !function
                    .parameters
                    .iter()
                    .any(|parameter| parameter.as_deref() == Some("arguments"))
                && !function
                    .parameter_pattern_bindings
                    .iter()
                    .any(|binding| binding.name == "arguments");
        if let Some((binding_scope, binding)) =
            function.binding_id_from_scope(function.current_scope, name)
            && matches!(
                function.bindings[binding.0].kind,
                BindingKind::Lexical { .. }
            )
        {
            let catch_parameter = function.bindings[binding.0].is_catch_parameter;
            let masked_program_lexical = matches!(function.kind, FunctionKind::Script)
                && binding_scope == function.body_scope
                && function.first_global_declaration_is_normal(name);
            if !catch_parameter && !masked_program_lexical {
                return Err(Error::syntax(
                    "invalid redefinition of lexical identifier",
                    source_span(conflict_span),
                ));
            }
        }
        if matches!(function.kind, FunctionKind::Script) {
            function.global_declarations.push(IrGlobalDeclaration {
                name: name.to_owned(),
                is_lexical: false,
                is_const: false,
                function_constant: None,
                closure_index: None,
            });
        }
        if let Some(binding) = function.binding_in_scope(function.var_scope, name) {
            if selects_arguments_object {
                let BindingStorage::Local(index) = binding.storage else {
                    return Err(Error::internal(
                        "implicit arguments declaration did not select a root local",
                    ));
                };
                if function
                    .arguments_local
                    .replace(index)
                    .is_some_and(|old| old != index)
                {
                    return Err(Error::internal(
                        "ordinary function selected more than one arguments local",
                    ));
                }
            }
            return Ok(());
        }
        if matches!(function.kind, FunctionKind::Script) {
            function.add_binding(
                function.var_scope,
                function.current_scope,
                name.to_owned(),
                BindingStorage::Global,
                BindingKind::Normal,
                Some(declaration_span),
            );
            return Ok(());
        }
        if function.locals.len() >= MAX_LOCAL_VARIABLES {
            return Err(
                Error::new(ErrorKind::JsInternal, "too many local variables")
                    .with_span(source_span(declaration_span)),
            );
        }
        let index = u16::try_from(function.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        function.locals.push(name.to_owned());
        function.add_binding(
            function.var_scope,
            function.current_scope,
            name.to_owned(),
            BindingStorage::Local(index),
            BindingKind::Normal,
            Some(declaration_span),
        );
        if selects_arguments_object
            && function
                .arguments_local
                .replace(index)
                .is_some_and(|old| old != index)
        {
            return Err(Error::internal(
                "ordinary function selected more than one arguments local",
            ));
        }
        Ok(())
    }

    fn current_eval_declaration_mode(&self) -> Result<EvalDeclarationMode, Error> {
        let function = self.current_ir();
        let FunctionKind::Eval(kind) = function.kind else {
            return Err(Error::internal(
                "eval declaration mode requested outside an eval root",
            ));
        };
        if function.strict {
            return Ok(EvalDeclarationMode::Local);
        }
        match kind {
            EvalKind::Indirect => Ok(EvalDeclarationMode::Global),
            EvalKind::Direct => match function.eval_caller_profile.variable_target {
                EvalCallerVariableTarget::Global => Ok(EvalDeclarationMode::Global),
                EvalCallerVariableTarget::ExternalBinding(index) => function
                    .external_bindings
                    .get(usize::from(index))
                    .filter(|binding| {
                        matches!(
                            binding.kind,
                            ClosureVariableKind::EvalVariableObject
                                | ClosureVariableKind::ArgEvalVariableObject
                        ) && !binding.is_lexical
                            && !binding.is_const
                            && !binding.is_catch_parameter
                    })
                    .map(|_| EvalDeclarationMode::Dynamic(EvalVariableSource::Closure(index)))
                    .ok_or_else(|| {
                        Error::internal("eval caller variable target is not authenticated")
                    }),
                EvalCallerVariableTarget::StrictLocal => Err(Error::internal(
                    "sloppy eval root retained a strict-local variable target",
                )),
            },
            EvalKind::None => Err(Error::internal("eval root has no eval kind")),
        }
    }

    fn eval_dynamic_declaration_target(
        &mut self,
        name: &str,
        object: EvalVariableSource,
        _conflict_span: Span,
    ) -> Result<EvalDeclarationTarget, Error> {
        let EvalVariableSource::Closure(object_index) = object else {
            return Err(Error::internal(
                "eval root declaration targeted a non-external variable object",
            ));
        };
        let external_bindings = self.current_ir().external_bindings.clone();
        for (index, binding) in external_bindings.iter().enumerate() {
            let index = u16::try_from(index)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
            if index == object_index {
                if !matches!(
                    binding.kind,
                    ClosureVariableKind::EvalVariableObject
                        | ClosureVariableKind::ArgEvalVariableObject
                ) {
                    return Err(Error::internal(
                        "eval variable object external index changed",
                    ));
                }
                return Ok(EvalDeclarationTarget::Dynamic(object));
            }
            if binding.name.to_utf8_lossy() != name {
                continue;
            }
            if binding.is_lexical
                && !binding.is_catch_parameter
                && self.current_ir().eval_redeclaration.is_none()
            {
                self.current_ir_mut().eval_redeclaration = Some(name.to_owned());
            }
            let kind =
                binding_kind_from_closure_flags(binding.kind, binding.is_lexical, binding.is_const)
                    .ok_or_else(|| Error::internal("eval caller binding flags are inconsistent"))?;
            return Ok(EvalDeclarationTarget::External { index, kind });
        }
        Err(Error::internal(
            "sloppy direct eval has no variable object external",
        ))
    }

    fn register_eval_var_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
        conflict_span: Span,
    ) -> Result<(), Error> {
        let mode = self.current_eval_declaration_mode()?;
        let function = self.current_ir();
        if let Some((_, binding)) = function.binding_id_from_scope(function.current_scope, name) {
            let binding = &function.bindings[binding.0];
            if matches!(binding.kind, BindingKind::Lexical { .. })
                && !matches!(binding.storage, BindingStorage::External(_))
                && !binding.is_catch_parameter
            {
                return Err(Error::syntax(
                    "invalid redefinition of lexical identifier",
                    source_span(conflict_span),
                ));
            }
        }

        match mode {
            EvalDeclarationMode::Dynamic(object) => {
                let target = self.eval_dynamic_declaration_target(name, object, conflict_span)?;
                self.current_ir_mut()
                    .eval_declarations
                    .push(IrEvalDeclaration {
                        name: name.to_owned(),
                        target,
                        value: EvalDeclarationValue::Undefined,
                    });
                Ok(())
            }
            EvalDeclarationMode::Local => {
                let function = self.current_ir_mut();
                let existing = function.scopes[function.var_scope.0]
                    .bindings
                    .iter()
                    .rev()
                    .copied()
                    .find(|binding| {
                        let binding = &function.bindings[binding.0];
                        binding.name == name
                            && !matches!(binding.storage, BindingStorage::External(_))
                    });
                if existing.is_some() {
                    return Ok(());
                }
                if function.locals.len() >= MAX_LOCAL_VARIABLES {
                    return Err(
                        Error::new(ErrorKind::JsInternal, "too many local variables")
                            .with_span(source_span(declaration_span)),
                    );
                }
                let index = u16::try_from(function.locals.len())
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
                function.locals.push(name.to_owned());
                function.add_binding(
                    function.var_scope,
                    function.current_scope,
                    name.to_owned(),
                    BindingStorage::Local(index),
                    BindingKind::Normal,
                    Some(declaration_span),
                );
                Ok(())
            }
            EvalDeclarationMode::Global => {
                let function = self.current_ir_mut();
                function.global_declarations.push(IrGlobalDeclaration {
                    name: name.to_owned(),
                    is_lexical: false,
                    is_const: false,
                    function_constant: None,
                    closure_index: None,
                });
                let caller_lexical_conflict = function
                    .external_bindings
                    .iter()
                    .find(|binding| binding.name.to_utf8_lossy() == name)
                    .is_some_and(|binding| binding.is_lexical && !binding.is_catch_parameter);
                if caller_lexical_conflict && function.eval_redeclaration.is_none() {
                    function.eval_redeclaration = Some(name.to_owned());
                }
                let existing = function.scopes[function.var_scope.0]
                    .bindings
                    .iter()
                    .rev()
                    .copied()
                    .find(|binding| {
                        let binding = &function.bindings[binding.0];
                        binding.name == name && binding.storage == BindingStorage::Global
                    });
                if existing.is_none() {
                    function.add_binding(
                        function.var_scope,
                        function.current_scope,
                        name.to_owned(),
                        BindingStorage::Global,
                        BindingKind::Normal,
                        Some(declaration_span),
                    );
                }
                Ok(())
            }
        }
    }

    fn register_lexical_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
        conflict_span: Span,
        is_const: bool,
        allow_body_parameter_shadow: bool,
    ) -> Result<(), Error> {
        let scope = self.current_ir().current_scope;
        let scope_kind = self.current_ir().scopes[scope.0].kind;
        let is_global = matches!(scope_kind, ScopeKind::ProgramBody)
            && matches!(self.current_ir().kind, FunctionKind::Script)
            && scope == self.current_ir().body_scope;
        let is_eval_body = matches!(scope_kind, ScopeKind::ProgramBody)
            && matches!(self.current_ir().kind, FunctionKind::Eval(_))
            && scope == self.current_ir().body_scope;
        let is_module_body = matches!(scope_kind, ScopeKind::ProgramBody)
            && matches!(self.current_ir().kind, FunctionKind::Module)
            && scope == self.current_ir().body_scope;
        if is_module_body {
            if let Some(module_binding) = self
                .module
                .as_ref()
                .and_then(|module| module.binding_id(name))
            {
                let record = self
                    .module
                    .as_ref()
                    .and_then(|module| module.binding(module_binding).ok())
                    .ok_or_else(|| Error::internal("module lexical binding record is missing"))?;
                if record.declaration.is_some() || record.import.is_none() {
                    return Err(Error::syntax(
                        "invalid redefinition of lexical identifier",
                        source_span(conflict_span),
                    ));
                }
                let existing = self
                    .current_ir()
                    .binding_id_in_scope(scope, name)
                    .ok_or_else(|| {
                        Error::internal("module import has no body-scope binding record")
                    })?;
                if self.current_ir().bindings[existing.0].storage
                    != BindingStorage::Module(module_binding)
                {
                    return Err(Error::internal(
                        "module lexical collision resolved to different storage",
                    ));
                }
                let binding = self
                    .current_ir_mut()
                    .bindings
                    .get_mut(existing.0)
                    .ok_or_else(|| Error::internal("module lexical binding moved"))?;
                binding.declaration_scope = scope;
                binding.declaration_span = Some(declaration_span);
                self.add_module_binding(
                    name,
                    module::ModuleDeclarationOrigin::Lexical { is_const },
                )?;
                self.export_module_declaration(name, module_binding, declaration_span)?;
                return Ok(());
            }
            let module_binding = self
                .add_module_binding(name, module::ModuleDeclarationOrigin::Lexical { is_const })?;
            let function = self.current_ir_mut();
            function.add_binding(
                scope,
                scope,
                name.to_owned(),
                BindingStorage::Module(module_binding),
                BindingKind::Lexical { is_const },
                Some(declaration_span),
            );
            self.export_module_declaration(name, module_binding, declaration_span)?;
            return Ok(());
        }

        let function = &mut self.functions[self.current_function];
        if is_eval_body
            && (function
                .eval_declarations
                .iter()
                .any(|declaration| declaration.name == name)
                || function
                    .global_declarations
                    .iter()
                    .any(|declaration| !declaration.is_lexical && declaration.name == name))
        {
            return Err(Error::syntax(
                "invalid redefinition of lexical identifier",
                source_span(conflict_span),
            ));
        }
        let supported_scope = is_global
            || is_eval_body
            || is_module_body
            || matches!(
                scope_kind,
                ScopeKind::Block
                    | ScopeKind::ClassPrivate
                    | ScopeKind::If
                    | ScopeKind::For
                    | ScopeKind::Switch
                    | ScopeKind::Catch
            )
            || (matches!(scope_kind, ScopeKind::FunctionBody)
                && matches!(
                    function.kind,
                    FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
                )
                && scope == function.body_scope);
        if !supported_scope {
            return Err(Error::internal(
                "lexical declaration escaped its supported parser scope",
            ));
        }
        let direct_catch_parameter_conflict = function.scopes[scope.0]
            .parent
            .filter(|parent| function.scopes[parent.0].kind == ScopeKind::Catch)
            .and_then(|parent| function.binding_id_in_scope(parent, name))
            .is_some_and(|binding| function.bindings[binding.0].is_catch_parameter);
        if direct_catch_parameter_conflict {
            return Err(Error::syntax(
                "invalid redefinition of lexical identifier",
                source_span(conflict_span),
            ));
        }
        if let Some(existing) = function.binding_id_in_scope(scope, name) {
            let masked_program_duplicate = is_global
                && function.first_global_declaration_is_normal(name)
                && matches!(
                    function.bindings[existing.0].kind,
                    BindingKind::Lexical { .. }
                );
            if masked_program_duplicate {
                function.global_declarations.push(IrGlobalDeclaration {
                    name: name.to_owned(),
                    is_lexical: true,
                    is_const,
                    function_constant: None,
                    closure_index: None,
                });
                return Ok(());
            }
            return Err(Error::syntax(
                "invalid redefinition of lexical identifier",
                source_span(conflict_span),
            ));
        }
        if let Some(binding) = function.binding_in_scope(function.var_scope, name) {
            let message = match (binding.storage, binding.kind) {
                (BindingStorage::Argument(_), _)
                    if scope == function.body_scope && allow_body_parameter_shadow =>
                {
                    ""
                }
                (BindingStorage::Argument(_), _) if scope == function.body_scope => {
                    "invalid redefinition of parameter name"
                }
                (BindingStorage::Argument(_), _) => "",
                (BindingStorage::Local(_), BindingKind::Normal)
                    if function.scope_is_within(binding.declaration_scope, scope) =>
                {
                    "invalid redefinition of a variable"
                }
                (BindingStorage::Local(_), BindingKind::Normal) => "",
                (BindingStorage::Local(_), BindingKind::FunctionName { .. }) => {
                    // The private named-expression binding lives outside the
                    // authored environments and may be shadowed there.
                    ""
                }
                (
                    BindingStorage::Local(_),
                    BindingKind::EvalVariableObject | BindingKind::ArgEvalVariableObject,
                ) => "",
                (BindingStorage::Local(_), BindingKind::WithObject) => {
                    return Err(Error::internal(
                        "with object binding leaked into the function var scope",
                    ));
                }
                (BindingStorage::Local(_), BindingKind::Lexical { .. }) => {
                    return Err(Error::internal(
                        "lexical binding leaked into the function var scope",
                    ));
                }
                (
                    BindingStorage::Local(_),
                    BindingKind::PrivateField { .. }
                    | BindingKind::PrivateMethod { .. }
                    | BindingKind::PrivateGetter { .. }
                    | BindingKind::PrivateSetter { .. }
                    | BindingKind::PrivateGetterSetter { .. },
                ) => {
                    return Err(Error::internal(
                        "private binding leaked into the function var scope",
                    ));
                }
                (BindingStorage::External(_), _) => "",
                (BindingStorage::Module(_), BindingKind::Normal)
                    if function.scope_is_within(binding.declaration_scope, scope) =>
                {
                    "invalid redefinition of module identifier"
                }
                (BindingStorage::Module(_), BindingKind::Normal) => "",
                (BindingStorage::Module(_), _) => {
                    return Err(Error::internal(
                        "non-var module binding leaked into the function var scope",
                    ));
                }
                (BindingStorage::Global, BindingKind::Normal)
                    if function.scope_is_within(binding.declaration_scope, scope) =>
                {
                    "invalid redefinition of global identifier"
                }
                (BindingStorage::Global, BindingKind::Normal) => "",
                (BindingStorage::Global, _) => {
                    return Err(Error::internal(
                        "non-var global binding leaked into the function var scope",
                    ));
                }
            };
            if !message.is_empty() {
                return Err(Error::syntax(message, source_span(conflict_span)));
            }
        }
        if is_global {
            function.global_declarations.push(IrGlobalDeclaration {
                name: name.to_owned(),
                is_lexical: true,
                is_const,
                function_constant: None,
                closure_index: None,
            });
            function.add_binding(
                scope,
                scope,
                name.to_owned(),
                BindingStorage::Global,
                BindingKind::Lexical { is_const },
                Some(declaration_span),
            );
            return Ok(());
        }
        if function.locals.len() >= MAX_LOCAL_VARIABLES {
            return Err(
                Error::new(ErrorKind::JsInternal, "too many local variables")
                    .with_span(source_span(declaration_span)),
            );
        }
        let index = u16::try_from(function.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        function.locals.push(name.to_owned());
        function.add_binding(
            scope,
            scope,
            name.to_owned(),
            BindingStorage::Local(index),
            BindingKind::Lexical { is_const },
            Some(declaration_span),
        );
        Ok(())
    }

    fn consume_statement_terminator(&mut self) -> Result<(), Error> {
        if self.consume_punctuator(Punctuator::Semicolon)?
            || self.at_eof()
            || self.is_punctuator(Punctuator::RightBrace)
            || self.current().line_terminator_before
        {
            Ok(())
        } else {
            Err(self.syntax_here("expecting ';'"))
        }
    }

    fn parse_expression(&mut self) -> Result<(), Error> {
        self.with_in_mode(InMode::Allow, Self::parse_comma)
    }

    fn parse_expression_no_in(&mut self) -> Result<(), Error> {
        self.with_in_mode(InMode::Disallow, Self::parse_comma)
    }

    fn parse_assignment_allow_in(&mut self) -> Result<(), Error> {
        self.with_in_mode(InMode::Allow, Self::parse_assignment)
    }

    fn with_in_mode<T>(
        &mut self,
        mode: InMode,
        parse: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let previous = std::mem::replace(&mut self.in_mode, mode);
        let result = parse(self);
        self.in_mode = previous;
        result
    }

    fn parse_comma(&mut self) -> Result<(), Error> {
        self.parse_assignment()?;
        let mut has_comma = false;
        while self.is_punctuator(Punctuator::Comma) {
            self.advance()?;
            has_comma = true;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_assignment()?;
        }
        if has_comma {
            self.anonymous_function_definition = None;
            self.current_ir_mut().last_member_reference = None;
            self.current_ir_mut().last_identifier_reference = None;
            self.current_ir_mut().last_optional_chain = None;
        }
        Ok(())
    }

    /// Parse assignment targets through typed unresolved References. Keeping
    /// identifier writes unresolved lets the late resolver select argument,
    /// local, closure, global and private-function-name behavior after the
    /// complete nested scope tree is known.
    fn parse_assignment(&mut self) -> Result<(), Error> {
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Yield)) {
            return self.parse_yield_expression();
        }
        if let Some(head) = self.async_arrow_ahead() {
            return self.parse_async_arrow_function(head);
        }
        if self.reserved_arrow_head_ahead()
            && !matches!(self.current().kind, TokenKind::Keyword(Keyword::Await))
        {
            return Err(self.syntax_here("invalid arrow function parameter"));
        }
        if let Some(head) = self.arrow_head_ahead() {
            return self.parse_arrow_function(head);
        }
        // Destructuring assignment is recognized only after every arrow-head
        // probe has had its say. Like pinned QuickJS, the Array form takes a
        // control-inverted path only when the matching `]` is immediately
        // followed by `=`; otherwise this remains an ordinary Array literal.
        if self.object_assignment_pattern_ahead() {
            return self.parse_object_assignment_expression();
        }
        if self.array_assignment_pattern_ahead() {
            return self.parse_array_assignment_expression();
        }
        // QuickJS's `name0` is captured only when the AssignmentExpression
        // starts with the identifier token itself. Parenthesized lvalues are
        // valid References but intentionally do not trigger NamedEvaluation.
        let direct_identifier_name = match &self.current().kind {
            TokenKind::Identifier(identifier) => Some(identifier.value.clone()),
            _ => None,
        };
        self.parse_conditional()?;
        if self.current_ir().last_optional_chain.is_some()
            && matches!(
                self.current().kind,
                TokenKind::Punctuator(
                    Punctuator::Equal
                        | Punctuator::PlusAssign
                        | Punctuator::MinusAssign
                        | Punctuator::MultiplyAssign
                        | Punctuator::DivideAssign
                        | Punctuator::RemainderAssign
                        | Punctuator::ExponentAssign
                        | Punctuator::ShiftLeftAssign
                        | Punctuator::ShiftRightAssign
                        | Punctuator::UnsignedShiftRightAssign
                        | Punctuator::BitAndAssign
                        | Punctuator::BitOrAssign
                        | Punctuator::BitXorAssign
                        | Punctuator::LogicalAndAssign
                        | Punctuator::LogicalOrAssign
                        | Punctuator::NullishAssign
                )
            )
        {
            // QuickJS consumes every assignment operator before get_lvalue
            // rejects an optional-chain Reference, so the diagnostic points
            // at the first RHS token.
            self.advance()?;
            return Err(self.syntax_here("invalid assignment left-hand side"));
        }
        let logical = match self.current().kind {
            TokenKind::Punctuator(Punctuator::LogicalAndAssign) => Some(LogicalAssignment::And),
            TokenKind::Punctuator(Punctuator::LogicalOrAssign) => Some(LogicalAssignment::Or),
            TokenKind::Punctuator(Punctuator::NullishAssign) => Some(LogicalAssignment::Nullish),
            _ => None,
        };
        if let Some(logical) = logical {
            if let Some(target) =
                self.promote_tail_identifier_get(IdentifierReferenceAccess::Get)?
            {
                let infer_name = direct_identifier_name.as_deref() == Some(target.name.as_str());
                return self.parse_logical_identifier_assignment(target, logical, infer_name);
            }
            return self.parse_logical_member_assignment(logical);
        }

        let assignment_span = self.current().span;
        let compound = match self.current().kind {
            TokenKind::Punctuator(Punctuator::Equal) => None,
            TokenKind::Punctuator(Punctuator::PlusAssign) => Some(Instruction::Add),
            TokenKind::Punctuator(Punctuator::MinusAssign) => Some(Instruction::Sub),
            TokenKind::Punctuator(Punctuator::MultiplyAssign) => Some(Instruction::Mul),
            TokenKind::Punctuator(Punctuator::DivideAssign) => Some(Instruction::Div),
            TokenKind::Punctuator(Punctuator::RemainderAssign) => Some(Instruction::Mod),
            TokenKind::Punctuator(Punctuator::ExponentAssign) => Some(Instruction::Pow),
            TokenKind::Punctuator(Punctuator::ShiftLeftAssign) => Some(Instruction::Shl),
            TokenKind::Punctuator(Punctuator::ShiftRightAssign) => Some(Instruction::Sar),
            TokenKind::Punctuator(Punctuator::UnsignedShiftRightAssign) => Some(Instruction::Shr),
            TokenKind::Punctuator(Punctuator::BitAndAssign) => Some(Instruction::BitAnd),
            TokenKind::Punctuator(Punctuator::BitXorAssign) => Some(Instruction::BitXor),
            TokenKind::Punctuator(Punctuator::BitOrAssign) => Some(Instruction::BitOr),
            _ => return Ok(()),
        };

        if let Some(operation) = compound {
            if let Some(target) =
                self.promote_tail_identifier_get(IdentifierReferenceAccess::Get)?
            {
                self.advance()?;
                self.validate_identifier_assignment_target(&target)?;
                self.parse_assignment()?;
                self.emit_instruction_at(operation, source_offset(assignment_span)?)?;
                self.anonymous_function_definition = None;
                if target.object_environment {
                    self.emit_identifier_reference_inherited(
                        target.name,
                        target.span,
                        target.scope,
                        IdentifierReferenceAccess::Set,
                    )?;
                } else {
                    self.emit_identifier_inherited(
                        target.name,
                        target.span,
                        target.scope,
                        IdentifierAccess::Set,
                    )?;
                }
                return Ok(());
            }
            let Some(target) = self.promote_tail_member_get_for_compound()? else {
                // QuickJS consumes the assignment operator before rejecting
                // a non-Reference left side, so the diagnostic points at the
                // first RHS token rather than the operator.
                self.advance()?;
                return Err(self.syntax_here("invalid assignment left-hand side"));
            };
            self.advance()?;
            self.parse_assignment()?;
            self.emit_instruction_at(operation, source_offset(assignment_span)?)?;
            self.anonymous_function_definition = None;
            self.emit_member_put(target)?;
            return Ok(());
        }

        if let Some(target) = self.take_tail_identifier_reference()? {
            self.advance()?;
            self.validate_identifier_assignment_target(&target)?;
            let rhs_start = self.current_ir().ops.len();
            self.parse_assignment()?;
            self.inherit_source_marker_at(rhs_start, source_offset(target.span)?)?;
            let anonymous_rhs = self.take_anonymous_function_definition();
            if direct_identifier_name.as_deref() == Some(target.name.as_str())
                && let Some(definition) = anonymous_rhs
            {
                let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::try_from_utf8(&target.name)?,
                )))?;
                self.emit_anonymous_set_name(definition, Instruction::SetName(name_constant))?;
            }
            // QuickJS emits no source position for ordinary `=`. The Set
            // inherits the LHS marker for an unmarked RHS or the last marker
            // produced while evaluating the RHS.
            if target.object_environment {
                self.emit_identifier_reference_inherited(
                    target.name,
                    target.span,
                    target.scope,
                    IdentifierReferenceAccess::Set,
                )?;
            } else {
                self.emit_identifier_inherited(
                    target.name,
                    target.span,
                    target.scope,
                    IdentifierAccess::Set,
                )?;
            }
            self.anonymous_function_definition = None;
            return Ok(());
        }

        let Some(target) = self.take_tail_member_reference()? else {
            // As with compound assignment, QuickJS has already advanced to
            // the RHS before reporting a non-Reference left side.
            self.advance()?;
            return Err(self.syntax_here("invalid assignment left-hand side"));
        };
        self.advance()?;
        let rhs_start = self.current_ir().ops.len();
        self.parse_assignment()?;
        let site = match &target {
            MemberReference::Field { site, .. }
            | MemberReference::Computed { site }
            | MemberReference::Super { site }
            | MemberReference::Private { site, .. } => *site,
        };
        self.inherit_source_marker_at(rhs_start, site)?;

        // QuickJS does not apply NamedEvaluation to member assignment. The
        // anonymous function marker also must not escape to an enclosing
        // identifier assignment such as `x = obj.p = function(){}`.
        self.anonymous_function_definition = None;
        self.emit_member_put(target)
    }

    /// Identifier logical assignment is QuickJS's depth-zero lvalue case. The
    /// short branch already contains only the old value, while the write branch
    /// replaces it with the RHS and resolves one preserving Set operation.
    fn parse_logical_identifier_assignment(
        &mut self,
        target: IdentifierReference,
        logical: LogicalAssignment,
        infer_name: bool,
    ) -> Result<(), Error> {
        self.advance()?;
        self.validate_identifier_assignment_target(&target)?;
        self.emit_instruction(Instruction::Dup)?;
        if logical == LogicalAssignment::Nullish {
            self.emit_instruction(Instruction::IsUndefinedOrNull)?;
        }
        let short_circuit = self.emit_instruction(match logical {
            LogicalAssignment::Or => Instruction::IfTrue(u32::MAX),
            LogicalAssignment::And | LogicalAssignment::Nullish => Instruction::IfFalse(u32::MAX),
        })?;
        let short_circuit_depth = self.current_ir().stack_depth;

        self.emit_instruction(Instruction::Drop)?;
        self.parse_assignment()?;
        let anonymous_rhs = self.take_anonymous_function_definition();
        if infer_name && let Some(definition) = anonymous_rhs {
            let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::try_from_utf8(&target.name)?,
            )))?;
            self.emit_anonymous_set_name(definition, Instruction::SetName(name_constant))?;
        }
        if target.object_environment {
            self.emit_identifier_reference_inherited(
                target.name,
                target.span,
                target.scope,
                IdentifierReferenceAccess::Set,
            )?;
        } else {
            self.emit_identifier_inherited(
                target.name,
                target.span,
                target.scope,
                IdentifierAccess::Set,
            )?;
        }
        let end = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let joined_depth = self.current_ir().stack_depth;

        let short_target = self.current_ir().ops.len();
        self.patch_jump(short_circuit, short_target)?;
        if target.object_environment {
            self.current_ir_mut().stack_depth = short_circuit_depth;
            self.emit_instruction(Instruction::Nip)?;
        }
        if self.current_ir().stack_depth != joined_depth {
            return Err(Error::internal(
                "identifier logical assignment branches have unequal stack depth",
            ));
        }
        self.patch_jump(end, self.current_ir().ops.len())?;
        self.anonymous_function_definition = None;
        self.current_ir_mut().last_member_reference = None;
        self.current_ir_mut().last_identifier_reference = None;
        self.current_ir_mut().last_optional_chain = None;
        Ok(())
    }

    fn validate_identifier_assignment_target(
        &self,
        target: &IdentifierReference,
    ) -> Result<(), Error> {
        if self.current_ir().strict && matches!(target.name.as_str(), "eval" | "arguments") {
            return Err(self.syntax_here("invalid lvalue in strict mode"));
        }
        Ok(())
    }

    /// Lower a logical member assignment with the same two-branch stack shape
    /// as QuickJS `js_parse_assign_expr2`. The kept member Reference is used
    /// only by the assignment branch; the short-circuit branch removes its
    /// base/key operands with `Nip` and returns the original property value.
    fn parse_logical_member_assignment(&mut self, logical: LogicalAssignment) -> Result<(), Error> {
        let Some(target) = self.promote_tail_member_get_for_compound()? else {
            // As with every other assignment operator, QuickJS advances to
            // the RHS before get_lvalue rejects a non-Reference left side.
            self.advance()?;
            return Err(self.syntax_here("invalid assignment left-hand side"));
        };
        let lvalue_depth = match &target {
            MemberReference::Field { .. } => 1,
            MemberReference::Computed { .. } => 2,
            MemberReference::Super { .. } => 3,
            MemberReference::Private { .. } => 1,
        };

        self.advance()?;
        self.emit_instruction(Instruction::Dup)?;
        if logical == LogicalAssignment::Nullish {
            self.emit_instruction(Instruction::IsUndefinedOrNull)?;
        }
        let short_circuit = self.emit_instruction(match logical {
            LogicalAssignment::Or => Instruction::IfTrue(u32::MAX),
            LogicalAssignment::And | LogicalAssignment::Nullish => Instruction::IfFalse(u32::MAX),
        })?;
        let short_circuit_depth = self.current_ir().stack_depth;

        self.emit_instruction(Instruction::Drop)?;
        self.parse_assignment()?;
        // Member assignment never applies NamedEvaluation to an anonymous RHS.
        self.anonymous_function_definition = None;
        self.emit_member_put(target)?;
        let end = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let joined_depth = self.current_ir().stack_depth;

        self.patch_jump(short_circuit, self.current_ir().ops.len())?;
        self.current_ir_mut().stack_depth = short_circuit_depth;
        for _ in 0..lvalue_depth {
            self.emit_instruction(Instruction::Nip)?;
        }
        if self.current_ir().stack_depth != joined_depth {
            return Err(Error::internal(
                "logical assignment branches have unequal stack depth",
            ));
        }
        self.patch_jump(end, self.current_ir().ops.len())?;
        self.current_ir_mut().last_member_reference = None;
        self.current_ir_mut().last_identifier_reference = None;
        self.current_ir_mut().last_optional_chain = None;
        Ok(())
    }

    fn parse_conditional(&mut self) -> Result<(), Error> {
        self.parse_coalesce()?;
        if !self.is_punctuator(Punctuator::Question) {
            return Ok(());
        }
        self.advance()?;
        self.anonymous_function_definition = None;

        let false_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        let branch_stack = self.current_ir().stack_depth;
        // QuickJS parses the consequent with ordinary AssignmentExpression
        // even when the surrounding classic-for initializer is NoIn.
        self.parse_assignment_allow_in()?;
        self.expect_punctuator(Punctuator::Colon)?;
        let end_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let joined_stack = self.current_ir().stack_depth;

        self.patch_jump(false_jump, self.current_ir().ops.len())?;
        self.current_ir_mut().stack_depth = branch_stack;
        self.parse_assignment()?;
        self.anonymous_function_definition = None;
        if self.current_ir().stack_depth != joined_stack {
            return Err(Error::internal(
                "conditional branches have unequal stack depth",
            ));
        }
        self.patch_jump(end_jump, self.current_ir().ops.len())?;
        self.current_ir_mut().last_member_reference = None;
        self.current_ir_mut().last_identifier_reference = None;
        self.current_ir_mut().last_optional_chain = None;
        Ok(())
    }

    /// QuickJS lowers a nullish-coalescing chain to one shared short-circuit
    /// label. Each segment preserves the selected value, and the RHS enters
    /// below the logical-and/or grammar level so unparenthesized mixing remains
    /// a syntax error instead of changing precedence.
    fn parse_coalesce(&mut self) -> Result<(), Error> {
        self.parse_logical_or()?;
        let mut short_circuits = Vec::new();
        while self.is_punctuator(Punctuator::NullishCoalesce) {
            self.advance()?;
            self.emit_instruction(Instruction::Dup)?;
            self.emit_instruction(Instruction::IsUndefinedOrNull)?;
            short_circuits.push(self.emit_instruction(Instruction::IfFalse(u32::MAX))?);
            self.emit_instruction(Instruction::Drop)?;
            self.parse_bitwise_or()?;
            self.anonymous_function_definition = None;
        }
        if !short_circuits.is_empty() {
            let end = self.current_ir().ops.len();
            for short_circuit in short_circuits {
                self.patch_jump(short_circuit, end)?;
            }
            self.current_ir_mut().last_member_reference = None;
            self.current_ir_mut().last_identifier_reference = None;
            self.current_ir_mut().last_optional_chain = None;
        }
        Ok(())
    }

    fn parse_logical_or(&mut self) -> Result<(), Error> {
        self.parse_logical_and()?;
        let mut composed = false;
        while self.is_punctuator(Punctuator::LogicalOr) {
            composed = true;
            self.advance()?;
            self.emit_instruction(Instruction::Dup)?;
            let end_jump = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_logical_and()?;
            self.patch_jump(end_jump, self.current_ir().ops.len())?;
            self.anonymous_function_definition = None;
        }
        if composed && self.is_punctuator(Punctuator::NullishCoalesce) {
            return Err(self.syntax_here("cannot mix ?? with && or ||"));
        }
        if composed {
            self.current_ir_mut().last_member_reference = None;
            self.current_ir_mut().last_identifier_reference = None;
            self.current_ir_mut().last_optional_chain = None;
        }
        Ok(())
    }

    fn parse_logical_and(&mut self) -> Result<(), Error> {
        self.parse_bitwise_or()?;
        let mut composed = false;
        while self.is_punctuator(Punctuator::LogicalAnd) {
            composed = true;
            self.advance()?;
            self.emit_instruction(Instruction::Dup)?;
            let end_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_bitwise_or()?;
            self.patch_jump(end_jump, self.current_ir().ops.len())?;
            self.anonymous_function_definition = None;
        }
        if composed && self.is_punctuator(Punctuator::NullishCoalesce) {
            return Err(self.syntax_here("cannot mix ?? with && or ||"));
        }
        if composed {
            self.current_ir_mut().last_member_reference = None;
            self.current_ir_mut().last_identifier_reference = None;
            self.current_ir_mut().last_optional_chain = None;
        }
        Ok(())
    }

    fn parse_bitwise_or(&mut self) -> Result<(), Error> {
        self.parse_bitwise_xor()?;
        while self.is_punctuator(Punctuator::BitOr) {
            let operation_span = self.current().span;
            self.advance()?;
            self.parse_bitwise_xor()?;
            self.emit_instruction_at(Instruction::BitOr, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_bitwise_xor(&mut self) -> Result<(), Error> {
        self.parse_bitwise_and()?;
        while self.is_punctuator(Punctuator::BitXor) {
            let operation_span = self.current().span;
            self.advance()?;
            self.parse_bitwise_and()?;
            self.emit_instruction_at(Instruction::BitXor, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_bitwise_and(&mut self) -> Result<(), Error> {
        self.parse_equality()?;
        while self.is_punctuator(Punctuator::BitAnd) {
            let operation_span = self.current().span;
            self.advance()?;
            self.parse_equality()?;
            self.emit_instruction_at(Instruction::BitAnd, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_equality(&mut self) -> Result<(), Error> {
        self.parse_relational()?;
        loop {
            let operation_span = self.current().span;
            let operation = match self.current().kind {
                TokenKind::Punctuator(Punctuator::EqualEqual) => Instruction::Eq,
                TokenKind::Punctuator(Punctuator::StrictEqual) => Instruction::StrictEq,
                TokenKind::Punctuator(Punctuator::NotEqual) => Instruction::Neq,
                TokenKind::Punctuator(Punctuator::StrictNotEqual) => Instruction::StrictNeq,
                _ => break,
            };
            self.advance()?;
            self.parse_relational()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_relational(&mut self) -> Result<(), Error> {
        if !self.parse_private_in_head()? {
            self.parse_shift()?;
        }
        loop {
            let operation_span = self.current().span;
            let operation = match self.current().kind {
                TokenKind::Punctuator(Punctuator::Less) => Instruction::Lt,
                TokenKind::Punctuator(Punctuator::LessEqual) => Instruction::Lte,
                TokenKind::Punctuator(Punctuator::Greater) => Instruction::Gt,
                TokenKind::Punctuator(Punctuator::GreaterEqual) => Instruction::Gte,
                TokenKind::Keyword(Keyword::Instanceof) => Instruction::InstanceOf,
                TokenKind::Keyword(Keyword::In) if self.in_mode == InMode::Disallow => break,
                TokenKind::Keyword(Keyword::In) => Instruction::In,
                _ => break,
            };
            self.advance()?;
            self.parse_shift()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_shift(&mut self) -> Result<(), Error> {
        self.parse_additive()?;
        loop {
            let operation_span = self.current().span;
            let operation = match self.current().kind {
                TokenKind::Punctuator(Punctuator::ShiftLeft) => Instruction::Shl,
                TokenKind::Punctuator(Punctuator::ShiftRight) => Instruction::Sar,
                TokenKind::Punctuator(Punctuator::UnsignedShiftRight) => Instruction::Shr,
                _ => break,
            };
            self.advance()?;
            self.parse_additive()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_additive(&mut self) -> Result<(), Error> {
        self.parse_multiplicative()?;
        loop {
            let operation_span = self.current().span;
            let operation = match self.current().kind {
                TokenKind::Punctuator(Punctuator::Plus) => Instruction::Add,
                TokenKind::Punctuator(Punctuator::Minus) => Instruction::Sub,
                _ => break,
            };
            self.advance()?;
            self.parse_multiplicative()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_multiplicative(&mut self) -> Result<(), Error> {
        self.parse_unary()?;
        loop {
            let operation_span = self.current().span;
            let operation = match self.current().kind {
                TokenKind::Punctuator(Punctuator::Multiply) => Instruction::Mul,
                TokenKind::Punctuator(Punctuator::Divide) => Instruction::Div,
                TokenKind::Punctuator(Punctuator::Remainder) => Instruction::Mod,
                _ => break,
            };
            self.advance()?;
            self.parse_unary()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_unary(&mut self) -> Result<(), Error> {
        self.parse_unary_with_power(PowerMode::Allowed)
    }

    fn parse_unary_with_power(&mut self, power_mode: PowerMode) -> Result<(), Error> {
        if matches!(
            self.current().kind,
            TokenKind::Punctuator(Punctuator::Increment | Punctuator::Decrement)
        ) {
            let operator_span = self.current().span;
            let increment = self.is_punctuator(Punctuator::Increment);
            self.advance()?;
            // QuickJS passes no power flag for a prefix-update operand. This
            // leaves `**` for the outer update expression, so `++x ** 2` is
            // valid while the operand itself must still be an lvalue.
            self.parse_unary_with_power(PowerMode::None)?;
            self.lower_update_expression(operator_span, increment, false)?;
            return self.parse_power_suffix(power_mode);
        }
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Await)) {
            if !matches!(
                self.current_ir().execution_kind,
                BytecodeFunctionKind::Async | BytecodeFunctionKind::AsyncGenerator
            ) {
                return Err(self.syntax_here("unexpected 'await' keyword"));
            }
            if !self.current_ir().in_function_body {
                return Err(self.syntax_here("await in default expression"));
            }
            self.advance()?;
            self.parse_unary_with_power(PowerMode::Forbidden)?;
            self.emit_instruction(Instruction::Await)?;
            self.anonymous_function_definition = None;
            self.current_ir_mut().last_member_reference = None;
            self.current_ir_mut().last_identifier_reference = None;
            self.current_ir_mut().last_optional_chain = None;
            return Ok(());
        }
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Typeof)) {
            self.advance()?;
            let operand_start = self.current_ir().ops.len();
            self.parse_unary_with_power(PowerMode::Forbidden)?;

            // Parentheses do not change an IdentifierReference into a value
            // expression, so both `typeof missing` and `typeof (missing)` use
            // QuickJS' non-throwing global lookup.  Calls, comma expressions,
            // binary operators, and every other composed expression emit
            // additional IR and therefore retain ordinary throwing lookup.
            if self.current_ir().ops.len() == operand_start + 1
                && let Some(SpannedIrOp {
                    op: IrOp::Identifier { access, .. },
                    ..
                }) = self.current_ir_mut().ops.get_mut(operand_start)
                && *access == IdentifierAccess::Get
            {
                *access = IdentifierAccess::GetOrUndefined;
            }
            self.emit_instruction(Instruction::TypeOf)?;
            self.anonymous_function_definition = None;
            return self.parse_power_suffix(power_mode);
        }
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Delete)) {
            self.advance()?;
            let operand_start = self.current_ir().ops.len();
            self.parse_unary_with_power(PowerMode::Forbidden)?;
            // Chain close deliberately clears a terminal private Reference,
            // so optional private delete falls through to the ordinary
            // value-delete path: it performs the branded read when live and
            // returns true without deleting. Only public terminal References
            // retain metadata for the Delete-specific branch rewrite.
            let terminal_optional_chain = {
                let function = self.current_ir_mut();
                if function.last_optional_chain.as_ref().is_some_and(|chain| {
                    chain.terminal_member_get == function.last_member_reference
                        && chain.terminal_member_get == function.ops.len().checked_sub(1)
                }) {
                    function.last_optional_chain.take()
                } else {
                    None
                }
            };
            if let Some(target) = self.take_tail_member_reference()? {
                match target {
                    MemberReference::Field { key, site } => {
                        self.emit_with_site(IrOp::PushConstant(key), Some(site))?;
                        self.emit_instruction(Instruction::Delete)?;
                    }
                    MemberReference::Computed { site } => {
                        self.emit_instruction_at(Instruction::Delete, site)?;
                    }
                    MemberReference::Super { site } => {
                        self.emit_instruction_at(Instruction::ThrowDeleteSuper, site)?;
                    }
                    MemberReference::Private { .. } => {
                        // QuickJS diagnoses after the complete operand has
                        // been parsed, so the current token is the one after
                        // the private reference.
                        return Err(self.syntax_here("cannot delete a private class field"));
                    }
                }
                if let Some(chain) = terminal_optional_chain {
                    self.rewrite_optional_chain_delete_fallback(chain)?;
                }
            } else if self.current_ir().ops.len() == operand_start + 1
                && matches!(
                    self.current_ir().ops[operand_start].op,
                    IrOp::Identifier {
                        access: IdentifierAccess::Get,
                        ..
                    }
                )
            {
                if self.current_ir().strict {
                    return Err(self.syntax_here("cannot delete a direct reference in strict mode"));
                }
                let Some(SpannedIrOp {
                    op: IrOp::Identifier { access, .. },
                    ..
                }) = self.current_ir_mut().ops.get_mut(operand_start)
                else {
                    return Err(Error::internal(
                        "direct delete identifier operation disappeared",
                    ));
                };
                *access = IdentifierAccess::Delete;
                self.current_ir_mut().last_identifier_reference = None;
            } else {
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction(Instruction::PushTrue)?;
            }
            self.anonymous_function_definition = None;
            return self.parse_power_suffix(power_mode);
        }
        let operation_span = self.current().span;
        let operation = match self.current().kind {
            TokenKind::Punctuator(Punctuator::Plus) => Some(Instruction::Plus),
            TokenKind::Punctuator(Punctuator::Minus) => Some(Instruction::Neg),
            TokenKind::Punctuator(Punctuator::BitNot) => Some(Instruction::BitNot),
            TokenKind::Punctuator(Punctuator::Not) => Some(Instruction::Not),
            TokenKind::Keyword(Keyword::Void) => {
                self.advance()?;
                self.parse_unary_with_power(PowerMode::Forbidden)?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction(Instruction::Undefined)?;
                self.anonymous_function_definition = None;
                return self.parse_power_suffix(power_mode);
            }
            _ => None,
        };
        if let Some(operation) = operation {
            self.advance()?;
            self.parse_unary_with_power(PowerMode::Forbidden)?;
            if matches!(
                operation,
                Instruction::Plus | Instruction::Neg | Instruction::BitNot
            ) {
                self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            } else {
                self.emit_instruction(operation)?;
            }
            self.anonymous_function_definition = None;
            return self.parse_power_suffix(power_mode);
        }
        self.parse_postfix()?;
        self.parse_power_suffix(power_mode)
    }

    fn parse_power_suffix(&mut self, power_mode: PowerMode) -> Result<(), Error> {
        if !self.is_punctuator(Punctuator::Exponent) {
            return Ok(());
        }
        match power_mode {
            PowerMode::None => return Ok(()),
            PowerMode::Forbidden => {
                return Err(Error::new(
                    ErrorKind::Syntax,
                    "unparenthesized unary expression can't appear on the left-hand side of '**'",
                ));
            }
            PowerMode::Allowed => {}
        }

        let operation_span = self.current().span;
        self.advance()?;
        self.parse_unary_with_power(PowerMode::Allowed)?;
        self.emit_instruction_at(Instruction::Pow, source_offset(operation_span)?)?;
        self.anonymous_function_definition = None;
        Ok(())
    }

    fn parse_postfix(&mut self) -> Result<(), Error> {
        self.parse_left_hand_side_expression()?;
        if !self.current().line_terminator_before
            && matches!(
                self.current().kind,
                TokenKind::Punctuator(Punctuator::Increment | Punctuator::Decrement)
            )
        {
            let operator_span = self.current().span;
            let increment = self.is_punctuator(Punctuator::Increment);
            self.lower_update_expression(operator_span, increment, true)?;
            self.advance()?;
        }
        Ok(())
    }

    /// Parse the LeftHandSideExpression subset shared by assignment and a
    /// for-of assignment target, deliberately stopping before postfix update.
    fn parse_left_hand_side_expression(&mut self) -> Result<(), Error> {
        self.parse_primary(true)?;
        let mut optional_chain: Option<PendingOptionalChain> = None;
        loop {
            let optional_call = if self.is_punctuator(Punctuator::OptionalChain) {
                let optional_span = self.current().span;
                self.advance()?;
                let chain = optional_chain.get_or_insert_default();
                if !self.is_punctuator(Punctuator::LeftParen) {
                    self.emit_optional_chain_test(chain, 1)?;
                    self.parse_optional_member_suffix(optional_span)?;
                    continue;
                }
                true
            } else {
                false
            };

            if self.parse_member_suffix()? {
                continue;
            }

            if optional_call || self.is_punctuator(Punctuator::LeftParen) {
                let call_span = self.current().span;
                let member_method = self.promote_last_member_get_for_call()?;
                // QuickJS selects OP_eval from the syntactic callee before
                // binding resolution. Parentheses preserve IdentifierReference,
                // while comma/member/other composed expressions have already
                // cleared this marker. Runtime identity still decides whether
                // this is original eval or an ordinary replacement call.
                //
                // `eval?.()` is deliberately indirect: the optional call
                // consumes the identifier marker as an ordinary callable
                // Reference and never emits EvalCall.
                let direct_eval_scope = if member_method || optional_call {
                    None
                } else {
                    self.take_direct_eval_scope()?
                };
                // An authored `with` may turn an identifier call into a method
                // call whose receiver is the selected object environment. The
                // unresolved Reference keeps the same two-slot call shape even
                // when resolution later proves the receiver is `undefined`.
                let identifier_method = if member_method || direct_eval_scope.is_some() {
                    false
                } else {
                    self.promote_tail_identifier_get(IdentifierReferenceAccess::Call)?
                        .is_some_and(|reference| reference.object_environment)
                };
                if optional_call && identifier_method {
                    // Pinned QuickJS 2026-06-04 promotes an authored-with
                    // identifier to a two-slot Reference, then applies the
                    // optional-call test with the one-slot accounting used by
                    // ordinary identifier calls. Its verifier rejects the
                    // resulting function before execution.
                    return Err(Error::new(ErrorKind::JsInternal, "inconsistent stack size"));
                }
                if optional_call {
                    self.emit_optional_chain_test(
                        optional_chain
                            .as_mut()
                            .ok_or_else(|| Error::internal("optional call lost its chain"))?,
                        if member_method || identifier_method {
                            2
                        } else {
                            1
                        },
                    )?;
                }
                self.advance()?;
                let arguments = self.parse_call_arguments()?;
                if let Some(scope) = direct_eval_scope {
                    self.emit_at(
                        IrOp::EvalCall {
                            arguments,
                            scope,
                            environment: None,
                        },
                        source_offset(call_span)?,
                    )?;
                } else {
                    match arguments {
                        CallArguments::Fixed(argument_count) => {
                            let instruction = if member_method || identifier_method {
                                Instruction::CallMethod(argument_count)
                            } else {
                                Instruction::Call(argument_count)
                            };
                            self.emit_instruction_at(instruction, source_offset(call_span)?)?;
                        }
                        CallArguments::Spread => {
                            if member_method || identifier_method {
                                // QuickJS `obj func array -> func obj array`.
                                self.emit_instruction(Instruction::Perm3)?;
                            } else {
                                // QuickJS `func array -> func undefined array`.
                                self.emit_instruction(Instruction::Undefined)?;
                                self.emit_instruction(Instruction::Insert2)?;
                                self.emit_instruction(Instruction::Drop)?;
                            }
                            self.emit_instruction_at(
                                Instruction::Apply(ApplyKind::Call),
                                source_offset(call_span)?,
                            )?;
                        }
                    }
                }
                self.anonymous_function_definition = None;
                continue;
            }
            if optional_chain.is_some() && matches!(self.current().kind, TokenKind::Template(_)) {
                return Err(self.syntax_here("template literal cannot appear in an optional chain"));
            }
            if self.parse_tagged_template_suffix()? {
                continue;
            }
            break;
        }
        if let Some(optional_chain) = optional_chain {
            self.finish_optional_chain(optional_chain)?;
        }
        Ok(())
    }

    /// Lower QuickJS's `get_lvalue(..., keep = TRUE)` followed by one of
    /// `inc`, `dec`, `post_inc`, or `post_dec`, then its matching
    /// `put_lvalue` keep mode. Prefix updates preserve the replacement value;
    /// postfix updates preserve the old, already-converted numeric value.
    fn lower_update_expression(
        &mut self,
        operator_span: Span,
        increment: bool,
        postfix: bool,
    ) -> Result<(), Error> {
        if self.current_ir().last_optional_chain.is_some() {
            return Err(self.syntax_here("invalid increment/decrement operand"));
        }
        let operation = match (postfix, increment) {
            (false, true) => Instruction::Inc,
            (false, false) => Instruction::Dec,
            (true, true) => Instruction::PostInc,
            (true, false) => Instruction::PostDec,
        };

        if let Some(target) = self.promote_tail_identifier_get(IdentifierReferenceAccess::Get)? {
            self.validate_identifier_assignment_target(&target)?;
            self.emit_instruction_at(operation, source_offset(operator_span)?)?;
            if target.object_environment {
                self.emit_identifier_reference_inherited(
                    target.name,
                    target.span,
                    target.scope,
                    if postfix {
                        IdentifierReferenceAccess::PostPut
                    } else {
                        IdentifierReferenceAccess::Set
                    },
                )?;
            } else {
                self.emit_identifier_inherited(
                    target.name,
                    target.span,
                    target.scope,
                    if postfix {
                        IdentifierAccess::Put
                    } else {
                        IdentifierAccess::Set
                    },
                )?;
            }
            self.anonymous_function_definition = None;
            return Ok(());
        }

        let Some(target) = self.promote_tail_member_get_for_compound()? else {
            return Err(self.syntax_here("invalid increment/decrement operand"));
        };
        self.emit_instruction_at(operation, source_offset(operator_span)?)?;
        self.anonymous_function_definition = None;
        if postfix {
            self.emit_member_post_put(target)
        } else {
            self.emit_member_put(target)
        }
    }

    /// Parse one member suffix without accepting a call. This is shared by
    /// ordinary postfix chains and constructor heads after `new`, matching
    /// QuickJS's `PF_POSTFIX_CALL` split.
    fn parse_member_suffix(&mut self) -> Result<bool, Error> {
        if self.is_punctuator(Punctuator::Dot) {
            let member_span = self.current().span;
            self.advance()?;
            let token = self.current().clone();
            let name = match token.kind {
                TokenKind::PrivateIdentifier(identifier) => {
                    let name = private_reference::private_binding_name(&identifier.value);
                    self.advance()?;
                    let operation =
                        self.emit_private_field_get(name, token.span, source_offset(member_span)?)?;
                    self.current_ir_mut().last_member_reference = Some(operation);
                    self.anonymous_function_definition = None;
                    return Ok(true);
                }
                TokenKind::Identifier(identifier) => identifier.value,
                TokenKind::Keyword(keyword) => keyword.as_str().to_owned(),
                _ => return Err(self.syntax_here("expecting field name")),
            };
            self.advance()?;
            let key = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::try_from_utf8(&name)?,
            )))?;
            let operation =
                self.emit_instruction_at(Instruction::GetField(key), source_offset(member_span)?)?;
            self.current_ir_mut().last_member_reference = Some(operation);
            self.anonymous_function_definition = None;
            return Ok(true);
        }

        if self.is_punctuator(Punctuator::LeftBracket) {
            let member_span = self.current().span;
            self.advance()?;
            self.parse_expression()?;
            self.expect_punctuator(Punctuator::RightBracket)?;
            let operation =
                self.emit_instruction_at(Instruction::GetArrayEl, source_offset(member_span)?)?;
            self.current_ir_mut().last_member_reference = Some(operation);
            self.anonymous_function_definition = None;
            return Ok(true);
        }
        Ok(false)
    }

    /// QuickJS rewrites the immediately preceding member getter when `(`
    /// proves that its Reference is being called. The keep form leaves the
    /// original base below the function so `CallMethod` receives the exact
    /// receiver without re-evaluating either base or computed key.
    fn promote_last_member_get_for_call(&mut self) -> Result<bool, Error> {
        let function = self.current_ir_mut();
        if function.last_member_reference != function.ops.len().checked_sub(1) {
            return Ok(false);
        }
        let terminal_get = function.last_member_reference;
        let Some(last) = function.ops.last_mut() else {
            return Ok(false);
        };
        let promoted = match &mut last.op {
            IrOp::Bytecode(instruction @ Instruction::GetField(_)) => {
                let Instruction::GetField(key) = *instruction else {
                    unreachable!();
                };
                *instruction = Instruction::GetField2(key);
                true
            }
            IrOp::Bytecode(instruction @ Instruction::GetArrayEl) => {
                *instruction = Instruction::GetArrayEl2;
                true
            }
            IrOp::Bytecode(instruction @ Instruction::GetSuperValue) => {
                *instruction = Instruction::GetSuperValueForCall;
                true
            }
            IrOp::PrivateField { access, .. } if *access == PrivateFieldAccess::Get => {
                *access = PrivateFieldAccess::GetKeepReceiver;
                true
            }
            _ => false,
        };
        if promoted {
            function.stack_depth = function
                .stack_depth
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
            optional_chain::pad_grouped_method_receiver(function, terminal_get)?;
        }
        function.last_member_reference = None;
        Ok(promoted)
    }

    /// Keep the final identifier read in place while exposing its unresolved
    /// binding metadata to compound-assignment lowering. Parentheses preserve
    /// this marker; every operation that turns the Reference into a value
    /// clears it through `emit_with_site` or the composing parser level.
    fn promote_tail_identifier_get(
        &mut self,
        reference_access: IdentifierReferenceAccess,
    ) -> Result<Option<IdentifierReference>, Error> {
        let function_id = self.current_function;
        let function = self.current_ir();
        if function.last_identifier_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        let Some(SpannedIrOp {
            op:
                IrOp::Identifier {
                    name,
                    span,
                    scope,
                    access: IdentifierAccess::Get,
                },
            ..
        }) = function.ops.last()
        else {
            return Err(Error::internal(
                "identifier Reference marker did not point to a getter",
            ));
        };
        let object_environment = self.parser_scope_has_authored_with(function_id, *scope)?;
        let reference = IdentifierReference {
            name: name.clone(),
            span: *span,
            scope: *scope,
            object_environment,
        };
        let function = self.current_ir_mut();
        function.last_identifier_reference = None;
        if object_environment {
            let Some(SpannedIrOp { op, .. }) = function.ops.last_mut() else {
                return Err(Error::internal(
                    "identifier Reference marker did not point to a getter",
                ));
            };
            *op = IrOp::IdentifierReference {
                name: reference.name.clone(),
                span: reference.span,
                scope: reference.scope,
                access: reference_access,
            };
            function.stack_depth = function
                .stack_depth
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        }
        Ok(Some(reference))
    }

    fn parser_scope_has_authored_with(
        &self,
        mut function_id: FunctionId,
        mut scope: ScopeId,
    ) -> Result<bool, Error> {
        loop {
            loop {
                let current = self.functions[function_id]
                    .scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("identifier use scope is out of bounds"))?;
                if current.kind == ScopeKind::With {
                    return Ok(true);
                }
                let Some(parent) = current.parent else {
                    break;
                };
                scope = parent;
            }
            let Some(parent) = self.functions[function_id].parent else {
                return Ok(false);
            };
            function_id = parent.function;
            scope = parent.definition_scope;
        }
    }

    /// Consume only the parser marker for a syntactic direct-eval callee. The
    /// getter itself deliberately remains an ordinary Identifier operation so
    /// `EvalCall` retains QuickJS's undefined-receiver fallback when the
    /// resolved function is not the realm's original `%eval%`.
    fn take_direct_eval_scope(&mut self) -> Result<Option<ScopeId>, Error> {
        let function = self.current_ir_mut();
        if function.last_identifier_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        let Some(SpannedIrOp {
            op:
                IrOp::Identifier {
                    name,
                    scope,
                    access: IdentifierAccess::Get,
                    ..
                },
            ..
        }) = function.ops.last()
        else {
            return Err(Error::internal(
                "identifier Reference marker did not point to a getter",
            ));
        };
        if name != "eval" {
            return Ok(None);
        }
        let scope = *scope;
        function.last_identifier_reference = None;
        Ok(Some(scope))
    }

    /// Turn the final getter into a base-only Reference preparation for `=`.
    /// Both operations push one abstract value, so no stack-depth correction
    /// is needed while the late resolver decides whether that value is a
    /// selected object or the static `undefined` sentinel.
    fn take_tail_identifier_reference(&mut self) -> Result<Option<IdentifierReference>, Error> {
        let function_id = self.current_function;
        let function = self.current_ir();
        if function.last_identifier_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        let Some(SpannedIrOp {
            op:
                IrOp::Identifier {
                    name,
                    span,
                    scope,
                    access: IdentifierAccess::Get,
                },
            ..
        }) = function.ops.last()
        else {
            return Err(Error::internal(
                "identifier Reference operation disappeared",
            ));
        };
        let object_environment = self.parser_scope_has_authored_with(function_id, *scope)?;
        let reference = IdentifierReference {
            name: name.clone(),
            span: *span,
            scope: *scope,
            object_environment,
        };
        let function = self.current_ir_mut();
        function.last_identifier_reference = None;
        if object_environment {
            let Some(SpannedIrOp { op, .. }) = function.ops.last_mut() else {
                return Err(Error::internal(
                    "identifier Reference operation disappeared",
                ));
            };
            *op = IrOp::IdentifierReference {
                name: reference.name.clone(),
                span: reference.span,
                scope: reference.scope,
                access: IdentifierReferenceAccess::Prepare,
            };
        } else {
            function
                .ops
                .pop()
                .ok_or_else(|| Error::internal("identifier Reference operation disappeared"))?;
            function.stack_depth = function.stack_depth.checked_sub(1).ok_or_else(|| {
                Error::internal("identifier lvalue removal underflowed the stack")
            })?;
        }
        Ok(Some(reference))
    }

    /// Remove the final getter while leaving its already-evaluated base/key
    /// operands on the abstract stack. This mirrors QuickJS `get_lvalue` and
    /// is shared by assignment and `delete` rewrites.
    fn take_tail_member_reference(&mut self) -> Result<Option<MemberReference>, Error> {
        let function = self.current_ir_mut();
        if function.last_member_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        function.last_member_reference = None;
        let SpannedIrOp { op, pc_site } = function
            .ops
            .pop()
            .ok_or_else(|| Error::internal("member Reference operation disappeared"))?;
        let site = pc_site.ok_or_else(|| Error::internal("member getter has no source site"))?;
        match op {
            IrOp::Bytecode(Instruction::GetField(key)) => {
                Ok(Some(MemberReference::Field { key, site }))
            }
            IrOp::Bytecode(Instruction::GetArrayEl) => {
                // Removing a 2 -> 1 getter restores the raw `[base, key]`
                // operands produced by the preceding IR.
                function.stack_depth = function
                    .stack_depth
                    .checked_add(1)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
                Ok(Some(MemberReference::Computed { site }))
            }
            IrOp::Bytecode(Instruction::GetSuperValue) => {
                // Removing a 3 -> 1 getter restores the authenticated method
                // receiver, frozen super base, and raw property key.
                function.stack_depth = function
                    .stack_depth
                    .checked_add(2)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
                Ok(Some(MemberReference::Super { site }))
            }
            IrOp::PrivateField {
                name,
                span,
                scope,
                access: PrivateFieldAccess::Get,
            } => Ok(Some(MemberReference::Private {
                name,
                span,
                scope,
                site,
            })),
            _ => Err(Error::internal(
                "member Reference marker did not point to a getter",
            )),
        }
    }

    /// Keep both the lvalue operands and the old value for compound
    /// assignment. The computed form also retains the already-converted key,
    /// exactly matching QuickJS `get_array_el3`.
    fn promote_tail_member_get_for_compound(&mut self) -> Result<Option<MemberReference>, Error> {
        let super_site = {
            let function = self.current_ir();
            if function.last_member_reference == function.ops.len().checked_sub(1) {
                function.ops.last().and_then(|last| {
                    matches!(last.op, IrOp::Bytecode(Instruction::GetSuperValue))
                        .then_some(last.pc_site)
                        .flatten()
                })
            } else {
                None
            }
        };
        if let Some(site) = super_site {
            {
                let function = self.current_ir_mut();
                function.last_member_reference = None;
                let last = function
                    .ops
                    .last_mut()
                    .ok_or_else(|| Error::internal("super Reference operation disappeared"))?;
                last.op = IrOp::Bytecode(Instruction::ToPropKey);
                last.pc_site = None;
                // Replacing 3 -> 1 with 1 -> 1 restores the three Reference
                // operands before QuickJS's dup3/get_super_value keep form.
                function.stack_depth = function
                    .stack_depth
                    .checked_add(2)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
            }
            self.emit_instruction(Instruction::Dup3)?;
            self.emit_instruction_at(Instruction::GetSuperValue, site)?;
            return Ok(Some(MemberReference::Super { site }));
        }

        let function = self.current_ir_mut();
        if function.last_member_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        function.last_member_reference = None;
        let last = function
            .ops
            .last_mut()
            .ok_or_else(|| Error::internal("member Reference operation disappeared"))?;
        let site = last
            .pc_site
            .ok_or_else(|| Error::internal("member getter has no source site"))?;
        let (target, extra_depth) = match &mut last.op {
            IrOp::Bytecode(instruction @ Instruction::GetField(_)) => {
                let Instruction::GetField(key) = *instruction else {
                    unreachable!();
                };
                *instruction = Instruction::GetField2(key);
                (MemberReference::Field { key, site }, 1)
            }
            IrOp::Bytecode(instruction @ Instruction::GetArrayEl) => {
                *instruction = Instruction::GetArrayEl3;
                (MemberReference::Computed { site }, 2)
            }
            IrOp::PrivateField {
                name,
                span,
                scope,
                access,
            } if *access == PrivateFieldAccess::Get => {
                *access = PrivateFieldAccess::GetKeepReceiver;
                (
                    MemberReference::Private {
                        name: name.clone(),
                        span: *span,
                        scope: *scope,
                        site,
                    },
                    1,
                )
            }
            _ => {
                return Err(Error::internal(
                    "member Reference marker did not point to a getter",
                ));
            }
        };
        function.stack_depth = function
            .stack_depth
            .checked_add(extra_depth)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        Ok(Some(target))
    }

    fn emit_member_put(&mut self, target: MemberReference) -> Result<(), Error> {
        match target {
            MemberReference::Field { key, .. } => {
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::PutField(key))?;
            }
            MemberReference::Computed { .. } => {
                self.emit_instruction(Instruction::Insert3)?;
                self.emit_instruction(Instruction::PutArrayEl)?;
            }
            MemberReference::Super { .. } => {
                self.emit_instruction(Instruction::Insert4)?;
                self.emit_instruction(Instruction::PutSuperValue)?;
            }
            MemberReference::Private {
                name,
                span,
                scope,
                site,
            } => {
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_private_field_operation(
                    name,
                    span,
                    scope,
                    PrivateFieldAccess::Put,
                    site,
                )?;
            }
        }
        Ok(())
    }

    /// QuickJS `PUT_LVALUE_KEEP_SECOND`: move the old numeric value below the
    /// kept member Reference, then consume the Reference and replacement.
    fn emit_member_post_put(&mut self, target: MemberReference) -> Result<(), Error> {
        match target {
            MemberReference::Field { key, .. } => {
                self.emit_instruction(Instruction::Perm3)?;
                self.emit_instruction(Instruction::PutField(key))?;
            }
            MemberReference::Computed { .. } => {
                self.emit_instruction(Instruction::Perm4)?;
                self.emit_instruction(Instruction::PutArrayEl)?;
            }
            MemberReference::Super { .. } => {
                self.emit_instruction(Instruction::Perm5)?;
                self.emit_instruction(Instruction::PutSuperValue)?;
            }
            MemberReference::Private {
                name,
                span,
                scope,
                site,
            } => {
                self.emit_instruction(Instruction::Perm3)?;
                self.emit_private_field_operation(
                    name,
                    span,
                    scope,
                    PrivateFieldAccess::Put,
                    site,
                )?;
            }
        }
        Ok(())
    }

    /// Parse the contents of an already-consumed call/construct `(`.
    ///
    /// This mirrors QuickJS's two-phase lowering. Fixed arguments remain
    /// directly on the operand stack. The first spread converts the fixed
    /// prefix into a fresh dense Array plus a dynamic index; subsequent
    /// values use `DefineArrayEl` and subsequent spreads use `Append`.
    fn parse_call_arguments(&mut self) -> Result<CallArguments, Error> {
        let mut argument_count = 0_usize;
        while !self.is_punctuator(Punctuator::RightParen) {
            // QuickJS accepts 65,535 encoded fixed arguments and only rejects
            // the next parser iteration. If that next token is `...`, the
            // guard still wins before spread lowering begins.
            if argument_count >= MAX_CALL_ARGUMENTS {
                return Err(self.syntax_here("Too many call arguments"));
            }
            if self.is_punctuator(Punctuator::Ellipsis) {
                break;
            }
            // Call arguments use AssignmentExpression with `in` enabled,
            // even when the surrounding expression is a classic-for NoIn
            // initializer.
            self.parse_assignment_allow_in()?;
            argument_count += 1;
            if self.is_punctuator(Punctuator::RightParen) {
                break;
            }
            // A trailing comma advances directly to `)` and ends the loop.
            self.expect_punctuator(Punctuator::Comma)?;
        }

        if !self.is_punctuator(Punctuator::Ellipsis) {
            self.expect_punctuator(Punctuator::RightParen)?;
            return u16::try_from(argument_count)
                .map(CallArguments::Fixed)
                .map_err(|_| Error::internal("call argument count escaped the parser limit"));
        }

        let prefix = u16::try_from(argument_count)
            .map_err(|_| Error::internal("spread argument prefix escaped the parser limit"))?;
        self.emit_instruction(Instruction::ArrayFrom(prefix))?;
        self.emit_instruction(Instruction::PushI32(
            i32::try_from(argument_count)
                .map_err(|_| Error::internal("spread argument index does not fit i32"))?,
        ))?;

        // Once spread lowering begins, QuickJS no longer applies the encoded
        // fixed-call u16 count guard: the runtime Array length guard is the
        // authoritative 65,534-argument boundary.
        while !self.is_punctuator(Punctuator::RightParen) {
            if self.is_punctuator(Punctuator::Ellipsis) {
                self.advance_expression_start()?;
                self.parse_assignment_allow_in()?;
                self.emit_instruction(Instruction::Append)?;
            } else {
                self.parse_assignment_allow_in()?;
                self.emit_instruction(Instruction::DefineArrayEl)?;
                self.emit_instruction(Instruction::Inc)?;
            }
            if self.is_punctuator(Punctuator::RightParen) {
                break;
            }
            self.expect_punctuator(Punctuator::Comma)?;
        }
        self.expect_punctuator(Punctuator::RightParen)?;
        self.emit_instruction(Instruction::Drop)?;
        Ok(CallArguments::Spread)
    }

    fn parse_new_expression(&mut self) -> Result<(), Error> {
        let new_span = self.current().span;
        self.advance()?;
        if self.consume_punctuator(Punctuator::Dot)? {
            let token = self.current().clone();
            let TokenKind::Identifier(identifier) = token.kind else {
                return Err(self.syntax_here("expecting target"));
            };
            if identifier.value != "target" || identifier.has_escape {
                return Err(self.syntax_here("expecting target"));
            }
            if matches!(self.current_ir().kind, FunctionKind::Eval(EvalKind::None)) {
                return Err(Error::internal("eval root has no eval kind"));
            }
            if !self.current_new_target_allowed() {
                return Err(Error::syntax(
                    "new.target only allowed within functions",
                    source_span(new_span),
                ));
            }
            self.advance()?;
            if matches!(
                self.current_ir().kind,
                FunctionKind::Arrow | FunctionKind::Eval(EvalKind::Direct)
            ) {
                self.emit_identifier(
                    NEW_TARGET_LOCAL_NAME.to_owned(),
                    new_span,
                    IdentifierAccess::Get,
                )?;
            } else {
                self.emit_instruction(Instruction::PushNewTarget)?;
            }
            self.anonymous_function_definition = None;
            return Ok(());
        }

        // QuickJS parses the constructor head with calls disabled but member
        // suffixes enabled. The following `(` therefore belongs to this `new`,
        // while calls after the completed construction remain postfix calls.
        self.parse_primary(false)?;
        loop {
            if self.is_punctuator(Punctuator::OptionalChain) {
                return Err(self.syntax_here("new keyword cannot be used with an optional chain"));
            }
            if self.parse_member_suffix()? {
                continue;
            }
            if self.parse_tagged_template_suffix()? {
                continue;
            }
            break;
        }
        self.emit_instruction(Instruction::Dup)?;
        let no_arguments_span = self.current().span;
        let (arguments, construct_span) = if self.is_punctuator(Punctuator::LeftParen) {
            let call_span = self.current().span;
            self.advance()?;
            (self.parse_call_arguments()?, call_span)
        } else {
            (CallArguments::Fixed(0), no_arguments_span)
        };
        match arguments {
            CallArguments::Fixed(argument_count) => {
                self.emit_instruction_at(
                    Instruction::Construct(argument_count),
                    source_offset(construct_span)?,
                )?;
            }
            CallArguments::Spread => {
                // QuickJS emits this permutation for the shared
                // `object, function, array` apply ABI. For ordinary `new`,
                // object and function are the same value produced by `Dup`.
                self.emit_instruction(Instruction::Perm3)?;
                self.emit_instruction_at(
                    Instruction::Apply(ApplyKind::Construct),
                    source_offset(construct_span)?,
                )?;
            }
        }
        self.anonymous_function_definition = None;
        Ok(())
    }

    /// Parse the ObjectLiteral-method SuperProperty subset with the same
    /// operand order as QuickJS: lexical `this` and HomeObject are fixed
    /// before a computed key expression begins. Arrows relay both authenticated
    /// pseudo bindings through ordinary closure slots.
    fn parse_super_property(&mut self, super_span: Span) -> Result<(), Error> {
        self.advance()?;
        if self.is_punctuator(Punctuator::LeftParen) {
            if !self.current_ir().super_call_allowed {
                return Err(
                    self.syntax_here("super() is only valid in a derived class constructor")
                );
            }
            let call_span = self.current().span;
            // Match QuickJS's `this_active_func; get_super; new.target`
            // sequence before argument evaluation. A parameter expression may
            // mutate the derived constructor's [[Prototype]], but that mutation
            // affects only a later super() call.
            self.emit_identifier(
                ACTIVE_FUNCTION_LOCAL_NAME.to_owned(),
                super_span,
                IdentifierAccess::Get,
            )?;
            self.emit_instruction(Instruction::GetSuper)?;
            self.emit_identifier(
                NEW_TARGET_LOCAL_NAME.to_owned(),
                super_span,
                IdentifierAccess::Get,
            )?;
            self.emit_instruction(Instruction::MarkSuperCall)?;
            self.advance()?;
            let arguments = self.parse_call_arguments()?;
            match arguments {
                CallArguments::Fixed(argument_count) => {
                    self.emit_instruction_at(
                        Instruction::ConstructSuper(argument_count),
                        source_offset(call_span)?,
                    )?;
                }
                CallArguments::Spread => {
                    self.emit_instruction_at(Instruction::ApplySuper, source_offset(call_span)?)?;
                }
            }
            self.emit_instruction(Instruction::Dup)?;
            self.emit_identifier(
                THIS_LOCAL_NAME.to_owned(),
                super_span,
                IdentifierAccess::InitializeDerivedThis,
            )?;
            self.emit_identifier(
                ACTIVE_FUNCTION_LOCAL_NAME.to_owned(),
                super_span,
                IdentifierAccess::Get,
            )?;
            self.emit_instruction(Instruction::CallClassInstanceInitializer)?;
            self.anonymous_function_definition = None;
            return Ok(());
        }
        if !matches!(
            self.current().kind,
            TokenKind::Punctuator(Punctuator::Dot | Punctuator::LeftBracket)
        ) {
            return Err(Error::syntax(
                "invalid use of 'super'",
                source_span(super_span),
            ));
        }

        if !self.current_ir().super_allowed {
            return Err(Error::syntax(
                "'super' is only valid in a method",
                source_span(super_span),
            ));
        }

        self.emit_identifier(
            THIS_LOCAL_NAME.to_owned(),
            super_span,
            IdentifierAccess::Get,
        )?;
        self.emit_identifier(
            HOME_OBJECT_LOCAL_NAME.to_owned(),
            super_span,
            IdentifierAccess::Get,
        )?;
        self.emit_instruction(Instruction::GetSuper)?;

        let member_span = self.current().span;
        if self.is_punctuator(Punctuator::Dot) {
            self.advance()?;
            let token = self.current().clone();
            let name = match token.kind {
                TokenKind::PrivateIdentifier(_) => {
                    return Err(Error::syntax(
                        "private class field forbidden after super",
                        source_span(token.span),
                    ));
                }
                TokenKind::Identifier(identifier) => identifier.value,
                TokenKind::Keyword(keyword) => keyword.as_str().to_owned(),
                _ => return Err(self.syntax_here("expecting field name")),
            };
            self.advance()?;
            let key = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::try_from_utf8(&name)?,
            )))?;
            self.emit(IrOp::PushConstant(key))?;
        } else {
            self.advance_expression_start()?;
            self.parse_expression()?;
            self.expect_punctuator(Punctuator::RightBracket)?;
        }
        let operation =
            self.emit_instruction_at(Instruction::GetSuperValue, source_offset(member_span)?)?;
        self.current_ir_mut().last_member_reference = Some(operation);
        self.anonymous_function_definition = None;
        Ok(())
    }

    /// Parse and lower the Script/Eval import-expression grammar.
    fn parse_import_expression(
        &mut self,
        import_span: Span,
        import_call_allowed: bool,
    ) -> Result<(), Error> {
        self.advance()?;
        if self.is_punctuator(Punctuator::Dot) {
            self.advance()?;
            let exact_meta = matches!(
                &self.current().kind,
                TokenKind::Identifier(identifier)
                    if identifier.value == "meta" && !identifier.has_escape
            );
            if !exact_meta {
                return Err(self.syntax_here("meta expected"));
            }
            if self.module.is_none() {
                return Err(self.syntax_here("import.meta only valid in module code"));
            }
            self.advance()?;
            self.ensure_module_import_meta_binding()?;
            self.emit_at(
                IrOp::ImportMeta {
                    span: import_span,
                    scope: self.current_ir().current_scope,
                },
                source_offset(import_span)?,
            )?;
            self.anonymous_function_definition = None;
            return Ok(());
        }

        self.expect_punctuator(Punctuator::LeftParen)?;
        if !import_call_allowed {
            return Err(self.syntax_here("invalid use of 'import()'"));
        }

        // ImportCall accepts one required AssignmentExpression and at most one
        // options AssignmentExpression. Spread is not part of this grammar;
        // ordinary expression parsing therefore supplies QuickJS's exact
        // unexpected-token diagnostics for `import()` and `import(...x)`.
        self.parse_assignment_allow_in()?;
        if self.is_punctuator(Punctuator::Comma) {
            self.advance()?;
            if !self.is_punctuator(Punctuator::RightParen) {
                self.parse_assignment_allow_in()?;
                if self.is_punctuator(Punctuator::Comma) {
                    self.advance()?;
                }
            } else {
                self.emit_instruction(Instruction::Undefined)?;
            }
        } else {
            self.emit_instruction(Instruction::Undefined)?;
        }
        self.expect_punctuator(Punctuator::RightParen)?;

        self.emit_instruction_at(Instruction::Import, source_offset(import_span)?)?;
        self.anonymous_function_definition = None;
        Ok(())
    }

    fn parse_primary(&mut self, import_call_allowed: bool) -> Result<(), Error> {
        // QuickJS initially tokenizes a leading slash as `/` or `/=`, then
        // rewinds from `js_parse_postfix_expr` once the grammar has proved
        // that the current position requires a PrimaryExpression.  Keep the
        // same parser-owned decision here: operator parsing consumes genuine
        // division before it can reach this function, while a slash which is
        // asked to begin an operand is rescanned as one complete RegExp token.
        if matches!(
            self.current().kind,
            TokenKind::Punctuator(Punctuator::Divide | Punctuator::DivideAssign)
        ) {
            self.relex_current_with_goal(LexicalGoal::RegExp)?;
        }
        let token = self.current().clone();
        self.anonymous_function_definition = None;
        match token.kind {
            TokenKind::Keyword(Keyword::Null) => {
                self.advance()?;
                self.emit_instruction(Instruction::Null)?;
            }
            TokenKind::Keyword(Keyword::False) => {
                self.advance()?;
                self.emit_instruction(Instruction::PushFalse)?;
            }
            TokenKind::Keyword(Keyword::True) => {
                self.advance()?;
                self.emit_instruction(Instruction::PushTrue)?;
            }
            TokenKind::Keyword(Keyword::This) => {
                self.advance()?;
                if self.current_ir().derived_class_constructor
                    || matches!(
                        self.current_ir().kind,
                        FunctionKind::Arrow | FunctionKind::Eval(EvalKind::Direct)
                    )
                {
                    self.emit_identifier(
                        THIS_LOCAL_NAME.to_owned(),
                        token.span,
                        IdentifierAccess::Get,
                    )?;
                } else {
                    self.emit_instruction(Instruction::PushThis)?;
                }
            }
            TokenKind::Number(number) => {
                if self.current_ir().strict
                    && matches!(
                        number.kind,
                        NumberKind::LegacyOctal | NumberKind::LegacyDecimal
                    )
                {
                    return Err(Error::syntax(
                        "legacy leading-zero numeric literals are forbidden in strict mode",
                        source_span(token.span),
                    ));
                }
                self.advance()?;
                let value = parse_number(&number)
                    .map_err(|message| Error::syntax(message, source_span(token.span)))?;
                self.emit_value(value)?;
            }
            TokenKind::String(string) => {
                if self.current_ir().strict && string.has_legacy_octal_escape {
                    return Err(Error::syntax(
                        "legacy octal escapes are forbidden in strict mode",
                        source_span(token.span),
                    ));
                }
                self.advance()?;
                self.emit_atom_string(JsString::try_from_utf16(string.value.utf16)?)?;
            }
            TokenKind::Punctuator(Punctuator::LeftParen) => {
                self.advance()?;
                self.parse_expression()?;
                self.expect_punctuator(Punctuator::RightParen)?;
            }
            TokenKind::Punctuator(Punctuator::LeftBrace) => {
                self.parse_object_literal()?;
            }
            TokenKind::Punctuator(Punctuator::LeftBracket) => {
                self.parse_array_literal()?;
            }
            TokenKind::Template(_) => {
                self.parse_template_literal()?;
            }
            TokenKind::Identifier(ref identifier)
                if identifier.value == "async"
                    && !identifier.has_escape
                    && self.async_function_ahead() =>
            {
                self.parse_function_expression()?;
            }
            TokenKind::Identifier(identifier) => {
                self.reject_forbidden_identifier_reference(&identifier.value, token.span)?;
                validate_identifier(
                    &identifier,
                    token.span,
                    self.current_ir().strict,
                    IdentifierContext::Reference,
                )?;
                self.advance()?;
                let operation =
                    self.emit_identifier(identifier.value, token.span, IdentifierAccess::Get)?;
                self.current_ir_mut().last_identifier_reference = Some(operation);
            }
            TokenKind::Keyword(Keyword::Function) => {
                self.parse_function_expression()?;
            }
            TokenKind::Keyword(Keyword::Class) => {
                self.parse_class_expression()?;
            }
            TokenKind::Keyword(Keyword::New) => {
                self.parse_new_expression()?;
            }
            TokenKind::Keyword(Keyword::Super) => {
                self.parse_super_property(token.span)?;
            }
            TokenKind::Keyword(
                keyword @ (Keyword::Const
                | Keyword::Enum
                | Keyword::Export
                | Keyword::Extends
                | Keyword::Var),
            ) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    keyword.as_str()
                )));
            }
            TokenKind::Keyword(Keyword::Import) => {
                self.parse_import_expression(token.span, import_call_allowed)?;
            }
            TokenKind::Keyword(Keyword::Yield)
                if matches!(
                    self.current_ir().execution_kind,
                    BytecodeFunctionKind::Generator | BytecodeFunctionKind::AsyncGenerator
                ) =>
            {
                return Err(self.syntax_here("unexpected token in expression: 'yield'"));
            }
            TokenKind::Keyword(keyword)
                if self.current_ir().strict && strict_reserved_identifier(keyword) =>
            {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    keyword.as_str()
                )));
            }
            TokenKind::Keyword(
                keyword @ (Keyword::If
                | Keyword::For
                | Keyword::Else
                | Keyword::In
                | Keyword::Case
                | Keyword::Default
                | Keyword::Catch
                | Keyword::Finally
                | Keyword::Debugger),
            ) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    keyword.as_str()
                )));
            }
            TokenKind::Keyword(keyword) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    keyword.as_str()
                )));
            }
            TokenKind::RegExp(literal) => {
                // Pinned QuickJS calls `compile_regexp` before advancing to the
                // next token. Preserve that diagnostic precedence and retain
                // the resulting program in immutable bytecode so evaluation
                // only allocates a fresh branded object.
                let pattern_start = token
                    .span
                    .start
                    .byte_offset
                    .checked_add(1)
                    .ok_or_else(|| Error::internal("regexp source range overflowed"))?;
                let pattern_end = pattern_start
                    .checked_add(literal.pattern.len())
                    .ok_or_else(|| Error::internal("regexp source range overflowed"))?;
                let pattern = self
                    .lexer
                    .source_range_to_js_string(pattern_start..pattern_end)?
                    .ok_or_else(|| Error::internal("regexp source range is invalid"))?;
                let flags = JsString::try_from_utf8(literal.flags)?;
                let program = crate::regexp::compile(&pattern, &flags).map_err(|error| {
                    let kind = crate::regexp::javascript_compile_error_kind(&error);
                    let message =
                        crate::regexp::javascript_compile_error_message(&error).to_owned();
                    Error::new(kind, message).with_span(source_span(token.span))
                })?;
                let constant = self.add_constant(IrConstant::RegExp {
                    pattern,
                    program: Rc::new(program),
                })?;
                self.emit_instruction_at(
                    Instruction::RegExp(constant),
                    source_offset(token.span)?,
                )?;
                self.advance()?;
            }
            TokenKind::PrivateIdentifier(identifier) => {
                let mut message = NativeErrorMessage::new();
                message.push_utf8("unexpected token in expression: '");
                message.push_utf8(identifier.raw);
                message.push_utf8("'");
                return Err(Error::from_native_message(ErrorKind::Syntax, message)
                    .with_span(source_span(token.span)));
            }
            TokenKind::Punctuator(punctuator) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    punctuator.as_str()
                )));
            }
            // QuickJS gives a raw NUL token an empty printable spelling. Do
            // not embed the byte itself in the diagnostic: the later native
            // error C-string boundary would truncate the closing quote.
            TokenKind::RawAscii(0) | TokenKind::Eof => {
                return Err(self.syntax_here("unexpected token in expression: ''"));
            }
            TokenKind::RawAscii(byte) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    char::from(byte)
                )));
            }
        }
        Ok(())
    }

    fn reject_forbidden_identifier_reference(&self, name: &str, span: Span) -> Result<(), Error> {
        if name == "arguments" && self.current_ir().arguments_forbidden {
            return Err(Error::syntax(
                "'arguments' identifier is not allowed in class field initializer",
                source_span(span),
            ));
        }
        Ok(())
    }

    /// Lower an Array literal with the same three phases as QuickJS
    /// `js_parse_array_literal`: a dense prefix carried by `ArrayFrom`, fixed
    /// post-prefix elements defined by atom, then a dynamic-index tail for
    /// holes and spread.  Element grammar always admits `in`, even when the
    /// literal occurs in an enclosing ExpressionNoIn.
    fn parse_array_literal(&mut self) -> Result<(), Error> {
        const DENSE_PREFIX_LIMIT: u32 = 32;
        const MAX_STATIC_INDEX: u32 = i32::MAX as u32;

        if !self.is_punctuator(Punctuator::LeftBracket) {
            return Err(self.syntax_here("expecting '['"));
        }
        self.advance_expression_start()?;
        let mut index = 0_u32;

        // QuickJS keeps the common small dense case entirely on the operand
        // stack and lets one ArrayFrom operation consume the prefix values.
        while !self.is_punctuator(Punctuator::RightBracket) && index < DENSE_PREFIX_LIMIT {
            if self.is_punctuator(Punctuator::Comma) || self.is_punctuator(Punctuator::Ellipsis) {
                break;
            }
            self.parse_assignment_allow_in()?;
            index += 1;
            if self.is_punctuator(Punctuator::Comma) {
                self.advance_expression_start()?;
                // A comma immediately before `]` is trailing, not a hole.
            } else if !self.is_punctuator(Punctuator::RightBracket) {
                self.expect_punctuator(Punctuator::RightBracket)?;
            }
        }
        self.emit_instruction(Instruction::ArrayFrom(u16::try_from(index).map_err(
            |_| Error::internal("dense Array literal prefix does not fit u16"),
        )?))?;

        // Holes and elements after the dense prefix retain the Array on the
        // stack. A final hole needs an explicit length write because no later
        // indexed definition extends the exotic length for it.
        let mut need_length = false;
        while !self.is_punctuator(Punctuator::RightBracket) && index < MAX_STATIC_INDEX {
            if self.is_punctuator(Punctuator::Ellipsis) {
                break;
            }
            need_length = true;
            if !self.is_punctuator(Punctuator::Comma) {
                self.parse_assignment_allow_in()?;
                let key = self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::try_from_utf8(&index.to_string())?,
                )))?;
                self.emit_instruction(Instruction::DefineField(key))?;
                need_length = false;
            }
            index += 1;
            if self.is_punctuator(Punctuator::Comma) {
                self.advance_expression_start()?;
                // Continue with the next element or trailing hole.
            } else if !self.is_punctuator(Punctuator::RightBracket) {
                self.expect_punctuator(Punctuator::RightBracket)?;
            }
        }

        if self.is_punctuator(Punctuator::RightBracket) {
            if need_length {
                let length = self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::from_static("length"),
                )))?;
                self.emit_instruction(Instruction::Dup)?;
                self.emit_instruction(Instruction::PushI32(
                    i32::try_from(index)
                        .map_err(|_| Error::internal("Array literal index does not fit i32"))?,
                ))?;
                self.emit_instruction(Instruction::PutField(length))?;
            }
            self.expect_punctuator(Punctuator::RightBracket)?;
            self.anonymous_function_definition = None;
            return Ok(());
        }

        // A spread, or the static-index boundary, switches to a runtime index
        // kept immediately above the Array. DefineArrayEl and Append preserve
        // both operands so the parser can continue without synthetic locals.
        self.emit_instruction(Instruction::PushI32(
            i32::try_from(index)
                .map_err(|_| Error::internal("Array literal index does not fit i32"))?,
        ))?;
        while !self.is_punctuator(Punctuator::RightBracket) {
            if self.is_punctuator(Punctuator::Ellipsis) {
                self.advance_expression_start()?;
                self.parse_assignment_allow_in()?;
                self.emit_instruction(Instruction::Append)?;
            } else {
                need_length = true;
                if !self.is_punctuator(Punctuator::Comma) {
                    self.parse_assignment_allow_in()?;
                    self.emit_instruction(Instruction::DefineArrayEl)?;
                    need_length = false;
                }
                self.emit_instruction(Instruction::Inc)?;
            }

            if !self.is_punctuator(Punctuator::Comma) {
                break;
            }
            self.advance_expression_start()?;
        }

        if need_length {
            let length = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::from_static("length"),
            )))?;
            self.emit_instruction(Instruction::Dup1)?;
            self.emit_instruction(Instruction::PutField(length))?;
        } else {
            self.emit_instruction(Instruction::Drop)?;
        }
        self.expect_punctuator(Punctuator::RightBracket)?;
        self.anonymous_function_definition = None;
        Ok(())
    }

    fn parse_program_function_declaration(&mut self) -> Result<(), Error> {
        let parsed = self.parse_function_definition(true, false)?;
        let (name, declaration_span) = parsed
            .name
            .ok_or_else(|| Error::internal("required Program function lost its name"))?;
        let function = &mut self.functions[self.current_function];
        if !matches!(function.kind, FunctionKind::Script) {
            return Err(Error::internal(
                "Program function declaration escaped the root script",
            ));
        }

        // QuickJS appends one GLOBAL_FUNCTION_DECL record per syntax node,
        // including duplicates. It deliberately does not run the ordinary
        // `define_var` conflict check here, which permits a preceding Program
        // lexical with the same name.
        function.global_declarations.push(IrGlobalDeclaration {
            name: name.clone(),
            is_lexical: false,
            is_const: false,
            function_constant: Some(parsed.constant),
            closure_index: None,
        });
        if function
            .binding_in_scope(function.var_scope, &name)
            .is_none()
        {
            function.add_binding(
                function.var_scope,
                function.current_scope,
                name,
                BindingStorage::Global,
                BindingKind::Normal,
                Some(declaration_span),
            );
        }
        Ok(())
    }

    fn parse_eval_program_function_declaration(&mut self) -> Result<(), Error> {
        let parsed = self.parse_function_definition(true, false)?;
        let (name, declaration_span) = parsed
            .name
            .ok_or_else(|| Error::internal("required eval function lost its name"))?;
        if !matches!(self.current_ir().kind, FunctionKind::Eval(_)) {
            return Err(Error::internal(
                "eval function declaration escaped its synthetic root",
            ));
        }
        let conflict_span = self.current().span;
        if self
            .current_ir()
            .binding_id_in_scope(self.current_ir().body_scope, &name)
            .is_some_and(|binding| {
                matches!(
                    self.current_ir().bindings[binding.0].kind,
                    BindingKind::Lexical { .. }
                )
            })
        {
            return Err(Error::syntax(
                "invalid redefinition of lexical identifier",
                source_span(conflict_span),
            ));
        }

        match self.current_eval_declaration_mode()? {
            EvalDeclarationMode::Global => {
                let function = self.current_ir_mut();
                function.global_declarations.push(IrGlobalDeclaration {
                    name: name.clone(),
                    is_lexical: false,
                    is_const: false,
                    function_constant: Some(parsed.constant),
                    closure_index: None,
                });
                let caller_lexical_conflict = function
                    .external_bindings
                    .iter()
                    .find(|binding| binding.name.to_utf8_lossy() == name)
                    .is_some_and(|binding| binding.is_lexical && !binding.is_catch_parameter);
                if caller_lexical_conflict && function.eval_redeclaration.is_none() {
                    function.eval_redeclaration = Some(name.clone());
                }
                let has_global = function.scopes[function.var_scope.0]
                    .bindings
                    .iter()
                    .copied()
                    .any(|binding| {
                        let binding = &function.bindings[binding.0];
                        binding.name == name && binding.storage == BindingStorage::Global
                    });
                if !has_global {
                    function.add_binding(
                        function.var_scope,
                        function.current_scope,
                        name,
                        BindingStorage::Global,
                        BindingKind::Normal,
                        Some(declaration_span),
                    );
                }
            }
            EvalDeclarationMode::Local => {
                self.register_eval_var_binding(&name, declaration_span, conflict_span)?;
                let function = self.current_ir_mut();
                let binding = function.scopes[function.var_scope.0]
                    .bindings
                    .iter()
                    .rev()
                    .copied()
                    .find(|binding| {
                        let binding = &function.bindings[binding.0];
                        binding.name == name
                            && !matches!(binding.storage, BindingStorage::External(_))
                    })
                    .ok_or_else(|| {
                        Error::internal("eval-local function binding was not registered")
                    })?;
                if let Some(existing) = function
                    .hoisted_functions
                    .iter_mut()
                    .find(|hoist| hoist.binding == binding)
                {
                    existing.constant = parsed.constant;
                } else {
                    function.hoisted_functions.push(IrHoistedFunction {
                        binding,
                        constant: parsed.constant,
                    });
                }
            }
            EvalDeclarationMode::Dynamic(object) => {
                let target = self.eval_dynamic_declaration_target(&name, object, conflict_span)?;
                self.current_ir_mut()
                    .eval_declarations
                    .push(IrEvalDeclaration {
                        name,
                        target,
                        value: EvalDeclarationValue::Function(parsed.constant),
                    });
            }
        }
        Ok(())
    }

    fn parse_function_body_declaration(&mut self) -> Result<(), Error> {
        let parsed = self.parse_function_definition(true, false)?;
        let (name, declaration_span) = parsed
            .name
            .ok_or_else(|| Error::internal("required function declaration lost its name"))?;
        if !matches!(
            self.current_ir().kind,
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
        ) {
            return Err(Error::internal(
                "function-body declaration escaped its ordinary function",
            ));
        }
        let conflict_span = self.current().span;
        self.register_var_binding(&name, declaration_span, conflict_span)?;

        let function = &mut self.functions[self.current_function];
        let binding = function.scopes[function.var_scope.0]
            .bindings
            .iter()
            .rev()
            .copied()
            .find(|binding| function.bindings[binding.0].name == name)
            .ok_or_else(|| Error::internal("function declaration binding was not registered"))?;
        let metadata = &function.bindings[binding.0];
        if metadata.kind != BindingKind::Normal
            || !matches!(
                metadata.storage,
                BindingStorage::Argument(_) | BindingStorage::Local(_)
            )
        {
            return Err(Error::internal(
                "function declaration did not resolve to an ordinary frame binding",
            ));
        }
        if let Some(existing) = function
            .hoisted_functions
            .iter_mut()
            .find(|hoist| hoist.binding == binding)
        {
            existing.constant = parsed.constant;
        } else {
            function.hoisted_functions.push(IrHoistedFunction {
                binding,
                constant: parsed.constant,
            });
        }
        Ok(())
    }

    fn parse_annex_b_function_declaration(&mut self) -> Result<(), Error> {
        let function = self.current_ir();
        let program_body = matches!(function.kind, FunctionKind::Script)
            && function.current_scope == function.body_scope
            && matches!(
                function.scopes[function.current_scope.0].kind,
                ScopeKind::ProgramBody
            );
        if program_body {
            self.parse_program_annex_b_function_declaration()
        } else {
            self.parse_scoped_function_declaration()
        }
    }

    fn parse_program_annex_b_function_declaration(&mut self) -> Result<(), Error> {
        let header = self.parse_function_definition_header(true)?;
        let (name, declaration_span) = header
            .name
            .as_ref()
            .map(|(identifier, span)| (identifier.value.clone(), *span))
            .ok_or_else(|| Error::internal("required Program Annex B function lost its name"))?;
        let conflict_span = self.current().span;

        let (body_scope, var_scope) = {
            let function = self.current_ir();
            if !matches!(function.kind, FunctionKind::Script)
                || function.current_scope != function.body_scope
            {
                return Err(Error::internal(
                    "Program Annex B function escaped the Program body",
                ));
            }
            (function.body_scope, function.var_scope)
        };
        let conflicts_with_authored_global = {
            let function = self.current_ir();
            if let Some(binding) = function.binding_id_in_scope(var_scope, &name) {
                // The root binding retains the first ordinary declaration's
                // scope. A prior nested/Annex declaration therefore masks a
                // later Program lexical in QuickJS's first-global-record
                // lookup, while an authored Program var/function still
                // conflicts here.
                function.bindings[binding.0].declaration_scope == body_scope
            } else {
                function.binding_id_in_scope(body_scope, &name).is_some()
            }
        };
        if conflicts_with_authored_global {
            return Err(Error::syntax(
                "invalid redefinition of global identifier",
                source_span(conflict_span),
            ));
        }

        let parsed = self.parse_function_definition_tail(header, false)?;
        if parsed.name.as_ref().map(|(parsed, _)| parsed.as_str()) != Some(name.as_str()) {
            return Err(Error::internal(
                "Program Annex B function header changed while parsing its child",
            ));
        }
        // QuickJS publishes this synthetic global only after the child has
        // parsed successfully. Deferred tree-wide identifier resolution still
        // lets the child capture the resulting recursive binding.
        let IrAnnexBinding::Static(binding) =
            self.ensure_annex_b_binding(&name, declaration_span)?
        else {
            return Err(Error::internal(
                "Program Annex B declaration targeted dynamic eval storage",
            ));
        };

        let authored_closure = self.emit(IrOp::MakeClosure(parsed.constant))?;
        self.emit_instruction(Instruction::Dup)?;
        self.emit_identifier_inherited(
            name.clone(),
            declaration_span,
            var_scope,
            IdentifierAccess::AnnexBPut,
        )?;
        // `JS_PARSE_FUNC_VAR` performs a second source-position write when the
        // Program-body lexical exception is active. It is observable through
        // pre-existing global accessors, whose setter runs twice.
        self.emit_identifier_inherited(name, declaration_span, body_scope, IdentifierAccess::Put)?;
        self.current_ir_mut()
            .program_annex_functions
            .push(IrProgramAnnexFunction {
                binding,
                constant: parsed.constant,
                authored_closure,
            });
        Ok(())
    }

    fn parse_scoped_function_declaration(&mut self) -> Result<(), Error> {
        let header = self.parse_function_definition_header(true)?;
        let (name, declaration_span) = header
            .name
            .as_ref()
            .map(|(identifier, span)| (identifier.value.clone(), *span))
            .ok_or_else(|| Error::internal("required scoped function lost its name"))?;
        let non_ordinary = header.execution_kind != BytecodeFunctionKind::Normal;
        let prepared = self.prepare_scoped_function(&name, declaration_span, non_ordinary)?;
        let parsed = self.parse_function_definition_tail(header, false)?;
        if parsed.name.as_ref().map(|(parsed, _)| parsed.as_str()) != Some(name.as_str()) {
            return Err(Error::internal(
                "scoped function header changed while parsing its child",
            ));
        }

        let annex_binding = if prepared.create_annex_binding {
            Some(self.ensure_annex_b_binding(&name, declaration_span)?)
        } else {
            None
        };
        let authored_closure = self.emit(IrOp::MakeClosure(parsed.constant))?;
        if annex_binding.is_some() {
            self.emit_instruction(Instruction::Dup)?;
            let root_scope = self.current_ir().var_scope;
            let access = if annex_binding == Some(IrAnnexBinding::Dynamic) {
                IdentifierAccess::Put
            } else {
                IdentifierAccess::AnnexBPut
            };
            self.emit_identifier_inherited(name, declaration_span, root_scope, access)?;
        }
        self.emit_instruction(Instruction::Drop)?;
        self.current_ir_mut()
            .scoped_functions
            .push(IrScopedFunction {
                binding: prepared.binding,
                constant: parsed.constant,
                annex_binding,
                authored_closure,
            });
        Ok(())
    }

    fn prepare_scoped_function(
        &mut self,
        name: &str,
        declaration_span: Span,
        lexical_only: bool,
    ) -> Result<PreparedScopedFunction, Error> {
        let function = self.current_ir();
        let scope_kind = function.scopes[function.current_scope.0].kind;
        let eval_program_body = matches!(function.kind, FunctionKind::Eval(_))
            && function.current_scope == function.body_scope
            && matches!(scope_kind, ScopeKind::ProgramBody);
        if !matches!(
            scope_kind,
            ScopeKind::Block | ScopeKind::If | ScopeKind::Switch | ScopeKind::FunctionBody
        ) && !eval_program_body
        {
            return Err(Error::internal(
                "scoped function escaped an Annex B declaration scope",
            ));
        }
        // Annex B.3.2 applies only to synchronous ordinary
        // FunctionDeclarations. Generator and async declarations remain
        // lexical even in sloppy blocks.
        let create_annex_binding = !lexical_only && self.scoped_function_is_annex_b_eligible(name);
        let conflict_span = self.current().span;
        let binding = self.register_scoped_function_binding(
            name,
            declaration_span,
            conflict_span,
            lexical_only,
        )?;
        Ok(PreparedScopedFunction {
            binding,
            create_annex_binding,
        })
    }

    fn scoped_function_is_annex_b_eligible(&self, name: &str) -> bool {
        let function = self.current_ir();
        if function.strict {
            return false;
        }
        if (matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
            && name == "arguments")
            || (matches!(
                function.kind,
                FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
            ) && function
                .parameter_names
                .iter()
                .any(|parameter| parameter == name))
        {
            return false;
        }

        let mut scope = function.current_scope;
        loop {
            if let Some(binding) = function.binding_in_scope(scope, name)
                && matches!(binding.kind, BindingKind::Lexical { .. })
            {
                // Annex B.3.5 deliberately treats a simple catch parameter as
                // compatible with the synthetic outer `var` introduced for a
                // block FunctionDeclaration. The catch-local lexical remains
                // the function's inner binding; only the eligibility scan
                // skips it while looking for a blocking lexical declaration.
                if binding.is_catch_parameter {
                    let Some(parent) = function.scopes[scope.0].parent else {
                        break;
                    };
                    scope = parent;
                    continue;
                }
                if matches!(function.kind, FunctionKind::Eval(_))
                    && matches!(binding.storage, BindingStorage::External(_))
                {
                    let Some(parent) = function.scopes[scope.0].parent else {
                        break;
                    };
                    scope = parent;
                    continue;
                }
                let masked_program_lexical = matches!(function.kind, FunctionKind::Script)
                    && scope == function.body_scope
                    && matches!(function.scopes[scope.0].kind, ScopeKind::ProgramBody)
                    && function.first_global_declaration_is_normal(name);
                if !masked_program_lexical {
                    return false;
                }
            }
            let Some(parent) = function.scopes[scope.0].parent else {
                break;
            };
            scope = parent;
        }
        true
    }

    fn register_scoped_function_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
        conflict_span: Span,
        lexical_only: bool,
    ) -> Result<BindingId, Error> {
        let scope = self.current_ir().current_scope;
        if let Some(existing) = self.current_ir().binding_id_in_scope(scope, name) {
            let existing = &self.current_ir().bindings[existing.0];
            let duplicate_ordinary_function =
                existing.is_scoped_function && !existing.is_scoped_generator && !lexical_only;
            if self.current_ir().strict || !duplicate_ordinary_function {
                return Err(Error::syntax(
                    "invalid redefinition of lexical identifier",
                    source_span(conflict_span),
                ));
            }

            let function = self.current_ir_mut();
            if function.locals.len() >= MAX_LOCAL_VARIABLES {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(declaration_span)),
                );
            }
            let index = u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
            function.locals.push(name.to_owned());
            let binding = function.add_binding(
                scope,
                scope,
                name.to_owned(),
                BindingStorage::Local(index),
                BindingKind::Lexical { is_const: false },
                Some(declaration_span),
            );
            function.bindings[binding.0].is_scoped_function = true;
            function.bindings[binding.0].is_scoped_generator = lexical_only;
            return Ok(binding);
        }

        self.register_lexical_binding(name, declaration_span, conflict_span, false, true)?;
        let binding = self
            .current_ir()
            .binding_id_in_scope(scope, name)
            .ok_or_else(|| Error::internal("scoped function binding was not registered"))?;
        self.current_ir_mut().bindings[binding.0].is_scoped_function = true;
        self.current_ir_mut().bindings[binding.0].is_scoped_generator = lexical_only;
        Ok(binding)
    }

    fn ensure_annex_b_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
    ) -> Result<IrAnnexBinding, Error> {
        let eval_mode = if matches!(self.current_ir().kind, FunctionKind::Eval(_)) {
            Some(self.current_eval_declaration_mode()?)
        } else {
            None
        };

        if let Some(EvalDeclarationMode::Dynamic(object)) = eval_mode {
            let target = self.eval_dynamic_declaration_target(name, object, declaration_span)?;
            self.current_ir_mut()
                .eval_declarations
                .push(IrEvalDeclaration {
                    name: name.to_owned(),
                    target,
                    value: EvalDeclarationValue::Undefined,
                });
            return match target {
                EvalDeclarationTarget::Dynamic(_) => Ok(IrAnnexBinding::Dynamic),
                EvalDeclarationTarget::External { index, .. } => {
                    let function = self.current_ir();
                    let binding = function.scopes[function.var_scope.0]
                        .bindings
                        .iter()
                        .copied()
                        .find(|binding| {
                            function.bindings[binding.0].storage == BindingStorage::External(index)
                        })
                        .ok_or_else(|| {
                            Error::internal("Annex B external target has no binding identity")
                        })?;
                    Ok(IrAnnexBinding::Static(binding))
                }
            };
        }

        let function = self.current_ir_mut();
        let root = function.var_scope;
        let global = matches!(function.kind, FunctionKind::Script)
            || eval_mode == Some(EvalDeclarationMode::Global);
        if eval_mode == Some(EvalDeclarationMode::Global)
            && function
                .external_bindings
                .iter()
                .find(|binding| binding.name.to_utf8_lossy() == name)
                .is_some_and(|binding| binding.is_lexical && !binding.is_catch_parameter)
            && function.eval_redeclaration.is_none()
        {
            function.eval_redeclaration = Some(name.to_owned());
        }
        if global {
            function.global_declarations.push(IrGlobalDeclaration {
                name: name.to_owned(),
                is_lexical: false,
                is_const: false,
                function_constant: None,
                closure_index: None,
            });
        }
        if let Some(binding) =
            function.scopes[root.0]
                .bindings
                .iter()
                .rev()
                .copied()
                .find(|binding| {
                    let binding = &function.bindings[binding.0];
                    binding.name == name
                        && match (function.kind, eval_mode) {
                            (FunctionKind::Script, _) => binding.storage == BindingStorage::Global,
                            (FunctionKind::Module, _) => false,
                            (
                                FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow,
                                _,
                            ) => {
                                matches!(binding.storage, BindingStorage::Local(_))
                            }
                            (FunctionKind::Eval(_), Some(EvalDeclarationMode::Global)) => {
                                binding.storage == BindingStorage::Global
                            }
                            (FunctionKind::Eval(_), Some(EvalDeclarationMode::Local)) => {
                                matches!(binding.storage, BindingStorage::Local(_))
                            }
                            (FunctionKind::Eval(_), Some(EvalDeclarationMode::Dynamic(_)))
                            | (FunctionKind::Eval(_), None) => false,
                        }
                })
        {
            if function.bindings[binding.0].kind != BindingKind::Normal {
                return Err(Error::internal(
                    "Annex B declaration found a malformed function-root binding",
                ));
            }
            return Ok(IrAnnexBinding::Static(binding));
        }

        let storage = if global {
            BindingStorage::Global
        } else {
            if function.locals.len() >= MAX_LOCAL_VARIABLES {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(declaration_span)),
                );
            }
            let index = u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
            function.locals.push(name.to_owned());
            BindingStorage::Local(index)
        };
        let binding = function.add_binding(
            root,
            root,
            name.to_owned(),
            storage,
            BindingKind::Normal,
            Some(declaration_span),
        );
        Ok(IrAnnexBinding::Static(binding))
    }

    fn emit_value(&mut self, value: Value) -> Result<(), Error> {
        self.emit_value_with_site(value, None)
    }

    fn emit_atom_string(&mut self, value: JsString) -> Result<(), Error> {
        let index = self.add_constant(IrConstant::AtomString(value))?;
        self.emit(IrOp::PushConstant(index)).map(|_| ())
    }

    fn emit_value_with_site(
        &mut self,
        value: Value,
        site: Option<SourceOffset>,
    ) -> Result<(), Error> {
        if let Value::Int(value) = value {
            return self
                .emit_with_site(IrOp::Bytecode(Instruction::PushI32(value)), site)
                .map(|_| ());
        }
        let index = self.add_constant(IrConstant::Primitive(value))?;
        self.emit_with_site(IrOp::PushConstant(index), site)
            .map(|_| ())
    }

    fn add_constant(&mut self, constant: IrConstant) -> Result<u32, Error> {
        let function = self.current_ir_mut();
        let index = u32::try_from(function.constants.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?;
        function.constants.push(constant);
        Ok(index)
    }

    fn emit_instruction(&mut self, instruction: Instruction) -> Result<usize, Error> {
        self.emit(IrOp::Bytecode(instruction))
    }

    fn take_anonymous_function_definition(&mut self) -> Option<AnonymousFunctionDefinition> {
        self.anonymous_function_definition.take()
    }

    fn emit_anonymous_set_name(
        &mut self,
        definition: AnonymousFunctionDefinition,
        instruction: Instruction,
    ) -> Result<usize, Error> {
        if !matches!(
            instruction,
            Instruction::SetName(_) | Instruction::SetNameComputed
        ) {
            return Err(Error::internal(
                "anonymous definition received a non-naming instruction",
            ));
        }
        match definition {
            AnonymousFunctionDefinition::Function => self.emit_instruction(instruction),
            AnonymousFunctionDefinition::Class {
                owner,
                static_initializer_start: Some(at),
            } => {
                if owner != self.current_function {
                    return Err(Error::internal(
                        "anonymous class static initializer changed defining function",
                    ));
                }
                insert_hoist_fragment(
                    self.current_ir_mut(),
                    at,
                    vec![SpannedIrOp {
                        op: IrOp::Bytecode(instruction),
                        pc_site: None,
                    }],
                )?;
                Ok(at)
            }
            AnonymousFunctionDefinition::Class {
                static_initializer_start: None,
                ..
            } => self.emit_instruction(instruction),
        }
    }

    fn emit_instruction_at(
        &mut self,
        instruction: Instruction,
        site: SourceOffset,
    ) -> Result<usize, Error> {
        self.emit_at(IrOp::Bytecode(instruction), site)
    }

    fn emit_identifier(
        &mut self,
        name: String,
        span: Span,
        access: IdentifierAccess,
    ) -> Result<usize, Error> {
        self.emit_identifier_at(name, span, access, source_offset(span)?)
    }

    fn emit_identifier_at(
        &mut self,
        name: String,
        span: Span,
        access: IdentifierAccess,
        pc_site: SourceOffset,
    ) -> Result<usize, Error> {
        let scope = self.current_ir().current_scope;
        self.emit_at(
            IrOp::Identifier {
                name,
                span,
                scope,
                access,
            },
            pc_site,
        )
    }

    fn emit_identifier_inherited(
        &mut self,
        name: String,
        span: Span,
        scope: ScopeId,
        access: IdentifierAccess,
    ) -> Result<usize, Error> {
        self.emit(IrOp::Identifier {
            name,
            span,
            scope,
            access,
        })
    }

    fn emit_identifier_reference_inherited(
        &mut self,
        name: String,
        span: Span,
        scope: ScopeId,
        access: IdentifierReferenceAccess,
    ) -> Result<usize, Error> {
        self.emit(IrOp::IdentifierReference {
            name,
            span,
            scope,
            access,
        })
    }

    /// Materialize an `emit_source_pos` which appeared before an expression's
    /// first real opcode. If the RHS emitted its own marker before that same
    /// opcode, QuickJS's later `OP_line_num` wins and the pending marker is
    /// intentionally discarded.
    fn inherit_source_marker_at(
        &mut self,
        first_operation: usize,
        marker: SourceOffset,
    ) -> Result<(), Error> {
        let operation = self
            .current_ir_mut()
            .ops
            .get_mut(first_operation)
            .ok_or_else(|| Error::internal("assignment RHS emitted no operation"))?;
        if operation.pc_site.is_none() {
            operation.pc_site = Some(marker);
        }
        Ok(())
    }

    fn emit(&mut self, operation: IrOp) -> Result<usize, Error> {
        self.emit_with_site(operation, None)
    }

    fn emit_at(&mut self, operation: IrOp, site: SourceOffset) -> Result<usize, Error> {
        self.emit_with_site(operation, Some(site))
    }

    fn emit_with_site(
        &mut self,
        operation: IrOp,
        pc_site: Option<SourceOffset>,
    ) -> Result<usize, Error> {
        let (popped, pushed) = operation.stack_effect();
        let function = self.current_ir_mut();
        function.last_member_reference = None;
        function.last_identifier_reference = None;
        function.last_optional_chain = None;
        function.stack_depth = function
            .stack_depth
            .checked_sub(popped)
            .ok_or_else(|| Error::internal("compiler produced a stack underflow"))?;
        function.stack_depth = function
            .stack_depth
            .checked_add(pushed)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        let index = function.ops.len();
        function.ops.push(SpannedIrOp {
            op: operation,
            pc_site,
        });
        Ok(index)
    }

    fn patch_jump(&mut self, instruction_index: usize, target: usize) -> Result<(), Error> {
        let target = u32::try_from(target)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?;
        let operation = self
            .current_ir_mut()
            .ops
            .get_mut(instruction_index)
            .ok_or_else(|| Error::internal("missing jump instruction"))?;
        match &mut operation.op {
            IrOp::Bytecode(
                Instruction::IfFalse(value)
                | Instruction::IfTrue(value)
                | Instruction::Goto(value)
                | Instruction::Catch(value)
                | Instruction::Gosub(value),
            ) => {
                *value = target;
                Ok(())
            }
            _ => Err(Error::internal("attempted to patch a non-jump instruction")),
        }
    }

    fn expect_punctuator(&mut self, punctuator: Punctuator) -> Result<(), Error> {
        if self.consume_punctuator(punctuator)? {
            Ok(())
        } else {
            Err(self.syntax_here(format!("expecting '{}'", punctuator.as_str())))
        }
    }

    fn consume_punctuator(&mut self, punctuator: Punctuator) -> Result<bool, Error> {
        if self.is_punctuator(punctuator) {
            self.advance()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn is_punctuator(&self, punctuator: Punctuator) -> bool {
        matches!(self.current().kind, TokenKind::Punctuator(current) if current == punctuator)
    }

    /// `of` is a QuickJS pseudo-keyword: escapes prevent it from acting as
    /// the for-of delimiter even though the decoded identifier text matches.
    fn is_for_of_keyword(&self) -> bool {
        matches!(
            &self.current().kind,
            TokenKind::Identifier(identifier)
                if identifier.value == "of" && !identifier.has_escape
        )
    }

    /// QuickJS `peek_token(FALSE) == TOK_OF` from an already-consumed
    /// pseudo-keyword. This simplified, non-committing lookahead skips trivia
    /// but does not let an escaped spelling act as the contextual delimiter.
    fn next_token_is_for_of_keyword(&self) -> bool {
        self.lexer
            .source()
            .get(self.current().span.end.byte_offset..)
            .is_some_and(quickjs_simple_lookahead_is_of)
    }

    /// Non-committing delimiter probe for a semicolon-free for head. It selects
    /// the retained record shape before the assignment fragment is lowered;
    /// the real parser still validates the complete LeftHandSideExpression and
    /// reports source-ordered syntax errors.
    fn for_iteration_kind_ahead(&self) -> Option<ForIterationKind> {
        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.start);
        let mut delimiters = Vec::new();
        let mut goal = LexicalGoal::Div;
        let mut regexp_allowed = true;

        loop {
            let requested_goal = goal;
            goal = LexicalGoal::Div;
            let Ok(mut token) = lexer.next_token_with_goal(requested_goal) else {
                return None;
            };
            if requested_goal == LexicalGoal::Div
                && regexp_allowed
                && matches!(
                    token.kind,
                    TokenKind::Punctuator(Punctuator::Divide | Punctuator::DivideAssign)
                )
            {
                lexer.seek(token.span.start);
                let Ok(regexp) = lexer.next_token_with_goal(LexicalGoal::RegExp) else {
                    return None;
                };
                token = regexp;
            }

            if delimiters.is_empty() {
                match &token.kind {
                    TokenKind::Keyword(Keyword::In) => return Some(ForIterationKind::In),
                    TokenKind::Identifier(identifier)
                        if identifier.value == "of" && !identifier.has_escape =>
                    {
                        return Some(ForIterationKind::Of);
                    }
                    TokenKind::Punctuator(Punctuator::RightParen) | TokenKind::Eof => return None,
                    _ => {}
                }
            }

            match &token.kind {
                TokenKind::Punctuator(Punctuator::LeftParen) => {
                    delimiters.push(ForHeadDelimiter::Parenthesis);
                }
                TokenKind::Punctuator(Punctuator::LeftBracket) => {
                    delimiters.push(ForHeadDelimiter::Bracket);
                }
                TokenKind::Punctuator(Punctuator::LeftBrace) => {
                    delimiters.push(ForHeadDelimiter::Brace);
                }
                TokenKind::Punctuator(Punctuator::RightParen) => {
                    if delimiters.pop() != Some(ForHeadDelimiter::Parenthesis) {
                        return None;
                    }
                }
                TokenKind::Punctuator(Punctuator::RightBracket) => {
                    if delimiters.pop() != Some(ForHeadDelimiter::Bracket) {
                        return None;
                    }
                }
                TokenKind::Punctuator(Punctuator::RightBrace) => {
                    if delimiters.last() == Some(&ForHeadDelimiter::Template) {
                        goal = LexicalGoal::TemplateContinuation;
                        regexp_allowed = true;
                        continue;
                    }
                    if delimiters.pop() != Some(ForHeadDelimiter::Brace) {
                        return None;
                    }
                }
                TokenKind::Template(part) => match part.kind {
                    TemplatePartKind::Head => delimiters.push(ForHeadDelimiter::Template),
                    TemplatePartKind::Middle => {
                        if delimiters.last() != Some(&ForHeadDelimiter::Template) {
                            return None;
                        }
                    }
                    TemplatePartKind::Tail => {
                        if delimiters.pop() != Some(ForHeadDelimiter::Template) {
                            return None;
                        }
                    }
                    TemplatePartKind::NoSubstitution => {}
                },
                TokenKind::Eof => return None,
                _ => {}
            }
            regexp_allowed = for_head_regexp_allowed_after(&token.kind);
        }
    }

    /// Mirror QuickJS `js_parse_skip_parens_token` for the one decision needed
    /// by classic `for`: any semicolon at the outer head depth selects classic
    /// grammar, even when its NoIn initializer later stops at `in` or `of`.
    /// This is a non-committing probe; lexical failures are encountered again
    /// by the real parser in source order.
    fn for_head_has_top_level_semicolon(&self) -> bool {
        if !self.is_punctuator(Punctuator::LeftParen) {
            return false;
        }

        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.start);
        let mut delimiters = Vec::new();
        let mut goal = LexicalGoal::Div;
        let mut regexp_allowed = true;
        let mut has_semicolon = false;

        loop {
            let requested_goal = goal;
            goal = LexicalGoal::Div;
            let Ok(mut token) = lexer.next_token_with_goal(requested_goal) else {
                return has_semicolon;
            };
            if requested_goal == LexicalGoal::Div
                && regexp_allowed
                && matches!(
                    token.kind,
                    TokenKind::Punctuator(Punctuator::Divide | Punctuator::DivideAssign)
                )
            {
                lexer.seek(token.span.start);
                let Ok(regexp) = lexer.next_token_with_goal(LexicalGoal::RegExp) else {
                    return has_semicolon;
                };
                token = regexp;
            }

            match &token.kind {
                TokenKind::Punctuator(Punctuator::LeftParen) => {
                    if delimiters.len() >= 255 {
                        return has_semicolon;
                    }
                    delimiters.push(ForHeadDelimiter::Parenthesis);
                }
                TokenKind::Punctuator(Punctuator::LeftBracket) => {
                    if delimiters.len() >= 255 {
                        return has_semicolon;
                    }
                    delimiters.push(ForHeadDelimiter::Bracket);
                }
                TokenKind::Punctuator(Punctuator::LeftBrace) => {
                    if delimiters.len() >= 255 {
                        return has_semicolon;
                    }
                    delimiters.push(ForHeadDelimiter::Brace);
                }
                TokenKind::Punctuator(Punctuator::RightParen) => {
                    if delimiters.pop() != Some(ForHeadDelimiter::Parenthesis) {
                        return has_semicolon;
                    }
                    if delimiters.is_empty() {
                        return has_semicolon;
                    }
                }
                TokenKind::Punctuator(Punctuator::RightBracket) => {
                    if delimiters.pop() != Some(ForHeadDelimiter::Bracket) {
                        return has_semicolon;
                    }
                }
                TokenKind::Punctuator(Punctuator::RightBrace) => {
                    if delimiters.last() == Some(&ForHeadDelimiter::Template) {
                        goal = LexicalGoal::TemplateContinuation;
                        regexp_allowed = true;
                        continue;
                    }
                    if delimiters.pop() != Some(ForHeadDelimiter::Brace) {
                        return has_semicolon;
                    }
                }
                TokenKind::Punctuator(Punctuator::Semicolon) if delimiters.len() == 1 => {
                    has_semicolon = true;
                }
                TokenKind::Template(part) => match part.kind {
                    TemplatePartKind::Head => {
                        if delimiters.len() >= 255 {
                            return has_semicolon;
                        }
                        delimiters.push(ForHeadDelimiter::Template);
                    }
                    TemplatePartKind::Middle => {
                        if delimiters.last() != Some(&ForHeadDelimiter::Template) {
                            return has_semicolon;
                        }
                    }
                    TemplatePartKind::Tail => {
                        if delimiters.pop() != Some(ForHeadDelimiter::Template) {
                            return has_semicolon;
                        }
                    }
                    TemplatePartKind::NoSubstitution => {}
                },
                TokenKind::Eof => return has_semicolon,
                _ => {}
            }
            regexp_allowed = for_head_regexp_allowed_after(&token.kind);
        }
    }

    /// QuickJS `is_let(..., DECL_MASK_OTHER)` resolves sloppy `let` before the
    /// statement parser chooses declaration or expression grammar. In
    /// particular, `let [` is always lexical and must never silently execute
    /// as a member assignment while destructuring remains an explicit boundary.
    fn lexical_declaration_ahead(&self, allow_line_terminated_other: bool) -> Result<bool, Error> {
        if matches!(
            self.current().kind,
            TokenKind::Keyword(Keyword::Let | Keyword::Const)
        ) {
            return Ok(true);
        }
        let TokenKind::Identifier(identifier) = &self.current().kind else {
            return Ok(false);
        };
        if identifier.value != "let" || identifier.has_escape {
            return Ok(false);
        }

        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.start);
        lexer.next_token().map_err(lex_error)?;
        let next = lexer.next_token().map_err(lex_error)?;
        let other_declaration_start = matches!(
            &next.kind,
            TokenKind::Punctuator(Punctuator::LeftBrace)
                | TokenKind::Identifier(Identifier {
                    escaped_reserved_word: false,
                    ..
                })
                | TokenKind::Keyword(Keyword::Let | Keyword::Yield | Keyword::Await)
        );
        Ok(
            matches!(&next.kind, TokenKind::Punctuator(Punctuator::LeftBracket))
                || (other_declaration_start
                    && (!next.line_terminator_before || allow_line_terminated_other)),
        )
    }

    /// QuickJS `is_label` accepts only a non-reserved Identifier followed by
    /// `:` using a non-committing simplified scanner. Keep the probe separate
    /// from the parser token cache; a lexical failure after the identifier is
    /// still reported later by the real parser in source order.
    fn label_ahead(&self) -> Option<String> {
        let TokenKind::Identifier(identifier) = &self.current().kind else {
            return None;
        };
        if identifier.escaped_reserved_word {
            return None;
        }
        let label_name = identifier.value.clone();
        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.end);
        let Ok(next) = lexer.next_token() else {
            return None;
        };
        matches!(next.kind, TokenKind::Punctuator(Punctuator::Colon)).then_some(label_name)
    }

    /// QuickJS gates generator and pseudo-keyword `async function` declarations
    /// before entering the ordinary-function parser when DECL_MASK_OTHER is
    /// absent. Preserve that diagnostic priority without consuming lookahead.
    fn restricted_function_declaration_ahead(
        &self,
        annex_b_function_allowed: bool,
    ) -> Result<bool, Error> {
        let generator = matches!(self.current().kind, TokenKind::Keyword(Keyword::Function));
        let async_function = self.async_function_ahead();
        if (!generator || !annex_b_function_allowed) && !async_function {
            return Ok(false);
        }
        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.end);
        let next = lexer.next_token().map_err(lex_error)?;
        if generator {
            Ok(matches!(
                next.kind,
                TokenKind::Punctuator(Punctuator::Multiply)
            ))
        } else {
            Ok(async_function)
        }
    }

    /// Non-committing recognition of QuickJS's `async function`
    /// pseudo-keyword pair. Escapes and a LineTerminator after `async` leave it
    /// as an ordinary IdentifierReference.
    fn async_function_ahead(&self) -> bool {
        let TokenKind::Identifier(identifier) = &self.current().kind else {
            return false;
        };
        if identifier.value != "async" || identifier.has_escape {
            return false;
        }
        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.end);
        let Ok(function) = lexer.next_token_with_goal(LexicalGoal::Div) else {
            return false;
        };
        !function.line_terminator_before
            && matches!(function.kind, TokenKind::Keyword(Keyword::Function))
    }

    fn at_eof(&self) -> bool {
        matches!(self.current().kind, TokenKind::Eof)
    }

    /// QuickJS arrows inherit the `new.target` capability through parse
    /// parents. A direct-eval root authenticates the inherited capability by
    /// carrying the hidden imported binding in its root environment.
    fn current_new_target_allowed(&self) -> bool {
        let mut function_id = self.current_function;
        loop {
            let function = &self.functions[function_id];
            match function.kind {
                FunctionKind::Ordinary | FunctionKind::Method => return true,
                FunctionKind::Script
                | FunctionKind::Module
                | FunctionKind::Eval(EvalKind::Indirect) => return false,
                FunctionKind::Eval(EvalKind::Direct) => {
                    return function
                        .binding_from_scope(function.var_scope, NEW_TARGET_LOCAL_NAME)
                        .is_some();
                }
                FunctionKind::Eval(EvalKind::None) => return false,
                FunctionKind::Arrow => {
                    let Some(parent) = function.parent else {
                        return false;
                    };
                    function_id = parent.function;
                }
            }
        }
    }

    fn current(&self) -> &Token<'source> {
        // Construction and every advance ensure the current token exists.
        &self.tokens[self.cursor]
    }

    fn advance(&mut self) -> Result<(), Error> {
        self.advance_with_goal(LexicalGoal::Div)
    }

    /// Advance from a grammar delimiter to the first token of an expression,
    /// selecting RegExp only when the ordinary scanner sees a leading slash.
    fn advance_expression_start(&mut self) -> Result<(), Error> {
        let start = self.current().span.end;
        if self.tokens.len() > self.cursor + 1 {
            self.tokens.truncate(self.cursor + 1);
            self.lexer.seek(start);
        }
        let mut probe = self.lexer.clone();
        probe.seek(start);
        let next = probe.next_token().map_err(lex_error)?;
        let goal = if matches!(
            next.kind,
            TokenKind::Punctuator(Punctuator::Divide | Punctuator::DivideAssign)
        ) {
            LexicalGoal::RegExp
        } else {
            LexicalGoal::Div
        };
        self.advance_with_goal(goal)
    }

    fn advance_with_goal(&mut self, goal: LexicalGoal) -> Result<(), Error> {
        if !self.at_eof() {
            self.cursor += 1;
            self.ensure_token_with_goal(self.cursor, goal)?;
        }
        Ok(())
    }

    fn ensure_token(&mut self, index: usize) -> Result<(), Error> {
        self.ensure_token_with_goal(index, LexicalGoal::Div)
    }

    fn ensure_token_with_goal(&mut self, index: usize, goal: LexicalGoal) -> Result<(), Error> {
        while self.tokens.len() <= index {
            let token = self.lexer.next_token_with_goal(goal).map_err(lex_error)?;
            self.tokens.push(token);
        }
        Ok(())
    }

    /// Rescan the current token after the parser has selected its lexical
    /// goal.  Seeking to the token itself intentionally avoids committing a
    /// lexer heuristic; preserve the already-observed trivia bit because the
    /// rescan starts after that trivia rather than before it.
    fn relex_current_with_goal(&mut self, goal: LexicalGoal) -> Result<(), Error> {
        let position = self.current().span.start;
        let line_terminator_before = self.current().line_terminator_before;
        self.tokens.truncate(self.cursor);
        self.lexer.seek(position);
        self.ensure_token_with_goal(self.cursor, goal)?;
        self.tokens[self.cursor].line_terminator_before = line_terminator_before;
        Ok(())
    }

    fn relex_current_with_strict(&mut self, strict: bool) -> Result<(), Error> {
        let mut context = self.lexer.context();
        context.strict = strict;
        self.relex_current_with_context(context)
    }

    /// Rescan the current token and all future tokens under one complete
    /// function lexical context. Function nesting must restore all of strict,
    /// module, generator and async state; changing only `strict` leaks a
    /// parent's contextual `yield`/`await` classification into its child.
    fn relex_current_with_context(&mut self, context: LexContext) -> Result<(), Error> {
        let position = self.current().span.start;
        let line_terminator_before = self.current().line_terminator_before;
        self.tokens.truncate(self.cursor);
        self.lexer.seek(position);
        self.lexer.set_context(context);
        self.ensure_token(self.cursor)?;
        self.tokens[self.cursor].line_terminator_before = line_terminator_before;
        Ok(())
    }

    /// Change how tokens after the current token are classified without
    /// reparsing the current token. QuickJS relies on this distinction when an
    /// async arrow's unparenthesized parameter was already read in its parent.
    fn set_future_lex_context(&mut self, context: LexContext) {
        let position = self.current().span.end;
        self.tokens.truncate(self.cursor + 1);
        self.lexer.seek(position);
        self.lexer.set_context(context);
    }

    fn directive_prologue_has_use_strict(
        &self,
        start: usize,
        inherited_strict: bool,
    ) -> Result<bool, Error> {
        let use_strict = "use strict".encode_utf16().collect::<Vec<_>>();
        let position = self.tokens[start].span.start;
        let mut lexer = self.lexer.clone();
        lexer.seek(position);
        let mut context = lexer.context();
        context.strict = inherited_strict;
        lexer.set_context(context);
        let mut token = lexer.next_token().map_err(lex_error)?;
        let mut found_strict = false;

        loop {
            let candidate = match &token.kind {
                TokenKind::String(literal) => {
                    !literal.has_escape && literal.value.utf16 == use_strict
                }
                _ => return Ok(found_strict),
            };

            let next = lexer.next_token().map_err(lex_error)?;
            let consumed = match &next.kind {
                TokenKind::Punctuator(Punctuator::Semicolon) => 2,
                TokenKind::Punctuator(Punctuator::RightBrace) | TokenKind::Eof => 1,
                _ if next.line_terminator_before && quickjs_directive_asi_token(&next.kind) => 1,
                _ => return Ok(found_strict),
            };
            if candidate {
                found_strict = true;
            }
            token = if consumed == 1 {
                next
            } else {
                lexer.next_token().map_err(lex_error)?
            };
            if candidate {
                let mut context = lexer.context();
                context.strict = true;
                lexer.set_context(context);
            }
        }
    }

    fn syntax_here(&self, message: impl Into<String>) -> Error {
        Error::syntax(message, source_span(self.current().span))
    }

    fn unsupported_here(&self, message: impl Into<String>) -> Error {
        Error::unsupported(message, source_span(self.current().span))
    }

    fn current_ir(&self) -> &FunctionIr {
        &self.functions[self.current_function]
    }

    fn current_ir_mut(&mut self) -> &mut FunctionIr {
        &mut self.functions[self.current_function]
    }

    /// Add one physical argument input without deciding where authored reads
    /// resolve. A later default may promote every source binding to the
    /// independent parameter environment while retaining these slots as the
    /// call-frame input ABI.
    fn append_identifier_parameter(&mut self, name: String, span: Span) -> Result<u16, Error> {
        let function = self.current_ir_mut();
        if function.parameter_scope.is_some()
            && function
                .parameter_names
                .iter()
                .any(|parameter| parameter == &name)
        {
            return Err(Error::syntax(
                "duplicate parameter names not allowed in this context",
                source_span(span),
            ));
        }
        if function.parameters.len() >= MAX_LOCAL_VARIABLES {
            return Err(Error::new(ErrorKind::JsInternal, "too many arguments")
                .with_span(source_span(span)));
        }
        let index = u16::try_from(function.parameters.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
        function.parameters.push(Some(name.clone()));
        function.parameter_argument_locals.push(None);
        function.parameter_names.push(name.clone());
        function.add_binding(
            function.var_scope,
            function.var_scope,
            name,
            BindingStorage::Argument(index),
            BindingKind::Normal,
            None,
        );
        Ok(index)
    }

    /// Reserve QuickJS's unnamed physical argument slot for one authored
    /// BindingPattern. Its BoundNames are registered separately as ordinary
    /// function-root variables while the raw call input remains inaccessible
    /// after the entry destructuring phase.
    fn append_pattern_parameter(&mut self, span: Span) -> Result<u16, Error> {
        let function = self.current_ir_mut();
        if function.parameters.len() >= MAX_LOCAL_VARIABLES {
            return Err(Error::new(ErrorKind::JsInternal, "too many arguments")
                .with_span(source_span(span)));
        }
        let index = u16::try_from(function.parameters.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
        function.parameters.push(None);
        function.parameter_argument_locals.push(None);
        function.has_simple_parameter_list = false;
        Ok(index)
    }

    /// Move the body scope entry behind non-default parameter destructuring.
    /// QuickJS evaluates these patterns in FunctionRoot: body `var` bindings
    /// exist as undefined, while body lexicals and function initializers have
    /// not been installed yet.
    fn activate_pattern_parameter_initialization(&mut self) -> Result<(), Error> {
        let function = self.current_ir_mut();
        if function.pattern_parameter_initialization {
            return Ok(());
        }
        if let Some(parameter_scope) = function.parameter_scope {
            if function.current_scope != parameter_scope {
                return Err(Error::internal(
                    "pattern parameter escaped its parameter environment",
                ));
            }
            function.pattern_parameter_initialization = true;
            return Ok(());
        }
        if function.stack_depth != 0
            || function.ops.len() != 1
            || !matches!(
                function.ops.first(),
                Some(SpannedIrOp {
                    op: IrOp::EnterScope(scope),
                    pc_site: None,
                }) if *scope == function.body_scope
            )
        {
            return Err(Error::internal(
                "pattern parameter initialization started after function body bytecode",
            ));
        }
        function.ops.clear();
        function.current_scope = function.var_scope;
        function.pattern_parameter_initialization = true;
        Ok(())
    }

    fn register_pattern_parameter_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
        conflict_span: Span,
    ) -> Result<(), Error> {
        let expected_scope = self
            .current_ir()
            .parameter_scope
            .unwrap_or(self.current_ir().var_scope);
        if !self.current_ir().pattern_parameter_initialization
            || self.current_ir().current_scope != expected_scope
        {
            return Err(Error::internal(
                "pattern parameter binding escaped its initialization phase",
            ));
        }
        if self
            .current_ir()
            .parameter_names
            .iter()
            .any(|parameter| parameter == name)
        {
            return Err(Error::syntax(
                "duplicate parameter names not allowed in this context",
                source_span(conflict_span),
            ));
        }
        self.current_ir_mut().parameter_names.push(name.to_owned());
        if self.current_ir().parameter_scope.is_none() {
            return self.register_var_binding(name, declaration_span, conflict_span);
        }

        let parameter_local =
            self.allocate_parameter_binding_local(name.to_owned(), declaration_span)?;
        self.current_ir_mut()
            .parameter_pattern_bindings
            .push(IrParameterPatternBinding {
                name: name.to_owned(),
                parameter_local,
                body_local: None,
                declaration_span,
            });
        Ok(())
    }

    fn allocate_parameter_binding_local(&mut self, name: String, span: Span) -> Result<u16, Error> {
        let function = self.current_ir_mut();
        let parameter_scope = function
            .parameter_scope
            .ok_or_else(|| Error::internal("parameter local has no parameter scope"))?;
        let local = if let Some(reserved) = function.parameter_local_reservation_count {
            let cell = function.parameter_locals.len();
            if cell >= reserved || function.locals.get(cell).is_none() {
                return Err(Error::internal(
                    "parameter binding exceeded its pre-scan reservation",
                ));
            }
            function.locals[cell] = name.clone();
            u16::try_from(cell)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?
        } else {
            if function.locals.len() >= MAX_LOCAL_VARIABLES {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(span)),
                );
            }
            let local = u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
            function.locals.push(name.clone());
            local
        };
        function.parameter_locals.push(local);
        function.add_binding(
            parameter_scope,
            parameter_scope,
            name,
            BindingStorage::Local(local),
            BindingKind::Lexical { is_const: false },
            Some(span),
        );
        Ok(local)
    }

    fn allocate_parameter_local(&mut self, argument: u16, span: Span) -> Result<u16, Error> {
        let argument_index = usize::from(argument);
        let name = self
            .current_ir()
            .parameters
            .get(argument_index)
            .and_then(Clone::clone)
            .ok_or_else(|| Error::internal("parameter local referenced an unnamed argument"))?;
        if self
            .current_ir()
            .parameter_argument_locals
            .get(argument_index)
            .is_none_or(Option::is_some)
        {
            return Err(Error::internal(
                "parameter argument cell was allocated more than once",
            ));
        }
        let local = self.allocate_parameter_binding_local(name, span)?;
        self.current_ir_mut().parameter_argument_locals[argument_index] = Some(local);
        Ok(local)
    }

    /// Create QuickJS's parentless argument scope. Valid source reaches this
    /// from the whole-list standalone-`=` pre-scan, before the first formal is
    /// parsed; the lazy caller remains as a defensive fallback for malformed
    /// or scanner-limit input which later exposes an identifier default.
    fn activate_parameter_environment_from_scan(
        &mut self,
        bound_name_count: Option<usize>,
    ) -> Result<(), Error> {
        if self.current_ir().parameter_scope.is_some() {
            return Ok(());
        }
        let scan_span = self.current().span;
        let function = self.current_ir_mut();
        if !matches!(
            function.kind,
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
        ) || !function.parameters.is_empty()
            || !function.parameter_names.is_empty()
            || function.pattern_parameter_initialization
            || function.ops.len() != 1
            || !matches!(
                function.ops.first(),
                Some(SpannedIrOp {
                    op: IrOp::EnterScope(scope),
                    pc_site: None,
                }) if *scope == function.body_scope
            )
        {
            return Err(Error::internal(
                "parameter environment pre-scan ran after formal parsing",
            ));
        }
        if let Some(bound_name_count) = bound_name_count {
            if !function.locals.is_empty()
                || bound_name_count > MAX_LOCAL_VARIABLES
                || function.parameter_local_reservation_count.is_some()
            {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(scan_span)),
                );
            }
            function
                .locals
                .resize(bound_name_count, "<parameter-reserved>".to_owned());
            function.parameter_local_reservation_count = Some(bound_name_count);
        }
        function.ops.clear();
        let parameter_scope = ScopeId(function.scopes.len());
        function.scopes.push(IrScope {
            parent: None,
            kind: ScopeKind::Parameter,
            is_parameter_initializer: true,
            bindings: Vec::new(),
        });
        function.parameter_scope = Some(parameter_scope);
        function.current_scope = parameter_scope;
        function.ops.push(SpannedIrOp {
            op: IrOp::EnterScope(parameter_scope),
            pc_site: None,
        });
        Ok(())
    }

    fn activate_identifier_parameter_environment(
        &mut self,
        current: u16,
        span: Span,
    ) -> Result<u16, Error> {
        if self.current_ir().parameter_scope.is_some() {
            return self.allocate_parameter_local(current, span);
        }

        let parameter_count = {
            let function = self.current_ir_mut();
            if !matches!(
                function.kind,
                FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
            ) || usize::from(current) + 1 != function.parameters.len()
                || function.pattern_parameter_initialization
                || function.parameters.iter().any(Option::is_none)
                || function.ops.len() != 1
                || !matches!(
                    function.ops.first(),
                    Some(SpannedIrOp {
                        op: IrOp::EnterScope(scope),
                        pc_site: None,
                    }) if *scope == function.body_scope
                )
            {
                return Err(Error::internal(
                    "parameter environment was activated after body bytecode",
                ));
            }
            function.ops.clear();
            let parameter_scope = ScopeId(function.scopes.len());
            function.scopes.push(IrScope {
                parent: None,
                kind: ScopeKind::Parameter,
                is_parameter_initializer: true,
                bindings: Vec::new(),
            });
            function.parameter_scope = Some(parameter_scope);
            function.current_scope = parameter_scope;
            function.ops.push(SpannedIrOp {
                op: IrOp::EnterScope(parameter_scope),
                pc_site: None,
            });
            function.parameters.len()
        };

        for argument in 0..parameter_count {
            let argument = u16::try_from(argument)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
            let local = self.allocate_parameter_local(argument, span)?;
            if argument < current {
                self.emit_instruction(Instruction::GetArg(argument))?;
                self.emit_instruction(Instruction::InitializeLocal(local))?;
            }
        }
        self.current_ir()
            .parameter_argument_locals
            .get(usize::from(current))
            .copied()
            .flatten()
            .ok_or_else(|| Error::internal("current parameter local was not allocated"))
    }

    fn register_plain_identifier_parameter(
        &mut self,
        name: String,
        span: Span,
    ) -> Result<(), Error> {
        let argument = self.append_identifier_parameter(name, span)?;
        if self.current_ir().defined_argument_count == usize::from(argument) {
            self.current_ir_mut().defined_argument_count += 1;
        }
        if self.current_ir().parameter_scope.is_some() {
            let local = self.allocate_parameter_local(argument, span)?;
            self.emit_instruction(Instruction::GetArg(argument))?;
            self.emit_instruction(Instruction::InitializeLocal(local))?;
        }
        Ok(())
    }

    fn parse_default_identifier_parameter(
        &mut self,
        name: String,
        span: Span,
    ) -> Result<(), Error> {
        let argument = self.append_identifier_parameter(name.clone(), span)?;
        self.current_ir_mut().has_simple_parameter_list = false;
        self.current_ir_mut()
            .parameter_default_sources
            .push(ParameterDefaultSource::Argument(argument));
        let local = self.activate_identifier_parameter_environment(argument, span)?;

        self.expect_punctuator(Punctuator::Equal)?;
        self.emit_instruction(Instruction::GetArg(argument))?;
        self.emit_instruction(Instruction::Dup)?;
        self.emit_instruction(Instruction::Undefined)?;
        self.emit_instruction(Instruction::StrictEq)?;
        let has_value = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        self.emit_instruction(Instruction::Drop)?;
        self.anonymous_function_definition = None;
        self.parse_assignment_allow_in()?;
        if let Some(definition) = self.take_anonymous_function_definition() {
            let name = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::try_from_utf8(&name)?,
            )))?;
            self.emit_anonymous_set_name(definition, Instruction::SetName(name))?;
        }
        self.emit_instruction(Instruction::Dup)?;
        self.emit_instruction(Instruction::PutArg(argument))?;
        let has_value_target = self.current_ir().ops.len();
        self.patch_jump(has_value, has_value_target)?;
        self.emit_instruction(Instruction::InitializeLocal(local))?;
        Ok(())
    }

    fn register_rest_identifier_parameter(
        &mut self,
        name: String,
        span: Span,
    ) -> Result<(), Error> {
        let argument = self.append_identifier_parameter(name, span)?;
        self.current_ir_mut().has_simple_parameter_list = false;
        self.current_ir_mut().rest_parameter = Some(argument);
        if self.current_ir().parameter_scope.is_some() {
            let local = self.allocate_parameter_local(argument, span)?;
            self.emit_instruction(Instruction::Rest(argument))?;
            self.emit_instruction(Instruction::Dup)?;
            self.emit_instruction(Instruction::PutArg(argument))?;
            self.emit_instruction(Instruction::InitializeLocal(local))?;
        } else if self.current_ir().pattern_parameter_initialization {
            self.emit_instruction(Instruction::Rest(argument))?;
            self.emit_instruction(Instruction::PutArg(argument))?;
        }
        Ok(())
    }

    fn register_rest_pattern_parameter(&mut self) -> Result<u16, Error> {
        let function = self.current_ir_mut();
        if !function.pattern_parameter_initialization
            || function.rest_parameter.is_some()
            || function.rest_pattern_start.is_some()
        {
            return Err(Error::internal(
                "rest BindingPattern has malformed formal metadata",
            ));
        }
        let start = u16::try_from(function.parameters.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
        function.has_simple_parameter_list = false;
        function.rest_pattern_start = Some(start);
        Ok(start)
    }

    fn finish_pattern_parameter_length(
        &mut self,
        argument: u16,
        has_initializer: bool,
    ) -> Result<(), Error> {
        let function = self.current_ir_mut();
        if has_initializer {
            let source = if function.rest_pattern_start == Some(argument)
                && usize::from(argument) == function.parameters.len()
            {
                ParameterDefaultSource::RestPattern(argument)
            } else {
                ParameterDefaultSource::Argument(argument)
            };
            function.parameter_default_sources.push(source);
        }
        if !has_initializer && function.defined_argument_count == usize::from(argument) {
            function.defined_argument_count = function
                .defined_argument_count
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
        }
        Ok(())
    }

    fn allocate_parameter_pattern_body_bindings(&mut self) -> Result<Vec<(u16, u16)>, Error> {
        let function = self.current_ir_mut();
        let mut copies = Vec::with_capacity(function.parameter_pattern_bindings.len());
        for binding_index in 0..function.parameter_pattern_bindings.len() {
            let (name, parameter_local, declaration_span) = {
                let binding = &function.parameter_pattern_bindings[binding_index];
                (
                    binding.name.clone(),
                    binding.parameter_local,
                    binding.declaration_span,
                )
            };
            if function
                .binding_in_scope(function.var_scope, &name)
                .is_some()
            {
                return Err(Error::internal(
                    "parameter pattern body binding already exists",
                ));
            }
            if function.locals.len() >= MAX_LOCAL_VARIABLES {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(declaration_span)),
                );
            }
            let body_local = u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
            function.locals.push(name.clone());
            function.add_binding(
                function.var_scope,
                function.var_scope,
                name,
                BindingStorage::Local(body_local),
                BindingKind::Normal,
                Some(declaration_span),
            );
            function.parameter_pattern_bindings[binding_index].body_local = Some(body_local);
            copies.push((parameter_local, body_local));
        }
        Ok(copies)
    }

    fn finish_identifier_parameter_environment(&mut self) -> Result<(), Error> {
        if let Some(parameter_scope) = self.current_ir().parameter_scope {
            if self.current_ir().current_scope != parameter_scope
                || self.current_ir().stack_depth != 0
            {
                return Err(Error::internal(
                    "parameter environment finished with unbalanced parser state",
                ));
            }
            let copies = self.allocate_parameter_pattern_body_bindings()?;
            for (parameter_local, body_local) in copies.into_iter().rev() {
                self.emit_instruction(Instruction::GetLocalCheck(parameter_local))?;
                self.emit_instruction(Instruction::PutLocal(body_local))?;
            }
            self.emit(IrOp::ParameterInitializationEnd)?;
            let body_scope = self.current_ir().body_scope;
            let function = self.current_ir_mut();
            function.ops.push(SpannedIrOp {
                op: IrOp::LeaveScope(parameter_scope),
                pc_site: None,
            });
            function.current_scope = body_scope;
            function.ops.push(SpannedIrOp {
                op: IrOp::EnterScope(body_scope),
                pc_site: None,
            });
            return Ok(());
        }

        if self.current_ir().pattern_parameter_initialization {
            if self.current_ir().current_scope != self.current_ir().var_scope
                || self.current_ir().stack_depth != 0
            {
                return Err(Error::internal(
                    "pattern parameter initialization finished with unbalanced parser state",
                ));
            }
            let body_scope = self.current_ir().body_scope;
            let function = self.current_ir_mut();
            function.ops.push(SpannedIrOp {
                op: IrOp::ParameterInitializationEnd,
                pc_site: None,
            });
            function.current_scope = body_scope;
            function.ops.push(SpannedIrOp {
                op: IrOp::EnterScope(body_scope),
                pc_site: None,
            });
        }
        Ok(())
    }

    fn push_scope(&mut self, kind: ScopeKind) -> ScopeId {
        let function = self.current_ir_mut();
        let parent = function.current_scope;
        let is_parameter_initializer = function.scopes[parent.0].is_parameter_initializer
            || (function.pattern_parameter_initialization && parent == function.var_scope);
        let scope = ScopeId(function.scopes.len());
        function.scopes.push(IrScope {
            parent: Some(parent),
            kind,
            is_parameter_initializer,
            bindings: Vec::new(),
        });
        function.ops.push(SpannedIrOp {
            op: IrOp::EnterScope(scope),
            pc_site: None,
        });
        function.current_scope = scope;
        scope
    }

    fn pop_scope(&mut self, expected: ScopeId) -> Result<(), Error> {
        let function = self.current_ir_mut();
        if function.current_scope != expected {
            return Err(Error::internal("parser scope stack is unbalanced"));
        }
        function.ops.push(SpannedIrOp {
            op: IrOp::LeaveScope(expected),
            pc_site: None,
        });
        function.current_scope = function.scopes[expected.0]
            .parent
            .ok_or_else(|| Error::internal("cannot pop a function root scope"))?;
        Ok(())
    }

    /// Emit the runtime lexical exits which QuickJS's `close_scopes` inserts
    /// on an abrupt break/continue edge. Parser scope state is intentionally
    /// unchanged because parsing continues along the unreachable linear path.
    fn emit_scope_closures(&mut self, mut scope: ScopeId, stop: ScopeId) -> Result<(), Error> {
        while scope != stop {
            let parent = self
                .current_ir()
                .scopes
                .get(scope.0)
                .and_then(|scope| scope.parent)
                .ok_or_else(|| Error::internal("abrupt scope target is not an ancestor"))?;
            self.current_ir_mut().ops.push(SpannedIrOp {
                op: IrOp::LeaveScope(scope),
                pc_site: None,
            });
            scope = parent;
        }
        Ok(())
    }
}

fn relocate_ir_fragment(
    operations: &mut [SpannedIrOp],
    old_range: Range<usize>,
    new_start: usize,
) -> Result<(), Error> {
    for operation in operations {
        let IrOp::Bytecode(
            Instruction::Goto(target)
            | Instruction::IfFalse(target)
            | Instruction::IfTrue(target)
            | Instruction::Catch(target)
            | Instruction::Gosub(target),
        ) = &mut operation.op
        else {
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

#[derive(Clone, Copy)]
enum IdentifierContext {
    Reference,
    Variable,
    FunctionName,
    Argument,
}

fn validate_identifier(
    identifier: &Identifier<'_>,
    span: Span,
    strict: bool,
    context: IdentifierContext,
) -> Result<(), Error> {
    validate_identifier_reservation(identifier, span, strict, context)?;
    if strict
        && !matches!(context, IdentifierContext::Reference)
        && matches!(identifier.value.as_str(), "eval" | "arguments")
    {
        let message = match context {
            IdentifierContext::Variable => "invalid variable name in strict mode",
            IdentifierContext::FunctionName => "invalid function name in strict code",
            IdentifierContext::Argument => "invalid argument name in strict code",
            IdentifierContext::Reference => unreachable!("reference context was excluded"),
        };
        return Err(Error::syntax(message, source_span(span)));
    }
    Ok(())
}

fn validate_identifier_reservation(
    identifier: &Identifier<'_>,
    span: Span,
    strict: bool,
    context: IdentifierContext,
) -> Result<(), Error> {
    if identifier.escaped_reserved_word {
        return Err(syntax_atom_error(
            "'",
            &identifier.value,
            "' is a reserved identifier",
            span,
        )?);
    }
    if strict
        && identifier
            .keyword_hint
            .is_some_and(strict_reserved_identifier)
    {
        let message = match context {
            IdentifierContext::Reference => {
                return Err(syntax_atom_error(
                    "'",
                    &identifier.value,
                    "' is a reserved identifier",
                    span,
                )?);
            }
            IdentifierContext::Variable => "invalid variable name in strict mode",
            IdentifierContext::FunctionName => "invalid function name in strict code",
            IdentifierContext::Argument => "invalid argument name in strict code",
        };
        return Err(Error::syntax(message, source_span(span)));
    }
    Ok(())
}

fn syntax_atom_error(
    prefix: &str,
    atom: &str,
    suffix: &str,
    span: Span,
) -> Result<Error, JsStringError> {
    Ok(syntax_atom_error_without_span(prefix, atom, suffix)?.with_span(source_span(span)))
}

fn syntax_atom_error_without_span(
    prefix: &str,
    atom: &str,
    suffix: &str,
) -> Result<Error, JsStringError> {
    let atom = JsString::try_from_utf8(atom)?;
    let mut message = NativeErrorMessage::new();
    message.push_utf8(prefix);
    atom.push_atom_get_str_to(&mut message);
    message.push_utf8(suffix);
    Ok(Error::from_native_message(ErrorKind::Syntax, message))
}

const fn strict_reserved_identifier(keyword: Keyword) -> bool {
    matches!(
        keyword,
        Keyword::Implements
            | Keyword::Interface
            | Keyword::Let
            | Keyword::Package
            | Keyword::Private
            | Keyword::Protected
            | Keyword::Public
            | Keyword::Static
            | Keyword::Yield
    )
}

fn unlinked_primitive(value: Value) -> Result<UnlinkedConstant, Error> {
    UnlinkedConstant::primitive(value).map_err(|error| {
        Error::internal(format!(
            "compiler emitted a runtime-bound constant into an unlinked function: {error}"
        ))
    })
}

fn parse_number(
    number: &crate::engine::compiler::lexer::NumberLiteral<'_>,
) -> Result<Value, String> {
    let raw = number.raw.replace('_', "");
    if let NumberKind::BigInt(radix) = number.kind {
        let literal = raw
            .strip_suffix('n')
            .ok_or_else(|| "BigInt literal is missing its suffix".to_owned())?;
        let (digits, base) = match radix {
            NumericRadix::Binary => (literal.get(2..).unwrap_or_default(), 2),
            NumericRadix::Octal => (literal.get(2..).unwrap_or_default(), 8),
            NumericRadix::Decimal => (literal, 10),
            NumericRadix::Hexadecimal => (literal.get(2..).unwrap_or_default(), 16),
        };
        return JsBigInt::parse_radix(digits, base)
            .map(Value::BigInt)
            .map_err(|error| error.to_string());
    }

    let value = match number.kind {
        NumberKind::Integer(radix) => parse_radix_literal(&raw, radix)?,
        NumberKind::Float | NumberKind::LegacyDecimal => raw
            .parse::<f64>()
            .map_err(|_| format!("invalid numeric literal '{raw}'"))?,
        NumberKind::LegacyOctal => parse_digits(&raw, 8)?,
        NumberKind::BigInt(_) => unreachable!("handled above"),
    };
    Ok(Value::number(value))
}

/// Mirrors the token switch in QuickJS 2026-06-04 `js_parse_directives`.
/// Its observable ASI behavior is intentionally narrower than a generic
/// "can this token continue an expression" test.
fn quickjs_directive_asi_token(kind: &TokenKind<'_>) -> bool {
    matches!(
        kind,
        TokenKind::Number(_)
            | TokenKind::String(_)
            | TokenKind::Template(_)
            | TokenKind::Identifier(_)
            | TokenKind::RegExp(_)
            | TokenKind::Punctuator(Punctuator::Decrement | Punctuator::Increment)
            | TokenKind::Keyword(
                Keyword::Null
                    | Keyword::False
                    | Keyword::True
                    | Keyword::If
                    | Keyword::Return
                    | Keyword::Var
                    | Keyword::This
                    | Keyword::Delete
                    | Keyword::Typeof
                    | Keyword::New
                    | Keyword::Do
                    | Keyword::While
                    | Keyword::For
                    | Keyword::Switch
                    | Keyword::Throw
                    | Keyword::Try
                    | Keyword::Function
                    | Keyword::Debugger
                    | Keyword::With
                    | Keyword::Class
                    | Keyword::Const
                    | Keyword::Enum
                    | Keyword::Export
                    | Keyword::Import
                    | Keyword::Super
                    | Keyword::Interface
                    | Keyword::Let
                    | Keyword::Package
                    | Keyword::Private
                    | Keyword::Protected
                    | Keyword::Public
                    | Keyword::Static
            )
    )
}

/// QuickJS `is_regexp_allowed`, used only by the non-committing `for`-head
/// probe. The real parser still owns the eventual lexical goal and diagnostic.
fn for_head_regexp_allowed_after(kind: &TokenKind<'_>) -> bool {
    if matches!(
        kind,
        TokenKind::Identifier(identifier)
            if !identifier.has_escape && matches!(identifier.value.as_str(), "of" | "yield")
    ) {
        return true;
    }
    !matches!(
        kind,
        TokenKind::Number(_)
            | TokenKind::String(_)
            | TokenKind::RegExp(_)
            | TokenKind::Identifier(_)
            | TokenKind::Keyword(Keyword::Null | Keyword::False | Keyword::True | Keyword::This)
            | TokenKind::Punctuator(
                Punctuator::RightParen
                    | Punctuator::RightBracket
                    | Punctuator::RightBrace
                    | Punctuator::Increment
                    | Punctuator::Decrement
            )
    )
}

fn parse_radix_literal(raw: &str, radix: NumericRadix) -> Result<f64, String> {
    let (digits, base) = match radix {
        NumericRadix::Binary => (raw.get(2..).unwrap_or_default(), 2),
        NumericRadix::Octal => (raw.get(2..).unwrap_or_default(), 8),
        NumericRadix::Decimal => (raw, 10),
        NumericRadix::Hexadecimal => (raw.get(2..).unwrap_or_default(), 16),
    };
    parse_digits(digits, base)
}

fn parse_digits(digits: &str, radix: u32) -> Result<f64, String> {
    if digits.is_empty() {
        return Err("numeric literal has no digits".to_owned());
    }
    let value = BigUint::parse_bytes(digits.as_bytes(), radix)
        .ok_or_else(|| format!("invalid base-{radix} numeric literal"))?;
    Ok(value.to_f64().unwrap_or(f64::INFINITY))
}

fn lex_error(error: LexError) -> Error {
    if error.kind == LexErrorKind::StringTooLong {
        Error::new(ErrorKind::JsInternal, error.message)
    } else {
        Error::syntax(error.message, source_span(error.span))
    }
}

fn source_offset(span: Span) -> Result<SourceOffset, Error> {
    SourceOffset::try_from_usize(span.start.byte_offset)
        .map_err(|error| Error::internal(error.to_string()))
}

const fn source_span(span: Span) -> SourceSpan {
    SourceSpan::new(
        SourceLocation::new(span.start.byte_offset, span.start.line, span.start.column),
        SourceLocation::new(span.end.byte_offset, span.end.line, span.end.column),
    )
}

#[cfg(test)]
mod tests;
