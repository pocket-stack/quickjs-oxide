//! Closed registration of native algorithms that yield owned domain requests.
use super::native::NativeFunctionId;
use super::object::{
    definitions::{DefinitionsKind, DefinitionsStep},
    predicate::{PredicateKind, PredicateStep},
    property::{PropertyKind, PropertyStep},
    prototype::{BuiltinPrototypeKind, BuiltinPrototypeStep},
    string::{ObjectStringKind, ObjectStringStep},
};
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    vm::call::{NativeArguments, NativeInvocation},
};

mod output;

/// Closed synchronous entry families. The ordinary native ABI remains shared;
/// only the generic waiting dispatcher is absent from this corridor.
pub(crate) enum SynchronousNative {
    Pure(NativeFunctionId),
    PrimitiveConstructor(super::native::PrimitiveKind),
    Math(super::math::operation::MathKind),
}
impl SynchronousNative {
    pub(crate) fn start(
        self,
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
        callable: &crate::engine::object::CallableRef,
    ) -> Result<crate::engine::vm::Completion, RuntimeError> {
        match self {
            Self::Pure(target) => runtime.dispatch_adapted_native_function(
                callable,
                target,
                realm,
                invocation.clone(),
                arguments,
            ),
            Self::PrimitiveConstructor(kind) => match super::PrimitiveConstructorStep::start(
                runtime, realm, kind, invocation, arguments,
            )? {
                super::PrimitiveConstructorStep::Complete(result) => Ok(result),
                _ => Err(RuntimeError::Invariant(
                    "synchronous primitive constructor unexpectedly waited",
                )),
            },
            Self::Math(kind) => {
                match super::MathStep::start(runtime, realm, kind, invocation, arguments)? {
                    super::MathStep::Complete(result) => Ok(result),
                    _ => Err(RuntimeError::Invariant(
                        "synchronous Math unexpectedly required conversion",
                    )),
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum NativeOperation {
    #[cfg(test)]
    ActiveFrameProbe,
    ModuleCallback(NativeFunctionId),
    #[cfg(feature = "test262-host")]
    Test262Agent(super::native::Test262AgentKind),
    #[cfg(feature = "test262-host")]
    EvalScript,
    HostOutput(NativeFunctionId),
    AsyncGenerator(NativeFunctionId),
    FromSync(NativeFunctionId),
    AsyncResume(crate::engine::heap::AsyncFunctionResumeKind),
    Promise(NativeFunctionId),
    GeneratorResume(super::native::GeneratorResumeKind),
    Atomics(super::native::AtomicsNativeKind),
    TypedCreate(super::native::TypedArrayNativeKind),
    SharedBufferConstructor,
    SharedBufferGrow,
    BufferSlice(super::array_buffer::BufferSliceKind),
    TypedWith,
    Uint8Codec(super::native::Uint8ArrayCodecKind),

    TypedSearch(super::array_buffer::typed_array::TypedSearchKind),
    TypedString(super::native::ArrayJoinKind),
    TypedSlice(super::array_buffer::typed_array::TypedSliceKind),
    TypedMutation(super::array_buffer::typed_array::TypedMutationKind),
    StringFactory(super::string::StringFactoryKind),

    WeakConstructor(super::weak_ref::WeakIntrinsicKind),
    RegExpMatchAll,
    RegExpSplit,
    RegExpIterator,

    GlobalEval,
    JsonRaw,
    ObjectConstructor,
    Bind,
    FunctionText,
    DynamicFunction(super::native::DynamicFunctionKind),
    JsonParse,
    JsonStringify,
    BufferConstructor,
    DataViewConstructor,
    TypedSet,
    RegExpConstructor,
    RegExpSearch,
    RegExpMatch,
    RegExpCompile,
    StringProtocol(super::string::StringProtocolKind),

    TypedSort(bool),
    Math(super::math::operation::MathKind),
    Sum,
    PrimitiveConstructor(super::native::PrimitiveKind),
    Global(super::primitive::globals::GlobalKind),
    Numeric(super::primitive::numeric::NumericKind),
    ScalarText(super::primitive::text::ScalarTextKind),
    DateConstructor(super::native::DateNativeKind),
    DatePrototype(super::native::DateNativeKind),
    Error(super::error::operation::ErrorKind),

    MapCallback(super::map::callback::CallbackKind),
    SetEach,
    SetOperation(super::set::operations::SetOperation),
    Collection(super::iterator::collection::CollectionKind),
    WeakComputed,
    Pure(NativeFunctionId),
    ArrayConstructor,
    ArraySlice(super::array::slice::SliceKind),
    IteratorConstructor,
    IteratorAccessor,
    IteratorTag,
    TypedTraversal(super::array_buffer::typed_array::TypedTraversalKind),
    TypedIteration(super::native::ArrayIterationKind),
    ArrayConcat,
    ArrayFlatten(super::native::ArrayFlattenKind),
    StringText(super::string::StringTextKind),
    StringSearch(super::string::StringSearchKind),
    StringSplit,
    Instance,
    IteratorFrom,
    IteratorWrap(crate::engine::heap::IteratorResumeKind),
    IteratorConcat(super::iterator::concat::ConcatKind),
    ArrayBuild(super::array::build::BuildKind),
    ArraySpeciesGetter,
    ArraySort(bool),
    ArrayIndexed(super::array::indexed::IndexedKind),
    ArrayReverse,
    ArrayString(super::array::string::ArrayStringKind),
    RegExpExec(super::native::RegExpNativeKind),
    RegExpPresentation(super::native::RegExpNativeKind),
    RegExpReplace,
    IteratorConsume(super::iterator::consume::ConsumeKind),
    IteratorHelper(crate::engine::heap::IteratorResumeKind),
    IteratorCreate(crate::engine::heap::IteratorHelperKind),

    ArrayNext,
    PureIterator(NativeFunctionId),
    ArrayMutation(super::array::mutation::MutationKind),
    ArrayCallback(super::array::callback::CallbackKind),
    ObjectIteration(super::object::iteration::IterationKind),
    StringReplace(super::native::StringReplaceKind),
    DataView(super::native::DataViewNativeKind),
    BufferMutation(super::native::ArrayBufferNativeKind),
    Invoke(super::function::invoke::InvokeKind),
    Prototype(BuiltinPrototypeKind),
    Property(PropertyKind),
    String(ObjectStringKind),
    Proxy(NativeFunctionId),
    ObjectValueOf,
    ObjectIs,
    Definitions(DefinitionsKind),
    Predicate(PredicateKind),
}
pub(crate) enum NativeStep {
    ModuleCallback(crate::engine::modules::callback::CallbackStep),
    #[cfg(feature = "test262-host")]
    Test262Agent(crate::engine::api::test262_agent::operation::AgentStep),
    #[cfg(feature = "test262-host")]
    EvalScript(crate::engine::api::test262_host::operation::EvalScriptStep),
    AsyncGenerator(crate::engine::vm::async_generator::AsyncGeneratorStep),
    FromSync(crate::engine::vm::async_from_sync_iterator::FromSyncStep),
    Async(crate::engine::vm::async_function::AsyncStep),
    Promise(super::promise::operation::PromiseStep),
    GeneratorResume(crate::engine::vm::generator::GeneratorStep),
    Atomics(super::AtomicsStep),
    TypedCreate(super::TypedCreateStep),
    BufferSlice(super::BufferSliceStep),
    TypedWith(super::TypedWithStep),
    Uint8Codec(super::Uint8CodecStep),

    TypedSearch(super::TypedSearchStep),
    TypedString(super::TypedStringStep),
    TypedSlice(super::TypedSliceStep),
    TypedMutation(super::TypedMutationStep),
    StringFactory(super::StringFactoryStep),

    WeakConstructor(super::WeakConstructorStep),
    RegExpMatchAll(super::RegExpMatchAllStep),
    RegExpSplit(super::RegExpSplitStep),
    RegExpIterator(super::RegExpIteratorStep),

    GlobalEval(crate::engine::value::Value),
    JsonRaw {
        value: crate::engine::value::Value,
        resume: super::JsonRawResume,
    },
    ObjectConstructor(super::ObjectConstructorStep),
    Bind(super::BindStep),
    FunctionText(super::FunctionTextStep),
    DynamicFunction(super::DynamicFunctionStep),
    JsonParse(super::JsonParseStep),
    JsonStringify(super::JsonStringifyStep),
    BufferConstructor(super::BufferConstructorStep),
    DataViewConstructor(super::DataViewConstructorStep),
    TypedSet(super::TypedSetStep),
    RegExpConstructor(super::RegExpConstructorStep),
    RegExpSearch(super::RegExpSearchStep),
    RegExpMatch(super::RegExpMatchStep),
    RegExpCompile(super::RegExpCompileStep),
    StringProtocol(super::StringProtocolStep),

    TypedSort(super::TypedSortStep),
    Math(super::MathStep),
    Sum(super::SumStep),
    PrimitiveConstructor(super::PrimitiveConstructorStep),
    Global(super::GlobalStep),
    Numeric(super::NumericStep),
    ScalarText(super::ScalarTextStep),
    DateConstructor(super::DateConstructorStep),
    DatePrototype(super::DatePrototypeStep),
    Error(super::ErrorStep),

    MapCallback(super::MapCallbackStep),
    SetEach(super::SetEachStep),
    SetOperation(super::SetOperationStep),
    Collection(super::CollectionStep),
    WeakComputed(super::WeakComputedStep),
    ArrayConstructor(super::ArrayConstructorStep),
    ArraySlice(super::ArraySliceStep),
    IteratorConstructor(super::IteratorConstructorStep),
    IteratorTag(super::IteratorTagStep),
    TypedTraversal(super::TypedTraversalStep),
    TypedIteration(super::TypedIterationStep),

    ArrayConcat(super::ArrayConcatStep),
    ArrayFlatten(super::ArrayFlattenStep),
    StringText(super::StringTextStep),
    StringSearch(super::StringSearchStep),
    StringSplit(super::StringSplitStep),

    Instance(super::InstanceStep),
    IteratorFrom(super::IteratorFromStep),
    IteratorWrap(super::IteratorWrapStep),
    IteratorConcat(super::IteratorConcatStep),
    ArrayBuild(super::ArrayBuildStep),
    ArraySort(super::ArraySortStep),
    ArrayIndexed(super::ArrayIndexedStep),
    ArrayReverse(super::ArrayReverseStep),
    ArrayString(super::ArrayStringStep),
    RegExpExec(super::RegExpExecStep),
    RegExpPresentation(super::RegExpPresentationStep),
    RegExpReplace(super::RegExpReplaceStep),
    IteratorConsume(super::IteratorConsumeStep),
    IteratorHelper(super::IteratorHelperStep),
    IteratorCreate(super::IteratorCreateStep),

    ArrayNext(super::ArrayNextStep),
    Raw(crate::engine::vm::call::NativeInvokeOutcome),
    ArrayMutation(super::ArrayMutationStep),
    ArrayCallback(super::ArrayCallbackStep),
    ObjectIteration(super::ObjectIterationStep),
    StringReplace(super::StringReplaceStep),
    DataView(super::DataViewAccessStep),
    BufferMutation(super::BufferMutationStep),
    Invoke(super::function::invoke::InvokeStep),
    Prototype(BuiltinPrototypeStep),
    Property(PropertyStep),
    String(ObjectStringStep),
    Complete(crate::engine::vm::Completion),
    Definitions(DefinitionsStep),
    Predicate(PredicateStep),
}
impl NativeOperation {
    pub(crate) fn synchronous(
        &self,
        arguments: &[crate::engine::value::Value],
    ) -> Option<SynchronousNative> {
        match self {
            Self::Pure(target) => Some(SynchronousNative::Pure(*target)),
            Self::PrimitiveConstructor(kind)
                if matches!(kind, super::native::PrimitiveKind::Boolean)
                    || !matches!(
                        arguments.first(),
                        Some(crate::engine::value::Value::Object(_))
                    ) =>
            {
                Some(SynchronousNative::PrimitiveConstructor(*kind))
            }
            Self::Math(kind) => {
                use super::math::operation::MathKind;
                let count = match kind {
                    MathKind::Unary(_) | MathKind::Clz32 => 1,
                    MathKind::Binary(_) | MathKind::Imul => 2,
                    _ => arguments.len(),
                };
                arguments
                    .iter()
                    .take(count)
                    .all(|value| !matches!(value, crate::engine::value::Value::Object(_)))
                    .then_some(SynchronousNative::Math(*kind))
            }
            _ => None,
        }
    }

    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        if matches!(
            target,
            NativeFunctionId::ModuleEvaluation(_) | NativeFunctionId::DynamicImportHandler(_)
        ) {
            return Some(Self::ModuleCallback(target));
        }
        #[cfg(feature = "test262-host")]
        match target {
            NativeFunctionId::Test262Agent(kind) => return Some(Self::Test262Agent(kind)),
            NativeFunctionId::Test262EvalScript => return Some(Self::EvalScript),
            NativeFunctionId::Test262DetachArrayBuffer
            | NativeFunctionId::Test262CreateRealm
            | NativeFunctionId::Test262IsHtmlDda
            | NativeFunctionId::Test262Gc => return Some(Self::Pure(target)),
            _ => {}
        }
        #[cfg(test)]
        if target == NativeFunctionId::ActiveFrameProbe {
            return Some(Self::ActiveFrameProbe);
        }
        #[cfg(test)]
        if matches!(
            target,
            NativeFunctionId::ArgumentProbe
                | NativeFunctionId::ConstructorProbe
                | NativeFunctionId::ConstructorOrFunctionProbe
        ) {
            return Some(Self::Pure(target));
        }

        if matches!(
            target,
            NativeFunctionId::QjsPrint | NativeFunctionId::QjsConsoleLog
        ) {
            return Some(Self::HostOutput(target));
        }
        if matches!(
            target,
            NativeFunctionId::AsyncGeneratorPrototypeResume(_)
                | NativeFunctionId::AsyncGeneratorResume(_)
        ) {
            return Some(Self::AsyncGenerator(target));
        }
        if matches!(
            target,
            NativeFunctionId::AsyncFromSyncIteratorResume(_)
                | NativeFunctionId::AsyncFromSyncIteratorUnwrap
                | NativeFunctionId::AsyncFromSyncIteratorClose
        ) {
            return Some(Self::FromSync(target));
        }
        if let NativeFunctionId::AsyncFunctionResume(kind) = target {
            return Some(Self::AsyncResume(kind));
        }
        if matches!(
            target,
            NativeFunctionId::PromiseResolving(_)
                | NativeFunctionId::PromiseAllResolveElement
                | NativeFunctionId::PromiseAllSettledElement(_)
                | NativeFunctionId::PromiseAnyRejectElement
                | NativeFunctionId::PromiseCapabilityExecutor
                | NativeFunctionId::PromiseFinallyHandler(_)
                | NativeFunctionId::PromiseFinallyThunk(_)
                | NativeFunctionId::Promise(
                    super::native::PromiseNativeKind::Constructor
                        | super::native::PromiseNativeKind::Species
                        | super::native::PromiseNativeKind::Then
                        | super::native::PromiseNativeKind::Catch
                        | super::native::PromiseNativeKind::Resolve
                        | super::native::PromiseNativeKind::Reject
                        | super::native::PromiseNativeKind::Try
                        | super::native::PromiseNativeKind::WithResolvers
                        | super::native::PromiseNativeKind::Finally
                        | super::native::PromiseNativeKind::All
                        | super::native::PromiseNativeKind::AllSettled
                        | super::native::PromiseNativeKind::Any
                        | super::native::PromiseNativeKind::Race
                )
        ) {
            return Some(Self::Promise(target));
        }
        if let NativeFunctionId::GeneratorPrototypeResume(kind) = target {
            return Some(Self::GeneratorResume(kind));
        }
        match target {
            NativeFunctionId::RegExp(super::native::RegExpNativeKind::MatchAll) => {
                return Some(Self::RegExpMatchAll);
            }
            NativeFunctionId::RegExp(super::native::RegExpNativeKind::Split) => {
                return Some(Self::RegExpSplit);
            }
            NativeFunctionId::RegExpStringIteratorNext => return Some(Self::RegExpIterator),
            _ => {}
        }
        match target {
            NativeFunctionId::WeakRef(super::native::WeakRefNativeKind::Constructor) => {
                return Some(Self::WeakConstructor(
                    super::weak_ref::WeakIntrinsicKind::WeakRef,
                ));
            }
            NativeFunctionId::FinalizationRegistry(
                super::native::FinalizationRegistryNativeKind::Constructor,
            ) => {
                return Some(Self::WeakConstructor(
                    super::weak_ref::WeakIntrinsicKind::FinalizationRegistry,
                ));
            }
            NativeFunctionId::WeakRef(super::native::WeakRefNativeKind::Deref)
            | NativeFunctionId::FinalizationRegistry(
                super::native::FinalizationRegistryNativeKind::Register
                | super::native::FinalizationRegistryNativeKind::Unregister,
            )
            | NativeFunctionId::FunctionPrototype
            | NativeFunctionId::ThrowTypeError
            | NativeFunctionId::FunctionPrototypeFileName
            | NativeFunctionId::FunctionPrototypePosition(_) => return Some(Self::Pure(target)),
            _ => {}
        }
        match target {
            NativeFunctionId::ArrayBuffer(kind) => match kind {
                super::native::ArrayBufferNativeKind::Slice => {
                    return Some(Self::BufferSlice(
                        super::array_buffer::BufferSliceKind::Array,
                    ));
                }
                super::native::ArrayBufferNativeKind::IsView
                | super::native::ArrayBufferNativeKind::Species
                | super::native::ArrayBufferNativeKind::ByteLength
                | super::native::ArrayBufferNativeKind::MaxByteLength
                | super::native::ArrayBufferNativeKind::Resizable
                | super::native::ArrayBufferNativeKind::Detached => {
                    return Some(Self::Pure(target));
                }
                _ => {}
            },
            NativeFunctionId::SharedArrayBuffer(kind) => {
                return Some(match kind {
                    super::native::SharedArrayBufferNativeKind::Constructor => {
                        Self::SharedBufferConstructor
                    }
                    super::native::SharedArrayBufferNativeKind::Grow => Self::SharedBufferGrow,
                    super::native::SharedArrayBufferNativeKind::Slice => {
                        Self::BufferSlice(super::array_buffer::BufferSliceKind::Shared)
                    }
                    super::native::SharedArrayBufferNativeKind::Species
                    | super::native::SharedArrayBufferNativeKind::ByteLength
                    | super::native::SharedArrayBufferNativeKind::MaxByteLength
                    | super::native::SharedArrayBufferNativeKind::Growable => Self::Pure(target),
                });
            }
            NativeFunctionId::TypedArray(super::native::TypedArrayNativeKind::Uint8Codec(kind)) => {
                return Some(Self::Uint8Codec(kind));
            }
            NativeFunctionId::TypedArray(super::native::TypedArrayNativeKind::With) => {
                return Some(Self::TypedWith);
            }
            NativeFunctionId::TypedArray(super::native::TypedArrayNativeKind::ToReversed) => {
                return Some(Self::Pure(target));
            }
            _ => {}
        }
        match target {
            NativeFunctionId::TypedArray(kind) => match kind {
                super::native::TypedArrayNativeKind::Constructor(_)
                | super::native::TypedArrayNativeKind::From
                | super::native::TypedArrayNativeKind::Of => return Some(Self::TypedCreate(kind)),
                super::native::TypedArrayNativeKind::BaseConstructor
                | super::native::TypedArrayNativeKind::Species
                | super::native::TypedArrayNativeKind::Length
                | super::native::TypedArrayNativeKind::Buffer
                | super::native::TypedArrayNativeKind::ByteLength
                | super::native::TypedArrayNativeKind::ByteOffset
                | super::native::TypedArrayNativeKind::Iterator(_)
                | super::native::TypedArrayNativeKind::ToStringTag => {
                    return Some(Self::Pure(target));
                }
                _ => {}
            },
            _ => {}
        }
        match target {
            NativeFunctionId::Atomics(kind) => return Some(Self::Atomics(kind)),
            NativeFunctionId::DataView(
                super::native::DataViewNativeKind::Buffer
                | super::native::DataViewNativeKind::ByteLength
                | super::native::DataViewNativeKind::ByteOffset,
            )
            | NativeFunctionId::RegExp(
                super::native::RegExpNativeKind::Escape | super::native::RegExpNativeKind::Species,
            ) => return Some(Self::Pure(target)),
            _ => {}
        }
        if let Some(kind) = super::array_buffer::typed_array::TypedSearchKind::for_target(target) {
            return Some(Self::TypedSearch(kind));
        }
        if let Some(kind) = super::array_buffer::typed_array::TypedSliceKind::for_target(target) {
            return Some(Self::TypedSlice(kind));
        }
        if let Some(kind) = super::array_buffer::typed_array::TypedMutationKind::for_target(target)
        {
            return Some(Self::TypedMutation(kind));
        }
        if let Some(kind) = super::string::StringFactoryKind::for_target(target) {
            return Some(Self::StringFactory(kind));
        }
        match target {
            NativeFunctionId::TypedArray(super::native::TypedArrayNativeKind::Join(kind)) => {
                return Some(Self::TypedString(kind));
            }
            NativeFunctionId::TypedArray(super::native::TypedArrayNativeKind::Reverse) => {
                return Some(Self::Pure(target));
            }
            _ => {}
        }
        if let Some(kind) = super::string::StringProtocolKind::for_target(target) {
            return Some(Self::StringProtocol(kind));
        }
        match target {
            NativeFunctionId::ObjectConstructor => return Some(Self::ObjectConstructor),
            NativeFunctionId::FunctionPrototypeBind => return Some(Self::Bind),
            NativeFunctionId::FunctionPrototypeToString => return Some(Self::FunctionText),
            NativeFunctionId::FunctionConstructor(kind) => {
                return Some(Self::DynamicFunction(kind));
            }
            NativeFunctionId::GlobalEval => return Some(Self::GlobalEval),
            NativeFunctionId::Json(kind) => {
                return Some(match kind {
                    super::native::JsonNativeKind::Parse => Self::JsonParse,
                    super::native::JsonNativeKind::Stringify => Self::JsonStringify,
                    super::native::JsonNativeKind::RawJson => Self::JsonRaw,
                    super::native::JsonNativeKind::IsRawJson => Self::Pure(target),
                });
            }
            NativeFunctionId::ArrayBuffer(super::native::ArrayBufferNativeKind::Constructor) => {
                return Some(Self::BufferConstructor);
            }
            NativeFunctionId::DataView(super::native::DataViewNativeKind::Constructor) => {
                return Some(Self::DataViewConstructor);
            }
            NativeFunctionId::TypedArray(super::native::TypedArrayNativeKind::Set) => {
                return Some(Self::TypedSet);
            }
            NativeFunctionId::RegExp(kind) => match kind {
                super::native::RegExpNativeKind::Constructor => {
                    return Some(Self::RegExpConstructor);
                }
                super::native::RegExpNativeKind::Search => return Some(Self::RegExpSearch),
                super::native::RegExpNativeKind::Match => return Some(Self::RegExpMatch),
                super::native::RegExpNativeKind::Compile => return Some(Self::RegExpCompile),
                _ => {}
            },
            _ => {}
        }
        if let Some(kind) = super::math::operation::MathKind::for_target(target) {
            return Some(Self::Math(kind));
        }
        if let Some(kind) = super::primitive::globals::GlobalKind::for_target(target) {
            return Some(Self::Global(kind));
        }
        if let Some(kind) = super::primitive::numeric::NumericKind::for_target(target) {
            return Some(Self::Numeric(kind));
        }
        if let Some(kind) = super::primitive::text::ScalarTextKind::for_target(target) {
            return Some(Self::ScalarText(kind));
        }
        if let Some(kind) = super::error::operation::ErrorKind::for_target(target) {
            return Some(Self::Error(kind));
        }
        match target {
            NativeFunctionId::TypedArray(super::native::TypedArrayNativeKind::Sort) => {
                return Some(Self::TypedSort(false));
            }
            NativeFunctionId::TypedArray(super::native::TypedArrayNativeKind::ToSorted) => {
                return Some(Self::TypedSort(true));
            }
            NativeFunctionId::MathSumPrecise => return Some(Self::Sum),
            NativeFunctionId::PrimitiveConstructor(kind) => {
                return Some(Self::PrimitiveConstructor(kind));
            }
            NativeFunctionId::Date(kind) => {
                return Some(match kind {
                    super::native::DateNativeKind::Constructor
                    | super::native::DateNativeKind::Now
                    | super::native::DateNativeKind::Parse
                    | super::native::DateNativeKind::Utc => Self::DateConstructor(kind),
                    super::native::DateNativeKind::SetTime
                    | super::native::DateNativeKind::SetField(_)
                    | super::native::DateNativeKind::SetYear
                    | super::native::DateNativeKind::ToPrimitive
                    | super::native::DateNativeKind::ToJson => Self::DatePrototype(kind),
                    _ => Self::Pure(target),
                });
            }
            NativeFunctionId::MathRandom
            | NativeFunctionId::NumberPredicate(_)
            | NativeFunctionId::SymbolRegistry(super::native::SymbolRegistryKind::KeyFor)
            | NativeFunctionId::SymbolPrototypeDescription
            | NativeFunctionId::PrimitivePrototypeValueOf(_)
            | NativeFunctionId::ErrorIsError => return Some(Self::Pure(target)),
            _ => {}
        }
        if let Some(kind) = super::iterator::collection::CollectionKind::for_target(target) {
            return Some(Self::Collection(kind));
        }
        if let NativeFunctionId::Set(kind) = target
            && let Some(kind) = super::set::operations::SetOperation::from_native(kind)
        {
            return Some(Self::SetOperation(kind));
        }
        match target {
            NativeFunctionId::Map(super::native::MapNativeKind::ForEach) => {
                return Some(Self::MapCallback(super::map::callback::CallbackKind::Each));
            }
            NativeFunctionId::Map(super::native::MapNativeKind::GetOrInsert) => {
                return Some(Self::MapCallback(
                    super::map::callback::CallbackKind::Insert { computed: false },
                ));
            }
            NativeFunctionId::Map(super::native::MapNativeKind::GetOrInsertComputed) => {
                return Some(Self::MapCallback(
                    super::map::callback::CallbackKind::Insert { computed: true },
                ));
            }
            NativeFunctionId::Map(super::native::MapNativeKind::GroupBy)
            | NativeFunctionId::Set(super::native::SetNativeKind::GroupBy) => {
                return Some(Self::ObjectIteration(
                    super::object::iteration::IterationKind::MapGroup,
                ));
            }
            NativeFunctionId::Set(super::native::SetNativeKind::ForEach) => {
                return Some(Self::SetEach);
            }
            NativeFunctionId::WeakMap(super::native::WeakMapNativeKind::GetOrInsertComputed) => {
                return Some(Self::WeakComputed);
            }
            NativeFunctionId::Map(_)
            | NativeFunctionId::Set(_)
            | NativeFunctionId::WeakMap(_)
            | NativeFunctionId::WeakSet(_) => return Some(Self::Pure(target)),
            _ => {}
        }
        if let Some(kind) = super::array::indexed::IndexedKind::for_target(target) {
            return Some(Self::ArrayIndexed(kind));
        }
        if let Some(kind) = super::array::string::ArrayStringKind::for_target(target) {
            return Some(Self::ArrayString(kind));
        }
        if let Some(kind) = super::iterator::consume::ConsumeKind::for_target(target) {
            return Some(Self::IteratorConsume(kind));
        }
        if let Some(kind) = super::string::StringTextKind::for_target(target) {
            return Some(Self::StringText(kind));
        }
        if let Some(kind) = super::string::StringSearchKind::for_target(target) {
            return Some(Self::StringSearch(kind));
        }
        if let Some(kind) = super::array::slice::SliceKind::for_target(target) {
            return Some(Self::ArraySlice(kind));
        }
        if let Some(kind) = super::array_buffer::typed_array::TypedTraversalKind::for_target(target)
        {
            return Some(Self::TypedTraversal(kind));
        }
        match target {
            NativeFunctionId::ArrayConstructor => return Some(Self::ArrayConstructor),
            NativeFunctionId::IteratorConstructor => return Some(Self::IteratorConstructor),
            NativeFunctionId::IteratorConstructorAccessor => return Some(Self::IteratorAccessor),
            NativeFunctionId::IteratorPrototypeToStringTagSetter => return Some(Self::IteratorTag),
            NativeFunctionId::TypedArray(super::native::TypedArrayNativeKind::Iteration(kind)) => {
                return Some(Self::TypedIteration(kind));
            }
            NativeFunctionId::ArrayIsArray
            | NativeFunctionId::ArrayPrototypeIterator(_)
            | NativeFunctionId::IteratorPrototypeIterator
            | NativeFunctionId::IteratorPrototypeToStringTagGetter => {
                return Some(Self::Pure(target));
            }
            _ => {}
        }
        match target {
            NativeFunctionId::ArrayPrototypeConcat => return Some(Self::ArrayConcat),
            NativeFunctionId::ArrayPrototypeFlatten(kind) => return Some(Self::ArrayFlatten(kind)),
            NativeFunctionId::StringPrototypeSplit => return Some(Self::StringSplit),
            _ => {}
        }
        match target {
            NativeFunctionId::FunctionPrototypeHasInstance => return Some(Self::Instance),
            NativeFunctionId::IteratorFrom => return Some(Self::IteratorFrom),
            NativeFunctionId::IteratorWrapResume(kind) => return Some(Self::IteratorWrap(kind)),
            NativeFunctionId::IteratorConcat => {
                return Some(Self::IteratorConcat(
                    super::iterator::concat::ConcatKind::Create,
                ));
            }
            NativeFunctionId::IteratorConcatNext => {
                return Some(Self::IteratorConcat(
                    super::iterator::concat::ConcatKind::Next,
                ));
            }
            NativeFunctionId::IteratorConcatReturn => {
                return Some(Self::IteratorConcat(
                    super::iterator::concat::ConcatKind::Return,
                ));
            }
            NativeFunctionId::ArrayFrom => {
                return Some(Self::ArrayBuild(super::array::build::BuildKind::From));
            }
            NativeFunctionId::ArrayOf => {
                return Some(Self::ArrayBuild(super::array::build::BuildKind::Of));
            }
            NativeFunctionId::ArraySpeciesGetter => return Some(Self::ArraySpeciesGetter),
            _ => {}
        }
        match target {
            NativeFunctionId::ArrayPrototypeSort => return Some(Self::ArraySort(false)),
            NativeFunctionId::ArrayPrototypeToSorted => return Some(Self::ArraySort(true)),
            NativeFunctionId::ArrayPrototypeReverse => return Some(Self::ArrayReverse),
            NativeFunctionId::RegExp(
                kind @ (super::native::RegExpNativeKind::Exec
                | super::native::RegExpNativeKind::Test),
            ) => return Some(Self::RegExpExec(kind)),
            NativeFunctionId::RegExp(
                kind @ (super::native::RegExpNativeKind::Flags
                | super::native::RegExpNativeKind::Source
                | super::native::RegExpNativeKind::Flag(_)
                | super::native::RegExpNativeKind::ToString),
            ) => return Some(Self::RegExpPresentation(kind)),
            NativeFunctionId::RegExp(super::native::RegExpNativeKind::Replace) => {
                return Some(Self::RegExpReplace);
            }
            NativeFunctionId::IteratorHelperResume(kind) => {
                return Some(Self::IteratorHelper(kind));
            }
            NativeFunctionId::IteratorPrototypeCreateHelper(kind) => {
                return Some(Self::IteratorCreate(kind));
            }
            _ => {}
        }
        match target {
            NativeFunctionId::ArrayIteratorNext => return Some(Self::ArrayNext),
            NativeFunctionId::StringIteratorNext
            | NativeFunctionId::MapIteratorNext
            | NativeFunctionId::SetIteratorNext => return Some(Self::PureIterator(target)),
            _ => {}
        }
        if let Some(kind) = super::array::mutation::MutationKind::for_target(target) {
            return Some(Self::ArrayMutation(kind));
        }
        if let Some(kind) = super::array::callback::CallbackKind::for_target(target) {
            return Some(Self::ArrayCallback(kind));
        }
        if let Some(kind) = super::object::iteration::IterationKind::for_target(target) {
            return Some(Self::ObjectIteration(kind));
        }
        match target {
            NativeFunctionId::StringPrototypeReplace(kind) => {
                return Some(Self::StringReplace(kind));
            }
            NativeFunctionId::DataView(
                kind @ (super::native::DataViewNativeKind::Get(_)
                | super::native::DataViewNativeKind::Set(_)),
            ) => return Some(Self::DataView(kind)),
            NativeFunctionId::ArrayBuffer(
                kind @ (super::native::ArrayBufferNativeKind::Resize
                | super::native::ArrayBufferNativeKind::Transfer
                | super::native::ArrayBufferNativeKind::TransferToFixedLength),
            ) => return Some(Self::BufferMutation(kind)),
            _ => {}
        }
        if matches!(
            target,
            NativeFunctionId::ProxyConstructor
                | NativeFunctionId::ProxyRevocable
                | NativeFunctionId::ProxyRevoke
        ) {
            return Some(Self::Proxy(target));
        }
        if target == NativeFunctionId::ObjectPrototypeValueOf {
            return Some(Self::ObjectValueOf);
        }
        if target == NativeFunctionId::ObjectIs {
            return Some(Self::ObjectIs);
        }
        if let Some(kind) = super::function::invoke::InvokeKind::for_target(target) {
            return Some(Self::Invoke(kind));
        }
        BuiltinPrototypeKind::for_target(target)
            .map(Self::Prototype)
            .or_else(|| PropertyKind::for_target(target).map(Self::Property))
            .or_else(|| ObjectStringKind::for_target(target).map(Self::String))
            .or_else(|| DefinitionsKind::for_target(target).map(Self::Definitions))
            .or_else(|| PredicateKind::for_target(target).map(Self::Predicate))
    }
    /// Compatibility adapter; execution uses start_into to keep immediate
    /// outcomes out of the generic waiting payload.
    #[expect(
        dead_code,
        reason = "compatibility adapter for callers requiring an owned NativeStep"
    )]
    pub(crate) fn start(
        self,
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
        callable: &crate::engine::object::CallableRef,
    ) -> Result<NativeStep, RuntimeError> {
        let mut pending = None;
        match self.start_into(runtime, realm, invocation, arguments, callable, |step| {
            pending = Some(step)
        })? {
            Some(crate::engine::vm::call::NativeInvokeOutcome::Completion(completion)) => {
                Ok(NativeStep::Complete(completion))
            }
            Some(result) => Ok(NativeStep::Raw(result)),
            None => pending.ok_or(RuntimeError::Invariant(
                "native start omitted its waiting step",
            )),
        }
    }

    pub(crate) fn start_into(
        self,
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
        callable: &crate::engine::object::CallableRef,
        mut waiting: impl FnMut(NativeStep),
    ) -> Result<Option<crate::engine::vm::call::NativeInvokeOutcome>, RuntimeError> {
        let step = match self {
            #[cfg(test)]
            Self::ActiveFrameProbe => {
                NativeStep::Invoke(runtime.prepare_active_frame_probe(realm, arguments)?)
            }
            Self::AsyncGenerator(target) => NativeStep::AsyncGenerator(
                crate::engine::vm::async_generator::AsyncGeneratorStep::start(
                    runtime, realm, target, invocation, arguments,
                )?,
            ),
            Self::FromSync(target) => NativeStep::FromSync(
                crate::engine::vm::async_from_sync_iterator::FromSyncStep::start(
                    runtime, realm, target, invocation, arguments,
                )?,
            ),
            Self::AsyncResume(kind) => NativeStep::Async(runtime.start_async_function_resume(
                realm,
                kind,
                invocation.clone(),
                arguments,
            )?),
            Self::Promise(target) => {
                NativeStep::Promise(super::promise::operation::PromiseStep::start(
                    runtime, realm, target, invocation, arguments,
                )?)
            }
            Self::GeneratorResume(kind) => {
                NativeStep::GeneratorResume(runtime.start_generator_prototype_resume(
                    realm,
                    kind,
                    invocation.clone(),
                    arguments,
                )?)
            }
            Self::Atomics(kind) => {
                NativeStep::Atomics(super::AtomicsStep::start(runtime, realm, kind, arguments)?)
            }
            Self::TypedCreate(kind) => NativeStep::TypedCreate(match kind {
                super::native::TypedArrayNativeKind::Constructor(element) => {
                    super::TypedCreateStep::constructor(
                        runtime, realm, element, invocation, arguments,
                    )?
                }
                super::native::TypedArrayNativeKind::From => {
                    super::TypedCreateStep::from(runtime, realm, invocation, arguments)?
                }
                super::native::TypedArrayNativeKind::Of => {
                    super::TypedCreateStep::of(runtime, realm, invocation, arguments)?
                }
                _ => {
                    return Err(RuntimeError::Invariant(
                        "invalid TypedArray creation registration",
                    ));
                }
            }),

            Self::BufferSlice(kind) => NativeStep::BufferSlice(super::BufferSliceStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::TypedWith => NativeStep::TypedWith(super::TypedWithStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::Uint8Codec(kind) => NativeStep::Uint8Codec(super::Uint8CodecStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::SharedBufferConstructor => NativeStep::BufferConstructor(
                super::BufferConstructorStep::start_shared(runtime, realm, invocation, arguments)?,
            ),
            Self::SharedBufferGrow => NativeStep::BufferMutation(
                super::BufferMutationStep::start_grow(runtime, realm, invocation, arguments)?,
            ),

            Self::TypedSearch(kind) => NativeStep::TypedSearch(super::TypedSearchStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::TypedString(kind) => NativeStep::TypedString(super::TypedStringStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::TypedSlice(kind) => NativeStep::TypedSlice(super::TypedSliceStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::TypedMutation(kind) => NativeStep::TypedMutation(
                super::TypedMutationStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::StringFactory(kind) => NativeStep::StringFactory(
                super::StringFactoryStep::start(runtime, realm, kind, arguments)?,
            ),
            Self::WeakConstructor(kind) => NativeStep::WeakConstructor(
                super::WeakConstructorStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::RegExpMatchAll => NativeStep::RegExpMatchAll(super::RegExpMatchAllStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::RegExpSplit => NativeStep::RegExpSplit(super::RegExpSplitStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::RegExpIterator => NativeStep::RegExpIterator(super::RegExpIteratorStep::start(
                runtime, realm, invocation,
            )?),
            Self::ObjectConstructor => NativeStep::ObjectConstructor(
                super::ObjectConstructorStep::start(runtime, realm, invocation, arguments)?,
            ),
            Self::Bind => NativeStep::Bind(super::BindStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::FunctionText => NativeStep::FunctionText(super::FunctionTextStep::start(
                runtime, realm, invocation,
            )?),
            Self::DynamicFunction(kind) => NativeStep::DynamicFunction(
                super::DynamicFunctionStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::JsonParse => {
                NativeStep::JsonParse(super::JsonParseStep::start(runtime, realm, arguments)?)
            }
            Self::JsonStringify => NativeStep::JsonStringify(super::JsonStringifyStep::start(
                runtime, realm, arguments,
            )?),
            Self::BufferConstructor => NativeStep::BufferConstructor(
                super::BufferConstructorStep::start(runtime, realm, invocation, arguments)?,
            ),
            Self::DataViewConstructor => NativeStep::DataViewConstructor(
                super::DataViewConstructorStep::start(runtime, realm, invocation, arguments)?,
            ),
            Self::TypedSet => NativeStep::TypedSet(super::TypedSetStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::RegExpConstructor => NativeStep::RegExpConstructor(
                super::RegExpConstructorStep::start(runtime, realm, invocation, arguments)?,
            ),
            Self::RegExpSearch => NativeStep::RegExpSearch(super::RegExpSearchStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::RegExpMatch => NativeStep::RegExpMatch(super::RegExpMatchStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::RegExpCompile => NativeStep::RegExpCompile(super::RegExpCompileStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::StringProtocol(kind) => NativeStep::StringProtocol(
                super::StringProtocolStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::GlobalEval => NativeStep::GlobalEval(arguments.readable[0].clone()),
            Self::JsonRaw => NativeStep::JsonRaw {
                value: arguments.readable[0].clone(),
                resume: super::JsonRawResume::new(realm),
            },

            Self::TypedSort(copying) => NativeStep::TypedSort(super::TypedSortStep::start(
                runtime, realm, copying, invocation, arguments,
            )?),
            Self::Math(kind) => {
                return Ok(output::deliver(
                    super::MathStep::start(runtime, realm, kind, invocation, arguments)?,
                    &mut waiting,
                ));
            }
            Self::Sum => NativeStep::Sum(super::SumStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::PrimitiveConstructor(kind) => {
                return Ok(output::deliver(
                    super::PrimitiveConstructorStep::start(
                        runtime, realm, kind, invocation, arguments,
                    )?,
                    &mut waiting,
                ));
            }
            Self::Global(kind) => {
                return Ok(output::deliver(
                    super::GlobalStep::start(runtime, realm, kind, invocation, arguments)?
                        .advance_primitive(runtime, realm)?,
                    &mut waiting,
                ));
            }
            Self::Numeric(kind) => NativeStep::Numeric(super::NumericStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::ScalarText(kind) => NativeStep::ScalarText(super::ScalarTextStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::DateConstructor(kind) => NativeStep::DateConstructor(
                super::DateConstructorStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::DatePrototype(kind) => NativeStep::DatePrototype(
                super::DatePrototypeStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::Error(kind) => NativeStep::Error(super::ErrorStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),

            Self::MapCallback(kind) => NativeStep::MapCallback(super::MapCallbackStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::SetEach => NativeStep::SetEach(super::SetEachStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::SetOperation(kind) => NativeStep::SetOperation(super::SetOperationStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::Collection(kind) => NativeStep::Collection(super::CollectionStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::WeakComputed => NativeStep::WeakComputed(super::WeakComputedStep::start(
                runtime, realm, invocation, arguments,
            )?),

            #[cfg(feature = "test262-host")]
            Self::Test262Agent(kind) => NativeStep::Test262Agent(
                crate::engine::api::test262_agent::operation::AgentStep::start(
                    runtime,
                    realm,
                    kind,
                    invocation.clone(),
                    arguments,
                )?,
            ),
            #[cfg(feature = "test262-host")]
            Self::EvalScript => NativeStep::EvalScript(
                crate::engine::api::test262_host::operation::EvalScriptStep::start(
                    realm,
                    invocation.clone(),
                    arguments,
                )?,
            ),
            Self::ModuleCallback(target) => {
                NativeStep::ModuleCallback(crate::engine::modules::callback::CallbackStep::start(
                    runtime,
                    realm,
                    target,
                    invocation.clone(),
                    arguments,
                )?)
            }
            Self::HostOutput(target) => NativeStep::Complete(runtime.call_qjs_output(
                target,
                invocation.clone(),
                arguments,
            )?),
            Self::Pure(target) => {
                // This registered domain cannot wait. Keep its completion in
                // the small result channel, without constructing NativeStep.
                let completion = runtime.dispatch_adapted_native_function(
                    callable,
                    target,
                    realm,
                    invocation.clone(),
                    arguments,
                )?;
                return Ok(Some(
                    crate::engine::vm::call::NativeInvokeOutcome::Completion(completion),
                ));
            }
            Self::ArrayConstructor => NativeStep::ArrayConstructor(
                super::ArrayConstructorStep::start(runtime, realm, invocation, arguments)?,
            ),
            Self::ArraySlice(kind) => NativeStep::ArraySlice(super::ArraySliceStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::IteratorConstructor => NativeStep::IteratorConstructor(
                super::IteratorConstructorStep::start(runtime, realm, invocation)?,
            ),
            Self::IteratorAccessor => {
                NativeStep::IteratorConstructor(super::IteratorConstructorStep::accessor(
                    runtime, realm, callable, invocation, arguments,
                )?)
            }
            Self::IteratorTag => NativeStep::IteratorTag(super::IteratorTagStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::TypedTraversal(kind) => NativeStep::TypedTraversal(
                super::TypedTraversalStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::TypedIteration(kind) => NativeStep::TypedIteration(
                super::TypedIterationStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::ArrayConcat => NativeStep::ArrayConcat(super::ArrayConcatStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::ArrayFlatten(kind) => NativeStep::ArrayFlatten(super::ArrayFlattenStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::StringText(kind) => NativeStep::StringText(super::StringTextStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::StringSearch(kind) => NativeStep::StringSearch(super::StringSearchStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::StringSplit => NativeStep::StringSplit(super::StringSplitStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::Instance => NativeStep::Instance(super::InstanceStep::native(
                runtime, realm, invocation, arguments,
            )?),
            Self::IteratorFrom => NativeStep::IteratorFrom(super::IteratorFromStep::start(
                runtime, realm, invocation, arguments,
            )?),
            Self::IteratorWrap(kind) => NativeStep::IteratorWrap(super::IteratorWrapStep::start(
                runtime, realm, kind, invocation,
            )?),
            Self::IteratorConcat(kind) => NativeStep::IteratorConcat(
                super::IteratorConcatStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::ArrayBuild(kind) => NativeStep::ArrayBuild(super::ArrayBuildStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::ArraySpeciesGetter => {
                NativeStep::Complete(runtime.call_array_species_getter(invocation.clone())?)
            }
            Self::ArraySort(kind) => NativeStep::ArraySort(super::ArraySortStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::ArrayIndexed(kind) => NativeStep::ArrayIndexed(super::ArrayIndexedStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::ArrayReverse => NativeStep::ArrayReverse(super::ArrayReverseStep::start(
                runtime, realm, invocation,
            )?),
            Self::ArrayString(kind) => NativeStep::ArrayString(super::ArrayStringStep::start(
                runtime,
                realm,
                kind,
                invocation,
                arguments,
                crate::engine::value::JsString::MAX_LEN,
            )?),
            Self::RegExpExec(kind) => {
                return Ok(output::deliver(
                    super::RegExpExecStep::start(runtime, realm, kind, invocation, arguments)?,
                    &mut waiting,
                ));
            }
            Self::RegExpPresentation(kind) => NativeStep::RegExpPresentation(
                super::RegExpPresentationStep::start(runtime, realm, kind, invocation)?,
            ),
            Self::RegExpReplace => {
                return Ok(output::deliver(
                    super::RegExpReplaceStep::start(runtime, realm, invocation, arguments)?,
                    &mut waiting,
                ));
            }
            Self::IteratorConsume(kind) => NativeStep::IteratorConsume(
                super::IteratorConsumeStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::IteratorHelper(kind) => NativeStep::IteratorHelper(
                super::IteratorHelperStep::start(runtime, realm, kind, invocation)?,
            ),
            Self::IteratorCreate(kind) => NativeStep::IteratorCreate(
                super::IteratorCreateStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::ArrayNext => {
                return start_array_next_into(runtime, realm, invocation, &mut waiting);
            }
            Self::PureIterator(target) => NativeStep::Raw(match target {
                NativeFunctionId::StringIteratorNext => {
                    runtime.call_string_iterator_next_raw(realm, invocation.clone())?
                }
                NativeFunctionId::MapIteratorNext => {
                    runtime.call_map_iterator_next_raw(realm, invocation.clone())?
                }
                NativeFunctionId::SetIteratorNext => {
                    runtime.call_set_iterator_next_raw(realm, invocation.clone())?
                }
                _ => unreachable!("closed pure iterator registration"),
            }),
            Self::ArrayMutation(kind) => {
                return Ok(output::deliver(
                    super::ArrayMutationStep::start(runtime, realm, kind, invocation, arguments)?,
                    &mut waiting,
                ));
            }
            Self::ArrayCallback(kind) => NativeStep::ArrayCallback(
                super::ArrayCallbackStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::ObjectIteration(kind) => NativeStep::ObjectIteration(
                super::ObjectIterationStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::StringReplace(kind) => {
                return Ok(output::deliver(
                    super::StringReplaceStep::start(runtime, realm, kind, invocation, arguments)?,
                    &mut waiting,
                ));
            }
            Self::DataView(kind) => NativeStep::DataView(super::DataViewAccessStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::BufferMutation(kind) => NativeStep::BufferMutation(
                super::BufferMutationStep::start(runtime, realm, kind, invocation, arguments)?,
            ),
            Self::Proxy(target) => NativeStep::Complete(match target {
                NativeFunctionId::ProxyConstructor => {
                    runtime.call_proxy_constructor(realm, invocation.clone(), arguments)?
                }
                NativeFunctionId::ProxyRevocable => {
                    runtime.call_proxy_revocable(realm, invocation.clone(), arguments)?
                }
                NativeFunctionId::ProxyRevoke => runtime.call_proxy_revoke(invocation.clone())?,
                _ => unreachable!("closed Proxy native registration"),
            }),
            Self::Invoke(kind) => NativeStep::Invoke(super::function::invoke::InvokeStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::ObjectValueOf => NativeStep::Complete(
                runtime.call_object_prototype_value_of(realm, invocation.clone())?,
            ),
            Self::ObjectIs => {
                NativeStep::Complete(runtime.call_object_is(invocation.clone(), arguments)?)
            }
            Self::String(kind) => {
                NativeStep::String(ObjectStringStep::start(runtime, realm, kind, invocation)?)
            }
            Self::Definitions(kind) => {
                NativeStep::Definitions(DefinitionsStep::start(runtime, realm, kind, arguments)?)
            }
            Self::Predicate(kind) => NativeStep::Predicate(PredicateStep::start(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::Prototype(kind) => NativeStep::Prototype(BuiltinPrototypeStep::start_invocation(
                runtime, realm, kind, invocation, arguments,
            )?),
            Self::Property(kind) => {
                NativeStep::Property(PropertyStep::start(runtime, realm, kind, arguments)?)
            }
        };
        Ok(match step {
            NativeStep::Complete(completion) => Some(
                crate::engine::vm::call::NativeInvokeOutcome::Completion(completion),
            ),
            NativeStep::Raw(result) => Some(result),
            step => {
                waiting(step);
                None
            }
        })
    }
}

/// One Array-next domain entry shared by generic and already-classified native
/// callers. Invocation adaptation and activation ownership remain with the VM.
#[inline(never)]
pub(crate) fn start_array_next_into(
    runtime: &Runtime,
    realm: ContextId,
    invocation: &NativeInvocation,
    mut waiting: impl FnMut(NativeStep),
) -> Result<Option<crate::engine::vm::call::NativeInvokeOutcome>, RuntimeError> {
    Ok(output::deliver(
        super::ArrayNextStep::start(runtime, realm, invocation)?.advance_local(runtime, realm)?,
        &mut waiting,
    ))
}

const _: () = assert!(std::mem::size_of::<NativeStep>() <= 64);

// Inline domain payloads leave room for the NativeStep discriminant.
const _: () = assert!(std::mem::size_of::<crate::engine::modules::callback::CallbackStep>() <= 56);
const _: () =
    assert!(std::mem::size_of::<crate::engine::vm::async_generator::AsyncGeneratorStep>() <= 56);
const _: () =
    assert!(std::mem::size_of::<crate::engine::vm::async_from_sync_iterator::FromSyncStep>() <= 56);
const _: () = assert!(std::mem::size_of::<crate::engine::vm::async_function::AsyncStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::promise::operation::PromiseStep>() <= 56);
const _: () = assert!(std::mem::size_of::<crate::engine::vm::generator::GeneratorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::AtomicsStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::TypedCreateStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::BufferSliceStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::TypedWithStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::Uint8CodecStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::TypedSearchStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::TypedStringStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::TypedSliceStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::TypedMutationStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::StringFactoryStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::WeakConstructorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::RegExpMatchAllStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::RegExpSplitStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::RegExpIteratorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<crate::engine::value::Value>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ObjectConstructorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::BindStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::FunctionTextStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::DynamicFunctionStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::JsonParseStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::JsonStringifyStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::BufferConstructorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::DataViewConstructorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::TypedSetStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::RegExpConstructorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::RegExpSearchStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::RegExpMatchStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::RegExpCompileStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::StringProtocolStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::TypedSortStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::MathStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::SumStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::PrimitiveConstructorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::GlobalStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::NumericStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ScalarTextStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::DateConstructorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::DatePrototypeStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ErrorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::MapCallbackStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::SetEachStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::SetOperationStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::CollectionStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::WeakComputedStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArrayConstructorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArraySliceStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::IteratorConstructorStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::IteratorTagStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::TypedTraversalStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::TypedIterationStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArrayConcatStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArrayFlattenStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::StringTextStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::StringSearchStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::StringSplitStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::InstanceStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::IteratorFromStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::IteratorWrapStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::IteratorConcatStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArrayBuildStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArraySortStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArrayIndexedStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArrayReverseStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArrayStringStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::RegExpExecStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::RegExpPresentationStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::RegExpReplaceStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::IteratorConsumeStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::IteratorHelperStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::IteratorCreateStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArrayNextStep>() <= 56);
const _: () = assert!(std::mem::size_of::<crate::engine::vm::call::NativeInvokeOutcome>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArrayMutationStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ArrayCallbackStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::ObjectIterationStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::StringReplaceStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::DataViewAccessStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::BufferMutationStep>() <= 56);
const _: () = assert!(std::mem::size_of::<super::function::invoke::InvokeStep>() <= 56);
const _: () = assert!(std::mem::size_of::<BuiltinPrototypeStep>() <= 56);
const _: () = assert!(std::mem::size_of::<PropertyStep>() <= 56);
const _: () = assert!(std::mem::size_of::<ObjectStringStep>() <= 56);
const _: () = assert!(std::mem::size_of::<crate::engine::vm::Completion>() <= 56);
const _: () = assert!(std::mem::size_of::<DefinitionsStep>() <= 56);
const _: () = assert!(std::mem::size_of::<PredicateStep>() <= 56);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<NativeStep>() <= 64);
