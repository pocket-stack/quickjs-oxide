//! Source-to-bytecode compilation with late lexical-name resolution.
//!
//! QuickJS first emits scope-variable operations, then `resolve_scope_var`
//! rewrites them after every nested function and lexical scope is known.  Its
//! `get_closure_var` helper also installs relay closure slots on intervening
//! functions.  This module keeps the same boundary in typed form: parsing emits
//! [`IrOp`]s into a recursive [`FunctionIr`] arena, identifier resolution runs
//! child-first, and only then are VM instructions and recursive unlinked
//! function constants produced.

use crate::bigint::JsBigInt;
use crate::bytecode::{BytecodeFunction, Instruction, verify_parts};
use crate::debug::{DebugInfoMode, Pc2LineEntry, Pc2LineTable, QuickJsSourceLocator, SourceOffset};
use crate::error::{Error, ErrorKind, SourceLocation, SourceSpan};
use crate::function::{UnlinkedConstant, UnlinkedFunction, UnlinkedFunctionDebug};
use crate::heap::{
    ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName, ConstructorKind,
    FunctionKind as BytecodeFunctionKind, FunctionMetadata,
};
use crate::lexer::{
    Identifier, Keyword, LexError, Lexer, NumberKind, NumericRadix, Punctuator, Span, Token,
    TokenKind,
};
use crate::value::{JsString, Value};
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
    let mut tree = Parser::parse(source, JsString::from(DEFAULT_EVAL_FILENAME))?;
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
    let mut tree = Parser::parse(source, JsString::from_utf8(filename))?;
    resolve_identifiers(&mut tree)?;
    lower_unlinked_tree(tree, debug_info)
}

type FunctionId = usize;
// QuickJS 2026-06-04 `JS_MAX_LOCAL_VARS` and `JS_STACK_SIZE_MAX` are both
// 65,534. Call opcodes encode one more argument count value; the resulting
// operand stack is checked against the smaller stack limit during lowering.
const MAX_LOCAL_VARIABLES: usize = 65_534;
const MAX_BYTECODE_STACK: usize = 65_534;
const MAX_CALL_ARGUMENTS: usize = 65_535;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FunctionKind {
    Script,
    Ordinary,
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
    Put,
    Set,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MemberReference {
    Field { key: u32, site: SourceOffset },
    Computed { site: SourceOffset },
}

#[derive(Debug)]
enum IrOp {
    Bytecode(Instruction),
    PushConstant(u32),
    MakeClosure(u32),
    /// Lowering-only assignment-expression form. QuickJS has no `set_var`;
    /// this expands to `dup; put_var` before verification/publication.
    GlobalSet(u16),
    Identifier {
        name: String,
        span: Span,
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

impl IrOp {
    fn stack_effect(&self) -> (usize, usize) {
        match self {
            Self::Bytecode(instruction) => instruction.stack_effect(),
            Self::PushConstant(_) | Self::MakeClosure(_) => (0, 1),
            Self::GlobalSet(_) => (1, 1),
            Self::Identifier {
                access: IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
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
    parent: Option<FunctionId>,
    kind: FunctionKind,
    source: FunctionSourceInfo,
    /// Intrinsic name of a named function expression, independent of
    /// contextual `SetName` inference for anonymous definitions.
    function_name: Option<String>,
    /// Lazily allocated private self-binding local.
    function_name_local: Option<u16>,
    parameters: Vec<String>,
    locals: Vec<String>,
    ops: Vec<SpannedIrOp>,
    /// Parser-only Reference marker for the final member getter. QuickJS uses
    /// `last_opcode_pos` for the same rewrite, but an explicit index prevents
    /// comma/conditional values from accidentally retaining a method receiver.
    last_member_reference: Option<usize>,
    constants: Vec<IrConstant>,
    closure_variables: Vec<ClosureVariable>,
    stack_depth: usize,
    max_stack: usize,
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
        parent: Option<FunctionId>,
        kind: FunctionKind,
        source: FunctionSourceInfo,
        function_name: Option<String>,
        parameters: Vec<String>,
        strict: bool,
    ) -> Self {
        Self {
            parent,
            kind,
            source,
            function_name,
            function_name_local: None,
            parameters,
            locals: Vec::new(),
            ops: Vec::new(),
            last_member_reference: None,
            constants: Vec::new(),
            closure_variables: Vec::new(),
            stack_depth: 0,
            max_stack: 0,
            strict,
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
    tokens: Vec<Token<'source>>,
    cursor: usize,
    current_function: FunctionId,
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
        let tokens = Lexer::new(source).tokenize().map_err(lex_error)?;
        let strict = directive_prologue_has_use_strict(&tokens);
        let source_span = tokens
            .first()
            .expect("the lexer always emits an EOF token")
            .span;
        let mut parser = Self {
            tokens,
            cursor: 0,
            current_function: 0,
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
                strict,
            )],
        };
        parser.parse_script_body()?;
        Ok(FunctionTree {
            functions: parser.functions,
            source: source.into(),
            filename,
        })
    }

    fn parse_script_body(&mut self) -> Result<(), Error> {
        let mut has_completion_value = false;

        while !self.at_eof() {
            if self.consume_punctuator(Punctuator::Semicolon) {
                continue;
            }
            if has_completion_value {
                self.emit_instruction(Instruction::Drop)?;
            }

            if matches!(self.current().kind, TokenKind::Keyword(Keyword::Function)) {
                return Err(self.unsupported_here(
                    "top-level function declarations and global bindings are not implemented yet",
                ));
            }
            if matches!(self.current().kind, TokenKind::Keyword(Keyword::Var)) {
                return Err(self.unsupported_here(
                    "top-level var declarations and global bindings are not implemented yet",
                ));
            }
            if matches!(self.current().kind, TokenKind::Keyword(Keyword::Return)) {
                return Err(self.syntax_here("'return' is only valid inside a function body"));
            }

            if matches!(self.current().kind, TokenKind::Keyword(Keyword::Throw)) {
                let throw_span = self.current().span;
                self.advance();
                if self.current().line_terminator_before {
                    return Err(Error::syntax(
                        "line terminator not allowed after throw",
                        source_span(throw_span),
                    ));
                }
                self.parse_expression()?;
                self.emit_instruction_at(Instruction::Throw, source_offset(throw_span)?)?;
                has_completion_value = false;
            } else {
                self.parse_expression()?;
                has_completion_value = true;
            }

            if self.consume_punctuator(Punctuator::Semicolon) {
                continue;
            }
            if self.at_eof() {
                break;
            }
            if self.current().line_terminator_before {
                if matches!(self.current().kind, TokenKind::Template(_)) {
                    return Err(self.unsupported_here(
                        "tagged-template continuations are not implemented yet",
                    ));
                }
                continue;
            }
            return Err(self.syntax_here("expected ';' or a line terminator"));
        }

        if !has_completion_value {
            self.emit_instruction(Instruction::Undefined)?;
        }
        self.emit_instruction(Instruction::Return)?;
        Ok(())
    }

    fn parse_function_body(&mut self) -> Result<(), Error> {
        while !self.is_punctuator(Punctuator::RightBrace) {
            if self.at_eof() {
                return Err(self.syntax_here("unterminated function body"));
            }
            if self.consume_punctuator(Punctuator::Semicolon) {
                continue;
            }

            match self.current().kind {
                TokenKind::Keyword(Keyword::Function) => {
                    return Err(self.unsupported_here(
                        "function declarations are not implemented yet; use an anonymous function expression",
                    ));
                }
                TokenKind::Keyword(Keyword::Return) => self.parse_return_statement()?,
                TokenKind::Keyword(Keyword::Throw) => self.parse_throw_statement()?,
                TokenKind::Keyword(Keyword::Var) => self.parse_var_statement()?,
                _ => {
                    self.parse_expression()?;
                    self.emit_instruction(Instruction::Drop)?;
                    self.consume_statement_terminator(true)?;
                }
            }
        }

        // QuickJS ends ordinary function bytecode with `return_undef`. It may
        // be unreachable after an explicit return, but keeps fallthrough
        // behavior structural and gives every function a terminal opcode.
        self.emit_instruction(Instruction::Undefined)?;
        self.emit_instruction(Instruction::Return)?;
        Ok(())
    }

    fn parse_return_statement(&mut self) -> Result<(), Error> {
        let return_span = self.current().span;
        self.advance();
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
                op: IrOp::Bytecode(Instruction::Call(_) | Instruction::CallMethod(_)),
                pc_site,
            }) = self.current_ir_mut().ops.last_mut()
            {
                *pc_site = Some(source_offset(return_span)?);
            }
        }
        self.emit_instruction_at(Instruction::Return, source_offset(return_span)?)?;
        self.consume_statement_terminator(true)
    }

    fn parse_throw_statement(&mut self) -> Result<(), Error> {
        let throw_span = self.current().span;
        self.advance();
        if self.current().line_terminator_before {
            return Err(Error::syntax(
                "line terminator not allowed after throw",
                source_span(throw_span),
            ));
        }
        self.parse_expression()?;
        self.emit_instruction_at(Instruction::Throw, source_offset(throw_span)?)?;
        self.consume_statement_terminator(true)
    }

    fn parse_var_statement(&mut self) -> Result<(), Error> {
        self.advance();
        loop {
            let token = self.current().clone();
            let TokenKind::Identifier(identifier) = token.kind else {
                return Err(self.syntax_here("expected an identifier in var declaration"));
            };
            validate_identifier(&identifier, token.span, self.current_ir().strict, true)?;
            if identifier.value == "arguments"
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
            let name = identifier.value;
            self.register_local(&name, token.span)?;
            self.advance();

            let initializer_span = self.current().span;
            if self.consume_punctuator(Punctuator::Equal) {
                self.parse_assignment()?;
                if self.anonymous_function_definition.take().is_some() {
                    // QuickJS emits a dummy OP_set_name after an anonymous
                    // closure and rewrites its atom when NamedEvaluation
                    // applies to this initializer. Keep that contextual name
                    // separate from the child bytecode's intrinsic func_name.
                    let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                        JsString::from_utf8(&name),
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

            if !self.consume_punctuator(Punctuator::Comma) {
                break;
            }
        }
        self.consume_statement_terminator(true)
    }

    fn register_local(&mut self, name: &str, span: Span) -> Result<(), Error> {
        let function = &mut self.functions[self.current_function];
        if function
            .parameters
            .iter()
            .any(|parameter| parameter == name)
            || function.locals.iter().any(|local| local == name)
        {
            return Ok(());
        }
        if function.locals.len() >= MAX_LOCAL_VARIABLES {
            return Err(
                Error::new(ErrorKind::JsInternal, "too many local variables")
                    .with_span(source_span(span)),
            );
        }
        function.locals.push(name.to_owned());
        Ok(())
    }

    fn consume_statement_terminator(&mut self, allow_right_brace: bool) -> Result<(), Error> {
        if self.consume_punctuator(Punctuator::Semicolon)
            || self.at_eof()
            || (allow_right_brace && self.is_punctuator(Punctuator::RightBrace))
            || self.current().line_terminator_before
        {
            Ok(())
        } else {
            Err(self.syntax_here("expected ';' or a line terminator"))
        }
    }

    fn parse_expression(&mut self) -> Result<(), Error> {
        self.parse_comma()
    }

    fn parse_comma(&mut self) -> Result<(), Error> {
        self.parse_assignment()?;
        let mut has_comma = false;
        while self.is_punctuator(Punctuator::Comma) {
            self.advance();
            has_comma = true;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_assignment()?;
        }
        if has_comma {
            self.anonymous_function_definition = None;
            self.current_ir_mut().last_member_reference = None;
        }
        Ok(())
    }

    /// Parse the currently supported simple assignment targets.
    ///
    /// Keeping the unresolved write as a typed identifier operation lets the
    /// late resolver apply QuickJS's special function-name behavior: normal
    /// bindings use `Set*`, a sloppy private function name is a no-op, and a
    /// strict private function name throws without mutating the cell.
    fn parse_assignment(&mut self) -> Result<(), Error> {
        let token = self.current().clone();
        if let TokenKind::Identifier(identifier) = token.kind
            && self.next_is_punctuator(Punctuator::Equal)
        {
            validate_identifier(&identifier, token.span, self.current_ir().strict, false)?;
            let name = identifier.value;
            self.advance();
            self.advance();
            let rhs_start = self.current_ir().ops.len();
            self.parse_assignment()?;
            self.inherit_source_marker_at(rhs_start, source_offset(token.span)?)?;
            if self.anonymous_function_definition.take().is_some() {
                let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::from_utf8(&name),
                )))?;
                self.emit_instruction(Instruction::SetName(name_constant))?;
            }
            // QuickJS `js_parse_assign_expr2` does not emit a new source
            // position for ordinary `=`. `put_lvalue` inherits whichever
            // marker was last: the LHS marker for an unmarked RHS, or a marker
            // emitted while evaluating the RHS.
            self.emit_identifier_inherited(name, token.span, IdentifierAccess::Set)?;
            self.anonymous_function_definition = None;
            return Ok(());
        }
        self.parse_conditional()?;
        if !self.is_punctuator(Punctuator::Equal) {
            return Ok(());
        }

        let Some(target) = self.take_tail_member_reference()? else {
            return Err(self.syntax_here("invalid assignment left-hand side"));
        };
        self.advance();
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

    fn parse_conditional(&mut self) -> Result<(), Error> {
        self.parse_logical_or()?;
        if !self.is_punctuator(Punctuator::Question) {
            return Ok(());
        }
        self.advance();
        self.anonymous_function_definition = None;

        let false_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        let branch_stack = self.current_ir().stack_depth;
        self.parse_assignment()?;
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
        Ok(())
    }

    fn parse_logical_or(&mut self) -> Result<(), Error> {
        self.parse_logical_and()?;
        let mut composed = false;
        while self.is_punctuator(Punctuator::LogicalOr) {
            composed = true;
            self.advance();
            self.emit_instruction(Instruction::Dup)?;
            let end_jump = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_logical_and()?;
            self.patch_jump(end_jump, self.current_ir().ops.len())?;
            self.anonymous_function_definition = None;
        }
        if self.is_punctuator(Punctuator::NullishCoalesce) {
            return Err(self.unsupported_here("nullish coalescing is not implemented yet"));
        }
        if composed {
            self.current_ir_mut().last_member_reference = None;
        }
        Ok(())
    }

    fn parse_logical_and(&mut self) -> Result<(), Error> {
        self.parse_equality()?;
        let mut composed = false;
        while self.is_punctuator(Punctuator::LogicalAnd) {
            composed = true;
            self.advance();
            self.emit_instruction(Instruction::Dup)?;
            let end_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_equality()?;
            self.patch_jump(end_jump, self.current_ir().ops.len())?;
            self.anonymous_function_definition = None;
        }
        if composed {
            self.current_ir_mut().last_member_reference = None;
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
            self.advance();
            self.parse_relational()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_relational(&mut self) -> Result<(), Error> {
        self.parse_additive()?;
        loop {
            let operation_span = self.current().span;
            let operation = match self.current().kind {
                TokenKind::Punctuator(Punctuator::Less) => Instruction::Lt,
                TokenKind::Punctuator(Punctuator::LessEqual) => Instruction::Lte,
                TokenKind::Punctuator(Punctuator::Greater) => Instruction::Gt,
                TokenKind::Punctuator(Punctuator::GreaterEqual) => Instruction::Gte,
                _ => break,
            };
            self.advance();
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
            self.advance();
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
                TokenKind::Punctuator(Punctuator::Exponent) => {
                    return Err(self.unsupported_here("exponentiation is not implemented yet"));
                }
                _ => break,
            };
            self.advance();
            self.parse_unary()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_unary(&mut self) -> Result<(), Error> {
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Typeof)) {
            self.advance();
            let operand_start = self.current_ir().ops.len();
            self.parse_unary()?;

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
            return Ok(());
        }
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Delete)) {
            let delete_span = self.current().span;
            self.advance();
            let operand_start = self.current_ir().ops.len();
            self.parse_unary()?;
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
                    return Err(Error::syntax(
                        "cannot delete a direct reference in strict mode",
                        source_span(delete_span),
                    ));
                }
                return Err(self.unsupported_here(
                    "sloppy direct-identifier delete resolution is not implemented yet",
                ));
            } else {
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction(Instruction::PushTrue)?;
            }
            self.anonymous_function_definition = None;
            return Ok(());
        }
        let operation_span = self.current().span;
        let operation = match self.current().kind {
            TokenKind::Punctuator(Punctuator::Plus) => Some(Instruction::Plus),
            TokenKind::Punctuator(Punctuator::Minus) => Some(Instruction::Neg),
            TokenKind::Punctuator(Punctuator::Not) => Some(Instruction::Not),
            TokenKind::Keyword(Keyword::Void) => {
                self.advance();
                self.parse_unary()?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction(Instruction::Undefined)?;
                self.anonymous_function_definition = None;
                return Ok(());
            }
            TokenKind::Punctuator(Punctuator::BitNot) => {
                return Err(self.unsupported_here("this unary operator is not implemented yet"));
            }
            _ => None,
        };
        if let Some(operation) = operation {
            self.advance();
            self.parse_unary()?;
            if matches!(operation, Instruction::Plus | Instruction::Neg) {
                self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            } else {
                self.emit_instruction(operation)?;
            }
            self.anonymous_function_definition = None;
            return Ok(());
        }
        self.parse_postfix()
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
                self.advance();
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
            break;
        }
        Ok(())
    }

    /// Parse one member suffix without accepting a call. This is shared by
    /// ordinary postfix chains and constructor heads after `new`, matching
    /// QuickJS's `PF_POSTFIX_CALL` split.
    fn parse_member_suffix(&mut self) -> Result<bool, Error> {
        if self.is_punctuator(Punctuator::Dot) {
            let member_span = self.current().span;
            self.advance();
            let token = self.current().clone();
            let name = match token.kind {
                TokenKind::Identifier(identifier) => identifier.value,
                TokenKind::Keyword(keyword) => keyword.as_str().to_owned(),
                _ => return Err(self.syntax_here("expecting field name")),
            };
            self.advance();
            let key = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::from_utf8(&name),
            )))?;
            let operation =
                self.emit_instruction_at(Instruction::GetField(key), source_offset(member_span)?)?;
            self.current_ir_mut().last_member_reference = Some(operation);
            self.anonymous_function_definition = None;
            return Ok(true);
        }

        if self.is_punctuator(Punctuator::LeftBracket) {
            let member_span = self.current().span;
            self.advance();
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
            function.max_stack = function.max_stack.max(function.stack_depth);
        }
        function.last_member_reference = None;
        Ok(promoted)
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

    /// Parse the contents of an already-consumed call/construct `(`.
    fn parse_call_arguments(&mut self) -> Result<u16, Error> {
        let mut argument_count = 0_usize;
        if !self.consume_punctuator(Punctuator::RightParen) {
            loop {
                // QuickJS accepts 65,535 encoded arguments and only rejects
                // the next one in `js_parse_postfix_expr`. The accepted
                // boundary is subsequently rejected as a JavaScript-visible
                // stack overflow when bytecode stack size is computed.
                if argument_count >= MAX_CALL_ARGUMENTS {
                    return Err(self.syntax_here("Too many call arguments"));
                }
                self.parse_assignment()?;
                argument_count += 1;
                if !self.consume_punctuator(Punctuator::Comma) {
                    self.expect_punctuator(Punctuator::RightParen)?;
                    break;
                }
                if self.consume_punctuator(Punctuator::RightParen) {
                    break;
                }
            }
        }
        u16::try_from(argument_count)
            .map_err(|_| Error::internal("call argument count escaped the parser limit"))
    }

    fn parse_new_expression(&mut self) -> Result<(), Error> {
        let new_span = self.current().span;
        self.advance();
        if self.consume_punctuator(Punctuator::Dot) {
            let token = self.current().clone();
            let TokenKind::Identifier(identifier) = token.kind else {
                return Err(self.syntax_here("expecting target"));
            };
            if identifier.value != "target" {
                return Err(self.syntax_here("expecting target"));
            }
            if matches!(self.current_ir().kind, FunctionKind::Script) {
                return Err(Error::syntax(
                    "new.target only allowed within functions",
                    source_span(new_span),
                ));
            }
            self.advance();
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
            self.advance();
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
                self.advance();
                self.emit_instruction(Instruction::Null)?;
            }
            TokenKind::Keyword(Keyword::False) => {
                self.advance();
                self.emit_instruction(Instruction::PushFalse)?;
            }
            TokenKind::Keyword(Keyword::True) => {
                self.advance();
                self.emit_instruction(Instruction::PushTrue)?;
            }
            TokenKind::Keyword(Keyword::This) => {
                self.advance();
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
                self.advance();
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
                self.advance();
                self.emit_value(Value::String(JsString::from_utf16(string.value.utf16)))?;
            }
            TokenKind::Punctuator(Punctuator::LeftParen) => {
                self.advance();
                self.parse_expression()?;
                self.expect_punctuator(Punctuator::RightParen)?;
            }
            TokenKind::Identifier(identifier) => {
                validate_identifier(&identifier, token.span, self.current_ir().strict, false)?;
                self.advance();
                self.emit_identifier(identifier.value, token.span, IdentifierAccess::Get)?;
            }
            TokenKind::Keyword(Keyword::Function) => {
                self.parse_function_expression()?;
            }
            TokenKind::Keyword(Keyword::New) => {
                self.parse_new_expression()?;
            }
            TokenKind::Keyword(keyword) => {
                return Err(Error::syntax(
                    format!("{} syntax is not implemented yet", keyword.as_str()),
                    source_span(token.span),
                ));
            }
            TokenKind::Template(_) | TokenKind::RegExp(_) | TokenKind::PrivateIdentifier(_) => {
                return Err(self.unsupported_here("this literal form is not implemented yet"));
            }
            TokenKind::Punctuator(punctuator) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    punctuator.as_str()
                )));
            }
            TokenKind::Eof => {
                return Err(self.syntax_here("expected an expression"));
            }
        }
        Ok(())
    }

    fn parse_function_expression(&mut self) -> Result<(), Error> {
        let function_span = self.current().span;
        self.advance();
        let function_name_token =
            if let TokenKind::Identifier(identifier) = self.current().kind.clone() {
                let span = self.current().span;
                validate_identifier(&identifier, span, false, true)?;
                self.advance();
                Some((identifier, span))
            } else {
                None
            };
        self.expect_punctuator(Punctuator::LeftParen)?;

        let mut parameters = Vec::new();
        let mut parameter_tokens = Vec::new();
        if !self.consume_punctuator(Punctuator::RightParen) {
            loop {
                let token = self.current().clone();
                let TokenKind::Identifier(identifier) = token.kind else {
                    return Err(self.syntax_here("missing formal parameter"));
                };
                validate_identifier(&identifier, token.span, false, true)?;
                parameter_tokens.push((identifier.clone(), token.span));
                parameters.push(identifier.value);
                if parameters.len() > MAX_LOCAL_VARIABLES {
                    return Err(Error::new(ErrorKind::JsInternal, "too many arguments")
                        .with_span(source_span(token.span)));
                }
                self.advance();
                if !self.consume_punctuator(Punctuator::Comma) {
                    if !self.consume_punctuator(Punctuator::RightParen) {
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
        let strict = self.functions[parent].strict
            || directive_prologue_has_use_strict(&self.tokens[self.cursor..]);
        if strict {
            if let Some((identifier, span)) = &function_name_token {
                validate_identifier(identifier, *span, true, true)?;
            }
            for (index, (identifier, span)) in parameter_tokens.iter().enumerate() {
                validate_identifier(identifier, *span, true, true)?;
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
        self.functions.push(FunctionIr::new(
            Some(parent),
            FunctionKind::Ordinary,
            FunctionSourceInfo {
                span: function_span,
                definition: source_offset(function_span)?,
                range: None,
            },
            function_name,
            parameters,
            strict,
        ));
        self.current_function = child;
        self.parse_function_body()?;
        let closing_brace = self.current().span;
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
        self.emit_at(IrOp::Identifier { name, span, access }, pc_site)
    }

    fn emit_identifier_inherited(
        &mut self,
        name: String,
        span: Span,
        access: IdentifierAccess,
    ) -> Result<usize, Error> {
        self.emit(IrOp::Identifier { name, span, access })
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
        function.stack_depth = function
            .stack_depth
            .checked_sub(popped)
            .ok_or_else(|| Error::internal("compiler produced a stack underflow"))?;
        function.stack_depth = function
            .stack_depth
            .checked_add(pushed)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        function.max_stack = function.max_stack.max(function.stack_depth);
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
        if self.consume_punctuator(punctuator) {
            Ok(())
        } else {
            Err(self.syntax_here(format!("expected '{}'", punctuator.as_str())))
        }
    }

    fn consume_punctuator(&mut self, punctuator: Punctuator) -> bool {
        if self.is_punctuator(punctuator) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn is_punctuator(&self, punctuator: Punctuator) -> bool {
        matches!(self.current().kind, TokenKind::Punctuator(current) if current == punctuator)
    }

    fn next_is_punctuator(&self, punctuator: Punctuator) -> bool {
        matches!(
            self.tokens.get(self.cursor + 1).map(|token| &token.kind),
            Some(TokenKind::Punctuator(current)) if *current == punctuator
        )
    }

    fn at_eof(&self) -> bool {
        matches!(self.current().kind, TokenKind::Eof)
    }

    fn current(&self) -> &Token<'source> {
        // The lexer always emits exactly one EOF token.
        &self.tokens[self.cursor]
    }

    fn advance(&mut self) {
        if !self.at_eof() {
            self.cursor += 1;
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
}

fn validate_identifier(
    identifier: &Identifier<'_>,
    span: Span,
    strict: bool,
    binding: bool,
) -> Result<(), Error> {
    if identifier.escaped_reserved_word
        || (strict
            && identifier
                .keyword_hint
                .is_some_and(strict_reserved_identifier))
    {
        return Err(Error::syntax(
            format!("'{}' is a reserved identifier", identifier.value),
            source_span(span),
        ));
    }
    if binding && strict && matches!(identifier.value.as_str(), "eval" | "arguments") {
        return Err(Error::syntax(
            format!(
                "'{}' is not a valid strict-mode binding name",
                identifier.value
            ),
            source_span(span),
        ));
    }
    Ok(())
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
enum BindingKind {
    Normal,
    FunctionName { is_const: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Binding {
    Argument(u16),
    Local { index: u16, kind: BindingKind },
}

fn resolve_identifiers(tree: &mut FunctionTree) -> Result<(), Error> {
    // Parents are inserted before children, so reverse arena order is a
    // child-first traversal. Descendant resolution may add a relay descriptor
    // to an intermediate function, just like QuickJS `get_closure_var`.
    for function_id in (0..tree.functions.len()).rev() {
        let unresolved = tree.functions[function_id]
            .ops
            .iter()
            .enumerate()
            .filter_map(|(index, operation)| match &operation.op {
                IrOp::Identifier { name, span, access } => {
                    Some((index, name.clone(), *span, *access))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        for (operation_index, name, span, access) in unresolved {
            let operation = resolve_identifier(tree, function_id, &name, span, access)?;
            tree.functions[function_id].ops[operation_index].op = operation;
        }
    }
    Ok(())
}

fn resolve_identifier(
    tree: &mut FunctionTree,
    function_id: FunctionId,
    name: &str,
    span: Span,
    access: IdentifierAccess,
) -> Result<IrOp, Error> {
    if let Some(binding) = find_or_create_own_binding(tree, function_id, name, span)? {
        return binding_instruction(&mut tree.functions[function_id], binding, access, name)
            .map(IrOp::Bytecode);
    }

    let mut defining_function = tree.functions[function_id].parent;
    let binding = loop {
        let Some(candidate) = defining_function else {
            let closure_index = capture_global_path(tree, function_id, name)?;
            return Ok(match access {
                IdentifierAccess::Get => IrOp::Bytecode(Instruction::GetVar(closure_index)),
                IdentifierAccess::GetOrUndefined => {
                    IrOp::Bytecode(Instruction::GetVarUndef(closure_index))
                }
                IdentifierAccess::Put => IrOp::Bytecode(Instruction::PutVar(closure_index)),
                IdentifierAccess::Set => IrOp::GlobalSet(closure_index),
            });
        };
        if let Some(binding) = find_or_create_own_binding(tree, candidate, name, span)? {
            break binding;
        }
        defining_function = tree.functions[candidate].parent;
    };
    let defining_function = defining_function.expect("binding search stopped at a live ancestor");
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
        cursor = tree.functions[function_id].parent;
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
    let name = JsString::from(name);
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
    name: &str,
    span: Span,
) -> Result<Option<Binding>, Error> {
    let function = &tree.functions[function_id];
    if let Some(index) = function
        .parameters
        .iter()
        .rposition(|parameter| parameter == name)
    {
        return u16::try_from(index)
            .map(Binding::Argument)
            .map(Some)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"));
    }
    if let Some(index) = function.locals.iter().position(|local| local == name) {
        let index = u16::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        let kind = if function.function_name_local == Some(index) {
            BindingKind::FunctionName {
                is_const: function.strict,
            }
        } else {
            BindingKind::Normal
        };
        return Ok(Some(Binding::Local { index, kind }));
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
    function.locals.push(name.to_owned());
    function.function_name_local = Some(index);
    Ok(Some(Binding::Local {
        index,
        kind: BindingKind::FunctionName {
            is_const: function.strict,
        },
    }))
}

fn binding_instruction(
    function: &mut FunctionIr,
    binding: Binding,
    access: IdentifierAccess,
    name: &str,
) -> Result<Instruction, Error> {
    match (binding, access) {
        (Binding::Argument(index), IdentifierAccess::Get | IdentifierAccess::GetOrUndefined) => {
            Ok(Instruction::GetArg(index))
        }
        (Binding::Argument(index), IdentifierAccess::Put) => Ok(Instruction::PutArg(index)),
        (Binding::Argument(index), IdentifierAccess::Set) => Ok(Instruction::SetArg(index)),
        (
            Binding::Local { index, .. },
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
        ) => Ok(Instruction::GetLocal(index)),
        (
            Binding::Local {
                index,
                kind: BindingKind::Normal,
            },
            IdentifierAccess::Put,
        ) => Ok(Instruction::PutLocal(index)),
        (
            Binding::Local {
                index,
                kind: BindingKind::Normal,
            },
            IdentifierAccess::Set,
        ) => Ok(Instruction::SetLocal(index)),
        (
            Binding::Local {
                kind: BindingKind::FunctionName { is_const },
                ..
            },
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
        IdentifierAccess::Get | IdentifierAccess::GetOrUndefined => {
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
    binding: Binding,
    name: &str,
) -> Result<(u16, BindingKind), Error> {
    let mut path = Vec::new();
    let mut cursor = consuming_function;
    while cursor != defining_function {
        path.push(cursor);
        cursor = tree.functions[cursor]
            .parent
            .ok_or_else(|| Error::internal("closure binding owner is not an ancestor"))?;
    }
    path.reverse();

    let (mut source, kind) = match binding {
        Binding::Argument(index) => (ClosureSource::ParentArgument(index), BindingKind::Normal),
        Binding::Local { index, kind } => (ClosureSource::ParentLocal(index), kind),
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
    if let Some(index) = function
        .closure_variables
        .iter()
        .position(|candidate| *candidate == descriptor)
    {
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
    let max_stack = lower_max_stack(function.max_stack)?;
    let bytecode = BytecodeFunction {
        name: Some("<eval>".to_owned()),
        code,
        constants,
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
        let lowered_ops = lower_ops(function.ops)?;
        let code = lowered_ops.code;
        let constant_count = function.constants.len();
        // Preserve the source-IR overflow classification used for catchable
        // QuickJS InternalError before the expanded bytecode verifier runs.
        lower_max_stack(function.max_stack)?;
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
        let verified =
            verify_parts(&code, constant_count, MAX_BYTECODE_STACK as u16).map_err(|error| {
                if error.message() == "declared maximum stack is smaller than required" {
                    Error::new(ErrorKind::JsInternal, "stack overflow")
                } else {
                    error
                }
            })?;
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
            max_stack: verified.max_stack,
            strict: function.strict,
            function_kind: BytecodeFunctionKind::Normal,
            has_prototype: matches!(function.kind, FunctionKind::Ordinary),
            constructor_kind: if matches!(function.kind, FunctionKind::Ordinary) {
                ConstructorKind::Base
            } else {
                ConstructorKind::None
            },
        };
        let func_name = function.function_name.as_deref().map(JsString::from_utf8);
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
        let unlinked = unlinked.with_name(func_name);
        lowered[function_id] = Some(match debug {
            Some(debug) => unlinked.with_debug(debug),
            None => unlinked,
        });
    }

    lowered[0]
        .take()
        .ok_or_else(|| Error::internal("root script was not lowered"))
}

fn lower_max_stack(max_stack: usize) -> Result<u16, Error> {
    if max_stack > MAX_BYTECODE_STACK {
        return Err(Error::new(ErrorKind::JsInternal, "stack overflow"));
    }
    u16::try_from(max_stack).map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))
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
    Ok(LoweredOps { code, pc_sites })
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

fn directive_prologue_has_use_strict(tokens: &[Token<'_>]) -> bool {
    let use_strict = "use strict".encode_utf16().collect::<Vec<_>>();
    let mut cursor = 0;

    loop {
        let Some(Token {
            kind: TokenKind::String(literal),
            ..
        }) = tokens.get(cursor)
        else {
            return false;
        };
        let candidate = !literal.has_escape && literal.value.utf16 == use_strict;

        let Some(next) = tokens.get(cursor + 1) else {
            return false;
        };
        let consumed = match next.kind {
            TokenKind::Punctuator(Punctuator::Semicolon) => 2,
            TokenKind::Punctuator(Punctuator::RightBrace) | TokenKind::Eof => 1,
            _ if next.line_terminator_before && quickjs_directive_asi_token(&next.kind) => 1,
            _ => return false,
        };
        if candidate {
            return true;
        }
        cursor += consumed;
    }
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
    Error::syntax(error.message, source_span(error.span))
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
    use crate::lexer::{Position, Span};
    use crate::object::{
        AccessorValue, CompleteOrdinaryPropertyDescriptor, DescriptorField,
        OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
    };
    use crate::runtime::{Runtime, RuntimeError};
    use crate::value::{JsString, Value};
    use crate::vm::Vm;

    use super::{
        FunctionIr, FunctionKind, FunctionSourceInfo, MAX_CALL_ARGUMENTS, MAX_LOCAL_VARIABLES,
        SourceOffset, compile_script, compile_unlinked_script, ensure_closure_variable,
    };

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
    fn compiles_primitive_coercion_and_equality() {
        assert_eq!(
            evaluate("'answer: ' + 42"),
            Value::String(JsString::from("answer: 42"))
        );
        assert_eq!(evaluate("'42' == 42"), Value::Bool(true));
        assert_eq!(evaluate("'42' === 42"), Value::Bool(false));
    }

    #[test]
    fn compiles_short_circuit_and_conditional_control_flow() {
        assert_eq!(evaluate("false && 42"), Value::Bool(false));
        assert_eq!(
            evaluate("'left' || 'right'"),
            Value::String(JsString::from("left"))
        );
        assert_eq!(evaluate("false ? 1 : 2"), Value::Int(2));
        assert!(compile_script("true ? 1, 2 : 3").is_err());
        assert_eq!(evaluate("true ? 1 : 2, 3"), Value::Int(3));
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
    fn detached_vm_rejects_runtime_global_execution_explicitly() {
        let error = compile_script("answer").unwrap_err();
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
            Value::String(JsString::from("number"))
        );
        assert_eq!(
            defining_context.eval("typeof missingGlobal").unwrap(),
            Value::String(JsString::from("undefined"))
        );
        assert_eq!(
            defining_context.eval("typeof ((missingGlobal))").unwrap(),
            Value::String(JsString::from("undefined"))
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
            Value::String(value) if value == JsString::from("'missingGlobal' is not defined")
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
            Value::String(JsString::from("'readonly' is read-only"))
        );

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
            Value::String(JsString::from("'inheritedReadOnly' is read-only"))
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
            Value::String(JsString::from("no setter for property"))
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
            Value::String(JsString::from("object"))
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
            Value::String(JsString::from("shadowed is not initialized"))
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
            Value::String(JsString::from("'shadowed' is read-only"))
        );
        assert_eq!(
            context.get_property(&global, &shadowed).unwrap(),
            Value::Int(1)
        );

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
            Value::String(JsString::from("mutableLexical is not initialized"))
        );
        context
            .initialize_global_lexical_for_test("mutableLexical", Value::Int(4))
            .unwrap();
        assert_eq!(context.eval("mutableLexical = 5").unwrap(), Value::Int(5));
        assert_eq!(context.eval("mutableLexical").unwrap(), Value::Int(5));
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
                Some(Value::String(name)) if name == &JsString::from("relayName")
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
                .define_own_property(&global, &hint, &data(Value::String(JsString::from("none"))),)
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
            Value::String(JsString::from("string"))
        );
        assert_eq!(
            context.eval("keyHint = 'none'").unwrap(),
            Value::String(JsString::from("none"))
        );
        assert!(matches!(
            context.eval("null[keyObject]"),
            Err(RuntimeError::Exception)
        ));
        context.take_exception().unwrap().unwrap();
        assert_eq!(
            context.eval("keyHint").unwrap(),
            Value::String(JsString::from("none"))
        );

        assert_eq!(context.eval("'abc'.length").unwrap(), Value::Int(3));
        assert_eq!(
            context.eval("'abc'[1]").unwrap(),
            Value::String(JsString::from("b"))
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
        assert!(context.compile("delete Function").is_err());
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
            Value::String(JsString::from("undefined"))
        );

        let (name, writable, enumerable, configurable) =
            evaluate_function_name("(function named() {})");
        assert_eq!(name, JsString::from("named"));
        assert!(!writable);
        assert!(!enumerable);
        assert!(configurable);

        let (name, ..) = evaluate_function_name(
            "(function() { var inferred = function intrinsic() {}; return inferred; })()",
        );
        assert_eq!(name, JsString::from("intrinsic"));

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
            Some(&Value::String(JsString::from("named")))
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
            Some(&Value::String(JsString::from("named")))
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
            Value::String(JsString::from("TypeError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from("'named' is read-only"))
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
            Value::String(JsString::from("TypeError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from("'named' is read-only"))
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
                (JsString::from("f"), false, false, true),
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
                (JsString::from(""), false, false, true),
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
            Value::String(JsString::from("object"))
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
            Value::String(JsString::from("object"))
        );
        assert_eq!(
            evaluate_in_context("(function(){ return new.target; })()"),
            Value::Undefined
        );

        let error = compile_unlinked_script("new.target").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax);
        assert_eq!(error.message(), "new.target only allowed within functions");
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
            Value::String(JsString::from("InternalError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from("too many arguments"))
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
            Value::String(JsString::from("InternalError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from("stack overflow"))
        );

        let mut too_many = source;
        let closing_parenthesis = too_many
            .rfind(')')
            .expect("generated call expression has a closing parenthesis");
        too_many.insert_str(closing_parenthesis, ",0");
        let error = compile_unlinked_script(&too_many).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax);
        assert_eq!(error.message(), "Too many call arguments");
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
        );
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
            Some((JsString::from("<cmdline>"), crate::LineColumn::new(0, 55)))
        );
        assert_eq!(
            runtime
                .test_function_debug_location(&outer, Some(outer_call))
                .unwrap(),
            Some((JsString::from("<cmdline>"), crate::LineColumn::new(0, 19)))
        );
        assert_eq!(
            runtime
                .test_function_debug_location(&root, Some(root_call))
                .unwrap(),
            Some((JsString::from("<cmdline>"), crate::LineColumn::new(0, 68)))
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
        let expected = Some((JsString::from("globals.js"), crate::LineColumn::new(0, lhs)));
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
                JsString::from("globals.js"),
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
                JsString::from("globals.js"),
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
                JsString::from("globals.js"),
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
            Some((JsString::from("calls.js"), crate::LineColumn::new(0, 5)))
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
                JsString::from("construct.js"),
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
                JsString::from("construct.js"),
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
                    Some((JsString::from("primary.js"), crate::LineColumn::new(0, 0))),
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
            Some(JsString::from("<eval>"))
        );
        assert_eq!(runtime.test_function_name(&child).unwrap(), None);
        assert_eq!(
            runtime.test_function_debug_location(&root, None).unwrap(),
            Some((
                JsString::from(super::DEFAULT_EVAL_FILENAME),
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
                JsString::from("strip-source-unique.js"),
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
