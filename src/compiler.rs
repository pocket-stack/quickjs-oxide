//! Source-to-bytecode compilation with late lexical-name resolution.
//!
//! QuickJS first emits scope-variable operations, then `resolve_scope_var`
//! rewrites them after every nested function and lexical scope is known.  Its
//! `get_closure_var` helper also installs relay closure slots on intervening
//! functions.  This module keeps the same boundary in typed form: parsing emits
//! [`IrOp`]s into a recursive [`FunctionIr`] arena, identifier resolution runs
//! child-first, and only then are VM instructions and recursive unlinked
//! function constants produced. The parser owns its Lexer and requests tokens
//! through fallible advances, so an error on later source cannot preempt a
//! diagnostic on the current token. Directive-prologue probes clone and seek
//! the lexer, then the committed stream is rescanned under its strict context.

use crate::atom::AtomTable;
use crate::bigint::JsBigInt;
use crate::bytecode::{
    ArgumentsKind, BytecodeFunction, DynamicEnvironmentSource, EvalVariableSource, Instruction,
    MAX_LOCAL_SLOTS, WithObjectSource, verify_parts,
};
use crate::debug::{DebugInfoMode, Pc2LineEntry, Pc2LineTable, QuickJsSourceLocator, SourceOffset};
use crate::error::{Error, ErrorKind, NativeErrorMessage, SourceLocation, SourceSpan};
use crate::function::{
    UnlinkedConstant, UnlinkedFunction, UnlinkedFunctionDebug, UnlinkedVariableDefinition,
};
use crate::heap::{
    ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName, ConstructorKind,
    EvalBinding, EvalBindingSource, EvalCallerProfile, EvalCallerVariableTarget, EvalEnvironment,
    EvalKind, EvalRootBinding, EvalScope, EvalScopeKind, EvalVariableEnvironment,
    FunctionKind as BytecodeFunctionKind, FunctionMetadata,
};
use crate::lexer::{
    Identifier, Keyword, LexError, LexErrorKind, Lexer, LexicalGoal, NumberKind, NumericRadix,
    Punctuator, Span, TemplatePartKind, Token, TokenKind,
};
use crate::value::{JsString, JsStringError, Value};
use num_bigint::BigUint;
use num_traits::ToPrimitive;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

mod arrow;
mod destructuring;
mod function;
mod object_literal;
mod pseudo_binding;
mod template;

use pseudo_binding::{
    HOME_OBJECT_LOCAL_NAME, NEW_TARGET_LOCAL_NAME, PseudoBinding, THIS_LOCAL_NAME,
    ensure_eval_visible_pseudo_bindings, find_or_create_own_pseudo_binding,
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
pub(crate) struct EvalCompileContext {
    pub kind: EvalKind,
    pub caller_strict: bool,
    pub bindings: Box<[EvalRootBinding<JsString>]>,
    pub caller_profile: EvalCallerProfile,
    pub super_call_allowed: bool,
    pub super_allowed: bool,
}

impl EvalCompileContext {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn direct(caller_strict: bool, bindings: Vec<EvalRootBinding<JsString>>) -> Self {
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
        let variable_target = if caller_strict {
            EvalCallerVariableTarget::StrictLocal
        } else {
            bindings
                .iter()
                .position(|binding| binding.kind == ClosureVariableKind::EvalVariableObject)
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

    pub(crate) fn direct_with_profile(
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
        }
    }

    pub(crate) fn indirect() -> Self {
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
        }
    }
}

/// Compile one ECMAScript script directly to stack bytecode.
///
/// # Errors
/// Returns a syntax error for invalid source and an unsupported diagnostic for
/// grammar which has not yet reached the feature-parity implementation path.
pub fn compile_script(source: &str) -> Result<BytecodeFunction, Error> {
    let mut tree = Parser::parse(source, JsString::from_static(DEFAULT_EVAL_FILENAME))?;
    if tree.functions.len() != 1 {
        return Err(Error::unsupported(
            "nested function bytecode requires runtime publication; use Context::compile or Context::eval",
            source_span(tree.functions[1].source.span),
        ));
    }
    resolve_identifiers(&mut tree)?;
    lower_detached_script(tree)
}

/// Compile a script into a runtime-independent draft ready for publication.
///
/// The ordinary public compiler result remains available for the detached VM,
/// while runtime publication uses this boundary to keep primitive constants
/// structural and to carry execution metadata into the heap node.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn compile_unlinked_script(source: &str) -> Result<UnlinkedFunction, Error> {
    compile_unlinked_script_with_filename(source, DEFAULT_EVAL_FILENAME, DebugInfoMode::Full)
}

pub(crate) fn compile_unlinked_script_with_filename(
    source: &str,
    filename: &str,
    debug_info: DebugInfoMode,
) -> Result<UnlinkedFunction, Error> {
    let mut tree = Parser::parse(source, JsString::try_from_utf8(filename)?)?;
    resolve_identifiers(&mut tree)?;
    lower_unlinked_tree(tree, debug_info)
}

/// Compile one primitive-String eval as an independent synthetic root.
///
/// This deliberately does not reuse the ordinary Script root. QuickJS gives
/// eval its own body environment and, for direct eval, attaches that root to
/// the active caller's VarRefs only while publishing and executing it.
pub(crate) fn compile_unlinked_eval_with_filename(
    source: &str,
    filename: &str,
    debug_info: DebugInfoMode,
    context: EvalCompileContext,
) -> Result<UnlinkedFunction, Error> {
    let mut tree = Parser::parse_eval(source, JsString::try_from_utf8(filename)?, context)?;
    resolve_identifiers(&mut tree)?;
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
    FunctionBody,
    ProgramBody,
    Block,
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
    bindings: Vec<BindingId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingKind {
    Normal,
    Lexical { is_const: bool },
    FunctionName { is_const: bool },
    EvalVariableObject,
    WithObject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingStorage {
    Argument(u16),
    Local(u16),
    /// Exact slot on a synthetic direct-eval root, supplied by the active
    /// caller environment rather than by a compiler-tree parent.
    External(u16),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Debug)]
enum IrOp {
    Bytecode(Instruction),
    /// Typed counterparts of QuickJS `OP_enter_scope` / `OP_leave_scope`.
    /// They remain scope identities until every declaration and child capture
    /// is known, then lowering expands entry to lexical TDZ initialization and
    /// exit to `CloseLocal` for exactly the locals captured by children.
    EnterScope(ScopeId),
    LeaveScope(ScopeId),
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
    /// non-String shell intentionally publishes only the argument count.
    EvalCall {
        argument_count: u16,
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
    /// QuickJS's `has_iterator` BlockEnv. Its target depth retains the
    /// conceptual `iterator`, `next`, and private unwind marker slots. A
    /// same-loop continue keeps that record, a break reaches the shared close
    /// tail, and an edge crossing the loop closes it immediately.
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
            Self::EnterScope(_) | Self::LeaveScope(_) => (0, 0),
            Self::TemplateCall {
                argument_count,
                method,
            } => (argument_count + usize::from(*method) + 1, 1),
            Self::EvalCall { argument_count, .. } => (usize::from(*argument_count) + 1, 1),
            Self::PushConstant(_) | Self::MakeClosure(_) => (0, 1),
            Self::GlobalSet(_) | Self::CapturedLexicalSet(_) => (1, 1),
            Self::DynamicIdentifier { access, .. } => match access {
                IdentifierAccess::Get
                | IdentifierAccess::GetOrUndefined
                | IdentifierAccess::Delete => (0, 1),
                IdentifierAccess::Initialize
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
                    IdentifierAccess::Initialize | IdentifierAccess::Put | IdentifierAccess::AnnexBPut,
                ..
            } => (1, 0),
            Self::Identifier {
                access: IdentifierAccess::Set,
                ..
            } => (1, 1),
        }
    }
}

#[derive(Debug)]
struct FunctionIr {
    /// Parent function plus the scope which was current at this function's
    /// definition. This is QuickJS `parent` + `parent_scope_level` as one
    /// invariant-preserving typed link.
    parent: Option<ParentLink>,
    kind: FunctionKind,
    /// QuickJS parser authority copied independently from HomeObject storage.
    super_call_allowed: bool,
    super_allowed: bool,
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
    /// implicit binding. An explicit `arguments` parameter suppresses it.
    arguments_local: Option<u16>,
    /// Lazily materialized QuickJS pseudo variables captured by descendant
    /// arrows or exposed to direct eval. Arrow frames never own these locals;
    /// only concise methods can own the HomeObject cell.
    home_object_local: Option<u16>,
    this_local: Option<u16>,
    new_target_local: Option<u16>,
    /// Hidden null-prototype variable object for sloppy authored function code
    /// containing syntactic direct eval. Its identity is explicit rather than
    /// inferred from local allocation order.
    eval_variable_object_local: Option<u16>,
    /// A lazily allocated HomeObject pseudo local requires the published
    /// method function to retain its object literal as HomeObject. Descendant
    /// arrows relay the local without carrying this metadata themselves.
    needs_home_object: bool,
    parameters: Vec<String>,
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
    parameters: Vec<String>,
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
                bindings: Vec::new(),
            },
            IrScope {
                parent: Some(function_root),
                kind: if matches!(kind, FunctionKind::Script | FunctionKind::Eval(_)) {
                    ScopeKind::ProgramBody
                } else {
                    ScopeKind::FunctionBody
                },
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
            super_call_allowed: super_capabilities.super_call_allowed,
            super_allowed: super_capabilities.super_allowed,
            source,
            function_name: options.function_name,
            private_name_binding: options.private_name_binding,
            function_name_local: None,
            arguments_local: None,
            home_object_local: None,
            this_local: None,
            new_target_local: None,
            eval_variable_object_local: None,
            needs_home_object: false,
            parameters: options.parameters,
            locals,
            scopes,
            bindings: Vec::new(),
            global_declarations: Vec::new(),
            hoisted_functions: Vec::new(),
            eval_declarations: Vec::new(),
            eval_declarations_installed: false,
            eval_redeclaration: None,
            function_hoists_installed: false,
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
            let index = u16::try_from(index)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
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
        binding.is_catch_parameter != (scope_kind == EvalScopeKind::Catch)
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
    let has_variable_object = bindings
        .iter()
        .any(|binding| binding.kind == ClosureVariableKind::EvalVariableObject);
    match (caller_strict, caller_profile.variable_target) {
        (false, EvalCallerVariableTarget::Global) if !has_variable_object => {}
        (true, EvalCallerVariableTarget::StrictLocal) if kind == EvalKind::Direct => {}
        (false, EvalCallerVariableTarget::ExternalBinding(index))
            if bindings.get(usize::from(index)).is_some_and(|binding| {
                binding.kind == ClosureVariableKind::EvalVariableObject
                    && !binding.is_lexical
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
        let kind = match binding.kind {
            ClosureVariableKind::Normal if binding.is_lexical => BindingKind::Lexical {
                is_const: binding.is_const,
            },
            ClosureVariableKind::Normal if !binding.is_const => BindingKind::Normal,
            ClosureVariableKind::FunctionName if !binding.is_lexical => BindingKind::FunctionName {
                is_const: binding.is_const,
            },
            ClosureVariableKind::EvalVariableObject if !binding.is_lexical && !binding.is_const => {
                BindingKind::EvalVariableObject
            }
            ClosureVariableKind::WithObject if !binding.is_lexical && !binding.is_const => {
                BindingKind::WithObject
            }
            ClosureVariableKind::Normal
            | ClosureVariableKind::FunctionName
            | ClosureVariableKind::GlobalFunction
            | ClosureVariableKind::EvalVariableObject
            | ClosureVariableKind::WithObject => {
                return Err(Error::internal(
                    "eval caller binding flags are inconsistent",
                ));
            }
        };
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
    source: Box<str>,
    filename: JsString,
}

#[derive(Clone, Copy, Debug)]
struct PreparedScopedFunction {
    binding: BindingId,
    create_annex_binding: bool,
}

struct Parser<'source> {
    lexer: Lexer<'source>,
    tokens: Vec<Token<'source>>,
    cursor: usize,
    current_function: FunctionId,
    in_mode: InMode,
    functions: Vec<FunctionIr>,
    /// Function expression eligible for QuickJS's assignment-name inference.
    /// Operators which make the surrounding expression cease to be an
    /// AnonymousFunctionDefinition clear this marker.
    anonymous_function_definition: Option<FunctionId>,
}

enum RootCompileContext {
    Script,
    Eval(EvalCompileContext),
}

impl<'source> Parser<'source> {
    fn parse(source: &'source str, filename: JsString) -> Result<FunctionTree, Error> {
        Self::parse_root(source, filename, RootCompileContext::Script)
    }

    fn parse_eval(
        source: &'source str,
        filename: JsString,
        context: EvalCompileContext,
    ) -> Result<FunctionTree, Error> {
        Self::parse_root(source, filename, RootCompileContext::Eval(context))
    }

    fn parse_root(
        source: &'source str,
        filename: JsString,
        context: RootCompileContext,
    ) -> Result<FunctionTree, Error> {
        if source.len() > i32::MAX as usize {
            return Err(Error::new(
                ErrorKind::JsInternal,
                "source is too large for QuickJS debug metadata",
            ));
        }
        let (root_kind, inherited_strict, external_bindings, caller_profile, super_capabilities) =
            match context {
                RootCompileContext::Script => (
                    FunctionKind::Script,
                    false,
                    Vec::<EvalRootBinding<JsString>>::new().into_boxed_slice(),
                    EvalCallerProfile {
                        scope_kinds: Box::new([]),
                        variable_target: EvalCallerVariableTarget::Global,
                    },
                    SuperCapabilities::NONE,
                ),
                RootCompileContext::Eval(context) => {
                    if !matches!(context.kind, EvalKind::Direct | EvalKind::Indirect) {
                        return Err(Error::internal(
                            "eval compiler received a non-eval root kind",
                        ));
                    }
                    if context.kind == EvalKind::Indirect
                        && (!context.bindings.is_empty()
                            || !context.caller_profile.scope_kinds.is_empty()
                            || context.caller_profile.variable_target
                                != EvalCallerVariableTarget::Global
                            || context.super_call_allowed
                            || context.super_allowed)
                    {
                        return Err(Error::internal(
                            "indirect eval compiler received a caller environment",
                        ));
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
                    )
                }
            };
        let mut lexer = Lexer::new(source);
        let first_token = lexer.next_token().map_err(lex_error)?;
        let source_span = first_token.span;
        let mut parser = Self {
            lexer,
            tokens: vec![first_token],
            cursor: 0,
            current_function: 0,
            in_mode: InMode::Allow,
            anonymous_function_definition: None,
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
                    function_name: Some("<eval>".to_owned()),
                    private_name_binding: false,
                    parameters: Vec::new(),
                    strict: inherited_strict,
                    super_capabilities,
                },
            )?],
        };
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
        parser.parse_script_body()?;
        Ok(FunctionTree {
            functions: parser.functions,
            source: source.into(),
            filename,
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
        self.emit_instruction(Instruction::Return)?;
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
                if matches!(self.current_ir().kind, FunctionKind::Script)
                    && position == StatementPosition::ProgramBody
                {
                    self.parse_program_function_declaration()
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
                    Err(self.syntax_here(
                        "function declarations can't appear in single-statement context",
                    ))
                }
            }
            TokenKind::Keyword(Keyword::Var) => self.parse_var_statement(),
            TokenKind::Keyword(Keyword::Return) => {
                if matches!(
                    self.current_ir().kind,
                    FunctionKind::Script | FunctionKind::Eval(_)
                ) {
                    Err(self.syntax_here("return not in a function"))
                } else {
                    self.parse_return_statement()
                }
            }
            TokenKind::Keyword(Keyword::Throw) => self.parse_throw_statement(),
            _ => self.parse_expression_statement(completion),
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
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Await))
            || matches!(
                &self.current().kind,
                TokenKind::Identifier(identifier)
                    if identifier.value == "await" && !identifier.has_escape
            )
        {
            return Err(self.unsupported_here("for-await-of loops are not implemented yet"));
        }
        let classic_head = self.for_head_has_top_level_semicolon();
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

    /// Lower the simple-binding synchronous half of QuickJS
    /// `js_parse_for_in_of`. The assignment fragment is emitted before the
    /// enumerated expression and skipped on first entry, just as upstream
    /// does; each `done == false` edge jumps back with the yielded value above
    /// the retained for-in object or for-of iterator record.
    fn parse_for_in_of_statement(
        &mut self,
        completion: StatementCompletion,
        label_name: Option<String>,
        entry_depth: usize,
        outer_scope: ScopeId,
        scope: ScopeId,
    ) -> Result<(), Error> {
        let iteration_hint = self
            .for_iteration_kind_ahead()
            .ok_or_else(|| self.syntax_here("expected 'of' or 'in' in for control expression"))?;
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
        let target = self.parse_for_iteration_assignment_target(iteration_hint)?;
        self.require_stack_depth(entry_depth + retained_slots, "for-in/of assignment target")?;
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
        self.emit_instruction(match iteration_kind {
            ForIterationKind::In => Instruction::ForInStart,
            ForIterationKind::Of => Instruction::ForOfStart,
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
        self.emit_instruction(match iteration_kind {
            ForIterationKind::In => Instruction::ForInNext,
            ForIterationKind::Of => Instruction::ForOfNext(0),
        })?;
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
    ) -> Result<ForAssignmentTargetInfo, Error> {
        let loop_name = if iteration_kind == ForIterationKind::In {
            "for-in"
        } else {
            "for-of"
        };
        if matches!(
            self.current().kind,
            TokenKind::Punctuator(Punctuator::LeftBrace | Punctuator::LeftBracket)
        ) {
            return Err(self.unsupported_here(format!(
                "{loop_name} destructuring assignment patterns are not implemented yet"
            )));
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
                return Err(self.unsupported_here(format!(
                    "{loop_name} destructuring bindings are not implemented yet"
                )));
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
                return Err(self.unsupported_here(format!(
                    "{loop_name} destructuring bindings are not implemented yet"
                )));
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
        self.parse_left_hand_side_expression()?;
        if let Some(async_span) = async_span
            && iteration_kind == ForIterationKind::Of
            && self.is_for_of_keyword()
        {
            return Err(Error::syntax(
                "'for of' expression cannot start with 'async'",
                source_span(async_span),
            ));
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
            return Err(self.syntax_here("invalid assignment left-hand side"));
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
        // parsing the handler's linear IR.
        let catch_target = self.current_ir().ops.len();
        self.patch_jump(catch_jump, catch_target)?;
        self.current_ir_mut().stack_depth = entry_depth + 1;

        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Catch)) {
            self.advance()?;
            let catch_scope = self.push_scope(ScopeKind::Catch);

            if self.is_punctuator(Punctuator::LeftBrace) {
                // Optional catch binding: discard the exception before the
                // catch body installs its own protection marker.
                self.emit_instruction(Instruction::Drop)?;
            } else {
                self.expect_punctuator(Punctuator::LeftParen)?;
                if self.is_punctuator(Punctuator::LeftBrace)
                    || self.is_punctuator(Punctuator::LeftBracket)
                {
                    return Err(self
                        .unsupported_here("catch destructuring bindings are not implemented yet"));
                }
                let token = self.current().clone();
                let TokenKind::Identifier(identifier) = token.kind else {
                    return Err(self.syntax_here("identifier expected"));
                };
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
                let catch_binding = self
                    .current_ir()
                    .binding_id_in_scope(catch_scope, &name)
                    .ok_or_else(|| Error::internal("catch binding was not registered"))?;
                self.current_ir_mut().bindings[catch_binding.0].is_catch_parameter = true;
                self.emit_identifier(name, token.span, IdentifierAccess::Initialize)?;
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

    /// QuickJS changes a for-of `BlockEnv`'s scope level back to the level
    /// outside the enumeration scope. Thus a same-loop break/continue closes
    /// the current lexical head cell while retaining the three-slot iterator
    /// record for the loop's shared next/close tail.
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
        let statement_depth = self.current_ir().stack_depth;
        let return_span = self.current().span;
        self.advance()?;
        if self.current().line_terminator_before
            || self.at_eof()
            || self.is_punctuator(Punctuator::Semicolon)
            || self.is_punctuator(Punctuator::RightBrace)
        {
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
                    BreakControlKind::ForOf | BreakControlKind::TryFinally
                )
                .then_some((index, control.kind, control.entry_depth))
            })
            .collect::<Vec<_>>();
        for (control_index, kind, handler_depth) in unwind_controls {
            match kind {
                BreakControlKind::ForOf => {
                    if handler_depth < 3 || handler_depth >= self.current_ir().stack_depth {
                        return Err(Error::internal(
                            "return unwind targeted an invalid iterator record",
                        ));
                    }
                    self.emit_instruction(Instruction::IteratorClosePreserve)?;
                    // The generic instruction effect is value preserving, but
                    // this typed form also truncates the complete iterator
                    // record and any intermediate finally operands.
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
        self.emit_instruction_at(Instruction::Return, source_offset(return_span)?)?;
        self.consume_statement_terminator()?;
        // Parsing continues through unreachable source. Retain the enclosing
        // statement's marker/discriminant shape just as break/continue do.
        self.current_ir_mut().stack_depth = statement_depth;
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
            } else {
                if self.is_punctuator(Punctuator::LeftBrace) {
                    return Err(self.unsupported_here(
                        "lexical destructuring bindings are not implemented yet",
                    ));
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
                    if self.anonymous_function_definition.take().is_some() {
                        let name_constant = self.add_constant(IrConstant::Primitive(
                            Value::String(JsString::try_from_utf8(&name)?),
                        ))?;
                        self.emit_instruction(Instruction::SetName(name_constant))?;
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
                    if self.anonymous_function_definition.take().is_some() {
                        // QuickJS emits a dummy OP_set_name after an anonymous
                        // closure and rewrites its atom when NamedEvaluation
                        // applies to this initializer. Keep that contextual name
                        // separate from the child bytecode's intrinsic func_name.
                        let name_constant = self.add_constant(IrConstant::Primitive(
                            Value::String(JsString::try_from_utf8(&name)?),
                        ))?;
                        self.emit_instruction(Instruction::SetName(name_constant))?;
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
        let function = &mut self.functions[self.current_function];
        let selects_arguments_object =
            matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
                && name == "arguments"
                && !function
                    .parameters
                    .iter()
                    .any(|parameter| parameter == "arguments");
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
                        binding.kind == ClosureVariableKind::EvalVariableObject
                            && !binding.is_lexical
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
                if binding.kind != ClosureVariableKind::EvalVariableObject {
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
            let kind = match binding.kind {
                ClosureVariableKind::Normal if binding.is_lexical => BindingKind::Lexical {
                    is_const: binding.is_const,
                },
                ClosureVariableKind::Normal if !binding.is_const => BindingKind::Normal,
                ClosureVariableKind::FunctionName if !binding.is_lexical => {
                    BindingKind::FunctionName {
                        is_const: binding.is_const,
                    }
                }
                ClosureVariableKind::Normal
                | ClosureVariableKind::FunctionName
                | ClosureVariableKind::GlobalFunction
                | ClosureVariableKind::EvalVariableObject
                | ClosureVariableKind::WithObject => {
                    return Err(Error::internal(
                        "eval caller binding flags are inconsistent",
                    ));
                }
            };
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
        let function = &mut self.functions[self.current_function];
        let scope = function.current_scope;
        let scope_kind = function.scopes[scope.0].kind;
        let is_global = matches!(scope_kind, ScopeKind::ProgramBody)
            && matches!(function.kind, FunctionKind::Script)
            && scope == function.body_scope;
        let is_eval_body = matches!(scope_kind, ScopeKind::ProgramBody)
            && matches!(function.kind, FunctionKind::Eval(_))
            && scope == function.body_scope;
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
            || matches!(
                scope_kind,
                ScopeKind::Block
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
                (BindingStorage::Local(_), BindingKind::EvalVariableObject) => "",
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
                (BindingStorage::External(_), _) => "",
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
        }
        Ok(())
    }

    /// Parse assignment targets through typed unresolved References. Keeping
    /// identifier writes unresolved lets the late resolver select argument,
    /// local, closure, global and private-function-name behavior after the
    /// complete nested scope tree is known.
    fn parse_assignment(&mut self) -> Result<(), Error> {
        if self.async_arrow_ahead() {
            return Err(self.unsupported_here("async arrow functions are not implemented yet"));
        }
        if self.reserved_arrow_head_ahead() {
            return Err(self.syntax_here("invalid arrow function parameter"));
        }
        if let Some(head) = self.arrow_head_ahead() {
            return self.parse_arrow_function(head);
        }
        // QuickJS's `name0` is captured only when the AssignmentExpression
        // starts with the identifier token itself. Parenthesized lvalues are
        // valid References but intentionally do not trigger NamedEvaluation.
        let direct_identifier_name = match &self.current().kind {
            TokenKind::Identifier(identifier) => Some(identifier.value.clone()),
            _ => None,
        };
        self.parse_conditional()?;
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
            let anonymous_rhs = self.anonymous_function_definition.take().is_some();
            if direct_identifier_name.as_deref() == Some(target.name.as_str()) && anonymous_rhs {
                let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::try_from_utf8(&target.name)?,
                )))?;
                self.emit_instruction(Instruction::SetName(name_constant))?;
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
            return Err(self.syntax_here("invalid assignment left-hand side"));
        };
        self.advance()?;
        let rhs_start = self.current_ir().ops.len();
        self.parse_assignment()?;
        let site = match target {
            MemberReference::Field { site, .. }
            | MemberReference::Computed { site }
            | MemberReference::Super { site } => site,
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
        let anonymous_rhs = self.anonymous_function_definition.take().is_some();
        if infer_name && anonymous_rhs {
            let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::try_from_utf8(&target.name)?,
            )))?;
            self.emit_instruction(Instruction::SetName(name_constant))?;
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
            return Err(self.syntax_here("invalid assignment left-hand side"));
        };
        let lvalue_depth = match target {
            MemberReference::Field { .. } => 1,
            MemberReference::Computed { .. } => 2,
            MemberReference::Super { .. } => 3,
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
        self.parse_shift()?;
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
        self.parse_primary()?;
        loop {
            if self.parse_member_suffix()? {
                continue;
            }

            if self.is_punctuator(Punctuator::LeftParen) {
                let call_span = self.current().span;
                let member_method = self.promote_last_member_get_for_call()?;
                // QuickJS selects OP_eval from the syntactic callee before
                // binding resolution. Parentheses preserve IdentifierReference,
                // while comma/member/other composed expressions have already
                // cleared this marker. Runtime identity still decides whether
                // this is original eval or an ordinary replacement call.
                let direct_eval_scope = if member_method {
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
                self.advance()?;
                let argument_count = self.parse_call_arguments()?;
                if let Some(scope) = direct_eval_scope {
                    self.emit_at(
                        IrOp::EvalCall {
                            argument_count,
                            scope,
                            environment: None,
                        },
                        source_offset(call_span)?,
                    )?;
                } else {
                    let instruction = if member_method || identifier_method {
                        Instruction::CallMethod(argument_count)
                    } else {
                        Instruction::Call(argument_count)
                    };
                    self.emit_instruction_at(instruction, source_offset(call_span)?)?;
                }
                self.anonymous_function_definition = None;
                continue;
            }
            if self.parse_tagged_template_suffix()? {
                continue;
            }
            break;
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
            _ => false,
        };
        if promoted {
            function.stack_depth = function
                .stack_depth
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
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
        }
        Ok(())
    }

    /// Parse the contents of an already-consumed call/construct `(`.
    fn parse_call_arguments(&mut self) -> Result<u16, Error> {
        let mut argument_count = 0_usize;
        if !self.consume_punctuator(Punctuator::RightParen)? {
            loop {
                // QuickJS accepts 65,535 encoded arguments and only rejects
                // the next one in `js_parse_postfix_expr`. The accepted
                // boundary is subsequently rejected as a JavaScript-visible
                // stack overflow when bytecode stack size is computed.
                if argument_count >= MAX_CALL_ARGUMENTS {
                    return Err(self.syntax_here("Too many call arguments"));
                }
                // Call arguments use AssignmentExpression with `in` enabled,
                // even when the surrounding expression is a classic-for NoIn
                // initializer.
                self.parse_assignment_allow_in()?;
                argument_count += 1;
                if !self.consume_punctuator(Punctuator::Comma)? {
                    self.expect_punctuator(Punctuator::RightParen)?;
                    break;
                }
                if self.consume_punctuator(Punctuator::RightParen)? {
                    break;
                }
            }
        }
        u16::try_from(argument_count)
            .map_err(|_| Error::internal("call argument count escaped the parser limit"))
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
        self.parse_primary()?;
        loop {
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
        let (argument_count, construct_span) = if self.is_punctuator(Punctuator::LeftParen) {
            let call_span = self.current().span;
            self.advance()?;
            (self.parse_call_arguments()?, call_span)
        } else {
            (0, no_arguments_span)
        };
        self.emit_instruction_at(
            Instruction::Construct(argument_count),
            source_offset(construct_span)?,
        )?;
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
                return Err(Error::syntax(
                    "super() is only valid in a derived class constructor",
                    source_span(super_span),
                ));
            }
            return Err(Error::unsupported(
                "derived constructor super() is not implemented yet",
                source_span(super_span),
            ));
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

    fn parse_primary(&mut self) -> Result<(), Error> {
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
                if matches!(
                    self.current_ir().kind,
                    FunctionKind::Arrow | FunctionKind::Eval(EvalKind::Direct)
                ) {
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
            TokenKind::Identifier(identifier) => {
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
            TokenKind::Keyword(Keyword::New) => {
                self.parse_new_expression()?;
            }
            TokenKind::Keyword(Keyword::Super) => {
                self.parse_super_property(token.span)?;
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
                keyword @ (Keyword::Else
                | Keyword::Case
                | Keyword::Default
                | Keyword::Catch
                | Keyword::Finally),
            ) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    keyword.as_str()
                )));
            }
            TokenKind::Keyword(keyword) => {
                return Err(Error::unsupported(
                    format!("{} syntax is not implemented yet", keyword.as_str()),
                    source_span(token.span),
                ));
            }
            TokenKind::RegExp(literal) => {
                // Pinned QuickJS calls `compile_regexp` before advancing to the
                // next token. Preserve that diagnostic precedence and retain
                // the resulting program in immutable bytecode so evaluation
                // only allocates a fresh branded object.
                let pattern = JsString::try_from_utf8(literal.pattern)?;
                let flags = JsString::try_from_utf8(literal.flags)?;
                let program = crate::regexp::compile(&pattern, &flags).map_err(|error| {
                    let kind = crate::regexp::javascript_compile_error_kind(&error);
                    let message = if kind == ErrorKind::Unsupported {
                        error.to_string()
                    } else {
                        crate::regexp::javascript_compile_error_message(&error).to_owned()
                    };
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
            TokenKind::PrivateIdentifier(_) => {
                return Err(self.unsupported_here("this literal form is not implemented yet"));
            }
            TokenKind::Punctuator(punctuator) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    punctuator.as_str()
                )));
            }
            TokenKind::RawAscii(byte) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    char::from(byte)
                )));
            }
            TokenKind::Eof => {
                return Err(self.syntax_here("unexpected token in expression: ''"));
            }
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
        let prepared = self.prepare_scoped_function(&name, declaration_span)?;
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
        let create_annex_binding = self.scoped_function_is_annex_b_eligible(name);
        let conflict_span = self.current().span;
        let binding =
            self.register_scoped_function_binding(name, declaration_span, conflict_span)?;
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
                .parameters
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
    ) -> Result<BindingId, Error> {
        let scope = self.current_ir().current_scope;
        if let Some(existing) = self.current_ir().binding_id_in_scope(scope, name) {
            let duplicate_function = self.current_ir().bindings[existing.0].is_scoped_function;
            if self.current_ir().strict || !duplicate_function {
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
            return Ok(binding);
        }

        self.register_lexical_binding(name, declaration_span, conflict_span, false, true)?;
        let binding = self
            .current_ir()
            .binding_id_in_scope(scope, name)
            .ok_or_else(|| Error::internal("scoped function binding was not registered"))?;
        self.current_ir_mut().bindings[binding.0].is_scoped_function = true;
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
        let async_function = matches!(
            &self.current().kind,
            TokenKind::Identifier(identifier)
                if identifier.value == "async" && !identifier.has_escape
        );
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
            Ok(!next.line_terminator_before
                && matches!(next.kind, TokenKind::Keyword(Keyword::Function)))
        }
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
                FunctionKind::Script | FunctionKind::Eval(EvalKind::Indirect) => return false,
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
        let position = self.current().span.start;
        self.tokens.truncate(self.cursor);
        self.lexer.seek(position);
        let mut context = self.lexer.context();
        context.strict = strict;
        self.lexer.set_context(context);
        self.ensure_token(self.cursor)
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

    fn push_scope(&mut self, kind: ScopeKind) -> ScopeId {
        let function = self.current_ir_mut();
        let parent = function.current_scope;
        let scope = ScopeId(function.scopes.len());
        function.scopes.push(IrScope {
            parent: Some(parent),
            kind,
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
    let atom = JsString::try_from_utf8(atom)?;
    let mut message = NativeErrorMessage::new();
    message.push_utf8(prefix);
    atom.push_atom_get_str_to(&mut message);
    message.push_utf8(suffix);
    Ok(Error::from_native_message(ErrorKind::Syntax, message).with_span(source_span(span)))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResolvedBinding {
    storage: BindingStorage,
    kind: BindingKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FunctionResolutionEvent {
    Enter(FunctionId),
    Resolve(FunctionId),
}

fn function_resolution_events(tree: &FunctionTree) -> Result<Vec<FunctionResolutionEvent>, Error> {
    if tree.functions.is_empty() {
        return Err(Error::internal("compiler produced no root function"));
    }
    let mut children = vec![Vec::new(); tree.functions.len()];
    for (function_id, function) in tree.functions.iter().enumerate().skip(1) {
        let parent = function
            .parent
            .ok_or_else(|| Error::internal("non-root function has no parent"))?
            .function;
        if parent >= function_id {
            return Err(Error::internal(
                "function parent must precede its child in the arena",
            ));
        }
        let siblings = children
            .get_mut(parent)
            .ok_or_else(|| Error::internal("function parent is out of bounds"))?;
        siblings.push(function_id);
    }
    if tree.functions[0].parent.is_some() {
        return Err(Error::internal("root function unexpectedly has a parent"));
    }

    let mut events = Vec::with_capacity(tree.functions.len().saturating_mul(2));
    let mut stack = vec![(0_usize, false)];
    while let Some((function_id, visited)) = stack.pop() {
        if visited {
            events.push(FunctionResolutionEvent::Resolve(function_id));
            continue;
        }
        events.push(FunctionResolutionEvent::Enter(function_id));
        stack.push((function_id, true));
        for &child in children[function_id].iter().rev() {
            stack.push((child, false));
        }
    }
    if events.len() != tree.functions.len().saturating_mul(2) {
        return Err(Error::internal("function arena is not one rooted tree"));
    }
    Ok(events)
}

fn validate_scope_graph(tree: &FunctionTree) -> Result<(), Error> {
    for (function_id, function) in tree.functions.iter().enumerate() {
        if function.scopes.len() < 2
            || function.var_scope != ScopeId(0)
            || function.current_scope != function.body_scope
            || function.scopes[0].parent.is_some()
            || function.scopes[0].kind != ScopeKind::FunctionRoot
            || function.body_scope.0 >= function.scopes.len()
            || function.body_scope == function.var_scope
        {
            return Err(Error::internal("function scope roots are malformed"));
        }
        let expected_body = if matches!(function.kind, FunctionKind::Script | FunctionKind::Eval(_))
        {
            ScopeKind::ProgramBody
        } else {
            ScopeKind::FunctionBody
        };
        if function.scopes[function.body_scope.0].kind != expected_body {
            return Err(Error::internal("function body scope kind is malformed"));
        }
        if function.super_call_allowed && !function.super_allowed {
            return Err(Error::internal(
                "function permits super() without SuperProperty",
            ));
        }
        match function.kind {
            FunctionKind::Method if !function.super_call_allowed && function.super_allowed => {}
            FunctionKind::Arrow => {
                let parent = function
                    .parent
                    .and_then(|parent| tree.functions.get(parent.function))
                    .ok_or_else(|| Error::internal("arrow function has no valid parent"))?;
                if (function.super_call_allowed, function.super_allowed)
                    != (parent.super_call_allowed, parent.super_allowed)
                {
                    return Err(Error::internal(
                        "arrow super capability disagrees with its parent",
                    ));
                }
            }
            FunctionKind::Eval(EvalKind::Direct) => {}
            FunctionKind::Script
            | FunctionKind::Ordinary
            | FunctionKind::Eval(EvalKind::Indirect)
                if !function.super_call_allowed && !function.super_allowed => {}
            FunctionKind::Eval(EvalKind::None) => {
                return Err(Error::internal("eval root has no eval kind"));
            }
            FunctionKind::Method
            | FunctionKind::Script
            | FunctionKind::Ordinary
            | FunctionKind::Eval(EvalKind::Indirect) => {
                return Err(Error::internal(
                    "function kind retained malformed super capability",
                ));
            }
        }
        if function.private_name_binding
            && (!matches!(function.kind, FunctionKind::Ordinary)
                || function.function_name.is_none())
        {
            return Err(Error::internal(
                "private function-name capability is malformed",
            ));
        }
        if matches!(function.kind, FunctionKind::Script)
            && (!function.hoisted_functions.is_empty() || function.function_hoists_installed)
        {
            return Err(Error::internal(
                "root script contains ordinary function-body hoists",
            ));
        }
        match function.kind {
            FunctionKind::Eval(EvalKind::Direct) => {
                if function.external_bindings.len() > function.closure_variables.len() {
                    return Err(Error::internal(
                        "direct eval external bindings exceed closure slots",
                    ));
                }
                if function.external_bindings.iter().any(|binding| {
                    let Some(&scope_kind) = function
                        .eval_caller_profile
                        .scope_kinds
                        .get(usize::from(binding.scope))
                    else {
                        return true;
                    };
                    binding.is_catch_parameter != (scope_kind == EvalScopeKind::Catch)
                        || (binding.kind == ClosureVariableKind::WithObject)
                            != (scope_kind == EvalScopeKind::With)
                }) || function
                    .eval_caller_profile
                    .scope_kinds
                    .iter()
                    .enumerate()
                    .any(|(scope, kind)| {
                        *kind == EvalScopeKind::With
                            && function
                                .external_bindings
                                .iter()
                                .filter(|binding| usize::from(binding.scope) == scope)
                                .count()
                                != 1
                    })
                {
                    return Err(Error::internal(
                        "direct eval bindings disagree with the caller scope profile",
                    ));
                }
                match function.eval_caller_profile.variable_target {
                    EvalCallerVariableTarget::Global => {}
                    EvalCallerVariableTarget::StrictLocal if function.strict => {}
                    EvalCallerVariableTarget::ExternalBinding(index)
                        if function
                            .external_bindings
                            .get(usize::from(index))
                            .is_some_and(|binding| {
                                binding.kind == ClosureVariableKind::EvalVariableObject
                                    && !binding.is_lexical
                                    && !binding.is_const
                                    && !binding.is_catch_parameter
                            }) => {}
                    EvalCallerVariableTarget::StrictLocal
                    | EvalCallerVariableTarget::ExternalBinding(_) => {
                        return Err(Error::internal(
                            "direct eval caller variable target is malformed",
                        ));
                    }
                }
            }
            FunctionKind::Eval(EvalKind::Indirect) => {
                if !function.external_bindings.is_empty()
                    || !function.eval_caller_profile.scope_kinds.is_empty()
                    || function.eval_caller_profile.variable_target
                        != EvalCallerVariableTarget::Global
                {
                    return Err(Error::internal(
                        "indirect eval retained a caller environment",
                    ));
                }
            }
            FunctionKind::Eval(EvalKind::None) => {
                return Err(Error::internal("eval root has no eval kind"));
            }
            FunctionKind::Script
            | FunctionKind::Ordinary
            | FunctionKind::Method
            | FunctionKind::Arrow => {
                if !function.external_bindings.is_empty()
                    || !function.eval_caller_profile.scope_kinds.is_empty()
                    || function.eval_caller_profile.variable_target
                        != EvalCallerVariableTarget::Global
                {
                    return Err(Error::internal(
                        "non-eval function retained an eval caller environment",
                    ));
                }
            }
        }
        if let Some(index) = function.arguments_local {
            let matches_binding = function.bindings.iter().any(|binding| {
                binding.name == "arguments"
                    && binding.storage_scope == function.var_scope
                    && binding.kind == BindingKind::Normal
                    && binding.storage == BindingStorage::Local(index)
            });
            if !matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
                || usize::from(index) >= function.locals.len()
                || function.locals[usize::from(index)] != "arguments"
                || function
                    .parameters
                    .iter()
                    .any(|parameter| parameter == "arguments")
                || !matches_binding
            {
                return Err(Error::internal(
                    "implicit arguments local metadata is malformed",
                ));
            }
        }
        for (pseudo, local) in [
            (PseudoBinding::HomeObject, function.home_object_local),
            (PseudoBinding::This, function.this_local),
            (PseudoBinding::NewTarget, function.new_target_local),
        ] {
            let local_bindings = function
                .bindings
                .iter()
                .filter(|binding| {
                    binding.name == pseudo.name()
                        && matches!(binding.storage, BindingStorage::Local(_))
                })
                .collect::<Vec<_>>();
            match (local, local_bindings.as_slice()) {
                (Some(index), [binding])
                    if function_owns_pseudo_binding(function.kind, pseudo)
                        && usize::from(index) < function.locals.len()
                        && function.locals[usize::from(index)] == pseudo.name()
                        && binding.storage == BindingStorage::Local(index)
                        && binding.kind == BindingKind::Normal
                        && binding.storage_scope == function.var_scope
                        && binding.declaration_scope == function.var_scope
                        && binding.declaration_span.is_none() =>
                {
                    let initialized = function
                        .ops
                        .windows(2)
                        .filter(|window| {
                            matches!(
                                (&window[0].op, &window[1].op, pseudo),
                                (
                                    IrOp::Bytecode(Instruction::PushHomeObject),
                                    IrOp::Bytecode(Instruction::PutLocal(target)),
                                    PseudoBinding::HomeObject,
                                ) if *target == index
                            ) || matches!(
                                (&window[0].op, &window[1].op, pseudo),
                                (
                                    IrOp::Bytecode(Instruction::PushThis),
                                    IrOp::Bytecode(Instruction::PutLocal(target)),
                                    PseudoBinding::This,
                                ) if *target == index
                            ) || matches!(
                                (&window[0].op, &window[1].op, pseudo),
                                (
                                    IrOp::Bytecode(Instruction::PushNewTarget),
                                    IrOp::Bytecode(Instruction::PutLocal(target)),
                                    PseudoBinding::NewTarget,
                                ) if *target == index
                            )
                        })
                        .count();
                    if initialized != 1 {
                        return Err(Error::internal(
                            "pseudo local entry initialization is malformed",
                        ));
                    }
                }
                (None, []) => {}
                _ => {
                    return Err(Error::internal(
                        "pseudo local binding metadata is malformed",
                    ));
                }
            }
        }
        if function.home_object_local.is_some()
            && (!function.needs_home_object || !function.super_allowed)
        {
            return Err(Error::internal(
                "HomeObject pseudo local lacks publication metadata",
            ));
        }
        if function.needs_home_object && !matches!(function.kind, FunctionKind::Method) {
            return Err(Error::internal(
                "non-method function retained HomeObject metadata",
            ));
        }
        let eval_variable_object_bindings = function
            .bindings
            .iter()
            .filter(|binding| {
                binding.kind == BindingKind::EvalVariableObject
                    && matches!(binding.storage, BindingStorage::Local(_))
            })
            .collect::<Vec<_>>();
        match (
            function.eval_variable_object_local,
            eval_variable_object_bindings.as_slice(),
        ) {
            (Some(index), [binding])
                if matches!(
                    function.kind,
                    FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
                ) && !function.strict
                    && binding.name == EVAL_VARIABLE_OBJECT_LOCAL_NAME
                    && binding.storage == BindingStorage::Local(index)
                    && binding.storage_scope == function.var_scope
                    && binding.declaration_scope == function.var_scope
                    && binding.declaration_span.is_none()
                    && function
                        .ops
                        .iter()
                        .any(|operation| matches!(operation.op, IrOp::EvalCall { .. })) => {}
            (None, []) => {}
            _ => {
                return Err(Error::internal(
                    "eval variable object local metadata is malformed",
                ));
            }
        }
        if function.strict
            && function.bindings.iter().any(|binding| {
                binding.kind == BindingKind::WithObject
                    && matches!(binding.storage, BindingStorage::Local(_))
            })
        {
            return Err(Error::internal(
                "strict function retained a local with object binding",
            ));
        }
        let mut seen_hoisted_bindings = vec![false; function.bindings.len()];
        for hoist in &function.hoisted_functions {
            let binding = function
                .bindings
                .get(hoist.binding.0)
                .ok_or_else(|| Error::internal("hoisted function binding is out of bounds"))?;
            if std::mem::replace(&mut seen_hoisted_bindings[hoist.binding.0], true)
                || binding.storage_scope != function.var_scope
                || binding.kind != BindingKind::Normal
                || !matches!(
                    binding.storage,
                    BindingStorage::Argument(_) | BindingStorage::Local(_)
                )
            {
                return Err(Error::internal(
                    "hoisted function has malformed frame binding metadata",
                ));
            }
            let constant = usize::try_from(hoist.constant)
                .map_err(|_| Error::internal("hoisted function constant is out of bounds"))?;
            let Some(IrConstant::Child(child)) = function.constants.get(constant) else {
                return Err(Error::internal(
                    "hoisted function does not reference child bytecode",
                ));
            };
            let child = tree
                .functions
                .get(*child)
                .ok_or_else(|| Error::internal("hoisted child function is out of bounds"))?;
            if child.function_name.as_deref() != Some(binding.name.as_str())
                || child.private_name_binding
            {
                return Err(Error::internal(
                    "hoisted child name metadata disagrees with its binding",
                ));
            }
        }
        if function.function_hoists_installed {
            let mut hoist_start = 0_usize;
            if let Some(local) = function.eval_variable_object_local {
                if !matches!(
                    function.ops.first(),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::VariableEnvironment),
                        pc_site: None,
                    })
                ) || !matches!(
                    function.ops.get(1),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::PutLocal(target)),
                        pc_site: None,
                    }) if *target == local
                ) {
                    return Err(Error::internal(
                        "installed eval-variable-object prologue is malformed",
                    ));
                }
                hoist_start = 2;
            }
            if let Some(local) = function.arguments_local {
                let expected_kind = if function.strict {
                    ArgumentsKind::Unmapped
                } else {
                    ArgumentsKind::Mapped
                };
                if !matches!(
                    function.ops.get(hoist_start),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::Arguments(kind)),
                        pc_site: None,
                    }) if *kind == expected_kind
                ) || !matches!(
                    function.ops.get(hoist_start + 1),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::PutLocal(target)),
                        pc_site: None,
                    }) if *target == local
                ) {
                    return Err(Error::internal(
                        "installed arguments-object prologue is malformed",
                    ));
                }
                hoist_start += 2;
            }
            for (ordinal, hoist) in ordered_hoisted_functions(function)?.into_iter().enumerate() {
                let closure_pc = ordinal
                    .checked_mul(2)
                    .and_then(|pc| pc.checked_add(hoist_start))
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
                if !matches!(
                    function.ops.get(closure_pc),
                    Some(SpannedIrOp {
                        op: IrOp::MakeClosure(constant),
                        pc_site: None,
                    }) if *constant == hoist.constant
                ) {
                    return Err(Error::internal(
                        "installed function hoist lost its child closure",
                    ));
                }
                let binding = &function.bindings[hoist.binding.0];
                let write_matches = match binding.storage {
                    BindingStorage::Argument(index) => matches!(
                        function.ops.get(closure_pc + 1),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::PutArg(target)),
                            pc_site: None,
                        }) if *target == index
                    ),
                    BindingStorage::Local(index) => matches!(
                        function.ops.get(closure_pc + 1),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::PutLocal(target)),
                            pc_site: None,
                        }) if *target == index
                    ),
                    BindingStorage::Global => false,
                    BindingStorage::External(_) => false,
                };
                if !write_matches {
                    return Err(Error::internal(
                        "installed function hoist targeted the wrong frame slot",
                    ));
                }
            }
        }
        let mut seen_scoped_function_bindings = vec![false; function.bindings.len()];
        for scoped in &function.scoped_functions {
            let binding = function
                .bindings
                .get(scoped.binding.0)
                .ok_or_else(|| Error::internal("scoped function binding is out of bounds"))?;
            let scope_kind = function.scopes[binding.storage_scope.0].kind;
            if std::mem::replace(&mut seen_scoped_function_bindings[scoped.binding.0], true)
                || binding.storage_scope != binding.declaration_scope
                || binding.kind != (BindingKind::Lexical { is_const: false })
                || !binding.is_scoped_function
                || !matches!(binding.storage, BindingStorage::Local(_))
                || !(matches!(
                    scope_kind,
                    ScopeKind::Block | ScopeKind::If | ScopeKind::Switch | ScopeKind::FunctionBody
                ) || (matches!(scope_kind, ScopeKind::ProgramBody)
                    && matches!(function.kind, FunctionKind::Eval(_))
                    && binding.storage_scope == function.body_scope))
            {
                return Err(Error::internal(
                    "scoped function has malformed lexical binding metadata",
                ));
            }
            let constant = usize::try_from(scoped.constant)
                .map_err(|_| Error::internal("scoped function constant is out of bounds"))?;
            let Some(IrConstant::Child(child_id)) = function.constants.get(constant) else {
                return Err(Error::internal(
                    "scoped function does not reference child bytecode",
                ));
            };
            let child = tree
                .functions
                .get(*child_id)
                .ok_or_else(|| Error::internal("scoped child function is out of bounds"))?;
            if child.function_name.as_deref() != Some(binding.name.as_str())
                || child.private_name_binding
                || child.parent
                    != Some(ParentLink {
                        function: function_id,
                        definition_scope: binding.storage_scope,
                    })
            {
                return Err(Error::internal(
                    "scoped child name or parent metadata disagrees with its binding",
                ));
            }
            if !matches!(
                function.ops.get(scoped.authored_closure),
                Some(SpannedIrOp {
                    op: IrOp::MakeClosure(found),
                    pc_site: None,
                }) if *found == scoped.constant
            ) {
                return Err(Error::internal(
                    "scoped function lost its authored closure allocation",
                ));
            }
            let drop_offset = if let Some(annex_binding) = scoped.annex_binding {
                if function.strict
                    || !matches!(
                        function.ops.get(scoped.authored_closure + 1),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::Dup),
                            pc_site: None,
                        })
                    )
                {
                    return Err(Error::internal(
                        "Annex B function lost its duplicate outer value",
                    ));
                }
                let write = function.ops.get(scoped.authored_closure + 2);
                let (unresolved, resolved) = match annex_binding {
                    IrAnnexBinding::Static(annex_binding) => {
                        let annex = function.bindings.get(annex_binding.0).ok_or_else(|| {
                            Error::internal("Annex B function binding is out of bounds")
                        })?;
                        if annex.storage_scope != function.var_scope
                            || (annex.kind != BindingKind::Normal
                                && !matches!(annex.storage, BindingStorage::External(_)))
                            || annex.name != binding.name
                        {
                            return Err(Error::internal(
                                "Annex B function has malformed root binding metadata",
                            ));
                        }
                        let unresolved = matches!(
                            write,
                            Some(SpannedIrOp {
                                op: IrOp::Identifier {
                                    name,
                                    scope,
                                    access: IdentifierAccess::AnnexBPut,
                                    ..
                                },
                                pc_site: None,
                            }) if name == &binding.name && *scope == function.var_scope
                        );
                        let resolved = match annex.storage {
                            BindingStorage::Local(index) => matches!(
                                write,
                                Some(SpannedIrOp {
                                    op: IrOp::Bytecode(Instruction::PutLocal(target)),
                                    pc_site: None,
                                }) if *target == index
                            ),
                            BindingStorage::Global => matches!(
                                write,
                                Some(SpannedIrOp {
                                    op: IrOp::Bytecode(Instruction::PutVar(index)),
                                    pc_site: None,
                                }) if function.closure_variables.get(usize::from(*index)).is_some_and(
                                    |descriptor| {
                                        descriptor.source == ClosureSource::Global
                                            && match descriptor.name {
                                                ClosureVariableName::Constant(name) => matches!(
                                                    function.constants.get(name as usize),
                                                    Some(IrConstant::Primitive(Value::String(found)))
                                                        if found.to_utf8_lossy() == annex.name
                                                ),
                                                _ => false,
                                            }
                                    }
                                )
                            ),
                            BindingStorage::External(index) => matches!(
                                write,
                                Some(SpannedIrOp {
                                    op: IrOp::Bytecode(
                                        Instruction::PutVarRef(target)
                                            | Instruction::PutVarRefCheck(target)
                                    ),
                                    pc_site: None,
                                }) if *target == index
                            ),
                            BindingStorage::Argument(_) => false,
                        };
                        (unresolved, resolved)
                    }
                    IrAnnexBinding::Dynamic => {
                        let unresolved = matches!(
                            write,
                            Some(SpannedIrOp {
                                op: IrOp::Identifier {
                                    name,
                                    scope,
                                    access: IdentifierAccess::Put,
                                    ..
                                },
                                pc_site: None,
                            }) if name == &binding.name && *scope == function.var_scope
                        );
                        let resolved = matches!(
                            write,
                            Some(SpannedIrOp {
                                op: IrOp::DynamicIdentifier {
                                    access: IdentifierAccess::Put,
                                    ..
                                },
                                pc_site: None,
                            })
                        );
                        (unresolved, resolved)
                    }
                };
                if !unresolved && !resolved {
                    return Err(Error::internal(
                        "Annex B function targeted the wrong outer binding",
                    ));
                }
                3
            } else {
                1
            };
            if !matches!(
                function.ops.get(scoped.authored_closure + drop_offset),
                Some(SpannedIrOp {
                    op: IrOp::Bytecode(Instruction::Drop),
                    pc_site: None,
                })
            ) {
                return Err(Error::internal(
                    "scoped function lost its authored closure drop",
                ));
            }
        }
        if function
            .bindings
            .iter()
            .enumerate()
            .any(|(index, binding)| {
                binding.is_scoped_function && !seen_scoped_function_bindings[index]
            })
        {
            return Err(Error::internal(
                "scoped function binding has no child declaration record",
            ));
        }
        for annex in &function.program_annex_functions {
            if !matches!(function.kind, FunctionKind::Script) || function.strict {
                return Err(Error::internal(
                    "Program Annex B function escaped sloppy script code",
                ));
            }
            let binding = function
                .bindings
                .get(annex.binding.0)
                .ok_or_else(|| Error::internal("Program Annex B binding is out of bounds"))?;
            if binding.storage_scope != function.var_scope
                || binding.storage != BindingStorage::Global
                || binding.kind != BindingKind::Normal
            {
                return Err(Error::internal(
                    "Program Annex B function has malformed global binding metadata",
                ));
            }
            let constant = usize::try_from(annex.constant).map_err(|_| {
                Error::internal("Program Annex B function constant is out of bounds")
            })?;
            let Some(IrConstant::Child(child_id)) = function.constants.get(constant) else {
                return Err(Error::internal(
                    "Program Annex B function does not reference child bytecode",
                ));
            };
            let child = tree.functions.get(*child_id).ok_or_else(|| {
                Error::internal("Program Annex B child function is out of bounds")
            })?;
            if child.function_name.as_deref() != Some(binding.name.as_str())
                || child.private_name_binding
                || child.parent
                    != Some(ParentLink {
                        function: function_id,
                        definition_scope: function.body_scope,
                    })
            {
                return Err(Error::internal(
                    "Program Annex B child metadata disagrees with its binding",
                ));
            }
            if !matches!(
                function.ops.get(annex.authored_closure),
                Some(SpannedIrOp {
                    op: IrOp::MakeClosure(found),
                    pc_site: None,
                }) if *found == annex.constant
            ) || !matches!(
                function.ops.get(annex.authored_closure + 1),
                Some(SpannedIrOp {
                    op: IrOp::Bytecode(Instruction::Dup),
                    pc_site: None,
                })
            ) {
                return Err(Error::internal(
                    "Program Annex B function lost its authored closure allocation",
                ));
            }
            let outer_write = function.ops.get(annex.authored_closure + 2);
            let unresolved_outer = matches!(
                outer_write,
                Some(SpannedIrOp {
                    op: IrOp::Identifier {
                        name,
                        scope,
                        access: IdentifierAccess::AnnexBPut,
                        ..
                    },
                    pc_site: None,
                }) if name == &binding.name && *scope == function.var_scope
            );
            let resolved_outer = matches!(
                outer_write,
                Some(SpannedIrOp {
                    op: IrOp::Bytecode(Instruction::PutVar(index)),
                    pc_site: None,
                }) if function.closure_variables.get(usize::from(*index)).is_some_and(
                    |descriptor| {
                        descriptor.source == ClosureSource::Global
                            && match descriptor.name {
                                ClosureVariableName::Constant(name) => matches!(
                                    function.constants.get(name as usize),
                                    Some(IrConstant::Primitive(Value::String(found)))
                                        if found.to_utf8_lossy() == binding.name
                                ),
                                _ => false,
                            }
                    }
                )
            );
            if !unresolved_outer && !resolved_outer {
                return Err(Error::internal(
                    "Program Annex B function targeted the wrong root binding",
                ));
            }

            let current_write = function.ops.get(annex.authored_closure + 3);
            let unresolved_current = matches!(
                current_write,
                Some(SpannedIrOp {
                    op: IrOp::Identifier {
                        name,
                        scope,
                        access: IdentifierAccess::Put,
                        ..
                    },
                    pc_site: None,
                }) if name == &binding.name && *scope == function.body_scope
            );
            let resolved_current = matches!(
                current_write,
                Some(SpannedIrOp {
                    op: IrOp::Bytecode(Instruction::PutVar(index)),
                    pc_site: None,
                }) if function.closure_variables.get(usize::from(*index)).is_some_and(
                    |descriptor| {
                        descriptor.source == ClosureSource::GlobalDeclaration
                            && match descriptor.name {
                                ClosureVariableName::Constant(name) => matches!(
                                    function.constants.get(name as usize),
                                    Some(IrConstant::Primitive(Value::String(found)))
                                        if found.to_utf8_lossy() == binding.name
                                ),
                                _ => false,
                            }
                    }
                )
            );
            if !unresolved_current && !resolved_current {
                return Err(Error::internal(
                    "Program Annex B function lost its current-environment write",
                ));
            }
        }
        match function.kind {
            FunctionKind::Script | FunctionKind::Eval(_) => {
                for declaration in &function.global_declarations {
                    let binding_scope = if declaration.is_lexical {
                        function.body_scope
                    } else {
                        function.var_scope
                    };
                    let binding = function.scopes[binding_scope.0]
                        .bindings
                        .iter()
                        .rev()
                        .map(|binding| &function.bindings[binding.0])
                        .find(|binding| {
                            binding.name == declaration.name
                                && binding.storage == BindingStorage::Global
                        });
                    let expected_kind = if declaration.is_lexical {
                        BindingKind::Lexical {
                            is_const: declaration.is_const,
                        }
                    } else {
                        if declaration.is_const {
                            return Err(Error::internal(
                                "ordinary global declaration is marked const",
                            ));
                        }
                        BindingKind::Normal
                    };
                    let masked_program_lexical = declaration.is_lexical
                        && function.first_global_declaration_is_normal(&declaration.name);
                    if binding.is_none_or(|binding| {
                        binding.storage != BindingStorage::Global
                            || if masked_program_lexical {
                                !matches!(binding.kind, BindingKind::Lexical { .. })
                            } else {
                                binding.kind != expected_kind
                            }
                    }) {
                        return Err(Error::internal(
                            "global declaration has no matching binding identity",
                        ));
                    }
                    if let Some(constant) = declaration.function_constant {
                        if declaration.is_lexical || declaration.is_const {
                            return Err(Error::internal(
                                "global function declaration has lexical metadata",
                            ));
                        }
                        let constant = usize::try_from(constant).map_err(|_| {
                            Error::internal("global function constant is out of bounds")
                        })?;
                        if !matches!(function.constants.get(constant), Some(IrConstant::Child(_))) {
                            return Err(Error::internal(
                                "global function declaration does not reference child bytecode",
                            ));
                        }
                    }
                    if let Some(index) = declaration.closure_index {
                        let descriptor = function
                            .closure_variables
                            .get(usize::from(index))
                            .ok_or_else(|| {
                                Error::internal("global declaration closure is out of bounds")
                            })?;
                        let expected_kind = if declaration.function_constant.is_some() {
                            ClosureVariableKind::GlobalFunction
                        } else {
                            ClosureVariableKind::Normal
                        };
                        if descriptor.source != ClosureSource::GlobalDeclaration
                            || descriptor.is_lexical != declaration.is_lexical
                            || descriptor.is_const != declaration.is_const
                            || descriptor.kind != expected_kind
                        {
                            return Err(Error::internal(
                                "global declaration closure metadata disagrees",
                            ));
                        }
                    }
                }
            }
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
                if function.global_declarations.is_empty() => {}
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow => {
                return Err(Error::internal(
                    "ordinary function contains Program global declarations",
                ));
            }
        }
        if let Some(parent_link) = function.parent {
            let parent = tree
                .functions
                .get(parent_link.function)
                .ok_or_else(|| Error::internal("function parent is out of bounds"))?;
            if parent_link.definition_scope.0 >= parent.scopes.len() {
                return Err(Error::internal("child definition scope is out of bounds"));
            }
        }

        for (scope_index, scope) in function.scopes.iter().enumerate() {
            if scope_index > 0 && scope.parent.is_none_or(|parent| parent.0 >= scope_index) {
                return Err(Error::internal("lexical scope parent is malformed"));
            }
        }

        let mut seen_bindings = vec![false; function.bindings.len()];
        let mut seen_arguments = vec![false; function.parameters.len()];
        let mut seen_locals = vec![false; function.locals.len()];
        let mut seen_external = vec![false; function.external_bindings.len()];
        for (index, external) in function.external_bindings.iter().enumerate() {
            let sentinel = match external.kind {
                ClosureVariableKind::EvalVariableObject => EVAL_VARIABLE_OBJECT_LOCAL_NAME,
                ClosureVariableKind::WithObject => WITH_OBJECT_LOCAL_NAME,
                ClosureVariableKind::Normal
                | ClosureVariableKind::FunctionName
                | ClosureVariableKind::GlobalFunction => continue,
            };
            let descriptor = function
                .closure_variables
                .get(index)
                .ok_or_else(|| Error::internal("eval hidden object closure is out of bounds"))?;
            if external.is_lexical
                || external.is_const
                || external.is_catch_parameter
                || external.name.to_utf8_lossy() != sentinel
                || descriptor.source
                    != ClosureSource::EvalEnvironment(u16::try_from(index).map_err(|_| {
                        Error::new(ErrorKind::JsInternal, "too many closure variables")
                    })?)
                || descriptor.kind != external.kind
                || descriptor.is_lexical
                || descriptor.is_const
            {
                return Err(Error::internal(
                    "eval hidden object external metadata is malformed",
                ));
            }
        }
        for (scope_index, scope) in function.scopes.iter().enumerate() {
            if scope.kind == ScopeKind::With
                && (scope.bindings.len() != 1
                    || scope.bindings.first().is_none_or(|binding| {
                        function.bindings.get(binding.0).is_none_or(|binding| {
                            binding.kind != BindingKind::WithObject
                                || !matches!(binding.storage, BindingStorage::Local(_))
                        })
                    }))
            {
                return Err(Error::internal(
                    "with scope does not own exactly one hidden object binding",
                ));
            }
            for &binding_id in &scope.bindings {
                let binding = function
                    .bindings
                    .get(binding_id.0)
                    .ok_or_else(|| Error::internal("scope binding is out of bounds"))?;
                if std::mem::replace(&mut seen_bindings[binding_id.0], true) {
                    return Err(Error::internal(
                        "binding appears more than once in the scope graph",
                    ));
                }
                if binding.storage_scope != ScopeId(scope_index)
                    || binding.declaration_scope.0 >= function.scopes.len()
                {
                    return Err(Error::internal("binding scope metadata is malformed"));
                }
                let mut declaration_ancestor = Some(binding.declaration_scope);
                while let Some(scope) = declaration_ancestor {
                    if scope == binding.storage_scope {
                        break;
                    }
                    declaration_ancestor = function.scopes[scope.0].parent;
                }
                if declaration_ancestor.is_none() {
                    return Err(Error::internal(
                        "binding storage scope does not contain its declaration",
                    ));
                }
                if binding
                    .declaration_span
                    .is_some_and(|span| span.start.byte_offset > span.end.byte_offset)
                {
                    return Err(Error::internal("binding declaration span is malformed"));
                }
                match binding.storage {
                    BindingStorage::Argument(index) => {
                        let index = usize::from(index);
                        let parameter = function
                            .parameters
                            .get(index)
                            .ok_or_else(|| Error::internal("argument binding is out of bounds"))?;
                        if binding.is_catch_parameter
                            || binding.storage_scope != function.var_scope
                            || binding.declaration_scope != function.var_scope
                            || binding.kind != BindingKind::Normal
                            || binding.name != *parameter
                        {
                            return Err(Error::internal("argument binding metadata is malformed"));
                        }
                        if std::mem::replace(&mut seen_arguments[index], true) {
                            return Err(Error::internal(
                                "argument slot has more than one binding identity",
                            ));
                        }
                    }
                    BindingStorage::Local(index) => {
                        let index = usize::from(index);
                        let local = function
                            .locals
                            .get(index)
                            .ok_or_else(|| Error::internal("local binding is out of bounds"))?;
                        if binding.name != *local {
                            return Err(Error::internal("local binding metadata is malformed"));
                        }
                        let catch_parameter_metadata = binding.is_catch_parameter
                            && binding.storage_scope == binding.declaration_scope
                            && function.scopes[binding.storage_scope.0].kind == ScopeKind::Catch
                            && binding.kind == (BindingKind::Lexical { is_const: false });
                        if binding.is_catch_parameter != catch_parameter_metadata {
                            return Err(Error::internal(
                                "catch parameter binding metadata is malformed",
                            ));
                        }
                        if binding.kind == BindingKind::WithObject
                            && (binding.name != WITH_OBJECT_LOCAL_NAME
                                || binding.storage_scope != binding.declaration_scope
                                || function.scopes[binding.storage_scope.0].kind != ScopeKind::With
                                || binding.is_catch_parameter)
                        {
                            return Err(Error::internal(
                                "with object local binding metadata is malformed",
                            ));
                        }
                        if matches!(binding.kind, BindingKind::Lexical { .. }) {
                            let scope_kind = function.scopes[binding.storage_scope.0].kind;
                            let supported_scope = matches!(
                                scope_kind,
                                ScopeKind::Block
                                    | ScopeKind::If
                                    | ScopeKind::For
                                    | ScopeKind::Switch
                                    | ScopeKind::Catch
                            ) || (matches!(
                                scope_kind,
                                ScopeKind::FunctionBody
                            ) && matches!(
                                function.kind,
                                FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
                            ) && binding.storage_scope
                                == function.body_scope)
                                || (scope_kind == ScopeKind::ProgramBody
                                    && matches!(function.kind, FunctionKind::Eval(_))
                                    && binding.storage_scope == function.body_scope);
                            if binding.storage_scope != binding.declaration_scope
                                || !supported_scope
                            {
                                return Err(Error::internal(
                                    "lexical binding scope metadata is malformed",
                                ));
                            }
                        }
                        if std::mem::replace(&mut seen_locals[index], true) {
                            return Err(Error::internal(
                                "local slot has more than one binding identity",
                            ));
                        }
                    }
                    BindingStorage::Global => {
                        let valid_lexical = matches!(binding.kind, BindingKind::Lexical { .. })
                            && binding.storage_scope == function.body_scope
                            && binding.declaration_scope == function.body_scope
                            && function.scopes[binding.storage_scope.0].kind
                                == ScopeKind::ProgramBody;
                        let valid_var = binding.kind == BindingKind::Normal
                            && binding.storage_scope == function.var_scope;
                        if binding.is_catch_parameter
                            || (!matches!(function.kind, FunctionKind::Script)
                                && !(matches!(function.kind, FunctionKind::Eval(_)) && valid_var))
                            || (!valid_lexical && !valid_var)
                        {
                            return Err(Error::internal("global binding metadata is malformed"));
                        }
                    }
                    BindingStorage::External(index) => {
                        let external_index = usize::from(index);
                        let external =
                            function
                                .external_bindings
                                .get(external_index)
                                .ok_or_else(|| {
                                    Error::internal("eval external binding is out of bounds")
                                })?;
                        if std::mem::replace(&mut seen_external[external_index], true) {
                            return Err(Error::internal(
                                "eval external slot has more than one binding identity",
                            ));
                        }
                        let expected_kind = match external.kind {
                            ClosureVariableKind::Normal if external.is_lexical => {
                                BindingKind::Lexical {
                                    is_const: external.is_const,
                                }
                            }
                            ClosureVariableKind::Normal if !external.is_const => {
                                BindingKind::Normal
                            }
                            ClosureVariableKind::FunctionName if !external.is_lexical => {
                                BindingKind::FunctionName {
                                    is_const: external.is_const,
                                }
                            }
                            ClosureVariableKind::EvalVariableObject
                                if !external.is_lexical && !external.is_const =>
                            {
                                BindingKind::EvalVariableObject
                            }
                            ClosureVariableKind::WithObject
                                if !external.is_lexical && !external.is_const =>
                            {
                                BindingKind::WithObject
                            }
                            ClosureVariableKind::Normal
                            | ClosureVariableKind::FunctionName
                            | ClosureVariableKind::GlobalFunction
                            | ClosureVariableKind::EvalVariableObject
                            | ClosureVariableKind::WithObject => {
                                return Err(Error::internal(
                                    "eval external binding flags are inconsistent",
                                ));
                            }
                        };
                        if !matches!(function.kind, FunctionKind::Eval(EvalKind::Direct))
                            || function.parent.is_some()
                            || binding.storage_scope != function.var_scope
                            || binding.declaration_scope != function.var_scope
                            || binding.declaration_span.is_some()
                            || binding.is_catch_parameter != external.is_catch_parameter
                            || binding.name != external.name.to_utf8_lossy()
                            || binding.kind != expected_kind
                        {
                            return Err(Error::internal(
                                "eval external binding metadata is malformed",
                            ));
                        }
                        let descriptor = function
                            .closure_variables
                            .get(external_index)
                            .ok_or_else(|| {
                                Error::internal("eval external closure is out of bounds")
                            })?;
                        if descriptor.source != ClosureSource::EvalEnvironment(index)
                            || descriptor.is_lexical != external.is_lexical
                            || descriptor.is_const != external.is_const
                            || descriptor.kind != external.kind
                            || !matches!(
                                descriptor.name,
                                ClosureVariableName::Constant(name)
                                    if matches!(
                                        function.constants.get(name as usize),
                                        Some(IrConstant::Primitive(Value::String(found)))
                                            if found == &external.name
                                    )
                            )
                        {
                            return Err(Error::internal(
                                "eval external closure metadata disagrees",
                            ));
                        }
                    }
                }
            }
        }
        if seen_bindings.iter().any(|seen| !seen) {
            return Err(Error::internal("binding is missing from the scope graph"));
        }
        if seen_arguments.iter().any(|seen| !seen) {
            return Err(Error::internal(
                "argument slot is missing its binding identity",
            ));
        }
        if seen_external.iter().any(|seen| !seen) {
            return Err(Error::internal(
                "eval external slot is missing its binding identity",
            ));
        }
        let eval_ret_index = function.eval_ret_local.map(usize::from);
        let mut seen_synthetic = vec![false; function.locals.len()];
        let mut synthetic_eval_ret = None;
        for synthetic in &function.synthetic_locals {
            let index = usize::from(synthetic.index);
            let name = function
                .locals
                .get(index)
                .ok_or_else(|| Error::internal("synthetic local is out of bounds"))?;
            if std::mem::replace(
                seen_synthetic
                    .get_mut(index)
                    .ok_or_else(|| Error::internal("synthetic local is out of bounds"))?,
                true,
            ) || name != synthetic.kind.name()
            {
                return Err(Error::internal("synthetic local metadata is malformed"));
            }
            match synthetic.kind {
                SyntheticLocalKind::EvalCompletion => {
                    if synthetic_eval_ret.replace(index).is_some() {
                        return Err(Error::internal(
                            "eval completion slot metadata is malformed",
                        ));
                    }
                }
                SyntheticLocalKind::FinallySavedEvalCompletion
                    if matches!(function.kind, FunctionKind::Script | FunctionKind::Eval(_)) => {}
                SyntheticLocalKind::FinallySavedEvalCompletion => {
                    return Err(Error::internal(
                        "ordinary function contains a finally eval-completion save slot",
                    ));
                }
            }
        }
        match function.kind {
            FunctionKind::Script | FunctionKind::Eval(_)
                if eval_ret_index == Some(0)
                    && synthetic_eval_ret == eval_ret_index
                    && function
                        .locals
                        .first()
                        .is_some_and(|name| name == EVAL_RET_LOCAL_NAME) => {}
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
                if eval_ret_index.is_none() && synthetic_eval_ret.is_none() => {}
            _ => {
                return Err(Error::internal(
                    "eval completion slot metadata is malformed",
                ));
            }
        }
        if function.bindings.iter().any(|binding| {
            matches!(
                binding.name.as_str(),
                EVAL_RET_LOCAL_NAME | FINALLY_EVAL_RET_LOCAL_NAME
            )
        }) || function.locals.iter().enumerate().any(|(index, name)| {
            matches!(
                name.as_str(),
                EVAL_RET_LOCAL_NAME | FINALLY_EVAL_RET_LOCAL_NAME
            ) && !seen_synthetic[index]
        }) {
            return Err(Error::internal(
                "synthetic local leaked into source binding lookup",
            ));
        }
        for (index, seen) in seen_locals.into_iter().enumerate() {
            if seen_synthetic[index] {
                if seen {
                    return Err(Error::internal(
                        "synthetic local has a source binding identity",
                    ));
                }
            } else if !seen {
                return Err(Error::internal(
                    "local slot is missing its binding identity",
                ));
            }
        }

        let mut scope_entries = vec![0_usize; function.scopes.len()];
        let mut scope_leaves = vec![0_usize; function.scopes.len()];
        for operation in &function.ops {
            let (scope, counts) = match operation.op {
                IrOp::EnterScope(scope) => (scope, &mut scope_entries),
                IrOp::LeaveScope(scope) => (scope, &mut scope_leaves),
                _ => continue,
            };
            let count = counts
                .get_mut(scope.0)
                .ok_or_else(|| Error::internal("scope lifecycle target is out of bounds"))?;
            *count = count
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "out of memory"))?;
        }
        for (scope_index, scope) in function.scopes.iter().enumerate() {
            let entries = scope_entries[scope_index];
            let leaves = scope_leaves[scope_index];
            if scope_index == function.var_scope.0 {
                if entries != 0 || leaves != 0 {
                    return Err(Error::internal(
                        "function root unexpectedly has scope lifecycle operations",
                    ));
                }
            } else if scope_index == function.body_scope.0 {
                match function.kind {
                    FunctionKind::Script if entries == 0 && leaves == 0 => {}
                    FunctionKind::Ordinary
                    | FunctionKind::Method
                    | FunctionKind::Arrow
                    | FunctionKind::Eval(_)
                        if entries == 1
                            && leaves == 0
                            && function.ops.iter().any(|operation| {
                                matches!(
                                    operation.op,
                                    IrOp::EnterScope(body) if body == function.body_scope
                                )
                            }) => {}
                    _ => {
                        return Err(Error::internal(
                            "function body scope lifecycle metadata is malformed",
                        ));
                    }
                }
            } else if entries != 1 || leaves == 0 || scope.parent.is_none() {
                return Err(Error::internal(
                    "nested scope lifecycle metadata is malformed",
                ));
            }
        }

        let function_name_bindings = function
            .bindings
            .iter()
            .filter(|binding| matches!(binding.kind, BindingKind::FunctionName { .. }))
            .collect::<Vec<_>>();
        match function.kind {
            FunctionKind::Ordinary => match (
                function.function_name_local,
                function_name_bindings.as_slice(),
            ) {
                (Some(index), [binding])
                    if function.private_name_binding
                        && binding.storage == BindingStorage::Local(index)
                        && binding.storage_scope == function.var_scope
                        && binding.declaration_scope == function.var_scope
                        && function.function_name.as_deref() == Some(binding.name.as_str())
                        && binding.kind
                            == (BindingKind::FunctionName {
                                is_const: function.strict,
                            }) => {}
                // Private function names are materialized lazily when the
                // body references them or an eval site makes them visible.
                (None, []) => {}
                _ => return Err(Error::internal("function-name binding metadata disagrees")),
            },
            FunctionKind::Method | FunctionKind::Arrow
                if function.function_name_local.is_none()
                    && !function.private_name_binding
                    && function_name_bindings.is_empty() => {}
            FunctionKind::Method | FunctionKind::Arrow => {
                return Err(Error::internal(
                    "unnamed function retained private function-name metadata",
                ));
            }
            FunctionKind::Eval(EvalKind::Direct)
                if function.function_name_local.is_none()
                    && !function.private_name_binding
                    && function_name_bindings
                        .iter()
                        .all(|binding| matches!(binding.storage, BindingStorage::External(_))) => {}
            FunctionKind::Script | FunctionKind::Eval(EvalKind::Indirect)
                if function.function_name_local.is_none()
                    && !function.private_name_binding
                    && function_name_bindings.is_empty() => {}
            FunctionKind::Eval(EvalKind::None) => {
                return Err(Error::internal("eval root has no eval kind"));
            }
            FunctionKind::Script | FunctionKind::Eval(_) => {
                return Err(Error::internal("function-name binding metadata disagrees"));
            }
        }
        let root_kind = matches!(function.kind, FunctionKind::Script | FunctionKind::Eval(_));
        if (function_id == 0) != function.parent.is_none() || (function_id == 0) != root_kind {
            return Err(Error::internal("function topology is malformed"));
        }
    }
    Ok(())
}

fn resolve_identifiers(tree: &mut FunctionTree) -> Result<(), Error> {
    install_eval_variable_objects(tree)?;
    validate_scope_graph(tree)?;
    seed_global_declarations(tree)?;
    // QuickJS enters each function by pre-populating its direct-eval closure
    // table, then creates children depth-first in source order, and only then
    // resolves the parent's ordinary identifiers. The entry event matters:
    // `get_closure_var` is first-slot-wins, so a descendant eval can establish
    // an ancestor relay before that ancestor's own bytecode is resolved.
    for event in function_resolution_events(tree)? {
        match event {
            FunctionResolutionEvent::Enter(function_id) => {
                link_eval_environments(tree, function_id)?;
            }
            FunctionResolutionEvent::Resolve(function_id) => {
                let unresolved = tree.functions[function_id]
                    .ops
                    .iter()
                    .enumerate()
                    .filter_map(|(index, operation)| match &operation.op {
                        IrOp::Identifier {
                            name,
                            span,
                            scope,
                            access,
                        } => Some((index, name.clone(), *span, *scope, Ok(*access))),
                        IrOp::IdentifierReference {
                            name,
                            span,
                            scope,
                            access,
                        } => Some((index, name.clone(), *span, *scope, Err(*access))),
                        _ => None,
                    })
                    .collect::<Vec<_>>();

                for (operation_index, name, span, scope, access) in unresolved {
                    let operation = match access {
                        Ok(access) => {
                            resolve_identifier(tree, function_id, scope, &name, span, access)?
                        }
                        Err(access) => resolve_identifier_reference(
                            tree,
                            function_id,
                            scope,
                            &name,
                            span,
                            access,
                        )?,
                    };
                    tree.functions[function_id].ops[operation_index].op = operation;
                }
            }
        }
    }
    install_pseudo_binding_prologues(tree)?;
    install_global_function_hoists(tree)?;
    install_eval_declaration_hoists(tree)?;
    install_function_body_hoists(tree)?;
    validate_scope_graph(tree)
}

/// QuickJS `add_eval_variables` allocates one hidden `<var>` local before
/// resolving any identifier in sloppy authored function code which contains a
/// syntactic direct-eval site. Keeping this as a separate prepass is essential:
/// children authored before the eval call must resolve through the same object.
fn install_eval_variable_objects(tree: &mut FunctionTree) -> Result<(), Error> {
    for function in &mut tree.functions {
        let needs_object = matches!(
            function.kind,
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
        ) && !function.strict
            && function
                .ops
                .iter()
                .any(|operation| matches!(operation.op, IrOp::EvalCall { .. }));
        if !needs_object {
            if function.eval_variable_object_local.is_some() {
                return Err(Error::internal(
                    "function retained an unnecessary eval variable object",
                ));
            }
            continue;
        }
        if function.eval_variable_object_local.is_some() {
            return Err(Error::internal(
                "eval variable object was installed more than once",
            ));
        }
        if function.locals.len() >= MAX_LOCAL_VARIABLES {
            return Err(Error::new(
                ErrorKind::JsInternal,
                "too many local variables",
            ));
        }
        let index = u16::try_from(function.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        function
            .locals
            .push(EVAL_VARIABLE_OBJECT_LOCAL_NAME.to_owned());
        function.add_binding(
            function.var_scope,
            function.var_scope,
            EVAL_VARIABLE_OBJECT_LOCAL_NAME.to_owned(),
            BindingStorage::Local(index),
            BindingKind::EvalVariableObject,
            None,
        );
        function.eval_variable_object_local = Some(index);
    }
    Ok(())
}

const fn eval_scope_kind(kind: ScopeKind) -> EvalScopeKind {
    match kind {
        ScopeKind::FunctionRoot => EvalScopeKind::FunctionRoot,
        ScopeKind::FunctionBody => EvalScopeKind::FunctionBody,
        ScopeKind::ProgramBody => EvalScopeKind::ProgramBody,
        ScopeKind::Block => EvalScopeKind::Block,
        ScopeKind::If => EvalScopeKind::If,
        ScopeKind::For => EvalScopeKind::For,
        ScopeKind::Switch => EvalScopeKind::Switch,
        ScopeKind::Catch => EvalScopeKind::Catch,
        ScopeKind::With => EvalScopeKind::With,
    }
}

/// Publish the immutable scope chains for one function at its QuickJS
/// creation-entry event. Besides retaining names for later String compilation,
/// this deliberately pre-populates closure slots before children and ordinary
/// identifier resolution so source-order first-slot-wins behavior is stable.
fn link_eval_environments(tree: &mut FunctionTree, function_id: FunctionId) -> Result<(), Error> {
    let has_eval = tree.functions[function_id]
        .ops
        .iter()
        .any(|operation| matches!(operation.op, IrOp::EvalCall { .. }));
    if !has_eval {
        return Ok(());
    }

    // Lazy pseudo-bindings must exist before any descriptor snapshots a scope.
    // The nearest ordinary-function or method frame owns `arguments`;
    // named-expression self bindings from every enclosing function are also
    // visible even when no ordinary identifier opcode forced them first.
    ensure_eval_visible_pseudo_bindings(tree, function_id)?;

    let sites = tree.functions[function_id]
        .ops
        .iter()
        .enumerate()
        .filter_map(|(operation_index, operation)| match operation.op {
            IrOp::EvalCall {
                scope,
                environment: None,
                ..
            } => Some((operation_index, scope)),
            IrOp::EvalCall {
                environment: Some(_),
                ..
            } => None,
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut by_scope = HashMap::<ScopeId, u16>::new();
    let mut linked_sites = Vec::with_capacity(sites.len());
    for (operation_index, scope) in sites {
        let environment = if let Some(&environment) = by_scope.get(&scope) {
            environment
        } else {
            let environment = u16::try_from(tree.functions[function_id].eval_environments.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many eval environments"))?;
            let descriptor = link_eval_environment(tree, function_id, scope)?;
            tree.functions[function_id]
                .eval_environments
                .push(descriptor);
            by_scope.insert(scope, environment);
            environment
        };
        linked_sites.push((operation_index, environment));
    }
    for (operation_index, environment) in linked_sites {
        let Some(operation) = tree.functions[function_id].ops.get_mut(operation_index) else {
            return Err(Error::internal("eval operation moved while linking scopes"));
        };
        let IrOp::EvalCall {
            environment: linked,
            ..
        } = &mut operation.op
        else {
            return Err(Error::internal(
                "eval operation changed while linking scopes",
            ));
        };
        if linked.replace(environment).is_some() {
            return Err(Error::internal("eval operation was linked more than once"));
        }
    }
    Ok(())
}

fn link_eval_environment(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
    call_scope: ScopeId,
) -> Result<EvalEnvironment<JsString>, Error> {
    let caller_strict = tree.functions[consuming_function].strict;
    let caller_kind = tree.functions[consuming_function].kind;
    let super_call_allowed = tree.functions[consuming_function].super_call_allowed;
    let super_allowed = tree.functions[consuming_function].super_allowed;
    let mut scope_path = Vec::<(FunctionId, ScopeId)>::new();
    let mut owner = consuming_function;
    let mut scope = call_scope;
    loop {
        loop {
            let ir_scope = tree.functions[owner]
                .scopes
                .get(scope.0)
                .ok_or_else(|| Error::internal("eval call scope is out of bounds"))?;
            scope_path.push((owner, scope));
            let Some(parent) = ir_scope.parent else {
                break;
            };
            scope = parent;
        }
        let Some(parent) = tree.functions[owner].parent else {
            break;
        };
        owner = parent.function;
        scope = parent.definition_scope;
    }

    let mut scopes = Vec::with_capacity(scope_path.len());
    let mut current_function_root = None;
    for (owner, scope) in scope_path {
        let scope_ordinal = u16::try_from(scopes.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many eval scopes"))?;
        let (kind, binding_snapshots) = {
            let function = &tree.functions[owner];
            let ir_scope = function
                .scopes
                .get(scope.0)
                .ok_or_else(|| Error::internal("eval scope path is out of bounds"))?;
            let mut ordered = ir_scope.bindings.iter().rev().copied().collect::<Vec<_>>();
            if ir_scope.kind == ScopeKind::FunctionRoot
                && function.eval_variable_object_local.is_some()
            {
                // Existing authored root bindings precede `<var>`. The lazy
                // `arguments` object and private function name are the two
                // QuickJS pseudo-bindings exposed after `<var>` to eval source.
                ordered.sort_by_key(|binding| {
                    let binding = &function.bindings[binding.0];
                    match binding.kind {
                        BindingKind::EvalVariableObject => 1_u8,
                        BindingKind::FunctionName { .. } => 2,
                        BindingKind::Normal
                            if matches!(
                                binding.storage,
                                BindingStorage::Local(index)
                                    if function.arguments_local == Some(index)
                            ) =>
                        {
                            2
                        }
                        BindingKind::Normal
                        | BindingKind::Lexical { .. }
                        | BindingKind::WithObject => 0,
                    }
                });
            }
            let bindings = ordered
                .into_iter()
                .map(|binding| {
                    function
                        .bindings
                        .get(binding.0)
                        .map(|binding| {
                            (
                                binding.name.clone(),
                                binding.storage,
                                binding.kind,
                                binding.is_catch_parameter,
                            )
                        })
                        .ok_or_else(|| Error::internal("eval binding is out of bounds"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            (ir_scope.kind, bindings)
        };
        if owner == consuming_function && kind == ScopeKind::FunctionRoot {
            current_function_root = Some(scope_ordinal);
        }

        let mut bindings = Vec::with_capacity(binding_snapshots.len());
        for (name, storage, binding_kind, is_catch_parameter) in binding_snapshots {
            // The synthetic eval root installs imported bindings in its own
            // parser root only for identifier resolution.  Their semantic
            // scope provenance is reconstructed from `eval_caller_profile`
            // below, so serializing them here would falsely turn caller catch
            // or block bindings into FunctionRoot bindings.
            if matches!(storage, BindingStorage::External(_)) {
                continue;
            }
            let (source, resolved_kind) = if owner == consuming_function {
                match storage {
                    BindingStorage::Argument(index) => {
                        (EvalBindingSource::Argument(index), binding_kind)
                    }
                    BindingStorage::Local(index) => (EvalBindingSource::Local(index), binding_kind),
                    BindingStorage::External(_) => unreachable!("filtered above"),
                    BindingStorage::Global => continue,
                }
            } else {
                if storage == BindingStorage::Global {
                    continue;
                }
                let (index, resolved_kind) = capture_binding_path(
                    tree,
                    owner,
                    consuming_function,
                    ResolvedBinding {
                        storage,
                        kind: binding_kind,
                    },
                    &name,
                    true,
                    true,
                )?;
                if resolved_kind != binding_kind
                    && !matches!(
                        (binding_kind, resolved_kind),
                        (BindingKind::FunctionName { .. }, BindingKind::Normal)
                    )
                {
                    return Err(Error::internal(
                        "eval closure relay changed binding metadata",
                    ));
                }
                (EvalBindingSource::Closure(index), resolved_kind)
            };
            bindings.push(EvalBinding {
                name: JsString::try_from_utf8(&name)?,
                source,
                is_lexical: matches!(resolved_kind, BindingKind::Lexical { .. }),
                is_const: matches!(
                    resolved_kind,
                    BindingKind::Lexical { is_const: true }
                        | BindingKind::FunctionName { is_const: true }
                ),
                kind: closure_kind(resolved_kind),
                is_catch_parameter,
            });
        }
        scopes.push(EvalScope {
            kind: eval_scope_kind(kind),
            bindings: bindings.into_boxed_slice(),
        });
    }

    // A direct eval root is a real frame, but its imported caller bindings do
    // not become declarations in that synthetic FunctionRoot.  Append the
    // exact original scope suffix (including empty scopes) and relay each
    // flattened external slot through any intervening eval-created function.
    let imported_profile = tree.functions[0].eval_caller_profile.clone();
    let imported_bindings = tree.functions[0].external_bindings.clone();
    for (scope_index, kind) in imported_profile.scope_kinds.iter().copied().enumerate() {
        let scope_index = u16::try_from(scope_index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many eval scopes"))?;
        let snapshots = imported_bindings
            .iter()
            .enumerate()
            .filter(|(_, binding)| binding.scope == scope_index)
            .map(|(index, binding)| (index, binding.clone()))
            .collect::<Vec<_>>();
        let mut bindings = Vec::with_capacity(snapshots.len());
        for (external_index, binding) in snapshots {
            let external_index = u16::try_from(external_index)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
            let binding_kind = match binding.kind {
                ClosureVariableKind::Normal if binding.is_lexical => BindingKind::Lexical {
                    is_const: binding.is_const,
                },
                ClosureVariableKind::Normal if !binding.is_const => BindingKind::Normal,
                ClosureVariableKind::FunctionName if !binding.is_lexical => {
                    BindingKind::FunctionName {
                        is_const: binding.is_const,
                    }
                }
                ClosureVariableKind::EvalVariableObject
                    if !binding.is_lexical && !binding.is_const =>
                {
                    BindingKind::EvalVariableObject
                }
                ClosureVariableKind::WithObject if !binding.is_lexical && !binding.is_const => {
                    BindingKind::WithObject
                }
                ClosureVariableKind::Normal
                | ClosureVariableKind::FunctionName
                | ClosureVariableKind::GlobalFunction
                | ClosureVariableKind::EvalVariableObject
                | ClosureVariableKind::WithObject => {
                    return Err(Error::internal(
                        "imported eval binding flags are inconsistent",
                    ));
                }
            };
            let source = if consuming_function == 0 {
                EvalBindingSource::Closure(external_index)
            } else {
                let name = binding.name.to_utf8_lossy();
                let (closure, relayed_kind) = capture_binding_path(
                    tree,
                    0,
                    consuming_function,
                    ResolvedBinding {
                        storage: BindingStorage::External(external_index),
                        kind: binding_kind,
                    },
                    &name,
                    true,
                    false,
                )?;
                if relayed_kind != binding_kind {
                    return Err(Error::internal(
                        "imported eval binding relay changed metadata",
                    ));
                }
                EvalBindingSource::Closure(closure)
            };
            bindings.push(EvalBinding {
                name: binding.name,
                source,
                is_lexical: binding.is_lexical,
                is_const: binding.is_const,
                kind: binding.kind,
                is_catch_parameter: binding.is_catch_parameter,
            });
        }
        scopes.push(EvalScope {
            kind,
            bindings: bindings.into_boxed_slice(),
        });
    }

    let variable_environment =
        match caller_kind {
            FunctionKind::Script => EvalVariableEnvironment::Global,
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow => {
                EvalVariableEnvironment::Scope(current_function_root.ok_or_else(|| {
                    Error::internal("eval environment has no current function root")
                })?)
            }
            FunctionKind::Eval(_) if caller_strict => {
                EvalVariableEnvironment::Scope(current_function_root.ok_or_else(|| {
                    Error::internal("strict eval environment has no current function root")
                })?)
            }
            FunctionKind::Eval(EvalKind::Direct) => {
                match tree.functions[consuming_function]
                    .eval_caller_profile
                    .variable_target
                {
                    EvalCallerVariableTarget::Global => EvalVariableEnvironment::Global,
                    EvalCallerVariableTarget::ExternalBinding(index) => {
                        EvalVariableEnvironment::Closure(index)
                    }
                    EvalCallerVariableTarget::StrictLocal => {
                        return Err(Error::internal(
                            "sloppy eval root retained a strict-local variable target",
                        ));
                    }
                }
            }
            FunctionKind::Eval(EvalKind::Indirect) => {
                if tree.functions[consuming_function]
                    .eval_caller_profile
                    .variable_target
                    != EvalCallerVariableTarget::Global
                {
                    return Err(Error::internal(
                        "indirect eval root retained a caller variable target",
                    ));
                }
                EvalVariableEnvironment::Global
            }
            FunctionKind::Eval(EvalKind::None) => {
                return Err(Error::internal("eval root has no eval kind"));
            }
        };
    Ok(EvalEnvironment {
        scopes: scopes.into_boxed_slice(),
        variable_environment,
        caller_strict,
        super_call_allowed,
        super_allowed,
    })
}

/// QuickJS adds every Program declaration to the root GLOBAL_DECL list in
/// source order before child-first identifier resolution. Pre-seeding prevents
/// a child capture of a later name from changing declaration-check priority.
fn seed_global_declarations(tree: &mut FunctionTree) -> Result<(), Error> {
    let declarations = tree
        .functions
        .first()
        .ok_or_else(|| Error::internal("compiler produced no root function"))?
        .global_declarations
        .iter()
        .map(|declaration| {
            (
                declaration.name.clone(),
                declaration.is_lexical,
                declaration.is_const,
                declaration.function_constant.is_some(),
            )
        })
        .collect::<Vec<_>>();

    for (declaration_index, (name, is_lexical, is_const, is_function)) in
        declarations.into_iter().enumerate()
    {
        let name_index = ensure_string_constant(&mut tree.functions[0], &name)?;
        let closure_index = push_closure_variable(
            &mut tree.functions[0],
            ClosureVariable {
                source: ClosureSource::GlobalDeclaration,
                name: ClosureVariableName::Constant(name_index),
                is_lexical,
                is_const,
                kind: if is_function {
                    ClosureVariableKind::GlobalFunction
                } else {
                    ClosureVariableKind::Normal
                },
            },
        )?;
        tree.functions[0].global_declarations[declaration_index].closure_index =
            Some(closure_index);
    }
    Ok(())
}

/// QuickJS emits every Program function initializer before authored body
/// bytecode. Each declaration retains its own child constant, while the raw
/// write resolves by name to the first same-name `GLOBAL_DECL` slot.
fn install_global_function_hoists(tree: &mut FunctionTree) -> Result<(), Error> {
    let declarations = tree
        .functions
        .first()
        .ok_or_else(|| Error::internal("compiler produced no root function"))?
        .global_declarations
        .iter()
        .filter_map(|declaration| {
            declaration
                .function_constant
                .map(|constant| (declaration.name.clone(), constant))
        })
        .collect::<Vec<_>>();
    if declarations.is_empty() {
        return Ok(());
    }

    let mut prefix = Vec::with_capacity(declarations.len().saturating_mul(2));
    for (name, constant) in declarations {
        let target = tree.functions[0]
            .global_declarations
            .iter()
            .find(|declaration| declaration.name == name)
            .and_then(|declaration| declaration.closure_index)
            .ok_or_else(|| Error::internal("global function hoist target was not seeded"))?;
        prefix.push(SpannedIrOp {
            op: IrOp::MakeClosure(constant),
            pc_site: None,
        });
        // PutVarInit is the declaration-time raw VarRef write. It is not an
        // ordinary assignment and intentionally bypasses TDZ, const, and
        // global-property setter fallback.
        prefix.push(SpannedIrOp {
            op: IrOp::Bytecode(Instruction::PutVarInit(target)),
            pc_site: None,
        });
    }
    prepend_hoist_prefix(&mut tree.functions[0], prefix)
}

/// Install the source-ordered declaration prelude used by sloppy direct eval
/// in an ordinary caller. Unlike ordinary function hoists, every novel `var`
/// record is retained and writes `undefined`; this preserves QuickJS's
/// observable overwrite behavior across repeated eval invocations.
fn install_eval_declaration_hoists(tree: &mut FunctionTree) -> Result<(), Error> {
    let Some(function) = tree.functions.first_mut() else {
        return Err(Error::internal("compiler produced no root function"));
    };
    if function.eval_declarations_installed {
        return Err(Error::internal(
            "eval declaration hoists were installed more than once",
        ));
    }
    if function.eval_declarations.is_empty() && function.eval_redeclaration.is_none() {
        function.eval_declarations_installed = true;
        return Ok(());
    }
    if !matches!(function.kind, FunctionKind::Eval(EvalKind::Direct)) || function.strict {
        return Err(Error::internal(
            "dynamic eval declarations escaped sloppy direct eval",
        ));
    }

    let declarations = function.eval_declarations.clone();
    let mut prefix = Vec::with_capacity(
        declarations
            .len()
            .saturating_mul(2)
            .saturating_add(usize::from(function.eval_redeclaration.is_some())),
    );
    if let Some(name) = function.eval_redeclaration.clone() {
        let name = ensure_string_constant(function, &name)?;
        prefix.push(SpannedIrOp {
            op: IrOp::Bytecode(Instruction::ThrowRedeclaration(name)),
            pc_site: None,
        });
    }
    for declaration in declarations {
        match declaration.value {
            EvalDeclarationValue::Undefined => {
                if matches!(declaration.target, EvalDeclarationTarget::Dynamic(_)) {
                    prefix.push(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::Undefined),
                        pc_site: None,
                    });
                }
            }
            EvalDeclarationValue::Function(constant) => prefix.push(SpannedIrOp {
                op: IrOp::MakeClosure(constant),
                pc_site: None,
            }),
        }

        let write = match declaration.target {
            EvalDeclarationTarget::Dynamic(source) => {
                let name = ensure_string_constant(function, &declaration.name)?;
                Some(IrOp::Bytecode(Instruction::DefineEvalVariable {
                    source,
                    name,
                }))
            }
            EvalDeclarationTarget::External { index, kind } => match declaration.value {
                EvalDeclarationValue::Undefined => None,
                EvalDeclarationValue::Function(_) => Some(closure_binding_operation(
                    function,
                    index,
                    kind,
                    IdentifierAccess::Put,
                    &declaration.name,
                )?),
            },
        };
        if let Some(write) = write {
            prefix.push(SpannedIrOp {
                op: write,
                pc_site: None,
            });
        }
    }
    prepend_hoist_prefix(function, prefix)?;
    function.eval_declarations_installed = true;
    Ok(())
}

/// QuickJS initializes the lazily selected arguments binding before storing
/// direct body function declarations into argument/root-local slots.
fn install_function_body_hoists(tree: &mut FunctionTree) -> Result<(), Error> {
    for function_id in 0..tree.functions.len() {
        if matches!(tree.functions[function_id].kind, FunctionKind::Script) {
            continue;
        }
        let hoists = ordered_hoisted_functions(&tree.functions[function_id])?;
        let arguments_local = tree.functions[function_id].arguments_local;
        let eval_variable_object_local = tree.functions[function_id].eval_variable_object_local;
        let mut prefix = Vec::with_capacity(
            hoists
                .len()
                .saturating_mul(2)
                .saturating_add(usize::from(arguments_local.is_some()) * 2),
        );
        if let Some(local) = eval_variable_object_local {
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::VariableEnvironment),
                pc_site: None,
            });
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::PutLocal(local)),
                pc_site: None,
            });
        }
        if let Some(local) = arguments_local {
            let kind = if tree.functions[function_id].strict {
                ArgumentsKind::Unmapped
            } else {
                ArgumentsKind::Mapped
            };
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::Arguments(kind)),
                pc_site: None,
            });
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::PutLocal(local)),
                pc_site: None,
            });
        }
        for hoist in hoists {
            let binding = &tree.functions[function_id].bindings[hoist.binding.0];
            prefix.push(SpannedIrOp {
                op: IrOp::MakeClosure(hoist.constant),
                pc_site: None,
            });
            let instruction = match binding.storage {
                BindingStorage::Argument(index) => Instruction::PutArg(index),
                BindingStorage::Local(index) => Instruction::PutLocal(index),
                BindingStorage::External(_) | BindingStorage::Global => {
                    return Err(Error::internal(
                        "ordinary function hoist targeted global storage",
                    ));
                }
            };
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(instruction),
                pc_site: None,
            });
        }
        prepend_hoist_prefix(&mut tree.functions[function_id], prefix)?;
        tree.functions[function_id].function_hoists_installed = true;
    }
    Ok(())
}

fn ordered_hoisted_functions(function: &FunctionIr) -> Result<Vec<IrHoistedFunction>, Error> {
    let mut hoists = function.hoisted_functions.clone();
    for hoist in &hoists {
        let binding = function
            .bindings
            .get(hoist.binding.0)
            .ok_or_else(|| Error::internal("hoisted function binding is out of bounds"))?;
        if matches!(
            binding.storage,
            BindingStorage::External(_) | BindingStorage::Global
        ) {
            return Err(Error::internal(
                "ordinary function hoist targeted global storage",
            ));
        }
    }
    hoists.sort_by_key(|hoist| match function.bindings[hoist.binding.0].storage {
        BindingStorage::Argument(index) => (0_u8, index),
        BindingStorage::Local(index) => (1_u8, index),
        BindingStorage::External(_) => unreachable!("validated above"),
        BindingStorage::Global => unreachable!("validated above"),
    });
    Ok(hoists)
}

fn prepend_hoist_prefix(
    function: &mut FunctionIr,
    mut prefix: Vec<SpannedIrOp>,
) -> Result<(), Error> {
    if prefix.is_empty() {
        return Ok(());
    }
    let shift = u32::try_from(prefix.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    for scoped in &mut function.scoped_functions {
        scoped.authored_closure = scoped
            .authored_closure
            .checked_add(prefix.len())
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    }
    for annex in &mut function.program_annex_functions {
        annex.authored_closure = annex
            .authored_closure
            .checked_add(prefix.len())
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    }
    for operation in &mut function.ops {
        let target = match &mut operation.op {
            IrOp::Bytecode(
                Instruction::Goto(target)
                | Instruction::IfFalse(target)
                | Instruction::IfTrue(target)
                | Instruction::Catch(target)
                | Instruction::Gosub(target),
            ) => target,
            _ => continue,
        };
        *target = target
            .checked_add(shift)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    }
    prefix.append(&mut function.ops);
    function.ops = prefix;
    Ok(())
}

fn apply_quickjs_late_throw_sites(
    code: &[Instruction],
    pc_sites: &mut [Option<SourceOffset>],
) -> Result<(), Error> {
    if code.len() != pc_sites.len() {
        return Err(Error::internal(
            "lowered instructions and source markers have different lengths",
        ));
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
            Instruction::Goto(_) | Instruction::Return | Instruction::Throw | Instruction::Ret
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

fn resolve_identifier(
    tree: &mut FunctionTree,
    function_id: FunctionId,
    use_scope: ScopeId,
    name: &str,
    span: Span,
    access: IdentifierAccess,
) -> Result<IrOp, Error> {
    if access == IdentifierAccess::AnnexBPut {
        let binding = find_or_create_own_binding(tree, function_id, use_scope, name, span)?
            .ok_or_else(|| Error::internal("Annex B root binding was not registered"))?;
        if binding.kind != BindingKind::Normal
            && !matches!(binding.storage, BindingStorage::External(_))
        {
            return Err(Error::internal(
                "Annex B root write resolved to a non-ordinary binding",
            ));
        }
        if binding.storage == BindingStorage::Global {
            let closure_index = capture_global_path(tree, function_id, name)?;
            return Ok(IrOp::Bytecode(Instruction::PutVar(closure_index)));
        }
        if let BindingStorage::External(index) = binding.storage {
            return closure_binding_operation(
                &mut tree.functions[function_id],
                index,
                binding.kind,
                IdentifierAccess::Put,
                name,
            );
        }
        return binding_instruction(
            &mut tree.functions[function_id],
            binding,
            IdentifierAccess::Put,
            name,
        )
        .map(IrOp::Bytecode);
    }
    let path = resolve_identifier_path(tree, function_id, use_scope, name, span, access)?;
    wrap_dynamic_identifier(
        &mut tree.functions[function_id],
        name,
        access,
        path.sources,
        path.fallback,
    )
}

#[derive(Debug)]
struct ResolvedIdentifierPath {
    sources: Vec<DynamicEnvironmentSource>,
    fallback: IrOp,
    fallback_readonly: bool,
}

fn resolve_identifier_reference(
    tree: &mut FunctionTree,
    function_id: FunctionId,
    use_scope: ScopeId,
    name: &str,
    span: Span,
    access: IdentifierReferenceAccess,
) -> Result<IrOp, Error> {
    let fallback_access = match access {
        IdentifierReferenceAccess::Prepare | IdentifierReferenceAccess::Set => {
            IdentifierAccess::Set
        }
        IdentifierReferenceAccess::Get | IdentifierReferenceAccess::Call => IdentifierAccess::Get,
        IdentifierReferenceAccess::PostPut => IdentifierAccess::Put,
    };
    let path = resolve_identifier_path(tree, function_id, use_scope, name, span, fallback_access)?;
    let syntactic_with = has_authored_with_scope(tree, function_id, use_scope)?;
    let (sources, late_sources) = if syntactic_with {
        (path.sources, Vec::new())
    } else {
        (Vec::new(), path.sources)
    };
    let name = ensure_string_constant(&mut tree.functions[function_id], name)?;
    Ok(IrOp::DynamicIdentifierReference {
        name,
        access,
        sources: sources.into_boxed_slice(),
        late_sources: late_sources.into_boxed_slice(),
        fallback: Box::new(path.fallback),
        syntactic_with,
        fallback_readonly: path.fallback_readonly,
    })
}

fn has_authored_with_scope(
    tree: &FunctionTree,
    function_id: FunctionId,
    use_scope: ScopeId,
) -> Result<bool, Error> {
    let mut owner = function_id;
    let mut scope = use_scope;
    loop {
        loop {
            let current = tree.functions[owner]
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
        let Some(parent) = tree.functions[owner].parent else {
            return Ok(false);
        };
        owner = parent.function;
        scope = parent.definition_scope;
    }
}

/// Resolve one identifier scope-by-scope.  An exact authored binding wins in
/// its scope; otherwise that scope's hidden `with` record is appended before
/// continuing outward. Synthetic eval roots replay their imported descriptors
/// in the original inner-to-outer order, including `<var>` and `<with>`.
fn resolve_identifier_path(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
    use_scope: ScopeId,
    name: &str,
    span: Span,
    access: IdentifierAccess,
) -> Result<ResolvedIdentifierPath, Error> {
    let mut sources = Vec::new();
    let pseudo = PseudoBinding::from_name(name);
    if pseudo.is_some()
        && !matches!(
            access,
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined
        )
    {
        return Err(Error::internal(
            "pseudo binding received a non-read operation",
        ));
    }
    let mut owner = consuming_function;
    let mut scope = use_scope;
    loop {
        loop {
            let (scope_kind, parent, exact, with_binding) = {
                let function = tree
                    .functions
                    .get(owner)
                    .ok_or_else(|| Error::internal("identifier owner is out of bounds"))?;
                let current = function
                    .scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("identifier use scope is out of bounds"))?;
                let exact = current.bindings.iter().rev().find_map(|binding| {
                    let binding = function.bindings.get(binding.0)?;
                    (binding.name == name
                        && (pseudo.is_some()
                            || !matches!(binding.storage, BindingStorage::External(_))))
                    .then_some(ResolvedBinding {
                        storage: binding.storage,
                        kind: binding.kind,
                    })
                });
                let with_binding = (pseudo.is_none() && current.kind == ScopeKind::With)
                    .then(|| {
                        current
                            .bindings
                            .iter()
                            .find_map(|binding| {
                                let binding = function.bindings.get(binding.0)?;
                                (binding.kind == BindingKind::WithObject).then_some(
                                    ResolvedBinding {
                                        storage: binding.storage,
                                        kind: binding.kind,
                                    },
                                )
                            })
                            .ok_or_else(|| {
                                Error::internal("with scope lost its hidden object binding")
                            })
                    })
                    .transpose()?;
                (current.kind, current.parent, exact, with_binding)
            };

            if let Some(binding) = exact {
                let fallback_readonly = binding_is_readonly(binding.kind);
                let fallback = resolved_binding_operation(
                    tree,
                    owner,
                    consuming_function,
                    binding,
                    access,
                    name,
                )?;
                return Ok(ResolvedIdentifierPath {
                    sources,
                    fallback,
                    fallback_readonly,
                });
            }

            if let Some(binding) = with_binding {
                push_dynamic_environment_source(
                    tree,
                    owner,
                    consuming_function,
                    binding,
                    WITH_OBJECT_LOCAL_NAME,
                    &mut sources,
                )?;
            }

            let Some(parent) = parent else {
                debug_assert_eq!(scope_kind, ScopeKind::FunctionRoot);
                break;
            };
            scope = parent;
        }

        let own_binding = if let Some(pseudo) = pseudo {
            find_or_create_own_pseudo_binding(tree, owner, pseudo, span)?
        } else {
            // `arguments` and a private function-expression name are
            // logically rooted before the function's own eval variable
            // object. A sloppy delete of implicit `arguments` is false
            // without materializing it.
            if name == "arguments"
                && access == IdentifierAccess::Delete
                && matches!(
                    tree.functions[owner].kind,
                    FunctionKind::Ordinary | FunctionKind::Method
                )
            {
                return Ok(ResolvedIdentifierPath {
                    sources,
                    fallback: IrOp::Bytecode(Instruction::PushFalse),
                    fallback_readonly: false,
                });
            }
            if matches!(tree.functions[owner].kind, FunctionKind::Eval(_)) {
                None
            } else {
                find_or_create_own_binding(tree, owner, ScopeId(0), name, span)?
            }
        };
        if let Some(binding) = own_binding {
            let fallback_readonly = binding_is_readonly(binding.kind);
            let fallback =
                resolved_binding_operation(tree, owner, consuming_function, binding, access, name)?;
            return Ok(ResolvedIdentifierPath {
                sources,
                fallback,
                fallback_readonly,
            });
        }

        if matches!(tree.functions[owner].kind, FunctionKind::Eval(_)) {
            if let Some(binding) =
                resolve_eval_external_chain(tree, owner, consuming_function, name, &mut sources)?
            {
                let fallback_readonly = binding_is_readonly(binding.kind);
                let fallback = resolved_binding_operation(
                    tree,
                    owner,
                    consuming_function,
                    binding,
                    access,
                    name,
                )?;
                return Ok(ResolvedIdentifierPath {
                    sources,
                    fallback,
                    fallback_readonly,
                });
            }
        } else if pseudo.is_none() {
            push_owned_eval_variable_source(tree, owner, consuming_function, &mut sources)?;
        }

        let Some(parent) = tree.functions[owner].parent else {
            break;
        };
        owner = parent.function;
        scope = parent.definition_scope;
    }

    if pseudo.is_some() {
        return Err(Error::internal(
            "pseudo binding escaped every authenticated function owner",
        ));
    }
    let closure_index = capture_global_path(tree, consuming_function, name)?;
    let fallback = match access {
        IdentifierAccess::Get => IrOp::Bytecode(Instruction::GetVar(closure_index)),
        IdentifierAccess::GetOrUndefined => IrOp::Bytecode(Instruction::GetVarUndef(closure_index)),
        IdentifierAccess::Delete => IrOp::Bytecode(Instruction::DeleteVar(closure_index)),
        IdentifierAccess::Initialize => {
            return Err(Error::internal(
                "lexical initializer did not resolve to its owning local",
            ));
        }
        IdentifierAccess::Put => IrOp::Bytecode(Instruction::PutVar(closure_index)),
        IdentifierAccess::AnnexBPut => {
            return Err(Error::internal(
                "Annex B write escaped its dedicated resolver path",
            ));
        }
        IdentifierAccess::Set => IrOp::GlobalSet(closure_index),
    };
    Ok(ResolvedIdentifierPath {
        sources,
        fallback,
        fallback_readonly: false,
    })
}

const fn binding_is_readonly(kind: BindingKind) -> bool {
    matches!(
        kind,
        BindingKind::Lexical { is_const: true } | BindingKind::FunctionName { is_const: true }
    )
}

fn resolved_binding_operation(
    tree: &mut FunctionTree,
    defining_function: FunctionId,
    consuming_function: FunctionId,
    binding: ResolvedBinding,
    access: IdentifierAccess,
    name: &str,
) -> Result<IrOp, Error> {
    if binding.storage == BindingStorage::Global {
        return global_declaration_operation(tree, consuming_function, binding.kind, access, name);
    }
    if defining_function == consuming_function {
        if let BindingStorage::External(index) = binding.storage {
            return closure_binding_operation(
                &mut tree.functions[consuming_function],
                index,
                binding.kind,
                access,
                name,
            );
        }
        return binding_instruction(
            &mut tree.functions[consuming_function],
            binding,
            access,
            name,
        )
        .map(IrOp::Bytecode);
    }
    let (closure_index, kind) = capture_binding_path(
        tree,
        defining_function,
        consuming_function,
        binding,
        name,
        false,
        false,
    )?;
    closure_binding_operation(
        &mut tree.functions[consuming_function],
        closure_index,
        kind,
        access,
        name,
    )
}

fn push_owned_eval_variable_source(
    tree: &mut FunctionTree,
    defining_function: FunctionId,
    consuming_function: FunctionId,
    sources: &mut Vec<DynamicEnvironmentSource>,
) -> Result<(), Error> {
    let Some(index) = tree.functions[defining_function].eval_variable_object_local else {
        return Ok(());
    };
    push_dynamic_environment_source(
        tree,
        defining_function,
        consuming_function,
        ResolvedBinding {
            storage: BindingStorage::Local(index),
            kind: BindingKind::EvalVariableObject,
        },
        EVAL_VARIABLE_OBJECT_LOCAL_NAME,
        sources,
    )
}

fn push_dynamic_environment_source(
    tree: &mut FunctionTree,
    defining_function: FunctionId,
    consuming_function: FunctionId,
    binding: ResolvedBinding,
    sentinel: &str,
    sources: &mut Vec<DynamicEnvironmentSource>,
) -> Result<(), Error> {
    let storage = if defining_function == consuming_function {
        binding.storage
    } else {
        let (closure, kind) = capture_binding_path(
            tree,
            defining_function,
            consuming_function,
            binding,
            sentinel,
            true,
            false,
        )?;
        if kind != binding.kind {
            return Err(Error::internal(
                "dynamic environment closure relay changed kind",
            ));
        }
        BindingStorage::External(closure)
    };
    let source = match (binding.kind, storage) {
        (BindingKind::EvalVariableObject, BindingStorage::Local(index)) => {
            DynamicEnvironmentSource::Eval(EvalVariableSource::Local(index))
        }
        (BindingKind::EvalVariableObject, BindingStorage::External(index)) => {
            DynamicEnvironmentSource::Eval(EvalVariableSource::Closure(index))
        }
        (BindingKind::WithObject, BindingStorage::Local(index)) => {
            DynamicEnvironmentSource::With(WithObjectSource::Local(index))
        }
        (BindingKind::WithObject, BindingStorage::External(index)) => {
            DynamicEnvironmentSource::With(WithObjectSource::Closure(index))
        }
        (
            BindingKind::Normal | BindingKind::Lexical { .. } | BindingKind::FunctionName { .. },
            _,
        )
        | (_, BindingStorage::Argument(_) | BindingStorage::Global) => {
            return Err(Error::internal(
                "ordinary binding reached dynamic environment selection",
            ));
        }
    };
    sources.push(source);
    Ok(())
}

fn resolve_eval_external_chain(
    tree: &mut FunctionTree,
    defining_function: FunctionId,
    consuming_function: FunctionId,
    name: &str,
    sources: &mut Vec<DynamicEnvironmentSource>,
) -> Result<Option<ResolvedBinding>, Error> {
    let external = tree.functions[defining_function].external_bindings.clone();
    for (index, binding) in external.into_iter().enumerate() {
        let index = u16::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
        if matches!(
            binding.kind,
            ClosureVariableKind::EvalVariableObject | ClosureVariableKind::WithObject
        ) {
            let (kind, sentinel) = match binding.kind {
                ClosureVariableKind::EvalVariableObject => (
                    BindingKind::EvalVariableObject,
                    EVAL_VARIABLE_OBJECT_LOCAL_NAME,
                ),
                ClosureVariableKind::WithObject => {
                    (BindingKind::WithObject, WITH_OBJECT_LOCAL_NAME)
                }
                _ => unreachable!(),
            };
            push_dynamic_environment_source(
                tree,
                defining_function,
                consuming_function,
                ResolvedBinding {
                    storage: BindingStorage::External(index),
                    kind,
                },
                sentinel,
                sources,
            )?;
            continue;
        }
        if binding.name.to_utf8_lossy() != name {
            continue;
        }
        let kind = match binding.kind {
            ClosureVariableKind::Normal if binding.is_lexical => BindingKind::Lexical {
                is_const: binding.is_const,
            },
            ClosureVariableKind::Normal if !binding.is_const => BindingKind::Normal,
            ClosureVariableKind::FunctionName if !binding.is_lexical => BindingKind::FunctionName {
                is_const: binding.is_const,
            },
            ClosureVariableKind::Normal
            | ClosureVariableKind::FunctionName
            | ClosureVariableKind::GlobalFunction
            | ClosureVariableKind::EvalVariableObject
            | ClosureVariableKind::WithObject => {
                return Err(Error::internal(
                    "eval caller binding flags are inconsistent",
                ));
            }
        };
        return Ok(Some(ResolvedBinding {
            storage: BindingStorage::External(index),
            kind,
        }));
    }
    Ok(None)
}

fn wrap_dynamic_identifier(
    function: &mut FunctionIr,
    name: &str,
    access: IdentifierAccess,
    sources: Vec<DynamicEnvironmentSource>,
    fallback: IrOp,
) -> Result<IrOp, Error> {
    if sources.is_empty() {
        return Ok(fallback);
    }
    if matches!(
        access,
        IdentifierAccess::Initialize | IdentifierAccess::AnnexBPut
    ) {
        return Err(Error::internal(
            "declaration-only identifier access crossed a dynamic environment",
        ));
    }
    let name = ensure_string_constant(function, name)?;
    Ok(IrOp::DynamicIdentifier {
        name,
        access,
        sources: sources.into_boxed_slice(),
        fallback: Box::new(fallback),
    })
}

fn global_declaration_operation(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
    kind: BindingKind,
    access: IdentifierAccess,
    name: &str,
) -> Result<IrOp, Error> {
    let binding_is_lexical = match kind {
        BindingKind::Normal => false,
        BindingKind::Lexical { .. } => true,
        BindingKind::FunctionName { .. } => {
            return Err(Error::internal(
                "global declaration has function-name binding metadata",
            ));
        }
        BindingKind::EvalVariableObject => {
            return Err(Error::internal(
                "eval variable object reached global declaration resolution",
            ));
        }
        BindingKind::WithObject => {
            return Err(Error::internal(
                "with object reached global declaration resolution",
            ));
        }
    };
    // `resolve_scope_var` scans QuickJS's ordered GLOBAL_DECL list by name and
    // therefore resolves through the first matching descriptor, even when a
    // preceding Annex B normal record has masked a later Program lexical. The
    // two descriptors share the lexical VarRef after instantiation, but the
    // first descriptor's non-lexical flag remains observable: an uninitialized
    // read falls back to the replacement global-object property instead of
    // throwing a lexical TDZ error. Writes still inspect the shared VarRef's
    // lexical/const metadata in the VM.
    let descriptor_is_lexical =
        binding_is_lexical && !tree.functions[0].first_global_declaration_is_normal(name);
    let closure_index =
        capture_global_declaration_path(tree, consuming_function, name, descriptor_is_lexical)?;
    Ok(match access {
        IdentifierAccess::Get => IrOp::Bytecode(Instruction::GetVar(closure_index)),
        IdentifierAccess::GetOrUndefined => IrOp::Bytecode(Instruction::GetVarUndef(closure_index)),
        IdentifierAccess::Delete => IrOp::Bytecode(Instruction::DeleteVar(closure_index)),
        IdentifierAccess::Initialize if binding_is_lexical => {
            IrOp::Bytecode(Instruction::PutVarInit(closure_index))
        }
        IdentifierAccess::Initialize => {
            return Err(Error::internal(
                "ordinary global declaration used lexical initialization",
            ));
        }
        IdentifierAccess::Put => IrOp::Bytecode(Instruction::PutVar(closure_index)),
        IdentifierAccess::AnnexBPut => {
            return Err(Error::internal(
                "Annex B write reached declaration-bound global resolution",
            ));
        }
        IdentifierAccess::Set => IrOp::GlobalSet(closure_index),
    })
}

/// Install QuickJS's `GLOBAL_DECL -> PARENT_GLOBAL` chain for a Program
/// binding. The root descriptor triggers declaration instantiation;
/// descendants reuse that exact VarRef without repeating the definition.
fn capture_global_declaration_path(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
    name: &str,
    is_lexical: bool,
) -> Result<u16, Error> {
    let mut path = Vec::new();
    let mut cursor = Some(consuming_function);
    while let Some(function_id) = cursor {
        path.push(function_id);
        cursor = tree.functions[function_id]
            .parent
            .map(|parent| parent.function);
    }
    path.reverse();

    let (root_index, root_descriptor) = tree.functions[0]
        .global_declarations
        .iter()
        .find(|declaration| declaration.name == name && declaration.is_lexical == is_lexical)
        .and_then(|declaration| declaration.closure_index)
        .and_then(|index| {
            tree.functions[0]
                .closure_variables
                .get(usize::from(index))
                .copied()
                .map(|descriptor| (index, descriptor))
        })
        .ok_or_else(|| Error::internal("global declaration closure was not seeded"))?;
    let mut source = ClosureSource::ParentGlobal(root_index);
    let mut final_index = Some(root_index);
    for function_id in path.into_iter().skip(1) {
        let name_index = ensure_string_constant(&mut tree.functions[function_id], name)?;
        let descriptor = ClosureVariable {
            source,
            name: ClosureVariableName::Constant(name_index),
            is_lexical: root_descriptor.is_lexical,
            is_const: root_descriptor.is_const,
            kind: root_descriptor.kind,
        };
        let index = ensure_closure_variable(&mut tree.functions[function_id], descriptor)?;
        source = ClosureSource::ParentGlobal(index);
        final_index = Some(index);
    }
    final_index.ok_or_else(|| Error::internal("global declaration closure path was empty"))
}

/// Install the same Global -> ParentGlobal relay chain QuickJS creates while
/// resolving an otherwise-unbound identifier. Every function owns its exact
/// name atom after publication; descendants share the root VarRef identity.
fn capture_global_path(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
    name: &str,
) -> Result<u16, Error> {
    let mut path = Vec::new();
    let mut cursor = Some(consuming_function);
    while let Some(function_id) = cursor {
        path.push(function_id);
        cursor = tree.functions[function_id]
            .parent
            .map(|parent| parent.function);
    }
    path.reverse();

    let mut source = ClosureSource::Global;
    let mut final_index = None;
    for function_id in path {
        let name_index = ensure_string_constant(&mut tree.functions[function_id], name)?;
        let descriptor = ClosureVariable {
            source,
            name: ClosureVariableName::Constant(name_index),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        };
        let index = ensure_closure_variable(&mut tree.functions[function_id], descriptor)?;
        source = ClosureSource::ParentGlobal(index);
        final_index = Some(index);
    }
    final_index.ok_or_else(|| Error::internal("global closure path was empty"))
}

fn ensure_string_constant(function: &mut FunctionIr, name: &str) -> Result<u32, Error> {
    let name = JsString::try_from_utf8(name)?;
    if let Some(index) = function.constants.iter().position(
        |constant| matches!(constant, IrConstant::Primitive(Value::String(value)) if value == &name),
    ) {
        return u32::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"));
    }
    let index = u32::try_from(function.constants.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?;
    function
        .constants
        .push(IrConstant::Primitive(Value::String(name)));
    Ok(index)
}

fn find_or_create_own_binding(
    tree: &mut FunctionTree,
    function_id: FunctionId,
    start_scope: ScopeId,
    name: &str,
    span: Span,
) -> Result<Option<ResolvedBinding>, Error> {
    let function = &tree.functions[function_id];
    if start_scope.0 >= function.scopes.len() {
        return Err(Error::internal("identifier use scope is out of bounds"));
    }
    if let Some(binding) = function.binding_from_scope(start_scope, name) {
        return Ok(Some(binding));
    }
    if name == "arguments" && matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
    {
        let function = &mut tree.functions[function_id];
        if function.arguments_local.is_some() {
            return Err(Error::internal(
                "implicit arguments local is missing its root binding",
            ));
        }
        if function.locals.len() >= MAX_LOCAL_VARIABLES {
            return Err(
                Error::new(ErrorKind::JsInternal, "too many local variables")
                    .with_span(source_span(span)),
            );
        }
        let index = u16::try_from(function.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        function.locals.push(name.to_owned());
        function.arguments_local = Some(index);
        function.add_binding(
            function.var_scope,
            function.var_scope,
            name.to_owned(),
            BindingStorage::Local(index),
            BindingKind::Normal,
            None,
        );
        return Ok(Some(ResolvedBinding {
            storage: BindingStorage::Local(index),
            kind: BindingKind::Normal,
        }));
    }
    if !function.private_name_binding || function.function_name.as_deref() != Some(name) {
        return Ok(None);
    }

    let function = &mut tree.functions[function_id];
    if function.locals.len() >= MAX_LOCAL_VARIABLES {
        return Err(
            Error::new(ErrorKind::JsInternal, "too many local variables")
                .with_span(source_span(span)),
        );
    }
    let index = u16::try_from(function.locals.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
    let kind = BindingKind::FunctionName {
        is_const: function.strict,
    };
    function.locals.push(name.to_owned());
    function.function_name_local = Some(index);
    function.add_binding(
        function.var_scope,
        function.var_scope,
        name.to_owned(),
        BindingStorage::Local(index),
        kind,
        None,
    );
    Ok(Some(ResolvedBinding {
        storage: BindingStorage::Local(index),
        kind,
    }))
}

fn binding_instruction(
    function: &mut FunctionIr,
    binding: ResolvedBinding,
    access: IdentifierAccess,
    name: &str,
) -> Result<Instruction, Error> {
    match (binding.storage, binding.kind, access) {
        (BindingStorage::Global, _, _) => Err(Error::internal(
            "global binding reached local binding instruction selection",
        )),
        (BindingStorage::External(_), _, _) => Err(Error::internal(
            "eval external binding reached local instruction selection",
        )),
        (
            BindingStorage::Local(_),
            BindingKind::EvalVariableObject | BindingKind::WithObject,
            _,
        ) => Err(Error::internal(
            "hidden object binding reached source binding instruction selection",
        )),
        (
            BindingStorage::Argument(index),
            _,
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
        ) => Ok(Instruction::GetArg(index)),
        (BindingStorage::Argument(_) | BindingStorage::Local(_), _, IdentifierAccess::Delete) => {
            Ok(Instruction::PushFalse)
        }
        (BindingStorage::Argument(_), _, IdentifierAccess::Initialize) => Err(Error::internal(
            "lexical initializer resolved to an argument binding",
        )),
        (BindingStorage::Argument(index), _, IdentifierAccess::Put) => {
            Ok(Instruction::PutArg(index))
        }
        (BindingStorage::Argument(index), _, IdentifierAccess::Set) => {
            Ok(Instruction::SetArg(index))
        }
        (
            BindingStorage::Local(index),
            BindingKind::Normal | BindingKind::FunctionName { .. },
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
        ) => Ok(Instruction::GetLocal(index)),
        (
            BindingStorage::Local(index),
            BindingKind::Lexical { .. },
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
        ) => Ok(Instruction::GetLocalCheck(index)),
        (
            BindingStorage::Local(index),
            BindingKind::Lexical { .. },
            IdentifierAccess::Initialize,
        ) => Ok(Instruction::InitializeLocal(index)),
        (
            BindingStorage::Local(_),
            BindingKind::Normal | BindingKind::FunctionName { .. },
            IdentifierAccess::Initialize,
        ) => Err(Error::internal(
            "lexical initializer resolved to an ordinary local",
        )),
        (BindingStorage::Local(index), BindingKind::Normal, IdentifierAccess::Put) => {
            Ok(Instruction::PutLocal(index))
        }
        (BindingStorage::Local(index), BindingKind::Normal, IdentifierAccess::Set) => {
            Ok(Instruction::SetLocal(index))
        }
        (
            BindingStorage::Local(index),
            BindingKind::Lexical { is_const: false },
            IdentifierAccess::Put,
        ) => Ok(Instruction::PutLocalCheck(index)),
        (
            BindingStorage::Local(index),
            BindingKind::Lexical { is_const: false },
            IdentifierAccess::Set,
        ) => Ok(Instruction::SetLocalCheck(index)),
        (
            BindingStorage::Local(_),
            BindingKind::Lexical { is_const: true },
            IdentifierAccess::Put | IdentifierAccess::Set,
        ) => {
            let name = ensure_string_constant(function, name)?;
            Ok(Instruction::ThrowReadOnly(name))
        }
        (
            BindingStorage::Local(_),
            BindingKind::FunctionName { is_const },
            IdentifierAccess::Put | IdentifierAccess::Set,
        ) => function_name_write_instruction(function, name, is_const, access),
        (_, _, IdentifierAccess::AnnexBPut) => Err(Error::internal(
            "Annex B write reached ordinary binding instruction selection",
        )),
    }
}

fn closure_binding_operation(
    function: &mut FunctionIr,
    index: u16,
    kind: BindingKind,
    access: IdentifierAccess,
    name: &str,
) -> Result<IrOp, Error> {
    match (kind, access) {
        (BindingKind::EvalVariableObject | BindingKind::WithObject, _) => Err(Error::internal(
            "hidden object binding reached source closure operation selection",
        )),
        (
            BindingKind::Normal | BindingKind::FunctionName { .. },
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
        ) => Ok(IrOp::Bytecode(Instruction::GetVarRef(index))),
        (BindingKind::Lexical { .. }, IdentifierAccess::Get | IdentifierAccess::GetOrUndefined) => {
            Ok(IrOp::Bytecode(Instruction::GetVarRefCheck(index)))
        }
        (_, IdentifierAccess::Delete) => Ok(IrOp::Bytecode(Instruction::PushFalse)),
        (_, IdentifierAccess::Initialize) => Err(Error::internal(
            "lexical initializer crossed a function boundary",
        )),
        (BindingKind::Normal, IdentifierAccess::Put) => {
            Ok(IrOp::Bytecode(Instruction::PutVarRef(index)))
        }
        (BindingKind::Normal, IdentifierAccess::Set) => {
            Ok(IrOp::Bytecode(Instruction::SetVarRef(index)))
        }
        (BindingKind::Lexical { is_const: false }, IdentifierAccess::Put) => {
            Ok(IrOp::Bytecode(Instruction::PutVarRefCheck(index)))
        }
        (BindingKind::Lexical { is_const: false }, IdentifierAccess::Set) => {
            Ok(IrOp::CapturedLexicalSet(index))
        }
        (
            BindingKind::Lexical { is_const: true },
            IdentifierAccess::Put | IdentifierAccess::Set,
        ) => {
            let name = ensure_string_constant(function, name)?;
            Ok(IrOp::Bytecode(Instruction::ThrowReadOnly(name)))
        }
        (BindingKind::FunctionName { is_const }, IdentifierAccess::Put | IdentifierAccess::Set) => {
            function_name_write_instruction(function, name, is_const, access).map(IrOp::Bytecode)
        }
        (_, IdentifierAccess::AnnexBPut) => {
            Err(Error::internal("Annex B write crossed a function boundary"))
        }
    }
}

fn function_name_write_instruction(
    function: &mut FunctionIr,
    name: &str,
    is_const: bool,
    access: IdentifierAccess,
) -> Result<Instruction, Error> {
    if is_const {
        let name = ensure_string_constant(function, name)?;
        return Ok(Instruction::ThrowReadOnly(name));
    }
    Ok(match access {
        IdentifierAccess::Put => Instruction::Drop,
        IdentifierAccess::Set => Instruction::Nop,
        IdentifierAccess::Get
        | IdentifierAccess::GetOrUndefined
        | IdentifierAccess::Delete
        | IdentifierAccess::Initialize
        | IdentifierAccess::AnnexBPut => {
            return Err(Error::internal(
                "function-name write received a read access",
            ));
        }
    })
}

const fn closure_kind(kind: BindingKind) -> ClosureVariableKind {
    match kind {
        BindingKind::Normal | BindingKind::Lexical { .. } => ClosureVariableKind::Normal,
        BindingKind::FunctionName { .. } => ClosureVariableKind::FunctionName,
        BindingKind::EvalVariableObject => ClosureVariableKind::EvalVariableObject,
        BindingKind::WithObject => ClosureVariableKind::WithObject,
    }
}

fn capture_binding_path(
    tree: &mut FunctionTree,
    defining_function: FunctionId,
    consuming_function: FunctionId,
    binding: ResolvedBinding,
    name: &str,
    retain_name: bool,
    erase_function_name: bool,
) -> Result<(u16, BindingKind), Error> {
    let mut path = Vec::new();
    let mut cursor = consuming_function;
    while cursor != defining_function {
        path.push(cursor);
        cursor = tree.functions[cursor]
            .parent
            .ok_or_else(|| Error::internal("closure binding owner is not an ancestor"))?
            .function;
    }
    path.reverse();

    // QuickJS's `add_eval_variables` requests ordinary metadata for an
    // ancestor's unscoped FunctionName. `get_closure_var` recursively carries
    // that request unchanged but de-duplicates each hop solely by physical
    // source, so the first descriptor already occupying a slot wins. A later
    // plain descendant may therefore restore FunctionName on its own final
    // descriptor while relaying through an erased parent view. Synthetic Eval
    // roots use a different copy branch and never request this erasure.
    let may_lose_function_name = matches!(binding.kind, BindingKind::FunctionName { .. })
        && matches!(
            tree.functions[defining_function].kind,
            FunctionKind::Ordinary
        );
    let original_kind = binding.kind;
    let requested_kind = if erase_function_name && may_lose_function_name {
        BindingKind::Normal
    } else {
        original_kind
    };
    let mut source = match binding.storage {
        BindingStorage::Argument(index) => ClosureSource::ParentArgument(index),
        BindingStorage::Local(index) => ClosureSource::ParentLocal(index),
        BindingStorage::External(index) => ClosureSource::ParentClosure(index),
        BindingStorage::Global => {
            return Err(Error::internal(
                "global binding reached local closure capture",
            ));
        }
    };
    let mut final_index = None;
    let mut final_kind = None;
    for function_id in path {
        let function = &mut tree.functions[function_id];
        let descriptor_name = if retain_name
            || matches!(
                original_kind,
                BindingKind::Lexical { .. }
                    | BindingKind::FunctionName { .. }
                    | BindingKind::EvalVariableObject
                    | BindingKind::WithObject
            ) {
            ClosureVariableName::Constant(ensure_string_constant(function, name)?)
        } else {
            ClosureVariableName::None
        };
        let descriptor = ClosureVariable {
            source,
            name: descriptor_name,
            is_lexical: matches!(requested_kind, BindingKind::Lexical { .. }),
            is_const: matches!(
                requested_kind,
                BindingKind::Lexical { is_const: true }
                    | BindingKind::FunctionName { is_const: true }
            ),
            kind: closure_kind(requested_kind),
        };
        let (index, actual_kind) =
            ensure_captured_closure_variable(function, descriptor, requested_kind)?;
        source = ClosureSource::ParentClosure(index);
        final_index = Some(index);
        final_kind = Some(actual_kind);
    }
    final_index
        .zip(final_kind)
        .ok_or_else(|| Error::internal("closure path did not cross a function boundary"))
}

/// Mirror QuickJS `get_closure_var`: the physical parent source, rather than
/// the requested flags, identifies a closure slot. The first request wins its
/// observable metadata. Later eval linking may still upgrade an omitted Rust
/// name because the source compiler always retained the corresponding atom.
fn ensure_captured_closure_variable(
    function: &mut FunctionIr,
    descriptor: ClosureVariable,
    requested_kind: BindingKind,
) -> Result<(u16, BindingKind), Error> {
    if let Some((index, candidate)) = function
        .closure_variables
        .iter_mut()
        .enumerate()
        .find(|(_, candidate)| candidate.source == descriptor.source)
    {
        let actual_kind = binding_kind_from_closure_descriptor(*candidate)?;
        let function_name_erasure = matches!(
            (actual_kind, requested_kind),
            (BindingKind::FunctionName { .. }, BindingKind::Normal)
                | (BindingKind::Normal, BindingKind::FunctionName { .. })
        );
        if actual_kind != requested_kind && !function_name_erasure {
            return Err(Error::internal(
                "closure storage source has conflicting binding metadata",
            ));
        }
        match (candidate.name, descriptor.name) {
            (ClosureVariableName::None, ClosureVariableName::Constant(_)) => {
                candidate.name = descriptor.name;
            }
            (ClosureVariableName::Constant(left), ClosureVariableName::Constant(right))
                if left != right =>
            {
                return Err(Error::internal(
                    "closure storage source has conflicting binding names",
                ));
            }
            _ => {}
        }
        let index = u16::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
        return Ok((index, actual_kind));
    }
    let index = push_closure_variable(function, descriptor)?;
    Ok((index, requested_kind))
}

fn binding_kind_from_closure_descriptor(descriptor: ClosureVariable) -> Result<BindingKind, Error> {
    match descriptor.kind {
        ClosureVariableKind::Normal if descriptor.is_lexical => Ok(BindingKind::Lexical {
            is_const: descriptor.is_const,
        }),
        ClosureVariableKind::Normal if !descriptor.is_const => Ok(BindingKind::Normal),
        ClosureVariableKind::FunctionName if !descriptor.is_lexical => {
            Ok(BindingKind::FunctionName {
                is_const: descriptor.is_const,
            })
        }
        ClosureVariableKind::EvalVariableObject
            if !descriptor.is_lexical && !descriptor.is_const =>
        {
            Ok(BindingKind::EvalVariableObject)
        }
        ClosureVariableKind::WithObject if !descriptor.is_lexical && !descriptor.is_const => {
            Ok(BindingKind::WithObject)
        }
        ClosureVariableKind::Normal
        | ClosureVariableKind::FunctionName
        | ClosureVariableKind::GlobalFunction
        | ClosureVariableKind::EvalVariableObject
        | ClosureVariableKind::WithObject => Err(Error::internal(
            "captured closure descriptor has inconsistent binding metadata",
        )),
    }
}

fn ensure_closure_variable(
    function: &mut FunctionIr,
    descriptor: ClosureVariable,
) -> Result<u16, Error> {
    if let Some((index, candidate)) = function
        .closure_variables
        .iter()
        .enumerate()
        .find(|(_, candidate)| same_closure_storage(candidate, &descriptor))
    {
        if *candidate != descriptor {
            return Err(Error::internal(
                "closure storage source has conflicting binding metadata",
            ));
        }
        return u16::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"));
    }
    push_closure_variable(function, descriptor)
}

fn push_closure_variable(
    function: &mut FunctionIr,
    descriptor: ClosureVariable,
) -> Result<u16, Error> {
    if function.closure_variables.len() >= MAX_LOCAL_VARIABLES {
        return Err(Error::new(
            ErrorKind::JsInternal,
            "too many closure variables",
        ));
    }
    let index = u16::try_from(function.closure_variables.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
    function.closure_variables.push(descriptor);
    Ok(index)
}

fn same_closure_storage(left: &ClosureVariable, right: &ClosureVariable) -> bool {
    match (left.source, right.source) {
        (ClosureSource::Global, ClosureSource::Global) => left.name == right.name,
        (ClosureSource::GlobalDeclaration, ClosureSource::GlobalDeclaration) => {
            left.name == right.name
        }
        (left, right) => left == right,
    }
}

#[derive(Debug, Default)]
struct ScopeLifecycle {
    tdz_locals: Vec<u16>,
    function_entries: Vec<ScopedFunctionEntry>,
    close_locals: Vec<u16>,
}

#[derive(Clone, Copy, Debug)]
struct ScopedFunctionEntry {
    constant: u32,
    local: u16,
}

fn captured_locals_by_function(functions: &[FunctionIr]) -> Result<Vec<Vec<bool>>, Error> {
    let mut captured = functions
        .iter()
        .map(|function| vec![false; function.locals.len()])
        .collect::<Vec<_>>();
    for (function_id, function) in functions.iter().enumerate().skip(1) {
        let parent = function
            .parent
            .ok_or_else(|| Error::internal("non-root function has no parent while lowering"))?
            .function;
        let parent_captured = captured
            .get_mut(parent)
            .ok_or_else(|| Error::internal("captured-local parent is out of bounds"))?;
        for descriptor in &function.closure_variables {
            let ClosureSource::ParentLocal(index) = descriptor.source else {
                continue;
            };
            let captured = parent_captured.get_mut(usize::from(index)).ok_or_else(|| {
                Error::internal("child closure captures an out-of-bounds parent local")
            })?;
            *captured = true;
        }
        if parent >= function_id {
            return Err(Error::internal(
                "function parent must precede its child while lowering",
            ));
        }
    }
    for (function_id, function) in functions.iter().enumerate() {
        let function_captured = captured
            .get_mut(function_id)
            .ok_or_else(|| Error::internal("eval captured-local function is out of bounds"))?;
        for binding in function
            .eval_environments
            .iter()
            .flat_map(|environment| environment.scopes.iter())
            .flat_map(|scope| scope.bindings.iter())
        {
            let EvalBindingSource::Local(index) = binding.source else {
                continue;
            };
            let captured = function_captured
                .get_mut(usize::from(index))
                .ok_or_else(|| Error::internal("eval captures an out-of-bounds local"))?;
            *captured = true;
        }
    }
    Ok(captured)
}

fn build_scope_lifecycles(
    function: &FunctionIr,
    captured_locals: &[bool],
) -> Result<Vec<ScopeLifecycle>, Error> {
    if captured_locals.len() != function.locals.len() {
        return Err(Error::internal(
            "captured-local metadata has the wrong length",
        ));
    }
    let mut scoped_constants = vec![None; function.bindings.len()];
    for scoped in &function.scoped_functions {
        let slot = scoped_constants
            .get_mut(scoped.binding.0)
            .ok_or_else(|| Error::internal("scoped function binding is out of bounds"))?;
        if slot.replace(scoped.constant).is_some() {
            return Err(Error::internal(
                "scoped function binding has more than one child record",
            ));
        }
    }
    function
        .scopes
        .iter()
        .map(|scope| {
            let mut lifecycle = ScopeLifecycle::default();
            // QuickJS links scope variables newest-first and expands both
            // enter and leave in that order after variable resolution.
            for &binding_id in scope.bindings.iter().rev() {
                let binding = function
                    .bindings
                    .get(binding_id.0)
                    .ok_or_else(|| Error::internal("scope binding is out of bounds"))?;
                let is_lexical = matches!(binding.kind, BindingKind::Lexical { .. });
                let is_with_object = binding.kind == BindingKind::WithObject;
                if !is_lexical && !is_with_object {
                    continue;
                }
                let index = match binding.storage {
                    BindingStorage::Local(index) => index,
                    BindingStorage::External(_) | BindingStorage::Global => continue,
                    BindingStorage::Argument(_) => {
                        return Err(Error::internal(
                            "scoped binding lifecycle referenced an argument",
                        ));
                    }
                };
                if is_lexical {
                    if let Some(constant) = scoped_constants[binding_id.0] {
                        lifecycle.function_entries.push(ScopedFunctionEntry {
                            constant,
                            local: index,
                        });
                    } else {
                        lifecycle.tdz_locals.push(index);
                    }
                }
                if captured_locals[usize::from(index)] {
                    lifecycle.close_locals.push(index);
                }
            }
            Ok(lifecycle)
        })
        .collect()
}

fn lower_detached_script(tree: FunctionTree) -> Result<BytecodeFunction, Error> {
    let mut functions = tree.functions;
    let function = functions
        .pop()
        .ok_or_else(|| Error::internal("compiler produced no script function"))?;
    if !function.closure_variables.is_empty() {
        return Err(Error::internal(
            "detached compiler cannot publish global-environment closure variables",
        ));
    }
    let captured_locals = vec![false; function.locals.len()];
    let scope_lifecycles = build_scope_lifecycles(&function, &captured_locals)?;
    let code = lower_ops(function.ops, &scope_lifecycles)?.code;
    let mut atom_strings = HashMap::<u32, Vec<JsString>>::new();
    let constants = function
        .constants
        .into_iter()
        .map(|constant| match constant {
            IrConstant::Primitive(value) => Ok(value),
            IrConstant::AtomString(value) => {
                if AtomTable::immediate_integer_atom(&value).is_some() {
                    Ok(Value::String(value))
                } else {
                    let strings = atom_strings.entry(value.content_hash()).or_default();
                    if let Some(canonical) = strings.iter().find(|string| *string == &value) {
                        Ok(Value::String(canonical.clone()))
                    } else {
                        strings.push(value.clone());
                        Ok(Value::String(value))
                    }
                }
            }
            IrConstant::RegExp { .. } => Err(Error::new(
                ErrorKind::Unsupported,
                "RegExp literals require runtime publication; use Context::compile or Context::eval",
            )),
            IrConstant::TemplateObject { .. } => Err(Error::new(
                ErrorKind::Unsupported,
                "tagged template objects require runtime publication; use Context::compile or Context::eval",
            )),
            IrConstant::Child(_) => Err(Error::internal(
                "detached compiler accepted a child-function constant",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let max_stack = verify_lowered_max_stack(&code, constants.len())?;
    let bytecode = BytecodeFunction {
        name: Some("<eval>".to_owned()),
        code,
        constants,
        local_count: u16::try_from(function.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?,
        max_stack,
    };
    bytecode.verify()?;
    Ok(bytecode)
}

fn lower_unlinked_tree(
    tree: FunctionTree,
    debug_info: DebugInfoMode,
) -> Result<UnlinkedFunction, Error> {
    let FunctionTree {
        functions: tree_functions,
        source,
        filename,
    } = tree;
    let function_count = tree_functions.len();
    let captured_locals = captured_locals_by_function(&tree_functions)?;
    // A descendant eval descriptor names bindings owned by each ancestor and
    // by every intervening closure relay. A synthetic eval tree also needs the
    // names on its external root and every child relay. Those names are
    // semantic linker metadata, not optional debug labels, so StripDebug must
    // retain them the way QuickJS retains vardef names on eval-visible chains.
    let synthetic_eval_tree = tree_functions
        .first()
        .is_some_and(|function| matches!(function.kind, FunctionKind::Eval(_)));
    let mut retains_eval_names = tree_functions
        .iter()
        .map(|function| synthetic_eval_tree || !function.eval_environments.is_empty())
        .collect::<Vec<_>>();
    for function_id in (1..function_count).rev() {
        if !retains_eval_names[function_id] {
            continue;
        }
        let parent = tree_functions[function_id]
            .parent
            .ok_or_else(|| Error::internal("eval-visible function has no parent"))?
            .function;
        let retained = retains_eval_names
            .get_mut(parent)
            .ok_or_else(|| Error::internal("eval-visible parent is out of bounds"))?;
        *retained = true;
    }
    let mut functions = tree_functions.into_iter().map(Some).collect::<Vec<_>>();
    let mut lowered = (0..function_count).map(|_| None).collect::<Vec<_>>();

    for function_id in (0..function_count).rev() {
        let mut function = functions[function_id]
            .take()
            .ok_or_else(|| Error::internal("function IR was lowered more than once"))?;
        let retain_eval_names = retains_eval_names[function_id];
        if debug_info == DebugInfoMode::StripDebug && !retain_eval_names {
            for descriptor in &mut function.closure_variables {
                if descriptor.is_lexical
                    && descriptor.kind == ClosureVariableKind::Normal
                    && !matches!(
                        descriptor.source,
                        ClosureSource::GlobalDeclaration
                            | ClosureSource::Global
                            | ClosureSource::ParentGlobal(_)
                    )
                {
                    descriptor.name = ClosureVariableName::None;
                }
            }
        }
        let argument_definitions = function
            .parameters
            .iter()
            .map(|name| {
                JsString::try_from_utf8(name)
                    .map(|name| UnlinkedVariableDefinition::ordinary(Some(name)))
                    .map_err(Error::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut local_definitions = (0..function.locals.len())
            .map(|_| UnlinkedVariableDefinition::ordinary(None))
            .collect::<Vec<_>>();
        for binding in &function.bindings {
            let BindingStorage::Local(index) = binding.storage else {
                continue;
            };
            let definition = local_definitions
                .get_mut(usize::from(index))
                .ok_or_else(|| Error::internal("local binding definition is out of bounds"))?;
            let name = if debug_info == DebugInfoMode::StripDebug
                && !retain_eval_names
                && matches!(binding.kind, BindingKind::Lexical { .. })
            {
                None
            } else {
                Some(JsString::try_from_utf8(&binding.name)?)
            };
            *definition = match binding.kind {
                BindingKind::Lexical { is_const } => {
                    UnlinkedVariableDefinition::lexical(name, is_const)
                }
                BindingKind::Normal | BindingKind::FunctionName { .. } => {
                    UnlinkedVariableDefinition::ordinary(name)
                }
                BindingKind::EvalVariableObject => UnlinkedVariableDefinition {
                    name: Some(JsString::from_static(EVAL_VARIABLE_OBJECT_LOCAL_NAME)),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::EvalVariableObject,
                },
                BindingKind::WithObject => UnlinkedVariableDefinition::with_object(),
            };
        }
        let scope_lifecycles = build_scope_lifecycles(
            &function,
            captured_locals
                .get(function_id)
                .ok_or_else(|| Error::internal("captured-local function is out of bounds"))?,
        )?;
        let lowered_ops = lower_ops(function.ops, &scope_lifecycles)?;
        let code = lowered_ops.code;
        let constant_count = function.constants.len();
        let constants = function
            .constants
            .into_iter()
            .map(|constant| match constant {
                IrConstant::AtomString(value) => Ok(UnlinkedConstant::atom_string(value)),
                IrConstant::Primitive(value) => unlinked_primitive(value),
                IrConstant::RegExp { pattern, program } => {
                    Ok(UnlinkedConstant::regexp(pattern, program))
                }
                IrConstant::TemplateObject { cooked, raw } => {
                    UnlinkedConstant::template_object(cooked, raw).map_err(|error| {
                        Error::internal(format!(
                            "compiler produced an invalid template object: {error}"
                        ))
                    })
                }
                IrConstant::Child(child) => lowered
                    .get_mut(child)
                    .and_then(Option::take)
                    .map(UnlinkedConstant::child)
                    .ok_or_else(|| Error::internal("child function was not lowered exactly once")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let max_stack = verify_lowered_max_stack(&code, constant_count)?;
        let metadata = FunctionMetadata {
            argument_count: u16::try_from(function.parameters.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?,
            defined_argument_count: u16::try_from(function.parameters.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?,
            local_count: u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?,
            function_name_local: function.function_name_local,
            eval_variable_object_local: function.eval_variable_object_local,
            needs_home_object: function.needs_home_object,
            closure_count: u16::try_from(function.closure_variables.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?,
            max_stack,
            strict: function.strict,
            super_call_allowed: function.super_call_allowed,
            super_allowed: function.super_allowed,
            eval_kind: match function.kind {
                FunctionKind::Eval(kind) => kind,
                FunctionKind::Script
                | FunctionKind::Ordinary
                | FunctionKind::Method
                | FunctionKind::Arrow => EvalKind::None,
            },
            function_kind: BytecodeFunctionKind::Normal,
            has_prototype: matches!(function.kind, FunctionKind::Ordinary),
            constructor_kind: if matches!(function.kind, FunctionKind::Ordinary) {
                ConstructorKind::Base
            } else {
                ConstructorKind::None
            },
        };
        let func_name = function
            .function_name
            .as_deref()
            .map(JsString::try_from_utf8)
            .transpose()?;
        let debug = match debug_info {
            DebugInfoMode::Full | DebugInfoMode::StripSource => Some(build_unlinked_debug(
                &source,
                filename.clone(),
                function.source.definition,
                if debug_info == DebugInfoMode::Full {
                    function.source.range
                } else {
                    None
                },
                &lowered_ops.pc_sites,
            )?),
            DebugInfoMode::StripDebug => None,
        };
        let unlinked = if function.closure_variables.is_empty() {
            UnlinkedFunction::new(code, constants, metadata)
        } else {
            UnlinkedFunction::new_with_closure_variables(
                code,
                constants,
                metadata,
                function.closure_variables,
            )
        };
        let unlinked = unlinked
            .with_eval_environments(function.eval_environments)
            .with_name(func_name)
            .with_variable_definitions(argument_definitions, local_definitions);
        lowered[function_id] = Some(match debug {
            Some(debug) => unlinked.with_debug(debug),
            None => unlinked,
        });
    }

    lowered[0]
        .take()
        .ok_or_else(|| Error::internal("root script was not lowered"))
}

fn verify_lowered_max_stack(code: &[Instruction], constant_count: usize) -> Result<u16, Error> {
    verify_parts(code, constant_count, MAX_BYTECODE_STACK as u16)
        .map(|verified| verified.max_stack)
        .map_err(|error| {
            if matches!(
                error.message(),
                "declared maximum stack is smaller than required"
                    | "bytecode stack exceeds u16::MAX"
            ) {
                Error::new(ErrorKind::JsInternal, "stack overflow")
            } else {
                error
            }
        })
}

struct LoweredOps {
    code: Vec<Instruction>,
    pc_sites: Vec<Option<SourceOffset>>,
}

fn resolved_operation_len(operation: &IrOp) -> Result<usize, Error> {
    match operation {
        IrOp::Bytecode(_) | IrOp::PushConstant(_) | IrOp::MakeClosure(_) => Ok(1),
        IrOp::GlobalSet(_) | IrOp::CapturedLexicalSet(_) => Ok(2),
        _ => Err(Error::internal(
            "dynamic identifier retained a non-resolved fallback",
        )),
    }
}

fn dynamic_identifier_len(
    access: IdentifierAccess,
    sources: &[DynamicEnvironmentSource],
    fallback: &IrOp,
) -> Result<usize, Error> {
    let action_len = match access {
        IdentifierAccess::Get
        | IdentifierAccess::GetOrUndefined
        | IdentifierAccess::Put
        | IdentifierAccess::Delete => 1_usize,
        IdentifierAccess::Set => 2,
        IdentifierAccess::Initialize | IdentifierAccess::AnnexBPut => {
            return Err(Error::internal(
                "declaration-only access reached dynamic identifier lowering",
            ));
        }
    };
    sources
        .len()
        .checked_mul(action_len.saturating_add(3))
        .and_then(|length| length.checked_add(resolved_operation_len(fallback).ok()?))
        .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))
}

fn dynamic_identifier_reference_len(
    access: IdentifierReferenceAccess,
    sources: &[DynamicEnvironmentSource],
    late_sources: &[DynamicEnvironmentSource],
    fallback: &IrOp,
    syntactic_with: bool,
    fallback_readonly: bool,
) -> Result<usize, Error> {
    let global_reference = global_reference_index(access, fallback);
    let fallback_access = match access {
        IdentifierReferenceAccess::Prepare | IdentifierReferenceAccess::Set => {
            IdentifierAccess::Set
        }
        IdentifierReferenceAccess::Get | IdentifierReferenceAccess::Call => IdentifierAccess::Get,
        IdentifierReferenceAccess::PostPut => IdentifierAccess::Put,
    };
    let fallback_len = if syntactic_with {
        resolved_operation_len(fallback)?
    } else {
        dynamic_identifier_len(fallback_access, late_sources, fallback)?
    };
    match access {
        IdentifierReferenceAccess::Prepare => sources
            .len()
            .checked_mul(4)
            .and_then(|length| {
                length.checked_add(if syntactic_with && fallback_readonly {
                    2
                } else {
                    1
                })
            })
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow")),
        IdentifierReferenceAccess::Get | IdentifierReferenceAccess::Call => sources
            .len()
            .checked_mul(5)
            .and_then(|length| length.checked_add(1))
            .and_then(|length| length.checked_add(fallback_len))
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow")),
        IdentifierReferenceAccess::Set | IdentifierReferenceAccess::PostPut
            if global_reference.is_some() =>
        {
            Ok(6)
        }
        IdentifierReferenceAccess::Set | IdentifierReferenceAccess::PostPut => 11_usize
            .checked_add(fallback_len)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow")),
    }
}

/// QuickJS keeps an unresolved global assignment as `OP_make_var_ref` when an
/// authored `with` can intercept the name.  The runtime operation snapshots
/// the global reference before the RHS, including TDZ/readonly checks and the
/// missing-global sentinel.  Calls deliberately keep the ordinary
/// `undefined, GetVar` shape because `OP_scope_get_ref` is not an lvalue.
fn global_reference_index(access: IdentifierReferenceAccess, fallback: &IrOp) -> Option<u16> {
    match (access, fallback) {
        (
            IdentifierReferenceAccess::Prepare | IdentifierReferenceAccess::Set,
            IrOp::GlobalSet(index),
        )
        | (IdentifierReferenceAccess::Get, IrOp::Bytecode(Instruction::GetVar(index)))
        | (IdentifierReferenceAccess::PostPut, IrOp::Bytecode(Instruction::PutVar(index))) => {
            Some(*index)
        }
        _ => None,
    }
}

fn emit_resolved_operation(
    operation: IrOp,
    site: Option<SourceOffset>,
    code: &mut Vec<Instruction>,
    pc_sites: &mut Vec<Option<SourceOffset>>,
) -> Result<(), Error> {
    match operation {
        IrOp::Bytecode(instruction) => {
            if matches!(
                instruction,
                Instruction::Goto(_)
                    | Instruction::IfFalse(_)
                    | Instruction::IfTrue(_)
                    | Instruction::Catch(_)
                    | Instruction::Gosub(_)
            ) {
                return Err(Error::internal(
                    "dynamic identifier fallback retained a control-flow edge",
                ));
            }
            code.push(instruction);
            pc_sites.push(site);
        }
        IrOp::PushConstant(index) => {
            code.push(Instruction::PushConst(index));
            pc_sites.push(site);
        }
        IrOp::MakeClosure(index) => {
            code.push(Instruction::FClosure(index));
            pc_sites.push(site);
        }
        IrOp::GlobalSet(index) => {
            code.push(Instruction::Dup);
            pc_sites.push(site);
            code.push(Instruction::PutVar(index));
            pc_sites.push(None);
        }
        IrOp::CapturedLexicalSet(index) => {
            code.push(Instruction::Dup);
            pc_sites.push(site);
            code.push(Instruction::PutVarRefCheck(index));
            pc_sites.push(None);
        }
        _ => {
            return Err(Error::internal(
                "dynamic identifier retained a non-resolved fallback",
            ));
        }
    }
    Ok(())
}

fn emit_dynamic_identifier_operation(
    name: u32,
    access: IdentifierAccess,
    sources: &[DynamicEnvironmentSource],
    fallback: IrOp,
    site: Option<SourceOffset>,
    code: &mut Vec<Instruction>,
    pc_sites: &mut Vec<Option<SourceOffset>>,
) -> Result<(), Error> {
    let emitted = dynamic_identifier_len(access, sources, &fallback)?;
    let end = code
        .len()
        .checked_add(emitted)
        .and_then(|target| u32::try_from(target).ok())
        .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    let action_len = if access == IdentifierAccess::Set {
        2_usize
    } else {
        1
    };
    let mut first = true;
    for &source in sources {
        let next = code
            .len()
            .checked_add(action_len.saturating_add(3))
            .and_then(|target| u32::try_from(target).ok())
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        code.push(Instruction::HasDynamicBinding { source, name });
        pc_sites.push(if first { site } else { None });
        first = false;
        code.push(Instruction::IfFalse(next));
        pc_sites.push(None);
        match access {
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined => {
                code.push(Instruction::GetDynamicBinding { source, name });
                pc_sites.push(None);
            }
            IdentifierAccess::Put => {
                code.push(Instruction::PutDynamicBinding { source, name });
                pc_sites.push(None);
            }
            IdentifierAccess::Set => {
                code.push(Instruction::Dup);
                pc_sites.push(None);
                code.push(Instruction::PutDynamicBinding { source, name });
                pc_sites.push(None);
            }
            IdentifierAccess::Delete => {
                code.push(Instruction::DeleteDynamicBinding { source, name });
                pc_sites.push(None);
            }
            IdentifierAccess::Initialize | IdentifierAccess::AnnexBPut => {
                return Err(Error::internal(
                    "declaration-only access reached dynamic identifier lowering",
                ));
            }
        }
        code.push(Instruction::Goto(end));
        pc_sites.push(None);
    }
    emit_resolved_operation(fallback, if first { site } else { None }, code, pc_sites)?;
    if u32::try_from(code.len()).ok() != Some(end) {
        return Err(Error::internal(
            "dynamic identifier lowering length changed",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_dynamic_identifier_reference(
    name: u32,
    access: IdentifierReferenceAccess,
    sources: &[DynamicEnvironmentSource],
    late_sources: &[DynamicEnvironmentSource],
    fallback: IrOp,
    syntactic_with: bool,
    fallback_readonly: bool,
    site: Option<SourceOffset>,
    code: &mut Vec<Instruction>,
    pc_sites: &mut Vec<Option<SourceOffset>>,
) -> Result<(), Error> {
    let emitted = dynamic_identifier_reference_len(
        access,
        sources,
        late_sources,
        &fallback,
        syntactic_with,
        fallback_readonly,
    )?;
    let start = code.len();
    let end = start
        .checked_add(emitted)
        .and_then(|target| u32::try_from(target).ok())
        .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    let global_reference = global_reference_index(access, &fallback);
    let mut first = true;

    match access {
        IdentifierReferenceAccess::Prepare
        | IdentifierReferenceAccess::Get
        | IdentifierReferenceAccess::Call => {
            let action_len = if access == IdentifierReferenceAccess::Prepare {
                2_usize
            } else {
                3
            };
            for &source in sources {
                let next = code
                    .len()
                    .checked_add(action_len.saturating_add(2))
                    .and_then(|target| u32::try_from(target).ok())
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
                code.push(Instruction::HasDynamicBinding { source, name });
                pc_sites.push(if first { site } else { None });
                first = false;
                code.push(Instruction::IfFalse(next));
                pc_sites.push(None);
                code.push(Instruction::DynamicEnvironmentObject(source));
                pc_sites.push(None);
                if matches!(
                    access,
                    IdentifierReferenceAccess::Get | IdentifierReferenceAccess::Call
                ) {
                    code.push(if access == IdentifierReferenceAccess::Call {
                        Instruction::GetRefValueUndef(name)
                    } else {
                        Instruction::GetRefValue(name)
                    });
                    pc_sites.push(None);
                }
                code.push(Instruction::Goto(end));
                pc_sites.push(None);
            }

            if access == IdentifierReferenceAccess::Prepare {
                if syntactic_with && fallback_readonly {
                    code.push(Instruction::Undefined);
                    pc_sites.push(if first { site } else { None });
                    code.push(Instruction::ThrowReadOnly(name));
                    pc_sites.push(None);
                } else if let Some(index) = global_reference {
                    code.push(Instruction::GlobalReference(index));
                    pc_sites.push(if first { site } else { None });
                } else {
                    code.push(Instruction::Undefined);
                    pc_sites.push(if first { site } else { None });
                }
            } else if access == IdentifierReferenceAccess::Get
                && syntactic_with
                && fallback_readonly
            {
                code.push(Instruction::Undefined);
                pc_sites.push(if first { site } else { None });
                code.push(Instruction::ThrowReadOnly(name));
                pc_sites.push(None);
            } else if access == IdentifierReferenceAccess::Get
                && let Some(index) = global_reference
            {
                code.push(Instruction::GlobalReference(index));
                pc_sites.push(if first { site } else { None });
                code.push(Instruction::GetRefValue(name));
                pc_sites.push(None);
            } else if syntactic_with {
                code.push(Instruction::Undefined);
                pc_sites.push(if first { site } else { None });
                emit_resolved_operation(fallback, None, code, pc_sites)?;
            } else {
                code.push(Instruction::Undefined);
                pc_sites.push(if first { site } else { None });
                emit_dynamic_identifier_operation(
                    name,
                    IdentifierAccess::Get,
                    late_sources,
                    fallback,
                    None,
                    code,
                    pc_sites,
                )?;
            }
        }
        IdentifierReferenceAccess::Set | IdentifierReferenceAccess::PostPut => {
            if access == IdentifierReferenceAccess::PostPut {
                code.push(Instruction::Perm3);
                pc_sites.push(site);
            } else {
                code.push(Instruction::Insert2);
                pc_sites.push(site);
                code.push(Instruction::Drop);
                pc_sites.push(None);
            }
            // Put the candidate base on top while retaining the result value
            // below it, then branch to the object-reference write when it is
            // not the static `undefined` sentinel.
            if access == IdentifierReferenceAccess::PostPut {
                code.push(Instruction::Insert2);
                pc_sites.push(None);
                code.push(Instruction::Drop);
                pc_sites.push(None);
            }
            if global_reference.is_some() {
                code.push(Instruction::Insert2);
                pc_sites.push(None);
                code.push(Instruction::Drop);
                pc_sites.push(None);
                if access == IdentifierReferenceAccess::Set {
                    code.push(Instruction::Insert2);
                    pc_sites.push(None);
                }
                code.push(Instruction::PutRefValue(name));
                pc_sites.push(None);
            } else {
                code.push(Instruction::Dup);
                pc_sites.push(None);
                code.push(Instruction::IsUndefinedOrNull);
                pc_sites.push(None);
                let dynamic_target_index = code.len();
                code.push(Instruction::IfFalse(u32::MAX));
                pc_sites.push(None);

                code.push(Instruction::Drop);
                pc_sites.push(None);
                if syntactic_with {
                    emit_resolved_operation(fallback, None, code, pc_sites)?;
                } else {
                    emit_dynamic_identifier_operation(
                        name,
                        if access == IdentifierReferenceAccess::Set {
                            IdentifierAccess::Set
                        } else {
                            IdentifierAccess::Put
                        },
                        late_sources,
                        fallback,
                        None,
                        code,
                        pc_sites,
                    )?;
                }
                code.push(Instruction::Goto(end));
                pc_sites.push(None);

                let dynamic_target = u32::try_from(code.len())
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
                let Some(Instruction::IfFalse(target)) = code.get_mut(dynamic_target_index) else {
                    return Err(Error::internal("reference branch instruction disappeared"));
                };
                *target = dynamic_target;
                code.push(Instruction::Insert2);
                pc_sites.push(None);
                code.push(Instruction::Drop);
                pc_sites.push(None);
                if access == IdentifierReferenceAccess::Set {
                    code.push(Instruction::Insert2);
                    pc_sites.push(None);
                }
                code.push(Instruction::PutRefValue(name));
                pc_sites.push(None);
            }
        }
    }
    if u32::try_from(code.len()).ok() != Some(end) {
        return Err(Error::internal(
            "dynamic identifier Reference lowering length changed",
        ));
    }
    Ok(())
}

fn lower_ops(operations: Vec<SpannedIrOp>, scopes: &[ScopeLifecycle]) -> Result<LoweredOps, Error> {
    let mut offsets = Vec::with_capacity(operations.len() + 1);
    let mut code_len = 0_usize;
    for operation in &operations {
        offsets.push(code_len);
        let emitted = match &operation.op {
            IrOp::EnterScope(scope) => scopes
                .get(scope.0)
                .ok_or_else(|| Error::internal("scope entry is out of bounds"))?
                .tdz_locals
                .len()
                .checked_add(
                    scopes
                        .get(scope.0)
                        .ok_or_else(|| Error::internal("scope entry is out of bounds"))?
                        .function_entries
                        .len()
                        .saturating_mul(2),
                )
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?,
            IrOp::LeaveScope(scope) => scopes
                .get(scope.0)
                .ok_or_else(|| Error::internal("scope exit is out of bounds"))?
                .close_locals
                .len(),
            IrOp::GlobalSet(_) | IrOp::CapturedLexicalSet(_) => 2,
            IrOp::DynamicIdentifier {
                access,
                sources,
                fallback,
                ..
            } => dynamic_identifier_len(*access, sources, fallback)?,
            IrOp::DynamicIdentifierReference {
                access,
                sources,
                late_sources,
                fallback,
                syntactic_with,
                fallback_readonly,
                ..
            } => dynamic_identifier_reference_len(
                *access,
                sources,
                late_sources,
                fallback,
                *syntactic_with,
                *fallback_readonly,
            )?,
            _ => 1,
        };
        code_len = code_len
            .checked_add(emitted)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    }
    offsets.push(code_len);

    let remap_target = |target: u32| -> Result<u32, Error> {
        let old = usize::try_from(target)
            .map_err(|_| Error::internal("jump target did not fit usize"))?;
        let new = offsets
            .get(old)
            .copied()
            .ok_or_else(|| Error::internal("jump target is out of bounds"))?;
        u32::try_from(new).map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))
    };

    let mut code = Vec::with_capacity(code_len);
    let mut pc_sites = Vec::with_capacity(code_len);
    for operation in operations {
        let SpannedIrOp { op, pc_site } = operation;
        match op {
            IrOp::EnterScope(scope) => {
                let lifecycle = scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("scope entry is out of bounds"))?;
                // Reset every ordinary lexical lifetime before any block
                // closure captures it. This is observationally equivalent to
                // QuickJS's mixed newest-first expansion, while preserving the
                // runtime invariant that an initialized captured cell cannot
                // silently begin a new lifetime without CloseLocal.
                for &index in &lifecycle.tdz_locals {
                    code.push(Instruction::SetLocalUninitialized(index));
                    pc_sites.push(None);
                }
                for entry in &lifecycle.function_entries {
                    code.push(Instruction::FClosure(entry.constant));
                    pc_sites.push(None);
                    code.push(Instruction::InitializeLocal(entry.local));
                    pc_sites.push(None);
                }
            }
            IrOp::LeaveScope(scope) => {
                for &index in &scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("scope exit is out of bounds"))?
                    .close_locals
                {
                    code.push(Instruction::CloseLocal(index));
                    pc_sites.push(None);
                }
            }
            IrOp::Bytecode(Instruction::Goto(target)) => {
                code.push(Instruction::Goto(remap_target(target)?));
                pc_sites.push(pc_site);
            }
            IrOp::Bytecode(Instruction::IfFalse(target)) => {
                code.push(Instruction::IfFalse(remap_target(target)?));
                pc_sites.push(pc_site);
            }
            IrOp::Bytecode(Instruction::IfTrue(target)) => {
                code.push(Instruction::IfTrue(remap_target(target)?));
                pc_sites.push(pc_site);
            }
            IrOp::Bytecode(Instruction::Catch(target)) => {
                code.push(Instruction::Catch(remap_target(target)?));
                pc_sites.push(pc_site);
            }
            IrOp::Bytecode(Instruction::Gosub(target)) => {
                code.push(Instruction::Gosub(remap_target(target)?));
                pc_sites.push(pc_site);
            }
            IrOp::Bytecode(instruction) => {
                code.push(instruction);
                pc_sites.push(pc_site);
            }
            IrOp::TemplateCall {
                argument_count,
                method,
            } => {
                // QuickJS `emit_u16` writes the low operand bits even when an
                // unreachable template has more than 65,535 arguments. The
                // reachability verifier still observes every push on a live
                // path and rejects its stack before this truncated call can
                // execute.
                code.push(if method {
                    Instruction::CallMethod(argument_count as u16)
                } else {
                    Instruction::Call(argument_count as u16)
                });
                pc_sites.push(pc_site);
            }
            IrOp::EvalCall {
                argument_count,
                scope,
                environment,
            } => {
                // Retain the parser scope as an IR invariant until lowering,
                // but publish only its immutable linked descriptor ordinal.
                scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("eval call scope is out of bounds"))?;
                let environment = environment
                    .ok_or_else(|| Error::internal("eval call has no linked environment"))?;
                code.push(Instruction::Eval {
                    argument_count,
                    environment,
                });
                pc_sites.push(pc_site);
            }
            IrOp::PushConstant(index) => {
                code.push(Instruction::PushConst(index));
                pc_sites.push(pc_site);
            }
            IrOp::MakeClosure(index) => {
                code.push(Instruction::FClosure(index));
                pc_sites.push(pc_site);
            }
            IrOp::GlobalSet(index) => {
                code.push(Instruction::Dup);
                pc_sites.push(pc_site);
                code.push(Instruction::PutVar(index));
                pc_sites.push(None);
            }
            IrOp::CapturedLexicalSet(index) => {
                code.push(Instruction::Dup);
                pc_sites.push(pc_site);
                code.push(Instruction::PutVarRefCheck(index));
                pc_sites.push(None);
            }
            IrOp::DynamicIdentifier {
                name,
                access,
                sources,
                fallback,
            } => {
                emit_dynamic_identifier_operation(
                    name,
                    access,
                    &sources,
                    *fallback,
                    pc_site,
                    &mut code,
                    &mut pc_sites,
                )?;
            }
            IrOp::DynamicIdentifierReference {
                name,
                access,
                sources,
                late_sources,
                fallback,
                syntactic_with,
                fallback_readonly,
            } => {
                emit_dynamic_identifier_reference(
                    name,
                    access,
                    &sources,
                    &late_sources,
                    *fallback,
                    syntactic_with,
                    fallback_readonly,
                    pc_site,
                    &mut code,
                    &mut pc_sites,
                )?;
            }
            IrOp::Identifier { .. } | IrOp::IdentifierReference { .. } => {
                return Err(Error::internal(
                    "identifier reached bytecode lowering before resolution",
                ));
            }
        }
    }
    apply_quickjs_late_throw_sites(&code, &mut pc_sites)?;
    fold_quickjs_constant_branches(&mut code);
    Ok(LoweredOps { code, pc_sites })
}

/// QuickJS `resolve_labels` folds this deliberately narrow constant set before
/// `compute_stack_size`. Keep instruction slots stable with Nops so existing
/// IR-index jump remapping and debug PCs remain valid.
fn fold_quickjs_constant_branches(code: &mut [Instruction]) {
    let mut targeted = vec![false; code.len()];
    for instruction in code.iter() {
        if let Instruction::Goto(target)
        | Instruction::IfFalse(target)
        | Instruction::IfTrue(target)
        | Instruction::Catch(target)
        | Instruction::Gosub(target) = instruction
            && let Ok(target) = usize::try_from(*target)
            && let Some(targeted) = targeted.get_mut(target)
        {
            *targeted = true;
        }
    }

    for pc in 0..code.len().saturating_sub(1) {
        // A hostile or hand-built control-flow edge may enter the conditional
        // without executing its adjacent constant. Compiler-generated QuickJS
        // patterns never do, but skipping preserves the verifier trust boundary.
        if targeted[pc + 1] {
            continue;
        }
        let truthy = match code[pc] {
            Instruction::Undefined | Instruction::Null | Instruction::PushFalse => false,
            Instruction::PushTrue => true,
            Instruction::PushI32(value) => value != 0,
            _ => continue,
        };
        let (branch_on_true, target) = match code[pc + 1] {
            Instruction::IfFalse(target) => (false, target),
            Instruction::IfTrue(target) => (true, target),
            _ => continue,
        };
        code[pc] = if truthy == branch_on_true {
            Instruction::Goto(target)
        } else {
            Instruction::Nop
        };
        code[pc + 1] = Instruction::Nop;
    }
}

fn build_unlinked_debug(
    source: &str,
    filename: JsString,
    definition: SourceOffset,
    source_range: Option<Range<SourceOffset>>,
    pc_sites: &[Option<SourceOffset>],
) -> Result<UnlinkedFunctionDebug, Error> {
    let locator = QuickJsSourceLocator::new(source);
    let definition = locator
        .locate(definition)
        .map_err(|error| Error::internal(error.to_string()))?;
    let mut entries = Vec::new();
    let mut previous_position = Some(definition);
    for (pc, site) in pc_sites.iter().copied().enumerate() {
        let Some(site) = site else {
            continue;
        };
        let position = locator
            .locate(site)
            .map_err(|error| Error::internal(error.to_string()))?;
        if previous_position == Some(position) {
            continue;
        }
        entries.push(Pc2LineEntry {
            pc: u32::try_from(pc)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?,
            position,
        });
        previous_position = Some(position);
    }

    let source = source_range
        .map(|range| {
            let start = range.start.as_usize();
            let end = range.end.as_usize();
            if start > end
                || end > source.len()
                || !source.is_char_boundary(start)
                || !source.is_char_boundary(end)
            {
                return Err(Error::internal("function source range is invalid"));
            }
            Ok(source.as_bytes()[start..end].to_vec().into_boxed_slice())
        })
        .transpose()?;

    Ok(UnlinkedFunctionDebug {
        filename,
        pc2line: Some(Pc2LineTable::new(definition, entries)),
        source,
    })
}

fn unlinked_primitive(value: Value) -> Result<UnlinkedConstant, Error> {
    UnlinkedConstant::primitive(value).map_err(|error| {
        Error::internal(format!(
            "compiler emitted a runtime-bound constant into an unlinked function: {error}"
        ))
    })
}

fn parse_number(number: &crate::lexer::NumberLiteral<'_>) -> Result<Value, String> {
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
