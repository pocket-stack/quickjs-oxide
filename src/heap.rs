//! Runtime-owned garbage-collected heap primitives.
//!
//! QuickJS combines explicit reference counts with a trial-deletion cycle
//! collector.  This module keeps the same ownership model while replacing raw
//! pointers with typed generational handles:
//!
//! - every heap edge is a raw, runtime-internal owned handle;
//! - publishing any node retains all of its outgoing heap edges;
//! - zero-reference destruction is driven by an iterative queue;
//! - cycle collection computes external references as
//!   `strong_count - internal_incoming_count`, marks their closure, and then
//!   actively dismantles unreachable object and function-bytecode anchors;
//! - shapes, variable-reference cells, and contexts participate in the graph,
//!   but cascade from active anchor destruction rather than acting as cycle
//!   anchors.
//!
//! Atom ownership remains at the runtime boundary.  A shape is expected to
//! arrive with one atom reference for every entry.  Finalization returns those
//! atoms in [`HeapCleanup::atoms`] so the caller can release them without
//! making this low-level arena depend on `AtomTable` mutability or callbacks.

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::rc::Rc;

use crate::atom::Atom;
use crate::bigint::JsBigInt;
use crate::bytecode::{Instruction, MAX_LOCAL_SLOTS, PrivateNameSource};
use crate::debug::Pc2LineTable;
use crate::error::NativeErrorKind;
use crate::regexp::CompiledRegExp;
use crate::shape::{PropertyFlags, PropertyStorageKind, Shape};
use crate::value::JsString;

/// Stable identity of an object slot until that slot is reclaimed.
///
/// The parts are exposed only for diagnostics.  There is intentionally no
/// public constructor: identities must originate from [`Heap::allocate_object`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId {
    index: u32,
    generation: u32,
}

impl ObjectId {
    /// Arena index, intended for diagnostics and serialized debug traces only.
    #[must_use]
    pub const fn debug_index(self) -> u32 {
        self.index
    }

    /// Slot generation, intended for diagnostics and serialized debug traces.
    #[must_use]
    pub const fn debug_generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectId")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

/// Stable identity of a shape slot until that slot is reclaimed.
///
/// Shapes and objects share one arena, but their typed handles prevent normal
/// callers from mixing the two node kinds.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShapeId {
    index: u32,
    generation: u32,
}

impl ShapeId {
    /// Arena index, intended for diagnostics and serialized debug traces only.
    #[must_use]
    pub const fn debug_index(self) -> u32 {
        self.index
    }

    /// Slot generation, intended for diagnostics and serialized debug traces.
    #[must_use]
    pub const fn debug_generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Debug for ShapeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShapeId")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

/// Stable identity of one captured-variable cell.
///
/// QuickJS initially lets a `JSVarRef` point into a live stack frame and moves
/// the value into the `JSVarRef` when that frame closes.  This arena uses the
/// equivalent safe representation in which a captured local lives in its
/// `VarRefData` cell from the moment it is captured.  An active frame owns one
/// `VarRefId` root and every closure slot owns another reference to that same
/// identity, so reads and writes remain shared without storing stack pointers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarRefId {
    index: u32,
    generation: u32,
}

impl VarRefId {
    /// Arena index, intended for diagnostics and serialized debug traces only.
    #[must_use]
    pub const fn debug_index(self) -> u32 {
        self.index
    }

    /// Slot generation, intended for diagnostics and serialized debug traces.
    #[must_use]
    pub const fn debug_generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Debug for VarRefId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VarRefId")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

/// Stable identity of a realm/context node until its arena slot is reclaimed.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextId {
    index: u32,
    generation: u32,
}

impl ContextId {
    /// Arena index, intended for diagnostics and serialized debug traces only.
    #[must_use]
    pub const fn debug_index(self) -> u32 {
        self.index
    }

    /// Slot generation, intended for diagnostics and serialized debug traces.
    #[must_use]
    pub const fn debug_generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Debug for ContextId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContextId")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

/// Stable identity of immutable executable bytecode and its constant pool.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FunctionBytecodeId {
    index: u32,
    generation: u32,
}

impl FunctionBytecodeId {
    /// Arena index, intended for diagnostics and serialized debug traces only.
    #[must_use]
    pub const fn debug_index(self) -> u32 {
        self.index
    }

    /// Slot generation, intended for diagnostics and serialized debug traces.
    #[must_use]
    pub const fn debug_generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Debug for FunctionBytecodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FunctionBytecodeId")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

/// Runtime heap node category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeapNodeKind {
    Object,
    Shape,
    VarRef,
    Context,
    FunctionBytecode,
}

/// Observable arena lifecycle state used by diagnostics and invariant tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeapSlotState {
    Initializing,
    Live,
    ZeroQueued,
    Finalizing,
    Zombie,
    Vacant,
    Retired,
}

/// Failure of a checked heap ownership operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeapError {
    WrongKind {
        expected: HeapNodeKind,
        actual: HeapNodeKind,
    },
    Stale {
        index: u32,
        generation: u32,
    },
    Overflow {
        operation: &'static str,
    },
    Underflow {
        kind: HeapNodeKind,
        index: u32,
        generation: u32,
    },
    Invariant(&'static str),
}

impl fmt::Display for HeapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKind { expected, actual } => {
                write!(
                    formatter,
                    "expected {expected:?} heap node, found {actual:?}"
                )
            }
            Self::Stale { index, generation } => {
                write!(
                    formatter,
                    "stale heap handle at slot {index}, generation {generation}"
                )
            }
            Self::Overflow { operation } => write!(formatter, "heap overflow during {operation}"),
            Self::Underflow {
                kind,
                index,
                generation,
            } => write!(
                formatter,
                "{kind:?} reference-count underflow at slot {index}, generation {generation}"
            ),
            Self::Invariant(message) => write!(formatter, "heap invariant failed: {message}"),
        }
    }
}

impl Error for HeapError {}

/// Heap-internal value payload.
///
/// `Clone` duplicates raw payload bytes and primitive backing stores; it does
/// **not** retain an object edge.  Owned clones may enter the heap only through
/// checked methods such as [`Heap::allocate_object`] and
/// [`Heap::replace_object_slot`], which retain their edges transactionally.
#[derive(Clone, Debug, PartialEq)]
pub enum RawValue {
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    Float(f64),
    BigInt(JsBigInt),
    String(JsString),
    Symbol(Atom),
    /// Heap-internal class-private identity. This owns one private-atom
    /// reference exactly like `Symbol`, but it is not an ECMAScript Value and
    /// must never cross `Runtime::root_raw_value` or enter ordinary storage.
    Private(Atom),
    Object(ObjectId),
    Uninitialized,
    Exception,
}

/// Parallel property payload for one shape entry.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertySlot {
    Data(RawValue),
    /// QuickJS `JS_PROP_VARREF`: an ordinary data descriptor whose mutable
    /// payload lives in a shared variable cell.
    VarRef(VarRefId),
    Accessor {
        get: Option<ObjectId>,
        set: Option<ObjectId>,
    },
    /// QuickJS-style lazy intrinsic property. It has ordinary data-property
    /// flags in the shape; only the payload and owned realm edge are deferred.
    AutoInit(AutoInitProperty),
}

/// Typed autoinit payloads. Keeping the creation realm in the per-object slot
/// mirrors QuickJS's `JSProperty.u.init.realm_and_id` and allows objects which
/// share a shape to retain different realms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AutoInitProperty {
    FunctionPrototype {
        realm: ContextId,
    },
    NativeBuiltin {
        realm: ContextId,
        target: NativeFunctionId,
        name: &'static str,
        length: u8,
        min_readable_args: u8,
    },
    String {
        realm: ContextId,
        value: &'static str,
    },
    ArrayUnscopables {
        realm: ContextId,
    },
    /// QuickJS `JS_OBJECT_DEF` payload for the realm's global `Math` object.
    Math {
        realm: ContextId,
    },
    /// QuickJS `JS_OBJECT_DEF` payload for the realm's global `Reflect` object.
    Reflect {
        realm: ContextId,
    },
    /// QuickJS `JS_OBJECT_DEF` payload for the realm's global `JSON` object.
    Json {
        realm: ContextId,
    },
    #[cfg(test)]
    FailureProbe {
        realm: ContextId,
    },
}

/// Realm-owned identities needed to allocate and dispatch genuine RegExp
/// objects and their string iterators. QuickJS roots the constructor, iterator
/// prototype, and initial instance shape independently from their public
/// property graph because user code may replace or delete those properties
/// after bootstrap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegExpRealmData {
    pub prototype: ObjectId,
    pub constructor: ObjectId,
    /// Realm-local `%RegExpStringIteratorPrototype%`, created with the RegExp
    /// intrinsic and inheriting from this realm's `%IteratorPrototype%`.
    pub string_iterator_prototype: ObjectId,
    pub object_shape: ShapeId,
}

/// Realm-owned identities required to allocate genuine Map objects and their
/// iterators. QuickJS roots the two class prototypes, but not the public Map
/// constructor: deleting the global and `Map.prototype.constructor` edges may
/// therefore make that constructor collectible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapRealmData {
    pub prototype: ObjectId,
    /// Realm-local `%MapIteratorPrototype%`, inheriting from this realm's
    /// `%IteratorPrototype%`.
    pub iterator_prototype: ObjectId,
}

/// Realm-owned identities required to allocate genuine Set objects and their
/// iterators. As with Map, QuickJS roots the two class prototypes without
/// independently rooting the public Set constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetRealmData {
    pub prototype: ObjectId,
    /// Realm-local `%SetIteratorPrototype%`, inheriting from this realm's
    /// `%IteratorPrototype%`.
    pub iterator_prototype: ObjectId,
}

/// Realm-owned identities required by synchronous generator functions and
/// generator instances.
///
/// QuickJS roots both class prototypes independently: generator function
/// objects inherit from `function_prototype`, while generator instances use
/// `prototype` as the cross-realm fallback when a callable's public
/// `.prototype` is not an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeneratorRealmData {
    pub prototype: ObjectId,
    pub function_prototype: ObjectId,
}

/// Realm-owned Promise identities used by allocation and species fallback.
///
/// Both identities remain explicit Context roots.  User code may delete the
/// public global and constructor/prototype properties without changing the
/// intrinsic identities used by Promise abstract operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromiseRealmData {
    pub prototype: ObjectId,
    pub constructor: ObjectId,
}

/// Realm-owned roots which participate in QuickJS's cycle graph.
///
/// The bootstrap roots needed by ordinary script evaluation are explicit;
/// additional intrinsic and module roots can extend the vectors without
/// changing `ContextId` ownership.
#[derive(Debug, PartialEq)]
pub struct ContextData {
    pub object_prototype: ObjectId,
    pub function_prototype: ObjectId,
    /// Realm-local `%Array.prototype%`. QuickJS creates the prototype itself
    /// as a genuine Array exotic object, so this root is never substituted by
    /// an ordinary object even before the global `%Array%` constructor is
    /// published.
    pub array_prototype: ObjectId,
    /// Realm-local `%IteratorPrototype%`.  It is rooted independently rather
    /// than rediscovered through a concrete iterator so empty realms retain
    /// the intrinsic identity required by cross-realm iterator creation.
    pub iterator_prototype: ObjectId,
    /// Realm-local `%ArrayIteratorPrototype%`, whose prototype is the realm's
    /// `%IteratorPrototype%`.
    pub array_iterator_prototype: ObjectId,
    /// Realm-local `%Array%` constructor, attached after the cyclic Context
    /// has been published.
    pub array_constructor: Option<ObjectId>,
    /// Original realm-local `Array.prototype.values` identity. QuickJS caches
    /// this callable in `JSContext.array_proto_values` and installs that
    /// cached value on every later arguments object even if user code mutates
    /// or deletes `Array.prototype.values`.
    pub array_prototype_values: Option<ObjectId>,
    /// Realm-local `%StringIteratorPrototype%`, whose prototype is the realm's
    /// `%IteratorPrototype%`.
    pub string_iterator_prototype: ObjectId,
    /// Realm-local equivalents of QuickJS `class_proto[JS_CLASS_*]` for the
    /// five primitive wrapper classes. An absent entry remains an explicit
    /// implementation gap rather than inheriting from the wrong prototype.
    pub primitive_prototypes: [Option<ObjectId>; PrimitiveKind::COUNT],
    /// Realm-local `%Date.prototype%`. Pinned QuickJS creates this as an
    /// ordinary object without a Date time-value slot, then uses it as the
    /// default prototype for genuine Date instances.
    pub date_prototype: Option<ObjectId>,
    /// Realm-local RegExp constructor, ordinary prototype, RegExp String
    /// Iterator prototype, and canonical one-slot instance shape. They are
    /// attached atomically after the cyclic Context has been published.
    pub regexp: Option<RegExpRealmData>,
    /// Realm-local Map constructor, ordinary prototype, and Map Iterator
    /// prototype, attached atomically after the cyclic Context is published.
    pub map: Option<MapRealmData>,
    /// Realm-local Set ordinary prototype and Set Iterator prototype,
    /// attached atomically after the cyclic Context is published.
    pub set: Option<SetRealmData>,
    /// Realm-local `%GeneratorPrototype%` and
    /// `%GeneratorFunction.prototype%`, attached after their reciprocal
    /// constructor/prototype graph has been initialized.
    pub generator: Option<GeneratorRealmData>,
    /// Realm-local `%Promise.prototype%` and `%Promise%`, attached atomically
    /// after their reciprocal public property graph is initialized.
    pub promise: Option<PromiseRealmData>,
    /// `%Function%`, published after the cyclic realm bootstrap has created
    /// `%Function.prototype%` and the global object.
    pub function_constructor: Option<ObjectId>,
    /// Shared frozen poison callable used by legacy restricted function
    /// accessors and strict arguments objects.
    pub throw_type_error: Option<ObjectId>,
    /// Original realm-local `%eval%` identity. QuickJS caches this callable
    /// separately from the writable/configurable global `eval` property so
    /// direct-eval dispatch can compare identity after user mutation.
    pub eval_function: Option<ObjectId>,
    pub global_object: ObjectId,
    /// Null-prototype storage for global lexical bindings (`let`/`const`).
    pub global_var_object: ObjectId,
    pub error_prototype: Option<ObjectId>,
    pub native_error_prototypes: [Option<ObjectId>; NativeErrorKind::COUNT],
    /// QuickJS keeps Math.random's xorshift64* state on each JSContext.  Zero
    /// is reserved for the not-yet-seeded bootstrap state.
    math_random_state: u64,
    pub global_objects: Vec<ObjectId>,
    pub intrinsics: Vec<RawValue>,
    pub initial_shapes: Vec<ShapeId>,
}

impl ContextData {
    /// Construct the complete mandatory realm root set.
    ///
    /// Iterator prototype identities are required arguments rather than
    /// builder-populated placeholders so the public low-level allocator
    /// cannot publish a realm whose `%IteratorPrototype%` silently aliases
    /// `%Object.prototype%`.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        object_prototype: ObjectId,
        function_prototype: ObjectId,
        array_prototype: ObjectId,
        iterator_prototype: ObjectId,
        array_iterator_prototype: ObjectId,
        string_iterator_prototype: ObjectId,
        global_object: ObjectId,
        global_var_object: ObjectId,
    ) -> Self {
        Self {
            object_prototype,
            function_prototype,
            array_prototype,
            iterator_prototype,
            array_iterator_prototype,
            array_constructor: None,
            array_prototype_values: None,
            string_iterator_prototype,
            primitive_prototypes: [None; PrimitiveKind::COUNT],
            date_prototype: None,
            regexp: None,
            map: None,
            set: None,
            generator: None,
            promise: None,
            function_constructor: None,
            throw_type_error: None,
            eval_function: None,
            global_object,
            global_var_object,
            error_prototype: None,
            native_error_prototypes: [None; NativeErrorKind::COUNT],
            math_random_state: 0,
            global_objects: Vec::new(),
            intrinsics: Vec::new(),
            initial_shapes: Vec::new(),
        }
    }

    /// Attach one implemented primitive wrapper prototype to this realm.
    #[must_use]
    pub const fn with_primitive_prototype(
        mut self,
        kind: PrimitiveKind,
        prototype: ObjectId,
    ) -> Self {
        self.primitive_prototypes[kind.index()] = Some(prototype);
        self
    }

    /// Attach the ordinary Date prototype to this realm before publish.
    #[must_use]
    pub const fn with_date_prototype(mut self, prototype: ObjectId) -> Self {
        self.date_prototype = Some(prototype);
        self
    }

    /// Attach the Error intrinsic prototype graph to this realm.
    #[must_use]
    pub const fn with_error_prototypes(
        mut self,
        error_prototype: ObjectId,
        native_error_prototypes: [ObjectId; NativeErrorKind::COUNT],
    ) -> Self {
        self.error_prototype = Some(error_prototype);
        let mut index = 0;
        while index < NativeErrorKind::COUNT {
            self.native_error_prototypes[index] = Some(native_error_prototypes[index]);
            index += 1;
        }
        self
    }
}

/// Constant-pool entry owned by a [`FunctionBytecodeData`] node.
#[derive(Clone, Debug, PartialEq)]
pub enum BytecodeConstant {
    Value(RawValue),
    /// Compile-once RegExp literal payload. These reference-counted Rust
    /// leaves own no arena or atom edge; executing `Instruction::RegExp`
    /// clones them into a fresh realm-local RegExp object.
    RegExp {
        pattern: JsString,
        program: Rc<CompiledRegExp>,
    },
    Function(FunctionBytecodeId),
}

/// Body storage selected for one named physical parameter after its
/// initializer-visible lexical cell has been initialized. Ordinary parameter
/// environments keep the raw argument slot; the explicit enum leaves room
/// for QuickJS's direct-eval `arguments` override without weakening the ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParameterBodyStorage {
    Argument(u16),
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

/// Compiler-authored frame pseudo bindings have one canonical QuickJS entry
/// order.  Keeping the rank in the heap layer lets every publication shape
/// (plain/default/rest/pattern parameters) authenticate the same ABI.
const fn pseudo_binding_entry_rank(instruction: &Instruction) -> Option<u8> {
    match instruction {
        Instruction::PushHomeObject => Some(1),
        Instruction::PushActiveFunction => Some(2),
        Instruction::PushNewTarget => Some(3),
        Instruction::PushThis => Some(4),
        _ => None,
    }
}

const fn is_derived_initialization_source(instruction: &Instruction) -> bool {
    matches!(
        instruction,
        Instruction::ConstructSuper(_)
            | Instruction::ApplySuper
            | Instruction::InitDerivedConstructor
    )
}

/// Whether pinned QuickJS copies `defined_arg_count` out of its parser record.
///
/// QuickJS gates that copy on `arg_count + var_count > 0`. Most Rust locals
/// correspond directly to QuickJS variables, but owning-function HomeObject,
/// active-function, `new.target`, and ordinary `this` reads use dedicated
/// bytecode here while QuickJS resolves each to a hidden variable. This
/// predicate keeps the observable empty terminal rest-BindingPattern `length`
/// quirk shared by lowering and both publication boundaries.
pub(crate) fn quickjs_copies_defined_argument_count(
    argument_count: usize,
    local_count: usize,
    code: &[Instruction],
) -> bool {
    argument_count != 0
        || local_count != 0
        || code
            .iter()
            .any(|instruction| pseudo_binding_entry_rank(instruction).is_some())
}

/// Authenticate the call-frame ABI encoded by formal-parameter bytecode.
///
/// This stays independent from compiler IR so both unlinked publication and
/// the final heap allocation boundary authenticate the same structural ABI.
/// The unlinked publisher separately authenticates source-level binding names.
/// A successful parameter environment returns the first body instruction so
/// that the unlinked boundary can authenticate segment-specific captures.
fn validate_class_constructor_guard(
    metadata: &FunctionMetadata,
    code: &[Instruction],
) -> Result<Option<usize>, &'static str> {
    let guard_pcs = code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| matches!(instruction, Instruction::CheckCtor).then_some(pc))
        .collect::<Vec<_>>();
    let is_class_constructor = metadata.constructor_kind != ConstructorKind::None
        && metadata.strict
        && !metadata.has_prototype;
    let guard_pc = match (is_class_constructor, guard_pcs.as_slice()) {
        (true, [pc]) => *pc,
        (true, []) => return Err("class constructor has no constructor-call guard"),
        (true, _) => return Err("class constructor guard is not unique"),
        (false, []) => return Ok(None),
        (false, _) => return Err("non-class function contains a constructor-call guard"),
    };

    // The guard may follow only entry ABI work: authenticated pseudo-binding
    // materialization, lexical TDZ reset, arguments/eval objects and function
    // hoists. Parameter-specific validators below pin it to the exact slot
    // between that prologue and the first parameter selection skeleton.
    let mut depth = 0_usize;
    for instruction in &code[..guard_pc] {
        if !matches!(
            instruction,
            Instruction::PushHomeObject
                | Instruction::PushActiveFunction
                | Instruction::PushNewTarget
                | Instruction::PushThis
                | Instruction::FClosure(_)
                | Instruction::Arguments(_)
                | Instruction::VariableEnvironment
                | Instruction::Dup
                | Instruction::PutLocal(_)
                | Instruction::InitializeLocal(_)
                | Instruction::SetLocalUninitialized(_)
        ) {
            return Err("class constructor guard is not in the entry prologue");
        }
        let (popped, pushed) = instruction.stack_effect();
        depth = depth
            .checked_sub(popped)
            .ok_or("class constructor entry prologue has stack underflow")?
            .checked_add(pushed)
            .ok_or("class constructor entry prologue has stack overflow")?;
    }
    if depth != 0 {
        return Err("class constructor guard interrupts its entry prologue");
    }
    Ok(Some(guard_pc))
}

fn consume_class_constructor_guard(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    guard_pc: Option<usize>,
    expected_pc: usize,
) -> Result<usize, &'static str> {
    match guard_pc {
        Some(actual) if actual == expected_pc => {
            let after_guard = expected_pc
                .checked_add(1)
                .ok_or("class constructor guard position overflowed bytecode")?;
            if metadata.constructor_kind == ConstructorKind::Base {
                if !matches!(
                    code.get(after_guard..after_guard + 4),
                    Some([
                        Instruction::PushThis,
                        Instruction::PushActiveFunction,
                        Instruction::CallClassInstanceInitializer,
                        Instruction::Drop,
                    ])
                ) {
                    return Err("base class constructor has no exact field initializer hook");
                }
                after_guard
                    .checked_add(4)
                    .ok_or("class field initializer position overflowed bytecode")
            } else {
                Ok(after_guard)
            }
        }
        Some(_) => Err("class constructor guard is not at parameter entry"),
        None => Ok(expected_pc),
    }
}

/// Authenticate the frame layout and opcode authority used by derived class
/// constructors and by arrows/direct eval which relay their one-shot `this`
/// binding. This validation runs again at final heap allocation so dead code
/// and hand-authored unlinked bytecode cannot smuggle constructor-only state
/// transitions into an ordinary function.
pub(crate) fn validate_derived_constructor_bytecode_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    lexical_locals: &[bool],
    const_locals: &[bool],
    closure_variables: &[ClosureVariable],
) -> Result<(), &'static str> {
    if lexical_locals.len() != usize::from(metadata.local_count)
        || const_locals.len() != usize::from(metadata.local_count)
    {
        return Err("derived constructor local classification has the wrong length");
    }

    let derived = metadata.constructor_kind == ConstructorKind::Derived;
    let (this_local, active_local) = match (
        derived,
        metadata.derived_this_local,
        metadata.active_function_local,
    ) {
        (true, Some(this), Some(active))
            if this < metadata.local_count
                && active < metadata.local_count
                && this != active
                && metadata.strict
                && !metadata.has_prototype
                && metadata.super_call_allowed
                && metadata.super_allowed
                && lexical_locals[usize::from(this)]
                && !const_locals[usize::from(this)]
                && !lexical_locals[usize::from(active)]
                && !const_locals[usize::from(active)] =>
        {
            (Some(this), Some(active))
        }
        (true, _, _) => return Err("derived constructor metadata is malformed"),
        (false, None, None) => (None, None),
        (false, _, _) => return Err("derived constructor locals escaped their constructor"),
    };

    let mut entry_pc = 0_usize;
    let mut pseudo_rank = 0_u8;
    let mut pseudo_targets = Vec::with_capacity(4);
    let mut active_initialized_at_entry = false;
    while let Some([source, Instruction::PutLocal(local)]) = code.get(entry_pc..entry_pc + 2) {
        let Some(rank) = pseudo_binding_entry_rank(source) else {
            break;
        };
        if rank <= pseudo_rank || *local >= metadata.local_count || pseudo_targets.contains(local) {
            return Err("pseudo-binding entry prologue is malformed");
        }
        if derived && matches!(source, Instruction::PushThis) {
            return Err("derived constructor contains an ordinary this prologue");
        }
        if matches!(source, Instruction::PushActiveFunction) {
            if active_local != Some(*local) {
                return Err("active-function opcode targets another entry local");
            }
            active_initialized_at_entry = true;
        }
        pseudo_rank = rank;
        pseudo_targets.push(*local);
        entry_pc += 2;
    }

    let mut active_initializations = 0_usize;
    let mut default_initializers = 0_usize;
    let explicit_targets = code
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::Goto(target)
            | Instruction::IfFalse(target)
            | Instruction::IfTrue(target)
            | Instruction::Catch(target)
            | Instruction::Gosub(target) => usize::try_from(*target).ok(),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let base_initializer_hook = if metadata.constructor_kind == ConstructorKind::Base
        && metadata.strict
        && !metadata.has_prototype
    {
        let Some(guard_pc) = validate_class_constructor_guard(metadata, code)? else {
            return Err("base class constructor has no constructor-call guard");
        };
        let call_pc = guard_pc
            .checked_add(3)
            .ok_or("class field initializer position overflowed bytecode")?;
        if !matches!(
            code.get(guard_pc..guard_pc + 5),
            Some([
                Instruction::CheckCtor,
                Instruction::PushThis,
                Instruction::PushActiveFunction,
                Instruction::CallClassInstanceInitializer,
                Instruction::Drop,
            ])
        ) {
            return Err("base class constructor has no exact field initializer hook");
        }
        if (guard_pc..=guard_pc + 4).any(|pc| explicit_targets.contains(&pc)) {
            return Err("base class initializer protocol has a non-fallthrough entry");
        }
        Some(call_pc)
    } else {
        None
    };
    for (pc, instruction) in code.iter().enumerate() {
        match instruction {
            Instruction::MarkSuperCall
            | Instruction::ConstructSuper(_)
            | Instruction::ApplySuper
                if !metadata.super_call_allowed || !metadata.super_allowed =>
            {
                return Err("typed super-call opcode has no inherited super authority");
            }
            Instruction::PushActiveFunction => {
                let base_field_hook = base_initializer_hook == pc.checked_add(1);
                if base_field_hook {
                    continue;
                }
                let Some(active) = active_local else {
                    return Err("active-function opcode escaped a derived constructor");
                };
                if !matches!(code.get(pc + 1), Some(Instruction::PutLocal(local)) if *local == active)
                {
                    return Err("active-function opcode has no authenticated local store");
                }
                active_initializations += 1;
            }
            Instruction::InitDerivedConstructor => {
                if !derived {
                    return Err("default-derived initializer escaped a derived constructor");
                }
                default_initializers += 1;
            }
            Instruction::InitializeDerivedLocal(local) => {
                if Some(*local) != this_local {
                    return Err("derived local initializer targets another local");
                }
                if pc < 2
                    || !matches!(code.get(pc - 1), Some(Instruction::Dup))
                    || code
                        .get(pc - 2)
                        .is_none_or(|source| !is_derived_initialization_source(source))
                {
                    return Err("derived local initializer has no constructor result");
                }
                if explicit_targets.contains(&(pc - 1)) || explicit_targets.contains(&pc) {
                    return Err("derived local initializer protocol has a non-fallthrough entry");
                }
            }
            Instruction::InitializeDerivedVarRef(index) => {
                if derived || !metadata.super_call_allowed || !metadata.super_allowed {
                    return Err("captured derived initializer has no inherited super authority");
                }
                let Some(descriptor) = closure_variables.get(usize::from(*index)) else {
                    return Err("captured derived initializer is outside closure slots");
                };
                if descriptor.kind != ClosureVariableKind::Normal
                    || !descriptor.is_lexical
                    || descriptor.is_const
                {
                    return Err("captured derived initializer targets a non-mutable lexical cell");
                }
                if pc < 2
                    || !matches!(code.get(pc - 1), Some(Instruction::Dup))
                    || code
                        .get(pc - 2)
                        .is_none_or(|source| !is_derived_initialization_source(source))
                {
                    return Err("captured derived initializer has no constructor result");
                }
                if explicit_targets.contains(&(pc - 1)) || explicit_targets.contains(&pc) {
                    return Err(
                        "captured derived initializer protocol has a non-fallthrough entry",
                    );
                }
            }
            Instruction::CallClassInstanceInitializer => {
                let valid_base = base_initializer_hook == Some(pc);
                let valid_derived_local = derived
                    && pc >= 2
                    && matches!(
                        code.get(pc - 2),
                        Some(Instruction::InitializeDerivedLocal(local)) if Some(*local) == this_local
                    )
                    && matches!(
                        code.get(pc - 1),
                        Some(Instruction::GetLocal(local)) if Some(*local) == active_local
                    );
                let valid_relay = !derived
                    && metadata.super_call_allowed
                    && metadata.super_allowed
                    && pc >= 2
                    && matches!(
                        (code.get(pc - 2), code.get(pc - 1)),
                        (
                            Some(Instruction::InitializeDerivedVarRef(initialized)),
                            Some(Instruction::GetVarRef(read)),
                        ) if initialized != read
                            && closure_variables.get(usize::from(*read)).is_some_and(
                                |descriptor| descriptor.kind == ClosureVariableKind::Normal
                                    && !descriptor.is_lexical
                                    && !descriptor.is_const
                            )
                    );
                if !valid_base && !valid_derived_local && !valid_relay {
                    return Err("class instance initializer call has no authenticated receiver");
                }
                if explicit_targets.contains(&pc)
                    || explicit_targets.contains(&(pc.saturating_sub(1)))
                {
                    return Err("class instance initializer hook has a non-fallthrough entry");
                }
            }
            Instruction::ReturnDerived(local) => {
                if Some(*local) != this_local {
                    return Err("derived return targets another local");
                }
            }
            Instruction::Return if derived => {
                return Err("derived constructor contains an ordinary return");
            }
            _ => {}
        }

        let local = match instruction {
            Instruction::GetLocal(local)
            | Instruction::PutLocal(local)
            | Instruction::SetLocal(local)
            | Instruction::SetLocalUninitialized(local)
            | Instruction::GetLocalCheck(local)
            | Instruction::InitializeLocal(local)
            | Instruction::InitializeDerivedLocal(local)
            | Instruction::PutLocalCheck(local)
            | Instruction::SetLocalCheck(local)
            | Instruction::CloseLocal(local)
            | Instruction::ReturnDerived(local) => Some(*local),
            _ => None,
        };
        if active_local.is_some() && local == active_local {
            let authenticated_store = matches!(instruction, Instruction::PutLocal(local)
                if pc > 0
                    && Some(*local) == active_local
                    && matches!(code.get(pc - 1), Some(Instruction::PushActiveFunction)));
            if !authenticated_store && !matches!(instruction, Instruction::GetLocal(_)) {
                return Err("active-function local has an unauthenticated access");
            }
        }
        if this_local.is_some()
            && local == this_local
            && !matches!(
                instruction,
                Instruction::GetLocalCheck(_)
                    | Instruction::InitializeDerivedLocal(_)
                    | Instruction::ReturnDerived(_)
            )
        {
            return Err("derived this local has an unauthenticated access");
        }
    }

    if derived && active_initializations != 1 {
        return Err("derived constructor active-function initialization is not unique");
    }
    if derived && !active_initialized_at_entry {
        return Err("derived constructor active-function initialization is not at entry");
    }
    if default_initializers > 1 {
        return Err("derived constructor has more than one default initializer");
    }
    if default_initializers == 1 {
        let (Some(this), Some(active)) = (this_local, active_local) else {
            return Err("default-derived constructor lost its authenticated locals");
        };
        let exact_metadata = metadata.argument_count == 0
            && metadata.defined_argument_count == 0
            && metadata.rest_parameter.is_none()
            && metadata.rest_pattern_start.is_none()
            && metadata.parameter_environment_local_count == 0
            && metadata.pattern_argument_count == 0
            && metadata.parameter_pattern_end.is_none()
            && metadata.local_count == 2
            && metadata.function_name_local.is_none()
            && metadata.eval_variable_object_local.is_none()
            && metadata.closure_count == 0
            && !metadata.needs_home_object
            && metadata.eval_kind == EvalKind::None
            && metadata.function_kind == FunctionKind::Normal;
        let exact_code = matches!(
            code,
            [
                Instruction::PushActiveFunction,
                Instruction::PutLocal(active_target),
                Instruction::CheckCtor,
                Instruction::InitDerivedConstructor,
                Instruction::Dup,
                Instruction::InitializeDerivedLocal(this_target),
                Instruction::GetLocal(active_read),
                Instruction::CallClassInstanceInitializer,
                Instruction::ReturnDerived(return_target),
            ] if *active_target == active
                && *this_target == this
                && *active_read == active
                && *return_target == this
        );
        if !exact_metadata || !exact_code {
            return Err("default-derived constructor has no exact synthesized shape");
        }
    }
    Ok(())
}

/// Authenticate the non-visible function roles used by class element
/// initialization.  These functions execute as ordinary synchronous frames,
/// but they are never ordinary authored callables: no constructor/parameter or
/// eval ABI may leak into them, `super` property access is permitted, and
/// `super()` is not.
pub(crate) fn validate_class_initializer_bytecode_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
) -> Result<(), &'static str> {
    if metadata.class_private_brand
        && !matches!(
            metadata.class_initializer_kind,
            Some(ClassInitializerKind::InstanceFields | ClassInitializerKind::StaticElements)
        )
    {
        return Err("private brand metadata escaped a class initializer");
    }
    let Some(_) = metadata.class_initializer_kind else {
        return Ok(());
    };
    if metadata.argument_count != 0
        || metadata.defined_argument_count != 0
        || metadata.rest_parameter.is_some()
        || metadata.rest_pattern_start.is_some()
        || metadata.parameter_environment_local_count != 0
        || metadata.pattern_argument_count != 0
        || metadata.parameter_pattern_end.is_some()
        || metadata.constructor_kind != ConstructorKind::None
        || metadata.has_prototype
        || !metadata.strict
        || metadata.super_call_allowed
        || !metadata.super_allowed
        || metadata.eval_kind != EvalKind::None
        || metadata.function_kind != FunctionKind::Normal
        || !metadata.arguments_forbidden
        || !metadata.needs_home_object
    {
        return Err("class initializer function metadata is malformed");
    }
    if code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::CheckCtor
                | Instruction::InitDerivedConstructor
                | Instruction::MarkSuperCall
                | Instruction::ConstructSuper(_)
                | Instruction::ApplySuper
                | Instruction::InitializeDerivedLocal(_)
                | Instruction::InitializeDerivedVarRef(_)
                | Instruction::ReturnDerived(_)
        )
    }) {
        return Err("constructor-only bytecode escaped into a class initializer");
    }
    if metadata.arguments_forbidden
        && code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Arguments(_)))
    {
        return Err("arguments object escaped into an arguments-forbidden function");
    }
    Ok(())
}

pub(crate) fn validate_parameter_bytecode_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    parameter_initializer_locals: &[bool],
    parameter_environment: Option<&ParameterEnvironmentLayout>,
) -> Result<Option<usize>, &'static str> {
    if parameter_initializer_locals.len() != usize::from(metadata.local_count) {
        return Err("parameter-initializer local classification has the wrong length");
    }
    let class_constructor_guard = validate_class_constructor_guard(metadata, code)?;
    if metadata.rest_parameter.is_some() && metadata.rest_pattern_start.is_some() {
        return Err("identifier rest and rest BindingPattern metadata overlap");
    }
    let maximum_defined_arguments = metadata
        .argument_count
        .checked_add(u16::from(metadata.rest_pattern_start.is_some()))
        .ok_or("defined argument count overflowed function argument slots")?;
    if metadata.defined_argument_count > maximum_defined_arguments {
        return Err("defined argument count exceeds function argument slots");
    }
    if metadata
        .rest_pattern_start
        .is_some_and(|start| start != metadata.argument_count)
    {
        return Err("rest BindingPattern metadata disagrees with argument slots");
    }
    if metadata.pattern_argument_count > metadata.argument_count {
        return Err("pattern argument count exceeds function argument slots");
    }
    let has_pattern_parameters =
        metadata.pattern_argument_count != 0 || metadata.rest_pattern_start.is_some();
    if let Some(end) = metadata.parameter_pattern_end {
        if !has_pattern_parameters {
            return Err("parameter initialization marker has no BindingPattern");
        }
        let end = usize::try_from(end)
            .map_err(|_| "parameter BindingPattern marker is outside bytecode")?;
        if !matches!(code.get(end), Some(Instruction::Nop)) {
            return Err("parameter BindingPattern marker is outside bytecode");
        }
    } else if has_pattern_parameters {
        return Err("parameter BindingPattern has no initialization marker");
    }

    let parameter_locals = metadata.parameter_environment_local_count;
    let mut explicit_body_pc = None;
    if let Some(layout) = parameter_environment {
        explicit_body_pc = validate_explicit_parameter_environment_layout(
            metadata,
            code,
            layout,
            class_constructor_guard,
        )?;
        let has_extended_parameter_entry = metadata.eval_variable_object_local.is_some()
            || layout.synthetic_arguments_local.is_some()
            || layout.arg_eval_variable_object_local.is_some()
            || code
                .iter()
                .any(|instruction| instruction.eval_environment().is_some());
        if has_pattern_parameters || parameter_locals == 0 || has_extended_parameter_entry {
            return Ok(explicit_body_pc);
        }
    }
    if parameter_environment.is_none() && parameter_locals != 0 {
        return Err("parameter-environment cells have no immutable layout");
    }
    if parameter_locals == 0 {
        if metadata.parameter_pattern_end.is_some() {
            return match (metadata.rest_parameter, metadata.rest_pattern_start) {
                (Some(rest), None)
                    if rest.checked_add(1) == Some(metadata.argument_count)
                        && metadata.defined_argument_count == rest =>
                {
                    Ok(None)
                }
                (Some(_), None) => Err("rest parameter metadata disagrees with argument slots"),
                (None, Some(start))
                    if metadata.defined_argument_count
                        == if quickjs_copies_defined_argument_count(
                            usize::from(metadata.argument_count),
                            usize::from(metadata.local_count),
                            code,
                        ) {
                            start.saturating_add(1)
                        } else {
                            0
                        } =>
                {
                    Ok(None)
                }
                (None, Some(_)) => {
                    Err("rest BindingPattern metadata disagrees with function length")
                }
                (None, None) if metadata.defined_argument_count == metadata.argument_count => {
                    Ok(None)
                }
                (None, None) => Err("default parameter metadata has no parameter environment"),
                (Some(_), Some(_)) => {
                    Err("identifier rest and rest BindingPattern metadata overlap")
                }
            };
        }
        return match metadata.rest_parameter {
            Some(rest)
                if rest.checked_add(1) == Some(metadata.argument_count)
                    && metadata.defined_argument_count == rest =>
            {
                let mut rest_pc = 0_usize;
                let mut pseudo_rank = 0_u8;
                let mut entry_targets = Vec::with_capacity(6);
                while let Some([source, Instruction::PutLocal(local)]) =
                    code.get(rest_pc..rest_pc + 2)
                {
                    let Some(rank) = pseudo_binding_entry_rank(source) else {
                        break;
                    };
                    if rank <= pseudo_rank
                        || *local >= metadata.local_count
                        || metadata.eval_variable_object_local == Some(*local)
                        || metadata.function_name_local == Some(*local)
                        || entry_targets.contains(local)
                    {
                        return Err("rest parameter contains a malformed pseudo-binding prologue");
                    }
                    pseudo_rank = rank;
                    entry_targets.push(*local);
                    rest_pc += 2;
                }
                let arguments = code
                    .iter()
                    .enumerate()
                    .filter_map(|(pc, instruction)| match instruction {
                        Instruction::Arguments(kind) => Some((pc, *kind)),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                match arguments.as_slice() {
                    [] => {}
                    [(pc, crate::bytecode::ArgumentsKind::Unmapped)] if *pc == rest_pc => {
                        let Some(Instruction::PutLocal(local)) = code.get(rest_pc + 1) else {
                            return Err("rest parameter arguments object has no entry binding");
                        };
                        if *local >= metadata.local_count
                            || metadata.eval_variable_object_local == Some(*local)
                            || metadata.function_name_local == Some(*local)
                            || entry_targets.contains(local)
                        {
                            return Err("rest parameter arguments object has no entry binding");
                        }
                        entry_targets.push(*local);
                        rest_pc += 2;
                    }
                    _ => return Err("rest parameter contains a malformed arguments prologue"),
                }
                match metadata.eval_variable_object_local {
                    Some(local)
                        if local < metadata.local_count
                            && metadata.function_name_local != Some(local)
                            && !entry_targets.contains(&local)
                            && matches!(
                                code.get(rest_pc..rest_pc + 2),
                                Some([
                                    Instruction::VariableEnvironment,
                                    Instruction::PutLocal(target),
                                ]) if *target == local
                            ) =>
                    {
                        rest_pc += 2;
                    }
                    Some(_) => {
                        return Err("eval variable-object local has no exact entry prologue");
                    }
                    None => {}
                }
                if code
                    .iter()
                    .filter(|instruction| matches!(instruction, Instruction::VariableEnvironment))
                    .count()
                    != usize::from(metadata.eval_variable_object_local.is_some())
                {
                    return Err("variable-environment opcode has no authenticated local");
                }
                rest_pc = consume_class_constructor_guard(
                    metadata,
                    code,
                    class_constructor_guard,
                    rest_pc,
                )?;
                if !matches!(
                    code.get(rest_pc..rest_pc + 2),
                    Some([Instruction::Rest(start), Instruction::PutArg(target)])
                        if *start == rest && *target == rest
                ) || code.iter().enumerate().any(|(pc, instruction)| {
                    pc != rest_pc && matches!(instruction, Instruction::Rest(_))
                }) {
                    return Err("rest parameter has no exact entry initialization");
                }
                Ok(None)
            }
            Some(_) => Err("rest parameter metadata disagrees with argument slots"),
            None if metadata.defined_argument_count != metadata.argument_count => {
                Err("default parameter metadata has no parameter environment")
            }
            None if code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Rest(_))) =>
            {
                Err("rest opcode has no authenticated parameter metadata")
            }
            None => Ok(None),
        };
    }

    if metadata.pattern_argument_count != 0
        || metadata.parameter_pattern_end.is_some()
        || metadata.rest_pattern_start.is_some()
        || parameter_locals != metadata.argument_count
        || parameter_locals > metadata.local_count
        || metadata.defined_argument_count >= metadata.argument_count
        || metadata.rest_parameter.is_some_and(|rest| {
            rest.checked_add(1) != Some(metadata.argument_count)
                || metadata.defined_argument_count >= rest
        })
    {
        return Err("parameter environment metadata disagrees with function slots");
    }
    if metadata.eval_variable_object_local.is_some()
        || code
            .iter()
            .any(|instruction| instruction.eval_environment().is_some())
    {
        return Err("direct eval is not supported in a parameter environment");
    }

    let mut entry_pc = 0_usize;
    // QuickJS materializes owned HomeObject/active-function/new.target/this
    // cells before entering the argument scope when parameter code needs one.
    // The compiler emits the same fixed-order pairs before the arguments
    // object and the parameter TDZ reset; every target must remain outside the
    // leading parameter-local range.
    let mut pseudo_rank = 0_u8;
    let mut pseudo_targets = Vec::with_capacity(4);
    while let Some([source, Instruction::PutLocal(local)]) = code.get(entry_pc..entry_pc + 2) {
        let Some(rank) = pseudo_binding_entry_rank(source) else {
            break;
        };
        if rank <= pseudo_rank
            || *local < parameter_locals
            || *local >= metadata.local_count
            || metadata.function_name_local == Some(*local)
            || pseudo_targets.contains(local)
        {
            return Err("parameter environment contains a malformed pseudo-binding prologue");
        }
        pseudo_rank = rank;
        pseudo_targets.push(*local);
        entry_pc += 2;
    }

    let arguments = code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| match instruction {
            Instruction::Arguments(kind) => Some((pc, *kind)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let arguments_local = match arguments.as_slice() {
        [] => None,
        [(pc, crate::bytecode::ArgumentsKind::Unmapped)] if *pc == entry_pc => {
            let Some(Instruction::PutLocal(local)) = code.get(entry_pc + 1) else {
                return Err("parameter environment arguments object has no entry binding");
            };
            if *local < parameter_locals
                || *local >= metadata.local_count
                || metadata.function_name_local == Some(*local)
                || pseudo_targets.contains(local)
            {
                return Err("parameter environment arguments object has no entry binding");
            }
            entry_pc += 2;
            Some(*local)
        }
        _ => return Err("parameter environment contains a malformed arguments prologue"),
    };

    let parameter_count = usize::from(parameter_locals);
    for (offset, local) in (0..parameter_locals).rev().enumerate() {
        if !matches!(
            code.get(entry_pc + offset),
            Some(Instruction::SetLocalUninitialized(target)) if *target == local
        ) {
            return Err("parameter environment has no exact TDZ entry initialization");
        }
    }
    let parameter_body_pc = consume_class_constructor_guard(
        metadata,
        code,
        class_constructor_guard,
        entry_pc + parameter_count,
    )?;
    let mut initializer_pcs = vec![None; parameter_count];
    for (pc, instruction) in code.iter().enumerate() {
        let Instruction::InitializeLocal(target) = instruction else {
            continue;
        };
        let target = usize::from(*target);
        if target >= parameter_count {
            continue;
        }
        if initializer_pcs[target].replace(pc).is_some() {
            return Err("parameter cell does not have one exact initializer");
        }
    }
    let mut previous_initializer = None;
    for initializer in &initializer_pcs {
        let Some(pc) = *initializer else {
            return Err("parameter cell does not have one exact initializer");
        };
        if pc < parameter_body_pc || previous_initializer.is_some_and(|previous| previous >= pc) {
            return Err("parameter cells are not initialized left to right");
        }
        previous_initializer = Some(pc);
    }
    let initializer_pcs = initializer_pcs
        .into_iter()
        .map(|pc| pc.expect("parameter initializer presence checked above"))
        .collect::<Vec<_>>();
    if code.iter().enumerate().any(|(pc, instruction)| {
        matches!(instruction, Instruction::SetLocalUninitialized(local) if *local < parameter_locals)
            && !(entry_pc..parameter_body_pc).contains(&pc)
    }) {
        return Err("parameter cell has an unauthenticated TDZ reset");
    }

    // Consume the compiler's contiguous argument-initialization ABI. The
    // public `defined_argument_count` identifies the first default, while
    // later slots are distinguished by their exact plain/default skeleton.
    // This authenticates the branch which skips an initializer as well as the
    // raw argument-slot synchronization on the default path.
    let mut parameter_pc = parameter_body_pc;
    let mut authenticated_rest_pc = None;
    for (local, &initializer_pc) in (0..parameter_locals).zip(&initializer_pcs) {
        if metadata.rest_parameter == Some(local) {
            if !matches!(
                code.get(parameter_pc..parameter_pc + 4),
                Some([
                    Instruction::Rest(start),
                    Instruction::Dup,
                    Instruction::PutArg(argument),
                    Instruction::InitializeLocal(target),
                ]) if *start == local && *argument == local && *target == local
            ) || initializer_pc != parameter_pc + 3
            {
                return Err("parameter-environment rest parameter has no exact initialization");
            }
            authenticated_rest_pc = Some(parameter_pc);
            parameter_pc += 4;
            continue;
        }

        let plain = matches!(
            code.get(parameter_pc..parameter_pc + 2),
            Some([Instruction::GetArg(argument), Instruction::InitializeLocal(target)])
                if *argument == local && *target == local
        ) && initializer_pc == parameter_pc + 1;
        if plain {
            if local == metadata.defined_argument_count {
                return Err("first default parameter has no default entry initialization");
            }
            parameter_pc += 2;
            continue;
        }
        if local < metadata.defined_argument_count {
            return Err("leading plain parameter has no exact entry initialization");
        }

        let Some(default_header_end) = parameter_pc.checked_add(6) else {
            return Err("default parameter entry initialization overflowed bytecode");
        };
        if !matches!(
            code.get(parameter_pc..default_header_end),
            Some([
                Instruction::GetArg(argument),
                Instruction::Dup,
                Instruction::Undefined,
                Instruction::StrictEq,
                Instruction::IfFalse(target),
                Instruction::Drop,
            ]) if *argument == local && usize::try_from(*target).ok() == Some(initializer_pc)
        ) {
            return Err("default parameter has no exact selection branch");
        }
        let Some(sync_pc) = initializer_pc.checked_sub(2) else {
            return Err("default parameter has no exact argument synchronization");
        };
        if sync_pc <= default_header_end
            || !matches!(
                code.get(sync_pc..initializer_pc + 1),
                Some([
                    Instruction::Dup,
                    Instruction::PutArg(argument),
                    Instruction::InitializeLocal(target),
                ]) if *argument == local && *target == local
            )
        {
            return Err("default parameter has no exact argument synchronization");
        }
        for instruction in &code[default_header_end..sync_pc] {
            if matches!(
                instruction,
                Instruction::GetArg(_)
                    | Instruction::PutArg(_)
                    | Instruction::SetArg(_)
                    | Instruction::Rest(_)
            ) {
                return Err("default parameter initializer bypasses parameter cells");
            }
            let unauthenticated_local_access = match instruction {
                Instruction::GetLocal(target) => {
                    *target < parameter_locals
                        || (arguments_local != Some(*target)
                            && metadata.function_name_local != Some(*target)
                            && !pseudo_targets.contains(target))
                }
                Instruction::PutLocal(target) | Instruction::SetLocal(target) => {
                    arguments_local != Some(*target)
                }
                Instruction::GetLocalCheck(target) => {
                    *target >= parameter_locals
                        && metadata.derived_this_local != Some(*target)
                        && !parameter_initializer_locals
                            .get(usize::from(*target))
                            .copied()
                            .unwrap_or(false)
                }
                Instruction::PutLocalCheck(target) | Instruction::SetLocalCheck(target) => {
                    *target >= parameter_locals
                        && !parameter_initializer_locals
                            .get(usize::from(*target))
                            .copied()
                            .unwrap_or(false)
                }
                Instruction::InitializeDerivedLocal(target) => {
                    metadata.derived_this_local != Some(*target)
                }
                // Closure provenance and inherited super authority are
                // authenticated by `validate_derived_constructor_bytecode_layout`.
                Instruction::InitializeDerivedVarRef(_) => false,
                Instruction::SetLocalUninitialized(_)
                | Instruction::InitializeLocal(_)
                | Instruction::CloseLocal(_) => match instruction {
                    Instruction::SetLocalUninitialized(target)
                    | Instruction::InitializeLocal(target)
                    | Instruction::CloseLocal(target) => !parameter_initializer_locals
                        .get(usize::from(*target))
                        .copied()
                        .unwrap_or(false),
                    _ => unreachable!("matched parameter-initializer lifecycle opcode"),
                },
                _ => false,
            };
            if unauthenticated_local_access {
                return Err("default parameter initializer has an unauthenticated local access");
            }
            let target = match instruction {
                Instruction::Goto(target)
                | Instruction::IfFalse(target)
                | Instruction::IfTrue(target)
                | Instruction::Catch(target)
                | Instruction::Gosub(target) => Some(*target),
                _ => None,
            };
            if target.is_some_and(|target| {
                usize::try_from(target)
                    .ok()
                    .is_none_or(|target| !(default_header_end..=sync_pc).contains(&target))
            }) {
                return Err("default parameter initializer escaped its entry segment");
            }
        }
        parameter_pc = initializer_pc + 1;
    }

    let rest_pcs = code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| matches!(instruction, Instruction::Rest(_)).then_some(pc))
        .collect::<Vec<_>>();
    match (metadata.rest_parameter, authenticated_rest_pc) {
        (Some(_), Some(expected)) if rest_pcs.as_slice() == [expected] => {}
        (Some(_), _) => {
            return Err("parameter-environment rest parameter is not unique");
        }
        (None, None) if rest_pcs.is_empty() => {}
        (None, None) => {
            return Err("rest opcode has no authenticated parameter metadata");
        }
        (None, Some(_)) => {
            return Err("rest opcode has no authenticated parameter metadata");
        }
    }
    let mut body_pc = parameter_pc;
    if let Some(layout) = parameter_environment {
        if usize::try_from(layout.initialization_end).ok() != Some(body_pc)
            || !matches!(code.get(body_pc), Some(Instruction::Nop))
        {
            return Err("parameter environment marker does not follow exact initialization");
        }
        body_pc += 1;
    } else if explicit_body_pc.is_some() {
        return Err("parameter environment lost its immutable layout");
    }
    let mut closed_parameter_cells = vec![false; parameter_count];
    while let Some(Instruction::CloseLocal(target)) = code.get(body_pc) {
        let target = usize::from(*target);
        if target >= parameter_count {
            break;
        }
        if std::mem::replace(&mut closed_parameter_cells[target], true) {
            return Err("parameter cell is closed more than once");
        }
        body_pc += 1;
    }
    if code[body_pc..].iter().any(|instruction| {
        let target = match instruction {
            Instruction::GetLocal(target)
            | Instruction::PutLocal(target)
            | Instruction::SetLocal(target)
            | Instruction::SetLocalUninitialized(target)
            | Instruction::GetLocalCheck(target)
            | Instruction::InitializeLocal(target)
            | Instruction::PutLocalCheck(target)
            | Instruction::SetLocalCheck(target)
            | Instruction::CloseLocal(target) => Some(*target),
            _ => None,
        };
        target.is_some_and(|target| target < parameter_locals)
    }) {
        return Err("function body accesses a parameter-environment cell");
    }
    if code.iter().skip(body_pc).any(|instruction| {
        let target = match instruction {
            Instruction::Goto(target)
            | Instruction::IfFalse(target)
            | Instruction::IfTrue(target)
            | Instruction::Catch(target)
            | Instruction::Gosub(target) => Some(*target),
            _ => None,
        };
        target.is_some_and(|target| {
            usize::try_from(target)
                .ok()
                .is_some_and(|target| target < body_pc)
        })
    }) {
        return Err("function body jumps back into parameter initialization");
    }
    Ok(Some(body_pc))
}

fn validate_explicit_parameter_environment_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    layout: &ParameterEnvironmentLayout,
    class_constructor_guard: Option<usize>,
) -> Result<Option<usize>, &'static str> {
    let parameter_locals = metadata.parameter_environment_local_count;
    if parameter_locals > metadata.local_count {
        return Err("parameter environment exceeds function local slots");
    }
    let marker = usize::try_from(layout.initialization_end)
        .map_err(|_| "parameter environment marker is outside bytecode")?;
    if !matches!(code.get(marker), Some(Instruction::Nop)) {
        return Err("parameter environment marker is outside bytecode");
    }
    if code[..marker].iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::InitDerivedConstructor | Instruction::ReturnDerived(_)
        )
    }) {
        return Err("constructor completion protocol escaped into parameter initialization");
    }
    let has_pattern = metadata.pattern_argument_count != 0 || metadata.rest_pattern_start.is_some();
    match (has_pattern, metadata.parameter_pattern_end) {
        (true, Some(pattern_end)) if pattern_end == layout.initialization_end => {}
        (true, _) => return Err("parameter BindingPattern marker disagrees with its environment"),
        (false, None) => {}
        (false, Some(_)) => return Err("parameter marker has no BindingPattern"),
    }

    let expected_cells = layout
        .argument_cells
        .len()
        .checked_add(layout.pattern_copies.len())
        .ok_or("parameter environment cell count overflowed")?;
    if expected_cells != usize::from(parameter_locals) {
        return Err("parameter environment layout does not cover every cell");
    }
    let mut cell_roles = vec![false; usize::from(parameter_locals)];
    let mut arguments = vec![false; usize::from(metadata.argument_count)];
    let mut defaulted_arguments = vec![false; usize::from(metadata.argument_count)];
    let mut rest_pattern_defaulted = false;
    let mut previous_default = None;
    for source in layout.default_sources.iter().copied() {
        let formal = match source {
            ParameterDefaultSource::Argument(argument) => {
                let argument_index = usize::from(argument);
                if argument_index >= defaulted_arguments.len()
                    || std::mem::replace(&mut defaulted_arguments[argument_index], true)
                    || metadata.rest_parameter == Some(argument)
                {
                    return Err("parameter default source overlaps or is out of bounds");
                }
                argument
            }
            ParameterDefaultSource::RestPattern(start)
                if metadata.rest_pattern_start == Some(start) && !rest_pattern_defaulted =>
            {
                rest_pattern_defaulted = true;
                start
            }
            ParameterDefaultSource::RestPattern(_) => {
                return Err("parameter default source disagrees with rest BindingPattern");
            }
        };
        if previous_default.is_some_and(|previous| previous >= formal) {
            return Err("parameter default sources are not in formal order");
        }
        previous_default = Some(formal);
    }
    let expected_defined_arguments = if let Some(first_default) =
        layout.default_sources.first().map(|source| match source {
            ParameterDefaultSource::Argument(argument)
            | ParameterDefaultSource::RestPattern(argument) => *argument,
        }) {
        first_default
    } else {
        match (metadata.rest_parameter, metadata.rest_pattern_start) {
            (Some(rest), None) => rest,
            (None, Some(start))
                if quickjs_copies_defined_argument_count(
                    usize::from(metadata.argument_count),
                    usize::from(metadata.local_count),
                    code,
                ) =>
            {
                start
                    .checked_add(1)
                    .ok_or("defined argument count overflowed function argument slots")?
            }
            (None, Some(_)) => 0,
            (None, None) => metadata.argument_count,
            (Some(_), Some(_)) => {
                return Err("identifier rest and rest BindingPattern metadata overlap");
            }
        }
    };
    if metadata.defined_argument_count != expected_defined_arguments {
        return Err("parameter default sources disagree with function length");
    }
    let mut body_targets = vec![false; usize::from(metadata.local_count)];
    for cell in layout.argument_cells.iter() {
        let local = usize::from(cell.parameter_local);
        let argument = usize::from(cell.argument);
        if local >= cell_roles.len()
            || std::mem::replace(&mut cell_roles[local], true)
            || argument >= arguments.len()
            || std::mem::replace(&mut arguments[argument], true)
        {
            return Err("parameter argument cell mapping overlaps or is out of bounds");
        }
        match cell.body {
            ParameterBodyStorage::Argument(body) if body == cell.argument => {}
            ParameterBodyStorage::Argument(_) => {
                return Err("parameter body argument mapping changed physical slots");
            }
            ParameterBodyStorage::Local(_) => {
                return Err("parameter body local storage requires direct-eval support");
            }
        }
    }
    for copy in layout.pattern_copies.iter() {
        let source = usize::from(copy.parameter_local);
        if source >= cell_roles.len() || std::mem::replace(&mut cell_roles[source], true) {
            return Err("parameter pattern copy source overlaps or is out of bounds");
        }
        let target = usize::from(copy.body_local);
        if copy.body_local < parameter_locals
            || target >= body_targets.len()
            || std::mem::replace(&mut body_targets[target], true)
        {
            return Err("parameter pattern copy target overlaps or is out of bounds");
        }
    }
    if let Some(local) = layout.synthetic_arguments_local {
        let local = usize::from(local);
        if local < cell_roles.len()
            || local >= usize::from(metadata.local_count)
            || metadata.eval_variable_object_local == u16::try_from(local).ok()
            || metadata.function_name_local == u16::try_from(local).ok()
            || body_targets.get(local).copied().unwrap_or(false)
        {
            return Err("synthetic parameter arguments cell overlaps or is out of bounds");
        }
    }
    if let Some(local) = layout.arg_eval_variable_object_local {
        let local = usize::from(local);
        if local < cell_roles.len()
            || local >= usize::from(metadata.local_count)
            || layout.synthetic_arguments_local == u16::try_from(local).ok()
            || metadata.eval_variable_object_local == u16::try_from(local).ok()
            || metadata.function_name_local == u16::try_from(local).ok()
            || body_targets.get(local).copied().unwrap_or(false)
        {
            return Err("parameter eval variable-object local overlaps or is out of bounds");
        }
        if metadata.strict || metadata.eval_variable_object_local.is_none() {
            return Err("parameter eval variable object escaped a sloppy eval-enabled function");
        }
    }
    if cell_roles.iter().any(|covered| !covered) {
        return Err("parameter environment layout left an untyped cell");
    }

    let mut entry_pc = 0_usize;
    let mut pseudo_rank = 0_u8;
    let mut pseudo_targets = Vec::with_capacity(4);
    while let Some([source, Instruction::PutLocal(local)]) = code.get(entry_pc..entry_pc + 2) {
        let Some(rank) = pseudo_binding_entry_rank(source) else {
            break;
        };
        if rank <= pseudo_rank
            || *local < parameter_locals
            || *local >= metadata.local_count
            || pseudo_targets.contains(local)
            || metadata.eval_variable_object_local == Some(*local)
            || layout.synthetic_arguments_local == Some(*local)
            || layout.arg_eval_variable_object_local == Some(*local)
            || metadata.function_name_local == Some(*local)
            || body_targets
                .get(usize::from(*local))
                .copied()
                .unwrap_or(false)
        {
            return Err("parameter environment contains a malformed pseudo-binding prologue");
        }
        pseudo_rank = rank;
        pseudo_targets.push(*local);
        entry_pc += 2;
    }

    let arguments_prologues = code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| match instruction {
            Instruction::Arguments(kind) => Some((pc, *kind)),
            _ => None,
        })
        .collect::<Vec<_>>();
    match layout.synthetic_arguments_local {
        None => match arguments_prologues.as_slice() {
            [] => {}
            [(pc, crate::bytecode::ArgumentsKind::Unmapped)] => {
                let Some(Instruction::PutLocal(local)) = code.get(entry_pc + 1) else {
                    return Err("parameter environment arguments object has no entry binding");
                };
                if *pc != entry_pc {
                    return Err("parameter environment arguments object is not at function entry");
                }
                if *local < parameter_locals
                    || *local >= metadata.local_count
                    || metadata.eval_variable_object_local == Some(*local)
                    || layout.arg_eval_variable_object_local == Some(*local)
                    || metadata.function_name_local == Some(*local)
                    || pseudo_targets.contains(local)
                {
                    return Err("parameter environment arguments object has no entry binding");
                }
                entry_pc += 2;
            }
            _ => return Err("parameter environment contains a malformed arguments prologue"),
        },
        Some(synthetic) => {
            if !matches!(
                arguments_prologues.as_slice(),
                [(pc, crate::bytecode::ArgumentsKind::Unmapped)] if *pc == entry_pc
            ) || !matches!(
                code.get(entry_pc..entry_pc + 4),
                Some([
                    Instruction::Arguments(crate::bytecode::ArgumentsKind::Unmapped),
                    Instruction::Dup,
                    Instruction::InitializeLocal(target),
                    Instruction::PutLocal(body),
                ]) if *target == synthetic
                    && *body >= parameter_locals
                    && *body < metadata.local_count
                    && *body != synthetic
                    && metadata.eval_variable_object_local != Some(*body)
                    && layout.arg_eval_variable_object_local != Some(*body)
                    && metadata.function_name_local != Some(*body)
                    && !pseudo_targets.contains(body)
            ) {
                return Err("parameter environment contains a malformed arguments prologue");
            }
            entry_pc += 4;
        }
    }

    let mut variable_environment_targets = Vec::with_capacity(2);
    for expected in [
        metadata.eval_variable_object_local,
        layout.arg_eval_variable_object_local,
    ]
    .into_iter()
    .flatten()
    {
        if !matches!(
            code.get(entry_pc..entry_pc + 2),
            Some([Instruction::VariableEnvironment, Instruction::PutLocal(target)])
                if *target == expected
        ) {
            return Err("eval variable-object local has no exact entry prologue");
        }
        variable_environment_targets.push(expected);
        entry_pc += 2;
    }
    if code
        .iter()
        .filter(|instruction| matches!(instruction, Instruction::VariableEnvironment))
        .count()
        != variable_environment_targets.len()
    {
        return Err("variable-environment opcode has no authenticated local");
    }

    for (offset, local) in (0..parameter_locals).rev().enumerate() {
        if !matches!(
            code.get(entry_pc + offset),
            Some(Instruction::SetLocalUninitialized(target)) if *target == local
        ) {
            return Err("parameter environment has no exact TDZ entry initialization");
        }
    }
    let initialization_pc = consume_class_constructor_guard(
        metadata,
        code,
        class_constructor_guard,
        entry_pc + usize::from(parameter_locals),
    )?;
    if initialization_pc > marker {
        return Err("parameter environment marker precedes its TDZ prologue");
    }

    let mut initializer_pcs = vec![None; usize::from(parameter_locals)];
    for (pc, instruction) in code.iter().enumerate() {
        let Instruction::InitializeLocal(local) = instruction else {
            continue;
        };
        let local = usize::from(*local);
        if local >= initializer_pcs.len() {
            continue;
        }
        if pc < initialization_pc || pc >= marker || initializer_pcs[local].replace(pc).is_some() {
            return Err("parameter cell does not have one exact initializer");
        }
    }
    let mut previous = None;
    for &initializer in &initializer_pcs {
        let Some(initializer) = initializer else {
            return Err("parameter cell does not have one exact initializer");
        };
        if previous.is_some_and(|previous| previous >= initializer) {
            return Err("parameter cells are not initialized in BoundName order");
        }
        previous = Some(initializer);
    }
    if code.iter().enumerate().any(|(pc, instruction)| {
        matches!(instruction, Instruction::SetLocalUninitialized(local) if *local < parameter_locals)
            && !(entry_pc..initialization_pc).contains(&pc)
    }) {
        return Err("parameter cell has an unauthenticated TDZ reset");
    }

    let copy_start = marker
        .checked_sub(layout.pattern_copies.len().saturating_mul(2))
        .ok_or("parameter copy phase begins outside bytecode")?;
    for (offset, copy) in layout.pattern_copies.iter().rev().enumerate() {
        let pc = copy_start + offset * 2;
        if !matches!(
            code.get(pc..pc + 2),
            Some([Instruction::GetLocalCheck(source), Instruction::PutLocal(target)])
                if *source == copy.parameter_local && *target == copy.body_local
        ) {
            return Err("parameter pattern copy phase disagrees with immutable layout");
        }
    }
    if copy_start < initialization_pc {
        return Err("parameter pattern copy phase overlaps the TDZ prologue");
    }

    for cell in layout.argument_cells.iter() {
        let argument = cell.argument;
        let argument_index = usize::from(argument);
        let parameter_local = usize::from(cell.parameter_local);
        let initializer_pc = initializer_pcs[parameter_local]
            .ok_or("parameter argument cell has no exact initialization")?;
        let raw_reads = code[..marker]
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| {
                matches!(instruction, Instruction::GetArg(source) if *source == argument)
                    .then_some(pc)
            })
            .collect::<Vec<_>>();
        let raw_writes = code[..marker]
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| {
                matches!(instruction, Instruction::PutArg(target) | Instruction::SetArg(target) if *target == argument)
                    .then_some(pc)
            })
            .collect::<Vec<_>>();

        if metadata.rest_parameter == Some(argument) {
            if defaulted_arguments[argument_index]
                || !raw_reads.is_empty()
                || raw_writes.as_slice() != [initializer_pc.saturating_sub(1)]
                || initializer_pc < 3
                || !matches!(
                    code.get(initializer_pc - 3..=initializer_pc),
                    Some([
                        Instruction::Rest(start),
                        Instruction::Dup,
                        Instruction::PutArg(target),
                        Instruction::InitializeLocal(local),
                    ]) if *start == argument && *target == argument && usize::from(*local) == parameter_local
                )
            {
                return Err("parameter rest cell has no exact initialization");
            }
            continue;
        }

        if defaulted_arguments[argument_index] {
            let Some(&source_pc) = raw_reads.first().filter(|_| raw_reads.len() == 1) else {
                return Err("parameter default cell has no exact argument selection");
            };
            if raw_writes.as_slice() != [initializer_pc.saturating_sub(1)]
                || initializer_pc < 2
                || !matches!(
                    code.get(source_pc..source_pc + 5),
                    Some([
                        Instruction::GetArg(source),
                        Instruction::Dup,
                        Instruction::Undefined,
                        Instruction::StrictEq,
                        Instruction::IfFalse(target),
                    ]) if *source == argument && usize::try_from(*target).ok() == Some(initializer_pc)
                )
                || !matches!(
                    code.get(initializer_pc - 2..=initializer_pc),
                    Some([
                        Instruction::Dup,
                        Instruction::PutArg(target),
                        Instruction::InitializeLocal(local),
                    ]) if *target == argument && usize::from(*local) == parameter_local
                )
            {
                return Err("parameter default cell has no exact argument selection");
            }
        } else if raw_reads.as_slice() != [initializer_pc.saturating_sub(1)]
            || !raw_writes.is_empty()
            || initializer_pc == 0
            || !matches!(
                code.get(initializer_pc - 1..=initializer_pc),
                Some([
                    Instruction::GetArg(source),
                    Instruction::InitializeLocal(local),
                ]) if *source == argument && usize::from(*local) == parameter_local
            )
        {
            return Err("plain parameter argument cell has no exact initialization");
        }
    }

    if has_pattern {
        if arguments.iter().filter(|mapped| !**mapped).count()
            != usize::from(metadata.pattern_argument_count)
        {
            return Err("parameter argument-cell map disagrees with BindingPattern slots");
        }
        let validate_pattern_default = |source_pc: usize,
                                        expected: bool|
         -> Result<(), &'static str> {
            let initializer_pc = match code.get(source_pc + 1..source_pc + 5) {
                Some(
                    [
                        Instruction::Dup,
                        Instruction::Undefined,
                        Instruction::StrictEq,
                        Instruction::IfTrue(target),
                    ],
                ) => usize::try_from(*target).ok(),
                _ => None,
            };
            if !expected {
                return if initializer_pc.is_some() {
                    Err("BindingPattern has an unauthenticated top-level initializer")
                } else {
                    Ok(())
                };
            }
            let Some(initializer_pc) = initializer_pc else {
                return Err("BindingPattern default has no exact argument selection");
            };
            let assignment_pc = source_pc + 5;
            let Some(Instruction::Goto(done_pc)) =
                initializer_pc.checked_sub(1).and_then(|pc| code.get(pc))
            else {
                return Err("BindingPattern default has no exact argument selection");
            };
            let Some(done_pc) = usize::try_from(*done_pc).ok() else {
                return Err("BindingPattern default has no exact argument selection");
            };
            if initializer_pc <= assignment_pc
                || initializer_pc >= copy_start
                || !matches!(code.get(initializer_pc), Some(Instruction::Drop))
                || done_pc <= initializer_pc
                || done_pc > copy_start
                || !matches!(
                    done_pc.checked_sub(1).and_then(|pc| code.get(pc)),
                    Some(Instruction::Goto(target)) if usize::try_from(*target).ok() == Some(assignment_pc)
                )
            {
                return Err("BindingPattern default has no exact argument selection");
            }
            Ok(())
        };
        for (argument, mapped) in arguments.iter().copied().enumerate() {
            if mapped {
                continue;
            }
            let argument = u16::try_from(argument)
                .map_err(|_| "parameter BindingPattern argument is out of bounds")?;
            let mut sources =
                code[..copy_start]
                    .iter()
                    .enumerate()
                    .filter_map(|(pc, instruction)| {
                        matches!(instruction, Instruction::GetArg(source) if *source == argument)
                            .then_some(pc)
                    });
            let Some(source_pc) = sources.next() else {
                return Err("parameter BindingPattern has no exact raw argument source");
            };
            if sources.next().is_some()
            || code[..marker].iter().any(|instruction| {
                matches!(instruction, Instruction::PutArg(target) | Instruction::SetArg(target) if *target == argument)
            })
        {
            return Err("parameter BindingPattern bypasses its anonymous argument slot");
        }
            validate_pattern_default(source_pc, defaulted_arguments[usize::from(argument)])?;
        }
        if let Some(start) = metadata.rest_pattern_start {
            let mut sources =
                code[..copy_start]
                    .iter()
                    .enumerate()
                    .filter_map(|(pc, instruction)| {
                        matches!(instruction, Instruction::Rest(source) if *source == start)
                            .then_some(pc)
                    });
            let Some(source_pc) = sources.next() else {
                return Err("rest BindingPattern has no exact raw argument source");
            };
            if sources.next().is_some() {
                return Err("rest BindingPattern bypasses its raw argument source");
            }
            validate_pattern_default(source_pc, rest_pattern_defaulted)?;
        }
    }

    let rest_pcs = code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| match instruction {
            Instruction::Rest(start) => Some((pc, *start)),
            _ => None,
        })
        .collect::<Vec<_>>();
    match (metadata.rest_parameter, metadata.rest_pattern_start) {
        (Some(rest), None) if matches!(rest_pcs.as_slice(), [(pc, start)] if *pc < marker && *start == rest) =>
            {}
        (None, Some(rest)) if matches!(rest_pcs.as_slice(), [(pc, start)] if *pc < marker && *start == rest) =>
            {}
        (None, None) if rest_pcs.is_empty() => {}
        (Some(_), Some(_)) => {
            return Err("identifier rest and rest BindingPattern metadata overlap");
        }
        _ => return Err("parameter environment rest opcode disagrees with metadata"),
    }

    for (pc, instruction) in code.iter().enumerate() {
        let target = match instruction {
            Instruction::Goto(target)
            | Instruction::IfFalse(target)
            | Instruction::IfTrue(target)
            | Instruction::Catch(target)
            | Instruction::Gosub(target) => usize::try_from(*target).ok(),
            _ => None,
        };
        if target.is_some_and(|target| {
            (pc < copy_start && target > copy_start) || (pc > marker && target <= marker)
        }) {
            return Err("bytecode crosses the parameter environment boundary");
        }
    }

    let mut body_pc = marker + 1;
    let mut parameter_owned = vec![false; usize::from(metadata.local_count)];
    parameter_owned[..usize::from(parameter_locals)].fill(true);
    if let Some(local) = layout.synthetic_arguments_local {
        parameter_owned[usize::from(local)] = true;
    }
    let mut closed = vec![false; usize::from(metadata.local_count)];
    while let Some(Instruction::CloseLocal(local)) = code.get(body_pc) {
        let local = usize::from(*local);
        if !parameter_owned.get(local).copied().unwrap_or(false) {
            break;
        }
        if std::mem::replace(&mut closed[local], true) {
            return Err("parameter cell is closed more than once");
        }
        body_pc += 1;
    }
    if code[body_pc..].iter().any(|instruction| {
        let local = match instruction {
            Instruction::GetLocal(local)
            | Instruction::PutLocal(local)
            | Instruction::SetLocal(local)
            | Instruction::SetLocalUninitialized(local)
            | Instruction::GetLocalCheck(local)
            | Instruction::InitializeLocal(local)
            | Instruction::PutLocalCheck(local)
            | Instruction::SetLocalCheck(local)
            | Instruction::CloseLocal(local) => Some(*local),
            _ => None,
        };
        local.is_some_and(|local| {
            parameter_owned
                .get(usize::from(local))
                .copied()
                .unwrap_or(false)
        })
    }) {
        return Err("function body accesses a parameter-environment cell");
    }
    Ok(Some(body_pc))
}

/// Authenticate the source-only argument slots and entry segment used by a
/// formal-parameter BindingPattern.
///
/// QuickJS gives an ordinary BindingPattern one anonymous physical argument
/// slot, while a terminal rest BindingPattern owns no slot at all. The public
/// argument definitions are therefore required to agree with the bytecode
/// segment instead of treating a missing argument name as harmless debug
/// metadata. `parameter_pattern_end` is the compiler-authored boundary: entry
/// destructuring may branch within it, but the function body cannot re-enter
/// it and cannot read an anonymous raw argument after destructuring.
pub(crate) fn validate_pattern_parameter_bytecode_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    unnamed_arguments: &[bool],
    lexical_locals: &[bool],
    parameter_initializer_locals: &[bool],
    parameter_environment: Option<&ParameterEnvironmentLayout>,
) -> Result<(), &'static str> {
    if unnamed_arguments.len() != usize::from(metadata.argument_count) {
        return Err("argument definition count does not match bytecode metadata");
    }
    if lexical_locals.len() != usize::from(metadata.local_count) {
        return Err("local definition count does not match bytecode metadata");
    }
    if parameter_initializer_locals.len() != usize::from(metadata.local_count) {
        return Err("parameter-initializer local classification has the wrong length");
    }
    if parameter_initializer_locals
        .iter()
        .enumerate()
        .any(|(index, initializer)| {
            *initializer
                && (index < usize::from(metadata.parameter_environment_local_count)
                    || !lexical_locals[index])
        })
    {
        return Err("parameter-initializer classification names a non-nested lexical local");
    }

    let has_pattern = metadata.pattern_argument_count != 0 || metadata.rest_pattern_start.is_some();
    if !has_pattern {
        return if metadata.parameter_pattern_end.is_some() {
            Err("parameter initialization marker has no BindingPattern")
        } else {
            Ok(())
        };
    }
    if unnamed_arguments.iter().filter(|unnamed| **unnamed).count()
        != usize::from(metadata.pattern_argument_count)
    {
        return Err("pattern argument definitions disagree with bytecode metadata");
    }
    let Some(marker) = metadata.parameter_pattern_end else {
        return Err("parameter BindingPattern has no initialization marker");
    };
    let marker = usize::try_from(marker)
        .map_err(|_| "parameter BindingPattern marker is outside bytecode")?;
    if !matches!(code.get(marker), Some(Instruction::Nop)) {
        return Err("parameter BindingPattern marker is outside bytecode");
    }
    if code[..marker].iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::InitDerivedConstructor | Instruction::ReturnDerived(_)
        )
    }) {
        return Err("constructor completion protocol escaped into parameter initialization");
    }

    if let Some(rest) = metadata.rest_parameter {
        if unnamed_arguments
            .get(usize::from(rest))
            .copied()
            .unwrap_or(true)
        {
            return Err("identifier rest parameter has no named argument slot");
        }
    }

    let synthetic_arguments_local =
        parameter_environment.and_then(|layout| layout.synthetic_arguments_local);
    let mut synthetic_arguments_initialization_pc = None;
    let mut arguments_pcs =
        code.iter()
            .enumerate()
            .filter_map(|(pc, instruction)| match instruction {
                Instruction::Arguments(kind) => Some((pc, *kind)),
                _ => None,
            });
    let mut expected_pc = 0_usize;
    let mut pseudo_rank = 0_u8;
    let mut pseudo_targets = Vec::with_capacity(4);
    while let Some([source, Instruction::PutLocal(local)]) = code.get(expected_pc..expected_pc + 2)
    {
        let Some(rank) = pseudo_binding_entry_rank(source) else {
            break;
        };
        if rank <= pseudo_rank || *local >= metadata.local_count || pseudo_targets.contains(local) {
            return Err("parameter BindingPattern contains a malformed pseudo-binding prologue");
        }
        pseudo_rank = rank;
        pseudo_targets.push(*local);
        expected_pc += 2;
    }
    if let Some((pc, kind)) = arguments_pcs.next() {
        let arguments_shape_matches = match synthetic_arguments_local {
            Some(synthetic) => matches!(
                code.get(pc..pc + 4),
                Some([
                    Instruction::Arguments(crate::bytecode::ArgumentsKind::Unmapped),
                    Instruction::Dup,
                    Instruction::InitializeLocal(target),
                    Instruction::PutLocal(body),
                ]) if *target == synthetic && *body < metadata.local_count
            ),
            None => matches!(
                code.get(pc + 1),
                Some(Instruction::PutLocal(local)) if *local < metadata.local_count
            ),
        };
        if arguments_pcs.next().is_some()
            || pc != expected_pc
            || pc >= marker
            || kind != crate::bytecode::ArgumentsKind::Unmapped
            || !arguments_shape_matches
        {
            return Err("parameter BindingPattern contains a malformed arguments prologue");
        }
        if synthetic_arguments_local.is_some() {
            synthetic_arguments_initialization_pc = Some(pc + 2);
        }
    }

    let expected_unnamed_reads = unnamed_arguments
        .iter()
        .enumerate()
        .filter_map(|(argument, unnamed)| unnamed.then_some(argument))
        .collect::<Vec<_>>();
    let mut unnamed_reads = Vec::with_capacity(expected_unnamed_reads.len());
    let mut rest_operations = Vec::new();
    for (pc, instruction) in code.iter().enumerate() {
        let local = match instruction {
            Instruction::GetLocal(local)
            | Instruction::PutLocal(local)
            | Instruction::SetLocal(local)
            | Instruction::SetLocalUninitialized(local)
            | Instruction::GetLocalCheck(local)
            | Instruction::InitializeLocal(local)
            | Instruction::InitializeDerivedLocal(local)
            | Instruction::PutLocalCheck(local)
            | Instruction::SetLocalCheck(local)
            | Instruction::CloseLocal(local) => Some(*local),
            _ => None,
        };
        let is_synthetic_arguments_access = match instruction {
            Instruction::InitializeLocal(local) => {
                synthetic_arguments_local == Some(*local)
                    && synthetic_arguments_initialization_pc == Some(pc)
            }
            Instruction::GetLocalCheck(local)
            | Instruction::PutLocalCheck(local)
            | Instruction::SetLocalCheck(local) => synthetic_arguments_local == Some(*local),
            _ => false,
        };
        if pc < marker
            && !is_synthetic_arguments_access
            && local.is_some_and(|local| {
                local >= metadata.parameter_environment_local_count
                    && metadata.derived_this_local != Some(local)
                    && lexical_locals
                        .get(usize::from(local))
                        .copied()
                        .unwrap_or(false)
                    && !parameter_initializer_locals
                        .get(usize::from(local))
                        .copied()
                        .unwrap_or(false)
            })
        {
            return Err("parameter BindingPattern bytecode accessed a body lexical local");
        }

        match instruction {
            Instruction::GetArg(argument)
                if unnamed_arguments
                    .get(usize::from(*argument))
                    .copied()
                    .unwrap_or(false) =>
            {
                if pc >= marker {
                    return Err("function body reads an anonymous pattern argument slot");
                }
                unnamed_reads.push(usize::from(*argument));
            }
            Instruction::PutArg(argument) | Instruction::SetArg(argument)
                if unnamed_arguments
                    .get(usize::from(*argument))
                    .copied()
                    .unwrap_or(false) =>
            {
                return Err("bytecode writes an anonymous pattern argument slot");
            }
            Instruction::Rest(start) => rest_operations.push((pc, *start)),
            _ => {}
        }

        let target = match instruction {
            Instruction::Goto(target)
            | Instruction::IfFalse(target)
            | Instruction::IfTrue(target)
            | Instruction::Catch(target)
            | Instruction::Gosub(target) => Some(*target),
            _ => None,
        };
        if let Some(target) = target {
            let target = usize::try_from(target)
                .map_err(|_| "parameter BindingPattern jump target is outside bytecode")?;
            if pc < marker && target > marker {
                return Err("parameter BindingPattern escaped its initialization segment");
            }
            if pc > marker && target <= marker {
                return Err("function body jumps back into pattern initialization");
            }
        }
    }
    if unnamed_reads != expected_unnamed_reads {
        return Err("anonymous pattern arguments do not have exact entry reads");
    }

    match (metadata.rest_parameter, metadata.rest_pattern_start) {
        (Some(rest), None) => {
            let valid = matches!(rest_operations.as_slice(), [(pc, start)] if {
                *pc < marker
                    && *start == rest
                    && match parameter_environment {
                        Some(layout) => layout
                            .argument_cells
                            .iter()
                            .find(|cell| cell.argument == rest)
                            .is_some_and(|cell| {
                                matches!(
                                    code.get(*pc + 1..*pc + 4),
                                    Some([
                                        Instruction::Dup,
                                        Instruction::PutArg(target),
                                        Instruction::InitializeLocal(local),
                                    ]) if *target == rest && *local == cell.parameter_local
                                )
                            }),
                        None => matches!(
                            code.get(*pc + 1),
                            Some(Instruction::PutArg(target)) if *target == rest
                        ),
                    }
            });
            if valid {
                Ok(())
            } else {
                Err("identifier rest parameter has no exact pattern-segment entry")
            }
        }
        (None, Some(rest))
            if matches!(
                rest_operations.as_slice(),
                [(pc, start)] if *pc < marker && *start == rest
            ) =>
        {
            Ok(())
        }

        (None, Some(_)) => Err("rest BindingPattern has no exact entry initialization"),
        (None, None) if rest_operations.is_empty() => Ok(()),
        (None, None) => Err("rest opcode has no authenticated parameter metadata"),
        (Some(_), Some(_)) => Err("identifier rest and rest BindingPattern metadata overlap"),
    }
}

/// Authenticate lexical locals whose complete scope lifetime belongs to the
/// formal-parameter initializer segment.  This is deliberately independent
/// from the leading Parameter Environment cells: a nested class-name scope,
/// for example, may be captured by a computed method key before the body
/// boundary, but authored body bytecode must never access that local.
pub(crate) fn validate_parameter_initializer_scope_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    parameter_body_pc: Option<usize>,
    lexical_locals: &[bool],
    parameter_initializer_locals: &[bool],
) -> Result<(), &'static str> {
    let local_count = usize::from(metadata.local_count);
    if lexical_locals.len() != local_count || parameter_initializer_locals.len() != local_count {
        return Err("parameter-initializer local classification has the wrong length");
    }
    let any_initializer_local = parameter_initializer_locals.iter().any(|value| *value);
    let Some(body_pc) = parameter_body_pc else {
        return if any_initializer_local {
            Err("parameter-initializer local has no parameter/body boundary")
        } else {
            Ok(())
        };
    };
    if body_pc > code.len() {
        return Err("parameter/body boundary is outside bytecode");
    }

    for (index, is_initializer) in parameter_initializer_locals.iter().copied().enumerate() {
        if !is_initializer {
            continue;
        }
        if index < usize::from(metadata.parameter_environment_local_count) || !lexical_locals[index]
        {
            return Err("parameter-initializer classification names a non-nested lexical local");
        }
        let index = u16::try_from(index)
            .map_err(|_| "parameter-initializer local index is outside bytecode range")?;
        let references = |instruction: &Instruction| {
            matches!(
                instruction,
                Instruction::GetLocal(local)
                    | Instruction::PutLocal(local)
                    | Instruction::SetLocal(local)
                    | Instruction::SetLocalUninitialized(local)
                    | Instruction::GetLocalCheck(local)
                    | Instruction::InitializeLocal(local)
                    | Instruction::PutLocalCheck(local)
                    | Instruction::SetLocalCheck(local)
                    | Instruction::CloseLocal(local)
                    if *local == index
            )
        };
        if !code[..body_pc].iter().any(references) {
            return Err("parameter-initializer local has no initializer-segment lifetime");
        }
        let reset_pc = code[..body_pc]
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| {
                matches!(instruction, Instruction::SetLocalUninitialized(local) if *local == index)
                    .then_some(pc)
            })
            .collect::<Vec<_>>();
        let initialize_pc = code[..body_pc]
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| {
                matches!(instruction, Instruction::InitializeLocal(local) if *local == index)
                    .then_some(pc)
            })
            .collect::<Vec<_>>();
        if !matches!((reset_pc.as_slice(), initialize_pc.as_slice()), ([reset], [initialize]) if reset < initialize)
        {
            return Err("parameter-initializer local has no exact pre-boundary TDZ lifecycle");
        }
        if code[body_pc..].iter().any(references) {
            return Err("function body accesses a parameter-initializer local");
        }
    }
    Ok(())
}

/// Build the exact local set which compiler-authored code may expose to a
/// direct eval while an explicit Parameter Environment is active. This is the
/// eval counterpart of the child-closure capture allowlist: leading parameter
/// cells, nested initializer lexicals, and authenticated entry pseudo-bindings
/// are visible, while authored body storage is not.
pub(crate) fn parameter_initializer_visible_locals(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    parameter_body_pc: Option<usize>,
    parameter_initializer_locals: &[bool],
    parameter_environment: Option<&ParameterEnvironmentLayout>,
) -> Result<Option<Vec<bool>>, &'static str> {
    if parameter_initializer_locals.len() != usize::from(metadata.local_count) {
        return Err("parameter-initializer local classification has the wrong length");
    }
    let Some(_) = parameter_body_pc else {
        return Ok(None);
    };
    let layout = parameter_environment.ok_or("parameter boundary has no immutable layout")?;
    let mut allowed = vec![false; usize::from(metadata.local_count)];
    allowed
        .get_mut(..usize::from(metadata.parameter_environment_local_count))
        .ok_or("parameter environment exceeds function local slots")?
        .fill(true);
    for (allowed, is_initializer) in allowed.iter_mut().zip(parameter_initializer_locals) {
        *allowed |= *is_initializer;
    }
    if let Some(local) = metadata.function_name_local {
        *allowed
            .get_mut(usize::from(local))
            .ok_or("function-name local is outside bytecode local slots")? = true;
    }
    if let Some(local) = layout.arg_eval_variable_object_local {
        *allowed
            .get_mut(usize::from(local))
            .ok_or("parameter eval variable-object local is outside bytecode local slots")? = true;
    }
    if let Some(local) = metadata.derived_this_local {
        *allowed
            .get_mut(usize::from(local))
            .ok_or("derived this local is outside bytecode local slots")? = true;
    }

    let mut entry_pc = 0_usize;
    while let Some([source, Instruction::PutLocal(local)]) = code.get(entry_pc..entry_pc + 2) {
        if pseudo_binding_entry_rank(source).is_none() {
            break;
        }
        *allowed
            .get_mut(usize::from(*local))
            .ok_or("parameter pseudo-binding local is out of bounds")? = true;
        entry_pc += 2;
    }
    if let Some(synthetic) = layout.synthetic_arguments_local
        && matches!(
            code.get(entry_pc..entry_pc + 4),
            Some([
                Instruction::Arguments(_),
                Instruction::Dup,
                Instruction::InitializeLocal(target),
                Instruction::PutLocal(_),
            ]) if *target == synthetic
        )
    {
        *allowed
            .get_mut(usize::from(synthetic))
            .ok_or("synthetic parameter arguments local is out of bounds")? = true;
    } else if matches!(code.get(entry_pc), Some(Instruction::Arguments(_)))
        && let Some(Instruction::PutLocal(local)) = code.get(entry_pc + 1)
    {
        *allowed
            .get_mut(usize::from(*local))
            .ok_or("parameter arguments local is out of bounds")? = true;
    }
    Ok(Some(allowed))
}

/// Bind every immutable direct-eval descriptor to the bytecode phase which
/// references it. Descriptor topology and source metadata alone are
/// insufficient: a forged body-side `Eval` could otherwise reuse a Parameter
/// or BindingPattern initializer descriptor after its lexical scope ended.
pub(crate) struct EvalEnvironmentPhaseContext<'a> {
    pub(crate) metadata: &'a FunctionMetadata,
    pub(crate) code: &'a [Instruction],
    pub(crate) parameter_body_pc: Option<usize>,
    pub(crate) pattern_body_pc: Option<usize>,
    pub(crate) lexical_locals: &'a [bool],
    pub(crate) parameter_initializer_locals: &'a [bool],
    pub(crate) parameter_initializer_visible_locals: Option<&'a [bool]>,
    pub(crate) parameter_environment: Option<&'a ParameterEnvironmentLayout>,
}

pub(crate) fn validate_eval_environment_phase_layout<Name>(
    environments: &[EvalEnvironment<Name>],
    context: EvalEnvironmentPhaseContext<'_>,
) -> Result<(), &'static str> {
    let EvalEnvironmentPhaseContext {
        metadata,
        code,
        parameter_body_pc,
        pattern_body_pc,
        lexical_locals,
        parameter_initializer_locals,
        parameter_initializer_visible_locals,
        parameter_environment,
    } = context;
    let local_count = usize::from(metadata.local_count);
    if lexical_locals.len() != local_count
        || parameter_initializer_locals.len() != local_count
        || parameter_initializer_visible_locals.is_some_and(|visible| visible.len() != local_count)
    {
        return Err("eval parameter-phase local classification has the wrong length");
    }
    if parameter_body_pc.is_some() != parameter_initializer_visible_locals.is_some() {
        return Err("eval parameter-phase allowlist disagrees with its boundary");
    }

    let mut environment_pcs = vec![Vec::new(); environments.len()];
    for (pc, instruction) in code.iter().enumerate() {
        let Some(environment) = instruction.eval_environment() else {
            continue;
        };
        environment_pcs
            .get_mut(usize::from(environment))
            .ok_or("Eval bytecode environment operand is out of bounds")?
            .push(pc);
    }
    if environment_pcs.iter().any(Vec::is_empty) {
        return Err("eval environment descriptor is not referenced by bytecode");
    }

    let Some(body_pc) = parameter_body_pc.or(pattern_body_pc) else {
        return Ok(());
    };
    let explicit_parameter_environment = parameter_body_pc.is_some();
    let synthetic_arguments_local =
        parameter_environment.and_then(|layout| layout.synthetic_arguments_local);

    for (environment, pcs) in environments.iter().zip(environment_pcs) {
        let referenced_in_initializer = pcs.iter().any(|pc| *pc < body_pc);
        let referenced_in_body = pcs.iter().any(|pc| *pc >= body_pc);

        if explicit_parameter_environment {
            let current_anchor = environment
                .scopes
                .iter()
                .find(|scope| {
                    matches!(
                        scope.kind,
                        EvalScopeKind::FunctionRoot | EvalScopeKind::Parameter
                    )
                })
                .map(|scope| scope.kind)
                .ok_or("eval environment contains no current function anchor")?;
            if referenced_in_initializer && current_anchor != EvalScopeKind::Parameter {
                return Err("parameter initializer eval used a body environment descriptor");
            }
            if referenced_in_body && current_anchor != EvalScopeKind::FunctionRoot {
                return Err("function body eval used a parameter environment descriptor");
            }
        }

        for binding in environment
            .scopes
            .iter()
            .flat_map(|scope| scope.bindings.iter())
        {
            match binding.source {
                EvalBindingSource::Argument(_)
                    if referenced_in_initializer && explicit_parameter_environment =>
                {
                    return Err("parameter initializer eval captured a raw argument slot");
                }
                EvalBindingSource::Local(index) => {
                    let index_usize = usize::from(index);
                    let is_lexical = *lexical_locals
                        .get(index_usize)
                        .ok_or("eval binding local source is out of bounds")?;
                    let is_parameter_initializer =
                        *parameter_initializer_locals
                            .get(index_usize)
                            .ok_or("eval binding local source is out of bounds")?;
                    if referenced_in_initializer {
                        if let Some(visible) = parameter_initializer_visible_locals {
                            if !visible[index_usize] {
                                return Err(
                                    "parameter initializer eval captured a body-only local",
                                );
                            }
                        } else if pattern_body_pc.is_some()
                            && index >= metadata.parameter_environment_local_count
                            && synthetic_arguments_local != Some(index)
                            && is_lexical
                            && !is_parameter_initializer
                        {
                            return Err("pattern initializer eval captured a body lexical local");
                        }
                    }
                    // A body-side direct eval may legitimately retain the
                    // function's parameter cells and `<arg_var>` chain: those
                    // bindings remain part of name resolution after parameter
                    // initialization. Only a nested lexical whose complete
                    // lifetime ended at the boundary is forbidden here.
                    if referenced_in_body && is_parameter_initializer {
                        return Err("function body eval captured a parameter-initializer local");
                    }
                }
                EvalBindingSource::Argument(_) | EvalBindingSource::Closure(_) => {}
            }
        }
    }
    Ok(())
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

/// Publication-authenticated role of one class-private lexical capability.
///
/// Setter primary and synthetic `<set>` cells deliberately share
/// [`ClosureVariableKind::PrivateSetter`]. The runtime publisher is the only
/// layer which still owns both the exact source spelling and its interned
/// [`Atom`], so it seals that distinction here before handing linked bytecode
/// to the atom-table-independent heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PublishedPrivateBindingRole {
    Primary,
    SetterStorage,
}

/// One non-owning identity authenticated by the runtime publisher. `name`
/// must equal the atom already owned by the corresponding variable definition
/// or closure descriptor. Local setter halves also carry reciprocal `pair`
/// indices; closure captures may legitimately retain only one half and leave
/// it absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PublishedPrivateBinding {
    name: Atom,
    role: PublishedPrivateBindingRole,
    pair: Option<u16>,
}

impl PublishedPrivateBinding {
    #[must_use]
    pub(crate) const fn primary(name: Atom, pair: Option<u16>) -> Self {
        Self {
            name,
            role: PublishedPrivateBindingRole::Primary,
            pair,
        }
    }

    #[must_use]
    pub(crate) const fn setter_storage(name: Atom, pair: Option<u16>) -> Self {
        Self {
            name,
            role: PublishedPrivateBindingRole::SetterStorage,
            pair,
        }
    }
}

#[derive(Debug)]
struct AuthenticatedPrivateBindings {
    locals: Box<[Option<PublishedPrivateBinding>]>,
    closures: Box<[Option<PublishedPrivateBinding>]>,
}

/// Sealed bridge between name-aware bytecode publication and the heap.
///
/// Public linked-bytecode callers can construct only [`Self::none`]. Any
/// private definition or closure descriptor requires the crate-internal
/// authenticated constructor, preventing a forged `FunctionBytecodeData`
/// from choosing whether a `PrivateSetter` cell is a primary name or its
/// synthetic write capability.
#[derive(Debug)]
pub struct PublishedPrivateBindings {
    authenticated: Option<AuthenticatedPrivateBindings>,
}

impl PublishedPrivateBindings {
    #[must_use]
    pub const fn none() -> Self {
        Self {
            authenticated: None,
        }
    }

    #[must_use]
    pub(crate) fn authenticated(
        locals: Vec<Option<PublishedPrivateBinding>>,
        closures: Vec<Option<PublishedPrivateBinding>>,
    ) -> Self {
        Self {
            authenticated: Some(AuthenticatedPrivateBindings {
                locals: locals.into_boxed_slice(),
                closures: closures.into_boxed_slice(),
            }),
        }
    }
}

impl Default for PublishedPrivateBindings {
    fn default() -> Self {
        Self::none()
    }
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
/// Compiler drafts use [`JsString`] names. Runtime publication interns each
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

/// Runtime-owned immutable bytecode, constant pool, and function realm.
///
/// `code` is an `Rc` leaf with no runtime edges.  The VM may cheaply clone it
/// before dropping the runtime's `RefCell` borrow, while a bytecode root keeps
/// the raw constant pool alive for the duration of execution.
#[derive(Debug)]
pub struct FunctionBytecodeData {
    pub code: Rc<[Instruction]>,
    pub constants: Rc<[BytecodeConstant]>,
    pub realm: ContextId,
    pub metadata: FunctionMetadata,
    pub parameter_environment: Option<ParameterEnvironmentLayout>,
    /// Intrinsic source-level name. Contextual `SetName` inference remains a
    /// separate opcode and is only emitted for anonymous definitions.
    pub func_name: Option<JsString>,
    pub argument_definitions: Rc<[VariableDefinition]>,
    pub local_definitions: Rc<[VariableDefinition]>,
    pub closure_variables: Rc<[ClosureVariable]>,
    /// Name-bound private capability roles sealed by the runtime publisher.
    /// This metadata owns no atoms; identities alias definition/descriptor
    /// atoms whose references remain in `auxiliary_atoms`.
    pub private_bindings: PublishedPrivateBindings,
    pub eval_environments: Rc<[EvalEnvironment<Atom>]>,
    pub debug: Option<FunctionDebugInfo>,
    /// Atom references owned by bytecode metadata/opcode operands.
    ///
    /// Symbol constants are excluded: every `RawValue::Symbol` occurrence owns
    /// and releases its own separate atom reference.
    pub auxiliary_atoms: Box<[Atom]>,
}

/// Runtime-owned debug metadata for one bytecode function.
///
/// `filename` is backed by one distinct reference in `auxiliary_atoms`; it is
/// intentionally not released separately when the bytecode node dies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionDebugInfo {
    pub filename: Atom,
    pub pc2line: Option<Pc2LineTable>,
    pub source: Option<Box<[u8]>>,
}

/// Mutable storage shared by an active frame and all closures capturing one
/// argument or local.
///
/// `is_lexical` and `is_const` mirror the metadata carried by QuickJS
/// `JSVarRef`.  Enforcement belongs to the VM; the heap owns and traces the
/// current value.  The root returned by [`Heap::allocate_var_ref`] is intended
/// to be the active frame's ownership.  Function-object closure slots retain
/// the same identity and therefore keep the cell alive after frame teardown.
#[derive(Clone, Debug, PartialEq)]
pub struct VarRefData {
    pub value: RawValue,
    pub is_lexical: bool,
    pub is_const: bool,
    pub kind: ClosureVariableKind,
}

impl VarRefData {
    /// Construct a captured argument/local cell.
    #[must_use]
    pub const fn local(value: RawValue) -> Self {
        Self {
            value,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }
    }

    /// Construct a lexical/global-style cell with explicit const metadata.
    #[must_use]
    pub const fn lexical(value: RawValue, is_const: bool) -> Self {
        Self {
            value,
            is_lexical: true,
            is_const,
            kind: ClosureVariableKind::Normal,
        }
    }

    /// Construct a cell from one compiler-produced closure descriptor.
    #[must_use]
    pub const fn captured(
        value: RawValue,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Self {
        Self {
            value,
            is_lexical,
            is_const,
            kind,
        }
    }
}

fn private_binding_flags_are_valid(
    kind: ClosureVariableKind,
    is_lexical: bool,
    is_const: bool,
) -> bool {
    kind.is_private() && is_lexical && is_const
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PublishedPrivateBindingInfo {
    kind: ClosureVariableKind,
    role: PublishedPrivateBindingRole,
}

fn validate_published_private_binding_metadata(
    bytecode: &FunctionBytecodeData,
) -> Result<Option<&AuthenticatedPrivateBindings>, HeapError> {
    let has_private_bindings = bytecode
        .local_definitions
        .iter()
        .any(|definition| definition.kind.is_private())
        || bytecode
            .closure_variables
            .iter()
            .any(|descriptor| descriptor.kind.is_private());
    let Some(authenticated) = bytecode.private_bindings.authenticated.as_ref() else {
        return if has_private_bindings {
            Err(HeapError::Invariant(
                "published private bindings are missing sealed role metadata",
            ))
        } else {
            Ok(None)
        };
    };
    if authenticated.locals.len() != bytecode.local_definitions.len()
        || authenticated.closures.len() != bytecode.closure_variables.len()
    {
        return Err(HeapError::Invariant(
            "published private binding role table has the wrong shape",
        ));
    }

    for (definition, binding) in bytecode
        .local_definitions
        .iter()
        .zip(authenticated.locals.iter())
    {
        match (definition.kind.is_private(), binding) {
            (false, None) => continue,
            (false, Some(_)) => {
                return Err(HeapError::Invariant(
                    "ordinary local carries a sealed private binding role",
                ));
            }
            (true, None) => {
                return Err(HeapError::Invariant(
                    "published private local is missing its sealed binding role",
                ));
            }
            (true, Some(binding)) => {
                if !private_binding_flags_are_valid(
                    definition.kind,
                    definition.is_lexical,
                    definition.is_const,
                ) || definition.is_parameter_initializer
                    || definition.name != Some(binding.name)
                    || (binding.role == PublishedPrivateBindingRole::SetterStorage
                        && definition.kind != ClosureVariableKind::PrivateSetter)
                {
                    return Err(HeapError::Invariant(
                        "published private-name local has invalid sealed metadata",
                    ));
                }
            }
        }
    }
    for (descriptor, binding) in bytecode
        .closure_variables
        .iter()
        .zip(authenticated.closures.iter())
    {
        match (descriptor.kind.is_private(), binding) {
            (false, None) => continue,
            (false, Some(_)) => {
                return Err(HeapError::Invariant(
                    "ordinary closure carries a sealed private binding role",
                ));
            }
            (true, None) => {
                return Err(HeapError::Invariant(
                    "published private closure is missing its sealed binding role",
                ));
            }
            (true, Some(binding)) => {
                if !private_binding_flags_are_valid(
                    descriptor.kind,
                    descriptor.is_lexical,
                    descriptor.is_const,
                ) || descriptor.name != ClosureVariableName::Atom(binding.name)
                    || !matches!(
                        descriptor.source,
                        ClosureSource::ParentLocal(_)
                            | ClosureSource::ParentClosure(_)
                            | ClosureSource::EvalEnvironment(_)
                    )
                    || binding.pair.is_some()
                    || (binding.role == PublishedPrivateBindingRole::SetterStorage
                        && descriptor.kind != ClosureVariableKind::PrivateSetter)
                {
                    return Err(HeapError::Invariant(
                        "published private-name closure has invalid sealed metadata",
                    ));
                }
            }
        }
    }

    for (index, (definition, binding)) in bytecode
        .local_definitions
        .iter()
        .zip(authenticated.locals.iter())
        .enumerate()
    {
        let Some(binding) = binding else {
            continue;
        };
        let pair_required = binding.role == PublishedPrivateBindingRole::SetterStorage
            || (binding.role == PublishedPrivateBindingRole::Primary
                && matches!(
                    definition.kind,
                    ClosureVariableKind::PrivateSetter | ClosureVariableKind::PrivateGetterSetter
                ));
        if pair_required != binding.pair.is_some() {
            return Err(HeapError::Invariant(
                "published private setter has malformed pair metadata",
            ));
        }
        let Some(pair) = binding.pair else {
            continue;
        };
        let pair_index = usize::from(pair);
        let Some(Some(other)) = authenticated.locals.get(pair_index) else {
            return Err(HeapError::Invariant(
                "published private setter pair is out of bounds",
            ));
        };
        let Some(other_definition) = bytecode.local_definitions.get(pair_index) else {
            return Err(HeapError::Invariant(
                "published private setter pair is out of bounds",
            ));
        };
        if pair_index == index
            || other.pair != u16::try_from(index).ok()
            || other.role == binding.role
            || binding.name == other.name
            || match binding.role {
                PublishedPrivateBindingRole::Primary => {
                    !matches!(
                        definition.kind,
                        ClosureVariableKind::PrivateSetter
                            | ClosureVariableKind::PrivateGetterSetter
                    ) || other_definition.kind != ClosureVariableKind::PrivateSetter
                }
                PublishedPrivateBindingRole::SetterStorage => {
                    definition.kind != ClosureVariableKind::PrivateSetter
                        || !matches!(
                            other_definition.kind,
                            ClosureVariableKind::PrivateSetter
                                | ClosureVariableKind::PrivateGetterSetter
                        )
                }
            }
        {
            return Err(HeapError::Invariant(
                "published private setter pair is not reciprocal",
            ));
        }
    }
    Ok(Some(authenticated))
}

fn validate_published_private_source(
    bytecode: &FunctionBytecodeData,
    source: PrivateNameSource,
) -> Result<PublishedPrivateBindingInfo, HeapError> {
    let authenticated =
        bytecode
            .private_bindings
            .authenticated
            .as_ref()
            .ok_or(HeapError::Invariant(
                "private bytecode source has no sealed binding role",
            ))?;
    let info = match source {
        PrivateNameSource::Local(index) => bytecode
            .local_definitions
            .get(usize::from(index))
            .zip(authenticated.locals.get(usize::from(index)))
            .and_then(|(definition, binding)| {
                binding.map(|binding| PublishedPrivateBindingInfo {
                    kind: definition.kind,
                    role: binding.role,
                })
            }),
        PrivateNameSource::Closure(index) => bytecode
            .closure_variables
            .get(usize::from(index))
            .zip(authenticated.closures.get(usize::from(index)))
            .and_then(|(descriptor, binding)| {
                binding.map(|binding| PublishedPrivateBindingInfo {
                    kind: descriptor.kind,
                    role: binding.role,
                })
            }),
    };
    info.ok_or(HeapError::Invariant(
        "private bytecode source is not an authenticated lexical binding",
    ))
}

fn explicit_private_control_flow_target(instruction: &Instruction) -> Option<usize> {
    let target = match instruction {
        Instruction::Goto(target)
        | Instruction::IfFalse(target)
        | Instruction::IfTrue(target)
        | Instruction::Catch(target)
        | Instruction::Gosub(target) => *target,
        _ => return None,
    };
    usize::try_from(target).ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrivateCallableInitializerKind {
    Method,
    Accessor,
}

fn validate_published_private_callable_initializer(
    heap: &Heap,
    bytecode: &FunctionBytecodeData,
    pc: usize,
    binding_index: u16,
    accessor_role: Option<PublishedPrivateBindingRole>,
    explicit_control_flow_targets: &HashSet<usize>,
    kind: PrivateCallableInitializerKind,
) -> Result<(), HeapError> {
    let adjacent_error = match kind {
        PrivateCallableInitializerKind::Method => {
            "private-method initializer did not consume an adjacent closure"
        }
        PrivateCallableInitializerKind::Accessor => {
            "private-accessor initializer did not consume an adjacent closure"
        }
    };
    let closure_pc = pc
        .checked_sub(1)
        .ok_or(HeapError::Invariant(adjacent_error))?;
    if explicit_control_flow_targets.contains(&closure_pc)
        || explicit_control_flow_targets.contains(&pc)
    {
        return Err(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method closure/initializer pair has a non-fallthrough entry"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor closure/initializer pair has a non-fallthrough entry"
            }
        }));
    }
    let Some(Instruction::FClosure(constant)) = bytecode.code.get(closure_pc) else {
        return Err(HeapError::Invariant(adjacent_error));
    };
    let child_id = usize::try_from(*constant)
        .ok()
        .and_then(|constant| bytecode.constants.get(constant))
        .and_then(|constant| match constant {
            BytecodeConstant::Function(child) => Some(*child),
            BytecodeConstant::Value(_) | BytecodeConstant::RegExp { .. } => None,
        })
        .ok_or(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method initializer did not reference child bytecode"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor initializer did not reference child bytecode"
            }
        }))?;
    let child = heap.function_bytecode(child_id).map_err(|_| {
        HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method initializer did not reference live child bytecode"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor initializer did not reference live child bytecode"
            }
        })
    })?;
    let callable_shape_valid = match kind {
        PrivateCallableInitializerKind::Method => matches!(
            (child.metadata.function_kind, child.metadata.has_prototype),
            (FunctionKind::Normal, false) | (FunctionKind::Generator, true)
        ),
        PrivateCallableInitializerKind::Accessor => {
            child.metadata.function_kind == FunctionKind::Normal && !child.metadata.has_prototype
        }
    };
    if !child.metadata.needs_home_object
        || !child.metadata.strict
        || child.metadata.eval_kind != EvalKind::None
        || !callable_shape_valid
        || child.metadata.constructor_kind != ConstructorKind::None
        || child.metadata.class_initializer_kind.is_some()
    {
        return Err(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method child has invalid HomeObject metadata"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor child has invalid HomeObject metadata"
            }
        }));
    }
    if bytecode
        .code
        .iter()
        .filter(|instruction| {
            matches!(instruction, Instruction::FClosure(candidate) if candidate == constant)
        })
        .count()
        != 1
    {
        return Err(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method child did not have one unique closure site"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor child did not have one unique closure site"
            }
        }));
    }

    if let Some(role) = accessor_role {
        let expected_arguments = match role {
            PublishedPrivateBindingRole::Primary => 0,
            PublishedPrivateBindingRole::SetterStorage => 1,
        };
        if child.metadata.argument_count != expected_arguments {
            return Err(HeapError::Invariant(
                "private-accessor child has invalid authored arity",
            ));
        }
        if child
            .func_name
            .as_ref()
            .is_some_and(|name| !name.is_empty())
        {
            return Err(HeapError::Invariant(
                "private-accessor child retained a non-empty intrinsic name",
            ));
        }
    }

    let mut scope_entries = bytecode
        .code
        .iter()
        .enumerate()
        .filter_map(|(entry_pc, instruction)| {
            matches!(instruction, Instruction::SetLocalUninitialized(index) if *index == binding_index)
                .then_some(entry_pc)
        });
    let scope_entry_pc = scope_entries
        .next()
        .ok_or(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method initializer has no lexical scope entry"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor initializer has no lexical scope entry"
            }
        }))?;
    if scope_entries.next().is_some() || scope_entry_pc >= closure_pc {
        return Err(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method initializer has an invalid lexical scope entry"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor initializer has an invalid lexical scope entry"
            }
        }));
    }
    for (source_pc, instruction) in bytecode.code.iter().enumerate().skip(pc) {
        let Some(target_pc) = explicit_private_control_flow_target(instruction) else {
            continue;
        };
        if target_pc > scope_entry_pc && target_pc <= pc && source_pc >= pc {
            return Err(HeapError::Invariant(match kind {
                PrivateCallableInitializerKind::Method => {
                    "private-method initializer is reachable by a repeated-lifetime backedge"
                }
                PrivateCallableInitializerKind::Accessor => {
                    "private-accessor initializer is reachable by a repeated-lifetime backedge"
                }
            }));
        }
    }
    Ok(())
}

fn ordinary_private_local_operand(instruction: &Instruction) -> Option<u16> {
    match instruction {
        Instruction::GetLocal(index)
        | Instruction::PutLocal(index)
        | Instruction::SetLocal(index)
        | Instruction::GetLocalCheck(index)
        | Instruction::InitializeLocal(index)
        | Instruction::InitializeDerivedLocal(index)
        | Instruction::PutLocalCheck(index)
        | Instruction::SetLocalCheck(index)
        | Instruction::ReturnDerived(index) => Some(*index),
        _ => None,
    }
}

fn ordinary_private_closure_operand(instruction: &Instruction) -> Option<u16> {
    match instruction {
        Instruction::GetVarRef(index)
        | Instruction::PutVarRef(index)
        | Instruction::SetVarRef(index)
        | Instruction::GetVarRefCheck(index)
        | Instruction::PutVarRefCheck(index)
        | Instruction::InitializeDerivedVarRef(index)
        | Instruction::GetVar(index)
        | Instruction::GetVarUndef(index)
        | Instruction::DeleteVar(index)
        | Instruction::PutVar(index)
        | Instruction::PutVarInit(index)
        | Instruction::GlobalReference(index) => Some(*index),
        _ => None,
    }
}

fn validate_published_private_elements(
    heap: &Heap,
    bytecode: &FunctionBytecodeData,
) -> Result<(), HeapError> {
    let mut initialization_counts = vec![0_u8; bytecode.local_definitions.len()];
    let mut scope_entry_counts = vec![0_u8; bytecode.local_definitions.len()];
    let _ = validate_published_private_binding_metadata(bytecode)?;
    let explicit_control_flow_targets = bytecode
        .code
        .iter()
        .filter_map(explicit_private_control_flow_target)
        .collect::<HashSet<_>>();

    for (pc, instruction) in bytecode.code.iter().enumerate() {
        if let Some(index) = ordinary_private_local_operand(instruction)
            && bytecode
                .local_definitions
                .get(usize::from(index))
                .is_some_and(|definition| definition.kind.is_private())
        {
            return Err(HeapError::Invariant(
                "ordinary local bytecode references a private-name binding",
            ));
        }
        if let Some(index) = ordinary_private_closure_operand(instruction)
            && bytecode
                .closure_variables
                .get(usize::from(index))
                .is_some_and(|descriptor| descriptor.kind.is_private())
        {
            return Err(HeapError::Invariant(
                "ordinary closure bytecode references a private-name binding",
            ));
        }

        match *instruction {
            Instruction::SetLocalUninitialized(index)
                if bytecode
                    .local_definitions
                    .get(usize::from(index))
                    .is_some_and(|definition| definition.kind.is_private()) =>
            {
                let count =
                    scope_entry_counts
                        .get_mut(usize::from(index))
                        .ok_or(HeapError::Invariant(
                            "private-name scope-entry local is out of bounds",
                        ))?;
                *count = count.checked_add(1).ok_or(HeapError::Invariant(
                    "private-name scope-entry count overflowed",
                ))?;
            }
            Instruction::InitializePrivateName(index) => {
                if validate_published_private_source(bytecode, PrivateNameSource::Local(index))?
                    != (PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateField,
                        role: PublishedPrivateBindingRole::Primary,
                    })
                {
                    return Err(HeapError::Invariant(
                        "private-name initializer referenced a non-field binding",
                    ));
                }
                let count = initialization_counts.get_mut(usize::from(index)).ok_or(
                    HeapError::Invariant("private-name initializer local is out of bounds"),
                )?;
                *count = count.checked_add(1).ok_or(HeapError::Invariant(
                    "private-name initializer count overflowed",
                ))?;
            }
            Instruction::InitializePrivateMethod(index) => {
                if validate_published_private_source(bytecode, PrivateNameSource::Local(index))?
                    != (PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateMethod,
                        role: PublishedPrivateBindingRole::Primary,
                    })
                {
                    return Err(HeapError::Invariant(
                        "private-method initializer referenced a non-method binding",
                    ));
                }
                validate_published_private_callable_initializer(
                    heap,
                    bytecode,
                    pc,
                    index,
                    None,
                    &explicit_control_flow_targets,
                    PrivateCallableInitializerKind::Method,
                )?;
                let count = initialization_counts.get_mut(usize::from(index)).ok_or(
                    HeapError::Invariant("private-method initializer local is out of bounds"),
                )?;
                *count = count.checked_add(1).ok_or(HeapError::Invariant(
                    "private-method initializer count overflowed",
                ))?;
            }
            Instruction::InitializePrivateAccessor(index) => {
                let binding =
                    validate_published_private_source(bytecode, PrivateNameSource::Local(index))?;
                if !matches!(
                    binding,
                    PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateGetter
                            | ClosureVariableKind::PrivateGetterSetter,
                        role: PublishedPrivateBindingRole::Primary,
                    } | PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateSetter,
                        role: PublishedPrivateBindingRole::SetterStorage,
                    }
                ) {
                    return Err(HeapError::Invariant(
                        "private-accessor initializer referenced an incompatible binding",
                    ));
                }
                validate_published_private_callable_initializer(
                    heap,
                    bytecode,
                    pc,
                    index,
                    Some(binding.role),
                    &explicit_control_flow_targets,
                    PrivateCallableInitializerKind::Accessor,
                )?;
                let count = initialization_counts.get_mut(usize::from(index)).ok_or(
                    HeapError::Invariant("private-accessor initializer local is out of bounds"),
                )?;
                *count = count.checked_add(1).ok_or(HeapError::Invariant(
                    "private-accessor initializer count overflowed",
                ))?;
            }
            Instruction::GetPrivateField(source) | Instruction::GetPrivateField2(source) => {
                let binding = validate_published_private_source(bytecode, source)?;
                if binding.role != PublishedPrivateBindingRole::Primary
                    || !matches!(
                        binding.kind,
                        ClosureVariableKind::PrivateField
                            | ClosureVariableKind::PrivateMethod
                            | ClosureVariableKind::PrivateGetter
                            | ClosureVariableKind::PrivateGetterSetter
                    )
                {
                    return Err(HeapError::Invariant(
                        "private get referenced an incompatible binding",
                    ));
                }
            }
            Instruction::PutPrivateField(source) => {
                let binding = validate_published_private_source(bytecode, source)?;
                if !matches!(
                    binding,
                    PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateField,
                        role: PublishedPrivateBindingRole::Primary,
                    } | PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateSetter,
                        role: PublishedPrivateBindingRole::SetterStorage,
                    }
                ) {
                    return Err(HeapError::Invariant(
                        "private put referenced an incompatible binding",
                    ));
                }
            }
            Instruction::PrivateIn(source) => {
                if validate_published_private_source(bytecode, source)?.role
                    != PublishedPrivateBindingRole::Primary
                {
                    return Err(HeapError::Invariant(
                        "private-in referenced a synthetic setter binding",
                    ));
                }
            }
            Instruction::DefinePrivateField(source) => {
                if validate_published_private_source(bytecode, source)?
                    != (PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateField,
                        role: PublishedPrivateBindingRole::Primary,
                    })
                {
                    return Err(HeapError::Invariant(
                        "private-field definition referenced a non-field binding",
                    ));
                }
                if !matches!(
                    bytecode.metadata.class_initializer_kind,
                    Some(
                        ClassInitializerKind::InstanceFields | ClassInitializerKind::StaticElements
                    )
                ) {
                    return Err(HeapError::Invariant(
                        "private-field definition escaped a class initializer",
                    ));
                }
            }
            _ => {}
        }
    }

    let authenticated = bytecode.private_bindings.authenticated.as_ref();
    for (index, definition) in bytecode.local_definitions.iter().enumerate() {
        if !definition.kind.is_private() {
            continue;
        }
        let binding = authenticated
            .and_then(|authenticated| authenticated.locals.get(index))
            .and_then(Option::as_ref)
            .ok_or(HeapError::Invariant(
                "published private local lost its sealed binding role",
            ))?;
        let expected_initializers = usize::from(
            !(definition.kind == ClosureVariableKind::PrivateSetter
                && binding.role == PublishedPrivateBindingRole::Primary),
        );
        if usize::from(initialization_counts[index]) != expected_initializers {
            return Err(HeapError::Invariant(
                match (definition.kind, binding.role) {
                    (ClosureVariableKind::PrivateField, PublishedPrivateBindingRole::Primary) => {
                        "private-name local does not have exactly one lexical initializer"
                    }
                    (ClosureVariableKind::PrivateSetter, PublishedPrivateBindingRole::Primary) => {
                        "private-setter primary local must remain uninitialized"
                    }
                    (
                        ClosureVariableKind::PrivateGetter
                        | ClosureVariableKind::PrivateSetter
                        | ClosureVariableKind::PrivateGetterSetter,
                        _,
                    ) => "private-accessor local does not have its required typed initializer",
                    _ => "private-method local does not have exactly one typed initializer",
                },
            ));
        }
        if scope_entry_counts[index] != 1 {
            return Err(HeapError::Invariant(
                if definition.kind == ClosureVariableKind::PrivateField {
                    "private-name local does not have exactly one lexical scope entry"
                } else {
                    "private-method local does not have exactly one lexical scope entry"
                },
            ));
        }
    }

    let has_private_method_initializer = bytecode
        .code
        .iter()
        .any(|instruction| matches!(instruction, Instruction::InitializePrivateMethod(_)));
    let has_private_accessor_initializer = bytecode
        .code
        .iter()
        .any(|instruction| matches!(instruction, Instruction::InitializePrivateAccessor(_)));
    let mut has_private_brand_child = false;
    for constant in bytecode.constants.iter() {
        let BytecodeConstant::Function(child) = constant else {
            continue;
        };
        let child = heap.function_bytecode(*child).map_err(|_| {
            HeapError::Invariant("private declaration referenced non-live child bytecode")
        })?;
        has_private_brand_child |= child.metadata.class_private_brand
            && matches!(
                child.metadata.class_initializer_kind,
                Some(ClassInitializerKind::InstanceFields | ClassInitializerKind::StaticElements)
            );
    }
    if (has_private_method_initializer || has_private_accessor_initializer)
        != has_private_brand_child
    {
        return Err(HeapError::Invariant(if has_private_accessor_initializer {
            "private-callable declarations disagree with class brand initializer metadata"
        } else {
            "private-method declarations disagree with class brand initializer metadata"
        }));
    }
    Ok(())
}

/// Internal primitive payload carried by implemented wrapper classes.
///
/// New variants are added only with their complete class slice so Symbol atom
/// ownership and String exotic storage cannot be accidentally skipped by a
/// prematurely generic raw-value container.
#[derive(Clone, Debug, PartialEq)]
pub enum PrimitiveObjectData {
    Number(f64),
    /// Exact UTF-16 backing store for a genuine String wrapper. Unlike Symbol,
    /// the reference-counted string payload owns no atom or heap edge.
    String(JsString),
    Boolean(bool),
    /// One owned atom reference for a genuine local, global, or well-known
    /// Symbol. `object_atoms` returns it during wrapper finalization.
    Symbol(Atom),
    BigInt(JsBigInt),
}

impl PrimitiveObjectData {
    #[must_use]
    pub const fn kind(&self) -> PrimitiveKind {
        match self {
            Self::Number(_) => PrimitiveKind::Number,
            Self::String(_) => PrimitiveKind::String,
            Self::Boolean(_) => PrimitiveKind::Boolean,
            Self::Symbol(_) => PrimitiveKind::Symbol,
            Self::BigInt(_) => PrimitiveKind::BigInt,
        }
    }
}

/// Internal payload of one genuine `JS_CLASS_REGEXP` object.
///
/// QuickJS allocates the branded object before compiling its pattern, so the
/// explicit uninitialized state preserves that observable allocation/error
/// order. Compiled programs and their source strings are reference-counted
/// leaves outside the GC arena and own no heap or atom edge.
#[derive(Clone, Debug, PartialEq)]
pub enum RegExpObjectData {
    Uninitialized,
    Compiled {
        pattern: JsString,
        program: Rc<CompiledRegExp>,
    },
}

/// One stable insertion-order slot in a genuine Map.
///
/// Deletion clears `key` and resets `value` to `undefined` rather than
/// removing the record. Stable indices are observable through live Map
/// iterators: deleting and re-adding a key appends a fresh record, and records
/// appended after iterator creation remain visible until exhaustion.
#[derive(Clone, Debug, PartialEq)]
pub struct MapRecord {
    pub key: Option<RawValue>,
    pub value: RawValue,
}

/// ECMAScript-visible lifecycle of a branded synchronous generator object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GeneratorState {
    SuspendedStart,
    SuspendedYield,
    SuspendedYieldStar,
    Executing,
    Completed,
}

/// Heap-native representation of one argument or local binding retained by a
/// dormant generator frame. Runtime-owning root wrappers must never enter this
/// structure: every GC identity is stored as a raw arena edge instead.
#[derive(Clone, Debug, PartialEq)]
pub enum GeneratorFrameBinding {
    Direct(RawValue),
    Private(Atom),
    PrivateCallable(ObjectId),
    Uninitialized,
    Captured(VarRefId),
}

/// Raw VM fields retained across a synchronous-generator suspension.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratorVmActivation {
    pub stack: Vec<RawValue>,
    pub regions: Vec<crate::vm::VmUnwindRegion>,
    pub pc: usize,
    pub callee_realm: ContextId,
    pub current_function: ObjectId,
    pub this_value: RawValue,
    pub normalized_this: Option<RawValue>,
    pub new_target: RawValue,
    pub strict: bool,
    pub callee_global: ObjectId,
}

/// Complete dormant execution state owned by one generator object.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratorActivationData {
    pub bytecode: FunctionBytecodeId,
    pub vm: GeneratorVmActivation,
    pub actual_argument_count: usize,
    pub arguments: Vec<GeneratorFrameBinding>,
    pub locals: Vec<GeneratorFrameBinding>,
    pub reusable_captured_locals: Vec<bool>,
}

/// ECMAScript-visible state of one genuine Promise object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PromiseState {
    Pending,
    Fulfilled,
    Rejected,
}

/// Which settlement path owns a Promise reaction record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PromiseReactionKind {
    Fulfill,
    Reject,
}

/// Raw, heap-owned resolving functions retained by a Promise reaction.
///
/// These are internal arena edges, not public runtime-owning wrappers.  A
/// reaction keeps both callables alive until it is detached or its owning
/// Promise is finalized. The result Promise itself is returned synchronously
/// from `then`; QuickJS does not retain it as a separate reaction edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromiseCapabilityData {
    pub resolve: ObjectId,
    pub reject: ObjectId,
}

/// One `PerformPromiseThen` reaction retained by a pending Promise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromiseReaction {
    pub kind: PromiseReactionKind,
    pub handler: Option<ObjectId>,
    pub capability: PromiseCapabilityData,
}

/// Complete hidden state of one genuine Promise object.
///
/// `result` is `undefined` while pending.  Reaction vectors are kept separate
/// to preserve QuickJS's fulfill/reject list order, while each record also
/// carries its kind so queued jobs remain self-describing after detachment.
#[derive(Clone, Debug, PartialEq)]
pub struct PromiseData {
    pub state: PromiseState,
    pub result: RawValue,
    pub fulfill_reactions: Vec<PromiseReaction>,
    pub reject_reactions: Vec<PromiseReaction>,
    pub is_handled: bool,
}

/// Mutable edge capture owned by an internal NewPromiseCapability executor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PromiseCapabilityExecutorData {
    pub resolve: Option<RawValue>,
    pub reject: Option<RawValue>,
}

/// Typed hidden payload carried only by runtime-created native functions.
///
/// The shared `already_resolved` cell is intentionally a non-arena leaf: the
/// resolve/reject pair shares one first-call-wins bit without introducing a
/// `Runtime -> heap -> Runtime` ownership cycle.  Every raw object identity in
/// this enum is still an ordinary traced and reference-counted heap edge.
#[derive(Clone, Debug, PartialEq)]
pub enum InternalCallableData {
    PromiseResolving {
        promise: ObjectId,
        already_resolved: Rc<Cell<bool>>,
        kind: PromiseResolvingKind,
    },
    PromiseCapabilityExecutor(PromiseCapabilityExecutorData),
}

/// Class-specific edges stored alongside an object's ordinary properties.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectPayload {
    Ordinary,
    /// Runtime-wide, unforgeable `JS_CLASS_RAWJSON` brand. The exact source
    /// text remains in the object's frozen ordinary `rawJSON` data slot, so
    /// the payload needs no duplicate GC edge or string owner.
    RawJson,
    /// A genuine `JS_CLASS_ARRAY` exotic object. Indexed elements and the
    /// mandatory length property remain in the ordinary shape/slot arrays;
    /// this class marker selects ArraySetLength and index-growth semantics at
    /// the runtime boundary without disguising an Array as an ordinary object.
    Array {
        /// QuickJS's observable dense `fast_array` representation. `Some`
        /// stores `u.array.count`; `None` means the object was irreversibly
        /// converted to ordinary indexed properties.
        fast_len: Option<u32>,
    },
    /// QuickJS's two arguments classes share the same fast indexed storage
    /// protocol. Mapped entries use `PropertySlot::VarRef`; unmapped entries
    /// use ordinary data slots. `None` records the irreversible fast-to-slow
    /// transition caused by redefining or deleting a non-tail index.
    Arguments {
        mapped: bool,
        fast_len: Option<u32>,
    },
    /// `JS_CLASS_ARRAY_ITERATOR`: the boxed source is released permanently at
    /// exhaustion, while `kind` selects keys, values, or entry pairs.
    ArrayIterator {
        object: Option<ObjectId>,
        next_index: u32,
        kind: ArrayIteratorKind,
    },
    /// QuickJS `JS_CLASS_FOR_IN_ITERATOR`. Property names and enumerable bits
    /// are snapshotted one prototype level at a time; the current object is
    /// the payload's only GC edge.
    ForInIterator(ForInIteratorData),
    /// QuickJS `JSObject.u.object_data` for implemented primitive wrappers.
    Primitive(PrimitiveObjectData),
    /// QuickJS `JS_CLASS_DATE`'s internal millisecond time value. NaN is the
    /// required invalid-Date sentinel for genuine Date instances.
    Date(f64),
    /// QuickJS `JS_CLASS_REGEXP`'s source and compiled matcher program.
    RegExp(RegExpObjectData),
    /// `JS_CLASS_REGEXP_STRING_ITERATOR`: the species-created matcher is the
    /// payload's sole arena edge. The iterated UTF-16 string is reference
    /// counted outside the arena. Completion deliberately retains both values
    /// until finalization, matching QuickJS's class finalizer.
    RegExpStringIterator {
        regexp: ObjectId,
        string: JsString,
        global: bool,
        full_unicode: bool,
        done: bool,
    },
    /// `JS_CLASS_MAP`: stable insertion-order records plus the live-entry
    /// count used by the `size` getter. Tombstones are never compacted while
    /// the Map is live, preserving mutation-sensitive iterator semantics.
    Map {
        records: Vec<MapRecord>,
        size: usize,
    },
    /// `JS_CLASS_MAP_ITERATOR`: the source Map remains an owned edge until
    /// exhaustion, while `next_index` walks stable record indices and skips
    /// tombstones in the runtime layer.
    MapIterator {
        object: Option<ObjectId>,
        next_index: usize,
        kind: MapIteratorKind,
    },
    /// `JS_CLASS_SET`: the ordered record layout is shared with Map, but each
    /// live record stores its element in `key` and keeps `value` exactly
    /// `undefined`. A distinct payload preserves the unforgeable Set brand.
    Set {
        records: Vec<MapRecord>,
        size: usize,
    },
    /// `JS_CLASS_SET_ITERATOR`: the source Set remains an owned edge until
    /// exhaustion. `kind` distinguishes value iteration from entry-pair
    /// iteration while both `keys` and `values` use the value projection.
    SetIterator {
        object: Option<ObjectId>,
        next_index: usize,
        kind: SetIteratorKind,
    },
    /// Realm global object and its hidden table of unresolved global VarRefs.
    GlobalObject {
        uninitialized_vars: ObjectId,
    },
    Error,
    /// `%StringIteratorPrototype%` instances own the iterated UTF-16 string
    /// and the next code-unit index.  The string is reference counted outside
    /// the GC arena, so this payload adds no arena edge while still keeping
    /// lone surrogates and rope-backed strings exact.
    StringIterator {
        string: Option<JsString>,
        next_index: usize,
    },
    NativeFunction {
        data: NativeFunctionData,
        internal: Option<InternalCallableData>,
    },
    /// QuickJS `JSBoundFunction`: the target, bound receiver and each bound
    /// argument are independently owned edges of the function object.
    BoundFunction {
        target: ObjectId,
        this_value: RawValue,
        arguments: Rc<[RawValue]>,
    },
    BytecodeFunction {
        bytecode: FunctionBytecodeId,
        home_object: Option<ObjectId>,
        /// Hidden instance-field initializer owned by a class constructor.
        /// The edge is deliberately internal: authored code cannot forge or
        /// overwrite QuickJS's `<class_fields_init>` binding.
        class_instance_initializer: Option<ObjectId>,
        /// One-shot guard for the aggregate static-elements program. Authored
        /// loops create a fresh constructor and therefore a fresh guard; forged
        /// bytecode cannot replay static initialization on the same class.
        class_static_initializer_started: bool,
        /// One owned reference per bytecode closure slot, matching QuickJS's
        /// `JSObject.u.func.var_refs[]` ownership.
        closure_slots: Vec<VarRefId>,
    },
    /// `JS_CLASS_GENERATOR`: the branded result object owns the complete
    /// dormant frame while suspended. `Executing` temporarily moves that
    /// activation into rooted Rust values so reentrant calls can observe the
    /// state without creating an invisible side-table root.
    Generator {
        state: GeneratorState,
        activation: Option<Box<GeneratorActivationData>>,
    },
    /// `JS_CLASS_PROMISE`: settlement state, result, and pending reactions are
    /// traced directly in the arena rather than hidden in a runtime side map.
    Promise(PromiseData),
}

/// Object storage category.  Additional QuickJS classes will extend this enum
/// while retaining the same arena and collection protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Ordinary,
    Array,
    Arguments,
    ArrayIterator,
    ForInIterator,
    Primitive,
    Date,
    RegExp,
    RegExpStringIterator,
    Map,
    MapIterator,
    Set,
    SetIterator,
    GlobalObject,
    Error,
    StringIterator,
    NativeFunction,
    BoundFunction,
    BytecodeFunction,
    Generator,
    Promise,
}

/// One string-key entry captured by QuickJS's `JS_GPN_SET_ENUM` enumeration.
/// `JsString` avoids storing runtime-owning `PropertyKey` roots in the heap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForInProperty {
    pub name: JsString,
    pub enumerable: bool,
}

/// Mutable state of one hidden for-in enumeration object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForInIteratorData {
    pub object: Option<ObjectId>,
    pub index: usize,
    pub properties: Vec<ForInProperty>,
    /// The iterator, not the source object, remembers whether QuickJS selected
    /// its count-only fast-Array path at loop entry.
    pub fast_array: bool,
    pub array_count: u32,
    pub in_prototype_chain: bool,
    pub visited: HashSet<JsString>,
}

/// One non-observable step selected from a hidden for-in iterator. The
/// runtime performs live property/prototype operations only after the heap
/// borrow used to advance the cursor has ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForInCandidate {
    Done,
    BaseComplete { object: ObjectId, fast_array: bool },
    LevelComplete(ObjectId),
    ArrayIndex { object: ObjectId, index: u32 },
    Property { object: ObjectId, name: JsString },
}

/// Observable mode selected by `Array.prototype.keys`, `values`, or
/// `entries`. QuickJS stores this in `JSArrayIteratorData.kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayIteratorKind {
    Key,
    Value,
    KeyAndValue,
}

/// Observable projection selected by `Map.prototype.keys`, `values`, or
/// `entries`. QuickJS stores this mode in its Map Iterator class payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapIteratorKind {
    Key,
    Value,
    KeyAndValue,
}

/// Observable projection selected by `Set.prototype.values`/`keys` or
/// `entries`. QuickJS stores values in the shared ordered Map record key slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SetIteratorKind {
    Value,
    KeyAndValue,
}

/// Observable comparison and traversal mode selected by
/// `Array.prototype.includes`, `indexOf`, or `lastIndexOf`. QuickJS exposes
/// three generic C functions with one algorithmic kernel per mode; retaining
/// that distinction in the native identity avoids dispatch by property name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArraySearchKind {
    Includes,
    IndexOf,
    LastIndexOf,
}

/// Stringification mode selected by QuickJS's shared `js_array_join` kernel.
/// `join` converts an optional separator, whereas `toLocaleString` always uses
/// a comma and invokes each non-nullish element's locale-string method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayJoinKind {
    Join,
    ToLocaleString,
}

/// Head/tail removal mode selected by QuickJS's shared `js_array_pop` kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayPopKind {
    Pop,
    Shift,
}

/// Tail/head insertion mode selected by QuickJS's shared `js_array_push`
/// kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayPushKind {
    Push,
    Unshift,
}

/// Copy/removal mode selected by QuickJS's shared `js_array_slice` kernel.
/// `slice` only materializes the selected range, whereas `splice` returns the
/// same species-created range before mutating the receiver in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArraySliceKind {
    Slice,
    Splice,
}

/// Observable traversal and result mode selected by the four
/// `Array.prototype.find*` methods. QuickJS passes this as the magic value to
/// one shared generic callback kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayFindKind {
    Find,
    FindIndex,
    FindLast,
    FindLastIndex,
}

/// Modes of QuickJS's shared `js_array_every` callback kernel. The typed
/// selector preserves the upstream branch identity without leaking C magic
/// integers into runtime dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayIterationKind {
    Every,
    Some,
    ForEach,
    Map,
    Filter,
}

/// Direction selected by QuickJS's shared `js_array_reduce` accumulator
/// kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayReduceKind {
    Reduce,
    ReduceRight,
}

/// Mode selected by QuickJS's shared `js_array_flatten` kernel. `flatMap`
/// validates and applies a mapper to the outer source before flattening one
/// level, while `flat` converts its requested depth with `JS_ToInt32Sat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayFlattenKind {
    FlatMap,
    Flat,
}

/// Observable key category selected by `%Object%.getOwnPropertyNames` and
/// `%Object%.getOwnPropertySymbols`. QuickJS uses two thin C wrappers around
/// the same `JS_GetOwnPropertyNames2` kernel; retaining the distinction in the
/// native identity keeps Rust dispatch free of string-name tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectOwnPropertyKeysKind {
    Names,
    Symbols,
}

/// Result shape selected by QuickJS's shared `js_object_keys` implementation
/// for `%Object%.keys`, `%Object%.values`, and `%Object%.entries`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKeysKind {
    Keys,
    Values,
    Entries,
}

/// Operation selected by QuickJS's adjacent `%Object%.isExtensible` and
/// `%Object%.preventExtensions` generic-magic builtins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectExtensibilityKind {
    IsExtensible,
    PreventExtensions,
}

/// Operation selected by QuickJS's four generic-magic Object integrity
/// builtins. The mutation and predicate pairs share the same freeze bit in C.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectIntegrityKind {
    Seal,
    Freeze,
    IsSealed,
    IsFrozen,
}

/// Type-safe replacement for QuickJS's getter/setter magic values shared by
/// the Annex-B `__define*__` and `__lookup*__` method families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectAccessorKind {
    Getter,
    Setter,
}

/// Operation selected by pinned QuickJS's complete `js_reflect_funcs` table.
///
/// Keeping the thirteen entries typed preserves both upstream table order and
/// the Generic versus GenericMagic ABI distinction without dispatching on a
/// mutable JavaScript function name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReflectKind {
    Apply,
    Construct,
    DefineProperty,
    DeleteProperty,
    Get,
    GetOwnPropertyDescriptor,
    GetPrototypeOf,
    Has,
    IsExtensible,
    OwnKeys,
    PreventExtensions,
    Set,
    SetPrototypeOf,
}

/// Operation selected by pinned QuickJS's complete `js_json_funcs` table.
///
/// The typed family keeps the public property order stable while parse,
/// stringify and Raw JSON land as separately reviewable algorithmic slices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JsonNativeKind {
    IsRawJson,
    Parse,
    RawJson,
    Stringify,
}

/// Operation selected by QuickJS's shared `js_math_min_max` generic-magic
/// builtin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathMinMaxKind {
    Min,
    Max,
}

/// QuickJS `JS_CFUNC_f_f` Math functions, in pinned table order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathUnaryKind {
    Abs,
    Floor,
    Ceil,
    Round,
    Sqrt,
    Acos,
    Asin,
    Atan,
    Cos,
    Exp,
    Log,
    Sin,
    Tan,
    Trunc,
    Sign,
    Cosh,
    Sinh,
    Tanh,
    Acosh,
    Asinh,
    Atanh,
    Expm1,
    Log1p,
    Log2,
    Log10,
    Cbrt,
    F16Round,
    FRound,
}

/// QuickJS `JS_CFUNC_f_f_f` Math functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MathBinaryKind {
    Atan2,
    Pow,
}

/// The eight pinned QuickJS `get_date_string` formatter modes.
///
/// The typed selector is the Rust equivalent of the C callback's magic value:
/// callers can route on the semantic mode while [`Self::quickjs_magic`] keeps
/// the exact upstream table encoding available for differential assertions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateStringMethod {
    ToString,
    ToDateString,
    ToTimeString,
    ToUtcString,
    ToIsoString,
    ToLocaleString,
    ToLocaleDateString,
    ToLocaleTimeString,
}

impl DateStringMethod {
    pub const ALL: [Self; 8] = [
        Self::ToString,
        Self::ToUtcString,
        Self::ToIsoString,
        Self::ToDateString,
        Self::ToTimeString,
        Self::ToLocaleString,
        Self::ToLocaleDateString,
        Self::ToLocaleTimeString,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ToString => "toString",
            Self::ToDateString => "toDateString",
            Self::ToTimeString => "toTimeString",
            Self::ToUtcString => "toUTCString",
            Self::ToIsoString => "toISOString",
            Self::ToLocaleString => "toLocaleString",
            Self::ToLocaleDateString => "toLocaleDateString",
            Self::ToLocaleTimeString => "toLocaleTimeString",
        }
    }

    #[must_use]
    pub const fn uses_local_time(self) -> bool {
        matches!(
            self,
            Self::ToString
                | Self::ToDateString
                | Self::ToTimeString
                | Self::ToLocaleString
                | Self::ToLocaleDateString
                | Self::ToLocaleTimeString
        )
    }

    /// Pinned `JS_CFUNC_MAGIC_DEF` value from `js_date_proto_funcs`.
    #[must_use]
    pub const fn quickjs_magic(self) -> u16 {
        match self {
            Self::ToString => 0x13,
            Self::ToDateString => 0x11,
            Self::ToTimeString => 0x12,
            Self::ToUtcString => 0x03,
            Self::ToIsoString => 0x23,
            Self::ToLocaleString => 0x33,
            Self::ToLocaleDateString => 0x31,
            Self::ToLocaleTimeString => 0x32,
        }
    }
}

/// Field selected by QuickJS's shared `get_date_field` callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateGetFieldKind {
    Year,
    FullYear,
    UtcFullYear,
    Month,
    UtcMonth,
    Date,
    UtcDate,
    Hours,
    UtcHours,
    Minutes,
    UtcMinutes,
    Seconds,
    UtcSeconds,
    Milliseconds,
    UtcMilliseconds,
    Day,
    UtcDay,
}

impl DateGetFieldKind {
    pub const ALL: [Self; 17] = [
        Self::Year,
        Self::FullYear,
        Self::UtcFullYear,
        Self::Month,
        Self::UtcMonth,
        Self::Date,
        Self::UtcDate,
        Self::Hours,
        Self::UtcHours,
        Self::Minutes,
        Self::UtcMinutes,
        Self::Seconds,
        Self::UtcSeconds,
        Self::Milliseconds,
        Self::UtcMilliseconds,
        Self::Day,
        Self::UtcDay,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Year => "getYear",
            Self::FullYear => "getFullYear",
            Self::UtcFullYear => "getUTCFullYear",
            Self::Month => "getMonth",
            Self::UtcMonth => "getUTCMonth",
            Self::Date => "getDate",
            Self::UtcDate => "getUTCDate",
            Self::Hours => "getHours",
            Self::UtcHours => "getUTCHours",
            Self::Minutes => "getMinutes",
            Self::UtcMinutes => "getUTCMinutes",
            Self::Seconds => "getSeconds",
            Self::UtcSeconds => "getUTCSeconds",
            Self::Milliseconds => "getMilliseconds",
            Self::UtcMilliseconds => "getUTCMilliseconds",
            Self::Day => "getDay",
            Self::UtcDay => "getUTCDay",
        }
    }

    /// Index in QuickJS's nine-element decomposed Date field array.
    #[must_use]
    pub const fn field_index(self) -> u8 {
        match self {
            Self::Year | Self::FullYear | Self::UtcFullYear => 0,
            Self::Month | Self::UtcMonth => 1,
            Self::Date | Self::UtcDate => 2,
            Self::Hours | Self::UtcHours => 3,
            Self::Minutes | Self::UtcMinutes => 4,
            Self::Seconds | Self::UtcSeconds => 5,
            Self::Milliseconds | Self::UtcMilliseconds => 6,
            Self::Day | Self::UtcDay => 7,
        }
    }

    #[must_use]
    pub const fn uses_local_time(self) -> bool {
        matches!(
            self,
            Self::Year
                | Self::FullYear
                | Self::Month
                | Self::Date
                | Self::Hours
                | Self::Minutes
                | Self::Seconds
                | Self::Milliseconds
                | Self::Day
        )
    }

    #[must_use]
    pub const fn is_legacy_year(self) -> bool {
        matches!(self, Self::Year)
    }

    /// Pinned `JS_CFUNC_MAGIC_DEF` value from `js_date_proto_funcs`.
    #[must_use]
    pub const fn quickjs_magic(self) -> u16 {
        ((self.is_legacy_year() as u16) << 8)
            | (self.field_index() as u16) << 4
            | self.uses_local_time() as u16
    }
}

/// Field range and UTC/local mode selected by QuickJS's shared
/// `set_date_field` callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateSetFieldKind {
    Milliseconds,
    UtcMilliseconds,
    Seconds,
    UtcSeconds,
    Minutes,
    UtcMinutes,
    Hours,
    UtcHours,
    Date,
    UtcDate,
    Month,
    UtcMonth,
    FullYear,
    UtcFullYear,
}

impl DateSetFieldKind {
    pub const ALL: [Self; 14] = [
        Self::Milliseconds,
        Self::UtcMilliseconds,
        Self::Seconds,
        Self::UtcSeconds,
        Self::Minutes,
        Self::UtcMinutes,
        Self::Hours,
        Self::UtcHours,
        Self::Date,
        Self::UtcDate,
        Self::Month,
        Self::UtcMonth,
        Self::FullYear,
        Self::UtcFullYear,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Milliseconds => "setMilliseconds",
            Self::UtcMilliseconds => "setUTCMilliseconds",
            Self::Seconds => "setSeconds",
            Self::UtcSeconds => "setUTCSeconds",
            Self::Minutes => "setMinutes",
            Self::UtcMinutes => "setUTCMinutes",
            Self::Hours => "setHours",
            Self::UtcHours => "setUTCHours",
            Self::Date => "setDate",
            Self::UtcDate => "setUTCDate",
            Self::Month => "setMonth",
            Self::UtcMonth => "setUTCMonth",
            Self::FullYear => "setFullYear",
            Self::UtcFullYear => "setUTCFullYear",
        }
    }

    /// First Date field replaced by the first supplied argument.
    #[must_use]
    pub const fn first_field(self) -> u8 {
        match self {
            Self::FullYear | Self::UtcFullYear => 0,
            Self::Month | Self::UtcMonth => 1,
            Self::Date | Self::UtcDate => 2,
            Self::Hours | Self::UtcHours => 3,
            Self::Minutes | Self::UtcMinutes => 4,
            Self::Seconds | Self::UtcSeconds => 5,
            Self::Milliseconds | Self::UtcMilliseconds => 6,
        }
    }

    /// Exclusive end of the consecutive Date field range accepted by this
    /// setter. The published function length is `end_field - first_field`.
    #[must_use]
    pub const fn end_field(self) -> u8 {
        match self {
            Self::Date | Self::UtcDate => 3,
            Self::Month | Self::UtcMonth => 3,
            Self::FullYear | Self::UtcFullYear => 3,
            Self::Hours | Self::UtcHours => 7,
            Self::Minutes | Self::UtcMinutes => 7,
            Self::Seconds | Self::UtcSeconds => 7,
            Self::Milliseconds | Self::UtcMilliseconds => 7,
        }
    }

    #[must_use]
    pub const fn uses_local_time(self) -> bool {
        matches!(
            self,
            Self::Milliseconds
                | Self::Seconds
                | Self::Minutes
                | Self::Hours
                | Self::Date
                | Self::Month
                | Self::FullYear
        )
    }

    #[must_use]
    pub const fn length(self) -> u8 {
        self.end_field() - self.first_field()
    }

    /// Pinned `JS_CFUNC_MAGIC_DEF` value from `js_date_proto_funcs`.
    #[must_use]
    pub const fn quickjs_magic(self) -> u16 {
        (self.first_field() as u16) << 8
            | (self.end_field() as u16) << 4
            | self.uses_local_time() as u16
    }
}

/// Typed handler family for every callable in pinned QuickJS's Date tables.
///
/// `TimeValue` deliberately backs both `valueOf` and `getTime`, matching their
/// shared C callback. The per-property name and length remain ordinary
/// function-object metadata rather than part of [`NativeFunctionDescriptor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateNativeKind {
    Constructor,
    Now,
    Parse,
    Utc,
    TimeValue,
    String(DateStringMethod),
    ToPrimitive,
    TimezoneOffset,
    GetField(DateGetFieldKind),
    SetTime,
    SetField(DateSetFieldKind),
    SetYear,
    ToJson,
}

impl DateNativeKind {
    /// The unique published function name for this target. `TimeValue` is the
    /// sole shared target: QuickJS publishes distinct `valueOf` and `getTime`
    /// function objects backed by that same callback, so their names remain
    /// per-object intrinsic-table metadata.
    #[must_use]
    pub const fn unique_name(self) -> Option<&'static str> {
        match self {
            Self::Constructor => Some("Date"),
            Self::Now => Some("now"),
            Self::Parse => Some("parse"),
            Self::Utc => Some("UTC"),
            Self::TimeValue => None,
            Self::String(kind) => Some(kind.name()),
            Self::ToPrimitive => Some("[Symbol.toPrimitive]"),
            Self::TimezoneOffset => Some("getTimezoneOffset"),
            Self::GetField(kind) => Some(kind.name()),
            Self::SetTime => Some("setTime"),
            Self::SetField(kind) => Some(kind.name()),
            Self::SetYear => Some("setYear"),
            Self::ToJson => Some("toJSON"),
        }
    }

    /// Published `length` for this handler. Both properties using
    /// `TimeValue` have length zero.
    #[must_use]
    pub const fn length(self) -> u8 {
        match self {
            Self::Constructor | Self::Utc => 7,
            Self::Now
            | Self::TimeValue
            | Self::String(_)
            | Self::TimezoneOffset
            | Self::GetField(_) => 0,
            Self::Parse | Self::ToPrimitive | Self::SetTime | Self::SetYear | Self::ToJson => 1,
            Self::SetField(kind) => kind.length(),
        }
    }
}

/// Typed selector for pinned QuickJS's shared RegExp flag getter.  The order
/// follows the public flag surface rather than exposing the engine's bitmask
/// constants to runtime dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegExpFlagKind {
    HasIndices,
    Global,
    IgnoreCase,
    Multiline,
    DotAll,
    Unicode,
    UnicodeSets,
    Sticky,
}

/// Typed handler family for the published RegExp constructor/prototype
/// surface. `Flag` corresponds to QuickJS's getter-magic callback; all other
/// variants preserve their table's concrete C protocol through
/// [`NativeFunctionId::descriptor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegExpNativeKind {
    Constructor,
    Species,
    Exec,
    Compile,
    Test,
    ToString,
    Replace,
    Match,
    MatchAll,
    Search,
    Split,
    Source,
    Flags,
    Flag(RegExpFlagKind),
}

/// Typed handler family for pinned QuickJS's Map constructor and prototype
/// surface. Iterator-producing methods retain their exact projection mode in
/// the native identity instead of dispatching by property name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapNativeKind {
    Constructor,
    Species,
    GroupBy,
    Set,
    Get,
    GetOrInsert,
    GetOrInsertComputed,
    Has,
    Delete,
    Clear,
    Size,
    ForEach,
    Iterator(MapIteratorKind),
}

/// Typed handler family for pinned QuickJS's Set constructor, prototype, and
/// set-composition surface. Iterator identities retain their result projection
/// instead of dispatching by the property that exposed the callable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SetNativeKind {
    Constructor,
    Species,
    GroupBy,
    Add,
    Has,
    Delete,
    Clear,
    Size,
    ForEach,
    IsDisjointFrom,
    IsSubsetOf,
    IsSupersetOf,
    Intersection,
    Difference,
    SymmetricDifference,
    Union,
    Iterator(SetIteratorKind),
}

/// Typed handler family for the Promise constructor and its initial surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PromiseNativeKind {
    Constructor,
    Species,
    Then,
    Catch,
    Resolve,
    Reject,
}

/// Selector shared by the paired internal Promise resolving functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PromiseResolvingKind {
    Resolve,
    Reject,
}

/// Runtime-provided callable identities. The enum is stored in heap payloads
/// so native dispatch stays typed and does not rely on function pointers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeFunctionId {
    FunctionPrototype,
    FunctionConstructor(DynamicFunctionKind),
    GeneratorPrototypeResume(GeneratorResumeKind),
    ArrayConstructor,
    ArrayIsArray,
    ArrayFrom,
    ArrayOf,
    ArraySpeciesGetter,
    ArrayPrototypeIterator(ArrayIteratorKind),
    ArrayPrototypeAt,
    ArrayPrototypeWith,
    ArrayPrototypeConcat,
    ArrayPrototypeIteration(ArrayIterationKind),
    ArrayPrototypeReduce(ArrayReduceKind),
    ArrayPrototypeFill,
    ArrayPrototypeFind(ArrayFindKind),
    ArrayPrototypeSearch(ArraySearchKind),
    ArrayPrototypeJoin(ArrayJoinKind),
    ArrayPrototypeToString,
    ArrayPrototypePop(ArrayPopKind),
    ArrayPrototypePush(ArrayPushKind),
    ArrayPrototypeReverse,
    ArrayPrototypeToReversed,
    ArrayPrototypeSort,
    ArrayPrototypeToSorted,
    ArrayPrototypeSlice(ArraySliceKind),
    ArrayPrototypeToSpliced,
    ArrayPrototypeCopyWithin,
    ArrayPrototypeFlatten(ArrayFlattenKind),
    ArrayIteratorNext,
    ThrowTypeError,
    FunctionPrototypeCall,
    FunctionPrototypeApply,
    FunctionPrototypeBind,
    FunctionPrototypeToString,
    FunctionPrototypeHasInstance,
    FunctionPrototypeFileName,
    FunctionPrototypePosition(FunctionDebugPosition),
    ObjectConstructor,
    ObjectCreate,
    ObjectGetPrototypeOf,
    ObjectSetPrototypeOf,
    ObjectDefineProperty,
    ObjectDefineProperties,
    ObjectGetOwnPropertyKeys(ObjectOwnPropertyKeysKind),
    ObjectGroupBy,
    ObjectKeys(ObjectKeysKind),
    ObjectExtensibility(ObjectExtensibilityKind),
    ObjectGetOwnPropertyDescriptor,
    ObjectGetOwnPropertyDescriptors,
    ObjectIs,
    ObjectAssign,
    ObjectIntegrity(ObjectIntegrityKind),
    ObjectFromEntries,
    ObjectHasOwn,
    ObjectPrototypeToString,
    ObjectPrototypeToLocaleString,
    ObjectPrototypeValueOf,
    ObjectPrototypeHasOwnProperty,
    ObjectPrototypeIsPrototypeOf,
    ObjectPrototypePropertyIsEnumerable,
    ObjectPrototypeProtoGetter,
    ObjectPrototypeProtoSetter,
    ObjectPrototypeDefineAccessor(ObjectAccessorKind),
    ObjectPrototypeLookupAccessor(ObjectAccessorKind),
    Json(JsonNativeKind),
    Reflect(ReflectKind),
    Date(DateNativeKind),
    RegExp(RegExpNativeKind),
    Map(MapNativeKind),
    MapIteratorNext,
    Set(SetNativeKind),
    SetIteratorNext,
    Promise(PromiseNativeKind),
    PromiseResolving(PromiseResolvingKind),
    PromiseCapabilityExecutor,
    PrimitiveConstructor(PrimitiveKind),
    StringStatic(StringStaticKind),
    /// QuickJS's test262-only `js_string_codePointRange` helper.
    StringCodePointRange,
    /// qjs-host `print`, installed explicitly by the CLI rather than as an
    /// ECMAScript intrinsic in every Context.
    QjsPrint,
    PrimitivePrototypeToString(PrimitiveKind),
    PrimitivePrototypeValueOf(PrimitiveKind),
    StringPrototypeCharAt(StringCharAtKind),
    StringPrototypeCharCodeAt,
    StringPrototypeConcat,
    StringPrototypeCodePointAt,
    StringPrototypeWellFormed(StringWellFormedKind),
    StringPrototypeIndexOf(StringIndexOfKind),
    StringPrototypeIncludes(StringIncludesKind),
    StringPrototypeReplace(StringReplaceKind),
    StringPrototypeMatch,
    StringPrototypeMatchAll,
    StringPrototypeSearch,
    StringPrototypeSplit,
    MathMinMax(MathMinMaxKind),
    MathUnary(MathUnaryKind),
    MathBinary(MathBinaryKind),
    MathHypot,
    MathRandom,
    MathImul,
    MathClz32,
    MathSumPrecise,
    StringPrototypeSubrange(StringSubrangeKind),
    StringPrototypeRepeat,
    StringPrototypePad(StringPadKind),
    StringPrototypeTrim(StringTrimKind),
    StringPrototypeCase(StringCaseKind),
    StringPrototypeCreateHtml(StringCreateHtmlKind),
    IteratorPrototypeIterator,
    IteratorPrototypeToStringTagGetter,
    IteratorPrototypeToStringTagSetter,
    StringPrototypeIterator,
    StringIteratorNext,
    RegExpStringIteratorNext,
    SymbolRegistry(SymbolRegistryKind),
    SymbolPrototypeDescription,
    BigIntAsN(BigIntAsNKind),
    GlobalEval,
    GlobalNumberParse(NumberParseKind),
    GlobalNumberPredicate(GlobalNumberPredicateKind),
    GlobalUriCodec(GlobalUriCodecKind),
    NumberPredicate(NumberPredicateKind),
    NumberPrototypeFormat(NumberFormatKind),
    ErrorConstructor(ErrorConstructorKind),
    ErrorPrototypeToString,
    ErrorIsError,
    #[cfg(test)]
    ArgumentProbe,
    #[cfg(test)]
    ConstructorProbe,
    #[cfg(test)]
    ConstructorOrFunctionProbe,
    #[cfg(test)]
    ActiveFrameProbe,
}

/// Typed equivalent of QuickJS's magic selector shared by the dynamic
/// Function-family constructors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DynamicFunctionKind {
    Normal,
    Generator,
    Async,
    AsyncGenerator,
}

/// Typed replacement for QuickJS's `GEN_MAGIC_*` selector shared by
/// `%GeneratorPrototype%.next`, `.return`, and `.throw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GeneratorResumeKind {
    Next,
    Return,
    Throw,
}

/// QuickJS primitive wrapper classes which own one realm-local prototype root.
///
/// The complete typed table is present up front, but runtime initialization may
/// leave entries absent until that class reaches a feature-parity milestone.
/// Callers must therefore reject an absent entry instead of falling through to
/// `%Object.prototype%` and silently changing observable behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PrimitiveKind {
    Number,
    String,
    Boolean,
    Symbol,
    BigInt,
}

/// Magic selector shared by `%BigInt%.asUintN` and `%BigInt%.asIntN`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BigIntAsNKind {
    AsUintN,
    AsIntN,
}

/// Static operation selected by QuickJS's `%String%` constructor table.
/// Each entry uses the generic C function protocol, while retaining a typed
/// identity for runtime dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringStaticKind {
    FromCharCode,
    FromCodePoint,
    Raw,
}

/// QuickJS's magic selector shared by `String.prototype.at` and
/// `String.prototype.charAt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringCharAtKind {
    At,
    CharAt,
}

/// Typed selector for the adjacent well-formed UTF-16 methods. QuickJS uses
/// separate C functions; retaining one Rust family keeps the shared scan
/// explicit without changing either function's generic C protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringWellFormedKind {
    IsWellFormed,
    ToWellFormed,
}

/// Direction selected by QuickJS's shared `js_string_indexOf` kernel.
/// `indexOf` clamps a saturated Int32 position and scans forward, whereas
/// `lastIndexOf` applies its distinct floating-point default and scans back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringIndexOfKind {
    IndexOf,
    LastIndexOf,
}

/// QuickJS magic selector shared by `String.prototype.includes`, `endsWith`,
/// and `startsWith`. The release's table publishes them in that order with
/// magic values 0, 2, and 1 respectively.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringIncludesKind {
    Includes,
    EndsWith,
    StartsWith,
}

/// QuickJS magic selector shared by `String.prototype.replace` and
/// `String.prototype.replaceAll`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringReplaceKind {
    Replace,
    ReplaceAll,
}

/// Operation selected by the adjacent `substring`, Annex-B `substr`, and
/// `slice` generic String functions. QuickJS publishes three distinct generic
/// C functions; the selector only shares their UTF-16 subrange machinery in
/// Rust and does not change the native function protocol to generic-magic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringSubrangeKind {
    Substring,
    Substr,
    Slice,
}

/// Direction selected by QuickJS's shared `js_string_pad` generic-magic
/// function. The pinned table passes magic one for `padEnd` and zero for
/// `padStart`; typed variants keep that otherwise implicit contract visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringPadKind {
    End,
    Start,
}

/// Ends selected by QuickJS's shared `js_string_trim` generic-magic function.
/// Its bitmask uses one for the leading end and two for the trailing end;
/// typed variants preserve the exact table magic values without exposing raw
/// integers to runtime dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringTrimKind {
    Both,
    End,
    Start,
}

/// Direction selected by QuickJS's shared `js_string_toLowerCase`
/// generic-magic function. The locale-named methods use the same two magic
/// values and intentionally ignore their locale arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringCaseKind {
    Lower,
    Upper,
}

/// Annex-B operation selected by QuickJS's shared `js_string_CreateHTML`
/// generic-magic function. Each variant preserves one table magic value and
/// its corresponding tag/optional attribute pair without exposing raw
/// integers to runtime dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringCreateHtmlKind {
    Anchor,
    Big,
    Blink,
    Bold,
    Fixed,
    FontColor,
    FontSize,
    Italics,
    Link,
    Small,
    Strike,
    Sub,
    Sup,
}

/// Static selector shared by `%Symbol%.for` and `%Symbol%.keyFor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolRegistryKind {
    For,
    KeyFor,
}

impl PrimitiveKind {
    pub const COUNT: usize = 5;

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Global numeric prefix parsers which are also captured by `%Number%` as
/// identity-preserving `parseInt` and `parseFloat` aliases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumberParseKind {
    ParseInt,
    ParseFloat,
}

/// Coercing global numeric predicates from QuickJS's base-object table.
/// These stay distinct from the non-coercing static `%Number%` predicates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlobalNumberPredicateKind {
    IsNaN,
    IsFinite,
}

/// QuickJS global URI percent codecs and Annex-B escape helpers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlobalUriCodecKind {
    DecodeUri,
    DecodeUriComponent,
    EncodeUri,
    EncodeUriComponent,
    Escape,
    Unescape,
}

/// Non-coercing numeric predicates installed as static `%Number%` methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumberPredicateKind {
    IsNaN,
    IsFinite,
    IsInteger,
    IsSafeInteger,
}

/// Number-specific prototype formatting operations. The ordinary `toString`
/// and `valueOf` methods continue to use the shared primitive selectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumberFormatKind {
    ToExponential,
    ToFixed,
    ToPrecision,
    ToLocaleString,
}

/// Typed replacement for the magic selector shared by QuickJS's function
/// definition-position getter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionDebugPosition {
    Line,
    Column,
}

/// Type-safe replacement for QuickJS's integer magic selector on the shared
/// Error constructor handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorConstructorKind {
    Error,
    Native(NativeErrorKind),
}

/// Typed equivalent of QuickJS's C-function protocol selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeCProto {
    Generic,
    GenericMagic,
    Constructor,
    ConstructorMagic,
    ConstructorOrFunction,
    ConstructorOrFunctionMagic,
    UnaryF64,
    BinaryF64,
    Getter,
    Setter,
    GetterMagic,
    SetterMagic,
    IteratorNext,
}

impl NativeCProto {
    /// QuickJS initializes the mutable constructor bit directly from cproto.
    /// Embedders may change the bit later without changing this protocol.
    #[must_use]
    pub const fn default_is_constructor(self) -> bool {
        matches!(
            self,
            Self::Constructor
                | Self::ConstructorMagic
                | Self::ConstructorOrFunction
                | Self::ConstructorOrFunctionMagic
        )
    }
}

/// Static handler-family metadata. Per-object realm, readable arity and the
/// mutable constructor bit deliberately remain outside this descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeFunctionDescriptor {
    pub cproto: NativeCProto,
}

impl NativeFunctionId {
    #[must_use]
    pub const fn descriptor(self) -> NativeFunctionDescriptor {
        match self {
            Self::FunctionPrototype
            | Self::ThrowTypeError
            | Self::FunctionPrototypeCall
            | Self::FunctionPrototypeApply
            | Self::FunctionPrototypeBind
            | Self::FunctionPrototypeToString
            | Self::FunctionPrototypeHasInstance
            | Self::ObjectCreate
            | Self::ObjectSetPrototypeOf
            | Self::ObjectDefineProperties
            | Self::ObjectGetOwnPropertyKeys(_)
            | Self::ObjectGetOwnPropertyDescriptors
            | Self::ObjectIs
            | Self::ObjectAssign
            | Self::ObjectFromEntries
            | Self::ObjectHasOwn
            | Self::ObjectPrototypeToString
            | Self::ObjectPrototypeToLocaleString
            | Self::ObjectPrototypeValueOf
            | Self::ObjectPrototypeHasOwnProperty
            | Self::ObjectPrototypeIsPrototypeOf
            | Self::ObjectPrototypePropertyIsEnumerable
            | Self::Json(_)
            | Self::Date(
                DateNativeKind::Now
                | DateNativeKind::Parse
                | DateNativeKind::Utc
                | DateNativeKind::TimeValue
                | DateNativeKind::ToPrimitive
                | DateNativeKind::TimezoneOffset
                | DateNativeKind::SetTime
                | DateNativeKind::SetYear
                | DateNativeKind::ToJson,
            )
            | Self::RegExp(
                RegExpNativeKind::Exec
                | RegExpNativeKind::Compile
                | RegExpNativeKind::Test
                | RegExpNativeKind::ToString
                | RegExpNativeKind::Replace
                | RegExpNativeKind::Match
                | RegExpNativeKind::MatchAll
                | RegExpNativeKind::Search
                | RegExpNativeKind::Split,
            )
            | Self::Map(
                MapNativeKind::GroupBy
                | MapNativeKind::Set
                | MapNativeKind::Get
                | MapNativeKind::GetOrInsert
                | MapNativeKind::GetOrInsertComputed
                | MapNativeKind::Has
                | MapNativeKind::Delete
                | MapNativeKind::Clear
                | MapNativeKind::ForEach
                | MapNativeKind::Iterator(_),
            )
            | Self::Set(
                SetNativeKind::GroupBy
                | SetNativeKind::Add
                | SetNativeKind::Has
                | SetNativeKind::Delete
                | SetNativeKind::Clear
                | SetNativeKind::ForEach
                | SetNativeKind::IsDisjointFrom
                | SetNativeKind::IsSubsetOf
                | SetNativeKind::IsSupersetOf
                | SetNativeKind::Intersection
                | SetNativeKind::Difference
                | SetNativeKind::SymmetricDifference
                | SetNativeKind::Union
                | SetNativeKind::Iterator(_),
            )
            | Self::Promise(
                PromiseNativeKind::Then
                | PromiseNativeKind::Catch
                | PromiseNativeKind::Resolve
                | PromiseNativeKind::Reject,
            )
            | Self::PromiseResolving(_)
            | Self::PromiseCapabilityExecutor
            | Self::Reflect(
                ReflectKind::Apply
                | ReflectKind::Construct
                | ReflectKind::DeleteProperty
                | ReflectKind::Get
                | ReflectKind::Has
                | ReflectKind::OwnKeys
                | ReflectKind::Set
                | ReflectKind::SetPrototypeOf,
            )
            | Self::PrimitivePrototypeToString(_)
            | Self::PrimitivePrototypeValueOf(_)
            | Self::StringStatic(_)
            | Self::StringCodePointRange
            | Self::QjsPrint
            | Self::StringPrototypeCharCodeAt
            | Self::StringPrototypeConcat
            | Self::StringPrototypeCodePointAt
            | Self::StringPrototypeWellFormed(_)
            | Self::StringPrototypeSplit
            | Self::MathHypot
            | Self::MathRandom
            | Self::MathImul
            | Self::MathClz32
            | Self::MathSumPrecise
            | Self::StringPrototypeSubrange(_)
            | Self::StringPrototypeRepeat
            | Self::IteratorPrototypeIterator
            | Self::StringPrototypeIterator
            | Self::ArrayIsArray
            | Self::ArrayFrom
            | Self::ArrayOf
            | Self::ArrayPrototypeIterator(_)
            | Self::ArrayPrototypeAt
            | Self::ArrayPrototypeWith
            | Self::ArrayPrototypeConcat
            | Self::ArrayPrototypeFill
            | Self::ArrayPrototypeSearch(_)
            | Self::ArrayPrototypeToString
            | Self::ArrayPrototypeReverse
            | Self::ArrayPrototypeToReversed
            | Self::ArrayPrototypeSort
            | Self::ArrayPrototypeToSorted
            | Self::ArrayPrototypeToSpliced
            | Self::ArrayPrototypeCopyWithin
            | Self::SymbolRegistry(_)
            | Self::GlobalEval
            | Self::GlobalNumberParse(_)
            | Self::GlobalNumberPredicate(_)
            | Self::NumberPredicate(_)
            | Self::NumberPrototypeFormat(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::Generic,
            },
            Self::ObjectGetPrototypeOf
            | Self::ObjectDefineProperty
            | Self::ObjectGroupBy
            | Self::ObjectKeys(_)
            | Self::ObjectExtensibility(_)
            | Self::ObjectGetOwnPropertyDescriptor
            | Self::ObjectIntegrity(_)
            | Self::ObjectPrototypeDefineAccessor(_)
            | Self::ObjectPrototypeLookupAccessor(_)
            | Self::Date(
                DateNativeKind::String(_)
                | DateNativeKind::GetField(_)
                | DateNativeKind::SetField(_),
            )
            | Self::Reflect(
                ReflectKind::DefineProperty
                | ReflectKind::GetOwnPropertyDescriptor
                | ReflectKind::GetPrototypeOf
                | ReflectKind::IsExtensible
                | ReflectKind::PreventExtensions,
            )
            | Self::MathMinMax(_)
            | Self::StringPrototypeCharAt(_)
            | Self::StringPrototypeIndexOf(_)
            | Self::StringPrototypeIncludes(_)
            | Self::StringPrototypeReplace(_)
            | Self::StringPrototypeMatch
            | Self::StringPrototypeMatchAll
            | Self::StringPrototypeSearch
            | Self::StringPrototypePad(_)
            | Self::StringPrototypeTrim(_)
            | Self::StringPrototypeCase(_)
            | Self::StringPrototypeCreateHtml(_)
            | Self::ArrayPrototypeFind(_)
            | Self::ArrayPrototypeIteration(_)
            | Self::ArrayPrototypeReduce(_)
            | Self::ArrayPrototypeFlatten(_)
            | Self::ArrayPrototypeJoin(_)
            | Self::ArrayPrototypePop(_)
            | Self::ArrayPrototypePush(_)
            | Self::ArrayPrototypeSlice(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::GenericMagic,
            },
            Self::GlobalUriCodec(
                GlobalUriCodecKind::DecodeUri
                | GlobalUriCodecKind::DecodeUriComponent
                | GlobalUriCodecKind::EncodeUri
                | GlobalUriCodecKind::EncodeUriComponent,
            ) => NativeFunctionDescriptor {
                cproto: NativeCProto::GenericMagic,
            },
            Self::BigIntAsN(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::GenericMagic,
            },
            Self::GlobalUriCodec(GlobalUriCodecKind::Escape | GlobalUriCodecKind::Unescape) => {
                NativeFunctionDescriptor {
                    cproto: NativeCProto::Generic,
                }
            }
            Self::FunctionConstructor(_) | Self::PrimitiveConstructor(_) => {
                NativeFunctionDescriptor {
                    cproto: NativeCProto::ConstructorOrFunctionMagic,
                }
            }
            Self::ArrayConstructor
            | Self::ObjectConstructor
            | Self::Date(DateNativeKind::Constructor)
            | Self::RegExp(RegExpNativeKind::Constructor) => NativeFunctionDescriptor {
                cproto: NativeCProto::ConstructorOrFunction,
            },
            Self::Map(MapNativeKind::Constructor)
            | Self::Set(SetNativeKind::Constructor)
            | Self::Promise(PromiseNativeKind::Constructor) => NativeFunctionDescriptor {
                cproto: NativeCProto::Constructor,
            },
            Self::FunctionPrototypeFileName
            | Self::ObjectPrototypeProtoGetter
            | Self::RegExp(
                RegExpNativeKind::Species | RegExpNativeKind::Source | RegExpNativeKind::Flags,
            )
            | Self::Map(MapNativeKind::Species | MapNativeKind::Size)
            | Self::Set(SetNativeKind::Species | SetNativeKind::Size)
            | Self::Promise(PromiseNativeKind::Species) => NativeFunctionDescriptor {
                cproto: NativeCProto::Getter,
            },
            Self::ObjectPrototypeProtoSetter => NativeFunctionDescriptor {
                cproto: NativeCProto::Setter,
            },
            Self::SymbolPrototypeDescription | Self::ArraySpeciesGetter => {
                NativeFunctionDescriptor {
                    cproto: NativeCProto::Getter,
                }
            }
            Self::IteratorPrototypeToStringTagGetter => NativeFunctionDescriptor {
                cproto: NativeCProto::Getter,
            },
            Self::IteratorPrototypeToStringTagSetter => NativeFunctionDescriptor {
                cproto: NativeCProto::Setter,
            },
            Self::StringIteratorNext
            | Self::RegExpStringIteratorNext
            | Self::ArrayIteratorNext
            | Self::MapIteratorNext
            | Self::SetIteratorNext
            | Self::GeneratorPrototypeResume(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::IteratorNext,
            },
            Self::FunctionPrototypePosition(_) | Self::RegExp(RegExpNativeKind::Flag(_)) => {
                NativeFunctionDescriptor {
                    cproto: NativeCProto::GetterMagic,
                }
            }
            Self::ErrorConstructor(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::ConstructorOrFunctionMagic,
            },
            Self::ErrorPrototypeToString | Self::ErrorIsError => NativeFunctionDescriptor {
                cproto: NativeCProto::Generic,
            },
            Self::MathUnary(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::UnaryF64,
            },
            Self::MathBinary(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::BinaryF64,
            },
            #[cfg(test)]
            Self::ArgumentProbe => NativeFunctionDescriptor {
                cproto: NativeCProto::Generic,
            },
            #[cfg(test)]
            Self::ConstructorProbe => NativeFunctionDescriptor {
                cproto: NativeCProto::Constructor,
            },
            #[cfg(test)]
            Self::ConstructorOrFunctionProbe => NativeFunctionDescriptor {
                cproto: NativeCProto::ConstructorOrFunction,
            },
            #[cfg(test)]
            Self::ActiveFrameProbe => NativeFunctionDescriptor {
                cproto: NativeCProto::Generic,
            },
        }
    }
}

/// Per-object native callable metadata. The own `length` property remains an
/// independent ordinary property and may be modified without affecting
/// `min_readable_args`, just as in QuickJS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeFunctionData {
    pub target: NativeFunctionId,
    /// `None` exists only during `%Function.prototype%` realm bootstrap.
    pub realm: Option<ContextId>,
    pub min_readable_args: u8,
}

/// Runtime-owned ordinary object record.
///
/// The shape entries and slots are parallel arrays and must have identical
/// lengths and storage kinds.  Allocation validates that invariant.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectData {
    pub shape: ShapeId,
    pub slots: Vec<PropertySlot>,
    /// QuickJS's hidden `JS_CLASS_PRIVATE` brand stored on a private method's
    /// HomeObject. The object owns one atom reference independently from any
    /// receiver marker using the same private atom in its shape.
    pub private_brand_home: Option<Atom>,
    pub extensible: bool,
    pub immutable_prototype: bool,
    pub is_constructor: bool,
    pub kind: ObjectKind,
    pub payload: ObjectPayload,
}

impl ObjectData {
    /// Construct an ordinary extensible object with a mutable prototype.
    #[must_use]
    pub const fn ordinary(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Ordinary,
            payload: ObjectPayload::Ordinary,
        }
    }

    /// Construct one Raw JSON branded object with ordinary internal methods.
    #[must_use]
    pub const fn raw_json(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Ordinary,
            payload: ObjectPayload::RawJson,
        }
    }

    /// Construct one genuine Array exotic object. The caller supplies the
    /// validated `length`-first layout used by QuickJS's initial Array shape.
    #[must_use]
    pub const fn array(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Array,
            payload: ObjectPayload::Array { fast_len: Some(0) },
        }
    }

    /// Construct one mapped or unmapped Arguments exotic object. The caller
    /// installs the exact actual-argument prefix and the class-specific
    /// `length`, `callee`, and `@@iterator` properties after allocation.
    #[must_use]
    pub const fn arguments(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        mapped: bool,
        fast_len: u32,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Arguments,
            payload: ObjectPayload::Arguments {
                mapped,
                fast_len: Some(fast_len),
            },
        }
    }

    /// Construct a branded Array Iterator at index zero.
    #[must_use]
    pub const fn array_iterator(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        object: ObjectId,
        kind: ArrayIteratorKind,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::ArrayIterator,
            payload: ObjectPayload::ArrayIterator {
                object: Some(object),
                next_index: 0,
                kind,
            },
        }
    }

    /// Construct one hidden QuickJS-compatible for-in enumeration object.
    #[must_use]
    pub const fn for_in_iterator(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        data: ForInIteratorData,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::ForInIterator,
            payload: ObjectPayload::ForInIterator(data),
        }
    }

    /// Construct one extensible primitive wrapper object with its validated
    /// internal primitive data slot.
    #[must_use]
    pub const fn primitive(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        data: PrimitiveObjectData,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Primitive,
            payload: ObjectPayload::Primitive(data),
        }
    }

    /// Construct one genuine Date object with an internal millisecond value.
    /// The runtime is responsible for applying TimeClip before publication;
    /// NaN remains valid because it represents an invalid Date.
    #[must_use]
    pub const fn date(shape: ShapeId, slots: Vec<PropertySlot>, value: f64) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Date,
            payload: ObjectPayload::Date(value),
        }
    }

    /// Construct a branded RegExp object before its pattern is compiled.
    /// This mirrors QuickJS's derived-constructor order, in which object
    /// allocation can succeed before compilation reports a SyntaxError.
    #[must_use]
    pub const fn regexp(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::RegExp,
            payload: ObjectPayload::RegExp(RegExpObjectData::Uninitialized),
        }
    }

    /// Construct a branded RegExp object whose program is already compiled.
    #[must_use]
    pub fn compiled_regexp(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        pattern: JsString,
        program: Rc<CompiledRegExp>,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::RegExp,
            payload: ObjectPayload::RegExp(RegExpObjectData::Compiled { pattern, program }),
        }
    }

    /// Construct a branded RegExp String Iterator over one species-created
    /// matcher. The matcher and input string remain retained after completion;
    /// only finalization releases them in pinned QuickJS.
    #[must_use]
    pub const fn regexp_string_iterator(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        regexp: ObjectId,
        string: JsString,
        global: bool,
        full_unicode: bool,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::RegExpStringIterator,
            payload: ObjectPayload::RegExpStringIterator {
                regexp,
                string,
                global,
                full_unicode,
                done: false,
            },
        }
    }

    /// Construct one empty genuine Map object. Stable records are appended by
    /// [`Heap::map_insert_record`] after key equality has been resolved by the
    /// runtime's SameValueZero logic.
    #[must_use]
    pub const fn map(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Map,
            payload: ObjectPayload::Map {
                records: Vec::new(),
                size: 0,
            },
        }
    }

    /// Construct a branded Map Iterator at stable record index zero.
    #[must_use]
    pub const fn map_iterator(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        object: ObjectId,
        kind: MapIteratorKind,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::MapIterator,
            payload: ObjectPayload::MapIterator {
                object: Some(object),
                next_index: 0,
                kind,
            },
        }
    }

    /// Construct one empty genuine Set object. Stable records are appended by
    /// [`Heap::set_insert_record`] after the runtime resolves SameValueZero
    /// equality. The shared record value slot remains `undefined`.
    #[must_use]
    pub const fn set(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Set,
            payload: ObjectPayload::Set {
                records: Vec::new(),
                size: 0,
            },
        }
    }

    /// Construct a branded Set Iterator at stable record index zero.
    #[must_use]
    pub const fn set_iterator(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        object: ObjectId,
        kind: SetIteratorKind,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::SetIterator,
            payload: ObjectPayload::SetIterator {
                object: Some(object),
                next_index: 0,
                kind,
            },
        }
    }

    /// Construct a realm global object with QuickJS's hidden unresolved-name
    /// VarRef table.
    #[must_use]
    pub const fn global_object(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        uninitialized_vars: ObjectId,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::GlobalObject,
            payload: ObjectPayload::GlobalObject { uninitialized_vars },
        }
    }

    /// Construct an Error-class object. Its ordinary `name`/`message`
    /// properties remain in the shape/slot arrays; the payload preserves the
    /// native class tag used by `Error.isError`.
    #[must_use]
    pub const fn error(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Error,
            payload: ObjectPayload::Error,
        }
    }

    /// Construct a branded String Iterator at code-unit index zero.
    #[must_use]
    pub const fn string_iterator(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        string: JsString,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::StringIterator,
            payload: ObjectPayload::StringIterator {
                string: Some(string),
                next_index: 0,
            },
        }
    }

    /// Construct a non-constructable runtime-provided function object.
    #[must_use]
    pub(crate) const fn native_function(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        target: NativeFunctionId,
        min_readable_args: u8,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: target.descriptor().cproto.default_is_constructor(),
            kind: ObjectKind::NativeFunction,
            payload: ObjectPayload::NativeFunction {
                data: NativeFunctionData {
                    target,
                    realm: None,
                    min_readable_args,
                },
                internal: None,
            },
        }
    }

    /// Construct a native callable whose defining realm is already live.
    #[must_use]
    pub(crate) const fn bound_native_function(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        target: NativeFunctionId,
        realm: ContextId,
        min_readable_args: u8,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: target.descriptor().cproto.default_is_constructor(),
            kind: ObjectKind::NativeFunction,
            payload: ObjectPayload::NativeFunction {
                data: NativeFunctionData {
                    target,
                    realm: Some(realm),
                    min_readable_args,
                },
                internal: None,
            },
        }
    }

    /// Construct a realm-bound internal native callable with typed hidden
    /// capture data.  Allocation retains every raw edge in `internal`; the
    /// caller transfers no public runtime-owning wrapper into the heap.
    #[must_use]
    pub(crate) const fn bound_internal_native_function(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        target: NativeFunctionId,
        realm: ContextId,
        min_readable_args: u8,
        internal: InternalCallableData,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::NativeFunction,
            payload: ObjectPayload::NativeFunction {
                data: NativeFunctionData {
                    target,
                    realm: Some(realm),
                    min_readable_args,
                },
                internal: Some(internal),
            },
        }
    }

    /// Construct a QuickJS-style bound function. Its ordinary `length` and
    /// `name` properties are installed by the runtime after allocation; the
    /// class payload owns the target, bound receiver and argument vector.
    #[must_use]
    pub(crate) const fn bound_function(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        target: ObjectId,
        this_value: RawValue,
        arguments: Rc<[RawValue]>,
        is_constructor: bool,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor,
            kind: ObjectKind::BoundFunction,
            payload: ObjectPayload::BoundFunction {
                target,
                this_value,
                arguments,
            },
        }
    }

    /// Construct an ordinary bytecode-function object.
    #[must_use]
    pub const fn bytecode_function(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        bytecode: FunctionBytecodeId,
        home_object: Option<ObjectId>,
        is_constructor: bool,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor,
            kind: ObjectKind::BytecodeFunction,
            payload: ObjectPayload::BytecodeFunction {
                bytecode,
                home_object,
                class_instance_initializer: None,
                class_static_initializer_started: false,
                closure_slots: Vec::new(),
            },
        }
    }

    /// Construct a bytecode-function object whose closure slots own the given
    /// captured-variable cells. Repeated identities are intentional: each
    /// slot contributes one strong reference, as in QuickJS.
    #[must_use]
    pub const fn bytecode_function_with_closures(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        bytecode: FunctionBytecodeId,
        home_object: Option<ObjectId>,
        closure_slots: Vec<VarRefId>,
        is_constructor: bool,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor,
            kind: ObjectKind::BytecodeFunction,
            payload: ObjectPayload::BytecodeFunction {
                bytecode,
                home_object,
                class_instance_initializer: None,
                class_static_initializer_started: false,
                closure_slots,
            },
        }
    }

    /// Construct a branded synchronous generator in its initial suspended
    /// state. The complete activation is retained as heap-visible raw edges.
    #[must_use]
    pub fn generator(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        activation: GeneratorActivationData,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Generator,
            payload: ObjectPayload::Generator {
                state: GeneratorState::SuspendedStart,
                activation: Some(Box::new(activation)),
            },
        }
    }

    /// Construct one genuine pending Promise.  Its result slot starts at
    /// `undefined` and owns no reactions until `PerformPromiseThen` appends
    /// them through [`Heap::promise_add_reactions`].
    #[must_use]
    pub const fn promise(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Promise,
            payload: ObjectPayload::Promise(PromiseData {
                state: PromiseState::Pending,
                result: RawValue::Undefined,
                fulfill_reactions: Vec::new(),
                reject_reactions: Vec::new(),
                is_handled: false,
            }),
        }
    }
}

/// Resources finalized by a release, mutation, or collection operation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeapCleanup {
    pub finalized_objects: usize,
    pub finalized_shapes: usize,
    pub finalized_var_refs: usize,
    pub finalized_contexts: usize,
    pub finalized_function_bytecodes: usize,
    /// Finalized shape identities for O(1) weak-cache unlinking.
    pub finalized_shape_ids: Vec<ShapeId>,
    /// Owned non-GC atom edges detached from shapes and symbol values.
    pub atoms: Vec<Atom>,
}

impl HeapCleanup {
    fn merge(&mut self, mut other: Self) {
        self.finalized_objects = self
            .finalized_objects
            .saturating_add(other.finalized_objects);
        self.finalized_shapes = self.finalized_shapes.saturating_add(other.finalized_shapes);
        self.finalized_var_refs = self
            .finalized_var_refs
            .saturating_add(other.finalized_var_refs);
        self.finalized_contexts = self
            .finalized_contexts
            .saturating_add(other.finalized_contexts);
        self.finalized_function_bytecodes = self
            .finalized_function_bytecodes
            .saturating_add(other.finalized_function_bytecodes);
        self.finalized_shape_ids
            .append(&mut other.finalized_shape_ids);
        self.atoms.append(&mut other.atoms);
    }
}

/// Statistics and caller-owned cleanup produced by one cycle collection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GcStats {
    pub examined_nodes: usize,
    pub external_root_nodes: usize,
    pub candidate_nodes: usize,
    pub cleanup: HeapCleanup,
}

/// Current arena population, split by lifecycle state and node kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeapCounts {
    pub object_nodes: usize,
    pub shape_nodes: usize,
    pub var_ref_nodes: usize,
    pub context_nodes: usize,
    pub function_bytecode_nodes: usize,
    pub initializing: usize,
    pub live: usize,
    pub zero_queued: usize,
    pub finalizing: usize,
    pub zombies: usize,
    pub vacant: usize,
    pub retired: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RawId {
    Object(ObjectId),
    Shape(ShapeId),
    VarRef(VarRefId),
    Context(ContextId),
    FunctionBytecode(FunctionBytecodeId),
}

impl RawId {
    const fn kind(self) -> HeapNodeKind {
        match self {
            Self::Object(_) => HeapNodeKind::Object,
            Self::Shape(_) => HeapNodeKind::Shape,
            Self::VarRef(_) => HeapNodeKind::VarRef,
            Self::Context(_) => HeapNodeKind::Context,
            Self::FunctionBytecode(_) => HeapNodeKind::FunctionBytecode,
        }
    }

    const fn index(self) -> u32 {
        match self {
            Self::Object(id) => id.index,
            Self::Shape(id) => id.index,
            Self::VarRef(id) => id.index,
            Self::Context(id) => id.index,
            Self::FunctionBytecode(id) => id.index,
        }
    }

    const fn generation(self) -> u32 {
        match self {
            Self::Object(id) => id.generation,
            Self::Shape(id) => id.generation,
            Self::VarRef(id) => id.generation,
            Self::Context(id) => id.generation,
            Self::FunctionBytecode(id) => id.generation,
        }
    }
}

// Context nodes intentionally stay inline in the generational arena: boxing
// only this variant would add a second allocator/failure boundary to realm
// publication and collection without shrinking any live Context graph.
#[allow(clippy::large_enum_variant)]
enum NodeData {
    Object(ObjectData),
    Shape(Shape),
    VarRef(VarRefData),
    Context(ContextData),
    FunctionBytecode(FunctionBytecodeData),
}

impl NodeData {
    const fn kind(&self) -> HeapNodeKind {
        match self {
            Self::Object(_) => HeapNodeKind::Object,
            Self::Shape(_) => HeapNodeKind::Shape,
            Self::VarRef(_) => HeapNodeKind::VarRef,
            Self::Context(_) => HeapNodeKind::Context,
            Self::FunctionBytecode(_) => HeapNodeKind::FunctionBytecode,
        }
    }

    fn edges(&self) -> Vec<RawId> {
        match self {
            Self::Object(object) => object_edges(object),
            Self::Shape(shape) => shape_edges(shape),
            Self::VarRef(var_ref) => var_ref_edges(var_ref),
            Self::Context(context) => context_edges(context),
            Self::FunctionBytecode(bytecode) => function_bytecode_edges(bytecode),
        }
    }
}

struct Node {
    strong: u32,
    data: NodeData,
}

enum SlotState {
    Initializing { kind: HeapNodeKind, strong: u32 },
    Live(Node),
    ZeroQueued(Node),
    Finalizing(Node),
    Zombie { kind: HeapNodeKind, strong: u32 },
    Vacant,
    Retired,
}

impl SlotState {
    const fn public_state(&self) -> HeapSlotState {
        match self {
            Self::Initializing { .. } => HeapSlotState::Initializing,
            Self::Live(_) => HeapSlotState::Live,
            Self::ZeroQueued(_) => HeapSlotState::ZeroQueued,
            Self::Finalizing(_) => HeapSlotState::Finalizing,
            Self::Zombie { .. } => HeapSlotState::Zombie,
            Self::Vacant => HeapSlotState::Vacant,
            Self::Retired => HeapSlotState::Retired,
        }
    }

    const fn kind(&self) -> Option<HeapNodeKind> {
        match self {
            Self::Initializing { kind, .. } | Self::Zombie { kind, .. } => Some(*kind),
            Self::Live(node) | Self::ZeroQueued(node) | Self::Finalizing(node) => {
                Some(node.data.kind())
            }
            Self::Vacant | Self::Retired => None,
        }
    }

    const fn strong(&self) -> Option<u32> {
        match self {
            Self::Initializing { strong, .. } | Self::Zombie { strong, .. } => Some(*strong),
            Self::Live(node) | Self::ZeroQueued(node) | Self::Finalizing(node) => Some(node.strong),
            Self::Vacant | Self::Retired => None,
        }
    }
}

struct ArenaSlot {
    generation: u32,
    state: SlotState,
}

/// Runtime-local object and shape arena.
///
/// A `Heap` is deliberately not internally synchronized.  The enclosing
/// runtime chooses its single-threaded ownership boundary, as QuickJS does.
pub struct Heap {
    slots: Vec<ArenaSlot>,
    free: Vec<u32>,
    zero_queue: VecDeque<RawId>,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            zero_queue: VecDeque::new(),
        }
    }

    /// Allocate and publish a shape, retaining its prototype edge.
    ///
    /// The caller owns one returned shape reference and must eventually call
    /// [`Heap::release_shape`].  Atom references are owned by the caller until
    /// this succeeds, then by the shape until returned as cleanup.
    pub fn allocate_shape(&mut self, shape: Shape) -> Result<ShapeId, HeapError> {
        let (index, generation) = self.reserve(HeapNodeKind::Shape)?;
        let id = ShapeId { index, generation };
        let edges = shape_edges(&shape);

        if let Err(error) = self.retain_edges_transactionally(&edges) {
            self.abort_initializing(index)?;
            return Err(error);
        }

        self.publish(index, NodeData::Shape(shape))?;
        Ok(id)
    }

    /// Allocate and publish an object, retaining its shape and property edges.
    ///
    /// The caller owns one returned object reference and must eventually call
    /// [`Heap::release_object`].
    pub fn allocate_object(&mut self, object: ObjectData) -> Result<ObjectId, HeapError> {
        if matches!(
            &object.payload,
            ObjectPayload::NativeFunction {
                data: NativeFunctionData { realm: None, .. },
                ..
            }
        ) {
            return Err(HeapError::Invariant(
                "an unbound native function may only be allocated during realm bootstrap",
            ));
        }
        self.allocate_object_inner(object)
    }

    /// Allocate the one provisional native callable needed to bootstrap a
    /// realm. The caller must synchronously finish it with
    /// [`Self::attach_native_function_realm`] before exposing it.
    pub(crate) fn allocate_bootstrap_native_function(
        &mut self,
        object: ObjectData,
    ) -> Result<ObjectId, HeapError> {
        if !matches!(
            &object.payload,
            ObjectPayload::NativeFunction {
                data: NativeFunctionData {
                    target: NativeFunctionId::FunctionPrototype,
                    realm: None,
                    ..
                },
                ..
            }
        ) {
            return Err(HeapError::Invariant(
                "bootstrap native-function allocation requires an unbound Function.prototype",
            ));
        }
        self.allocate_object_inner(object)
    }

    fn allocate_object_inner(&mut self, object: ObjectData) -> Result<ObjectId, HeapError> {
        self.validate_object_layout(&object)?;
        let (index, generation) = self.reserve(HeapNodeKind::Object)?;
        let id = ObjectId { index, generation };
        let edges = object_edges(&object);

        if let Err(error) = self.retain_edges_transactionally(&edges) {
            self.abort_initializing(index)?;
            return Err(error);
        }

        self.publish(index, NodeData::Object(object))?;
        Ok(id)
    }

    /// Allocate and publish a realm/context node, retaining all realm roots.
    /// Symbol atoms in `intrinsics` transfer to the node on success.
    pub fn allocate_context(&mut self, context: ContextData) -> Result<ContextId, HeapError> {
        if context
            .intrinsics
            .iter()
            .any(|value| matches!(value, RawValue::Private(_)))
        {
            return Err(HeapError::Invariant(
                "private-name identity escaped into a realm intrinsic",
            ));
        }
        let (index, generation) = self.reserve(HeapNodeKind::Context)?;
        let id = ContextId { index, generation };
        let edges = context_edges(&context);

        if let Err(error) = self.retain_edges_transactionally(&edges) {
            self.abort_initializing(index)?;
            return Err(error);
        }

        self.publish(index, NodeData::Context(context))?;
        Ok(id)
    }

    /// Finish two-phase native-function bootstrap by installing its defining
    /// realm as an owned GC edge.
    ///
    /// Realm construction is necessarily cyclic: the Context owns
    /// `%Function.prototype%`, while that native callable owns its defining
    /// Context. The object is therefore allocated provisionally, the Context
    /// is published, and this operation closes the cycle transactionally.
    pub(crate) fn attach_native_function_realm(
        &mut self,
        object: ObjectId,
        realm: ContextId,
    ) -> Result<(), HeapError> {
        self.context(realm)?;
        match &self.object(object)?.payload {
            ObjectPayload::NativeFunction {
                data: NativeFunctionData { realm: None, .. },
                ..
            } => {}
            ObjectPayload::NativeFunction {
                data: NativeFunctionData { realm: Some(_), .. },
                ..
            } => {
                return Err(HeapError::Invariant(
                    "native function already has a defining realm",
                ));
            }
            ObjectPayload::Ordinary
            | ObjectPayload::RawJson
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(_)
            | ObjectPayload::Date(_)
            | ObjectPayload::RegExp(_)
            | ObjectPayload::RegExpStringIterator { .. }
            | ObjectPayload::Map { .. }
            | ObjectPayload::MapIterator { .. }
            | ObjectPayload::Set { .. }
            | ObjectPayload::SetIterator { .. }
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. }
            | ObjectPayload::Generator { .. }
            | ObjectPayload::Promise(_) => {
                return Err(HeapError::Invariant(
                    "attempted to attach a native realm to a non-native function",
                ));
            }
        }

        self.retain_raw(RawId::Context(realm), 1)?;
        let ObjectPayload::NativeFunction { data, .. } = &mut self.object_mut(object)?.payload
        else {
            unreachable!("native-function payload was validated before retaining its realm")
        };
        data.realm = Some(realm);
        Ok(())
    }

    /// Publish the realm's shared frozen `%ThrowTypeError%` root after the
    /// context exists. Its native callable already owns the context, so this
    /// deliberately closes the same collectable realm cycle as QuickJS.
    pub(crate) fn attach_throw_type_error(
        &mut self,
        realm: ContextId,
        thrower: ObjectId,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.throw_type_error.is_some() {
            return Err(HeapError::Invariant(
                "context already has a %ThrowTypeError% root",
            ));
        }
        if !matches!(
            self.object(thrower)?.payload,
            ObjectPayload::NativeFunction {
                data: NativeFunctionData {
                    target: NativeFunctionId::ThrowTypeError,
                    realm: Some(target_realm),
                    ..
                },
                ..
            } if target_realm == realm
        ) {
            return Err(HeapError::Invariant(
                "%ThrowTypeError% root is not the realm's poison native function",
            ));
        }

        self.retain_raw(RawId::Object(thrower), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining the thrower")
        };
        context.throw_type_error = Some(thrower);
        Ok(())
    }

    /// Cache the original realm-local `Array.prototype.values` callable.
    /// This is a distinct Context root because the public prototype property
    /// is writable and configurable while arguments creation must keep using
    /// the bootstrap identity.
    pub(crate) fn attach_array_prototype_values(
        &mut self,
        realm: ContextId,
        values: ObjectId,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.array_prototype_values.is_some() {
            return Err(HeapError::Invariant(
                "context already has an Array.prototype.values cache root",
            ));
        }
        if !matches!(
            self.object(values)?.payload,
            ObjectPayload::NativeFunction {
                data: NativeFunctionData {
                    target: NativeFunctionId::ArrayPrototypeIterator(ArrayIteratorKind::Value),
                    realm: Some(target_realm),
                    ..
                },
                ..
            } if target_realm == realm
        ) {
            return Err(HeapError::Invariant(
                "Array.prototype.values cache is not the realm's values native",
            ));
        }

        self.retain_raw(RawId::Object(values), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Array values")
        };
        context.array_prototype_values = Some(values);
        Ok(())
    }

    /// Publish the realm's `%Function%` root after its native callable and
    /// constructor/prototype cycle have been fully initialized.
    pub(crate) fn attach_function_constructor(
        &mut self,
        realm: ContextId,
        constructor: ObjectId,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.function_constructor.is_some() {
            return Err(HeapError::Invariant(
                "context already has a Function constructor root",
            ));
        }
        let constructor_object = self.object(constructor)?;
        if !constructor_object.is_constructor
            || !matches!(
                constructor_object.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Normal),
                        realm: Some(target_realm),
                        ..
                    },
                    ..
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "Function constructor root is not the realm's Function native",
            ));
        }

        self.retain_raw(RawId::Object(constructor), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Function")
        };
        context.function_constructor = Some(constructor);
        Ok(())
    }

    /// Cache the realm's original `%eval%` callable independently from its
    /// mutable global property, matching `JSContext.eval_obj`.
    pub(crate) fn attach_eval_function(
        &mut self,
        realm: ContextId,
        function: ObjectId,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.eval_function.is_some() {
            return Err(HeapError::Invariant(
                "context already has an eval function root",
            ));
        }
        let function_object = self.object(function)?;
        if function_object.is_constructor
            || !matches!(
                function_object.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::GlobalEval,
                        realm: Some(target_realm),
                        ..
                    },
                    ..
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "eval function root is not the realm's non-constructor eval native",
            ));
        }

        self.retain_raw(RawId::Object(function), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining eval")
        };
        context.eval_function = Some(function);
        Ok(())
    }

    /// Publish the realm's `%Array%` root after its native callable and
    /// constructor/prototype cycle have been initialized.
    pub(crate) fn attach_array_constructor(
        &mut self,
        realm: ContextId,
        constructor: ObjectId,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.array_constructor.is_some() {
            return Err(HeapError::Invariant(
                "context already has an Array constructor root",
            ));
        }
        let constructor_object = self.object(constructor)?;
        if !constructor_object.is_constructor
            || !matches!(
                constructor_object.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::ArrayConstructor,
                        realm: Some(target_realm),
                        ..
                    },
                    ..
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "Array constructor root is not the realm's Array native",
            ));
        }

        self.retain_raw(RawId::Object(constructor), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Array")
        };
        context.array_constructor = Some(constructor);
        Ok(())
    }

    /// Atomically publish the realm's RegExp intrinsic roots after its native
    /// constructor, ordinary prototype, RegExp String Iterator prototype, and
    /// canonical instance shape exist.
    ///
    /// `last_index_atom` is validation-only: the shape already owns its atom
    /// edge. Passing it explicitly lets this heap layer prove that the sole
    /// instance slot really is `lastIndex` without depending on `AtomTable`
    /// string lookup. The four GC edges are retained as one transaction
    /// before the Context is mutated.
    pub(crate) fn attach_regexp_intrinsics(
        &mut self,
        realm: ContextId,
        regexp: RegExpRealmData,
        last_index_atom: Atom,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.regexp.is_some() {
            return Err(HeapError::Invariant(
                "context already has RegExp intrinsic roots",
            ));
        }
        let iterator_prototype = context.iterator_prototype;

        let constructor = self.object(regexp.constructor)?;
        if !constructor.is_constructor
            || !matches!(
                constructor.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::RegExp(RegExpNativeKind::Constructor),
                        realm: Some(target_realm),
                        ..
                    },
                    ..
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "RegExp constructor root is not the realm's RegExp native",
            ));
        }

        let prototype = self.object(regexp.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "RegExp prototype root is not an ordinary object",
            ));
        }

        let string_iterator_prototype = self.object(regexp.string_iterator_prototype)?;
        if string_iterator_prototype.kind != ObjectKind::Ordinary
            || !matches!(string_iterator_prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "RegExp String Iterator prototype root is not an ordinary object",
            ));
        }
        if self.shape(string_iterator_prototype.shape)?.prototype() != Some(iterator_prototype) {
            return Err(HeapError::Invariant(
                "RegExp String Iterator prototype does not inherit from the realm's Iterator prototype",
            ));
        }

        let object_shape = self.shape(regexp.object_shape)?;
        if object_shape.prototype() != Some(regexp.prototype) {
            return Err(HeapError::Invariant(
                "RegExp object shape does not inherit from the realm's RegExp prototype",
            ));
        }
        let [last_index] = object_shape.entries() else {
            return Err(HeapError::Invariant(
                "RegExp object shape does not contain exactly one lastIndex property",
            ));
        };
        if last_index.atom != last_index_atom
            || last_index.flags != PropertyFlags::data(true, false, false)
        {
            return Err(HeapError::Invariant(
                "RegExp object shape has an invalid lastIndex property",
            ));
        }

        let edges = [
            RawId::Object(regexp.prototype),
            RawId::Object(regexp.constructor),
            RawId::Object(regexp.string_iterator_prototype),
            RawId::Shape(regexp.object_shape),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining RegExp roots")
        };
        context.regexp = Some(regexp);
        Ok(())
    }

    /// Atomically publish the realm's Map constructor, ordinary prototype,
    /// and Map Iterator prototype roots after all three have been initialized.
    pub(crate) fn attach_map_intrinsics(
        &mut self,
        realm: ContextId,
        map: MapRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.map.is_some() {
            return Err(HeapError::Invariant(
                "context already has Map intrinsic roots",
            ));
        }
        let iterator_prototype = context.iterator_prototype;

        let prototype = self.object(map.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "Map prototype root is not an ordinary object",
            ));
        }

        let map_iterator_prototype = self.object(map.iterator_prototype)?;
        if map_iterator_prototype.kind != ObjectKind::Ordinary
            || !matches!(map_iterator_prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "Map Iterator prototype root is not an ordinary object",
            ));
        }
        if self.shape(map_iterator_prototype.shape)?.prototype() != Some(iterator_prototype) {
            return Err(HeapError::Invariant(
                "Map Iterator prototype does not inherit from the realm's Iterator prototype",
            ));
        }

        let edges = [
            RawId::Object(map.prototype),
            RawId::Object(map.iterator_prototype),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Map roots")
        };
        context.map = Some(map);
        Ok(())
    }

    /// Atomically publish the realm's ordinary Set prototype and Set Iterator
    /// prototype roots after both have been initialized.
    pub(crate) fn attach_set_intrinsics(
        &mut self,
        realm: ContextId,
        set: SetRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.set.is_some() {
            return Err(HeapError::Invariant(
                "context already has Set intrinsic roots",
            ));
        }
        let iterator_prototype = context.iterator_prototype;

        let prototype = self.object(set.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "Set prototype root is not an ordinary object",
            ));
        }

        let set_iterator_prototype = self.object(set.iterator_prototype)?;
        if set_iterator_prototype.kind != ObjectKind::Ordinary
            || !matches!(set_iterator_prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "Set Iterator prototype root is not an ordinary object",
            ));
        }
        if self.shape(set_iterator_prototype.shape)?.prototype() != Some(iterator_prototype) {
            return Err(HeapError::Invariant(
                "Set Iterator prototype does not inherit from the realm's Iterator prototype",
            ));
        }

        let edges = [
            RawId::Object(set.prototype),
            RawId::Object(set.iterator_prototype),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Set roots")
        };
        context.set = Some(set);
        Ok(())
    }

    /// Atomically publish the realm's Promise constructor and ordinary
    /// prototype after their public constructor/prototype links exist.
    pub(crate) fn attach_promise_intrinsics(
        &mut self,
        realm: ContextId,
        promise: PromiseRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.promise.is_some() {
            return Err(HeapError::Invariant(
                "context already has Promise intrinsic roots",
            ));
        }
        let object_prototype = context.object_prototype;

        let constructor = self.object(promise.constructor)?;
        if !constructor.is_constructor
            || !matches!(
                constructor.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::Promise(PromiseNativeKind::Constructor),
                        realm: Some(target_realm),
                        ..
                    },
                    internal: None,
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "Promise constructor root is not the realm's Promise native",
            ));
        }

        let prototype = self.object(promise.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
            || self.shape(prototype.shape)?.prototype() != Some(object_prototype)
        {
            return Err(HeapError::Invariant(
                "Promise prototype is not an ordinary child of Object.prototype",
            ));
        }

        let edges = [
            RawId::Object(promise.prototype),
            RawId::Object(promise.constructor),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Promise roots")
        };
        context.promise = Some(promise);
        Ok(())
    }

    /// Atomically publish the two realm-local synchronous-generator class
    /// prototypes after their property graph has been initialized.
    pub(crate) fn attach_generator_intrinsics(
        &mut self,
        realm: ContextId,
        generator: GeneratorRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.generator.is_some() {
            return Err(HeapError::Invariant(
                "context already has Generator intrinsic roots",
            ));
        }
        let iterator_prototype = context.iterator_prototype;
        let function_prototype = context.function_prototype;

        let prototype = self.object(generator.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
            || self.shape(prototype.shape)?.prototype() != Some(iterator_prototype)
        {
            return Err(HeapError::Invariant(
                "Generator prototype does not inherit from the realm's Iterator prototype",
            ));
        }

        let generator_function_prototype = self.object(generator.function_prototype)?;
        if generator_function_prototype.kind != ObjectKind::Ordinary
            || !matches!(
                generator_function_prototype.payload,
                ObjectPayload::Ordinary
            )
            || self.shape(generator_function_prototype.shape)?.prototype()
                != Some(function_prototype)
        {
            return Err(HeapError::Invariant(
                "GeneratorFunction prototype does not inherit from Function.prototype",
            ));
        }

        let edges = [
            RawId::Object(generator.prototype),
            RawId::Object(generator.function_prototype),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Generator roots")
        };
        context.generator = Some(generator);
        Ok(())
    }

    /// Allocate and publish immutable function bytecode, retaining its realm
    /// and every GC edge in its constant pool. `auxiliary_atoms` and symbol
    /// constants transfer to the node on success. No arena slot is reserved
    /// until both metadata authentication and shared bytecode verification
    /// succeed.
    pub fn allocate_function_bytecode(
        &mut self,
        bytecode: FunctionBytecodeData,
    ) -> Result<FunctionBytecodeId, HeapError> {
        if bytecode
            .constants
            .iter()
            .any(|constant| matches!(constant, BytecodeConstant::Value(RawValue::Private(_))))
        {
            return Err(HeapError::Invariant(
                "private-name identity escaped into a bytecode constant",
            ));
        }
        if bytecode.metadata.local_count > MAX_LOCAL_SLOTS {
            return Err(HeapError::Invariant(
                "bytecode local count exceeds QuickJS JS_MAX_LOCAL_VARS",
            ));
        }
        let parameter_initializer_locals = bytecode
            .local_definitions
            .iter()
            .map(|definition| definition.is_parameter_initializer)
            .collect::<Vec<_>>();
        let parameter_body_pc = validate_parameter_bytecode_layout(
            &bytecode.metadata,
            &bytecode.code,
            &parameter_initializer_locals,
            bytecode.parameter_environment.as_ref(),
        )
        .map_err(HeapError::Invariant)?;
        let initial_yields = bytecode
            .code
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| {
                matches!(instruction, Instruction::InitialYield).then_some(pc)
            })
            .collect::<Vec<_>>();
        let has_generator_only_instruction = bytecode.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::Yield
                    | Instruction::YieldStar
                    | Instruction::IteratorStart
                    | Instruction::IteratorNext
                    | Instruction::IteratorCall(_)
                    | Instruction::IteratorCheckObject
                    | Instruction::ThrowIteratorMissingThrow
            )
        });
        match bytecode.metadata.function_kind {
            FunctionKind::Generator => {
                if initial_yields.len() != 1
                    || !bytecode.metadata.has_prototype
                    || bytecode.metadata.constructor_kind != ConstructorKind::None
                    || bytecode.metadata.class_initializer_kind.is_some()
                    || parameter_body_pc.is_some_and(|body_pc| initial_yields[0] < body_pc)
                {
                    return Err(HeapError::Invariant(
                        "generator bytecode has invalid suspension metadata",
                    ));
                }
            }
            FunctionKind::Normal | FunctionKind::Async | FunctionKind::AsyncGenerator => {
                if !initial_yields.is_empty() || has_generator_only_instruction {
                    return Err(HeapError::Invariant(
                        "non-generator bytecode contains a generator suspension opcode",
                    ));
                }
            }
        }
        if bytecode
            .metadata
            .function_name_local
            .is_some_and(|index| index >= bytecode.metadata.local_count)
        {
            return Err(HeapError::Invariant(
                "function-name local is outside bytecode local slots",
            ));
        }
        for (local, message) in [
            (
                bytecode.metadata.derived_this_local,
                "derived this local is outside bytecode local slots",
            ),
            (
                bytecode.metadata.active_function_local,
                "active-function local is outside bytecode local slots",
            ),
        ] {
            if local.is_some_and(|index| index >= bytecode.metadata.local_count) {
                return Err(HeapError::Invariant(message));
            }
        }
        if bytecode
            .metadata
            .eval_variable_object_local
            .is_some_and(|index| index >= bytecode.metadata.local_count)
        {
            return Err(HeapError::Invariant(
                "eval variable-object local is outside bytecode local slots",
            ));
        }
        if bytecode.metadata.eval_variable_object_local.is_some()
            && bytecode.metadata.eval_variable_object_local == bytecode.metadata.function_name_local
        {
            return Err(HeapError::Invariant(
                "eval variable-object and function-name locals overlap",
            ));
        }
        let arg_eval_variable_object_local = bytecode
            .parameter_environment
            .as_ref()
            .and_then(|layout| layout.arg_eval_variable_object_local);
        if arg_eval_variable_object_local
            .is_some_and(|index| index >= bytecode.metadata.local_count)
        {
            return Err(HeapError::Invariant(
                "parameter eval variable-object local is outside bytecode local slots",
            ));
        }
        if let Some(index) = arg_eval_variable_object_local
            && (bytecode.metadata.eval_variable_object_local == Some(index)
                || bytecode.metadata.function_name_local == Some(index))
        {
            return Err(HeapError::Invariant(
                "parameter eval variable-object local overlaps another private local",
            ));
        }
        let private_locals = [
            bytecode.metadata.function_name_local,
            bytecode.metadata.eval_variable_object_local,
            arg_eval_variable_object_local,
            bytecode.metadata.derived_this_local,
            bytecode.metadata.active_function_local,
        ];
        for (index, local) in private_locals.iter().enumerate() {
            if local.is_some()
                && private_locals[..index]
                    .iter()
                    .any(|earlier| earlier == local)
            {
                return Err(HeapError::Invariant("authenticated private locals overlap"));
            }
        }
        if bytecode.argument_definitions.len() != usize::from(bytecode.metadata.argument_count) {
            return Err(HeapError::Invariant(
                "argument definition count does not match bytecode metadata",
            ));
        }
        if bytecode.local_definitions.len() != usize::from(bytecode.metadata.local_count) {
            return Err(HeapError::Invariant(
                "local definition count does not match bytecode metadata",
            ));
        }
        validate_published_private_elements(self, &bytecode)?;
        let unnamed_arguments = bytecode
            .argument_definitions
            .iter()
            .map(|definition| definition.name.is_none())
            .collect::<Vec<_>>();
        let lexical_locals = bytecode
            .local_definitions
            .iter()
            .map(|definition| definition.is_lexical)
            .collect::<Vec<_>>();
        let const_locals = bytecode
            .local_definitions
            .iter()
            .map(|definition| definition.is_const)
            .collect::<Vec<_>>();
        validate_derived_constructor_bytecode_layout(
            &bytecode.metadata,
            &bytecode.code,
            &lexical_locals,
            &const_locals,
            &bytecode.closure_variables,
        )
        .map_err(HeapError::Invariant)?;
        validate_class_initializer_bytecode_layout(&bytecode.metadata, &bytecode.code)
            .map_err(HeapError::Invariant)?;
        let pattern_body_pc = bytecode
            .metadata
            .parameter_pattern_end
            .and_then(|marker| usize::try_from(marker).ok())
            .and_then(|marker| marker.checked_add(1));
        validate_parameter_initializer_scope_layout(
            &bytecode.metadata,
            &bytecode.code,
            parameter_body_pc.or(pattern_body_pc),
            &lexical_locals,
            &parameter_initializer_locals,
        )
        .map_err(HeapError::Invariant)?;
        validate_pattern_parameter_bytecode_layout(
            &bytecode.metadata,
            &bytecode.code,
            &unnamed_arguments,
            &lexical_locals,
            &parameter_initializer_locals,
            bytecode.parameter_environment.as_ref(),
        )
        .map_err(HeapError::Invariant)?;
        let parameter_initializer_capture_locals = parameter_initializer_visible_locals(
            &bytecode.metadata,
            &bytecode.code,
            parameter_body_pc,
            &parameter_initializer_locals,
            bytecode.parameter_environment.as_ref(),
        )
        .map_err(HeapError::Invariant)?;
        validate_eval_environment_phase_layout(
            &bytecode.eval_environments,
            EvalEnvironmentPhaseContext {
                metadata: &bytecode.metadata,
                code: &bytecode.code,
                parameter_body_pc,
                pattern_body_pc,
                lexical_locals: &lexical_locals,
                parameter_initializer_locals: &parameter_initializer_locals,
                parameter_initializer_visible_locals: parameter_initializer_capture_locals
                    .as_deref(),
                parameter_environment: bytecode.parameter_environment.as_ref(),
            },
        )
        .map_err(HeapError::Invariant)?;
        for definition in bytecode.argument_definitions.iter() {
            if definition.kind != ClosureVariableKind::Normal
                || definition.is_lexical
                || definition.is_const
                || definition.is_parameter_initializer
            {
                return Err(HeapError::Invariant(
                    "argument definition is not an ordinary mutable binding",
                ));
            }
        }
        if let Some(layout) = bytecode.parameter_environment.as_ref() {
            let parameter_definitions = bytecode
                .local_definitions
                .iter()
                .take(usize::from(
                    bytecode.metadata.parameter_environment_local_count,
                ))
                .collect::<Vec<_>>();
            for (index, local) in parameter_definitions.iter().enumerate() {
                if local.kind != ClosureVariableKind::Normal
                    || !local.is_lexical
                    || local.is_const
                    || local.is_parameter_initializer
                    || local.name.is_none()
                    || parameter_definitions[..index]
                        .iter()
                        .any(|earlier| earlier.name == local.name)
                {
                    return Err(HeapError::Invariant(
                        "parameter environment cell definition is not authenticated",
                    ));
                }
            }
            let mut mapped_arguments = vec![false; bytecode.argument_definitions.len()];
            for cell in layout.argument_cells.iter() {
                mapped_arguments[usize::from(cell.argument)] = true;
                let argument = &bytecode.argument_definitions[usize::from(cell.argument)];
                let local = &bytecode.local_definitions[usize::from(cell.parameter_local)];
                if argument.name.is_none() || argument.name != local.name {
                    return Err(HeapError::Invariant(
                        "parameter argument cell name disagrees with its physical argument",
                    ));
                }
            }
            if bytecode
                .argument_definitions
                .iter()
                .zip(mapped_arguments)
                .any(|(argument, mapped)| argument.name.is_some() != mapped)
            {
                return Err(HeapError::Invariant(
                    "parameter argument-cell map is not one-to-one with named arguments",
                ));
            }
            for copy in layout.pattern_copies.iter() {
                let source = &bytecode.local_definitions[usize::from(copy.parameter_local)];
                let target = &bytecode.local_definitions[usize::from(copy.body_local)];
                if target.kind != ClosureVariableKind::Normal
                    || target.is_lexical
                    || target.is_const
                    || source.is_parameter_initializer
                    || target.is_parameter_initializer
                    || source.name != target.name
                {
                    return Err(HeapError::Invariant(
                        "parameter pattern copy definitions are not same-name lexical-to-root storage",
                    ));
                }
            }
            if let Some(index) = layout.synthetic_arguments_local {
                let definition = &bytecode.local_definitions[usize::from(index)];
                if definition.kind != ClosureVariableKind::Normal
                    || !definition.is_lexical
                    || definition.is_const
                    || definition.is_parameter_initializer
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "synthetic parameter arguments definition is not authenticated",
                    ));
                }
            }
        }
        for (index, definition) in bytecode.local_definitions.iter().enumerate() {
            let is_function_name =
                bytecode.metadata.function_name_local == u16::try_from(index).ok();
            let is_derived_this = bytecode.metadata.derived_this_local == u16::try_from(index).ok();
            let is_active_function =
                bytecode.metadata.active_function_local == u16::try_from(index).ok();
            let is_eval_variable_object =
                bytecode.metadata.eval_variable_object_local == u16::try_from(index).ok();
            let is_arg_eval_variable_object =
                arg_eval_variable_object_local == u16::try_from(index).ok();
            if is_function_name {
                if definition.kind != ClosureVariableKind::FunctionName
                    || definition.is_lexical
                    || definition.is_const != bytecode.metadata.strict
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "function-name definition disagrees with bytecode metadata",
                    ));
                }
            } else if is_derived_this {
                if definition.kind != ClosureVariableKind::Normal
                    || !definition.is_lexical
                    || definition.is_const
                    || definition.is_parameter_initializer
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "derived this definition disagrees with bytecode metadata",
                    ));
                }
            } else if is_active_function {
                if definition.kind != ClosureVariableKind::Normal
                    || definition.is_lexical
                    || definition.is_const
                    || definition.is_parameter_initializer
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "active-function definition disagrees with bytecode metadata",
                    ));
                }
            } else if is_eval_variable_object {
                if definition.kind != ClosureVariableKind::EvalVariableObject
                    || definition.is_lexical
                    || definition.is_const
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "eval variable-object definition disagrees with bytecode metadata",
                    ));
                }
            } else if is_arg_eval_variable_object {
                if definition.kind != ClosureVariableKind::ArgEvalVariableObject
                    || definition.is_lexical
                    || definition.is_const
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "parameter eval variable-object definition disagrees with bytecode layout",
                    ));
                }
            } else if definition.kind == ClosureVariableKind::WithObject {
                if bytecode.metadata.strict
                    || definition.is_lexical
                    || definition.is_const
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "strict or malformed bytecode contains a with-object local",
                    ));
                }
            } else if definition.kind != ClosureVariableKind::Normal
                && !definition.kind.is_private()
            {
                return Err(HeapError::Invariant(
                    "ordinary local definition uses a non-local binding kind",
                ));
            } else if definition.is_const && !definition.is_lexical {
                return Err(HeapError::Invariant(
                    "a const local definition must also be lexical",
                ));
            }
        }
        if bytecode.closure_variables.len() != usize::from(bytecode.metadata.closure_count) {
            return Err(HeapError::Invariant(
                "function closure descriptor count does not match its bytecode metadata",
            ));
        }
        let mut owned_name_atoms = HashMap::<Atom, usize>::new();
        for atom in bytecode.auxiliary_atoms.iter().copied() {
            *owned_name_atoms.entry(atom).or_default() += 1;
        }
        if let Some(debug) = &bytecode.debug {
            if debug.filename.is_null() {
                return Err(HeapError::Invariant(
                    "bytecode debug filename is the null atom",
                ));
            }
            let Some(count) = owned_name_atoms.get_mut(&debug.filename) else {
                return Err(HeapError::Invariant(
                    "debug filename atom is not owned by bytecode metadata",
                ));
            };
            if *count == 0 {
                return Err(HeapError::Invariant(
                    "debug filename atom ownership multiplicity is too small",
                ));
            }
            *count -= 1;
            if debug
                .source
                .as_deref()
                .is_some_and(|source| std::str::from_utf8(source).is_err())
            {
                return Err(HeapError::Invariant(
                    "bytecode debug source is not valid UTF-8",
                ));
            }
            if let Some(table) = &debug.pc2line {
                if table.definition.line == u32::MAX || table.definition.column == u32::MAX {
                    return Err(HeapError::Invariant(
                        "bytecode debug definition cannot be represented one-based",
                    ));
                }
                let mut previous_pc = None;
                for entry in &table.entries {
                    if usize::try_from(entry.pc)
                        .ok()
                        .is_none_or(|pc| pc >= bytecode.code.len())
                    {
                        return Err(HeapError::Invariant(
                            "bytecode debug PC is outside the instruction stream",
                        ));
                    }
                    if previous_pc.is_some_and(|previous| entry.pc < previous) {
                        return Err(HeapError::Invariant("bytecode debug PCs are not ordered"));
                    }
                    if entry.position.line == u32::MAX || entry.position.column == u32::MAX {
                        return Err(HeapError::Invariant(
                            "bytecode debug position cannot be represented one-based",
                        ));
                    }
                    previous_pc = Some(entry.pc);
                }
            }
        }
        for definition in bytecode
            .argument_definitions
            .iter()
            .chain(bytecode.local_definitions.iter())
        {
            let Some(atom) = definition.name else {
                continue;
            };
            let Some(count) = owned_name_atoms.get_mut(&atom) else {
                return Err(HeapError::Invariant(
                    "variable-definition name atom is not owned by bytecode metadata",
                ));
            };
            if *count == 0 {
                return Err(HeapError::Invariant(
                    "variable-definition name atom ownership multiplicity is too small",
                ));
            }
            *count -= 1;
        }
        let mut global_declaration_names = HashMap::new();
        for descriptor in bytecode.closure_variables.iter().copied() {
            if matches!(descriptor.source, ClosureSource::EvalEnvironment(_))
                && bytecode.metadata.eval_kind != EvalKind::Direct
            {
                return Err(HeapError::Invariant(
                    "eval-environment closure escaped a direct-eval root",
                ));
            }
            if descriptor.kind == ClosureVariableKind::GlobalFunction
                && (descriptor.is_lexical || descriptor.is_const)
            {
                return Err(HeapError::Invariant(
                    "global function declaration descriptor has lexical metadata",
                ));
            }
            if descriptor.kind.is_eval_variable_object()
                && (descriptor.is_lexical
                    || descriptor.is_const
                    || !matches!(
                        descriptor.source,
                        ClosureSource::ParentLocal(_)
                            | ClosureSource::ParentClosure(_)
                            | ClosureSource::EvalEnvironment(_)
                    ))
            {
                return Err(HeapError::Invariant(
                    "eval variable-object descriptor has invalid binding metadata",
                ));
            }
            if descriptor.kind == ClosureVariableKind::WithObject
                && (descriptor.is_lexical
                    || descriptor.is_const
                    || !matches!(
                        descriptor.source,
                        ClosureSource::ParentLocal(_)
                            | ClosureSource::ParentClosure(_)
                            | ClosureSource::EvalEnvironment(_)
                    ))
            {
                return Err(HeapError::Invariant(
                    "with-object descriptor has invalid binding metadata",
                ));
            }
            if descriptor.is_const
                && !descriptor.is_lexical
                && descriptor.kind != ClosureVariableKind::FunctionName
            {
                return Err(HeapError::Invariant(
                    "a const closure descriptor must also be lexical",
                ));
            }
            if (descriptor.source == ClosureSource::GlobalDeclaration
                && !matches!(
                    descriptor.kind,
                    ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
                ))
                || (descriptor.source == ClosureSource::Global
                    && descriptor.kind != ClosureVariableKind::Normal)
                || (matches!(descriptor.source, ClosureSource::ParentGlobal(_))
                    && !matches!(
                        descriptor.kind,
                        ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
                    ))
            {
                return Err(HeapError::Invariant(
                    "global declaration descriptor has non-global binding metadata",
                ));
            }
            if descriptor.kind == ClosureVariableKind::GlobalFunction
                && !matches!(
                    descriptor.source,
                    ClosureSource::GlobalDeclaration | ClosureSource::ParentGlobal(_)
                )
            {
                return Err(HeapError::Invariant(
                    "global function binding kind escaped a declaration relay",
                ));
            }
            let requires_name = matches!(
                descriptor.source,
                ClosureSource::GlobalDeclaration
                    | ClosureSource::Global
                    | ClosureSource::ParentGlobal(_)
                    | ClosureSource::EvalEnvironment(_)
            ) || matches!(
                descriptor.kind,
                ClosureVariableKind::FunctionName
                    | ClosureVariableKind::EvalVariableObject
                    | ClosureVariableKind::ArgEvalVariableObject
                    | ClosureVariableKind::WithObject
            ) || descriptor.kind.is_private();
            let allows_name = requires_name
                || descriptor.is_lexical
                || matches!(
                    descriptor.source,
                    ClosureSource::ParentLocal(_)
                        | ClosureSource::ParentArgument(_)
                        | ClosureSource::ParentClosure(_)
                );
            if descriptor.source == ClosureSource::GlobalDeclaration
                && let ClosureVariableName::Atom(atom) = descriptor.name
            {
                match global_declaration_names.entry(atom) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert((descriptor.is_lexical, descriptor.is_lexical));
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        let (first_is_lexical, seen_lexical) = *entry.get();
                        if first_is_lexical
                            && seen_lexical
                            && (descriptor.is_lexical
                                || descriptor.kind != ClosureVariableKind::GlobalFunction)
                        {
                            return Err(HeapError::Invariant(
                                "duplicate lexical global declaration descriptor name",
                            ));
                        }
                        // A first sloppy Annex B normal record masks every
                        // later same-name declaration in QuickJS's conflict
                        // lookup, including repeated lexical and var records.
                        // A first lexical remains stricter.
                        if descriptor.is_lexical {
                            entry.get_mut().1 = true;
                        }
                    }
                }
            }
            if matches!(
                descriptor.source,
                ClosureSource::GlobalDeclaration
                    | ClosureSource::Global
                    | ClosureSource::ParentGlobal(_)
            ) && !matches!(
                descriptor.kind,
                ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
            ) {
                return Err(HeapError::Invariant(
                    "published global closure descriptor has a non-global binding kind",
                ));
            }
            match descriptor.name {
                ClosureVariableName::Atom(atom) if allows_name => {
                    let Some(count) = owned_name_atoms.get_mut(&atom) else {
                        return Err(HeapError::Invariant(
                            "closure name atom is not owned by bytecode metadata",
                        ));
                    };
                    if *count == 0 {
                        return Err(HeapError::Invariant(
                            "closure name atom ownership multiplicity is too small",
                        ));
                    }
                    *count -= 1;
                }
                ClosureVariableName::None if !requires_name => {}
                ClosureVariableName::Constant(_) => {
                    return Err(HeapError::Invariant(
                        "published closure descriptor retained an unlinked name constant",
                    ));
                }
                ClosureVariableName::None | ClosureVariableName::Atom(_) => {
                    return Err(HeapError::Invariant(
                        "published closure descriptor name does not match its binding kind",
                    ));
                }
            }
        }
        for environment in bytecode.eval_environments.iter() {
            let first_function_anchor = environment
                .scopes
                .iter()
                .position(|scope| {
                    matches!(
                        scope.kind,
                        EvalScopeKind::FunctionRoot | EvalScopeKind::Parameter
                    )
                })
                .and_then(|scope| u16::try_from(scope).ok())
                .ok_or(HeapError::Invariant(
                    "eval environment contains no representable current function anchor",
                ))?;
            match environment.variable_environment {
                EvalVariableEnvironment::Global => {
                    let current_body_is_program = first_function_anchor
                        .checked_sub(1)
                        .and_then(|scope| environment.scopes.get(usize::from(scope)))
                        .is_some_and(|scope| scope.kind == EvalScopeKind::ProgramBody);
                    if !current_body_is_program
                        || (environment.caller_strict
                            && bytecode.metadata.eval_kind != EvalKind::None)
                    {
                        return Err(HeapError::Invariant(
                            "global eval variable environment escaped an authored Script root",
                        ));
                    }
                }
                EvalVariableEnvironment::StrictLocal(scope) if environment.caller_strict => {
                    if scope != first_function_anchor {
                        return Err(HeapError::Invariant(
                            "strict eval variable environment selected the wrong current function segment",
                        ));
                    }
                    let current_body_is_program = first_function_anchor
                        .checked_sub(1)
                        .and_then(|scope| environment.scopes.get(usize::from(scope)))
                        .is_some_and(|scope| scope.kind == EvalScopeKind::ProgramBody);
                    if current_body_is_program && bytecode.metadata.eval_kind == EvalKind::None {
                        return Err(HeapError::Invariant(
                            "authored Script eval environment used a non-canonical strict-local target",
                        ));
                    }
                    let Some(scope) = environment.scopes.get(usize::from(scope)) else {
                        return Err(HeapError::Invariant(
                            "strict eval variable-environment scope is out of bounds",
                        ));
                    };
                    if !matches!(
                        scope.kind,
                        EvalScopeKind::FunctionRoot | EvalScopeKind::Parameter
                    ) {
                        return Err(HeapError::Invariant(
                            "strict eval variable environment has the wrong function segment anchor",
                        ));
                    }
                }
                EvalVariableEnvironment::VariableObject { scope, source }
                    if !environment.caller_strict =>
                {
                    let target_matches_function_segment =
                        if bytecode.metadata.eval_kind == EvalKind::None {
                            scope == first_function_anchor
                                && matches!(source, EvalBindingSource::Local(_))
                        } else {
                            bytecode.metadata.eval_kind == EvalKind::Direct
                                && scope > first_function_anchor
                                && matches!(source, EvalBindingSource::Closure(_))
                        };
                    if !target_matches_function_segment {
                        return Err(HeapError::Invariant(
                            "eval variable object selected the wrong current function segment",
                        ));
                    }
                    let Some(scope) = environment.scopes.get(usize::from(scope)) else {
                        return Err(HeapError::Invariant(
                            "eval variable-object scope is out of bounds",
                        ));
                    };
                    let expected_kind = match scope.kind {
                        EvalScopeKind::FunctionRoot => ClosureVariableKind::EvalVariableObject,
                        EvalScopeKind::Parameter => ClosureVariableKind::ArgEvalVariableObject,
                        _ => {
                            return Err(HeapError::Invariant(
                                "eval variable object has the wrong function segment scope",
                            ));
                        }
                    };
                    if matches!(source, EvalBindingSource::Argument(_))
                        || scope
                            .bindings
                            .iter()
                            .filter(|binding| {
                                binding.source == source && binding.kind == expected_kind
                            })
                            .count()
                            != 1
                    {
                        return Err(HeapError::Invariant(
                            "eval variable-object target is not exact",
                        ));
                    }
                }
                EvalVariableEnvironment::StrictLocal(_)
                | EvalVariableEnvironment::VariableObject { .. } => {
                    return Err(HeapError::Invariant(
                        "eval variable environment disagrees with caller strictness",
                    ));
                }
            }
            for scope in environment.scopes.iter() {
                if scope.kind == EvalScopeKind::With && scope.bindings.len() != 1 {
                    return Err(HeapError::Invariant(
                        "eval with scope does not contain exactly one object binding",
                    ));
                }
                for binding in &scope.bindings {
                    if binding.name.is_null() {
                        return Err(HeapError::Invariant("eval binding name is the null atom"));
                    }
                    let source_name_matches = match binding.source {
                        EvalBindingSource::Local(index) => bytecode
                            .local_definitions
                            .get(usize::from(index))
                            .is_some_and(|definition| definition.name == Some(binding.name)),
                        EvalBindingSource::Argument(index) => bytecode
                            .argument_definitions
                            .get(usize::from(index))
                            .is_some_and(|definition| definition.name == Some(binding.name)),
                        EvalBindingSource::Closure(index) => bytecode
                            .closure_variables
                            .get(usize::from(index))
                            .is_some_and(|descriptor| {
                                descriptor.name == ClosureVariableName::Atom(binding.name)
                            }),
                    };
                    if !source_name_matches {
                        return Err(HeapError::Invariant(
                            "eval binding name atom disagrees with its source metadata",
                        ));
                    }
                    if (binding.is_catch_parameter && scope.kind != EvalScopeKind::Catch)
                        || (binding.is_catch_parameter
                            && (!binding.is_lexical
                                || binding.is_const
                                || binding.kind != ClosureVariableKind::Normal))
                    {
                        return Err(HeapError::Invariant(
                            "eval catch binding metadata disagrees with its scope",
                        ));
                    }
                    if (binding.kind == ClosureVariableKind::WithObject)
                        != (scope.kind == EvalScopeKind::With)
                        || (binding.kind == ClosureVariableKind::WithObject
                            && (binding.is_lexical
                                || binding.is_const
                                || binding.is_catch_parameter
                                || matches!(binding.source, EvalBindingSource::Argument(_))))
                    {
                        return Err(HeapError::Invariant(
                            "eval with-object binding metadata disagrees with its scope",
                        ));
                    }
                    if binding.kind.is_eval_variable_object() {
                        let role_allowed = match scope.kind {
                            EvalScopeKind::FunctionRoot => true,
                            EvalScopeKind::Parameter => {
                                binding.kind == ClosureVariableKind::ArgEvalVariableObject
                            }
                            _ => false,
                        };
                        if !role_allowed
                            || binding.is_lexical
                            || binding.is_const
                            || binding.is_catch_parameter
                        {
                            return Err(HeapError::Invariant(
                                "eval variable-object binding has invalid metadata",
                            ));
                        }
                        let authenticated = match binding.source {
                            EvalBindingSource::Local(index) => {
                                let expected = match binding.kind {
                                    ClosureVariableKind::EvalVariableObject => {
                                        bytecode.metadata.eval_variable_object_local
                                    }
                                    ClosureVariableKind::ArgEvalVariableObject => {
                                        arg_eval_variable_object_local
                                    }
                                    _ => {
                                        unreachable!("eval variable-object role was checked above")
                                    }
                                };
                                expected == Some(index)
                                    && bytecode
                                        .local_definitions
                                        .get(usize::from(index))
                                        .is_some_and(|definition| definition.kind == binding.kind)
                            }
                            EvalBindingSource::Closure(index) => bytecode
                                .closure_variables
                                .get(usize::from(index))
                                .is_some_and(|descriptor| descriptor.kind == binding.kind),
                            EvalBindingSource::Argument(_) => false,
                        };
                        if !authenticated {
                            return Err(HeapError::Invariant(
                                "eval variable-object binding source is not authenticated",
                            ));
                        }
                    }
                    if binding.kind == ClosureVariableKind::WithObject {
                        let authenticated = match binding.source {
                            EvalBindingSource::Local(index) => bytecode
                                .local_definitions
                                .get(usize::from(index))
                                .is_some_and(|definition| {
                                    definition.kind == ClosureVariableKind::WithObject
                                        && !definition.is_lexical
                                        && !definition.is_const
                                }),
                            EvalBindingSource::Closure(index) => bytecode
                                .closure_variables
                                .get(usize::from(index))
                                .is_some_and(|descriptor| {
                                    descriptor.kind == ClosureVariableKind::WithObject
                                        && !descriptor.is_lexical
                                        && !descriptor.is_const
                                }),
                            EvalBindingSource::Argument(_) => false,
                        };
                        if !authenticated {
                            return Err(HeapError::Invariant(
                                "eval with-object binding source is not authenticated",
                            ));
                        }
                    }
                    let Some(count) = owned_name_atoms.get_mut(&binding.name) else {
                        return Err(HeapError::Invariant(
                            "eval binding name atom is not owned by bytecode metadata",
                        ));
                    };
                    if *count == 0 {
                        return Err(HeapError::Invariant(
                            "eval binding name atom ownership multiplicity is too small",
                        ));
                    }
                    *count -= 1;
                }
            }
        }
        crate::bytecode::verify_parts(
            &bytecode.code,
            bytecode.constants.len(),
            bytecode.metadata.max_stack,
        )
        .map_err(|_| HeapError::Invariant("function bytecode failed generic verification"))?;
        let (index, generation) = self.reserve(HeapNodeKind::FunctionBytecode)?;
        let id = FunctionBytecodeId { index, generation };
        let edges = function_bytecode_edges(&bytecode);

        if let Err(error) = self.retain_edges_transactionally(&edges) {
            self.abort_initializing(index)?;
            return Err(error);
        }

        self.publish(index, NodeData::FunctionBytecode(bytecode))?;
        Ok(id)
    }

    /// Allocate a captured-variable cell and transfer ownership of its value
    /// to the heap. The returned reference is normally owned by the active
    /// frame; closure objects retain the same `VarRefId` when published.
    pub fn allocate_var_ref(&mut self, var_ref: VarRefData) -> Result<VarRefId, HeapError> {
        validate_var_ref_payload(&var_ref)?;
        let (index, generation) = self.reserve(HeapNodeKind::VarRef)?;
        let id = VarRefId { index, generation };
        let edges = var_ref_edges(&var_ref);

        if let Err(error) = self.retain_edges_transactionally(&edges) {
            self.abort_initializing(index)?;
            return Err(error);
        }

        self.publish(index, NodeData::VarRef(var_ref))?;
        Ok(id)
    }

    /// Duplicate one externally owned object reference.
    pub fn retain_object(&mut self, id: ObjectId) -> Result<(), HeapError> {
        self.retain_raw(RawId::Object(id), 1)
    }

    /// Duplicate one externally owned shape reference.
    pub fn retain_shape(&mut self, id: ShapeId) -> Result<(), HeapError> {
        self.retain_raw(RawId::Shape(id), 1)
    }

    /// Duplicate one frame or closure ownership of a captured-variable cell.
    pub fn retain_var_ref(&mut self, id: VarRefId) -> Result<(), HeapError> {
        self.retain_raw(RawId::VarRef(id), 1)
    }

    /// Duplicate one externally owned context reference.
    pub fn retain_context(&mut self, id: ContextId) -> Result<(), HeapError> {
        self.retain_raw(RawId::Context(id), 1)
    }

    /// Duplicate one externally owned function-bytecode reference.
    pub fn retain_function_bytecode(&mut self, id: FunctionBytecodeId) -> Result<(), HeapError> {
        self.retain_raw(RawId::FunctionBytecode(id), 1)
    }

    /// Release one object reference and iteratively drain zero-reference nodes.
    pub fn release_object(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        self.release_and_drain(RawId::Object(id))
    }

    /// Release one shape reference and iteratively drain zero-reference nodes.
    pub fn release_shape(&mut self, id: ShapeId) -> Result<HeapCleanup, HeapError> {
        self.release_and_drain(RawId::Shape(id))
    }

    /// Release one frame or closure ownership of a captured-variable cell.
    pub fn release_var_ref(&mut self, id: VarRefId) -> Result<HeapCleanup, HeapError> {
        self.release_and_drain(RawId::VarRef(id))
    }

    /// Release one context reference and iteratively drain cascading nodes.
    pub fn release_context(&mut self, id: ContextId) -> Result<HeapCleanup, HeapError> {
        self.release_and_drain(RawId::Context(id))
    }

    /// Release one function-bytecode reference and iteratively drain nodes.
    pub fn release_function_bytecode(
        &mut self,
        id: FunctionBytecodeId,
    ) -> Result<HeapCleanup, HeapError> {
        self.release_and_drain(RawId::FunctionBytecode(id))
    }

    /// Read one live object record.
    pub fn object(&self, id: ObjectId) -> Result<&ObjectData, HeapError> {
        match self.live_node(RawId::Object(id))?.data {
            NodeData::Object(ref object) => Ok(object),
            NodeData::Shape(_)
            | NodeData::VarRef(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed object lookup reached another node payload",
            )),
        }
    }

    /// Read the private-method brand owned by an object's HomeObject slot.
    ///
    /// The atom is an internal identity rather than an ECMAScript property.
    /// Its ownership remains with the object until finalization.
    pub fn object_private_brand_home(&self, id: ObjectId) -> Result<Option<Atom>, HeapError> {
        Ok(self.object(id)?.private_brand_home)
    }

    /// Attach the freshly allocated private-method brand for one class side.
    ///
    /// The caller transfers one owned atom reference on success. A HomeObject
    /// has exactly one brand even when the class declares several methods.
    pub fn attach_object_private_brand_home(
        &mut self,
        id: ObjectId,
        brand: Atom,
    ) -> Result<(), HeapError> {
        let object = self.object_mut(id)?;
        if object.private_brand_home.is_some() {
            return Err(HeapError::Invariant(
                "private-method HomeObject already has a brand",
            ));
        }
        object.private_brand_home = Some(brand);
        Ok(())
    }

    /// Read the optional HomeObject edge of one bytecode function.
    ///
    /// Native, bound, and ordinary objects are rejected rather than silently
    /// impersonating bytecode functions at the super-resolution boundary.
    pub fn bytecode_function_home_object(
        &self,
        id: ObjectId,
    ) -> Result<Option<ObjectId>, HeapError> {
        let ObjectPayload::BytecodeFunction { home_object, .. } = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "HomeObject lookup reached a non-bytecode function",
            ));
        };
        Ok(*home_object)
    }

    /// Read the hidden public-instance-field initializer attached to one class
    /// constructor bytecode function.
    pub fn bytecode_class_instance_initializer(
        &self,
        id: ObjectId,
    ) -> Result<Option<ObjectId>, HeapError> {
        let ObjectPayload::BytecodeFunction {
            class_instance_initializer,
            ..
        } = &self.object(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "class initializer lookup reached a non-bytecode function",
            ));
        };
        Ok(*class_instance_initializer)
    }

    /// Read one live shape record.
    pub fn shape(&self, id: ShapeId) -> Result<&Shape, HeapError> {
        match self.live_node(RawId::Shape(id))?.data {
            NodeData::Shape(ref shape) => Ok(shape),
            NodeData::Object(_)
            | NodeData::VarRef(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed shape lookup reached another node payload",
            )),
        }
    }

    /// Read one captured-variable cell. All functions holding the same
    /// `VarRefId` observe this shared value.
    pub fn var_ref(&self, id: VarRefId) -> Result<&VarRefData, HeapError> {
        match self.live_node(RawId::VarRef(id))?.data {
            NodeData::VarRef(ref var_ref) => Ok(var_ref),
            NodeData::Object(_)
            | NodeData::Shape(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed var-ref lookup reached another node payload",
            )),
        }
    }

    /// Read one live context record.
    pub fn context(&self, id: ContextId) -> Result<&ContextData, HeapError> {
        match self.live_node(RawId::Context(id))?.data {
            NodeData::Context(ref context) => Ok(context),
            NodeData::Object(_)
            | NodeData::Shape(_)
            | NodeData::VarRef(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed context lookup reached another node payload",
            )),
        }
    }

    /// Seed the realm-local xorshift64* stream used by `Math.random`.
    /// QuickJS replaces an all-zero time seed with one because zero is the
    /// generator's absorbing state.
    pub(crate) fn initialize_math_random_state(
        &mut self,
        id: ContextId,
        seed: u64,
    ) -> Result<(), HeapError> {
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(id))?.data else {
            return Err(HeapError::Invariant(
                "typed context lookup reached another node payload",
            ));
        };
        if context.math_random_state != 0 {
            return Err(HeapError::Invariant(
                "Math.random state was initialized more than once",
            ));
        }
        context.math_random_state = if seed == 0 { 1 } else { seed };
        Ok(())
    }

    /// Advance the pinned QuickJS xorshift64* stream for one realm.
    pub(crate) fn next_math_random_u64(&mut self, id: ContextId) -> Result<u64, HeapError> {
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(id))?.data else {
            return Err(HeapError::Invariant(
                "typed context lookup reached another node payload",
            ));
        };
        if context.math_random_state == 0 {
            return Err(HeapError::Invariant(
                "Math.random state was used before initialization",
            ));
        }
        let mut state = context.math_random_state;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        context.math_random_state = state;
        Ok(state.wrapping_mul(0x2545_f491_4f6c_dd1d))
    }

    /// Clone one native function's typed hidden capture.  Returned raw values
    /// and identities are borrowed snapshots; callers that keep them across a
    /// heap mutation must first promote or otherwise retain their edges.
    pub(crate) fn native_internal_callable(
        &self,
        id: ObjectId,
    ) -> Result<Option<InternalCallableData>, HeapError> {
        let ObjectPayload::NativeFunction { internal, .. } = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "internal callable lookup reached a non-native function",
            ));
        };
        Ok(internal.clone())
    }

    /// Store the two arbitrary arguments supplied to a NewPromiseCapability
    /// executor.  Callability is deliberately checked later by the runtime,
    /// after the custom constructor returns, as required by the specification.
    ///
    /// Object edges are retained before publication.  Symbol atoms must be
    /// pre-owned by the caller and transfer to the capture only when this
    /// returns `true`. `false` reports the spec-visible repeated invocation;
    /// the runtime must throw a TypeError and retain caller ownership.
    pub(crate) fn set_promise_capability_capture(
        &mut self,
        id: ObjectId,
        resolve: RawValue,
        reject: RawValue,
    ) -> Result<bool, HeapError> {
        if !is_promise_storable_value(&resolve) || !is_promise_storable_value(&reject) {
            return Err(HeapError::Invariant(
                "Promise capability capture contains an internal value sentinel",
            ));
        }
        match &self.object(id)?.payload {
            ObjectPayload::NativeFunction {
                data:
                    NativeFunctionData {
                        target: NativeFunctionId::PromiseCapabilityExecutor,
                        ..
                    },
                internal: Some(InternalCallableData::PromiseCapabilityExecutor(capture)),
            } if capture
                .resolve
                .as_ref()
                .is_none_or(|value| matches!(value, RawValue::Undefined))
                && capture
                    .reject
                    .as_ref()
                    .is_none_or(|value| matches!(value, RawValue::Undefined)) => {}
            ObjectPayload::NativeFunction {
                data:
                    NativeFunctionData {
                        target: NativeFunctionId::PromiseCapabilityExecutor,
                        ..
                    },
                internal: Some(InternalCallableData::PromiseCapabilityExecutor(_)),
            } => {
                return Ok(false);
            }
            _ => {
                return Err(HeapError::Invariant(
                    "Promise capability capture reached the wrong native function",
                ));
            }
        }

        let mut edges = raw_value_edges(&resolve);
        edges.extend(raw_value_edges(&reject));
        self.retain_edges_transactionally(&edges)?;
        let ObjectPayload::NativeFunction {
            internal: Some(InternalCallableData::PromiseCapabilityExecutor(capture)),
            ..
        } = &mut self.object_mut(id)?.payload
        else {
            unreachable!("Promise capability executor was validated before retaining arguments")
        };
        capture.resolve = Some(resolve);
        capture.reject = Some(reject);
        Ok(true)
    }

    /// Borrow a copy of the current NewPromiseCapability capture.
    /// Raw values in the result do not own additional heap or atom references.
    pub(crate) fn promise_capability_capture(
        &self,
        id: ObjectId,
    ) -> Result<PromiseCapabilityExecutorData, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::NativeFunction {
                data:
                    NativeFunctionData {
                        target: NativeFunctionId::PromiseCapabilityExecutor,
                        ..
                    },
                internal: Some(InternalCallableData::PromiseCapabilityExecutor(capture)),
            } => Ok(capture.clone()),
            _ => Err(HeapError::Invariant(
                "Promise capability lookup reached the wrong native function",
            )),
        }
    }

    /// Borrow a complete snapshot of one genuine Promise's hidden state.
    /// Raw edges in the clone are not independently retained.
    pub(crate) fn promise_snapshot(&self, id: ObjectId) -> Result<PromiseData, HeapError> {
        let ObjectPayload::Promise(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "Promise snapshot reached an object with the wrong class",
            ));
        };
        Ok(data.clone())
    }

    /// Append the paired reactions created by one `PerformPromiseThen` call.
    /// Every handler and capability identity is retained transactionally
    /// before either vector becomes observable to the collector.
    pub(crate) fn promise_add_reactions(
        &mut self,
        id: ObjectId,
        fulfill: PromiseReaction,
        reject: PromiseReaction,
    ) -> Result<(), HeapError> {
        if fulfill.kind != PromiseReactionKind::Fulfill
            || reject.kind != PromiseReactionKind::Reject
        {
            return Err(HeapError::Invariant(
                "Promise reactions were appended to the wrong settlement lists",
            ));
        }
        match &self.object(id)?.payload {
            ObjectPayload::Promise(PromiseData {
                state: PromiseState::Pending,
                ..
            }) => {}
            ObjectPayload::Promise(_) => {
                return Err(HeapError::Invariant(
                    "cannot append reactions to a settled Promise",
                ));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "Promise reaction append reached an object with the wrong class",
                ));
            }
        }

        let mut edges = promise_reaction_edges(&fulfill);
        edges.extend(promise_reaction_edges(&reject));
        self.retain_edges_transactionally(&edges)?;
        let ObjectPayload::Promise(data) = &mut self.object_mut(id)?.payload else {
            unreachable!("Promise payload was validated before retaining reactions")
        };
        data.fulfill_reactions.push(fulfill);
        data.reject_reactions.push(reject);
        Ok(())
    }

    /// Settle one pending Promise and detach all pending reaction ownership.
    ///
    /// The runtime must first snapshot and enqueue the selected reaction list;
    /// job enqueue retains its own edges.  This method then publishes the
    /// settled result, releases both obsolete reaction lists, and returns any
    /// detached Symbol atom ownership through the usual cleanup channel.
    pub(crate) fn promise_settle(
        &mut self,
        id: ObjectId,
        state: PromiseState,
        result: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        if state == PromiseState::Pending || !is_promise_storable_value(&result) {
            return Err(HeapError::Invariant(
                "Promise settlement requires a final state and ordinary value",
            ));
        }
        match &self.object(id)?.payload {
            ObjectPayload::Promise(PromiseData {
                state: PromiseState::Pending,
                ..
            }) => {}
            ObjectPayload::Promise(_) => {
                return Err(HeapError::Invariant("Promise was settled more than once"));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "Promise settlement reached an object with the wrong class",
                ));
            }
        }

        let new_edges = raw_value_edges(&result);
        self.retain_edges_transactionally(&new_edges)?;
        let (previous, fulfill_reactions, reject_reactions) = {
            let ObjectPayload::Promise(data) = &mut self.object_mut(id)?.payload else {
                unreachable!("Promise payload was validated before retaining its result")
            };
            let previous = std::mem::replace(&mut data.result, result);
            data.state = state;
            (
                previous,
                std::mem::take(&mut data.fulfill_reactions),
                std::mem::take(&mut data.reject_reactions),
            )
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&previous));
        for edge in raw_value_edges(&previous) {
            self.release_raw_no_drain(edge)?;
        }
        for reaction in fulfill_reactions.iter().chain(&reject_reactions) {
            for edge in promise_reaction_edges(reaction) {
                self.release_raw_no_drain(edge)?;
            }
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Mark one genuine Promise handled and report whether it was already
    /// handled, allowing the runtime to mirror QuickJS rejection tracking.
    pub(crate) fn promise_mark_handled(&mut self, id: ObjectId) -> Result<bool, HeapError> {
        let ObjectPayload::Promise(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "Promise handled update reached an object with the wrong class",
            ));
        };
        Ok(std::mem::replace(&mut data.is_handled, true))
    }

    /// Read immutable executable data without promoting any raw cpool edges.
    pub fn function_bytecode(
        &self,
        id: FunctionBytecodeId,
    ) -> Result<&FunctionBytecodeData, HeapError> {
        match self.live_node(RawId::FunctionBytecode(id))?.data {
            NodeData::FunctionBytecode(ref bytecode) => Ok(bytecode),
            NodeData::Object(_)
            | NodeData::Shape(_)
            | NodeData::VarRef(_)
            | NodeData::Context(_) => Err(HeapError::Invariant(
                "typed bytecode lookup reached another node payload",
            )),
        }
    }

    /// Replace the value stored in a captured-variable cell transactionally.
    ///
    /// The new value's GC edge is retained before the old edge is detached.
    /// A symbol atom transfers to the heap on success; any atom owned by the
    /// previous value is returned in the cleanup.
    pub fn replace_var_ref_value(
        &mut self,
        id: VarRefId,
        replacement: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        let current = self.var_ref(id)?;
        validate_var_ref_value(
            current.kind,
            current.is_lexical,
            current.is_const,
            &replacement,
        )?;
        let new_edges = raw_value_edges(&replacement);
        self.retain_edges_transactionally(&new_edges)?;

        let previous = {
            let var_ref = self.var_ref_mut(id)?;
            std::mem::replace(&mut var_ref.value, replacement)
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&previous));
        for edge in raw_value_edges(&previous) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Update binding-mode metadata without disturbing the shared value or
    /// any of its retained GC edges.
    pub fn set_var_ref_metadata(
        &mut self,
        id: VarRefId,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Result<(), HeapError> {
        let var_ref = self.var_ref_mut(id)?;
        validate_var_ref_value(kind, is_lexical, is_const, &var_ref.value)?;
        var_ref.is_lexical = is_lexical;
        var_ref.is_const = is_const;
        var_ref.kind = kind;
        Ok(())
    }

    /// Advance one branded String Iterator by one Unicode code point.
    ///
    /// The stored cursor is a UTF-16 code-unit index. A valid lead/trail pair
    /// advances by two and is returned unchanged as a two-unit string; every
    /// lone surrogate advances by one and is preserved verbatim. At end the
    /// backing string is released eagerly, matching QuickJS's transition to
    /// an undefined iterator target.
    pub fn string_iterator_next(&mut self, id: ObjectId) -> Result<Option<JsString>, HeapError> {
        let object = self.object_mut(id)?;
        let ObjectPayload::StringIterator { string, next_index } = &mut object.payload else {
            return Err(HeapError::Invariant(
                "String Iterator next reached an object with the wrong class",
            ));
        };
        let Some(value) = string.as_ref() else {
            return Ok(None);
        };
        if *next_index >= value.len() {
            *string = None;
            return Ok(None);
        }

        let first = value
            .code_unit_at(*next_index)
            .expect("validated String Iterator index must name a code unit");
        let pair = (0xd800..=0xdbff).contains(&first)
            && next_index
                .checked_add(1)
                .and_then(|index| value.code_unit_at(index))
                .is_some_and(|unit| (0xdc00..=0xdfff).contains(&unit));
        let width = if pair { 2 } else { 1 };
        let result = JsString::try_from_utf16((0..width).map(|offset| {
            value
                .code_unit_at(*next_index + offset)
                .expect("validated String Iterator code-point width must remain in bounds")
        }))
        .map_err(|_| HeapError::Invariant("String Iterator produced an oversized code point"))?;
        *next_index += width;
        Ok(Some(result))
    }

    /// Read the internal millisecond time value of one genuine Date object.
    pub fn date_value(&self, id: ObjectId) -> Result<f64, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Date(value) => Ok(*value),
            _ => Err(HeapError::Invariant(
                "Date value requested for an object with the wrong class",
            )),
        }
    }

    /// Replace the internal millisecond time value of one genuine Date.
    /// This payload owns no arena or atom edges, so mutation is infallible
    /// after the branded object identity has been validated.
    pub fn set_date_value(&mut self, id: ObjectId, value: f64) -> Result<(), HeapError> {
        let ObjectPayload::Date(current) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "Date value update reached an object with the wrong class",
            ));
        };
        *current = value;
        Ok(())
    }

    /// Read the typed internal state of one genuine RegExp object.
    pub fn regexp_data(&self, id: ObjectId) -> Result<&RegExpObjectData, HeapError> {
        let ObjectPayload::RegExp(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "RegExp data requested for an object with the wrong class",
            ));
        };
        Ok(data)
    }

    /// Replace one genuine RegExp object's source/program state.
    ///
    /// Both variants are reference-counted leaves without arena or atom
    /// edges, so mutation needs no retain/release transaction in this heap.
    pub fn replace_regexp_data(
        &mut self,
        id: ObjectId,
        replacement: RegExpObjectData,
    ) -> Result<RegExpObjectData, HeapError> {
        let ObjectPayload::RegExp(current) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "RegExp data update reached an object with the wrong class",
            ));
        };
        Ok(std::mem::replace(current, replacement))
    }

    /// Snapshot one branded RegExp String Iterator's retained matcher, input
    /// string, cached flag modes, and completion state.
    pub fn regexp_string_iterator_state(
        &self,
        id: ObjectId,
    ) -> Result<(ObjectId, JsString, bool, bool, bool), HeapError> {
        let ObjectPayload::RegExpStringIterator {
            regexp,
            string,
            global,
            full_unicode,
            done,
        } = &self.object(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "RegExp String Iterator state reached an object with the wrong class",
            ));
        };
        Ok((*regexp, string.clone(), *global, *full_unicode, *done))
    }

    /// Mark one branded RegExp String Iterator complete without releasing its
    /// matcher or input string. Pinned QuickJS retains both payload values until
    /// the iterator object itself is finalized.
    pub fn finish_regexp_string_iterator(&mut self, id: ObjectId) -> Result<(), HeapError> {
        let ObjectPayload::RegExpStringIterator { done, .. } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "RegExp String Iterator completion reached an object with the wrong class",
            ));
        };
        *done = true;
        Ok(())
    }

    /// Snapshot one branded Array Iterator's live target, cursor, and mode.
    pub fn array_iterator_state(
        &self,
        id: ObjectId,
    ) -> Result<(Option<ObjectId>, u32, ArrayIteratorKind), HeapError> {
        let ObjectPayload::ArrayIterator {
            object,
            next_index,
            kind,
        } = &self.object(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Array Iterator state reached an object with the wrong class",
            ));
        };
        Ok((*object, *next_index, *kind))
    }

    /// Advance a branded Array Iterator after its current element has been
    /// selected. Property lookup may still throw after this update, matching
    /// QuickJS's cursor order.
    pub fn set_array_iterator_index(
        &mut self,
        id: ObjectId,
        next_index: u32,
    ) -> Result<(), HeapError> {
        let ObjectPayload::ArrayIterator {
            object,
            next_index: stored,
            ..
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Array Iterator advance reached an object with the wrong class",
            ));
        };
        if object.is_none() {
            return Err(HeapError::Invariant(
                "completed Array Iterator was advanced",
            ));
        }
        *stored = next_index;
        Ok(())
    }

    /// Permanently detach a completed Array Iterator target and release its
    /// owned object edge.
    pub fn finish_array_iterator(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        let source = {
            let ObjectPayload::ArrayIterator { object, .. } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Array Iterator completion reached an object with the wrong class",
                ));
            };
            object.take()
        };
        let Some(source) = source else {
            return Ok(HeapCleanup::default());
        };
        self.release_raw_no_drain(RawId::Object(source))?;
        self.drain_zero_queue()
    }

    /// Borrow the stable insertion-order record array of one genuine Map.
    /// Tombstones remain present with a `None` key and `undefined` value.
    pub fn map_records(&self, id: ObjectId) -> Result<&[MapRecord], HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Map { records, .. } => Ok(records),
            _ => Err(HeapError::Invariant(
                "Map records requested for an object with the wrong class",
            )),
        }
    }

    /// Read the number of live records in one genuine Map.
    pub fn map_size(&self, id: ObjectId) -> Result<usize, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Map { size, .. } => Ok(*size),
            _ => Err(HeapError::Invariant(
                "Map size requested for an object with the wrong class",
            )),
        }
    }

    /// Append a new live Map record after the caller has established that no
    /// SameValueZero-equal live key exists. Object edges are retained before
    /// publication. Symbol atom references must already be owned by the
    /// caller and transfer to the Map only when this operation succeeds.
    pub fn map_insert_record(
        &mut self,
        id: ObjectId,
        key: RawValue,
        value: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        if !is_map_storable_value(&key) || !is_map_storable_value(&value) {
            return Err(HeapError::Invariant(
                "Map record contains an internal value sentinel",
            ));
        }
        let next_size = match &self.object(id)?.payload {
            ObjectPayload::Map { size, .. } => size.checked_add(1).ok_or(HeapError::Overflow {
                operation: "growing Map size",
            })?,
            _ => {
                return Err(HeapError::Invariant(
                    "Map insertion reached an object with the wrong class",
                ));
            }
        };

        let mut new_edges = raw_value_edges(&key);
        new_edges.extend(raw_value_edges(&value));
        self.retain_edges_transactionally(&new_edges)?;

        let ObjectPayload::Map { records, size } = &mut self.object_mut(id)?.payload else {
            unreachable!("Map payload was validated before retaining record edges")
        };
        records.push(MapRecord {
            key: Some(key),
            value,
        });
        *size = next_size;
        Ok(HeapCleanup::default())
    }

    /// Replace the value of a caller-resolved live Map record. The replacement
    /// edge is retained before the previous value is detached. A replacement
    /// Symbol atom transfers on success; the previous Symbol atom is returned
    /// through [`HeapCleanup::atoms`].
    pub fn map_replace_record_value(
        &mut self,
        id: ObjectId,
        index: usize,
        value: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        if !is_map_storable_value(&value) {
            return Err(HeapError::Invariant(
                "Map record contains an internal value sentinel",
            ));
        }
        match &self.object(id)?.payload {
            ObjectPayload::Map { records, .. }
                if records
                    .get(index)
                    .is_some_and(|record| record.key.is_some()) => {}
            ObjectPayload::Map { .. } => {
                return Err(HeapError::Invariant(
                    "Map value replacement requires a live record index",
                ));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "Map value replacement reached an object with the wrong class",
                ));
            }
        }

        let new_edges = raw_value_edges(&value);
        self.retain_edges_transactionally(&new_edges)?;
        let previous = {
            let ObjectPayload::Map { records, .. } = &mut self.object_mut(id)?.payload else {
                unreachable!("Map payload was validated before retaining replacement edges")
            };
            std::mem::replace(&mut records[index].value, value)
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&previous));
        for edge in raw_value_edges(&previous) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Turn a caller-resolved live Map record into a tombstone without
    /// changing stable record indices. Owned key/value edges and Symbol atoms
    /// are detached together.
    pub fn map_delete_record(
        &mut self,
        id: ObjectId,
        index: usize,
    ) -> Result<HeapCleanup, HeapError> {
        let (key, value) = {
            let ObjectPayload::Map { records, size } = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "Map deletion reached an object with the wrong class",
                ));
            };
            if *size == 0 {
                return Err(HeapError::Invariant(
                    "Map deletion requires a live record index",
                ));
            }
            let record = records.get_mut(index).ok_or(HeapError::Invariant(
                "Map deletion requires a live record index",
            ))?;
            let key = record.key.take().ok_or(HeapError::Invariant(
                "Map deletion requires a live record index",
            ))?;
            let value = std::mem::replace(&mut record.value, RawValue::Undefined);
            *size -= 1;
            (key, value)
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&key));
        cleanup.atoms.extend(raw_value_atom(&value));
        for edge in raw_value_edges(&key)
            .into_iter()
            .chain(raw_value_edges(&value))
        {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Tombstone every live Map record while preserving the record array for
    /// existing iterators. All detached edges and Symbol atoms are finalized
    /// after the object mutation has completed.
    pub fn map_clear(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        let removed = {
            let ObjectPayload::Map { records, size } = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "Map clear reached an object with the wrong class",
                ));
            };
            let mut removed = Vec::with_capacity(*size);
            for record in records {
                if let Some(key) = record.key.take() {
                    let value = std::mem::replace(&mut record.value, RawValue::Undefined);
                    removed.push((key, value));
                }
            }
            *size = 0;
            removed
        };

        let mut cleanup = HeapCleanup::default();
        for (key, value) in removed {
            cleanup.atoms.extend(raw_value_atom(&key));
            cleanup.atoms.extend(raw_value_atom(&value));
            for edge in raw_value_edges(&key)
                .into_iter()
                .chain(raw_value_edges(&value))
            {
                self.release_raw_no_drain(edge)?;
            }
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Snapshot one branded Map Iterator's live source, stable record cursor,
    /// and result projection.
    pub fn map_iterator_state(
        &self,
        id: ObjectId,
    ) -> Result<(Option<ObjectId>, usize, MapIteratorKind), HeapError> {
        let ObjectPayload::MapIterator {
            object,
            next_index,
            kind,
        } = &self.object(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Map Iterator state reached an object with the wrong class",
            ));
        };
        Ok((*object, *next_index, *kind))
    }

    /// Advance a live Map Iterator to the next stable record index. The source
    /// edge remains retained so later record appends are visible.
    pub fn set_map_iterator_index(
        &mut self,
        id: ObjectId,
        next_index: usize,
    ) -> Result<(), HeapError> {
        let ObjectPayload::MapIterator {
            object,
            next_index: stored,
            ..
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Map Iterator advance reached an object with the wrong class",
            ));
        };
        if object.is_none() {
            return Err(HeapError::Invariant("completed Map Iterator was advanced"));
        }
        *stored = next_index;
        Ok(())
    }

    /// Permanently detach an exhausted Map Iterator source and release its
    /// owned object edge. Repeated completion is idempotent.
    pub fn finish_map_iterator(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        let source = {
            let ObjectPayload::MapIterator { object, .. } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Map Iterator completion reached an object with the wrong class",
                ));
            };
            object.take()
        };
        let Some(source) = source else {
            return Ok(HeapCleanup::default());
        };
        self.release_raw_no_drain(RawId::Object(source))?;
        self.drain_zero_queue()
    }

    /// Borrow the stable insertion-order record array of one genuine Set.
    /// Live elements occupy `key`; both live and tombstoned `value` slots are
    /// always `undefined`.
    pub fn set_records(&self, id: ObjectId) -> Result<&[MapRecord], HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Set { records, .. } => Ok(records),
            _ => Err(HeapError::Invariant(
                "Set records requested for an object with the wrong class",
            )),
        }
    }

    /// Read the number of live records in one genuine Set.
    pub fn set_size(&self, id: ObjectId) -> Result<usize, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Set { size, .. } => Ok(*size),
            _ => Err(HeapError::Invariant(
                "Set size requested for an object with the wrong class",
            )),
        }
    }

    /// Append a new live Set record after the caller has established that no
    /// SameValueZero-equal element exists. Object edges are retained before
    /// publication. A Symbol atom must already be owned by the caller and
    /// transfers to the Set only when this operation succeeds.
    pub fn set_insert_record(
        &mut self,
        id: ObjectId,
        key: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        if !is_map_storable_value(&key) {
            return Err(HeapError::Invariant(
                "Set record contains an internal value sentinel",
            ));
        }
        let next_size = match &self.object(id)?.payload {
            ObjectPayload::Set { size, .. } => size.checked_add(1).ok_or(HeapError::Overflow {
                operation: "growing Set size",
            })?,
            _ => {
                return Err(HeapError::Invariant(
                    "Set insertion reached an object with the wrong class",
                ));
            }
        };

        let new_edges = raw_value_edges(&key);
        self.retain_edges_transactionally(&new_edges)?;

        let ObjectPayload::Set { records, size } = &mut self.object_mut(id)?.payload else {
            unreachable!("Set payload was validated before retaining record edges")
        };
        records.push(MapRecord {
            key: Some(key),
            value: RawValue::Undefined,
        });
        *size = next_size;
        Ok(HeapCleanup::default())
    }

    /// Turn a caller-resolved live Set record into a tombstone without
    /// changing stable record indices. The owned element edge and Symbol atom
    /// are detached together.
    pub fn set_delete_record(
        &mut self,
        id: ObjectId,
        index: usize,
    ) -> Result<HeapCleanup, HeapError> {
        let key = {
            let ObjectPayload::Set { records, size } = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "Set deletion reached an object with the wrong class",
                ));
            };
            if *size == 0 {
                return Err(HeapError::Invariant(
                    "Set deletion requires a live record index",
                ));
            }
            let record = records.get_mut(index).ok_or(HeapError::Invariant(
                "Set deletion requires a live record index",
            ))?;
            if !matches!(record.value, RawValue::Undefined) {
                return Err(HeapError::Invariant(
                    "Set record value slot is not undefined",
                ));
            }
            let key = record.key.take().ok_or(HeapError::Invariant(
                "Set deletion requires a live record index",
            ))?;
            *size -= 1;
            key
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&key));
        for edge in raw_value_edges(&key) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Tombstone every live Set record while preserving stable indices for
    /// existing iterators. Detached edges and Symbol atoms are finalized only
    /// after the payload mutation completes.
    pub fn set_clear(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        let removed = {
            let ObjectPayload::Set { records, size } = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "Set clear reached an object with the wrong class",
                ));
            };
            if records
                .iter()
                .any(|record| !matches!(record.value, RawValue::Undefined))
            {
                return Err(HeapError::Invariant(
                    "Set record value slot is not undefined",
                ));
            }
            let mut removed = Vec::with_capacity(*size);
            for record in records {
                if let Some(key) = record.key.take() {
                    removed.push(key);
                }
            }
            *size = 0;
            removed
        };

        let mut cleanup = HeapCleanup::default();
        for key in removed {
            cleanup.atoms.extend(raw_value_atom(&key));
            for edge in raw_value_edges(&key) {
                self.release_raw_no_drain(edge)?;
            }
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Snapshot one branded Set Iterator's live source, stable record cursor,
    /// and result projection.
    pub fn set_iterator_state(
        &self,
        id: ObjectId,
    ) -> Result<(Option<ObjectId>, usize, SetIteratorKind), HeapError> {
        let ObjectPayload::SetIterator {
            object,
            next_index,
            kind,
        } = &self.object(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Set Iterator state reached an object with the wrong class",
            ));
        };
        Ok((*object, *next_index, *kind))
    }

    /// Advance a live Set Iterator to the next stable record index. The source
    /// edge remains retained so later record appends are visible.
    pub fn set_set_iterator_index(
        &mut self,
        id: ObjectId,
        next_index: usize,
    ) -> Result<(), HeapError> {
        let ObjectPayload::SetIterator {
            object,
            next_index: stored,
            ..
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Set Iterator advance reached an object with the wrong class",
            ));
        };
        if object.is_none() {
            return Err(HeapError::Invariant("completed Set Iterator was advanced"));
        }
        *stored = next_index;
        Ok(())
    }

    /// Permanently detach an exhausted Set Iterator source and release its
    /// owned object edge. Repeated completion is idempotent.
    pub fn finish_set_iterator(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        let source = {
            let ObjectPayload::SetIterator { object, .. } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Set Iterator completion reached an object with the wrong class",
                ));
            };
            object.take()
        };
        let Some(source) = source else {
            return Ok(HeapCleanup::default());
        };
        self.release_raw_no_drain(RawId::Object(source))?;
        self.drain_zero_queue()
    }

    /// Read the representation-sensitive dense prefix tracked for a genuine
    /// QuickJS Array. `None` means the Array has converted to slow properties.
    pub fn array_fast_len(&self, id: ObjectId) -> Result<Option<u32>, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Array { fast_len } => Ok(*fast_len),
            _ => Err(HeapError::Invariant(
                "Array fast state requested for an object with the wrong class",
            )),
        }
    }

    /// Update the logical QuickJS fast-Array representation after a property
    /// operation. Converting to `None` is intentionally irreversible.
    pub fn set_array_fast_len(
        &mut self,
        id: ObjectId,
        fast_len: Option<u32>,
    ) -> Result<(), HeapError> {
        let ObjectPayload::Array { fast_len: current } = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "Array fast state update reached an object with the wrong class",
            ));
        };
        *current = fast_len;
        Ok(())
    }

    /// Read one Arguments object's representation-sensitive indexed prefix.
    pub fn arguments_state(&self, id: ObjectId) -> Result<(bool, Option<u32>), HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Arguments { mapped, fast_len } => Ok((*mapped, *fast_len)),
            _ => Err(HeapError::Invariant(
                "Arguments state requested for an object with the wrong class",
            )),
        }
    }

    /// Update one Arguments object's fast indexed representation. Conversion
    /// to `None` is irreversible at the runtime semantic boundary.
    pub fn set_arguments_fast_len(
        &mut self,
        id: ObjectId,
        fast_len: Option<u32>,
    ) -> Result<(), HeapError> {
        let ObjectPayload::Arguments {
            fast_len: current, ..
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Arguments fast state update reached an object with the wrong class",
            ));
        };
        *current = fast_len;
        Ok(())
    }

    /// Clone one generator's raw dormant activation while its object still
    /// owns every referenced edge. The runtime immediately maps this snapshot
    /// to rooted handles before beginning the destructive resume transition.
    pub fn generator_snapshot(
        &self,
        id: ObjectId,
    ) -> Result<(GeneratorState, Option<GeneratorActivationData>), HeapError> {
        let ObjectPayload::Generator { state, activation } = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "Generator state requested for an object with the wrong class",
            ));
        };
        Ok((*state, activation.as_deref().cloned()))
    }

    /// Move a suspended generator to `Executing` and detach the heap-owned
    /// activation edges. The caller must already have rooted a snapshot of the
    /// returned activation so releasing these occurrences cannot invalidate
    /// the active Rust representation.
    pub fn begin_generator_resume(
        &mut self,
        id: ObjectId,
    ) -> Result<(GeneratorState, GeneratorActivationData, HeapCleanup), HeapError> {
        let (state, activation) = {
            let ObjectPayload::Generator { state, activation } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Generator resume reached an object with the wrong class",
                ));
            };
            if !matches!(
                state,
                GeneratorState::SuspendedStart
                    | GeneratorState::SuspendedYield
                    | GeneratorState::SuspendedYieldStar
            ) {
                return Err(HeapError::Invariant(
                    "Generator resume began outside a suspended state",
                ));
            }
            let previous = *state;
            let activation = activation.take().ok_or(HeapError::Invariant(
                "suspended Generator has no activation",
            ))?;
            *state = GeneratorState::Executing;
            (previous, *activation)
        };
        let edges = generator_activation_edges(&activation);
        let atoms = generator_activation_atoms(&activation);
        for edge in edges {
            self.release_raw_no_drain(edge)?;
        }
        let mut cleanup = self.drain_zero_queue()?;
        cleanup.atoms.extend(atoms);
        Ok((state, activation, cleanup))
    }

    /// Reattach a fully encoded dormant frame after an executing generator
    /// reaches its next suspension point. Atom occurrences are retained by the
    /// runtime before this call; arena edges are retained transactionally here.
    pub fn suspend_generator(
        &mut self,
        id: ObjectId,
        state: GeneratorState,
        activation: GeneratorActivationData,
    ) -> Result<(), HeapError> {
        if !matches!(
            state,
            GeneratorState::SuspendedStart
                | GeneratorState::SuspendedYield
                | GeneratorState::SuspendedYieldStar
        ) {
            return Err(HeapError::Invariant(
                "Generator suspended with a non-suspended state",
            ));
        }
        match &self.object(id)?.payload {
            ObjectPayload::Generator {
                state: GeneratorState::Executing,
                activation: None,
            } => {}
            ObjectPayload::Generator { .. } => {
                return Err(HeapError::Invariant(
                    "Generator suspension did not follow an executing state",
                ));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "Generator suspension reached an object with the wrong class",
                ));
            }
        }
        let mut candidate = self.object(id)?.clone();
        candidate.payload = ObjectPayload::Generator {
            state,
            activation: Some(Box::new(activation.clone())),
        };
        self.validate_object_layout(&candidate)?;
        let edges = generator_activation_edges(&activation);
        self.retain_edges_transactionally(&edges)?;
        let ObjectPayload::Generator {
            state: current,
            activation: current_activation,
        } = &mut self.object_mut(id)?.payload
        else {
            unreachable!("Generator payload was validated before suspension")
        };
        *current = state;
        *current_activation = Some(Box::new(activation));
        Ok(())
    }

    /// Permanently finish an executing generator after return, throw, or an
    /// abrupt resume failure. Its dormant edges were already detached by
    /// [`Self::begin_generator_resume`].
    pub fn complete_generator(&mut self, id: ObjectId) -> Result<(), HeapError> {
        let ObjectPayload::Generator { state, activation } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Generator completion reached an object with the wrong class",
            ));
        };
        if *state != GeneratorState::Executing || activation.is_some() {
            return Err(HeapError::Invariant(
                "Generator completion did not follow an executing state",
            ));
        }
        *state = GeneratorState::Completed;
        Ok(())
    }

    /// Advance within one snapshotted level without cloning the complete key
    /// vector or visited set. Non-enumerable and duplicate prototype keys are
    /// consumed internally because neither can be yielded.
    pub fn next_for_in_candidate(&mut self, id: ObjectId) -> Result<ForInCandidate, HeapError> {
        let ObjectPayload::ForInIterator(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "for-in advance reached an object with the wrong class",
            ));
        };
        loop {
            let Some(object) = data.object else {
                return Ok(ForInCandidate::Done);
            };
            if data.fast_array {
                let index = u32::try_from(data.index)
                    .map_err(|_| HeapError::Invariant("for-in fast Array index exceeded Uint32"))?;
                if index < data.array_count {
                    data.index += 1;
                    return Ok(ForInCandidate::ArrayIndex { object, index });
                }
                return Ok(ForInCandidate::BaseComplete {
                    object,
                    fast_array: true,
                });
            }
            if data.index >= data.properties.len() {
                if !data.in_prototype_chain {
                    return Ok(ForInCandidate::BaseComplete {
                        object,
                        fast_array: false,
                    });
                }
                return Ok(ForInCandidate::LevelComplete(object));
            }

            let entry = data.properties[data.index].clone();
            data.index += 1;
            if data.in_prototype_chain && !data.visited.insert(entry.name.clone()) {
                continue;
            }
            if !entry.enumerable {
                continue;
            }
            let object = data.object.ok_or(HeapError::Invariant(
                "for-in property snapshot lost its current object",
            ))?;
            return Ok(ForInCandidate::Property {
                object,
                name: entry.name,
            });
        }
    }

    /// Complete QuickJS's one-time prototype-chain preparation. A generic
    /// iterator records its original base snapshot; a fast Array records a
    /// fresh own-key snapshot supplied after the prototype pre-scan.
    pub fn enter_for_in_prototype_chain(
        &mut self,
        id: ObjectId,
        refreshed_fast_properties: Option<Vec<ForInProperty>>,
    ) -> Result<(), HeapError> {
        let ObjectPayload::ForInIterator(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "for-in prototype preparation reached an object with the wrong class",
            ));
        };
        if data.in_prototype_chain {
            return Err(HeapError::Invariant(
                "for-in prototype chain was prepared more than once",
            ));
        }
        let properties = refreshed_fast_properties
            .as_ref()
            .unwrap_or(&data.properties);
        data.visited
            .extend(properties.iter().map(|entry| entry.name.clone()));
        data.in_prototype_chain = true;
        Ok(())
    }

    /// Install the next prototype level transactionally. The new current edge
    /// is retained before the old level is detached; `None` marks exhaustion.
    pub fn replace_for_in_level(
        &mut self,
        id: ObjectId,
        next_object: Option<ObjectId>,
        properties: Vec<ForInProperty>,
    ) -> Result<HeapCleanup, HeapError> {
        if !matches!(self.object(id)?.payload, ObjectPayload::ForInIterator(_)) {
            return Err(HeapError::Invariant(
                "for-in level update reached an object with the wrong class",
            ));
        }
        if let Some(object) = next_object {
            self.retain_raw(RawId::Object(object), 1)?;
        }
        let previous = {
            let ObjectPayload::ForInIterator(data) = &mut self.object_mut(id)?.payload else {
                unreachable!("for-in payload was validated before level replacement")
            };
            data.index = 0;
            data.properties = properties;
            data.fast_array = false;
            data.array_count = 0;
            std::mem::replace(&mut data.object, next_object)
        };
        if let Some(object) = previous {
            self.release_raw_no_drain(RawId::Object(object))?;
        }
        self.drain_zero_queue()
    }

    /// Update the ordinary object's extensibility bit without changing its
    /// shape or property payloads.
    pub fn set_object_extensible(
        &mut self,
        id: ObjectId,
        extensible: bool,
    ) -> Result<(), HeapError> {
        self.object_mut(id)?.extensible = extensible;
        Ok(())
    }

    /// Permanently lock the object's prototype, matching QuickJS's
    /// immutable-prototype flag used by selected intrinsics.
    pub fn set_immutable_prototype(&mut self, id: ObjectId) -> Result<(), HeapError> {
        self.object_mut(id)?.immutable_prototype = true;
        Ok(())
    }

    /// Transactionally replace one property payload.
    ///
    /// New edges are retained before the old payload is detached.  Releasing
    /// the old payload can reclaim an unrooted receiver, so callers must treat
    /// `id` as potentially stale after this operation unless they hold a root.
    pub fn replace_object_slot(
        &mut self,
        id: ObjectId,
        slot_index: usize,
        replacement: PropertySlot,
    ) -> Result<HeapCleanup, HeapError> {
        self.validate_replacement_slot(id, slot_index, &replacement)?;
        let new_edges = property_slot_edges(&replacement);
        self.retain_edges_transactionally(&new_edges)?;

        let previous = {
            let object = self.object_mut(id)?;
            let slot = object
                .slots
                .get_mut(slot_index)
                .ok_or(HeapError::Invariant(
                    "validated property slot disappeared before replacement",
                ))?;
            std::mem::replace(slot, replacement)
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(property_slot_atoms(&previous));
        for edge in property_slot_edges(&previous) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Transactionally replace a bytecode function's optional HomeObject.
    ///
    /// The replacement is retained before the previous edge is detached, so
    /// changing from an object which owns the replacement cannot make the new
    /// handle stale mid-operation. Identical `Some` values and `None -> None`
    /// are no-ops and therefore cannot overflow or perturb reference counts.
    /// Releasing the old edge may reclaim an unrooted receiver; callers must
    /// keep `id` rooted if they need to use it after this operation.
    pub fn replace_bytecode_function_home_object(
        &mut self,
        id: ObjectId,
        replacement: Option<ObjectId>,
    ) -> Result<HeapCleanup, HeapError> {
        let previous = self.bytecode_function_home_object(id)?;
        if previous == replacement {
            return Ok(HeapCleanup::default());
        }
        if let Some(home_object) = replacement {
            self.retain_raw(RawId::Object(home_object), 1)?;
        }

        let ObjectPayload::BytecodeFunction { home_object, .. } = &mut self.object_mut(id)?.payload
        else {
            unreachable!("bytecode-function payload was validated before HomeObject replacement")
        };
        *home_object = replacement;

        if let Some(home_object) = previous {
            self.release_raw_no_drain(RawId::Object(home_object))?;
        }
        self.drain_zero_queue()
    }

    /// Atomically attach a fresh instance-field initializer to one class.
    ///
    /// The constructor-to-initializer and initializer-to-prototype edges are a
    /// single publication transaction.  Neither edge can be replaced: these
    /// are compiler-owned capabilities, not mutable JavaScript state.
    pub fn attach_bytecode_class_instance_initializer(
        &mut self,
        constructor: ObjectId,
        prototype: ObjectId,
        initializer: ObjectId,
    ) -> Result<(), HeapError> {
        if constructor == prototype || constructor == initializer || prototype == initializer {
            return Err(HeapError::Invariant(
                "class initializer publication reused an object identity",
            ));
        }
        self.object(prototype)?;
        let constructor_object = self.object(constructor)?;
        let ObjectPayload::BytecodeFunction {
            bytecode: constructor_bytecode,
            class_instance_initializer: existing_initializer,
            ..
        } = &constructor_object.payload
        else {
            return Err(HeapError::Invariant(
                "class initializer owner is not a bytecode function",
            ));
        };
        let constructor_metadata = self.function_bytecode(*constructor_bytecode)?;
        if !constructor_object.is_constructor
            || constructor_metadata.metadata.constructor_kind == ConstructorKind::None
            || constructor_metadata.metadata.has_prototype
            || !constructor_metadata.metadata.strict
            || constructor_metadata
                .metadata
                .class_initializer_kind
                .is_some()
            || existing_initializer.is_some()
        {
            return Err(HeapError::Invariant(
                "class initializer owner is not a fresh class constructor",
            ));
        }
        let constructor_realm = constructor_metadata.realm;

        let initializer_object = self.object(initializer)?;
        let ObjectPayload::BytecodeFunction {
            bytecode: initializer_bytecode,
            home_object,
            class_instance_initializer,
            ..
        } = &initializer_object.payload
        else {
            return Err(HeapError::Invariant(
                "class instance initializer is not a bytecode function",
            ));
        };
        let initializer_bytecode = self.function_bytecode(*initializer_bytecode)?;
        if initializer_object.is_constructor
            || home_object.is_some()
            || class_instance_initializer.is_some()
            || initializer_bytecode.realm != constructor_realm
            || initializer_bytecode.metadata.class_initializer_kind
                != Some(ClassInitializerKind::InstanceFields)
            || !initializer_bytecode.metadata.needs_home_object
        {
            return Err(HeapError::Invariant(
                "class instance initializer is not fresh or has the wrong owner realm",
            ));
        }

        self.retain_edges_transactionally(&[RawId::Object(prototype), RawId::Object(initializer)])?;
        let ObjectPayload::BytecodeFunction { home_object, .. } =
            &mut self.object_mut(initializer)?.payload
        else {
            unreachable!("initializer payload was authenticated before edge publication")
        };
        *home_object = Some(prototype);
        let ObjectPayload::BytecodeFunction {
            class_instance_initializer,
            ..
        } = &mut self.object_mut(constructor)?.payload
        else {
            unreachable!("constructor payload was authenticated before edge publication")
        };
        *class_instance_initializer = Some(initializer);
        Ok(())
    }

    /// Claim the one permitted aggregate static-initializer execution for a
    /// class constructor. The claim is deliberately not rolled back after an
    /// abrupt initializer: a leaked constructor must never replay fields or
    /// static blocks through forged privileged bytecode.
    pub fn begin_bytecode_class_static_initializer(
        &mut self,
        constructor: ObjectId,
    ) -> Result<(), HeapError> {
        {
            let constructor_object = self.object(constructor)?;
            let ObjectPayload::BytecodeFunction {
                bytecode,
                class_static_initializer_started,
                ..
            } = &constructor_object.payload
            else {
                return Err(HeapError::Invariant(
                    "class static initializer owner is not a bytecode function",
                ));
            };
            let metadata = self.function_bytecode(*bytecode)?.metadata;
            if !constructor_object.is_constructor
                || metadata.constructor_kind == ConstructorKind::None
                || metadata.has_prototype
                || !metadata.strict
                || metadata.class_initializer_kind.is_some()
            {
                return Err(HeapError::Invariant(
                    "class static initializer owner is not a class constructor",
                ));
            }
            if *class_static_initializer_started {
                return Err(HeapError::Invariant(
                    "class static initializer was already started",
                ));
            }
        }

        let ObjectPayload::BytecodeFunction {
            class_static_initializer_started,
            ..
        } = &mut self.object_mut(constructor)?.payload
        else {
            unreachable!("static initializer owner was authenticated before its one-shot claim")
        };
        *class_static_initializer_started = true;
        Ok(())
    }

    /// Transactionally replace an object's complete shape/slot layout.
    ///
    /// This is the low-level primitive used by immutable shape transitions.
    /// The caller must already own atom references for symbol values in
    /// `slots`; on success those references transfer to the heap.  The returned
    /// cleanup contains every symbol atom detached from the previous slots.
    pub fn replace_object_layout(
        &mut self,
        id: ObjectId,
        shape: ShapeId,
        slots: Vec<PropertySlot>,
    ) -> Result<HeapCleanup, HeapError> {
        let (private_brand_home, extensible, immutable_prototype, is_constructor, kind, payload) = {
            let object = self.object(id)?;
            (
                object.private_brand_home,
                object.extensible,
                object.immutable_prototype,
                object.is_constructor,
                object.kind,
                object.payload.clone(),
            )
        };
        let replacement = ObjectData {
            shape,
            slots,
            private_brand_home,
            extensible,
            immutable_prototype,
            is_constructor,
            kind,
            payload,
        };
        self.validate_object_layout(&replacement)?;
        let new_edges = object_edges(&replacement);
        self.retain_edges_transactionally(&new_edges)?;

        let previous = {
            let object = self.object_mut(id)?;
            std::mem::replace(object, replacement)
        };

        let mut cleanup = HeapCleanup::default();
        // The class payload and private HomeObject brand are cloned unchanged
        // into `replacement`, so their non-GC atom ownership transfers in
        // place. Only detached property slots relinquish atom references here.
        cleanup.atoms.extend(object_slot_atoms(&previous));
        for edge in object_edges(&previous) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Change only the object's `[[Construct]]` capability bit.
    /// QuickJS keeps this bit independent from the native cproto used to
    /// initialize it, so changing it must not rewrite callable metadata.
    pub(crate) fn set_object_constructor_bit(
        &mut self,
        id: ObjectId,
        enabled: bool,
    ) -> Result<(), HeapError> {
        self.object_mut(id)?.is_constructor = enabled;
        Ok(())
    }

    /// Run QuickJS-style trial deletion over every live heap node.
    pub fn run_gc(&mut self) -> Result<GcStats, HeapError> {
        let mut cleanup = self.drain_zero_queue()?;
        if self
            .slots
            .iter()
            .any(|slot| matches!(slot.state, SlotState::Zombie { .. }))
        {
            return Err(HeapError::Invariant(
                "zombie survived past the preceding collection boundary",
            ));
        }

        let mut trial = vec![None; self.slots.len()];
        let mut examined_nodes = 0usize;
        for (index, slot) in self.slots.iter().enumerate() {
            if let SlotState::Live(node) = &slot.state {
                trial[index] = Some(node.strong);
                examined_nodes = examined_nodes.saturating_add(1);
            }
        }

        // Subtract every internal incoming edge.  The remainder is precisely
        // the external root count, matching QuickJS's gc_decref phase.
        for slot in &self.slots {
            let SlotState::Live(node) = &slot.state else {
                continue;
            };
            for edge in node.data.edges() {
                let target = self.live_index(edge)?;
                let count = trial[target].ok_or(HeapError::Invariant(
                    "live edge targeted a node outside the trial set",
                ))?;
                trial[target] = Some(count.checked_sub(1).ok_or(HeapError::Invariant(
                    "internal incoming references exceeded strong count",
                ))?);
            }
        }

        // Restore the closure reachable from nodes with external references,
        // equivalent to QuickJS's gc_scan phase.
        let mut reachable = vec![false; self.slots.len()];
        let mut work = VecDeque::new();
        let mut external_root_nodes = 0usize;
        for (index, count) in trial.iter().copied().enumerate() {
            if count.is_some_and(|count| count != 0) {
                reachable[index] = true;
                work.push_back(index);
                external_root_nodes = external_root_nodes.saturating_add(1);
            }
        }
        while let Some(index) = work.pop_front() {
            let SlotState::Live(node) = &self.slots[index].state else {
                return Err(HeapError::Invariant(
                    "mark worklist contained a non-live node",
                ));
            };
            for edge in node.data.edges() {
                let target = self.live_index(edge)?;
                if !reachable[target] {
                    reachable[target] = true;
                    work.push_back(target);
                }
            }
        }

        let mut candidate_nodes = 0usize;
        let mut anchors = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
            let SlotState::Live(node) = &slot.state else {
                continue;
            };
            if reachable[index] {
                continue;
            }
            candidate_nodes = candidate_nodes.saturating_add(1);
            let index = u32::try_from(index).map_err(|_| HeapError::Overflow {
                operation: "constructing a collection worklist",
            })?;
            match &node.data {
                NodeData::Object(_) => anchors.push(RawId::Object(ObjectId {
                    index,
                    generation: slot.generation,
                })),
                NodeData::FunctionBytecode(_) => {
                    anchors.push(RawId::FunctionBytecode(FunctionBytecodeId {
                        index,
                        generation: slot.generation,
                    }));
                }
                NodeData::Shape(_) | NodeData::VarRef(_) | NodeData::Context(_) => {}
            }
        }

        // Objects and function bytecodes are the active anchors used by
        // QuickJS. Mark each as a zombie before dropping its outgoing edges so
        // other candidate nodes can still release the old generation.
        for id in anchors {
            if self.is_live(id) {
                self.finalize_cycle_anchor(id, &mut cleanup)?;
                cleanup.merge(self.drain_zero_queue()?);
            }
        }
        cleanup.merge(self.drain_zero_queue()?);

        if self
            .slots
            .iter()
            .any(|slot| matches!(slot.state, SlotState::Zombie { .. }))
        {
            return Err(HeapError::Invariant(
                "cycle collection left an anchor zombie with incoming references",
            ));
        }

        Ok(GcStats {
            examined_nodes,
            external_root_nodes,
            candidate_nodes,
            cleanup,
        })
    }

    /// Strong count for diagnostics.  A zombie remains queryable until all
    /// candidate incoming edges have been detached.
    pub fn object_strong_count(&self, id: ObjectId) -> Result<u32, HeapError> {
        self.strong_count(RawId::Object(id))
    }

    /// Strong count for diagnostics.
    pub fn shape_strong_count(&self, id: ShapeId) -> Result<u32, HeapError> {
        self.strong_count(RawId::Shape(id))
    }

    /// Strong count for captured-variable diagnostics.
    pub fn var_ref_strong_count(&self, id: VarRefId) -> Result<u32, HeapError> {
        self.strong_count(RawId::VarRef(id))
    }

    /// Strong count for context diagnostics.
    pub fn context_strong_count(&self, id: ContextId) -> Result<u32, HeapError> {
        self.strong_count(RawId::Context(id))
    }

    /// Strong count for function-bytecode diagnostics.
    pub fn function_bytecode_strong_count(&self, id: FunctionBytecodeId) -> Result<u32, HeapError> {
        self.strong_count(RawId::FunctionBytecode(id))
    }

    /// Return the lifecycle state at one physical slot, if it exists.
    #[must_use]
    pub fn slot_state(&self, debug_index: u32) -> Option<HeapSlotState> {
        self.slots
            .get(debug_index as usize)
            .map(|slot| slot.state.public_state())
    }

    /// Snapshot aggregate arena counts for tests and runtime diagnostics.
    #[must_use]
    pub fn counts(&self) -> HeapCounts {
        let mut counts = HeapCounts::default();
        for slot in &self.slots {
            match &slot.state {
                SlotState::Initializing { kind, .. } => {
                    counts.initializing = counts.initializing.saturating_add(1);
                    increment_kind_count(&mut counts, *kind);
                }
                SlotState::Live(node) => {
                    counts.live = counts.live.saturating_add(1);
                    increment_kind_count(&mut counts, node.data.kind());
                }
                SlotState::ZeroQueued(node) => {
                    counts.zero_queued = counts.zero_queued.saturating_add(1);
                    increment_kind_count(&mut counts, node.data.kind());
                }
                SlotState::Finalizing(node) => {
                    counts.finalizing = counts.finalizing.saturating_add(1);
                    increment_kind_count(&mut counts, node.data.kind());
                }
                SlotState::Zombie { kind, .. } => {
                    counts.zombies = counts.zombies.saturating_add(1);
                    increment_kind_count(&mut counts, *kind);
                }
                SlotState::Vacant => counts.vacant = counts.vacant.saturating_add(1),
                SlotState::Retired => counts.retired = counts.retired.saturating_add(1),
            }
        }
        counts
    }

    fn reserve(&mut self, kind: HeapNodeKind) -> Result<(u32, u32), HeapError> {
        let index = if let Some(index) = self.free.pop() {
            index
        } else {
            let index = u32::try_from(self.slots.len()).map_err(|_| HeapError::Overflow {
                operation: "allocating an arena slot",
            })?;
            self.slots.push(ArenaSlot {
                generation: 1,
                state: SlotState::Vacant,
            });
            index
        };
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(HeapError::Invariant("free list referenced a missing slot"))?;
        if !matches!(slot.state, SlotState::Vacant) {
            return Err(HeapError::Invariant(
                "free list referenced an occupied slot",
            ));
        }
        slot.state = SlotState::Initializing { kind, strong: 1 };
        Ok((index, slot.generation))
    }

    fn abort_initializing(&mut self, index: u32) -> Result<(), HeapError> {
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(HeapError::Invariant("initializing slot disappeared"))?;
        if !matches!(slot.state, SlotState::Initializing { .. }) {
            return Err(HeapError::Invariant(
                "attempted to abort a published arena slot",
            ));
        }
        slot.state = SlotState::Vacant;
        self.free.push(index);
        Ok(())
    }

    fn publish(&mut self, index: u32, data: NodeData) -> Result<(), HeapError> {
        let expected = data.kind();
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(HeapError::Invariant("initializing slot disappeared"))?;
        let (kind, strong) = match &slot.state {
            SlotState::Initializing { kind, strong } => (*kind, *strong),
            _ => {
                return Err(HeapError::Invariant(
                    "attempted to publish a non-initializing slot",
                ));
            }
        };
        if kind != expected || strong != 1 {
            return Err(HeapError::Invariant(
                "initializing slot metadata did not match its payload",
            ));
        }
        slot.state = SlotState::Live(Node { strong, data });
        Ok(())
    }

    fn validate_object_layout(&self, object: &ObjectData) -> Result<(), HeapError> {
        if !matches!(
            (object.kind, &object.payload),
            (
                ObjectKind::Ordinary,
                ObjectPayload::Ordinary | ObjectPayload::RawJson
            ) | (ObjectKind::Array, ObjectPayload::Array { .. })
                | (ObjectKind::Arguments, ObjectPayload::Arguments { .. })
                | (
                    ObjectKind::ArrayIterator,
                    ObjectPayload::ArrayIterator { .. }
                )
                | (ObjectKind::ForInIterator, ObjectPayload::ForInIterator(_))
                | (ObjectKind::Primitive, ObjectPayload::Primitive(_))
                | (ObjectKind::Date, ObjectPayload::Date(_))
                | (ObjectKind::RegExp, ObjectPayload::RegExp(_))
                | (
                    ObjectKind::RegExpStringIterator,
                    ObjectPayload::RegExpStringIterator { .. }
                )
                | (ObjectKind::Map, ObjectPayload::Map { .. })
                | (ObjectKind::MapIterator, ObjectPayload::MapIterator { .. })
                | (ObjectKind::Set, ObjectPayload::Set { .. })
                | (ObjectKind::SetIterator, ObjectPayload::SetIterator { .. })
                | (ObjectKind::GlobalObject, ObjectPayload::GlobalObject { .. })
                | (ObjectKind::Error, ObjectPayload::Error)
                | (
                    ObjectKind::StringIterator,
                    ObjectPayload::StringIterator { .. }
                )
                | (
                    ObjectKind::NativeFunction,
                    ObjectPayload::NativeFunction { .. }
                )
                | (
                    ObjectKind::BoundFunction,
                    ObjectPayload::BoundFunction { .. }
                )
                | (
                    ObjectKind::BytecodeFunction,
                    ObjectPayload::BytecodeFunction { .. }
                )
                | (ObjectKind::Generator, ObjectPayload::Generator { .. })
                | (ObjectKind::Promise, ObjectPayload::Promise(_))
        ) {
            return Err(HeapError::Invariant(
                "object kind does not match its class payload",
            ));
        }
        let shape = self.shape(object.shape)?;
        if shape.entries().len() != object.slots.len() {
            return Err(HeapError::Invariant(
                "object slot count does not match its shape",
            ));
        }
        for (entry, slot) in shape.entries().iter().zip(&object.slots) {
            if !slot_matches_storage(slot, entry.flags.storage) {
                return Err(HeapError::Invariant(
                    "object property storage does not match its shape flags",
                ));
            }
            if matches!(slot, PropertySlot::Data(RawValue::Private(_))) {
                return Err(HeapError::Invariant(
                    "private-name identity escaped into an object value slot",
                ));
            }
        }
        if let ObjectPayload::NativeFunction { data, internal } = &object.payload {
            match (data.target, internal) {
                (
                    NativeFunctionId::PromiseResolving(target_kind),
                    Some(InternalCallableData::PromiseResolving { promise, kind, .. }),
                ) if target_kind == *kind => {
                    if object.is_constructor
                        || !matches!(self.object(*promise)?.payload, ObjectPayload::Promise(_))
                    {
                        return Err(HeapError::Invariant(
                            "Promise resolving callable has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::PromiseCapabilityExecutor,
                    Some(InternalCallableData::PromiseCapabilityExecutor(capture)),
                ) => {
                    if object.is_constructor
                        || capture
                            .resolve
                            .iter()
                            .chain(capture.reject.iter())
                            .any(|value| !is_promise_storable_value(value))
                    {
                        return Err(HeapError::Invariant(
                            "Promise capability executor has invalid hidden state",
                        ));
                    }
                }
                (NativeFunctionId::PromiseResolving(_), _)
                | (NativeFunctionId::PromiseCapabilityExecutor, _)
                | (_, Some(_)) => {
                    return Err(HeapError::Invariant(
                        "native target does not match its internal callable capture",
                    ));
                }
                (_, None) => {}
            }
        }
        if let ObjectPayload::Promise(data) = &object.payload {
            if !is_promise_storable_value(&data.result)
                || (data.state == PromiseState::Pending && data.result != RawValue::Undefined)
                || (data.state != PromiseState::Pending
                    && (!data.fulfill_reactions.is_empty() || !data.reject_reactions.is_empty()))
                || data
                    .fulfill_reactions
                    .iter()
                    .any(|reaction| reaction.kind != PromiseReactionKind::Fulfill)
                || data
                    .reject_reactions
                    .iter()
                    .any(|reaction| reaction.kind != PromiseReactionKind::Reject)
            {
                return Err(HeapError::Invariant(
                    "Promise payload has invalid hidden state",
                ));
            }
        }
        if let ObjectPayload::BytecodeFunction {
            bytecode,
            class_instance_initializer,
            class_static_initializer_started,
            closure_slots,
            ..
        } = &object.payload
        {
            let owner_bytecode = self.function_bytecode(*bytecode)?;
            let expected = usize::from(owner_bytecode.metadata.closure_count);
            if closure_slots.len() != expected {
                return Err(HeapError::Invariant(
                    "function closure slot count does not match its bytecode metadata",
                ));
            }
            if *class_static_initializer_started
                && (!object.is_constructor
                    || owner_bytecode.metadata.constructor_kind == ConstructorKind::None
                    || owner_bytecode.metadata.has_prototype
                    || !owner_bytecode.metadata.strict
                    || owner_bytecode.metadata.class_initializer_kind.is_some())
            {
                return Err(HeapError::Invariant(
                    "class static initializer guard has malformed ownership metadata",
                ));
            }
            if let Some(initializer) = class_instance_initializer {
                let initializer_object = self.object(*initializer)?;
                let ObjectPayload::BytecodeFunction {
                    bytecode: initializer_bytecode,
                    home_object,
                    class_instance_initializer: nested_initializer,
                    ..
                } = &initializer_object.payload
                else {
                    return Err(HeapError::Invariant(
                        "class instance initializer is not a bytecode function",
                    ));
                };
                let initializer_bytecode = self.function_bytecode(*initializer_bytecode)?;
                if !object.is_constructor
                    || owner_bytecode.metadata.constructor_kind == ConstructorKind::None
                    || owner_bytecode.metadata.has_prototype
                    || !owner_bytecode.metadata.strict
                    || owner_bytecode.metadata.class_initializer_kind.is_some()
                    || initializer_object.is_constructor
                    || home_object.is_none()
                    || nested_initializer.is_some()
                    || initializer_bytecode.realm != owner_bytecode.realm
                    || initializer_bytecode.metadata.class_initializer_kind
                        != Some(ClassInitializerKind::InstanceFields)
                    || !initializer_bytecode.metadata.needs_home_object
                {
                    return Err(HeapError::Invariant(
                        "class instance initializer edge has malformed ownership metadata",
                    ));
                }
            }
        }
        if let ObjectPayload::Generator { state, activation } = &object.payload {
            if object.is_constructor
                || matches!(state, GeneratorState::Executing | GeneratorState::Completed)
                    != activation.is_none()
            {
                return Err(HeapError::Invariant(
                    "generator object has inconsistent state and activation",
                ));
            }
            if let Some(activation) = activation.as_deref() {
                let bytecode = self.function_bytecode(activation.bytecode)?;
                let vm = &activation.vm;
                let function = self.object(vm.current_function)?;
                if bytecode.metadata.function_kind != FunctionKind::Generator
                    || bytecode.metadata.constructor_kind != ConstructorKind::None
                    || !bytecode.metadata.has_prototype
                    || bytecode.realm != vm.callee_realm
                    || vm.strict != bytecode.metadata.strict
                    || self.context(vm.callee_realm)?.global_object != vm.callee_global
                    || !matches!(
                        function.payload,
                        ObjectPayload::BytecodeFunction {
                            bytecode: owner,
                            ..
                        } if owner == activation.bytecode
                    )
                    || function.is_constructor
                    || activation.arguments.len() < usize::from(bytecode.metadata.argument_count)
                    || activation.locals.len() != usize::from(bytecode.metadata.local_count)
                    || activation.reusable_captured_locals.len() != activation.locals.len()
                    || vm.stack.len() > usize::from(bytecode.metadata.max_stack)
                    || vm.pc == 0
                    || vm.pc > bytecode.code.len()
                {
                    return Err(HeapError::Invariant(
                        "generator activation has invalid frame metadata",
                    ));
                }
                let suspension_matches = matches!(
                    (state, bytecode.code.get(vm.pc - 1)),
                    (
                        GeneratorState::SuspendedStart,
                        Some(Instruction::InitialYield)
                    ) | (GeneratorState::SuspendedYield, Some(Instruction::Yield))
                        | (
                            GeneratorState::SuspendedYieldStar,
                            Some(Instruction::YieldStar)
                        )
                );
                if !suspension_matches {
                    return Err(HeapError::Invariant(
                        "generator activation is not parked after its suspension opcode",
                    ));
                }
                for value in vm
                    .stack
                    .iter()
                    .chain(std::iter::once(&vm.this_value))
                    .chain(vm.normalized_this.iter())
                    .chain(std::iter::once(&vm.new_target))
                {
                    if !is_map_storable_value(value) {
                        return Err(HeapError::Invariant(
                            "generator activation contains an internal-only value",
                        ));
                    }
                }
                for binding in activation.arguments.iter().chain(activation.locals.iter()) {
                    match binding {
                        GeneratorFrameBinding::Direct(value) if !is_map_storable_value(value) => {
                            return Err(HeapError::Invariant(
                                "generator frame binding contains an internal-only value",
                            ));
                        }
                        GeneratorFrameBinding::Private(atom) if atom.is_null() => {
                            return Err(HeapError::Invariant(
                                "generator private binding contains the null atom",
                            ));
                        }
                        GeneratorFrameBinding::PrivateCallable(callable) => {
                            let callable = self.object(*callable)?;
                            if !matches!(
                                callable.payload,
                                ObjectPayload::NativeFunction { .. }
                                    | ObjectPayload::BoundFunction { .. }
                                    | ObjectPayload::BytecodeFunction { .. }
                            ) {
                                return Err(HeapError::Invariant(
                                    "generator private callable binding is not callable",
                                ));
                            }
                        }
                        GeneratorFrameBinding::Captured(var_ref) => {
                            self.var_ref(*var_ref)?;
                        }
                        GeneratorFrameBinding::Direct(_)
                        | GeneratorFrameBinding::Private(_)
                        | GeneratorFrameBinding::Uninitialized => {}
                    }
                }
                for region in &vm.regions {
                    match *region {
                        crate::vm::VmUnwindRegion::Catch {
                            target,
                            stack_depth,
                        } if target >= bytecode.code.len() || stack_depth > vm.stack.len() => {
                            return Err(HeapError::Invariant(
                                "generator catch region is outside its saved frame",
                            ));
                        }
                        crate::vm::VmUnwindRegion::Iterator { record_base, .. }
                            if record_base.saturating_add(1) >= vm.stack.len() =>
                        {
                            return Err(HeapError::Invariant(
                                "generator iterator region is outside its saved frame",
                            ));
                        }
                        crate::vm::VmUnwindRegion::Catch { .. }
                        | crate::vm::VmUnwindRegion::Iterator { .. } => {}
                    }
                }
            }
        }
        if let ObjectPayload::BoundFunction {
            target,
            this_value,
            arguments,
        } = &object.payload
        {
            let target = self.object(*target)?;
            if !matches!(
                target.payload,
                ObjectPayload::NativeFunction { .. }
                    | ObjectPayload::BoundFunction { .. }
                    | ObjectPayload::BytecodeFunction { .. }
            ) {
                return Err(HeapError::Invariant(
                    "bound function target is not callable",
                ));
            }
            if std::iter::once(this_value)
                .chain(arguments.iter())
                .any(|value| !is_map_storable_value(value))
            {
                return Err(HeapError::Invariant(
                    "bound function payload contains an internal-only value",
                ));
            }
        }
        if let ObjectPayload::Map { records, size } = &object.payload {
            let mut live = 0usize;
            for record in records {
                match &record.key {
                    Some(key) => {
                        if !is_map_storable_value(key) || !is_map_storable_value(&record.value) {
                            return Err(HeapError::Invariant(
                                "Map record contains an internal value sentinel",
                            ));
                        }
                        live = live.checked_add(1).ok_or(HeapError::Overflow {
                            operation: "validating Map size",
                        })?;
                    }
                    None if !matches!(record.value, RawValue::Undefined) => {
                        return Err(HeapError::Invariant(
                            "Map tombstone retains a value payload",
                        ));
                    }
                    None => {}
                }
            }
            if live != *size {
                return Err(HeapError::Invariant(
                    "Map live record count does not match its payload",
                ));
            }
        }
        if let ObjectPayload::MapIterator {
            object: Some(map), ..
        } = &object.payload
        {
            if !matches!(self.object(*map)?.payload, ObjectPayload::Map { .. }) {
                return Err(HeapError::Invariant(
                    "Map Iterator source does not have the Map class",
                ));
            }
        }
        if let ObjectPayload::Set { records, size } = &object.payload {
            let mut live = 0usize;
            for record in records {
                if !matches!(record.value, RawValue::Undefined) {
                    return Err(HeapError::Invariant(
                        "Set record value slot is not undefined",
                    ));
                }
                if let Some(key) = &record.key {
                    if !is_map_storable_value(key) {
                        return Err(HeapError::Invariant(
                            "Set record contains an internal value sentinel",
                        ));
                    }
                    live = live.checked_add(1).ok_or(HeapError::Overflow {
                        operation: "validating Set size",
                    })?;
                }
            }
            if live != *size {
                return Err(HeapError::Invariant(
                    "Set live record count does not match its payload",
                ));
            }
        }
        if let ObjectPayload::SetIterator {
            object: Some(set), ..
        } = &object.payload
        {
            if !matches!(self.object(*set)?.payload, ObjectPayload::Set { .. }) {
                return Err(HeapError::Invariant(
                    "Set Iterator source does not have the Set class",
                ));
            }
        }
        Ok(())
    }

    fn validate_replacement_slot(
        &self,
        id: ObjectId,
        slot_index: usize,
        replacement: &PropertySlot,
    ) -> Result<(), HeapError> {
        let object = self.object(id)?;
        let shape = self.shape(object.shape)?;
        let entry = shape.entries().get(slot_index).ok_or(HeapError::Invariant(
            "property slot index is outside the object shape",
        ))?;
        if !slot_matches_storage(replacement, entry.flags.storage) {
            return Err(HeapError::Invariant(
                "replacement property storage does not match its shape flags",
            ));
        }
        if matches!(replacement, PropertySlot::Data(RawValue::Private(_))) {
            return Err(HeapError::Invariant(
                "private-name identity escaped into an object value slot",
            ));
        }
        Ok(())
    }

    fn retain_edges_transactionally(&mut self, edges: &[RawId]) -> Result<(), HeapError> {
        let mut counts = HashMap::<RawId, u32>::new();
        for &edge in edges {
            let count = counts.entry(edge).or_default();
            *count = count.checked_add(1).ok_or(HeapError::Overflow {
                operation: "counting outgoing heap edges",
            })?;
        }

        // A complete preflight makes the following increments infallible and
        // avoids a rollback path that could itself need to report atom cleanup.
        for (&edge, &additional) in &counts {
            let strong = self.live_node(edge)?.strong;
            strong.checked_add(additional).ok_or(HeapError::Overflow {
                operation: "retaining outgoing heap edges",
            })?;
        }
        for (edge, additional) in counts {
            self.retain_raw(edge, additional)?;
        }
        Ok(())
    }

    fn retain_raw(&mut self, id: RawId, additional: u32) -> Result<(), HeapError> {
        let node = self.live_node_mut(id)?;
        node.strong = node
            .strong
            .checked_add(additional)
            .ok_or(HeapError::Overflow {
                operation: "retaining a heap reference",
            })?;
        Ok(())
    }

    fn release_and_drain(&mut self, id: RawId) -> Result<HeapCleanup, HeapError> {
        self.release_raw_no_drain(id)?;
        self.drain_zero_queue()
    }

    fn release_raw_no_drain(&mut self, id: RawId) -> Result<(), HeapError> {
        let index = self.validate_slot_identity(id)?;
        let mut queue = false;
        let mut vacate_zombie = false;
        {
            let slot = &mut self.slots[index];
            match &mut slot.state {
                SlotState::Live(node) => {
                    node.strong = node.strong.checked_sub(1).ok_or(HeapError::Underflow {
                        kind: id.kind(),
                        index: id.index(),
                        generation: id.generation(),
                    })?;
                    if node.strong == 0 {
                        let state = std::mem::replace(&mut slot.state, SlotState::Vacant);
                        let SlotState::Live(node) = state else {
                            return Err(HeapError::Invariant(
                                "live node changed while entering the zero queue",
                            ));
                        };
                        slot.state = SlotState::ZeroQueued(node);
                        queue = true;
                    }
                }
                SlotState::Zombie { strong, .. } => {
                    *strong = strong.checked_sub(1).ok_or(HeapError::Underflow {
                        kind: id.kind(),
                        index: id.index(),
                        generation: id.generation(),
                    })?;
                    vacate_zombie = *strong == 0;
                }
                SlotState::Initializing { .. }
                | SlotState::ZeroQueued(_)
                | SlotState::Finalizing(_) => {
                    return Err(HeapError::Underflow {
                        kind: id.kind(),
                        index: id.index(),
                        generation: id.generation(),
                    });
                }
                SlotState::Vacant | SlotState::Retired => {
                    return Err(HeapError::Stale {
                        index: id.index(),
                        generation: id.generation(),
                    });
                }
            }
        }
        if queue {
            self.zero_queue.push_back(id);
        }
        if vacate_zombie {
            self.reclaim_slot(id.index())?;
        }
        Ok(())
    }

    fn drain_zero_queue(&mut self) -> Result<HeapCleanup, HeapError> {
        let mut cleanup = HeapCleanup::default();
        while let Some(id) = self.zero_queue.pop_front() {
            let index = self.validate_slot_identity(id)?;
            let node = {
                let slot = &mut self.slots[index];
                let state = std::mem::replace(&mut slot.state, SlotState::Vacant);
                let SlotState::ZeroQueued(node) = state else {
                    return Err(HeapError::Invariant(
                        "zero queue referenced a node not in ZeroQueued state",
                    ));
                };
                if node.strong != 0 {
                    return Err(HeapError::Invariant(
                        "zero queue contained a nonzero reference count",
                    ));
                }
                slot.state = SlotState::Finalizing(node);

                let state = std::mem::replace(&mut slot.state, SlotState::Vacant);
                let SlotState::Finalizing(node) = state else {
                    return Err(HeapError::Invariant(
                        "node left Finalizing state without a callback boundary",
                    ));
                };
                node
            };

            // A zero-count node cannot have an incoming self-edge, so its slot
            // can be recycled before outgoing edges are processed.
            self.finish_node(id, node, &mut cleanup)?;
            self.reclaim_vacant_slot(id.index())?;
        }
        Ok(cleanup)
    }

    fn finish_node(
        &mut self,
        id: RawId,
        node: Node,
        cleanup: &mut HeapCleanup,
    ) -> Result<(), HeapError> {
        match node.data {
            NodeData::Object(object) => {
                cleanup.finalized_objects = cleanup.finalized_objects.saturating_add(1);
                cleanup.atoms.extend(object_atoms(&object));
                for edge in object_edges(&object) {
                    self.release_raw_no_drain(edge)?;
                }
            }
            NodeData::Shape(shape) => {
                let RawId::Shape(shape_id) = id else {
                    return Err(HeapError::Invariant(
                        "shape payload finalized through a non-shape handle",
                    ));
                };
                cleanup.finalized_shapes = cleanup.finalized_shapes.saturating_add(1);
                cleanup.finalized_shape_ids.push(shape_id);
                cleanup
                    .atoms
                    .extend(shape.entries().iter().map(|entry| entry.atom));
                for edge in shape_edges(&shape) {
                    self.release_raw_no_drain(edge)?;
                }
            }
            NodeData::VarRef(var_ref) => {
                cleanup.finalized_var_refs = cleanup.finalized_var_refs.saturating_add(1);
                cleanup.atoms.extend(var_ref_atoms(&var_ref));
                for edge in var_ref_edges(&var_ref) {
                    self.release_raw_no_drain(edge)?;
                }
            }
            NodeData::Context(context) => {
                cleanup.finalized_contexts = cleanup.finalized_contexts.saturating_add(1);
                cleanup.atoms.extend(context_atoms(&context));
                for edge in context_edges(&context) {
                    self.release_raw_no_drain(edge)?;
                }
            }
            NodeData::FunctionBytecode(bytecode) => {
                cleanup.finalized_function_bytecodes =
                    cleanup.finalized_function_bytecodes.saturating_add(1);
                cleanup.atoms.extend(function_bytecode_atoms(&bytecode));
                for edge in function_bytecode_edges(&bytecode) {
                    self.release_raw_no_drain(edge)?;
                }
            }
        }
        Ok(())
    }

    fn finalize_cycle_anchor(
        &mut self,
        id: RawId,
        cleanup: &mut HeapCleanup,
    ) -> Result<(), HeapError> {
        if !matches!(id, RawId::Object(_) | RawId::FunctionBytecode(_)) {
            return Err(HeapError::Invariant(
                "non-anchor node entered active cycle finalization",
            ));
        }
        let index = self.validate_slot_identity(id)?;
        let node = {
            let slot = &mut self.slots[index];
            let state = std::mem::replace(&mut slot.state, SlotState::Vacant);
            let SlotState::Live(node) = state else {
                return Err(HeapError::Invariant(
                    "cycle anchor was not live when finalization began",
                ));
            };
            if node.data.kind() != id.kind() {
                return Err(HeapError::WrongKind {
                    expected: id.kind(),
                    actual: node.data.kind(),
                });
            }
            slot.state = SlotState::Zombie {
                kind: id.kind(),
                strong: node.strong,
            };
            node
        };
        self.finish_node(id, node, cleanup)
    }

    fn reclaim_slot(&mut self, index: u32) -> Result<(), HeapError> {
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(HeapError::Invariant("reclaimed slot disappeared"))?;
        match slot.state {
            SlotState::Zombie { strong: 0, .. } => slot.state = SlotState::Vacant,
            _ => {
                return Err(HeapError::Invariant(
                    "attempted to reclaim a nonzero or non-zombie slot",
                ));
            }
        }
        self.reclaim_vacant_slot(index)
    }

    fn reclaim_vacant_slot(&mut self, index: u32) -> Result<(), HeapError> {
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(HeapError::Invariant("reclaimed slot disappeared"))?;
        if !matches!(slot.state, SlotState::Vacant) {
            return Err(HeapError::Invariant(
                "generation advanced before node payload was detached",
            ));
        }
        if let Some(generation) = slot.generation.checked_add(1) {
            slot.generation = generation;
            self.free.push(index);
        } else {
            slot.state = SlotState::Retired;
        }
        Ok(())
    }

    fn validate_slot_identity(&self, id: RawId) -> Result<usize, HeapError> {
        let index = id.index() as usize;
        let slot = self.slots.get(index).ok_or(HeapError::Stale {
            index: id.index(),
            generation: id.generation(),
        })?;
        if slot.generation != id.generation() {
            return Err(HeapError::Stale {
                index: id.index(),
                generation: id.generation(),
            });
        }
        let actual = slot.state.kind().ok_or(HeapError::Stale {
            index: id.index(),
            generation: id.generation(),
        })?;
        if actual != id.kind() {
            return Err(HeapError::WrongKind {
                expected: id.kind(),
                actual,
            });
        }
        Ok(index)
    }

    fn live_index(&self, id: RawId) -> Result<usize, HeapError> {
        let index = self.validate_slot_identity(id)?;
        if !matches!(self.slots[index].state, SlotState::Live(_)) {
            return Err(HeapError::Invariant(
                "heap edge targeted a node outside Live state",
            ));
        }
        Ok(index)
    }

    fn live_node(&self, id: RawId) -> Result<&Node, HeapError> {
        let index = self.validate_slot_identity(id)?;
        match &self.slots[index].state {
            SlotState::Live(node) => Ok(node),
            _ => Err(HeapError::Stale {
                index: id.index(),
                generation: id.generation(),
            }),
        }
    }

    fn live_node_mut(&mut self, id: RawId) -> Result<&mut Node, HeapError> {
        let index = self.validate_slot_identity(id)?;
        match &mut self.slots[index].state {
            SlotState::Live(node) => Ok(node),
            _ => Err(HeapError::Stale {
                index: id.index(),
                generation: id.generation(),
            }),
        }
    }

    fn object_mut(&mut self, id: ObjectId) -> Result<&mut ObjectData, HeapError> {
        match &mut self.live_node_mut(RawId::Object(id))?.data {
            NodeData::Object(object) => Ok(object),
            NodeData::Shape(_)
            | NodeData::VarRef(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed object lookup reached another node payload",
            )),
        }
    }

    fn var_ref_mut(&mut self, id: VarRefId) -> Result<&mut VarRefData, HeapError> {
        match &mut self.live_node_mut(RawId::VarRef(id))?.data {
            NodeData::VarRef(var_ref) => Ok(var_ref),
            NodeData::Object(_)
            | NodeData::Shape(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed var-ref lookup reached another node payload",
            )),
        }
    }

    fn strong_count(&self, id: RawId) -> Result<u32, HeapError> {
        let index = self.validate_slot_identity(id)?;
        self.slots[index].state.strong().ok_or(HeapError::Stale {
            index: id.index(),
            generation: id.generation(),
        })
    }

    fn is_live(&self, id: RawId) -> bool {
        self.validate_slot_identity(id)
            .is_ok_and(|index| matches!(self.slots[index].state, SlotState::Live(_)))
    }
}

fn object_edges(object: &ObjectData) -> Vec<RawId> {
    let closure_count = match &object.payload {
        ObjectPayload::Ordinary
        | ObjectPayload::RawJson
        | ObjectPayload::Array { .. }
        | ObjectPayload::Arguments { .. }
        | ObjectPayload::ArrayIterator { .. }
        | ObjectPayload::ForInIterator(_)
        | ObjectPayload::Primitive(_)
        | ObjectPayload::Date(_)
        | ObjectPayload::RegExp(_)
        | ObjectPayload::GlobalObject { .. }
        | ObjectPayload::Error
        | ObjectPayload::StringIterator { .. }
        | ObjectPayload::Generator { .. } => 0,
        ObjectPayload::NativeFunction { internal, .. } => internal
            .as_ref()
            .map_or(0, |internal| internal_callable_edges(internal).len()),
        ObjectPayload::RegExpStringIterator { .. } => 1,
        ObjectPayload::Map { records, .. } => records
            .iter()
            .map(|record| {
                record
                    .key
                    .as_ref()
                    .map_or(0, |key| raw_value_edges(key).len())
                    .saturating_add(raw_value_edges(&record.value).len())
            })
            .sum(),
        ObjectPayload::MapIterator { .. } => 1,
        ObjectPayload::Set { records, .. } => records
            .iter()
            .filter_map(|record| record.key.as_ref())
            .map(|key| raw_value_edges(key).len())
            .sum(),
        ObjectPayload::SetIterator { .. } => 1,
        ObjectPayload::BoundFunction { arguments, .. } => arguments.len().saturating_add(2),
        ObjectPayload::BytecodeFunction { closure_slots, .. } => closure_slots.len(),
        ObjectPayload::Promise(data) => raw_value_edges(&data.result).len().saturating_add(
            data.fulfill_reactions
                .len()
                .saturating_add(data.reject_reactions.len())
                .saturating_mul(4),
        ),
    };
    let mut edges = Vec::with_capacity(
        object
            .slots
            .len()
            .saturating_add(closure_count)
            .saturating_add(3),
    );
    for slot in &object.slots {
        edges.extend(property_slot_edges(slot));
    }
    edges.push(RawId::Shape(object.shape));
    match &object.payload {
        ObjectPayload::Ordinary
        | ObjectPayload::RawJson
        | ObjectPayload::Array { .. }
        | ObjectPayload::Arguments { .. }
        | ObjectPayload::Primitive(_)
        | ObjectPayload::Date(_)
        | ObjectPayload::RegExp(_)
        | ObjectPayload::Error
        | ObjectPayload::StringIterator { .. } => {}
        ObjectPayload::RegExpStringIterator { regexp, .. } => {
            edges.push(RawId::Object(*regexp));
        }
        ObjectPayload::ArrayIterator { object, .. } => {
            edges.extend(object.map(RawId::Object));
        }
        ObjectPayload::Map { records, .. } => {
            for record in records {
                if let Some(key) = &record.key {
                    edges.extend(raw_value_edges(key));
                    edges.extend(raw_value_edges(&record.value));
                }
            }
        }
        ObjectPayload::MapIterator { object, .. } => {
            edges.extend(object.map(RawId::Object));
        }
        ObjectPayload::Set { records, .. } => {
            for record in records {
                if let Some(key) = &record.key {
                    edges.extend(raw_value_edges(key));
                }
            }
        }
        ObjectPayload::SetIterator { object, .. } => {
            edges.extend(object.map(RawId::Object));
        }
        ObjectPayload::ForInIterator(data) => {
            edges.extend(data.object.map(RawId::Object));
        }
        ObjectPayload::GlobalObject { uninitialized_vars } => {
            edges.push(RawId::Object(*uninitialized_vars))
        }
        ObjectPayload::NativeFunction { data, internal } => {
            edges.extend(data.realm.map(RawId::Context));
            if let Some(internal) = internal {
                edges.extend(internal_callable_edges(internal));
            }
        }
        ObjectPayload::BoundFunction {
            target,
            this_value,
            arguments,
        } => {
            edges.push(RawId::Object(*target));
            edges.extend(raw_value_edges(this_value));
            for argument in arguments.iter() {
                edges.extend(raw_value_edges(argument));
            }
        }
        ObjectPayload::BytecodeFunction {
            bytecode,
            home_object,
            class_instance_initializer,
            closure_slots,
            ..
        } => {
            if let Some(home_object) = home_object {
                edges.push(RawId::Object(*home_object));
            }
            if let Some(initializer) = class_instance_initializer {
                edges.push(RawId::Object(*initializer));
            }
            edges.push(RawId::FunctionBytecode(*bytecode));
            edges.extend(closure_slots.iter().copied().map(RawId::VarRef));
        }
        ObjectPayload::Generator { activation, .. } => {
            if let Some(activation) = activation.as_deref() {
                edges.extend(generator_activation_edges(activation));
            }
        }
        ObjectPayload::Promise(data) => {
            edges.extend(raw_value_edges(&data.result));
            for reaction in data.fulfill_reactions.iter().chain(&data.reject_reactions) {
                edges.extend(promise_reaction_edges(reaction));
            }
        }
    }
    edges
}

fn promise_capability_edges(capability: &PromiseCapabilityData) -> [RawId; 2] {
    [
        RawId::Object(capability.resolve),
        RawId::Object(capability.reject),
    ]
}

fn promise_reaction_edges(reaction: &PromiseReaction) -> Vec<RawId> {
    reaction
        .handler
        .map(RawId::Object)
        .into_iter()
        .chain(promise_capability_edges(&reaction.capability))
        .collect()
}

fn internal_callable_edges(internal: &InternalCallableData) -> Vec<RawId> {
    match internal {
        InternalCallableData::PromiseResolving { promise, .. } => {
            vec![RawId::Object(*promise)]
        }
        InternalCallableData::PromiseCapabilityExecutor(capture) => capture
            .resolve
            .iter()
            .chain(capture.reject.iter())
            .flat_map(raw_value_edges)
            .collect(),
    }
}

fn generator_activation_edges(activation: &GeneratorActivationData) -> Vec<RawId> {
    let vm = &activation.vm;
    let mut edges = Vec::with_capacity(
        vm.stack
            .len()
            .saturating_add(activation.arguments.len())
            .saturating_add(activation.locals.len())
            .saturating_add(8),
    );
    edges.push(RawId::FunctionBytecode(activation.bytecode));
    edges.push(RawId::Context(vm.callee_realm));
    edges.push(RawId::Object(vm.current_function));
    edges.push(RawId::Object(vm.callee_global));
    for value in vm
        .stack
        .iter()
        .chain(std::iter::once(&vm.this_value))
        .chain(vm.normalized_this.iter())
        .chain(std::iter::once(&vm.new_target))
    {
        edges.extend(raw_value_edges(value));
    }
    for binding in activation.arguments.iter().chain(activation.locals.iter()) {
        match binding {
            GeneratorFrameBinding::Direct(value) => edges.extend(raw_value_edges(value)),
            GeneratorFrameBinding::PrivateCallable(object) => {
                edges.push(RawId::Object(*object));
            }
            GeneratorFrameBinding::Captured(var_ref) => {
                edges.push(RawId::VarRef(*var_ref));
            }
            GeneratorFrameBinding::Private(_) | GeneratorFrameBinding::Uninitialized => {}
        }
    }
    edges
}

fn shape_edges(shape: &Shape) -> Vec<RawId> {
    shape
        .prototype()
        .map(|prototype| vec![RawId::Object(prototype)])
        .unwrap_or_default()
}

fn var_ref_edges(var_ref: &VarRefData) -> Vec<RawId> {
    raw_value_edges(&var_ref.value)
}

fn property_slot_edges(slot: &PropertySlot) -> Vec<RawId> {
    match slot {
        PropertySlot::Data(value) => raw_value_edges(value),
        PropertySlot::VarRef(var_ref) => vec![RawId::VarRef(*var_ref)],
        PropertySlot::Accessor { get, set } => get
            .iter()
            .chain(set.iter())
            .copied()
            .map(RawId::Object)
            .collect(),
        PropertySlot::AutoInit(
            AutoInitProperty::FunctionPrototype { realm }
            | AutoInitProperty::NativeBuiltin { realm, .. }
            | AutoInitProperty::String { realm, .. }
            | AutoInitProperty::ArrayUnscopables { realm }
            | AutoInitProperty::Math { realm }
            | AutoInitProperty::Reflect { realm }
            | AutoInitProperty::Json { realm },
        ) => vec![RawId::Context(*realm)],
        #[cfg(test)]
        PropertySlot::AutoInit(AutoInitProperty::FailureProbe { realm }) => {
            vec![RawId::Context(*realm)]
        }
    }
}

fn raw_value_edges(value: &RawValue) -> Vec<RawId> {
    match value {
        RawValue::Object(object) => vec![RawId::Object(*object)],
        RawValue::Undefined
        | RawValue::Null
        | RawValue::Bool(_)
        | RawValue::Int(_)
        | RawValue::Float(_)
        | RawValue::BigInt(_)
        | RawValue::String(_)
        | RawValue::Symbol(_)
        | RawValue::Private(_)
        | RawValue::Uninitialized
        | RawValue::Exception => Vec::new(),
    }
}

fn context_edges(context: &ContextData) -> Vec<RawId> {
    let mut edges = Vec::with_capacity(
        13usize
            .saturating_add(PrimitiveKind::COUNT)
            .saturating_add(NativeErrorKind::COUNT)
            .saturating_add(context.regexp.map_or(0, |_| 4))
            .saturating_add(context.map.map_or(0, |_| 2))
            .saturating_add(context.set.map_or(0, |_| 2))
            .saturating_add(context.generator.map_or(0, |_| 2))
            .saturating_add(context.promise.map_or(0, |_| 2))
            .saturating_add(context.global_objects.len())
            .saturating_add(context.intrinsics.len())
            .saturating_add(context.initial_shapes.len()),
    );
    edges.push(RawId::Object(context.object_prototype));
    edges.push(RawId::Object(context.function_prototype));
    edges.push(RawId::Object(context.array_prototype));
    edges.push(RawId::Object(context.iterator_prototype));
    edges.push(RawId::Object(context.array_iterator_prototype));
    edges.push(RawId::Object(context.string_iterator_prototype));
    edges.extend(
        context
            .primitive_prototypes
            .iter()
            .flatten()
            .copied()
            .map(RawId::Object),
    );
    edges.extend(context.date_prototype.map(RawId::Object));
    if let Some(regexp) = context.regexp {
        edges.push(RawId::Object(regexp.prototype));
        edges.push(RawId::Object(regexp.constructor));
        edges.push(RawId::Object(regexp.string_iterator_prototype));
        edges.push(RawId::Shape(regexp.object_shape));
    }
    if let Some(map) = context.map {
        edges.push(RawId::Object(map.prototype));
        edges.push(RawId::Object(map.iterator_prototype));
    }
    if let Some(set) = context.set {
        edges.push(RawId::Object(set.prototype));
        edges.push(RawId::Object(set.iterator_prototype));
    }
    if let Some(generator) = context.generator {
        edges.push(RawId::Object(generator.prototype));
        edges.push(RawId::Object(generator.function_prototype));
    }
    if let Some(promise) = context.promise {
        edges.push(RawId::Object(promise.prototype));
        edges.push(RawId::Object(promise.constructor));
    }
    edges.extend(context.function_constructor.map(RawId::Object));
    edges.extend(context.array_constructor.map(RawId::Object));
    edges.extend(context.array_prototype_values.map(RawId::Object));
    edges.extend(context.throw_type_error.map(RawId::Object));
    edges.extend(context.eval_function.map(RawId::Object));
    edges.push(RawId::Object(context.global_object));
    edges.push(RawId::Object(context.global_var_object));
    edges.extend(context.error_prototype.map(RawId::Object));
    edges.extend(
        context
            .native_error_prototypes
            .iter()
            .flatten()
            .copied()
            .map(RawId::Object),
    );
    edges.extend(context.global_objects.iter().copied().map(RawId::Object));
    for value in &context.intrinsics {
        edges.extend(raw_value_edges(value));
    }
    edges.extend(context.initial_shapes.iter().copied().map(RawId::Shape));
    edges
}

fn function_bytecode_edges(bytecode: &FunctionBytecodeData) -> Vec<RawId> {
    let mut edges = Vec::with_capacity(bytecode.constants.len().saturating_add(1));
    for constant in bytecode.constants.iter() {
        match constant {
            BytecodeConstant::Value(value) => edges.extend(raw_value_edges(value)),
            BytecodeConstant::RegExp { .. } => {}
            BytecodeConstant::Function(function) => {
                edges.push(RawId::FunctionBytecode(*function));
            }
        }
    }
    edges.push(RawId::Context(bytecode.realm));
    edges
}

fn property_slot_atoms(slot: &PropertySlot) -> impl Iterator<Item = Atom> + '_ {
    match slot {
        PropertySlot::Data(RawValue::Symbol(atom) | RawValue::Private(atom)) => Some(*atom),
        PropertySlot::Data(_)
        | PropertySlot::VarRef(_)
        | PropertySlot::Accessor { .. }
        | PropertySlot::AutoInit(_) => None,
    }
    .into_iter()
}

fn object_slot_atoms(object: &ObjectData) -> impl Iterator<Item = Atom> + '_ {
    object.slots.iter().flat_map(property_slot_atoms)
}

fn object_atoms(object: &ObjectData) -> impl Iterator<Item = Atom> + '_ {
    let payload = match &object.payload {
        ObjectPayload::Primitive(PrimitiveObjectData::Symbol(atom)) => vec![*atom],
        ObjectPayload::Primitive(
            PrimitiveObjectData::Number(_)
            | PrimitiveObjectData::String(_)
            | PrimitiveObjectData::Boolean(_)
            | PrimitiveObjectData::BigInt(_),
        ) => Vec::new(),
        ObjectPayload::BoundFunction {
            this_value,
            arguments,
            ..
        } => raw_value_atom(this_value)
            .into_iter()
            .chain(arguments.iter().filter_map(raw_value_atom))
            .collect::<Vec<_>>(),
        ObjectPayload::Map { records, .. } => records
            .iter()
            .flat_map(|record| {
                record
                    .key
                    .as_ref()
                    .and_then(raw_value_atom)
                    .into_iter()
                    .chain(raw_value_atom(&record.value))
            })
            .collect::<Vec<_>>(),
        ObjectPayload::Set { records, .. } => records
            .iter()
            .filter_map(|record| record.key.as_ref().and_then(raw_value_atom))
            .collect::<Vec<_>>(),
        ObjectPayload::Generator { activation, .. } => activation
            .as_deref()
            .map(generator_activation_atoms)
            .unwrap_or_default(),
        ObjectPayload::Promise(data) => raw_value_atom(&data.result).into_iter().collect(),
        ObjectPayload::NativeFunction {
            internal: Some(InternalCallableData::PromiseCapabilityExecutor(capture)),
            ..
        } => capture
            .resolve
            .iter()
            .chain(capture.reject.iter())
            .filter_map(raw_value_atom)
            .collect(),
        ObjectPayload::Ordinary
        | ObjectPayload::RawJson
        | ObjectPayload::Array { .. }
        | ObjectPayload::Arguments { .. }
        | ObjectPayload::ArrayIterator { .. }
        | ObjectPayload::ForInIterator(_)
        | ObjectPayload::Date(_)
        | ObjectPayload::RegExp(_)
        | ObjectPayload::RegExpStringIterator { .. }
        | ObjectPayload::MapIterator { .. }
        | ObjectPayload::SetIterator { .. }
        | ObjectPayload::GlobalObject { .. }
        | ObjectPayload::Error
        | ObjectPayload::StringIterator { .. }
        | ObjectPayload::NativeFunction { .. }
        | ObjectPayload::BytecodeFunction { .. } => Vec::new(),
    };
    object_slot_atoms(object)
        .chain(payload)
        .chain(object.private_brand_home)
}

fn generator_activation_atoms(activation: &GeneratorActivationData) -> Vec<Atom> {
    let vm = &activation.vm;
    vm.stack
        .iter()
        .chain(std::iter::once(&vm.this_value))
        .chain(vm.normalized_this.iter())
        .chain(std::iter::once(&vm.new_target))
        .filter_map(raw_value_atom)
        .chain(
            activation
                .arguments
                .iter()
                .chain(activation.locals.iter())
                .filter_map(|binding| match binding {
                    GeneratorFrameBinding::Direct(value) => raw_value_atom(value),
                    GeneratorFrameBinding::Private(atom) => Some(*atom),
                    GeneratorFrameBinding::PrivateCallable(_)
                    | GeneratorFrameBinding::Uninitialized
                    | GeneratorFrameBinding::Captured(_) => None,
                }),
        )
        .collect()
}

fn raw_value_atom(value: &RawValue) -> Option<Atom> {
    match value {
        RawValue::Symbol(atom) | RawValue::Private(atom) => Some(*atom),
        RawValue::Undefined
        | RawValue::Null
        | RawValue::Bool(_)
        | RawValue::Int(_)
        | RawValue::Float(_)
        | RawValue::BigInt(_)
        | RawValue::String(_)
        | RawValue::Object(_)
        | RawValue::Uninitialized
        | RawValue::Exception => None,
    }
}

const fn is_map_storable_value(value: &RawValue) -> bool {
    !matches!(
        value,
        RawValue::Private(_) | RawValue::Uninitialized | RawValue::Exception
    )
}

const fn is_promise_storable_value(value: &RawValue) -> bool {
    !matches!(
        value,
        RawValue::Private(_) | RawValue::Uninitialized | RawValue::Exception
    )
}

fn validate_var_ref_payload(var_ref: &VarRefData) -> Result<(), HeapError> {
    validate_var_ref_value(
        var_ref.kind,
        var_ref.is_lexical,
        var_ref.is_const,
        &var_ref.value,
    )
}

fn validate_var_ref_value(
    kind: ClosureVariableKind,
    is_lexical: bool,
    is_const: bool,
    value: &RawValue,
) -> Result<(), HeapError> {
    if kind.is_private() && (!is_lexical || !is_const) {
        return Err(HeapError::Invariant(
            "private-element VarRef is not an immutable lexical binding",
        ));
    }
    match kind {
        ClosureVariableKind::PrivateField
            if !matches!(value, RawValue::Private(_) | RawValue::Uninitialized) =>
        {
            return Err(HeapError::Invariant(
                "private-name VarRef contains an ordinary ECMAScript value",
            ));
        }
        ClosureVariableKind::PrivateMethod
        | ClosureVariableKind::PrivateGetter
        | ClosureVariableKind::PrivateSetter
        | ClosureVariableKind::PrivateGetterSetter
            if !matches!(value, RawValue::Object(_) | RawValue::Uninitialized) =>
        {
            return Err(HeapError::Invariant(
                "private-method VarRef contains a non-callable representation",
            ));
        }
        _ => {}
    }
    if !kind.is_private() && matches!(value, RawValue::Private(_)) {
        return Err(HeapError::Invariant(
            "private-name identity escaped into an ordinary VarRef",
        ));
    }
    Ok(())
}

fn context_atoms(context: &ContextData) -> impl Iterator<Item = Atom> + '_ {
    context.intrinsics.iter().filter_map(raw_value_atom)
}

fn function_bytecode_atoms(bytecode: &FunctionBytecodeData) -> impl Iterator<Item = Atom> + '_ {
    bytecode
        .auxiliary_atoms
        .iter()
        .copied()
        .chain(
            bytecode
                .constants
                .iter()
                .filter_map(|constant| match constant {
                    BytecodeConstant::Value(value) => raw_value_atom(value),
                    BytecodeConstant::RegExp { .. } | BytecodeConstant::Function(_) => None,
                }),
        )
}

fn var_ref_atoms(var_ref: &VarRefData) -> impl Iterator<Item = Atom> + '_ {
    raw_value_atom(&var_ref.value).into_iter()
}

const fn slot_matches_storage(slot: &PropertySlot, storage: PropertyStorageKind) -> bool {
    matches!(
        (slot, storage),
        (PropertySlot::Data(_), PropertyStorageKind::Data)
            | (PropertySlot::VarRef(_), PropertyStorageKind::Data)
            | (PropertySlot::AutoInit(_), PropertyStorageKind::Data)
            | (PropertySlot::Accessor { .. }, PropertyStorageKind::Accessor)
    )
}

fn increment_kind_count(counts: &mut HeapCounts, kind: HeapNodeKind) {
    match kind {
        HeapNodeKind::Object => counts.object_nodes = counts.object_nodes.saturating_add(1),
        HeapNodeKind::Shape => counts.shape_nodes = counts.shape_nodes.saturating_add(1),
        HeapNodeKind::VarRef => counts.var_ref_nodes = counts.var_ref_nodes.saturating_add(1),
        HeapNodeKind::Context => counts.context_nodes = counts.context_nodes.saturating_add(1),
        HeapNodeKind::FunctionBytecode => {
            counts.function_bytecode_nodes = counts.function_bytecode_nodes.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::LineColumn;
    use crate::shape::{PropertyFlags, ShapeEntry};

    const DATA_FLAGS: PropertyFlags = PropertyFlags::data(true, true, true);

    fn empty_shape(heap: &mut Heap) -> ShapeId {
        heap.allocate_shape(Shape::new(None, []).unwrap()).unwrap()
    }

    #[test]
    fn provisional_native_function_is_confined_to_transactional_realm_bootstrap() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let unbound = || {
            ObjectData::native_function(shape, Vec::new(), NativeFunctionId::FunctionPrototype, 0)
        };

        assert_eq!(
            heap.allocate_object(unbound()),
            Err(HeapError::Invariant(
                "an unbound native function may only be allocated during realm bootstrap"
            ))
        );
        assert_eq!(heap.counts().object_nodes, 0);

        assert_eq!(
            heap.allocate_bootstrap_native_function(ObjectData::native_function(
                shape,
                Vec::new(),
                NativeFunctionId::ErrorIsError,
                1,
            )),
            Err(HeapError::Invariant(
                "bootstrap native-function allocation requires an unbound Function.prototype"
            ))
        );
        assert_eq!(heap.counts().object_nodes, 0);

        let prototype = heap
            .allocate_object(ObjectData::ordinary(shape, Vec::new()))
            .unwrap();
        let regexp = heap
            .allocate_object(ObjectData::regexp(shape, Vec::new()))
            .unwrap();
        let native_shape = heap
            .allocate_shape(Shape::new(Some(prototype), []).unwrap())
            .unwrap();
        let function = heap
            .allocate_bootstrap_native_function(ObjectData::native_function(
                native_shape,
                Vec::new(),
                NativeFunctionId::FunctionPrototype,
                0,
            ))
            .unwrap();
        let realm = heap
            .allocate_context(ContextData::new(
                prototype, function, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();

        heap.attach_native_function_realm(function, realm).unwrap();
        assert_eq!(heap.context_strong_count(realm), Ok(2));
        assert!(heap.attach_native_function_realm(function, realm).is_err());
        assert_eq!(heap.context_strong_count(realm), Ok(2));
        assert!(heap.attach_native_function_realm(prototype, realm).is_err());
        assert_eq!(heap.context_strong_count(realm), Ok(2));
        assert!(heap.attach_native_function_realm(regexp, realm).is_err());
        assert_eq!(heap.context_strong_count(realm), Ok(2));

        heap.release_context(realm).unwrap();
        heap.release_object(function).unwrap();
        heap.release_object(regexp).unwrap();
        heap.release_object(prototype).unwrap();
        heap.release_shape(native_shape).unwrap();
        heap.release_shape(shape).unwrap();
        heap.run_gc().unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn math_random_state_is_realm_local_seeded_and_pinned() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = heap
            .allocate_object(ObjectData::ordinary(shape, Vec::new()))
            .unwrap();
        let first_realm = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        let second_realm = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();

        assert_eq!(
            heap.next_math_random_u64(first_realm),
            Err(HeapError::Invariant(
                "Math.random state was used before initialization"
            ))
        );
        heap.initialize_math_random_state(first_realm, 1).unwrap();
        heap.initialize_math_random_state(second_realm, 1).unwrap();
        assert_eq!(
            heap.initialize_math_random_state(first_realm, 2),
            Err(HeapError::Invariant(
                "Math.random state was initialized more than once"
            ))
        );
        let first = heap.next_math_random_u64(first_realm).unwrap();
        assert_eq!(first, 0x47e4_ce4b_896c_dd1d);
        assert_eq!(heap.next_math_random_u64(second_realm).unwrap(), first);
        let second = heap.next_math_random_u64(first_realm).unwrap();
        assert_eq!(heap.next_math_random_u64(second_realm).unwrap(), second);

        heap.release_context(first_realm).unwrap();
        heap.release_context(second_realm).unwrap();
        heap.release_object(prototype).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn primitive_object_payload_category_is_structurally_validated() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let number_payload = PrimitiveObjectData::Number(f64::NAN);
        assert_eq!(number_payload.kind(), PrimitiveKind::Number);
        assert_eq!(
            PrimitiveObjectData::String(JsString::try_from_utf16([0x61, 0xd800, 0x62]).unwrap(),)
                .kind(),
            PrimitiveKind::String
        );
        assert_eq!(
            PrimitiveObjectData::Boolean(false).kind(),
            PrimitiveKind::Boolean
        );
        let symbol_atom = Atom::from_immediate_integer(47).unwrap();
        assert_eq!(
            PrimitiveObjectData::Symbol(symbol_atom).kind(),
            PrimitiveKind::Symbol
        );
        assert_eq!(
            PrimitiveObjectData::BigInt(JsBigInt::one()).kind(),
            PrimitiveKind::BigInt
        );

        let mut invalid = ObjectData::primitive(shape, Vec::new(), number_payload.clone());
        invalid.kind = ObjectKind::Ordinary;
        assert_eq!(
            heap.allocate_object(invalid),
            Err(HeapError::Invariant(
                "object kind does not match its class payload"
            ))
        );
        assert_eq!(heap.counts().object_nodes, 0);

        let boolean = heap
            .allocate_object(ObjectData::primitive(
                shape,
                Vec::new(),
                PrimitiveObjectData::Boolean(false),
            ))
            .unwrap();
        assert!(matches!(
            heap.object(boolean).unwrap().payload,
            ObjectPayload::Primitive(PrimitiveObjectData::Boolean(false))
        ));

        let number = heap
            .allocate_object(ObjectData::primitive(shape, Vec::new(), number_payload))
            .unwrap();
        let number_data = heap.object(number).unwrap();
        assert!(matches!(
            &number_data.payload,
            ObjectPayload::Primitive(PrimitiveObjectData::Number(value)) if value.is_nan()
        ));
        assert_eq!(object_edges(number_data), vec![RawId::Shape(shape)]);
        assert_eq!(object_atoms(number_data).count(), 0);

        let string_value = JsString::try_from_utf16([0x61, 0xd800, 0x62]).unwrap();
        let string = heap
            .allocate_object(ObjectData::primitive(
                shape,
                Vec::new(),
                PrimitiveObjectData::String(string_value.clone()),
            ))
            .unwrap();
        let string_data = heap.object(string).unwrap();
        assert!(matches!(
            &string_data.payload,
            ObjectPayload::Primitive(PrimitiveObjectData::String(value))
                if value == &string_value
        ));
        assert_eq!(object_edges(string_data), vec![RawId::Shape(shape)]);
        assert_eq!(object_atoms(string_data).count(), 0);

        let symbol = heap
            .allocate_object(ObjectData::primitive(
                shape,
                Vec::new(),
                PrimitiveObjectData::Symbol(symbol_atom),
            ))
            .unwrap();
        let symbol_data = heap.object(symbol).unwrap();
        assert!(matches!(
            symbol_data.payload,
            ObjectPayload::Primitive(PrimitiveObjectData::Symbol(atom)) if atom == symbol_atom
        ));
        assert_eq!(object_edges(symbol_data), vec![RawId::Shape(shape)]);
        assert_eq!(object_atoms(symbol_data).collect::<Vec<_>>(), [symbol_atom]);

        let bigint = heap
            .allocate_object(ObjectData::primitive(
                shape,
                Vec::new(),
                PrimitiveObjectData::BigInt(JsBigInt::from(i64::MAX)),
            ))
            .unwrap();
        let bigint_data = heap.object(bigint).unwrap();
        assert!(matches!(
            &bigint_data.payload,
            ObjectPayload::Primitive(PrimitiveObjectData::BigInt(value))
                if value == &JsBigInt::from(i64::MAX)
        ));
        assert_eq!(object_edges(bigint_data), vec![RawId::Shape(shape)]);
        assert_eq!(object_atoms(bigint_data).count(), 0);

        heap.release_object(bigint).unwrap();
        let symbol_cleanup = heap.release_object(symbol).unwrap();
        assert_eq!(symbol_cleanup.atoms, [symbol_atom]);
        let string_cleanup = heap.release_object(string).unwrap();
        assert!(string_cleanup.atoms.is_empty());
        heap.release_object(number).unwrap();
        heap.release_object(boolean).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn date_payload_is_branded_edge_free_and_mutable() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);

        let mut invalid = ObjectData::date(shape, Vec::new(), f64::NAN);
        invalid.kind = ObjectKind::Ordinary;
        assert_eq!(
            heap.allocate_object(invalid),
            Err(HeapError::Invariant(
                "object kind does not match its class payload"
            ))
        );
        assert_eq!(heap.counts().object_nodes, 0);

        let date = heap
            .allocate_object(ObjectData::date(shape, Vec::new(), f64::NAN))
            .unwrap();
        assert!(heap.date_value(date).unwrap().is_nan());
        let date_data = heap.object(date).unwrap();
        assert!(matches!(date_data.payload, ObjectPayload::Date(value) if value.is_nan()));
        assert_eq!(object_edges(date_data), vec![RawId::Shape(shape)]);
        assert_eq!(object_atoms(date_data).count(), 0);

        heap.set_date_value(date, -0.0).unwrap();
        assert_eq!(
            heap.date_value(date).unwrap().to_bits(),
            (-0.0f64).to_bits()
        );

        let ordinary = heap
            .allocate_object(ObjectData::ordinary(shape, Vec::new()))
            .unwrap();
        assert_eq!(
            heap.date_value(ordinary),
            Err(HeapError::Invariant(
                "Date value requested for an object with the wrong class"
            ))
        );
        assert_eq!(
            heap.set_date_value(ordinary, 1.0),
            Err(HeapError::Invariant(
                "Date value update reached an object with the wrong class"
            ))
        );

        heap.release_object(ordinary).unwrap();
        heap.release_object(date).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn regexp_payload_is_branded_edge_free_and_structurally_validated() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);

        let mut invalid = ObjectData::regexp(shape, Vec::new());
        invalid.kind = ObjectKind::Ordinary;
        assert_eq!(
            heap.allocate_object(invalid),
            Err(HeapError::Invariant(
                "object kind does not match its class payload"
            ))
        );
        assert_eq!(heap.counts().object_nodes, 0);

        let regexp = heap
            .allocate_object(ObjectData::regexp(shape, Vec::new()))
            .unwrap();
        let regexp_object = heap.object(regexp).unwrap();
        assert_eq!(regexp_object.kind, ObjectKind::RegExp);
        assert!(matches!(
            regexp_object.payload,
            ObjectPayload::RegExp(RegExpObjectData::Uninitialized)
        ));
        assert!(matches!(
            heap.regexp_data(regexp),
            Ok(RegExpObjectData::Uninitialized)
        ));
        assert_eq!(object_edges(regexp_object), vec![RawId::Shape(shape)]);
        assert_eq!(object_atoms(regexp_object).count(), 0);

        let ordinary = heap
            .allocate_object(ObjectData::ordinary(shape, Vec::new()))
            .unwrap();
        assert_eq!(
            heap.regexp_data(ordinary),
            Err(HeapError::Invariant(
                "RegExp data requested for an object with the wrong class"
            ))
        );
        assert_eq!(
            heap.replace_regexp_data(ordinary, RegExpObjectData::Uninitialized),
            Err(HeapError::Invariant(
                "RegExp data update reached an object with the wrong class"
            ))
        );

        heap.release_object(ordinary).unwrap();
        heap.release_object(regexp).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn compiled_regexp_payload_replacement_and_finalization_release_rc_leaves() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let pattern = JsString::try_from_utf16([u16::from(b'a'), 0xd800, u16::from(b'b')]).unwrap();
        let flags = JsString::from_static("");
        let program = Rc::new(crate::regexp::compile(&pattern, &flags).unwrap());
        assert_eq!(Rc::strong_count(&program), 1);

        let regexp = heap
            .allocate_object(ObjectData::regexp(shape, Vec::new()))
            .unwrap();
        let previous = heap
            .replace_regexp_data(
                regexp,
                RegExpObjectData::Compiled {
                    pattern: pattern.clone(),
                    program: program.clone(),
                },
            )
            .unwrap();
        assert_eq!(previous, RegExpObjectData::Uninitialized);
        assert_eq!(Rc::strong_count(&program), 2);
        let RegExpObjectData::Compiled {
            pattern: stored_pattern,
            program: stored_program,
        } = heap.regexp_data(regexp).unwrap()
        else {
            panic!("RegExp replacement did not install compiled data");
        };
        assert_eq!(stored_pattern, &pattern);
        assert!(Rc::ptr_eq(stored_program, &program));
        assert_eq!(
            object_edges(heap.object(regexp).unwrap()),
            vec![RawId::Shape(shape)]
        );
        assert_eq!(object_atoms(heap.object(regexp).unwrap()).count(), 0);

        let precompiled = heap
            .allocate_object(ObjectData::compiled_regexp(
                shape,
                Vec::new(),
                pattern.clone(),
                program.clone(),
            ))
            .unwrap();
        assert_eq!(Rc::strong_count(&program), 3);
        heap.release_object(precompiled).unwrap();
        assert_eq!(Rc::strong_count(&program), 2);

        let previous = heap
            .replace_regexp_data(regexp, RegExpObjectData::Uninitialized)
            .unwrap();
        assert_eq!(Rc::strong_count(&program), 2);
        let RegExpObjectData::Compiled {
            pattern: previous_pattern,
            program: previous_program,
        } = previous
        else {
            panic!("RegExp replacement did not return the previous compiled data");
        };
        assert_eq!(previous_pattern, pattern);
        assert!(Rc::ptr_eq(&previous_program, &program));
        drop(previous_program);
        assert_eq!(Rc::strong_count(&program), 1);

        heap.release_object(regexp).unwrap();
        assert_eq!(Rc::strong_count(&program), 1);
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn string_iterator_payload_advances_by_code_point_and_releases_at_end() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let source = JsString::try_from_utf16([
            u16::from(b'A'),
            0xd83d,
            0xde00,
            0xd800,
            u16::from(b'X'),
            0xdc00,
            u16::from(b'Z'),
        ])
        .unwrap();
        let iterator = heap
            .allocate_object(ObjectData::string_iterator(shape, Vec::new(), source))
            .unwrap();

        let mut next = || {
            heap.string_iterator_next(iterator)
                .unwrap()
                .map(|value| value.utf16_units().collect::<Vec<_>>())
        };
        assert_eq!(next(), Some(vec![u16::from(b'A')]));
        assert_eq!(next(), Some(vec![0xd83d, 0xde00]));
        assert_eq!(next(), Some(vec![0xd800]));
        assert_eq!(next(), Some(vec![u16::from(b'X')]));
        assert_eq!(next(), Some(vec![0xdc00]));
        assert_eq!(next(), Some(vec![u16::from(b'Z')]));
        assert_eq!(next(), None);
        assert_eq!(next(), None);
        assert!(matches!(
            &heap.object(iterator).unwrap().payload,
            ObjectPayload::StringIterator {
                string: None,
                next_index: 7
            }
        ));
        assert_eq!(
            object_edges(heap.object(iterator).unwrap()),
            vec![RawId::Shape(shape)]
        );

        heap.release_object(iterator).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn regexp_string_iterator_retains_matcher_after_completion_until_finalization() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let regexp = leaf(&mut heap, shape);
        let string = JsString::from_static("input");
        let iterator = heap
            .allocate_object(ObjectData::regexp_string_iterator(
                shape,
                Vec::new(),
                regexp,
                string.clone(),
                true,
                true,
            ))
            .unwrap();

        assert_eq!(heap.object_strong_count(regexp), Ok(2));
        heap.release_object(regexp).unwrap();
        assert_eq!(heap.object_strong_count(regexp), Ok(1));
        assert_eq!(
            heap.regexp_string_iterator_state(iterator),
            Ok((regexp, string, true, true, false))
        );
        assert_eq!(
            object_edges(heap.object(iterator).unwrap()),
            vec![RawId::Shape(shape), RawId::Object(regexp)]
        );

        heap.finish_regexp_string_iterator(iterator).unwrap();
        assert_eq!(
            heap.regexp_string_iterator_state(iterator),
            Ok((regexp, JsString::from_static("input"), true, true, true))
        );
        assert_eq!(heap.object_strong_count(regexp), Ok(1));
        heap.finish_regexp_string_iterator(iterator).unwrap();
        assert_eq!(heap.object_strong_count(regexp), Ok(1));

        let ordinary = leaf(&mut heap, shape);
        assert!(matches!(
            heap.regexp_string_iterator_state(ordinary),
            Err(HeapError::Invariant(
                "RegExp String Iterator state reached an object with the wrong class"
            ))
        ));
        assert!(matches!(
            heap.finish_regexp_string_iterator(ordinary),
            Err(HeapError::Invariant(
                "RegExp String Iterator completion reached an object with the wrong class"
            ))
        ));
        heap.release_object(ordinary).unwrap();

        let cleanup = heap.release_object(iterator).unwrap();
        assert_eq!(cleanup.finalized_objects, 2);
        assert!(matches!(heap.object(regexp), Err(HeapError::Stale { .. })));
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn array_iterator_payload_retains_its_source_until_completion() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let source = leaf(&mut heap, shape);
        let iterator = heap
            .allocate_object(ObjectData::array_iterator(
                shape,
                Vec::new(),
                source,
                ArrayIteratorKind::KeyAndValue,
            ))
            .unwrap();

        assert_eq!(heap.object_strong_count(source), Ok(2));
        heap.release_object(source).unwrap();
        assert_eq!(heap.object_strong_count(source), Ok(1));
        assert_eq!(
            heap.array_iterator_state(iterator),
            Ok((Some(source), 0, ArrayIteratorKind::KeyAndValue))
        );
        heap.set_array_iterator_index(iterator, 7).unwrap();
        assert_eq!(
            heap.array_iterator_state(iterator),
            Ok((Some(source), 7, ArrayIteratorKind::KeyAndValue))
        );

        let cleanup = heap.finish_array_iterator(iterator).unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert!(matches!(heap.object(source), Err(HeapError::Stale { .. })));
        assert_eq!(
            heap.array_iterator_state(iterator),
            Ok((None, 7, ArrayIteratorKind::KeyAndValue))
        );
        assert_eq!(
            heap.finish_array_iterator(iterator).unwrap(),
            HeapCleanup::default()
        );
        assert!(matches!(
            heap.set_array_iterator_index(iterator, 8),
            Err(HeapError::Invariant(
                "completed Array Iterator was advanced"
            ))
        ));

        heap.release_object(iterator).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn map_records_retain_gc_edges_and_delete_releases_them() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let map = heap
            .allocate_object(ObjectData::map(shape, Vec::new()))
            .unwrap();
        let key = leaf(&mut heap, shape);
        let initial_value = leaf(&mut heap, shape);
        let replacement_value = leaf(&mut heap, shape);

        assert_eq!(
            heap.map_insert_record(map, RawValue::Object(key), RawValue::Object(initial_value),),
            Ok(HeapCleanup::default())
        );
        assert_eq!(heap.map_size(map), Ok(1));
        assert_eq!(heap.object_strong_count(key), Ok(2));
        assert_eq!(heap.object_strong_count(initial_value), Ok(2));
        heap.release_object(key).unwrap();
        heap.release_object(initial_value).unwrap();

        let cleanup = heap
            .map_replace_record_value(map, 0, RawValue::Object(replacement_value))
            .unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert!(matches!(
            heap.object(initial_value),
            Err(HeapError::Stale { .. })
        ));
        assert_eq!(heap.object_strong_count(replacement_value), Ok(2));
        heap.release_object(replacement_value).unwrap();

        let cleanup = heap.map_delete_record(map, 0).unwrap();
        assert_eq!(cleanup.finalized_objects, 2);
        assert!(matches!(heap.object(key), Err(HeapError::Stale { .. })));
        assert!(matches!(
            heap.object(replacement_value),
            Err(HeapError::Stale { .. })
        ));
        assert_eq!(heap.map_size(map), Ok(0));
        assert_eq!(
            heap.map_records(map),
            Ok(&[MapRecord {
                key: None,
                value: RawValue::Undefined,
            }][..])
        );

        heap.release_object(map).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn map_tombstones_preserve_readd_order_and_live_iterator_sees_appends() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let map = heap
            .allocate_object(ObjectData::map(shape, Vec::new()))
            .unwrap();
        heap.map_insert_record(
            map,
            RawValue::Int(1),
            RawValue::String(JsString::from_static("one")),
        )
        .unwrap();
        let iterator = heap
            .allocate_object(ObjectData::map_iterator(
                shape,
                Vec::new(),
                map,
                MapIteratorKind::KeyAndValue,
            ))
            .unwrap();

        heap.set_map_iterator_index(iterator, 1).unwrap();
        heap.map_insert_record(
            map,
            RawValue::Int(2),
            RawValue::String(JsString::from_static("first")),
        )
        .unwrap();
        assert_eq!(
            heap.map_records(map).unwrap()[1].key,
            Some(RawValue::Int(2))
        );
        heap.map_delete_record(map, 1).unwrap();
        heap.map_insert_record(
            map,
            RawValue::Int(2),
            RawValue::String(JsString::from_static("second")),
        )
        .unwrap();

        let records = heap.map_records(map).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].key, Some(RawValue::Int(1)));
        assert_eq!(records[1].key, None);
        assert_eq!(records[1].value, RawValue::Undefined);
        assert_eq!(records[2].key, Some(RawValue::Int(2)));
        assert_eq!(
            records[2].value,
            RawValue::String(JsString::from_static("second"))
        );
        let (source, next_index, kind) = heap.map_iterator_state(iterator).unwrap();
        assert_eq!(source, Some(map));
        assert_eq!(next_index, 1);
        assert_eq!(kind, MapIteratorKind::KeyAndValue);
        assert_eq!(
            records[next_index..]
                .iter()
                .find_map(|record| record.key.as_ref()),
            Some(&RawValue::Int(2))
        );

        assert_eq!(heap.object_strong_count(map), Ok(2));
        heap.release_object(map).unwrap();
        assert_eq!(heap.object_strong_count(map), Ok(1));
        let cleanup = heap.finish_map_iterator(iterator).unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert!(matches!(heap.object(map), Err(HeapError::Stale { .. })));
        assert_eq!(
            heap.map_iterator_state(iterator),
            Ok((None, 1, MapIteratorKind::KeyAndValue))
        );
        assert!(matches!(
            heap.set_map_iterator_index(iterator, 2),
            Err(HeapError::Invariant("completed Map Iterator was advanced"))
        ));

        heap.release_object(iterator).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn map_symbol_atoms_transfer_and_return_on_replace_delete_and_clear() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let map = heap
            .allocate_object(ObjectData::map(shape, Vec::new()))
            .unwrap();
        let first_key = Atom::from_immediate_integer(101).unwrap();
        let first_value = Atom::from_immediate_integer(102).unwrap();
        let replacement = Atom::from_immediate_integer(103).unwrap();
        let second_key = Atom::from_immediate_integer(104).unwrap();
        let second_value = Atom::from_immediate_integer(105).unwrap();

        heap.map_insert_record(
            map,
            RawValue::Symbol(first_key),
            RawValue::Symbol(first_value),
        )
        .unwrap();
        let cleanup = heap
            .map_replace_record_value(map, 0, RawValue::Symbol(replacement))
            .unwrap();
        assert_eq!(cleanup.atoms, vec![first_value]);
        heap.map_insert_record(
            map,
            RawValue::Symbol(second_key),
            RawValue::Symbol(second_value),
        )
        .unwrap();

        let cleanup = heap.map_delete_record(map, 0).unwrap();
        assert_eq!(cleanup.atoms, vec![first_key, replacement]);
        let cleanup = heap.map_clear(map).unwrap();
        assert_eq!(cleanup.atoms, vec![second_key, second_value]);
        assert_eq!(heap.map_size(map), Ok(0));

        heap.release_object(map).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn map_intrinsics_attach_transactionally_and_root_the_realm_graph() {
        let mut heap = Heap::new();
        let empty_shape = empty_shape(&mut heap);
        let root = leaf(&mut heap, empty_shape);
        let realm = heap
            .allocate_context(ContextData::new(
                root, root, root, root, root, root, root, root,
            ))
            .unwrap();
        let intrinsic_shape = heap
            .allocate_shape(Shape::new(Some(root), []).unwrap())
            .unwrap();
        let prototype = heap
            .allocate_object(ObjectData::ordinary(intrinsic_shape, Vec::new()))
            .unwrap();
        let iterator_prototype = heap
            .allocate_object(ObjectData::ordinary(intrinsic_shape, Vec::new()))
            .unwrap();
        let constructor = heap
            .allocate_object(ObjectData::bound_native_function(
                intrinsic_shape,
                Vec::new(),
                NativeFunctionId::Map(MapNativeKind::Constructor),
                realm,
                0,
            ))
            .unwrap();
        let map = MapRealmData {
            prototype,
            iterator_prototype,
        };
        let prototype_strong = heap.object_strong_count(prototype).unwrap();
        let constructor_strong = heap.object_strong_count(constructor).unwrap();
        let iterator_strong = heap.object_strong_count(iterator_prototype).unwrap();

        heap.live_node_mut(RawId::Object(iterator_prototype))
            .unwrap()
            .strong = u32::MAX;
        assert_eq!(
            heap.attach_map_intrinsics(realm, map),
            Err(HeapError::Overflow {
                operation: "retaining outgoing heap edges",
            })
        );
        assert_eq!(heap.context(realm).unwrap().map, None);
        assert_eq!(heap.object_strong_count(prototype), Ok(prototype_strong));
        assert_eq!(
            heap.object_strong_count(constructor),
            Ok(constructor_strong)
        );
        heap.live_node_mut(RawId::Object(iterator_prototype))
            .unwrap()
            .strong = iterator_strong;

        heap.attach_map_intrinsics(realm, map).unwrap();
        assert_eq!(heap.context(realm).unwrap().map, Some(map));
        assert_eq!(
            heap.object_strong_count(prototype),
            Ok(prototype_strong + 1)
        );
        assert_eq!(
            heap.object_strong_count(constructor),
            Ok(constructor_strong)
        );
        assert_eq!(
            heap.object_strong_count(iterator_prototype),
            Ok(iterator_strong + 1)
        );
        assert!(matches!(
            heap.attach_map_intrinsics(realm, map),
            Err(HeapError::Invariant(
                "context already has Map intrinsic roots"
            ))
        ));

        heap.release_object(prototype).unwrap();
        heap.release_object(iterator_prototype).unwrap();
        let constructor_cleanup = heap.release_object(constructor).unwrap();
        assert_eq!(constructor_cleanup.finalized_objects, 1);
        let context_cleanup = heap.release_context(realm).unwrap();
        assert_eq!(context_cleanup.finalized_contexts, 1);
        assert_eq!(context_cleanup.finalized_objects, 2);
        let stats = heap.run_gc().unwrap();
        assert_eq!(stats.cleanup.finalized_contexts, 0);
        assert_eq!(stats.cleanup.finalized_objects, 0);
        heap.release_shape(intrinsic_shape).unwrap();
        heap.release_object(root).unwrap();
        heap.release_shape(empty_shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn map_native_descriptors_preserve_quickjs_call_protocols() {
        assert_eq!(
            NativeFunctionId::Map(MapNativeKind::Constructor)
                .descriptor()
                .cproto,
            NativeCProto::Constructor
        );
        for kind in [MapNativeKind::Species, MapNativeKind::Size] {
            assert_eq!(
                NativeFunctionId::Map(kind).descriptor().cproto,
                NativeCProto::Getter
            );
        }
        for kind in [
            MapNativeKind::GroupBy,
            MapNativeKind::Set,
            MapNativeKind::Get,
            MapNativeKind::GetOrInsert,
            MapNativeKind::GetOrInsertComputed,
            MapNativeKind::Has,
            MapNativeKind::Delete,
            MapNativeKind::Clear,
            MapNativeKind::ForEach,
            MapNativeKind::Iterator(MapIteratorKind::Key),
            MapNativeKind::Iterator(MapIteratorKind::Value),
            MapNativeKind::Iterator(MapIteratorKind::KeyAndValue),
        ] {
            assert_eq!(
                NativeFunctionId::Map(kind).descriptor().cproto,
                NativeCProto::Generic
            );
        }
        assert_eq!(
            NativeFunctionId::MapIteratorNext.descriptor().cproto,
            NativeCProto::IteratorNext
        );
    }

    #[test]
    fn set_records_retain_key_edges_and_tombstone_with_undefined_value() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let set = heap
            .allocate_object(ObjectData::set(shape, Vec::new()))
            .unwrap();
        let key = leaf(&mut heap, shape);

        assert_eq!(
            heap.set_insert_record(set, RawValue::Object(key)),
            Ok(HeapCleanup::default())
        );
        assert_eq!(heap.set_size(set), Ok(1));
        assert_eq!(heap.object_strong_count(key), Ok(2));
        assert_eq!(
            heap.set_records(set),
            Ok(&[MapRecord {
                key: Some(RawValue::Object(key)),
                value: RawValue::Undefined,
            }][..])
        );
        heap.release_object(key).unwrap();

        let cleanup = heap.set_delete_record(set, 0).unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert!(matches!(heap.object(key), Err(HeapError::Stale { .. })));
        assert_eq!(heap.set_size(set), Ok(0));
        assert_eq!(
            heap.set_records(set),
            Ok(&[MapRecord {
                key: None,
                value: RawValue::Undefined,
            }][..])
        );
        assert!(matches!(
            heap.set_delete_record(set, 0),
            Err(HeapError::Invariant(
                "Set deletion requires a live record index"
            ))
        ));
        assert!(matches!(
            heap.set_insert_record(set, RawValue::Uninitialized),
            Err(HeapError::Invariant(
                "Set record contains an internal value sentinel"
            ))
        ));
        assert!(matches!(
            heap.set_insert_record(set, RawValue::Exception),
            Err(HeapError::Invariant(
                "Set record contains an internal value sentinel"
            ))
        ));

        let map = heap
            .allocate_object(ObjectData::map(shape, Vec::new()))
            .unwrap();
        assert!(matches!(
            heap.set_insert_record(map, RawValue::Int(1)),
            Err(HeapError::Invariant(
                "Set insertion reached an object with the wrong class"
            ))
        ));
        heap.release_object(map).unwrap();
        heap.release_object(set).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn set_tombstones_preserve_readd_order_and_live_iterator_sees_appends() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let set = heap
            .allocate_object(ObjectData::set(shape, Vec::new()))
            .unwrap();
        heap.set_insert_record(set, RawValue::Int(1)).unwrap();
        let iterator = heap
            .allocate_object(ObjectData::set_iterator(
                shape,
                Vec::new(),
                set,
                SetIteratorKind::KeyAndValue,
            ))
            .unwrap();

        heap.set_set_iterator_index(iterator, 1).unwrap();
        heap.set_insert_record(set, RawValue::Int(2)).unwrap();
        heap.set_delete_record(set, 1).unwrap();
        heap.set_insert_record(set, RawValue::Int(2)).unwrap();

        let records = heap.set_records(set).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].key, Some(RawValue::Int(1)));
        assert_eq!(records[1].key, None);
        assert_eq!(records[2].key, Some(RawValue::Int(2)));
        assert!(
            records
                .iter()
                .all(|record| matches!(record.value, RawValue::Undefined))
        );
        assert_eq!(heap.set_size(set), Ok(2));
        assert_eq!(
            heap.set_iterator_state(iterator),
            Ok((Some(set), 1, SetIteratorKind::KeyAndValue))
        );
        assert_eq!(
            records[1..].iter().find_map(|record| record.key.as_ref()),
            Some(&RawValue::Int(2))
        );

        assert_eq!(heap.object_strong_count(set), Ok(2));
        heap.release_object(set).unwrap();
        assert_eq!(heap.object_strong_count(set), Ok(1));
        let cleanup = heap.finish_set_iterator(iterator).unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert!(matches!(heap.object(set), Err(HeapError::Stale { .. })));
        assert_eq!(
            heap.set_iterator_state(iterator),
            Ok((None, 1, SetIteratorKind::KeyAndValue))
        );
        assert_eq!(
            heap.finish_set_iterator(iterator).unwrap(),
            HeapCleanup::default()
        );
        assert!(matches!(
            heap.set_set_iterator_index(iterator, 2),
            Err(HeapError::Invariant("completed Set Iterator was advanced"))
        ));

        heap.release_object(iterator).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn set_symbol_atoms_transfer_and_return_on_delete_clear_and_finalize() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let set = heap
            .allocate_object(ObjectData::set(shape, Vec::new()))
            .unwrap();
        let first = Atom::from_immediate_integer(201).unwrap();
        let second = Atom::from_immediate_integer(202).unwrap();
        let third = Atom::from_immediate_integer(203).unwrap();

        heap.set_insert_record(set, RawValue::Symbol(first))
            .unwrap();
        heap.set_insert_record(set, RawValue::Symbol(second))
            .unwrap();
        let cleanup = heap.set_delete_record(set, 0).unwrap();
        assert_eq!(cleanup.atoms, vec![first]);
        let cleanup = heap.set_clear(set).unwrap();
        assert_eq!(cleanup.atoms, vec![second]);
        assert_eq!(heap.set_size(set), Ok(0));

        heap.set_insert_record(set, RawValue::Symbol(third))
            .unwrap();
        let cleanup = heap.release_object(set).unwrap();
        assert_eq!(cleanup.atoms, vec![third]);
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn set_layout_and_iterator_source_are_structurally_validated() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);

        let malformed = ObjectData {
            shape,
            slots: Vec::new(),
            private_brand_home: None,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Set,
            payload: ObjectPayload::Set {
                records: vec![MapRecord {
                    key: Some(RawValue::Int(1)),
                    value: RawValue::Int(2),
                }],
                size: 1,
            },
        };
        assert!(matches!(
            heap.allocate_object(malformed),
            Err(HeapError::Invariant(
                "Set record value slot is not undefined"
            ))
        ));
        assert_eq!(heap.counts().object_nodes, 0);

        let map = heap
            .allocate_object(ObjectData::map(shape, Vec::new()))
            .unwrap();
        assert!(matches!(
            heap.allocate_object(ObjectData::set_iterator(
                shape,
                Vec::new(),
                map,
                SetIteratorKind::Value,
            )),
            Err(HeapError::Invariant(
                "Set Iterator source does not have the Set class"
            ))
        ));
        assert_eq!(heap.object_strong_count(map), Ok(1));

        let mut mismatched = ObjectData::set(shape, Vec::new());
        mismatched.kind = ObjectKind::Map;
        assert!(matches!(
            heap.allocate_object(mismatched),
            Err(HeapError::Invariant(
                "object kind does not match its class payload"
            ))
        ));

        heap.release_object(map).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn set_intrinsics_attach_transactionally_and_root_the_realm_graph() {
        let mut heap = Heap::new();
        let empty_shape = empty_shape(&mut heap);
        let root = leaf(&mut heap, empty_shape);
        let realm = heap
            .allocate_context(ContextData::new(
                root, root, root, root, root, root, root, root,
            ))
            .unwrap();
        let intrinsic_shape = heap
            .allocate_shape(Shape::new(Some(root), []).unwrap())
            .unwrap();
        let prototype = heap
            .allocate_object(ObjectData::ordinary(intrinsic_shape, Vec::new()))
            .unwrap();
        let iterator_prototype = heap
            .allocate_object(ObjectData::ordinary(intrinsic_shape, Vec::new()))
            .unwrap();
        let constructor = heap
            .allocate_object(ObjectData::bound_native_function(
                intrinsic_shape,
                Vec::new(),
                NativeFunctionId::Set(SetNativeKind::Constructor),
                realm,
                0,
            ))
            .unwrap();
        let set = SetRealmData {
            prototype,
            iterator_prototype,
        };
        let prototype_strong = heap.object_strong_count(prototype).unwrap();
        let constructor_strong = heap.object_strong_count(constructor).unwrap();
        let iterator_strong = heap.object_strong_count(iterator_prototype).unwrap();

        heap.live_node_mut(RawId::Object(iterator_prototype))
            .unwrap()
            .strong = u32::MAX;
        assert_eq!(
            heap.attach_set_intrinsics(realm, set),
            Err(HeapError::Overflow {
                operation: "retaining outgoing heap edges",
            })
        );
        assert_eq!(heap.context(realm).unwrap().set, None);
        assert_eq!(heap.object_strong_count(prototype), Ok(prototype_strong));
        assert_eq!(
            heap.object_strong_count(constructor),
            Ok(constructor_strong)
        );
        heap.live_node_mut(RawId::Object(iterator_prototype))
            .unwrap()
            .strong = iterator_strong;

        heap.attach_set_intrinsics(realm, set).unwrap();
        assert_eq!(heap.context(realm).unwrap().set, Some(set));
        assert_eq!(
            heap.object_strong_count(prototype),
            Ok(prototype_strong + 1)
        );
        assert_eq!(
            heap.object_strong_count(constructor),
            Ok(constructor_strong)
        );
        assert_eq!(
            heap.object_strong_count(iterator_prototype),
            Ok(iterator_strong + 1)
        );
        assert!(matches!(
            heap.attach_set_intrinsics(realm, set),
            Err(HeapError::Invariant(
                "context already has Set intrinsic roots"
            ))
        ));

        heap.release_object(prototype).unwrap();
        heap.release_object(iterator_prototype).unwrap();
        let constructor_cleanup = heap.release_object(constructor).unwrap();
        assert_eq!(constructor_cleanup.finalized_objects, 1);
        let context_cleanup = heap.release_context(realm).unwrap();
        assert_eq!(context_cleanup.finalized_contexts, 1);
        assert_eq!(context_cleanup.finalized_objects, 2);
        let stats = heap.run_gc().unwrap();
        assert_eq!(stats.cleanup.finalized_contexts, 0);
        assert_eq!(stats.cleanup.finalized_objects, 0);
        heap.release_shape(intrinsic_shape).unwrap();
        heap.release_object(root).unwrap();
        heap.release_shape(empty_shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn set_native_descriptors_preserve_quickjs_call_protocols() {
        assert_eq!(
            NativeFunctionId::Set(SetNativeKind::Constructor)
                .descriptor()
                .cproto,
            NativeCProto::Constructor
        );
        for kind in [SetNativeKind::Species, SetNativeKind::Size] {
            assert_eq!(
                NativeFunctionId::Set(kind).descriptor().cproto,
                NativeCProto::Getter
            );
        }
        for kind in [
            SetNativeKind::GroupBy,
            SetNativeKind::Add,
            SetNativeKind::Has,
            SetNativeKind::Delete,
            SetNativeKind::Clear,
            SetNativeKind::ForEach,
            SetNativeKind::IsDisjointFrom,
            SetNativeKind::IsSubsetOf,
            SetNativeKind::IsSupersetOf,
            SetNativeKind::Intersection,
            SetNativeKind::Difference,
            SetNativeKind::SymmetricDifference,
            SetNativeKind::Union,
            SetNativeKind::Iterator(SetIteratorKind::Value),
            SetNativeKind::Iterator(SetIteratorKind::KeyAndValue),
        ] {
            assert_eq!(
                NativeFunctionId::Set(kind).descriptor().cproto,
                NativeCProto::Generic
            );
        }
        assert_eq!(
            NativeFunctionId::SetIteratorNext.descriptor().cproto,
            NativeCProto::IteratorNext
        );
    }

    #[test]
    fn for_in_iterator_advances_snapshots_and_transfers_current_edges() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let source = leaf(&mut heap, shape);
        let iterator = heap
            .allocate_object(ObjectData::for_in_iterator(
                shape,
                Vec::new(),
                ForInIteratorData {
                    object: Some(source),
                    index: 0,
                    properties: vec![ForInProperty {
                        name: JsString::from_static("a"),
                        enumerable: true,
                    }],
                    fast_array: false,
                    array_count: 0,
                    in_prototype_chain: false,
                    visited: HashSet::new(),
                },
            ))
            .unwrap();

        assert_eq!(heap.object_strong_count(source), Ok(2));
        heap.release_object(source).unwrap();
        assert_eq!(
            heap.next_for_in_candidate(iterator),
            Ok(ForInCandidate::Property {
                object: source,
                name: JsString::from_static("a"),
            })
        );
        assert_eq!(
            heap.next_for_in_candidate(iterator),
            Ok(ForInCandidate::BaseComplete {
                object: source,
                fast_array: false,
            })
        );
        heap.enter_for_in_prototype_chain(iterator, None).unwrap();

        let prototype = leaf(&mut heap, shape);
        let cleanup = heap
            .replace_for_in_level(
                iterator,
                Some(prototype),
                vec![ForInProperty {
                    name: JsString::from_static("b"),
                    enumerable: true,
                }],
            )
            .unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert!(matches!(heap.object(source), Err(HeapError::Stale { .. })));
        heap.release_object(prototype).unwrap();
        assert_eq!(
            heap.next_for_in_candidate(iterator),
            Ok(ForInCandidate::Property {
                object: prototype,
                name: JsString::from_static("b"),
            })
        );
        assert_eq!(
            heap.next_for_in_candidate(iterator),
            Ok(ForInCandidate::LevelComplete(prototype))
        );
        let cleanup = heap
            .replace_for_in_level(iterator, None, Vec::new())
            .unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert_eq!(
            heap.next_for_in_candidate(iterator),
            Ok(ForInCandidate::Done)
        );

        heap.release_object(iterator).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn numeric_and_uri_native_selectors_use_pinned_cproto() {
        let targets = [
            NativeFunctionId::GlobalEval,
            NativeFunctionId::GlobalNumberParse(NumberParseKind::ParseInt),
            NativeFunctionId::GlobalNumberParse(NumberParseKind::ParseFloat),
            NativeFunctionId::GlobalNumberPredicate(GlobalNumberPredicateKind::IsNaN),
            NativeFunctionId::GlobalNumberPredicate(GlobalNumberPredicateKind::IsFinite),
            NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::Escape),
            NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::Unescape),
            NativeFunctionId::NumberPredicate(NumberPredicateKind::IsNaN),
            NativeFunctionId::NumberPredicate(NumberPredicateKind::IsFinite),
            NativeFunctionId::NumberPredicate(NumberPredicateKind::IsInteger),
            NativeFunctionId::NumberPredicate(NumberPredicateKind::IsSafeInteger),
            NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::ToExponential),
            NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::ToFixed),
            NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::ToPrecision),
            NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::ToLocaleString),
            NativeFunctionId::SymbolRegistry(SymbolRegistryKind::For),
            NativeFunctionId::SymbolRegistry(SymbolRegistryKind::KeyFor),
            NativeFunctionId::StringPrototypeCharCodeAt,
            NativeFunctionId::StringPrototypeConcat,
            NativeFunctionId::StringPrototypeCodePointAt,
            NativeFunctionId::StringPrototypeWellFormed(StringWellFormedKind::IsWellFormed),
            NativeFunctionId::StringPrototypeWellFormed(StringWellFormedKind::ToWellFormed),
            NativeFunctionId::StringPrototypeSplit,
            NativeFunctionId::StringPrototypeSubrange(StringSubrangeKind::Substring),
            NativeFunctionId::StringPrototypeSubrange(StringSubrangeKind::Substr),
            NativeFunctionId::StringPrototypeSubrange(StringSubrangeKind::Slice),
            NativeFunctionId::StringPrototypeRepeat,
            NativeFunctionId::StringCodePointRange,
            NativeFunctionId::MathHypot,
            NativeFunctionId::MathRandom,
            NativeFunctionId::MathImul,
            NativeFunctionId::MathClz32,
            NativeFunctionId::MathSumPrecise,
        ];

        for target in targets {
            assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
        for target in [
            NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::DecodeUri),
            NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::DecodeUriComponent),
            NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::EncodeUri),
            NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::EncodeUriComponent),
            NativeFunctionId::BigIntAsN(BigIntAsNKind::AsUintN),
            NativeFunctionId::BigIntAsN(BigIntAsNKind::AsIntN),
            NativeFunctionId::StringPrototypeCharAt(StringCharAtKind::At),
            NativeFunctionId::StringPrototypeCharAt(StringCharAtKind::CharAt),
            NativeFunctionId::StringPrototypePad(StringPadKind::End),
            NativeFunctionId::StringPrototypePad(StringPadKind::Start),
            NativeFunctionId::StringPrototypeReplace(StringReplaceKind::Replace),
            NativeFunctionId::StringPrototypeReplace(StringReplaceKind::ReplaceAll),
            NativeFunctionId::StringPrototypeMatch,
            NativeFunctionId::StringPrototypeMatchAll,
            NativeFunctionId::StringPrototypeSearch,
            NativeFunctionId::StringPrototypeTrim(StringTrimKind::Both),
            NativeFunctionId::StringPrototypeTrim(StringTrimKind::End),
            NativeFunctionId::StringPrototypeTrim(StringTrimKind::Start),
            NativeFunctionId::MathMinMax(MathMinMaxKind::Min),
            NativeFunctionId::MathMinMax(MathMinMaxKind::Max),
        ] {
            assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
        assert_eq!(
            NativeFunctionId::SymbolPrototypeDescription
                .descriptor()
                .cproto,
            NativeCProto::Getter
        );
        assert_eq!(
            NativeFunctionId::StringIteratorNext.descriptor().cproto,
            NativeCProto::IteratorNext
        );
        assert_eq!(
            NativeFunctionId::RegExpStringIteratorNext
                .descriptor()
                .cproto,
            NativeCProto::IteratorNext
        );
        assert!(!NativeCProto::IteratorNext.default_is_constructor());
        assert_eq!(
            NativeFunctionId::MathUnary(MathUnaryKind::Round)
                .descriptor()
                .cproto,
            NativeCProto::UnaryF64
        );
        assert_eq!(
            NativeFunctionId::MathBinary(MathBinaryKind::Pow)
                .descriptor()
                .cproto,
            NativeCProto::BinaryF64
        );
        assert!(!NativeCProto::UnaryF64.default_is_constructor());
        assert!(!NativeCProto::BinaryF64.default_is_constructor());
    }

    #[test]
    fn date_native_selectors_preserve_pinned_descriptors_and_magic() {
        let constructor = NativeFunctionId::Date(DateNativeKind::Constructor);
        assert_eq!(
            constructor.descriptor().cproto,
            NativeCProto::ConstructorOrFunction
        );
        assert!(constructor.descriptor().cproto.default_is_constructor());
        assert_eq!(DateNativeKind::Constructor.unique_name(), Some("Date"));
        assert_eq!(DateNativeKind::Constructor.length(), 7);

        for kind in [
            DateNativeKind::Now,
            DateNativeKind::Parse,
            DateNativeKind::Utc,
            DateNativeKind::TimeValue,
            DateNativeKind::ToPrimitive,
            DateNativeKind::TimezoneOffset,
            DateNativeKind::SetTime,
            DateNativeKind::SetYear,
            DateNativeKind::ToJson,
        ] {
            let target = NativeFunctionId::Date(kind);
            assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
        assert_eq!(DateNativeKind::Now.unique_name(), Some("now"));
        assert_eq!(DateNativeKind::Now.length(), 0);
        assert_eq!(DateNativeKind::Parse.unique_name(), Some("parse"));
        assert_eq!(DateNativeKind::Parse.length(), 1);
        assert_eq!(DateNativeKind::Utc.unique_name(), Some("UTC"));
        assert_eq!(DateNativeKind::Utc.length(), 7);
        assert_eq!(DateNativeKind::TimeValue.unique_name(), None);
        assert_eq!(DateNativeKind::TimeValue.length(), 0);
        assert_eq!(
            DateNativeKind::ToPrimitive.unique_name(),
            Some("[Symbol.toPrimitive]")
        );
        assert_eq!(DateNativeKind::ToPrimitive.length(), 1);
        assert_eq!(
            DateNativeKind::TimezoneOffset.unique_name(),
            Some("getTimezoneOffset")
        );
        assert_eq!(DateNativeKind::TimezoneOffset.length(), 0);
        assert_eq!(DateNativeKind::SetTime.unique_name(), Some("setTime"));
        assert_eq!(DateNativeKind::SetTime.length(), 1);
        assert_eq!(DateNativeKind::SetYear.unique_name(), Some("setYear"));
        assert_eq!(DateNativeKind::SetYear.length(), 1);
        assert_eq!(DateNativeKind::ToJson.unique_name(), Some("toJSON"));
        assert_eq!(DateNativeKind::ToJson.length(), 1);

        let string_metadata = [
            (DateStringMethod::ToString, "toString", 0x13, true),
            (DateStringMethod::ToUtcString, "toUTCString", 0x03, false),
            (DateStringMethod::ToIsoString, "toISOString", 0x23, false),
            (DateStringMethod::ToDateString, "toDateString", 0x11, true),
            (DateStringMethod::ToTimeString, "toTimeString", 0x12, true),
            (
                DateStringMethod::ToLocaleString,
                "toLocaleString",
                0x33,
                true,
            ),
            (
                DateStringMethod::ToLocaleDateString,
                "toLocaleDateString",
                0x31,
                true,
            ),
            (
                DateStringMethod::ToLocaleTimeString,
                "toLocaleTimeString",
                0x32,
                true,
            ),
        ];
        assert_eq!(
            string_metadata.map(|(kind, ..)| kind),
            DateStringMethod::ALL
        );
        for (kind, name, magic, uses_local_time) in string_metadata {
            let native = DateNativeKind::String(kind);
            assert_eq!(native.unique_name(), Some(name));
            assert_eq!(native.length(), 0);
            assert_eq!(kind.quickjs_magic(), magic);
            assert_eq!(kind.uses_local_time(), uses_local_time);
            assert_eq!(
                NativeFunctionId::Date(native).descriptor().cproto,
                NativeCProto::GenericMagic
            );
        }

        let get_metadata = [
            (DateGetFieldKind::Year, "getYear", 0x101),
            (DateGetFieldKind::FullYear, "getFullYear", 0x01),
            (DateGetFieldKind::UtcFullYear, "getUTCFullYear", 0x00),
            (DateGetFieldKind::Month, "getMonth", 0x11),
            (DateGetFieldKind::UtcMonth, "getUTCMonth", 0x10),
            (DateGetFieldKind::Date, "getDate", 0x21),
            (DateGetFieldKind::UtcDate, "getUTCDate", 0x20),
            (DateGetFieldKind::Hours, "getHours", 0x31),
            (DateGetFieldKind::UtcHours, "getUTCHours", 0x30),
            (DateGetFieldKind::Minutes, "getMinutes", 0x41),
            (DateGetFieldKind::UtcMinutes, "getUTCMinutes", 0x40),
            (DateGetFieldKind::Seconds, "getSeconds", 0x51),
            (DateGetFieldKind::UtcSeconds, "getUTCSeconds", 0x50),
            (DateGetFieldKind::Milliseconds, "getMilliseconds", 0x61),
            (
                DateGetFieldKind::UtcMilliseconds,
                "getUTCMilliseconds",
                0x60,
            ),
            (DateGetFieldKind::Day, "getDay", 0x71),
            (DateGetFieldKind::UtcDay, "getUTCDay", 0x70),
        ];
        assert_eq!(get_metadata.map(|(kind, ..)| kind), DateGetFieldKind::ALL);
        for (kind, name, magic) in get_metadata {
            let native = DateNativeKind::GetField(kind);
            assert_eq!(native.unique_name(), Some(name));
            assert_eq!(native.length(), 0);
            assert_eq!(kind.quickjs_magic(), magic);
            assert_eq!(
                NativeFunctionId::Date(native).descriptor().cproto,
                NativeCProto::GenericMagic
            );
        }

        let set_metadata = [
            (DateSetFieldKind::Milliseconds, "setMilliseconds", 1, 0x671),
            (
                DateSetFieldKind::UtcMilliseconds,
                "setUTCMilliseconds",
                1,
                0x670,
            ),
            (DateSetFieldKind::Seconds, "setSeconds", 2, 0x571),
            (DateSetFieldKind::UtcSeconds, "setUTCSeconds", 2, 0x570),
            (DateSetFieldKind::Minutes, "setMinutes", 3, 0x471),
            (DateSetFieldKind::UtcMinutes, "setUTCMinutes", 3, 0x470),
            (DateSetFieldKind::Hours, "setHours", 4, 0x371),
            (DateSetFieldKind::UtcHours, "setUTCHours", 4, 0x370),
            (DateSetFieldKind::Date, "setDate", 1, 0x231),
            (DateSetFieldKind::UtcDate, "setUTCDate", 1, 0x230),
            (DateSetFieldKind::Month, "setMonth", 2, 0x131),
            (DateSetFieldKind::UtcMonth, "setUTCMonth", 2, 0x130),
            (DateSetFieldKind::FullYear, "setFullYear", 3, 0x031),
            (DateSetFieldKind::UtcFullYear, "setUTCFullYear", 3, 0x030),
        ];
        assert_eq!(set_metadata.map(|(kind, ..)| kind), DateSetFieldKind::ALL);
        for (kind, name, length, magic) in set_metadata {
            let native = DateNativeKind::SetField(kind);
            assert_eq!(native.unique_name(), Some(name));
            assert_eq!(native.length(), length);
            assert_eq!(kind.length(), length);
            assert_eq!(kind.quickjs_magic(), magic);
            assert_eq!(
                NativeFunctionId::Date(native).descriptor().cproto,
                NativeCProto::GenericMagic
            );
        }
    }

    #[test]
    fn date_native_payload_owns_only_its_shape_and_defining_realm_edges() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = heap
            .allocate_object(ObjectData::ordinary(shape, Vec::new()))
            .unwrap();
        let realm = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        let function_shape = heap
            .allocate_shape(Shape::new(Some(prototype), []).unwrap())
            .unwrap();
        let target = NativeFunctionId::Date(DateNativeKind::Constructor);
        let function = heap
            .allocate_object(ObjectData::bound_native_function(
                function_shape,
                Vec::new(),
                target,
                realm,
                1,
            ))
            .unwrap();

        let function_data = heap.object(function).unwrap();
        assert!(function_data.is_constructor);
        assert!(matches!(
            function_data.payload,
            ObjectPayload::NativeFunction {
                data: NativeFunctionData {
                    target: stored_target,
                    realm: Some(stored_realm),
                    min_readable_args: 1,
                },
                ..
            } if stored_target == target && stored_realm == realm
        ));
        assert_eq!(
            object_edges(function_data),
            vec![RawId::Shape(function_shape), RawId::Context(realm)]
        );
        assert_eq!(heap.context_strong_count(realm), Ok(2));

        heap.release_context(realm).unwrap();
        assert_eq!(heap.context_strong_count(realm), Ok(1));
        let cleanup = heap.release_object(function).unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert_eq!(cleanup.finalized_contexts, 1);
        heap.release_shape(function_shape).unwrap();
        heap.release_object(prototype).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn string_static_native_selectors_use_pinned_cproto() {
        for target in [
            NativeFunctionId::StringStatic(StringStaticKind::FromCharCode),
            NativeFunctionId::StringStatic(StringStaticKind::FromCodePoint),
            NativeFunctionId::StringStatic(StringStaticKind::Raw),
        ] {
            assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
    }

    #[test]
    fn array_search_native_selectors_use_pinned_cproto() {
        for target in [
            NativeFunctionId::ArrayPrototypeAt,
            NativeFunctionId::ArrayPrototypeSearch(ArraySearchKind::Includes),
            NativeFunctionId::ArrayPrototypeSearch(ArraySearchKind::IndexOf),
            NativeFunctionId::ArrayPrototypeSearch(ArraySearchKind::LastIndexOf),
        ] {
            assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
    }

    #[test]
    fn array_with_native_selector_uses_pinned_cproto() {
        let target = NativeFunctionId::ArrayPrototypeWith;
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }

    #[test]
    fn array_concat_native_selector_uses_pinned_cproto() {
        let target = NativeFunctionId::ArrayPrototypeConcat;
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }

    #[test]
    fn array_stringification_native_selectors_use_pinned_cproto() {
        for kind in [ArrayJoinKind::Join, ArrayJoinKind::ToLocaleString] {
            let target = NativeFunctionId::ArrayPrototypeJoin(kind);
            assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
        let target = NativeFunctionId::ArrayPrototypeToString;
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }

    #[test]
    fn array_pop_push_native_selectors_use_pinned_cproto() {
        for target in [
            NativeFunctionId::ArrayPrototypePop(ArrayPopKind::Pop),
            NativeFunctionId::ArrayPrototypePop(ArrayPopKind::Shift),
            NativeFunctionId::ArrayPrototypePush(ArrayPushKind::Push),
            NativeFunctionId::ArrayPrototypePush(ArrayPushKind::Unshift),
        ] {
            assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
    }

    #[test]
    fn array_reverse_native_targets_use_pinned_cproto() {
        for target in [
            NativeFunctionId::ArrayPrototypeReverse,
            NativeFunctionId::ArrayPrototypeToReversed,
        ] {
            assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
    }

    #[test]
    fn array_sort_native_targets_use_pinned_cproto() {
        for target in [
            NativeFunctionId::ArrayPrototypeSort,
            NativeFunctionId::ArrayPrototypeToSorted,
        ] {
            assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
    }

    #[test]
    fn array_slice_native_targets_use_pinned_cproto() {
        for kind in [ArraySliceKind::Slice, ArraySliceKind::Splice] {
            let target = NativeFunctionId::ArrayPrototypeSlice(kind);
            assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
        let target = NativeFunctionId::ArrayPrototypeToSpliced;
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }

    #[test]
    fn array_fill_native_selector_uses_pinned_cproto() {
        let target = NativeFunctionId::ArrayPrototypeFill;
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }

    #[test]
    fn array_find_native_selectors_use_pinned_cproto() {
        for kind in [
            ArrayFindKind::Find,
            ArrayFindKind::FindIndex,
            ArrayFindKind::FindLast,
            ArrayFindKind::FindLastIndex,
        ] {
            let target = NativeFunctionId::ArrayPrototypeFind(kind);
            assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
    }

    #[test]
    fn array_iteration_native_selectors_use_pinned_cproto() {
        for kind in [
            ArrayIterationKind::Every,
            ArrayIterationKind::Some,
            ArrayIterationKind::ForEach,
            ArrayIterationKind::Map,
            ArrayIterationKind::Filter,
        ] {
            let target = NativeFunctionId::ArrayPrototypeIteration(kind);
            assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
    }

    #[test]
    fn array_reduce_native_selectors_use_pinned_cproto() {
        for kind in [ArrayReduceKind::Reduce, ArrayReduceKind::ReduceRight] {
            let target = NativeFunctionId::ArrayPrototypeReduce(kind);
            assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
    }

    #[test]
    fn array_copy_within_native_selector_uses_pinned_cproto() {
        let target = NativeFunctionId::ArrayPrototypeCopyWithin;
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }

    #[test]
    fn array_flatten_native_selectors_use_pinned_cproto() {
        for kind in [ArrayFlattenKind::FlatMap, ArrayFlattenKind::Flat] {
            let target = NativeFunctionId::ArrayPrototypeFlatten(kind);
            assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
    }

    #[test]
    fn object_native_selectors_use_pinned_cproto() {
        for target in [
            NativeFunctionId::ObjectCreate,
            NativeFunctionId::ObjectSetPrototypeOf,
            NativeFunctionId::ObjectDefineProperties,
            NativeFunctionId::ObjectGetOwnPropertyKeys(ObjectOwnPropertyKeysKind::Names),
            NativeFunctionId::ObjectGetOwnPropertyKeys(ObjectOwnPropertyKeysKind::Symbols),
            NativeFunctionId::ObjectGetOwnPropertyDescriptors,
            NativeFunctionId::ObjectIs,
            NativeFunctionId::ObjectAssign,
            NativeFunctionId::ObjectFromEntries,
            NativeFunctionId::ObjectHasOwn,
            NativeFunctionId::ObjectPrototypeHasOwnProperty,
            NativeFunctionId::ObjectPrototypeIsPrototypeOf,
            NativeFunctionId::ObjectPrototypePropertyIsEnumerable,
        ] {
            assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }

        for target in [
            NativeFunctionId::ObjectGetPrototypeOf,
            NativeFunctionId::ObjectDefineProperty,
            NativeFunctionId::ObjectGroupBy,
            NativeFunctionId::ObjectKeys(ObjectKeysKind::Keys),
            NativeFunctionId::ObjectKeys(ObjectKeysKind::Values),
            NativeFunctionId::ObjectKeys(ObjectKeysKind::Entries),
            NativeFunctionId::ObjectExtensibility(ObjectExtensibilityKind::IsExtensible),
            NativeFunctionId::ObjectExtensibility(ObjectExtensibilityKind::PreventExtensions),
            NativeFunctionId::ObjectGetOwnPropertyDescriptor,
            NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::Seal),
            NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::Freeze),
            NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::IsSealed),
            NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::IsFrozen),
            NativeFunctionId::ObjectPrototypeDefineAccessor(ObjectAccessorKind::Getter),
            NativeFunctionId::ObjectPrototypeDefineAccessor(ObjectAccessorKind::Setter),
            NativeFunctionId::ObjectPrototypeLookupAccessor(ObjectAccessorKind::Getter),
            NativeFunctionId::ObjectPrototypeLookupAccessor(ObjectAccessorKind::Setter),
        ] {
            assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }

        assert_eq!(
            NativeFunctionId::ObjectConstructor.descriptor().cproto,
            NativeCProto::ConstructorOrFunction
        );
        assert!(
            NativeFunctionId::ObjectConstructor
                .descriptor()
                .cproto
                .default_is_constructor()
        );
        assert_eq!(
            NativeFunctionId::ObjectPrototypeProtoGetter
                .descriptor()
                .cproto,
            NativeCProto::Getter
        );
        assert_eq!(
            NativeFunctionId::ObjectPrototypeProtoSetter
                .descriptor()
                .cproto,
            NativeCProto::Setter
        );
    }

    #[test]
    fn regexp_native_selectors_use_pinned_cproto() {
        let constructor = NativeFunctionId::RegExp(RegExpNativeKind::Constructor);
        assert_eq!(
            constructor.descriptor().cproto,
            NativeCProto::ConstructorOrFunction
        );
        assert!(constructor.descriptor().cproto.default_is_constructor());

        for target in [
            NativeFunctionId::RegExp(RegExpNativeKind::Exec),
            NativeFunctionId::RegExp(RegExpNativeKind::Compile),
            NativeFunctionId::RegExp(RegExpNativeKind::Test),
            NativeFunctionId::RegExp(RegExpNativeKind::ToString),
            NativeFunctionId::RegExp(RegExpNativeKind::Replace),
            NativeFunctionId::RegExp(RegExpNativeKind::Match),
            NativeFunctionId::RegExp(RegExpNativeKind::MatchAll),
            NativeFunctionId::RegExp(RegExpNativeKind::Search),
            NativeFunctionId::RegExp(RegExpNativeKind::Split),
        ] {
            assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
        for target in [
            NativeFunctionId::RegExp(RegExpNativeKind::Species),
            NativeFunctionId::RegExp(RegExpNativeKind::Source),
            NativeFunctionId::RegExp(RegExpNativeKind::Flags),
        ] {
            assert_eq!(target.descriptor().cproto, NativeCProto::Getter);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
        for flag in [
            RegExpFlagKind::HasIndices,
            RegExpFlagKind::Global,
            RegExpFlagKind::IgnoreCase,
            RegExpFlagKind::Multiline,
            RegExpFlagKind::DotAll,
            RegExpFlagKind::Unicode,
            RegExpFlagKind::UnicodeSets,
            RegExpFlagKind::Sticky,
        ] {
            let target = NativeFunctionId::RegExp(RegExpNativeKind::Flag(flag));
            assert_eq!(target.descriptor().cproto, NativeCProto::GetterMagic);
            assert!(!target.descriptor().cproto.default_is_constructor());
        }
    }

    fn one_slot_shape(heap: &mut Heap) -> ShapeId {
        let atom = Atom::from_immediate_integer(0).unwrap();
        heap.allocate_shape(
            Shape::new(
                None,
                [ShapeEntry {
                    atom,
                    flags: DATA_FLAGS,
                }],
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn leaf(heap: &mut Heap, shape: ShapeId) -> ObjectId {
        heap.allocate_object(ObjectData::ordinary(shape, Vec::new()))
            .unwrap()
    }

    struct RegExpFixture {
        heap: Heap,
        empty_shape: ShapeId,
        root: ObjectId,
        realm: ContextId,
        prototype_shape: ShapeId,
        prototype: ObjectId,
        string_iterator_prototype: ObjectId,
        function_shape: ShapeId,
        constructor: ObjectId,
        object_shape: ShapeId,
        last_index: Atom,
    }

    impl RegExpFixture {
        const fn realm_data(&self) -> RegExpRealmData {
            RegExpRealmData {
                prototype: self.prototype,
                constructor: self.constructor,
                string_iterator_prototype: self.string_iterator_prototype,
                object_shape: self.object_shape,
            }
        }

        fn dispose(mut self) -> GcStats {
            self.heap.release_shape(self.object_shape).unwrap();
            self.heap.release_object(self.prototype).unwrap();
            self.heap
                .release_object(self.string_iterator_prototype)
                .unwrap();
            self.heap.release_object(self.constructor).unwrap();
            self.heap.release_context(self.realm).unwrap();
            let stats = self.heap.run_gc().unwrap();

            self.heap.release_shape(self.function_shape).unwrap();
            self.heap.release_shape(self.prototype_shape).unwrap();
            self.heap.release_object(self.root).unwrap();
            self.heap.release_shape(self.empty_shape).unwrap();
            assert_eq!(self.heap.counts().live, 0);
            stats
        }
    }

    fn regexp_fixture() -> RegExpFixture {
        let mut heap = Heap::new();
        let empty_shape = empty_shape(&mut heap);
        let root = leaf(&mut heap, empty_shape);
        let realm = heap
            .allocate_context(ContextData::new(
                root, root, root, root, root, root, root, root,
            ))
            .unwrap();
        let prototype_shape = heap
            .allocate_shape(Shape::new(Some(root), []).unwrap())
            .unwrap();
        let prototype = heap
            .allocate_object(ObjectData::ordinary(prototype_shape, Vec::new()))
            .unwrap();
        let string_iterator_prototype = heap
            .allocate_object(ObjectData::ordinary(prototype_shape, Vec::new()))
            .unwrap();
        let function_shape = heap
            .allocate_shape(Shape::new(Some(root), []).unwrap())
            .unwrap();
        let constructor = heap
            .allocate_object(ObjectData::bound_native_function(
                function_shape,
                Vec::new(),
                NativeFunctionId::RegExp(RegExpNativeKind::Constructor),
                realm,
                2,
            ))
            .unwrap();
        let last_index = Atom::from_immediate_integer(0).unwrap();
        let object_shape = heap
            .allocate_shape(
                Shape::new(
                    Some(prototype),
                    [ShapeEntry {
                        atom: last_index,
                        flags: PropertyFlags::data(true, false, false),
                    }],
                )
                .unwrap(),
            )
            .unwrap();
        RegExpFixture {
            heap,
            empty_shape,
            root,
            realm,
            prototype_shape,
            prototype,
            string_iterator_prototype,
            function_shape,
            constructor,
            object_shape,
            last_index,
        }
    }

    #[test]
    fn regexp_intrinsics_attach_transactionally_once_and_finalize_with_realm() {
        let mut fixture = regexp_fixture();
        let realm_data = fixture.realm_data();
        let prototype_strong = fixture.heap.object_strong_count(fixture.prototype).unwrap();
        let constructor_strong = fixture
            .heap
            .object_strong_count(fixture.constructor)
            .unwrap();
        let string_iterator_prototype_strong = fixture
            .heap
            .object_strong_count(fixture.string_iterator_prototype)
            .unwrap();
        let object_shape_strong = fixture
            .heap
            .shape_strong_count(fixture.object_shape)
            .unwrap();

        fixture
            .heap
            .live_node_mut(RawId::Shape(fixture.object_shape))
            .unwrap()
            .strong = u32::MAX;
        assert_eq!(
            fixture
                .heap
                .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index,),
            Err(HeapError::Overflow {
                operation: "retaining outgoing heap edges",
            })
        );
        assert_eq!(fixture.heap.context(fixture.realm).unwrap().regexp, None);
        assert_eq!(
            fixture.heap.object_strong_count(fixture.prototype).unwrap(),
            prototype_strong
        );
        assert_eq!(
            fixture
                .heap
                .object_strong_count(fixture.constructor)
                .unwrap(),
            constructor_strong
        );
        assert_eq!(
            fixture
                .heap
                .object_strong_count(fixture.string_iterator_prototype)
                .unwrap(),
            string_iterator_prototype_strong
        );
        fixture
            .heap
            .live_node_mut(RawId::Shape(fixture.object_shape))
            .unwrap()
            .strong = object_shape_strong;

        fixture
            .heap
            .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index)
            .unwrap();
        assert_eq!(
            fixture.heap.context(fixture.realm).unwrap().regexp,
            Some(realm_data)
        );
        assert_eq!(
            fixture.heap.object_strong_count(fixture.prototype).unwrap(),
            prototype_strong + 1
        );
        assert_eq!(
            fixture
                .heap
                .object_strong_count(fixture.constructor)
                .unwrap(),
            constructor_strong + 1
        );
        assert_eq!(
            fixture
                .heap
                .object_strong_count(fixture.string_iterator_prototype)
                .unwrap(),
            string_iterator_prototype_strong + 1
        );
        assert_eq!(
            fixture
                .heap
                .shape_strong_count(fixture.object_shape)
                .unwrap(),
            object_shape_strong + 1
        );

        assert_eq!(
            fixture
                .heap
                .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index,),
            Err(HeapError::Invariant(
                "context already has RegExp intrinsic roots"
            ))
        );
        assert_eq!(
            fixture
                .heap
                .shape_strong_count(fixture.object_shape)
                .unwrap(),
            object_shape_strong + 1
        );

        let stats = fixture.dispose();
        assert_eq!(stats.cleanup.finalized_contexts, 1);
        assert_eq!(stats.cleanup.finalized_objects, 3);
        assert_eq!(stats.cleanup.finalized_shapes, 1);
    }

    #[test]
    fn regexp_intrinsics_reject_mismatched_constructor_prototype_and_shape() {
        let mut fixture = regexp_fixture();
        let realm_data = fixture.realm_data();

        {
            let ObjectPayload::NativeFunction { data, .. } = &mut fixture
                .heap
                .object_mut(fixture.constructor)
                .unwrap()
                .payload
            else {
                panic!("fixture constructor must be a native function");
            };
            data.target = NativeFunctionId::Date(DateNativeKind::Constructor);
        }
        assert!(
            fixture
                .heap
                .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index)
                .is_err()
        );
        {
            let ObjectPayload::NativeFunction { data, .. } = &mut fixture
                .heap
                .object_mut(fixture.constructor)
                .unwrap()
                .payload
            else {
                panic!("fixture constructor must be a native function");
            };
            data.target = NativeFunctionId::RegExp(RegExpNativeKind::Constructor);
        }

        {
            let ObjectPayload::NativeFunction { data, .. } = &mut fixture
                .heap
                .object_mut(fixture.constructor)
                .unwrap()
                .payload
            else {
                panic!("fixture constructor must be a native function");
            };
            data.realm = None;
        }
        assert!(
            fixture
                .heap
                .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index)
                .is_err()
        );
        {
            let ObjectPayload::NativeFunction { data, .. } = &mut fixture
                .heap
                .object_mut(fixture.constructor)
                .unwrap()
                .payload
            else {
                panic!("fixture constructor must be a native function");
            };
            data.realm = Some(fixture.realm);
        }

        fixture.heap.object_mut(fixture.prototype).unwrap().payload =
            ObjectPayload::RegExp(RegExpObjectData::Uninitialized);
        assert!(
            fixture
                .heap
                .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index)
                .is_err()
        );
        fixture.heap.object_mut(fixture.prototype).unwrap().payload = ObjectPayload::Ordinary;

        fixture
            .heap
            .object_mut(fixture.string_iterator_prototype)
            .unwrap()
            .payload = ObjectPayload::RegExp(RegExpObjectData::Uninitialized);
        assert!(
            fixture
                .heap
                .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index)
                .is_err()
        );
        fixture
            .heap
            .object_mut(fixture.string_iterator_prototype)
            .unwrap()
            .payload = ObjectPayload::Ordinary;

        assert!(
            fixture
                .heap
                .attach_regexp_intrinsics(
                    fixture.realm,
                    RegExpRealmData {
                        string_iterator_prototype: fixture.root,
                        ..realm_data
                    },
                    fixture.last_index,
                )
                .is_err()
        );

        let wrong_prototype_shape = fixture
            .heap
            .allocate_shape(
                Shape::new(
                    Some(fixture.root),
                    [ShapeEntry {
                        atom: fixture.last_index,
                        flags: PropertyFlags::data(true, false, false),
                    }],
                )
                .unwrap(),
            )
            .unwrap();
        assert!(
            fixture
                .heap
                .attach_regexp_intrinsics(
                    fixture.realm,
                    RegExpRealmData {
                        object_shape: wrong_prototype_shape,
                        ..realm_data
                    },
                    fixture.last_index,
                )
                .is_err()
        );
        fixture.heap.release_shape(wrong_prototype_shape).unwrap();

        let wrong_flags_shape = fixture
            .heap
            .allocate_shape(
                Shape::new(
                    Some(fixture.prototype),
                    [ShapeEntry {
                        atom: fixture.last_index,
                        flags: DATA_FLAGS,
                    }],
                )
                .unwrap(),
            )
            .unwrap();
        assert!(
            fixture
                .heap
                .attach_regexp_intrinsics(
                    fixture.realm,
                    RegExpRealmData {
                        object_shape: wrong_flags_shape,
                        ..realm_data
                    },
                    fixture.last_index,
                )
                .is_err()
        );
        fixture.heap.release_shape(wrong_flags_shape).unwrap();

        let wrong_atom = Atom::from_immediate_integer(1).unwrap();
        assert!(
            fixture
                .heap
                .attach_regexp_intrinsics(fixture.realm, realm_data, wrong_atom)
                .is_err()
        );
        assert_eq!(fixture.heap.context(fixture.realm).unwrap().regexp, None);
        fixture.dispose();
    }

    #[test]
    fn retain_and_release_leaf_uses_explicit_strong_count() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let object = leaf(&mut heap, shape);
        assert_eq!(heap.object_strong_count(object), Ok(1));
        assert_eq!(heap.shape_strong_count(shape), Ok(2));

        heap.retain_object(object).unwrap();
        assert_eq!(heap.object_strong_count(object), Ok(2));
        assert_eq!(heap.release_object(object).unwrap(), HeapCleanup::default());
        assert_eq!(heap.object_strong_count(object), Ok(1));

        let cleanup = heap.release_object(object).unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert!(matches!(heap.object(object), Err(HeapError::Stale { .. })));
        assert_eq!(heap.shape_strong_count(shape), Ok(1));
        assert_eq!(heap.release_shape(shape).unwrap().finalized_shapes, 1);
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn mapped_arguments_payload_roots_varrefs_and_tracks_fast_state() {
        let mut heap = Heap::new();
        let shape = one_slot_shape(&mut heap);
        let cell = heap
            .allocate_var_ref(VarRefData::local(RawValue::Int(7)))
            .unwrap();
        let arguments = heap
            .allocate_object(ObjectData::arguments(
                shape,
                vec![PropertySlot::VarRef(cell)],
                true,
                1,
            ))
            .unwrap();
        assert_eq!(heap.arguments_state(arguments), Ok((true, Some(1))));
        assert_eq!(heap.var_ref_strong_count(cell), Ok(2));

        heap.set_arguments_fast_len(arguments, None).unwrap();
        assert_eq!(heap.arguments_state(arguments), Ok((true, None)));
        heap.release_var_ref(cell).unwrap();
        assert_eq!(heap.var_ref_strong_count(cell), Ok(1));

        let cleanup = heap.release_object(arguments).unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert_eq!(cleanup.finalized_var_refs, 1);
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn context_roots_intrinsic_prototypes_until_realm_finalization() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let object_prototype = leaf(&mut heap, shape);
        let function_prototype = leaf(&mut heap, shape);
        let array_prototype = leaf(&mut heap, shape);
        let iterator_prototype = leaf(&mut heap, shape);
        let array_iterator_prototype = leaf(&mut heap, shape);
        let string_iterator_prototype = leaf(&mut heap, shape);
        let date_prototype = leaf(&mut heap, shape);
        let global_object = leaf(&mut heap, shape);
        let global_var_object = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(
                ContextData::new(
                    object_prototype,
                    function_prototype,
                    array_prototype,
                    iterator_prototype,
                    array_iterator_prototype,
                    string_iterator_prototype,
                    global_object,
                    global_var_object,
                )
                .with_date_prototype(date_prototype),
            )
            .unwrap();
        assert_eq!(
            heap.context(context).unwrap().array_prototype,
            array_prototype
        );
        assert_eq!(
            heap.context(context).unwrap().iterator_prototype,
            iterator_prototype
        );
        assert_eq!(
            heap.context(context).unwrap().array_iterator_prototype,
            array_iterator_prototype
        );
        assert_eq!(
            heap.context(context).unwrap().string_iterator_prototype,
            string_iterator_prototype
        );
        assert_eq!(
            heap.context(context).unwrap().date_prototype,
            Some(date_prototype)
        );
        assert!(matches!(
            heap.object(date_prototype).unwrap().payload,
            ObjectPayload::Ordinary
        ));

        for object in [
            object_prototype,
            function_prototype,
            array_prototype,
            iterator_prototype,
            array_iterator_prototype,
            string_iterator_prototype,
            date_prototype,
            global_object,
            global_var_object,
        ] {
            assert_eq!(heap.release_object(object).unwrap(), HeapCleanup::default());
            assert_eq!(heap.object_strong_count(object), Ok(1));
        }
        let cleanup = heap.release_context(context).unwrap();
        assert_eq!(cleanup.finalized_contexts, 1);
        assert_eq!(cleanup.finalized_objects, 9);
        assert_eq!(heap.release_shape(shape).unwrap().finalized_shapes, 1);
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn hundred_thousand_node_chain_finalizes_iteratively() {
        const LENGTH: usize = 100_000;

        let mut heap = Heap::new();
        let shape = one_slot_shape(&mut heap);
        let mut head = heap
            .allocate_object(ObjectData::ordinary(
                shape,
                vec![PropertySlot::Data(RawValue::Undefined)],
            ))
            .unwrap();

        for _ in 1..LENGTH {
            let next = heap
                .allocate_object(ObjectData::ordinary(
                    shape,
                    vec![PropertySlot::Data(RawValue::Object(head))],
                ))
                .unwrap();
            assert_eq!(heap.release_object(head).unwrap(), HeapCleanup::default());
            head = next;
        }

        let cleanup = heap.release_object(head).unwrap();
        assert_eq!(cleanup.finalized_objects, LENGTH);
        assert_eq!(heap.counts().object_nodes, 0);
        assert_eq!(heap.release_shape(shape).unwrap().finalized_shapes, 1);
    }

    #[test]
    fn self_cycle_requires_trial_deletion() {
        let mut heap = Heap::new();
        let shape = one_slot_shape(&mut heap);
        let object = heap
            .allocate_object(ObjectData::ordinary(
                shape,
                vec![PropertySlot::Data(RawValue::Undefined)],
            ))
            .unwrap();
        heap.replace_object_slot(object, 0, PropertySlot::Data(RawValue::Object(object)))
            .unwrap();

        // The object now owns the only shape reference, matching a runtime
        // shape cache whose entry is weak.
        assert_eq!(heap.release_shape(shape).unwrap(), HeapCleanup::default());

        assert_eq!(heap.release_object(object).unwrap(), HeapCleanup::default());
        assert_eq!(heap.object_strong_count(object), Ok(1));
        let stats = heap.run_gc().unwrap();
        assert_eq!(stats.candidate_nodes, 2);
        assert_eq!(stats.cleanup.finalized_objects, 1);
        assert_eq!(stats.cleanup.finalized_shapes, 1);
        assert!(matches!(heap.object(object), Err(HeapError::Stale { .. })));
    }

    #[test]
    fn object_slot_replacement_rejects_private_name_payloads() {
        let mut heap = Heap::new();
        let shape = one_slot_shape(&mut heap);
        let object = heap
            .allocate_object(ObjectData::ordinary(
                shape,
                vec![PropertySlot::Data(RawValue::Undefined)],
            ))
            .unwrap();
        let private = Atom::from_raw(91);

        assert_eq!(
            heap.replace_object_slot(object, 0, PropertySlot::Data(RawValue::Private(private)),),
            Err(HeapError::Invariant(
                "private-name identity escaped into an object value slot"
            ))
        );
        assert!(matches!(
            heap.object(object).unwrap().slots[0],
            PropertySlot::Data(RawValue::Undefined)
        ));

        assert_eq!(heap.release_object(object).unwrap().finalized_objects, 1);
        assert_eq!(heap.release_shape(shape).unwrap().finalized_shapes, 1);
    }

    #[test]
    fn two_node_cycle_requires_trial_deletion() {
        let mut heap = Heap::new();
        let shape = one_slot_shape(&mut heap);
        let first = heap
            .allocate_object(ObjectData::ordinary(
                shape,
                vec![PropertySlot::Data(RawValue::Undefined)],
            ))
            .unwrap();
        let second = heap
            .allocate_object(ObjectData::ordinary(
                shape,
                vec![PropertySlot::Data(RawValue::Object(first))],
            ))
            .unwrap();
        heap.replace_object_slot(first, 0, PropertySlot::Data(RawValue::Object(second)))
            .unwrap();

        assert_eq!(heap.release_shape(shape).unwrap(), HeapCleanup::default());

        heap.release_object(first).unwrap();
        heap.release_object(second).unwrap();
        assert_eq!(heap.counts().object_nodes, 2);

        let stats = heap.run_gc().unwrap();
        assert_eq!(stats.cleanup.finalized_objects, 2);
        assert_eq!(stats.cleanup.finalized_shapes, 1);
        assert_eq!(heap.counts().object_nodes, 0);
    }

    #[test]
    fn external_root_preserves_cycle_closure_until_released() {
        let mut heap = Heap::new();
        let shape = one_slot_shape(&mut heap);
        let first = heap
            .allocate_object(ObjectData::ordinary(
                shape,
                vec![PropertySlot::Data(RawValue::Undefined)],
            ))
            .unwrap();
        let second = heap
            .allocate_object(ObjectData::ordinary(
                shape,
                vec![PropertySlot::Data(RawValue::Object(first))],
            ))
            .unwrap();
        heap.replace_object_slot(first, 0, PropertySlot::Data(RawValue::Object(second)))
            .unwrap();

        assert_eq!(heap.release_shape(shape).unwrap(), HeapCleanup::default());

        heap.retain_object(first).unwrap();
        heap.release_object(first).unwrap();
        heap.release_object(second).unwrap();

        let preserved = heap.run_gc().unwrap();
        assert_eq!(preserved.candidate_nodes, 0);
        assert_eq!(preserved.external_root_nodes, 1);
        assert_eq!(heap.counts().object_nodes, 2);

        heap.release_object(first).unwrap();
        let collected = heap.run_gc().unwrap();
        assert_eq!(collected.cleanup.finalized_objects, 2);
        assert_eq!(collected.cleanup.finalized_shapes, 1);
    }

    #[test]
    fn reclaimed_generation_rejects_stale_handles() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let stale = leaf(&mut heap, shape);
        let index = stale.debug_index();
        let generation = stale.debug_generation();
        heap.release_object(stale).unwrap();

        let replacement = leaf(&mut heap, shape);
        assert_eq!(replacement.debug_index(), index);
        assert_eq!(replacement.debug_generation(), generation + 1);
        assert!(matches!(
            heap.retain_object(stale),
            Err(HeapError::Stale { .. })
        ));

        heap.release_object(replacement).unwrap();
        heap.release_shape(shape).unwrap();
    }

    #[test]
    fn forged_cross_kind_handle_reports_wrong_kind() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let forged = ObjectId {
            index: shape.index,
            generation: shape.generation,
        };

        assert_eq!(
            heap.object(forged),
            Err(HeapError::WrongKind {
                expected: HeapNodeKind::Object,
                actual: HeapNodeKind::Shape,
            })
        );
        heap.release_shape(shape).unwrap();
    }

    #[test]
    fn shape_prototype_edge_participates_in_cycle_collection() {
        let mut heap = Heap::new();
        let original_shape = empty_shape(&mut heap);
        let object = leaf(&mut heap, original_shape);

        let prototype_shape = heap
            .allocate_shape(Shape::new(Some(object), []).unwrap())
            .unwrap();
        heap.replace_object_layout(object, prototype_shape, Vec::new())
            .unwrap();

        assert_eq!(
            heap.release_shape(original_shape).unwrap().finalized_shapes,
            1
        );
        assert_eq!(
            heap.release_shape(prototype_shape).unwrap(),
            HeapCleanup::default()
        );
        assert_eq!(heap.release_object(object).unwrap(), HeapCleanup::default());

        let stats = heap.run_gc().unwrap();
        assert_eq!(stats.candidate_nodes, 2);
        assert_eq!(stats.cleanup.finalized_objects, 1);
        assert_eq!(stats.cleanup.finalized_shapes, 1);
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn symbol_atom_ownership_is_returned_on_replace_and_finalize() {
        let mut heap = Heap::new();
        let shape = one_slot_shape(&mut heap);
        let first_symbol = Atom::from_raw(17);
        let second_symbol = Atom::from_raw(23);
        let object = heap
            .allocate_object(ObjectData::ordinary(
                shape,
                vec![PropertySlot::Data(RawValue::Symbol(first_symbol))],
            ))
            .unwrap();

        let replacement = heap
            .replace_object_slot(
                object,
                0,
                PropertySlot::Data(RawValue::Symbol(second_symbol)),
            )
            .unwrap();
        assert_eq!(replacement.atoms, vec![first_symbol]);

        let object_cleanup = heap.release_object(object).unwrap();
        assert_eq!(object_cleanup.atoms, vec![second_symbol]);
        let shape_cleanup = heap.release_shape(shape).unwrap();
        assert_eq!(
            shape_cleanup.atoms,
            vec![Atom::from_immediate_integer(0).unwrap()]
        );
    }

    fn bytecode(
        code: &Rc<[Instruction]>,
        realm: ContextId,
        constants: Vec<BytecodeConstant>,
        auxiliary_atoms: Vec<Atom>,
    ) -> FunctionBytecodeData {
        FunctionBytecodeData {
            code: code.clone(),
            constants: constants.into(),
            realm,
            metadata: FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            parameter_environment: None,
            func_name: None,
            argument_definitions: Rc::from([]),
            local_definitions: Rc::from([]),
            closure_variables: Rc::from([]),
            private_bindings: PublishedPrivateBindings::none(),
            eval_environments: Rc::from([]),
            debug: None,
            auxiliary_atoms: auxiliary_atoms.into_boxed_slice(),
        }
    }

    fn closure_bytecode(
        code: &Rc<[Instruction]>,
        realm: ContextId,
        closure_count: u16,
    ) -> FunctionBytecodeData {
        let mut bytecode = bytecode(code, realm, Vec::new(), Vec::new());
        bytecode.metadata.closure_count = closure_count;
        bytecode.closure_variables = (0..closure_count)
            .map(|index| ClosureVariable {
                source: ClosureSource::ParentClosure(index),
                name: ClosureVariableName::None,
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            })
            .collect::<Vec<_>>()
            .into();
        bytecode
    }

    fn bytecode_test_realm(heap: &mut Heap) -> ContextId {
        let shape = empty_shape(heap);
        let prototype = heap
            .allocate_object(ObjectData::ordinary(shape, Vec::new()))
            .unwrap();
        heap.allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap()
    }

    fn allocate_private_callable_child(
        heap: &mut Heap,
        realm: ContextId,
        argument_count: u16,
        valid_home_object_metadata: bool,
        func_name: Option<JsString>,
        function_kind: FunctionKind,
        has_prototype: bool,
    ) -> FunctionBytecodeId {
        let code: Rc<[Instruction]> = if function_kind == FunctionKind::Generator {
            Rc::from([
                Instruction::InitialYield,
                Instruction::Undefined,
                Instruction::Return,
            ])
        } else {
            Rc::from([Instruction::Undefined, Instruction::Return])
        };
        let mut child = bytecode(&code, realm, Vec::new(), Vec::new());
        child.metadata.argument_count = argument_count;
        child.metadata.defined_argument_count = argument_count;
        child.metadata.strict = true;
        child.metadata.needs_home_object = valid_home_object_metadata;
        child.metadata.function_kind = function_kind;
        child.metadata.has_prototype = has_prototype;
        child.func_name = func_name;
        child.argument_definitions = (0..argument_count)
            .map(|_| VariableDefinition {
                name: None,
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::Normal,
            })
            .collect::<Vec<_>>()
            .into();
        heap.allocate_function_bytecode(child).unwrap()
    }

    fn allocate_private_accessor_child(
        heap: &mut Heap,
        realm: ContextId,
        argument_count: u16,
        valid_home_object_metadata: bool,
        func_name: Option<JsString>,
    ) -> FunctionBytecodeId {
        allocate_private_callable_child(
            heap,
            realm,
            argument_count,
            valid_home_object_metadata,
            func_name,
            FunctionKind::Normal,
            false,
        )
    }

    fn allocate_private_brand_child(heap: &mut Heap, realm: ContextId) -> FunctionBytecodeId {
        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let mut child = bytecode(&code, realm, Vec::new(), Vec::new());
        child.metadata.strict = true;
        child.metadata.super_allowed = true;
        child.metadata.arguments_forbidden = true;
        child.metadata.needs_home_object = true;
        child.metadata.class_initializer_kind = Some(ClassInitializerKind::InstanceFields);
        child.metadata.class_private_brand = true;
        heap.allocate_function_bytecode(child).unwrap()
    }

    #[derive(Clone, Copy)]
    enum LinkedPrivateAccessorShape {
        Getter,
        Setter,
        Pair,
    }

    fn linked_private_accessor_bytecode(
        heap: &mut Heap,
        realm: ContextId,
        shape: LinkedPrivateAccessorShape,
    ) -> FunctionBytecodeData {
        let primary_name = Atom::from_raw(601);
        let setter_name = Atom::from_raw(602);
        let callable_arguments: &[u16] = match shape {
            LinkedPrivateAccessorShape::Getter => &[0],
            LinkedPrivateAccessorShape::Setter => &[1],
            LinkedPrivateAccessorShape::Pair => &[0, 1],
        };
        let mut constants = callable_arguments
            .iter()
            .map(|argument_count| {
                BytecodeConstant::Function(allocate_private_accessor_child(
                    heap,
                    realm,
                    *argument_count,
                    true,
                    None,
                ))
            })
            .collect::<Vec<_>>();
        constants.push(BytecodeConstant::Function(allocate_private_brand_child(
            heap, realm,
        )));

        let (definitions, roles, code, names) = match shape {
            LinkedPrivateAccessorShape::Getter => (
                vec![VariableDefinition {
                    name: Some(primary_name),
                    is_lexical: true,
                    is_const: true,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::PrivateGetter,
                }],
                vec![Some(PublishedPrivateBinding::primary(primary_name, None))],
                vec![
                    Instruction::SetLocalUninitialized(0),
                    Instruction::Undefined,
                    Instruction::FClosure(0),
                    Instruction::InitializePrivateAccessor(0),
                    Instruction::Drop,
                    Instruction::CloseLocal(0),
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                vec![primary_name],
            ),
            LinkedPrivateAccessorShape::Setter => (
                vec![
                    VariableDefinition {
                        name: Some(primary_name),
                        is_lexical: true,
                        is_const: true,
                        is_parameter_initializer: false,
                        kind: ClosureVariableKind::PrivateSetter,
                    },
                    VariableDefinition {
                        name: Some(setter_name),
                        is_lexical: true,
                        is_const: true,
                        is_parameter_initializer: false,
                        kind: ClosureVariableKind::PrivateSetter,
                    },
                ],
                vec![
                    Some(PublishedPrivateBinding::primary(primary_name, Some(1))),
                    Some(PublishedPrivateBinding::setter_storage(
                        setter_name,
                        Some(0),
                    )),
                ],
                vec![
                    Instruction::SetLocalUninitialized(0),
                    Instruction::SetLocalUninitialized(1),
                    Instruction::Undefined,
                    Instruction::FClosure(0),
                    Instruction::InitializePrivateAccessor(1),
                    Instruction::Drop,
                    Instruction::CloseLocal(1),
                    Instruction::CloseLocal(0),
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                vec![primary_name, setter_name],
            ),
            LinkedPrivateAccessorShape::Pair => (
                vec![
                    VariableDefinition {
                        name: Some(primary_name),
                        is_lexical: true,
                        is_const: true,
                        is_parameter_initializer: false,
                        kind: ClosureVariableKind::PrivateGetterSetter,
                    },
                    VariableDefinition {
                        name: Some(setter_name),
                        is_lexical: true,
                        is_const: true,
                        is_parameter_initializer: false,
                        kind: ClosureVariableKind::PrivateSetter,
                    },
                ],
                vec![
                    Some(PublishedPrivateBinding::primary(primary_name, Some(1))),
                    Some(PublishedPrivateBinding::setter_storage(
                        setter_name,
                        Some(0),
                    )),
                ],
                vec![
                    Instruction::SetLocalUninitialized(0),
                    Instruction::SetLocalUninitialized(1),
                    Instruction::Undefined,
                    Instruction::FClosure(0),
                    Instruction::InitializePrivateAccessor(0),
                    Instruction::Drop,
                    Instruction::Undefined,
                    Instruction::FClosure(1),
                    Instruction::InitializePrivateAccessor(1),
                    Instruction::Drop,
                    Instruction::CloseLocal(1),
                    Instruction::CloseLocal(0),
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                vec![primary_name, setter_name],
            ),
        };
        let mut parent = bytecode(&Rc::from(code), realm, constants, names);
        parent.metadata.local_count = u16::try_from(definitions.len()).unwrap();
        parent.metadata.max_stack = 2;
        parent.local_definitions = definitions.into();
        parent.private_bindings = PublishedPrivateBindings::authenticated(roles, Vec::new());
        parent
    }

    fn linked_private_method_bytecode(
        heap: &mut Heap,
        realm: ContextId,
        function_kind: FunctionKind,
        has_prototype: bool,
    ) -> FunctionBytecodeData {
        let name = Atom::from_raw(604);
        let method = allocate_private_callable_child(
            heap,
            realm,
            0,
            true,
            None,
            function_kind,
            has_prototype,
        );
        let brand = allocate_private_brand_child(heap, realm);
        let code: Rc<[Instruction]> = Rc::from([
            Instruction::SetLocalUninitialized(0),
            Instruction::Undefined,
            Instruction::FClosure(0),
            Instruction::InitializePrivateMethod(0),
            Instruction::Drop,
            Instruction::CloseLocal(0),
            Instruction::Undefined,
            Instruction::Return,
        ]);
        let mut parent = bytecode(
            &code,
            realm,
            vec![
                BytecodeConstant::Function(method),
                BytecodeConstant::Function(brand),
            ],
            vec![name],
        );
        parent.metadata.local_count = 1;
        parent.metadata.max_stack = 2;
        parent.local_definitions = Rc::from([VariableDefinition {
            name: Some(name),
            is_lexical: true,
            is_const: true,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::PrivateMethod,
        }]);
        parent.private_bindings = PublishedPrivateBindings::authenticated(
            vec![Some(PublishedPrivateBinding::primary(name, None))],
            Vec::new(),
        );
        parent
    }

    #[test]
    fn linked_private_methods_accept_generator_callable_shape() {
        for (function_kind, has_prototype) in [
            (FunctionKind::Normal, false),
            (FunctionKind::Generator, true),
        ] {
            let mut heap = Heap::new();
            let realm = bytecode_test_realm(&mut heap);
            let candidate =
                linked_private_method_bytecode(&mut heap, realm, function_kind, has_prototype);
            assert!(
                heap.allocate_function_bytecode(candidate).is_ok(),
                "{function_kind:?}/{has_prototype}"
            );
        }
    }

    #[test]
    fn linked_private_callables_reject_cross_role_execution_shapes() {
        for (function_kind, has_prototype) in [
            (FunctionKind::Normal, true),
            (FunctionKind::Async, false),
            (FunctionKind::AsyncGenerator, true),
        ] {
            let mut heap = Heap::new();
            let realm = bytecode_test_realm(&mut heap);
            let candidate =
                linked_private_method_bytecode(&mut heap, realm, function_kind, has_prototype);
            assert_eq!(
                heap.allocate_function_bytecode(candidate),
                Err(HeapError::Invariant(
                    "private-method child has invalid HomeObject metadata"
                )),
                "{function_kind:?}/{has_prototype}"
            );
        }

        let mut heap = Heap::new();
        let realm = bytecode_test_realm(&mut heap);
        let generator = allocate_private_callable_child(
            &mut heap,
            realm,
            0,
            true,
            None,
            FunctionKind::Generator,
            true,
        );
        let mut getter =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
        getter.constants = Rc::from([
            BytecodeConstant::Function(generator),
            getter.constants[1].clone(),
        ]);
        assert_eq!(
            heap.allocate_function_bytecode(getter),
            Err(HeapError::Invariant(
                "private-accessor child has invalid HomeObject metadata"
            ))
        );
    }

    #[test]
    fn linked_private_accessors_accept_only_sealed_legal_lifecycles() {
        let mut heap = Heap::new();
        let realm = bytecode_test_realm(&mut heap);

        for shape in [
            LinkedPrivateAccessorShape::Getter,
            LinkedPrivateAccessorShape::Setter,
            LinkedPrivateAccessorShape::Pair,
        ] {
            let candidate = linked_private_accessor_bytecode(&mut heap, realm, shape);
            assert!(heap.allocate_function_bytecode(candidate).is_ok());
        }

        let mut unsealed =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
        unsealed.private_bindings = PublishedPrivateBindings::none();
        assert_eq!(
            heap.allocate_function_bytecode(unsealed),
            Err(HeapError::Invariant(
                "published private bindings are missing sealed role metadata"
            ))
        );
    }

    #[test]
    fn linked_private_accessors_reject_forged_setter_capability_uses() {
        let mut heap = Heap::new();
        let realm = bytecode_test_realm(&mut heap);

        let mut initialized_primary =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Setter);
        initialized_primary.code = initialized_primary
            .code
            .iter()
            .map(|instruction| match instruction {
                Instruction::InitializePrivateAccessor(1) => {
                    Instruction::InitializePrivateAccessor(0)
                }
                instruction => instruction.clone(),
            })
            .collect::<Vec<_>>()
            .into();
        assert_eq!(
            heap.allocate_function_bytecode(initialized_primary),
            Err(HeapError::Invariant(
                "private-accessor initializer referenced an incompatible binding"
            ))
        );

        let mut primary_put =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Setter);
        let mut code = primary_put.code.to_vec();
        code.insert(
            code.len() - 2,
            Instruction::PutPrivateField(PrivateNameSource::Local(0)),
        );
        primary_put.code = code.into();
        assert_eq!(
            heap.allocate_function_bytecode(primary_put),
            Err(HeapError::Invariant(
                "private put referenced an incompatible binding"
            ))
        );

        let mut synthetic_in =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Setter);
        let mut code = synthetic_in.code.to_vec();
        code.insert(
            code.len() - 2,
            Instruction::PrivateIn(PrivateNameSource::Local(1)),
        );
        synthetic_in.code = code.into();
        assert_eq!(
            heap.allocate_function_bytecode(synthetic_in),
            Err(HeapError::Invariant(
                "private-in referenced a synthetic setter binding"
            ))
        );

        let primary_name = Atom::from_raw(603);
        let code: Rc<[Instruction]> = Rc::from([
            Instruction::SetLocalUninitialized(0),
            Instruction::CloseLocal(0),
            Instruction::Undefined,
            Instruction::Return,
        ]);
        let mut unpaired = bytecode(&code, realm, Vec::new(), vec![primary_name]);
        unpaired.metadata.local_count = 1;
        unpaired.local_definitions = Rc::from([VariableDefinition {
            name: Some(primary_name),
            is_lexical: true,
            is_const: true,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::PrivateSetter,
        }]);
        unpaired.private_bindings = PublishedPrivateBindings::authenticated(
            vec![Some(PublishedPrivateBinding::primary(primary_name, None))],
            Vec::new(),
        );
        assert_eq!(
            heap.allocate_function_bytecode(unpaired),
            Err(HeapError::Invariant(
                "published private setter has malformed pair metadata"
            ))
        );
    }

    #[test]
    fn linked_private_accessor_initializer_authenticates_its_unique_child_edge() {
        let mut heap = Heap::new();
        let realm = bytecode_test_realm(&mut heap);

        let mut missing_closure =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
        missing_closure.code = missing_closure
            .code
            .iter()
            .map(|instruction| match instruction {
                Instruction::FClosure(0) => Instruction::Undefined,
                instruction => instruction.clone(),
            })
            .collect::<Vec<_>>()
            .into();
        assert_eq!(
            heap.allocate_function_bytecode(missing_closure),
            Err(HeapError::Invariant(
                "private-accessor initializer did not consume an adjacent closure"
            ))
        );

        let mut invalid_child =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
        let invalid = allocate_private_accessor_child(&mut heap, realm, 0, false, None);
        invalid_child.constants = Rc::from([
            BytecodeConstant::Function(invalid),
            invalid_child.constants[1].clone(),
        ]);
        assert_eq!(
            heap.allocate_function_bytecode(invalid_child),
            Err(HeapError::Invariant(
                "private-accessor child has invalid HomeObject metadata"
            ))
        );

        let mut duplicate_site =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
        let mut code = duplicate_site.code.to_vec();
        code.splice(1..1, [Instruction::FClosure(0), Instruction::Drop]);
        duplicate_site.code = code.into();
        assert_eq!(
            heap.allocate_function_bytecode(duplicate_site),
            Err(HeapError::Invariant(
                "private-accessor child did not have one unique closure site"
            ))
        );

        let mut non_fallthrough =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
        let mut code = non_fallthrough.code.to_vec();
        code.insert(1, Instruction::Goto(3));
        non_fallthrough.code = code.into();
        assert_eq!(
            heap.allocate_function_bytecode(non_fallthrough),
            Err(HeapError::Invariant(
                "private-accessor closure/initializer pair has a non-fallthrough entry"
            ))
        );

        let mut repeated_lifetime =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
        let mut code = repeated_lifetime.code.to_vec();
        code.insert(code.len() - 2, Instruction::Goto(1));
        repeated_lifetime.code = code.into();
        assert_eq!(
            heap.allocate_function_bytecode(repeated_lifetime),
            Err(HeapError::Invariant(
                "private-accessor initializer is reachable by a repeated-lifetime backedge"
            ))
        );
    }

    #[test]
    fn linked_private_accessor_initializer_authenticates_role_arity_and_empty_name() {
        let mut heap = Heap::new();
        let realm = bytecode_test_realm(&mut heap);

        let mut zero_argument_setter =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Setter);
        let forged_setter = allocate_private_accessor_child(&mut heap, realm, 0, true, None);
        zero_argument_setter.constants = Rc::from([
            BytecodeConstant::Function(forged_setter),
            zero_argument_setter.constants[1].clone(),
        ]);
        assert_eq!(
            heap.allocate_function_bytecode(zero_argument_setter),
            Err(HeapError::Invariant(
                "private-accessor child has invalid authored arity"
            ))
        );

        let mut one_argument_getter =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
        let forged_getter = allocate_private_accessor_child(&mut heap, realm, 1, true, None);
        one_argument_getter.constants = Rc::from([
            BytecodeConstant::Function(forged_getter),
            one_argument_getter.constants[1].clone(),
        ]);
        assert_eq!(
            heap.allocate_function_bytecode(one_argument_getter),
            Err(HeapError::Invariant(
                "private-accessor child has invalid authored arity"
            ))
        );

        let mut named_getter =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
        let forged_name = allocate_private_accessor_child(
            &mut heap,
            realm,
            0,
            true,
            Some(JsString::from_static("#value")),
        );
        named_getter.constants = Rc::from([
            BytecodeConstant::Function(forged_name),
            named_getter.constants[1].clone(),
        ]);
        assert_eq!(
            heap.allocate_function_bytecode(named_getter),
            Err(HeapError::Invariant(
                "private-accessor child retained a non-empty intrinsic name"
            ))
        );
    }

    #[test]
    fn bytecode_debug_filename_requires_one_auxiliary_atom_ownership() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = heap
            .allocate_object(ObjectData::ordinary(shape, Vec::new()))
            .unwrap();
        let realm = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let filename = Atom::from_raw(19);
        let mut missing = bytecode(&code, realm, Vec::new(), Vec::new());
        missing.metadata.max_stack = 1;
        missing.debug = Some(FunctionDebugInfo {
            filename,
            pc2line: Some(Pc2LineTable::new(LineColumn::new(0, 0), Vec::new())),
            source: None,
        });
        assert_eq!(
            heap.allocate_function_bytecode(missing),
            Err(HeapError::Invariant(
                "debug filename atom is not owned by bytecode metadata"
            ))
        );

        let mut owned = bytecode(&code, realm, Vec::new(), vec![filename]);
        owned.metadata.max_stack = 1;
        owned.debug = Some(FunctionDebugInfo {
            filename,
            pc2line: Some(Pc2LineTable::new(LineColumn::new(0, 0), Vec::new())),
            source: None,
        });
        let bytecode = heap.allocate_function_bytecode(owned).unwrap();
        assert_eq!(
            heap.release_function_bytecode(bytecode).unwrap().atoms,
            vec![filename]
        );
    }

    #[test]
    fn bytecode_allocation_rejects_mismatched_closure_descriptor_count() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let mut too_many_locals = bytecode(&code, context, Vec::new(), Vec::new());
        too_many_locals.metadata.local_count = u16::MAX;
        assert_eq!(
            heap.allocate_function_bytecode(too_many_locals),
            Err(HeapError::Invariant(
                "bytecode local count exceeds QuickJS JS_MAX_LOCAL_VARS"
            ))
        );

        let mut malformed = bytecode(&code, context, Vec::new(), Vec::new());
        malformed.metadata.closure_count = 1;
        assert!(matches!(
            heap.allocate_function_bytecode(malformed),
            Err(HeapError::Invariant(_))
        ));

        let mut lexical_argument = bytecode(&code, context, Vec::new(), Vec::new());
        lexical_argument.metadata.argument_count = 1;
        lexical_argument.metadata.defined_argument_count = 1;
        lexical_argument.argument_definitions = Rc::from([VariableDefinition {
            name: None,
            is_lexical: true,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        }]);
        assert_eq!(
            heap.allocate_function_bytecode(lexical_argument),
            Err(HeapError::Invariant(
                "argument definition is not an ordinary mutable binding"
            ))
        );
        assert_eq!(heap.counts().function_bytecode_nodes, 0);

        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn active_function_counts_as_a_quickjs_hidden_variable() {
        assert!(!quickjs_copies_defined_argument_count(0, 0, &[]));
        assert!(quickjs_copies_defined_argument_count(
            0,
            0,
            &[Instruction::PushActiveFunction],
        ));
    }

    #[test]
    fn parameter_pseudo_prologue_accepts_active_function_in_fixed_order() {
        let metadata = FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 0,
            rest_parameter: Some(0),
            local_count: 4,
            ..FunctionMetadata::default()
        };
        let ordered = vec![
            Instruction::PushHomeObject,
            Instruction::PutLocal(0),
            Instruction::PushActiveFunction,
            Instruction::PutLocal(1),
            Instruction::PushNewTarget,
            Instruction::PutLocal(2),
            Instruction::PushThis,
            Instruction::PutLocal(3),
            Instruction::Rest(0),
            Instruction::PutArg(0),
            Instruction::Undefined,
            Instruction::Return,
        ];
        assert_eq!(
            validate_parameter_bytecode_layout(&metadata, &ordered, &[false; 4], None),
            Ok(None),
        );

        let mut out_of_order = ordered;
        out_of_order.swap(2, 4);
        out_of_order.swap(3, 5);
        assert_eq!(
            validate_parameter_bytecode_layout(&metadata, &out_of_order, &[false; 4], None),
            Err("rest parameter contains a malformed pseudo-binding prologue"),
        );
    }

    #[test]
    fn parameter_initializers_preserve_derived_this_authority() {
        let metadata = FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            pattern_argument_count: 1,
            parameter_pattern_end: Some(6),
            local_count: 2,
            derived_this_local: Some(0),
            active_function_local: Some(1),
            constructor_kind: ConstructorKind::Derived,
            strict: true,
            super_call_allowed: true,
            super_allowed: true,
            ..FunctionMetadata::default()
        };
        let code = [
            Instruction::GetArg(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::InitializeDerivedLocal(0),
            Instruction::GetLocalCheck(0),
            Instruction::Drop,
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Return,
        ];
        assert_eq!(
            validate_pattern_parameter_bytecode_layout(
                &metadata,
                &code,
                &[true],
                &[true, false],
                &[false, false],
                None,
            ),
            Ok(()),
        );
        let mut escaped_protocol = code.clone();
        escaped_protocol[2] = Instruction::InitDerivedConstructor;
        assert_eq!(
            validate_pattern_parameter_bytecode_layout(
                &metadata,
                &escaped_protocol,
                &[true],
                &[true, false],
                &[false, false],
                None,
            ),
            Err("constructor completion protocol escaped into parameter initialization"),
        );

        let ordinary = FunctionMetadata {
            derived_this_local: None,
            active_function_local: None,
            constructor_kind: ConstructorKind::None,
            super_call_allowed: false,
            super_allowed: false,
            ..metadata
        };
        assert_eq!(
            validate_pattern_parameter_bytecode_layout(
                &ordinary,
                &code,
                &[true],
                &[true, false],
                &[false, false],
                None,
            ),
            Err("parameter BindingPattern bytecode accessed a body lexical local"),
        );

        let visibility_code = [
            Instruction::PushActiveFunction,
            Instruction::PutLocal(1),
            Instruction::Nop,
        ];
        let visibility_layout = ParameterEnvironmentLayout {
            initialization_end: 2,
            argument_cells: Box::new([]),
            pattern_copies: Box::new([]),
            default_sources: Box::new([]),
            synthetic_arguments_local: None,
            arg_eval_variable_object_local: None,
        };
        assert_eq!(
            parameter_initializer_visible_locals(
                &metadata,
                &visibility_code,
                Some(2),
                &[false, false],
                Some(&visibility_layout),
            ),
            Ok(Some(vec![true, true])),
        );

        let relay_metadata = FunctionMetadata {
            closure_count: 1,
            super_call_allowed: true,
            super_allowed: true,
            ..FunctionMetadata::default()
        };
        let relay = [
            Instruction::ConstructSuper(0),
            Instruction::Dup,
            Instruction::InitializeDerivedVarRef(0),
            Instruction::Undefined,
            Instruction::Return,
        ];
        let descriptor = ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::None,
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        };
        assert_eq!(
            validate_derived_constructor_bytecode_layout(
                &relay_metadata,
                &relay,
                &[],
                &[],
                &[descriptor],
            ),
            Ok(()),
        );

        let mut generic_relay = relay;
        generic_relay[0] = Instruction::Construct(0);
        assert_eq!(
            validate_derived_constructor_bytecode_layout(
                &relay_metadata,
                &generic_relay,
                &[],
                &[],
                &[descriptor],
            ),
            Err("captured derived initializer has no constructor result"),
        );
        let generic_apply_relay = [
            Instruction::Apply(crate::bytecode::ApplyKind::Construct),
            Instruction::Dup,
            Instruction::InitializeDerivedVarRef(0),
            Instruction::Undefined,
            Instruction::Return,
        ];
        assert_eq!(
            validate_derived_constructor_bytecode_layout(
                &relay_metadata,
                &generic_apply_relay,
                &[],
                &[],
                &[descriptor],
            ),
            Err("captured derived initializer has no constructor result"),
        );
        let injected_result = [
            Instruction::PushTrue,
            Instruction::IfFalse(6),
            Instruction::ConstructSuper(0),
            Instruction::Dup,
            Instruction::InitializeDerivedVarRef(0),
            Instruction::Return,
            Instruction::Object,
            Instruction::Goto(3),
        ];
        assert_eq!(
            validate_derived_constructor_bytecode_layout(
                &relay_metadata,
                &injected_result,
                &[],
                &[],
                &[descriptor],
            ),
            Err("captured derived initializer protocol has a non-fallthrough entry"),
        );

        let ordinary_relay = FunctionMetadata {
            super_call_allowed: false,
            super_allowed: false,
            ..relay_metadata
        };
        assert_eq!(
            validate_derived_constructor_bytecode_layout(
                &ordinary_relay,
                &[Instruction::MarkSuperCall],
                &[],
                &[],
                &[],
            ),
            Err("typed super-call opcode has no inherited super authority"),
        );
    }

    #[test]
    fn derived_active_function_initialization_is_an_entry_capability() {
        let metadata = FunctionMetadata {
            local_count: 2,
            derived_this_local: Some(0),
            active_function_local: Some(1),
            constructor_kind: ConstructorKind::Derived,
            strict: true,
            super_call_allowed: true,
            super_allowed: true,
            ..FunctionMetadata::default()
        };
        let entry = [
            Instruction::PushActiveFunction,
            Instruction::PutLocal(1),
            Instruction::Undefined,
            Instruction::ReturnDerived(0),
        ];
        assert_eq!(
            validate_derived_constructor_bytecode_layout(
                &metadata,
                &entry,
                &[true, false],
                &[false, false],
                &[],
            ),
            Ok(()),
        );

        let dead = [
            Instruction::Undefined,
            Instruction::ReturnDerived(0),
            Instruction::PushActiveFunction,
            Instruction::PutLocal(1),
        ];
        assert_eq!(
            validate_derived_constructor_bytecode_layout(
                &metadata,
                &dead,
                &[true, false],
                &[false, false],
                &[],
            ),
            Err("derived constructor active-function initialization is not at entry"),
        );

        let default = [
            Instruction::PushActiveFunction,
            Instruction::PutLocal(1),
            Instruction::CheckCtor,
            Instruction::InitDerivedConstructor,
            Instruction::Dup,
            Instruction::InitializeDerivedLocal(0),
            Instruction::GetLocal(1),
            Instruction::CallClassInstanceInitializer,
            Instruction::ReturnDerived(0),
        ];
        assert_eq!(
            validate_derived_constructor_bytecode_layout(
                &metadata,
                &default,
                &[true, false],
                &[false, false],
                &[],
            ),
            Ok(()),
        );

        let mut forged_default = default.to_vec();
        forged_default.insert(3, Instruction::Nop);
        assert_eq!(
            validate_derived_constructor_bytecode_layout(
                &metadata,
                &forged_default,
                &[true, false],
                &[false, false],
                &[],
            ),
            Err("default-derived constructor has no exact synthesized shape"),
        );

        let arbitrary_this = [
            Instruction::PushActiveFunction,
            Instruction::PutLocal(1),
            Instruction::PushI32(1),
            Instruction::Dup,
            Instruction::InitializeDerivedLocal(0),
            Instruction::Undefined,
            Instruction::ReturnDerived(0),
        ];
        assert_eq!(
            validate_derived_constructor_bytecode_layout(
                &metadata,
                &arbitrary_this,
                &[true, false],
                &[false, false],
                &[],
            ),
            Err("derived local initializer has no constructor result"),
        );
    }

    #[test]
    fn base_class_initializer_hook_is_unique_and_entry_only() {
        let metadata = FunctionMetadata {
            constructor_kind: ConstructorKind::Base,
            strict: true,
            ..FunctionMetadata::default()
        };
        let canonical = [
            Instruction::CheckCtor,
            Instruction::PushThis,
            Instruction::PushActiveFunction,
            Instruction::CallClassInstanceInitializer,
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ];
        assert_eq!(
            validate_derived_constructor_bytecode_layout(&metadata, &canonical, &[], &[], &[]),
            Ok(()),
        );

        let mut duplicate = canonical.to_vec();
        duplicate.splice(
            5..5,
            [
                Instruction::PushThis,
                Instruction::PushActiveFunction,
                Instruction::CallClassInstanceInitializer,
                Instruction::Drop,
            ],
        );
        assert_eq!(
            validate_derived_constructor_bytecode_layout(&metadata, &duplicate, &[], &[], &[]),
            Err("active-function opcode escaped a derived constructor"),
        );

        let mut backedge = canonical.to_vec();
        backedge.splice(5..5, [Instruction::Goto(0)]);
        assert_eq!(
            validate_derived_constructor_bytecode_layout(&metadata, &backedge, &[], &[], &[]),
            Err("base class initializer protocol has a non-fallthrough entry"),
        );
    }

    #[test]
    fn class_initializers_require_a_home_object_slot() {
        let mut metadata = FunctionMetadata {
            class_initializer_kind: Some(ClassInitializerKind::InstanceFields),
            strict: true,
            super_allowed: true,
            arguments_forbidden: true,
            ..FunctionMetadata::default()
        };
        let code = [Instruction::Undefined, Instruction::Return];
        assert_eq!(
            validate_class_initializer_bytecode_layout(&metadata, &code),
            Err("class initializer function metadata is malformed"),
        );
        metadata.needs_home_object = true;
        assert_eq!(
            validate_class_initializer_bytecode_layout(&metadata, &code),
            Ok(()),
        );
    }

    #[test]
    fn bytecode_allocation_rejects_stack_underflow_in_privileged_class_initializer() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let code: Rc<[Instruction]> = Rc::from([
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ]);
        let mut initializer = bytecode(&code, context, Vec::new(), Vec::new());
        initializer.metadata.class_initializer_kind = Some(ClassInitializerKind::InstanceFields);
        initializer.metadata.strict = true;
        initializer.metadata.super_allowed = true;
        initializer.metadata.arguments_forbidden = true;
        initializer.metadata.needs_home_object = true;
        assert_eq!(
            heap.allocate_function_bytecode(initializer),
            Err(HeapError::Invariant(
                "function bytecode failed generic verification"
            ))
        );
        assert_eq!(heap.counts().function_bytecode_nodes, 0);

        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn class_static_initializer_claim_is_one_shot_per_constructor() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let code: Rc<[Instruction]> = Rc::from([
            Instruction::CheckCtor,
            Instruction::PushThis,
            Instruction::PushActiveFunction,
            Instruction::CallClassInstanceInitializer,
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ]);
        let mut owner = bytecode(&code, context, Vec::new(), Vec::new());
        owner.metadata.constructor_kind = ConstructorKind::Base;
        owner.metadata.strict = true;
        owner.metadata.max_stack = 2;
        let owner = heap.allocate_function_bytecode(owner).unwrap();
        let constructor = heap
            .allocate_object(ObjectData::bytecode_function(
                shape,
                Vec::new(),
                owner,
                None,
                true,
            ))
            .unwrap();

        assert_eq!(
            heap.begin_bytecode_class_static_initializer(constructor),
            Ok(())
        );
        assert_eq!(
            heap.begin_bytecode_class_static_initializer(constructor),
            Err(HeapError::Invariant(
                "class static initializer was already started"
            ))
        );

        heap.release_object(constructor).unwrap();
        heap.release_function_bytecode(owner).unwrap();
        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn bytecode_allocation_accepts_typed_class_heritage() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let code: Rc<[Instruction]> = Rc::from([
            Instruction::Undefined,
            Instruction::PushI32(7),
            Instruction::DefineClass {
                name: 0,
                has_heritage: true,
            },
            Instruction::Drop,
            Instruction::Return,
        ]);
        let mut candidate = bytecode(
            &code,
            context,
            vec![BytecodeConstant::Value(RawValue::String(
                JsString::from_static("Derived"),
            ))],
            Vec::new(),
        );
        candidate.metadata.max_stack = 2;
        let function = heap.allocate_function_bytecode(candidate).unwrap();
        heap.release_function_bytecode(function).unwrap();

        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn bytecode_allocation_rejects_pattern_access_to_body_lexical() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let code: Rc<[Instruction]> = Rc::from([
            Instruction::GetArg(0),
            Instruction::Drop,
            Instruction::GetLocalCheck(1),
            Instruction::Drop,
            Instruction::Nop,
            Instruction::GetLocal(0),
            Instruction::Return,
        ]);
        let mut malformed = bytecode(&code, context, Vec::new(), Vec::new());
        malformed.metadata.argument_count = 1;
        malformed.metadata.defined_argument_count = 1;
        malformed.metadata.pattern_argument_count = 1;
        malformed.metadata.parameter_pattern_end = Some(4);
        malformed.metadata.local_count = 2;
        malformed.metadata.max_stack = 1;
        malformed.argument_definitions = Rc::from([VariableDefinition {
            name: None,
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        }]);
        malformed.local_definitions = Rc::from([
            VariableDefinition {
                name: None,
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::Normal,
            },
            VariableDefinition {
                name: None,
                is_lexical: true,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::Normal,
            },
        ]);
        assert_eq!(
            heap.allocate_function_bytecode(malformed),
            Err(HeapError::Invariant(
                "parameter BindingPattern bytecode accessed a body lexical local"
            ))
        );

        let empty_rest = |body_value, defined_argument_count| {
            let code: Rc<[Instruction]> = Rc::from([
                Instruction::Rest(0),
                Instruction::Drop,
                Instruction::Nop,
                body_value,
                Instruction::Return,
            ]);
            let mut candidate = bytecode(&code, context, Vec::new(), Vec::new());
            candidate.metadata.defined_argument_count = defined_argument_count;
            candidate.metadata.rest_pattern_start = Some(0);
            candidate.metadata.parameter_pattern_end = Some(2);
            candidate.metadata.max_stack = 1;
            candidate
        };
        let direct_this = heap
            .allocate_function_bytecode(empty_rest(Instruction::PushThis, 1))
            .unwrap();
        heap.release_function_bytecode(direct_this).unwrap();
        let direct_new_target = heap
            .allocate_function_bytecode(empty_rest(Instruction::PushNewTarget, 1))
            .unwrap();
        heap.release_function_bytecode(direct_new_target).unwrap();
        let mut direct_home_object = empty_rest(Instruction::PushHomeObject, 1);
        direct_home_object.metadata.needs_home_object = true;
        let direct_home_object = heap.allocate_function_bytecode(direct_home_object).unwrap();
        heap.release_function_bytecode(direct_home_object).unwrap();
        assert_eq!(
            heap.allocate_function_bytecode(empty_rest(Instruction::Undefined, 1)),
            Err(HeapError::Invariant(
                "rest BindingPattern metadata disagrees with function length"
            ))
        );
        assert_eq!(
            heap.allocate_function_bytecode(empty_rest(Instruction::PushThis, 0)),
            Err(HeapError::Invariant(
                "rest BindingPattern metadata disagrees with function length"
            ))
        );

        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn bytecode_allocation_rejects_eval_environments_across_parameter_phases() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let value_name = Atom::from_raw(61);
        let scoped_name = Atom::from_raw(62);
        let argument_definitions: Rc<[VariableDefinition]> = Rc::from([VariableDefinition {
            name: Some(value_name),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        }]);
        let parameter_layout = |initialization_end| ParameterEnvironmentLayout {
            initialization_end,
            argument_cells: vec![ParameterArgumentCell {
                argument: 0,
                parameter_local: 0,
                body: ParameterBodyStorage::Argument(0),
            }]
            .into_boxed_slice(),
            pattern_copies: Box::new([]),
            default_sources: vec![ParameterDefaultSource::Argument(0)].into_boxed_slice(),
            synthetic_arguments_local: None,
            arg_eval_variable_object_local: None,
        };
        let binding = || EvalBinding {
            name: scoped_name,
            source: EvalBindingSource::Local(1),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        };

        let body_eval_code: Rc<[Instruction]> = Rc::from([
            Instruction::SetLocalUninitialized(0),
            Instruction::GetArg(0),
            Instruction::Dup,
            Instruction::Undefined,
            Instruction::StrictEq,
            Instruction::IfFalse(14),
            Instruction::Drop,
            Instruction::SetLocalUninitialized(1),
            Instruction::Undefined,
            Instruction::InitializeLocal(1),
            Instruction::CloseLocal(1),
            Instruction::Undefined,
            Instruction::Dup,
            Instruction::PutArg(0),
            Instruction::InitializeLocal(0),
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Eval {
                argument_count: 0,
                environment: 0,
            },
            Instruction::Return,
        ]);
        let mut body_eval = bytecode(
            &body_eval_code,
            context,
            Vec::new(),
            vec![value_name, value_name, scoped_name, scoped_name],
        );
        body_eval.metadata = FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 0,
            parameter_environment_local_count: 1,
            local_count: 2,
            max_stack: 3,
            strict: true,
            ..FunctionMetadata::default()
        };
        body_eval.parameter_environment = Some(parameter_layout(15));
        body_eval.argument_definitions = argument_definitions.clone();
        body_eval.local_definitions = Rc::from([
            VariableDefinition {
                name: Some(value_name),
                is_lexical: true,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::Normal,
            },
            VariableDefinition {
                name: Some(scoped_name),
                is_lexical: true,
                is_const: false,
                is_parameter_initializer: true,
                kind: ClosureVariableKind::Normal,
            },
        ]);
        body_eval.eval_environments = Rc::from([EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: vec![binding()].into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::StrictLocal(1),
            caller_strict: true,
            super_call_allowed: false,
            super_allowed: false,
        }]);
        assert_eq!(
            heap.allocate_function_bytecode(body_eval),
            Err(HeapError::Invariant(
                "function body eval captured a parameter-initializer local"
            ))
        );

        let initializer_eval_code: Rc<[Instruction]> = Rc::from([
            Instruction::SetLocalUninitialized(0),
            Instruction::GetArg(0),
            Instruction::Dup,
            Instruction::Undefined,
            Instruction::StrictEq,
            Instruction::IfFalse(12),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::ApplyEval { environment: 0 },
            Instruction::Dup,
            Instruction::PutArg(0),
            Instruction::InitializeLocal(0),
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Return,
        ]);
        let mut initializer_eval = bytecode(
            &initializer_eval_code,
            context,
            Vec::new(),
            vec![value_name, value_name, scoped_name, scoped_name],
        );
        initializer_eval.metadata = FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 0,
            parameter_environment_local_count: 1,
            local_count: 2,
            max_stack: 3,
            strict: true,
            ..FunctionMetadata::default()
        };
        initializer_eval.parameter_environment = Some(parameter_layout(13));
        initializer_eval.argument_definitions = argument_definitions;
        initializer_eval.local_definitions = Rc::from([
            VariableDefinition {
                name: Some(value_name),
                is_lexical: true,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::Normal,
            },
            VariableDefinition {
                name: Some(scoped_name),
                is_lexical: true,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::Normal,
            },
        ]);
        initializer_eval.eval_environments = Rc::from([EvalEnvironment {
            scopes: vec![EvalScope {
                kind: EvalScopeKind::Parameter,
                bindings: vec![binding()].into_boxed_slice(),
            }]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::StrictLocal(0),
            caller_strict: true,
            super_call_allowed: false,
            super_allowed: false,
        }]);
        assert_eq!(
            heap.allocate_function_bytecode(initializer_eval),
            Err(HeapError::Invariant(
                "parameter initializer eval captured a body-only local"
            ))
        );

        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn eval_variable_object_local_requires_exact_metadata_authentication() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let name = Atom::from_raw(43);
        let make_bytecode = |metadata_slot, definition_kind| {
            let mut bytecode = bytecode(&code, context, Vec::new(), vec![name]);
            bytecode.metadata.local_count = 1;
            bytecode.metadata.eval_variable_object_local = metadata_slot;
            bytecode.local_definitions = Rc::from([VariableDefinition {
                name: Some(name),
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: definition_kind,
            }]);
            bytecode
        };

        assert_eq!(
            heap.allocate_function_bytecode(make_bytecode(Some(0), ClosureVariableKind::Normal)),
            Err(HeapError::Invariant(
                "eval variable-object definition disagrees with bytecode metadata"
            ))
        );
        assert_eq!(
            heap.allocate_function_bytecode(make_bytecode(
                None,
                ClosureVariableKind::EvalVariableObject
            )),
            Err(HeapError::Invariant(
                "ordinary local definition uses a non-local binding kind"
            ))
        );

        let published = heap
            .allocate_function_bytecode(make_bytecode(
                Some(0),
                ClosureVariableKind::EvalVariableObject,
            ))
            .unwrap();
        assert_eq!(
            heap.release_function_bytecode(published).unwrap().atoms,
            vec![name]
        );
        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
    }

    #[test]
    fn strict_script_global_eval_anchor_is_fail_closed_at_heap_boundary() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let code: Rc<[Instruction]> = Rc::from([
            Instruction::Undefined,
            Instruction::Eval {
                argument_count: 0,
                environment: 0,
            },
            Instruction::Return,
        ]);
        let make_bytecode = |scopes: Vec<EvalScope<Atom>>, eval_kind, variable_environment| {
            let mut bytecode = bytecode(&code, context, Vec::new(), Vec::new());
            bytecode.metadata.strict = true;
            bytecode.metadata.eval_kind = eval_kind;
            bytecode.eval_environments = Rc::from([EvalEnvironment {
                scopes: scopes.into_boxed_slice(),
                variable_environment,
                caller_strict: true,
                super_call_allowed: false,
                super_allowed: false,
            }]);
            bytecode
        };
        let script_scopes = || {
            vec![
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
        };

        let published = heap
            .allocate_function_bytecode(make_bytecode(
                script_scopes(),
                EvalKind::None,
                EvalVariableEnvironment::Global,
            ))
            .unwrap();
        heap.release_function_bytecode(published).unwrap();

        assert_eq!(
            heap.allocate_function_bytecode(make_bytecode(
                script_scopes(),
                EvalKind::None,
                EvalVariableEnvironment::StrictLocal(1),
            )),
            Err(HeapError::Invariant(
                "authored Script eval environment used a non-canonical strict-local target"
            ))
        );
        let synthetic = heap
            .allocate_function_bytecode(make_bytecode(
                script_scopes(),
                EvalKind::Direct,
                EvalVariableEnvironment::StrictLocal(1),
            ))
            .unwrap();
        heap.release_function_bytecode(synthetic).unwrap();

        let function_scopes = vec![
            EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ];
        assert_eq!(
            heap.allocate_function_bytecode(make_bytecode(
                function_scopes,
                EvalKind::None,
                EvalVariableEnvironment::Global,
            )),
            Err(HeapError::Invariant(
                "global eval variable environment escaped an authored Script root"
            ))
        );
        assert_eq!(
            heap.allocate_function_bytecode(make_bytecode(
                script_scopes(),
                EvalKind::Direct,
                EvalVariableEnvironment::Global,
            )),
            Err(HeapError::Invariant(
                "global eval variable environment escaped an authored Script root"
            ))
        );

        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
    }

    #[test]
    fn with_object_metadata_is_fail_closed_at_the_heap_boundary() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let eval_code: Rc<[Instruction]> = Rc::from([
            Instruction::Undefined,
            Instruction::Eval {
                argument_count: 0,
                environment: 0,
            },
            Instruction::Return,
        ]);
        let name = Atom::from_raw(45);
        let mut local = bytecode(&code, context, Vec::new(), vec![name]);
        local.metadata.local_count = 1;
        local.local_definitions = Rc::from([VariableDefinition {
            name: Some(name),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::WithObject,
        }]);
        let published = heap.allocate_function_bytecode(local).unwrap();
        assert_eq!(
            heap.release_function_bytecode(published).unwrap().atoms,
            vec![name]
        );

        let mut lexical = bytecode(&code, context, Vec::new(), vec![name]);
        lexical.metadata.local_count = 1;
        lexical.local_definitions = Rc::from([VariableDefinition {
            name: Some(name),
            is_lexical: true,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::WithObject,
        }]);
        assert_eq!(
            heap.allocate_function_bytecode(lexical),
            Err(HeapError::Invariant(
                "strict or malformed bytecode contains a with-object local"
            ))
        );

        let mut strict = bytecode(&code, context, Vec::new(), vec![name]);
        strict.metadata.local_count = 1;
        strict.metadata.strict = true;
        strict.local_definitions = Rc::from([VariableDefinition {
            name: Some(name),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::WithObject,
        }]);
        assert_eq!(
            heap.allocate_function_bytecode(strict),
            Err(HeapError::Invariant(
                "strict or malformed bytecode contains a with-object local"
            ))
        );

        let mut argument_source = bytecode(&code, context, Vec::new(), vec![name]);
        argument_source.metadata.argument_count = 1;
        argument_source.metadata.defined_argument_count = 1;
        argument_source.argument_definitions = Rc::from([VariableDefinition {
            name: Some(name),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        }]);
        argument_source.code = eval_code.clone();
        argument_source.eval_environments = Rc::from([EvalEnvironment {
            scopes: Box::new([
                EvalScope {
                    kind: EvalScopeKind::With,
                    bindings: Box::new([EvalBinding {
                        name,
                        source: EvalBindingSource::Argument(0),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::WithObject,
                        is_catch_parameter: false,
                    }]),
                },
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]),
            variable_environment: EvalVariableEnvironment::Global,
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        }]);
        assert_eq!(
            heap.allocate_function_bytecode(argument_source),
            Err(HeapError::Invariant(
                "eval with-object binding metadata disagrees with its scope"
            ))
        );

        let other_name = Atom::from_raw(46);
        let with_scope = |source| {
            Rc::from([EvalEnvironment {
                scopes: Box::new([
                    EvalScope {
                        kind: EvalScopeKind::With,
                        bindings: Box::new([EvalBinding {
                            name: other_name,
                            source,
                            is_lexical: false,
                            is_const: false,
                            kind: ClosureVariableKind::WithObject,
                            is_catch_parameter: false,
                        }]),
                    },
                    EvalScope {
                        kind: EvalScopeKind::ProgramBody,
                        bindings: Box::new([]),
                    },
                    EvalScope {
                        kind: EvalScopeKind::FunctionRoot,
                        bindings: Box::new([]),
                    },
                ]),
                variable_environment: EvalVariableEnvironment::Global,
                caller_strict: false,
                super_call_allowed: false,
                super_allowed: false,
            }])
        };

        let mut local_name_mismatch = bytecode(&code, context, Vec::new(), vec![name, other_name]);
        local_name_mismatch.metadata.local_count = 1;
        local_name_mismatch.local_definitions = Rc::from([VariableDefinition {
            name: Some(name),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::WithObject,
        }]);
        local_name_mismatch.code = eval_code.clone();
        local_name_mismatch.eval_environments = with_scope(EvalBindingSource::Local(0));
        assert_eq!(
            heap.allocate_function_bytecode(local_name_mismatch),
            Err(HeapError::Invariant(
                "eval binding name atom disagrees with its source metadata"
            ))
        );

        let mut closure_name_mismatch =
            bytecode(&code, context, Vec::new(), vec![name, other_name]);
        closure_name_mismatch.metadata.closure_count = 1;
        closure_name_mismatch.closure_variables = Rc::from([ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Atom(name),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::WithObject,
        }]);
        closure_name_mismatch.code = eval_code;
        closure_name_mismatch.eval_environments = with_scope(EvalBindingSource::Closure(0));
        assert_eq!(
            heap.allocate_function_bytecode(closure_name_mismatch),
            Err(HeapError::Invariant(
                "eval binding name atom disagrees with its source metadata"
            ))
        );

        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
    }

    #[test]
    fn bytecode_allocation_requires_published_closure_name_atom_ownership() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();
        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let name = Atom::from_raw(47);

        let descriptor = |name| ClosureVariable {
            source: ClosureSource::Global,
            name,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        };
        for (name_kind, atoms) in [
            (ClosureVariableName::Constant(0), Vec::new()),
            (ClosureVariableName::Atom(name), Vec::new()),
        ] {
            let mut malformed = bytecode(&code, context, Vec::new(), atoms);
            malformed.metadata.closure_count = 1;
            malformed.closure_variables = vec![descriptor(name_kind)].into();
            assert!(matches!(
                heap.allocate_function_bytecode(malformed),
                Err(HeapError::Invariant(_))
            ));
        }

        let mut non_lexical_const = bytecode(&code, context, Vec::new(), vec![name]);
        non_lexical_const.metadata.closure_count = 1;
        non_lexical_const.closure_variables = vec![ClosureVariable {
            source: ClosureSource::GlobalDeclaration,
            name: ClosureVariableName::Atom(name),
            is_lexical: false,
            is_const: true,
            kind: ClosureVariableKind::Normal,
        }]
        .into();
        assert_eq!(
            heap.allocate_function_bytecode(non_lexical_const),
            Err(HeapError::Invariant(
                "a const closure descriptor must also be lexical"
            ))
        );

        let mut published = bytecode(&code, context, Vec::new(), vec![name]);
        published.metadata.closure_count = 1;
        published.closure_variables = vec![descriptor(ClosureVariableName::Atom(name))].into();
        let published = heap.allocate_function_bytecode(published).unwrap();
        assert_eq!(
            heap.release_function_bytecode(published).unwrap().atoms,
            vec![name]
        );
        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
    }

    #[test]
    fn eval_environment_closure_is_confined_to_direct_eval_root() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let name = Atom::from_raw(59);
        let make_bytecode = |kind| {
            let mut bytecode = bytecode(&code, context, Vec::new(), vec![name]);
            bytecode.metadata.eval_kind = kind;
            bytecode.metadata.closure_count = 1;
            bytecode.closure_variables = Rc::from([ClosureVariable {
                source: ClosureSource::EvalEnvironment(0),
                name: ClosureVariableName::Atom(name),
                is_lexical: true,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }]);
            bytecode
        };

        for kind in [EvalKind::None, EvalKind::Indirect] {
            assert_eq!(
                heap.allocate_function_bytecode(make_bytecode(kind)),
                Err(HeapError::Invariant(
                    "eval-environment closure escaped a direct-eval root"
                ))
            );
        }
        let direct = heap
            .allocate_function_bytecode(make_bytecode(EvalKind::Direct))
            .unwrap();
        assert_eq!(
            heap.release_function_bytecode(direct).unwrap().atoms,
            vec![name]
        );
        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
    }

    #[test]
    fn bytecode_allocation_requires_one_atom_reference_per_eval_binding_name() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let code: Rc<[Instruction]> = Rc::from([
            Instruction::Undefined,
            Instruction::Eval {
                argument_count: 0,
                environment: 0,
            },
            Instruction::Return,
        ]);
        let name = Atom::from_raw(53);
        let make_bytecode = |owned_names: Vec<Atom>| {
            let mut bytecode = bytecode(&code, context, Vec::new(), owned_names);
            bytecode.metadata.local_count = 1;
            bytecode.local_definitions = Rc::from([VariableDefinition {
                name: Some(name),
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::Normal,
            }]);
            bytecode.eval_environments = Rc::from([EvalEnvironment {
                scopes: vec![
                    EvalScope {
                        kind: EvalScopeKind::ProgramBody,
                        bindings: Box::new([]),
                    },
                    EvalScope {
                        kind: EvalScopeKind::FunctionRoot,
                        bindings: vec![EvalBinding {
                            name,
                            source: EvalBindingSource::Local(0),
                            is_lexical: false,
                            is_const: false,
                            kind: ClosureVariableKind::Normal,
                            is_catch_parameter: false,
                        }]
                        .into_boxed_slice(),
                    },
                ]
                .into_boxed_slice(),
                variable_environment: EvalVariableEnvironment::Global,
                caller_strict: false,
                super_call_allowed: false,
                super_allowed: false,
            }]);
            bytecode
        };

        assert_eq!(
            heap.allocate_function_bytecode(make_bytecode(vec![name])),
            Err(HeapError::Invariant(
                "eval binding name atom ownership multiplicity is too small"
            ))
        );
        let published = heap
            .allocate_function_bytecode(make_bytecode(vec![name, name]))
            .unwrap();
        assert_eq!(
            heap.release_function_bytecode(published).unwrap().atoms,
            vec![name, name]
        );
        heap.release_context(context).unwrap();
        heap.release_shape(shape).unwrap();
    }

    #[test]
    fn two_closures_share_one_mutable_var_ref_cell() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();

        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let bytecode = heap
            .allocate_function_bytecode(closure_bytecode(&code, context, 1))
            .unwrap();
        let cell = heap
            .allocate_var_ref(VarRefData::local(RawValue::Int(1)))
            .unwrap();
        let first = heap
            .allocate_object(ObjectData::bytecode_function_with_closures(
                shape,
                Vec::new(),
                bytecode,
                None,
                vec![cell],
                false,
            ))
            .unwrap();
        let second = heap
            .allocate_object(ObjectData::bytecode_function_with_closures(
                shape,
                Vec::new(),
                bytecode,
                None,
                vec![cell],
                false,
            ))
            .unwrap();

        assert_eq!(heap.var_ref_strong_count(cell), Ok(3));
        for function in [first, second] {
            let ObjectPayload::BytecodeFunction { closure_slots, .. } =
                &heap.object(function).unwrap().payload
            else {
                panic!("expected a bytecode function payload");
            };
            assert_eq!(closure_slots, &[cell]);
        }

        assert_eq!(
            heap.replace_var_ref_value(cell, RawValue::Int(9)).unwrap(),
            HeapCleanup::default()
        );
        assert_eq!(heap.var_ref(cell).unwrap().value, RawValue::Int(9));

        assert_eq!(heap.release_var_ref(cell).unwrap(), HeapCleanup::default());
        assert_eq!(heap.release_object(first).unwrap().finalized_objects, 1);
        let cleanup = heap.release_object(second).unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert_eq!(cleanup.finalized_var_refs, 1);

        heap.release_shape(shape).unwrap();
        heap.release_function_bytecode(bytecode).unwrap();
        let cleanup = heap.release_context(context).unwrap();
        assert_eq!(cleanup.finalized_contexts, 1);
        assert_eq!(cleanup.finalized_objects, 1);
        assert_eq!(cleanup.finalized_shapes, 1);
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn bytecode_home_object_replacement_is_retain_first_and_idempotent() {
        let mut heap = Heap::new();
        assert!(!FunctionMetadata::default().needs_home_object);

        let empty = empty_shape(&mut heap);
        let realm_root = leaf(&mut heap, empty);
        let context = heap
            .allocate_context(ContextData::new(
                realm_root, realm_root, realm_root, realm_root, realm_root, realm_root, realm_root,
                realm_root,
            ))
            .unwrap();
        heap.release_object(realm_root).unwrap();

        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let bytecode = heap
            .allocate_function_bytecode(bytecode(&code, context, Vec::new(), Vec::new()))
            .unwrap();
        let function = heap
            .allocate_object(ObjectData::bytecode_function(
                empty,
                Vec::new(),
                bytecode,
                None,
                false,
            ))
            .unwrap();
        assert_eq!(heap.bytecode_function_home_object(function), Ok(None));
        assert_eq!(
            heap.bytecode_function_home_object(realm_root),
            Err(HeapError::Invariant(
                "HomeObject lookup reached a non-bytecode function"
            ))
        );

        let replacement = leaf(&mut heap, empty);
        let owner_shape = one_slot_shape(&mut heap);
        let previous = heap
            .allocate_object(ObjectData::ordinary(
                owner_shape,
                vec![PropertySlot::Data(RawValue::Object(replacement))],
            ))
            .unwrap();
        assert_eq!(heap.object_strong_count(replacement), Ok(2));

        assert_eq!(
            heap.replace_bytecode_function_home_object(function, Some(previous))
                .unwrap(),
            HeapCleanup::default()
        );
        heap.release_object(previous).unwrap();
        heap.release_object(replacement).unwrap();
        assert_eq!(heap.object_strong_count(previous), Ok(1));
        assert_eq!(heap.object_strong_count(replacement), Ok(1));

        // `previous` owns `replacement`. Retaining the new edge first keeps it
        // live while detaching and finalizing the old HomeObject.
        let cleanup = heap
            .replace_bytecode_function_home_object(function, Some(replacement))
            .unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert_eq!(
            heap.bytecode_function_home_object(function),
            Ok(Some(replacement))
        );
        assert_eq!(heap.object_strong_count(replacement), Ok(1));

        assert_eq!(
            heap.replace_bytecode_function_home_object(function, Some(replacement))
                .unwrap(),
            HeapCleanup::default()
        );
        assert_eq!(heap.object_strong_count(replacement), Ok(1));

        let cleanup = heap
            .replace_bytecode_function_home_object(function, None)
            .unwrap();
        assert_eq!(cleanup.finalized_objects, 1);
        assert_eq!(heap.bytecode_function_home_object(function), Ok(None));
        assert_eq!(
            heap.replace_bytecode_function_home_object(function, None)
                .unwrap(),
            HeapCleanup::default()
        );

        assert_eq!(heap.release_object(function).unwrap().finalized_objects, 1);
        heap.release_function_bytecode(bytecode).unwrap();
        heap.release_context(context).unwrap();
        heap.release_shape(owner_shape).unwrap();
        heap.release_shape(empty).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn bytecode_home_object_edge_participates_in_cycle_collection() {
        let mut heap = Heap::new();
        let empty = empty_shape(&mut heap);
        let realm_root = leaf(&mut heap, empty);
        let context = heap
            .allocate_context(ContextData::new(
                realm_root, realm_root, realm_root, realm_root, realm_root, realm_root, realm_root,
                realm_root,
            ))
            .unwrap();
        heap.release_object(realm_root).unwrap();

        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let bytecode = heap
            .allocate_function_bytecode(bytecode(&code, context, Vec::new(), Vec::new()))
            .unwrap();
        let function = heap
            .allocate_object(ObjectData::bytecode_function(
                empty,
                Vec::new(),
                bytecode,
                None,
                false,
            ))
            .unwrap();
        let home_shape = one_slot_shape(&mut heap);
        let home_object = heap
            .allocate_object(ObjectData::ordinary(
                home_shape,
                vec![PropertySlot::Data(RawValue::Undefined)],
            ))
            .unwrap();

        heap.replace_bytecode_function_home_object(function, Some(home_object))
            .unwrap();
        heap.replace_object_slot(
            home_object,
            0,
            PropertySlot::Data(RawValue::Object(function)),
        )
        .unwrap();
        heap.release_object(function).unwrap();
        heap.release_object(home_object).unwrap();

        let stats = heap.run_gc().unwrap();
        assert_eq!(stats.candidate_nodes, 2);
        assert_eq!(stats.cleanup.finalized_objects, 2);

        heap.release_function_bytecode(bytecode).unwrap();
        heap.release_context(context).unwrap();
        heap.release_shape(home_shape).unwrap();
        heap.release_shape(empty).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn autoinit_property_slot_retains_and_releases_its_creation_realm() {
        let mut heap = Heap::new();
        let empty = empty_shape(&mut heap);
        let property_shape = one_slot_shape(&mut heap);
        let prototype = leaf(&mut heap, empty);
        let realm = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        assert_eq!(heap.context_strong_count(realm), Ok(1));

        let function = heap
            .allocate_object(ObjectData::ordinary(
                property_shape,
                vec![PropertySlot::AutoInit(
                    AutoInitProperty::FunctionPrototype { realm },
                )],
            ))
            .unwrap();
        assert_eq!(heap.context_strong_count(realm), Ok(2));

        heap.replace_object_slot(function, 0, PropertySlot::Data(RawValue::Undefined))
            .unwrap();
        assert_eq!(heap.context_strong_count(realm), Ok(1));

        heap.release_object(function).unwrap();
        heap.release_context(realm).unwrap();
        heap.release_object(prototype).unwrap();
        heap.release_shape(property_shape).unwrap();
        heap.release_shape(empty).unwrap();
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn function_var_ref_object_cycle_is_collected() {
        let mut heap = Heap::new();
        let empty = empty_shape(&mut heap);
        let data = one_slot_shape(&mut heap);
        let prototype = leaf(&mut heap, empty);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let bytecode = heap
            .allocate_function_bytecode(closure_bytecode(&code, context, 1))
            .unwrap();
        let captured = heap
            .allocate_object(ObjectData::ordinary(
                data,
                vec![PropertySlot::Data(RawValue::Undefined)],
            ))
            .unwrap();
        let cell = heap
            .allocate_var_ref(VarRefData::local(RawValue::Object(captured)))
            .unwrap();
        let function = heap
            .allocate_object(ObjectData::bytecode_function_with_closures(
                empty,
                Vec::new(),
                bytecode,
                None,
                vec![cell],
                false,
            ))
            .unwrap();
        heap.replace_object_slot(captured, 0, PropertySlot::Data(RawValue::Object(function)))
            .unwrap();

        heap.release_shape(empty).unwrap();
        heap.release_shape(data).unwrap();
        heap.release_object(prototype).unwrap();
        heap.release_context(context).unwrap();
        heap.release_function_bytecode(bytecode).unwrap();
        heap.release_object(captured).unwrap();
        heap.release_var_ref(cell).unwrap();
        heap.release_object(function).unwrap();

        assert_eq!(heap.counts().live, 8);
        let stats = heap.run_gc().unwrap();
        assert_eq!(stats.candidate_nodes, 8);
        assert_eq!(stats.cleanup.finalized_objects, 3);
        assert_eq!(stats.cleanup.finalized_shapes, 2);
        assert_eq!(stats.cleanup.finalized_var_refs, 1);
        assert_eq!(stats.cleanup.finalized_contexts, 1);
        assert_eq!(stats.cleanup.finalized_function_bytecodes, 1);
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn fifty_thousand_closure_cells_finalize_iteratively() {
        const LENGTH: usize = 50_000;

        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();
        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let bytecode = heap
            .allocate_function_bytecode(closure_bytecode(&code, context, 1))
            .unwrap();

        let mut head = None;
        for _ in 0..LENGTH {
            let value = head.map_or(RawValue::Undefined, RawValue::Object);
            let cell = heap.allocate_var_ref(VarRefData::local(value)).unwrap();
            let function = heap
                .allocate_object(ObjectData::bytecode_function_with_closures(
                    shape,
                    Vec::new(),
                    bytecode,
                    None,
                    vec![cell],
                    false,
                ))
                .unwrap();
            heap.release_var_ref(cell).unwrap();
            if let Some(previous) = head {
                assert_eq!(
                    heap.release_object(previous).unwrap(),
                    HeapCleanup::default()
                );
            }
            head = Some(function);
        }

        heap.release_shape(shape).unwrap();
        let cleanup = heap.release_object(head.unwrap()).unwrap();
        assert_eq!(cleanup.finalized_objects, LENGTH);
        assert_eq!(cleanup.finalized_var_refs, LENGTH);
        assert_eq!(heap.counts().object_nodes, 1);
        assert_eq!(heap.counts().var_ref_nodes, 0);

        heap.release_function_bytecode(bytecode).unwrap();
        let cleanup = heap.release_context(context).unwrap();
        assert_eq!(cleanup.finalized_contexts, 1);
        assert_eq!(cleanup.finalized_objects, 1);
        assert_eq!(cleanup.finalized_shapes, 1);
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn context_object_bytecode_realm_cycle_is_collected() {
        let mut heap = Heap::new();
        let shape = one_slot_shape(&mut heap);
        let prototype = heap
            .allocate_object(ObjectData::ordinary(
                shape,
                vec![PropertySlot::Data(RawValue::Undefined)],
            ))
            .unwrap();
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let function_bytecode = heap
            .allocate_function_bytecode(bytecode(&code, context, Vec::new(), Vec::new()))
            .unwrap();
        let function = heap
            .allocate_object(ObjectData::bytecode_function(
                shape,
                vec![PropertySlot::Data(RawValue::Undefined)],
                function_bytecode,
                None,
                false,
            ))
            .unwrap();
        heap.replace_object_slot(prototype, 0, PropertySlot::Data(RawValue::Object(function)))
            .unwrap();

        heap.release_shape(shape).unwrap();
        heap.release_object(prototype).unwrap();
        heap.release_context(context).unwrap();
        heap.release_function_bytecode(function_bytecode).unwrap();
        heap.release_object(function).unwrap();

        assert_eq!(heap.counts().live, 5);
        let stats = heap.run_gc().unwrap();
        assert_eq!(stats.candidate_nodes, 5);
        assert_eq!(stats.cleanup.finalized_objects, 2);
        assert_eq!(stats.cleanup.finalized_shapes, 1);
        assert_eq!(stats.cleanup.finalized_contexts, 1);
        assert_eq!(stats.cleanup.finalized_function_bytecodes, 1);
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn bytecode_constant_pool_owns_child_and_returns_all_atoms() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();
        heap.release_shape(shape).unwrap();

        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let child_atom = Atom::from_raw(41);
        let parent_atom = Atom::from_raw(42);
        let symbol_atom = Atom::from_raw(43);
        let child = heap
            .allocate_function_bytecode(bytecode(&code, context, Vec::new(), vec![child_atom]))
            .unwrap();
        let parent = heap
            .allocate_function_bytecode(bytecode(
                &code,
                context,
                vec![
                    BytecodeConstant::Function(child),
                    BytecodeConstant::Value(RawValue::Symbol(symbol_atom)),
                ],
                vec![parent_atom],
            ))
            .unwrap();
        assert_eq!(heap.function_bytecode_strong_count(child), Ok(2));

        assert_eq!(
            heap.release_function_bytecode(child).unwrap(),
            HeapCleanup::default()
        );
        let mut cleanup = heap.release_function_bytecode(parent).unwrap();
        assert_eq!(cleanup.finalized_function_bytecodes, 2);
        cleanup.atoms.sort_unstable();
        let mut expected = vec![child_atom, parent_atom, symbol_atom];
        expected.sort_unstable();
        assert_eq!(cleanup.atoms, expected);

        let context_cleanup = heap.release_context(context).unwrap();
        assert_eq!(context_cleanup.finalized_contexts, 1);
        assert_eq!(context_cleanup.finalized_objects, 1);
        assert_eq!(context_cleanup.finalized_shapes, 1);
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn bytecode_regexp_constant_releases_its_rc_leaf_on_finalization() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();
        heap.release_shape(shape).unwrap();

        let pattern = JsString::from_static("a");
        let flags = JsString::from_static("g");
        let program = Rc::new(crate::regexp::compile(&pattern, &flags).unwrap());
        let weak = Rc::downgrade(&program);
        let code: Rc<[Instruction]> = Rc::from([Instruction::RegExp(0), Instruction::Return]);
        let function = heap
            .allocate_function_bytecode(bytecode(
                &code,
                context,
                vec![BytecodeConstant::RegExp { pattern, program }],
                Vec::new(),
            ))
            .unwrap();
        assert_eq!(weak.strong_count(), 1);

        let cleanup = heap.release_function_bytecode(function).unwrap();
        assert_eq!(cleanup.finalized_function_bytecodes, 1);
        assert_eq!(weak.strong_count(), 0);

        let context_cleanup = heap.release_context(context).unwrap();
        assert_eq!(context_cleanup.finalized_contexts, 1);
        assert_eq!(context_cleanup.finalized_objects, 1);
        assert_eq!(context_cleanup.finalized_shapes, 1);
        assert_eq!(heap.counts().live, 0);
    }

    #[test]
    fn long_bytecode_constant_chain_finalizes_iteratively() {
        const LENGTH: usize = 50_000;

        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let prototype = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                prototype, prototype, prototype, prototype, prototype, prototype, prototype,
                prototype,
            ))
            .unwrap();
        heap.release_object(prototype).unwrap();
        heap.release_shape(shape).unwrap();

        let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
        let mut head = heap
            .allocate_function_bytecode(bytecode(&code, context, Vec::new(), Vec::new()))
            .unwrap();
        for _ in 1..LENGTH {
            let next = heap
                .allocate_function_bytecode(bytecode(
                    &code,
                    context,
                    vec![BytecodeConstant::Function(head)],
                    Vec::new(),
                ))
                .unwrap();
            assert_eq!(
                heap.release_function_bytecode(head).unwrap(),
                HeapCleanup::default()
            );
            head = next;
        }

        let cleanup = heap.release_function_bytecode(head).unwrap();
        assert_eq!(cleanup.finalized_function_bytecodes, LENGTH);
        assert_eq!(heap.counts().function_bytecode_nodes, 0);
        let cleanup = heap.release_context(context).unwrap();
        assert_eq!(cleanup.finalized_contexts, 1);
        assert_eq!(cleanup.finalized_objects, 1);
        assert_eq!(cleanup.finalized_shapes, 1);
    }
}
