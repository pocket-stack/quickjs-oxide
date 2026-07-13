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

use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt;
use std::rc::Rc;

use crate::atom::Atom;
use crate::bigint::JsBigInt;
use crate::bytecode::{Instruction, MAX_LOCAL_SLOTS};
use crate::debug::Pc2LineTable;
use crate::error::NativeErrorKind;
use crate::shape::{PropertyStorageKind, Shape};
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
    #[cfg(test)]
    FailureProbe {
        realm: ContextId,
    },
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
    /// Realm-local `%StringIteratorPrototype%`, whose prototype is the realm's
    /// `%IteratorPrototype%`.
    pub string_iterator_prototype: ObjectId,
    /// Realm-local equivalents of QuickJS `class_proto[JS_CLASS_*]` for the
    /// five primitive wrapper classes. An absent entry remains an explicit
    /// implementation gap rather than inheriting from the wrong prototype.
    pub primitive_prototypes: [Option<ObjectId>; PrimitiveKind::COUNT],
    /// `%Function%`, published after the cyclic realm bootstrap has created
    /// `%Function.prototype%` and the global object.
    pub function_constructor: Option<ObjectId>,
    /// Shared frozen poison callable used by legacy restricted function
    /// accessors and future restricted arguments objects.
    pub throw_type_error: Option<ObjectId>,
    pub global_object: ObjectId,
    /// Null-prototype storage for global lexical bindings (`let`/`const`).
    pub global_var_object: ObjectId,
    pub error_prototype: Option<ObjectId>,
    pub native_error_prototypes: [Option<ObjectId>; NativeErrorKind::COUNT],
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
            string_iterator_prototype,
            primitive_prototypes: [None; PrimitiveKind::COUNT],
            function_constructor: None,
            throw_type_error: None,
            global_object,
            global_var_object,
            error_prototype: None,
            native_error_prototypes: [None; NativeErrorKind::COUNT],
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
    Function(FunctionBytecodeId),
}

/// Immutable execution metadata kept beside bytecode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FunctionMetadata {
    pub argument_count: u16,
    /// Number of leading parameters before the first default/rest parameter.
    /// This is the observable `length`, distinct from frame slot count.
    pub defined_argument_count: u16,
    pub local_count: u16,
    /// Synthetic local initialized to the active function object for a named
    /// function expression. This is the typed equivalent of QuickJS's
    /// `func_var_idx` entry prologue.
    pub function_name_local: Option<u16>,
    pub closure_count: u16,
    pub max_stack: u16,
    pub strict: bool,
    /// Source-level callable kind used by `Function.prototype.toString` when
    /// debug source has been stripped. This mirrors QuickJS `func_kind`
    /// independently from constructor protocol.
    pub function_kind: FunctionKind,
    /// Whether closure instantiation defines an own `.prototype` property.
    pub has_prototype: bool,
    /// Base/derived constructor protocol carried by QuickJS bytecode.
    pub constructor_kind: ConstructorKind,
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
    pub kind: ClosureVariableKind,
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
    /// Intrinsic source-level name. Contextual `SetName` inference remains a
    /// separate opcode and is only emitted for anonymous definitions.
    pub func_name: Option<JsString>,
    pub argument_definitions: Rc<[VariableDefinition]>,
    pub local_definitions: Rc<[VariableDefinition]>,
    pub closure_variables: Rc<[ClosureVariable]>,
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

/// Class-specific edges stored alongside an object's ordinary properties.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectPayload {
    Ordinary,
    /// A genuine `JS_CLASS_ARRAY` exotic object. Indexed elements and the
    /// mandatory length property remain in the ordinary shape/slot arrays;
    /// this class marker selects ArraySetLength and index-growth semantics at
    /// the runtime boundary without disguising an Array as an ordinary object.
    Array,
    /// `JS_CLASS_ARRAY_ITERATOR`: the boxed source is released permanently at
    /// exhaustion, while `kind` selects keys, values, or entry pairs.
    ArrayIterator {
        object: Option<ObjectId>,
        next_index: u32,
        kind: ArrayIteratorKind,
    },
    /// QuickJS `JSObject.u.object_data` for implemented primitive wrappers.
    Primitive(PrimitiveObjectData),
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
        /// One owned reference per bytecode closure slot, matching QuickJS's
        /// `JSObject.u.func.var_refs[]` ownership.
        closure_slots: Vec<VarRefId>,
    },
}

/// Object storage category.  Additional QuickJS classes will extend this enum
/// while retaining the same arena and collection protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Ordinary,
    Array,
    ArrayIterator,
    Primitive,
    GlobalObject,
    Error,
    StringIterator,
    NativeFunction,
    BoundFunction,
    BytecodeFunction,
}

/// Observable mode selected by `Array.prototype.keys`, `values`, or
/// `entries`. QuickJS stores this in `JSArrayIteratorData.kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArrayIteratorKind {
    Key,
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

/// Runtime-provided callable identities. The enum is stored in heap payloads
/// so native dispatch stays typed and does not rely on function pointers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeFunctionId {
    FunctionPrototype,
    FunctionConstructor(DynamicFunctionKind),
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
    PrimitiveConstructor(PrimitiveKind),
    PrimitivePrototypeToString(PrimitiveKind),
    PrimitivePrototypeValueOf(PrimitiveKind),
    StringPrototypeCharAt(StringCharAtKind),
    StringPrototypeCharCodeAt,
    StringPrototypeConcat,
    StringPrototypeCodePointAt,
    StringPrototypeWellFormed(StringWellFormedKind),
    StringPrototypeIndexOf(StringIndexOfKind),
    IteratorPrototypeIterator,
    IteratorPrototypeToStringTagGetter,
    IteratorPrototypeToStringTagSetter,
    StringPrototypeIterator,
    StringIteratorNext,
    SymbolRegistry(SymbolRegistryKind),
    SymbolPrototypeDescription,
    BigIntAsN(BigIntAsNKind),
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
            | Self::PrimitivePrototypeToString(_)
            | Self::PrimitivePrototypeValueOf(_)
            | Self::StringPrototypeCharCodeAt
            | Self::StringPrototypeConcat
            | Self::StringPrototypeCodePointAt
            | Self::StringPrototypeWellFormed(_)
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
            | Self::StringPrototypeCharAt(_)
            | Self::StringPrototypeIndexOf(_)
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
            Self::ArrayConstructor | Self::ObjectConstructor => NativeFunctionDescriptor {
                cproto: NativeCProto::ConstructorOrFunction,
            },
            Self::FunctionPrototypeFileName | Self::ObjectPrototypeProtoGetter => {
                NativeFunctionDescriptor {
                    cproto: NativeCProto::Getter,
                }
            }
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
            Self::StringIteratorNext | Self::ArrayIteratorNext => NativeFunctionDescriptor {
                cproto: NativeCProto::IteratorNext,
            },
            Self::FunctionPrototypePosition(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::GetterMagic,
            },
            Self::ErrorConstructor(_) => NativeFunctionDescriptor {
                cproto: NativeCProto::ConstructorOrFunctionMagic,
            },
            Self::ErrorPrototypeToString | Self::ErrorIsError => NativeFunctionDescriptor {
                cproto: NativeCProto::Generic,
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
#[derive(Debug, PartialEq)]
pub struct ObjectData {
    pub shape: ShapeId,
    pub slots: Vec<PropertySlot>,
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
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Ordinary,
            payload: ObjectPayload::Ordinary,
        }
    }

    /// Construct one genuine Array exotic object. The caller supplies the
    /// validated `length`-first layout used by QuickJS's initial Array shape.
    #[must_use]
    pub const fn array(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Array,
            payload: ObjectPayload::Array,
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
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Primitive,
            payload: ObjectPayload::Primitive(data),
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
            extensible: true,
            immutable_prototype: false,
            is_constructor,
            kind: ObjectKind::BytecodeFunction,
            payload: ObjectPayload::BytecodeFunction {
                bytecode,
                home_object,
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
            extensible: true,
            immutable_prototype: false,
            is_constructor,
            kind: ObjectKind::BytecodeFunction,
            payload: ObjectPayload::BytecodeFunction {
                bytecode,
                home_object,
                closure_slots,
            },
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
                data: NativeFunctionData { realm: None, .. }
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
                }
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
            } => {}
            ObjectPayload::NativeFunction {
                data: NativeFunctionData { realm: Some(_), .. },
            } => {
                return Err(HeapError::Invariant(
                    "native function already has a defining realm",
                ));
            }
            ObjectPayload::Ordinary
            | ObjectPayload::Array
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::Primitive(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. } => {
                return Err(HeapError::Invariant(
                    "attempted to attach a native realm to a non-native function",
                ));
            }
        }

        self.retain_raw(RawId::Context(realm), 1)?;
        let ObjectPayload::NativeFunction { data } = &mut self.object_mut(object)?.payload else {
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
                }
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
                    }
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
                    }
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

    /// Allocate and publish immutable function bytecode, retaining its realm
    /// and every GC edge in its constant pool. `auxiliary_atoms` and symbol
    /// constants transfer to the node on success.
    pub fn allocate_function_bytecode(
        &mut self,
        bytecode: FunctionBytecodeData,
    ) -> Result<FunctionBytecodeId, HeapError> {
        if bytecode.metadata.local_count > MAX_LOCAL_SLOTS {
            return Err(HeapError::Invariant(
                "bytecode local count exceeds QuickJS JS_MAX_LOCAL_VARS",
            ));
        }
        if bytecode.metadata.defined_argument_count > bytecode.metadata.argument_count {
            return Err(HeapError::Invariant(
                "defined argument count exceeds function argument slots",
            ));
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
        for definition in bytecode.argument_definitions.iter() {
            if definition.kind != ClosureVariableKind::Normal
                || definition.is_lexical
                || definition.is_const
            {
                return Err(HeapError::Invariant(
                    "argument definition is not an ordinary mutable binding",
                ));
            }
        }
        for (index, definition) in bytecode.local_definitions.iter().enumerate() {
            let is_function_name =
                bytecode.metadata.function_name_local == u16::try_from(index).ok();
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
            } else if definition.kind != ClosureVariableKind::Normal {
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
            if descriptor.kind == ClosureVariableKind::GlobalFunction
                && (descriptor.is_lexical || descriptor.is_const)
            {
                return Err(HeapError::Invariant(
                    "global function declaration descriptor has lexical metadata",
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
            ) || descriptor.kind == ClosureVariableKind::FunctionName;
            let allows_name = requires_name || descriptor.is_lexical;
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
        self.var_ref(id)?;
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
        let (extensible, immutable_prototype, is_constructor, kind, payload) = {
            let object = self.object(id)?;
            (
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
        // The class payload is cloned unchanged into `replacement`, so its
        // non-GC atom ownership transfers in place. Only detached property
        // slots relinquish atom references here.
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
            (ObjectKind::Ordinary, ObjectPayload::Ordinary)
                | (ObjectKind::Array, ObjectPayload::Array)
                | (
                    ObjectKind::ArrayIterator,
                    ObjectPayload::ArrayIterator { .. }
                )
                | (ObjectKind::Primitive, ObjectPayload::Primitive(_))
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
        }
        if let ObjectPayload::BytecodeFunction {
            bytecode,
            closure_slots,
            ..
        } = &object.payload
        {
            let expected = usize::from(self.function_bytecode(*bytecode)?.metadata.closure_count);
            if closure_slots.len() != expected {
                return Err(HeapError::Invariant(
                    "function closure slot count does not match its bytecode metadata",
                ));
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
                .any(|value| matches!(value, RawValue::Uninitialized | RawValue::Exception))
            {
                return Err(HeapError::Invariant(
                    "bound function payload contains an internal value sentinel",
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
        | ObjectPayload::Array
        | ObjectPayload::ArrayIterator { .. }
        | ObjectPayload::Primitive(_)
        | ObjectPayload::GlobalObject { .. }
        | ObjectPayload::Error
        | ObjectPayload::StringIterator { .. }
        | ObjectPayload::NativeFunction { .. } => 0,
        ObjectPayload::BoundFunction { arguments, .. } => arguments.len().saturating_add(2),
        ObjectPayload::BytecodeFunction { closure_slots, .. } => closure_slots.len(),
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
        | ObjectPayload::Array
        | ObjectPayload::Primitive(_)
        | ObjectPayload::Error
        | ObjectPayload::StringIterator { .. } => {}
        ObjectPayload::ArrayIterator { object, .. } => {
            edges.extend(object.map(RawId::Object));
        }
        ObjectPayload::GlobalObject { uninitialized_vars } => {
            edges.push(RawId::Object(*uninitialized_vars))
        }
        ObjectPayload::NativeFunction { data } => {
            edges.extend(data.realm.map(RawId::Context));
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
            closure_slots,
        } => {
            if let Some(home_object) = home_object {
                edges.push(RawId::Object(*home_object));
            }
            edges.push(RawId::FunctionBytecode(*bytecode));
            edges.extend(closure_slots.iter().copied().map(RawId::VarRef));
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
            | AutoInitProperty::ArrayUnscopables { realm },
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
        | RawValue::Uninitialized
        | RawValue::Exception => Vec::new(),
    }
}

fn context_edges(context: &ContextData) -> Vec<RawId> {
    let mut edges = Vec::with_capacity(
        10usize
            .saturating_add(PrimitiveKind::COUNT)
            .saturating_add(NativeErrorKind::COUNT)
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
    edges.extend(context.function_constructor.map(RawId::Object));
    edges.extend(context.array_constructor.map(RawId::Object));
    edges.extend(context.throw_type_error.map(RawId::Object));
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
        PropertySlot::Data(RawValue::Symbol(atom)) => Some(*atom),
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
        ObjectPayload::Ordinary
        | ObjectPayload::Array
        | ObjectPayload::ArrayIterator { .. }
        | ObjectPayload::GlobalObject { .. }
        | ObjectPayload::Error
        | ObjectPayload::StringIterator { .. }
        | ObjectPayload::NativeFunction { .. }
        | ObjectPayload::BytecodeFunction { .. } => Vec::new(),
    };
    object_slot_atoms(object).chain(payload)
}

fn raw_value_atom(value: &RawValue) -> Option<Atom> {
    match value {
        RawValue::Symbol(atom) => Some(*atom),
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
                    BytecodeConstant::Function(_) => None,
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

        heap.release_context(realm).unwrap();
        heap.release_object(function).unwrap();
        heap.release_object(prototype).unwrap();
        heap.release_shape(native_shape).unwrap();
        heap.release_shape(shape).unwrap();
        heap.run_gc().unwrap();
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
    fn numeric_and_uri_native_selectors_use_pinned_cproto() {
        let targets = [
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
        assert!(!NativeCProto::IteratorNext.default_is_constructor());
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
    fn context_roots_iterator_prototypes_until_realm_finalization() {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let object_prototype = leaf(&mut heap, shape);
        let function_prototype = leaf(&mut heap, shape);
        let array_prototype = leaf(&mut heap, shape);
        let iterator_prototype = leaf(&mut heap, shape);
        let array_iterator_prototype = leaf(&mut heap, shape);
        let string_iterator_prototype = leaf(&mut heap, shape);
        let global_object = leaf(&mut heap, shape);
        let global_var_object = leaf(&mut heap, shape);
        let context = heap
            .allocate_context(ContextData::new(
                object_prototype,
                function_prototype,
                array_prototype,
                iterator_prototype,
                array_iterator_prototype,
                string_iterator_prototype,
                global_object,
                global_var_object,
            ))
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

        for object in [
            object_prototype,
            function_prototype,
            array_prototype,
            iterator_prototype,
            array_iterator_prototype,
            string_iterator_prototype,
            global_object,
            global_var_object,
        ] {
            assert_eq!(heap.release_object(object).unwrap(), HeapCleanup::default());
            assert_eq!(heap.object_strong_count(object), Ok(1));
        }
        let cleanup = heap.release_context(context).unwrap();
        assert_eq!(cleanup.finalized_contexts, 1);
        assert_eq!(cleanup.finalized_objects, 8);
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
            metadata: FunctionMetadata::default(),
            func_name: None,
            argument_definitions: Rc::from([]),
            local_definitions: Rc::from([]),
            closure_variables: Rc::from([]),
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

        let code: Rc<[Instruction]> = Rc::from([]);
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
        lexical_argument.argument_definitions = Rc::from([VariableDefinition {
            name: None,
            is_lexical: true,
            is_const: false,
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
        let code: Rc<[Instruction]> = Rc::from([]);
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

        let code: Rc<[Instruction]> = Rc::from([]);
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
        let code: Rc<[Instruction]> = Rc::from([]);
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
        let code: Rc<[Instruction]> = Rc::from([]);
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
        let code: Rc<[Instruction]> = Rc::from([]);
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

        let code: Rc<[Instruction]> = Rc::from([]);
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

        let code: Rc<[Instruction]> = Rc::from([]);
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
