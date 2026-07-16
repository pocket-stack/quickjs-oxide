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
    ArgumentsKind, BytecodeFunction, Instruction, MAX_LOCAL_SLOTS, verify_parts,
};
use crate::debug::{DebugInfoMode, Pc2LineEntry, Pc2LineTable, QuickJsSourceLocator, SourceOffset};
use crate::error::{Error, ErrorKind, NativeErrorMessage, SourceLocation, SourceSpan};
use crate::function::{
    UnlinkedConstant, UnlinkedFunction, UnlinkedFunctionDebug, UnlinkedVariableDefinition,
};
use crate::heap::{
    ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName, ConstructorKind,
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
// A finally clause in script code must preserve the incoming completion value
// when it terminates normally. Keep those implementation-only save slots in
// the same explicit metadata domain as `<ret>` rather than letting an unbound
// ordinary local silently escape the scope-graph trust boundary.
const FINALLY_EVAL_RET_LOCAL_NAME: &str = "<finally-ret>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FunctionKind {
    Script,
    Ordinary,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingStorage {
    Argument(u16),
    Local(u16),
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

#[derive(Clone, Copy, Debug)]
struct IrScopedFunction {
    binding: BindingId,
    constant: u32,
    annex_binding: Option<BindingId>,
    authored_closure: usize,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MemberReference {
    Field { key: u32, site: SourceOffset },
    Computed { site: SourceOffset },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IdentifierReference {
    name: String,
    span: Span,
    scope: ScopeId,
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
    TemplateCall(usize),
    PushConstant(u32),
    MakeClosure(u32),
    /// Lowering-only assignment-expression form. QuickJS has no `set_var`;
    /// this expands to `dup; put_var` before verification/publication.
    GlobalSet(u16),
    /// QuickJS has no value-preserving checked VarRef write. Keep the typed
    /// operation unresolved until lowering expands it to `dup; put_var_ref_check`.
    CapturedLexicalSet(u16),
    Identifier {
        name: String,
        span: Span,
        scope: ScopeId,
        access: IdentifierAccess,
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
            Self::TemplateCall(argument_count) => (argument_count + 2, 1),
            Self::PushConstant(_) | Self::MakeClosure(_) => (0, 1),
            Self::GlobalSet(_) | Self::CapturedLexicalSet(_) => (1, 1),
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
    parameters: Vec<String>,
    locals: Vec<String>,
    scopes: Vec<IrScope>,
    bindings: Vec<IrBinding>,
    global_declarations: Vec<IrGlobalDeclaration>,
    /// Last direct function declaration attached to each ordinary
    /// function-scoped argument/local binding.
    hoisted_functions: Vec<IrHoistedFunction>,
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

impl FunctionIr {
    fn new(
        parent: Option<ParentLink>,
        kind: FunctionKind,
        source: FunctionSourceInfo,
        function_name: Option<String>,
        private_name_binding: bool,
        parameters: Vec<String>,
        strict: bool,
    ) -> Result<Self, Error> {
        let (locals, eval_ret_local, synthetic_locals) = if matches!(kind, FunctionKind::Script) {
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
                kind: if matches!(kind, FunctionKind::Script) {
                    ScopeKind::ProgramBody
                } else {
                    ScopeKind::FunctionBody
                },
                bindings: Vec::new(),
            },
        ];
        let current_scope = body;
        let var_scope = function_root;
        let ops = if matches!(kind, FunctionKind::Ordinary) {
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
            source,
            function_name,
            private_name_binding,
            function_name_local: None,
            arguments_local: None,
            parameters,
            locals,
            scopes,
            bindings: Vec::new(),
            global_declarations: Vec::new(),
            hoisted_functions: Vec::new(),
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
            break_controls: Vec::new(),
            stack_depth: 0,
            strict,
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

#[derive(Debug)]
struct FunctionTree {
    functions: Vec<FunctionIr>,
    source: Box<str>,
    filename: JsString,
}

struct ParsedFunctionDefinition {
    constant: u32,
    child: FunctionId,
    name: Option<(String, Span)>,
}

struct FunctionDefinitionHeader<'source> {
    span: Span,
    name: Option<(Identifier<'source>, Span)>,
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

impl<'source> Parser<'source> {
    fn parse(source: &'source str, filename: JsString) -> Result<FunctionTree, Error> {
        if source.len() > i32::MAX as usize {
            return Err(Error::new(
                ErrorKind::JsInternal,
                "source is too large for QuickJS debug metadata",
            ));
        }
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
                FunctionKind::Script,
                FunctionSourceInfo {
                    span: source_span,
                    definition: SourceOffset::try_from_usize(0)
                        .map_err(|error| Error::internal(error.to_string()))?,
                    range: None,
                },
                Some("<eval>".to_owned()),
                false,
                Vec::new(),
                false,
            )?],
        };
        let strict = parser.directive_prologue_has_use_strict(0, false)?;
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

        // QuickJS ends ordinary function bytecode with `return_undef`. It may
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
            TokenKind::Keyword(Keyword::With) => {
                Err(self.unsupported_here("with statements are not implemented yet"))
            }
            TokenKind::Keyword(Keyword::Break) => self.parse_loop_jump_statement(false),
            TokenKind::Keyword(Keyword::Continue) => self.parse_loop_jump_statement(true),
            TokenKind::Keyword(Keyword::Function) => {
                if matches!(self.current_ir().kind, FunctionKind::Script)
                    && position == StatementPosition::ProgramBody
                {
                    self.parse_program_function_declaration()
                } else if matches!(self.current_ir().kind, FunctionKind::Ordinary)
                    && position == StatementPosition::FunctionBody
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
                if matches!(self.current_ir().kind, FunctionKind::Script) {
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
                || self.current_ir().strict)
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
                "{loop_name} destructuring bindings are not implemented yet"
            )));
        }

        if self.lexical_declaration_ahead(true)? {
            let is_const = matches!(self.current().kind, TokenKind::Keyword(Keyword::Const));
            self.advance()?;
            if matches!(
                self.current().kind,
                TokenKind::Punctuator(Punctuator::LeftBrace | Punctuator::LeftBracket)
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
            });
        }

        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Var)) {
            self.advance()?;
            if matches!(
                self.current().kind,
                TokenKind::Punctuator(Punctuator::LeftBrace | Punctuator::LeftBracket)
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
            self.emit_identifier_inherited(
                target.name,
                target.span,
                target.scope,
                IdentifierAccess::Put,
            )?;
            return Ok(ForAssignmentTargetInfo {
                declaration: ForAssignmentDeclaration::Assignment,
                var_initializer: None,
            });
        }
        let Some(target) = self.take_tail_member_reference()? else {
            return Err(self.syntax_here("invalid assignment left-hand side"));
        };
        self.emit_for_of_member_put(target)?;
        Ok(ForAssignmentTargetInfo {
            declaration: ForAssignmentDeclaration::Assignment,
            var_initializer: None,
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
        if self.current().line_terminator_before
            && matches!(self.current().kind, TokenKind::Template(_))
        {
            return Err(
                self.unsupported_here("tagged-template continuations are not implemented yet")
            );
        }
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
                    | IrOp::TemplateCall(_),
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
            if matches!(
                self.current().kind,
                TokenKind::Punctuator(Punctuator::LeftBrace | Punctuator::LeftBracket)
            ) {
                return Err(
                    self.unsupported_here("lexical destructuring bindings are not implemented yet")
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

            let initializer_site = if self.consume_punctuator(Punctuator::Equal)? {
                let site = source_offset(self.tokens[self.cursor - 1].span)?;
                self.parse_assignment()?;
                if self.anonymous_function_definition.take().is_some() {
                    let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                        JsString::try_from_utf8(&name)?,
                    )))?;
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
                self.parse_assignment()?;
                if self.anonymous_function_definition.take().is_some() {
                    // QuickJS emits a dummy OP_set_name after an anonymous
                    // closure and rewrites its atom when NamedEvaluation
                    // applies to this initializer. Keep that contextual name
                    // separate from the child bytecode's intrinsic func_name.
                    let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                        JsString::try_from_utf8(&name)?,
                    )))?;
                    self.emit_instruction(Instruction::SetName(name_constant))?;
                }
                self.emit_identifier_at(
                    name,
                    token.span,
                    IdentifierAccess::Put,
                    source_offset(initializer_span)?,
                )?;
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
        let function = &mut self.functions[self.current_function];
        let selects_arguments_object = matches!(function.kind, FunctionKind::Ordinary)
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
        let supported_scope = is_global
            || matches!(
                scope_kind,
                ScopeKind::Block
                    | ScopeKind::If
                    | ScopeKind::For
                    | ScopeKind::Switch
                    | ScopeKind::Catch
            )
            || (matches!(scope_kind, ScopeKind::FunctionBody)
                && matches!(function.kind, FunctionKind::Ordinary)
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
                (BindingStorage::Local(_), BindingKind::Lexical { .. }) => {
                    return Err(Error::internal(
                        "lexical binding leaked into the function var scope",
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
        }
        Ok(())
    }

    /// Parse assignment targets through typed unresolved References. Keeping
    /// identifier writes unresolved lets the late resolver select argument,
    /// local, closure, global and private-function-name behavior after the
    /// complete nested scope tree is known.
    fn parse_assignment(&mut self) -> Result<(), Error> {
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
            if let Some(target) = self.promote_tail_identifier_get()? {
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
            if let Some(target) = self.promote_tail_identifier_get()? {
                self.advance()?;
                self.validate_identifier_assignment_target(&target)?;
                self.parse_assignment()?;
                self.emit_instruction_at(operation, source_offset(assignment_span)?)?;
                self.anonymous_function_definition = None;
                self.emit_identifier_inherited(
                    target.name,
                    target.span,
                    target.scope,
                    IdentifierAccess::Set,
                )?;
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
            self.emit_identifier_inherited(
                target.name,
                target.span,
                target.scope,
                IdentifierAccess::Set,
            )?;
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
            MemberReference::Field { site, .. } | MemberReference::Computed { site } => site,
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
        self.emit_identifier_inherited(
            target.name,
            target.span,
            target.scope,
            IdentifierAccess::Set,
        )?;
        let end = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let joined_depth = self.current_ir().stack_depth;
        if short_circuit_depth != joined_depth {
            return Err(Error::internal(
                "identifier logical assignment branches have unequal stack depth",
            ));
        }

        let target = self.current_ir().ops.len();
        self.patch_jump(short_circuit, target)?;
        self.patch_jump(end, target)?;
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
                let is_method = self.promote_last_member_get_for_call()?;
                self.advance()?;
                let argument_count = self.parse_call_arguments()?;
                let instruction = if is_method {
                    Instruction::CallMethod(argument_count)
                } else {
                    Instruction::Call(argument_count)
                };
                self.emit_instruction_at(instruction, source_offset(call_span)?)?;
                self.anonymous_function_definition = None;
                continue;
            }
            if matches!(self.current().kind, TokenKind::Template(_)) {
                return Err(
                    self.unsupported_here("tagged template literals are not implemented yet")
                );
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

        if let Some(target) = self.promote_tail_identifier_get()? {
            self.validate_identifier_assignment_target(&target)?;
            self.emit_instruction_at(operation, source_offset(operator_span)?)?;
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
    fn promote_tail_identifier_get(&mut self) -> Result<Option<IdentifierReference>, Error> {
        let function = self.current_ir_mut();
        if function.last_identifier_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        function.last_identifier_reference = None;
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
        Ok(Some(IdentifierReference {
            name: name.clone(),
            span: *span,
            scope: *scope,
        }))
    }

    /// Remove the final identifier getter for ordinary `=` while retaining
    /// its unresolved binding identity for the later Set operation.
    fn take_tail_identifier_reference(&mut self) -> Result<Option<IdentifierReference>, Error> {
        let function = self.current_ir_mut();
        if function.last_identifier_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        function.last_identifier_reference = None;
        let operation = function
            .ops
            .pop()
            .ok_or_else(|| Error::internal("identifier Reference operation disappeared"))?;
        let IrOp::Identifier {
            name,
            span,
            scope,
            access: IdentifierAccess::Get,
        } = operation.op
        else {
            return Err(Error::internal(
                "identifier Reference marker did not point to a getter",
            ));
        };
        function.stack_depth = function
            .stack_depth
            .checked_sub(1)
            .ok_or_else(|| Error::internal("identifier lvalue removal underflowed the stack"))?;
        Ok(Some(IdentifierReference { name, span, scope }))
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
            _ => Err(Error::internal(
                "member Reference marker did not point to a getter",
            )),
        }
    }

    /// Keep both the lvalue operands and the old value for compound
    /// assignment. The computed form also retains the already-converted key,
    /// exactly matching QuickJS `get_array_el3`.
    fn promote_tail_member_get_for_compound(&mut self) -> Result<Option<MemberReference>, Error> {
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
            if matches!(self.current_ir().kind, FunctionKind::Script) {
                return Err(Error::syntax(
                    "new.target only allowed within functions",
                    source_span(new_span),
                ));
            }
            self.advance()?;
            self.emit_instruction(Instruction::PushNewTarget)?;
            self.anonymous_function_definition = None;
            return Ok(());
        }

        // QuickJS parses the constructor head with calls disabled but member
        // suffixes enabled. The following `(` therefore belongs to this `new`,
        // while calls after the completed construction remain postfix calls.
        self.parse_primary()?;
        while self.parse_member_suffix()? {}
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
                self.emit_instruction(Instruction::PushThis)?;
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

    /// Lower the data-property portion of QuickJS
    /// `js_parse_object_literal`. The fresh Object stays below every property
    /// operation. Fixed names reuse `DefineField`; computed names are
    /// canonicalized before their RHS and use `DefineArrayEl` followed by the
    /// same key drop as upstream. Method/accessor syntax remains an explicit
    /// parser frontier until its home-object and descriptor lowering lands.
    fn parse_object_literal(&mut self) -> Result<(), Error> {
        if !self.is_punctuator(Punctuator::LeftBrace) {
            return Err(self.syntax_here("expecting '{'"));
        }
        self.advance()?;
        self.emit_instruction(Instruction::Object)?;
        let mut has_proto = false;

        while !self.is_punctuator(Punctuator::RightBrace) {
            if self.is_punctuator(Punctuator::Ellipsis) {
                let spread_span = self.current().span;
                self.advance_expression_start()?;
                self.parse_assignment_allow_in()?;
                self.emit_instruction_at(
                    Instruction::CopyDataProperties,
                    source_offset(spread_span)?,
                )?;
                self.anonymous_function_definition = None;
            } else if self.is_punctuator(Punctuator::Multiply) {
                return Err(self
                    .unsupported_here("object literal generator methods are not implemented yet"));
            } else if self.is_punctuator(Punctuator::LeftBracket) {
                let property_span = self.current().span;
                self.advance_expression_start()?;
                self.parse_assignment_allow_in()?;
                // QuickJS performs ToPropertyKey before evaluating the value.
                self.emit_instruction(Instruction::ToPropKey)?;
                self.expect_punctuator(Punctuator::RightBracket)?;
                if self.is_punctuator(Punctuator::LeftParen) {
                    return Err(Error::unsupported(
                        "computed object literal methods are not implemented yet",
                        source_span(property_span),
                    ));
                }
                if !self.is_punctuator(Punctuator::Colon) {
                    return Err(self.syntax_here("expecting ':'"));
                }
                self.advance_expression_start()?;
                self.parse_assignment_allow_in()?;
                if self.anonymous_function_definition.take().is_some() {
                    self.emit_instruction(Instruction::SetNameComputed)?;
                }
                self.emit_instruction(Instruction::DefineArrayEl)?;
                self.emit_instruction(Instruction::Drop)?;
            } else {
                let token = self.current().clone();
                let mut shorthand = None;
                let mut method_prefix = None;
                let key = match token.kind {
                    TokenKind::Identifier(identifier) => {
                        let name = identifier.value.clone();
                        shorthand = Some(identifier);
                        if matches!(name.as_str(), "get" | "set" | "async") {
                            method_prefix = Some(name.clone());
                        }
                        self.advance()?;
                        JsString::try_from_utf8(&name)?
                    }
                    TokenKind::Keyword(keyword) => {
                        self.advance()?;
                        JsString::from_static(keyword.as_str())
                    }
                    TokenKind::String(string) => {
                        if self.current_ir().strict && string.has_legacy_octal_escape {
                            return Err(Error::syntax(
                                "legacy octal escapes are forbidden in strict mode",
                                source_span(token.span),
                            ));
                        }
                        self.advance()?;
                        JsString::try_from_utf16(string.value.utf16)?
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
                        parse_number(&number)
                            .map_err(|message| Error::syntax(message, source_span(token.span)))?
                            .to_js_string()?
                    }
                    TokenKind::PrivateIdentifier(_) => {
                        return Err(Error::syntax(
                            "private identifiers are not valid in object literals",
                            source_span(token.span),
                        ));
                    }
                    _ => return Err(self.syntax_here("invalid property name")),
                };

                let next_starts_property_name = matches!(
                    self.current().kind,
                    TokenKind::Identifier(_)
                        | TokenKind::Keyword(_)
                        | TokenKind::String(_)
                        | TokenKind::Number(_)
                        | TokenKind::Punctuator(Punctuator::LeftBracket)
                );
                let is_method_prefix = method_prefix.as_deref().is_some_and(|prefix| {
                    (next_starts_property_name
                        || (prefix == "async" && self.is_punctuator(Punctuator::Multiply)))
                        && (prefix != "async" || !self.current().line_terminator_before)
                });
                if self.is_punctuator(Punctuator::LeftParen) || is_method_prefix {
                    return Err(Error::unsupported(
                        "object literal methods and accessors are not implemented yet",
                        source_span(token.span),
                    ));
                }

                if self.is_punctuator(Punctuator::Colon) {
                    self.advance_expression_start()?;
                    self.parse_assignment_allow_in()?;
                    if key == JsString::from_static("__proto__") {
                        if has_proto {
                            return Err(Error::syntax(
                                "duplicate __proto__ property name",
                                source_span(token.span),
                            ));
                        }
                        has_proto = true;
                        self.anonymous_function_definition = None;
                        self.emit_instruction(Instruction::SetProto)?;
                    } else {
                        let key_constant =
                            self.add_constant(IrConstant::Primitive(Value::String(key)))?;
                        if self.anonymous_function_definition.take().is_some() {
                            self.emit_instruction(Instruction::SetName(key_constant))?;
                        }
                        self.emit_instruction(Instruction::DefineField(key_constant))?;
                    }
                } else if let Some(identifier) = shorthand {
                    validate_identifier(
                        &identifier,
                        token.span,
                        self.current_ir().strict,
                        IdentifierContext::Reference,
                    )?;
                    self.emit_identifier(identifier.value, token.span, IdentifierAccess::Get)?;
                    let key_constant =
                        self.add_constant(IrConstant::Primitive(Value::String(key)))?;
                    self.emit_instruction(Instruction::DefineField(key_constant))?;
                    self.anonymous_function_definition = None;
                } else {
                    return Err(self.syntax_here("expecting ':'"));
                }
            }

            if !self.is_punctuator(Punctuator::Comma) {
                break;
            }
            self.advance()?;
        }
        self.expect_punctuator(Punctuator::RightBrace)?;
        self.anonymous_function_definition = None;
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

    /// Lower an untagged template exactly like QuickJS `js_parse_template`:
    /// the first cooked segment becomes the receiver for one observable
    /// `String.prototype.concat` lookup, substitutions are full comma
    /// expressions, and only non-empty later cooked segments become call
    /// arguments. Tagged templates require the separate template-object cache.
    fn parse_template_literal(&mut self) -> Result<(), Error> {
        let mut depth = 0_usize;

        loop {
            let token = self.current().clone();
            let TokenKind::Template(part) = token.kind else {
                return Err(Error::internal(
                    "template parser lost its continuation token",
                ));
            };
            let kind = part.kind;
            let invalid_span = part.invalid_escape.as_ref().map(|error| error.span);
            let Some(cooked) = part.cooked else {
                return Err(Error::syntax(
                    "malformed escape sequence in string literal",
                    source_span(invalid_span.unwrap_or(token.span)),
                ));
            };

            if !cooked.utf16.is_empty() || depth == 0 {
                self.emit_atom_string(JsString::try_from_utf16(cooked.utf16)?)?;
                if depth == 0 {
                    if kind == TemplatePartKind::NoSubstitution {
                        self.advance()?;
                        self.anonymous_function_definition = None;
                        return Ok(());
                    }
                    let concat = self.add_constant(IrConstant::Primitive(Value::String(
                        JsString::from_static("concat"),
                    )))?;
                    // `js_parse_template` emits no source marker for either
                    // synthetic concat operation. Inherit the surrounding
                    // expression marker, just as ordinary QuickJS bytecode.
                    self.emit_instruction(Instruction::GetField2(concat))?;
                }
                depth += 1;
            }

            if kind == TemplatePartKind::Tail {
                let argument_count = depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::internal("template receiver disappeared"))?;
                // `js_parse_template` emits no source marker for the final
                // call.  Preserve the last substitution marker so failures
                // during concat/coercion point into that expression. Keep the
                // full count in IR so a reached later syntax error still wins
                // over deferred JS_STACK_SIZE_MAX validation.
                self.emit(IrOp::TemplateCall(argument_count))?;
                self.advance()?;
                self.current_ir_mut().last_member_reference = None;
                self.current_ir_mut().last_identifier_reference = None;
                self.anonymous_function_definition = None;
                return Ok(());
            }
            if !matches!(kind, TemplatePartKind::Head | TemplatePartKind::Middle) {
                return Err(Error::internal("invalid template-part transition"));
            }

            self.advance()?;
            self.parse_expression()?;
            depth += 1;
            if !self.is_punctuator(Punctuator::RightBrace) {
                return Err(self.syntax_here("expected '}' after template expression"));
            }
            self.advance_with_goal(LexicalGoal::TemplateContinuation)?;
        }
    }

    fn parse_function_expression(&mut self) -> Result<(), Error> {
        let parsed = self.parse_function_definition(false, true)?;
        self.emit(IrOp::MakeClosure(parsed.constant))?;
        self.anonymous_function_definition = parsed.name.is_none().then_some(parsed.child);
        Ok(())
    }

    /// Parse the common ordinary-function grammar and publish its child
    /// constant in the defining function. The caller decides whether that
    /// constant is evaluated in expression position or recorded for Program
    /// declaration hoisting.
    fn parse_function_definition(
        &mut self,
        require_name: bool,
        private_name_binding: bool,
    ) -> Result<ParsedFunctionDefinition, Error> {
        let header = self.parse_function_definition_header(require_name)?;
        self.parse_function_definition_tail(header, private_name_binding)
    }

    fn parse_function_definition_header(
        &mut self,
        require_name: bool,
    ) -> Result<FunctionDefinitionHeader<'source>, Error> {
        let span = self.current().span;
        self.advance()?;
        let name = if let TokenKind::Identifier(identifier) = self.current().kind.clone() {
            let span = self.current().span;
            validate_identifier(&identifier, span, false, IdentifierContext::FunctionName)?;
            self.advance()?;
            Some((identifier, span))
        } else {
            None
        };
        if require_name && name.is_none() {
            return Err(self.syntax_here("function name expected"));
        }
        Ok(FunctionDefinitionHeader { span, name })
    }

    fn parse_function_definition_tail(
        &mut self,
        header: FunctionDefinitionHeader<'source>,
        private_name_binding: bool,
    ) -> Result<ParsedFunctionDefinition, Error> {
        let FunctionDefinitionHeader {
            span: function_span,
            name: function_name_token,
        } = header;
        self.expect_punctuator(Punctuator::LeftParen)?;

        let mut parameters = Vec::new();
        let mut parameter_tokens = Vec::new();
        if !self.consume_punctuator(Punctuator::RightParen)? {
            loop {
                let token = self.current().clone();
                let TokenKind::Identifier(identifier) = token.kind else {
                    return Err(self.syntax_here("missing formal parameter"));
                };
                validate_identifier(&identifier, token.span, false, IdentifierContext::Argument)?;
                parameter_tokens.push((identifier.clone(), token.span));
                parameters.push(identifier.value);
                if parameters.len() > MAX_LOCAL_VARIABLES {
                    return Err(Error::new(ErrorKind::JsInternal, "too many arguments")
                        .with_span(source_span(token.span)));
                }
                self.advance()?;
                if !self.consume_punctuator(Punctuator::Comma)? {
                    if !self.consume_punctuator(Punctuator::RightParen)? {
                        return Err(Error::syntax(
                            "expecting ','",
                            source_span(self.current().span),
                        ));
                    }
                    break;
                }
                if self.is_punctuator(Punctuator::RightParen) {
                    return Err(self.unsupported_here(
                        "a trailing comma in this simple parameter list is not implemented yet",
                    ));
                }
            }
        }
        self.expect_punctuator(Punctuator::LeftBrace)?;

        let parent = self.current_function;
        let parent_strict = self.functions[parent].strict;
        let has_use_strict = self.directive_prologue_has_use_strict(self.cursor, parent_strict)?;
        let strict = self.functions[parent].strict || has_use_strict;
        self.relex_current_with_strict(strict)?;
        if strict {
            let strict_validation_span = self.current().span;
            if let Some((identifier, _)) = &function_name_token {
                validate_identifier(
                    identifier,
                    strict_validation_span,
                    true,
                    IdentifierContext::FunctionName,
                )?;
            }
            for (index, (identifier, _)) in parameter_tokens.iter().enumerate() {
                validate_identifier(
                    identifier,
                    strict_validation_span,
                    true,
                    IdentifierContext::Argument,
                )?;
                let parameter = &identifier.value;
                if parameters[..index].contains(parameter) {
                    return Err(Error::syntax(
                        "duplicate argument names not allowed in this context",
                        source_span(self.current().span),
                    ));
                }
            }
        }

        let function_name = function_name_token
            .as_ref()
            .map(|(identifier, _)| identifier.value.clone());
        let child = self.functions.len();
        let parent_scope = self.functions[parent].current_scope;
        self.functions.push(FunctionIr::new(
            Some(ParentLink {
                function: parent,
                definition_scope: parent_scope,
            }),
            FunctionKind::Ordinary,
            FunctionSourceInfo {
                span: function_span,
                definition: source_offset(function_span)?,
                range: None,
            },
            function_name,
            private_name_binding && function_name_token.is_some(),
            parameters,
            strict,
        )?);
        self.current_function = child;
        self.parse_function_body()?;
        let closing_brace = self.current().span;
        let mut parent_context = self.lexer.context();
        parent_context.strict = self.functions[parent].strict;
        self.lexer.set_context(parent_context);
        self.expect_punctuator(Punctuator::RightBrace)?;
        self.functions[child].source.range = Some(
            source_offset(function_span)?
                ..SourceOffset::try_from_usize(closing_brace.end.byte_offset)
                    .map_err(|error| Error::internal(error.to_string()))?,
        );
        self.current_function = parent;

        let constant = self.add_constant(IrConstant::Child(child))?;
        Ok(ParsedFunctionDefinition {
            constant,
            child,
            name: function_name_token.map(|(identifier, span)| (identifier.value, span)),
        })
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

    fn parse_function_body_declaration(&mut self) -> Result<(), Error> {
        let parsed = self.parse_function_definition(true, false)?;
        let (name, declaration_span) = parsed
            .name
            .ok_or_else(|| Error::internal("required function declaration lost its name"))?;
        if !matches!(self.current_ir().kind, FunctionKind::Ordinary) {
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
        let binding = self.ensure_annex_b_binding(&name, declaration_span)?;

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
            self.emit_identifier_inherited(
                name,
                declaration_span,
                root_scope,
                IdentifierAccess::AnnexBPut,
            )?;
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
        let scope_kind = self.current_ir().scopes[self.current_ir().current_scope.0].kind;
        if !matches!(
            scope_kind,
            ScopeKind::Block | ScopeKind::If | ScopeKind::Switch | ScopeKind::FunctionBody
        ) {
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
        if matches!(function.kind, FunctionKind::Ordinary)
            && (name == "arguments"
                || function
                    .parameters
                    .iter()
                    .any(|parameter| parameter == name))
        {
            return false;
        }

        let mut scope = function.current_scope;
        loop {
            if function
                .binding_in_scope(scope, name)
                .is_some_and(|binding| matches!(binding.kind, BindingKind::Lexical { .. }))
            {
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
    ) -> Result<BindingId, Error> {
        let function = self.current_ir_mut();
        let root = function.var_scope;
        if matches!(function.kind, FunctionKind::Script) {
            function.global_declarations.push(IrGlobalDeclaration {
                name: name.to_owned(),
                is_lexical: false,
                is_const: false,
                function_constant: None,
                closure_index: None,
            });
        }
        if let Some(binding) = function.binding_id_in_scope(root, name) {
            let metadata = &function.bindings[binding.0];
            let valid = metadata.kind == BindingKind::Normal
                && match function.kind {
                    FunctionKind::Script => metadata.storage == BindingStorage::Global,
                    FunctionKind::Ordinary => {
                        matches!(metadata.storage, BindingStorage::Local(_))
                    }
                };
            if !valid {
                return Err(Error::internal(
                    "Annex B declaration found a malformed function-root binding",
                ));
            }
            return Ok(binding);
        }

        let storage = match function.kind {
            FunctionKind::Script => BindingStorage::Global,
            FunctionKind::Ordinary => {
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
            }
        };
        Ok(function.add_binding(
            root,
            root,
            name.to_owned(),
            storage,
            BindingKind::Normal,
            Some(declaration_span),
        ))
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

fn function_resolution_order(tree: &FunctionTree) -> Result<Vec<FunctionId>, Error> {
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

    let mut order = Vec::with_capacity(tree.functions.len());
    let mut stack = vec![(0_usize, false)];
    while let Some((function_id, visited)) = stack.pop() {
        if visited {
            order.push(function_id);
            continue;
        }
        stack.push((function_id, true));
        for &child in children[function_id].iter().rev() {
            stack.push((child, false));
        }
    }
    if order.len() != tree.functions.len() {
        return Err(Error::internal("function arena is not one rooted tree"));
    }
    Ok(order)
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
        let expected_body = if matches!(function.kind, FunctionKind::Script) {
            ScopeKind::ProgramBody
        } else {
            ScopeKind::FunctionBody
        };
        if function.scopes[function.body_scope.0].kind != expected_body {
            return Err(Error::internal("function body scope kind is malformed"));
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
        if let Some(index) = function.arguments_local {
            let matches_binding = function.bindings.iter().any(|binding| {
                binding.name == "arguments"
                    && binding.storage_scope == function.var_scope
                    && binding.kind == BindingKind::Normal
                    && binding.storage == BindingStorage::Local(index)
            });
            if !matches!(function.kind, FunctionKind::Ordinary)
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
            let hoist_start = if let Some(local) = function.arguments_local {
                let expected_kind = if function.strict {
                    ArgumentsKind::Unmapped
                } else {
                    ArgumentsKind::Mapped
                };
                if !matches!(
                    function.ops.first(),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::Arguments(kind)),
                        pc_site: None,
                    }) if *kind == expected_kind
                ) || !matches!(
                    function.ops.get(1),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::PutLocal(target)),
                        pc_site: None,
                    }) if *target == local
                ) {
                    return Err(Error::internal(
                        "installed arguments-object prologue is malformed",
                    ));
                }
                2
            } else {
                0
            };
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
                || !matches!(
                    scope_kind,
                    ScopeKind::Block | ScopeKind::If | ScopeKind::Switch | ScopeKind::FunctionBody
                )
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
                let annex = function
                    .bindings
                    .get(annex_binding.0)
                    .ok_or_else(|| Error::internal("Annex B function binding is out of bounds"))?;
                if annex.storage_scope != function.var_scope
                    || annex.kind != BindingKind::Normal
                    || annex.name != binding.name
                {
                    return Err(Error::internal(
                        "Annex B function has malformed root binding metadata",
                    ));
                }
                let write = function.ops.get(scoped.authored_closure + 2);
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
                    BindingStorage::Argument(_) => false,
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
            FunctionKind::Script => {
                for declaration in &function.global_declarations {
                    let binding = if declaration.is_lexical {
                        function.binding_in_scope(function.body_scope, &declaration.name)
                    } else {
                        function.binding_in_scope(function.var_scope, &declaration.name)
                    };
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
            FunctionKind::Ordinary if function.global_declarations.is_empty() => {}
            FunctionKind::Ordinary => {
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
        for (scope_index, scope) in function.scopes.iter().enumerate() {
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
                        if matches!(binding.kind, BindingKind::Lexical { .. }) {
                            let scope_kind = function.scopes[binding.storage_scope.0].kind;
                            let supported_scope =
                                matches!(
                                    scope_kind,
                                    ScopeKind::Block
                                        | ScopeKind::If
                                        | ScopeKind::For
                                        | ScopeKind::Switch
                                        | ScopeKind::Catch
                                ) || (matches!(scope_kind, ScopeKind::FunctionBody)
                                    && matches!(function.kind, FunctionKind::Ordinary)
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
                            || !matches!(function.kind, FunctionKind::Script)
                            || (!valid_lexical && !valid_var)
                        {
                            return Err(Error::internal("global binding metadata is malformed"));
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
                    if matches!(function.kind, FunctionKind::Script) => {}
                SyntheticLocalKind::FinallySavedEvalCompletion => {
                    return Err(Error::internal(
                        "ordinary function contains a finally eval-completion save slot",
                    ));
                }
            }
        }
        match function.kind {
            FunctionKind::Script
                if eval_ret_index == Some(0)
                    && synthetic_eval_ret == eval_ret_index
                    && function
                        .locals
                        .first()
                        .is_some_and(|name| name == EVAL_RET_LOCAL_NAME) => {}
            FunctionKind::Ordinary if eval_ret_index.is_none() && synthetic_eval_ret.is_none() => {}
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
                let body_entry = if function.function_hoists_installed {
                    function
                        .hoisted_functions
                        .len()
                        .saturating_mul(2)
                        .saturating_add(usize::from(function.arguments_local.is_some()) * 2)
                } else {
                    0
                };
                match function.kind {
                    FunctionKind::Script if entries == 0 && leaves == 0 => {}
                    FunctionKind::Ordinary
                        if entries == 1
                            && leaves == 0
                            && matches!(
                                function.ops.get(body_entry).map(|operation| &operation.op),
                                Some(IrOp::EnterScope(body)) if *body == function.body_scope
                            ) => {}
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
        match (
            function.function_name_local,
            function_name_bindings.as_slice(),
        ) {
            (Some(index), [binding])
                if matches!(function.kind, FunctionKind::Ordinary)
                    && function.private_name_binding
                    && binding.storage == BindingStorage::Local(index)
                    && binding.storage_scope == function.var_scope
                    && binding.declaration_scope == function.var_scope
                    && function.function_name.as_deref() == Some(binding.name.as_str())
                    && binding.kind
                        == (BindingKind::FunctionName {
                            is_const: function.strict,
                        }) => {}
            (None, []) => {}
            _ => return Err(Error::internal("function-name binding metadata disagrees")),
        }
        if (function_id == 0) != function.parent.is_none()
            || (function_id == 0) != matches!(function.kind, FunctionKind::Script)
        {
            return Err(Error::internal("function topology is malformed"));
        }
    }
    Ok(())
}

fn resolve_identifiers(tree: &mut FunctionTree) -> Result<(), Error> {
    validate_scope_graph(tree)?;
    seed_global_declarations(tree)?;
    // QuickJS creates and resolves children depth-first in source order before
    // resolving their parent. A plain reverse arena walk reverses sibling
    // capture insertion, so derive the exact stable postorder explicitly.
    for function_id in function_resolution_order(tree)? {
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
                } => Some((index, name.clone(), *span, *scope, *access)),
                _ => None,
            })
            .collect::<Vec<_>>();

        for (operation_index, name, span, scope, access) in unresolved {
            let operation = resolve_identifier(tree, function_id, scope, &name, span, access)?;
            tree.functions[function_id].ops[operation_index].op = operation;
        }
    }
    install_global_function_hoists(tree)?;
    install_function_body_hoists(tree)?;
    validate_scope_graph(tree)
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

/// QuickJS initializes the lazily selected arguments binding before storing
/// direct body function declarations into argument/root-local slots.
fn install_function_body_hoists(tree: &mut FunctionTree) -> Result<(), Error> {
    for function_id in 1..tree.functions.len() {
        let hoists = ordered_hoisted_functions(&tree.functions[function_id])?;
        let arguments_local = tree.functions[function_id].arguments_local;
        let mut prefix = Vec::with_capacity(
            hoists
                .len()
                .saturating_mul(2)
                .saturating_add(usize::from(arguments_local.is_some()) * 2),
        );
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
                BindingStorage::Global => {
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
        if matches!(binding.storage, BindingStorage::Global) {
            return Err(Error::internal(
                "ordinary function hoist targeted global storage",
            ));
        }
    }
    hoists.sort_by_key(|hoist| match function.bindings[hoist.binding.0].storage {
        BindingStorage::Argument(index) => (0_u8, index),
        BindingStorage::Local(index) => (1_u8, index),
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
        if matches!(code[terminal_index], Instruction::ThrowReadOnly(_)) {
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
        if binding.kind != BindingKind::Normal {
            return Err(Error::internal(
                "Annex B root write resolved to a non-ordinary binding",
            ));
        }
        if binding.storage == BindingStorage::Global {
            let closure_index = capture_global_path(tree, function_id, name)?;
            return Ok(IrOp::Bytecode(Instruction::PutVar(closure_index)));
        }
        return binding_instruction(
            &mut tree.functions[function_id],
            binding,
            IdentifierAccess::Put,
            name,
        )
        .map(IrOp::Bytecode);
    }
    if access == IdentifierAccess::Delete
        && name == "arguments"
        && matches!(tree.functions[function_id].kind, FunctionKind::Ordinary)
    {
        // Every ordinary function has an own arguments binding. Sloppy direct
        // delete is statically false and must not force the lazily selected
        // object to exist merely to reject deletion.
        return Ok(IrOp::Bytecode(Instruction::PushFalse));
    }
    if let Some(binding) = find_or_create_own_binding(tree, function_id, use_scope, name, span)? {
        if binding.storage == BindingStorage::Global {
            return global_declaration_operation(tree, function_id, binding.kind, access, name);
        }
        return binding_instruction(&mut tree.functions[function_id], binding, access, name)
            .map(IrOp::Bytecode);
    }

    let mut defining_link = tree.functions[function_id].parent;
    let (defining_function, binding) = loop {
        let Some(link) = defining_link else {
            let closure_index = capture_global_path(tree, function_id, name)?;
            return Ok(match access {
                IdentifierAccess::Get => IrOp::Bytecode(Instruction::GetVar(closure_index)),
                IdentifierAccess::GetOrUndefined => {
                    IrOp::Bytecode(Instruction::GetVarUndef(closure_index))
                }
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
            });
        };
        let candidate = link.function;
        let candidate_scope = link.definition_scope;
        if let Some(binding) =
            find_or_create_own_binding(tree, candidate, candidate_scope, name, span)?
        {
            break (candidate, binding);
        }
        defining_link = tree.functions[candidate].parent;
    };
    if binding.storage == BindingStorage::Global {
        return global_declaration_operation(tree, function_id, binding.kind, access, name);
    }
    let (closure_index, kind) =
        capture_binding_path(tree, defining_function, function_id, binding, name)?;
    closure_binding_operation(
        &mut tree.functions[function_id],
        closure_index,
        kind,
        access,
        name,
    )
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
    if name == "arguments" && matches!(function.kind, FunctionKind::Ordinary) {
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
    }
}

fn capture_binding_path(
    tree: &mut FunctionTree,
    defining_function: FunctionId,
    consuming_function: FunctionId,
    binding: ResolvedBinding,
    name: &str,
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

    let kind = binding.kind;
    let mut source = match binding.storage {
        BindingStorage::Argument(index) => ClosureSource::ParentArgument(index),
        BindingStorage::Local(index) => ClosureSource::ParentLocal(index),
        BindingStorage::Global => {
            return Err(Error::internal(
                "global binding reached local closure capture",
            ));
        }
    };
    let mut final_index = None;
    for function_id in path {
        let function = &mut tree.functions[function_id];
        let descriptor_name = if matches!(
            kind,
            BindingKind::Lexical { .. } | BindingKind::FunctionName { .. }
        ) {
            ClosureVariableName::Constant(ensure_string_constant(function, name)?)
        } else {
            ClosureVariableName::None
        };
        let descriptor = ClosureVariable {
            source,
            name: descriptor_name,
            is_lexical: matches!(kind, BindingKind::Lexical { .. }),
            is_const: matches!(
                kind,
                BindingKind::Lexical { is_const: true }
                    | BindingKind::FunctionName { is_const: true }
            ),
            kind: closure_kind(kind),
        };
        let index = ensure_closure_variable(function, descriptor)?;
        source = ClosureSource::ParentClosure(index);
        final_index = Some(index);
    }
    final_index
        .map(|index| (index, kind))
        .ok_or_else(|| Error::internal("closure path did not cross a function boundary"))
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
                if !matches!(binding.kind, BindingKind::Lexical { .. }) {
                    continue;
                }
                let index = match binding.storage {
                    BindingStorage::Local(index) => index,
                    BindingStorage::Global => continue,
                    BindingStorage::Argument(_) => {
                        return Err(Error::internal(
                            "lexical scope lifecycle referenced an argument",
                        ));
                    }
                };
                if let Some(constant) = scoped_constants[binding_id.0] {
                    lifecycle.function_entries.push(ScopedFunctionEntry {
                        constant,
                        local: index,
                    });
                } else {
                    lifecycle.tdz_locals.push(index);
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
    let mut functions = tree_functions.into_iter().map(Some).collect::<Vec<_>>();
    let mut lowered = (0..function_count).map(|_| None).collect::<Vec<_>>();

    for function_id in (0..function_count).rev() {
        let mut function = functions[function_id]
            .take()
            .ok_or_else(|| Error::internal("function IR was lowered more than once"))?;
        if debug_info == DebugInfoMode::StripDebug {
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
            closure_count: u16::try_from(function.closure_variables.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?,
            max_stack,
            strict: function.strict,
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
            IrOp::TemplateCall(argument_count) => {
                // QuickJS `emit_u16` writes the low operand bits even when an
                // unreachable template has more than 65,535 arguments. The
                // reachability verifier still observes every push on a live
                // path and rejects its stack before this truncated call can
                // execute.
                code.push(Instruction::CallMethod(argument_count as u16));
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
            IrOp::Identifier { .. } => {
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
mod tests {
    use crate::bigint::JsBigInt;
    use crate::bytecode::{ArgumentsKind, Instruction};
    use crate::debug::DebugInfoMode;
    use crate::error::ErrorKind;
    use crate::heap::{
        ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName, ConstructorKind,
    };
    use crate::lexer::{LexError, LexErrorKind, Position, Span};
    use crate::object::{
        AccessorValue, CompleteOrdinaryPropertyDescriptor, DescriptorField,
        OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
    };
    use crate::runtime::{Context, Runtime, RuntimeError};
    use crate::value::{JsString, Value};
    use crate::vm::Vm;

    use super::{
        BindingKind, BindingStorage, FunctionIr, FunctionKind, FunctionSourceInfo,
        MAX_BYTECODE_STACK, MAX_CALL_ARGUMENTS, MAX_LOCAL_VARIABLES, Parser, ScopeKind,
        SourceOffset, compile_script, compile_unlinked_script,
        compile_unlinked_script_with_filename, ensure_closure_variable, lex_error,
        resolve_identifiers,
    };

    #[test]
    fn string_too_long_lex_error_maps_to_js_internal() {
        let position = Position::new(7, 2, 3);
        let error = lex_error(LexError {
            kind: LexErrorKind::StringTooLong,
            span: Span::new(position, position),
            message: "string too long".to_owned(),
        });

        assert_eq!(error.kind(), ErrorKind::JsInternal);
        assert_eq!(error.message(), "string too long");
        assert_eq!(error.span(), None);
    }

    #[test]
    fn parser_records_quickjs_scope_boundaries_and_child_definition_sites() {
        let source = r#"
            { (function blockChild(){ return 1; }); }
            {}
            if ((function ifChild(){ return true; })()) (function ifBody(){});
            for ((function forChild(){ return 0; })(); false;) (function forBody(){});
            switch ((function discriminant(){ return 0; })()) {
                case (function caseChild(){ return 0; })(): (function bodyChild(){});
            }
        "#;
        let tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
        let root = &tree.functions[0];
        assert_eq!(
            root.scopes
                .iter()
                .map(|scope| scope.kind)
                .collect::<Vec<_>>(),
            vec![
                ScopeKind::FunctionRoot,
                ScopeKind::ProgramBody,
                ScopeKind::Block,
                ScopeKind::If,
                ScopeKind::For,
                ScopeKind::Switch,
            ]
        );

        let parent_scope_kind = |name: &str| {
            let function = tree.functions[1..]
                .iter()
                .find(|function| function.function_name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("missing parsed child {name}"));
            let scope = function
                .parent
                .expect("child definition scope")
                .definition_scope;
            root.scopes[scope.0].kind
        };
        assert_eq!(parent_scope_kind("blockChild"), ScopeKind::Block);
        assert_eq!(parent_scope_kind("ifChild"), ScopeKind::If);
        assert_eq!(parent_scope_kind("ifBody"), ScopeKind::If);
        assert_eq!(parent_scope_kind("forChild"), ScopeKind::For);
        assert_eq!(parent_scope_kind("forBody"), ScopeKind::For);
        assert_eq!(parent_scope_kind("discriminant"), ScopeKind::ProgramBody);
        assert_eq!(parent_scope_kind("caseChild"), ScopeKind::Switch);
        assert_eq!(parent_scope_kind("bodyChild"), ScopeKind::Switch);
    }

    #[test]
    fn var_bindings_keep_root_storage_and_first_declaration_scope() {
        let source = "(function(a,a){{var x=1;}{var x;}(function child(){return a;});return a+x;})";
        let mut tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
        let function = &tree.functions[1];
        assert_eq!(function.scopes[0].kind, ScopeKind::FunctionRoot);
        assert_eq!(function.scopes[1].kind, ScopeKind::FunctionBody);
        assert_eq!(function.scopes[2].kind, ScopeKind::Block);
        assert_eq!(function.scopes[3].kind, ScopeKind::Block);

        let parameters = function
            .bindings
            .iter()
            .filter(|binding| binding.name == "a")
            .map(|binding| binding.storage)
            .collect::<Vec<_>>();
        assert_eq!(
            parameters,
            vec![BindingStorage::Argument(0), BindingStorage::Argument(1)]
        );
        let x = function
            .bindings
            .iter()
            .find(|binding| binding.name == "x")
            .expect("function-scoped x binding");
        assert_eq!(x.storage_scope.0, 0);
        assert_eq!(x.declaration_scope.0, 2);
        assert_eq!(x.storage, BindingStorage::Local(0));
        assert_eq!(x.kind, BindingKind::Normal);

        resolve_identifiers(&mut tree).unwrap();
        assert!(tree.functions[1].ops.iter().any(|operation| matches!(
            operation.op,
            super::IrOp::Bytecode(Instruction::GetArg(1))
        )));
        assert!(tree.functions[1].ops.iter().any(|operation| matches!(
            operation.op,
            super::IrOp::Bytecode(Instruction::GetLocal(0))
        )));
        assert_eq!(
            tree.functions[2].closure_variables[0].source,
            ClosureSource::ParentArgument(1)
        );
    }

    #[test]
    fn definition_scope_selects_same_named_sibling_bindings() {
        let source = "{(function left(){return shadow;});}{(function right(){return shadow;});}";
        let mut tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
        let left_scope = tree.functions[1].parent.unwrap().definition_scope;
        let right_scope = tree.functions[2].parent.unwrap().definition_scope;
        assert_ne!(left_scope, right_scope);

        let root = &mut tree.functions[0];
        let left_local = u16::try_from(root.locals.len()).unwrap();
        root.locals.push("shadow".to_owned());
        root.add_binding(
            left_scope,
            left_scope,
            "shadow".to_owned(),
            BindingStorage::Local(left_local),
            BindingKind::Normal,
            None,
        );
        let right_local = u16::try_from(root.locals.len()).unwrap();
        root.locals.push("shadow".to_owned());
        root.add_binding(
            right_scope,
            right_scope,
            "shadow".to_owned(),
            BindingStorage::Local(right_local),
            BindingKind::Normal,
            None,
        );

        resolve_identifiers(&mut tree).unwrap();
        assert_eq!(
            tree.functions[1].closure_variables[0].source,
            ClosureSource::ParentLocal(left_local)
        );
        assert_eq!(
            tree.functions[2].closure_variables[0].source,
            ClosureSource::ParentLocal(right_local)
        );
    }

    #[test]
    fn ancestor_lookup_uses_each_function_definition_scope() {
        let source = "{(function middle(){return (function leaf(){return shadow;});});}";
        let mut tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
        let middle_definition_scope = tree.functions[1].parent.unwrap().definition_scope;
        let leaf_definition_scope = tree.functions[2].parent.unwrap().definition_scope;
        assert_eq!(
            tree.functions[1].scopes[leaf_definition_scope.0].kind,
            ScopeKind::FunctionBody
        );

        let root = &mut tree.functions[0];
        let local = u16::try_from(root.locals.len()).unwrap();
        root.locals.push("shadow".to_owned());
        root.add_binding(
            middle_definition_scope,
            middle_definition_scope,
            "shadow".to_owned(),
            BindingStorage::Local(local),
            BindingKind::Normal,
            None,
        );

        resolve_identifiers(&mut tree).unwrap();
        assert_eq!(
            tree.functions[1].closure_variables[0].source,
            ClosureSource::ParentLocal(local)
        );
        assert_eq!(
            tree.functions[2].closure_variables[0].source,
            ClosureSource::ParentClosure(0)
        );
    }

    #[test]
    fn identifier_rewrites_preserve_the_original_use_scope() {
        let source = "(function(value){{typeof value;delete value;value=1;value+=2;value||=3;++value;value++;}for(;;value+=1){break;}})";
        let tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
        let function = &tree.functions[1];
        let scope_kinds = function
            .ops
            .iter()
            .filter_map(|operation| match operation.op {
                super::IrOp::Identifier { scope, .. } => Some(function.scopes[scope.0].kind),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            scope_kinds
                .iter()
                .filter(|kind| **kind == ScopeKind::Block)
                .count(),
            11
        );
        assert_eq!(
            scope_kinds
                .iter()
                .filter(|kind| **kind == ScopeKind::For)
                .count(),
            2
        );
        assert_eq!(scope_kinds.len(), 13);
    }

    #[test]
    fn resolver_uses_source_order_dfs_postorder_for_sibling_relays() {
        let source = "(function outer(a,b){return (function middle(){(function childA(){return a;});(function childB(){return b;});});})";
        let mut tree = Parser::parse(source, JsString::from_static("<scope-test>")).unwrap();
        resolve_identifiers(&mut tree).unwrap();

        let function_id = |name: &str| {
            tree.functions
                .iter()
                .position(|function| function.function_name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("missing parsed function {name}"))
        };
        let middle = function_id("middle");
        let child_a = function_id("childA");
        let child_b = function_id("childB");
        assert_eq!(
            tree.functions[middle]
                .closure_variables
                .iter()
                .map(|binding| binding.source)
                .collect::<Vec<_>>(),
            vec![
                ClosureSource::ParentArgument(0),
                ClosureSource::ParentArgument(1),
            ]
        );
        assert_eq!(
            tree.functions[child_a].closure_variables[0].source,
            ClosureSource::ParentClosure(0)
        );
        assert_eq!(
            tree.functions[child_b].closure_variables[0].source,
            ClosureSource::ParentClosure(1)
        );
    }

    #[test]
    fn closure_slots_deduplicate_by_storage_identity_and_reject_metadata_conflicts() {
        let span = Span::new(Position::new(0, 1, 1), Position::new(0, 1, 1));
        let mut function = FunctionIr::new(
            None,
            FunctionKind::Ordinary,
            FunctionSourceInfo {
                span,
                definition: SourceOffset::try_from_usize(0).unwrap(),
                range: None,
            },
            None,
            false,
            Vec::new(),
            false,
        )
        .unwrap();
        let local = ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        };
        assert_eq!(ensure_closure_variable(&mut function, local).unwrap(), 0);
        assert_eq!(ensure_closure_variable(&mut function, local).unwrap(), 0);

        let other_local = ClosureVariable {
            source: ClosureSource::ParentLocal(1),
            ..local
        };
        assert_eq!(
            ensure_closure_variable(&mut function, other_local).unwrap(),
            1
        );

        let conflict = ClosureVariable {
            is_const: true,
            ..local
        };
        assert_eq!(
            ensure_closure_variable(&mut function, conflict)
                .unwrap_err()
                .message(),
            "closure storage source has conflicting binding metadata"
        );

        for name in [0, 1] {
            ensure_closure_variable(
                &mut function,
                ClosureVariable {
                    source: ClosureSource::Global,
                    name: ClosureVariableName::Constant(name),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
            )
            .unwrap();
        }
        assert_eq!(function.closure_variables.len(), 4);
    }

    #[test]
    fn scope_graph_validation_rejects_invalid_definition_and_binding_identity() {
        let mut bad_parent = Parser::parse(
            "(function child(){})",
            JsString::from_static("<scope-test>"),
        )
        .unwrap();
        bad_parent.functions[1]
            .parent
            .as_mut()
            .unwrap()
            .definition_scope = super::ScopeId(999);
        assert_eq!(
            resolve_identifiers(&mut bad_parent).unwrap_err().message(),
            "child definition scope is out of bounds"
        );

        let mut duplicate = Parser::parse(
            "(function child(value){return value;})",
            JsString::from_static("<scope-test>"),
        )
        .unwrap();
        let binding = duplicate.functions[1].scopes[0].bindings[0];
        duplicate.functions[1].scopes[0].bindings.push(binding);
        assert_eq!(
            resolve_identifiers(&mut duplicate).unwrap_err().message(),
            "binding appears more than once in the scope graph"
        );

        let mut aliased_slot = Parser::parse(
            "(function child(value,value){return value;})",
            JsString::from_static("<scope-test>"),
        )
        .unwrap();
        aliased_slot.functions[1].bindings[1].storage = BindingStorage::Argument(0);
        assert_eq!(
            resolve_identifiers(&mut aliased_slot)
                .unwrap_err()
                .message(),
            "argument slot has more than one binding identity"
        );

        let mut missing_slot = Parser::parse(
            "(function child(value){return value;})",
            JsString::from_static("<scope-test>"),
        )
        .unwrap();
        missing_slot.functions[1].scopes[0].bindings.clear();
        missing_slot.functions[1].bindings.clear();
        assert_eq!(
            resolve_identifiers(&mut missing_slot)
                .unwrap_err()
                .message(),
            "argument slot is missing its binding identity"
        );

        let mut malformed_scope =
            Parser::parse("0", JsString::from_static("<scope-test>")).unwrap();
        malformed_scope.functions[0].scopes.push(super::IrScope {
            parent: Some(super::ScopeId(0)),
            kind: ScopeKind::ProgramBody,
            bindings: Vec::new(),
        });
        malformed_scope.functions[0].body_scope = super::ScopeId(2);
        malformed_scope.functions[0].current_scope = super::ScopeId(2);
        malformed_scope.functions[0].scopes[1].parent = Some(super::ScopeId(99));
        assert_eq!(
            resolve_identifiers(&mut malformed_scope)
                .unwrap_err()
                .message(),
            "lexical scope parent is malformed"
        );
    }

    fn evaluate(source: &str) -> Value {
        let bytecode = compile_script(source).unwrap();
        Vm::new().execute(&bytecode).unwrap()
    }

    fn evaluate_in_context(source: &str) -> Value {
        Runtime::new().new_context().eval(source).unwrap()
    }

    fn evaluate_error(
        runtime: &Runtime,
        context: &mut Context,
        source: &str,
    ) -> (JsString, JsString) {
        assert_eq!(
            context.eval(source),
            Err(RuntimeError::Exception),
            "{source}"
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("source did not throw an Error object: {source}");
        };
        let name = runtime.intern_property_key("name").unwrap();
        let message = runtime.intern_property_key("message").unwrap();
        let Value::String(name) = context.get_property(&error, &name).unwrap() else {
            panic!("Error.name was not a string: {source}");
        };
        let Value::String(message) = context.get_property(&error, &message).unwrap() else {
            panic!("Error.message was not a string: {source}");
        };
        (name, message)
    }

    fn evaluate_function_name(source: &str) -> (JsString, bool, bool, bool) {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context.eval(source).unwrap() else {
            panic!("source did not evaluate to a function object");
        };
        let name = runtime.intern_property_key("name").unwrap();
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable,
            enumerable,
            configurable,
        } = runtime.get_own_property(&function, &name).unwrap().unwrap()
        else {
            panic!("function name did not have the ordinary data descriptor");
        };
        (value, writable, enumerable, configurable)
    }

    #[test]
    fn ordinary_function_body_lexicals_execute_local_capture_and_constructor_paths() {
        assert_eq!(
            evaluate_in_context(
                "(function(){let x=1,y=x+1,z;return x*10+y+(typeof z==='undefined'?0:100)})()"
            ),
            Value::Int(12)
        );
        assert_eq!(
            evaluate_in_context("(function(){let arguments=3;let eval=4;return arguments+eval})()"),
            Value::Int(7)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var read=function(){return x};let x=7;return read()})()"
            ),
            Value::Int(7)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){let x=function(){return x};return x()===x&&x.name==='x'})()"
            ),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){let x=1;var next=function(){x+=1;return x};return next()*10+next()})()"
            ),
            Value::Int(23)
        );
        assert_eq!(
            evaluate_in_context("(function(){const x=4;return function(){return x}()})()"),
            Value::Int(4)
        );
        assert_eq!(
            evaluate_in_context("Function('let x=1;const y=2;return x+y')()"),
            Value::Int(3)
        );
        assert_eq!(
            evaluate_in_context("(function(){var result=delete x;let x=1;return result})()"),
            Value::Bool(false)
        );
        assert_eq!(
            evaluate_in_context("(function(){const x=0;return x&&=missing})()"),
            Value::Int(0)
        );
    }

    #[test]
    fn lexical_lowering_publishes_tdz_vardefs_and_checked_capture_relays() {
        let script = compile_unlinked_script(
            "(function(){let x=1;const y=2;return function(){x+=y;return x}})",
        )
        .unwrap();
        let outer = script
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("script lost its outer function");
        assert!(matches!(
            outer.code(),
            [
                Instruction::SetLocalUninitialized(1),
                Instruction::SetLocalUninitialized(0),
                Instruction::PushI32(1),
                Instruction::InitializeLocal(0),
                Instruction::PushI32(2),
                Instruction::InitializeLocal(1),
                ..
            ]
        ));
        assert_eq!(outer.local_definitions().len(), 2);
        assert_eq!(
            outer.local_definitions()[0].name.as_ref(),
            Some(&JsString::from_static("x"))
        );
        assert!(outer.local_definitions()[0].is_lexical);
        assert!(!outer.local_definitions()[0].is_const);
        assert_eq!(
            outer.local_definitions()[1].name.as_ref(),
            Some(&JsString::from_static("y"))
        );
        assert!(outer.local_definitions()[1].is_lexical);
        assert!(outer.local_definitions()[1].is_const);

        let inner = outer
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("outer function lost its captured child");
        assert_eq!(inner.closure_variables().len(), 2);
        for (index, expected_name, is_const) in [(0, "x", false), (1, "y", true)] {
            let descriptor = inner.closure_variables()[index];
            assert_eq!(
                descriptor.source,
                ClosureSource::ParentLocal(u16::try_from(index).unwrap())
            );
            assert!(descriptor.is_lexical);
            assert_eq!(descriptor.is_const, is_const);
            assert_eq!(descriptor.kind, ClosureVariableKind::Normal);
            let ClosureVariableName::Constant(name) = descriptor.name else {
                panic!("lexical descriptor lost its source name");
            };
            assert_eq!(
                inner.constants()[usize::try_from(name).unwrap()].as_primitive(),
                Some(&Value::String(JsString::from_static(expected_name)))
            );
        }
        assert!(inner.code().windows(5).any(|window| matches!(
            window,
            [
                Instruction::GetVarRefCheck(0),
                Instruction::GetVarRefCheck(1),
                Instruction::Add,
                Instruction::Dup,
                Instruction::PutVarRefCheck(0),
            ]
        )));
        assert!(inner.code().windows(2).any(|window| matches!(
            window,
            [Instruction::GetVarRefCheck(0), Instruction::Return]
        )));
    }

    #[test]
    fn nested_block_and_switch_lexicals_lower_scope_lifetimes() {
        let script = compile_unlinked_script(
            "(function(){var read;{read=function(){return ++value};let value=40;}return read()*100+read();})()",
        )
        .unwrap();
        let outer = script
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("script lost its block function");
        assert!(
            outer
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::SetLocalUninitialized(1)))
        );
        assert!(
            outer
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::CloseLocal(1)))
        );
        let child = outer
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("block function lost its captured child");
        assert_eq!(
            child.closure_variables()[0].source,
            ClosureSource::ParentLocal(1)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var read;{read=function(){return ++value};let value=40;}return read()*100+read();})()"
            ),
            Value::Int(4142)
        );

        let switch_script = compile_unlinked_script(
            "(function(){var read;switch(0){case 0:let value=40;read=function(){return ++value};break;}return read()*100+read();})()",
        )
        .unwrap();
        let switch_function = switch_script
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("script lost its switch function");
        assert!(switch_function.code().windows(2).any(|window| matches!(
            window,
            [
                Instruction::PushI32(0),
                Instruction::SetLocalUninitialized(1)
            ]
        )));
        assert!(
            switch_function
                .code()
                .windows(2)
                .any(|window| matches!(window, [Instruction::Drop, Instruction::CloseLocal(1)]))
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var read;switch(0){case 0:let value=40;read=function(){return ++value};break;}return read()*100+read();})()"
            ),
            Value::Int(4142)
        );
        assert_eq!(evaluate_in_context("{let value=42;value;}"), Value::Int(42));
        assert_eq!(
            evaluate_in_context("switch(0){case 0:const value=42;value;}"),
            Value::Int(42)
        );

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert!(matches!(
            context.eval("(function(){{let value=40;throw function(){return ++value};}})()"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(thrown) = context.take_exception().unwrap().unwrap() else {
            panic!("nested lexical throw did not preserve the escaped closure");
        };
        let callable = runtime.as_callable(&thrown).unwrap().unwrap();
        assert_eq!(
            context.call(&callable, Value::Undefined, &[]).unwrap(),
            Value::Int(41)
        );
        assert_eq!(
            context.call(&callable, Value::Undefined, &[]).unwrap(),
            Value::Int(42)
        );
    }

    #[test]
    fn nested_lexical_cleanup_shadowing_and_quickjs_var_quirks_execute() {
        assert_eq!(
            evaluate_in_context(
                "(function(){var first,second,index=0;while(index<2){{let value=index++;if(index===1){first=function(){return ++value};continue;}second=function(){return ++value};}}return first()*100+second()*10+first()+second();})()"
            ),
            Value::Int(125)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var first,second,index=0;outer:while(index<2){switch(0){case 0:let value=index++;if(index===1){first=function(){return ++value};continue outer;}second=function(){return ++value};break outer;}}return first()*100+second()*10+first()+second();})()"
            ),
            Value::Int(125)
        );
        assert_eq!(
            evaluate_in_context(
                "(function self(parameter){var outer='O',result;{let parameter='P',outer='B',self='S';result=parameter+outer+self;}return result+'|'+parameter+'|'+outer+'|'+typeof self;})('p')"
            ),
            Value::String(JsString::from_static("PBS|p|O|function"))
        );
        for source in [
            "(function(){var value;{var value;let value;}return 1})()",
            "(function(value){{var value;let value;}return 1})(0)",
        ] {
            assert_eq!(evaluate_in_context(source), Value::Int(1), "{source}");
        }
        for (source, message) in [
            (
                "(function(){var value;{let value;var value;}})",
                "invalid redefinition of lexical identifier",
            ),
            (
                "(function(){{var value;}let value;})",
                "invalid redefinition of a variable",
            ),
            (
                "(function(){let value;{var value;}})",
                "invalid redefinition of lexical identifier",
            ),
            (
                "(function(){switch(0){case 0:let value;case 1:const value=1;}})",
                "invalid redefinition of lexical identifier",
            ),
        ] {
            assert_eq!(
                compile_unlinked_script(source).unwrap_err().message(),
                message,
                "{source}"
            );
        }
    }

    #[test]
    fn lexical_tdz_and_readonly_errors_follow_checked_local_and_capture_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for source in [
            "(function(){return x;let x=1})()",
            "(function(){return typeof x;let x=1})()",
            "(function(){x=1;let x})()",
            "(function(){return function(){return x};let x=1})()()",
            "(function(){var set=function(){x=1};set();let x})()",
            "(function(){var add=function(){x+=missing};add();const x=1})()",
        ] {
            assert_eq!(
                evaluate_error(&runtime, &mut context, source),
                (
                    JsString::from_static("ReferenceError"),
                    JsString::from_static("x is not initialized")
                ),
                "{source}"
            );
        }
        for source in [
            "(function(){const x=1;x=2})()",
            "(function(){const x=1;return function(){x=2}})()()",
            "(function(){var set=function(){x=1};set();const x=2})()",
        ] {
            assert_eq!(
                evaluate_error(&runtime, &mut context, source),
                (
                    JsString::from_static("TypeError"),
                    JsString::from_static("'x' is read-only")
                ),
                "{source}"
            );
        }
    }

    #[test]
    fn lexical_parser_matches_redefinition_priority_contextual_let_and_boundaries() {
        let syntax_cases = [
            (
                "(function(){\nlet x;\nlet x;\n})",
                "invalid redefinition of lexical identifier",
                3,
                6,
            ),
            (
                "(function(){\nvar x;\nlet x;\n})",
                "invalid redefinition of a variable",
                3,
                6,
            ),
            (
                "(function(){\nlet x;\nvar x;\n})",
                "invalid redefinition of lexical identifier",
                3,
                6,
            ),
            (
                "(function(x){\nlet x;\n})",
                "invalid redefinition of parameter name",
                2,
                6,
            ),
            (
                "(function(){\nconst x;\n})",
                "missing initializer for const variable",
                2,
                8,
            ),
        ];
        for (source, message, line, column) in syntax_cases {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
            assert_eq!(error.message(), message, "{source}");
            let span = error.span().expect("syntax error lost its source span");
            assert_eq!(
                (span.start.line, span.start.column),
                (line, column),
                "{source}"
            );
        }
        for (source, message) in [
            (
                "(function(){let let=1})",
                "'let' is not a valid lexical identifier",
            ),
            (
                "(function(){'use strict';let eval=1})",
                "invalid variable name in strict mode",
            ),
            (
                "(function(){if(true) let x=1})",
                "lexical declarations can't appear in single-statement context",
            ),
        ] {
            assert_eq!(
                compile_unlinked_script(source).unwrap_err().message(),
                message
            );
        }

        assert_eq!(
            evaluate_in_context("(function(){var let=0;let=2;return let})()"),
            Value::Int(2)
        );
        assert_eq!(
            evaluate_in_context("(function(){var x=0;if(false) let\nx=1;return x})()"),
            Value::Int(1)
        );
        assert_eq!(
            evaluate_in_context("(function named(){let named=3;return named})()"),
            Value::Int(3)
        );

        for source in [
            "(function(){let [item]=source})",
            "(function(){{let [item]=source}})",
            "(function(){switch(0){case 0:let [item]=source}})",
        ] {
            assert!(compile_unlinked_script(source).is_err(), "{source}");
        }
        assert_eq!(
            evaluate_in_context("(function(){{let nested=1;return nested}})()"),
            Value::Int(1)
        );
        assert_eq!(
            evaluate_in_context("(function(){switch(0){case 0:let inCase=1;return inCase}})()"),
            Value::Int(1)
        );
    }

    #[test]
    fn program_lexicals_lower_to_source_ordered_global_declarations() {
        let source = "let first=1,second=function(){return later};const later=2;first+second()";
        let script = compile_unlinked_script(source).unwrap();
        assert_eq!(script.local_definitions().len(), 1);
        assert_eq!(script.closure_variables().len(), 3);

        let declaration_names = script
            .closure_variables()
            .iter()
            .map(|descriptor| {
                assert_eq!(descriptor.source, ClosureSource::GlobalDeclaration);
                assert!(descriptor.is_lexical);
                let ClosureVariableName::Constant(index) = descriptor.name else {
                    panic!("global declaration lost its semantic name");
                };
                let Value::String(name) = script.constants()[index as usize]
                    .as_primitive()
                    .expect("global declaration name is not primitive")
                else {
                    panic!("global declaration name is not a string");
                };
                name.to_utf8_lossy()
            })
            .collect::<Vec<_>>();
        assert_eq!(declaration_names, ["first", "second", "later"]);
        assert_eq!(
            script
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::PutVarInit(_)))
                .count(),
            3
        );

        let child = script
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("global lexical initializer lost its child function");
        assert_eq!(child.closure_variables().len(), 1);
        assert_eq!(
            child.closure_variables()[0].source,
            ClosureSource::ParentGlobal(2)
        );
        assert!(child.closure_variables()[0].is_lexical);
        assert!(child.closure_variables()[0].is_const);

        let stripped = compile_unlinked_script_with_filename(
            source,
            "<global-strip>",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        assert!(stripped.closure_variables().iter().all(|descriptor| {
            descriptor.source == ClosureSource::GlobalDeclaration
                && matches!(descriptor.name, ClosureVariableName::Constant(_))
        }));
        let stripped_child = stripped
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("stripped global lexical lost its child function");
        assert!(matches!(
            stripped_child.closure_variables()[0].name,
            ClosureVariableName::Constant(_)
        ));
    }

    #[test]
    fn program_vars_keep_every_source_ordered_global_declaration() {
        let source = "var first;{var first;var second=function(){return later}}if(false)var later=3;for(var loop=0;false;){}";
        let script = compile_unlinked_script(source).unwrap();
        assert_eq!(script.local_definitions().len(), 1);

        let declaration_names = script
            .closure_variables()
            .iter()
            .map(|descriptor| {
                assert_eq!(descriptor.source, ClosureSource::GlobalDeclaration);
                assert!(!descriptor.is_lexical);
                assert!(!descriptor.is_const);
                assert_eq!(descriptor.kind, ClosureVariableKind::Normal);
                let ClosureVariableName::Constant(index) = descriptor.name else {
                    panic!("global var declaration lost its semantic name");
                };
                let Value::String(name) = script.constants()[index as usize]
                    .as_primitive()
                    .expect("global var declaration name is not primitive")
                else {
                    panic!("global var declaration name is not a string");
                };
                name.to_utf8_lossy()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            declaration_names,
            ["first", "first", "second", "later", "loop"]
        );
        assert_eq!(
            script
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::PutVar(_)))
                .count(),
            3
        );
        assert!(
            !script
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::PutVarInit(_)))
        );

        let child = script
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("global var initializer lost its child function");
        assert_eq!(child.closure_variables().len(), 1);
        assert_eq!(
            child.closure_variables()[0].source,
            ClosureSource::ParentGlobal(3)
        );
        assert!(!child.closure_variables()[0].is_lexical);

        let stripped = compile_unlinked_script_with_filename(
            source,
            "<global-var-strip>",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        assert!(stripped.closure_variables().iter().all(|descriptor| {
            descriptor.source == ClosureSource::GlobalDeclaration
                && matches!(descriptor.name, ClosureVariableName::Constant(_))
        }));
    }

    #[test]
    fn program_functions_keep_descriptors_but_hoist_into_the_first_name_slot() {
        let script = compile_unlinked_script(
            "let mixed;function mixed(){return mixed}function repeated(){return 1}function repeated(){return 2}repeated",
        )
        .unwrap();
        assert_eq!(script.closure_variables().len(), 4);
        assert!(script.closure_variables()[0].is_lexical);
        assert_eq!(
            script.closure_variables()[1].kind,
            ClosureVariableKind::GlobalFunction
        );
        assert_eq!(
            script.closure_variables()[2].kind,
            ClosureVariableKind::GlobalFunction
        );
        assert_eq!(
            script.closure_variables()[3].kind,
            ClosureVariableKind::GlobalFunction
        );
        assert!(matches!(script.code()[0], Instruction::FClosure(0)));
        assert!(matches!(script.code()[1], Instruction::PutVarInit(0)));
        assert!(matches!(script.code()[2], Instruction::FClosure(1)));
        assert!(matches!(script.code()[3], Instruction::PutVarInit(2)));
        assert!(matches!(script.code()[4], Instruction::FClosure(2)));
        assert!(matches!(script.code()[5], Instruction::PutVarInit(2)));

        let children = script
            .constants()
            .iter()
            .filter_map(|constant| constant.as_child())
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].metadata().function_name_local, None);
        assert_eq!(
            children[0].closure_variables()[0].source,
            ClosureSource::ParentGlobal(0)
        );
        assert!(children[0].closure_variables()[0].is_lexical);
        for child in &children[1..] {
            assert_eq!(child.metadata().function_name_local, None);
        }
    }

    #[test]
    fn program_function_then_lexical_remains_a_source_ordered_syntax_error() {
        let error = compile_unlinked_script("function clash(){};let clash").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax);
        assert_eq!(error.message(), "invalid redefinition of global identifier");
        assert!(compile_unlinked_script("let clash;function clash(){}").is_ok());
    }

    #[test]
    fn program_vars_instantiate_persist_and_preserve_existing_properties() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval("var value;var value=2;if(false){var dormant=3}for(var loop=0;loop<2;loop++){};value+'|'+typeof dormant+'|'+loop")
                .unwrap(),
            Value::String(JsString::from_static("2|undefined|2"))
        );
        let global = context.global_object().unwrap();
        for (name, value) in [
            ("value", Value::Int(2)),
            ("dormant", Value::Undefined),
            ("loop", Value::Int(2)),
        ] {
            let key = runtime.intern_property_key(name).unwrap();
            assert_eq!(
                context.get_own_property(&global, &key).unwrap(),
                Some(CompleteOrdinaryPropertyDescriptor::Data {
                    value,
                    writable: true,
                    enumerable: true,
                    configurable: false,
                })
            );
        }
        assert_eq!(context.eval("var value;value").unwrap(), Value::Int(2));
        assert_eq!(
            context
                .eval("var captured=1,read=function(){return captured};captured=4;read()")
                .unwrap(),
            Value::Int(4)
        );
        assert_eq!(context.eval("delete value").unwrap(), Value::Bool(false));

        assert_eq!(context.eval("hostValue=7").unwrap(), Value::Int(7));
        let host = runtime.intern_property_key("hostValue").unwrap();
        assert_eq!(
            context.eval("var hostValue;hostValue").unwrap(),
            Value::Int(7)
        );
        assert_eq!(
            context.get_own_property(&global, &host).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(7),
                writable: true,
                enumerable: true,
                configurable: true,
            })
        );
        assert_eq!(
            context
                .eval("Function.hostRead=function(){return hostValue};var hostValue=8;delete hostValue")
                .unwrap(),
            Value::Bool(true)
        );
        assert!(matches!(
            context.eval("Function.hostRead()"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert_eq!(
            context.eval("var hostValue;hostValue").unwrap(),
            Value::Undefined
        );
        assert_eq!(
            context.eval("Function.hostRead()").unwrap(),
            Value::Undefined
        );
        assert_eq!(
            context.get_own_property(&global, &host).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Undefined,
                writable: true,
                enumerable: true,
                configurable: false,
            })
        );

        let fixed = runtime.intern_property_key("fixedVar").unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &fixed,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(1)),
                        writable: DescriptorField::Present(false),
                        enumerable: DescriptorField::Present(false),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(
            context.eval("var fixedVar;fixedVar").unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            context.eval("var fixedVar=2;fixedVar").unwrap(),
            Value::Int(1)
        );
        assert!(matches!(
            context.eval("'use strict';var fixedVar=2"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert_eq!(
            context.get_own_property(&global, &fixed).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(1),
                writable: false,
                enumerable: false,
                configurable: true,
            })
        );

        assert_eq!(
            context.eval("Function.varSetterHits=0").unwrap(),
            Value::Int(0)
        );
        let Value::Object(setter) = context
            .eval("(function(value){Function.varSetterHits=value})")
            .unwrap()
        else {
            panic!("global var accessor probe did not create a setter");
        };
        let setter = runtime.as_callable(&setter).unwrap().unwrap();
        let accessor = runtime.intern_property_key("accessorVar").unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &accessor,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Undefined),
                        set: DescriptorField::Present(AccessorValue::Callable(setter.clone())),
                        enumerable: DescriptorField::Present(false),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(
            context
                .eval("var accessorVar;Function.varSetterHits")
                .unwrap(),
            Value::Int(0),
            "a var without initializer must not invoke an existing setter"
        );
        assert_eq!(
            context
                .eval("var accessorVar=9;Function.varSetterHits")
                .unwrap(),
            Value::Int(9)
        );
        assert_eq!(
            context.get_own_property(&global, &accessor).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Accessor {
                get: None,
                set: Some(setter),
                enumerable: false,
                configurable: true,
            })
        );

        let mut inherited = runtime.new_context();
        let inherited_key = runtime.intern_property_key("inheritedVar").unwrap();
        let prototype = inherited.object_prototype().unwrap();
        assert!(
            inherited
                .define_own_property(
                    &prototype,
                    &inherited_key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(5)),
                        writable: DescriptorField::Present(true),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(
            inherited.eval("var inheritedVar;inheritedVar").unwrap(),
            Value::Undefined
        );
        let inherited_global = inherited.global_object().unwrap();
        assert_eq!(
            inherited
                .get_own_property(&inherited_global, &inherited_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Undefined,
                writable: true,
                enumerable: true,
                configurable: false,
            })
        );

        let mut auto_init = runtime.new_context();
        assert_eq!(
            auto_init.eval("var Number;typeof Number").unwrap(),
            Value::String(JsString::from_static("function"))
        );
        assert_eq!(
            auto_init.eval("var Number=9;Number").unwrap(),
            Value::Int(9)
        );
        let number = runtime.intern_property_key("Number").unwrap();
        let auto_init_global = auto_init.global_object().unwrap();
        assert_eq!(
            auto_init
                .get_own_property(&auto_init_global, &number)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(9),
                writable: true,
                enumerable: false,
                configurable: true,
            })
        );
    }

    #[test]
    fn program_var_preflight_conflicts_and_parser_scope_match_quickjs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context.eval("let existingLexical=1").unwrap(),
            Value::Undefined
        );
        assert!(matches!(
            context.eval("var existingLexical"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("global var/lexical conflict did not throw an Error object");
        };
        let name = runtime.intern_property_key("name").unwrap();
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from_static("SyntaxError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static("redeclaration of 'existingLexical'"))
        );
        assert_eq!(
            context.eval("globalThis.varMarker=0").unwrap(),
            Value::Int(0)
        );
        assert!(matches!(
            context.eval("varMarker=1;var freshBefore=(varMarker=2),existingLexical=(varMarker=3),freshAfter=(varMarker=4)"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert_eq!(context.eval("varMarker").unwrap(), Value::Int(0));
        assert_eq!(
            context
                .eval("typeof freshBefore+'|'+typeof freshAfter")
                .unwrap(),
            Value::String(JsString::from_static("undefined|undefined"))
        );

        let mut sealed = runtime.new_context();
        assert_eq!(
            sealed.eval("let sealedLexical=1").unwrap(),
            Value::Undefined
        );
        let sealed_global = sealed.global_object().unwrap();
        runtime.prevent_extensions(&sealed_global).unwrap();
        assert!(matches!(
            sealed.eval("var sealedLexical"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = sealed.take_exception().unwrap().unwrap() else {
            panic!("sealed global var declaration did not throw an Error object");
        };
        assert_eq!(
            sealed.get_property(&error, &name).unwrap(),
            Value::String(JsString::from_static("TypeError"))
        );
        assert_eq!(
            sealed.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static(
                "cannot define variable 'sealedLexical'"
            ))
        );

        let mut atomic = runtime.new_context();
        assert_eq!(
            atomic
                .eval("globalThis.atomicExisting=5;globalThis.atomicMarker=0")
                .unwrap(),
            Value::Int(0)
        );
        let atomic_global = atomic.global_object().unwrap();
        runtime.prevent_extensions(&atomic_global).unwrap();
        assert!(matches!(
            atomic.eval("atomicMarker=1;var atomicExisting=6,atomicMissing=7"),
            Err(RuntimeError::Exception)
        ));
        atomic.take_exception().unwrap().unwrap();
        assert_eq!(atomic.eval("atomicMarker").unwrap(), Value::Int(0));
        assert_eq!(atomic.eval("atomicExisting").unwrap(), Value::Int(5));
        assert_eq!(
            atomic.eval("typeof atomicMissing").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );

        for (source, expected) in [
            (
                "let conflict;var conflict",
                "invalid redefinition of lexical identifier",
            ),
            (
                "var conflict;let conflict",
                "invalid redefinition of global identifier",
            ),
            (
                "{var conflict;var conflict;let conflict}",
                "invalid redefinition of global identifier",
            ),
            (
                "{let conflict;var conflict}",
                "invalid redefinition of lexical identifier",
            ),
        ] {
            assert_eq!(
                compile_unlinked_script(source).unwrap_err().message(),
                expected,
                "{source}"
            );
        }
        for source in [
            "var allowed;{var allowed;let allowed}",
            "{var sibling}{var sibling;let sibling}",
            "{let shadow}var shadow",
        ] {
            compile_unlinked_script(source).unwrap();
        }
    }

    #[test]
    fn program_var_cross_realm_instantiation_and_fallback_match_quickjs() {
        let runtime = Runtime::new();
        let mut defining = runtime.new_context();
        let mut caller = runtime.new_context();

        let fresh = defining.compile("var crossVar=41;crossVar+1").unwrap();
        assert_eq!(caller.execute(&fresh).unwrap(), Value::Int(42));
        assert_eq!(
            defining.eval("typeof crossVar").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );
        assert_eq!(caller.eval("crossVar").unwrap(), Value::Int(41));

        assert_eq!(
            defining.eval("crossData='A'").unwrap(),
            Value::String(JsString::from_static("A"))
        );
        assert_eq!(
            caller.eval("crossData='B'").unwrap(),
            Value::String(JsString::from_static("B"))
        );
        let data = defining
            .compile("var crossData='written';crossData")
            .unwrap();
        assert_eq!(
            caller.execute(&data).unwrap(),
            Value::String(JsString::from_static("written"))
        );
        assert_eq!(
            defining.eval("crossData").unwrap(),
            Value::String(JsString::from_static("A"))
        );
        assert_eq!(
            caller.eval("crossData").unwrap(),
            Value::String(JsString::from_static("written"))
        );

        assert_eq!(
            defining.eval("Function.aSeen='none'").unwrap(),
            Value::String(JsString::from_static("none"))
        );
        assert_eq!(
            caller.eval("Function.bSeen='none'").unwrap(),
            Value::String(JsString::from_static("none"))
        );
        let Value::Object(a_getter) = defining.eval("(function(){return 'Aget'})").unwrap() else {
            panic!("defining realm accessor getter was not callable");
        };
        let Value::Object(a_setter) = defining
            .eval("(function(value){Function.aSeen=value})")
            .unwrap()
        else {
            panic!("defining realm accessor setter was not callable");
        };
        let Value::Object(b_getter) = caller.eval("(function(){return 'Bget'})").unwrap() else {
            panic!("caller realm accessor getter was not callable");
        };
        let Value::Object(b_setter) = caller
            .eval("(function(value){Function.bSeen=value})")
            .unwrap()
        else {
            panic!("caller realm accessor setter was not callable");
        };
        let a_getter = runtime.as_callable(&a_getter).unwrap().unwrap();
        let a_setter = runtime.as_callable(&a_setter).unwrap().unwrap();
        let b_getter = runtime.as_callable(&b_getter).unwrap().unwrap();
        let b_setter = runtime.as_callable(&b_setter).unwrap().unwrap();
        let accessor = runtime.intern_property_key("crossAccessor").unwrap();
        for (context, getter, setter) in [
            (&mut defining, a_getter, a_setter),
            (&mut caller, b_getter, b_setter),
        ] {
            let global = context.global_object().unwrap();
            assert!(
                context
                    .define_own_property(
                        &global,
                        &accessor,
                        &OrdinaryPropertyDescriptor {
                            get: DescriptorField::Present(AccessorValue::Callable(getter)),
                            set: DescriptorField::Present(AccessorValue::Callable(setter)),
                            enumerable: DescriptorField::Present(true),
                            configurable: DescriptorField::Present(true),
                            ..OrdinaryPropertyDescriptor::new()
                        },
                    )
                    .unwrap()
            );
        }
        let accessor_script = defining
            .compile("var crossAccessor='written';crossAccessor")
            .unwrap();
        assert_eq!(
            caller.execute(&accessor_script).unwrap(),
            Value::String(JsString::from_static("Aget"))
        );
        assert_eq!(
            defining.eval("crossAccessor+'|'+Function.aSeen").unwrap(),
            Value::String(JsString::from_static("Aget|written"))
        );
        assert_eq!(
            caller.eval("crossAccessor+'|'+Function.bSeen").unwrap(),
            Value::String(JsString::from_static("Bget|none"))
        );

        let readonly = runtime.intern_property_key("crossReadonly").unwrap();
        let defining_global = defining.global_object().unwrap();
        assert!(
            defining
                .define_own_property(
                    &defining_global,
                    &readonly,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(1)),
                        writable: DescriptorField::Present(false),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        let caller_global = caller.global_object().unwrap();
        assert!(
            caller
                .define_own_property(
                    &caller_global,
                    &readonly,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Undefined),
                        set: DescriptorField::Present(AccessorValue::Undefined),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        let Value::Object(defining_type_prototype) = defining.eval("TypeError.prototype").unwrap()
        else {
            panic!("defining TypeError.prototype was not an object");
        };
        let readonly_script = defining
            .compile("'use strict';var crossReadonly=2")
            .unwrap();
        assert!(matches!(
            caller.execute(&readonly_script),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
            panic!("cross-realm var initializer did not throw an Error object");
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(defining_type_prototype)
        );

        let mut syntax_caller = runtime.new_context();
        syntax_caller.eval("let crossConflict=1").unwrap();
        let Value::Object(syntax_prototype) = syntax_caller.eval("SyntaxError.prototype").unwrap()
        else {
            panic!("caller SyntaxError.prototype was not an object");
        };
        let conflict = defining.compile("var crossConflict").unwrap();
        assert!(matches!(
            syntax_caller.execute(&conflict),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = syntax_caller.take_exception().unwrap().unwrap() else {
            panic!("cross-realm var conflict did not throw an Error object");
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(syntax_prototype)
        );

        let mut type_caller = runtime.new_context();
        let Value::Object(type_prototype) = type_caller.eval("TypeError.prototype").unwrap() else {
            panic!("caller TypeError.prototype was not an object");
        };
        let type_global = type_caller.global_object().unwrap();
        runtime.prevent_extensions(&type_global).unwrap();
        let missing = defining.compile("var missingCrossVar").unwrap();
        assert!(matches!(
            type_caller.execute(&missing),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = type_caller.take_exception().unwrap().unwrap() else {
            panic!("cross-realm non-extensible var did not throw an Error object");
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(type_prototype)
        );
    }

    #[test]
    fn program_var_function_cell_cycle_is_collectable_after_context_drop() {
        let runtime = Runtime::new();
        {
            let mut context = runtime.new_context();
            assert_eq!(
                context
                    .eval("var cycle=function(){return cycle};cycle()===cycle")
                    .unwrap(),
                Value::Bool(true)
            );
            let counts = runtime.heap_counts();
            assert_eq!(counts.context_nodes, 1);
            assert!(counts.var_ref_nodes > 0);
            assert!(counts.function_bytecode_nodes > 0);
        }

        assert_eq!(runtime.heap_counts().context_nodes, 1);
        runtime.run_gc().unwrap();
        let counts = runtime.heap_counts();
        assert_eq!(counts.context_nodes, 0);
        assert_eq!(counts.object_nodes, 0);
        assert_eq!(counts.shape_nodes, 0);
        assert_eq!(counts.var_ref_nodes, 0);
        assert_eq!(counts.function_bytecode_nodes, 0);
        assert_eq!(counts.live, 0);
    }

    #[test]
    fn program_function_declaration_cycle_is_collectable_after_context_drop() {
        let runtime = Runtime::new();
        {
            let mut context = runtime.new_context();
            assert_eq!(
                context
                    .eval("function declarationCycle(){return declarationCycle};declarationCycle()===declarationCycle")
                    .unwrap(),
                Value::Bool(true)
            );
            let counts = runtime.heap_counts();
            assert_eq!(counts.context_nodes, 1);
            assert!(counts.var_ref_nodes > 0);
            assert!(counts.function_bytecode_nodes > 0);
        }

        assert_eq!(runtime.heap_counts().context_nodes, 1);
        runtime.run_gc().unwrap();
        let counts = runtime.heap_counts();
        assert_eq!(counts.context_nodes, 0);
        assert_eq!(counts.object_nodes, 0);
        assert_eq!(counts.shape_nodes, 0);
        assert_eq!(counts.var_ref_nodes, 0);
        assert_eq!(counts.function_bytecode_nodes, 0);
        assert_eq!(counts.live, 0);
    }

    #[test]
    fn program_global_lexicals_persist_shadow_and_reject_redeclaration() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval("let mutable=1,named=function(){};const fixed=3;mutable+'|'+named.name+'|'+fixed")
                .unwrap(),
            Value::String(JsString::from_static("1|named|3"))
        );
        assert_eq!(context.eval("mutable+=2").unwrap(), Value::Int(3));
        assert_eq!(
            context.eval("typeof globalThis.mutable").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );
        assert_eq!(context.eval("delete mutable").unwrap(), Value::Bool(false));

        let global = context.global_object().unwrap();
        let lexical_environment = context.global_var_object().unwrap();
        for (binding_name, expected_value, writable) in [
            ("mutable", Value::Int(3), true),
            ("fixed", Value::Int(3), false),
        ] {
            let key = runtime.intern_property_key(binding_name).unwrap();
            assert!(context.get_own_property(&global, &key).unwrap().is_none());
            assert_eq!(
                context
                    .get_own_property(&lexical_environment, &key)
                    .unwrap(),
                Some(CompleteOrdinaryPropertyDescriptor::Data {
                    value: expected_value,
                    writable,
                    enumerable: true,
                    configurable: true,
                })
            );
        }

        assert!(matches!(
            context.eval("fixed=4"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("global const write did not throw an Error object");
        };
        let name = runtime.intern_property_key("name").unwrap();
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from_static("TypeError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static("'fixed' is read-only"))
        );

        assert!(matches!(
            context.eval("let mutable=9"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("global lexical redeclaration did not throw an Error object");
        };
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from_static("SyntaxError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static("redeclaration of 'mutable'"))
        );

        assert_eq!(context.eval("shadowedGlobal=1").unwrap(), Value::Int(1));
        assert_eq!(
            context.eval("let shadowedGlobal=2;shadowedGlobal").unwrap(),
            Value::Int(2)
        );
        assert_eq!(
            context.eval("globalThis.shadowedGlobal").unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            context.eval("delete globalThis.shadowedGlobal").unwrap(),
            Value::Bool(true)
        );
        assert_eq!(context.eval("shadowedGlobal").unwrap(), Value::Int(2));

        let mut sealed = runtime.new_context();
        let sealed_global = sealed.global_object().unwrap();
        runtime.prevent_extensions(&sealed_global).unwrap();
        assert_eq!(
            sealed
                .eval("let sealedLexical=6;const sealedConst=7;sealedLexical+sealedConst")
                .unwrap(),
            Value::Int(13)
        );
    }

    #[test]
    fn program_global_lexical_preflight_and_failed_initializers_match_quickjs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let name = runtime.intern_property_key("name").unwrap();
        let message = runtime.intern_property_key("message").unwrap();

        let delayed = context
            .compile("let delayedGlobal=7;delayedGlobal")
            .unwrap();
        assert_eq!(
            context.eval("typeof delayedGlobal").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );
        assert_eq!(context.execute(&delayed).unwrap(), Value::Int(7));
        assert!(matches!(
            context.execute(&delayed),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();

        let atomic = context
            .compile("let untouched=function(){return Infinity},NaN=1,Infinity=2")
            .unwrap();
        assert!(matches!(
            context.execute(&atomic),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("global declaration preflight did not throw an Error object");
        };
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static("redeclaration of 'NaN'"))
        );
        assert_eq!(
            context.eval("typeof untouched").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );
        assert_eq!(
            context.eval("let untouched=4;untouched").unwrap(),
            Value::Int(4)
        );

        assert!(matches!(
            context.eval(
                "Function.saved=function(){return captured};let captured=(function(){throw 17})()"
            ),
            Err(RuntimeError::Exception)
        ));
        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(17)));
        assert!(matches!(
            context.eval("Function.saved()"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("declaring-script capture did not preserve the global lexical TDZ");
        };
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static("captured is not initialized"))
        );
        assert!(matches!(
            context.eval("captured"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("later eval did not materialize a missing-global ReferenceError");
        };
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from_static("ReferenceError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static("'captured' is not defined"))
        );
        assert_eq!(
            context.eval("typeof captured").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );
        assert_eq!(context.eval("delete captured").unwrap(), Value::Bool(false));
        assert!(matches!(
            context.eval("let captured=1"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();

        let mut defining = runtime.new_context();
        let mut caller = runtime.new_context();
        let caller_syntax_prototype = caller.eval("SyntaxError.prototype").unwrap();
        let conflict = defining.compile("let NaN=1").unwrap();
        assert!(matches!(
            caller.execute(&conflict),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
            panic!("cross-realm declaration conflict did not throw an Error object");
        };
        let Value::Object(caller_syntax_prototype) = caller_syntax_prototype else {
            panic!("SyntaxError.prototype was not an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(caller_syntax_prototype)
        );

        let cross_realm = defining
            .compile("let crossRealmBinding=41;crossRealmBinding+1")
            .unwrap();
        assert_eq!(caller.execute(&cross_realm).unwrap(), Value::Int(42));
        assert_eq!(
            defining.eval("typeof crossRealmBinding").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );
        assert_eq!(caller.eval("crossRealmBinding").unwrap(), Value::Int(41));
    }

    #[test]
    fn strip_debug_removes_lexical_tdz_names_but_not_readonly_atoms() {
        let runtime = Runtime::new();
        runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
        let mut context = runtime.new_context();
        for source in [
            "(function(){return localName;let localName=1})()",
            "(function(){return function probe(){return capturedName};let capturedName=1})()()",
            "(function(){var read;outer:{read=function(){return blockName};break outer;let blockName=1;}return read();})()",
            "(function(){var read;switch(1){case 0:let switchName=1;case 1:read=function(){return switchName};break;}return read();})()",
        ] {
            assert_eq!(
                evaluate_error(&runtime, &mut context, source),
                (
                    JsString::from_static("ReferenceError"),
                    JsString::from_static("lexical variable is not initialized")
                )
            );
        }
        assert_eq!(
            evaluate_error(
                &runtime,
                &mut context,
                "(function(){var write;{const retainedName=1;write=function(){retainedName=2};}write();})()"
            ),
            (
                JsString::from_static("TypeError"),
                JsString::from_static("'retainedName' is read-only")
            )
        );
    }

    #[test]
    fn compiles_precedence_directly_to_stack_bytecode() {
        assert_eq!(evaluate("1 + 2 * 3"), Value::Int(7));
        assert_eq!(evaluate("(1 + 2) * 3"), Value::Int(9));
    }

    #[test]
    fn bitwise_operators_follow_quickjs_precedence_and_numeric_semantics() {
        assert_eq!(evaluate("~0"), Value::Int(-1));
        assert_eq!(evaluate("~4294967296"), Value::Int(-1));
        assert_eq!(evaluate("-1.9 & 3.7"), Value::Int(3));
        assert_eq!(evaluate("'7' ^ true"), Value::Int(6));
        assert_eq!(evaluate("1 | 2 ^ 3 & 4"), Value::Int(3));
        assert_eq!(evaluate("1 | 2 === 3"), Value::Int(1));
        assert_eq!(evaluate("null ?? 1 | 2"), Value::Int(3));
        assert_eq!(evaluate("0 || 1 | 2"), Value::Int(3));

        assert_eq!(evaluate("~0n"), Value::BigInt(JsBigInt::from(-1)));
        assert_eq!(evaluate("-1n ^ 255n"), Value::BigInt(JsBigInt::from(-256)));
        assert_eq!(
            evaluate("123456789012345678901234567890n & -1n"),
            Value::BigInt(JsBigInt::parse_js_string("123456789012345678901234567890").unwrap())
        );
    }

    #[test]
    fn shift_operators_follow_quickjs_precedence_and_numeric_semantics() {
        assert_eq!(evaluate("1 << 3"), Value::Int(8));
        assert_eq!(evaluate("-8 >> 2"), Value::Int(-2));
        assert_eq!(evaluate("-1 >>> 0"), Value::Float(4_294_967_295.0));
        assert_eq!(evaluate("1 << 33"), Value::Int(2));
        assert_eq!(evaluate("1 << -1"), Value::Int(i32::MIN));
        assert_eq!(evaluate("4294967295 >> 0"), Value::Int(-1));
        assert_eq!(evaluate("1 + 2 << 3"), Value::Int(24));
        assert_eq!(evaluate("16 >> 1 + 1"), Value::Int(4));
        assert_eq!(evaluate("1 << 2 < 5"), Value::Bool(true));
        assert_eq!(evaluate("8 >> 1 & 3"), Value::Int(0));
        assert_eq!(evaluate("64 >> 2 >> 1"), Value::Int(8));
        assert_eq!(evaluate("1 ?? 2 << 3"), Value::Int(1));

        assert_eq!(
            evaluate("1n << 65n"),
            Value::BigInt(JsBigInt::parse_js_string("36893488147419103232").unwrap())
        );
        assert_eq!(evaluate("-8n >> 2n"), Value::BigInt(JsBigInt::from(-2)));
        assert_eq!(evaluate("8n << -1n"), Value::BigInt(JsBigInt::from(4)));
        assert_eq!(evaluate("8n >> -2n"), Value::BigInt(JsBigInt::from(32)));
    }

    #[test]
    fn exponentiation_follows_quickjs_precedence_associativity_and_unary_rules() {
        assert_eq!(evaluate("2 ** 3 ** 2"), Value::Int(512));
        assert_eq!(evaluate("2 * 3 ** 2"), Value::Int(18));
        assert_eq!(evaluate("2 ** 3 * 4"), Value::Int(32));
        assert_eq!(evaluate("2 ** -2"), Value::Float(0.25));
        assert_eq!(evaluate("(-2) ** 2"), Value::Int(4));
        assert!(evaluate("(typeof 2) ** 2").as_number().unwrap().is_nan());

        assert_eq!(evaluate("0n ** 0n"), Value::BigInt(JsBigInt::one()));
        assert_eq!(evaluate("(-2n) ** 3n"), Value::BigInt(JsBigInt::from(-8)));
        assert_eq!(
            evaluate("2n ** 100n"),
            Value::BigInt(JsBigInt::parse_js_string("1267650600228229401496703205376").unwrap())
        );

        for source in [
            "-2 ** 2",
            "+2 ** 2",
            "!2 ** 2",
            "~2 ** 2",
            "typeof 2 ** 2",
            "void 2 ** 2",
            "delete Function ** 2",
            "2 ** -2 ** 3",
        ] {
            let error = compile_script(source).unwrap_err();
            assert_eq!(
                error.message(),
                "unparenthesized unary expression can't appear on the left-hand side of '**'",
                "source {source:?}"
            );
        }
    }

    #[test]
    fn update_expressions_follow_quickjs_lvalue_and_power_shapes() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        assert_eq!(
            context
                .eval("(function(){ var x = '01'; var old = x++; return old + '|' + x; })()")
                .unwrap(),
            Value::String(JsString::from_static("1|2"))
        );
        assert_eq!(
            context
                .eval("(function(){ var x = 2; return (++x ** 2) * 100 + (x++ ** 2) * 10 + x; })()")
                .unwrap(),
            Value::Int(994)
        );
        assert_eq!(
            context
                .eval("(function(){ var x = 4n; var old = x--; return old * 10n + --x; })()")
                .unwrap(),
            Value::BigInt(JsBigInt::from(42))
        );
        assert_eq!(
            context
                .eval("(function(){ Function.update = '4'; var old = Function.update++; return old + '|' + ++Function.update; })()")
                .unwrap(),
            Value::String(JsString::from_static("4|6"))
        );
        assert_eq!(
            context
                .eval("(function(){ Function['update'] = 5; var old = Function['update']--; return old * 10 + Function.update; })()")
                .unwrap(),
            Value::Int(54)
        );
        assert_eq!(
            context
                .eval("(function(){ var x = 1, y = 2; x\n++y; return x * 10 + y; })()")
                .unwrap(),
            Value::Int(13)
        );

        let prefix_argument = context
            .compile("(function(value){ return ++value; })")
            .unwrap();
        let prefix_argument = runtime
            .test_child_function_bytecode(&prefix_argument, 0)
            .unwrap();
        let prefix_code = runtime.test_function_code(&prefix_argument).unwrap();
        assert!(prefix_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::GetArg(0),
                Instruction::Inc,
                Instruction::SetArg(0)
            ]
        )));

        let postfix_argument = context
            .compile("(function(value){ return value++; })")
            .unwrap();
        let postfix_argument = runtime
            .test_child_function_bytecode(&postfix_argument, 0)
            .unwrap();
        let postfix_code = runtime.test_function_code(&postfix_argument).unwrap();
        assert!(postfix_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::GetArg(0),
                Instruction::PostInc,
                Instruction::PutArg(0)
            ]
        )));

        let fixed = context.compile("Function.update++").unwrap();
        let fixed_code = runtime.test_function_code(&fixed).unwrap();
        assert!(fixed_code.windows(4).any(|window| matches!(
            window,
            [
                Instruction::GetField2(_),
                Instruction::PostInc,
                Instruction::Perm3,
                Instruction::PutField(_)
            ]
        )));

        let computed = context.compile("--Function['update']").unwrap();
        let computed_code = runtime.test_function_code(&computed).unwrap();
        assert!(computed_code.windows(4).any(|window| matches!(
            window,
            [
                Instruction::GetArrayEl3,
                Instruction::Dec,
                Instruction::Insert3,
                Instruction::PutArrayEl
            ]
        )));

        for source in ["++1", "1++", "++(1 + 2)", "(1 + 2)--"] {
            let error = compile_script(source).unwrap_err();
            assert_eq!(error.message(), "invalid increment/decrement operand");
        }
        for source in ["'use strict'; ++eval", "'use strict'; arguments--"] {
            let error = compile_script(source).unwrap_err();
            assert_eq!(error.message(), "invalid lvalue in strict mode");
        }
    }

    #[test]
    fn compiles_primitive_coercion_and_equality() {
        assert_eq!(
            evaluate("'answer: ' + 42"),
            Value::String(JsString::from_static("answer: 42"))
        );
        assert_eq!(evaluate("'42' == 42"), Value::Bool(true));
        assert_eq!(evaluate("'42' === 42"), Value::Bool(false));
    }

    #[test]
    fn compiles_short_circuit_and_conditional_control_flow() {
        assert_eq!(evaluate("false && 42"), Value::Bool(false));
        assert_eq!(
            evaluate("'left' || 'right'"),
            Value::String(JsString::from_static("left"))
        );
        assert_eq!(evaluate("false ? 1 : 2"), Value::Int(2));
        assert!(compile_script("true ? 1, 2 : 3").is_err());
        assert_eq!(evaluate("true ? 1 : 2, 3"), Value::Int(3));
    }

    #[test]
    fn nullish_coalescing_uses_one_quickjs_short_circuit_join() {
        assert_eq!(evaluate("null ?? 42"), Value::Int(42));
        assert_eq!(evaluate("void 0 ?? 7"), Value::Int(7));
        assert_eq!(evaluate("false ?? true"), Value::Bool(false));
        assert_eq!(evaluate("-0 ?? 1"), Value::Float(-0.0));
        assert_eq!(
            evaluate("'' ?? 'fallback'"),
            Value::String(JsString::from_static(""))
        );
        assert_eq!(evaluate("null ?? void 0 ?? 9"), Value::Int(9));
        assert_eq!(evaluate("null ?? 1 + 2 * 3"), Value::Int(7));
        assert_eq!(evaluate("0 ?? 1 ? 2 : 3"), Value::Int(3));

        let chain = compile_script("null ?? void 0 ?? 9").unwrap();
        let targets = chain
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::IfFalse(target) => Some(*target),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0], targets[1]);
        let join = usize::try_from(targets[0]).unwrap();
        assert!(matches!(chain.code[join], Instruction::PutLocal(0)));
        assert!(matches!(chain.code[join + 1], Instruction::GetLocal(0)));
        assert!(matches!(chain.code[join + 2], Instruction::Return));

        for source in ["1 || 2 ?? 3", "1 && 2 ?? 3", "1 ?? 2 || 3", "1 ?? 2 && 3"] {
            assert!(compile_script(source).is_err(), "accepted {source:?}");
        }
        assert_eq!(evaluate("(false || 4) ?? 5"), Value::Int(4));
        assert_eq!(evaluate("null ?? (false || 6)"), Value::Int(6));
        assert_eq!(evaluate("(null ?? 0) || 7"), Value::Int(7));
        assert_eq!(evaluate("false || (null ?? 8)"), Value::Int(8));

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let call = context
            .compile(
                "Function.coalesce = function(){ return this === Function; }; \
                 (Function.coalesce ?? Function)()",
            )
            .unwrap();
        let call_code = runtime.test_function_code(&call).unwrap();
        assert!(
            call_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Call(0)))
        );
        assert!(
            !call_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::CallMethod(_)))
        );
        assert_eq!(
            context
                .eval(
                    "Function.coalesce = function(){ return this === Function; }; \
                     (Function.coalesce ?? Function)()"
                )
                .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            context
                .eval("inferred = null ?? function(){}; inferred.name")
                .unwrap(),
            Value::String(JsString::from_static(""))
        );
        assert_eq!(
            context.eval("1 ?? missingNullishRhs").unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            context
                .eval("Function.combo = 0; Function.combo ||= null ?? 4")
                .unwrap(),
            Value::Int(4)
        );
        assert_eq!(
            context
                .eval("Function.combo = null; Function.combo ??= void 0 ?? 5")
                .unwrap(),
            Value::Int(5)
        );
        assert!(
            context
                .compile("(Function.left ?? Function.right) = 1")
                .is_err()
        );
    }

    #[test]
    fn script_completion_obeys_semicolons_and_asi() {
        assert_eq!(evaluate("1;\n2"), Value::Int(2));
        assert_eq!(evaluate("0\u{2028}1"), Value::Int(1));
        assert_eq!(evaluate("0\u{2029}1"), Value::Int(1));
        assert_eq!(evaluate("0\u{00a0}+1"), Value::Int(1));
        assert!(compile_script("1 2").is_err());
    }

    #[test]
    fn block_and_if_statements_use_the_quickjs_eval_completion_slot() {
        assert_eq!(evaluate(""), Value::Undefined);
        assert_eq!(evaluate("1; {}"), Value::Int(1));
        assert_eq!(evaluate("1; {;;}"), Value::Int(1));
        assert_eq!(evaluate("{ 1; { 2; {} } }"), Value::Int(2));
        assert_eq!(evaluate("1; if (false) 2"), Value::Undefined);
        assert_eq!(evaluate("1; if (true) {}"), Value::Undefined);
        assert_eq!(evaluate("if (true) { 1; 2 } else 3"), Value::Int(2));
        assert_eq!(evaluate("if (false) { 1; 2 } else 3"), Value::Int(3));
        assert_eq!(evaluate("if (true) if (false) 1; else 2"), Value::Int(2));
        assert_eq!(evaluate("{ 'use strict'; } 010"), Value::Int(8));

        assert_eq!(
            evaluate_in_context("(function(x){ if (x) return 1; else return 2; })(0)"),
            Value::Int(2)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){ if (false) { var hidden = 1; } return typeof hidden; })()"
            ),
            Value::String(JsString::from_static("undefined"))
        );
        assert_eq!(
            evaluate_in_context("(function(){ { 'use strict'; } var eval = 7; return eval; })()"),
            Value::Int(7)
        );
        assert_eq!(
            evaluate_in_context(
                "Function.trace = ''; if ((Function.trace += 'c', true)) { Function.trace += 't'; } else { Function.trace += 'f'; } Function.trace"
            ),
            Value::String(JsString::from_static("ct"))
        );

        let bytecode = compile_script("if (true) 1; else 2").unwrap();
        assert!(matches!(bytecode.code.last(), Some(Instruction::Return)));
        assert!(
            bytecode
                .code
                .iter()
                .all(|instruction| !matches!(instruction, Instruction::IfFalse(_)))
        );
        assert!(
            bytecode
                .code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Goto(_)))
        );
        assert!(
            bytecode
                .code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::PutLocal(0)))
        );
        assert!(matches!(
            bytecode.code.get(bytecode.code.len() - 2),
            Some(Instruction::GetLocal(0))
        ));

        for source in [
            "if (false) 1",
            "if (true) 1",
            "if (null) 1",
            "if (void 0) 1",
            "if (0) 1",
            "if (1) 1",
        ] {
            let bytecode = compile_script(source).unwrap();
            assert!(
                bytecode.code.iter().all(|instruction| !matches!(
                    instruction,
                    Instruction::IfFalse(_) | Instruction::IfTrue(_)
                )),
                "QuickJS constant branch did not fold for {source:?}"
            );
        }
        for source in ["if ('') 1", "if (0.5) 1"] {
            let bytecode = compile_script(source).unwrap();
            assert!(
                bytecode
                    .code
                    .iter()
                    .any(|instruction| matches!(instruction, Instruction::IfFalse(_))),
                "QuickJS intentionally does not fold {source:?}"
            );
        }

        let root = compile_unlinked_script("(function(){ 1; })").unwrap();
        assert_eq!(root.metadata().local_count, 1);
        let ordinary = root.constants()[0].as_child().unwrap();
        assert_eq!(ordinary.metadata().local_count, 0);
    }

    #[test]
    fn while_and_do_while_use_per_function_quickjs_loop_controls() {
        assert_eq!(evaluate("1; while (false) 2"), Value::Undefined);
        assert_eq!(evaluate("while (true) { 3; break; }"), Value::Int(3));
        assert_eq!(evaluate("do 4; while (false)"), Value::Int(4));
        assert_eq!(
            evaluate_in_context("do { break; } while (missing)"),
            Value::Undefined
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){ var i=0; var total=0; while(i<5){ i++; if(i===3) continue; total+=i; } return total; })()"
            ),
            Value::Int(12)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){ var i=0; do { i++; if(i<3) continue; } while(i<3); return i; })()"
            ),
            Value::Int(3)
        );

        // Constant folding turns these into closed backward-edge CFGs. They
        // must compile and verify, but deliberately must not be executed.
        for source in ["while(true);", "while(true) continue;", "do{}while(true)"] {
            let bytecode = compile_script(source).unwrap();
            assert!(bytecode.code.iter().enumerate().any(|(pc, instruction)| {
                matches!(instruction, Instruction::Goto(target) if usize::try_from(*target).is_ok_and(|target| target <= pc))
            }));
            assert!(
                bytecode
                    .code
                    .iter()
                    .all(|instruction| !matches!(instruction, Instruction::Goto(u32::MAX)))
            );
        }

        for source in [
            "(function(){ break; })",
            "(function(){ continue; })",
            "while(false) (function(){ break; })",
            "do (function(){ continue; }); while(false)",
        ] {
            assert!(
                compile_unlinked_script(source).is_err(),
                "nested function saw an enclosing loop for {source:?}"
            );
        }
    }

    #[test]
    fn classic_for_uses_quickjs_test_update_and_loop_targets() {
        assert_eq!(evaluate("1; for(;false;) 2"), Value::Undefined);
        assert_eq!(evaluate("for(;;){ 3; break; }"), Value::Int(3));
        assert_eq!(
            evaluate_in_context(
                "(function(){ var sum=0; for(var i=0;i<5;i++){ if(i===2) continue; sum+=i; } return sum; })()"
            ),
            Value::Int(8)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){ var i=0; for(;;i++){ if(i===3) break; } return i; })()"
            ),
            Value::Int(3)
        );
        assert_eq!(
            evaluate_in_context("(function(){ var i=9; for(i=0;i<3;i++); return i; })()"),
            Value::Int(3)
        );
        assert_eq!(
            evaluate_in_context("(function(){ var i=0; for(;i<3;){ i++; } return i; })()"),
            Value::Int(3)
        );

        for source in ["for(;;);", "for(;;) continue;"] {
            let bytecode = compile_script(source).unwrap();
            assert!(bytecode.code.iter().enumerate().any(|(pc, instruction)| {
                matches!(instruction, Instruction::Goto(target) if usize::try_from(*target).is_ok_and(|target| target <= pc))
            }));
            assert!(
                bytecode
                    .code
                    .iter()
                    .all(|instruction| !matches!(instruction, Instruction::Goto(u32::MAX)))
            );
        }

        compile_unlinked_script("for(Function.item in Function);").unwrap();
        for source in [
            "for(Function.item of Function);",
            "for(Function.item of 'a;b');",
            "for(Function.item of `a;b`);",
            "for(Function.item of /a;b/);",
        ] {
            compile_unlinked_script(source).unwrap();
        }
        for source in [
            "for(Function.item in Function;;);",
            "for(Function.item of Function;;);",
        ] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(error.message(), "expecting ';'");
        }
        for source in [
            "for(Function.flag ? Function.item in Function : false;;);",
            "for((Function.item in Function);;);",
            "for(Function(Function.item in Function);;);",
            "for(;false;); Function.item in Function",
        ] {
            let bytecode = compile_unlinked_script(source).unwrap();
            assert!(
                bytecode
                    .code()
                    .iter()
                    .any(|instruction| matches!(instruction, Instruction::In)),
                "AllowIn boundary lost its in opcode for {source:?}"
            );
        }
        for source in [
            "for(Function.flag ? false : Function.item in Function;;);",
            "for(Function.item = Function.key in Function;;);",
        ] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(
                error.message(),
                "expecting ';'",
                "NoIn boundary drifted for {source:?}"
            );
        }
        let destructuring =
            compile_unlinked_script("(function(){ var let=Function; for(let[0]=1;false;); })")
                .unwrap_err();
        assert_eq!(
            destructuring.message(),
            "lexical destructuring bindings are not implemented yet"
        );
        for source in [
            "(function(){ for(let binding=0;false;); })",
            "(function(){ for(let\nbinding=0;false;); })",
            "(function(){ 'use strict'; for(let binding=0;false;); })",
            "(function(){ for(const binding=0;false;); })",
        ] {
            compile_unlinked_script(source)
                .unwrap_or_else(|error| panic!("lexical for head rejected {source:?}: {error}"));
        }
        assert_eq!(
            evaluate_in_context("(function(){var let=0;for(let=0;let<3;let++);return let;})()"),
            Value::Int(3)
        );
        for source in [
            "for(;;) (function(){ break; })",
            "for(;;) (function(){ continue; })",
        ] {
            assert!(
                compile_unlinked_script(source).is_err(),
                "nested function saw an enclosing for loop for {source:?}"
            );
        }
    }

    #[test]
    fn classic_for_lexicals_close_captured_cells_at_quickjs_boundaries() {
        let script = compile_unlinked_script(
            "(function(){var read;for(let value=0;value<1;value++){read=function(){return value}}return read;})",
        )
        .unwrap();
        let outer = script
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("script lost its lexical-for function");
        assert_eq!(
            outer
                .code()
                .iter()
                .filter(|instruction| {
                    matches!(instruction, Instruction::SetLocalUninitialized(1))
                })
                .count(),
            1
        );
        assert_eq!(
            outer
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::CloseLocal(1)))
                .count(),
            3,
            "initializer, normal body fallthrough, and loop exit each need a close site"
        );
        let reader = outer
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("lexical-for function lost its captured reader");
        assert_eq!(
            reader.closure_variables()[0].source,
            ClosureSource::ParentLocal(1)
        );

        let two_binding_script = compile_unlinked_script(
            "(function(){var readLeft,readRight;for(let left=0,right=2;left<1;(left++,right+=left===1?2:1)){readLeft=function(){return left};readRight=function(){return right};}return readLeft()*10+readRight();})()",
        )
        .unwrap();
        let two_binding = two_binding_script
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("script lost its two-binding lexical-for function");
        for index in [2, 3] {
            assert_eq!(
                two_binding
                    .code()
                    .iter()
                    .filter(|instruction| matches!(instruction, Instruction::CloseLocal(found) if *found == index))
                    .count(),
                3,
                "captured head local {index} lost one static close site"
            );
        }
        assert!(two_binding.code().iter().all(|instruction| !matches!(
            instruction,
            Instruction::Goto(u32::MAX)
                | Instruction::IfFalse(u32::MAX)
                | Instruction::IfTrue(u32::MAX)
        )));
        assert_eq!(
            evaluate_in_context(
                "(function(){var readLeft,readRight;for(let left=0,right=2;left<1;(left++,right+=left===1?2:1)){readLeft=function(){return left};readRight=function(){return right};}return readLeft()*10+readRight();})()"
            ),
            Value::Int(2)
        );

        assert_eq!(
            evaluate_in_context(
                "(function(){var first,second,third;for(let value=0;value<3;value++){if(value===0)first=function(){return value};else if(value===1)second=function(){return value};else third=function(){return value};}return first()*100+second()*10+third();})()"
            ),
            Value::Int(12)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var first,second,third;for(let value=0;value<3;value++){if(value===0)first=function(){return value};else if(value===1)second=function(){return value};else third=function(){return value};continue;}return first()*100+second()*10+third();})()"
            ),
            Value::Int(333)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var first,second,third;for(let value=0;value<3;value++){if(value===0)first=function(){return value};else if(value===1)second=function(){return value};else third=function(){return value};if(value===0)continue;}return first()*100+second()*10+third();})()"
            ),
            Value::Int(112)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var initial,body;for(let value=(initial=function(){return value},0);value<1;value++){body=function(){return value};value=5;}return initial()*10+body();})()"
            ),
            Value::Int(5)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var body0,update0,body1;for(let value=0;value<2;(update0=update0||function(){return value},value++)){if(value===0)body0=function(){return value};else body1=function(){return value};}return body0()*100+update0()*10+body1();})()"
            ),
            Value::Int(11)
        );
        assert_eq!(
            evaluate_in_context(
                "Function.saved=undefined;for(let value=0;value<1;value++){Function.saved=function(){return value};}Function.saved()*10+(typeof value==='undefined')"
            ),
            Value::Int(1)
        );
    }

    #[test]
    fn classic_for_lexicals_match_tdz_const_shadow_and_conflict_rules() {
        assert_eq!(
            evaluate_in_context(
                "(function(){let value=9,result;for(let value=0;value<1;value++)result=value;return value*10+result;})()"
            ),
            Value::Int(90)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var total=0;for(let left=0,right=3;left<right;left++,right--)total+=left+right;return total;})()"
            ),
            Value::Int(6)
        );
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for (source, name) in [
            ("(function(){for(let value=value;false;);})()", "value"),
            (
                "(function(){for(let first=second,second=1;false;);})()",
                "second",
            ),
        ] {
            assert_eq!(
                evaluate_error(&runtime, &mut context, source),
                (
                    JsString::from_static("ReferenceError"),
                    JsString::try_from_utf8(&format!("{name} is not initialized")).unwrap()
                ),
                "{source}"
            );
        }
        for (source, message) in [
            (
                "(function(){for(let value=0;value<1;value++){var value;}})",
                "invalid redefinition of lexical identifier",
            ),
            (
                "(function(){for(let value=0,value=1;false;);})",
                "invalid redefinition of lexical identifier",
            ),
            (
                "(function(){for(const value;false;);})",
                "missing initializer for const variable",
            ),
        ] {
            assert_eq!(
                compile_unlinked_script(source).unwrap_err().message(),
                message,
                "{source}"
            );
        }
        for source in [
            "(function(value){for(let value=0;false;);return value;})(7)",
            "(function(){var value=7;for(let value=0;false;);return value;})()",
            "(function(){for(let value=0;false;);var value=7;return value;})()",
        ] {
            assert_eq!(evaluate_in_context(source), Value::Int(7), "{source}");
        }
    }

    #[test]
    fn labels_use_per_function_quickjs_break_control_search() {
        assert_eq!(evaluate("plain: 6;"), Value::Int(6));
        assert_eq!(evaluate("7; empty: ;"), Value::Int(7));
        assert_eq!(evaluate("outer: { 2; break outer; 3; }"), Value::Int(2));
        assert_eq!(
            evaluate_in_context(
                "(function(){ var i=0; var x=0; outer: for(;i<3;i++){ while(true){ x++; continue outer; } } return i+'|'+x; })()"
            ),
            Value::String(JsString::from_static("3|3"))
        );
        assert_eq!(
            evaluate("first: { 1; break first; } first: 2;"),
            Value::Int(2)
        );

        for source in [
            "duplicate: { duplicate: 1; }",
            "regular: { continue regular; }",
            "outer: inner: while(true){ continue outer; }",
            "outer: while(false) (function(){ break outer; })",
        ] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::Syntax),
                "label error was not a SyntaxError for {source:?}: {error}"
            );
        }
        let duplicate = compile_unlinked_script("duplicate: { duplicate: 1; }").unwrap_err();
        assert_eq!(duplicate.message(), "duplicate label name");
        let multiple =
            compile_unlinked_script("outer: inner: while(true){ continue outer; }").unwrap_err();
        assert_eq!(multiple.message(), "break/continue label not found");

        let bytecode = compile_script("outer: while(true){ break outer; }").unwrap();
        assert!(
            bytecode
                .code
                .iter()
                .all(|instruction| !matches!(instruction, Instruction::Goto(u32::MAX)))
        );
    }

    #[test]
    fn switch_uses_quickjs_case_fallthrough_and_abrupt_cleanup() {
        assert_eq!(evaluate("1; switch(0){}"), Value::Undefined);
        assert_eq!(
            evaluate("switch(2){case 1: 1; case 2: 2; case 3: 3;}"),
            Value::Int(3)
        );
        assert_eq!(
            evaluate("switch(9){case 1: 1; default: 4; case 2: 2;}"),
            Value::Int(2)
        );
        assert_eq!(
            evaluate("switch(1){case 1: 1; default: 4; case 2: 2;}"),
            Value::Int(2)
        );
        assert_eq!(
            evaluate("switch(2){case 1: 1; default: 4; case 2: 2;}"),
            Value::Int(2)
        );
        assert_eq!(
            evaluate("switch('1'){case 1: 1; default: 2;}"),
            Value::Int(2)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var log='';switch((log+='s',2)){case (log+='a',1):log+='A';break;case (log+='b',2):log+='B';break;case (log+='c',3):log+='C';}return log})()"
            ),
            Value::String(JsString::from_static("sabB"))
        );
        assert_eq!(
            evaluate("outer: while(true){switch(1){case 1: break outer;}} 7"),
            Value::Int(7)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var i=0;outer:while(i++<2){switch(i){case 1:continue outer;default:break;}}return i})()"
            ),
            Value::Int(3)
        );
        assert_eq!(
            evaluate_in_context("(function(){switch(1){case 1:return 4;default:return 5;}})()"),
            Value::Int(4)
        );

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert!(matches!(
            context.eval("switch(1){case 1:throw 4;}"),
            Err(RuntimeError::Exception)
        ));
        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(4)));

        let bytecode =
            compile_unlinked_script("switch(Function){case Function:1;break;default:2;}").unwrap();
        assert!(
            bytecode
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::StrictEq))
        );
        assert!(
            bytecode
                .code()
                .iter()
                .all(|instruction| !matches!(instruction, Instruction::Goto(u32::MAX)))
        );

        for (source, message) in [
            ("switch(0){ 1; }", "invalid switch statement"),
            ("switch(0){default:1;default:2;}", "duplicate default"),
            ("switch(0){case 0 1;}", "expecting ':'"),
            (
                "switch(0){case 0:continue;}",
                "continue must be inside loop",
            ),
        ] {
            assert_eq!(
                compile_unlinked_script(source).unwrap_err().message(),
                message
            );
        }
    }

    #[test]
    fn relational_membership_uses_runtime_object_protocols() {
        assert_eq!(
            evaluate_in_context("'prototype' in Function"),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_in_context("'missingMembershipKey' in Function"),
            Value::Bool(false)
        );
        assert_eq!(
            evaluate_in_context("'toString' in Function"),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_in_context("Function instanceof Function"),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_in_context("(function(){}) instanceof Function"),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_in_context("1 instanceof Function"),
            Value::Bool(false)
        );
        assert_eq!(
            evaluate_in_context("(function(){}).bind(null) instanceof Function"),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){ var target=function DeepTarget(){}; var bound=target; for(var i=0;i<512;i++) bound=bound.bind(null); return 1 instanceof bound; })()"
            ),
            Value::Bool(false)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){ var result=false; for((result='prototype' in Function);false;); return result; })()"
            ),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){ var result=false; for(result=Function instanceof Function;false;); return result; })()"
            ),
            Value::Bool(true)
        );

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context
            .eval(
                "Function.membershipTrace=''; Function[Symbol.toPrimitive]=function(hint){ Function.membershipTrace+=hint; return 'prototype'; };",
            )
            .unwrap();
        assert!(matches!(
            context.eval("Function in (Function.membershipTrace+='R',1)"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert_eq!(
            context.eval("Function.membershipTrace").unwrap(),
            Value::String(JsString::from_static("R"))
        );
        assert_eq!(
            context.eval("Function in Function").unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            context.eval("Function.membershipTrace").unwrap(),
            Value::String(JsString::from_static("Rstring"))
        );
    }

    #[test]
    fn untagged_templates_follow_quickjs_concat_lowering() {
        assert_eq!(
            evaluate("`plain`"),
            Value::String(JsString::from_static("plain"))
        );
        assert_eq!(
            evaluate_in_context("`a${1 + 2}b${4}c`"),
            Value::String(JsString::from_static("a3b4c"))
        );
        assert_eq!(
            evaluate_in_context("`a${1, 2}b`"),
            Value::String(JsString::from_static("a2b"))
        );
        assert_eq!(
            evaluate_in_context("`a${`b${1}c`}d`"),
            Value::String(JsString::from_static("ab1cd"))
        );
        assert_eq!(evaluate_in_context("`x${8 / 2}y`.length"), Value::Int(3));

        let no_substitution = compile_script("`plain`").unwrap();
        assert!(!no_substitution.code.iter().any(|instruction| matches!(
            instruction,
            Instruction::GetField2(_) | Instruction::CallMethod(_)
        )));

        let interpolated = compile_unlinked_script("`a${1}b${2}c`").unwrap();
        assert!(
            interpolated
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetField2(_)))
        );
        assert!(
            interpolated
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::CallMethod(4)))
        );

        let invalid = compile_script("`\\8`").unwrap_err();
        assert_eq!(
            invalid.message(),
            "malformed escape sequence in string literal"
        );
        assert_eq!(
            compile_script("tag`x`").unwrap_err().message(),
            "tagged template literals are not implemented yet"
        );
        assert_eq!(
            compile_script("tag\n`x`").unwrap_err().message(),
            "tagged template literals are not implemented yet"
        );
    }

    #[test]
    fn detached_vm_rejects_runtime_global_execution_explicitly() {
        let error = compile_script("answer").unwrap_err();
        assert!(error.message().contains("global-environment"));

        let error = compile_script("delete answer").unwrap_err();
        assert!(error.message().contains("global-environment"));
    }

    #[test]
    fn runtime_global_get_and_direct_typeof_use_the_bytecode_realm() {
        let runtime = Runtime::new();
        let mut defining_context = runtime.new_context();
        let mut caller_context = runtime.new_context();
        let answer = runtime.intern_property_key("answer").unwrap();
        let marker = runtime.intern_property_key("marker").unwrap();
        let descriptor = |value| OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(value),
            writable: DescriptorField::Present(true),
            enumerable: DescriptorField::Present(true),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };
        assert!(
            defining_context
                .define_own_property(
                    &defining_context.global_object().unwrap(),
                    &answer,
                    &descriptor(Value::Int(1)),
                )
                .unwrap()
        );
        defining_context
            .create_global_lexical_for_test("answer", false, Some(Value::Int(2)))
            .unwrap();
        assert_eq!(defining_context.eval("answer").unwrap(), Value::Int(2));
        assert_eq!(
            defining_context.eval("typeof answer").unwrap(),
            Value::String(JsString::from_static("number"))
        );
        assert_eq!(
            defining_context.eval("typeof missingGlobal").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );
        assert_eq!(
            defining_context.eval("typeof ((missingGlobal))").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );
        assert!(matches!(
            defining_context.eval("typeof (0, missingGlobal)"),
            Err(RuntimeError::Exception)
        ));
        assert!(matches!(
            defining_context.take_exception().unwrap(),
            Some(Value::Object(_))
        ));

        let marker_object = defining_context.new_object().unwrap();
        assert!(
            defining_context
                .define_own_property(
                    &defining_context.global_object().unwrap(),
                    &marker,
                    &descriptor(Value::Object(marker_object.clone())),
                )
                .unwrap()
        );
        let Value::Object(function) = defining_context
            .eval("(0, function(){ return marker; })")
            .unwrap()
        else {
            panic!("global-realm probe did not produce a function");
        };
        let callable = runtime.as_callable(&function).unwrap().unwrap();
        assert_eq!(
            caller_context
                .call(&callable, Value::Undefined, &[])
                .unwrap(),
            Value::Object(marker_object)
        );

        assert!(matches!(
            caller_context.eval("missingGlobal"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = caller_context.take_exception().unwrap().unwrap() else {
            panic!("missing global did not materialize a ReferenceError");
        };
        let message = runtime.intern_property_key("message").unwrap();
        assert!(matches!(
            caller_context.get_property(&exception, &message).unwrap(),
            Value::String(value) if value == JsString::from_static("'missingGlobal' is not defined")
        ));
    }

    #[test]
    fn global_put_matches_strict_sloppy_readonly_and_setter_semantics() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let global = context.global_object().unwrap();
        let descriptor = |value, writable| OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(value),
            writable: DescriptorField::Present(writable),
            enumerable: DescriptorField::Present(true),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };

        assert_eq!(context.eval("created = 7").unwrap(), Value::Int(7));
        assert_eq!(context.eval("created").unwrap(), Value::Int(7));

        let readonly = runtime.intern_property_key("readonly").unwrap();
        assert!(
            context
                .define_own_property(&global, &readonly, &descriptor(Value::Int(1), false))
                .unwrap()
        );
        assert_eq!(context.eval("readonly = 2").unwrap(), Value::Int(2));
        assert_eq!(context.eval("readonly").unwrap(), Value::Int(1));
        assert!(matches!(
            context.eval("'use strict'; readonly = 2"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("strict read-only global assignment did not throw an object");
        };
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from_static("'readonly' is read-only"))
        );
        assert_eq!(context.eval("readonly += 2").unwrap(), Value::Int(3));
        assert_eq!(context.eval("readonly").unwrap(), Value::Int(1));
        assert_eq!(
            context.eval("'use strict'; readonly ||= 9").unwrap(),
            Value::Int(1)
        );
        assert!(matches!(
            context.eval("'use strict'; readonly += 2"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert!(matches!(
            context.eval("'use strict'; readonly &&= 2"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();

        let inherited = runtime.intern_property_key("inheritedReadOnly").unwrap();
        assert!(
            context
                .define_own_property(
                    &context.object_prototype().unwrap(),
                    &inherited,
                    &descriptor(Value::Int(5), false),
                )
                .unwrap()
        );
        assert_eq!(
            context.eval("inheritedReadOnly = 6").unwrap(),
            Value::Int(6)
        );
        assert_eq!(context.eval("inheritedReadOnly").unwrap(), Value::Int(5));
        assert!(matches!(
            context.eval("'use strict'; inheritedReadOnly = 6"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("strict inherited read-only assignment did not throw an object");
        };
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from_static("'inheritedReadOnly' is read-only"))
        );

        let no_setter = runtime.intern_property_key("noSetter").unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &no_setter,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Undefined),
                        set: DescriptorField::Present(AccessorValue::Undefined),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(context.eval("noSetter = 8").unwrap(), Value::Int(8));
        assert!(matches!(
            context.eval("'use strict'; noSetter = 8"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("strict setter-less assignment did not throw an object");
        };
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from_static("no setter for property"))
        );
        assert_eq!(context.eval("noSetter ||= 8").unwrap(), Value::Int(8));
        assert!(matches!(
            context.eval("'use strict'; noSetter ||= 8"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert_eq!(
            context.eval("'use strict'; noSetter &&= 8").unwrap(),
            Value::Undefined
        );

        assert!(matches!(
            context.eval("'use strict'; trulyMissing = 1"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        let truly_missing = runtime.intern_property_key("trulyMissing").unwrap();
        assert!(!runtime.has_own_property(&global, &truly_missing).unwrap());

        let sink = runtime.intern_property_key("sink").unwrap();
        assert!(
            context
                .define_own_property(&global, &sink, &descriptor(Value::Int(0), true))
                .unwrap()
        );
        let Value::Object(setter) = context
            .eval("(function(v) { sink = v; return 99; })")
            .unwrap()
        else {
            panic!("setter source did not produce a function");
        };
        let setter = runtime.as_callable(&setter).unwrap().unwrap();
        let target = runtime.intern_property_key("setterTarget").unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &target,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Undefined),
                        set: DescriptorField::Present(AccessorValue::Callable(setter)),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(context.eval("setterTarget = 42").unwrap(), Value::Int(42));
        assert_eq!(context.eval("sink").unwrap(), Value::Int(42));
        assert_eq!(context.eval("setterTarget ||= 17").unwrap(), Value::Int(17));
        assert_eq!(context.eval("sink").unwrap(), Value::Int(17));

        let Value::Object(getter) = context.eval("(function() { return this; })").unwrap() else {
            panic!("getter source did not produce a function");
        };
        let getter = runtime.as_callable(&getter).unwrap().unwrap();
        let getter_target = runtime.intern_property_key("getterTarget").unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &getter_target,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        set: DescriptorField::Present(AccessorValue::Undefined),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(
            context.eval("getterTarget").unwrap(),
            Value::Object(global.clone())
        );
        assert_eq!(
            context.eval("typeof getterTarget").unwrap(),
            Value::String(JsString::from_static("object"))
        );

        let Value::Object(throwing_getter) = context.eval("(function() { throw 17; })").unwrap()
        else {
            panic!("throwing getter source did not produce a function");
        };
        let throwing_getter = runtime.as_callable(&throwing_getter).unwrap().unwrap();
        let throwing_target = runtime.intern_property_key("throwingGetter").unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &throwing_target,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(throwing_getter)),
                        set: DescriptorField::Present(AccessorValue::Undefined),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert!(matches!(
            context.eval("typeof throwingGetter"),
            Err(RuntimeError::Exception)
        ));
        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(17)));

        assert_eq!(context.eval("compoundSide = 0").unwrap(), Value::Int(0));
        assert!(matches!(
            context.eval("missingCompound += (compoundSide = 1)"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert_eq!(context.eval("compoundSide").unwrap(), Value::Int(0));
        assert!(matches!(
            context.eval("missingLogical ||= (compoundSide = 2)"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert_eq!(context.eval("compoundSide").unwrap(), Value::Int(0));
    }

    #[test]
    fn global_lexical_tdz_const_shadow_and_initialization_share_the_resolved_cell() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let global = context.global_object().unwrap();
        let shadowed = runtime.intern_property_key("shadowed").unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &shadowed,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(1)),
                        writable: DescriptorField::Present(true),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        let Value::Object(reader) = context.eval("(function() { return shadowed; })").unwrap()
        else {
            panic!("lexical reader source did not produce a function");
        };
        let reader = runtime.as_callable(&reader).unwrap().unwrap();

        context
            .create_global_lexical_for_test("shadowed", true, None)
            .unwrap();
        assert_eq!(context.eval("delete shadowed").unwrap(), Value::Bool(false));
        assert_eq!(
            context.get_property(&global, &shadowed).unwrap(),
            Value::Int(1)
        );
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.call(&reader, Value::Undefined, &[]).unwrap(),
            Value::Int(1),
            "a precompiled ordinary global descriptor falls back to the global object while the later lexical is uninitialized"
        );

        context
            .initialize_global_lexical_for_test("shadowed", Value::Int(2))
            .unwrap();
        assert_eq!(
            context.call(&reader, Value::Undefined, &[]).unwrap(),
            Value::Int(2)
        );
        let Value::Object(writer) = context.eval("(function() { shadowed = 3; })").unwrap() else {
            panic!("lexical writer source did not produce a function");
        };
        let writer = runtime.as_callable(&writer).unwrap().unwrap();
        assert!(matches!(
            context.call(&writer, Value::Undefined, &[]),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("const assignment did not throw an object");
        };
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from_static("'shadowed' is read-only"))
        );
        assert_eq!(
            context.get_property(&global, &shadowed).unwrap(),
            Value::Int(1)
        );
        assert_eq!(context.eval("shadowed ||= 9").unwrap(), Value::Int(2));
        assert!(matches!(
            context.eval("shadowed += 3"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("const compound assignment did not throw an object");
        };
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from_static("'shadowed' is read-only"))
        );
        assert!(matches!(
            context.eval("shadowed &= 3"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("const bitwise compound assignment did not throw an object");
        };
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from_static("'shadowed' is read-only"))
        );
        assert!(matches!(
            context.eval("shadowed <<= 1"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("const shift compound assignment did not throw an object");
        };
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from_static("'shadowed' is read-only"))
        );
        assert!(matches!(
            context.eval("shadowed **= 3"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("const exponent compound assignment did not throw an object");
        };
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from_static("'shadowed' is read-only"))
        );
        assert!(matches!(
            context.eval("shadowed &&= 3"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();

        context
            .create_global_lexical_for_test("mutableLexical", false, None)
            .unwrap();
        assert_eq!(
            context.eval("typeof mutableLexical").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );
        assert!(matches!(
            context.eval("mutableLexical += 1"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert!(matches!(
            context.eval("mutableLexical |= 1"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert!(matches!(
            context.eval("mutableLexical **= 2"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        context
            .initialize_global_lexical_for_test("mutableLexical", Value::Int(4))
            .unwrap();
        assert_eq!(context.eval("mutableLexical |= 8").unwrap(), Value::Int(12));
        assert_eq!(context.eval("mutableLexical ^= 3").unwrap(), Value::Int(15));
        assert_eq!(context.eval("mutableLexical &= 7").unwrap(), Value::Int(7));
        assert_eq!(context.eval("mutableLexical += 3").unwrap(), Value::Int(10));
        assert_eq!(context.eval("mutableLexical &&= 5").unwrap(), Value::Int(5));
        assert_eq!(context.eval("mutableLexical ??= 9").unwrap(), Value::Int(5));
        assert_eq!(
            context.eval("mutableLexical **= 2").unwrap(),
            Value::Int(25)
        );
        assert_eq!(context.eval("mutableLexical").unwrap(), Value::Int(25));

        context
            .create_global_lexical_for_test("mutableShift", false, None)
            .unwrap();
        assert!(matches!(
            context.eval("mutableShift >>>= 1"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        context
            .initialize_global_lexical_for_test("mutableShift", Value::Int(-8))
            .unwrap();
        assert_eq!(context.eval("mutableShift >>= 1").unwrap(), Value::Int(-4));
        assert_eq!(
            context.eval("mutableShift >>>= 1").unwrap(),
            Value::Int(2_147_483_646)
        );
        assert_eq!(context.eval("mutableShift <<= 1").unwrap(), Value::Int(-4));
    }

    #[test]
    fn unresolved_name_compiles_to_one_global_then_parent_global_relays() {
        let script = compile_unlinked_script(
            "(function() { return function() { return function() { return relayName; }; }; })",
        )
        .unwrap();
        let mut function = &script;
        for depth in 0..4 {
            let descriptor = function
                .closure_variables()
                .first()
                .expect("every function on the unresolved-name path needs a closure slot");
            assert_eq!(
                descriptor.source,
                if depth == 0 {
                    ClosureSource::Global
                } else {
                    ClosureSource::ParentGlobal(0)
                }
            );
            let ClosureVariableName::Constant(name_index) = descriptor.name else {
                panic!("unlinked global relay did not retain a name constant");
            };
            assert!(matches!(
                function.constants()[name_index as usize].as_primitive(),
                Some(Value::String(name)) if name == &JsString::from_static("relayName")
            ));
            if depth == 3 {
                assert!(matches!(
                    function.code(),
                    [Instruction::GetVar(0), Instruction::Return, ..]
                ));
                break;
            }
            function = function
                .constants()
                .iter()
                .find_map(|constant| constant.as_child())
                .expect("global relay path lost its nested child");
        }
    }

    #[test]
    fn late_global_property_delete_reconnect_and_cross_realm_use_the_defining_realm() {
        let runtime = Runtime::new();
        let mut defining = runtime.new_context();
        let mut caller = runtime.new_context();
        let Value::Object(reader) = defining
            .eval("(function() { return lateRealmValue; })")
            .unwrap()
        else {
            panic!("late global reader source did not produce a function");
        };
        let reader = runtime.as_callable(&reader).unwrap().unwrap();
        let key = runtime.intern_property_key("lateRealmValue").unwrap();
        let descriptor = |value| OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::Int(value)),
            writable: DescriptorField::Present(true),
            enumerable: DescriptorField::Present(true),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };
        assert!(
            caller
                .define_own_property(&caller.global_object().unwrap(), &key, &descriptor(9),)
                .unwrap()
        );
        let defining_global = defining.global_object().unwrap();
        assert!(
            defining
                .define_own_property(&defining_global, &key, &descriptor(1))
                .unwrap()
        );
        runtime.run_gc().unwrap();
        assert_eq!(
            caller.call(&reader, Value::Undefined, &[]).unwrap(),
            Value::Int(1)
        );

        assert!(runtime.delete_property(&defining_global, &key).unwrap());
        runtime.run_gc().unwrap();
        assert!(matches!(
            caller.call(&reader, Value::Undefined, &[]),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = caller.take_exception().unwrap().unwrap() else {
            panic!("missing defining-realm global did not throw an object");
        };
        let reference_error = runtime.intern_property_key("ReferenceError").unwrap();
        let prototype = runtime.intern_property_key("prototype").unwrap();
        let Value::Object(reference_error) = defining
            .get_property(&defining_global, &reference_error)
            .unwrap()
        else {
            panic!("defining realm ReferenceError was not an object");
        };
        let Value::Object(reference_error_prototype) =
            defining.get_property(&reference_error, &prototype).unwrap()
        else {
            panic!("defining realm ReferenceError.prototype was not an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&exception).unwrap(),
            Some(reference_error_prototype)
        );
        assert!(
            defining
                .define_own_property(&defining_global, &key, &descriptor(2))
                .unwrap()
        );
        runtime.run_gc().unwrap();
        assert_eq!(
            caller.call(&reader, Value::Undefined, &[]).unwrap(),
            Value::Int(2)
        );
    }

    #[test]
    fn runtime_compiler_executes_anonymous_iife_parameters_and_direct_call() {
        let source = "(function(a, b) { return a + b; })(20, 22)";
        assert_eq!(evaluate_in_context(source), Value::Int(42));

        let detached_error = compile_script(source).unwrap_err();
        assert!(
            detached_error
                .message()
                .contains("requires runtime publication")
        );

        let script = compile_unlinked_script("(function(a, b) {})").unwrap();
        let function = script.constants()[0].as_child().unwrap();
        assert_eq!(function.metadata().argument_count, 2);
        assert_eq!(function.metadata().defined_argument_count, 2);
        assert!(function.metadata().has_prototype);
        assert_eq!(function.metadata().constructor_kind, ConstructorKind::Base);

        let runtime = Runtime::new();
        let Value::Object(function) = runtime.new_context().eval("(function() {})").unwrap() else {
            panic!("function expression did not produce an object");
        };
        assert!(runtime.is_constructor(&function).unwrap());
    }

    #[test]
    fn direct_function_body_declarations_hoist_into_argument_and_local_bindings() {
        for (source, expected) in [
            (
                "(function(){return before();function before(){return 1}})()",
                Value::Int(1),
            ),
            (
                "(function(parameter){function local(){return later}function parameter(){return 10}function local(){return later+1}let later=2;return parameter()+local()})(0)",
                Value::Int(13),
            ),
            (
                "(function(){function arguments(){return 4}return arguments()})()",
                Value::Int(4),
            ),
            (
                "(function(){var arguments;function arguments(){return 6}return arguments()})()",
                Value::Int(6),
            ),
            (
                "(function(){function arguments(){return 7}var arguments;return arguments()})()",
                Value::Int(7),
            ),
            (
                "(function(){var arguments=function(){return 9};function arguments(){return 8}return arguments()})()",
                Value::Int(9),
            ),
            (
                "(function named(){function named(){return 5}return named()})()",
                Value::Int(5),
            ),
            (
                "(function(){'use strict';function mutable(){mutable=6;return mutable}return mutable()})()",
                Value::Int(6),
            ),
            (
                "(function(){if(false)return 0;function branch(){return 2}var count=0;while(count<1)count++;return branch()+count})()",
                Value::Int(3),
            ),
        ] {
            assert_eq!(evaluate_in_context(source), expected, "{source}");
        }

        let script = compile_unlinked_script(
            "(function(first,second){var local;function local(){}function second(){}function first(){}function local(){return 1}})",
        )
        .unwrap();
        let outer = script.constants()[0].as_child().unwrap();
        assert!(matches!(outer.code()[0], Instruction::FClosure(2)));
        assert!(matches!(outer.code()[1], Instruction::PutArg(0)));
        assert!(matches!(outer.code()[2], Instruction::FClosure(1)));
        assert!(matches!(outer.code()[3], Instruction::PutArg(1)));
        assert!(matches!(outer.code()[4], Instruction::FClosure(3)));
        assert!(matches!(outer.code()[5], Instruction::PutLocal(0)));
        for child in outer
            .constants()
            .iter()
            .filter_map(|value| value.as_child())
        {
            assert_eq!(child.metadata().function_name_local, None);
        }
    }

    #[test]
    fn direct_function_body_declaration_conflicts_match_quickjs_order() {
        for (source, message) in [
            (
                "(function(){function conflict(){};let conflict})",
                "invalid redefinition of a variable",
            ),
            (
                "(function(){let conflict;function conflict(){}})",
                "invalid redefinition of lexical identifier",
            ),
            (
                "(function(conflict){function conflict(){};let conflict})",
                "invalid redefinition of parameter name",
            ),
            (
                "(function(){'use strict';function eval(){}})",
                "invalid function name in strict code",
            ),
        ] {
            assert_eq!(
                compile_unlinked_script(source).unwrap_err().message(),
                message,
                "{source}"
            );
        }
    }

    #[test]
    fn scoped_function_declarations_keep_entry_and_annex_closures_distinct() {
        for (source, expected) in [
            (
                "(function(){var inside;{function f(){return f}inside=f}return (inside!==f)+'|'+(inside()===inside)+'|'+(f()===inside)})()",
                "true|true|true",
            ),
            (
                "(function(){var inside;{function f(){return 1}function f(){return 2}inside=f}return inside()+'|'+f()+'|'+(inside===f)})()",
                "2|1|false",
            ),
            (
                "(function(){'use strict';var inside;{function f(){return 3}inside=f}return typeof f+'|'+inside()})()",
                "undefined|3",
            ),
            (
                "(function(parameter){var inside;{function parameter(){return 4}inside=parameter}return parameter+'|'+inside()})(1)",
                "1|4",
            ),
            (
                "(function(){var g;{f=8;function f(){}g=f}return g+'|'+typeof f+'|'+f.name})()",
                "8|function|f",
            ),
            (
                "(function(){var a,b,i=0;while(i<2){let x=i;function f(){return x}if(i===0)a=f;else b=f;i++}return (a!==b)+'|'+a()+'|'+b()})()",
                "true|0|1",
            ),
            (
                "(function(){var trace=typeof f;switch(0){case (trace+='|'+typeof f,0):function f(){return 5}}return trace+'|'+f()})()",
                "undefined|function|5",
            ),
            (
                "(function(){var original=function self(){{function self(){return 6}}return self};var replacement=original();return (replacement!==original)+'|'+replacement()})()",
                "true|6",
            ),
        ] {
            assert_eq!(
                evaluate_in_context(source),
                Value::String(JsString::from_static(expected)),
                "{source}"
            );
        }

        let script = compile_unlinked_script(
            "(function(){{function duplicate(){return 1}function duplicate(){return 2}}return duplicate()})",
        )
        .unwrap();
        let outer = script.constants()[0].as_child().unwrap();
        let closures = outer
            .code()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::FClosure(constant) => Some(*constant),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(closures, [1, 0, 0, 1]);
        assert!(outer.code().windows(4).any(|window| matches!(
            window,
            [
                Instruction::FClosure(0),
                Instruction::Dup,
                Instruction::PutLocal(_),
                Instruction::Drop,
            ]
        )));
        assert!(!outer.code().windows(3).any(|window| matches!(
            window,
            [
                Instruction::FClosure(1),
                Instruction::Dup,
                Instruction::PutLocal(_),
            ]
        )));

        let arguments_script =
            compile_unlinked_script("(function(){{function arguments(){return 3}}return 1})")
                .unwrap();
        let arguments_outer = arguments_script.constants()[0].as_child().unwrap();
        assert_eq!(arguments_outer.local_definitions().len(), 1);
        assert_eq!(
            arguments_outer.local_definitions()[0].name.as_ref(),
            Some(&JsString::from_static("arguments"))
        );
        assert!(arguments_outer.local_definitions()[0].is_lexical);
        assert!(
            !arguments_outer
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Dup)),
            "implicit arguments name incorrectly created an Annex root write"
        );

        let shadow_script = compile_unlinked_script(
            "(function(){let shadow=12;{function shadow(){return 13}}return shadow})",
        )
        .unwrap();
        let shadow_outer = shadow_script.constants()[0].as_child().unwrap();
        assert_eq!(shadow_outer.local_definitions().len(), 2);
        assert!(
            shadow_outer
                .local_definitions()
                .iter()
                .all(|definition| definition.is_lexical)
        );
        assert!(
            !shadow_outer
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Dup)),
            "prior enclosing lexical incorrectly allowed an Annex root write"
        );
    }

    #[test]
    fn scoped_function_conflicts_and_global_annex_order_match_quickjs() {
        for (source, message) in [
            (
                "(function(){{let conflict;function conflict(}})",
                "invalid redefinition of lexical identifier",
            ),
            (
                "(function(){'use strict';{function duplicate(){}function duplicate(}})",
                "invalid redefinition of lexical identifier",
            ),
            (
                "(function(){{var conflict;function conflict(){}}})",
                "invalid redefinition of a variable",
            ),
            (
                "(function(){{function conflict(){}var conflict;}})",
                "invalid redefinition of lexical identifier",
            ),
        ] {
            assert_eq!(
                compile_unlinked_script(source).unwrap_err().message(),
                message,
                "{source}"
            );
        }

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context.eval("{function lateGlobalLexical(){}}let lateGlobalLexical;"),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("Annex B global lexical collision did not throw an Error object");
        };
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static(
                "lateGlobalLexical is not initialized"
            ))
        );

        assert_eq!(
            context
                .eval(
                    "let priorGlobalLexical=7;{function priorGlobalLexical(){return 8}}\
                     typeof globalThis.priorGlobalLexical+'|'+priorGlobalLexical"
                )
                .unwrap(),
            Value::String(JsString::from_static("undefined|7"))
        );
    }

    #[test]
    fn annex_b_single_and_labelled_declarations_preserve_scope_shape() {
        let program = compile_unlinked_script(
            "programLabel:function programLabel(){return programLabel};programLabel",
        )
        .unwrap();
        assert_eq!(
            program.local_definitions().len(),
            1,
            "Program labelled functions must not allocate a lexical local"
        );
        assert!(program.code().windows(4).any(|window| matches!(
            window,
            [
                Instruction::FClosure(_),
                Instruction::Dup,
                Instruction::PutVar(_),
                Instruction::PutVar(_),
            ]
        )));

        let body = compile_unlinked_script(
            "(function(){bodyLabel:function bodyLabel(){return 3};return bodyLabel})",
        )
        .unwrap();
        let body = body.constants()[0].as_child().unwrap();
        assert_eq!(body.local_definitions().len(), 2);
        assert!(body.local_definitions()[0].is_lexical);
        assert!(!body.local_definitions()[1].is_lexical);
        assert!(body.code().windows(4).any(|window| matches!(
            window,
            [
                Instruction::FClosure(_),
                Instruction::Dup,
                Instruction::PutLocal(_),
                Instruction::Drop,
            ]
        )));

        let parameter = compile_unlinked_script(
            "(function(parameter){label:function parameter(){return 4};return parameter})",
        )
        .unwrap();
        let parameter = parameter.constants()[0].as_child().unwrap();
        assert_eq!(parameter.local_definitions().len(), 1);
        assert!(parameter.local_definitions()[0].is_lexical);
        assert!(
            !parameter
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Dup)),
            "same-name parameter incorrectly received an Annex root write"
        );

        let duplicate = compile_unlinked_script(
            "(function(){if(true)function duplicate(){return 1}else function duplicate(){return 2};return duplicate})",
        )
        .unwrap();
        let duplicate = duplicate.constants()[0].as_child().unwrap();
        assert_eq!(
            duplicate
                .local_definitions()
                .iter()
                .filter(|definition| definition.is_lexical)
                .count(),
            2
        );
        assert_eq!(
            duplicate
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::Dup))
                .count(),
            1,
            "only the first same-scope if declaration is Annex-eligible"
        );

        for source in [
            "var prior;label:function prior(){}",
            "function prior(){}label:function prior(){}",
            "let prior;label:function prior(){}",
        ] {
            assert_eq!(
                compile_unlinked_script(source).unwrap_err().message(),
                "invalid redefinition of global identifier",
                "{source}"
            );
        }
        compile_unlinked_script("{var nested;}label:function nested(){}")
            .expect("a nested first var must not block the Program label exception");
    }

    #[test]
    fn source_members_preserve_quickjs_reads_keys_references_and_method_receivers() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let global = context.global_object().unwrap();
        let data = |value| OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(value),
            writable: DescriptorField::Present(true),
            enumerable: DescriptorField::Present(true),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };

        let base = context.new_object().unwrap();
        for (name, value) in [("x", Value::Int(7)), ("default", Value::Int(8))] {
            let key = runtime.intern_property_key(name).unwrap();
            assert!(
                context
                    .define_own_property(&base, &key, &data(value))
                    .unwrap()
            );
        }
        let base_name = runtime.intern_property_key("base").unwrap();
        assert!(
            context
                .define_own_property(&global, &base_name, &data(Value::Object(base.clone())))
                .unwrap()
        );

        let Value::Object(method) = context
            .eval("(function(){ return this === base; })")
            .unwrap()
        else {
            panic!("method source did not produce a function");
        };
        let method_key = runtime.intern_property_key("m").unwrap();
        assert!(
            context
                .define_own_property(&base, &method_key, &data(Value::Object(method)))
                .unwrap()
        );

        let Value::Object(getter) = context.eval("(function(){ return this; })").unwrap() else {
            panic!("getter source did not produce a function");
        };
        let getter = runtime.as_callable(&getter).unwrap().unwrap();
        let getter_key = runtime.intern_property_key("receiver").unwrap();
        assert!(
            context
                .define_own_property(
                    &base,
                    &getter_key,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        set: DescriptorField::Present(AccessorValue::Undefined),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );

        assert_eq!(context.eval("base.x").unwrap(), Value::Int(7));
        assert_eq!(context.eval("base['x']").unwrap(), Value::Int(7));
        assert_eq!(context.eval("base.default").unwrap(), Value::Int(8));
        assert_eq!(context.eval("base\n.x").unwrap(), Value::Int(7));
        assert_eq!(context.eval("base\n['x']").unwrap(), Value::Int(7));
        assert_eq!(
            context.eval("base.receiver === base").unwrap(),
            Value::Bool(true)
        );
        assert_eq!(context.eval("base.m()").unwrap(), Value::Bool(true));
        assert_eq!(context.eval("base['m']()").unwrap(), Value::Bool(true));
        assert_eq!(context.eval("((base.m))()").unwrap(), Value::Bool(true));
        assert_eq!(context.eval("(0, base.m)()").unwrap(), Value::Bool(false));
        assert_eq!(
            context.eval("(true ? base.m : base.m)()").unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            context.eval("(true && base.m)()").unwrap(),
            Value::Bool(false)
        );

        let hint = runtime.intern_property_key("keyHint").unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &hint,
                    &data(Value::String(JsString::from_static("none"))),
                )
                .unwrap()
        );
        let Value::Object(to_key) = context
            .eval("(function(hint){ keyHint = hint; return 'x'; })")
            .unwrap()
        else {
            panic!("ToPropertyKey source did not produce a function");
        };
        let key_object = context.new_object().unwrap();
        let to_primitive =
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
        assert!(
            context
                .define_own_property(&key_object, &to_primitive, &data(Value::Object(to_key)))
                .unwrap()
        );
        let key_name = runtime.intern_property_key("keyObject").unwrap();
        assert!(
            context
                .define_own_property(&global, &key_name, &data(Value::Object(key_object)),)
                .unwrap()
        );
        assert_eq!(context.eval("base[keyObject]").unwrap(), Value::Int(7));
        assert_eq!(
            context.eval("keyHint").unwrap(),
            Value::String(JsString::from_static("string"))
        );
        assert_eq!(
            context.eval("keyHint = 'none'").unwrap(),
            Value::String(JsString::from_static("none"))
        );
        assert!(matches!(
            context.eval("null[keyObject]"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert_eq!(
            context.eval("keyHint").unwrap(),
            Value::String(JsString::from_static("none"))
        );

        assert_eq!(context.eval("'abc'.length").unwrap(), Value::Int(3));
        assert_eq!(
            context.eval("'abc'[1]").unwrap(),
            Value::String(JsString::from_static("b"))
        );
        assert!(matches!(
            context.eval("Function().toString()"),
            Ok(Value::String(_))
        ));
    }

    #[test]
    fn member_assignment_and_delete_lower_through_quickjs_lvalue_shapes() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        let fixed = context.compile("Function.fixed = 1").unwrap();
        let fixed_code = runtime.test_function_code(&fixed).unwrap();
        assert!(
            fixed_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::Insert2, Instruction::PutField(_)]))
        );
        assert!(
            !fixed_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetField(_)))
        );

        let computed = context.compile("Function['computed'] = 2").unwrap();
        let computed_code = runtime.test_function_code(&computed).unwrap();
        assert!(
            computed_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::Insert3, Instruction::PutArrayEl]))
        );
        assert!(
            !computed_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetArrayEl))
        );

        let fixed_compound = context.compile("Function.fixed += 3").unwrap();
        let fixed_compound_code = runtime.test_function_code(&fixed_compound).unwrap();
        assert!(
            fixed_compound_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetField2(_)))
        );
        assert!(fixed_compound_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::Add,
                Instruction::Insert2,
                Instruction::PutField(_)
            ]
        )));

        let computed_compound = context.compile("Function['computed'] += 4").unwrap();
        let computed_compound_code = runtime.test_function_code(&computed_compound).unwrap();
        assert!(
            computed_compound_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetArrayEl3))
        );
        assert!(computed_compound_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::Add,
                Instruction::Insert3,
                Instruction::PutArrayEl
            ]
        )));

        let fixed_delete = context.compile("delete Function.fixed").unwrap();
        let fixed_delete_code = runtime.test_function_code(&fixed_delete).unwrap();
        assert!(
            fixed_delete_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Delete))
        );
        assert!(
            !fixed_delete_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetField(_)))
        );

        assert_eq!(
            context
                .eval("Function.paren = 1; (Function.paren) = 2; Function.paren")
                .unwrap(),
            Value::Int(2)
        );
        assert!(context.compile("(0, Function.fixed) = 1").is_err());
        assert!(
            context
                .compile("(true ? Function.fixed : Function.fixed) = 1")
                .is_err()
        );

        assert_eq!(
            context
                .eval("Function.keep = 3; delete (0, Function.keep); Function.keep")
                .unwrap(),
            Value::Int(3)
        );
        assert_eq!(
            context
                .eval("Function.gone = 4; delete (Function.gone); Function.gone")
                .unwrap(),
            Value::Undefined
        );
        assert!(context.compile("'use strict'; delete Function").is_err());
        let direct_delete = context.compile("delete __qjo_delete_global").unwrap();
        assert!(
            runtime
                .test_function_code(&direct_delete)
                .unwrap()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::DeleteVar(0)))
        );
    }

    #[test]
    fn direct_identifier_delete_uses_quickjs_scope_resolution() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        assert_eq!(
            context.eval("delete __qjo_missing_delete").unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            context
                .eval("(function(){ var value = 1; return delete value; })()")
                .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            context
                .eval("(function(value){ return delete value; })(1)")
                .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            context
                .eval("(function(value){ return (function(){ return delete value; })(); })(1)")
                .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            context
                .eval("(function named(){ return delete named; })()")
                .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            context
                .eval("(function(){ return delete arguments; })()")
                .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            context
                .eval("(function(){ var value = 1; return delete (value); })()")
                .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            context
                .eval("(function(){ var value = 1; return delete (0, value); })()")
                .unwrap(),
            Value::Bool(true)
        );

        assert_eq!(
            context
                .eval("__qjo_delete_global = 1; delete __qjo_delete_global")
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            context.eval("typeof __qjo_delete_global").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );

        let Value::Object(reader) = context
            .eval("(function(){ return __qjo_delete_reconnect; })")
            .unwrap()
        else {
            panic!("delete/reconnect probe did not produce a function");
        };
        let reader = runtime.as_callable(&reader).unwrap().unwrap();
        assert_eq!(
            context.eval("__qjo_delete_reconnect = 1").unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            context.call(&reader, Value::Undefined, &[]).unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            context.eval("delete __qjo_delete_reconnect").unwrap(),
            Value::Bool(true)
        );
        assert!(matches!(
            context.call(&reader, Value::Undefined, &[]),
            Err(RuntimeError::Exception)
        ));
        assert!(context.take_exception().unwrap().is_some());
        assert_eq!(
            context.eval("__qjo_delete_reconnect = 2").unwrap(),
            Value::Int(2)
        );
        assert_eq!(
            context.call(&reader, Value::Undefined, &[]).unwrap(),
            Value::Int(2)
        );

        for name in ["undefined", "NaN", "Infinity"] {
            assert_eq!(
                context.eval(&format!("delete {name}")).unwrap(),
                Value::Bool(false),
                "global constant {name}"
            );
        }

        let mut caller = runtime.new_context();
        let realm_key = runtime.intern_property_key("__qjo_delete_realm").unwrap();
        let descriptor = OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::Int(1)),
            writable: DescriptorField::Present(true),
            enumerable: DescriptorField::Present(true),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };
        assert!(
            context
                .define_own_property(&context.global_object().unwrap(), &realm_key, &descriptor)
                .unwrap()
        );
        assert!(
            caller
                .define_own_property(&caller.global_object().unwrap(), &realm_key, &descriptor)
                .unwrap()
        );
        let Value::Object(deleter) = context
            .eval("(function(){ return delete __qjo_delete_realm; })")
            .unwrap()
        else {
            panic!("cross-realm delete source did not produce a function");
        };
        let deleter = runtime.as_callable(&deleter).unwrap().unwrap();
        assert_eq!(
            caller.call(&deleter, Value::Undefined, &[]).unwrap(),
            Value::Bool(true)
        );
        assert!(
            !runtime
                .has_own_property(&context.global_object().unwrap(), &realm_key)
                .unwrap()
        );
        assert!(
            runtime
                .has_own_property(&caller.global_object().unwrap(), &realm_key)
                .unwrap()
        );

        let global = context.compile("delete __qjo_delete_opcode").unwrap();
        assert!(
            runtime
                .test_function_code(&global)
                .unwrap()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::DeleteVar(0)))
        );
        let local_root = context
            .compile("(function(value){ return delete value; })")
            .unwrap();
        let local = runtime
            .test_child_function_bytecode(&local_root, 0)
            .unwrap();
        assert!(
            runtime
                .test_function_code(&local)
                .unwrap()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::PushFalse))
        );

        for source in [
            "'use strict'; delete direct",
            "'use strict'; delete (direct)",
        ] {
            let error = compile_script(source).unwrap_err();
            assert_eq!(
                error.message(),
                "cannot delete a direct reference in strict mode"
            );
            assert_eq!(
                error.span().unwrap().start.column,
                u32::try_from(source.len() + 1).unwrap()
            );
        }
    }

    #[test]
    fn bitwise_compound_assignment_reuses_quickjs_lvalue_shapes() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        let fixed = context.compile("Function.bits &= 3").unwrap();
        let fixed_code = runtime.test_function_code(&fixed).unwrap();
        assert!(fixed_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::BitAnd,
                Instruction::Insert2,
                Instruction::PutField(_)
            ]
        )));

        let computed = context.compile("Function['bits'] ^= 4").unwrap();
        let computed_code = runtime.test_function_code(&computed).unwrap();
        assert!(computed_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::BitXor,
                Instruction::Insert3,
                Instruction::PutArrayEl
            ]
        )));

        let identifier_root = context
            .compile("(function(value){ value |= 8; return value; })")
            .unwrap();
        let identifier = runtime
            .test_child_function_bytecode(&identifier_root, 0)
            .unwrap();
        let identifier_code = runtime.test_function_code(&identifier).unwrap();
        assert!(
            identifier_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::BitOr, Instruction::SetArg(0)]))
        );

        let local_root = context
            .compile("(function(){ var value = 7; value &= 3; return value; })")
            .unwrap();
        let local = runtime
            .test_child_function_bytecode(&local_root, 0)
            .unwrap();
        let local_code = runtime.test_function_code(&local).unwrap();
        assert!(
            local_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::BitAnd, Instruction::SetLocal(0)]))
        );

        let closure_root = context
            .compile("(function(value){ return function(){ value ^= 3; return value; }; })")
            .unwrap();
        let closure_outer = runtime
            .test_child_function_bytecode(&closure_root, 0)
            .unwrap();
        let closure = runtime
            .test_child_function_bytecode(&closure_outer, 0)
            .unwrap();
        let closure_code = runtime.test_function_code(&closure).unwrap();
        assert!(
            closure_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::BitXor, Instruction::SetVarRef(0)]))
        );

        let global = context.compile("__qjo_bit_global |= 8").unwrap();
        let global_code = runtime.test_function_code(&global).unwrap();
        assert!(
            global_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetVar(_)))
        );
        assert!(global_code.windows(3).any(|window| matches!(
            window,
            [Instruction::BitOr, Instruction::Dup, Instruction::PutVar(_)]
        )));

        let sloppy_private_root = context
            .compile("(function named(){ named &= 1; return named; })")
            .unwrap();
        let sloppy_private = runtime
            .test_child_function_bytecode(&sloppy_private_root, 0)
            .unwrap();
        let sloppy_private_code = runtime.test_function_code(&sloppy_private).unwrap();
        assert!(
            sloppy_private_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::BitAnd, Instruction::Nop]))
        );

        let strict_private_root = context
            .compile("(function named(){ 'use strict'; named |= 1; })")
            .unwrap();
        let strict_private = runtime
            .test_child_function_bytecode(&strict_private_root, 0)
            .unwrap();
        let strict_private_code = runtime.test_function_code(&strict_private).unwrap();
        assert!(
            strict_private_code.windows(2).any(|window| matches!(
                window,
                [Instruction::BitOr, Instruction::ThrowReadOnly(_)]
            ))
        );

        assert_eq!(
            context
                .eval(
                    "(function(){ var value = 14; value &= 11; value ^= 3; \
                     value |= 4; return value; })()"
                )
                .unwrap(),
            Value::Int(13)
        );
        assert_eq!(
            context
                .eval("(function(value){ value |= 8; return value; })(1)")
                .unwrap(),
            Value::Int(9)
        );
        assert_eq!(
            context
                .eval(
                    "(function(value){ return (function(){ value ^= 3; \
                     return value; })(); })(5)"
                )
                .unwrap(),
            Value::Int(6)
        );
        assert_eq!(
            context
                .eval(
                    "Function.bits = 14; Function.bits &= 11; \
                     Function['bits'] ^= 3; Function.bits |= 4"
                )
                .unwrap(),
            Value::Int(13)
        );
        assert_eq!(
            context
                .eval(
                    "(function(){ var left = 1, right = 3; \
                     left |= right &= 2; return left * 10 + right; })()"
                )
                .unwrap(),
            Value::Int(32)
        );
        assert_eq!(
            context
                .eval(
                    "(function(){ var value = -1n; (value) &= \
                     123456789012345678901234567890n; return value; })()"
                )
                .unwrap(),
            Value::BigInt(JsBigInt::parse_js_string("123456789012345678901234567890").unwrap())
        );
        assert_eq!(
            context
                .eval("(function(value){ (value) |= 2; return value; })(1)")
                .unwrap(),
            Value::Int(3)
        );

        assert!(context.compile("(Function.bits) |= 1").is_ok());
        assert!(context.compile("(bitwiseIdentifier) &= 1").is_ok());
        assert!(context.compile("(0, Function.bits) |= 1").is_err());
        assert!(
            context
                .compile("(true ? Function.bits : Function.bits) |= 1")
                .is_err()
        );
        assert!(context.compile("(Function.bits & 1) |= 1").is_err());
    }

    #[test]
    fn shift_compound_assignment_reuses_quickjs_lvalue_shapes() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        let fixed = context.compile("Function.shift <<= 3").unwrap();
        let fixed_code = runtime.test_function_code(&fixed).unwrap();
        assert!(fixed_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::Shl,
                Instruction::Insert2,
                Instruction::PutField(_)
            ]
        )));

        let computed = context.compile("Function['shift'] >>= 2").unwrap();
        let computed_code = runtime.test_function_code(&computed).unwrap();
        assert!(computed_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::Sar,
                Instruction::Insert3,
                Instruction::PutArrayEl
            ]
        )));

        let argument_root = context
            .compile("(function(value){ value >>>= 1; return value; })")
            .unwrap();
        let argument = runtime
            .test_child_function_bytecode(&argument_root, 0)
            .unwrap();
        let argument_code = runtime.test_function_code(&argument).unwrap();
        assert!(
            argument_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::Shr, Instruction::SetArg(0)]))
        );

        let closure_root = context
            .compile("(function(value){ return function(){ value >>= 2; return value; }; })")
            .unwrap();
        let closure_outer = runtime
            .test_child_function_bytecode(&closure_root, 0)
            .unwrap();
        let closure = runtime
            .test_child_function_bytecode(&closure_outer, 0)
            .unwrap();
        let closure_code = runtime.test_function_code(&closure).unwrap();
        assert!(
            closure_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::Sar, Instruction::SetVarRef(0)]))
        );

        let global = context.compile("__qjo_shift_global <<= 1").unwrap();
        let global_code = runtime.test_function_code(&global).unwrap();
        assert!(global_code.windows(3).any(|window| matches!(
            window,
            [Instruction::Shl, Instruction::Dup, Instruction::PutVar(_)]
        )));

        assert_eq!(
            context
                .eval(
                    "(function(){ var value = 3; value <<= 2; value >>= 1; \
                     value >>>= 1; return value; })()"
                )
                .unwrap(),
            Value::Int(3)
        );
        assert_eq!(
            context
                .eval(
                    "Function.shift = -8; Function.shift >>= 1; \
                     Function['shift'] >>>= 1"
                )
                .unwrap(),
            Value::Int(2_147_483_646)
        );
        assert_eq!(
            context
                .eval(
                    "(function(){ var left = 1, right = 3; \
                     left <<= right >>= 1; return left * 10 + right; })()"
                )
                .unwrap(),
            Value::Int(21)
        );
        assert_eq!(
            context
                .eval(
                    "(function(value){ return (function(){ value >>= 2; \
                     return value; })(); })(-8)"
                )
                .unwrap(),
            Value::Int(-2)
        );

        assert!(context.compile("(Function.shift) >>>= 1").is_ok());
        assert!(context.compile("(shiftIdentifier) <<= 1").is_ok());
        assert!(context.compile("(0, Function.shift) >>= 1").is_err());
        assert!(context.compile("(Function.shift << 1) >>= 1").is_err());
    }

    #[test]
    fn exponent_compound_assignment_reuses_quickjs_lvalue_shapes() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        let fixed = context.compile("Function.power **= 3").unwrap();
        let fixed_code = runtime.test_function_code(&fixed).unwrap();
        assert!(fixed_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::Pow,
                Instruction::Insert2,
                Instruction::PutField(_)
            ]
        )));

        let computed = context.compile("Function['power'] **= 2").unwrap();
        let computed_code = runtime.test_function_code(&computed).unwrap();
        assert!(computed_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::Pow,
                Instruction::Insert3,
                Instruction::PutArrayEl
            ]
        )));

        let argument_root = context
            .compile("(function(value){ value **= 3; return value; })")
            .unwrap();
        let argument = runtime
            .test_child_function_bytecode(&argument_root, 0)
            .unwrap();
        let argument_code = runtime.test_function_code(&argument).unwrap();
        assert!(
            argument_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::Pow, Instruction::SetArg(0)]))
        );

        let closure_root = context
            .compile("(function(value){ return function(){ value **= 2; return value; }; })")
            .unwrap();
        let closure_outer = runtime
            .test_child_function_bytecode(&closure_root, 0)
            .unwrap();
        let closure = runtime
            .test_child_function_bytecode(&closure_outer, 0)
            .unwrap();
        let closure_code = runtime.test_function_code(&closure).unwrap();
        assert!(
            closure_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::Pow, Instruction::SetVarRef(0)]))
        );

        let global = context.compile("__qjo_power_global **= 2").unwrap();
        let global_code = runtime.test_function_code(&global).unwrap();
        assert!(global_code.windows(3).any(|window| matches!(
            window,
            [Instruction::Pow, Instruction::Dup, Instruction::PutVar(_)]
        )));

        let sloppy_private_root = context
            .compile("(function named(){ named **= 1; return named; })")
            .unwrap();
        let sloppy_private = runtime
            .test_child_function_bytecode(&sloppy_private_root, 0)
            .unwrap();
        let sloppy_private_code = runtime.test_function_code(&sloppy_private).unwrap();
        assert!(
            sloppy_private_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::Pow, Instruction::Nop]))
        );

        let strict_private_root = context
            .compile("(function named(){ 'use strict'; named **= 1; })")
            .unwrap();
        let strict_private = runtime
            .test_child_function_bytecode(&strict_private_root, 0)
            .unwrap();
        let strict_private_code = runtime.test_function_code(&strict_private).unwrap();
        assert!(
            strict_private_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::Pow, Instruction::ThrowReadOnly(_)]))
        );

        assert_eq!(
            context
                .eval("(function(){ var value = 2; value **= 3; return value; })()")
                .unwrap(),
            Value::Int(8)
        );
        assert_eq!(
            context
                .eval(
                    "(function(value){ return (function(){ value **= 2; \
                     return value; })(); })(3)"
                )
                .unwrap(),
            Value::Int(9)
        );
        assert_eq!(
            context
                .eval(
                    "(function(){ var left = 2, right = 3; \
                     left **= right **= 2; return left + right; })()"
                )
                .unwrap(),
            Value::Int(521)
        );
        assert_eq!(
            context
                .eval("(function(){ var value = 2n; (value) **= 100n; return value; })()")
                .unwrap(),
            Value::BigInt(JsBigInt::parse_js_string("1267650600228229401496703205376").unwrap())
        );

        assert!(context.compile("(Function.power) **= 2").is_ok());
        assert!(context.compile("(powerIdentifier) **= 2").is_ok());
        assert!(context.compile("(0, Function.power) **= 2").is_err());
        assert!(context.compile("(Function.power ** 1) **= 2").is_err());
    }

    #[test]
    fn logical_member_assignment_uses_quickjs_branch_cleanup_shapes() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        let fixed = context.compile("Function.fixed &&= 3").unwrap();
        let fixed_code = runtime.test_function_code(&fixed).unwrap();
        assert!(
            fixed_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetField2(_)))
        );
        assert!(
            fixed_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::Insert2, Instruction::PutField(_)]))
        );
        let fixed_branch = fixed_code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::IfFalse(_)))
            .unwrap();
        let Instruction::IfFalse(fixed_short) = fixed_code[fixed_branch] else {
            unreachable!();
        };
        assert!(matches!(
            fixed_code[usize::try_from(fixed_short).unwrap()],
            Instruction::Nip
        ));
        assert_eq!(
            fixed_code
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::Nip))
                .count(),
            1
        );

        let computed = context.compile("Function['computed'] ||= 4").unwrap();
        let computed_code = runtime.test_function_code(&computed).unwrap();
        assert!(
            computed_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetArrayEl3))
        );
        assert!(
            computed_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::Insert3, Instruction::PutArrayEl]))
        );
        let computed_branch = computed_code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::IfTrue(_)))
            .unwrap();
        let Instruction::IfTrue(computed_short) = computed_code[computed_branch] else {
            unreachable!();
        };
        let computed_short = usize::try_from(computed_short).unwrap();
        assert!(matches!(computed_code[computed_short], Instruction::Nip));
        assert!(matches!(
            computed_code[computed_short + 1],
            Instruction::Nip
        ));
        assert_eq!(
            computed_code
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::Nip))
                .count(),
            2
        );
        let computed_goto = computed_code
            .iter()
            .find_map(|instruction| match instruction {
                Instruction::Goto(target) => Some(usize::try_from(*target).unwrap()),
                _ => None,
            })
            .unwrap();
        assert_eq!(computed_goto, computed_short + 2);

        let nullish = context.compile("Function.nullish ??= 5").unwrap();
        let nullish_code = runtime.test_function_code(&nullish).unwrap();
        assert!(nullish_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::Dup,
                Instruction::IsUndefinedOrNull,
                Instruction::IfFalse(_)
            ]
        )));

        assert_eq!(
            context
                .eval("Function.logic = 0; Function.logic ||= 7")
                .unwrap(),
            Value::Int(7)
        );
        assert_eq!(
            context
                .eval("Function.logic = 0; Function.logic &&= 8")
                .unwrap(),
            Value::Int(0)
        );
        assert_eq!(
            context
                .eval("Function.logic = null; Function.logic ??= 9")
                .unwrap(),
            Value::Int(9)
        );
        assert_eq!(
            context
                .eval(
                    "Function.left = 1; Function.right = 0; \
                     Function.left &&= Function.right ||= 9; \
                     Function.left + Function.right"
                )
                .unwrap(),
            Value::Int(18)
        );
        assert_eq!(
            context
                .eval(
                    "Function.outer = 1; Function.inner = 0; \
                     Function['outer'] += (Function['inner'] ||= 2); \
                     Function.outer + Function.inner"
                )
                .unwrap(),
            Value::Int(5)
        );

        let logical_call = context
            .compile("(Function.callable ||= function(){ return this === Function; })()")
            .unwrap();
        let logical_call_code = runtime.test_function_code(&logical_call).unwrap();
        assert!(
            logical_call_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Call(0)))
        );
        assert!(
            !logical_call_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::CallMethod(_)))
        );
        assert_eq!(
            context
                .eval("(Function.callable ||= function(){ return this === Function; })()")
                .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            context
                .eval("delete Function.anon; (Function.anon ??= function(){}).name")
                .unwrap(),
            Value::String(JsString::from_static(""))
        );

        assert!(context.compile("(Function.fixed) &&= 1").is_ok());
        assert!(context.compile("(0, Function.fixed) &&= 1").is_err());
        assert!(
            context
                .compile("(true ? Function.fixed : Function.fixed) &&= 1")
                .is_err()
        );
        assert!(
            context
                .compile("(Function.fixed || Function.computed) &&= 1")
                .is_err()
        );
    }

    #[test]
    fn identifier_compound_assignment_uses_resolved_get_set_paths() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        let argument_root = context
            .compile("(function(value){ value += 2; return value; })")
            .unwrap();
        let argument = runtime
            .test_child_function_bytecode(&argument_root, 0)
            .unwrap();
        let argument_code = runtime.test_function_code(&argument).unwrap();
        assert!(
            argument_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetArg(0)))
        );
        assert!(
            argument_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::SetArg(0)))
        );

        let local_root = context
            .compile("(function(){ var value = 1; value ||= 4; return value; })")
            .unwrap();
        let local = runtime
            .test_child_function_bytecode(&local_root, 0)
            .unwrap();
        let local_code = runtime.test_function_code(&local).unwrap();
        assert!(
            local_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetLocal(0)))
        );
        assert!(
            local_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::SetLocal(0)))
        );
        let branch = local_code
            .iter()
            .find_map(|instruction| match instruction {
                Instruction::IfTrue(target) => Some(usize::try_from(*target).unwrap()),
                _ => None,
            })
            .unwrap();
        let end = local_code
            .iter()
            .find_map(|instruction| match instruction {
                Instruction::Goto(target) => Some(usize::try_from(*target).unwrap()),
                _ => None,
            })
            .unwrap();
        assert_eq!(branch, end);
        assert!(
            !local_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Nip))
        );

        let closure_root = context
            .compile("(function(value){ return function(){ value += 2; return value; }; })")
            .unwrap();
        let closure_outer = runtime
            .test_child_function_bytecode(&closure_root, 0)
            .unwrap();
        let closure_inner = runtime
            .test_child_function_bytecode(&closure_outer, 0)
            .unwrap();
        let closure_code = runtime.test_function_code(&closure_inner).unwrap();
        assert!(
            closure_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetVarRef(0)))
        );
        assert!(
            closure_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::SetVarRef(0)))
        );

        let global = context.compile("identifierCompoundGlobal ||= 2").unwrap();
        let global_code = runtime.test_function_code(&global).unwrap();
        assert!(
            global_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetVar(_)))
        );
        assert!(
            global_code
                .windows(2)
                .any(|window| matches!(window, [Instruction::Dup, Instruction::PutVar(_)]))
        );
        let global_branch = global_code
            .iter()
            .find_map(|instruction| match instruction {
                Instruction::IfTrue(target) => Some(*target),
                _ => None,
            })
            .unwrap();
        let global_end = global_code
            .iter()
            .find_map(|instruction| match instruction {
                Instruction::Goto(target) => Some(*target),
                _ => None,
            })
            .unwrap();
        assert_eq!(global_branch, global_end);

        let sloppy_self_root = context
            .compile("(function named(){ named += ''; return named; })")
            .unwrap();
        let sloppy_self = runtime
            .test_child_function_bytecode(&sloppy_self_root, 0)
            .unwrap();
        let sloppy_self_code = runtime.test_function_code(&sloppy_self).unwrap();
        assert!(
            sloppy_self_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Nop))
        );
        assert!(
            !sloppy_self_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::SetLocal(_)))
        );

        let strict_self_root = context
            .compile("(function named(){ 'use strict'; named &&= 1; })")
            .unwrap();
        let strict_self = runtime
            .test_child_function_bytecode(&strict_self_root, 0)
            .unwrap();
        let strict_self_code = runtime.test_function_code(&strict_self).unwrap();
        assert!(
            strict_self_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::ThrowReadOnly(_)))
        );
        assert!(
            !strict_self_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::SetLocal(_)))
        );

        assert_eq!(
            context
                .eval("(function(value){ value += 2; return value; })(3)")
                .unwrap(),
            Value::Int(5)
        );
        assert_eq!(
            context
                .eval(
                    "(function(){ var value = 20; value += 2; value -= 4; \
                     value *= 3; value /= 2; value %= 5; return value; })()"
                )
                .unwrap(),
            Value::Int(2)
        );
        assert_eq!(
            context
                .eval(
                    "(function(value){ return (function(){ value += 2; return value; })() \
                     + (function(){ value += 3; return value; })(); })(1)"
                )
                .unwrap(),
            Value::Int(9)
        );
        assert_eq!(
            context
                .eval("identifierCompoundGlobal = 1; identifierCompoundGlobal += 2")
                .unwrap(),
            Value::Int(3)
        );

        for (source, expected) in [
            (
                "(function(value){ value &&= 9; return value; })(2)",
                Value::Int(9),
            ),
            (
                "(function(value){ value &&= 9; return value; })(0)",
                Value::Int(0),
            ),
            (
                "(function(value){ value ||= 9; return value; })(0)",
                Value::Int(9),
            ),
            (
                "(function(value){ value ||= 9; return value; })(2)",
                Value::Int(2),
            ),
            (
                "(function(value){ value ??= 9; return value; })(null)",
                Value::Int(9),
            ),
            (
                "(function(value){ value ??= 9; return value; })(false)",
                Value::Bool(false),
            ),
        ] {
            assert_eq!(context.eval(source).unwrap(), expected, "{source}");
        }

        assert_eq!(
            context
                .eval("(function(){ var named; named ??= function(){}; return named.name; })()")
                .unwrap(),
            Value::String(JsString::from_static("named"))
        );
        assert_eq!(
            context
                .eval("(function(){ var named; (named) ??= function(){}; return named.name; })()")
                .unwrap(),
            Value::String(JsString::from_static(""))
        );
        assert_eq!(
            context
                .eval("(function(){ var named; (named = function(){}); return named.name; })()")
                .unwrap(),
            Value::String(JsString::from_static("named"))
        );
        assert_eq!(
            context
                .eval("(function(){ var named; (named) = function(){}; return named.name; })()")
                .unwrap(),
            Value::String(JsString::from_static(""))
        );

        assert_eq!(
            context
                .eval("(function(value){ (value) += 2; return value; })(3)")
                .unwrap(),
            Value::Int(5)
        );
        assert!(
            context
                .compile("(function(a,b){ (0, a) += 1; return a; })")
                .is_err()
        );
        assert!(
            context
                .compile("(function(a,b){ (true ? a : b) ||= 1; return a; })")
                .is_err()
        );
        assert!(
            context
                .compile("(function(){ 'use strict'; eval += 1; })")
                .is_err()
        );
        assert!(
            context
                .compile("(function(){ 'use strict'; (arguments) ??= 1; })")
                .is_err()
        );

        assert_eq!(
            context
                .eval(
                    "(function named(){ var result = named += ''; \
                     return typeof result + '|' + typeof named; })()"
                )
                .unwrap(),
            Value::String(JsString::from_static("string|function"))
        );
        assert_eq!(
            context
                .eval(
                    "(function(wrapper){ return wrapper() === wrapper; })(function named(){ \
                     return named ||= 1; })"
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn named_function_expression_has_intrinsic_name_and_private_recursive_binding() {
        assert_eq!(
            evaluate_in_context("(function fact(n) { return n ? n * fact(n - 1) : 1; })(5)"),
            Value::Int(120)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(f) { return f() === f; })(function anonymous() { return anonymous; })"
            ),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_in_context("(function named(named) { return named; })(42)"),
            Value::Int(42)
        );
        assert_eq!(
            evaluate_in_context("(function named() { var named = 42; return named; })()"),
            Value::Int(42)
        );
        assert_eq!(
            evaluate_in_context("(function named() {}), typeof named"),
            Value::String(JsString::from_static("undefined"))
        );

        let (name, writable, enumerable, configurable) =
            evaluate_function_name("(function named() {})");
        assert_eq!(name, JsString::from_static("named"));
        assert!(!writable);
        assert!(!enumerable);
        assert!(configurable);

        let (name, ..) = evaluate_function_name(
            "(function() { var inferred = function intrinsic() {}; return inferred; })()",
        );
        assert_eq!(name, JsString::from_static("intrinsic"));

        let script = compile_unlinked_script("(function unusedName() { return 1; })").unwrap();
        let function = script.constants()[0].as_child().unwrap();
        assert_eq!(function.metadata().function_name_local, None);
        assert_eq!(function.metadata().local_count, 0);

        let script = compile_unlinked_script("(function self() { return self; })").unwrap();
        let function = script.constants()[0].as_child().unwrap();
        assert_eq!(function.metadata().function_name_local, Some(0));
        assert_eq!(function.metadata().local_count, 1);
    }

    #[test]
    fn named_function_self_binding_captures_through_relays_and_is_per_instance() {
        let source = "(function(f) { return f()()() === f; })(function named() { return function() { return function() { return named; }; }; })";
        assert_eq!(evaluate_in_context(source), Value::Bool(true));
        assert_eq!(
            evaluate_in_context(
                "(function() { var make = function() { return function named() { return named; }; }; var a = make(), b = make(); return a() === a && b() === b && a !== b; })()"
            ),
            Value::Bool(true)
        );

        let script = compile_unlinked_script(
            "(function named() { return function() { return function() { return named; }; }; })",
        )
        .unwrap();
        let named = script.constants()[0].as_child().unwrap();
        let relay = named.constants()[0].as_child().unwrap();
        let inner = relay.constants()[0].as_child().unwrap();
        assert_eq!(named.metadata().function_name_local, Some(0));
        assert_eq!(named.metadata().local_count, 1);
        assert_eq!(relay.closure_variables().len(), 1);
        assert_eq!(
            relay.closure_variables()[0].source,
            ClosureSource::ParentLocal(0)
        );
        assert_eq!(
            relay.closure_variables()[0].kind,
            ClosureVariableKind::FunctionName
        );
        let ClosureVariableName::Constant(relay_name) = relay.closure_variables()[0].name else {
            panic!("function-name relay did not retain its source name");
        };
        assert_eq!(
            relay.constants()[usize::try_from(relay_name).unwrap()].as_primitive(),
            Some(&Value::String(JsString::from_static("named")))
        );
        assert!(!relay.closure_variables()[0].is_const);
        assert_eq!(
            inner.closure_variables()[0].source,
            ClosureSource::ParentClosure(0)
        );
        assert_eq!(
            inner.closure_variables()[0].kind,
            ClosureVariableKind::FunctionName
        );
        let ClosureVariableName::Constant(inner_name) = inner.closure_variables()[0].name else {
            panic!("transitive function-name relay did not retain its source name");
        };
        assert_eq!(
            inner.constants()[usize::try_from(inner_name).unwrap()].as_primitive(),
            Some(&Value::String(JsString::from_static("named")))
        );
    }

    #[test]
    fn named_function_self_assignment_matches_quickjs_strict_and_sloppy_rules() {
        assert_eq!(
            evaluate_in_context("(function named() { return named = 1; })()"),
            Value::Int(1)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(f) { return f() === f; })(function named() { named = 1; return named; })"
            ),
            Value::Bool(true)
        );
        // QuickJS carries JS_VAR_FUNCTION_NAME semantics from the defining
        // function through closure relays. A nested strict directive does not
        // turn a sloppy outer function-name binding into a throwing write.
        assert_eq!(
            evaluate_in_context(
                "(function(f) { return f()() === f; })(function named() { return function() { 'use strict'; named = 1; return named; }; })"
            ),
            Value::Bool(true)
        );

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context.eval("(function named() { 'use strict'; named = 1; })()"),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("strict function-name assignment did not materialize TypeError");
        };
        let name = runtime.intern_property_key("name").unwrap();
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from_static("TypeError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static("'named' is read-only"))
        );

        assert_eq!(
            context
                .eval("(function named() { 'use strict'; return function() { named = 1; }; })()()"),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("captured strict function-name assignment did not materialize TypeError");
        };
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from_static("TypeError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static("'named' is read-only"))
        );

        let strict = compile_unlinked_script(
            "(function named() { 'use strict'; return function() { return named; }; })",
        )
        .unwrap();
        let named = strict.constants()[0].as_child().unwrap();
        let child = named.constants()[1].as_child().unwrap();
        assert_eq!(
            child.closure_variables()[0].kind,
            ClosureVariableKind::FunctionName
        );
        assert!(child.closure_variables()[0].is_const);
        assert!(!child.closure_variables()[0].is_lexical);
    }

    #[test]
    fn nested_capture_installs_parent_closure_relay_and_executes() {
        let source = "(function(a) { return function() { return function(b) { return a + b; }; }; })(20)()(22)";
        assert_eq!(evaluate_in_context(source), Value::Int(42));

        let script = compile_unlinked_script(source).unwrap();
        let outer = script.constants()[0].as_child().unwrap();
        let relay = outer.constants()[0].as_child().unwrap();
        let inner = relay.constants()[0].as_child().unwrap();

        assert!(outer.closure_variables().is_empty());
        assert_eq!(relay.closure_variables().len(), 1);
        assert_eq!(
            relay.closure_variables()[0].source,
            ClosureSource::ParentArgument(0)
        );
        assert_eq!(inner.closure_variables().len(), 1);
        assert_eq!(
            inner.closure_variables()[0].source,
            ClosureSource::ParentClosure(0)
        );
    }

    #[test]
    fn function_local_var_capture_uses_parent_local_then_parent_closure() {
        let source = "(function() { var a = 20; return function() { return function(b) { return a + b; }; }; })()()(22)";
        assert_eq!(evaluate_in_context(source), Value::Int(42));

        let script = compile_unlinked_script(source).unwrap();
        let outer = script.constants()[0].as_child().unwrap();
        let relay = outer.constants()[0].as_child().unwrap();
        let inner = relay.constants()[0].as_child().unwrap();
        assert_eq!(outer.metadata().local_count, 1);
        assert_eq!(
            relay.closure_variables()[0].source,
            ClosureSource::ParentLocal(0)
        );
        assert_eq!(
            inner.closure_variables()[0].source,
            ClosureSource::ParentClosure(0)
        );
    }

    #[test]
    fn ordinary_function_fallthrough_returns_undefined() {
        assert_eq!(
            evaluate_in_context("(function(a) { a; })(42)"),
            Value::Undefined
        );
    }

    #[test]
    fn strict_and_escaped_reserved_binding_names_are_rejected_late() {
        for source in [
            "(function(implements) { 'use strict'; return implements; })(1)",
            "'use strict'; (function(let) { return let; })(1)",
            "(function() { 'use strict'; var eval = 1; return eval; })()",
            "(function() { 'use strict'; return impl\\u0065ments; })()",
            "(function(\\u0069f) { return \\u0069f; })(1)",
        ] {
            assert!(
                compile_unlinked_script(source).is_err(),
                "accepted {source:?}"
            );
        }
        assert_eq!(
            evaluate_in_context("(function(implements) { return implements; })(1)"),
            Value::Int(1)
        );
        assert_eq!(
            evaluate_in_context("(function(impl\\u0065ments) { return impl\\u0065ments; })(1)"),
            Value::Int(1)
        );
    }

    #[test]
    fn parser_driven_lexing_preserves_quickjs_error_priority_and_locations() {
        let cases = [
            (
                r"(function(){ var \u0069f\u{}=14; })()",
                "'if' is a reserved identifier",
                1,
                18,
            ),
            (
                r"(function(){ var if\u{}=14; })()",
                "'if' is a reserved identifier",
                1,
                18,
            ),
            (
                r"(function(){ var if\x61=1; })()",
                "variable name expected",
                1,
                18,
            ),
            (
                r"(function(){ var \u{}=1; })()",
                "variable name expected",
                1,
                18,
            ),
            (r"(function(){ var a\u{}=1; })()", "expecting ';'", 1, 19),
            (
                "(function(){ var 'unterminated })()",
                "unexpected end of string",
                1,
                18,
            ),
            (
                "(function(a 'unterminated){})",
                "unexpected end of string",
                1,
                13,
            ),
            (
                "(function(){ return (1 'unterminated); })()",
                "unexpected end of string",
                1,
                24,
            ),
            (
                "(function(eval){ \"use strict\"; \"x\"; \"unterminated })()",
                "unexpected end of string",
                1,
                37,
            ),
            (
                "(function(){ \"use strict\"; (function(eval){ \"x\"; \"unterminated })() })()",
                "unexpected end of string",
                1,
                50,
            ),
        ];

        for (source, message, line, column) in cases {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
            assert_eq!(error.message(), message, "{source}");
            let span = error
                .span()
                .unwrap_or_else(|| panic!("missing span for {source}"));
            assert_eq!(
                (span.start.line, span.start.column),
                (line, column),
                "{source}"
            );
        }

        let reached_lex_error =
            compile_unlinked_script("(function(){ throw\n'unterminated })()").unwrap_err();
        assert_eq!(reached_lex_error.message(), "unexpected end of string");
        let reached_span = reached_lex_error.span().unwrap();
        assert_eq!((reached_span.start.line, reached_span.start.column), (2, 1));

        let raw_token_error =
            compile_unlinked_script("(function(){ throw\n\\u{}; })()").unwrap_err();
        assert_eq!(
            raw_token_error.message(),
            "line terminator not allowed after throw"
        );
        let raw_span = raw_token_error.span().unwrap();
        assert_eq!((raw_span.start.line, raw_span.start.column), (2, 1));
    }

    #[test]
    fn primary_expression_slashes_are_rescanned_as_complete_regexp_tokens() {
        // Pinned QuickJS makes this decision in its primary-expression parser:
        // `/` and `/=` are ordinary punctuators until the grammar requires an
        // operand, at which point it rewinds and scans the complete literal.
        for source in [
            "/start/g;",
            "/=prefix/m;",
            "Function.value = /rhs/gi;",
            "(function(){ return /ret/m; })",
            "Function(/argument/s);",
            "true ? /consequent/u : 0;",
            "false ? 0 : /alternate/y;",
            "false || /logical/d;",
            "1 / /denominator/u;",
        ] {
            compile_unlinked_script(source)
                .unwrap_or_else(|error| panic!("RegExp literal {source:?} failed: {error}"));
        }
        let script = compile_unlinked_script("/start/g;").unwrap();
        assert!(matches!(script.code()[0], Instruction::RegExp(0)));

        let invalid_pattern = compile_unlinked_script("/(/").unwrap_err();
        assert_eq!(invalid_pattern.kind(), ErrorKind::Syntax);
        assert_eq!(invalid_pattern.message(), "expecting ')'");
        let span = invalid_pattern.span().expect("literal SyntaxError span");
        assert_eq!((span.start.line, span.start.column), (1, 1));
        assert_eq!((span.start.byte_offset, span.end.byte_offset), (0, 3));

        let invalid_flags = compile_unlinked_script("/a/gg").unwrap_err();
        assert_eq!(invalid_flags.kind(), ErrorKind::Syntax);
        assert_eq!(invalid_flags.message(), "invalid regular expression flags");

        for (source, expected) in [
            ("/a", "unexpected end of regexp"),
            ("/a\n/", "unexpected line terminator in regexp"),
            ("/a\\\n/", "unexpected line terminator in regexp"),
        ] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}");
            assert_eq!(error.message(), expected, "{source:?}");
        }

        compile_unlinked_script("/(?=a)/").expect("forward lookahead literal should compile");
        let unsupported = compile_unlinked_script("/(?<=a)/").unwrap_err();
        assert_eq!(unsupported.kind(), ErrorKind::Unsupported);
        assert!(unsupported.message().contains("Lookaround"));
        let unsupported_v = compile_unlinked_script("1 / /denominator/v;").unwrap_err();
        assert_eq!(unsupported_v.kind(), ErrorKind::Unsupported);
        assert!(unsupported_v.message().contains("UnicodeSetOperation"));

        // The same slash tokens remain operators when the expression parser
        // has already produced their left operand.
        assert_eq!(evaluate("84 / 2"), Value::Int(42));
        assert_eq!(
            evaluate_in_context("(function(){ var value=84; value /= 2; return value; })()"),
            Value::Int(42)
        );
    }

    #[test]
    fn implicit_arguments_binding_is_lazy_and_precedes_body_hoists() {
        for source in [
            "(function() { return 1; })",
            "(function() { return delete arguments; })",
            "(function(arguments) { return arguments; })",
        ] {
            let script = compile_unlinked_script(source).unwrap();
            let function = script.constants()[0].as_child().unwrap();
            assert!(
                !function
                    .code()
                    .iter()
                    .any(|op| matches!(op, Instruction::Arguments(_))),
                "unexpected arguments object for {source}"
            );
        }

        for (source, kind) in [
            ("(function() { return arguments; })", ArgumentsKind::Mapped),
            (
                "(function() { 'use strict'; return arguments; })",
                ArgumentsKind::Unmapped,
            ),
            (
                "(function() { var arguments; return typeof arguments; })",
                ArgumentsKind::Mapped,
            ),
        ] {
            let script = compile_unlinked_script(source).unwrap();
            let function = script.constants()[0].as_child().unwrap();
            assert!(matches!(
                function.code(),
                [Instruction::Arguments(actual), Instruction::PutLocal(0), ..]
                    if *actual == kind
            ));
            assert_eq!(function.local_definitions().len(), 1, "{source}");
        }

        let script = compile_unlinked_script(
            "(function arguments() { function arguments() {} return arguments; })",
        )
        .unwrap();
        let function = script.constants()[0].as_child().unwrap();
        assert_eq!(function.metadata().function_name_local, None);
        assert!(matches!(
            function.code(),
            [
                Instruction::Arguments(ArgumentsKind::Mapped),
                Instruction::PutLocal(0),
                Instruction::FClosure(0),
                Instruction::PutLocal(0),
                ..
            ]
        ));

        let script = compile_unlinked_script(
            "(function outer() { return function inner() { return arguments; }; })",
        )
        .unwrap();
        let outer = script.constants()[0].as_child().unwrap();
        let inner = outer.constants()[0].as_child().unwrap();
        assert!(
            !outer
                .code()
                .iter()
                .any(|op| matches!(op, Instruction::Arguments(_)))
        );
        assert!(matches!(
            inner.code(),
            [
                Instruction::Arguments(ArgumentsKind::Mapped),
                Instruction::PutLocal(0),
                ..
            ]
        ));

        assert_eq!(
            evaluate_in_context("(function(arguments) { var arguments; return arguments; })(7)"),
            Value::Int(7)
        );
    }

    #[test]
    fn var_initializer_named_evaluation_follows_quickjs_set_name_marker() {
        for source in [
            "(function() { var f = function() {}; return f; })()",
            "(function() { var f = (((function() {}))); return f; })()",
            "(function() { var \\u0066 = function() {}; return f; })()",
            "(function() { var f; f = function() {}; return f; })()",
        ] {
            assert_eq!(
                evaluate_function_name(source),
                (JsString::from_static("f"), false, false, true),
                "direct anonymous initializer should inherit the binding name: {source}"
            );
        }

        for source in [
            "(function() { return function() {}; })()",
            "(function() { var f = (0, function() {}); return f; })()",
            "(function() { var f = true ? function() {} : function() {}; return f; })()",
            "(function() { var f = 0 || function() {}; return f; })()",
        ] {
            assert_eq!(
                evaluate_function_name(source),
                (JsString::from_static(""), false, false, true),
                "non-AnonymousFunctionDefinition expression must keep an empty name: {source}"
            );
        }
    }

    #[test]
    fn new_and_new_target_follow_quickjs_base_constructor_semantics() {
        assert_eq!(
            evaluate_in_context(
                "(function(){ var F = function(){ return this; }; return typeof new F(); })()"
            ),
            Value::String(JsString::from_static("object"))
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){ var F = function(){ return new.target; }; return new F() === F; })()"
            ),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){ var marker = function(){}; var F = function(a){ return a; }; return new F(marker) === marker; })()"
            ),
            Value::Bool(true)
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){ var F = function(){ return 1; }; return typeof new F; })()"
            ),
            Value::String(JsString::from_static("object"))
        );
        assert_eq!(
            evaluate_in_context("(function(){ return new.target; })()"),
            Value::Undefined
        );

        let error = compile_unlinked_script("new.target").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax);
        assert_eq!(error.message(), "new.target only allowed within functions");

        for source in [
            r"(function(){ return new.\u0074arget; })()",
            r"(function(){ return new.t\u0061rget; })()",
        ] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Syntax);
            assert_eq!(error.message(), "expecting target");
        }
    }

    #[test]
    fn quickjs_argument_slot_limit_uses_catchable_internal_error() {
        let parameters = std::iter::repeat_n("a", MAX_LOCAL_VARIABLES + 1)
            .collect::<Vec<_>>()
            .join(",");
        let source = format!("(function({parameters}) {{}})");
        let error = compile_unlinked_script(&source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::JsInternal);
        assert_eq!(error.message(), "too many arguments");

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.compile(&source), Err(RuntimeError::Exception));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("argument overflow must materialize InternalError");
        };
        let name = runtime.intern_property_key("name").unwrap();
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from_static("InternalError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static("too many arguments"))
        );
    }

    #[test]
    fn quickjs_call_argument_boundary_materializes_stack_overflow() {
        let arguments = std::iter::repeat_n("0", MAX_CALL_ARGUMENTS)
            .collect::<Vec<_>>()
            .join(",");
        let source = format!("(function() {{}})({arguments})");
        let error = compile_unlinked_script(&source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::JsInternal);
        assert_eq!(error.message(), "stack overflow");

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.compile(&source), Err(RuntimeError::Exception));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("bytecode stack overflow must materialize InternalError");
        };
        let name = runtime.intern_property_key("name").unwrap();
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from_static("InternalError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static("stack overflow"))
        );

        let mut too_many = source.clone();
        let closing_parenthesis = too_many
            .rfind(')')
            .expect("generated call expression has a closing parenthesis");
        too_many.insert_str(closing_parenthesis, ",0");
        let error = compile_unlinked_script(&too_many).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax);
        assert_eq!(error.message(), "Too many call arguments");

        let unreachable = format!("(function(){{ return 1; {source}; }})");
        compile_unlinked_script(&unreachable).unwrap();
    }

    #[test]
    fn quickjs_template_stack_overflow_is_deferred_until_after_parsing() {
        // Each substitution is one concat argument; the kept receiver and
        // method let 65,532 arguments exactly reach JS_STACK_SIZE_MAX.
        let largest_valid = "${0}".repeat(MAX_BYTECODE_STACK - 2);
        let largest_valid = compile_unlinked_script(&format!("`{largest_valid}`")).unwrap();
        assert_eq!(
            largest_valid.metadata().max_stack,
            MAX_BYTECODE_STACK as u16
        );

        // One more argument exceeds the limit without passing through the
        // ordinary call parser's argument guard.
        let substitutions = "${0}".repeat(MAX_BYTECODE_STACK - 1);
        let source = format!("`{substitutions}`");
        let error = compile_unlinked_script(&source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::JsInternal);
        assert_eq!(error.message(), "stack overflow");

        // QuickJS computes the bytecode stack only after parsing the whole
        // function, so a later reached lexical error has priority.
        let later_lexical_error = format!("{source}; \"unterminated");
        let error = compile_unlinked_script(&later_lexical_error).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax);
        assert_eq!(error.message(), "unexpected end of string");

        let later_parser_error = format!("{source} 0");
        let error = compile_unlinked_script(&later_parser_error).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax);
        assert_eq!(error.message(), "expecting ';'");

        // QuickJS computes stack depth over reachable bytecode PCs. The same
        // oversized call after a terminal return is encoded but ignored by
        // the control-flow walk.
        let unreachable = format!("(function(){{ return 1; {source}; }})");
        compile_unlinked_script(&unreachable).unwrap();

        // Once argc no longer fits u16, QuickJS encodes its low bits. A live
        // path has already crossed the stack cap before that call, while dead
        // bytecode remains valid and must not be diagnosed from the truncated
        // operand's residual stack effect.
        let wrapped_substitutions = "${0}".repeat(usize::from(u16::MAX) + 1);
        let wrapped = format!("`{wrapped_substitutions}`");
        let error = compile_unlinked_script(&wrapped).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::JsInternal);
        assert_eq!(error.message(), "stack overflow");
        let unreachable = format!("(function(){{ return 1; {wrapped}; }})");
        compile_unlinked_script(&unreachable).unwrap();
    }

    #[test]
    fn quickjs_closure_slot_limit_is_65534_and_uses_internal_error() {
        let span = Span::new(Position::new(0, 1, 1), Position::new(0, 1, 1));
        let mut function = FunctionIr::new(
            None,
            FunctionKind::Ordinary,
            FunctionSourceInfo {
                span,
                definition: SourceOffset::try_from_usize(0).unwrap(),
                range: None,
            },
            None,
            false,
            Vec::new(),
            false,
        )
        .unwrap();
        function.closure_variables = (0..MAX_LOCAL_VARIABLES - 1)
            .map(|index| ClosureVariable {
                source: ClosureSource::ParentLocal(
                    u16::try_from(index).expect("test index is below the QuickJS slot limit"),
                ),
                name: ClosureVariableName::None,
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            })
            .collect();

        assert_eq!(
            ensure_closure_variable(
                &mut function,
                ClosureVariable {
                    source: ClosureSource::ParentArgument(0),
                    name: ClosureVariableName::None,
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
            )
            .unwrap(),
            65_533
        );
        let error = ensure_closure_variable(
            &mut function,
            ClosureVariable {
                source: ClosureSource::ParentArgument(1),
                name: ClosureVariableName::None,
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::JsInternal);
        assert_eq!(error.message(), "too many closure variables");
    }

    #[test]
    fn compiles_throw_as_a_terminal_completion_and_enforces_no_line_terminator() {
        let bytecode = compile_script("throw 9").unwrap();
        assert!(
            bytecode
                .code
                .iter()
                .any(|instruction| matches!(instruction, crate::bytecode::Instruction::Throw))
        );
        assert!(compile_script("throw\n9").is_err());
    }

    #[test]
    fn try_catch_lowering_keeps_parameter_and_body_scopes_distinct() {
        let source = r#"
            try { throw 1; }
            catch (e) { let x = e; function f(){ return e + x; } }
        "#;
        let tree = Parser::parse(source, JsString::from_static("<try-scope-test>")).unwrap();
        let root = &tree.functions[0];
        let catch_scope = root
            .scopes
            .iter()
            .position(|scope| scope.kind == ScopeKind::Catch)
            .map(super::ScopeId)
            .unwrap();
        let catch_body_scope = root
            .scopes
            .iter()
            .enumerate()
            .find(|(_, scope)| scope.kind == ScopeKind::Block && scope.parent == Some(catch_scope))
            .map(|(index, _)| super::ScopeId(index))
            .unwrap();
        let catch_binding = root.binding_in_scope(catch_scope, "e").unwrap();
        let BindingStorage::Local(catch_local) = catch_binding.storage else {
            panic!("catch parameter did not use local storage");
        };
        assert!(catch_binding.is_catch_parameter);
        assert_eq!(catch_binding.kind, BindingKind::Lexical { is_const: false });
        assert!(root.binding_in_scope(catch_body_scope, "x").is_some());
        assert!(root.binding_in_scope(catch_body_scope, "f").is_some());
        assert_eq!(
            tree.functions
                .iter()
                .find(|function| function.function_name.as_deref() == Some("f"))
                .and_then(|function| function.parent)
                .map(|parent| parent.definition_scope),
            Some(catch_body_scope)
        );

        let bytecode = compile_unlinked_script(source).unwrap();
        assert_eq!(
            bytecode
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::Catch(_)))
                .count(),
            2
        );
        assert!(bytecode.code().iter().any(
            |instruction| matches!(instruction, Instruction::SetLocalUninitialized(index) if *index == catch_local)
        ));
        assert!(bytecode.code().iter().any(
            |instruction| matches!(instruction, Instruction::CloseLocal(index) if *index == catch_local)
        ));
        assert!(
            bytecode
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Ret))
        );
    }

    #[test]
    fn catch_binding_conflicts_and_var_initializer_follow_quickjs() {
        for (source, keyword) in [("catch (e) {}", "catch"), ("finally {}", "finally")] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(
                error.message(),
                format!("unexpected token in expression: '{keyword}'"),
                "{source}"
            );
        }
        let extra_catch = compile_unlinked_script("try {} finally {} catch (e) {}").unwrap_err();
        assert_eq!(
            extra_catch.message(),
            "unexpected token in expression: 'catch'"
        );

        for source in [
            "try {} catch (e) { let e; }",
            "try {} catch (e) { function e(){} }",
        ] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
            assert_eq!(
                error.message(),
                "invalid redefinition of lexical identifier",
                "{source}"
            );
        }
        compile_unlinked_script("try {} catch (e) { { let e = 1; e; } }").unwrap();

        for source in ["try {} catch ({e}) {}", "try {} catch ([e]) {}"] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Unsupported, "{source}");
            assert_eq!(
                error.message(),
                "catch destructuring bindings are not implemented yet",
                "{source}"
            );
        }

        let source = "try { throw 1; } catch (e) { var e = e + 1; e; }";
        let tree = Parser::parse(source, JsString::from_static("<catch-var-test>")).unwrap();
        let catch_local = tree.functions[0]
            .bindings
            .iter()
            .find(|binding| binding.is_catch_parameter)
            .and_then(|binding| match binding.storage {
                BindingStorage::Local(index) => Some(index),
                _ => None,
            })
            .unwrap();
        let bytecode = compile_unlinked_script(source).unwrap();
        assert!(bytecode.code().iter().any(
            |instruction| matches!(instruction, Instruction::GetLocalCheck(index) if *index == catch_local)
        ));
        assert!(bytecode.code().iter().any(
            |instruction| matches!(instruction, Instruction::PutLocalCheck(index) if *index == catch_local)
        ));
        assert_eq!(evaluate_in_context(source), Value::Int(2));

        let strict_source = "\"use strict\"; try {} catch (eval) {}";
        let strict_error = compile_unlinked_script(strict_source).unwrap_err();
        assert_eq!(
            strict_error.message(),
            "invalid variable name in strict mode"
        );
        assert_eq!(
            strict_error.span().unwrap().start.column,
            u32::try_from(strict_source.find(')').unwrap() + 1).unwrap()
        );
    }

    #[test]
    fn nested_finally_abrupt_edges_use_typed_cleanup_and_shared_subroutines() {
        let source = r#"
            (function f(){
                outer: while (1) {
                    try {
                        try { return 1; }
                        finally { break outer; }
                    } finally { return 3; }
                }
                return 4;
            })()
        "#;
        let bytecode = compile_unlinked_script(source).unwrap();
        let function = bytecode
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .unwrap();
        assert_eq!(
            function
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::Catch(_)))
                .count(),
            2
        );
        assert_eq!(
            function
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::Ret))
                .count(),
            2
        );
        assert!(
            function
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::NipCatch))
                .count()
                >= 2
        );
        assert!(
            function
                .code()
                .windows(2)
                .any(|window| matches!(window, [Instruction::DropGosub, Instruction::Drop]))
        );
        assert_eq!(evaluate_in_context(source), Value::Int(3));
    }

    #[test]
    fn script_finally_saves_and_normally_restores_eval_completion() {
        let source = "1; try { 2; } finally { 3; }";
        let bytecode = compile_unlinked_script(source).unwrap();
        assert_eq!(bytecode.local_definitions().len(), 2);
        assert!(
            bytecode
                .local_definitions()
                .iter()
                .all(|definition| definition.name.is_none() && !definition.is_lexical)
        );
        assert!(bytecode.code().windows(4).any(|window| matches!(
            window,
            [
                Instruction::GetLocal(0),
                Instruction::PutLocal(1),
                Instruction::Undefined,
                Instruction::PutLocal(0)
            ]
        )));
        assert!(bytecode.code().windows(3).any(|window| matches!(
            window,
            [
                Instruction::GetLocal(1),
                Instruction::PutLocal(0),
                Instruction::Ret
            ]
        )));
        assert_eq!(evaluate_in_context(source), Value::Int(2));
        assert_eq!(
            evaluate_in_context("try { throw 1; } catch { 4; } finally { 5; }"),
            Value::Int(4)
        );
    }

    #[test]
    fn compiles_bigint_literals_without_a_fixed_width_limit() {
        let expected =
            JsBigInt::parse_radix("10000000000000000000000000000000000000000", 16).unwrap();
        assert_eq!(
            evaluate("0x10000000000000000000000000000000000000000n"),
            Value::BigInt(expected)
        );
    }

    #[test]
    fn use_strict_directive_rejects_legacy_literals() {
        assert!(compile_script("'use strict'; 010").is_err());
        assert!(compile_script("'use strict'; '\\1'").is_err());
        assert!(compile_script("'\\1'; 'use strict'; 0").is_err());
        assert!(compile_script("'\\8'; 'use strict'; 0").is_err());
        assert!(compile_script("; 'use strict'; 010").is_ok());
        assert!(compile_script("'use\\x20strict'; 010").is_ok());
        assert!(compile_script("'not strict'\n'use strict'\n010").is_err());
        assert!(compile_script("'not strict'\n+ 'use strict'; 010").is_ok());
        assert!(compile_script("'use strict' + ''; 010").is_ok());
        assert!(compile_script("'use strict'\n!0; 010").is_ok());
        assert!(compile_script("'use strict'\nvoid 0; 010").is_ok());
    }

    #[test]
    fn unlinked_script_preserves_strict_mode_metadata() {
        let strict = compile_unlinked_script("'use strict'; 0").unwrap();
        let sloppy = compile_unlinked_script("'use\\x20strict'; 0").unwrap();

        assert!(strict.metadata().strict);
        assert!(!sloppy.metadata().strict);
    }

    #[test]
    fn unlinked_script_preserves_verified_maximum_stack() {
        let source = "'left' + (0.5 * 2.5)";
        let bytecode = compile_script(source).unwrap();
        let verified = bytecode.verify().unwrap();
        let unlinked = compile_unlinked_script(source).unwrap();

        assert_eq!(bytecode.max_stack, 3);
        assert_eq!(unlinked.metadata().max_stack, bytecode.max_stack);
        assert_eq!(unlinked.metadata().max_stack, verified.max_stack);
    }

    #[test]
    fn unlinked_script_converts_every_compiled_constant_to_a_primitive() {
        let source = "'\\ud800x'; 3.5; 0x100000000000000000000000000000000n";
        let bytecode = compile_script(source).unwrap();
        let unlinked = compile_unlinked_script(source).unwrap();

        assert_eq!(bytecode.constants.len(), 3);
        assert_eq!(unlinked.constants().len(), bytecode.constants.len());
        for (constant, expected) in unlinked.constants().iter().zip(&bytecode.constants) {
            assert_eq!(constant.as_primitive(), Some(expected));
            assert!(constant.as_child().is_none());
        }
    }

    #[test]
    fn detached_string_literals_follow_quickjs_atom_identity_boundaries() {
        let atomized = compile_script("['same','same']").unwrap();
        let Value::String(first) = &atomized.constants[0] else {
            panic!("first atomized literal was not a String");
        };
        let Value::String(second) = &atomized.constants[1] else {
            panic!("second atomized literal was not a String");
        };
        assert!(first.same_representation(second));

        let immediate = compile_script("['2147483647','2147483647']").unwrap();
        let Value::String(first) = &immediate.constants[0] else {
            panic!("first immediate-atom literal was not a String");
        };
        let Value::String(second) = &immediate.constants[1] else {
            panic!("second immediate-atom literal was not a String");
        };
        assert!(!first.same_representation(second));

        let table_backed = compile_script("['2147483648','2147483648']").unwrap();
        let Value::String(first) = &table_backed.constants[0] else {
            panic!("first table-backed numeric literal was not a String");
        };
        let Value::String(second) = &table_backed.constants[1] else {
            panic!("second table-backed numeric literal was not a String");
        };
        assert!(first.same_representation(second));
    }

    #[test]
    fn object_literals_lower_quickjs_data_proto_computed_and_spread_paths() {
        let fixed = compile_unlinked_script("({a:1,if:2,'x':3,0x10:4,a:5})").unwrap();
        assert!(
            fixed
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Object))
        );
        assert_eq!(
            fixed
                .code()
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::DefineField(_)))
                .count(),
            5
        );

        let shorthand = compile_unlinked_script("var value=1;({value})").unwrap();
        assert!(
            shorthand
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::DefineField(_)))
        );

        let computed = compile_unlinked_script("({[1]:function(){}})").unwrap();
        assert!(computed.code().windows(4).any(|window| matches!(
            window,
            [
                Instruction::FClosure(_),
                Instruction::SetNameComputed,
                Instruction::DefineArrayEl,
                Instruction::Drop
            ]
        )));
        assert!(
            computed
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::ToPropKey))
        );

        let proto = compile_unlinked_script("({__proto__:null})").unwrap();
        assert!(
            proto
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::SetProto))
        );
        assert!(
            !proto
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::DefineField(_)))
        );

        let spread = compile_unlinked_script("({a:1,...value,b:2})").unwrap();
        assert!(
            spread
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::CopyDataProperties))
        );
    }

    #[test]
    fn object_literal_grammar_is_fail_closed_at_method_and_pattern_frontiers() {
        for source in [
            "({})",
            "({a:1,})",
            "({a})",
            "({let})",
            "({['x']:2})",
            "({...null})",
            "({__proto__:null,a:1})",
        ] {
            compile_unlinked_script(source)
                .unwrap_or_else(|error| panic!("valid Object literal {source:?}: {error}"));
        }

        for source in ["({a(){}})", "({get a(){}})", "({set a(v){}})", "({*a(){}})"] {
            assert!(
                compile_unlinked_script(source)
                    .unwrap_err()
                    .message()
                    .contains("not implemented yet"),
                "method frontier was not explicit for {source}"
            );
        }
        assert_eq!(
            compile_unlinked_script("({__proto__:null,__proto__:{}})")
                .unwrap_err()
                .message(),
            "duplicate __proto__ property name"
        );
        assert_eq!(
            compile_unlinked_script("({get=1})").unwrap_err().message(),
            "expecting '}'"
        );
        assert_eq!(
            compile_unlinked_script("({#private:1})")
                .unwrap_err()
                .message(),
            "private identifiers are not valid in object literals"
        );
    }

    #[test]
    fn object_literal_runtime_preserves_descriptors_proto_names_and_pinned_spread() {
        assert_eq!(
            evaluate_in_context(
                "(function(){var x=3;var o={2:'two',a:1,x,a:4};var d=Object.getOwnPropertyDescriptor(o,'a');return o[2]+'|'+o.x+'|'+o.a+'|'+d.writable+'|'+d.enumerable+'|'+d.configurable})()"
            ),
            Value::String(JsString::from_static("two|3|4|true|true|true"))
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var p={marker:7};var a={__proto__:p};var b={__proto__:1};var c={['__proto__']:9};return a.marker+'|'+Object.hasOwn(a,'__proto__')+'|'+(Object.getPrototypeOf(b)===Object.prototype)+'|'+c.__proto__})()"
            ),
            Value::String(JsString::from_static("7|false|true|9"))
        );
        assert_eq!(
            evaluate_in_context(
                "(function(){var s=Symbol('key');var a={[s]:function(){},plain:function(){}};return a[s].name+'|'+a.plain.name+'|'+Object.hasOwn({...\"ab\"},'0')})()"
            ),
            Value::String(JsString::from_static("[key]|plain|false"))
        );
    }

    #[test]
    fn array_literals_lower_dense_fixed_hole_and_spread_phases() {
        let dense = compile_unlinked_script("[1,2,3]").unwrap();
        assert!(
            dense
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::ArrayFrom(3)))
        );
        assert!(!dense.code().iter().any(|instruction| matches!(
            instruction,
            Instruction::DefineField(_) | Instruction::DefineArrayEl | Instruction::Append
        )));

        let large_source = format!(
            "[{}]",
            (0..33)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let large = compile_unlinked_script(&large_source).unwrap();
        assert!(
            large
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::ArrayFrom(32)))
        );
        let fixed_key = large
            .code()
            .iter()
            .find_map(|instruction| match instruction {
                Instruction::DefineField(index) => Some(*index),
                _ => None,
            })
            .expect("33rd Array element must use DefineField");
        assert!(matches!(
            large.constants()[usize::try_from(fixed_key).unwrap()].as_primitive(),
            Some(Value::String(value)) if value == &JsString::from_static("32")
        ));

        let holes = compile_unlinked_script("[,1,,]").unwrap();
        let hole_code = holes.code();
        assert!(
            hole_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::ArrayFrom(0)))
        );
        assert!(
            hole_code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::DefineField(_)))
        );
        assert!(hole_code.windows(3).any(|window| matches!(
            window,
            [
                Instruction::Dup,
                Instruction::PushI32(3),
                Instruction::PutField(_)
            ]
        )));

        let spread = compile_unlinked_script("[1,...'ab',,4]").unwrap();
        assert!(
            spread
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Append))
        );
        assert!(spread.code().windows(3).any(|window| matches!(
            window,
            [
                Instruction::DefineArrayEl,
                Instruction::Inc,
                Instruction::Drop
            ]
        )));
    }

    #[test]
    fn array_literal_grammar_keeps_quickjs_boundaries_and_reference_state() {
        for source in [
            "[]",
            "[,]",
            "[,,]",
            "[1,]",
            "[...'',]",
            "for([1 in Function];false;);",
        ] {
            compile_unlinked_script(source)
                .unwrap_or_else(|error| panic!("valid Array literal {source:?}: {error}"));
        }
        assert_eq!(
            compile_unlinked_script("[1 2]").unwrap_err().message(),
            "expecting ']'"
        );
        compile_unlinked_script("[/a/]").unwrap();
        for source in ["[... ]", "[1,, 2 3]"] {
            assert!(
                compile_unlinked_script(source).is_err(),
                "invalid Array literal unexpectedly compiled: {source}"
            );
        }

        let named = compile_unlinked_script("var named=[function(){}]").unwrap();
        assert!(
            !named
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::SetName(_)))
        );
        let child = named
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("Array element function child");
        assert_eq!(child.func_name(), None);
    }

    #[test]
    fn debug_metadata_tracks_operator_tail_call_and_root_call_sites() {
        let source = "(function outer(){ return (function inner(){ return 1n + 1; })(); })()";
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let root = context.compile_with_filename(source, "<cmdline>").unwrap();
        let outer = runtime.test_child_function_bytecode(&root, 0).unwrap();
        let inner = runtime.test_child_function_bytecode(&outer, 0).unwrap();

        let root_code = runtime.test_function_code(&root).unwrap();
        let outer_code = runtime.test_function_code(&outer).unwrap();
        let inner_code = runtime.test_function_code(&inner).unwrap();
        let root_call = root_code
            .iter()
            .rposition(|instruction| matches!(instruction, Instruction::Call(0)))
            .unwrap();
        let outer_call = outer_code
            .iter()
            .rposition(|instruction| matches!(instruction, Instruction::Call(0)))
            .unwrap();
        let inner_add = inner_code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::Add))
            .unwrap();

        assert_eq!(
            runtime
                .test_function_debug_location(&inner, Some(inner_add))
                .unwrap(),
            Some((
                JsString::from_static("<cmdline>"),
                crate::LineColumn::new(0, 55)
            ))
        );
        assert_eq!(
            runtime
                .test_function_debug_location(&outer, Some(outer_call))
                .unwrap(),
            Some((
                JsString::from_static("<cmdline>"),
                crate::LineColumn::new(0, 19)
            ))
        );
        assert_eq!(
            runtime
                .test_function_debug_location(&root, Some(root_call))
                .unwrap(),
            Some((
                JsString::from_static("<cmdline>"),
                crate::LineColumn::new(0, 68)
            ))
        );
        assert_eq!(runtime.test_function_debug_source(&root).unwrap(), None);
        assert_eq!(
            runtime.test_function_debug_source(&outer).unwrap(),
            Some(b"function outer(){ return (function inner(){ return 1n + 1; })(); }".to_vec())
        );
        assert_eq!(
            runtime.test_function_debug_source(&inner).unwrap(),
            Some(b"function inner(){ return 1n + 1; }".to_vec())
        );
    }

    #[test]
    fn ordinary_assignment_inherits_last_rhs_marker_and_var_initializer_marks_equal() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let assignment_source = "\"use strict\"; missing = 1";
        let root = context
            .compile_with_filename(assignment_source, "globals.js")
            .unwrap();
        let code = runtime.test_function_code(&root).unwrap();
        let dup = code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::Dup))
            .unwrap();
        assert!(matches!(code.get(dup + 1), Some(Instruction::PutVar(_))));
        let lhs = u32::try_from(assignment_source.find("missing").unwrap()).unwrap();
        let expected = Some((
            JsString::from_static("globals.js"),
            crate::LineColumn::new(0, lhs),
        ));
        assert_eq!(
            runtime
                .test_function_debug_location(&root, Some(dup))
                .unwrap(),
            expected
        );
        assert_eq!(
            runtime
                .test_function_debug_location(&root, Some(dup + 1))
                .unwrap(),
            expected
        );

        let identifier_rhs_source = "(function(){ \"use strict\"; var y=1; missing = y; })";
        let identifier_rhs_root = context
            .compile_with_filename(identifier_rhs_source, "globals.js")
            .unwrap();
        let identifier_rhs = runtime
            .test_child_function_bytecode(&identifier_rhs_root, 0)
            .unwrap();
        let identifier_rhs_code = runtime.test_function_code(&identifier_rhs).unwrap();
        let identifier_put = identifier_rhs_code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::PutVar(_)))
            .unwrap();
        let rhs_identifier = u32::try_from(identifier_rhs_source.rfind("y;").unwrap()).unwrap();
        assert_eq!(
            runtime
                .test_function_debug_location(&identifier_rhs, Some(identifier_put))
                .unwrap(),
            Some((
                JsString::from_static("globals.js"),
                crate::LineColumn::new(0, rhs_identifier)
            ))
        );

        let operator_rhs_source = "\"use strict\"; missing = 1 + 2";
        let operator_rhs = context
            .compile_with_filename(operator_rhs_source, "globals.js")
            .unwrap();
        let operator_rhs_code = runtime.test_function_code(&operator_rhs).unwrap();
        let operator_put = operator_rhs_code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::PutVar(_)))
            .unwrap();
        let plus = u32::try_from(operator_rhs_source.find('+').unwrap()).unwrap();
        assert_eq!(
            runtime
                .test_function_debug_location(&operator_rhs, Some(operator_put))
                .unwrap(),
            Some((
                JsString::from_static("globals.js"),
                crate::LineColumn::new(0, plus)
            ))
        );

        let declaration_source = "(function(){ var x = 1; return x; })";
        let declaration_root = context
            .compile_with_filename(declaration_source, "globals.js")
            .unwrap();
        let declaration = runtime
            .test_child_function_bytecode(&declaration_root, 0)
            .unwrap();
        let declaration_code = runtime.test_function_code(&declaration).unwrap();
        let put_local = declaration_code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::PutLocal(_)))
            .unwrap();
        let equal = u32::try_from(declaration_source.find("= 1").unwrap()).unwrap();
        assert_eq!(
            runtime
                .test_function_debug_location(&declaration, Some(put_local))
                .unwrap(),
            Some((
                JsString::from_static("globals.js"),
                crate::LineColumn::new(0, equal)
            ))
        );
    }

    #[test]
    fn call_and_construct_debug_sites_follow_quickjs_tokens() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        let call_source = "Error()";
        let call_root = context
            .compile_with_filename(call_source, "calls.js")
            .unwrap();
        let call_code = runtime.test_function_code(&call_root).unwrap();
        let call_pc = call_code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::Call(0)))
            .unwrap();
        assert_eq!(
            runtime
                .test_function_debug_location(&call_root, Some(call_pc))
                .unwrap(),
            Some((
                JsString::from_static("calls.js"),
                crate::LineColumn::new(0, 5)
            ))
        );

        let construct_source = "(function f(){ return new Error('x'); })";
        let construct_root = context
            .compile_with_filename(construct_source, "construct.js")
            .unwrap();
        let constructor = runtime
            .test_child_function_bytecode(&construct_root, 0)
            .unwrap();
        let constructor_code = runtime.test_function_code(&constructor).unwrap();
        let construct_pc = constructor_code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::Construct(1)))
            .unwrap();
        let left_paren = construct_source.find("Error(").unwrap() + "Error".len();
        assert_eq!(
            runtime
                .test_function_debug_location(&constructor, Some(construct_pc))
                .unwrap(),
            Some((
                JsString::from_static("construct.js"),
                crate::LineColumn::new(0, u32::try_from(left_paren).unwrap())
            ))
        );

        let no_parens_source = "(function f(){ return new Error; })";
        let no_parens_root = context
            .compile_with_filename(no_parens_source, "construct.js")
            .unwrap();
        let no_parens_constructor = runtime
            .test_child_function_bytecode(&no_parens_root, 0)
            .unwrap();
        let no_parens_code = runtime.test_function_code(&no_parens_constructor).unwrap();
        let no_parens_pc = no_parens_code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::Construct(0)))
            .unwrap();
        let semicolon = no_parens_source.find("Error;").unwrap() + "Error".len();
        assert_eq!(
            runtime
                .test_function_debug_location(&no_parens_constructor, Some(no_parens_pc))
                .unwrap(),
            Some((
                JsString::from_static("construct.js"),
                crate::LineColumn::new(0, u32::try_from(semicolon).unwrap())
            ))
        );
    }

    #[test]
    fn primitive_and_function_primaries_do_not_emit_source_markers() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for source in [
            "(1)",
            "('x')",
            "(null)",
            "(true)",
            "(this)",
            "(function(){})",
            "(!1)",
            "(void 1)",
            "(typeof 1)",
            "(true && false)",
            "(1 ? 2 : 3)",
            "(1, 2)",
        ] {
            let root = context.compile_with_filename(source, "primary.js").unwrap();
            for pc in 0..runtime.test_function_code(&root).unwrap().len() {
                assert_eq!(
                    runtime
                        .test_function_debug_location(&root, Some(pc))
                        .unwrap(),
                    Some((
                        JsString::from_static("primary.js"),
                        crate::LineColumn::new(0, 0)
                    )),
                    "source: {source}, pc: {pc}"
                );
            }
        }
    }

    #[test]
    fn root_and_ordinary_function_names_stay_distinct() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let root = context.compile("(function(){ return 1; })").unwrap();
        let child = runtime.test_child_function_bytecode(&root, 0).unwrap();

        assert_eq!(
            runtime.test_function_name(&root).unwrap(),
            Some(JsString::from_static("<eval>"))
        );
        assert_eq!(runtime.test_function_name(&child).unwrap(), None);
        assert_eq!(
            runtime.test_function_debug_location(&root, None).unwrap(),
            Some((
                JsString::from_static(super::DEFAULT_EVAL_FILENAME),
                crate::LineColumn::new(0, 0)
            ))
        );
    }

    #[test]
    fn filename_atom_ownership_counts_every_function_and_same_atom_use() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let baseline_atoms = runtime.test_atom_count();
        let root = context
            .compile_with_filename("(function(){ return same; })", "same")
            .unwrap();
        let child = runtime.test_child_function_bytecode(&root, 0).unwrap();

        assert_eq!(
            runtime.test_debug_filename_atom_ownership(&root).unwrap(),
            Some((2, Some(4)))
        );
        assert_eq!(
            runtime.test_debug_filename_atom_ownership(&child).unwrap(),
            Some((2, Some(4)))
        );
        drop(root);
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 1);
        assert_eq!(
            runtime.test_function_debug_source(&child).unwrap(),
            Some(b"function(){ return same; }".to_vec())
        );
        drop(child);
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }

    #[test]
    fn runtime_strip_mode_controls_debug_payload_and_filename_atom_ownership() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let baseline_atoms = runtime.test_atom_count();

        runtime.set_debug_info_mode(DebugInfoMode::StripSource);
        let root = context
            .compile_with_filename("(function(){})", "strip-source-unique.js")
            .unwrap();
        let child = runtime.test_child_function_bytecode(&root, 0).unwrap();
        assert_eq!(
            runtime.test_function_debug_location(&child, None).unwrap(),
            Some((
                JsString::from_static("strip-source-unique.js"),
                crate::LineColumn::new(0, 1),
            ))
        );
        assert_eq!(runtime.test_function_debug_source(&child).unwrap(), None);
        assert!(
            runtime
                .test_debug_filename_atom_ownership(&child)
                .unwrap()
                .is_some()
        );
        drop(root);
        drop(child);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);

        runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
        let root = context
            .compile_with_filename("(function(){})", "strip-debug-unique.js")
            .unwrap();
        let child = runtime.test_child_function_bytecode(&root, 0).unwrap();
        assert_eq!(
            runtime.test_function_debug_location(&child, None).unwrap(),
            None
        );
        assert_eq!(runtime.test_function_debug_source(&child).unwrap(), None);
        assert_eq!(
            runtime.test_debug_filename_atom_ownership(&child).unwrap(),
            None
        );
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        drop(root);
        drop(child);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }
}
