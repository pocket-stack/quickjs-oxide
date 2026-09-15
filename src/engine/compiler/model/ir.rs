//! Linear operations and source sites preserved through resolution and lowering.

use super::scope::ScopeId;
use crate::engine::code::bytecode::{DynamicEnvironmentSource, Instruction};
use crate::engine::compiler::lexer::Span;
use crate::engine::value::{JsString, PrimitiveValue as Value};
use crate::source::SourceOffset;
use std::rc::Rc;

pub(in crate::engine::compiler) type FunctionId = usize;

#[derive(Debug)]
pub(in crate::engine::compiler) enum IrConstant {
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
pub(in crate::engine::compiler) enum IdentifierAccess {
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
pub(in crate::engine::compiler) enum IdentifierReferenceAccess {
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
pub(in crate::engine::compiler) enum PrivateFieldAccess {
    Get,
    GetKeepReceiver,
    Put,
    Define,
    In,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum CallArguments {
    Fixed(u16),
    Spread,
}

#[derive(Debug)]
pub(in crate::engine::compiler) enum IrOp {
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
pub(in crate::engine::compiler) struct SpannedIrOp {
    pub(in crate::engine::compiler) op: IrOp,
    /// `Some` is the typed equivalent of QuickJS `emit_source_pos`; `None`
    /// leaves the previous source position in force.
    pub(in crate::engine::compiler) pc_site: Option<SourceOffset>,
}

impl IrOp {
    pub(in crate::engine::compiler) fn stack_effect(&self) -> (usize, usize) {
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

pub(in crate::engine::compiler) mod function;
