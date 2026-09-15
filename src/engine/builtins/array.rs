//! Array constructor, prototype, iterator, and sorting intrinsics.

use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{
    ArrayFindKind, ArrayFlattenKind, ArrayIterationKind, ArrayIteratorKind, ArrayJoinKind,
    ArrayPopKind, ArrayPushKind, ArrayReduceKind, ArraySearchKind, ArraySliceKind,
    NativeFunctionId,
};
use crate::engine::heap::{AutoInitProperty, ContextId, ObjectData, PropertySlot};
use crate::engine::object::builtin_properties::NativeBuiltinProperty;
use crate::engine::object::operations::{
    ArrayLengthConversion, ArrayOwnKey, InternalDefineResult, PropertyDefineOutcome,
};
use crate::engine::object::shape::{PropertyFlags, ShapeEntry};
use crate::engine::object::{
    AccessorValue, CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor,
    PropertyKey, WellKnownSymbol,
};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation, NativeInvokeOutcome};
use std::cmp::Ordering as ComparisonOrdering;

pub(crate) mod build;
pub(crate) mod callback;
pub(crate) mod concat;
pub(crate) mod constructor;
pub(crate) mod copy;
pub(crate) mod flatten;
pub(crate) mod indexed;
pub(crate) mod mutation;
pub(crate) mod reverse;
pub(in crate::engine::builtins) mod rqsort;
pub(crate) mod slice;
pub(crate) mod sort;
pub(crate) mod species;
pub(crate) mod string;

struct ArraySortSlot {
    value: Value,
    cached_string: Option<JsString>,
    original_position: u64,
}

struct ArrayFlattenFrame {
    source: ObjectRef,
    length: u64,
    next_index: u64,
    depth: i32,
    apply_mapper: bool,
}

// The pinned macOS oracle accepts roughly 3.8k nested flatten frames before
// its C-stack guard fires. Keep the Rust traversal iterative and use the
// nearest stable probe boundary so the completion stays catchable.
const ARRAY_FLATTEN_FRAME_LIMIT: usize = 3_833;

/// Port of QuickJS's `rqsort` index choreography. The comparator receives
/// mutable access to the backing slice because Array's default comparison
/// caches each element's ToString result inside its moving sort slot.
#[cfg(test)]
pub(in crate::engine::builtins) fn quickjs_rqsort_by<T, E>(
    values: &mut [T],
    compare: impl FnMut(&mut [T], usize, usize) -> Result<ComparisonOrdering, E>,
) -> Result<(), E> {
    let length = values.len();
    let mut accessor = SliceSortAccessor { values, compare };
    quickjs_rqsort_with(length, &mut accessor)
}

/// Storage adapter for the shared `rqsort` choreography. Array sorts moving
/// value slots, while TypedArray's default path compares and swaps raw backing
/// words in place without allocating a second element buffer.
pub(in crate::engine::builtins) trait QuickJsSortAccessor {
    type Error;

    fn compare(&mut self, left: usize, right: usize) -> Result<ComparisonOrdering, Self::Error>;

    fn swap(&mut self, left: usize, right: usize) -> Result<(), Self::Error>;
}

#[cfg(test)]
struct SliceSortAccessor<'a, T, F> {
    values: &'a mut [T],
    compare: F,
}

#[cfg(test)]
impl<T, E, F> QuickJsSortAccessor for SliceSortAccessor<'_, T, F>
where
    F: FnMut(&mut [T], usize, usize) -> Result<ComparisonOrdering, E>,
{
    type Error = E;

    fn compare(&mut self, left: usize, right: usize) -> Result<ComparisonOrdering, E> {
        (self.compare)(self.values, left, right)
    }

    fn swap(&mut self, left: usize, right: usize) -> Result<(), E> {
        self.values.swap(left, right);
        Ok(())
    }
}

pub(in crate::engine::builtins) fn quickjs_rqsort_with<A>(
    length: usize,
    accessor: &mut A,
) -> Result<(), A::Error>
where
    A: QuickJsSortAccessor,
{
    let mut machine = rqsort::SortMachine::new(length);
    let mut reply = None;
    loop {
        match machine.advance(reply.take()) {
            rqsort::SortAction::Complete => return Ok(()),
            rqsort::SortAction::Compare(left, right) => {
                reply = Some(accessor.compare(left, right)?)
            }
            rqsort::SortAction::Swap(left, right) => accessor.swap(left, right)?,
        }
    }
}

impl Runtime {
    fn new_array_iterator(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        kind: ArrayIteratorKind,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Array Iterator target"));
        }
        let prototype_id = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .array_iterator_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype_id)?;
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let iterator = match state.heap.allocate_object(ObjectData::array_iterator(
            shape,
            Vec::new(),
            object.object_id(),
            kind,
        )) {
            Ok(iterator) => iterator,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), iterator))
    }

    pub(crate) fn initialize_array_intrinsics(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        array_prototype: &ObjectRef,
        array_iterator_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let mut methods = vec![
            NativeBuiltinProperty::new(NativeFunctionId::ArrayPrototypeAt, "at", 1, 1),
            NativeBuiltinProperty::new(NativeFunctionId::ArrayPrototypeWith, "with", 2, 2),
            NativeBuiltinProperty::new(NativeFunctionId::ArrayPrototypeConcat, "concat", 1, 0),
        ];
        for (kind, name) in [
            (ArrayIterationKind::Every, "every"),
            (ArrayIterationKind::Some, "some"),
            (ArrayIterationKind::ForEach, "forEach"),
            (ArrayIterationKind::Map, "map"),
            (ArrayIterationKind::Filter, "filter"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::ArrayPrototypeIteration(kind),
                name,
                1,
                1,
            ));
        }
        for (kind, name) in [
            (ArrayReduceKind::Reduce, "reduce"),
            (ArrayReduceKind::ReduceRight, "reduceRight"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::ArrayPrototypeReduce(kind),
                name,
                1,
                1,
            ));
        }
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::ArrayPrototypeFill,
            "fill",
            1,
            1,
        ));
        for (kind, name) in [
            (ArrayFindKind::Find, "find"),
            (ArrayFindKind::FindIndex, "findIndex"),
            (ArrayFindKind::FindLast, "findLast"),
            (ArrayFindKind::FindLastIndex, "findLastIndex"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::ArrayPrototypeFind(kind),
                name,
                1,
                1,
            ));
        }
        for (kind, name) in [
            (ArraySearchKind::IndexOf, "indexOf"),
            (ArraySearchKind::LastIndexOf, "lastIndexOf"),
            (ArraySearchKind::Includes, "includes"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::ArrayPrototypeSearch(kind),
                name,
                1,
                1,
            ));
        }
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::ArrayPrototypeJoin(ArrayJoinKind::Join),
            "join",
            1,
            1,
        ));
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::ArrayPrototypeToString,
            "toString",
            0,
            0,
        ));
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::ArrayPrototypeJoin(ArrayJoinKind::ToLocaleString),
            "toLocaleString",
            0,
            0,
        ));
        for (target, name, length) in [
            (
                NativeFunctionId::ArrayPrototypePop(ArrayPopKind::Pop),
                "pop",
                0,
            ),
            (
                NativeFunctionId::ArrayPrototypePush(ArrayPushKind::Push),
                "push",
                1,
            ),
            (
                NativeFunctionId::ArrayPrototypePop(ArrayPopKind::Shift),
                "shift",
                0,
            ),
            (
                NativeFunctionId::ArrayPrototypePush(ArrayPushKind::Unshift),
                "unshift",
                1,
            ),
        ] {
            methods.push(NativeBuiltinProperty::new(target, name, length, 0));
        }
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::ArrayPrototypeReverse,
            "reverse",
            0,
            0,
        ));
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::ArrayPrototypeToReversed,
            "toReversed",
            0,
            0,
        ));
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::ArrayPrototypeSort,
            "sort",
            1,
            1,
        ));
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::ArrayPrototypeToSorted,
            "toSorted",
            1,
            1,
        ));
        for (kind, name) in [
            (ArraySliceKind::Slice, "slice"),
            (ArraySliceKind::Splice, "splice"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::ArrayPrototypeSlice(kind),
                name,
                2,
                2,
            ));
        }
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::ArrayPrototypeToSpliced,
            "toSpliced",
            2,
            2,
        ));
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::ArrayPrototypeCopyWithin,
            "copyWithin",
            2,
            2,
        ));
        for (kind, name, length, min_readable_args) in [
            (ArrayFlattenKind::FlatMap, "flatMap", 1, 1),
            (ArrayFlattenKind::Flat, "flat", 0, 0),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::ArrayPrototypeFlatten(kind),
                name,
                length,
                min_readable_args,
            ));
        }
        for (kind, name) in [
            (ArrayIteratorKind::Value, "values"),
            (ArrayIteratorKind::Key, "keys"),
            (ArrayIteratorKind::KeyAndValue, "entries"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::ArrayPrototypeIterator(kind),
                name,
                0,
                0,
            ));
        }

        self.define_native_builtin_auto_init_batch(array_prototype, realm, methods)?;

        self.define_native_builtin_auto_init(
            array_iterator_prototype,
            realm,
            NativeFunctionId::ArrayIteratorNext,
            "next",
            0,
            0,
        )?;
        let tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            array_iterator_prototype,
            &tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static(
                    "Array Iterator",
                ))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Array Iterator toStringTag definition was rejected",
            ));
        }

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ArrayConstructor,
            1,
            "Array",
            1,
        )?;
        let methods = [
            NativeBuiltinProperty::new(NativeFunctionId::ArrayIsArray, "isArray", 1, 1),
            NativeBuiltinProperty::new(NativeFunctionId::ArrayFrom, "from", 1, 3),
            NativeBuiltinProperty::new(NativeFunctionId::ArrayOf, "of", 0, 0),
        ];
        self.define_native_builtin_auto_init_batch(constructor.as_object(), realm, methods)?;

        self.define_constructor_relationship(&constructor, array_prototype)?;

        let getter = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ArraySpeciesGetter,
            0,
            "get [Symbol.species]",
            0,
        )?;
        let species = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Species));
        if !self.define_own_property(
            constructor.as_object(),
            &species,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(getter)),
                set: DescriptorField::Present(AccessorValue::Undefined),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Array species definition was rejected",
            ));
        }

        let values = self.intern_property_key("values")?;
        let values = match self.get_property_in_realm(realm, array_prototype, &values)? {
            Completion::Return(value @ Value::Object(_)) => value,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "Array.prototype.values was not callable during alias bootstrap",
                ));
            }
            Completion::Throw(_) => {
                return Err(RuntimeError::Invariant(
                    "Array.prototype.values initialization threw during bootstrap",
                ));
            }
        };
        let Value::Object(values_object) = &values else {
            unreachable!("Array.prototype.values bootstrap validated an object value")
        };
        self.0
            .state
            .borrow_mut()
            .heap
            .attach_array_prototype_values(realm, values_object.object_id())?;
        let iterator = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        if !self.define_own_property(
            array_prototype,
            &iterator,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(values),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Array iterator alias definition was rejected",
            ));
        }
        self.define_array_unscopables_auto_init(array_prototype, realm)?;

        self.define_function_data_property(
            global_object,
            "Array",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.0
            .state
            .borrow_mut()
            .heap
            .attach_array_constructor(realm, constructor.as_object().object_id())?;
        Ok(())
    }

    fn define_array_unscopables_auto_init(
        &self,
        array_prototype: &ObjectRef,
        realm: ContextId,
    ) -> Result<(), RuntimeError> {
        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Unscopables));
        self.validate_object_and_key(array_prototype, &key)?;
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let object_id = array_prototype.object_id();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            if shape.find(key.atom()).is_some() {
                return Err(RuntimeError::Invariant(
                    "Array unscopables autoinit property already exists",
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
            flags: PropertyFlags::data(false, false, true),
        });
        slots.push(PropertySlot::AutoInit(AutoInitProperty::ArrayUnscopables {
            realm,
        }));
        state.replace_layout(object_id, prototype, &entries, slots)
    }

    pub(crate) fn instantiate_array_unscopables(
        &self,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let object = self.new_object(None)?;
        for name in [
            "at",
            "copyWithin",
            "entries",
            "fill",
            "find",
            "findIndex",
            "findLast",
            "findLastIndex",
            "flat",
            "flatMap",
            "includes",
            "keys",
            "toReversed",
            "toSorted",
            "toSpliced",
            "values",
        ] {
            let key = self.intern_property_key(name)?;
            if !self.define_own_property(
                &object,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Bool(true)),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )? {
                return Err(RuntimeError::Invariant(
                    "Array unscopables property definition was rejected",
                ));
            }
        }
        Ok(object)
    }

    pub(crate) fn call_array_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        constructor::finish(
            self,
            realm,
            constructor::ConstructorStep::start(self, realm, &invocation, arguments)?,
        )
    }

    fn array_constructor_length(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<ArrayLengthConversion, RuntimeError> {
        match value {
            Value::Int(value) if *value >= 0 => Ok(ArrayLengthConversion::Length(*value as u32)),
            Value::Float(value) => self.validate_array_length_number(Some(realm), *value, None),
            Value::Int(_) => self.invalid_array_length(Some(realm)),
            _ => Err(RuntimeError::Invariant(
                "Array constructor length validator received a non-number",
            )),
        }
    }

    pub(crate) fn create_array_data_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u32,
        value: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        self.create_indexed_data_property(realm, object, u64::from(index), value)
    }

    /// QuickJS `JS_DefinePropertyValueUint32` without `JS_PROP_THROW`: an
    /// ordinary `false` result is ignored, while an actual JavaScript throw is
    /// still returned to the caller. Promise aggregate element handlers use
    /// this form, which is observably distinct after a custom capability
    /// exposes and freezes their output Array early.
    pub(crate) fn define_array_data_property_without_throw(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u32,
        value: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        match self.define_indexed_data_property(realm, object, u64::from(index), value)? {
            NativeConversion::Value(_) => Ok(None),
            NativeConversion::Throw(value) => Ok(Some(value)),
        }
    }

    /// Array construction uses ordinary `Set`, not CreateDataProperty. A
    /// custom `newTarget.prototype` can therefore intercept an element with
    /// an inherited setter or reject it with a fixed data/accessor property.

    fn create_indexed_data_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u64,
        value: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        self.finish_create_indexed_data_property(
            realm,
            index,
            self.define_indexed_data_property(realm, object, index, value)?,
        )
    }

    fn finish_create_indexed_data_property(
        &self,
        realm: ContextId,
        index: u64,
        result: NativeConversion<InternalDefineResult>,
    ) -> Result<Option<Value>, RuntimeError> {
        match result {
            NativeConversion::Value(InternalDefineResult::Defined) => Ok(None),
            NativeConversion::Value(InternalDefineResult::RejectedProxyTrap) => {
                let error = Error::new(ErrorKind::Type, "proxy: defineProperty exception");
                Ok(Some(self.new_native_error_from_error(
                    realm,
                    NativeErrorKind::Type,
                    &error,
                )?))
            }
            NativeConversion::Value(InternalDefineResult::RejectedOrdinary(target)) => {
                let key = self.property_key_for_index(index)?;
                let array_length_read_only =
                    if let ArrayOwnKey::Index(index) = self.array_own_key(&target, &key)? {
                        let (length, writable) = self.array_length_state(&target)?;
                        index >= length && !writable
                    } else {
                        false
                    };
                let error = if array_length_read_only {
                    let length = self.intern_property_key("length")?;
                    self.native_atom_error(ErrorKind::Type, "'", &length, "' is read-only")?
                } else if !self.has_own_property(&target, &key)? && !self.is_extensible(&target)? {
                    Error::new(ErrorKind::Type, "object is not extensible")
                } else {
                    Error::new(ErrorKind::Type, "property is not configurable")
                };
                Ok(Some(self.new_native_error_from_error(
                    realm,
                    NativeErrorKind::Type,
                    &error,
                )?))
            }
            NativeConversion::Throw(value) => Ok(Some(value)),
        }
    }

    fn define_indexed_data_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u64,
        value: Value,
    ) -> Result<NativeConversion<InternalDefineResult>, RuntimeError> {
        let key = self.property_key_for_index(index)?;
        let descriptor = OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(value),
            writable: DescriptorField::Present(true),
            enumerable: DescriptorField::Present(true),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };
        self.internal_define_own_property(realm, object, &key, &descriptor)
    }

    pub(crate) fn call_array_is_array(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.isArray did not receive a generic invocation",
            ));
        };
        let result = match arguments.readable.first() {
            Some(value) => match self.internal_is_array(realm, value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
            None => false,
        };
        Ok(Completion::Return(Value::Bool(result)))
    }

    pub(crate) fn call_array_species_getter(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array species getter did not receive a getter invocation",
            ));
        };
        Ok(Completion::Return(this_value))
    }

    pub(crate) fn call_array_from(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        build::finish(
            self,
            realm,
            build::BuildStep::start(self, realm, build::BuildKind::From, &invocation, arguments)?,
        )
    }

    fn new_array_with_length(
        &self,
        realm: ContextId,
        length: Option<Value>,
    ) -> Result<Completion, RuntimeError> {
        let array = self.new_array(realm)?;
        if let Some(length) = length {
            let length = match self.array_constructor_length(realm, &length)? {
                ArrayLengthConversion::Length(length) => length,
                ArrayLengthConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let key = self.intern_property_key("length")?;
            match self.define_own_property_in_realm(
                Some(realm),
                &array,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Self::array_length_value(length)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )? {
                PropertyDefineOutcome::Defined(true) => {}
                PropertyDefineOutcome::Defined(false) => {
                    return Err(RuntimeError::Invariant(
                        "fresh Array.from result rejected its length",
                    ));
                }
                PropertyDefineOutcome::Throw(value) => return Ok(Completion::Throw(value)),
            }
        }
        Ok(Completion::Return(Value::Object(array)))
    }

    pub(crate) fn close_iterator_preserving_throw(
        &self,
        realm: ContextId,
        iterator: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        super::iterator::step::finish_close(
            self,
            realm,
            super::iterator::step::CloseStep::start(
                self,
                realm,
                iterator.clone(),
                Completion::Throw(Value::Undefined),
            )?,
        )?;
        Ok(())
    }

    pub(crate) fn call_array_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        build::finish(
            self,
            realm,
            build::BuildStep::start(self, realm, build::BuildKind::Of, &invocation, arguments)?,
        )
    }

    pub(crate) fn call_array_prototype_at(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        indexed::finish(
            self,
            realm,
            indexed::IndexedStep::start(
                self,
                realm,
                indexed::IndexedKind::At,
                &invocation,
                arguments,
            )?,
        )
    }

    fn native_allocate_fast_array_values(
        &self,
        realm: ContextId,
        length: u64,
    ) -> Result<NativeConversion<Vec<Value>>, RuntimeError> {
        const MAX_FAST_ARRAY_LENGTH: u64 = 2_147_483_647;

        if length > MAX_FAST_ARRAY_LENGTH {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid array length",
            )?));
        }
        let length = usize::try_from(length)
            .map_err(|_| RuntimeError::Invariant("fast Array length did not fit usize"))?;
        let mut values = Vec::new();
        values.try_reserve_exact(length).map_err(|_| {
            RuntimeError::Engine(Error::new(ErrorKind::JsInternal, "out of memory"))
        })?;
        values.resize(length, Value::Undefined);
        Ok(NativeConversion::Value(values))
    }

    pub(crate) fn call_array_prototype_with(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        indexed::finish(
            self,
            realm,
            indexed::IndexedStep::start(
                self,
                realm,
                indexed::IndexedKind::With,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_array_prototype_concat(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        concat::finish(
            self,
            realm,
            concat::ConcatStep::start(self, realm, &invocation, arguments)?,
        )
    }

    pub(crate) fn call_array_prototype_fill(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        indexed::finish(
            self,
            realm,
            indexed::IndexedStep::start(
                self,
                realm,
                indexed::IndexedKind::Fill,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_array_prototype_iteration(
        &self,
        realm: ContextId,
        kind: ArrayIterationKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        callback::finish(
            self,
            realm,
            callback::CallbackStep::start(
                self,
                realm,
                callback::CallbackKind::Iteration(kind),
                &invocation,
                arguments,
            )?,
        )
    }

    /// QuickJS `JS_ArraySpeciesCreate`: generic receivers always allocate a
    /// defining-realm base Array, while genuine Arrays observe constructor and
    /// @@species with the cross-realm default-Array compatibility exception.

    pub(crate) fn call_array_prototype_reduce(
        &self,
        realm: ContextId,
        kind: ArrayReduceKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        callback::finish(
            self,
            realm,
            callback::CallbackStep::start(
                self,
                realm,
                callback::CallbackKind::Reduce(kind),
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_array_prototype_find(
        &self,
        realm: ContextId,
        kind: ArrayFindKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        callback::finish(
            self,
            realm,
            callback::CallbackStep::start(
                self,
                realm,
                callback::CallbackKind::Find(kind),
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_array_prototype_copy_within(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        indexed::finish(
            self,
            realm,
            indexed::IndexedStep::start(
                self,
                realm,
                indexed::IndexedKind::CopyWithin,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_array_prototype_flatten(
        &self,
        realm: ContextId,
        kind: ArrayFlattenKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        flatten::finish(
            self,
            realm,
            flatten::FlattenStep::start(self, realm, kind, &invocation, arguments)?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    fn flatten_into_array_with_limits(
        &self,
        realm: ContextId,
        target: &ObjectRef,
        source: ObjectRef,
        source_length: u64,
        depth: i32,
        mapper: Option<&CallableRef>,
        mapper_this: &Value,
        target_limit: u64,
        frame_limit: usize,
    ) -> Result<NativeConversion<u64>, RuntimeError> {
        match flatten::finish(
            self,
            realm,
            flatten::FlattenStep::start_into(
                self,
                realm,
                target.clone(),
                source,
                source_length,
                depth,
                mapper.cloned(),
                mapper_this.clone(),
                target_limit,
                frame_limit,
            )?,
        )? {
            Completion::Return(value) => Ok(NativeConversion::Value(
                value
                    .as_number()
                    .ok_or(RuntimeError::Invariant("flatten count was not numeric"))?
                    as u64,
            )),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    pub(crate) fn call_array_prototype_search(
        &self,
        realm: ContextId,
        kind: ArraySearchKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        indexed::finish(
            self,
            realm,
            indexed::IndexedStep::start(
                self,
                realm,
                indexed::IndexedKind::Search(kind),
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_array_prototype_join(
        &self,
        realm: ContextId,
        kind: ArrayJoinKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_array_prototype_join_with_string_limit(
            realm,
            kind,
            invocation,
            arguments,
            JsString::MAX_LEN,
        )
    }

    pub(crate) fn call_array_prototype_join_with_string_limit(
        &self,
        realm: ContextId,
        kind: ArrayJoinKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        string::finish(
            self,
            realm,
            string::ArrayStringStep::start(
                self,
                realm,
                string::ArrayStringKind::Join(kind),
                &invocation,
                arguments,
                string_limit,
            )?,
        )
    }

    pub(crate) fn call_array_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let arguments = NativeArguments {
            readable: Vec::new(),
            actual_arg_count: 0,
        };
        string::finish(
            self,
            realm,
            string::ArrayStringStep::start(
                self,
                realm,
                string::ArrayStringKind::ToString,
                &invocation,
                &arguments,
                JsString::MAX_LEN,
            )?,
        )
    }

    pub(crate) fn call_array_prototype_pop(
        &self,
        realm: ContextId,
        kind: ArrayPopKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let arguments = NativeArguments {
            readable: Vec::new(),
            actual_arg_count: 0,
        };
        mutation::finish(
            self,
            realm,
            mutation::MutationStep::start(
                self,
                realm,
                mutation::MutationKind::Pop(kind),
                &invocation,
                &arguments,
            )?,
        )
    }

    pub(crate) fn call_array_prototype_push(
        &self,
        realm: ContextId,
        kind: ArrayPushKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        mutation::finish(
            self,
            realm,
            mutation::MutationStep::start(
                self,
                realm,
                mutation::MutationKind::Push(kind),
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_array_prototype_reverse(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        reverse::finish(
            self,
            realm,
            reverse::ReverseStep::start(self, realm, &invocation)?,
        )
    }

    pub(crate) fn call_array_prototype_to_reversed(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let arguments = NativeArguments {
            readable: Vec::new(),
            actual_arg_count: 0,
        };
        indexed::finish(
            self,
            realm,
            indexed::IndexedStep::start(
                self,
                realm,
                indexed::IndexedKind::ToReversed,
                &invocation,
                &arguments,
            )?,
        )
    }

    pub(in crate::engine::builtins) fn native_sort_comparator(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<NativeConversion<Option<CallableRef>>, RuntimeError> {
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "sort comparator argv was not padded",
        ))?;
        if matches!(argument, Value::Undefined) {
            return Ok(NativeConversion::Value(None));
        }
        if let Value::Object(object) = argument
            && let Some(callable) = self.as_callable(object)?
        {
            return Ok(NativeConversion::Value(Some(callable)));
        }
        Ok(NativeConversion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            "not a function",
        )?))
    }

    fn reserve_array_sort_slot_capacity(
        slots: &mut Vec<ArraySortSlot>,
        logical_capacity: &mut usize,
    ) -> Result<(), RuntimeError> {
        if slots.len() < *logical_capacity {
            return Ok(());
        }
        let next = logical_capacity
            .checked_add(*logical_capacity >> 1)
            .and_then(|value| value.checked_add(31))
            .map(|value| value & !15)
            .ok_or_else(|| {
                RuntimeError::Engine(Error::new(ErrorKind::JsInternal, "out of memory"))
            })?;
        if next > slots.capacity() {
            slots.try_reserve_exact(next - slots.len()).map_err(|_| {
                RuntimeError::Engine(Error::new(ErrorKind::JsInternal, "out of memory"))
            })?;
        }
        *logical_capacity = next;
        Ok(())
    }

    fn collect_dense_array_sort_slots(
        values: &[Value],
    ) -> Result<(Vec<ArraySortSlot>, u64), RuntimeError> {
        let mut slots = Vec::new();
        let mut logical_capacity = 0_usize;
        let mut undefined_count = 0_u64;
        for (position, value) in values.iter().cloned().enumerate() {
            Self::reserve_array_sort_slot_capacity(&mut slots, &mut logical_capacity)?;
            if matches!(value, Value::Undefined) {
                undefined_count = undefined_count
                    .checked_add(1)
                    .ok_or(RuntimeError::Invariant(
                        "Array sort undefined count overflowed Uint64",
                    ))?;
                continue;
            }
            slots.push(ArraySortSlot {
                value,
                cached_string: None,
                original_position: u64::try_from(position).map_err(|_| {
                    RuntimeError::Invariant("dense Array sort position exceeded Uint64")
                })?,
            });
        }
        Ok((slots, undefined_count))
    }

    pub(crate) fn call_array_prototype_sort(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        sort::finish(
            self,
            realm,
            sort::SortStep::start(self, realm, false, &invocation, arguments)?,
        )
    }

    pub(crate) fn call_array_prototype_to_sorted(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        sort::finish(
            self,
            realm,
            sort::SortStep::start(self, realm, true, &invocation, arguments)?,
        )
    }

    /// Shared Rust port of QuickJS `js_array_slice`. The upstream `splice`
    /// selector first creates and fills the deleted-elements result through
    /// ArraySpeciesCreate, then moves/deletes/inserts on the receiver while
    /// retaining every completed mutation if a later operation throws.
    pub(crate) fn call_array_prototype_slice(
        &self,
        realm: ContextId,
        kind: ArraySliceKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        slice::finish(
            self,
            realm,
            slice::SliceStep::start(
                self,
                realm,
                match kind {
                    ArraySliceKind::Slice => slice::SliceKind::Slice,
                    ArraySliceKind::Splice => slice::SliceKind::Splice,
                },
                &invocation,
                arguments,
            )?,
        )
    }

    /// QuickJS `js_array_toSpliced`: allocate a defining-realm dense base
    /// Array, copy the prefix and suffix through conditional Has/Get queries,
    /// and place the supplied values between them without consulting species.
    pub(crate) fn call_array_prototype_to_spliced(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        slice::finish(
            self,
            realm,
            slice::SliceStep::start(
                self,
                realm,
                slice::SliceKind::ToSpliced,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_array_prototype_iterator(
        &self,
        realm: ContextId,
        kind: ArrayIteratorKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array iterator factory did not receive a generic invocation",
            ));
        };
        let object = match self.native_to_object(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Object(
            self.new_array_iterator(realm, &object, kind)?,
        )))
    }

    pub(crate) fn call_array_iterator_next(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        match self.call_array_iterator_next_raw(realm, invocation)? {
            NativeInvokeOutcome::Completion(completion) => Ok(completion),
            NativeInvokeOutcome::IteratorNextRaw { value, done } => Ok(Completion::Return(
                Value::Object(self.new_iterator_result(realm, value, done)?),
            )),
        }
    }

    pub(crate) fn call_array_iterator_next_raw(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        super::iterator::array::finish(
            self,
            realm,
            super::iterator::array::ArrayNextStep::start(self, realm, &invocation)?,
        )
    }
}

#[cfg(test)]
mod tests;
