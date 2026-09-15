use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{
    DateGetFieldKind, DateNativeKind, DateSetFieldKind, DateStringMethod, NativeFunctionId,
};
use crate::engine::code::bytecode_publish;
use crate::engine::code::function::UnlinkedFunction;
use crate::source::QuickJsSourceLocator;

use crate::engine::api::compile::Compilation;
use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariableKind, ClosureVariableName,
};
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::compiler::DEFAULT_EVAL_FILENAME;
use crate::engine::heap::roots::VarRefRoot;

use crate::engine::heap::{AutoInitProperty, ContextId, PropertySlot};
#[cfg(test)]
use crate::engine::host::HostServices;
use crate::engine::object::operations::{InternalSetResult, PropertySetRejection};
use crate::engine::object::shape::PropertyFlags;
use crate::engine::object::{
    CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
    WellKnownSymbol,
};
use crate::engine::realm::bindings::GlobalBindingCreationMode;
use crate::engine::value::conversion::NativeConversion;

use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation};
use crate::engine::vm::frames::ExplicitBacktraceLocation;

mod array;
mod array_buffer;
mod atomics;
pub(crate) use array_buffer::typed_array::CanonicalNumericIndex;
pub(crate) use array_buffer::typed_array::write::TypedWriteStep;
#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{
    element::{ElementResume, ElementStep},
    write::TypedWriteResume,
};
pub(crate) mod date;
mod error;
mod eval;
#[cfg(feature = "stack-vm")]
pub(crate) use eval::DirectEvalPreparation;
#[cfg(feature = "stack-vm")]
pub(crate) mod continuation;
#[cfg(feature = "stack-vm")]
pub(crate) use function::arguments::{ArgumentsResume, ArgumentsStep};
#[cfg(feature = "stack-vm")]
pub(crate) use function::invoke::{InvokeResume, InvokeStep};
mod iterator;
mod json;
mod map;
mod math;
mod object;
#[cfg(feature = "stack-vm")]
pub(crate) use object::definitions::{DefinitionsResume, DefinitionsStep};
#[cfg(feature = "stack-vm")]
pub(crate) use object::predicate::{PredicateResume, PredicateStep};
#[cfg(feature = "stack-vm")]
pub(crate) use object::property::{PropertyResume, PropertyStep};
#[cfg(feature = "stack-vm")]
pub(crate) use object::prototype::{BuiltinPrototypeResume, BuiltinPrototypeStep};
#[cfg(feature = "stack-vm")]
pub(crate) use object::string::{ObjectStringResume, ObjectStringStep};
pub(crate) mod promise;
mod proxy;
mod reflect;
mod regexp;
mod replacement;
mod set;
mod shared_array_buffer;
mod string;
pub mod uri;
mod weak_collection;
mod weak_ref;

/// Pinned QuickJS `JS_ToInt64Free` for an already numeric value.
///
/// Rust's float-to-integer cast saturates outside the signed range. QuickJS
/// instead preserves the low 64 bits while the binary exponent remains close
/// enough to the mantissa, and maps still larger magnitudes to zero.
fn quickjs_to_int64_free(number: f64) -> i64 {
    const EXPONENT_BIAS: u64 = 1023;
    const MANTISSA_BITS: u64 = 52;
    const MANTISSA_MASK: u64 = (1_u64 << MANTISSA_BITS) - 1;

    let bits = number.to_bits();
    let exponent = (bits >> MANTISSA_BITS) & 0x7ff;
    if exponent <= EXPONENT_BIAS + 62 {
        return number as i64;
    }
    if exponent <= EXPONENT_BIAS + 62 + 53 {
        let significand = (bits & MANTISSA_MASK) | (1_u64 << MANTISSA_BITS);
        let shift = u32::try_from(exponent - EXPONENT_BIAS - MANTISSA_BITS)
            .expect("QuickJS ToInt64 exponent shift fits u32");
        let signed = (significand << shift) as i64;
        return if bits >> 63 == 0 {
            signed
        } else {
            signed.wrapping_neg()
        };
    }
    0
}

impl Runtime {
    /// Perform ordinary throwing Set for builtin algorithms which publish
    /// values through `[[Set]]` rather than CreateDataProperty.
    pub(crate) fn set_property_or_throw(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        self.finish_set_property_or_throw(
            realm,
            key,
            self.internal_set(realm, object, key, value, Value::Object(object.clone()))?,
        )
    }

    pub(crate) fn finish_set_property_or_throw(
        &self,
        realm: ContextId,
        key: &PropertyKey,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<Option<Value>, RuntimeError> {
        match result {
            NativeConversion::Value(InternalSetResult::Accepted) => Ok(None),
            NativeConversion::Throw(value) => Ok(Some(value)),
            NativeConversion::Value(result) => {
                let error = match result {
                    InternalSetResult::RejectedProxyTrap => {
                        Error::new(ErrorKind::Type, "proxy: cannot set property")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::ReadOnly) => {
                        self.native_atom_error(ErrorKind::Type, "'", key, "' is read-only")?
                    }
                    InternalSetResult::Rejected(PropertySetRejection::ArrayLengthReadOnly) => {
                        let length = self.intern_property_key("length")?;
                        self.native_atom_error(ErrorKind::Type, "'", &length, "' is read-only")?
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NotConfigurable) => {
                        Error::new(ErrorKind::Type, "not configurable")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NoSetter) => {
                        Error::new(ErrorKind::Type, "no setter for property")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NotExtensible) => {
                        Error::new(ErrorKind::Type, "object is not extensible")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NotObject) => {
                        Error::new(ErrorKind::Type, "not an object")
                    }
                    InternalSetResult::Accepted => unreachable!("accepted Set returned above"),
                };
                Ok(Some(self.new_native_error_from_error(
                    realm,
                    NativeErrorKind::Type,
                    &error,
                )?))
            }
        }
    }
}

pub(crate) mod dispatch;

pub(crate) mod buffer_access;

pub(crate) mod qjs_host;

pub(crate) mod qjs_value_printer;

pub(crate) mod function;

pub(crate) mod primitive;

pub(crate) mod native;

#[cfg(feature = "stack-vm")]
pub(crate) use array::callback::{
    CallbackResume as ArrayCallbackResume, CallbackStep as ArrayCallbackStep,
};
#[cfg(feature = "stack-vm")]
pub(crate) use array::mutation::{
    MutationResume as ArrayMutationResume, MutationStep as ArrayMutationStep,
};
#[cfg(feature = "stack-vm")]
pub(crate) use array::species::{
    SpeciesResume as ArraySpeciesResume, SpeciesStep as ArraySpeciesStep,
};
#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::{
    BufferMutationResume, BufferMutationStep, DataViewAccessResume, DataViewAccessStep,
};
pub(crate) use iterator::step::CloseStep as IteratorCloseStep;
#[cfg(not(feature = "stack-vm"))]
pub(crate) use iterator::step::finish_close as finish_iterator_close;
#[cfg(feature = "stack-vm")]
pub(crate) use iterator::step::{
    CloseResume as IteratorCloseResume, NextResume as IteratorNextResume,
    NextStep as IteratorNextStep,
};
#[cfg(feature = "stack-vm")]
pub(crate) use object::iteration::{
    IterationResume as ObjectIterationResume, IterationStep as ObjectIterationStep,
};
#[cfg(feature = "stack-vm")]
pub(crate) use string::{StringReplaceResume, StringReplaceStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array::mutation::MutationKind as ArrayMutationKind;

#[cfg(feature = "stack-vm")]
pub(crate) use iterator::array::{ArrayNextResume, ArrayNextStep};
#[cfg(feature = "stack-vm")]
pub(crate) use object::ObjectIteratorStep;

#[cfg(feature = "stack-vm")]
pub(crate) use array::sort::{SortResume as ArraySortResume, SortStep as ArraySortStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array::indexed::{
    IndexedResume as ArrayIndexedResume, IndexedStep as ArrayIndexedStep,
};

#[cfg(feature = "stack-vm")]
pub(crate) use array::reverse::{
    ReverseResume as ArrayReverseResume, ReverseStep as ArrayReverseStep,
};

#[cfg(feature = "stack-vm")]
pub(crate) use array::string::{ArrayStringResume, ArrayStringStep};

#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{RegExpExecResume, RegExpExecStep};

#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{RegExpPresentationResume, RegExpPresentationStep};

#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{RegExpReplaceResume, RegExpReplaceStep};

#[cfg(feature = "stack-vm")]
pub(crate) use iterator::consume::{
    ConsumeResume as IteratorConsumeResume, ConsumeStep as IteratorConsumeStep,
};

#[cfg(feature = "stack-vm")]
pub(crate) use iterator::helper::{
    HelperResume as IteratorHelperResume, HelperResumeStep as IteratorHelperStep,
};

#[cfg(feature = "stack-vm")]
pub(crate) use iterator::create::{
    CreateResume as IteratorCreateResume, CreateStep as IteratorCreateStep,
};

#[cfg(feature = "stack-vm")]
pub(crate) use object::string::ObjectStringKind;

#[cfg(feature = "stack-vm")]
pub(crate) use array::build::{BuildResume as ArrayBuildResume, BuildStep as ArrayBuildStep};
#[cfg(feature = "stack-vm")]
pub(crate) use function::instance::{InstanceResume, InstanceStep};
#[cfg(feature = "stack-vm")]
pub(crate) use iterator::concat::{
    ConcatResume as IteratorConcatResume, ConcatStep as IteratorConcatStep,
};
#[cfg(feature = "stack-vm")]
pub(crate) use iterator::from::{FromResume as IteratorFromResume, FromStep as IteratorFromStep};
#[cfg(feature = "stack-vm")]
pub(crate) use iterator::wrap::{WrapResume as IteratorWrapResume, WrapStep as IteratorWrapStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array::copy::{CopyResume as ArrayCopyResume, CopyStep as ArrayCopyStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array::concat::{ConcatResume as ArrayConcatResume, ConcatStep as ArrayConcatStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array::flatten::{
    FlattenResume as ArrayFlattenResume, FlattenStep as ArrayFlattenStep,
};

#[cfg(feature = "stack-vm")]
pub(crate) use object::copy::{CopyResume as ObjectCopyResume, CopyStep as ObjectCopyStep};

#[cfg(feature = "stack-vm")]
pub(crate) use string::{StringTextResume, StringTextStep};

#[cfg(feature = "stack-vm")]
pub(crate) use string::{StringSearchResume, StringSearchStep};

#[cfg(feature = "stack-vm")]
pub(crate) use string::{StringSplitResume, StringSplitStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array::constructor::{
    ConstructorResume as ArrayConstructorResume, ConstructorStep as ArrayConstructorStep,
};

#[cfg(feature = "stack-vm")]
pub(crate) use array::slice::{SliceResume as ArraySliceResume, SliceStep as ArraySliceStep};

#[cfg(feature = "stack-vm")]
pub(crate) use iterator::constructor::{
    ConstructorResume as IteratorConstructorResume, ConstructorStep as IteratorConstructorStep,
};

#[cfg(feature = "stack-vm")]
pub(crate) use iterator::entry::{
    TagSetterResume as IteratorTagResume, TagSetterStep as IteratorTagStep,
};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedTraversalResume, TypedTraversalStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedSpeciesResume, TypedSpeciesStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedIterationResume, TypedIterationStep};

#[cfg(feature = "stack-vm")]
pub(crate) use iterator::collection::{CollectionResume, CollectionStep};
#[cfg(feature = "stack-vm")]
pub(crate) use map::callback::{
    CallbackResume as MapCallbackResume, CallbackStep as MapCallbackStep,
};
#[cfg(feature = "stack-vm")]
pub(crate) use set::callback::{EachResume as SetEachResume, EachStep as SetEachStep};
#[cfg(feature = "stack-vm")]
pub(crate) use set::operations::{SetResume as SetOperationResume, SetStep as SetOperationStep};
#[cfg(feature = "stack-vm")]
pub(crate) use weak_collection::computed::{
    ComputedResume as WeakComputedResume, ComputedStep as WeakComputedStep,
};

#[cfg(feature = "stack-vm")]
pub(crate) use math::operation::{MathResume, MathStep};

#[cfg(feature = "stack-vm")]
pub(crate) use math::sum::{SumResume, SumStep};

#[cfg(feature = "stack-vm")]
pub(crate) use primitive::constructor::{PrimitiveConstructorResume, PrimitiveConstructorStep};

#[cfg(feature = "stack-vm")]
pub(crate) use primitive::globals::{GlobalResume, GlobalStep};

#[cfg(feature = "stack-vm")]
pub(crate) use primitive::numeric::{NumericResume, NumericStep};

#[cfg(feature = "stack-vm")]
pub(crate) use primitive::text::{ScalarTextResume, ScalarTextStep};

#[cfg(feature = "stack-vm")]
pub(crate) use date::{DateConstructorResume, DateConstructorStep};

#[cfg(feature = "stack-vm")]
pub(crate) use date::{DatePrototypeResume, DatePrototypeStep};

#[cfg(feature = "stack-vm")]
pub(crate) use error::operation::{ErrorResume, ErrorStep};

#[cfg(feature = "stack-vm")]
pub(crate) use error::aggregate::{AggregateResume, AggregateStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedSortResume, TypedSortStep};

#[cfg(feature = "stack-vm")]
pub(crate) use object::constructor::{ObjectConstructorResume, ObjectConstructorStep};

#[cfg(feature = "stack-vm")]
pub(crate) use function::bind::{BindResume, BindStep};

#[cfg(feature = "stack-vm")]
pub(crate) use function::text::{FunctionTextResume, FunctionTextStep};

#[cfg(feature = "stack-vm")]
pub(crate) use function::dynamic::{DynamicFunctionResume, DynamicFunctionStep};

#[cfg(feature = "stack-vm")]
pub(crate) use json::{JsonParseResume, JsonParseStep};

#[cfg(feature = "stack-vm")]
pub(crate) use json::{JsonStringifyResume, JsonStringifyStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::{BufferConstructorResume, BufferConstructorStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::{DataViewConstructorResume, DataViewConstructorStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedSetResume, TypedSetStep};

#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{RegExpConstructorResume, RegExpConstructorStep};

#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{RegExpSearchResume, RegExpSearchStep};

#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{RegExpMatchResume, RegExpMatchStep};

#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{RegExpCompileResume, RegExpCompileStep};

#[cfg(feature = "stack-vm")]
pub(crate) use string::{StringProtocolResume, StringProtocolStep};

#[cfg(feature = "stack-vm")]
pub(crate) use json::JsonRawResume;

#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{RegExpMatchAllResume, RegExpMatchAllStep};

#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{RegExpSplitResume, RegExpSplitStep};

#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{RegExpIteratorResume, RegExpIteratorStep};

#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{RegExpSpeciesResume, RegExpSpeciesStep};

#[cfg(feature = "stack-vm")]
pub(crate) use weak_ref::constructor::{WeakConstructorResume, WeakConstructorStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedSearchResume, TypedSearchStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedStringResume, TypedStringStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedSliceResume, TypedSliceStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedMutationResume, TypedMutationStep};

#[cfg(feature = "stack-vm")]
pub(crate) use string::{StringFactoryResume, StringFactoryStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::{BufferSliceResume, BufferSliceStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedWithResume, TypedWithStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{Uint8CodecResume, Uint8CodecStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedCreateResume, TypedCreateStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedCollectResume, TypedCollectStep};

#[cfg(feature = "stack-vm")]
pub(crate) use array_buffer::typed_array::{TypedIteratorMethodResume, TypedIteratorMethodStep};

#[cfg(feature = "stack-vm")]
pub(crate) use atomics::{AtomicsResume, AtomicsStep};
