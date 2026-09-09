use super::*;

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

/// Realm-owned `%WeakMap.prototype%` class root.
///
/// Weak collections have no iterator prototype. As in pinned QuickJS, the
/// public constructor remains reachable through the ordinary property graph
/// rather than through an additional Context edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeakMapRealmData {
    pub prototype: ObjectId,
}

/// Realm-owned `%WeakSet.prototype%` class root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeakSetRealmData {
    pub prototype: ObjectId,
}

/// Realm-owned class prototypes installed together by QuickJS's
/// `JS_AddIntrinsicWeakRef` bootstrap step. The public constructors remain
/// reachable through their ordinary prototype/global property graph and are
/// therefore not duplicated as Context roots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeakRefRealmData {
    pub weak_ref_prototype: ObjectId,
    pub finalization_registry_prototype: ObjectId,
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

/// Realm-owned `%AsyncFunction.prototype%` identity.
///
/// Async function objects inherit from this ordinary object, which is itself
/// a direct child of the realm's `%Function.prototype%`. The hidden
/// `AsyncFunction` constructor remains reachable through the reciprocal
/// property graph and therefore needs no independent Context root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AsyncFunctionRealmData {
    pub function_prototype: ObjectId,
}

/// Realm-owned identities required by async-generator functions and objects.
///
/// QuickJS keeps `%AsyncIteratorPrototype%`, `%AsyncGeneratorPrototype%`, and
/// `%AsyncGeneratorFunction.prototype%` as independent context roots. The
/// hidden dynamic constructor remains reachable from the reciprocal graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AsyncGeneratorRealmData {
    pub async_iterator_prototype: ObjectId,
    /// Realm-local `%AsyncFromSyncIteratorPrototype%`, inheriting from this
    /// realm's `%AsyncIteratorPrototype%`.
    pub async_from_sync_iterator_prototype: ObjectId,
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

/// Realm-owned identities used by the synchronous Iterator helpers proposal.
///
/// `%IteratorPrototype%` is already a mandatory [`ContextData`] root because
/// concrete built-in iterators depend on it before the public `%Iterator%`
/// constructor is installed. The four identities here are attached later as
/// one transaction, matching QuickJS's `iterator_ctor` and class-prototype
/// roots for `JS_CLASS_ITERATOR_CONCAT`, `JS_CLASS_ITERATOR_HELPER`, and
/// `JS_CLASS_ITERATOR_WRAP`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IteratorRealmData {
    pub constructor: ObjectId,
    pub concat_prototype: ObjectId,
    pub helper_prototype: ObjectId,
    pub wrap_prototype: ObjectId,
}

/// Realm-local `%ArrayBuffer.prototype%` class root.
///
/// The backing bytes live on each branded object, but constructor-realm
/// fallback must retain the original prototype even after authored code
/// replaces or deletes the writable global `ArrayBuffer` binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArrayBufferRealmData {
    pub prototype: ObjectId,
}

/// Realm-local `%SharedArrayBuffer.prototype%` class root.
///
/// Shared wrappers may outlive and share backing stores across runtimes, but
/// their JavaScript prototype identity remains owned by the importing realm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SharedArrayBufferRealmData {
    pub prototype: ObjectId,
}

/// Realm-local `%DataView.prototype%` class root.
///
/// DataView instances retain their backing ArrayBuffer-family object directly. The realm
/// keeps only the original prototype identity used by constructor fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataViewRealmData {
    pub prototype: ObjectId,
}

/// Realm-local concrete TypedArray class prototypes.
///
/// QuickJS roots these twelve identities in `ctx->class_proto`. The hidden
/// abstract prototype stays reachable through their `[[Prototype]]` edges;
/// retaining it separately here would add an arena root that upstream lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypedArrayRealmData {
    pub prototypes: [ObjectId; TypedArrayElementKind::COUNT],
}

/// Realm-owned roots which participate in QuickJS's cycle graph.
///
/// The bootstrap roots needed by ordinary script evaluation are explicit;
/// additional intrinsic and module roots can extend the vectors without
/// changing `ContextId` ownership.
#[derive(Debug, PartialEq)]
pub struct ContextData {
    /// Stable public handle identity assigned by `Runtime::new_context`.
    ///
    /// Low-level heap tests and internal job-only realms deliberately leave
    /// this absent; only a user-created Context may be reconstructed for a
    /// host callback.
    pub(crate) public_id: Option<u64>,
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
    /// Realm-local WeakMap ordinary prototype, attached after its public
    /// constructor/prototype cycle is initialized.
    pub weak_map: Option<WeakMapRealmData>,
    /// Realm-local WeakSet ordinary prototype, attached after its public
    /// constructor/prototype cycle is initialized.
    pub weak_set: Option<WeakSetRealmData>,
    /// Realm-local WeakRef and FinalizationRegistry class prototypes,
    /// attached atomically by the shared weak-reference bootstrap step.
    pub weak_ref: Option<WeakRefRealmData>,
    /// Realm-local `%ArrayBuffer.prototype%` class root, attached after the
    /// public constructor/prototype cycle has been initialized and validated.
    pub array_buffer: Option<ArrayBufferRealmData>,
    /// Realm-local `%SharedArrayBuffer.prototype%` class root, attached after
    /// the public constructor/prototype cycle has been initialized.
    pub shared_array_buffer: Option<SharedArrayBufferRealmData>,
    /// Realm-local `%DataView.prototype%` class root, attached after the
    /// public constructor/prototype cycle has been initialized and validated.
    pub data_view: Option<DataViewRealmData>,
    /// Realm-local concrete TypedArray prototype roots. The hidden abstract
    /// prototype stays reachable through the concrete prototype graph.
    pub typed_array: Option<TypedArrayRealmData>,
    /// Realm-local `%GeneratorPrototype%` and
    /// `%GeneratorFunction.prototype%`, attached after their reciprocal
    /// constructor/prototype graph has been initialized.
    pub generator: Option<GeneratorRealmData>,
    /// Realm-local `%AsyncFunction.prototype%`, attached after its hidden
    /// constructor/prototype graph has been initialized.
    pub async_function: Option<AsyncFunctionRealmData>,
    /// Realm-local async-iterator and async-generator intrinsic graph.
    pub async_generator: Option<AsyncGeneratorRealmData>,
    /// Realm-local `%Promise.prototype%` and `%Promise%`, attached atomically
    /// after their reciprocal public property graph is initialized.
    pub promise: Option<PromiseRealmData>,
    /// Realm-local `%Iterator%` constructor plus the hidden Iterator Concat,
    /// Iterator Helper, and Iterator Wrap class prototypes.
    pub iterator: Option<IteratorRealmData>,
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
    pub(in crate::engine::heap) math_random_state: u64,
    pub global_objects: Vec<ObjectId>,
    pub intrinsics: Vec<RawValue>,
    pub initial_shapes: Vec<ShapeId>,
    /// Context-local `JSModuleDef` ownership, matching QuickJS's
    /// `JSContext.loaded_modules` rather than a Rust-side parallel graph.
    pub(crate) loaded_modules: LoadedModuleCache,
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
    pub fn new(
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
            public_id: None,
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
            weak_map: None,
            weak_set: None,
            weak_ref: None,
            array_buffer: None,
            shared_array_buffer: None,
            data_view: None,
            typed_array: None,
            generator: None,
            async_function: None,
            async_generator: None,
            promise: None,
            iterator: None,
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
            loaded_modules: LoadedModuleCache::new(),
        }
    }

    /// Attach the stable public identity of a user-created Context.
    #[must_use]
    pub(crate) const fn with_public_id(mut self, id: u64) -> Self {
        self.public_id = Some(id);
        self
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
