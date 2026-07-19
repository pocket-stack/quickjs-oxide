//! Runtime and context ownership boundaries.
//!
//! As in QuickJS, a runtime owns resources shared by multiple contexts, while
//! each context is a separate realm and execution surface. The heap and
//! intrinsics extend this boundary; they are not hidden in the compiler or VM.

mod arguments;
mod bytecode_publish;
mod for_in;
mod intrinsics;
mod native_dispatch;
mod native_stack;
mod object_literal;
mod properties;
mod vm_host;

use self::intrinsics::date::{DateHost, SystemDateHost};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::error::Error as StdError;
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::atom::{Atom, AtomError, AtomKind, AtomSpelling, AtomTable, PropertyKeyKind};
use crate::compiler::{
    CompileOptions, DEFAULT_EVAL_FILENAME, compile_unlinked_script_with_filename,
};
use crate::debug::{DebugInfoMode, LineColumn, QuickJsSourceLocator};
use crate::error::{Error, ErrorKind, NativeErrorKind, NativeErrorMessage};
use crate::function::{
    FunctionBytecodeRef, UnlinkedConstant, UnlinkedFunction, UnlinkedFunctionDebug,
    UnlinkedVariableDefinition,
};
use crate::heap::{
    ArrayFindKind, ArrayFlattenKind, ArrayIterationKind, ArrayIteratorKind, ArrayJoinKind,
    ArrayPopKind, ArrayPushKind, ArrayReduceKind, ArraySearchKind, ArraySliceKind,
    AutoInitProperty, BigIntAsNKind, BytecodeConstant, ClosureSource, ClosureVariable,
    ClosureVariableKind, ClosureVariableName, ConstructorKind, ContextData, ContextId,
    DateGetFieldKind, DateNativeKind, DateSetFieldKind, DateStringMethod, DynamicFunctionKind,
    ErrorConstructorKind, EvalEnvironment, ForInCandidate, ForInIteratorData, ForInProperty,
    FunctionBytecodeData, FunctionBytecodeId, FunctionDebugInfo, FunctionDebugPosition,
    FunctionKind, FunctionMetadata, GcStats, GlobalNumberPredicateKind, GlobalUriCodecKind, Heap,
    HeapCleanup, HeapCounts, HeapError, MathBinaryKind, MathMinMaxKind, MathUnaryKind,
    NativeCProto, NativeFunctionId, NumberFormatKind, NumberParseKind, NumberPredicateKind,
    ObjectAccessorKind, ObjectData, ObjectExtensibilityKind, ObjectId, ObjectIntegrityKind,
    ObjectKeysKind, ObjectKind, ObjectOwnPropertyKeysKind, ObjectPayload, PrimitiveKind,
    PrimitiveObjectData, PropertySlot, RawValue, ReflectKind, RegExpNativeKind, ShapeId,
    StringCaseKind, StringCharAtKind, StringCreateHtmlKind, StringIncludesKind, StringIndexOfKind,
    StringPadKind, StringReplaceKind, StringStaticKind, StringSubrangeKind, StringTrimKind,
    StringWellFormedKind, SymbolRegistryKind, VarRefData, VarRefId, VariableDefinition,
};
use crate::object::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, DescriptorField, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, SymbolRef, WellKnownSymbol,
};
use crate::property::{
    CompletePropertyDescriptor, PropertyDefinitionError, PropertyDescriptor,
    validate_and_apply_property_descriptor,
};
use crate::shape::{PropertyFlags, Shape, ShapeEntry, ShapeError};
use crate::value::{CreateHtmlStringBuffer, JsString, JsStringBuilder, JsStringError, Value};
use crate::vm::{
    BytecodePc, Completion, ForInNextOutcome, ForInStartOutcome, ForOfNextOutcome,
    ForOfStartOutcome, IteratorCloseOutcome, ToPrimitiveHint,
};

static NEXT_RUNTIME_DOMAIN_ID: AtomicU64 = AtomicU64::new(1);

struct RuntimeInner {
    state: RefCell<RuntimeState>,
    deferred_references: RefCell<VecDeque<DeferredRefOp>>,
    date_host: Rc<dyn DateHost>,
    next_context_id: Cell<u64>,
    domain_id: u64,
}

#[derive(Clone, Copy, Debug)]
enum DeferredRefOp {
    Object(ObjectId),
    Context(ContextId),
    FunctionBytecode(FunctionBytecodeId),
    VarRef(VarRefId),
    Atom(Atom),
    ActiveFramePop {
        token: ActiveFrameToken,
        depth: usize,
    },
    BacktraceBarrierRestore {
        token: ActiveFrameToken,
        previous: bool,
    },
}

struct RuntimeOperation<'a>(&'a Runtime);

impl Drop for RuntimeOperation<'_> {
    fn drop(&mut self) {
        let result = self.0.drain_deferred_references();
        debug_assert!(result.is_ok(), "deferred root release failed: {result:?}");
    }
}

struct RuntimeState {
    atoms: AtomTable,
    heap: Heap,
    /// Runtime-owned pending JavaScript exception. Object and Symbol payloads
    /// carry one manually retained root; no public `Value::Exception` sentinel
    /// exists.
    pending_exception: Option<RawValue>,
    /// Observable function-debug portion of QuickJS `JS_SetStripInfo`, sampled
    /// by each subsequent compilation.
    debug_info_mode: DebugInfoMode,
    /// QuickJS's shape hash is non-owning. These generational IDs are likewise
    /// weak and are validated before reuse.
    shape_cache: HashMap<ShapeFingerprint, ShapeId>,
    shape_fingerprints: HashMap<ShapeId, ShapeFingerprint>,
    well_known_symbols: HashMap<WellKnownSymbol, Atom>,
    /// Unified QuickJS-style execution-frame chain. Records contain only raw
    /// stable identities and diagnostic state; the corresponding stack-local
    /// [`ActiveFrameGuard`] owns the object and bytecode roots.
    active_frames: Vec<ActiveFrameRecord>,
    next_active_frame_token: u64,
    #[cfg(test)]
    active_frame_probe_snapshots: Vec<Vec<ActiveFrameRecord>>,
    /// Counts the ordinary `{ value, done }` wrappers produced by the native
    /// iterator-next call adapter.  The direct VM fast path deliberately does
    /// not increment this counter.
    #[cfg(test)]
    iterator_result_allocations: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActiveFrameRecord {
    token: ActiveFrameToken,
    function: ObjectId,
    realm: ContextId,
    flags: ActiveFrameFlags,
    kind: ActiveFrameKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActiveFrameToken(u64);

/// Flags which belong to a QuickJS stack frame rather than to the callable
/// heap object. Async execution and backtrace barriers are reserved here for
/// the later stack/debug-info slice even though this first infrastructure step
/// only populates `strict`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ActiveFrameFlags {
    strict: bool,
    is_async: bool,
    backtrace_barrier: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActiveFrameKind {
    Bytecode {
        bytecode: FunctionBytecodeId,
        pc: Option<BytecodePc>,
    },
    Native {
        target: NativeFunctionId,
        actual_arg_count: usize,
        readable_arg_count: usize,
    },
}

#[derive(Clone, Debug)]
struct ExplicitBacktraceLocation {
    filename: JsString,
    position: LineColumn,
}

enum RawStringProperty {
    Missing,
    String(JsString),
    Other,
}

enum NativeInvocation {
    Call { this_value: Value },
    Construct { new_target: Value },
    Getter { this_value: Value },
    Setter { this_value: Value },
}

enum NativeInvocationAdaptation {
    Invoke(NativeInvocation),
    Complete(Completion),
}

/// Result of invoking one native function before the public call adapter has
/// necessarily materialized its JavaScript result shape.
///
/// QuickJS's `JS_CFUNC_iterator_next` ABI returns the iterated value and a
/// side-channel `done` bit.  An ordinary JavaScript call wraps that pair in an
/// iterator-result object, while `JS_IteratorNext2` consumes it directly.  A
/// normal completion remains available for future iterator-next natives which
/// return an already-materialized result object (`pdone == 2`).
enum NativeInvokeOutcome {
    Completion(Completion),
    IteratorNextRaw { value: Value, done: bool },
}

#[derive(Clone, Copy)]
enum NativeInvokeMode {
    Ordinary,
    IteratorNextRaw,
}

struct NativeArguments {
    actual_arg_count: usize,
    readable: Vec<Value>,
}

enum NativeConversion<T> {
    Value(T),
    Throw(Value),
}

enum Compilation {
    Published(FunctionBytecodeRef),
    Throw(Value),
}

/// Controls the property attributes used while instantiating declarations on
/// a realm's global object. Ordinary Script declarations are permanent;
/// QuickJS keeps properties first created or configurably replaced by direct
/// or indirect eval configurable so a later `delete` can remove them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GlobalBindingCreationMode {
    Script,
    Eval,
}

impl GlobalBindingCreationMode {
    const fn configurable(self) -> bool {
        matches!(self, Self::Eval)
    }
}

struct DynamicSourceBuilder {
    source: String,
    utf16_len: usize,
    limit: usize,
    failed: bool,
}

impl DynamicSourceBuilder {
    fn new() -> Self {
        Self::with_limit(JsString::MAX_LEN)
    }

    fn with_limit(limit: usize) -> Self {
        let limit = limit.min(JsString::MAX_LEN);
        Self {
            source: String::with_capacity(64.min(limit)),
            utf16_len: 0,
            limit,
            failed: false,
        }
    }

    fn push_str(&mut self, value: &str) -> Result<(), JsStringError> {
        if self.failed {
            return Err(JsStringError::TooLong);
        }
        let additional = value.encode_utf16().count();
        let next_len =
            match JsString::checked_length_with_limit(self.utf16_len, additional, self.limit) {
                Ok(next_len) => next_len,
                Err(error) => {
                    self.source = String::new();
                    self.utf16_len = 0;
                    self.failed = true;
                    return Err(error);
                }
            };
        self.source.push_str(value);
        self.utf16_len = next_len;
        Ok(())
    }

    fn finish(self) -> Result<String, JsStringError> {
        if self.failed {
            return Err(JsStringError::TooLong);
        }
        Ok(self.source)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ShapeFingerprint {
    prototype: Option<ObjectId>,
    entries: Box<[ShapeEntry]>,
}

enum PropertySnapshot {
    Data {
        value: RawValue,
        flags: PropertyFlags,
    },
    VarRef {
        var_ref: VarRefId,
        flags: PropertyFlags,
    },
    Accessor {
        get: Option<ObjectId>,
        set: Option<ObjectId>,
        flags: PropertyFlags,
    },
    AutoInit,
}

enum PropertyGetAction {
    Complete(Value),
    Call {
        getter: CallableRef,
        receiver: Value,
    },
}

enum PropertySetAction {
    Complete,
    Rejected(PropertySetRejection),
    Throw(Value),
    Call {
        setter: CallableRef,
        receiver: Value,
        argument: Value,
    },
}

enum PropertyDefineOutcome {
    Defined(bool),
    Throw(Value),
}

enum ArrayLengthConversion {
    Length(u32),
    Throw(Value),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArrayOwnKey {
    Length,
    Index(u32),
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PropertySetRejection {
    ReadOnly,
    ArrayLengthReadOnly,
    NotConfigurable,
    NoSetter,
    NotExtensible,
    NotObject,
}

/// Immutable VM inputs detached from the runtime `RefCell` borrow.
///
/// `constants` contains raw heap identities, so the owning bytecode root is
/// part of the snapshot. The raw constant pool therefore cannot outlive the GC
/// node whose edges keep those identities valid.
struct PublishedFunctionSnapshot {
    root: FunctionBytecodeRef,
    code: Rc<[crate::bytecode::Instruction]>,
    constants: Rc<[BytecodeConstant]>,
    argument_definitions: Rc<[VariableDefinition]>,
    local_definitions: Rc<[VariableDefinition]>,
    closure_variables: Rc<[ClosureVariable]>,
    eval_environments: Rc<[EvalEnvironment<Atom>]>,
    metadata: FunctionMetadata,
    realm: ContextId,
}

enum CallableExecution {
    Bytecode {
        bytecode: FunctionBytecodeRef,
        closure_slots: Vec<VarRefRoot>,
    },
    Native {
        target: NativeFunctionId,
        realm: ContextId,
        min_readable_args: u8,
    },
    Bound {
        target: CallableRef,
        this_value: Value,
        arguments: Vec<Value>,
    },
}

struct VarRefRoot {
    runtime: Runtime,
    id: VarRefId,
}

impl VarRefRoot {
    fn from_owned_handle(runtime: Runtime, id: VarRefId) -> Self {
        Self { runtime, id }
    }

    fn from_borrowed_handle(runtime: Runtime, id: VarRefId) -> Result<Self, HeapError> {
        runtime.retain_var_ref_handle(id)?;
        Ok(Self { runtime, id })
    }

    const fn id(&self) -> VarRefId {
        self.id
    }

    fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.runtime.is_same_runtime(runtime)
    }
}

impl Clone for VarRefRoot {
    fn clone(&self) -> Self {
        self.runtime
            .retain_var_ref_handle(self.id)
            .expect("a live VarRef root must retain its cell");
        Self {
            runtime: self.runtime.clone(),
            id: self.id,
        }
    }
}

impl Drop for VarRefRoot {
    fn drop(&mut self) {
        self.runtime.release_var_ref_handle(self.id);
    }
}

enum FlatConstant {
    Value(RawValue),
    AtomString(JsString),
    RegExp {
        pattern: JsString,
        program: Rc<crate::regexp::CompiledRegExp>,
    },
    Child(usize),
}

struct FlatFunction {
    code: Vec<crate::bytecode::Instruction>,
    constants: Vec<FlatConstant>,
    metadata: FunctionMetadata,
    func_name: Option<JsString>,
    argument_definitions: Vec<UnlinkedVariableDefinition>,
    local_definitions: Vec<UnlinkedVariableDefinition>,
    closure_variables: Vec<ClosureVariable>,
    eval_environments: Vec<EvalEnvironment<JsString>>,
    debug: Option<UnlinkedFunctionDebug>,
}

struct FlattenFrame {
    code: Vec<crate::bytecode::Instruction>,
    remaining: std::vec::IntoIter<UnlinkedConstant>,
    constants: Vec<FlatConstant>,
    metadata: FunctionMetadata,
    func_name: Option<JsString>,
    argument_definitions: Vec<UnlinkedVariableDefinition>,
    local_definitions: Vec<UnlinkedVariableDefinition>,
    closure_variables: Vec<ClosureVariable>,
    eval_environments: Vec<EvalEnvironment<JsString>>,
    debug: Option<UnlinkedFunctionDebug>,
}

impl FlattenFrame {
    fn new(function: UnlinkedFunction) -> Self {
        let parts = function.into_parts();
        Self {
            code: parts.code,
            constants: Vec::with_capacity(parts.constants.len()),
            remaining: parts.constants.into_iter(),
            metadata: parts.metadata,
            func_name: parts.func_name,
            argument_definitions: parts.argument_definitions,
            local_definitions: parts.local_definitions,
            closure_variables: parts.closure_variables,
            eval_environments: parts.eval_environments,
            debug: parts.debug,
        }
    }
}

/// Checked failures at the public runtime-domain boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    WrongRuntime(&'static str),
    Invariant(&'static str),
    Exception,
    Engine(Error),
    Atom(AtomError),
    Heap(HeapError),
    Shape(ShapeError),
    Property(PropertyDefinitionError),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongRuntime(kind) => write!(formatter, "{kind} belongs to another runtime"),
            Self::Invariant(message) => write!(formatter, "runtime invariant failed: {message}"),
            Self::Exception => formatter.write_str("JavaScript exception"),
            Self::Engine(error) => error.fmt(formatter),
            Self::Atom(error) => error.fmt(formatter),
            Self::Heap(error) => error.fmt(formatter),
            Self::Shape(error) => error.fmt(formatter),
            Self::Property(error) => error.fmt(formatter),
        }
    }
}

impl StdError for RuntimeError {}

impl From<Error> for RuntimeError {
    fn from(error: Error) -> Self {
        Self::Engine(error)
    }
}

impl From<JsStringError> for RuntimeError {
    fn from(error: JsStringError) -> Self {
        Self::Engine(Error::from(error))
    }
}

impl From<AtomError> for RuntimeError {
    fn from(error: AtomError) -> Self {
        Self::Atom(error)
    }
}

impl From<HeapError> for RuntimeError {
    fn from(error: HeapError) -> Self {
        Self::Heap(error)
    }
}

impl From<ShapeError> for RuntimeError {
    fn from(error: ShapeError) -> Self {
        Self::Shape(error)
    }
}

impl From<PropertyDefinitionError> for RuntimeError {
    fn from(error: PropertyDefinitionError) -> Self {
        Self::Property(error)
    }
}

/// A single-threaded QuickJS-compatible runtime.
///
/// Cloning this handle does not clone the runtime; it creates another owner of
/// the same heap/atom domain so multiple contexts can share runtime resources.
#[derive(Clone)]
pub struct Runtime(Rc<RuntimeInner>);

/// Stack-owned root set and LIFO token for one active execution frame.
///
/// Normal execution calls [`Self::finish`] so token/order corruption becomes
/// an engine error. `Drop` is a no-fail fallback for unwinding paths and keeps
/// stale diagnostic frames from escaping their invocation.
struct ActiveFrameGuard {
    runtime: Runtime,
    token: ActiveFrameToken,
    depth: usize,
    active: bool,
    _function_root: ObjectRef,
    _bytecode_root: Option<FunctionBytecodeRef>,
}

struct BacktraceBarrierGuard {
    runtime: Runtime,
    token: Option<ActiveFrameToken>,
    previous: bool,
    active: bool,
}

impl ActiveFrameGuard {
    const fn token(&self) -> ActiveFrameToken {
        self.token
    }

    fn finish(mut self) -> Result<(), RuntimeError> {
        let result = self.runtime.pop_active_frame(self.token, self.depth);
        self.active = false;
        result
    }
}

impl Drop for ActiveFrameGuard {
    fn drop(&mut self) {
        if self.active {
            self.runtime
                .pop_active_frame_fallback(self.token, self.depth);
            self.active = false;
        }
    }
}

impl BacktraceBarrierGuard {
    fn finish(mut self) -> Result<(), RuntimeError> {
        if let Some(token) = self.token {
            self.runtime
                .restore_backtrace_barrier(token, self.previous)?;
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for BacktraceBarrierGuard {
    fn drop(&mut self) {
        if self.active {
            if let Some(token) = self.token {
                self.runtime
                    .restore_backtrace_barrier_fallback(token, self.previous);
            }
            self.active = false;
        }
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_date_host(Rc::new(SystemDateHost::default()))
    }

    fn new_with_date_host(date_host: Rc<dyn DateHost>) -> Self {
        let domain_id = NEXT_RUNTIME_DOMAIN_ID.fetch_add(1, Ordering::Relaxed);
        assert_ne!(domain_id, 0, "runtime domain ID space exhausted");
        let mut atoms = AtomTable::new();
        let mut well_known_symbols = HashMap::new();
        for symbol in WellKnownSymbol::ALL {
            let atom = atoms
                .new_static_symbol(Some(symbol.description()))
                .expect("fixed well-known symbol set fits the atom table");
            well_known_symbols.insert(symbol, atom);
        }
        Self(Rc::new(RuntimeInner {
            state: RefCell::new(RuntimeState {
                atoms,
                heap: Heap::new(),
                pending_exception: None,
                debug_info_mode: DebugInfoMode::Full,
                shape_cache: HashMap::new(),
                shape_fingerprints: HashMap::new(),
                well_known_symbols,
                active_frames: Vec::new(),
                next_active_frame_token: 1,
                #[cfg(test)]
                active_frame_probe_snapshots: Vec::new(),
                #[cfg(test)]
                iterator_result_allocations: 0,
            }),
            deferred_references: RefCell::new(VecDeque::new()),
            date_host,
            next_context_id: Cell::new(0),
            domain_id,
        }))
    }

    /// Set the runtime-wide debug information policy for future compilations.
    /// Existing bytecode is immutable and keeps the mode used when published.
    pub fn set_debug_info_mode(&self, mode: DebugInfoMode) {
        self.0.state.borrow_mut().debug_info_mode = mode;
    }

    /// Return the policy which the next compilation will sample.
    #[must_use]
    pub fn debug_info_mode(&self) -> DebugInfoMode {
        self.0.state.borrow().debug_info_mode
    }

    #[must_use]
    pub fn new_context(&self) -> Context {
        let _operation = self.operation();
        let id = self.0.next_context_id.get();
        self.0.next_context_id.set(
            id.checked_add(1)
                .expect("runtime context identity space exhausted"),
        );
        let object_prototype = self
            .new_object(None)
            .expect("initial Object.prototype allocation must succeed");
        self.0
            .state
            .borrow_mut()
            .heap
            .set_immutable_prototype(object_prototype.object_id())
            .expect("Object.prototype immutable-prototype initialization must succeed");
        let function_prototype = self
            .new_native_function(&object_prototype, NativeFunctionId::FunctionPrototype, 0)
            .expect("initial Function.prototype allocation must succeed");
        self.define_function_data_property(
            &function_prototype,
            "length",
            Value::Int(0),
            false,
            true,
        )
        .expect("Function.prototype.length initialization must succeed");
        self.define_function_data_property(
            &function_prototype,
            "name",
            Value::String(JsString::from_static("")),
            false,
            true,
        )
        .expect("Function.prototype.name initialization must succeed");
        // QuickJS's `%Array.prototype%` is itself a genuine empty Array,
        // rather than an ordinary object wearing the same prototype chain.
        // Publish the class-correct root before the Array constructor and
        // method tables become observable in later milestones.
        let array_prototype = self
            .new_empty_array_with_prototype(&object_prototype)
            .expect("initial Array.prototype allocation must succeed");
        let iterator_prototype = self
            .new_object(Some(&object_prototype))
            .expect("initial Iterator.prototype allocation must succeed");
        let array_iterator_prototype = self
            .new_object(Some(&iterator_prototype))
            .expect("initial ArrayIterator.prototype allocation must succeed");
        let string_iterator_prototype = self
            .new_object(Some(&iterator_prototype))
            .expect("initial StringIterator.prototype allocation must succeed");
        let error_prototype = self
            .new_object(Some(&object_prototype))
            .expect("initial Error.prototype allocation must succeed");
        let mut native_error_prototypes = Vec::with_capacity(NativeErrorKind::COUNT);
        for kind in NativeErrorKind::ALL {
            let prototype = self
                .new_object(Some(&error_prototype))
                .expect("native Error prototype allocation must succeed");
            self.define_bootstrap_string_property(&prototype, "name", kind.name())
                .expect("native Error prototype name initialization must succeed");
            native_error_prototypes.push(prototype);
        }
        let number_prototype = self
            .new_primitive_object(&object_prototype, PrimitiveKind::Number, Value::Int(0))
            .expect("initial Number.prototype allocation must succeed");
        let boolean_prototype = self
            .new_primitive_object(
                &object_prototype,
                PrimitiveKind::Boolean,
                Value::Bool(false),
            )
            .expect("initial Boolean.prototype allocation must succeed");
        // QuickJS represents String.prototype as a genuine String-class
        // wrapper around the empty string. Its own `length` is the one
        // configurable String-prototype exception; ordinary wrappers use a
        // non-configurable length property.
        let string_prototype = self
            .new_string_object(&object_prototype, JsString::from_static(""), true)
            .expect("initial String.prototype allocation must succeed");
        // Like BigInt.prototype, QuickJS's Symbol.prototype is an ordinary
        // object and deliberately has no [[SymbolData]] payload.
        let symbol_prototype = self
            .new_object(Some(&object_prototype))
            .expect("initial Symbol.prototype allocation must succeed");
        // Unlike Number.prototype and Boolean.prototype, QuickJS creates
        // BigInt.prototype as an ordinary object without [[BigIntData]].
        let bigint_prototype = self
            .new_object(Some(&object_prototype))
            .expect("initial BigInt.prototype allocation must succeed");
        // Pinned QuickJS deliberately gives `%Date.prototype%` no Date
        // payload of its own. Genuine Date instances still use this ordinary
        // realm-local object as their default prototype.
        let date_prototype = self
            .new_object(Some(&object_prototype))
            .expect("initial Date.prototype allocation must succeed");
        let native_error_ids =
            std::array::from_fn(|index| native_error_prototypes[index].object_id());
        let uninitialized_vars = self
            .new_object(None)
            .expect("global unresolved-name table allocation must succeed");
        let global_object = self
            .new_global_object(&object_prototype, &uninitialized_vars)
            .expect("initial global object allocation must succeed");
        let global_var_object = self
            .new_object(None)
            .expect("initial global variable object allocation must succeed");
        self.0
            .state
            .borrow_mut()
            .heap
            .set_immutable_prototype(global_var_object.object_id())
            .expect("global lexical object immutable-prototype initialization must succeed");
        let realm = {
            let mut state = self.0.state.borrow_mut();
            state
                .heap
                .allocate_context(
                    ContextData::new(
                        object_prototype.object_id(),
                        function_prototype.object_id(),
                        array_prototype.object_id(),
                        iterator_prototype.object_id(),
                        array_iterator_prototype.object_id(),
                        string_iterator_prototype.object_id(),
                        global_object.object_id(),
                        global_var_object.object_id(),
                    )
                    .with_primitive_prototype(PrimitiveKind::Number, number_prototype.object_id())
                    .with_primitive_prototype(PrimitiveKind::Boolean, boolean_prototype.object_id())
                    .with_primitive_prototype(PrimitiveKind::String, string_prototype.object_id())
                    .with_primitive_prototype(PrimitiveKind::Symbol, symbol_prototype.object_id())
                    .with_primitive_prototype(PrimitiveKind::BigInt, bigint_prototype.object_id())
                    .with_date_prototype(date_prototype.object_id())
                    .with_error_prototypes(error_prototype.object_id(), native_error_ids),
                )
                .expect("initial realm allocation must succeed")
        };
        // Own the freshly published realm before any fallible intrinsic
        // initialization. If a bootstrap expect panics, Context::drop releases
        // the initial strong reference so the partial graph remains
        // collectable instead of permanently leaking its raw ContextId.
        let context = Context {
            runtime: self.clone(),
            id,
            realm,
        };
        self.0
            .state
            .borrow_mut()
            .heap
            .attach_native_function_realm(function_prototype.object_id(), realm)
            .expect("Function.prototype defining-realm attachment must succeed");
        self.initialize_function_restricted_properties(realm, &function_prototype)
            .expect("Function restricted-property initialization must succeed");
        self.initialize_object_prototype_intrinsics(realm, &object_prototype)
            .expect("Object.prototype intrinsic initialization must succeed");
        self.initialize_function_prototype_methods(realm, &function_prototype)
            .expect("Function.prototype method initialization must succeed");
        self.initialize_iterator_prototype(realm, &iterator_prototype)
            .expect("Iterator.prototype intrinsic initialization must succeed");
        self.initialize_error_intrinsics(
            realm,
            &function_prototype,
            &error_prototype,
            &native_error_prototypes,
            &global_object,
        )
        .expect("Error intrinsic initialization must succeed");
        self.initialize_array_intrinsics(
            realm,
            &function_prototype,
            &array_prototype,
            &array_iterator_prototype,
            &global_object,
        )
        .expect("Array intrinsic initialization must succeed");
        self.initialize_object_intrinsic(
            realm,
            &function_prototype,
            &object_prototype,
            &global_object,
        )
        .expect("Object intrinsic initialization must succeed");
        self.initialize_function_constructor(realm, &function_prototype, &global_object)
            .expect("Function constructor initialization must succeed");
        self.initialize_global_functions_prefix(realm, &function_prototype, &global_object)
            .expect("global function-list prefix initialization must succeed");
        // QuickJS's `js_global_funcs` table places these constants immediately
        // before @@toStringTag; the complete table precedes Number and Boolean.
        // Preserve the implemented entries' relative bootstrap order here.
        self.initialize_global_primitive_constants(&global_object)
            .expect("global primitive constant initialization must succeed");
        self.initialize_global_to_string_tag(&global_object)
            .expect("global toStringTag initialization must succeed");
        self.initialize_eval_intrinsic(realm, &function_prototype, &global_object)
            .expect("eval intrinsic initialization must succeed");
        self.initialize_number_intrinsic(
            realm,
            &function_prototype,
            &number_prototype,
            &global_object,
        )
        .expect("Number intrinsic initialization must succeed");
        self.initialize_boolean_intrinsic(
            realm,
            &function_prototype,
            &boolean_prototype,
            &global_object,
        )
        .expect("Boolean intrinsic initialization must succeed");
        self.initialize_string_prototype_methods(realm, &string_prototype)
            .expect("String prototype method initialization must succeed");
        self.initialize_string_conversion_core(realm, &string_prototype)
            .expect("String conversion-core initialization must succeed");
        self.initialize_string_case_methods(realm, &string_prototype)
            .expect("String case-method initialization must succeed");
        self.initialize_string_iterator_intrinsics(
            realm,
            &string_prototype,
            &string_iterator_prototype,
        )
        .expect("String iterator intrinsic initialization must succeed");
        self.initialize_string_annex_b_html_methods(realm, &string_prototype)
            .expect("String Annex-B HTML method initialization must succeed");
        self.initialize_string_constructor_intrinsic(
            realm,
            &function_prototype,
            &string_prototype,
            &global_object,
        )
        .expect("String constructor intrinsic initialization must succeed");
        self.initialize_math_intrinsic(realm, &global_object)
            .expect("Math intrinsic initialization must succeed");
        self.initialize_reflect_intrinsic(realm, &global_object)
            .expect("Reflect intrinsic initialization must succeed");
        self.initialize_symbol_intrinsic(
            realm,
            &function_prototype,
            &symbol_prototype,
            &global_object,
        )
        .expect("Symbol intrinsic initialization must succeed");
        // Upstream installs `globalThis` after String/Math/Reflect/Symbol and
        // generator setup, then installs BigInt. The remaining intervening
        // intrinsics are absent here, but this boundary preserves the
        // implemented `Boolean, String, Math, Reflect, Symbol, globalThis, BigInt` relative
        // order.
        self.initialize_global_this(&global_object)
            .expect("globalThis initialization must succeed");
        self.initialize_bigint_intrinsic(
            realm,
            &function_prototype,
            &bigint_prototype,
            &global_object,
        )
        .expect("BigInt intrinsic initialization must succeed");
        self.initialize_date_intrinsic(realm, &function_prototype, &date_prototype, &global_object)
            .expect("Date intrinsic initialization must succeed");
        self.initialize_regexp_intrinsic(
            realm,
            &function_prototype,
            &object_prototype,
            &iterator_prototype,
            &global_object,
        )
        .expect("RegExp intrinsic initialization must succeed");
        drop(global_var_object);
        drop(global_object);
        drop(uninitialized_vars);
        drop(date_prototype);
        drop(bigint_prototype);
        drop(symbol_prototype);
        drop(string_iterator_prototype);
        drop(array_iterator_prototype);
        drop(iterator_prototype);
        drop(string_prototype);
        drop(boolean_prototype);
        drop(number_prototype);
        drop(native_error_prototypes);
        drop(error_prototype);
        drop(array_prototype);
        drop(function_prototype);
        drop(object_prototype);
        context
    }

    fn define_bootstrap_string_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: &str,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let defined = self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::try_from_utf8(value)?)),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Invariant(
                "intrinsic bootstrap property definition was rejected",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn is_same_runtime(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// Stable identity used by rooted handle hashing and diagnostics.
    #[must_use]
    pub fn domain_id(&self) -> u64 {
        self.0.domain_id
    }

    /// Intern an exact ECMAScript string as a runtime-owned property key.
    pub fn intern_property_key_js_string(&self, text: &JsString) -> Result<PropertyKey, AtomError> {
        let _operation = self.operation();
        let atom = self
            .0
            .state
            .borrow_mut()
            .atoms
            .intern_property_key_js_string(text)?;
        Ok(PropertyKey::from_owned_atom(self.clone(), atom))
    }

    /// Intern a UTF-8 property spelling without losing the exact UTF-16 path
    /// used by language-level keys.
    pub fn intern_property_key(&self, text: &str) -> Result<PropertyKey, AtomError> {
        self.intern_property_key_js_string(&JsString::try_from_utf8(text)?)
    }

    /// Create a unique ECMAScript Symbol primitive.
    pub fn new_symbol(&self, description: Option<JsString>) -> Result<SymbolRef, AtomError> {
        let _operation = self.operation();
        let atom = self
            .0
            .state
            .borrow_mut()
            .atoms
            .new_symbol_js_string(description)?;
        Ok(SymbolRef::from_owned_atom(self.clone(), atom))
    }

    /// Return the runtime-global symbol for an exact registry key.
    pub fn symbol_for(&self, key: &JsString) -> Result<SymbolRef, AtomError> {
        let _operation = self.operation();
        let atom = self
            .0
            .state
            .borrow_mut()
            .atoms
            .intern_global_symbol_js_string(key)?;
        Ok(SymbolRef::from_owned_atom(self.clone(), atom))
    }

    /// Return one pinned, runtime-unique well-known symbol. It is deliberately
    /// absent from the `Symbol.for` registry.
    pub fn well_known_symbol(&self, symbol: WellKnownSymbol) -> SymbolRef {
        let _operation = self.operation();
        let atom = self.0.state.borrow().well_known_symbols[&symbol];
        SymbolRef::from_owned_atom(self.clone(), atom)
    }

    /// Implement the identity test used by `Symbol.keyFor`.
    pub fn symbol_key_for(&self, symbol: &SymbolRef) -> Result<Option<JsString>, RuntimeError> {
        let _operation = self.operation();
        if !symbol.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("symbol"));
        }
        let state = self.0.state.borrow();
        if state.atoms.kind(symbol.atom())? == AtomKind::GlobalSymbol {
            Ok(Some(state.atoms.to_js_string(symbol.atom())?))
        } else {
            Ok(None)
        }
    }

    /// Return a Symbol's exact optional UTF-16 description.
    ///
    /// `None` is observably distinct from an explicitly empty description via
    /// `%Symbol.prototype%.description`, even though both stringify as
    /// `Symbol()`.
    pub fn symbol_description(&self, symbol: &SymbolRef) -> Result<Option<JsString>, RuntimeError> {
        let _operation = self.operation();
        if !symbol.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("symbol"));
        }
        let state = self.0.state.borrow();
        let info = state.atoms.resolve(symbol.atom())?;
        if !matches!(info.kind, AtomKind::Symbol | AtomKind::GlobalSymbol) {
            return Err(RuntimeError::Invariant(
                "SymbolRef did not refer to a public symbol atom",
            ));
        }
        match info.spelling {
            AtomSpelling::Text(text) => Ok(Some(text.clone())),
            AtomSpelling::NoDescription => Ok(None),
            AtomSpelling::Integer(_) => Err(RuntimeError::Invariant(
                "symbol atom had an immediate-integer spelling",
            )),
        }
    }

    /// Return the exact UTF-16 spelling or symbol description of a key.
    pub fn property_key_to_js_string(&self, key: &PropertyKey) -> Result<JsString, RuntimeError> {
        let _operation = self.operation();
        if !key.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("property key"));
        }
        Ok(self.0.state.borrow().atoms.to_js_string(key.atom())?)
    }

    fn native_atom_error(
        &self,
        kind: ErrorKind,
        prefix: &str,
        key: &PropertyKey,
        suffix: &str,
    ) -> Result<Error, RuntimeError> {
        let _operation = self.operation();
        if !key.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("property key"));
        }
        let mut message = NativeErrorMessage::new();
        message.push_utf8(prefix);
        self.0
            .state
            .borrow()
            .atoms
            .push_atom_get_str(key.atom(), &mut message)?;
        message.push_utf8(suffix);
        Ok(Error::from_native_message(kind, message))
    }

    /// Allocate an ordinary object whose prototype is `prototype` or null.
    pub fn new_object(&self, prototype: Option<&ObjectRef>) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if prototype.is_some_and(|prototype| !prototype.belongs_to(self)) {
            return Err(RuntimeError::WrongRuntime("prototype"));
        }
        let prototype = prototype.map(ObjectRef::object_id);

        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(prototype, &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    /// Allocate one genuine empty Array with an explicit prototype. The
    /// non-configurable `length` data property is installed as physical slot
    /// zero so later ArraySetLength updates never depend on insertion order.
    fn new_empty_array_with_prototype(
        &self,
        prototype: &ObjectRef,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Array prototype"));
        }
        let length = self.intern_property_key("length")?;
        let entries = [ShapeEntry {
            atom: length.atom(),
            flags: PropertyFlags::data(true, false, false),
        }];
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &entries)?;
        let object = match state.heap.allocate_object(ObjectData::array(
            shape,
            vec![PropertySlot::Data(RawValue::Int(0))],
        )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    /// Allocate an empty Array rooted in `realm`'s `%Array.prototype%`.
    pub(crate) fn new_array(&self, realm: ContextId) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.0.state.borrow().heap.context(realm)?.array_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        self.new_empty_array_with_prototype(&prototype)
    }

    /// Allocate a realm-correct Array and create consecutive C/W/E indexed
    /// data properties from `values`. This is the final VM-facing substrate
    /// for QuickJS `OP_array_from` and the dense prefix of Array literals.
    pub(crate) fn new_array_from_values(
        &self,
        realm: ContextId,
        values: Vec<Value>,
    ) -> Result<ObjectRef, RuntimeError> {
        for value in &values {
            self.validate_value_domain(value, "Array element")?;
        }
        let array = self.new_array(realm)?;
        for (index, value) in values.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| {
                RuntimeError::Engine(Error::new(ErrorKind::Range, "invalid array length"))
            })?;
            if index == u32::MAX {
                return Err(RuntimeError::Engine(Error::new(
                    ErrorKind::Range,
                    "invalid array length",
                )));
            }
            let key = self.intern_property_key(&index.to_string())?;
            let defined = self.define_own_property(
                &array,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !defined {
                return Err(RuntimeError::Invariant(
                    "fresh Array element definition was rejected",
                ));
            }
        }
        Ok(array)
    }

    fn new_string_iterator(
        &self,
        realm: ContextId,
        string: JsString,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        let prototype_id = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .string_iterator_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype_id)?;
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object =
            match state
                .heap
                .allocate_object(ObjectData::string_iterator(shape, Vec::new(), string))
            {
                Ok(object) => object,
                Err(error) => {
                    let cleanup = state.heap.release_shape(shape)?;
                    state.apply_cleanup(cleanup)?;
                    return Err(error.into());
                }
            };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    fn new_iterator_result(
        &self,
        realm: ContextId,
        value: Value,
        done: bool,
    ) -> Result<ObjectRef, RuntimeError> {
        #[cfg(test)]
        {
            let mut state = self.0.state.borrow_mut();
            state.iterator_result_allocations = state
                .iterator_result_allocations
                .checked_add(1)
                .expect("iterator-result allocation counter overflow");
        }
        let prototype_id = self.0.state.borrow().heap.context(realm)?.object_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype_id)?;
        let result = self.new_object(Some(&prototype))?;
        for (name, value) in [("value", value), ("done", Value::Bool(done))] {
            let key = self.intern_property_key(name)?;
            if !self.define_own_property(
                &result,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )? {
                return Err(RuntimeError::Invariant(
                    "iterator result property definition was rejected",
                ));
            }
        }
        Ok(result)
    }

    fn new_primitive_object(
        &self,
        prototype: &ObjectRef,
        kind: PrimitiveKind,
        value: Value,
    ) -> Result<ObjectRef, RuntimeError> {
        self.new_primitive_object_with_string_length(prototype, kind, value, false)
    }

    fn new_string_object(
        &self,
        prototype: &ObjectRef,
        value: JsString,
        length_configurable: bool,
    ) -> Result<ObjectRef, RuntimeError> {
        self.new_primitive_object_with_string_length(
            prototype,
            PrimitiveKind::String,
            Value::String(value),
            length_configurable,
        )
    }

    fn new_primitive_object_with_string_length(
        &self,
        prototype: &ObjectRef,
        kind: PrimitiveKind,
        value: Value,
        string_length_configurable: bool,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("primitive prototype"));
        }
        let value = match (kind, value) {
            (PrimitiveKind::String, Value::String(value)) => {
                // QuickJS `JS_ToObject` always linearizes a rope before a
                // JS_CLASS_STRING wrapper owns its object_data payload.
                Value::String(value.linearize())
            }
            (_, value) => value,
        };
        self.validate_value_domain(&value, "primitive wrapper payload")?;
        let string_length = match &value {
            Value::String(value) if kind == PrimitiveKind::String => Some(value.len()),
            _ => None,
        };
        // Match by reference so a unique local Symbol root remains alive
        // until the wrapper has retained its own atom edge.
        let (data, payload_atom) = match (kind, &value) {
            (PrimitiveKind::Number, Value::Int(value)) => {
                (PrimitiveObjectData::Number(f64::from(*value)), None)
            }
            (PrimitiveKind::Number, Value::Float(value)) => {
                (PrimitiveObjectData::Number(*value), None)
            }
            (PrimitiveKind::String, Value::String(value)) => {
                (PrimitiveObjectData::String(value.clone()), None)
            }
            (PrimitiveKind::Boolean, Value::Bool(value)) => {
                (PrimitiveObjectData::Boolean(*value), None)
            }
            (PrimitiveKind::Symbol, Value::Symbol(value)) => {
                let atom = value.atom();
                (PrimitiveObjectData::Symbol(atom), Some(atom))
            }
            (PrimitiveKind::BigInt, Value::BigInt(value)) => {
                (PrimitiveObjectData::BigInt(value.clone()), None)
            }
            _ => {
                return Err(RuntimeError::Invariant(
                    "primitive wrapper class or payload is not implemented yet",
                ));
            }
        };
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        if let Some(atom) = payload_atom
            && let Err(error) = state.atoms.retain(atom)
        {
            let cleanup = state.heap.release_shape(shape)?;
            state.apply_cleanup(cleanup)?;
            return Err(error.into());
        }
        let object =
            match state
                .heap
                .allocate_object(ObjectData::primitive(shape, Vec::new(), data))
            {
                Ok(object) => object,
                Err(error) => {
                    if let Some(atom) = payload_atom {
                        state.atoms.release(atom)?;
                    }
                    let cleanup = state.heap.release_shape(shape)?;
                    state.apply_cleanup(cleanup)?;
                    return Err(error.into());
                }
            };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        let object = ObjectRef::from_owned_handle(self.clone(), object);
        if let Some(length) = string_length {
            let length = i32::try_from(length)
                .map(Value::Int)
                .unwrap_or_else(|_| Value::number(length as f64));
            let key = self.intern_property_key("length")?;
            let defined = self.define_own_property(
                &object,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(length),
                    writable: DescriptorField::Present(false),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(string_length_configurable),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !defined {
                return Err(RuntimeError::Invariant(
                    "String wrapper length definition was rejected",
                ));
            }
        }
        Ok(object)
    }

    fn new_global_object(
        &self,
        prototype: &ObjectRef,
        uninitialized_vars: &ObjectRef,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) || !uninitialized_vars.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("global object edge"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state.heap.allocate_object(ObjectData::global_object(
            shape,
            Vec::new(),
            uninitialized_vars.object_id(),
        )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    fn new_native_function(
        &self,
        prototype: &ObjectRef,
        target: NativeFunctionId,
        min_readable_args: u8,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object =
            match state
                .heap
                .allocate_bootstrap_native_function(ObjectData::native_function(
                    shape,
                    Vec::new(),
                    target,
                    min_readable_args,
                )) {
                Ok(object) => object,
                Err(error) => {
                    let cleanup = state.heap.release_shape(shape)?;
                    state.apply_cleanup(cleanup)?;
                    return Err(error.into());
                }
            };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    /// Allocate a native callable after its defining realm has been
    /// published. `%Function.prototype%` cannot use this path because it is
    /// itself one of the roots needed to publish the realm.
    fn new_bound_native_function(
        &self,
        prototype: &ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
    ) -> Result<CallableRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::bound_native_function(
                shape,
                Vec::new(),
                target,
                realm,
                min_readable_args,
            )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(CallableRef::from_validated_object(
            ObjectRef::from_owned_handle(self.clone(), object),
        ))
    }

    fn new_bound_function(
        &self,
        realm: ContextId,
        target: &CallableRef,
        this_value: &Value,
        arguments: &[Value],
    ) -> Result<CallableRef, RuntimeError> {
        let _operation = self.operation();
        if !target.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("bound function target"));
        }
        self.validate_value_domain(this_value, "bound this value")?;
        for argument in arguments {
            self.validate_value_domain(argument, "bound function argument")?;
        }

        let raw_this = self.raw_property_value(this_value)?;
        let raw_arguments = arguments
            .iter()
            .map(|argument| self.raw_property_value(argument))
            .collect::<Result<Vec<_>, _>>()?;
        let is_constructor = self.is_constructor(target.as_object())?;

        let mut state = self.0.state.borrow_mut();
        let function_prototype = state.heap.context(realm)?.function_prototype;
        let shape = state.get_or_create_shape(Some(function_prototype), &[])?;
        let retained_atoms = match state
            .retain_raw_value_atoms(std::iter::once(&raw_this).chain(raw_arguments.iter()))
        {
            Ok(atoms) => atoms,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error);
            }
        };
        let object = match state.heap.allocate_object(ObjectData::bound_function(
            shape,
            Vec::new(),
            target.as_object().object_id(),
            raw_this,
            raw_arguments.into(),
            is_constructor,
        )) {
            Ok(object) => object,
            Err(error) => {
                state.release_atoms(retained_atoms)?;
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(CallableRef::from_validated_object(
            ObjectRef::from_owned_handle(self.clone(), object),
        ))
    }

    /// Allocate a fully initialized realm-bound native builtin. Internal
    /// readable arity remains in the payload while own `length` is an
    /// independent configurable ordinary property.
    fn new_native_builtin(
        &self,
        prototype: &ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        name: &str,
        length: i32,
    ) -> Result<CallableRef, RuntimeError> {
        let callable =
            self.new_bound_native_function(prototype, realm, target, min_readable_args)?;
        self.define_function_data_property(
            callable.as_object(),
            "length",
            Value::Int(length),
            false,
            true,
        )?;
        self.define_function_data_property(
            callable.as_object(),
            "name",
            Value::String(JsString::try_from_utf8(name)?),
            false,
            true,
        )?;
        Ok(callable)
    }

    fn initialize_error_intrinsics(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        error_prototype: &ObjectRef,
        native_error_prototypes: &[ObjectRef],
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        if native_error_prototypes.len() != NativeErrorKind::COUNT {
            return Err(RuntimeError::Invariant(
                "native Error prototype count did not match NativeErrorKind",
            ));
        }

        // JS_NewCConstructor installs prototype fields before the constructor
        // back-reference. Preserve that observable own-key order.
        self.define_native_builtin_auto_init(
            error_prototype,
            realm,
            NativeFunctionId::ErrorPrototypeToString,
            "toString",
            0,
            0,
        )?;
        self.define_string_auto_init(error_prototype, realm, "name", "Error")?;
        self.define_string_auto_init(error_prototype, realm, "message", "")?;

        for prototype in native_error_prototypes {
            self.define_string_auto_init(prototype, realm, "message", "")?;
        }

        let error_constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ErrorConstructor(ErrorConstructorKind::Error),
            1,
            "Error",
            1,
        )?;
        self.define_native_builtin_auto_init(
            error_constructor.as_object(),
            realm,
            NativeFunctionId::ErrorIsError,
            "isError",
            1,
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "Error",
            Value::Object(error_constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&error_constructor, error_prototype)?;

        for kind in NativeErrorKind::ALL {
            // AggregateError's constructor, `errors` population, and stack
            // behavior remain a separate intrinsic milestone.
            if kind == NativeErrorKind::Aggregate {
                continue;
            }
            let prototype =
                native_error_prototypes
                    .get(kind.index())
                    .ok_or(RuntimeError::Invariant(
                        "native Error prototype index was out of bounds",
                    ))?;
            let constructor = self.new_native_builtin(
                error_constructor.as_object(),
                realm,
                NativeFunctionId::ErrorConstructor(ErrorConstructorKind::Native(kind)),
                1,
                kind.name(),
                1,
            )?;
            self.define_function_data_property(
                global_object,
                kind.name(),
                Value::Object(constructor.as_object().clone()),
                true,
                true,
            )?;
            self.define_constructor_relationship(&constructor, prototype)?;
        }
        Ok(())
    }

    fn initialize_function_constructor(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        // QuickJS publishes Function after its prototype table and the Error
        // family, then closes the constructor/prototype cycle. Its magic
        // selector makes this same handler reusable by the future dynamic
        // GeneratorFunction/AsyncFunction constructors.
        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Normal),
            1,
            "Function",
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "Function",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, function_prototype)?;
        self.0
            .state
            .borrow_mut()
            .heap
            .attach_function_constructor(realm, constructor.as_object().object_id())?;
        Ok(())
    }

    fn initialize_number_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        number_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let kind = PrimitiveKind::Number;
        for (target, name, arity) in [
            (
                NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::ToExponential),
                "toExponential",
                1,
            ),
            (
                NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::ToFixed),
                "toFixed",
                1,
            ),
            (
                NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::ToPrecision),
                "toPrecision",
                1,
            ),
            (
                NativeFunctionId::PrimitivePrototypeToString(kind),
                "toString",
                1,
            ),
            (
                NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::ToLocaleString),
                "toLocaleString",
                0,
            ),
            (
                NativeFunctionId::PrimitivePrototypeValueOf(kind),
                "valueOf",
                0,
            ),
        ] {
            self.define_native_builtin_auto_init(
                number_prototype,
                realm,
                target,
                name,
                arity,
                arity,
            )?;
        }

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::PrimitiveConstructor(kind),
            1,
            "Number",
            1,
        )?;
        // Upstream captures the already-published global parser callables by
        // identity before adding the remaining Number statics.
        for name in ["parseInt", "parseFloat"] {
            let key = self.intern_property_key(name)?;
            let value = match self.get_property_in_realm(realm, global_object, &key)? {
                Completion::Return(value @ Value::Object(_)) => value,
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "global numeric parser was not an object during Number bootstrap",
                    ));
                }
                Completion::Throw(_) => {
                    return Err(RuntimeError::Invariant(
                        "global numeric parser lookup threw during Number bootstrap",
                    ));
                }
            };
            self.define_function_data_property(constructor.as_object(), name, value, true, true)?;
        }
        for (predicate, name) in [
            (NumberPredicateKind::IsNaN, "isNaN"),
            (NumberPredicateKind::IsFinite, "isFinite"),
            (NumberPredicateKind::IsInteger, "isInteger"),
            (NumberPredicateKind::IsSafeInteger, "isSafeInteger"),
        ] {
            self.define_native_builtin_auto_init(
                constructor.as_object(),
                realm,
                NativeFunctionId::NumberPredicate(predicate),
                name,
                1,
                1,
            )?;
        }
        for (name, value) in [
            ("MAX_VALUE", Value::Float(f64::MAX)),
            ("MIN_VALUE", Value::Float(f64::from_bits(1))),
            ("NaN", Value::Float(f64::NAN)),
            ("NEGATIVE_INFINITY", Value::Float(f64::NEG_INFINITY)),
            ("POSITIVE_INFINITY", Value::Float(f64::INFINITY)),
            ("EPSILON", Value::Float(f64::EPSILON)),
            ("MAX_SAFE_INTEGER", Value::Float(9_007_199_254_740_991.0)),
            ("MIN_SAFE_INTEGER", Value::Float(-9_007_199_254_740_991.0)),
        ] {
            self.define_function_data_property(constructor.as_object(), name, value, false, false)?;
        }
        self.define_function_data_property(
            global_object,
            "Number",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, number_prototype)
    }

    fn initialize_boolean_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        boolean_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let kind = PrimitiveKind::Boolean;
        // QuickJS installs the complete Boolean prototype table before the
        // constructor back-reference, which fixes own-key order as
        // `toString,valueOf,constructor`.
        self.define_native_builtin_auto_init(
            boolean_prototype,
            realm,
            NativeFunctionId::PrimitivePrototypeToString(kind),
            "toString",
            0,
            0,
        )?;
        self.define_native_builtin_auto_init(
            boolean_prototype,
            realm,
            NativeFunctionId::PrimitivePrototypeValueOf(kind),
            "valueOf",
            0,
            0,
        )?;
        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::PrimitiveConstructor(kind),
            1,
            "Boolean",
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "Boolean",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, boolean_prototype)
    }

    /// Install the implemented String conversion pair before the constructor
    /// relationship is published later in bootstrap. This is not the prefix of
    /// QuickJS's 53-key table: these two brand methods are also the observable
    /// ordinary-ToPrimitive dependency for every generic String prototype
    /// method. The implemented method slice is already installed first;
    /// earlier missing String entries must continue to enter before this pair,
    /// while case conversion and later entries remain after it in pinned table
    /// order when each fresh context is bootstrapped.
    fn initialize_string_conversion_core(
        &self,
        realm: ContextId,
        string_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::PrimitivePrototypeToString(PrimitiveKind::String),
            "toString",
            0,
            0,
        )?;
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::PrimitivePrototypeValueOf(PrimitiveKind::String),
            "valueOf",
            0,
            0,
        )
    }

    /// Install the intrinsic iterator identity method without exposing the
    /// still-out-of-scope global `Iterator` constructor or Iterator Helpers.
    fn initialize_iterator_prototype(
        &self,
        realm: ContextId,
        iterator_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        self.define_native_builtin_auto_init_with_key(
            iterator_prototype,
            realm,
            &key,
            NativeFunctionId::IteratorPrototypeIterator,
            "[Symbol.iterator]",
            0,
            0,
            PropertyFlags::data(true, false, true),
        )?;

        // QuickJS installs @@toStringTag as a genuine C getter/setter pair,
        // not as the data property used by concrete iterator prototypes. The
        // setter deliberately gives inheriting iterator objects a writable,
        // enumerable and configurable own tag while keeping this intrinsic's
        // inherited value protected.
        let function_prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .function_prototype;
        let function_prototype = ObjectRef::from_borrowed_handle(self.clone(), function_prototype)?;
        let getter = self.new_native_builtin(
            &function_prototype,
            realm,
            NativeFunctionId::IteratorPrototypeToStringTagGetter,
            0,
            "get [Symbol.toStringTag]",
            0,
        )?;
        let setter = self.new_native_builtin(
            &function_prototype,
            realm,
            NativeFunctionId::IteratorPrototypeToStringTagSetter,
            1,
            "set [Symbol.toStringTag]",
            1,
        )?;
        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            iterator_prototype,
            &key,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(getter)),
                set: DescriptorField::Present(AccessorValue::Callable(setter)),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype toStringTag definition was rejected",
            ));
        }
        Ok(())
    }

    /// Complete the String iterator class slice: the generic String method,
    /// branded iterator prototype, native-next ABI and configurable tag.
    fn initialize_string_iterator_intrinsics(
        &self,
        realm: ContextId,
        string_prototype: &ObjectRef,
        string_iterator_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let iterator = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        self.define_native_builtin_auto_init_with_key(
            string_prototype,
            realm,
            &iterator,
            NativeFunctionId::StringPrototypeIterator,
            "[Symbol.iterator]",
            0,
            0,
            PropertyFlags::data(true, false, true),
        )?;
        self.define_native_builtin_auto_init(
            string_iterator_prototype,
            realm,
            NativeFunctionId::StringIteratorNext,
            "next",
            0,
            0,
        )?;

        let tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            string_iterator_prototype,
            &tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static(
                    "String Iterator",
                ))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "String Iterator toStringTag definition was rejected",
            ));
        }
        Ok(())
    }

    fn initialize_symbol_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        symbol_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let kind = PrimitiveKind::Symbol;
        self.define_native_builtin_auto_init(
            symbol_prototype,
            realm,
            NativeFunctionId::PrimitivePrototypeToString(kind),
            "toString",
            0,
            0,
        )?;
        self.define_native_builtin_auto_init(
            symbol_prototype,
            realm,
            NativeFunctionId::PrimitivePrototypeValueOf(kind),
            "valueOf",
            0,
            0,
        )?;

        let to_primitive = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToPrimitive));
        self.define_native_builtin_auto_init_with_key(
            symbol_prototype,
            realm,
            &to_primitive,
            NativeFunctionId::PrimitivePrototypeValueOf(kind),
            "[Symbol.toPrimitive]",
            1,
            1,
            // This is a pinned QuickJS quirk: the table entry is a C function,
            // but a symbol-named method is installed non-writable.
            PropertyFlags::data(false, false, true),
        )?;
        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            symbol_prototype,
            &to_string_tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static("Symbol"))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Symbol.prototype toStringTag definition was rejected",
            ));
        }
        self.define_native_builtin_getter_on(
            symbol_prototype,
            function_prototype,
            realm,
            NativeFunctionId::SymbolPrototypeDescription,
            "description",
            "get description",
        )?;

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::PrimitiveConstructor(kind),
            1,
            "Symbol",
            0,
        )?;
        for (selector, name) in [
            (SymbolRegistryKind::For, "for"),
            (SymbolRegistryKind::KeyFor, "keyFor"),
        ] {
            self.define_native_builtin_auto_init(
                constructor.as_object(),
                realm,
                NativeFunctionId::SymbolRegistry(selector),
                name,
                1,
                1,
            )?;
        }
        for (name, symbol) in [
            ("toPrimitive", WellKnownSymbol::ToPrimitive),
            ("iterator", WellKnownSymbol::Iterator),
            ("match", WellKnownSymbol::Match),
            ("matchAll", WellKnownSymbol::MatchAll),
            ("replace", WellKnownSymbol::Replace),
            ("search", WellKnownSymbol::Search),
            ("split", WellKnownSymbol::Split),
            ("toStringTag", WellKnownSymbol::ToStringTag),
            ("isConcatSpreadable", WellKnownSymbol::IsConcatSpreadable),
            ("hasInstance", WellKnownSymbol::HasInstance),
            ("species", WellKnownSymbol::Species),
            ("unscopables", WellKnownSymbol::Unscopables),
            ("asyncIterator", WellKnownSymbol::AsyncIterator),
        ] {
            let key = self.intern_property_key(name)?;
            if !self.define_own_property(
                constructor.as_object(),
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Symbol(self.well_known_symbol(symbol))),
                    writable: DescriptorField::Present(false),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )? {
                return Err(RuntimeError::Invariant(
                    "Symbol well-known property definition was rejected",
                ));
            }
        }
        self.define_function_data_property(
            global_object,
            "Symbol",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, symbol_prototype)
    }

    fn initialize_bigint_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        bigint_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let kind = PrimitiveKind::BigInt;
        // `js_bigint_proto_funcs` is installed before the constructor
        // back-reference. toString has observable length zero even though its
        // C handler reads one optional, padded radix argument.
        self.define_native_builtin_auto_init(
            bigint_prototype,
            realm,
            NativeFunctionId::PrimitivePrototypeToString(kind),
            "toString",
            0,
            1,
        )?;
        self.define_native_builtin_auto_init(
            bigint_prototype,
            realm,
            NativeFunctionId::PrimitivePrototypeValueOf(kind),
            "valueOf",
            0,
            0,
        )?;
        let tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            bigint_prototype,
            &tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static("BigInt"))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "BigInt.prototype toStringTag definition was rejected",
            ));
        }

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::PrimitiveConstructor(kind),
            1,
            "BigInt",
            1,
        )?;
        for (selector, name) in [
            (BigIntAsNKind::AsUintN, "asUintN"),
            (BigIntAsNKind::AsIntN, "asIntN"),
        ] {
            self.define_native_builtin_auto_init(
                constructor.as_object(),
                realm,
                NativeFunctionId::BigIntAsN(selector),
                name,
                2,
                2,
            )?;
        }
        self.define_function_data_property(
            global_object,
            "BigInt",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, bigint_prototype)
    }

    fn initialize_global_functions_prefix(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        // QuickJS publishes these entries at the head of `js_global_funcs`,
        // before the frozen global constants and `%Number%`. Number's parser
        // statics later capture the first two callable identities.
        for (kind, name, arity) in [
            (NumberParseKind::ParseInt, "parseInt", 2),
            (NumberParseKind::ParseFloat, "parseFloat", 1),
        ] {
            let callable = self.new_native_builtin(
                function_prototype,
                realm,
                NativeFunctionId::GlobalNumberParse(kind),
                arity,
                name,
                i32::from(arity),
            )?;
            self.define_function_data_property(
                global_object,
                name,
                Value::Object(callable.as_object().clone()),
                true,
                true,
            )?;
        }
        for (kind, name) in [
            (GlobalNumberPredicateKind::IsNaN, "isNaN"),
            (GlobalNumberPredicateKind::IsFinite, "isFinite"),
        ] {
            self.define_native_builtin_auto_init(
                global_object,
                realm,
                NativeFunctionId::GlobalNumberPredicate(kind),
                name,
                1,
                1,
            )?;
        }
        for (kind, name) in [
            (GlobalUriCodecKind::DecodeUri, "decodeURI"),
            (GlobalUriCodecKind::DecodeUriComponent, "decodeURIComponent"),
            (GlobalUriCodecKind::EncodeUri, "encodeURI"),
            (GlobalUriCodecKind::EncodeUriComponent, "encodeURIComponent"),
            (GlobalUriCodecKind::Escape, "escape"),
            (GlobalUriCodecKind::Unescape, "unescape"),
        ] {
            self.define_native_builtin_auto_init(
                global_object,
                realm,
                NativeFunctionId::GlobalUriCodec(kind),
                name,
                1,
                1,
            )?;
        }
        Ok(())
    }

    fn initialize_global_primitive_constants(
        &self,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        // QuickJS `js_global_funcs` entries immediately before @@toStringTag:
        // all three are non-writable, non-enumerable and non-configurable.
        for (name, value) in [
            ("Infinity", Value::Float(f64::INFINITY)),
            ("NaN", Value::Float(f64::NAN)),
            ("undefined", Value::Undefined),
        ] {
            self.define_function_data_property(global_object, name, value, false, false)?;
        }
        Ok(())
    }

    fn initialize_global_to_string_tag(
        &self,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        let defined = self.define_own_property(
            global_object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static("global"))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Invariant(
                "global toStringTag definition was rejected",
            ));
        }
        Ok(())
    }

    fn initialize_global_this(&self, global_object: &ObjectRef) -> Result<(), RuntimeError> {
        let key = self.intern_property_key("globalThis")?;
        let defined = self.define_own_property(
            global_object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::Object(global_object.clone())),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Invariant(
                "globalThis definition was rejected",
            ));
        }
        Ok(())
    }

    fn initialize_function_restricted_properties(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        // JS_AddIntrinsicBaseObjects creates one frozen %ThrowTypeError% and
        // installs that same callable as both halves of both legacy poison
        // accessors before publishing the Function prototype method table.
        let throw_type_error = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ThrowTypeError,
            0,
            "",
            0,
        )?;
        for name in ["length", "name"] {
            let key = self.intern_property_key(name)?;
            let accepted = self.define_own_property(
                throw_type_error.as_object(),
                &key,
                &OrdinaryPropertyDescriptor {
                    writable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !accepted {
                return Err(RuntimeError::Invariant(
                    "%ThrowTypeError% own property could not be frozen",
                ));
            }
        }
        self.prevent_extensions(throw_type_error.as_object())?;

        for name in ["caller", "arguments"] {
            let key = self.intern_property_key(name)?;
            let accepted = self.define_own_property(
                function_prototype,
                &key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(
                        throw_type_error.clone(),
                    )),
                    set: DescriptorField::Present(AccessorValue::Callable(
                        throw_type_error.clone(),
                    )),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !accepted {
                return Err(RuntimeError::Invariant(
                    "Function.prototype poison accessor definition was rejected",
                ));
            }
        }

        self.0
            .state
            .borrow_mut()
            .heap
            .attach_throw_type_error(realm, throw_type_error.as_object().object_id())?;
        Ok(())
    }

    fn initialize_function_prototype_methods(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        self.define_native_builtin_auto_init(
            function_prototype,
            realm,
            NativeFunctionId::FunctionPrototypeCall,
            "call",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            function_prototype,
            realm,
            NativeFunctionId::FunctionPrototypeApply,
            "apply",
            2,
            2,
        )?;
        self.define_native_builtin_auto_init(
            function_prototype,
            realm,
            NativeFunctionId::FunctionPrototypeBind,
            "bind",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            function_prototype,
            realm,
            NativeFunctionId::FunctionPrototypeToString,
            "toString",
            0,
            0,
        )?;

        let has_instance = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::HasInstance));
        self.define_native_builtin_auto_init_with_key(
            function_prototype,
            realm,
            &has_instance,
            NativeFunctionId::FunctionPrototypeHasInstance,
            "[Symbol.hasInstance]",
            1,
            1,
            PropertyFlags::data(false, false, false),
        )?;

        // Unlike C functions in JS_SetPropertyFunctionList, QuickJS's
        // CGETSET entries are instantiated eagerly. Keep that distinction so
        // descriptor reads and realm/GC edges match the upstream table.
        for (target, property_name, getter_name) in [
            (
                NativeFunctionId::FunctionPrototypeFileName,
                "fileName",
                "get fileName",
            ),
            (
                NativeFunctionId::FunctionPrototypePosition(FunctionDebugPosition::Line),
                "lineNumber",
                "get lineNumber",
            ),
            (
                NativeFunctionId::FunctionPrototypePosition(FunctionDebugPosition::Column),
                "columnNumber",
                "get columnNumber",
            ),
        ] {
            self.define_native_builtin_getter(
                function_prototype,
                realm,
                target,
                property_name,
                getter_name,
            )?;
        }
        Ok(())
    }

    fn define_native_builtin_getter(
        &self,
        function_prototype: &ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        property_name: &str,
        getter_name: &str,
    ) -> Result<(), RuntimeError> {
        self.define_native_builtin_getter_on(
            function_prototype,
            function_prototype,
            realm,
            target,
            property_name,
            getter_name,
        )
    }

    fn define_native_builtin_getter_on(
        &self,
        object: &ObjectRef,
        function_prototype: &ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        property_name: &str,
        getter_name: &str,
    ) -> Result<(), RuntimeError> {
        let getter =
            self.new_native_builtin(function_prototype, realm, target, 0, getter_name, 0)?;
        let key = self.intern_property_key(property_name)?;
        if !self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(getter)),
                set: DescriptorField::Present(AccessorValue::Undefined),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "native builtin getter definition was rejected",
            ));
        }
        Ok(())
    }

    fn define_constructor_relationship(
        &self,
        constructor: &CallableRef,
        prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        self.define_function_data_property(
            constructor.as_object(),
            "prototype",
            Value::Object(prototype.clone()),
            false,
            false,
        )?;
        self.define_function_data_property(
            prototype,
            "constructor",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )
    }

    fn new_native_error(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        message: &str,
    ) -> Result<Value, RuntimeError> {
        self.new_native_error_from_message(realm, kind, NativeErrorMessage::from_utf8(message))
    }

    fn new_native_error_from_error(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        error: &Error,
    ) -> Result<Value, RuntimeError> {
        let message = error
            .native_message()
            .cloned()
            .unwrap_or_else(|| NativeErrorMessage::from_utf8(error.message()));
        self.new_native_error_from_message(realm, kind, message)
    }

    fn new_native_error_from_message(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        message: NativeErrorMessage,
    ) -> Result<Value, RuntimeError> {
        let value = self.new_native_error_without_backtrace_from_message(realm, kind, message)?;
        let capture_now = self
            .0
            .state
            .borrow()
            .active_frames
            .last()
            .is_none_or(|frame| matches!(frame.kind, ActiveFrameKind::Native { .. }));
        if capture_now {
            self.ensure_error_backtrace(&value, false, None)?;
        }
        Ok(value)
    }

    /// `JS_ThrowError2(..., add_backtrace = FALSE)` construction path used by
    /// parser diagnostics, which prepend their explicit filename location
    /// before adding the active frame chain.
    fn new_native_error_without_backtrace_from_error(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        error: &Error,
    ) -> Result<Value, RuntimeError> {
        let message = error
            .native_message()
            .cloned()
            .unwrap_or_else(|| NativeErrorMessage::from_utf8(error.message()));
        self.new_native_error_without_backtrace_from_message(realm, kind, message)
    }

    fn new_native_error_without_backtrace_from_message(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        message: NativeErrorMessage,
    ) -> Result<Value, RuntimeError> {
        let prototype = {
            let state = self.0.state.borrow();
            state.heap.context(realm)?.native_error_prototypes[kind.index()].ok_or(
                RuntimeError::Invariant("realm has no native Error prototype"),
            )?
        };
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        let object = self.new_error_object(&prototype)?;
        let key = self.intern_property_key("message")?;
        let defined = self.define_own_property(
            &object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(message.to_js_string()?)),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Invariant(
                "native Error message definition was rejected",
            ));
        }
        Ok(Value::Object(object))
    }

    fn new_error_object(&self, prototype: &ObjectRef) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Error prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::error(shape, Vec::new()))
        {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    /// Return whether `object` carries the native Error class tag. Prototype
    /// spoofing alone does not make an object an Error.
    pub fn is_error_object(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        Ok(self.0.state.borrow().heap.object(object.object_id())?.kind == ObjectKind::Error)
    }

    /// Return whether `object` carries the genuine Array exotic class tag.
    /// Prototype spoofing alone never makes an ordinary object an Array.
    pub fn is_array_object(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        Ok(matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(object.object_id())?
                .payload,
            ObjectPayload::Array { .. }
        ))
    }

    /// Return the object's `[[Construct]]` capability bit. Callability and
    /// constructability are intentionally independent, as in QuickJS.
    pub fn is_constructor(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .object(object.object_id())?
            .is_constructor)
    }

    /// Set the object's `[[Construct]]` capability independently of its call
    /// protocol, matching QuickJS `JS_SetConstructorBit`.
    pub fn set_constructor_bit(
        &self,
        object: &ObjectRef,
        enabled: bool,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        self.0
            .state
            .borrow_mut()
            .heap
            .set_object_constructor_bit(object.object_id(), enabled)?;
        Ok(())
    }

    /// Promote an ordinary object root to a checked callable capability.
    /// Returns `None` for objects without `[[Call]]`; runtime-domain and stale
    /// handle failures remain explicit errors.
    pub fn as_callable(&self, object: &ObjectRef) -> Result<Option<CallableRef>, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        let callable = matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(object.object_id())?
                .payload,
            ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
        );
        if !callable {
            return Ok(None);
        }
        Ok(Some(CallableRef::from_validated_object(object.clone())))
    }

    /// Instantiate one runtime-owned bytecode node as a callable object in the
    /// caller's realm, matching QuickJS's `js_closure` boundary.
    fn new_bytecode_closure(
        &self,
        caller_realm: ContextId,
        function: &FunctionBytecodeRef,
    ) -> Result<CallableRef, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let descriptors = {
            let state = self.0.state.borrow();
            let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
            bytecode.closure_variables.clone()
        };
        // QuickJS checks every GLOBAL_DECL before creating any binding. A
        // later redeclaration must not leave earlier declarations installed.
        for descriptor in descriptors.iter().copied() {
            if !matches!(
                descriptor.source,
                ClosureSource::GlobalDeclaration | ClosureSource::Global
            ) {
                return Err(RuntimeError::Invariant(
                    "root bytecode closure descriptor did not use a root global source",
                ));
            }
            let ClosureVariableName::Atom(name) = descriptor.name else {
                return Err(RuntimeError::Invariant(
                    "published global closure descriptor has no atom",
                ));
            };
            if descriptor.source == ClosureSource::Global
                && descriptor.kind != ClosureVariableKind::Normal
            {
                return Err(RuntimeError::Invariant(
                    "resolved global has declaration-only binding metadata",
                ));
            }
            if descriptor.source == ClosureSource::GlobalDeclaration {
                let key = PropertyKey::from_borrowed_atom(self.clone(), name)?;
                match descriptor.kind {
                    ClosureVariableKind::Normal if descriptor.is_lexical => {
                        self.check_global_lexical_declaration(caller_realm, &key)?;
                    }
                    ClosureVariableKind::Normal => {
                        self.check_global_var_declaration(caller_realm, &key)?;
                    }
                    ClosureVariableKind::GlobalFunction
                        if !descriptor.is_lexical && !descriptor.is_const =>
                    {
                        self.check_global_function_declaration(caller_realm, &key)?;
                    }
                    ClosureVariableKind::FunctionName
                    | ClosureVariableKind::GlobalFunction
                    | ClosureVariableKind::EvalVariableObject
                    | ClosureVariableKind::WithObject => {
                        return Err(RuntimeError::Invariant(
                            "global declaration has non-global binding metadata",
                        ));
                    }
                }
            }
        }
        let mut slots = Vec::with_capacity(descriptors.len());
        let mut first_lexical_roots: HashMap<Atom, VarRefRoot> = HashMap::new();
        for descriptor in descriptors.iter().copied() {
            let ClosureVariableName::Atom(name) = descriptor.name else {
                return Err(RuntimeError::Invariant(
                    "published global closure descriptor has no atom",
                ));
            };
            let root = match descriptor.source {
                ClosureSource::GlobalDeclaration => {
                    let key = PropertyKey::from_borrowed_atom(self.clone(), name)?;
                    match descriptor.kind {
                        ClosureVariableKind::Normal if descriptor.is_lexical => {
                            if let Some(root) = first_lexical_roots.get(&name) {
                                root.clone()
                            } else {
                                let root = self.create_global_lexical_binding(
                                    caller_realm,
                                    &key,
                                    descriptor.is_const,
                                    None,
                                )?;
                                first_lexical_roots.insert(name, root.clone());
                                root
                            }
                        }
                        ClosureVariableKind::Normal => self.create_global_var_binding(
                            caller_realm,
                            &key,
                            GlobalBindingCreationMode::Script,
                        )?,
                        ClosureVariableKind::GlobalFunction
                            if !descriptor.is_lexical && !descriptor.is_const =>
                        {
                            self.create_global_function_binding(
                                caller_realm,
                                &key,
                                GlobalBindingCreationMode::Script,
                            )?
                        }
                        ClosureVariableKind::FunctionName
                        | ClosureVariableKind::GlobalFunction
                        | ClosureVariableKind::EvalVariableObject
                        | ClosureVariableKind::WithObject => {
                            return Err(RuntimeError::Invariant(
                                "global declaration has non-global binding metadata",
                            ));
                        }
                    }
                }
                ClosureSource::Global => self.resolve_global_var(caller_realm, name)?,
                ClosureSource::ParentLocal(_)
                | ClosureSource::ParentArgument(_)
                | ClosureSource::ParentClosure(_)
                | ClosureSource::ParentGlobal(_)
                | ClosureSource::EvalEnvironment(_) => {
                    return Err(RuntimeError::Invariant(
                        "root bytecode closure descriptor used a child source",
                    ));
                }
            };
            slots.push(root);
        }
        self.new_bytecode_closure_with_slots(caller_realm, function, &slots)
    }

    fn check_global_lexical_declaration(
        &self,
        realm: ContextId,
        key: &PropertyKey,
    ) -> Result<(), RuntimeError> {
        let conflicts = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let lexical = state.heap.object(context.global_var_object)?;
            let lexical_shape = state.heap.shape(lexical.shape)?;
            let global = state.heap.object(context.global_object)?;
            let global_shape = state.heap.shape(global.shape)?;
            let lexical_exists = lexical_shape.find(key.atom()).is_some();
            let fixed_global_exists = global_shape
                .find(key.atom())
                .and_then(|index| global_shape.entries().get(index as usize))
                .is_some_and(|entry| !entry.flags.configurable);
            lexical_exists || fixed_global_exists
        };
        if conflicts {
            let error =
                self.native_atom_error(ErrorKind::Syntax, "redeclaration of '", key, "'")?;
            return Err(RuntimeError::Engine(error));
        }
        Ok(())
    }

    fn check_global_var_declaration(
        &self,
        realm: ContextId,
        key: &PropertyKey,
    ) -> Result<(), RuntimeError> {
        let conflict = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let lexical = state.heap.object(context.global_var_object)?;
            let lexical_shape = state.heap.shape(lexical.shape)?;
            let global = state.heap.object(context.global_object)?;
            let global_shape = state.heap.shape(global.shape)?;
            if global_shape.find(key.atom()).is_none() && !global.extensible {
                Some(ErrorKind::Type)
            } else if lexical_shape.find(key.atom()).is_some() {
                Some(ErrorKind::Syntax)
            } else {
                None
            }
        };
        match conflict {
            Some(ErrorKind::Type) => {
                let error =
                    self.native_atom_error(ErrorKind::Type, "cannot define variable '", key, "'")?;
                Err(RuntimeError::Engine(error))
            }
            Some(ErrorKind::Syntax) => {
                let error =
                    self.native_atom_error(ErrorKind::Syntax, "redeclaration of '", key, "'")?;
                Err(RuntimeError::Engine(error))
            }
            Some(_) => Err(RuntimeError::Invariant(
                "global var preflight produced an impossible error kind",
            )),
            None => Ok(()),
        }
    }

    fn check_global_function_declaration(
        &self,
        realm: ContextId,
        key: &PropertyKey,
    ) -> Result<(), RuntimeError> {
        let conflict =
            {
                let state = self.0.state.borrow();
                let context = state.heap.context(realm)?;
                let lexical = state.heap.object(context.global_var_object)?;
                let lexical_shape = state.heap.shape(lexical.shape)?;
                let global = state.heap.object(context.global_object)?;
                let global_shape = state.heap.shape(global.shape)?;
                let cannot_define =
                    match global_shape.find(key.atom()) {
                        None => !global.extensible,
                        Some(index) => {
                            let index = usize::try_from(index).map_err(|_| {
                                RuntimeError::Invariant("shape index does not fit usize")
                            })?;
                            let entry = global_shape.entries().get(index).ok_or(
                                RuntimeError::Invariant("shape lookup index was out of bounds"),
                            )?;
                            let slot = global.slots.get(index).ok_or(RuntimeError::Invariant(
                                "shape property has no parallel object slot",
                            ))?;
                            !entry.flags.configurable
                                && (matches!(slot, PropertySlot::Accessor { .. })
                                    || !entry.flags.writable
                                    || !entry.flags.enumerable)
                        }
                    };
                if cannot_define {
                    Some(ErrorKind::Type)
                } else if lexical_shape.find(key.atom()).is_some() {
                    Some(ErrorKind::Syntax)
                } else {
                    None
                }
            };
        match conflict {
            Some(ErrorKind::Type) => {
                let error =
                    self.native_atom_error(ErrorKind::Type, "cannot define variable '", key, "'")?;
                Err(RuntimeError::Engine(error))
            }
            Some(ErrorKind::Syntax) => {
                let error =
                    self.native_atom_error(ErrorKind::Syntax, "redeclaration of '", key, "'")?;
                Err(RuntimeError::Engine(error))
            }
            Some(_) => Err(RuntimeError::Invariant(
                "global function preflight produced an impossible error kind",
            )),
            None => Ok(()),
        }
    }

    fn resolve_global_var(&self, realm: ContextId, name: Atom) -> Result<VarRefRoot, RuntimeError> {
        let key = PropertyKey::from_borrowed_atom(self.clone(), name)?;
        let global_var_object = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            context.global_var_object
        };
        let global_var_object = ObjectRef::from_borrowed_handle(self.clone(), global_var_object)?;
        if let Some(root) = self.own_var_ref_root(&global_var_object, &key)? {
            return Ok(root);
        }

        self.resolve_global_object_var(realm, &key)
    }

    /// Resolve only the object-environment half of a global name. Program
    /// function declarations use this after an earlier lexical declaration
    /// of the same name: QuickJS creates a distinct global-object binding even
    /// though ordinary identifier resolution still selects the lexical slot.
    fn resolve_global_object_var(
        &self,
        realm: ContextId,
        key: &PropertyKey,
    ) -> Result<VarRefRoot, RuntimeError> {
        let (global_object, hidden) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let global = state.heap.object(context.global_object)?;
            let ObjectPayload::GlobalObject { uninitialized_vars } = global.payload else {
                return Err(RuntimeError::Invariant(
                    "realm global object has no unresolved-name table",
                ));
            };
            (context.global_object, uninitialized_vars)
        };

        let global_object = ObjectRef::from_borrowed_handle(self.clone(), global_object)?;
        if self.is_auto_init_own_property(&global_object, key)? {
            self.materialize_auto_init_property(&global_object, key)?;
        }
        if let Some(root) = self.own_var_ref_root(&global_object, key)? {
            return Ok(root);
        }

        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
        if let Some(root) = self.own_var_ref_root(&hidden, key)? {
            return Ok(root);
        }
        let root = self.new_uninitialized_var_ref()?;
        self.store_property_slot(
            &hidden,
            key,
            PropertyFlags::data(true, true, true),
            PropertySlot::VarRef(root.id()),
        )?;
        Ok(root)
    }

    #[cfg(test)]
    fn create_global_lexical_for_test(
        &self,
        realm: ContextId,
        name: &str,
        is_const: bool,
        initial_value: Option<Value>,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        self.create_global_lexical_binding(realm, &key, is_const, initial_value)
            .map(drop)
    }

    #[cfg(test)]
    fn create_global_lexical_js_string_for_test(
        &self,
        realm: ContextId,
        name: &JsString,
        is_const: bool,
        initial_value: Option<Value>,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key_js_string(name)?;
        self.create_global_lexical_binding(realm, &key, is_const, initial_value)
            .map(drop)
    }

    fn create_global_lexical_binding(
        &self,
        realm: ContextId,
        key: &PropertyKey,
        is_const: bool,
        initial_value: Option<Value>,
    ) -> Result<VarRefRoot, RuntimeError> {
        let (global_var_object, global_object, hidden) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let global = state.heap.object(context.global_object)?;
            let ObjectPayload::GlobalObject { uninitialized_vars } = &global.payload else {
                return Err(RuntimeError::Invariant(
                    "realm global object has no unresolved-name table",
                ));
            };
            (
                context.global_var_object,
                context.global_object,
                *uninitialized_vars,
            )
        };
        let global_var_object = ObjectRef::from_borrowed_handle(self.clone(), global_var_object)?;
        if self.has_own_property(&global_var_object, key)? {
            return Err(RuntimeError::Invariant(
                "attempted to redeclare a global lexical binding after preflight",
            ));
        }
        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
        let global_object = ObjectRef::from_borrowed_handle(self.clone(), global_object)?;
        let root = if let Some(root) = self.own_var_ref_root(&global_object, key)? {
            let (flags, value) = {
                let state = self.0.state.borrow();
                let object = state.heap.object(global_object.object_id())?;
                let shape = state.heap.shape(object.shape)?;
                let index = shape.find(key.atom()).ok_or(RuntimeError::Invariant(
                    "global VarRef disappeared during lexical creation",
                ))? as usize;
                let flags = shape.entries()[index].flags;
                let value = state.heap.var_ref(root.id())?.value.clone();
                (flags, value)
            };
            let value = self.root_raw_value(&value)?;
            let replacement =
                self.new_var_ref(value, false, !flags.writable, ClosureVariableKind::Normal)?;
            self.store_property_slot(
                &global_object,
                key,
                flags,
                PropertySlot::VarRef(replacement.id()),
            )?;
            self.reset_var_ref_uninitialized(&root)?;
            root
        } else if let Some(root) = self.own_var_ref_root(&hidden, key)? {
            if !self.delete_property(&hidden, key)? {
                return Err(RuntimeError::Invariant(
                    "hidden global VarRef property was not configurable",
                ));
            }
            root
        } else {
            self.new_uninitialized_var_ref()?
        };
        self.set_var_ref_metadata(&root, true, is_const, ClosureVariableKind::Normal)?;
        if let Some(value) = initial_value {
            self.write_var_ref(&root, value)?;
        }
        self.store_property_slot(
            &global_var_object,
            key,
            PropertyFlags::data(!is_const, true, true),
            PropertySlot::VarRef(root.id()),
        )?;
        Ok(root)
    }

    fn create_global_var_binding(
        &self,
        realm: ContextId,
        key: &PropertyKey,
        mode: GlobalBindingCreationMode,
    ) -> Result<VarRefRoot, RuntimeError> {
        let (global_object, hidden) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let global = state.heap.object(context.global_object)?;
            let ObjectPayload::GlobalObject { uninitialized_vars } = global.payload else {
                return Err(RuntimeError::Invariant(
                    "realm global object has no unresolved-name table",
                ));
            };
            (context.global_object, uninitialized_vars)
        };
        let root = self.resolve_global_var(realm, key.atom())?;
        let global_object = ObjectRef::from_borrowed_handle(self.clone(), global_object)?;
        if self.has_own_property(&global_object, key)? {
            return Ok(root);
        }
        if !self.is_extensible(&global_object)? {
            return Err(RuntimeError::Invariant(
                "global object became non-extensible after var preflight",
            ));
        }

        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
        let Some(hidden_root) = self.own_var_ref_root(&hidden, key)? else {
            return Err(RuntimeError::Invariant(
                "new global var has no unresolved VarRef",
            ));
        };
        if hidden_root.id() != root.id() {
            return Err(RuntimeError::Invariant(
                "new global var resolved a different hidden VarRef",
            ));
        }
        if !self.delete_property(&hidden, key)? {
            return Err(RuntimeError::Invariant(
                "hidden global VarRef property was not configurable",
            ));
        }
        self.write_var_ref(&root, Value::Undefined)?;
        self.set_var_ref_metadata(&root, false, false, ClosureVariableKind::Normal)?;
        self.store_property_slot(
            &global_object,
            key,
            PropertyFlags::data(true, true, mode.configurable()),
            PropertySlot::VarRef(root.id()),
        )?;
        Ok(root)
    }

    fn create_global_function_binding(
        &self,
        realm: ContextId,
        key: &PropertyKey,
        mode: GlobalBindingCreationMode,
    ) -> Result<VarRefRoot, RuntimeError> {
        let (global_object, hidden) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let global = state.heap.object(context.global_object)?;
            let ObjectPayload::GlobalObject { uninitialized_vars } = global.payload else {
                return Err(RuntimeError::Invariant(
                    "realm global object has no unresolved-name table",
                ));
            };
            (context.global_object, uninitialized_vars)
        };
        // Deliberately skip the lexical environment here. QuickJS permits the
        // ordered `let f; function f(){}` descriptor pair and creates a
        // distinct global-object binding for the function descriptor; the
        // hoisted raw initialization still targets the first (lexical) slot.
        let root = self.resolve_global_object_var(realm, key)?;
        let global_object = ObjectRef::from_borrowed_handle(self.clone(), global_object)?;
        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
        if !self.has_own_property(&global_object, key)? {
            if !self.is_extensible(&global_object)? {
                return Err(RuntimeError::Invariant(
                    "global object became non-extensible after function preflight",
                ));
            }
            let Some(hidden_root) = self.own_var_ref_root(&hidden, key)? else {
                return Err(RuntimeError::Invariant(
                    "new global function has no unresolved VarRef",
                ));
            };
            if hidden_root.id() != root.id() {
                return Err(RuntimeError::Invariant(
                    "new global function resolved a different hidden VarRef",
                ));
            }
            if !self.delete_property(&hidden, key)? {
                return Err(RuntimeError::Invariant(
                    "hidden global VarRef property was not configurable",
                ));
            }
            self.write_var_ref(&root, Value::Undefined)?;
            self.set_var_ref_metadata(&root, false, false, ClosureVariableKind::Normal)?;
            self.store_property_slot(
                &global_object,
                key,
                PropertyFlags::data(true, true, mode.configurable()),
                PropertySlot::VarRef(root.id()),
            )?;
            return Ok(root);
        }

        // Existing configurable properties are replaced with ordinary
        // writable/enumerable data properties without invoking accessors.
        // Script makes the replacement permanent while eval preserves
        // configurability. Fixed W/E data properties keep their existing
        // non-configurable attributes and VarRef identity.
        let (flags, slot) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(global_object.object_id())?;
            let shape = state.heap.shape(object.shape)?;
            let index = usize::try_from(shape.find(key.atom()).ok_or(RuntimeError::Invariant(
                "global function property disappeared after declaration creation",
            ))?)
            .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
            let flags = shape
                .entries()
                .get(index)
                .ok_or(RuntimeError::Invariant(
                    "shape lookup index was out of bounds",
                ))?
                .flags;
            let slot = object
                .slots
                .get(index)
                .ok_or(RuntimeError::Invariant(
                    "shape property has no parallel object slot",
                ))?
                .clone();
            (flags, slot)
        };

        let hidden_root = self.own_var_ref_root(&hidden, key)?;
        match &slot {
            PropertySlot::VarRef(global_root) => {
                if *global_root != root.id() {
                    return Err(RuntimeError::Invariant(
                        "global function resolved a different property VarRef",
                    ));
                }
                if hidden_root.is_some() {
                    return Err(RuntimeError::Invariant(
                        "global function name exists in both property and hidden tables",
                    ));
                }
            }
            PropertySlot::Data(value) => {
                let value = self.root_raw_value(value)?;
                self.write_var_ref(&root, value)?;
                if hidden_root
                    .as_ref()
                    .is_none_or(|hidden_root| hidden_root.id() != root.id())
                {
                    return Err(RuntimeError::Invariant(
                        "global function data property has no matching hidden VarRef",
                    ));
                }
            }
            PropertySlot::Accessor { .. } => {
                if !flags.configurable {
                    return Err(RuntimeError::Invariant(
                        "fixed global accessor survived function preflight",
                    ));
                }
                if hidden_root
                    .as_ref()
                    .is_none_or(|hidden_root| hidden_root.id() != root.id())
                {
                    return Err(RuntimeError::Invariant(
                        "global function accessor has no matching hidden VarRef",
                    ));
                }
            }
            PropertySlot::AutoInit(_) => {
                return Err(RuntimeError::Invariant(
                    "global function autoinit property was not materialized",
                ));
            }
        }

        let function_flags = if flags.configurable {
            PropertyFlags::data(true, true, mode.configurable())
        } else {
            if !flags.writable || !flags.enumerable {
                return Err(RuntimeError::Invariant(
                    "fixed global data property survived function preflight",
                ));
            }
            PropertyFlags::data(true, true, false)
        };
        self.store_property_slot(
            &global_object,
            key,
            function_flags,
            PropertySlot::VarRef(root.id()),
        )?;
        if let Some(hidden_root) = hidden_root {
            if hidden_root.id() != root.id() {
                return Err(RuntimeError::Invariant(
                    "global function hidden VarRef changed during creation",
                ));
            }
            if !self.delete_property(&hidden, key)? {
                return Err(RuntimeError::Invariant(
                    "hidden global VarRef property was not configurable",
                ));
            }
        }
        if matches!(slot, PropertySlot::Accessor { .. }) {
            self.reset_var_ref_uninitialized(&root)?;
        }
        self.set_var_ref_metadata(&root, false, false, ClosureVariableKind::Normal)?;
        Ok(root)
    }

    #[cfg(test)]
    fn initialize_global_lexical_for_test(
        &self,
        realm: ContextId,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let global_var_object = {
            let state = self.0.state.borrow();
            state.heap.context(realm)?.global_var_object
        };
        let global_var_object = ObjectRef::from_borrowed_handle(self.clone(), global_var_object)?;
        let root =
            self.own_var_ref_root(&global_var_object, &key)?
                .ok_or(RuntimeError::Invariant(
                    "test initialized a missing global lexical binding",
                ))?;
        self.write_var_ref(&root, value)
    }

    fn new_bytecode_closure_with_slots(
        &self,
        caller_realm: ContextId,
        function: &FunctionBytecodeRef,
        closure_slots: &[VarRefRoot],
    ) -> Result<CallableRef, RuntimeError> {
        let _operation = self.operation();
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        if closure_slots.iter().any(|slot| !slot.belongs_to(self)) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }

        let mut state = self.0.state.borrow_mut();
        let function_prototype = state.heap.context(caller_realm)?.function_prototype;
        let (metadata, func_name) = {
            let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
            (bytecode.metadata, bytecode.func_name.clone())
        };
        let shape = state.get_or_create_shape(Some(function_prototype), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::bytecode_function_with_closures(
                shape,
                Vec::new(),
                function.bytecode_id(),
                None,
                closure_slots.iter().map(VarRefRoot::id).collect(),
                metadata.constructor_kind != ConstructorKind::None,
            )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        let callable =
            CallableRef::from_validated_object(ObjectRef::from_owned_handle(self.clone(), object));
        self.initialize_bytecode_function_properties(caller_realm, &callable, metadata, func_name)?;
        Ok(callable)
    }

    fn initialize_bytecode_function_properties(
        &self,
        realm: ContextId,
        callable: &CallableRef,
        metadata: FunctionMetadata,
        func_name: Option<JsString>,
    ) -> Result<(), RuntimeError> {
        self.define_function_data_property(
            callable.as_object(),
            "length",
            Value::Int(i32::from(metadata.defined_argument_count)),
            false,
            true,
        )?;
        self.define_function_data_property(
            callable.as_object(),
            "name",
            Value::String(func_name.unwrap_or_else(|| JsString::from_static(""))),
            false,
            true,
        )?;
        if !metadata.has_prototype {
            return Ok(());
        }
        self.define_function_auto_init_prototype(callable.as_object(), realm)
    }

    fn define_function_auto_init_prototype(
        &self,
        function: &ObjectRef,
        realm: ContextId,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key("prototype")?;
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let object_id = function.object_id();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            if shape.find(key.atom()).is_some() {
                return Err(RuntimeError::Invariant(
                    "function prototype autoinit property already exists",
                ));
            }
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object.slots.clone(),
            )
        };
        entries.push(ShapeEntry {
            atom: key.atom(),
            flags: PropertyFlags::data(true, false, false),
        });
        slots.push(PropertySlot::AutoInit(
            AutoInitProperty::FunctionPrototype { realm },
        ));
        state.replace_layout(object_id, prototype, &entries, slots)
    }

    fn define_native_builtin_auto_init(
        &self,
        object: &ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        name: &'static str,
        length: u8,
        min_readable_args: u8,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        self.define_native_builtin_auto_init_with_key(
            object,
            realm,
            &key,
            target,
            name,
            length,
            min_readable_args,
            PropertyFlags::data(true, false, true),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn define_native_builtin_auto_init_with_key(
        &self,
        object: &ObjectRef,
        realm: ContextId,
        key: &PropertyKey,
        target: NativeFunctionId,
        name: &'static str,
        length: u8,
        min_readable_args: u8,
        flags: PropertyFlags,
    ) -> Result<(), RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let object_id = object.object_id();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            if shape.find(key.atom()).is_some() {
                return Err(RuntimeError::Invariant(
                    "native builtin autoinit property already exists",
                ));
            }
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object.slots.clone(),
            )
        };
        entries.push(ShapeEntry {
            atom: key.atom(),
            flags,
        });
        slots.push(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
            realm,
            target,
            name,
            length,
            min_readable_args,
        }));
        state.replace_layout(object_id, prototype, &entries, slots)
    }

    fn define_string_auto_init(
        &self,
        object: &ObjectRef,
        realm: ContextId,
        name: &str,
        value: &'static str,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let object_id = object.object_id();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            if shape.find(key.atom()).is_some() {
                return Err(RuntimeError::Invariant(
                    "string autoinit property already exists",
                ));
            }
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object.slots.clone(),
            )
        };
        entries.push(ShapeEntry {
            atom: key.atom(),
            flags: PropertyFlags::data(true, false, true),
        });
        slots.push(PropertySlot::AutoInit(AutoInitProperty::String {
            realm,
            value,
        }));
        state.replace_layout(object_id, prototype, &entries, slots)
    }

    #[cfg(test)]
    fn define_failure_auto_init(
        &self,
        object: &ObjectRef,
        realm: ContextId,
        name: &str,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let object_id = object.object_id();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object.slots.clone(),
            )
        };
        entries.push(ShapeEntry {
            atom: key.atom(),
            flags: PropertyFlags::data(true, false, true),
        });
        slots.push(PropertySlot::AutoInit(AutoInitProperty::FailureProbe {
            realm,
        }));
        state.replace_layout(object_id, prototype, &entries, slots)
    }

    fn define_function_data_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: Value,
        writable: bool,
        configurable: bool,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let defined = self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(writable),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(configurable),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Invariant(
                "function intrinsic property definition was rejected",
            ));
        }
        Ok(())
    }

    /// QuickJS `JS_DefineObjectName`: define a configurable, non-writable,
    /// non-enumerable own `name` only when the object does not already carry a
    /// non-empty (or otherwise authoritative) own name.
    fn define_object_name(&self, value: &Value, name: &JsString) -> Result<(), RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(());
        };
        let key = self.intern_property_key("name")?;
        let should_define = match self.get_own_property(object, &key)? {
            None => true,
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(current),
                ..
            }) => current.is_empty(),
            Some(
                CompleteOrdinaryPropertyDescriptor::Data { .. }
                | CompleteOrdinaryPropertyDescriptor::Accessor { .. },
            ) => false,
        };
        if !should_define {
            return Ok(());
        }
        let defined = self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(name.clone())),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "cannot define function name",
            )));
        }
        Ok(())
    }

    fn new_var_ref(
        &self,
        value: Value,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Result<VarRefRoot, RuntimeError> {
        let _operation = self.operation();
        self.validate_value_domain(&value, "captured variable")?;
        let raw = self.raw_property_value(&value)?;
        let mut state = self.0.state.borrow_mut();
        let retained_atom = if let RawValue::Symbol(atom) = &raw {
            state.atoms.retain(*atom)?;
            Some(*atom)
        } else {
            None
        };
        let data = VarRefData::captured(raw, is_lexical, is_const, kind);
        let id = match state.heap.allocate_var_ref(data) {
            Ok(id) => id,
            Err(error) => {
                if let Some(atom) = retained_atom {
                    state.atoms.release(atom)?;
                }
                return Err(error.into());
            }
        };
        drop(state);
        drop(value);
        Ok(VarRefRoot::from_owned_handle(self.clone(), id))
    }

    fn new_uninitialized_var_ref(&self) -> Result<VarRefRoot, RuntimeError> {
        let _operation = self.operation();
        let id = self
            .0
            .state
            .borrow_mut()
            .heap
            .allocate_var_ref(VarRefData::captured(
                RawValue::Uninitialized,
                false,
                false,
                ClosureVariableKind::Normal,
            ))?;
        Ok(VarRefRoot::from_owned_handle(self.clone(), id))
    }

    fn new_uninitialized_captured_var_ref(
        &self,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Result<VarRefRoot, RuntimeError> {
        let _operation = self.operation();
        let id = self
            .0
            .state
            .borrow_mut()
            .heap
            .allocate_var_ref(VarRefData::captured(
                RawValue::Uninitialized,
                is_lexical,
                is_const,
                kind,
            ))?;
        Ok(VarRefRoot::from_owned_handle(self.clone(), id))
    }

    fn set_var_ref_metadata(
        &self,
        root: &VarRefRoot,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Result<(), RuntimeError> {
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        self.0.state.borrow_mut().heap.set_var_ref_metadata(
            root.id(),
            is_lexical,
            is_const,
            kind,
        )?;
        Ok(())
    }

    fn reset_var_ref_uninitialized(&self, root: &VarRefRoot) -> Result<(), RuntimeError> {
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        let mut state = self.0.state.borrow_mut();
        let cleanup = state
            .heap
            .replace_var_ref_value(root.id(), RawValue::Uninitialized)?;
        state.apply_cleanup(cleanup)
    }

    fn read_var_ref(&self, root: &VarRefRoot) -> Result<Value, RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        let raw = self.0.state.borrow().heap.var_ref(root.id())?.value.clone();
        self.root_raw_value(&raw)
    }

    fn raw_var_ref_value(&self, root: &VarRefRoot) -> Result<RawValue, RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        Ok(self.0.state.borrow().heap.var_ref(root.id())?.value.clone())
    }

    fn validate_var_ref_metadata(
        &self,
        root: &VarRefRoot,
        descriptor: ClosureVariable,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        let var_ref = self.0.state.borrow();
        let var_ref = var_ref.heap.var_ref(root.id())?;
        if !vm_host::closure_view_matches_cell(
            (var_ref.is_lexical, var_ref.is_const, var_ref.kind),
            descriptor,
        ) {
            return Err(RuntimeError::Invariant(
                "closure descriptor metadata does not match the shared variable cell",
            ));
        }
        Ok(())
    }

    fn write_var_ref(&self, root: &VarRefRoot, value: Value) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        self.validate_value_domain(&value, "captured variable")?;
        let raw = self.raw_property_value(&value)?;
        let mut state = self.0.state.borrow_mut();
        let retained_atom = if let RawValue::Symbol(atom) = &raw {
            state.atoms.retain(*atom)?;
            Some(*atom)
        } else {
            None
        };
        let cleanup = match state.heap.replace_var_ref_value(root.id(), raw) {
            Ok(cleanup) => cleanup,
            Err(error) => {
                if let Some(atom) = retained_atom {
                    state.atoms.release(atom)?;
                }
                return Err(error.into());
            }
        };
        state.apply_cleanup(cleanup)?;
        drop(state);
        drop(value);
        Ok(())
    }

    /// Run QuickJS-style cycle collection for this runtime.
    pub fn run_gc(&self) -> Result<GcStats, RuntimeError> {
        let _operation = self.operation();
        let mut state = self.0.state.borrow_mut();
        let mut stats = state.heap.run_gc()?;
        let atoms = std::mem::take(&mut stats.cleanup.atoms);
        state.unlink_finalized_shapes(stats.cleanup.finalized_shape_ids.iter().copied());
        state.release_atoms(atoms)?;
        state.atoms.sweep_released_strings();
        Ok(stats)
    }

    /// Runtime heap population for diagnostics and lifecycle tests.
    #[must_use]
    pub fn heap_counts(&self) -> HeapCounts {
        let _operation = self.operation();
        self.0.state.borrow().heap.counts()
    }

    fn set_pending_exception(&self, value: Value) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        self.validate_value_domain(&value, "exception value")?;
        let raw = self.raw_property_value(&value)?;
        {
            let mut state = self.0.state.borrow_mut();
            state.retain_raw_root(&raw)?;
            if let Some(previous) = state.pending_exception.replace(raw) {
                state.release_owned_raw_root(previous)?;
            }
        }
        // `raw` now owns its own retained occurrence.
        drop(value);
        Ok(())
    }

    fn take_pending_exception(&self) -> Result<Option<Value>, RuntimeError> {
        let _operation = self.operation();
        let pending = self.0.state.borrow_mut().pending_exception.take();
        pending
            .map(|value| self.take_owned_raw_value(value))
            .transpose()
    }

    fn has_pending_exception(&self) -> bool {
        let _operation = self.operation();
        self.0.state.borrow().pending_exception.is_some()
    }

    fn take_owned_raw_value(&self, value: RawValue) -> Result<Value, RuntimeError> {
        Ok(match value {
            RawValue::Undefined => Value::Undefined,
            RawValue::Null => Value::Null,
            RawValue::Bool(value) => Value::Bool(value),
            RawValue::Int(value) => Value::Int(value),
            RawValue::Float(value) => Value::Float(value),
            RawValue::BigInt(value) => Value::BigInt(value),
            RawValue::String(value) => Value::String(value),
            RawValue::Symbol(atom) => Value::Symbol(SymbolRef::from_owned_atom(self.clone(), atom)),
            RawValue::Object(object) => {
                Value::Object(ObjectRef::from_owned_handle(self.clone(), object))
            }
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel occupied the pending exception slot",
                ));
            }
        })
    }

    /// Consume a verified compiler draft and publish an immutable bytecode GC
    /// node in `realm`.
    fn publish_unlinked_function(
        &self,
        realm: ContextId,
        function: UnlinkedFunction,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        bytecode_publish::verify_unlinked_tree(&function)?;
        self.publish_verified_unlinked_function(realm, function)
    }

    fn publish_verified_unlinked_function(
        &self,
        realm: ContextId,
        function: UnlinkedFunction,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        let flat_functions = bytecode_publish::flatten_unlinked_tree(function)?;
        let _operation = self.operation();
        let mut roots: Vec<Option<FunctionBytecodeRef>> = Vec::with_capacity(flat_functions.len());

        for function in flat_functions {
            let mut linked_constants = Vec::with_capacity(function.constants.len());
            let mut atom_string_constants = Vec::new();
            let mut children = Vec::new();
            for constant in function.constants {
                match constant {
                    FlatConstant::Value(value) => {
                        linked_constants.push(BytecodeConstant::Value(value));
                    }
                    FlatConstant::AtomString(value) => {
                        atom_string_constants.push(linked_constants.len());
                        linked_constants.push(BytecodeConstant::Value(RawValue::String(value)));
                    }
                    FlatConstant::RegExp { pattern, program } => {
                        linked_constants.push(BytecodeConstant::RegExp { pattern, program });
                    }
                    FlatConstant::Child(index) => {
                        let child = roots.get(index).and_then(Option::as_ref).ok_or(
                            RuntimeError::Invariant(
                                "flattened child function root was unavailable",
                            ),
                        )?;
                        linked_constants.push(BytecodeConstant::Function(child.bytecode_id()));
                        children.push(index);
                    }
                }
            }

            let mut closure_variables = function.closure_variables;
            let eval_environments = function.eval_environments;
            let argument_definitions = function.argument_definitions;
            let local_definitions = function.local_definitions;
            let mut linked_argument_definitions = Vec::with_capacity(argument_definitions.len());
            let mut linked_local_definitions = Vec::with_capacity(local_definitions.len());
            let mut linked_eval_environments = Vec::with_capacity(eval_environments.len());
            let mut unlinked_debug = function.debug;
            let mut linked_debug = None;
            let mut auxiliary_atoms = Vec::new();
            let id = {
                let mut state = self.0.state.borrow_mut();
                let linking = (|| -> Result<(), RuntimeError> {
                    for index in atom_string_constants {
                        let value = match linked_constants.get(index) {
                            Some(BytecodeConstant::Value(RawValue::String(value))) => value.clone(),
                            Some(BytecodeConstant::Value(_))
                            | Some(BytecodeConstant::RegExp { .. })
                            | Some(BytecodeConstant::Function(_))
                            | None => {
                                return Err(RuntimeError::Invariant(
                                    "atom-string constant lost its String payload",
                                ));
                            }
                        };
                        let atom = state.atoms.intern_property_key_js_string(&value)?;
                        // QuickJS falls back to an ordinary independent cpool
                        // String when JS_NewAtomStr produces a tagged integer.
                        if atom.is_immediate_integer() {
                            continue;
                        }
                        auxiliary_atoms.push(atom);
                        let canonical = state.atoms.to_js_string(atom)?;
                        linked_constants[index] =
                            BytecodeConstant::Value(RawValue::String(canonical));
                    }
                    if let Some(debug) = unlinked_debug.take() {
                        let filename =
                            state.atoms.intern_property_key_js_string(&debug.filename)?;
                        auxiliary_atoms.push(filename);
                        linked_debug = Some(FunctionDebugInfo {
                            filename,
                            pc2line: debug.pc2line,
                            source: debug.source,
                        });
                    }
                    for descriptor in &mut closure_variables {
                        let ClosureVariableName::Constant(index) = descriptor.name else {
                            continue;
                        };
                        let name = usize::try_from(index)
                            .ok()
                            .and_then(|index| linked_constants.get(index))
                            .and_then(|constant| match constant {
                                BytecodeConstant::Value(RawValue::String(name)) => Some(name),
                                BytecodeConstant::Value(_)
                                | BytecodeConstant::RegExp { .. }
                                | BytecodeConstant::Function(_) => None,
                            })
                            .ok_or(RuntimeError::Invariant(
                                "verified closure name was not a string constant",
                            ))?;
                        let atom = state.atoms.intern_property_key_js_string(name)?;
                        auxiliary_atoms.push(atom);
                        descriptor.name = ClosureVariableName::Atom(atom);
                    }
                    for definition in argument_definitions {
                        let name = definition
                            .name
                            .as_ref()
                            .map(|name| state.atoms.intern_property_key_js_string(name))
                            .transpose()?;
                        auxiliary_atoms.extend(name);
                        linked_argument_definitions.push(VariableDefinition {
                            name,
                            is_lexical: definition.is_lexical,
                            is_const: definition.is_const,
                            kind: definition.kind,
                        });
                    }
                    for definition in local_definitions {
                        let name = definition
                            .name
                            .as_ref()
                            .map(|name| state.atoms.intern_property_key_js_string(name))
                            .transpose()?;
                        auxiliary_atoms.extend(name);
                        linked_local_definitions.push(VariableDefinition {
                            name,
                            is_lexical: definition.is_lexical,
                            is_const: definition.is_const,
                            kind: definition.kind,
                        });
                    }
                    linked_eval_environments = bytecode_publish::link_eval_environments(
                        &mut state,
                        eval_environments,
                        &mut auxiliary_atoms,
                    )?;
                    Ok(())
                })();
                if let Err(error) = linking {
                    state.release_atoms(auxiliary_atoms.drain(..))?;
                    return Err(error);
                }

                let owned_atoms = auxiliary_atoms.clone();
                let bytecode = FunctionBytecodeData {
                    code: function.code.into(),
                    constants: linked_constants.into(),
                    realm,
                    metadata: function.metadata,
                    func_name: function.func_name,
                    argument_definitions: linked_argument_definitions.into(),
                    local_definitions: linked_local_definitions.into(),
                    closure_variables: closure_variables.into(),
                    eval_environments: linked_eval_environments.into(),
                    debug: linked_debug,
                    auxiliary_atoms: auxiliary_atoms.into_boxed_slice(),
                };
                match state.heap.allocate_function_bytecode(bytecode) {
                    Ok(id) => id,
                    Err(error) => {
                        state.release_atoms(owned_atoms)?;
                        return Err(error.into());
                    }
                }
            };
            let root = FunctionBytecodeRef::from_owned_handle(self.clone(), id);

            // The parent node now owns each child through its cpool edge.
            for child in children {
                drop(roots[child].take());
            }
            roots.push(Some(root));
        }

        roots
            .last_mut()
            .and_then(Option::take)
            .ok_or(RuntimeError::Invariant(
                "unlinked function tree produced no published root",
            ))
    }

    /// Compile and publish source without mutating the runtime pending-
    /// exception slot. Native indirect-eval paths need the thrown value as a
    /// normal completion, while the public Context boundary installs that
    /// same value into the pending slot before returning `Exception`.
    fn compile_in_realm(
        &self,
        realm: ContextId,
        source: &str,
        filename: &str,
        preserve_unsupported_diagnostics: bool,
    ) -> Result<Compilation, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let debug_info = self.debug_info_mode();
        let function = match compile_unlinked_script_with_filename(source, filename, debug_info) {
            Ok(function) => function,
            Err(mut error) => {
                if error.kind() == ErrorKind::Unsupported && !preserve_unsupported_diagnostics {
                    let span = error.span();
                    error = Error::new(ErrorKind::Syntax, error.message().to_owned());
                    if let Some(span) = span {
                        error = error.with_span(span);
                    }
                }
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                let explicit_location = if error.kind() == ErrorKind::Syntax {
                    if let Some(span) = error.span() {
                        let position = QuickJsSourceLocator::new(source)
                            .locate_byte_offset(span.start.byte_offset)
                            .map_err(|_| {
                                RuntimeError::Invariant(
                                    "syntax-error byte offset is invalid for its source",
                                )
                            })?;
                        Some(ExplicitBacktraceLocation {
                            filename: JsString::try_from_utf8(filename)?,
                            position,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };
                let exception = if error.kind() == ErrorKind::Syntax {
                    self.new_native_error_without_backtrace_from_error(realm, kind, &error)?
                } else {
                    self.new_native_error_from_error(realm, kind, &error)?
                };
                self.ensure_error_backtrace(&exception, false, explicit_location)?;
                return Ok(Compilation::Throw(exception));
            }
        };
        Ok(Compilation::Published(
            self.publish_unlinked_function(realm, function)?,
        ))
    }

    fn snapshot_function_bytecode(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<PublishedFunctionSnapshot, RuntimeError> {
        let _operation = self.operation();
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let root = function.clone();
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        // The realm is a strong edge of the bytecode node. Validating it here
        // makes a corrupt realm edge fail before entering a VM frame.
        state.heap.context(bytecode.realm)?;
        Ok(PublishedFunctionSnapshot {
            root,
            code: bytecode.code.clone(),
            constants: bytecode.constants.clone(),
            argument_definitions: bytecode.argument_definitions.clone(),
            local_definitions: bytecode.local_definitions.clone(),
            closure_variables: bytecode.closure_variables.clone(),
            eval_environments: bytecode.eval_environments.clone(),
            metadata: bytecode.metadata,
            realm: bytecode.realm,
        })
    }

    #[cfg(test)]
    pub(crate) fn test_function_debug_location(
        &self,
        function: &FunctionBytecodeRef,
        pc: Option<usize>,
    ) -> Result<Option<(JsString, LineColumn)>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        let Some(debug) = &bytecode.debug else {
            return Ok(None);
        };
        let filename = state.atoms.to_js_string(debug.filename)?;
        let position = debug
            .pc2line
            .as_ref()
            .map(|table| table.lookup(pc.and_then(|pc| u32::try_from(pc).ok())));
        Ok(position.map(|position| (filename, position)))
    }

    #[cfg(test)]
    pub(crate) fn test_function_debug_source(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let state = self.0.state.borrow();
        Ok(state
            .heap
            .function_bytecode(function.bytecode_id())?
            .debug
            .as_ref()
            .and_then(|debug| debug.source.as_deref())
            .map(<[u8]>::to_vec))
    }

    #[cfg(test)]
    pub(crate) fn test_function_code(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Vec<crate::bytecode::Instruction>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(function.bytecode_id())?
            .code
            .to_vec())
    }

    #[cfg(test)]
    pub(crate) fn test_function_name(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Option<JsString>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(function.bytecode_id())?
            .func_name
            .clone())
    }

    #[cfg(test)]
    pub(crate) fn test_debug_filename_atom_ownership(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Option<(usize, Option<u32>)>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        let Some(filename) = bytecode.debug.as_ref().map(|debug| debug.filename) else {
            return Ok(None);
        };
        let local_ownership = bytecode
            .auxiliary_atoms
            .iter()
            .filter(|atom| **atom == filename)
            .count();
        let total_ref_count = state.atoms.resolve(filename)?.ref_count;
        Ok(Some((local_ownership, total_ref_count)))
    }

    #[cfg(test)]
    pub(crate) fn test_atom_count(&self) -> usize {
        self.0.state.borrow().atoms.len()
    }

    #[cfg(test)]
    pub(crate) fn test_child_function_bytecode(
        &self,
        function: &FunctionBytecodeRef,
        constant_index: usize,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let id = {
            let state = self.0.state.borrow();
            let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
            match bytecode.constants.get(constant_index) {
                Some(BytecodeConstant::Function(id)) => *id,
                Some(BytecodeConstant::Value(_) | BytecodeConstant::RegExp { .. }) => {
                    return Err(RuntimeError::Invariant(
                        "requested child constant is a value",
                    ));
                }
                None => {
                    return Err(RuntimeError::Invariant(
                        "requested child constant is out of bounds",
                    ));
                }
            }
        };
        Ok(FunctionBytecodeRef::from_borrowed_handle(self.clone(), id)?)
    }

    fn push_active_frame(
        &self,
        function_root: ObjectRef,
        bytecode_root: Option<FunctionBytecodeRef>,
        realm: ContextId,
        flags: ActiveFrameFlags,
        kind: ActiveFrameKind,
    ) -> Result<ActiveFrameGuard, RuntimeError> {
        if !function_root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("active-frame function"));
        }
        if bytecode_root
            .as_ref()
            .is_some_and(|root| !root.belongs_to(self))
        {
            return Err(RuntimeError::WrongRuntime("active-frame bytecode"));
        }

        let (token, depth) = {
            let mut state = self.0.state.borrow_mut();
            state.heap.context(realm)?;
            let object = state.heap.object(function_root.object_id())?;
            match (kind, &object.payload, bytecode_root.as_ref()) {
                (
                    ActiveFrameKind::Bytecode { bytecode, .. },
                    ObjectPayload::BytecodeFunction {
                        bytecode: object_bytecode,
                        ..
                    },
                    Some(root),
                ) if *object_bytecode == bytecode && root.bytecode_id() == bytecode => {
                    if state.heap.function_bytecode(bytecode)?.realm != realm {
                        return Err(RuntimeError::Invariant(
                            "bytecode active frame realm disagrees with its bytecode",
                        ));
                    }
                }
                (
                    ActiveFrameKind::Native {
                        target,
                        actual_arg_count,
                        readable_arg_count,
                    },
                    ObjectPayload::NativeFunction { data },
                    None,
                ) if data.target == target
                    && data.realm == Some(realm)
                    && readable_arg_count
                        == actual_arg_count.max(usize::from(data.min_readable_args)) => {}
                (ActiveFrameKind::Bytecode { .. }, _, _) => {
                    return Err(RuntimeError::Invariant(
                        "bytecode active frame disagrees with its rooted callable",
                    ));
                }
                (ActiveFrameKind::Native { .. }, _, _) => {
                    return Err(RuntimeError::Invariant(
                        "native active frame disagrees with its rooted callable",
                    ));
                }
            }

            let token = ActiveFrameToken(state.next_active_frame_token);
            state.next_active_frame_token =
                state
                    .next_active_frame_token
                    .checked_add(1)
                    .ok_or(RuntimeError::Invariant(
                        "active-frame token space was exhausted",
                    ))?;
            let depth = state.active_frames.len();
            state.active_frames.push(ActiveFrameRecord {
                token,
                function: function_root.object_id(),
                realm,
                flags,
                kind,
            });
            (token, depth)
        };

        Ok(ActiveFrameGuard {
            runtime: self.clone(),
            token,
            depth,
            active: true,
            _function_root: function_root,
            _bytecode_root: bytecode_root,
        })
    }

    fn push_bytecode_active_frame(
        &self,
        function_root: ObjectRef,
        bytecode_root: FunctionBytecodeRef,
        realm: ContextId,
        strict: bool,
    ) -> Result<ActiveFrameGuard, RuntimeError> {
        let bytecode = bytecode_root.bytecode_id();
        self.push_active_frame(
            function_root,
            Some(bytecode_root),
            realm,
            ActiveFrameFlags {
                strict,
                ..ActiveFrameFlags::default()
            },
            ActiveFrameKind::Bytecode { bytecode, pc: None },
        )
    }

    fn push_native_active_frame(
        &self,
        function_root: ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        actual_arg_count: usize,
        readable_arg_count: usize,
    ) -> Result<ActiveFrameGuard, RuntimeError> {
        self.push_active_frame(
            function_root,
            None,
            realm,
            ActiveFrameFlags::default(),
            ActiveFrameKind::Native {
                target,
                actual_arg_count,
                readable_arg_count,
            },
        )
    }

    fn update_active_bytecode_pc(
        &self,
        token: ActiveFrameToken,
        pc: BytecodePc,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let frame = state
            .active_frames
            .last_mut()
            .ok_or(RuntimeError::Invariant(
                "bytecode PC update ran without an active frame",
            ))?;
        if frame.token != token {
            return Err(RuntimeError::Invariant(
                "bytecode PC update did not target the top active frame",
            ));
        }
        let ActiveFrameKind::Bytecode { pc: frame_pc, .. } = &mut frame.kind else {
            return Err(RuntimeError::Invariant(
                "bytecode PC update targeted a native active frame",
            ));
        };
        *frame_pc = Some(pc);
        Ok(())
    }

    /// QuickJS `JS_EVAL_FLAG_BACKTRACE_BARRIER` temporarily marks the frame
    /// which existed before eval begins. New eval/nested frames remain visible
    /// and stack traversal stops before printing this caller frame.
    fn install_backtrace_barrier(
        &self,
        enabled: bool,
    ) -> Result<BacktraceBarrierGuard, RuntimeError> {
        if !enabled {
            return Ok(BacktraceBarrierGuard {
                runtime: self.clone(),
                token: None,
                previous: false,
                active: true,
            });
        }
        let (token, previous) = {
            let mut state = self.0.state.borrow_mut();
            let Some(frame) = state.active_frames.last_mut() else {
                return Ok(BacktraceBarrierGuard {
                    runtime: self.clone(),
                    token: None,
                    previous: false,
                    active: true,
                });
            };
            let previous = frame.flags.backtrace_barrier;
            frame.flags.backtrace_barrier = true;
            (frame.token, previous)
        };
        Ok(BacktraceBarrierGuard {
            runtime: self.clone(),
            token: Some(token),
            previous,
            active: true,
        })
    }

    fn restore_backtrace_barrier(
        &self,
        token: ActiveFrameToken,
        previous: bool,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let frame = state
            .active_frames
            .iter_mut()
            .find(|frame| frame.token == token)
            .ok_or(RuntimeError::Invariant(
                "backtrace-barrier caller frame disappeared during eval",
            ))?;
        frame.flags.backtrace_barrier = previous;
        Ok(())
    }

    fn restore_backtrace_barrier_fallback(&self, token: ActiveFrameToken, previous: bool) {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            if let Some(frame) = state
                .active_frames
                .iter_mut()
                .find(|frame| frame.token == token)
            {
                frame.flags.backtrace_barrier = previous;
            }
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_front(DeferredRefOp::BacktraceBarrierRestore { token, previous });
        }
    }

    fn pop_active_frame(&self, token: ActiveFrameToken, depth: usize) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        if state.active_frames.len() == depth + 1
            && state.active_frames.last().map(|frame| frame.token) == Some(token)
        {
            state.active_frames.pop();
            return Ok(());
        }

        if let Some(position) = state
            .active_frames
            .iter()
            .rposition(|frame| frame.token == token)
        {
            state.active_frames.truncate(position);
        } else if state.active_frames.len() > depth {
            state.active_frames.truncate(depth);
        }
        Err(RuntimeError::Invariant(
            "active frame stack was not restored in LIFO order",
        ))
    }

    fn pop_active_frame_fallback(&self, token: ActiveFrameToken, depth: usize) {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            if let Some(position) = state
                .active_frames
                .iter()
                .rposition(|frame| frame.token == token)
            {
                state.active_frames.truncate(position);
            } else if state.active_frames.len() > depth {
                state.active_frames.truncate(depth);
            }
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_front(DeferredRefOp::ActiveFramePop { token, depth });
        }
    }

    fn bytecode_for_callable(
        &self,
        callable: &CallableRef,
    ) -> Result<CallableExecution, RuntimeError> {
        let _operation = self.operation();
        if !callable.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("callable"));
        }
        let (bytecode, closure_slots) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(callable.as_object().object_id())?;
            match &object.payload {
                ObjectPayload::NativeFunction { data } => {
                    let realm = data.realm.ok_or(RuntimeError::Invariant(
                        "native function was called before its defining realm was attached",
                    ))?;
                    state.heap.context(realm)?;
                    return Ok(CallableExecution::Native {
                        target: data.target,
                        realm,
                        min_readable_args: data.min_readable_args,
                    });
                }
                ObjectPayload::BoundFunction {
                    target,
                    this_value,
                    arguments,
                } => {
                    let target = *target;
                    let this_value = this_value.clone();
                    let arguments = arguments.clone();
                    drop(state);
                    let target = ObjectRef::from_borrowed_handle(self.clone(), target)?;
                    let target = CallableRef::from_validated_object(target);
                    let this_value = self.root_raw_value(&this_value)?;
                    let arguments = arguments
                        .iter()
                        .map(|argument| self.root_raw_value(argument))
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(CallableExecution::Bound {
                        target,
                        this_value,
                        arguments,
                    });
                }
                ObjectPayload::BytecodeFunction {
                    bytecode,
                    closure_slots,
                    ..
                } => (*bytecode, closure_slots.clone()),
                ObjectPayload::Ordinary
                | ObjectPayload::Date(_)
                | ObjectPayload::RegExp(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. } => {
                    return Err(RuntimeError::Engine(Error::new(
                        ErrorKind::Type,
                        "not a function",
                    )));
                }
            }
        };
        let bytecode = FunctionBytecodeRef::from_borrowed_handle(self.clone(), bytecode)?;
        let closure_slots = closure_slots
            .into_iter()
            .map(|id| VarRefRoot::from_borrowed_handle(self.clone(), id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CallableExecution::Bytecode {
            bytecode,
            closure_slots,
        })
    }

    /// Snapshot only a direct native callable. Bound and bytecode functions
    /// deliberately return `None`: QuickJS's iterator-next fast path tests the
    /// method object itself and does not unwrap wrappers before deciding which
    /// ABI to use.
    fn direct_native_callable_metadata(
        &self,
        callable: &CallableRef,
    ) -> Result<Option<(NativeFunctionId, ContextId, u8)>, RuntimeError> {
        let _operation = self.operation();
        if !callable.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("native callable"));
        }
        let state = self.0.state.borrow();
        let object = state.heap.object(callable.as_object().object_id())?;
        match &object.payload {
            ObjectPayload::NativeFunction { data } => {
                let realm = data.realm.ok_or(RuntimeError::Invariant(
                    "native function was called before its defining realm was attached",
                ))?;
                state.heap.context(realm)?;
                Ok(Some((data.target, realm, data.min_readable_args)))
            }
            ObjectPayload::BoundFunction { .. } | ObjectPayload::BytecodeFunction { .. } => {
                Ok(None)
            }
            ObjectPayload::Ordinary
            | ObjectPayload::Date(_)
            | ObjectPayload::RegExp(_)
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::RegExpStringIterator { .. } => Err(RuntimeError::Invariant(
                "validated callable no longer has a callable payload",
            )),
        }
    }

    /// Invoke a direct `NativeCProto::IteratorNext` method through the same
    /// defining-realm native frame used by an ordinary call, but retain its raw
    /// value/done result for the VM. All other callable shapes use the generic
    /// JavaScript call and iterator-result parsing path.
    fn try_call_native_iterator_next_raw(
        &self,
        callable: &CallableRef,
        iterator: Value,
    ) -> Result<Option<NativeInvokeOutcome>, RuntimeError> {
        self.validate_value_domain(&iterator, "iterator-next receiver")?;
        let Some((target, realm, min_readable_args)) =
            self.direct_native_callable_metadata(callable)?
        else {
            return Ok(None);
        };
        if target.descriptor().cproto != NativeCProto::IteratorNext {
            return Ok(None);
        }
        self.invoke_native_function(
            callable,
            realm,
            target,
            min_readable_args,
            NativeInvocation::Call {
                this_value: iterator,
            },
            &[],
            NativeInvokeMode::IteratorNextRaw,
        )
        .map(Some)
    }

    fn callable_from_value(&self, value: Value) -> Result<CallableRef, RuntimeError> {
        let Value::Object(object) = value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("callable"));
        }
        let is_callable = matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(object.object_id())?
                .payload,
            ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
        );
        if !is_callable {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        }
        Ok(CallableRef::from_validated_object(object))
    }

    fn global_object_for_realm(&self, realm: ContextId) -> Result<ObjectRef, RuntimeError> {
        let global_object = self.0.state.borrow().heap.context(realm)?.global_object;
        Ok(ObjectRef::from_borrowed_handle(
            self.clone(),
            global_object,
        )?)
    }

    fn primitive_prototype_for_realm(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .primitive_prototypes[kind.index()]
        .ok_or(RuntimeError::Invariant(
            "primitive prototype is not implemented in this realm",
        ))?;
        Ok(ObjectRef::from_borrowed_handle(self.clone(), prototype)?)
    }

    fn construct_value_internal(
        &self,
        caller_realm: ContextId,
        function: Value,
        new_target: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let constructor = match self.constructor_from_value(caller_realm, function)? {
            NativeConversion::Value(constructor) => constructor,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let new_target = match self.constructor_from_value(caller_realm, new_target)? {
            NativeConversion::Value(new_target) => new_target,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.construct_internal(caller_realm, &constructor, &new_target, arguments)
    }

    fn constructor_from_value(
        &self,
        caller_realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<CallableRef>, RuntimeError> {
        let Value::Object(object) = value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("constructor"));
        }
        let (is_callable, is_constructor) = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            let callable = matches!(
                object_data.payload,
                ObjectPayload::NativeFunction { .. }
                    | ObjectPayload::BoundFunction { .. }
                    | ObjectPayload::BytecodeFunction { .. }
            );
            (callable, object_data.is_constructor)
        };
        if !is_constructor {
            let value = Value::Object(object);
            return Ok(NativeConversion::Throw(
                self.new_not_constructor_error(caller_realm, &value)?,
            ));
        }
        if !is_callable {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        }
        Ok(NativeConversion::Value(CallableRef::from_validated_object(
            object,
        )))
    }

    fn construct_internal(
        &self,
        caller_realm: ContextId,
        constructor: &CallableRef,
        new_target: &CallableRef,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        self.0.state.borrow().heap.context(caller_realm)?;
        if !constructor.belongs_to(self) || !new_target.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("constructor"));
        }
        for argument in arguments {
            self.validate_value_domain(argument, "construct argument")?;
        }
        let mut constructor = constructor.clone();
        let mut new_target = new_target.clone();
        let mut arguments = arguments.to_vec();
        loop {
            if !self.is_constructor(constructor.as_object())? {
                return Ok(Completion::Throw(self.new_not_constructor_error(
                    caller_realm,
                    &Value::Object(constructor.as_object().clone()),
                )?));
            }
            if !self.is_constructor(new_target.as_object())? {
                return Ok(Completion::Throw(self.new_not_constructor_error(
                    caller_realm,
                    &Value::Object(new_target.as_object().clone()),
                )?));
            }

            match self.bytecode_for_callable(&constructor)? {
                CallableExecution::Bound {
                    target,
                    this_value: _,
                    arguments: bound_arguments,
                } => {
                    arguments = match self.concatenate_bound_arguments(
                        caller_realm,
                        &bound_arguments,
                        &arguments,
                    )? {
                        NativeConversion::Value(arguments) => arguments,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if constructor.as_object() == new_target.as_object() {
                        new_target = target.clone();
                    }
                    constructor = target;
                }
                CallableExecution::Native {
                    target,
                    realm,
                    min_readable_args,
                } => {
                    return self.construct_native_function(
                        &constructor,
                        realm,
                        target,
                        min_readable_args,
                        &new_target,
                        &arguments,
                    );
                }
                CallableExecution::Bytecode {
                    bytecode,
                    closure_slots,
                } => {
                    let constructor_kind = self
                        .0
                        .state
                        .borrow()
                        .heap
                        .function_bytecode(bytecode.bytecode_id())?
                        .metadata
                        .constructor_kind;
                    match constructor_kind {
                        ConstructorKind::None => {
                            return Err(RuntimeError::Invariant(
                                "constructor bit disagrees with bytecode constructor metadata",
                            ));
                        }
                        ConstructorKind::Derived => {
                            return Err(RuntimeError::Engine(Error::internal(
                                "derived constructor execution is not implemented yet",
                            )));
                        }
                        ConstructorKind::Base => {}
                    }
                    let this_value =
                        match self.create_from_constructor(caller_realm, &new_target)? {
                            Completion::Return(value) => value,
                            Completion::Throw(value) => return Ok(Completion::Throw(value)),
                        };
                    let completion = self.execute_bytecode_callable(
                        caller_realm,
                        &constructor,
                        this_value.clone(),
                        Value::Object(new_target.as_object().clone()),
                        &arguments,
                        bytecode,
                        closure_slots,
                    )?;
                    return Ok(match completion {
                        Completion::Return(value @ Value::Object(_)) => Completion::Return(value),
                        Completion::Throw(value) => Completion::Throw(value),
                        Completion::Return(_) => Completion::Return(this_value),
                    });
                }
            }
        }
    }

    fn create_from_constructor(
        &self,
        caller_realm: ContextId,
        new_target: &CallableRef,
    ) -> Result<Completion, RuntimeError> {
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype = match self.prepare_get_property(new_target.as_object(), &prototype_key)? {
            PropertyGetAction::Complete(value) => value,
            PropertyGetAction::Call { getter, receiver } => {
                match self.call_internal(caller_realm, &getter, receiver, &[])? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
        };
        let prototype = if let Value::Object(prototype) = prototype {
            prototype
        } else {
            let realm = self.callable_realm(new_target)?;
            let object_prototype = self.0.state.borrow().heap.context(realm)?.object_prototype;
            ObjectRef::from_borrowed_handle(self.clone(), object_prototype)?
        };
        Ok(Completion::Return(Value::Object(
            self.new_object(Some(&prototype))?,
        )))
    }

    fn callable_realm(&self, callable: &CallableRef) -> Result<ContextId, RuntimeError> {
        if !callable.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("callable"));
        }
        let mut callable = callable.clone();
        loop {
            let state = self.0.state.borrow();
            let object = state.heap.object(callable.as_object().object_id())?;
            match &object.payload {
                ObjectPayload::NativeFunction { data } if data.realm.is_some() => {
                    let realm = data
                        .realm
                        .expect("guard proved native function has a defining realm");
                    state.heap.context(realm)?;
                    return Ok(realm);
                }
                ObjectPayload::BytecodeFunction { bytecode, .. } => {
                    let realm = state.heap.function_bytecode(*bytecode)?.realm;
                    state.heap.context(realm)?;
                    return Ok(realm);
                }
                ObjectPayload::BoundFunction { target, .. } => {
                    let target = *target;
                    drop(state);
                    let target = ObjectRef::from_borrowed_handle(self.clone(), target)?;
                    callable = CallableRef::from_validated_object(target);
                }
                ObjectPayload::NativeFunction { .. } => {
                    return Err(RuntimeError::Invariant(
                        "native function has no defining realm",
                    ));
                }
                ObjectPayload::Ordinary
                | ObjectPayload::Date(_)
                | ObjectPayload::RegExp(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. } => {
                    return Err(RuntimeError::Engine(Error::new(
                        ErrorKind::Type,
                        "not a function",
                    )));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn construct_native_function(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        new_target: &CallableRef,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let outcome = self.invoke_native_function(
            callable,
            realm,
            target,
            min_readable_args,
            NativeInvocation::Construct {
                new_target: Value::Object(new_target.as_object().clone()),
            },
            arguments,
            NativeInvokeMode::Ordinary,
        )?;
        Self::ordinary_native_completion(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    fn call_native_function(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let outcome = self.invoke_native_function(
            callable,
            realm,
            target,
            min_readable_args,
            NativeInvocation::Call { this_value },
            arguments,
            NativeInvokeMode::Ordinary,
        )?;
        Self::ordinary_native_completion(outcome)
    }

    fn ordinary_native_completion(
        outcome: NativeInvokeOutcome,
    ) -> Result<Completion, RuntimeError> {
        match outcome {
            NativeInvokeOutcome::Completion(completion) => Ok(completion),
            NativeInvokeOutcome::IteratorNextRaw { .. } => Err(RuntimeError::Invariant(
                "ordinary native call leaked an unwrapped iterator-next outcome",
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_native_function(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        invocation: NativeInvocation,
        arguments: &[Value],
        mode: NativeInvokeMode,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        if !callable.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("native callable"));
        }
        self.0.state.borrow().heap.context(realm)?;

        // The callable root held by the caller owns the native payload and its
        // defining-realm edge for the whole invocation. Revalidate the
        // detached snapshot before recording raw identities in the frame.
        {
            let state = self.0.state.borrow();
            let object = state.heap.object(callable.as_object().object_id())?;
            let ObjectPayload::NativeFunction { data } = &object.payload else {
                return Err(RuntimeError::Invariant(
                    "native invocation target was not a native function",
                ));
            };
            if data.target != target
                || data.realm != Some(realm)
                || data.min_readable_args != min_readable_args
            {
                return Err(RuntimeError::Invariant(
                    "native invocation metadata changed after snapshot",
                ));
            }
        }

        let actual_arg_count = arguments.len();
        let available_arg_count = actual_arg_count.max(usize::from(min_readable_args));
        let mut readable = Vec::with_capacity(available_arg_count);
        readable.extend_from_slice(arguments);
        readable.resize(available_arg_count, Value::Undefined);
        let arguments = NativeArguments {
            actual_arg_count,
            readable,
        };
        let active_frame = self.push_native_active_frame(
            callable.as_object().clone(),
            realm,
            target,
            actual_arg_count,
            available_arg_count,
        )?;

        // JavaScript-style engine errors are materialized in the native
        // function's defining realm while its frame is still visible. A
        // pre-existing Error returned as an ordinary Throw completion is not
        // captured here: QuickJS pops the C frame first and lets the enclosing
        // bytecode exception boundary add any missing stack.
        let result = (|| {
            let result = match mode {
                NativeInvokeMode::Ordinary => self
                    .dispatch_native_function(target, realm, invocation, &arguments)
                    .map(NativeInvokeOutcome::Completion),
                NativeInvokeMode::IteratorNextRaw => {
                    if target.descriptor().cproto != NativeCProto::IteratorNext {
                        return Err(RuntimeError::Invariant(
                            "raw iterator-next dispatch targeted another native cproto",
                        ));
                    }
                    self.dispatch_native_iterator_next_raw(target, realm, invocation, &arguments)
                }
            };
            match result {
                Err(RuntimeError::Engine(error))
                    if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
                {
                    let kind = NativeErrorKind::from_javascript_error(error.kind())
                        .expect("guard proved this is a JavaScript-visible native error");
                    let value = self.new_native_error_from_error(realm, kind, &error)?;
                    Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)))
                }
                result => result,
            }
        })();
        active_frame.finish()?;
        result
    }

    fn active_function(&self) -> Result<ObjectRef, RuntimeError> {
        let function = self
            .0
            .state
            .borrow()
            .active_frames
            .last()
            .ok_or(RuntimeError::Invariant(
                "active function was requested without an active frame",
            ))?
            .function;
        Ok(ObjectRef::from_borrowed_handle(self.clone(), function)?)
    }

    /// QuickJS `is_backtrace_needed` plus `build_backtrace`.
    ///
    /// Only real Error-class objects without any own `stack` property are
    /// eligible. Function names are read from raw ordinary data slots so this
    /// path never invokes user code while an exception is already in flight.
    fn ensure_error_backtrace(
        &self,
        value: &Value,
        skip_first_frame: bool,
        explicit_location: Option<ExplicitBacktraceLocation>,
    ) -> Result<(), RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(());
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("backtrace Error object"));
        }

        let stack_key = self.intern_property_key("stack")?;
        let needs_backtrace = {
            let state = self.0.state.borrow();
            let data = state.heap.object(object.object_id())?;
            if !matches!(data.payload, ObjectPayload::Error) {
                false
            } else {
                state
                    .heap
                    .shape(data.shape)?
                    .find(stack_key.atom())
                    .is_none()
            }
        };
        if !needs_backtrace {
            return Ok(());
        }

        let name_key = self.intern_property_key("name")?;
        let stack = match self.build_backtrace_string(
            name_key.atom(),
            skip_first_frame,
            explicit_location.as_ref(),
        ) {
            Ok(stack) => stack,
            Err(RuntimeError::Engine(error))
                if error.kind() == ErrorKind::JsInternal
                    && error.message() == "string too long" =>
            {
                // QuickJS's void build_backtrace helper must not replace the
                // Error already being materialized when its stack text cannot
                // become an ECMAScript String.
                return Ok(());
            }
            Err(error) => return Err(error),
        };

        // Parse errors add SpiderMonkey-compatible metadata before `stack`,
        // exactly as QuickJS does. Rejection (for example after
        // preventExtensions) is intentionally silent: build_backtrace must
        // not replace the original JavaScript completion.
        if let Some(location) = explicit_location {
            let Some((line, column)) = location.position.one_based() else {
                return Err(RuntimeError::Invariant(
                    "backtrace location cannot be represented one-based",
                ));
            };
            let line = i32::try_from(line).map_err(|_| {
                RuntimeError::Invariant("backtrace line does not fit an ECMAScript Int32")
            })?;
            let column = i32::try_from(column).map_err(|_| {
                RuntimeError::Invariant("backtrace column does not fit an ECMAScript Int32")
            })?;
            for (name, property_value) in [
                ("fileName", Value::String(location.filename)),
                ("lineNumber", Value::Int(line)),
                ("columnNumber", Value::Int(column)),
            ] {
                if !self.define_backtrace_property(object, name, property_value)? {
                    return Ok(());
                }
            }
        }

        let _ = self.define_backtrace_property(object, "stack", Value::String(stack))?;
        Ok(())
    }

    fn define_backtrace_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: Value,
    ) -> Result<bool, RuntimeError> {
        let key = self.intern_property_key(name)?;
        self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
    }

    fn build_backtrace_string(
        &self,
        name_atom: Atom,
        mut skip_first_frame: bool,
        explicit_location: Option<&ExplicitBacktraceLocation>,
    ) -> Result<JsString, RuntimeError> {
        let state = self.0.state.borrow();
        let mut output = JsStringBuilder::new(0);

        if let Some(location) = explicit_location {
            let (line, column) = location
                .position
                .one_based()
                .ok_or(RuntimeError::Invariant(
                    "backtrace location cannot be represented one-based",
                ))?;
            append_backtrace_ascii(&mut output, "    at ")?;
            append_backtrace_string(&mut output, &location.filename)?;
            append_backtrace_ascii(&mut output, ":")?;
            append_backtrace_ascii(&mut output, &line.to_string())?;
            append_backtrace_ascii(&mut output, ":")?;
            append_backtrace_ascii(&mut output, &column.to_string())?;
            append_backtrace_ascii(&mut output, "\n")?;
        }

        for frame in state.active_frames.iter().rev() {
            if frame.flags.backtrace_barrier {
                break;
            }
            if skip_first_frame {
                skip_first_frame = false;
                continue;
            }

            let name = raw_string_property_one_level(&state, frame.function, name_atom)?
                .map(truncate_backtrace_c_string)
                .transpose()?
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| JsString::from_static("<anonymous>"));
            append_backtrace_ascii(&mut output, "    at ")?;
            append_backtrace_string(&mut output, &name)?;

            match frame.kind {
                ActiveFrameKind::Native { .. } => {
                    append_backtrace_ascii(&mut output, " (native)")?;
                }
                ActiveFrameKind::Bytecode { bytecode, pc } => {
                    let bytecode = state.heap.function_bytecode(bytecode)?;
                    if let Some(debug) = &bytecode.debug {
                        let filename = state.atoms.to_js_string(debug.filename)?;
                        append_backtrace_ascii(&mut output, " (")?;
                        append_backtrace_string(&mut output, &filename)?;
                        if let Some(table) = &debug.pc2line {
                            let pc = pc
                                .map(BytecodePc::index)
                                .map(u32::try_from)
                                .transpose()
                                .map_err(|_| {
                                    RuntimeError::Invariant(
                                        "active bytecode PC does not fit debug metadata",
                                    )
                                })?;
                            let (line, column) =
                                table.lookup(pc).one_based().ok_or(RuntimeError::Invariant(
                                    "bytecode debug position cannot be represented one-based",
                                ))?;
                            append_backtrace_ascii(&mut output, ":")?;
                            append_backtrace_ascii(&mut output, &line.to_string())?;
                            append_backtrace_ascii(&mut output, ":")?;
                            append_backtrace_ascii(&mut output, &column.to_string())?;
                        }
                        append_backtrace_ascii(&mut output, ")")?;
                    }
                }
            }
            append_backtrace_ascii(&mut output, "\n")?;
        }

        Ok(output.finish()?)
    }

    fn get_property_in_realm(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Completion, RuntimeError> {
        Ok(self
            .get_property_or_missing_in_realm(realm, object, key)?
            .unwrap_or(Completion::Return(Value::Undefined)))
    }

    fn prepare_get_string_property_with_receiver(
        &self,
        realm: ContextId,
        string: &JsString,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<PropertyGetAction, RuntimeError> {
        let index = self.0.state.borrow().atoms.array_index(key.atom())?;
        if let Some(index) = index
            && let Ok(index) = usize::try_from(index)
            && let Some(unit) = string.code_unit_at(index)
        {
            return Ok(PropertyGetAction::Complete(Value::String(
                JsString::from_code_unit(unit),
            )));
        }
        let length = self.intern_property_key("length")?;
        if key == &length {
            let length = i32::try_from(string.len())
                .map(Value::Int)
                .unwrap_or_else(|_| Value::number(string.len() as f64));
            return Ok(PropertyGetAction::Complete(length));
        }
        let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::String)?;
        self.prepare_get_property_with_receiver(&prototype, key, receiver)
    }

    fn get_value_property_in_realm(
        &self,
        realm: ContextId,
        receiver: Value,
        key: &PropertyKey,
    ) -> Result<Completion, RuntimeError> {
        let action = match &receiver {
            Value::Object(object) => {
                self.prepare_get_property_with_receiver(object, key, receiver.clone())?
            }
            Value::String(string) => self.prepare_get_string_property_with_receiver(
                realm,
                string,
                key,
                receiver.clone(),
            )?,
            Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::Symbol(_) => {
                let kind = match &receiver {
                    Value::Bool(_) => PrimitiveKind::Boolean,
                    Value::Int(_) | Value::Float(_) => PrimitiveKind::Number,
                    Value::BigInt(_) => PrimitiveKind::BigInt,
                    Value::Symbol(_) => PrimitiveKind::Symbol,
                    _ => unreachable!(),
                };
                let prototype = self.primitive_prototype_for_realm(realm, kind)?;
                self.prepare_get_property_with_receiver(&prototype, key, receiver.clone())?
            }
            Value::Undefined | Value::Null => {
                return Err(RuntimeError::Engine(Error::internal(
                    "primitive value property lookup is not implemented yet",
                )));
            }
        };
        match action {
            PropertyGetAction::Complete(value) => Ok(Completion::Return(value)),
            PropertyGetAction::Call { getter, receiver } => {
                self.call_internal(realm, &getter, receiver, &[])
            }
        }
    }

    fn get_property_or_missing_in_realm(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<Completion>, RuntimeError> {
        match self.prepare_get_property_or_missing(object, key)? {
            None => Ok(None),
            Some(PropertyGetAction::Complete(value)) => Ok(Some(Completion::Return(value))),
            Some(PropertyGetAction::Call { getter, receiver }) => {
                self.call_internal(realm, &getter, receiver, &[]).map(Some)
            }
        }
    }

    fn has_property(&self, object: &ObjectRef, key: &PropertyKey) -> Result<bool, RuntimeError> {
        let mut cursor = Some(object.clone());
        while let Some(current) = cursor {
            if self.has_own_property(&current, key)? {
                return Ok(true);
            }
            cursor = self.get_prototype_of(&current)?;
        }
        Ok(false)
    }

    /// Completion-aware `[[HasProperty]]` boundary used by source `in`.
    /// Ordinary objects are synchronous today; a future Proxy/exotic path can
    /// return its trap throw here without changing the VM opcode contract.
    fn has_property_in_realm(
        &self,
        _realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Completion, RuntimeError> {
        self.has_property(object, key)
            .map(|present| Completion::Return(Value::Bool(present)))
    }

    /// Completion-aware `ToPropertyKey` used by native Object APIs. Symbols
    /// retain identity; every other value uses string-hint ToPrimitive before
    /// exact UTF-16 key interning.
    fn native_to_property_key(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<PropertyKey>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value, ToPrimitiveHint::String)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value
        };
        if let Value::Symbol(symbol) = value {
            if !symbol.belongs_to(self) {
                return Err(RuntimeError::WrongRuntime("property-key symbol"));
            }
            return Ok(NativeConversion::Value(PropertyKey::from_borrowed_atom(
                self.clone(),
                symbol.atom(),
            )?));
        }
        let string = match value.to_js_string() {
            Ok(string) => string,
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                return Ok(NativeConversion::Throw(
                    self.new_native_error_from_error(realm, kind, &error)?,
                ));
            }
        };
        Ok(NativeConversion::Value(
            self.intern_property_key_js_string(&string)?,
        ))
    }

    fn native_get_present_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        name: &str,
    ) -> Result<NativeConversion<Option<Value>>, RuntimeError> {
        let key = self.intern_property_key(name)?;
        if !self.has_property(object, &key)? {
            return Ok(NativeConversion::Value(None));
        }
        match self.get_property_in_realm(realm, object, &key)? {
            Completion::Return(value) => Ok(NativeConversion::Value(Some(value))),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    /// Port of pinned QuickJS `js_obj_to_desc`. Field probes deliberately use
    /// its C order and inherited HasProperty/Get behavior. The release also
    /// replaces a throw from the `get`/`set` field getter with its own
    /// `invalid getter`/`invalid setter` TypeError, which is preserved here.
    fn native_to_property_descriptor(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<OrdinaryPropertyDescriptor>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let mut descriptor = OrdinaryPropertyDescriptor::new();

        for (name, target) in [
            ("enumerable", &mut descriptor.enumerable),
            ("configurable", &mut descriptor.configurable),
        ] {
            match self.native_get_present_property(realm, &object, name)? {
                NativeConversion::Value(Some(value)) => {
                    *target = DescriptorField::Present(value.to_boolean());
                }
                NativeConversion::Value(None) => {}
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        }
        match self.native_get_present_property(realm, &object, "value")? {
            NativeConversion::Value(Some(value)) => {
                descriptor.value = DescriptorField::Present(value);
            }
            NativeConversion::Value(None) => {}
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        }
        match self.native_get_present_property(realm, &object, "writable")? {
            NativeConversion::Value(Some(value)) => {
                descriptor.writable = DescriptorField::Present(value.to_boolean());
            }
            NativeConversion::Value(None) => {}
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        }

        for (name, target, error_message) in [
            ("get", &mut descriptor.get, "invalid getter"),
            ("set", &mut descriptor.set, "invalid setter"),
        ] {
            let field = match self.native_get_present_property(realm, &object, name)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(_) => {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        error_message,
                    )?));
                }
            };
            let Some(value) = field else {
                continue;
            };
            let accessor = match value {
                Value::Undefined => AccessorValue::Undefined,
                Value::Object(object) => {
                    let Some(callable) = self.as_callable(&object)? else {
                        return Ok(NativeConversion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            error_message,
                        )?));
                    };
                    AccessorValue::Callable(callable)
                }
                _ => {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        error_message,
                    )?));
                }
            };
            *target = DescriptorField::Present(accessor);
        }
        if descriptor.is_mixed_descriptor() {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot have setter/getter and value or writable",
            )?));
        }
        Ok(NativeConversion::Value(descriptor))
    }

    fn native_to_js_string(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<JsString>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::String)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        match value.to_js_string() {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                Ok(NativeConversion::Throw(
                    self.new_native_error_from_error(realm, kind, &error)?,
                ))
            }
        }
    }

    /// QuickJS's `JS_ToStringCheckObject`: reject nullish receivers with its
    /// dedicated diagnostic before running any observable ToString steps.
    fn native_to_string_check_object(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<JsString>, RuntimeError> {
        if matches!(value, Value::Null | Value::Undefined) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "null or undefined are forbidden",
            )?));
        }
        self.native_to_js_string(realm, value)
    }

    fn native_to_dynamic_source_fragment(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<String>, RuntimeError> {
        let value = match self.native_to_js_string(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let units = value.utf16_units().collect::<Vec<_>>();
        match String::from_utf16(&units) {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(_) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "dynamic Function source containing a lone UTF-16 surrogate is not implemented",
            )?)),
        }
    }

    fn native_to_number(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<f64>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::Number)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        match value.to_number() {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                Ok(NativeConversion::Throw(
                    self.new_native_error_from_error(realm, kind, &error)?,
                ))
            }
        }
    }

    /// QuickJS's `%Number%` constructor uses `ToNumeric`, then converts a
    /// BigInt result to binary64. Ordinary `ToNumber` deliberately remains
    /// stricter and continues to reject BigInt everywhere else.
    fn native_to_number_constructor_value(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<f64>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::Number)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        if let Value::BigInt(value) = &value {
            return Ok(NativeConversion::Value(value.to_f64()));
        }
        match value.to_number() {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                Ok(NativeConversion::Throw(
                    self.new_native_error_from_error(realm, kind, &error)?,
                ))
            }
        }
    }

    fn native_bigint_from_string(
        &self,
        realm: ContextId,
        value: &JsString,
    ) -> Result<NativeConversion<crate::bigint::JsBigInt>, RuntimeError> {
        let units = value.utf16_units().collect::<Vec<_>>();
        let Ok(value) = String::from_utf16(&units) else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Syntax,
                "invalid bigint literal",
            )?));
        };
        match crate::bigint::JsBigInt::parse_js_string(&value) {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(crate::bigint::BigIntError::InvalidSyntax) => Ok(NativeConversion::Throw(
                self.new_native_error(realm, NativeErrorKind::Syntax, "invalid bigint literal")?,
            )),
            Err(
                crate::bigint::BigIntError::BigIntTooLarge
                | crate::bigint::BigIntError::AllocationTooLarge,
            ) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "BigInt is too large to allocate",
            )?)),
            Err(error) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                &error.to_string(),
            )?)),
        }
    }

    /// QuickJS `JS_ToBigInt`: Numbers, null, undefined and Symbols are
    /// rejected, while Boolean and String inputs are accepted after ordered
    /// number-hint `ToPrimitive` for objects.
    fn native_to_bigint(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<crate::bigint::JsBigInt>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::Number)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        match value {
            Value::BigInt(value) => Ok(NativeConversion::Value(value)),
            Value::Bool(value) => Ok(NativeConversion::Value(crate::bigint::JsBigInt::from(
                i64::from(value),
            ))),
            Value::String(value) => self.native_bigint_from_string(realm, &value),
            Value::Undefined
            | Value::Null
            | Value::Int(_)
            | Value::Float(_)
            | Value::Symbol(_)
            | Value::Object(_) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot convert to bigint",
            )?)),
        }
    }

    /// BigInt constructor conversion differs from ordinary `ToBigInt` by
    /// accepting integral Number values and by using the pinned capitalized
    /// TypeError spelling for unsupported primitives.
    fn native_to_bigint_constructor_value(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<crate::bigint::JsBigInt>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::Number)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        match value {
            Value::Int(value) => Ok(NativeConversion::Value(crate::bigint::JsBigInt::from(
                value,
            ))),
            Value::Bool(value) => Ok(NativeConversion::Value(crate::bigint::JsBigInt::from(
                i64::from(value),
            ))),
            Value::BigInt(value) => Ok(NativeConversion::Value(value)),
            Value::Float(value) if !value.is_finite() => {
                Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "cannot convert NaN or Infinity to BigInt",
                )?))
            }
            Value::Float(value) if value.fract() != 0.0 => {
                Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "cannot convert to BigInt: not an integer",
                )?))
            }
            Value::Float(value) => {
                let value = crate::bigint::JsBigInt::from_integral_f64(value).ok_or(
                    RuntimeError::Invariant("finite integral f64 could not become a BigInt"),
                )?;
                Ok(NativeConversion::Value(value))
            }
            Value::String(value) => self.native_bigint_from_string(realm, &value),
            Value::Undefined | Value::Null | Value::Symbol(_) | Value::Object(_) => {
                Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "cannot convert to BigInt",
                )?))
            }
        }
    }

    /// Pinned QuickJS `JS_ToIndex`: saturating ToInt64 followed by the
    /// non-negative MAX_SAFE_INTEGER range check.
    fn native_to_index(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<u64>, RuntimeError> {
        const MAX_SAFE_INTEGER: i64 = (1_i64 << 53) - 1;
        let value = match self.native_to_int64_sat(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !(0..=MAX_SAFE_INTEGER).contains(&value) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid array index",
            )?));
        }
        Ok(NativeConversion::Value(
            u64::try_from(value).expect("validated non-negative ToIndex value fits u64"),
        ))
    }

    /// Pinned QuickJS `JS_ToInt64Sat`: number-hint coercion followed by
    /// truncation toward zero with NaN mapped to zero and infinities/outliers
    /// saturated at the signed 64-bit bounds.
    fn native_to_int64_sat(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<i64>, RuntimeError> {
        let number = match self.native_to_number(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        Ok(NativeConversion::Value(if number.is_nan() {
            0
        } else if number < i64::MIN as f64 {
            i64::MIN
        } else if number >= 2_f64.powi(63) {
            i64::MAX
        } else {
            number as i64
        }))
    }

    /// Pinned QuickJS `JS_ToInt64Clamp`, including its negative offset before
    /// the final inclusive clamp.
    fn native_to_int64_clamp(
        &self,
        realm: ContextId,
        value: &Value,
        min: i64,
        max: i64,
        negative_offset: i64,
    ) -> Result<NativeConversion<i64>, RuntimeError> {
        let mut value = match self.native_to_int64_sat(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if value < 0 {
            value += negative_offset;
        }
        Ok(NativeConversion::Value(value.clamp(min, max)))
    }

    fn native_to_length(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<u64>, RuntimeError> {
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;

        let number = match self.native_to_number(realm, value)? {
            NativeConversion::Value(number) => number,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let length = if number.is_nan() || number <= 0.0 {
            0
        } else if number >= MAX_SAFE_INTEGER as f64 {
            MAX_SAFE_INTEGER
        } else {
            // This branch is finite, positive and below 2^53, so the
            // truncating cast is the exact ToIntegerOrInfinity result.
            number as u64
        };
        Ok(NativeConversion::Value(length))
    }

    fn to_primitive(
        &self,
        realm: ContextId,
        value: Value,
        hint: ToPrimitiveHint,
    ) -> Result<Completion, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(Completion::Return(value));
        };
        let to_primitive = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToPrimitive));
        let exotic = match self.get_property_in_realm(realm, &object, &to_primitive)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if !matches!(exotic, Value::Undefined | Value::Null) {
            let Value::Object(exotic_object) = exotic else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            let Some(exotic) = self.as_callable(&exotic_object)? else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            return match self.call_internal(
                realm,
                &exotic,
                Value::Object(object),
                &[Value::String(JsString::from_static(match hint {
                    ToPrimitiveHint::String => "string",
                    ToPrimitiveHint::Number => "number",
                    ToPrimitiveHint::Default => "default",
                }))],
            )? {
                Completion::Return(Value::Object(_)) => Ok(Completion::Throw(
                    self.new_native_error(realm, NativeErrorKind::Type, "toPrimitive")?,
                )),
                completion => Ok(completion),
            };
        }

        self.ordinary_to_primitive(realm, &object, hint)
    }

    fn call_throw_type_error(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "%ThrowTypeError% did not receive a generic invocation",
            ));
        };

        // QuickJS keeps the ES5-compatible sloppy ordinary-function getter
        // exception: reading `.caller`/`.arguments` returns undefined when
        // the receiver has bytecode, is non-strict, has a prototype and the
        // shared poison function was invoked without a setter argument.
        let sloppy_legacy_get = if arguments.actual_arg_count == 0 {
            match this_value {
                Value::Object(object) => {
                    let state = self.0.state.borrow();
                    let object = state.heap.object(object.object_id())?;
                    match object.payload {
                        ObjectPayload::BytecodeFunction { bytecode, .. } => {
                            let metadata = state.heap.function_bytecode(bytecode)?.metadata;
                            !metadata.strict && metadata.has_prototype
                        }
                        ObjectPayload::Ordinary
                        | ObjectPayload::Date(_)
                        | ObjectPayload::RegExp(_)
                        | ObjectPayload::Array { .. }
                        | ObjectPayload::Arguments { .. }
                        | ObjectPayload::ArrayIterator { .. }
                        | ObjectPayload::ForInIterator(_)
                        | ObjectPayload::Primitive(_)
                        | ObjectPayload::GlobalObject { .. }
                        | ObjectPayload::Error
                        | ObjectPayload::StringIterator { .. }
                        | ObjectPayload::RegExpStringIterator { .. }
                        | ObjectPayload::BoundFunction { .. }
                        | ObjectPayload::NativeFunction { .. } => false,
                    }
                }
                Value::Undefined
                | Value::Null
                | Value::Bool(_)
                | Value::Int(_)
                | Value::Float(_)
                | Value::String(_)
                | Value::BigInt(_)
                | Value::Symbol(_) => false,
            }
        } else {
            false
        };
        if sloppy_legacy_get {
            return Ok(Completion::Return(Value::Undefined));
        }
        Ok(Completion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            "invalid property access",
        )?))
    }

    fn call_function_constructor(
        &self,
        realm: ContextId,
        kind: DynamicFunctionKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function constructor did not receive constructor-or-function invocation",
            ));
        };

        // Match js_function_constructor byte-for-byte: all parameter strings
        // precede the final body string, argc == 0 reads no padded argument,
        // and the complete wrapper is parsed as one indirect-eval unit so a
        // body strict directive can retroactively validate the parameters.
        let mut source = DynamicSourceBuilder::new();
        source.push_str("(")?;
        match kind {
            DynamicFunctionKind::Normal | DynamicFunctionKind::Generator => {}
            DynamicFunctionKind::Async | DynamicFunctionKind::AsyncGenerator => {
                source.push_str("async ")?;
            }
        }
        source.push_str("function")?;
        if matches!(
            kind,
            DynamicFunctionKind::Generator | DynamicFunctionKind::AsyncGenerator
        ) {
            source.push_str("*")?;
        }
        source.push_str(" anonymous(")?;

        let parameter_count = arguments.actual_arg_count.saturating_sub(1);
        for index in 0..parameter_count {
            if index != 0 {
                source.push_str(",")?;
            }
            let parameter =
                match self.native_to_dynamic_source_fragment(realm, &arguments.readable[index])? {
                    NativeConversion::Value(parameter) => parameter,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            source.push_str(&parameter)?;
        }
        source.push_str("\n) {\n")?;
        if arguments.actual_arg_count != 0 {
            let body_index = arguments.actual_arg_count - 1;
            let body = match self
                .native_to_dynamic_source_fragment(realm, &arguments.readable[body_index])?
            {
                NativeConversion::Value(body) => body,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            source.push_str(&body)?;
        }
        source.push_str("\n})")?;
        let source = source.finish()?;

        let script = match self.compile_in_realm(realm, &source, DEFAULT_EVAL_FILENAME, false)? {
            Compilation::Published(script) => script,
            Compilation::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let script_callable = self.new_bytecode_closure(realm, &script)?;
        let global_object = {
            let global_object = self.0.state.borrow().heap.context(realm)?.global_object;
            ObjectRef::from_borrowed_handle(self.clone(), global_object)?
        };
        let value =
            match self.call_internal(realm, &script_callable, Value::Object(global_object), &[])? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };

        if matches!(new_target, Value::Undefined) {
            return Ok(Completion::Return(value));
        }
        let Value::Object(new_target) = new_target else {
            return Err(RuntimeError::Invariant(
                "Function constructor new.target was neither undefined nor an object",
            ));
        };
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype = match self.get_property_in_realm(realm, &new_target, &prototype_key)? {
            Completion::Return(Value::Object(prototype)) => prototype,
            Completion::Return(_) => {
                let new_target = self.callable_from_value(Value::Object(new_target))?;
                let fallback_realm = self.callable_realm(&new_target)?;
                let prototype = match kind {
                    DynamicFunctionKind::Normal => {
                        self.0
                            .state
                            .borrow()
                            .heap
                            .context(fallback_realm)?
                            .function_prototype
                    }
                    DynamicFunctionKind::Generator
                    | DynamicFunctionKind::Async
                    | DynamicFunctionKind::AsyncGenerator => {
                        return Err(RuntimeError::Invariant(
                            "dynamic Function kind has no intrinsic prototype yet",
                        ));
                    }
                };
                ObjectRef::from_borrowed_handle(self.clone(), prototype)?
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let Value::Object(function) = value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        if !self.set_prototype_of(&function, Some(&prototype))? {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "prototype is immutable",
            )?));
        }
        Ok(Completion::Return(Value::Object(function)))
    }

    fn call_function_prototype_apply(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.apply did not receive a generic invocation",
            ));
        };
        let Value::Object(target) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(target) = self.as_callable(&target)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };

        let this_argument = arguments.readable[0].clone();
        let array_argument = &arguments.readable[1];
        if matches!(array_argument, Value::Undefined | Value::Null) {
            return self.call_internal(realm, &target, this_argument, &[]);
        }
        let forwarded = match self.build_array_like_argument_list(realm, array_argument)? {
            NativeConversion::Value(values) => values,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.call_internal(realm, &target, this_argument, &forwarded)
    }

    fn call_function_prototype_bind(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.bind did not receive a generic invocation",
            ));
        };
        let Value::Object(target_object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(target) = self.as_callable(&target_object)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };

        let bound_argument_count = arguments.actual_arg_count.saturating_sub(1);
        let bound_arguments = if arguments.actual_arg_count > 1 {
            &arguments.readable[1..arguments.actual_arg_count]
        } else {
            &[]
        };
        let bound =
            self.new_bound_function(realm, &target, &arguments.readable[0], bound_arguments)?;

        let length_key = self.intern_property_key("length")?;
        let length = if self.has_own_property(&target_object, &length_key)? {
            let value = match self.get_property_in_realm(realm, &target_object, &length_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            bound_function_length(&value, bound_argument_count)?
        } else {
            Value::Int(0)
        };
        self.define_function_data_property(bound.as_object(), "length", length, false, true)?;

        let name_key = self.intern_property_key("name")?;
        let name = match self.get_property_in_realm(realm, &target_object, &name_key)? {
            Completion::Return(Value::String(name)) => name,
            Completion::Return(_) => JsString::from_static(""),
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let name = JsString::from_static("bound ").try_concat(&name)?;
        self.define_function_data_property(
            bound.as_object(),
            "name",
            Value::String(name),
            false,
            true,
        )?;
        Ok(Completion::Return(Value::Object(bound.into_object())))
    }

    fn call_function_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.toString did not receive a generic invocation",
            ));
        };
        let Value::Object(function) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };

        let (is_callable, source, function_kind) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(function.object_id())?;
            match &object.payload {
                ObjectPayload::BytecodeFunction { bytecode, .. } => {
                    let bytecode = state.heap.function_bytecode(*bytecode)?;
                    (
                        true,
                        bytecode
                            .debug
                            .as_ref()
                            .and_then(|debug| debug.source.clone()),
                        bytecode.metadata.function_kind,
                    )
                }
                ObjectPayload::NativeFunction { .. } | ObjectPayload::BoundFunction { .. } => {
                    (true, None, FunctionKind::Normal)
                }
                ObjectPayload::Ordinary
                | ObjectPayload::Date(_)
                | ObjectPayload::RegExp(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. } => (false, None, FunctionKind::Normal),
            }
        };
        if !is_callable {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        }
        if let Some(source) = source {
            let source = std::str::from_utf8(&source).map_err(|_| {
                RuntimeError::Invariant("published function source was not valid UTF-8")
            })?;
            return Ok(Completion::Return(Value::String(JsString::try_from_utf8(
                source,
            )?)));
        }

        let name_key = self.intern_property_key("name")?;
        let name = match self.get_property_in_realm(realm, &function, &name_key)? {
            Completion::Return(Value::Undefined) => JsString::from_static(""),
            Completion::Return(value) => match self.native_to_js_string(realm, &value)? {
                NativeConversion::Value(name) => name,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let prefix = match function_kind {
            FunctionKind::Normal => "function ",
            FunctionKind::Generator => "function *",
            FunctionKind::Async => "async function ",
            FunctionKind::AsyncGenerator => "async function *",
        };
        let source = JsString::from_static(prefix)
            .try_concat(&name)?
            .try_concat(&JsString::from_static("() {\n    [native code]\n}"))?;
        Ok(Completion::Return(Value::String(source)))
    }

    fn call_function_prototype_file_name(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.fileName getter received the wrong native invocation",
            ));
        };
        let Value::Object(function) = this_value else {
            return Ok(Completion::Return(Value::Undefined));
        };
        let filename = {
            let state = self.0.state.borrow();
            let object = state.heap.object(function.object_id())?;
            let ObjectPayload::BytecodeFunction { bytecode, .. } = &object.payload else {
                return Ok(Completion::Return(Value::Undefined));
            };
            let bytecode = state.heap.function_bytecode(*bytecode)?;
            bytecode
                .debug
                .as_ref()
                .map(|debug| state.atoms.to_js_string(debug.filename))
                .transpose()?
        };
        Ok(Completion::Return(
            filename.map_or(Value::Undefined, Value::String),
        ))
    }

    fn call_function_prototype_position(
        &self,
        invocation: NativeInvocation,
        selector: FunctionDebugPosition,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype position getter received the wrong native invocation",
            ));
        };
        let Value::Object(function) = this_value else {
            return Ok(Completion::Return(Value::Undefined));
        };
        let position = {
            let state = self.0.state.borrow();
            let object = state.heap.object(function.object_id())?;
            let ObjectPayload::BytecodeFunction { bytecode, .. } = &object.payload else {
                return Ok(Completion::Return(Value::Undefined));
            };
            let bytecode = state.heap.function_bytecode(*bytecode)?;
            bytecode
                .debug
                .as_ref()
                .map(|debug| debug.pc2line.as_ref().map(|table| table.lookup(None)))
        };
        let Some(position) = position else {
            return Ok(Completion::Return(Value::Undefined));
        };
        let Some(position) = position else {
            return Ok(Completion::Return(Value::Int(0)));
        };
        let (line, column) = position.one_based().ok_or(RuntimeError::Invariant(
            "function definition position cannot be represented one-based",
        ))?;
        let selected = match selector {
            FunctionDebugPosition::Line => line,
            FunctionDebugPosition::Column => column,
        };
        let selected = i32::try_from(selected).map_err(|_| {
            RuntimeError::Invariant("function definition position does not fit Int32")
        })?;
        Ok(Completion::Return(Value::Int(selected)))
    }

    /// QuickJS `JS_IsInstanceOf`: observe `@@hasInstance` before the legacy
    /// callable fallback, call a custom method with the RHS as receiver, and
    /// preserve arbitrary thrown values as completions.
    fn is_instance_of(
        &self,
        realm: ContextId,
        candidate: Value,
        target: ObjectRef,
    ) -> Result<Completion, RuntimeError> {
        let has_instance = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::HasInstance));
        let method = match self.get_property_in_realm(realm, &target, &has_instance)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if !matches!(method, Value::Undefined | Value::Null) {
            let method = self.callable_from_value(method)?;
            return Ok(
                match self.call_internal(
                    realm,
                    &method,
                    Value::Object(target),
                    std::slice::from_ref(&candidate),
                )? {
                    Completion::Return(value) => {
                        Completion::Return(Value::Bool(value.to_boolean()))
                    }
                    Completion::Throw(value) => Completion::Throw(value),
                },
            );
        }

        let Some(target) = self.as_callable(&target)? else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "invalid 'instanceof' right operand",
            )));
        };
        self.ordinary_is_instance_of(realm, &target, candidate)
    }

    fn call_function_prototype_has_instance(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype[Symbol.hasInstance] did not receive a generic invocation",
            ));
        };
        let Value::Object(target) = this_value else {
            return Ok(Completion::Return(Value::Bool(false)));
        };
        let Some(_target_callable) = self.as_callable(&target)? else {
            return Ok(Completion::Return(Value::Bool(false)));
        };
        let target = CallableRef::from_validated_object(target);
        self.ordinary_is_instance_of(
            realm,
            &target,
            arguments
                .readable
                .first()
                .cloned()
                .unwrap_or(Value::Undefined),
        )
    }

    fn ordinary_is_instance_of(
        &self,
        mut realm: ContextId,
        target: &CallableRef,
        candidate: Value,
    ) -> Result<Completion, RuntimeError> {
        let mut target = target.clone();
        let mut delegated_standard_frames = Vec::new();
        let has_instance = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::HasInstance));
        let result = (|| -> Result<Completion, RuntimeError> {
            loop {
                let bound_target = {
                    let state = self.0.state.borrow();
                    let object = state.heap.object(target.as_object().object_id())?;
                    match &object.payload {
                        ObjectPayload::BoundFunction { target, .. } => Some(*target),
                        ObjectPayload::NativeFunction { .. }
                        | ObjectPayload::BytecodeFunction { .. } => None,
                        ObjectPayload::Ordinary
                        | ObjectPayload::Date(_)
                        | ObjectPayload::RegExp(_)
                        | ObjectPayload::Array { .. }
                        | ObjectPayload::Arguments { .. }
                        | ObjectPayload::ArrayIterator { .. }
                        | ObjectPayload::ForInIterator(_)
                        | ObjectPayload::Primitive(_)
                        | ObjectPayload::GlobalObject { .. }
                        | ObjectPayload::Error
                        | ObjectPayload::StringIterator { .. }
                        | ObjectPayload::RegExpStringIterator { .. } => {
                            return Err(RuntimeError::Invariant(
                                "ordinary instanceof received a non-callable target",
                            ));
                        }
                    }
                };

                let Some(bound_target) = bound_target else {
                    let Value::Object(candidate) = &candidate else {
                        return Ok(Completion::Return(Value::Bool(false)));
                    };
                    let prototype_key = self.intern_property_key("prototype")?;
                    let prototype = match self.get_property_in_realm(
                        realm,
                        target.as_object(),
                        &prototype_key,
                    )? {
                        Completion::Return(Value::Object(prototype)) => prototype,
                        Completion::Return(_) => {
                            return Ok(Completion::Throw(self.new_native_error(
                                realm,
                                NativeErrorKind::Type,
                                "operand 'prototype' property is not an object",
                            )?));
                        }
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };

                    let mut cursor = self.get_prototype_of(candidate)?;
                    while let Some(current) = cursor {
                        if current == prototype {
                            return Ok(Completion::Return(Value::Bool(true)));
                        }
                        cursor = self.get_prototype_of(&current)?;
                    }
                    return Ok(Completion::Return(Value::Bool(false)));
                };

                // QuickJS bound OrdinaryHasInstance delegates through the full
                // JS_IsInstanceOf path. Perform every observable GetMethod in
                // order, but trampoline direct calls to the inherited standard
                // method so a deep bound chain cannot recurse through Rust's
                // host stack. Synthetic native frames preserve backtraces.
                let target_object = ObjectRef::from_borrowed_handle(self.clone(), bound_target)?;
                let method =
                    match self.get_property_in_realm(realm, &target_object, &has_instance)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                if matches!(method, Value::Undefined | Value::Null) {
                    let Some(next_target) = self.as_callable(&target_object)? else {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "invalid 'instanceof' right operand",
                        )?));
                    };
                    target = next_target;
                    continue;
                }

                let Value::Object(method_object) = method else {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a function",
                    )?));
                };
                let Some(method) = self.as_callable(&method_object)? else {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a function",
                    )?));
                };
                let standard_method = {
                    let state = self.0.state.borrow();
                    let object = state.heap.object(method.as_object().object_id())?;
                    match &object.payload {
                        ObjectPayload::NativeFunction { data }
                            if data.target == NativeFunctionId::FunctionPrototypeHasInstance =>
                        {
                            Some((
                                data.realm.ok_or(RuntimeError::Invariant(
                                    "standard hasInstance method has no defining realm",
                                ))?,
                                data.min_readable_args,
                            ))
                        }
                        ObjectPayload::Ordinary
                        | ObjectPayload::Date(_)
                        | ObjectPayload::RegExp(_)
                        | ObjectPayload::Array { .. }
                        | ObjectPayload::Arguments { .. }
                        | ObjectPayload::ArrayIterator { .. }
                        | ObjectPayload::ForInIterator(_)
                        | ObjectPayload::Primitive(_)
                        | ObjectPayload::GlobalObject { .. }
                        | ObjectPayload::Error
                        | ObjectPayload::StringIterator { .. }
                        | ObjectPayload::RegExpStringIterator { .. }
                        | ObjectPayload::NativeFunction { .. }
                        | ObjectPayload::BoundFunction { .. }
                        | ObjectPayload::BytecodeFunction { .. } => None,
                    }
                };
                if let Some((method_realm, min_readable_args)) = standard_method {
                    let readable_arg_count = 1usize.max(usize::from(min_readable_args));
                    delegated_standard_frames.push(self.push_native_active_frame(
                        method.as_object().clone(),
                        method_realm,
                        NativeFunctionId::FunctionPrototypeHasInstance,
                        1,
                        readable_arg_count,
                    )?);
                    let Some(next_target) = self.as_callable(&target_object)? else {
                        return Ok(Completion::Return(Value::Bool(false)));
                    };
                    realm = method_realm;
                    target = next_target;
                    continue;
                }

                return Ok(
                    match self.call_internal(
                        realm,
                        &method,
                        Value::Object(target_object),
                        std::slice::from_ref(&candidate),
                    )? {
                        Completion::Return(value) => {
                            Completion::Return(Value::Bool(value.to_boolean()))
                        }
                        Completion::Throw(value) => Completion::Throw(value),
                    },
                );
            }
        })();

        let mut frame_error = None;
        while let Some(frame) = delegated_standard_frames.pop() {
            if let Err(error) = frame.finish() {
                if frame_error.is_none() {
                    frame_error = Some(error);
                }
            }
        }
        if let Some(error) = frame_error {
            return Err(error);
        }
        result
    }

    fn native_to_object(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let (kind, value) = match value {
            Value::Object(object) => return Ok(NativeConversion::Value(object)),
            Value::Undefined | Value::Null => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "cannot convert to object",
                )?));
            }
            value @ Value::Bool(_) => (PrimitiveKind::Boolean, value),
            value @ (Value::Int(_) | Value::Float(_)) => (PrimitiveKind::Number, value),
            value @ Value::String(_) => (PrimitiveKind::String, value),
            value @ Value::BigInt(_) => (PrimitiveKind::BigInt, value),
            value @ Value::Symbol(_) => (PrimitiveKind::Symbol, value),
        };
        let prototype = self.primitive_prototype_for_realm(realm, kind)?;
        Ok(NativeConversion::Value(
            self.new_primitive_object(&prototype, kind, value)?,
        ))
    }

    fn call_primitive_constructor(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "primitive constructor readable argv was not padded to one",
        ))?;
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "primitive constructor did not receive constructor-or-function invocation",
            ));
        };
        if kind == PrimitiveKind::Symbol {
            // Like QuickJS's constructor-or-function C entry, Symbol keeps its
            // constructor bit but rejects a real new.target before ToString.
            if !matches!(new_target, Value::Undefined) {
                return Ok(Completion::Throw(
                    self.new_not_constructor_error(realm, &new_target)?,
                ));
            }
            let description =
                if arguments.actual_arg_count == 0 || matches!(argument, Value::Undefined) {
                    None
                } else {
                    match self.native_to_js_string(realm, argument)? {
                        NativeConversion::Value(value) => Some(value),
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                };
            return Ok(Completion::Return(Value::Symbol(
                self.new_symbol(description)?,
            )));
        }
        if kind == PrimitiveKind::BigInt {
            // BigInt deliberately keeps QuickJS's constructor-or-function
            // cproto bit, but its body rejects any real new.target before
            // touching the argument.
            if !matches!(new_target, Value::Undefined) {
                return Ok(Completion::Throw(
                    self.new_not_constructor_error(realm, &new_target)?,
                ));
            }
            let value = match self.native_to_bigint_constructor_value(realm, argument)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            return Ok(Completion::Return(Value::BigInt(value)));
        }
        let value = match kind {
            PrimitiveKind::Boolean => Value::Bool(argument.to_boolean()),
            PrimitiveKind::Number if arguments.actual_arg_count == 0 => Value::Int(0),
            PrimitiveKind::Number => {
                let value = match self.native_to_number_constructor_value(realm, argument)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                Value::number(value)
            }
            PrimitiveKind::String if arguments.actual_arg_count == 0 => {
                Value::String(JsString::from_static(""))
            }
            PrimitiveKind::String => {
                let value = if matches!(new_target, Value::Undefined)
                    && let Value::Symbol(symbol) = argument
                {
                    self.symbol_descriptive_string(symbol)?
                } else {
                    match self.native_to_js_string(realm, argument)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                };
                Value::String(value)
            }
            PrimitiveKind::Symbol | PrimitiveKind::BigInt => {
                return Err(RuntimeError::Invariant(
                    "unimplemented primitive constructor reached native dispatch",
                ));
            }
        };
        if matches!(new_target, Value::Undefined) {
            return Ok(Completion::Return(value));
        }
        let Value::Object(new_target) = new_target else {
            return Err(RuntimeError::Invariant(
                "primitive constructor new.target was neither undefined nor an object",
            ));
        };
        let new_target = self.callable_from_value(Value::Object(new_target))?;
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype =
            match self.get_property_in_realm(realm, new_target.as_object(), &prototype_key)? {
                Completion::Return(Value::Object(prototype)) => prototype,
                Completion::Return(_) => {
                    let fallback_realm = self.callable_realm(&new_target)?;
                    self.primitive_prototype_for_realm(fallback_realm, kind)?
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        Ok(Completion::Return(Value::Object(
            self.new_primitive_object(&prototype, kind, value)?,
        )))
    }

    fn new_not_constructor_error(
        &self,
        realm: ContextId,
        target: &Value,
    ) -> Result<Value, RuntimeError> {
        let name = if let Value::Object(object) = target
            && self.as_callable(object)?.is_some()
        {
            let name = self.intern_property_key("name")?;
            raw_string_property_one_level(&self.0.state.borrow(), object.object_id(), name.atom())?
                .filter(JsString::is_flat)
        } else {
            None
        };
        let mut message = NativeErrorMessage::new();
        if let Some(name) = name {
            name.push_c_string_to(&mut message);
            message.push_utf8(" is not a constructor");
        } else {
            message.push_utf8("not a constructor");
        }
        self.new_native_error_from_message(realm, NativeErrorKind::Type, message)
    }

    fn call_global_number_parse(
        &self,
        realm: ContextId,
        kind: NumberParseKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "global numeric parser did not receive a generic call",
            ));
        };
        let input = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "global numeric parser argv was not padded",
        ))?;
        let input = match self.native_to_js_string(realm, input)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = match kind {
            NumberParseKind::ParseFloat => crate::number_parse::parse_float(&input),
            NumberParseKind::ParseInt => {
                let radix = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                    "parseInt radix argv was not padded",
                ))?;
                let radix = match self.native_to_number(realm, radix)? {
                    NativeConversion::Value(value) => crate::number::to_int32(value),
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                crate::number_parse::parse_int(&input, radix)
            }
        };
        Ok(Completion::Return(Value::number(result)))
    }

    fn call_global_number_predicate(
        &self,
        realm: ContextId,
        kind: GlobalNumberPredicateKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "global numeric predicate did not receive a generic call",
            ));
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "global numeric predicate argv was not padded",
        ))?;
        let number = match self.native_to_number(realm, argument)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = match kind {
            GlobalNumberPredicateKind::IsNaN => number.is_nan(),
            GlobalNumberPredicateKind::IsFinite => number.is_finite(),
        };
        Ok(Completion::Return(Value::Bool(result)))
    }

    fn call_global_uri_codec(
        &self,
        realm: ContextId,
        kind: GlobalUriCodecKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "global URI codec did not receive a generic call",
            ));
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "global URI codec argv was not padded",
        ))?;
        let input = match self.native_to_js_string(realm, argument)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = match kind {
            GlobalUriCodecKind::DecodeUri => crate::uri::decode(&input, false),
            GlobalUriCodecKind::DecodeUriComponent => crate::uri::decode(&input, true),
            GlobalUriCodecKind::EncodeUri => crate::uri::encode(&input, false),
            GlobalUriCodecKind::EncodeUriComponent => crate::uri::encode(&input, true),
            GlobalUriCodecKind::Escape => crate::uri::escape(&input),
            GlobalUriCodecKind::Unescape => crate::uri::unescape(&input),
        };
        match result {
            Ok(value) => Ok(Completion::Return(Value::String(value))),
            Err(crate::uri::UriCodecError::String(error)) => Err(error.into()),
            Err(error) => Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Uri,
                error.message(),
            )?)),
        }
    }

    fn primitive_this_value(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        this_value: Value,
    ) -> Result<NativeConversion<Value>, RuntimeError> {
        let direct = matches!(
            (&this_value, kind),
            (Value::Int(_) | Value::Float(_), PrimitiveKind::Number)
                | (Value::String(_), PrimitiveKind::String)
                | (Value::Bool(_), PrimitiveKind::Boolean)
                | (Value::Symbol(_), PrimitiveKind::Symbol)
                | (Value::BigInt(_), PrimitiveKind::BigInt)
        );
        if direct {
            return Ok(NativeConversion::Value(this_value));
        }
        if let Value::Object(object) = &this_value {
            let payload = {
                let state = self.0.state.borrow();
                match &state.heap.object(object.object_id())?.payload {
                    ObjectPayload::Primitive(PrimitiveObjectData::Number(value))
                        if kind == PrimitiveKind::Number =>
                    {
                        Some(Ok(Value::number(*value)))
                    }
                    ObjectPayload::Primitive(PrimitiveObjectData::String(value))
                        if kind == PrimitiveKind::String =>
                    {
                        Some(Ok(Value::String(value.clone())))
                    }
                    ObjectPayload::Primitive(PrimitiveObjectData::Boolean(value))
                        if kind == PrimitiveKind::Boolean =>
                    {
                        Some(Ok(Value::Bool(*value)))
                    }
                    ObjectPayload::Primitive(PrimitiveObjectData::Symbol(atom))
                        if kind == PrimitiveKind::Symbol =>
                    {
                        // Promote the wrapper's raw owning atom only after the
                        // immutable heap borrow above has ended.
                        Some(Err(*atom))
                    }
                    ObjectPayload::Primitive(PrimitiveObjectData::BigInt(value))
                        if kind == PrimitiveKind::BigInt =>
                    {
                        Some(Ok(Value::BigInt(value.clone())))
                    }
                    ObjectPayload::Ordinary
                    | ObjectPayload::Date(_)
                    | ObjectPayload::RegExp(_)
                    | ObjectPayload::Array { .. }
                    | ObjectPayload::Arguments { .. }
                    | ObjectPayload::ArrayIterator { .. }
                    | ObjectPayload::ForInIterator(_)
                    | ObjectPayload::Primitive(_)
                    | ObjectPayload::GlobalObject { .. }
                    | ObjectPayload::Error
                    | ObjectPayload::StringIterator { .. }
                    | ObjectPayload::RegExpStringIterator { .. }
                    | ObjectPayload::NativeFunction { .. }
                    | ObjectPayload::BoundFunction { .. }
                    | ObjectPayload::BytecodeFunction { .. } => None,
                }
            };
            if let Some(payload) = payload {
                let payload = match payload {
                    Ok(value) => value,
                    Err(atom) => Value::Symbol(SymbolRef::from_borrowed_atom(self.clone(), atom)?),
                };
                return Ok(NativeConversion::Value(payload));
            }
        }
        let message = match kind {
            PrimitiveKind::Number => "not a number",
            PrimitiveKind::String => "not a string",
            PrimitiveKind::Boolean => "not a boolean",
            PrimitiveKind::Symbol => "not a symbol",
            PrimitiveKind::BigInt => "not a BigInt",
        };
        Ok(NativeConversion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            message,
        )?))
    }

    fn call_string_prototype_char_at(
        &self,
        realm: ContextId,
        selector: StringCharAtKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String character method did not receive a generic invocation",
            ));
        };
        let string = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String character method argv was not padded",
        ))?;
        let mut index = match self.native_to_number(realm, argument)? {
            NativeConversion::Value(value) => crate::number::to_int32_sat(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = i32::try_from(string.len()).map_err(|_| {
            RuntimeError::Invariant("String length exceeded QuickJS's signed index range")
        })?;
        if selector == StringCharAtKind::At && index < 0 {
            index += length;
        }
        if index < 0 || index >= length {
            return Ok(Completion::Return(match selector {
                StringCharAtKind::At => Value::Undefined,
                StringCharAtKind::CharAt => Value::String(JsString::from_static("")),
            }));
        }
        let index =
            usize::try_from(index).expect("validated non-negative String index always fits usize");
        let unit = string.code_unit_at(index).ok_or(RuntimeError::Invariant(
            "validated String character index did not name a code unit",
        ))?;
        Ok(Completion::Return(Value::String(JsString::from_code_unit(
            unit,
        ))))
    }

    fn call_iterator_prototype_iterator(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype iterator did not receive a generic invocation",
            ));
        };
        Ok(Completion::Return(this_value))
    }

    fn call_iterator_prototype_to_string_tag_getter(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype toStringTag getter received the wrong native invocation",
            ));
        };
        Ok(Completion::Return(Value::String(JsString::from_static(
            "Iterator",
        ))))
    }

    fn call_iterator_prototype_to_string_tag_setter(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Setter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype toStringTag setter received the wrong native invocation",
            ));
        };
        let Value::Object(receiver) = this_value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not an object",
            )));
        };
        let value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Iterator.prototype toStringTag setter argv was not padded",
            ))?;
        let iterator_prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .iterator_prototype;
        if receiver.object_id() == iterator_prototype {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "Cannot assign to read only property",
            )));
        }

        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.has_own_property(&receiver, &key)? {
            let defined = self.define_own_property(
                &receiver,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !defined {
                return Err(RuntimeError::Engine(Error::new(
                    ErrorKind::Type,
                    "object is not extensible",
                )));
            }
            return Ok(Completion::Return(Value::Undefined));
        }

        match self.prepare_set_property_in_realm(realm, &receiver, &key, value)? {
            PropertySetAction::Complete => Ok(Completion::Return(Value::Undefined)),
            PropertySetAction::Throw(value) => Ok(Completion::Throw(value)),
            PropertySetAction::Call {
                setter,
                receiver,
                argument,
            } => match self.call_internal(realm, &setter, receiver, &[argument])? {
                Completion::Return(_) => Ok(Completion::Return(Value::Undefined)),
                Completion::Throw(value) => Ok(Completion::Throw(value)),
            },
            PropertySetAction::Rejected(PropertySetRejection::ReadOnly) => {
                Err(RuntimeError::Engine(self.native_atom_error(
                    ErrorKind::Type,
                    "'",
                    &key,
                    "' is read-only",
                )?))
            }
            PropertySetAction::Rejected(PropertySetRejection::ArrayLengthReadOnly) => {
                let length = self.intern_property_key("length")?;
                Err(RuntimeError::Engine(self.native_atom_error(
                    ErrorKind::Type,
                    "'",
                    &length,
                    "' is read-only",
                )?))
            }
            PropertySetAction::Rejected(PropertySetRejection::NotConfigurable) => Err(
                RuntimeError::Engine(Error::new(ErrorKind::Type, "not configurable")),
            ),
            PropertySetAction::Rejected(PropertySetRejection::NoSetter) => Err(
                RuntimeError::Engine(Error::new(ErrorKind::Type, "no setter for property")),
            ),
            PropertySetAction::Rejected(PropertySetRejection::NotExtensible) => Err(
                RuntimeError::Engine(Error::new(ErrorKind::Type, "object is not extensible")),
            ),
            PropertySetAction::Rejected(PropertySetRejection::NotObject) => {
                Err(RuntimeError::Invariant(
                    "Iterator.prototype tag setter rejected its object receiver",
                ))
            }
        }
    }

    fn call_string_prototype_iterator(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String.prototype iterator did not receive a generic invocation",
            ));
        };
        let string = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Object(
            self.new_string_iterator(realm, string)?,
        )))
    }

    fn call_string_iterator_next(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        match self.call_string_iterator_next_raw(realm, invocation)? {
            NativeInvokeOutcome::Completion(completion) => Ok(completion),
            NativeInvokeOutcome::IteratorNextRaw { value, done } => Ok(Completion::Return(
                Value::Object(self.new_iterator_result(realm, value, done)?),
            )),
        }
    }

    /// Execute the QuickJS `JS_CFUNC_iterator_next` half of String Iterator
    /// without materializing the public iterator-result object. The ordinary
    /// JavaScript call adapter above wraps this outcome; the VM's direct-native
    /// `ForOfNext` path consumes it as-is.
    fn call_string_iterator_next_raw(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String Iterator next did not receive an iterator-next invocation",
            ));
        };
        let Value::Object(iterator) = this_value else {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "String Iterator object expected",
                )?,
            )));
        };
        let branded = matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(iterator.object_id())?
                .payload,
            ObjectPayload::StringIterator { .. }
        );
        if !branded {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "String Iterator object expected",
                )?,
            )));
        }
        let value = self
            .0
            .state
            .borrow_mut()
            .heap
            .string_iterator_next(iterator.object_id())?;
        let (value, done) = match value {
            Some(value) => (Value::String(value), false),
            None => (Value::Undefined, true),
        };
        Ok(NativeInvokeOutcome::IteratorNextRaw { value, done })
    }

    fn call_string_prototype_char_code_at(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String charCodeAt did not receive a generic invocation",
            ));
        };
        let string = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String charCodeAt argv was not padded",
        ))?;
        let index = match self.native_to_number(realm, argument)? {
            NativeConversion::Value(value) => crate::number::to_int32_sat(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let Some(unit) = usize::try_from(index)
            .ok()
            .and_then(|index| string.code_unit_at(index))
        else {
            return Ok(Completion::Return(Value::Float(f64::NAN)));
        };
        Ok(Completion::Return(Value::Int(i32::from(unit))))
    }

    fn call_string_prototype_code_point_at(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String codePointAt did not receive a generic invocation",
            ));
        };
        let string = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String codePointAt argv was not padded",
        ))?;
        let index = match self.native_to_number(realm, argument)? {
            NativeConversion::Value(value) => crate::number::to_int32_sat(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let Some(code_point) = usize::try_from(index)
            .ok()
            .and_then(|index| string.code_point_at(index))
        else {
            return Ok(Completion::Return(Value::Undefined));
        };
        Ok(Completion::Return(Value::Int(
            i32::try_from(code_point).expect("a Unicode code point always fits i32"),
        )))
    }

    fn call_string_prototype_concat(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String concat did not receive a generic invocation",
            ));
        };
        let receiver = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if arguments.actual_arg_count == 0 {
            return Ok(Completion::Return(Value::String(receiver)));
        }

        let mut result = receiver;
        for argument in &arguments.readable[..arguments.actual_arg_count] {
            let chunk = match argument {
                // QuickJS `JS_ConcatString` accepts an existing rope without
                // routing it back through `JS_ToString`/linearization.
                Value::String(value) => value.clone(),
                _ => match self.native_to_js_string(realm, argument)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                },
            };
            result = result.try_concat(&chunk).map_err(Error::from)?;
        }
        Ok(Completion::Return(Value::String(result)))
    }

    fn call_string_prototype_well_formed(
        &self,
        realm: ContextId,
        selector: StringWellFormedKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String well-formed method did not receive a generic invocation",
            ));
        };
        let string = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(match selector {
            StringWellFormedKind::IsWellFormed => Value::Bool(string.is_well_formed()),
            StringWellFormedKind::ToWellFormed => Value::String(string.to_well_formed()),
        }))
    }

    fn call_primitive_prototype_to_string(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "primitive toString did not receive a generic invocation",
            ));
        };
        let value = match self.primitive_this_value(realm, kind, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        match (kind, value) {
            (PrimitiveKind::Number, value @ (Value::Int(_) | Value::Float(_))) => {
                let number = value.as_number().ok_or(RuntimeError::Invariant(
                    "Number brand extraction did not return a Number",
                ))?;
                let radix_argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "Number.prototype.toString argv was not padded",
                ))?;
                let radix = if matches!(radix_argument, Value::Undefined) {
                    10
                } else {
                    let radix = match self.native_to_number(realm, radix_argument)? {
                        NativeConversion::Value(value) => crate::number::to_int32_sat(value),
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if !(2..=36).contains(&radix) {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Range,
                            "radix must be between 2 and 36",
                        )?));
                    }
                    u32::try_from(radix).expect("a Number radix between 2 and 36 always fits u32")
                };
                let formatted =
                    crate::number::to_string_radix(number, radix).map_err(|error| match error {
                        crate::number::NumberFormatError::InvalidRadix => RuntimeError::Invariant(
                            "validated Number radix was rejected by the formatter",
                        ),
                        crate::number::NumberFormatError::InvalidDigits => RuntimeError::Invariant(
                            "Number radix formatting reported a digit-count error",
                        ),
                    })?;
                Ok(Completion::Return(Value::String(JsString::try_from_utf8(
                    &formatted,
                )?)))
            }
            (PrimitiveKind::String, Value::String(value)) => {
                Ok(Completion::Return(Value::String(value)))
            }
            (PrimitiveKind::Boolean, Value::Bool(value)) => Ok(Completion::Return(Value::String(
                JsString::from_static(if value { "true" } else { "false" }),
            ))),
            (PrimitiveKind::Symbol, Value::Symbol(value)) => Ok(Completion::Return(Value::String(
                self.symbol_descriptive_string(&value)?,
            ))),
            (PrimitiveKind::BigInt, Value::BigInt(value)) => {
                let radix_argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "BigInt.prototype.toString argv was not padded",
                ))?;
                let radix = if matches!(radix_argument, Value::Undefined) {
                    10
                } else {
                    let radix = match self.native_to_number(realm, radix_argument)? {
                        NativeConversion::Value(value) => crate::number::to_int32_sat(value),
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if !(2..=36).contains(&radix) {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Range,
                            "radix must be between 2 and 36",
                        )?));
                    }
                    u32::try_from(radix).expect("a BigInt radix between 2 and 36 fits u32")
                };
                if value.exceeds_allocation_limit()
                    && (value.is_negative() || !radix.is_power_of_two())
                {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Range,
                        "BigInt is too large to allocate",
                    )?));
                }
                let text = value
                    .to_string_radix(radix)
                    .map_err(|_| RuntimeError::Invariant("validated BigInt radix was rejected"))?;
                Ok(Completion::Return(Value::String(JsString::try_from_utf8(
                    &text,
                )?)))
            }
            _ => Err(RuntimeError::Invariant(
                "unimplemented primitive toString reached native dispatch",
            )),
        }
    }

    fn finish_number_format(
        &self,
        realm: ContextId,
        result: Result<String, crate::number::NumberFormatError>,
    ) -> Result<Completion, RuntimeError> {
        match result {
            Ok(value) => Ok(Completion::Return(Value::String(JsString::try_from_utf8(
                &value,
            )?))),
            Err(crate::number::NumberFormatError::InvalidDigits) => Ok(Completion::Throw(
                self.new_native_error(realm, NativeErrorKind::Range, "invalid number of digits")?,
            )),
            Err(crate::number::NumberFormatError::InvalidRadix) => {
                Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "radix must be between 2 and 36",
                )?))
            }
        }
    }

    fn call_number_prototype_format(
        &self,
        realm: ContextId,
        kind: NumberFormatKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Number prototype formatter did not receive a generic invocation",
            ));
        };
        // QuickJS performs the receiver brand check before touching any
        // argument, including user-code coercion on the digit/radix value.
        let value = match self.primitive_this_value(realm, PrimitiveKind::Number, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let number = value.as_number().ok_or(RuntimeError::Invariant(
            "Number formatter brand extraction did not return a Number",
        ))?;

        let result = match kind {
            NumberFormatKind::ToLocaleString => crate::number::to_string_radix(number, 10),
            NumberFormatKind::ToFixed => {
                let digits = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "Number.prototype.toFixed argv was not padded",
                ))?;
                let digits = match self.native_to_number(realm, digits)? {
                    NativeConversion::Value(value) => crate::number::to_int32_sat(value),
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                crate::number::to_fixed(number, digits)
            }
            NumberFormatKind::ToExponential => {
                let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "Number.prototype.toExponential argv was not padded",
                ))?;
                // The pinned C implementation runs ToInt32Sat even for
                // undefined, then records undefined as the FREE-format case.
                let converted = match self.native_to_number(realm, argument)? {
                    NativeConversion::Value(value) => crate::number::to_int32_sat(value),
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let digits = (!matches!(argument, Value::Undefined)).then_some(converted);
                crate::number::to_exponential(number, digits)
            }
            NumberFormatKind::ToPrecision => {
                let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "Number.prototype.toPrecision argv was not padded",
                ))?;
                let precision = if matches!(argument, Value::Undefined) {
                    None
                } else {
                    match self.native_to_number(realm, argument)? {
                        NativeConversion::Value(value) => Some(crate::number::to_int32_sat(value)),
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                };
                crate::number::to_precision(number, precision)
            }
        };
        self.finish_number_format(realm, result)
    }

    fn call_number_predicate(
        &self,
        kind: NumberPredicateKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Number predicate did not receive a generic invocation",
            ));
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Number predicate argv was not padded",
        ))?;
        let result = argument.as_number().is_some_and(|number| match kind {
            NumberPredicateKind::IsNaN => number.is_nan(),
            NumberPredicateKind::IsFinite => number.is_finite(),
            NumberPredicateKind::IsInteger => number.is_finite() && number.fract() == 0.0,
            NumberPredicateKind::IsSafeInteger => {
                number.is_finite()
                    && number.fract() == 0.0
                    && number.abs() <= 9_007_199_254_740_991.0
            }
        });
        Ok(Completion::Return(Value::Bool(result)))
    }

    fn call_bigint_as_n(
        &self,
        realm: ContextId,
        kind: BigIntAsNKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "BigInt truncation method did not receive a generic call",
            ));
        };
        let bits = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "BigInt truncation bits argument was not padded",
        ))?;
        let bits = match self.native_to_index(realm, bits)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let value = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "BigInt truncation value argument was not padded",
        ))?;
        let value = match self.native_to_bigint(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let value = match kind {
            BigIntAsNKind::AsUintN => value.as_uint_n(bits),
            BigIntAsNKind::AsIntN => value.as_int_n(bits),
        };
        match value {
            Ok(value) => Ok(Completion::Return(Value::BigInt(value))),
            Err(_) => Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "BigInt is too large to allocate",
            )?)),
        }
    }

    fn call_symbol_registry(
        &self,
        realm: ContextId,
        kind: SymbolRegistryKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Symbol registry method did not receive a generic call",
            ));
        };
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Symbol registry argv was not padded",
        ))?;
        match kind {
            SymbolRegistryKind::For => {
                let key = match self.native_to_js_string(realm, argument)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                Ok(Completion::Return(Value::Symbol(self.symbol_for(&key)?)))
            }
            SymbolRegistryKind::KeyFor => {
                let Value::Symbol(symbol) = argument else {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a symbol",
                    )?));
                };
                Ok(Completion::Return(
                    self.symbol_key_for(symbol)?
                        .map_or(Value::Undefined, Value::String),
                ))
            }
        }
    }

    fn call_symbol_prototype_description(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Symbol.prototype.description received the wrong native invocation",
            ));
        };
        let value = match self.primitive_this_value(realm, PrimitiveKind::Symbol, this_value)? {
            NativeConversion::Value(Value::Symbol(value)) => value,
            NativeConversion::Value(_) => {
                return Err(RuntimeError::Invariant(
                    "Symbol brand extraction did not return a Symbol",
                ));
            }
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(
            self.symbol_description(&value)?
                .map_or(Value::Undefined, Value::String),
        ))
    }

    fn call_primitive_prototype_value_of(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "primitive valueOf did not receive a generic invocation",
            ));
        };
        match self.primitive_this_value(realm, kind, this_value)? {
            NativeConversion::Value(value) => Ok(Completion::Return(value)),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    fn call_error_constructor(
        &self,
        realm: ContextId,
        kind: ErrorConstructorKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        if kind == ErrorConstructorKind::Native(NativeErrorKind::Aggregate) {
            return Err(RuntimeError::Engine(Error::internal(
                "AggregateError construction requires iterable and Array support",
            )));
        }
        let NativeInvocation::Construct { mut new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Error constructor did not receive constructor-or-function invocation",
            ));
        };
        if matches!(new_target, Value::Undefined) {
            new_target = Value::Object(self.active_function()?);
        }
        let Value::Object(new_target_object) = new_target else {
            return Err(RuntimeError::Invariant(
                "Error constructor new.target was neither undefined nor an object",
            ));
        };
        let new_target_callable =
            self.callable_from_value(Value::Object(new_target_object.clone()))?;
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype =
            match self.get_property_in_realm(realm, &new_target_object, &prototype_key)? {
                Completion::Return(Value::Object(prototype)) => prototype,
                Completion::Return(_) => {
                    let fallback_realm = self.callable_realm(&new_target_callable)?;
                    let prototype = {
                        let state = self.0.state.borrow();
                        let context = state.heap.context(fallback_realm)?;
                        match kind {
                            ErrorConstructorKind::Error => context
                                .error_prototype
                                .ok_or(RuntimeError::Invariant("realm has no Error prototype"))?,
                            ErrorConstructorKind::Native(kind) => {
                                context.native_error_prototypes[kind.index()].ok_or(
                                    RuntimeError::Invariant("realm has no native Error prototype"),
                                )?
                            }
                        }
                    };
                    ObjectRef::from_borrowed_handle(self.clone(), prototype)?
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let object = self.new_error_object(&prototype)?;

        let message = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Error constructor readable argv was not padded to length one",
        ))?;
        if !matches!(message, Value::Undefined) {
            let message = match self.native_to_js_string(realm, message)? {
                NativeConversion::Value(message) => message,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            self.define_function_data_property(
                &object,
                "message",
                Value::String(message),
                true,
                true,
            )?;
        }

        if arguments.actual_arg_count > 1 {
            if let Some(Value::Object(options)) = arguments.readable.get(1) {
                let cause_key = self.intern_property_key("cause")?;
                if self.has_property(options, &cause_key)? {
                    let cause = match self.get_property_in_realm(realm, options, &cause_key)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    self.define_function_data_property(&object, "cause", cause, true, true)?;
                }
            }
        }

        let value = Value::Object(object);
        // `js_error_constructor` eagerly snapshots the stack after message,
        // cause, and AggregateError payload work, skipping only its own top
        // native frame.
        self.ensure_error_backtrace(&value, true, None)?;
        Ok(Completion::Return(value))
    }

    fn call_error_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Error.prototype.toString did not receive a generic invocation",
            ));
        };
        let Value::Object(object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let name_key = self.intern_property_key("name")?;
        let name = match self.get_property_in_realm(realm, &object, &name_key)? {
            Completion::Return(Value::Undefined) => JsString::from_static("Error"),
            Completion::Return(value) => match self.native_to_js_string(realm, &value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let message_key = self.intern_property_key("message")?;
        let message = match self.get_property_in_realm(realm, &object, &message_key)? {
            Completion::Return(Value::Undefined) => JsString::from_static(""),
            Completion::Return(value) => match self.native_to_js_string(realm, &value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = if name.is_empty() {
            message
        } else if message.is_empty() {
            name
        } else {
            name.try_concat(&JsString::from_static(": "))?
                .try_concat(&message)?
        };
        Ok(Completion::Return(Value::String(result)))
    }

    fn call_error_is_error(&self, arguments: &NativeArguments) -> Result<Completion, RuntimeError> {
        let value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Error.isError readable argv was not padded to length one",
        ))?;
        let is_error = match value {
            Value::Object(object) => self.is_error_object(object)?,
            Value::Undefined
            | Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => false,
        };
        Ok(Completion::Return(Value::Bool(is_error)))
    }

    #[cfg(test)]
    fn call_active_frame_probe(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match arguments.readable.first() {
            Some(Value::Object(value))
                if matches!(arguments.readable.get(1), Some(Value::Bool(false))) =>
            {
                Ok(Completion::Throw(Value::Object(value.clone())))
            }
            Some(Value::Object(callback)) => {
                let callback = self.callable_from_value(Value::Object(callback.clone()))?;
                let active_function = self.active_function()?;
                self.call_internal(
                    realm,
                    &callback,
                    Value::Undefined,
                    &[Value::Object(active_function)],
                )
            }
            Some(Value::Bool(false)) => Ok(Completion::Throw(Value::String(
                JsString::from_static("active frame probe throw"),
            ))),
            Some(Value::Bool(true)) => {
                Err(RuntimeError::Invariant("active frame probe engine error"))
            }
            Some(_) => Err(RuntimeError::Invariant(
                "active frame probe received an unsupported command",
            )),
            None => {
                let snapshot = self.0.state.borrow().active_frames.clone();
                self.0
                    .state
                    .borrow_mut()
                    .active_frame_probe_snapshots
                    .push(snapshot);
                Ok(Completion::Return(Value::Undefined))
            }
        }
    }

    fn validate_object_and_key(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<(), RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        if !key.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("property key"));
        }
        Ok(())
    }

    fn validate_descriptor_domains(
        &self,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<(), RuntimeError> {
        if let DescriptorField::Present(value) = &descriptor.value {
            match value {
                Value::Object(object) if !object.belongs_to(self) => {
                    return Err(RuntimeError::WrongRuntime("descriptor value"));
                }
                Value::Symbol(symbol) if !symbol.belongs_to(self) => {
                    return Err(RuntimeError::WrongRuntime("descriptor value"));
                }
                _ => {}
            }
        }
        for accessor in [&descriptor.get, &descriptor.set] {
            if let DescriptorField::Present(AccessorValue::Callable(callable)) = accessor {
                if !callable.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("descriptor accessor"));
                }
            }
        }
        Ok(())
    }

    fn validate_value_domain(&self, value: &Value, role: &'static str) -> Result<(), RuntimeError> {
        match value {
            Value::Object(object) if !object.belongs_to(self) => {
                Err(RuntimeError::WrongRuntime(role))
            }
            Value::Symbol(symbol) if !symbol.belongs_to(self) => {
                Err(RuntimeError::WrongRuntime(role))
            }
            _ => Ok(()),
        }
    }

    fn root_raw_value(&self, value: &RawValue) -> Result<Value, RuntimeError> {
        Ok(match value {
            RawValue::Undefined => Value::Undefined,
            RawValue::Null => Value::Null,
            RawValue::Bool(value) => Value::Bool(*value),
            RawValue::Int(value) => Value::Int(*value),
            RawValue::Float(value) => Value::Float(*value),
            RawValue::BigInt(value) => Value::BigInt(value.clone()),
            RawValue::String(value) => Value::String(value.clone()),
            RawValue::Symbol(atom) => {
                Value::Symbol(SymbolRef::from_borrowed_atom(self.clone(), *atom)?)
            }
            RawValue::Object(object) => {
                Value::Object(ObjectRef::from_borrowed_handle(self.clone(), *object)?)
            }
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel escaped from an object property",
                ));
            }
        })
    }

    fn raw_property_value(&self, value: &Value) -> Result<RawValue, RuntimeError> {
        Ok(match value {
            Value::Undefined => RawValue::Undefined,
            Value::Null => RawValue::Null,
            Value::Bool(value) => RawValue::Bool(*value),
            Value::Int(value) => RawValue::Int(*value),
            Value::Float(value) => RawValue::Float(*value),
            Value::BigInt(value) => RawValue::BigInt(value.clone()),
            Value::String(value) => RawValue::String(value.clone()),
            Value::Symbol(symbol) => {
                if !symbol.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("property value"));
                }
                RawValue::Symbol(symbol.atom())
            }
            Value::Object(object) => {
                if !object.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("property value"));
                }
                RawValue::Object(object.object_id())
            }
        })
    }

    fn store_complete_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        complete: CompleteOrdinaryPropertyDescriptor,
    ) -> Result<(), RuntimeError> {
        let global_hidden = {
            let state = self.0.state.borrow();
            match state.heap.object(object.object_id())?.payload {
                ObjectPayload::GlobalObject { uninitialized_vars } => Some(uninitialized_vars),
                ObjectPayload::Ordinary
                | ObjectPayload::Date(_)
                | ObjectPayload::RegExp(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. }
                | ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. } => None,
            }
        };
        if let Some(hidden) = global_hidden {
            return self.store_complete_global_property(object, hidden, key, complete);
        }

        let (flags, replacement) = match complete {
            CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable,
                enumerable,
                configurable,
            } => (
                PropertyFlags::data(writable, enumerable, configurable),
                PropertySlot::Data(self.raw_property_value(&value)?),
            ),
            CompleteOrdinaryPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            } => (
                PropertyFlags::accessor(enumerable, configurable),
                PropertySlot::Accessor {
                    get: get.as_ref().map(|value| value.as_object().object_id()),
                    set: set.as_ref().map(|value| value.as_object().object_id()),
                },
            ),
        };
        self.store_property_slot(object, key, flags, replacement)
    }

    fn store_complete_global_property(
        &self,
        object: &ObjectRef,
        hidden_id: ObjectId,
        key: &PropertyKey,
        complete: CompleteOrdinaryPropertyDescriptor,
    ) -> Result<(), RuntimeError> {
        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden_id)?;
        match complete {
            CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable,
                enumerable,
                configurable,
            } => {
                let global_root = self.own_var_ref_root(object, key)?;
                let hidden_root = if global_root.is_none() {
                    self.own_var_ref_root(&hidden, key)?
                } else {
                    None
                };
                let root = if let Some(root) = global_root {
                    self.write_var_ref(&root, value)?;
                    root
                } else if let Some(root) = hidden_root {
                    if !self.delete_property(&hidden, key)? {
                        return Err(RuntimeError::Invariant(
                            "hidden global VarRef property was not configurable",
                        ));
                    }
                    self.write_var_ref(&root, value)?;
                    root
                } else {
                    self.new_var_ref(value, false, !writable, ClosureVariableKind::Normal)?
                };
                self.set_var_ref_metadata(&root, false, !writable, ClosureVariableKind::Normal)?;
                self.store_property_slot(
                    object,
                    key,
                    PropertyFlags::data(writable, enumerable, configurable),
                    PropertySlot::VarRef(root.id()),
                )
            }
            CompleteOrdinaryPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            } => {
                if let Some(root) = self.own_var_ref_root(object, key)? {
                    let shared = self.0.state.borrow().heap.var_ref_strong_count(root.id())? > 2;
                    if shared {
                        if self.own_var_ref_root(&hidden, key)?.is_some() {
                            return Err(RuntimeError::Invariant(
                                "global property and hidden table contain distinct VarRefs",
                            ));
                        }
                        self.reset_var_ref_uninitialized(&root)?;
                        self.set_var_ref_metadata(
                            &root,
                            false,
                            false,
                            ClosureVariableKind::Normal,
                        )?;
                        self.store_property_slot(
                            &hidden,
                            key,
                            PropertyFlags::data(true, true, true),
                            PropertySlot::VarRef(root.id()),
                        )?;
                    }
                }
                self.store_property_slot(
                    object,
                    key,
                    PropertyFlags::accessor(enumerable, configurable),
                    PropertySlot::Accessor {
                        get: get.as_ref().map(|value| value.as_object().object_id()),
                        set: set.as_ref().map(|value| value.as_object().object_id()),
                    },
                )
            }
        }
    }

    fn own_var_ref_root(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<VarRefRoot>, RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let id = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object.object_id())?;
            let shape = state.heap.shape(object.shape)?;
            let Some(index) = shape.find(key.atom()) else {
                return Ok(None);
            };
            match object.slots.get(index as usize) {
                Some(PropertySlot::VarRef(id)) => Some(*id),
                Some(
                    PropertySlot::Data(_)
                    | PropertySlot::Accessor { .. }
                    | PropertySlot::AutoInit(_),
                ) => None,
                None => {
                    return Err(RuntimeError::Invariant(
                        "shape property has no parallel object slot",
                    ));
                }
            }
        };
        id.map(|id| VarRefRoot::from_borrowed_handle(self.clone(), id).map_err(Into::into))
            .transpose()
    }

    fn store_property_slot(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        flags: PropertyFlags,
        replacement: PropertySlot,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let object_id = object.object_id();
        let (prototype, mut entries, mut slots, existing) = {
            let object_data = state.heap.object(object_id)?;
            let shape = state.heap.shape(object_data.shape)?;
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object_data.slots.clone(),
                shape.find(key.atom()).map(|index| index as usize),
            )
        };

        if let Some(index) = existing {
            let entry = entries.get(index).ok_or(RuntimeError::Invariant(
                "shape lookup index was out of bounds",
            ))?;
            if entry.flags == flags {
                let retained_atoms = state.retain_slot_atoms(std::slice::from_ref(&replacement))?;
                match state
                    .heap
                    .replace_object_slot(object_id, index, replacement)
                {
                    Ok(cleanup) => return state.apply_cleanup(cleanup),
                    Err(error) => {
                        state.release_atoms(retained_atoms)?;
                        return Err(error.into());
                    }
                }
            }
            entries[index].flags = flags;
            slots[index] = replacement;
        } else {
            entries.push(ShapeEntry {
                atom: key.atom(),
                flags,
            });
            slots.push(replacement);
        }
        state.replace_layout(object_id, prototype, &entries, slots)
    }

    fn operation(&self) -> RuntimeOperation<'_> {
        let result = self.drain_deferred_references();
        debug_assert!(result.is_ok(), "deferred root release failed: {result:?}");
        RuntimeOperation(self)
    }

    fn drain_deferred_references(&self) -> Result<(), RuntimeError> {
        loop {
            let operation = self.0.deferred_references.borrow_mut().pop_front();
            let Some(operation) = operation else {
                return Ok(());
            };
            let Ok(mut state) = self.0.state.try_borrow_mut() else {
                self.0
                    .deferred_references
                    .borrow_mut()
                    .push_front(operation);
                return Ok(());
            };
            match operation {
                DeferredRefOp::Object(object) => {
                    let cleanup = state.heap.release_object(object)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::Context(context) => {
                    let cleanup = state.heap.release_context(context)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::FunctionBytecode(bytecode) => {
                    let cleanup = state.heap.release_function_bytecode(bytecode)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::VarRef(var_ref) => {
                    let cleanup = state.heap.release_var_ref(var_ref)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::Atom(atom) => {
                    state.atoms.release(atom)?;
                }
                DeferredRefOp::ActiveFramePop { token, depth } => {
                    if let Some(position) = state
                        .active_frames
                        .iter()
                        .rposition(|frame| frame.token == token)
                    {
                        state.active_frames.truncate(position);
                    } else if state.active_frames.len() > depth {
                        state.active_frames.truncate(depth);
                    }
                }
                DeferredRefOp::BacktraceBarrierRestore { token, previous } => {
                    if let Some(frame) = state
                        .active_frames
                        .iter_mut()
                        .find(|frame| frame.token == token)
                    {
                        frame.flags.backtrace_barrier = previous;
                    }
                }
            }
        }
    }

    pub(crate) fn retain_object_handle(&self, id: ObjectId) -> Result<(), HeapError> {
        let mut state = self.0.state.try_borrow_mut().map_err(|_| {
            HeapError::Invariant("object root retained during a runtime state borrow")
        })?;
        state.heap.retain_object(id)
    }

    pub(crate) fn release_object_handle(&self, id: ObjectId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state.heap.release_object(id).map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::Object(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid object root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred object release failed: {drain:?}");
    }

    pub(crate) fn retain_atom_handle(&self, atom: Atom) -> Result<(), AtomError> {
        self.0.state.borrow_mut().atoms.retain(atom).map(drop)
    }

    pub(crate) fn retain_function_bytecode_handle(
        &self,
        id: FunctionBytecodeId,
    ) -> Result<(), HeapError> {
        let mut state = self.0.state.try_borrow_mut().map_err(|_| {
            HeapError::Invariant("function bytecode retained during a runtime state borrow")
        })?;
        state.heap.retain_function_bytecode(id)
    }

    fn retain_context_handle(&self, id: ContextId) -> Result<(), HeapError> {
        let mut state =
            self.0.state.try_borrow_mut().map_err(|_| {
                HeapError::Invariant("context retained during a runtime state borrow")
            })?;
        state.heap.retain_context(id)
    }

    fn release_context_handle(&self, id: ContextId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state.heap.release_context(id).map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::Context(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid context root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred context release failed: {drain:?}");
    }

    pub(crate) fn release_function_bytecode_handle(&self, id: FunctionBytecodeId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state
                .heap
                .release_function_bytecode(id)
                .map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::FunctionBytecode(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid bytecode root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred bytecode release failed: {drain:?}");
    }

    fn retain_var_ref_handle(&self, id: VarRefId) -> Result<(), HeapError> {
        let mut state =
            self.0.state.try_borrow_mut().map_err(|_| {
                HeapError::Invariant("VarRef retained during a runtime state borrow")
            })?;
        state.heap.retain_var_ref(id)
    }

    fn release_var_ref_handle(&self, id: VarRefId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state.heap.release_var_ref(id).map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::VarRef(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid VarRef root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred VarRef release failed: {drain:?}");
    }

    pub(crate) fn release_atom_handle(&self, atom: Atom) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            state.atoms.release(atom).map(drop)
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::Atom(atom));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid atom root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred atom release failed: {drain:?}");
    }
}

fn bound_function_length(
    value: &Value,
    bound_argument_count: usize,
) -> Result<Value, RuntimeError> {
    let count = u32::try_from(bound_argument_count)
        .map_err(|_| RuntimeError::Invariant("bound argument count does not fit u32"))?;
    Ok(match value {
        Value::Int(length) => {
            let length = i64::from(*length);
            let count = i64::from(count);
            if length <= count {
                Value::Int(0)
            } else {
                Value::Int(i32::try_from(length - count).map_err(|_| {
                    RuntimeError::Invariant("bound function integer length does not fit i32")
                })?)
            }
        }
        Value::Float(length) => {
            let length = if length.is_nan() {
                0.0
            } else {
                let length = length.trunc();
                if length <= f64::from(count) {
                    0.0
                } else {
                    length - f64::from(count)
                }
            };
            Value::number(length)
        }
        Value::Undefined
        | Value::Null
        | Value::Bool(_)
        | Value::BigInt(_)
        | Value::String(_)
        | Value::Symbol(_)
        | Value::Object(_) => Value::Int(0),
    })
}

fn append_backtrace_ascii(output: &mut JsStringBuilder, value: &str) -> Result<(), JsStringError> {
    output.push_utf8(value)
}

fn append_backtrace_string(
    output: &mut JsStringBuilder,
    value: &JsString,
) -> Result<(), JsStringError> {
    output.push_js_string(value)
}

fn truncate_backtrace_c_string(value: JsString) -> Result<JsString, RuntimeError> {
    if !value.utf16_units().any(|unit| unit == 0) {
        return Ok(value);
    }
    let prefix = value.utf16_units().take_while(|unit| *unit != 0);
    Ok(JsString::try_from_utf16(prefix)?)
}

fn raw_string_property_on_object(
    state: &RuntimeState,
    object: ObjectId,
    atom: Atom,
) -> Result<RawStringProperty, RuntimeError> {
    let object = state.heap.object(object)?;
    let shape = state.heap.shape(object.shape)?;
    let Some(index) = shape.find(atom) else {
        return Ok(RawStringProperty::Missing);
    };
    let slot = object
        .slots
        .get(index as usize)
        .ok_or(RuntimeError::Invariant(
            "backtrace name shape has no parallel property slot",
        ))?;
    Ok(match slot {
        PropertySlot::Data(RawValue::String(value)) => RawStringProperty::String(value.clone()),
        PropertySlot::Data(_)
        | PropertySlot::VarRef(_)
        | PropertySlot::Accessor { .. }
        | PropertySlot::AutoInit(_) => RawStringProperty::Other,
    })
}

fn raw_string_property_one_level(
    state: &RuntimeState,
    object: ObjectId,
    atom: Atom,
) -> Result<Option<JsString>, RuntimeError> {
    match raw_string_property_on_object(state, object, atom)? {
        RawStringProperty::String(name) => return Ok(Some(name)),
        RawStringProperty::Other => return Ok(None),
        RawStringProperty::Missing => {}
    }

    let object = state.heap.object(object)?;
    let Some(prototype) = state.heap.shape(object.shape)?.prototype() else {
        return Ok(None);
    };
    Ok(
        match raw_string_property_on_object(state, prototype, atom)? {
            RawStringProperty::String(name) => Some(name),
            RawStringProperty::Missing | RawStringProperty::Other => None,
        },
    )
}

impl RuntimeState {
    fn retain_raw_root(&mut self, value: &RawValue) -> Result<(), RuntimeError> {
        match value {
            RawValue::Object(object) => self.heap.retain_object(*object)?,
            RawValue::Symbol(atom) => {
                self.atoms.retain(*atom)?;
            }
            RawValue::Undefined
            | RawValue::Null
            | RawValue::Bool(_)
            | RawValue::Int(_)
            | RawValue::Float(_)
            | RawValue::BigInt(_)
            | RawValue::String(_) => {}
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel cannot become a runtime root",
                ));
            }
        }
        Ok(())
    }

    fn release_owned_raw_root(&mut self, value: RawValue) -> Result<(), RuntimeError> {
        match value {
            RawValue::Object(object) => {
                let cleanup = self.heap.release_object(object)?;
                self.apply_cleanup(cleanup)?;
            }
            RawValue::Symbol(atom) => {
                self.atoms.release(atom)?;
            }
            RawValue::Undefined
            | RawValue::Null
            | RawValue::Bool(_)
            | RawValue::Int(_)
            | RawValue::Float(_)
            | RawValue::BigInt(_)
            | RawValue::String(_) => {}
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel occupied a runtime root",
                ));
            }
        }
        Ok(())
    }

    fn get_or_create_shape(
        &mut self,
        prototype: Option<ObjectId>,
        entries: &[ShapeEntry],
    ) -> Result<ShapeId, RuntimeError> {
        let fingerprint = ShapeFingerprint {
            prototype,
            entries: entries.into(),
        };
        if let Some(&shape) = self.shape_cache.get(&fingerprint) {
            if self.heap.shape(shape).is_ok() {
                self.heap.retain_shape(shape)?;
                return Ok(shape);
            }
            self.shape_cache.remove(&fingerprint);
            self.shape_fingerprints.remove(&shape);
        }

        let mut retained_atoms = Vec::with_capacity(entries.len());
        for entry in entries {
            if let Err(error) = self.atoms.resolve(entry.atom) {
                self.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
            if let Err(error) = self.atoms.retain(entry.atom) {
                self.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
            retained_atoms.push(entry.atom);
        }

        let shape = match Shape::new(prototype, entries.iter().copied()) {
            Ok(shape) => shape,
            Err(error) => {
                self.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
        };
        let shape = match self.heap.allocate_shape(shape) {
            Ok(shape) => shape,
            Err(error) => {
                self.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
        };
        self.shape_cache.insert(fingerprint.clone(), shape);
        self.shape_fingerprints.insert(shape, fingerprint);
        Ok(shape)
    }

    fn retain_slot_atoms(&mut self, slots: &[PropertySlot]) -> Result<Vec<Atom>, RuntimeError> {
        let atoms = slots
            .iter()
            .filter_map(|slot| match slot {
                PropertySlot::Data(RawValue::Symbol(atom)) => Some(*atom),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut retained = Vec::with_capacity(atoms.len());
        for atom in atoms {
            if let Err(error) = self.atoms.retain(atom) {
                self.release_atoms(retained)?;
                return Err(error.into());
            }
            retained.push(atom);
        }
        Ok(retained)
    }

    fn retain_raw_value_atoms<'a>(
        &mut self,
        values: impl IntoIterator<Item = &'a RawValue>,
    ) -> Result<Vec<Atom>, RuntimeError> {
        let atoms = values
            .into_iter()
            .filter_map(|value| match value {
                RawValue::Symbol(atom) => Some(*atom),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut retained = Vec::with_capacity(atoms.len());
        for atom in atoms {
            if let Err(error) = self.atoms.retain(atom) {
                self.release_atoms(retained)?;
                return Err(error.into());
            }
            retained.push(atom);
        }
        Ok(retained)
    }

    fn replace_layout(
        &mut self,
        object: ObjectId,
        prototype: Option<ObjectId>,
        entries: &[ShapeEntry],
        slots: Vec<PropertySlot>,
    ) -> Result<(), RuntimeError> {
        let shape = self.get_or_create_shape(prototype, entries)?;
        let retained_atoms = match self.retain_slot_atoms(&slots) {
            Ok(atoms) => atoms,
            Err(error) => {
                let cleanup = self.heap.release_shape(shape)?;
                self.apply_cleanup(cleanup)?;
                return Err(error);
            }
        };

        let layout_cleanup = match self.heap.replace_object_layout(object, shape, slots) {
            Ok(cleanup) => cleanup,
            Err(error) => {
                self.release_atoms(retained_atoms)?;
                let cleanup = self.heap.release_shape(shape)?;
                self.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let shape_cleanup = self.heap.release_shape(shape)?;
        self.apply_cleanup(layout_cleanup)?;
        self.apply_cleanup(shape_cleanup)
    }

    fn apply_cleanup(&mut self, cleanup: HeapCleanup) -> Result<(), RuntimeError> {
        self.unlink_finalized_shapes(cleanup.finalized_shape_ids);
        self.release_atoms(cleanup.atoms)
    }

    fn unlink_finalized_shapes(&mut self, shapes: impl IntoIterator<Item = ShapeId>) {
        for shape in shapes {
            let Some(fingerprint) = self.shape_fingerprints.remove(&shape) else {
                continue;
            };
            if self.shape_cache.get(&fingerprint) == Some(&shape) {
                self.shape_cache.remove(&fingerprint);
            }
        }
    }

    fn release_atoms(&mut self, atoms: impl IntoIterator<Item = Atom>) -> Result<(), RuntimeError> {
        for atom in atoms {
            self.atoms.release(atom)?;
        }
        Ok(())
    }
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        let state = self.state.get_mut();
        let deferred = self.deferred_references.get_mut();
        while let Some(operation) = deferred.pop_front() {
            let result = match operation {
                DeferredRefOp::Object(object) => state
                    .heap
                    .release_object(object)
                    .map_err(RuntimeError::Heap)
                    .and_then(|cleanup| state.apply_cleanup(cleanup)),
                DeferredRefOp::Context(context) => state
                    .heap
                    .release_context(context)
                    .map_err(RuntimeError::Heap)
                    .and_then(|cleanup| state.apply_cleanup(cleanup)),
                DeferredRefOp::FunctionBytecode(bytecode) => state
                    .heap
                    .release_function_bytecode(bytecode)
                    .map_err(RuntimeError::Heap)
                    .and_then(|cleanup| state.apply_cleanup(cleanup)),
                DeferredRefOp::VarRef(var_ref) => state
                    .heap
                    .release_var_ref(var_ref)
                    .map_err(RuntimeError::Heap)
                    .and_then(|cleanup| state.apply_cleanup(cleanup)),
                DeferredRefOp::Atom(atom) => {
                    state.atoms.release(atom).map(drop).map_err(Into::into)
                }
                DeferredRefOp::ActiveFramePop { token, depth } => {
                    if let Some(position) = state
                        .active_frames
                        .iter()
                        .rposition(|frame| frame.token == token)
                    {
                        state.active_frames.truncate(position);
                    } else if state.active_frames.len() > depth {
                        state.active_frames.truncate(depth);
                    }
                    Ok(())
                }
                DeferredRefOp::BacktraceBarrierRestore { token, previous } => {
                    if let Some(frame) = state
                        .active_frames
                        .iter_mut()
                        .find(|frame| frame.token == token)
                    {
                        frame.flags.backtrace_barrier = previous;
                    }
                    Ok(())
                }
            };
            debug_assert!(
                result.is_ok(),
                "runtime deferred teardown failed: {result:?}"
            );
        }
        if let Some(exception) = state.pending_exception.take() {
            let result = state.release_owned_raw_root(exception);
            debug_assert!(
                result.is_ok(),
                "pending exception teardown failed: {result:?}"
            );
        }
        let result = state
            .heap
            .run_gc()
            .map_err(RuntimeError::Heap)
            .and_then(|mut stats| {
                let atoms = std::mem::take(&mut stats.cleanup.atoms);
                state.release_atoms(atoms)
            });
        debug_assert!(result.is_ok(), "runtime teardown failed: {result:?}");
        debug_assert_eq!(
            state.heap.counts().live,
            0,
            "runtime teardown left live heap nodes"
        );
    }
}

fn descriptor_to_validation_record(
    descriptor: &OrdinaryPropertyDescriptor,
) -> PropertyDescriptor<Value> {
    PropertyDescriptor {
        value: descriptor.value.as_ref().into_option().cloned(),
        writable: descriptor.writable.as_ref().into_option().copied(),
        get: descriptor.get.as_ref().into_option().map(|accessor| {
            accessor
                .as_callable()
                .map(|callable| Value::Object(callable.as_object().clone()))
        }),
        set: descriptor.set.as_ref().into_option().map(|accessor| {
            accessor
                .as_callable()
                .map(|callable| Value::Object(callable.as_object().clone()))
        }),
        enumerable: descriptor.enumerable.as_ref().into_option().copied(),
        configurable: descriptor.configurable.as_ref().into_option().copied(),
    }
}

fn complete_to_validation_record(
    descriptor: &CompleteOrdinaryPropertyDescriptor,
) -> CompletePropertyDescriptor<Value> {
    match descriptor {
        CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        } => CompletePropertyDescriptor::Data {
            value: value.clone(),
            writable: *writable,
            enumerable: *enumerable,
            configurable: *configurable,
        },
        CompleteOrdinaryPropertyDescriptor::Accessor {
            get,
            set,
            enumerable,
            configurable,
        } => CompletePropertyDescriptor::Accessor {
            get: get
                .as_ref()
                .map(|callable| Value::Object(callable.as_object().clone())),
            set: set
                .as_ref()
                .map(|callable| Value::Object(callable.as_object().clone())),
            enumerable: *enumerable,
            configurable: *configurable,
        },
    }
}

fn validation_record_to_complete(
    descriptor: CompletePropertyDescriptor<Value>,
) -> Result<CompleteOrdinaryPropertyDescriptor, RuntimeError> {
    match descriptor {
        CompletePropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        } => Ok(CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        }),
        CompletePropertyDescriptor::Accessor {
            get,
            set,
            enumerable,
            configurable,
        } => {
            let get = get
                .map(|value| match value {
                    Value::Object(object) => Ok(CallableRef::from_validated_object(object)),
                    _ => Err(RuntimeError::Invariant(
                        "validated accessor getter was not callable",
                    )),
                })
                .transpose()?;
            let set = set
                .map(|value| match value {
                    Value::Object(object) => Ok(CallableRef::from_validated_object(object)),
                    _ => Err(RuntimeError::Invariant(
                        "validated accessor setter was not callable",
                    )),
                })
                .transpose()?;
            Ok(CompleteOrdinaryPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            })
        }
    }
}

/// One realm and its execution state.
/// Execution-only eval options. Compilation metadata remains in
/// [`CompileOptions`]; the barrier mirrors QuickJS
/// `JS_EVAL_FLAG_BACKTRACE_BARRIER` and temporarily marks the caller frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalOptions {
    pub filename: String,
    pub backtrace_barrier: bool,
}

impl EvalOptions {
    #[must_use]
    pub fn new(filename: impl Into<String>) -> Self {
        Self {
            filename: filename.into(),
            backtrace_barrier: false,
        }
    }
}

impl Default for EvalOptions {
    fn default() -> Self {
        Self::new(crate::compiler::DEFAULT_EVAL_FILENAME)
    }
}

pub struct Context {
    runtime: Runtime,
    id: u64,
    realm: ContextId,
}

impl Clone for Context {
    fn clone(&self) -> Self {
        self.runtime
            .retain_context_handle(self.realm)
            .expect("a live Context handle must retain its realm");
        Self {
            runtime: self.runtime.clone(),
            id: self.id,
            realm: self.realm,
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        self.runtime.release_context_handle(self.realm);
    }
}

impl Context {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Return this realm's `%Object.prototype%` root.
    pub fn object_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .object_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's genuine empty `%Array.prototype%` root.
    pub fn array_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .array_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's `%Function.prototype%` root.
    pub fn function_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .function_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's `%IteratorPrototype%` root. The global `Iterator`
    /// constructor and Iterator Helpers are intentionally not exposed yet.
    pub fn iterator_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .iterator_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's `%StringIteratorPrototype%` root.
    pub fn string_iterator_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .string_iterator_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's boxed-+0 `%Number.prototype%` root.
    pub fn number_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::Number)
    }

    /// Return this realm's boxed-false `%Boolean.prototype%` root.
    pub fn boolean_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::Boolean)
    }

    /// Return this realm's branded-empty partial `%String.prototype%` root.
    pub fn string_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::String)
    }

    /// Return this realm's ordinary `%Symbol.prototype%` root.
    pub fn symbol_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::Symbol)
    }

    /// Return this realm's ordinary `%BigInt.prototype%` root.
    pub fn bigint_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::BigInt)
    }

    /// Return this realm's `%Function%` constructor root.
    pub fn function_constructor(&self) -> Result<CallableRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .function_constructor
            .ok_or(RuntimeError::Invariant("realm has no Function constructor"))?;
        Ok(CallableRef::from_validated_object(
            ObjectRef::from_borrowed_handle(self.runtime.clone(), object)?,
        ))
    }

    /// Return this realm's global object root.
    pub fn global_object(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .global_object;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return the null-prototype object used for global lexical bindings.
    pub fn global_var_object(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .global_var_object;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    #[cfg(test)]
    pub(crate) fn create_global_lexical_for_test(
        &self,
        name: &str,
        is_const: bool,
        initial_value: Option<Value>,
    ) -> Result<(), RuntimeError> {
        self.runtime
            .create_global_lexical_for_test(self.realm, name, is_const, initial_value)
    }

    #[cfg(test)]
    pub(crate) fn initialize_global_lexical_for_test(
        &self,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        self.runtime
            .initialize_global_lexical_for_test(self.realm, name, value)
    }

    /// Allocate an ordinary object with this realm's `%Object.prototype%`.
    pub fn new_object(&mut self) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.object_prototype()?;
        self.runtime.new_object(Some(&prototype))
    }

    /// Create QuickJS's test262-only native `codePointRange` helper in this
    /// context's realm.
    ///
    /// The helper is intentionally not installed as an ECMAScript intrinsic;
    /// embedders such as the Test262 runner decide where to publish it.
    pub fn new_code_point_range_function(&mut self) -> Result<CallableRef, RuntimeError> {
        let function_prototype = self.function_prototype()?;
        self.runtime.new_native_builtin(
            &function_prototype,
            self.realm,
            NativeFunctionId::StringCodePointRange,
            2,
            "codePointRange",
            2,
        )
    }

    /// Allocate one genuine empty Array in this realm.
    pub fn new_array(&mut self) -> Result<ObjectRef, RuntimeError> {
        self.runtime.new_array(self.realm)
    }

    /// Allocate one genuine Array initialized from consecutive values.
    pub fn new_array_from_values(&mut self, values: Vec<Value>) -> Result<ObjectRef, RuntimeError> {
        self.runtime.new_array_from_values(self.realm, values)
    }

    /// Allocate an ordinary object with an explicit object-or-null prototype.
    pub fn new_object_with_prototype(
        &mut self,
        prototype: Option<&ObjectRef>,
    ) -> Result<ObjectRef, RuntimeError> {
        self.runtime.new_object(prototype)
    }

    pub fn get_own_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<CompleteOrdinaryPropertyDescriptor>, RuntimeError> {
        self.runtime.get_own_property(object, key)
    }

    pub fn define_own_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<bool, RuntimeError> {
        match self.runtime.define_own_property_in_realm(
            Some(self.realm),
            object,
            key,
            descriptor,
        )? {
            PropertyDefineOutcome::Defined(defined) => Ok(defined),
            PropertyDefineOutcome::Throw(value) => {
                self.runtime.set_pending_exception(value)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    pub fn get_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Value, RuntimeError> {
        match self.runtime.prepare_get_property(object, key)? {
            PropertyGetAction::Complete(value) => Ok(value),
            PropertyGetAction::Call { getter, receiver } => {
                let completion = self
                    .runtime
                    .call_internal(self.realm, &getter, receiver, &[])?;
                self.finish_completion(completion)
            }
        }
    }

    pub fn get_property_with_receiver(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<Value, RuntimeError> {
        match self
            .runtime
            .prepare_get_property_with_receiver(object, key, receiver)?
        {
            PropertyGetAction::Complete(value) => Ok(value),
            PropertyGetAction::Call { getter, receiver } => {
                let completion = self
                    .runtime
                    .call_internal(self.realm, &getter, receiver, &[])?;
                self.finish_completion(completion)
            }
        }
    }

    pub fn set_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<bool, RuntimeError> {
        match self
            .runtime
            .prepare_set_property_in_realm(self.realm, object, key, value)?
        {
            PropertySetAction::Complete => Ok(true),
            PropertySetAction::Throw(value) => {
                self.runtime.set_pending_exception(value)?;
                Err(RuntimeError::Exception)
            }
            PropertySetAction::Rejected(_) => Ok(false),
            PropertySetAction::Call {
                setter,
                receiver,
                argument,
            } => {
                let completion =
                    self.runtime
                        .call_internal(self.realm, &setter, receiver, &[argument])?;
                let returned = self.finish_completion(completion)?;
                drop(returned);
                Ok(true)
            }
        }
    }

    pub fn set_property_with_receiver(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<bool, RuntimeError> {
        match self.runtime.prepare_set_property_with_receiver_in_realm(
            Some(self.realm),
            object,
            key,
            value,
            receiver,
        )? {
            PropertySetAction::Complete => Ok(true),
            PropertySetAction::Throw(value) => {
                self.runtime.set_pending_exception(value)?;
                Err(RuntimeError::Exception)
            }
            PropertySetAction::Rejected(_) => Ok(false),
            PropertySetAction::Call {
                setter,
                receiver,
                argument,
            } => {
                let completion =
                    self.runtime
                        .call_internal(self.realm, &setter, receiver, &[argument])?;
                let returned = self.finish_completion(completion)?;
                drop(returned);
                Ok(true)
            }
        }
    }

    /// Compile one script and publish its immutable bytecode in this realm.
    ///
    /// The returned handle is a runtime root. Its constant pool and captured
    /// realm remain alive even if this particular `Context` handle is dropped.
    pub fn compile(&mut self, source: &str) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_with_options(source, &CompileOptions::default())
    }

    /// Compile one script with an explicit filename attached independently to
    /// every published function's debug metadata.
    pub fn compile_with_filename(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_with_options(source, &CompileOptions::new(filename))
    }

    /// Compile one script with named compilation options.
    pub fn compile_with_options(
        &mut self,
        source: &str,
        options: &CompileOptions,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_with_options_internal(source, options, false)
    }

    /// Compile while retaining implementation-frontier provenance as an
    /// engine [`ErrorKind::Unsupported`] diagnostic instead of converting it
    /// to the temporary public-eval `SyntaxError` compatibility boundary.
    ///
    /// Test harnesses can use this to avoid mistaking an unimplemented grammar
    /// branch for a conforming early error.
    pub fn compile_with_options_preserving_unsupported_diagnostics(
        &mut self,
        source: &str,
        options: &CompileOptions,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_with_options_internal(source, options, true)
    }

    fn compile_with_options_internal(
        &mut self,
        source: &str,
        options: &CompileOptions,
        preserve_unsupported_diagnostics: bool,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        match self.runtime.compile_in_realm(
            self.realm,
            source,
            &options.filename,
            preserve_unsupported_diagnostics,
        )? {
            Compilation::Published(function) => Ok(function),
            Compilation::Throw(exception) => {
                self.runtime.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    /// Instantiate and evaluate runtime-owned script bytecode.
    ///
    /// As in QuickJS's `JS_EvalFunctionInternal`, the raw bytecode is first
    /// wrapped in a callable object in the initiating context. The call then
    /// executes in the realm captured by the bytecode.
    pub fn execute(&mut self, function: &FunctionBytecodeRef) -> Result<Value, RuntimeError> {
        let callable = match self.runtime.new_bytecode_closure(self.realm, function) {
            Ok(callable) => callable,
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind())
                    .expect("guard proved this is a JavaScript-visible declaration error");
                let exception = self
                    .runtime
                    .new_native_error_from_error(self.realm, kind, &error)?;
                self.runtime
                    .ensure_error_backtrace(&exception, false, None)?;
                self.runtime.set_pending_exception(exception)?;
                return Err(RuntimeError::Exception);
            }
            Err(error) => return Err(error),
        };
        let this_value = Value::Object(self.global_object()?);
        self.call(&callable, this_value, &[])
    }

    /// Invoke a validated callable with an explicit `this` value and arguments.
    pub fn call(
        &mut self,
        callable: &CallableRef,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        let completion = self
            .runtime
            .call_internal(self.realm, callable, this_value, arguments)?;
        self.finish_completion(completion)
    }

    /// Invoke a validated constructor with itself as `new.target`, matching
    /// `JS_CallConstructor` and source-level `new`.
    pub fn construct(
        &mut self,
        constructor: &CallableRef,
        arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        self.construct_with_new_target(constructor, constructor, arguments)
    }

    /// Invoke a constructor with an explicit `new.target`, matching
    /// `JS_CallConstructor2`/`Reflect.construct` semantics.
    pub fn construct_with_new_target(
        &mut self,
        constructor: &CallableRef,
        new_target: &CallableRef,
        arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        match self
            .runtime
            .construct_internal(self.realm, constructor, new_target, arguments)
        {
            Ok(completion) => self.finish_completion(completion),
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind())
                    .expect("guard proved this is a JavaScript-visible native error");
                let exception = self
                    .runtime
                    .new_native_error_from_error(self.realm, kind, &error)?;
                self.runtime
                    .ensure_error_backtrace(&exception, false, None)?;
                self.runtime.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
            Err(error) => Err(error),
        }
    }

    fn finish_completion(&mut self, completion: Completion) -> Result<Value, RuntimeError> {
        match completion {
            Completion::Return(value) => Ok(value),
            Completion::Throw(value) => {
                self.runtime.set_pending_exception(value)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    /// Return whether this runtime currently carries a pending JavaScript
    /// exception completion.
    #[must_use]
    pub fn has_exception(&self) -> bool {
        self.runtime.has_pending_exception()
    }

    /// Move the pending JavaScript exception value out of the runtime slot.
    pub fn take_exception(&mut self) -> Result<Option<Value>, RuntimeError> {
        self.runtime.take_pending_exception()
    }

    /// Compile and evaluate one script through runtime-owned bytecode.
    ///
    /// # Errors
    /// Returns syntax, publication, runtime-domain, or execution errors.
    pub fn eval(&mut self, source: &str) -> Result<Value, RuntimeError> {
        self.eval_with_options(source, &EvalOptions::default())
    }

    /// Compile and evaluate a script with an explicit debug filename.
    pub fn eval_with_filename(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<Value, RuntimeError> {
        self.eval_with_options(source, &EvalOptions::new(filename))
    }

    /// Compile and evaluate a script with filename and execution options.
    pub fn eval_with_options(
        &mut self,
        source: &str,
        options: &EvalOptions,
    ) -> Result<Value, RuntimeError> {
        let barrier = self
            .runtime
            .install_backtrace_barrier(options.backtrace_barrier)?;
        let result = (|| {
            let function =
                self.compile_with_options(source, &CompileOptions::new(&options.filename))?;
            self.execute(&function)
        })();
        barrier.finish()?;
        result
    }
}

#[cfg(test)]
mod tests;
