//! Temporary state discarded when a function builder finishes.

use crate::engine::compiler::model::scope::ScopeId;
use crate::engine::compiler::optional_chain::FinalizedOptionalChain;

#[derive(Debug)]
pub(in crate::engine::compiler) struct FunctionParseContext {
    /// Parser-only Reference marker for the final member getter. QuickJS uses
    /// `last_opcode_pos` for the same rewrite, but an explicit index prevents
    /// comma/conditional values from accidentally retaining a method receiver.
    pub(in crate::engine::compiler) last_member_reference: Option<usize>,
    /// Parser-only Reference marker for a final identifier read. This lets
    /// parenthesized IdentifierReferences remain assignment targets while
    /// composed values (comma, conditional, logical and binary forms) do not.
    pub(in crate::engine::compiler) last_identifier_reference: Option<usize>,
    /// A completed optional chain whose value has not yet been composed with
    /// an outer operation. Parentheses deliberately preserve this marker.
    pub(in crate::engine::compiler) last_optional_chain: Option<FinalizedOptionalChain>,
    pub(in crate::engine::compiler) break_controls: Vec<BreakControlContext>,
    pub(in crate::engine::compiler) stack_depth: usize,
    pub(in crate::engine::compiler) current_scope: ScopeId,
    /// YieldExpression is disabled while generator formal initializers parse
    /// and becomes active only after the InitialYield boundary is installed.
    pub(in crate::engine::compiler) in_function_body: bool,
}

impl FunctionParseContext {
    pub(super) fn new(body_scope: ScopeId) -> Self {
        Self {
            last_member_reference: None,
            last_identifier_reference: None,
            last_optional_chain: None,
            break_controls: Vec::new(),
            stack_depth: 0,
            current_scope: body_scope,
            in_function_body: false,
        }
    }
}

use crate::engine::api::error::Error;
use crate::engine::compiler::EvalCompileContext;
use crate::engine::compiler::lexer::Lexer;
use crate::engine::compiler::lexer::Span;
use crate::engine::compiler::lexer::Token;
use crate::engine::compiler::model::bindings::BindingId;
use crate::engine::compiler::model::ir::FunctionId;
use crate::engine::compiler::module;
use crate::engine::compiler::parser::builder::FunctionBuilder;
use crate::source::SourceOffset;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum StatementCompletion {
    Eval,
    Discard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum StatementPosition {
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
    pub(in crate::engine::compiler) const fn allows_other_declaration(self) -> bool {
        matches!(
            self,
            Self::ProgramBody | Self::FunctionBody | Self::NestedList
        )
    }

    pub(in crate::engine::compiler) const fn allows_labelled_annex_b(self) -> bool {
        matches!(
            self,
            Self::ProgramBody | Self::FunctionBody | Self::NestedList | Self::AnnexBLabelBody
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum MemberReference {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) struct IdentifierReference {
    pub(in crate::engine::compiler) name: String,
    pub(in crate::engine::compiler) span: Span,
    pub(in crate::engine::compiler) scope: ScopeId,
    pub(in crate::engine::compiler) object_environment: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum LogicalAssignment {
    And,
    Or,
    Nullish,
}

/// Mirrors QuickJS's `PF_POW_ALLOWED`, `PF_POW_FORBIDDEN`, and zero flag.
/// The zero mode is reserved for prefix-update operands: `++x ** 2` may use
/// the updated value as the left operand, while ordinary unary expressions
/// such as `-x ** 2` are early errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum PowerMode {
    Allowed,
    Forbidden,
    None,
}

/// QuickJS `PF_IN_ACCEPTED`, kept as parser state so recursive assignment RHS
/// inherits ExpressionNoIn while parentheses and selected grammar entries can
/// temporarily restore the ordinary Expression grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum InMode {
    Allow,
    Disallow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum ForHeadDelimiter {
    Parenthesis,
    Bracket,
    Brace,
    Template,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum ForIterationKind {
    In,
    Of,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum ForAssignmentDeclaration {
    Assignment,
    Var,
    Lexical,
}

#[derive(Clone, Debug)]
pub(in crate::engine::compiler) struct ForAssignmentTargetInfo {
    pub(in crate::engine::compiler) declaration: ForAssignmentDeclaration,
    pub(in crate::engine::compiler) var_initializer: Option<IdentifierReference>,
    pub(in crate::engine::compiler) is_destructuring: bool,
}

/// Parser-only counterpart of the breakable-statement part of QuickJS
/// `BlockEnv`. Each function owns its own stack so a nested function cannot
/// target an outer statement. `drop_count` models the values which must be
/// removed when an abrupt jump crosses a control (the retained switch
/// discriminant today). Try/finally unwinding is represented by the dedicated
/// control kinds below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum BreakControlKind {
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
pub(in crate::engine::compiler) struct BreakControlContext {
    pub(in crate::engine::compiler) kind: BreakControlKind,
    pub(in crate::engine::compiler) label_name: Option<String>,
    /// Parser scope active when QuickJS pushes this `BlockEnv`. Abrupt jumps
    /// leave descendant lexical scopes, but keep the matched control's own
    /// scope active until its shared tail runs.
    pub(in crate::engine::compiler) scope: ScopeId,
    pub(in crate::engine::compiler) entry_depth: usize,
    pub(in crate::engine::compiler) drop_count: usize,
    pub(in crate::engine::compiler) break_jumps: Vec<usize>,
    pub(in crate::engine::compiler) continue_jumps: Vec<usize>,
    /// Parser-IR Gosub sites whose common target is known only after the catch
    /// and optional finally clauses have been parsed.
    pub(in crate::engine::compiler) finally_gosubs: Vec<usize>,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::engine::compiler) struct PreparedScopedFunction {
    pub(in crate::engine::compiler) binding: BindingId,
    pub(in crate::engine::compiler) create_annex_binding: bool,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::engine::compiler) enum AnonymousFunctionDefinition {
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
pub(in crate::engine::compiler) enum ModuleDeclarationExport {
    None,
    Named,
    Default,
}

pub(in crate::engine::compiler) struct Parser<'source> {
    pub(in crate::engine::compiler) lexer: Lexer<'source>,
    pub(in crate::engine::compiler) tokens: Vec<Token<'source>>,
    pub(in crate::engine::compiler) cursor: usize,
    pub(in crate::engine::compiler) current_function: FunctionId,
    pub(in crate::engine::compiler) in_mode: InMode,
    pub(in crate::engine::compiler) functions: Vec<FunctionBuilder>,
    pub(in crate::engine::compiler) module: Option<module::IrModule>,
    pub(in crate::engine::compiler) module_declaration_export: ModuleDeclarationExport,
    pub(in crate::engine::compiler) module_declaration_export_target: Option<(FunctionId, ScopeId)>,
    /// Function expression eligible for QuickJS's assignment-name inference.
    /// Operators which make the surrounding expression cease to be an
    /// AnonymousFunctionDefinition clear this marker.
    pub(in crate::engine::compiler) anonymous_function_definition:
        Option<AnonymousFunctionDefinition>,
    /// First syntactically valid construct which belongs to an unimplemented
    /// engine frontier. The parser keeps going so later grammar and early
    /// errors retain QuickJS priority over the implementation diagnostic.
    pub(in crate::engine::compiler) pending_unsupported: Option<Error>,
}

pub(in crate::engine::compiler) enum RootCompileContext {
    Script,
    Module,
    Eval(EvalCompileContext),
}
