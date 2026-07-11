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

use crate::bigint::JsBigInt;
use crate::bytecode::{BytecodeFunction, Instruction, MAX_LOCAL_SLOTS, verify_parts};
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
use std::ops::Range;

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
/// Returns a syntax error for invalid source and for grammar which has not yet
/// reached the feature-parity implementation path.
pub fn compile_script(source: &str) -> Result<BytecodeFunction, Error> {
    let mut tree = Parser::parse(source, JsString::from_static(DEFAULT_EVAL_FILENAME))?;
    if tree.functions.len() != 1 {
        return Err(Error::syntax(
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
enum ScopeKind {
    FunctionRoot,
    FunctionBody,
    ProgramBody,
    Block,
    If,
    For,
    Switch,
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
    FunctionName { is_const: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingStorage {
    Argument(u16),
    Local(u16),
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
    declaration_span: Option<Span>,
}

#[derive(Debug)]
enum IrConstant {
    Primitive(Value),
    Child(FunctionId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdentifierAccess {
    Get,
    GetOrUndefined,
    Delete,
    Put,
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

#[derive(Debug)]
enum IrOp {
    Bytecode(Instruction),
    /// QuickJS's template parser does not apply the ordinary call parser's
    /// u16 argument guard.  Retain the full count until the bytecode stack
    /// limit has been checked during lowering.
    TemplateCall(usize),
    PushConstant(u32),
    MakeClosure(u32),
    /// Lowering-only assignment-expression form. QuickJS has no `set_var`;
    /// this expands to `dup; put_var` before verification/publication.
    GlobalSet(u16),
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
/// discriminant today); iterator cleanup and finally unwinding remain later
/// slices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BreakControlKind {
    RegularStatement,
    Loop,
    Switch,
}

#[derive(Debug)]
struct BreakControlContext {
    kind: BreakControlKind,
    label_name: Option<String>,
    entry_depth: usize,
    drop_count: usize,
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
}

impl IrOp {
    fn stack_effect(&self) -> (usize, usize) {
        match self {
            Self::Bytecode(instruction) => instruction.stack_effect(),
            Self::TemplateCall(argument_count) => (argument_count + 2, 1),
            Self::PushConstant(_) | Self::MakeClosure(_) => (0, 1),
            Self::GlobalSet(_) => (1, 1),
            Self::Identifier {
                access:
                    IdentifierAccess::Get | IdentifierAccess::GetOrUndefined | IdentifierAccess::Delete,
                ..
            } => (0, 1),
            Self::Identifier {
                access: IdentifierAccess::Put,
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
    /// Intrinsic name of a named function expression, independent of
    /// contextual `SetName` inference for anonymous definitions.
    function_name: Option<String>,
    /// Lazily allocated private self-binding local.
    function_name_local: Option<u16>,
    parameters: Vec<String>,
    locals: Vec<String>,
    scopes: Vec<IrScope>,
    bindings: Vec<IrBinding>,
    current_scope: ScopeId,
    var_scope: ScopeId,
    body_scope: ScopeId,
    /// QuickJS `eval_ret_idx`: the script-only hidden completion local.
    /// Keeping the typed slot separate from its unspellable debug name avoids
    /// confusing it with future source bindings or other synthetic locals.
    eval_ret_local: Option<u16>,
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
        parameters: Vec<String>,
        strict: bool,
    ) -> Result<Self, Error> {
        let (locals, eval_ret_local) = if matches!(kind, FunctionKind::Script) {
            (vec![EVAL_RET_LOCAL_NAME.to_owned()], Some(0))
        } else {
            (Vec::new(), None)
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
        let mut function = Self {
            parent,
            kind,
            source,
            function_name,
            function_name_local: None,
            parameters,
            locals,
            scopes,
            bindings: Vec::new(),
            current_scope,
            var_scope,
            body_scope: body,
            eval_ret_local,
            ops: Vec::new(),
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
            declaration_span,
        });
        self.scopes[storage_scope.0].bindings.push(binding);
        binding
    }

    fn binding_in_scope(&self, scope: ScopeId, name: &str) -> Option<&IrBinding> {
        self.scopes[scope.0]
            .bindings
            .iter()
            .rev()
            .find_map(|binding| {
                let binding = &self.bindings[binding.0];
                (binding.name == name).then_some(binding)
            })
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
}

#[derive(Debug)]
struct FunctionTree {
    functions: Vec<FunctionIr>,
    source: Box<str>,
    filename: JsString,
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
            self.parse_statement_or_decl(StatementCompletion::Eval)?;
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
            self.parse_statement_or_decl(StatementCompletion::Discard)?;
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
    fn parse_statement_or_decl(&mut self, completion: StatementCompletion) -> Result<(), Error> {
        if self.consume_punctuator(Punctuator::Semicolon)? {
            return Ok(());
        }

        if let Some(label_name) = self.label_ahead() {
            return self.parse_labeled_statement(completion, label_name);
        }

        match self.current().kind {
            TokenKind::Punctuator(Punctuator::LeftBrace) => self.parse_block_statement(completion),
            TokenKind::Keyword(Keyword::If) => self.parse_if_statement(completion),
            TokenKind::Keyword(Keyword::While) => self.parse_while_statement(completion, None),
            TokenKind::Keyword(Keyword::Do) => self.parse_do_while_statement(completion, None),
            TokenKind::Keyword(Keyword::For) => self.parse_for_statement(completion, None),
            TokenKind::Keyword(Keyword::Switch) => self.parse_switch_statement(completion),
            TokenKind::Keyword(Keyword::Break) => self.parse_loop_jump_statement(false),
            TokenKind::Keyword(Keyword::Continue) => self.parse_loop_jump_statement(true),
            TokenKind::Keyword(Keyword::Function) => {
                if matches!(self.current_ir().kind, FunctionKind::Script) {
                    Err(self.unsupported_here(
                        "top-level function declarations and global bindings are not implemented yet",
                    ))
                } else {
                    Err(self.unsupported_here(
                        "function declarations are not implemented yet; use an anonymous function expression",
                    ))
                }
            }
            TokenKind::Keyword(Keyword::Var) => {
                if matches!(self.current_ir().kind, FunctionKind::Script) {
                    Err(self.unsupported_here(
                        "top-level var declarations and global bindings are not implemented yet",
                    ))
                } else {
                    self.parse_var_statement()
                }
            }
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
                self.parse_statement_or_decl(completion)?;
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
            self.parse_statement_or_decl(completion)?;
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
        self.parse_statement_or_decl(completion)?;
        let joined_stack = self.current_ir().stack_depth;

        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Else)) {
            let end_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
            self.advance()?;
            self.patch_jump(false_jump, self.current_ir().ops.len())?;
            self.current_ir_mut().stack_depth = branch_stack;
            self.parse_statement_or_decl(completion)?;
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

        self.parse_statement_or_decl(completion)?;
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
        self.parse_statement_or_decl(completion)?;
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
        let classic_head = self.for_head_has_top_level_semicolon();
        self.expect_punctuator(Punctuator::LeftParen)?;
        let scope = self.push_scope(ScopeKind::For);

        // QuickJS parses the classic initializer with PF_IN_ACCEPTED clear.
        // Keep that mode explicit even while the AllowIn operator itself
        // remains a later runtime slice.
        if !self.is_punctuator(Punctuator::Semicolon) {
            if self.for_head_lexical_declaration_ahead()? {
                return Err(self.unsupported_here(
                    "lexical declarations in for heads are not implemented yet",
                ));
            }
            if matches!(self.current().kind, TokenKind::Keyword(Keyword::Var)) {
                if matches!(self.current_ir().kind, FunctionKind::Script) {
                    return Err(self.unsupported_here(
                        "top-level var declarations and global bindings are not implemented yet",
                    ));
                }
                self.advance()?;
                self.parse_var_declarations_with_in(InMode::Disallow)?;
            } else {
                self.parse_expression_no_in()?;
                self.emit_instruction(Instruction::Drop)?;
            }
            self.require_stack_depth(entry_depth, "for initializer")?;
            if !classic_head {
                return Err(
                    self.unsupported_here("for-in and for-of loops are not implemented yet")
                );
            }
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
        self.parse_statement_or_decl(completion)?;
        self.require_stack_depth(entry_depth, "for body")?;
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
                    self.parse_statement_or_decl(completion)?;
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
                    control.kind == BreakControlKind::Loop
                        && control.label_name.as_deref() == Some(label_name)
                }
                Some(label_name) => control.label_name.as_deref() == Some(label_name),
                None if is_continue => control.kind == BreakControlKind::Loop,
                None => control.kind != BreakControlKind::RegularStatement,
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
        let drop_count = self.current_ir().break_controls[target + 1..]
            .iter()
            .try_fold(0_usize, |count, control| {
                count
                    .checked_add(control.drop_count)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))
            })?;
        for _ in 0..drop_count {
            self.emit_instruction(Instruction::Drop)?;
        }
        let entry_depth = self.current_ir().break_controls[target].entry_depth;
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

    fn push_break_control(
        &mut self,
        kind: BreakControlKind,
        label_name: Option<String>,
        entry_depth: usize,
        drop_count: usize,
    ) {
        self.current_ir_mut()
            .break_controls
            .push(BreakControlContext {
                kind,
                label_name,
                entry_depth,
                drop_count,
                break_jumps: Vec::new(),
                continue_jumps: Vec::new(),
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
        self.emit_instruction_at(Instruction::Return, source_offset(return_span)?)?;
        self.consume_statement_terminator()
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
            if name == "arguments"
                && !self
                    .current_ir()
                    .parameters
                    .iter()
                    .any(|parameter| parameter == "arguments")
            {
                return Err(self.unsupported_here(
                    "the implicit ordinary-function arguments binding is not implemented yet",
                ));
            }
            self.register_local(&name, token.span)?;

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

    fn register_local(&mut self, name: &str, span: Span) -> Result<(), Error> {
        let function = &mut self.functions[self.current_function];
        if function
            .binding_in_scope(function.var_scope, name)
            .is_some()
        {
            return Ok(());
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
        function.add_binding(
            function.var_scope,
            function.current_scope,
            name.to_owned(),
            BindingStorage::Local(index),
            BindingKind::Normal,
            Some(span),
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
                self.emit_value(Value::String(JsString::try_from_utf16(string.value.utf16)?))?;
            }
            TokenKind::Punctuator(Punctuator::LeftParen) => {
                self.advance()?;
                self.parse_expression()?;
                self.expect_punctuator(Punctuator::RightParen)?;
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
            TokenKind::Keyword(keyword @ (Keyword::Else | Keyword::Case | Keyword::Default)) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    keyword.as_str()
                )));
            }
            TokenKind::Keyword(keyword) => {
                return Err(Error::syntax(
                    format!("{} syntax is not implemented yet", keyword.as_str()),
                    source_span(token.span),
                ));
            }
            TokenKind::RegExp(_) | TokenKind::PrivateIdentifier(_) => {
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
                self.emit_value(Value::String(JsString::try_from_utf16(cooked.utf16)?))?;
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
        let function_span = self.current().span;
        self.advance()?;
        let function_name_token =
            if let TokenKind::Identifier(identifier) = self.current().kind.clone() {
                let span = self.current().span;
                validate_identifier(&identifier, span, false, IdentifierContext::FunctionName)?;
                self.advance()?;
                Some((identifier, span))
            } else {
                None
            };
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
                    return Err(self.syntax_here(
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
        let is_anonymous = function_name.is_none();
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
        self.emit(IrOp::MakeClosure(constant))?;
        self.anonymous_function_definition = is_anonymous.then_some(child);
        Ok(())
    }

    fn emit_value(&mut self, value: Value) -> Result<(), Error> {
        self.emit_value_with_site(value, None)
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
                Instruction::IfFalse(value) | Instruction::IfTrue(value) | Instruction::Goto(value),
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

    /// QuickJS `is_let(..., DECL_MASK_OTHER)` resolves the sloppy `let`
    /// ambiguity before parsing a classic-for initializer. In particular,
    /// `let [` is always lexical and must never silently execute as a member
    /// assignment while lexical declarations remain an explicit boundary.
    fn for_head_lexical_declaration_ahead(&self) -> Result<bool, Error> {
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
        Ok(matches!(
            next.kind,
            TokenKind::Punctuator(Punctuator::LeftBracket | Punctuator::LeftBrace)
                | TokenKind::Identifier(Identifier {
                    escaped_reserved_word: false,
                    ..
                })
                | TokenKind::Keyword(Keyword::Let | Keyword::Yield | Keyword::Await)
        ))
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
        self.syntax_here(message)
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
        function.current_scope = scope;
        scope
    }

    fn pop_scope(&mut self, expected: ScopeId) -> Result<(), Error> {
        let function = self.current_ir_mut();
        if function.current_scope != expected {
            return Err(Error::internal("parser scope stack is unbalanced"));
        }
        function.current_scope = function.scopes[expected.0]
            .parent
            .ok_or_else(|| Error::internal("cannot pop a function root scope"))?;
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
            Instruction::Goto(target) | Instruction::IfFalse(target) | Instruction::IfTrue(target),
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
                        if binding.storage_scope != function.var_scope
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
                        if std::mem::replace(&mut seen_locals[index], true) {
                            return Err(Error::internal(
                                "local slot has more than one binding identity",
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
        let eval_ret_index = function.eval_ret_local.map(usize::from);
        match function.kind {
            FunctionKind::Script
                if eval_ret_index == Some(0)
                    && function
                        .locals
                        .first()
                        .is_some_and(|name| name == EVAL_RET_LOCAL_NAME) => {}
            FunctionKind::Ordinary if eval_ret_index.is_none() => {}
            _ => {
                return Err(Error::internal(
                    "eval completion slot metadata is malformed",
                ));
            }
        }
        if function
            .locals
            .iter()
            .enumerate()
            .any(|(index, name)| name == EVAL_RET_LOCAL_NAME && Some(index) != eval_ret_index)
            || function
                .bindings
                .iter()
                .any(|binding| binding.name == EVAL_RET_LOCAL_NAME)
        {
            return Err(Error::internal(
                "eval completion slot leaked into source binding lookup",
            ));
        }
        for (index, seen) in seen_locals.into_iter().enumerate() {
            if Some(index) == eval_ret_index {
                if seen {
                    return Err(Error::internal(
                        "eval completion slot has a source binding identity",
                    ));
                }
            } else if !seen {
                return Err(Error::internal(
                    "local slot is missing its binding identity",
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
            if matches!(operation, IrOp::Bytecode(Instruction::ThrowReadOnly(_)))
                && let Some(return_site) = tree.functions[function_id]
                    .ops
                    .get(operation_index + 1)
                    .and_then(|operation| {
                        matches!(operation.op, IrOp::Bytecode(Instruction::Return))
                            .then_some(operation.pc_site)
                            .flatten()
                    })
            {
                // QuickJS emits the return source marker after parsing its
                // expression. When late scope resolution replaces the final
                // write with terminal OP_throw_error, that marker becomes the
                // observable fault site instead of the expression's marker.
                tree.functions[function_id].ops[operation_index].pc_site = Some(return_site);
            }
            tree.functions[function_id].ops[operation_index].op = operation;
        }
    }
    validate_scope_graph(tree)
}

fn resolve_identifier(
    tree: &mut FunctionTree,
    function_id: FunctionId,
    use_scope: ScopeId,
    name: &str,
    span: Span,
    access: IdentifierAccess,
) -> Result<IrOp, Error> {
    if access == IdentifierAccess::Delete
        && name == "arguments"
        && matches!(tree.functions[function_id].kind, FunctionKind::Ordinary)
    {
        // Every ordinary function has an own arguments binding even though
        // the wider arguments-object slice is not materialized yet. Sloppy
        // direct delete is statically false and must not force that object to
        // exist merely to reject deletion.
        return Ok(IrOp::Bytecode(Instruction::PushFalse));
    }
    if let Some(binding) = find_or_create_own_binding(tree, function_id, use_scope, name, span)? {
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
                IdentifierAccess::Put => IrOp::Bytecode(Instruction::PutVar(closure_index)),
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
    let (closure_index, kind) =
        capture_binding_path(tree, defining_function, function_id, binding, name)?;
    closure_binding_instruction(
        &mut tree.functions[function_id],
        closure_index,
        kind,
        access,
        name,
    )
    .map(IrOp::Bytecode)
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
        return Err(Error::syntax(
            "the implicit ordinary-function arguments binding is not implemented yet",
            source_span(span),
        ));
    }
    if function.function_name.as_deref() != Some(name) {
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
        (
            BindingStorage::Argument(index),
            _,
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
        ) => Ok(Instruction::GetArg(index)),
        (BindingStorage::Argument(_) | BindingStorage::Local(_), _, IdentifierAccess::Delete) => {
            Ok(Instruction::PushFalse)
        }
        (BindingStorage::Argument(index), _, IdentifierAccess::Put) => {
            Ok(Instruction::PutArg(index))
        }
        (BindingStorage::Argument(index), _, IdentifierAccess::Set) => {
            Ok(Instruction::SetArg(index))
        }
        (
            BindingStorage::Local(index),
            _,
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
        ) => Ok(Instruction::GetLocal(index)),
        (BindingStorage::Local(index), BindingKind::Normal, IdentifierAccess::Put) => {
            Ok(Instruction::PutLocal(index))
        }
        (BindingStorage::Local(index), BindingKind::Normal, IdentifierAccess::Set) => {
            Ok(Instruction::SetLocal(index))
        }
        (
            BindingStorage::Local(_),
            BindingKind::FunctionName { is_const },
            IdentifierAccess::Put | IdentifierAccess::Set,
        ) => function_name_write_instruction(function, name, is_const, access),
    }
}

fn closure_binding_instruction(
    function: &mut FunctionIr,
    index: u16,
    kind: BindingKind,
    access: IdentifierAccess,
    name: &str,
) -> Result<Instruction, Error> {
    match (kind, access) {
        (_, IdentifierAccess::Get | IdentifierAccess::GetOrUndefined) => {
            Ok(Instruction::GetVarRef(index))
        }
        (_, IdentifierAccess::Delete) => Ok(Instruction::PushFalse),
        (BindingKind::Normal, IdentifierAccess::Put) => Ok(Instruction::PutVarRef(index)),
        (BindingKind::Normal, IdentifierAccess::Set) => Ok(Instruction::SetVarRef(index)),
        (BindingKind::FunctionName { is_const }, IdentifierAccess::Put | IdentifierAccess::Set) => {
            function_name_write_instruction(function, name, is_const, access)
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
        IdentifierAccess::Get | IdentifierAccess::GetOrUndefined | IdentifierAccess::Delete => {
            return Err(Error::internal(
                "function-name write received a read access",
            ));
        }
    })
}

const fn closure_kind(kind: BindingKind) -> ClosureVariableKind {
    match kind {
        BindingKind::Normal => ClosureVariableKind::Normal,
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
    };
    let mut final_index = None;
    for function_id in path {
        let function = &mut tree.functions[function_id];
        let descriptor_name = if matches!(kind, BindingKind::FunctionName { .. }) {
            ClosureVariableName::Constant(ensure_string_constant(function, name)?)
        } else {
            ClosureVariableName::None
        };
        let descriptor = ClosureVariable {
            source,
            name: descriptor_name,
            is_lexical: false,
            is_const: matches!(kind, BindingKind::FunctionName { is_const: true }),
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
        (left, right) => left == right,
    }
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
    let code = lower_ops(function.ops)?.code;
    let constants = function
        .constants
        .into_iter()
        .map(|constant| match constant {
            IrConstant::Primitive(value) => Ok(value),
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
    let mut functions = tree_functions.into_iter().map(Some).collect::<Vec<_>>();
    let mut lowered = (0..function_count).map(|_| None).collect::<Vec<_>>();

    for function_id in (0..function_count).rev() {
        let function = functions[function_id]
            .take()
            .ok_or_else(|| Error::internal("function IR was lowered more than once"))?;
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
            *definition =
                UnlinkedVariableDefinition::ordinary(Some(JsString::try_from_utf8(&binding.name)?));
        }
        let lowered_ops = lower_ops(function.ops)?;
        let code = lowered_ops.code;
        let constant_count = function.constants.len();
        let constants = function
            .constants
            .into_iter()
            .map(|constant| match constant {
                IrConstant::Primitive(value) => unlinked_primitive(value),
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

fn lower_ops(operations: Vec<SpannedIrOp>) -> Result<LoweredOps, Error> {
    let mut offsets = Vec::with_capacity(operations.len() + 1);
    let mut code_len = 0_usize;
    for operation in &operations {
        offsets.push(code_len);
        code_len = code_len
            .checked_add(if matches!(operation.op, IrOp::GlobalSet(_)) {
                2
            } else {
                1
            })
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
            IrOp::Identifier { .. } => {
                return Err(Error::internal(
                    "identifier reached bytecode lowering before resolution",
                ));
            }
        }
    }
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
        | Instruction::IfTrue(target) = instruction
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
    use crate::bytecode::Instruction;
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
    use crate::runtime::{Runtime, RuntimeError};
    use crate::value::{JsString, Value};
    use crate::vm::Vm;

    use super::{
        BindingKind, BindingStorage, FunctionIr, FunctionKind, FunctionSourceInfo,
        MAX_BYTECODE_STACK, MAX_CALL_ARGUMENTS, MAX_LOCAL_VARIABLES, Parser, ScopeKind,
        SourceOffset, compile_script, compile_unlinked_script, ensure_closure_variable, lex_error,
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

        for source in [
            "for(Function.item in Function);",
            "for(Function.item of Function);",
            "for(Function.item of /a;b/);",
            "for(Function.item of 'a;b');",
            "for(Function.item of `a;b`);",
            "for(Function.item of `a${1;2}b`);",
        ] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(
                error.message(),
                "for-in and for-of loops are not implemented yet"
            );
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
        for source in [
            "(function(){ var let=Function; for(let[0]=1;false;); })",
            "(function(){ for(let binding=0;false;); })",
            "(function(){ for(let\nbinding=0;false;); })",
            "(function(){ 'use strict'; for(let binding=0;false;); })",
            "(function(){ for(const binding=0;false;); })",
        ] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(
                error.message(),
                "lexical declarations in for heads are not implemented yet",
                "lexical for-head boundary drifted for {source:?}"
            );
        }
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
        assert!(matches!(
            context.call(&reader, Value::Undefined, &[]),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("TDZ access did not throw an object");
        };
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from_static("shadowed is not initialized"))
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
        assert!(matches!(
            context.eval("typeof mutableLexical"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("typeof on a lexical TDZ did not throw an object");
        };
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from_static("mutableLexical is not initialized"))
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
    fn implicit_arguments_binding_is_not_faked_as_an_undefined_local() {
        assert!(
            compile_unlinked_script("(function() { var arguments; return typeof arguments; })()")
                .is_err()
        );
        for source in [
            "(function() { return arguments; })()",
            "(function arguments() { return arguments; })()",
            "(function named() { return function() { return arguments; }; })",
        ] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(
                error.kind(),
                ErrorKind::Syntax,
                "unexpected kind for {source}"
            );
            assert_eq!(
                error.message(),
                "the implicit ordinary-function arguments binding is not implemented yet"
            );
        }
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
