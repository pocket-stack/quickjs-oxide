use super::*;

/// Stable identity of an object slot until that slot is reclaimed.
///
/// The parts are exposed only for diagnostics.  There is intentionally no
/// public constructor: identities must originate from [`Heap::allocate_object`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId {
    pub(in crate::engine::heap) index: u32,
    pub(in crate::engine::heap) generation: u32,
}

impl ObjectId {
    /// Arena index, intended for diagnostics and serialized debug traces only.
    #[must_use]
    #[cfg(test)]
    pub const fn debug_index(self) -> u32 {
        self.index
    }

    /// Slot generation, intended for diagnostics and serialized debug traces.
    #[must_use]
    #[cfg(test)]
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
    pub(in crate::engine::heap) index: u32,
    pub(in crate::engine::heap) generation: u32,
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
    pub(in crate::engine::heap) index: u32,
    pub(in crate::engine::heap) generation: u32,
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
    pub(in crate::engine::heap) index: u32,
    pub(in crate::engine::heap) generation: u32,
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
    pub(in crate::engine::heap) index: u32,
    pub(in crate::engine::heap) generation: u32,
}

impl FunctionBytecodeId {
    /// Immutable publications cannot be replaced in place. Arena identity
    /// combines slot and generation, so reuse always changes this token.
    pub(crate) fn publish_generation(self) -> u64 {
        (u64::from(self.index) << 32) | u64::from(self.generation)
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

/// Stable identity of a string node until that slot is reclaimed.
///
/// String nodes own one `Rc<StringRepr>` payload (the public `JsString`) and
/// have no outgoing heap edges.  A value slot holding a `StringId` owns one
/// edge to this node; reading the payload clones the inner `Rc`, so string
/// identity (`ptr_eq`) semantics survive arena indirection unchanged.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StringId {
    pub(in crate::engine::heap) index: u32,
    pub(in crate::engine::heap) generation: u32,
}

impl StringId {
    /// Arena index, intended for diagnostics and serialized debug traces only.
    #[must_use]
    #[cfg(test)]
    pub const fn debug_index(self) -> u32 {
        self.index
    }

    /// Slot generation, intended for diagnostics and serialized debug traces.
    #[must_use]
    #[cfg(test)]
    pub const fn debug_generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Debug for StringId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StringId")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

/// Stable identity of a BigInt node until that slot is reclaimed.
///
/// BigInt nodes own one `JsBigInt` payload and have no outgoing heap edges.
/// As with strings, value slots holding a `BigIntId` own one edge, and reading
/// the payload hands out a clone of the payload itself.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BigIntId {
    pub(in crate::engine::heap) index: u32,
    pub(in crate::engine::heap) generation: u32,
}

impl fmt::Debug for BigIntId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BigIntId")
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
    String,
    BigInt,
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
    Allocation {
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
            Self::Allocation { operation } => {
                write!(formatter, "heap allocation failed while {operation}")
            }
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
/// `Clone` copies the raw payload and duplicates nothing: heap-backed kinds
/// are generational handles (string/BigInt nodes, unbranded atom indices,
/// object slots), so an owned clone enters the heap only through checked
/// methods such as [`Heap::allocate_object`] and
/// [`Heap::replace_object_slot`], which retain their edges transactionally.
///
/// There is deliberately no `PartialEq`: handle equality is *not* content
/// equality for strings and BigInts.  Key comparison goes through
/// `value::collection_key` (id fast path plus content fallback with heap
/// access); identity comparison uses explicit handle equality.
#[derive(Clone, Debug)]
pub enum RawValue {
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    Float(f64),
    BigInt(BigIntId),
    String(StringId),
    Symbol(AtomIdx),
    /// Heap-internal class-private identity. This owns one private-atom
    /// reference exactly like `Symbol`, but it is not an ECMAScript Value and
    /// must never cross `Runtime::root_raw_value` or enter ordinary storage.
    Private(AtomIdx),
    Object(ObjectId),
    Uninitialized,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Raw exception sentinels must be recognized and rejected at value boundaries."
        )
    )]
    Exception,
}

const _: () = assert!(std::mem::size_of::<RawValue>() <= 16);

impl RawValue {
    /// The one producer-owned heap edge carried by a value produced at a
    /// boundary conversion (`Runtime::raw_property_value` and friends):
    /// a freshly allocated string or BigInt node.
    ///
    /// Transactional store paths retain their own edge for the stored copy,
    /// so the caller releases this producer edge once the store has
    /// succeeded (or immediately when the value is never stored).
    #[must_use]
    pub(crate) fn conversion_node_edge(&self) -> Option<RawId> {
        match self {
            Self::String(id) => Some(RawId::String(*id)),
            Self::BigInt(id) => Some(RawId::BigInt(*id)),
            Self::Undefined
            | Self::Null
            | Self::Bool(_)
            | Self::Int(_)
            | Self::Float(_)
            | Self::Symbol(_)
            | Self::Private(_)
            | Self::Object(_)
            | Self::Uninitialized
            | Self::Exception => None,
        }
    }
}

/// Append-only identity of one module record in a Context-owned loaded-module
/// cache. A removed record leaves a tombstone and its identity is never
/// reused, matching the construction-order identity of QuickJS's
/// `JSContext.loaded_modules` list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ModuleId(pub(crate) usize);
