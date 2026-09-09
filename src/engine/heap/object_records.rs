use super::*;

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
    /// QuickJS `JS_OBJECT_DEF` payload for the realm's global `Atomics` object.
    Atomics {
        realm: ContextId,
    },
    #[cfg(test)]
    FailureProbe {
        realm: ContextId,
    },
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
    #[cfg(test)]
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

/// Hidden state of one genuine `JS_CLASS_PROXY` object.
///
/// QuickJS retains both edges after revocation because either value may still
/// be referenced by an active native call. `is_callable` is fixed at creation
/// time, while the object's constructor bit independently mirrors the target's
/// initial `[[Construct]]` capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProxyData {
    pub target: ObjectId,
    pub handler: ObjectId,
    pub is_callable: bool,
    pub is_revoked: bool,
}

/// Typed hidden payload carried only by runtime-created native functions.
///
/// The shared `already_resolved` cell is intentionally a non-arena leaf: the
/// resolve/reject pair shares one first-call-wins bit without introducing a
/// `Runtime -> heap -> Runtime` ownership cycle.  Every raw object identity in
/// this enum is still an ordinary traced and reference-counted heap edge.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum InternalCallableData {
    /// `Proxy.revocable`'s one-shot revocation closure. The edge is released
    /// after the first call, matching QuickJS's `func_data[0] = JS_NULL`.
    ProxyRevoke {
        proxy: Option<ObjectId>,
    },
    AsyncFunctionResume {
        state: ObjectId,
        kind: AsyncFunctionResumeKind,
    },
    AsyncGeneratorResume {
        generator: ObjectId,
        kind: AsyncGeneratorResumeKind,
    },
    PromiseResolving {
        promise: ObjectId,
        already_resolved: Rc<Cell<bool>>,
        kind: PromiseResolvingKind,
    },
    PromiseCapabilityExecutor(PromiseCapabilityExecutorData),
    PromiseFinallyHandler {
        /// `None` preserves QuickJS's `JS_UNDEFINED` default-constructor
        /// sentinel instead of eagerly materializing the intrinsic Promise.
        constructor: Option<ObjectId>,
        on_finally: ObjectId,
    },
    PromiseFinallyThunk {
        value: RawValue,
    },
    PromiseAllResolveElement {
        values: ObjectId,
        resolve: ObjectId,
        remaining: Rc<Cell<i32>>,
        already_called: Rc<Cell<bool>>,
        index: u32,
    },
    PromiseAllSettledElement {
        values: ObjectId,
        resolve: ObjectId,
        remaining: Rc<Cell<i32>>,
        already_called: Rc<Cell<bool>>,
        index: u32,
        outcome: PromiseReactionKind,
    },
    PromiseAnyRejectElement {
        errors: ObjectId,
        reject: ObjectId,
        remaining: Rc<Cell<i32>>,
        already_called: Rc<Cell<bool>>,
        index: u32,
    },
    AsyncFromSyncIteratorUnwrap {
        done: bool,
    },
    AsyncFromSyncIteratorClose {
        sync_iterator: ObjectId,
    },
    /// Fulfill/reject callback attached to an authored top-level-await body
    /// Promise. The Context edge keeps the loaded-module cache alive; the
    /// append-only ModuleId remains non-owning within that cache.
    ModuleEvaluation {
        module: RawModuleRef,
        kind: ModuleEvaluationKind,
    },
    /// Handler attached to a pending module-evaluation Promise by dynamic
    /// import. It retains both caller-facing settling functions and the
    /// Context-owned module cache until the evaluation settles.
    DynamicImportHandler {
        module: RawModuleRef,
        resolve: ObjectId,
        reject: ObjectId,
        kind: DynamicImportHandlerKind,
    },
}

/// Class-specific edges stored alongside an object's ordinary properties.
// `ObjectPayload` remains a public diagnostic envelope, but native-function
// captures are deliberately crate-authenticated and cannot be forged by an
// embedder. Public callers may still inspect that a payload is native with
// `internal: _` without naming the hidden capture type.
#[allow(private_interfaces)]
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectPayload {
    Ordinary,
    /// Runtime-wide, unforgeable `JS_CLASS_RAWJSON` brand. The exact source
    /// text remains in the object's frozen ordinary `rawJSON` data slot, so
    /// the payload needs no duplicate GC edge or string owner.
    RawJson,
    /// A genuine `JS_CLASS_ARRAY` exotic object. The mandatory `length`
    /// property and every named or slow indexed property remain in the
    /// ordinary shape/slot arrays. QuickJS's contiguous C/W/E prefix lives in
    /// `dense` instead and therefore does not allocate decimal property atoms.
    Array {
        /// QuickJS `u.array.values[0..count]`. `Some` is the fast form and
        /// `None` records its irreversible conversion to ordinary properties.
        /// A fast Array has no holes inside this vector; its logical `length`
        /// slot may nevertheless be greater than `dense.len()`.
        dense: Option<Vec<RawValue>>,
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
        /// Ordered stable indices of live records. Historical tombstones stay
        /// in `records` for iterators, but bounded diagnostic traversal does
        /// not need to rescan them.
        live_indices: BTreeSet<usize>,
        size: usize,
    },
    /// `JS_CLASS_MAP_ITERATOR`: the source Map remains an owned edge until
    /// exhaustion, while `next_index` walks stable record indices and skips
    /// tombstones in the runtime layer.
    MapIterator {
        object: Option<ObjectId>,
        next_index: usize,
        /// Stable record returned by the most recent successful `next()`.
        /// QuickJS retains that record until the iterator advances again, so
        /// deleting it can leave a printer-visible zombie in the source Map.
        current_index: Option<usize>,
        kind: MapIteratorKind,
    },
    /// `JS_CLASS_SET`: the ordered record layout is shared with Map, but each
    /// live record stores its element in `key` and keeps `value` exactly
    /// `undefined`. A distinct payload preserves the unforgeable Set brand.
    Set {
        records: Vec<MapRecord>,
        /// Ordered stable indices of live elements; see the Map counterpart.
        live_indices: BTreeSet<usize>,
        size: usize,
    },
    /// `JS_CLASS_SET_ITERATOR`: the source Set remains an owned edge until
    /// exhaustion. `kind` distinguishes value iteration from entry-pair
    /// iteration while both `keys` and `values` use the value projection.
    SetIterator {
        object: Option<ObjectId>,
        next_index: usize,
        /// Stable record returned by the most recent successful `next()`.
        /// This mirrors `JSMapIteratorData.cur_record` for Set iterators.
        current_index: Option<usize>,
        kind: SetIteratorKind,
    },
    /// `JS_CLASS_WEAKMAP`: generation-checked weak keys, strongly owned
    /// values, and a hash-indexed intrusive insertion-order list matching
    /// QuickJS's internal weak-record traversal.
    WeakMap {
        records: WeakCollectionRecords<RawValue>,
    },
    /// `JS_CLASS_WEAKSET`: ordered non-owning identities with no tombstones.
    WeakSet {
        records: WeakCollectionRecords<()>,
    },
    /// `JS_CLASS_WEAK_REF`: one non-owning generational target identity. A
    /// dead target is cleared by the ordered weak-object pass, never traced as
    /// an arena edge.
    WeakRef {
        target: Option<WeakCollectionKey>,
    },
    /// `JS_CLASS_FINALIZATION_REGISTRY`: weak registration target/token
    /// identities plus strong callback, creation-realm, and held-value edges.
    /// Entries remain ordered until unregister or finalization preparation.
    FinalizationRegistry(FinalizationRegistryData),
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
    /// `JS_CLASS_ITERATOR_HELPER`: lazy helper state with independently owned
    /// source, cached-next, callback, and optional inner-iterator edges.
    /// Completion changes only `done`; QuickJS retains all four values until
    /// the helper object is finalized.
    IteratorHelper(IteratorHelperData),
    /// `JS_CLASS_ITERATOR_WRAP`: source iterator and cached `next` method
    /// retained by the wrapper returned from `Iterator.from`.
    IteratorWrap(IteratorWrapData),
    /// `JS_CLASS_ASYNC_FROM_SYNC_ITERATOR`: synchronous source iterator and
    /// cached `next`, exposed only through Promise-returning adapter methods.
    AsyncFromSyncIterator(AsyncFromSyncIteratorData),
    /// `JS_CLASS_ITERATOR_CONCAT`: remaining iterable/method pairs plus the
    /// lazily created current iterator and cached `next` method.
    IteratorConcat(IteratorConcatData),
    /// QuickJS `JS_CLASS_PROXY`. A Proxy has no ordinary prototype of its own;
    /// every observable internal method dispatches through this payload.
    Proxy(ProxyData),
    /// `JS_CLASS_ARRAY_BUFFER`. Backing bytes are non-GC memory, so this
    /// payload introduces no arena edge. TypedArray/DataView objects retain
    /// the owning ArrayBuffer object in their own payloads.
    ArrayBuffer(ArrayBufferData),
    /// `JS_CLASS_SHARED_ARRAY_BUFFER`. The wrapper-local handle is a GC leaf;
    /// its `Arc` backing may outlive this heap object or be sent to a worker.
    SharedArrayBuffer(SharedArrayBufferData),
    /// `JS_CLASS_DATAVIEW`. The backing ArrayBuffer-family object is a strong arena edge;
    /// detached and currently out-of-bounds views retain this structural
    /// payload and become observable errors only when accessed.
    DataView(ArrayBufferViewData),
    /// One of QuickJS's twelve fast integer-indexed TypedArray classes.
    ///
    /// Element bytes remain owned by the branded ArrayBuffer-family object. The durable view
    /// metadata survives detach and resizable-buffer OOB transitions so a view
    /// can recover when its backing store grows into range again.
    TypedArray(TypedArrayData),
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
    /// `JS_CLASS_ASYNC_GENERATOR`: a dormant resumable frame plus the FIFO
    /// request queue whose Promise capabilities serialize public resumes.
    AsyncGenerator(AsyncGeneratorData),
    /// Hidden GC-visible async-function driver shared by its pending `await`
    /// reactions. It is never exposed to authored ECMAScript code.
    AsyncFunctionState(AsyncFunctionStateData),
    /// `JS_CLASS_PROMISE`: settlement state, result, and pending reactions are
    /// traced directly in the arena rather than hidden in a runtime side map.
    Promise(PromiseData),
}

/// Object storage category.  Additional QuickJS classes will extend this enum
/// while retaining the same arena and collection protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Ordinary,
    /// `JS_CLASS_MODULE_NS`: null-prototype, non-extensible live export view.
    ModuleNamespace,
    /// `JS_CLASS_ITERATOR`: ordinary internal methods with a distinct class
    /// tag for direct subclasses of the abstract Iterator constructor.
    Iterator,
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
    WeakMap,
    WeakSet,
    WeakRef,
    FinalizationRegistry,
    GlobalObject,
    Error,
    StringIterator,
    IteratorHelper,
    IteratorWrap,
    AsyncFromSyncIterator,
    IteratorConcat,
    Proxy,
    ArrayBuffer,
    SharedArrayBuffer,
    DataView,
    TypedArray,
    NativeFunction,
    BoundFunction,
    BytecodeFunction,
    Generator,
    AsyncGenerator,
    AsyncFunctionState,
    Promise,
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
    /// QuickJS's identity-local Annex B `is_HTMLDDA` bit. This is object
    /// metadata rather than a callable kind: `JS_SetIsHTMLDDA` can mark any
    /// object, although the pinned Test262 host marks one native function.
    pub is_html_dda: bool,
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
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Ordinary,
            payload: ObjectPayload::Ordinary,
        }
    }

    /// Construct one `JS_CLASS_ITERATOR` object. It deliberately shares the
    /// ordinary payload and internal methods; only the QuickJS class tag is
    /// distinct.
    #[must_use]
    pub const fn iterator(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Iterator,
            payload: ObjectPayload::Ordinary,
        }
    }

    /// Construct one `JS_CLASS_MODULE_NS` exotic object.
    ///
    /// Export bindings remain ordinary `PropertySlot::VarRef` edges, while
    /// the distinct class marker selects the namespace-only internal methods.
    /// The caller supplies a null-prototype shape and installs the complete
    /// sorted export table through the runtime's private construction path.
    #[must_use]
    pub const fn module_namespace(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: false,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::ModuleNamespace,
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
            is_html_dda: false,
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
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Array,
            payload: ObjectPayload::Array {
                dense: Some(Vec::new()),
            },
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
            is_html_dda: false,
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
            is_html_dda: false,
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
            is_html_dda: false,
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
            is_html_dda: false,
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
            is_html_dda: false,
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
            is_html_dda: false,
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
            is_html_dda: false,
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
            is_html_dda: false,
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
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Map,
            payload: ObjectPayload::Map {
                records: Vec::new(),
                live_indices: BTreeSet::new(),
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
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::MapIterator,
            payload: ObjectPayload::MapIterator {
                object: Some(object),
                next_index: 0,
                current_index: None,
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
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::Set,
            payload: ObjectPayload::Set {
                records: Vec::new(),
                live_indices: BTreeSet::new(),
                size: 0,
            },
        }
    }

    /// Construct one empty genuine WeakMap. Record keys are weak identities;
    /// values inserted later through [`Heap::weak_map_set`] retain
    /// their ordinary object and Symbol ownership.
    #[must_use]
    pub fn weak_map(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::WeakMap,
            payload: ObjectPayload::WeakMap {
                records: WeakCollectionRecords::new(),
            },
        }
    }

    /// Construct one empty genuine WeakSet.
    #[must_use]
    pub fn weak_set(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::WeakSet,
            payload: ObjectPayload::WeakSet {
                records: WeakCollectionRecords::new(),
            },
        }
    }

    /// Construct one heap-internal genuine WeakRef. The runtime intrinsic
    /// layer supplies the public constructor and selected prototype.
    #[must_use]
    pub(crate) const fn weak_ref(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        target: WeakCollectionKey,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::WeakRef,
            payload: ObjectPayload::WeakRef {
                target: Some(target),
            },
        }
    }

    /// Construct one heap-internal genuine FinalizationRegistry. Its callback
    /// and creation realm are ordinary traced payload edges.
    #[must_use]
    pub(crate) const fn finalization_registry(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        callback: ObjectId,
        realm: ContextId,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::FinalizationRegistry,
            payload: ObjectPayload::FinalizationRegistry(FinalizationRegistryData {
                callback,
                realm,
                entries: Vec::new(),
            }),
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
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::SetIterator,
            payload: ObjectPayload::SetIterator {
                object: Some(object),
                next_index: 0,
                current_index: None,
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
            is_html_dda: false,
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
            is_html_dda: false,
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
            is_html_dda: false,
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

    /// Construct one lazy synchronous Iterator Helper.
    ///
    /// Runtime creation passes `inner: None` (the internal `undefined` state);
    /// `flatMap` later replaces it while traversing a mapped iterator. All
    /// supplied edges transfer to the object when allocation succeeds.
    #[must_use]
    pub const fn iterator_helper(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        data: IteratorHelperData,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::IteratorHelper,
            payload: ObjectPayload::IteratorHelper(data),
        }
    }

    /// Construct the branded forwarding iterator used by `Iterator.from`.
    #[must_use]
    pub const fn iterator_wrap(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        source: RawValue,
        next: RawValue,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::IteratorWrap,
            payload: ObjectPayload::IteratorWrap(IteratorWrapData { source, next }),
        }
    }

    /// Construct the branded Promise adapter used by async iteration over a
    /// synchronous iterator.
    #[must_use]
    pub const fn async_from_sync_iterator(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        sync_iterator: ObjectId,
        next: RawValue,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::AsyncFromSyncIterator,
            payload: ObjectPayload::AsyncFromSyncIterator(AsyncFromSyncIteratorData {
                sync_iterator,
                next,
            }),
        }
    }

    /// Construct the lazy sequencing iterator returned by `Iterator.concat`.
    #[must_use]
    pub const fn iterator_concat(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        items: Vec<Option<IteratorConcatItem>>,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::IteratorConcat,
            payload: ObjectPayload::IteratorConcat(IteratorConcatData {
                items,
                index: 0,
                iterator: None,
                next: RawValue::Undefined,
                running: false,
            }),
        }
    }

    /// Construct one genuine Proxy with a null ordinary prototype.
    ///
    /// `is_constructor` is copied from the target at creation time, just as
    /// QuickJS sets the Proxy object's constructor bit independently from its
    /// callable class hook.
    #[must_use]
    pub const fn proxy(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        target: ObjectId,
        handler: ObjectId,
        is_callable: bool,
        is_constructor: bool,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor,
            kind: ObjectKind::Proxy,
            payload: ObjectPayload::Proxy(ProxyData {
                target,
                handler,
                is_callable,
                is_revoked: false,
            }),
        }
    }

    /// Construct one attached ArrayBuffer by transferring an existing byte
    /// vector. The runtime validates the vector length and maximum before
    /// entering this allocation boundary.
    #[must_use]
    pub fn array_buffer_from_bytes(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        bytes: Vec<u8>,
        max_byte_length: Option<u32>,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::ArrayBuffer,
            payload: ObjectPayload::ArrayBuffer(ArrayBufferData {
                bytes,
                max_byte_length,
                detached: false,
            }),
        }
    }

    /// Construct one genuine SharedArrayBuffer wrapper around a safe shared
    /// backing handle. Cloned handles share bytes while retaining independent
    /// wrapper-local length metadata.
    #[must_use]
    pub fn shared_array_buffer(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        handle: SharedBufferHandle,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::SharedArrayBuffer,
            payload: ObjectPayload::SharedArrayBuffer(SharedArrayBufferData { handle }),
        }
    }

    /// Construct one genuine DataView over an ArrayBuffer.
    ///
    /// The heap validates only the durable structural layout here. Detached
    /// and currently out-of-bounds states remain valid so later resize/detach
    /// operations never corrupt the object graph.
    #[must_use]
    pub const fn data_view(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        data: ArrayBufferViewData,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::DataView,
            payload: ObjectPayload::DataView(data),
        }
    }

    /// Construct one genuine integer-indexed TypedArray over an ArrayBuffer.
    #[must_use]
    pub const fn typed_array(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        data: TypedArrayData,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::TypedArray,
            payload: ObjectPayload::TypedArray(data),
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
            is_html_dda: false,
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
            is_html_dda: false,
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
            is_html_dda: false,
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
            is_html_dda: false,
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
    #[cfg(test)]
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
            is_html_dda: false,
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
            is_html_dda: false,
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
            is_html_dda: false,
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

    /// Construct a branded async generator in its initial suspended state.
    #[must_use]
    pub fn async_generator(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        activation: GeneratorActivationData,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::AsyncGenerator,
            payload: ObjectPayload::AsyncGenerator(AsyncGeneratorData {
                state: AsyncGeneratorState::SuspendedStart,
                activation: Some(Box::new(activation)),
                queue: VecDeque::new(),
                resume_realm: None,
            }),
        }
    }

    /// Construct one hidden async-function driver in its initial executing
    /// phase. The runtime roots the active frame until the first suspension;
    /// the state object owns the outer resolving functions immediately.
    #[must_use]
    pub const fn async_function_state(
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        driver_realm: ContextId,
        outer_resolve: ObjectId,
        outer_reject: ObjectId,
    ) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
            extensible: true,
            immutable_prototype: false,
            is_constructor: false,
            kind: ObjectKind::AsyncFunctionState,
            payload: ObjectPayload::AsyncFunctionState(AsyncFunctionStateData {
                driver_realm,
                outer_resolve,
                outer_reject,
                activation: None,
                phase: AsyncFunctionPhase::Executing,
            }),
        }
    }

    /// Construct one genuine pending Promise.  Its result slot starts at
    /// `undefined` and owns no reactions until `PerformPromiseThen` appends
    /// them through `Heap::promise_add_reactions`.
    #[must_use]
    pub const fn promise(shape: ShapeId, slots: Vec<PropertySlot>) -> Self {
        Self {
            shape,
            slots,
            private_brand_home: None,
            is_html_dda: false,
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
