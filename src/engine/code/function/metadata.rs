//! Function, parameter, closure, and eval descriptors shared by compilation and execution.
//! Heap allocation and published-node ownership remain in the heap module.

use crate::engine::atom::Atom;

/// Body storage selected for one named physical parameter after its
/// initializer-visible lexical cell has been initialized. Ordinary parameter
/// environments keep the raw argument slot; the explicit enum leaves room
/// for QuickJS's direct-eval `arguments` override without weakening the ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParameterBodyStorage {
    Argument(u16),
    #[expect(
        dead_code,
        reason = "Recognized by validation; current producers do not emit this form."
    )]
    Local(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParameterArgumentCell {
    pub argument: u16,
    pub parameter_local: u16,
    pub body: ParameterBodyStorage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParameterPatternCopy {
    pub parameter_local: u16,
    pub body_local: u16,
}

/// Authored top-level initializer attached to one formal. Defaults nested
/// inside a BindingPattern deliberately do not appear here: QuickJS lets only
/// the whole formal initializer cut `Function.length` and select the incoming
/// argument before destructuring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParameterDefaultSource {
    Argument(u16),
    RestPattern(u16),
}

/// Immutable, publication-authenticated description of QuickJS's parentless
/// argument scope. `Some` with empty cell arrays is semantically meaningful:
/// a standalone `=` can create a zero-cell environment whose expressions are
/// still barred from the function's variable scope.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ParameterEnvironmentLayout {
    pub initialization_end: u32,
    pub argument_cells: Box<[ParameterArgumentCell]>,
    pub pattern_copies: Box<[ParameterPatternCopy]>,
    pub default_sources: Box<[ParameterDefaultSource]>,
    /// QuickJS's sloppy direct-eval arg-scope `arguments` cell.
    pub synthetic_arguments_local: Option<u16>,
    /// The independent sloppy direct-eval `<arg_var>` object.
    pub arg_eval_variable_object_local: Option<u16>,
}

/// Immutable execution metadata kept beside bytecode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FunctionMetadata {
    pub argument_count: u16,
    /// QuickJS's observable `length`, distinct from frame slot count. A
    /// no-default terminal rest BindingPattern normally increments this even
    /// though it owns no physical slot. QuickJS loses that increment when the
    /// compiled function has zero arguments and zero locals because its
    /// zero-initialized bytecode record skips the metadata copy. Direct
    /// owning-function HomeObject/`this`/`new.target` reads count as QuickJS
    /// hidden locals even though this bytecode model does not allocate them a
    /// slot.
    pub defined_argument_count: u16,
    /// Physical argument slot initialized by authenticated `Rest` bytecode.
    /// `None` means this bytecode has no identifier rest parameter.
    pub rest_parameter: Option<u16>,
    /// First trailing argument consumed by a terminal rest BindingPattern.
    /// This equals `argument_count` because QuickJS allocates no named frame
    /// slot for the rest Array before destructuring it.
    pub rest_pattern_start: Option<u16>,
    /// Number of leading locals owned by the independent parameter
    /// environment. Each named formal contributes one mutable cell and each
    /// BindingPattern contributes one per BoundName; a standalone `=` can also
    /// create a meaningful zero-cell environment. The immutable layout assigns
    /// every leading cell an argument or pattern-copy role.
    pub parameter_environment_local_count: u16,
    /// Number of physical argument slots whose authored formal is a
    /// BindingPattern rather than a BindingIdentifier. The exact positions are
    /// cross-checked against anonymous argument definitions at publication;
    /// keeping the count explicit distinguishes semantic anonymity from
    /// ordinary bytecode whose debug argument names were erased.
    pub pattern_argument_count: u16,
    /// Bytecode PC of the authenticated Nop separating BindingPattern
    /// evaluation from the authored body. Parameter environments without a
    /// BindingPattern carry the same boundary only in
    /// `ParameterEnvironmentLayout::initialization_end`.
    pub parameter_pattern_end: Option<u32>,
    pub local_count: u16,
    /// Synthetic local initialized to the active function object for a named
    /// function expression. This is the typed equivalent of QuickJS's
    /// `func_var_idx` entry prologue.
    pub function_name_local: Option<u16>,
    /// Authenticated one-shot lexical `this` cell for a derived class
    /// constructor. Arrows/direct eval may capture this slot, but only the
    /// derived initialization opcodes may transition it out of TDZ.
    pub derived_this_local: Option<u16>,
    /// Authenticated QuickJS `this_active_func` pseudo local. It is initialized
    /// from the executing function object before parameters run and captured
    /// by arrows/direct eval which contain `super()`.
    pub active_function_local: Option<u16>,
    /// Synthetic local which owns QuickJS's hidden `<var>` object for one
    /// sloppy ordinary-function activation containing syntactic direct eval.
    /// Dynamic eval-name opcodes may name only this authenticated slot.
    pub eval_variable_object_local: Option<u16>,
    pub closure_count: u16,
    pub max_stack: u16,
    pub strict: bool,
    /// QuickJS `strip_var_debug`: this function sampled StripDebug and has no
    /// syntactic direct eval, so ordinary local/closure names are hidden from
    /// TDZ diagnostics even when Oxide retains them as semantic metadata.
    pub strip_variable_debug: bool,
    /// Authenticates the parentless callable owned by one Module record. A
    /// nested function may never carry this bit.
    pub is_module: bool,
    /// Whether `super()` is syntactically permitted in this bytecode. QuickJS
    /// carries this independently from HomeObject storage so direct eval can
    /// inherit constructor authority without inferring it from captured data.
    pub super_call_allowed: bool,
    /// Whether `super.` or `super[]` is syntactically permitted in this
    /// bytecode. This is parser authority, distinct from `needs_home_object`,
    /// which only controls whether closure publication retains an object edge.
    pub super_allowed: bool,
    /// QuickJS parser capability inherited by class field/static-block arrows
    /// and direct eval. When set, an implicit `arguments` binding is a syntax
    /// error rather than a lookup which can fall through to an outer/global
    /// environment.
    pub arguments_forbidden: bool,
    /// Whether closure publication must accept a method HomeObject. QuickJS
    /// derives this from `home_object_var_idx`/`need_home_object`; ordinary
    /// functions leave the flag clear and therefore retain no object edge.
    pub needs_home_object: bool,
    /// Whether this bytecode is the synthetic root compiled for an
    /// ECMAScript eval invocation. Ordinary scripts/functions use `None`;
    /// nested functions inside eval code also use `None` because only the
    /// synthetic root consumes an external caller environment.
    pub eval_kind: EvalKind,
    /// Source-level callable kind used by `Function.prototype.toString` when
    /// debug source has been stripped. This mirrors QuickJS `func_kind`
    /// independently from constructor protocol.
    pub function_kind: FunctionKind,
    /// Whether closure instantiation defines an own `.prototype` property.
    pub has_prototype: bool,
    /// Base/derived constructor protocol carried by QuickJS bytecode.
    pub constructor_kind: ConstructorKind,
    /// Synthetic class-element program emitted by `js_parse_class`-equivalent
    /// lowering.  Keeping this role orthogonal to ordinary/generator/async
    /// function kind lets publication and the VM reject forged initializer
    /// calls without exposing a JavaScript-visible marker.
    pub class_initializer_kind: Option<ClassInitializerKind>,
    /// Whether an aggregate instance/static class initializer owns the
    /// hidden brand lifecycle for private methods on that class side.
    /// Static blocks and ordinary authored functions may never carry it.
    pub class_private_brand: bool,
}

/// QuickJS eval type carried by one synthetic eval bytecode root.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EvalKind {
    #[default]
    None,
    Direct,
    Indirect,
}

/// QuickJS bytecode callable kind. Keeping it in immutable function metadata
/// makes source stripping a representation change rather than a semantic one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FunctionKind {
    #[default]
    Normal,
    Generator,
    Async,
    AsyncGenerator,
}

/// Bytecode constructor protocol. Derived constructors deliberately remain a
/// separate state because they do not pre-create `this` and apply different
/// return validation in the caller realm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ConstructorKind {
    #[default]
    None,
    Base,
    Derived,
}

/// Authenticated role of one synthetic class-element bytecode function.
///
/// QuickJS compiles public instance fields and the ordered static element
/// sequence into hidden functions, with every static block represented by a
/// nested hidden child.  Oxide preserves those three distinct authorities in
/// typed metadata even though all three execute as ordinary synchronous
/// bytecode call frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClassInitializerKind {
    InstanceFields,
    StaticElements,
    StaticBlock,
}

/// Where one child function obtains a closure slot when `FClosure` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClosureSource {
    ParentLocal(u16),
    ParentArgument(u16),
    ParentClosure(u16),
    /// Define a declared global binding when the eval/script closure is instantiated.
    GlobalDeclaration,
    /// Resolve a global binding when the eval/script closure is instantiated.
    Global,
    /// Reuse a parent's global binding without re-resolving the name.
    ParentGlobal(u16),
    /// Attach the synthetic direct-eval root to the exact live caller binding
    /// at this flattened environment index. This source is valid only on a
    /// verified direct-eval root and is instantiated from caller-owned
    /// `VarRef` roots in the same synchronous operation that publishes it.
    EvalEnvironment(u16),
    /// Root VarRef owned by an ECMAScript Module Environment Record.
    ModuleDeclaration,
    /// Root live binding supplied by a requested module during linking.
    ModuleImport,
    /// Root import slot which also receives a pinned QuickJS declaration-time
    /// raw write. This provenance never propagates through child closures.
    ModuleImportCollision,
    /// Hidden, immutable cell containing one module record's cached
    /// null-prototype `import.meta` object.
    ModuleImportMeta,
}

/// Closure-name representation before and after runtime publication.
///
/// Compiler drafts point at an exact string constant. Publication interns it
/// and replaces the operand with an atom owned by `auxiliary_atoms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClosureVariableName {
    None,
    Constant(u32),
    Atom(Atom),
}

/// Semantics carried along a closure relay in addition to its storage source.
/// QuickJS uses `JS_VAR_FUNCTION_NAME` to distinguish the immutable private
/// name of a function expression from an ordinary mutable local.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ClosureVariableKind {
    #[default]
    Normal,
    /// Sealed descriptor-only view of an imported ECMAScript module binding.
    ///
    /// The underlying VarRef remains the exporting module's ordinary
    /// [`Normal`](Self::Normal) cell. This kind authenticates the one legal
    /// metadata mismatch: an immutable lexical importing view over that live
    /// cell. It may appear on module-root, closure-relay, and direct-eval
    /// descriptors, but never on a VariableDefinition or VarRefData cell.
    ModuleImportView,
    FunctionName,
    /// QuickJS `JS_VAR_GLOBAL_FUNCTION_DECL`: a Program function declaration
    /// whose global-property preflight and creation rules differ from `var`.
    GlobalFunction,
    /// QuickJS's hidden `<var>` binding. A sloppy ordinary function with a
    /// syntactic direct-eval site stores one null-prototype variable object in
    /// this binding; eval-created names are properties of that object rather
    /// than fabricated ordinary locals.
    EvalVariableObject,
    /// QuickJS's hidden `<arg_var>` binding. A sloppy authored function with
    /// both a Parameter Environment and a syntactic direct-eval site stores a
    /// second, independent variable object here. Parameter-phase lookup uses
    /// this object as its declaration target; body lookup consults it only
    /// after the ordinary `<var>` object.
    ArgEvalVariableObject,
    /// QuickJS's hidden `<with>` binding. The binding carries the object
    /// environment introduced by one sloppy `with` statement and is relayed
    /// through closures and direct-eval environment descriptors without ever
    /// becoming a source-visible lexical or variable binding.
    WithObject,
    /// A class-private data-field identity. Its cell contains a fresh private
    /// atom for each class evaluation and may only be consumed through typed
    /// private-element bytecode; ordinary local/VarRef reads must reject it.
    PrivateField,
    /// A class-private method closure. Its cell contains the single callable
    /// shared by every branded receiver for one evaluated class side. Like a
    /// private-name cell, it is an immutable lexical capability which may
    /// only be consumed through authenticated private-element bytecode.
    PrivateMethod,
    /// Primary callable cell for a private getter.
    PrivateGetter,
    /// Private setter capability. Both the uninitialized source-visible
    /// primary cell of a setter-only declaration and the initialized synthetic
    /// `<set>` callable cell carry this exact QuickJS kind.
    PrivateSetter,
    /// Primary getter cell for a paired private getter/setter declaration.
    PrivateGetterSetter,
}

impl ClosureVariableKind {
    #[must_use]
    pub const fn is_eval_variable_object(self) -> bool {
        matches!(self, Self::EvalVariableObject | Self::ArgEvalVariableObject)
    }

    #[must_use]
    pub const fn is_private(self) -> bool {
        matches!(
            self,
            Self::PrivateField
                | Self::PrivateMethod
                | Self::PrivateGetter
                | Self::PrivateSetter
                | Self::PrivateGetterSetter
        )
    }
}

/// Runtime-independent closure metadata stored beside child bytecode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClosureVariable {
    pub source: ClosureSource,
    pub name: ClosureVariableName,
    pub is_lexical: bool,
    pub is_const: bool,
    pub kind: ClosureVariableKind,
}

/// Runtime-owned authoritative argument/local definition metadata.
///
/// This is the published counterpart of QuickJS's `JSVarDef`: present names
/// are atoms whose references are owned by the containing bytecode node's
/// `auxiliary_atoms` array.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VariableDefinition {
    pub name: Option<Atom>,
    pub is_lexical: bool,
    pub is_const: bool,
    /// True only for a nested lexical scope whose lifetime is wholly inside
    /// formal-parameter initialization.
    pub is_parameter_initializer: bool,
    pub kind: ClosureVariableKind,
}

/// Storage occupied by a binding visible to one syntactic direct-eval site.
///
/// Unlike [`ClosureSource`], these sources always refer to the function whose
/// bytecode contains the eval instruction.  Publication verifies the source
/// against that function's authoritative argument/local/closure metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EvalBindingSource {
    Local(u16),
    Argument(u16),
    Closure(u16),
}

/// Syntactic scope represented in a direct-eval environment descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EvalScopeKind {
    FunctionRoot,
    /// QuickJS's parentless argument scope for a function whose formal
    /// parameters contain expressions. This terminates one function segment
    /// without making the function's body/root bindings visible.
    Parameter,
    FunctionBody,
    ProgramBody,
    Block,
    If,
    For,
    Switch,
    Catch,
    With,
}

/// Variable-environment destination used for declarations introduced by eval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EvalVariableEnvironment {
    Global,
    /// Strict direct eval always creates declarations in its own eval frame.
    /// The scope ordinal authenticates the current caller-function segment
    /// which grants that strict-local destination; it may be a FunctionRoot or
    /// a parentless Parameter scope.
    StrictLocal(u16),
    /// Sloppy direct eval writes declarations through one exact hidden
    /// variable-object binding. `scope` identifies the descriptor scope which
    /// must contain `source`; publication rejects an Argument source and
    /// authenticates whether it is the body `<var>` or parameter `<arg_var>`.
    VariableObject {
        scope: u16,
        source: EvalBindingSource,
    },
}

/// Caller-side provenance retained while compiling one synthetic eval root.
///
/// `ExternalBinding` names the exact flattened caller binding which owns the
/// variable environment.  It is deliberately not a closure-vector guess:
/// the runtime derives it from the authenticated call-site descriptor before
/// the compiler can relay that binding into nested eval bytecode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EvalCallerVariableTarget {
    Global,
    ExternalBinding(u16),
    StrictLocal,
}

/// Shape of the caller scope chain imported by a synthetic eval root.
///
/// Bindings themselves remain in the root's ordered `EvalRootBinding` prefix;
/// their `scope` ordinals index this exact kind vector.  Keeping empty scopes
/// here is required to reproduce catch/block/function topology when that eval
/// source later contains another direct eval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalCallerProfile {
    pub scope_kinds: Box<[EvalScopeKind]>,
    pub variable_target: EvalCallerVariableTarget,
}

/// One named binding visible from a syntactic direct-eval call site.
///
/// Compiler drafts use [`JsString`](crate::engine::value::JsString) names. Runtime publication interns each
/// name and stores the corresponding [`Atom`] while the owning bytecode node
/// retains that atom through `auxiliary_atoms`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalBinding<Name> {
    pub name: Name,
    pub source: EvalBindingSource,
    pub is_lexical: bool,
    pub is_const: bool,
    pub kind: ClosureVariableKind,
    /// Declaration provenance needed when a direct eval imports this binding:
    /// unlike ordinary lexical bindings, QuickJS permits a sloppy eval `var`
    /// declaration to reuse the live catch-parameter cell.
    pub is_catch_parameter: bool,
}

/// One lexical scope visible from a syntactic direct-eval call site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalScope<Name> {
    pub kind: EvalScopeKind,
    pub bindings: Box<[EvalBinding<Name>]>,
}

/// Immutable description of the bindings and declaration destination visible
/// to one syntactic direct-eval call site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalEnvironment<Name> {
    pub scopes: Box<[EvalScope<Name>]>,
    pub variable_environment: EvalVariableEnvironment,
    pub caller_strict: bool,
    /// Exact caller-bytecode syntax capability copied into a direct eval root.
    pub super_call_allowed: bool,
    /// Exact caller-bytecode syntax capability copied into a direct eval root.
    pub super_allowed: bool,
}

/// One exact caller binding imported by a synthetic direct-eval root.
///
/// The index of an entry in this list is also the operand carried by
/// [`ClosureSource::EvalEnvironment`]. `scope` preserves provenance back to
/// the immutable R1w environment descriptor even though identifier lookup in
/// the eval compiler only needs innermost-name precedence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalRootBinding<Name> {
    pub name: Name,
    pub scope: u16,
    pub is_lexical: bool,
    pub is_const: bool,
    pub kind: ClosureVariableKind,
    /// Catch parameters are represented as lexical locals by the Oxide IR,
    /// but pinned QuickJS permits sloppy eval `var` to reuse their live cell.
    /// Retain that declaration-only distinction without weakening ordinary
    /// lexical reads, TDZ checks, or closure metadata.
    pub is_catch_parameter: bool,
}
