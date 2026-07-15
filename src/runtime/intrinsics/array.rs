//! Array constructor, prototype, iterator, and sorting intrinsics.

use std::cmp::Ordering as ComparisonOrdering;

use super::super::*;

struct ArraySortSlot {
    value: Value,
    cached_string: Option<JsString>,
    original_position: u64,
}

enum ArraySortAbort {
    Throw(Value),
    Runtime(RuntimeError),
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
fn quickjs_rqsort_by<T, E>(
    values: &mut [T],
    mut compare: impl FnMut(&mut [T], usize, usize) -> Result<ComparisonOrdering, E>,
) -> Result<(), E> {
    if values.len() < 2 {
        return Ok(());
    }

    let mut stack = [(0_usize, 0_usize, 0_usize); 50];
    stack[0] = (0, values.len(), 0);
    let mut stack_len = 1;

    while stack_len != 0 {
        stack_len -= 1;
        let (mut base, mut count, mut depth) = stack[stack_len];

        while count > 6 {
            depth += 1;
            if depth > 50 {
                quickjs_heapsort_by(values, base, count, &mut compare)?;
                count = 0;
                break;
            }

            let quarter = count >> 2;
            let pivot = quickjs_sort_median_of_three(
                values,
                base + quarter,
                base + 2 * quarter,
                base + 3 * quarter,
                &mut compare,
            )?;
            values.swap(base, pivot);

            let mut scanned = 1_usize;
            let mut lower_equal = 1_usize;
            let mut left = base + 1;
            let mut lower_equal_end = left;
            let mut upper_equal = count;
            let mut right = base + count;
            let mut upper_equal_start = right;
            let top = right;

            loop {
                while left < right {
                    let ordering = compare(values, base, left)?;
                    if ordering.is_lt() {
                        break;
                    }
                    if ordering.is_eq() {
                        values.swap(lower_equal_end, left);
                        lower_equal += 1;
                        lower_equal_end += 1;
                    }
                    scanned += 1;
                    left += 1;
                }

                loop {
                    right -= 1;
                    if left >= right {
                        break;
                    }
                    let ordering = compare(values, base, right)?;
                    if ordering.is_gt() {
                        break;
                    }
                    if ordering.is_eq() {
                        upper_equal -= 1;
                        upper_equal_start -= 1;
                        values.swap(upper_equal_start, right);
                    }
                }

                if left >= right {
                    break;
                }
                values.swap(left, right);
                scanned += 1;
                left += 1;
            }

            let mut span = lower_equal_end - base;
            let lower_middle_span = left - lower_equal_end;
            let lower_count = scanned - lower_equal;
            span = span.min(lower_middle_span);
            for offset in 0..span {
                values.swap(base + offset, left - span + offset);
            }

            span = top - upper_equal_start;
            let upper_middle_span = upper_equal_start - left;
            let upper_base = top - upper_middle_span;
            let upper_start = count - (upper_equal - scanned);
            span = span.min(upper_middle_span);
            for offset in 0..span {
                values.swap(left + offset, top - span + offset);
            }

            debug_assert!(stack_len < stack.len());
            if lower_count > count - upper_start {
                stack[stack_len] = (base, lower_count, depth);
                stack_len += 1;
                base = upper_base;
                count -= upper_start;
            } else {
                stack[stack_len] = (upper_base, count - upper_start, depth);
                stack_len += 1;
                count = lower_count;
            }
        }

        for current in (base + 1)..(base + count) {
            let mut position = current;
            while position > base && compare(values, position - 1, position)?.is_gt() {
                values.swap(position, position - 1);
                position -= 1;
            }
        }
    }
    Ok(())
}

fn quickjs_sort_median_of_three<T, E>(
    values: &mut [T],
    first: usize,
    second: usize,
    third: usize,
    compare: &mut impl FnMut(&mut [T], usize, usize) -> Result<ComparisonOrdering, E>,
) -> Result<usize, E> {
    if compare(values, first, second)?.is_lt() {
        if compare(values, second, third)?.is_lt() {
            Ok(second)
        } else if compare(values, first, third)?.is_lt() {
            Ok(third)
        } else {
            Ok(first)
        }
    } else if compare(values, second, third)?.is_gt() {
        Ok(second)
    } else if compare(values, first, third)?.is_lt() {
        Ok(first)
    } else {
        Ok(third)
    }
}

fn quickjs_heapsort_by<T, E>(
    values: &mut [T],
    base: usize,
    count: usize,
    compare: &mut impl FnMut(&mut [T], usize, usize) -> Result<ComparisonOrdering, E>,
) -> Result<(), E> {
    if count < 2 {
        return Ok(());
    }

    let mut root = count / 2;
    while root != 0 {
        root -= 1;
        quickjs_heap_sift(values, base, root, count, compare)?;
    }
    for end in (1..count).rev() {
        values.swap(base, base + end);
        quickjs_heap_sift(values, base, 0, end, compare)?;
    }
    Ok(())
}

fn quickjs_heap_sift<T, E>(
    values: &mut [T],
    base: usize,
    mut root: usize,
    end: usize,
    compare: &mut impl FnMut(&mut [T], usize, usize) -> Result<ComparisonOrdering, E>,
) -> Result<(), E> {
    loop {
        let mut child = root * 2 + 1;
        if child >= end {
            return Ok(());
        }
        if child + 1 < end && !compare(values, base + child, base + child + 1)?.is_gt() {
            child += 1;
        }
        if compare(values, base + root, base + child)?.is_gt() {
            return Ok(());
        }
        values.swap(base + root, base + child);
        root = child;
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

    fn create_array_from_constructor(
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
            let array_prototype = self.0.state.borrow().heap.context(realm)?.array_prototype;
            ObjectRef::from_borrowed_handle(self.clone(), array_prototype)?
        };
        Ok(Completion::Return(Value::Object(
            self.new_empty_array_with_prototype(&prototype)?,
        )))
    }

    pub(in crate::runtime) fn initialize_array_intrinsics(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        array_prototype: &ObjectRef,
        array_iterator_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeAt,
            "at",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeWith,
            "with",
            2,
            2,
        )?;
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeConcat,
            "concat",
            1,
            0,
        )?;
        for (kind, name) in [
            (ArrayIterationKind::Every, "every"),
            (ArrayIterationKind::Some, "some"),
            (ArrayIterationKind::ForEach, "forEach"),
            (ArrayIterationKind::Map, "map"),
            (ArrayIterationKind::Filter, "filter"),
        ] {
            self.define_native_builtin_auto_init(
                array_prototype,
                realm,
                NativeFunctionId::ArrayPrototypeIteration(kind),
                name,
                1,
                1,
            )?;
        }
        for (kind, name) in [
            (ArrayReduceKind::Reduce, "reduce"),
            (ArrayReduceKind::ReduceRight, "reduceRight"),
        ] {
            self.define_native_builtin_auto_init(
                array_prototype,
                realm,
                NativeFunctionId::ArrayPrototypeReduce(kind),
                name,
                1,
                1,
            )?;
        }
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeFill,
            "fill",
            1,
            1,
        )?;
        for (kind, name) in [
            (ArrayFindKind::Find, "find"),
            (ArrayFindKind::FindIndex, "findIndex"),
            (ArrayFindKind::FindLast, "findLast"),
            (ArrayFindKind::FindLastIndex, "findLastIndex"),
        ] {
            self.define_native_builtin_auto_init(
                array_prototype,
                realm,
                NativeFunctionId::ArrayPrototypeFind(kind),
                name,
                1,
                1,
            )?;
        }
        for (kind, name) in [
            (ArraySearchKind::IndexOf, "indexOf"),
            (ArraySearchKind::LastIndexOf, "lastIndexOf"),
            (ArraySearchKind::Includes, "includes"),
        ] {
            self.define_native_builtin_auto_init(
                array_prototype,
                realm,
                NativeFunctionId::ArrayPrototypeSearch(kind),
                name,
                1,
                1,
            )?;
        }
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeJoin(ArrayJoinKind::Join),
            "join",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeToString,
            "toString",
            0,
            0,
        )?;
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeJoin(ArrayJoinKind::ToLocaleString),
            "toLocaleString",
            0,
            0,
        )?;
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
            self.define_native_builtin_auto_init(array_prototype, realm, target, name, length, 0)?;
        }
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeReverse,
            "reverse",
            0,
            0,
        )?;
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeToReversed,
            "toReversed",
            0,
            0,
        )?;
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeSort,
            "sort",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeToSorted,
            "toSorted",
            1,
            1,
        )?;
        for (kind, name) in [
            (ArraySliceKind::Slice, "slice"),
            (ArraySliceKind::Splice, "splice"),
        ] {
            self.define_native_builtin_auto_init(
                array_prototype,
                realm,
                NativeFunctionId::ArrayPrototypeSlice(kind),
                name,
                2,
                2,
            )?;
        }
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeToSpliced,
            "toSpliced",
            2,
            2,
        )?;
        self.define_native_builtin_auto_init(
            array_prototype,
            realm,
            NativeFunctionId::ArrayPrototypeCopyWithin,
            "copyWithin",
            2,
            2,
        )?;
        for (kind, name, length, min_readable_args) in [
            (ArrayFlattenKind::FlatMap, "flatMap", 1, 1),
            (ArrayFlattenKind::Flat, "flat", 0, 0),
        ] {
            self.define_native_builtin_auto_init(
                array_prototype,
                realm,
                NativeFunctionId::ArrayPrototypeFlatten(kind),
                name,
                length,
                min_readable_args,
            )?;
        }
        for (kind, name) in [
            (ArrayIteratorKind::Value, "values"),
            (ArrayIteratorKind::Key, "keys"),
            (ArrayIteratorKind::KeyAndValue, "entries"),
        ] {
            self.define_native_builtin_auto_init(
                array_prototype,
                realm,
                NativeFunctionId::ArrayPrototypeIterator(kind),
                name,
                0,
                0,
            )?;
        }

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
        self.define_native_builtin_auto_init(
            constructor.as_object(),
            realm,
            NativeFunctionId::ArrayIsArray,
            "isArray",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            constructor.as_object(),
            realm,
            NativeFunctionId::ArrayFrom,
            "from",
            1,
            3,
        )?;
        self.define_native_builtin_auto_init(
            constructor.as_object(),
            realm,
            NativeFunctionId::ArrayOf,
            "of",
            0,
            0,
        )?;
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

    pub(in crate::runtime) fn instantiate_array_unscopables(
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

    pub(in crate::runtime) fn call_array_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { mut new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array constructor did not receive constructor-or-function invocation",
            ));
        };
        if matches!(new_target, Value::Undefined) {
            new_target = Value::Object(self.active_function()?);
        }
        let Value::Object(new_target) = new_target else {
            return Err(RuntimeError::Invariant(
                "Array constructor new.target was not an object",
            ));
        };
        let new_target = self.callable_from_value(Value::Object(new_target))?;
        let array = match self.create_array_from_constructor(realm, &new_target)? {
            Completion::Return(Value::Object(array)) => array,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "Array constructor allocation returned a primitive",
                ));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        if arguments.actual_arg_count == 1
            && matches!(arguments.readable[0], Value::Int(_) | Value::Float(_))
        {
            let length = match self.array_constructor_length(realm, &arguments.readable[0])? {
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
                        "fresh Array rejected its constructor length",
                    ));
                }
                PropertyDefineOutcome::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            for (index, value) in arguments.readable[..arguments.actual_arg_count]
                .iter()
                .cloned()
                .enumerate()
            {
                let index = u32::try_from(index).map_err(|_| {
                    RuntimeError::Invariant("native Array argument count exceeded Uint32")
                })?;
                if let Some(value) =
                    self.set_array_constructor_index(realm, &array, index, value)?
                {
                    return Ok(Completion::Throw(value));
                }
            }
        }
        Ok(Completion::Return(Value::Object(array)))
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

    fn create_array_data_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u32,
        value: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        self.create_indexed_data_property(realm, object, u64::from(index), value)
    }

    /// Array construction uses ordinary `Set`, not CreateDataProperty. A
    /// custom `newTarget.prototype` can therefore intercept an element with
    /// an inherited setter or reject it with a fixed data/accessor property.
    fn set_array_constructor_index(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u32,
        value: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let key = self.intern_property_key(&index.to_string())?;
        self.set_property_or_throw(realm, object, &key, value)
    }

    fn create_indexed_data_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u64,
        value: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let key = self.intern_property_key(&index.to_string())?;
        match self.define_own_property_in_realm(
            Some(realm),
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            PropertyDefineOutcome::Defined(true) => Ok(None),
            PropertyDefineOutcome::Defined(false) => {
                let array_length_read_only =
                    if let ArrayOwnKey::Index(index) = self.array_own_key(object, &key)? {
                        let (length, writable) = self.array_length_state(object)?;
                        index >= length && !writable
                    } else {
                        false
                    };
                let error = if array_length_read_only {
                    let length = self.intern_property_key("length")?;
                    self.native_atom_error(ErrorKind::Type, "'", &length, "' is read-only")?
                } else if !self.has_own_property(object, &key)? && !self.is_extensible(object)? {
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
            PropertyDefineOutcome::Throw(value) => Ok(Some(value)),
        }
    }

    pub(in crate::runtime) fn call_array_is_array(
        &self,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.isArray did not receive a generic invocation",
            ));
        };
        let result = match arguments.readable.first() {
            Some(Value::Object(object)) => self.is_array_object(object)?,
            Some(_) | None => false,
        };
        Ok(Completion::Return(Value::Bool(result)))
    }

    pub(in crate::runtime) fn call_array_species_getter(
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

    pub(in crate::runtime) fn call_array_from(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.from did not receive a generic invocation",
            ));
        };
        let items = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant("Array.from argv was not padded"))?;
        let mapping =
            arguments.actual_arg_count > 1 && !matches!(arguments.readable[1], Value::Undefined);
        let mapfn = if mapping {
            let Value::Object(object) = arguments.readable[1].clone() else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            let Some(callable) = self.as_callable(&object)? else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            Some(callable)
        } else {
            None
        };
        let this_argument = if arguments.actual_arg_count > 2 {
            arguments.readable[2].clone()
        } else {
            Value::Undefined
        };

        let iterator_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        let iterator_method = match &items {
            Value::Null | Value::Undefined => {
                let base = if matches!(items, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    &format!("cannot read property 'Symbol.iterator' of {base}"),
                )?));
            }
            _ => match self.get_value_property_in_realm(realm, items.clone(), &iterator_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            },
        };

        if !matches!(iterator_method, Value::Undefined | Value::Null) {
            let Value::Object(method_object) = iterator_method else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "value is not iterable",
                )?));
            };
            let Some(method) = self.as_callable(&method_object)? else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "value is not iterable",
                )?));
            };
            let result = match self.array_from_result(realm, this_value, None)? {
                Completion::Return(Value::Object(object)) => object,
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "Array.from result constructor returned a primitive",
                    ));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let iterator = match self.call_internal(realm, &method, items, &[])? {
                Completion::Return(Value::Object(iterator)) => iterator,
                Completion::Return(_) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not an object",
                    )?));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let next_key = self.intern_property_key("next")?;
            let next = match self.get_property_in_realm(realm, &iterator, &next_key)? {
                Completion::Return(Value::Object(next)) => {
                    let Some(next) = self.as_callable(&next)? else {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "not a function",
                        )?));
                    };
                    next
                }
                Completion::Return(_) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a function",
                    )?));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let done_key = self.intern_property_key("done")?;
            let value_key = self.intern_property_key("value")?;
            let mut index = 0_u64;
            loop {
                let iteration =
                    match self.call_internal(realm, &next, Value::Object(iterator.clone()), &[])? {
                        Completion::Return(Value::Object(result)) => result,
                        Completion::Return(_) => {
                            return Ok(Completion::Throw(self.new_native_error(
                                realm,
                                NativeErrorKind::Type,
                                "iterator must return an object",
                            )?));
                        }
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                let done = match self.get_property_in_realm(realm, &iteration, &done_key)? {
                    Completion::Return(value) => value.to_boolean(),
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if done {
                    if let Some(value) = self.set_array_like_length(realm, &result, index)? {
                        return Ok(Completion::Throw(value));
                    }
                    return Ok(Completion::Return(Value::Object(result)));
                }
                let mut value = match self.get_property_in_realm(realm, &iteration, &value_key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if let Some(mapfn) = &mapfn {
                    value = match self.call_internal(
                        realm,
                        mapfn,
                        this_argument.clone(),
                        &[value, Value::number(index as f64)],
                    )? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => {
                            self.close_iterator_preserving_throw(realm, &iterator)?;
                            return Ok(Completion::Throw(value));
                        }
                    };
                }
                if let Some(value) =
                    self.create_indexed_data_property(realm, &result, index, value)?
                {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
                index = index.checked_add(1).ok_or(RuntimeError::Invariant(
                    "Array.from iterator index overflowed u64",
                ))?;
            }
        }

        let source = match self.native_to_object(realm, items)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length_key = self.intern_property_key("length")?;
        let length_value = match self.get_property_in_realm(realm, &source, &length_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = match self.native_to_length(realm, &length_value)? {
            NativeConversion::Value(length) => length,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result =
            match self.array_from_result(realm, this_value, Some(Value::number(length as f64)))? {
                Completion::Return(Value::Object(object)) => object,
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "Array.from result constructor returned a primitive",
                    ));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        for index in 0..length {
            let key = self.intern_property_key(&index.to_string())?;
            let mut value = match self.get_property_in_realm(realm, &source, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if let Some(mapfn) = &mapfn {
                value = match self.call_internal(
                    realm,
                    mapfn,
                    this_argument.clone(),
                    &[value, Value::number(index as f64)],
                )? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            }
            if let Some(value) = self.create_indexed_data_property(realm, &result, index, value)? {
                return Ok(Completion::Throw(value));
            }
        }
        if let Some(value) = self.set_array_like_length(realm, &result, length)? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    fn array_from_result(
        &self,
        realm: ContextId,
        constructor: Value,
        length: Option<Value>,
    ) -> Result<Completion, RuntimeError> {
        if let Value::Object(object) = &constructor
            && self.is_constructor(object)?
        {
            let constructor = self.callable_from_value(constructor)?;
            let arguments = length.into_iter().collect::<Vec<_>>();
            return self.construct_internal(realm, &constructor, &constructor, &arguments);
        }
        let array = self.new_array(realm)?;
        if let Some(length) = length {
            let length = match self.to_array_length(Some(realm), &length)? {
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

    fn set_array_like_length(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        length: u64,
    ) -> Result<Option<Value>, RuntimeError> {
        let key = self.intern_property_key("length")?;
        self.set_property_or_throw(realm, object, &key, Value::number(length as f64))
    }

    pub(in crate::runtime) fn close_iterator_preserving_throw(
        &self,
        realm: ContextId,
        iterator: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key("return")?;
        let method = match self.get_property_in_realm(realm, iterator, &key)? {
            Completion::Return(value) => value,
            Completion::Throw(_) => return Ok(()),
        };
        if matches!(method, Value::Undefined | Value::Null) {
            return Ok(());
        }
        let Value::Object(method) = method else {
            return Ok(());
        };
        let Some(method) = self.as_callable(&method)? else {
            return Ok(());
        };
        let _ = self.call_internal(realm, &method, Value::Object(iterator.clone()), &[])?;
        Ok(())
    }

    pub(in crate::runtime) fn call_array_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.of did not receive a generic invocation",
            ));
        };
        let length = u32::try_from(arguments.actual_arg_count)
            .map_err(|_| RuntimeError::Invariant("Array.of argument count exceeded Uint32"))?;
        let result = if let Value::Object(object) = &this_value
            && self.is_constructor(object)?
        {
            let constructor = self.callable_from_value(this_value)?;
            match self.construct_internal(
                realm,
                &constructor,
                &constructor,
                &[Self::array_length_value(length)],
            )? {
                Completion::Return(Value::Object(object)) => object,
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "constructor invocation returned a primitive",
                    ));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            self.new_array(realm)?
        };
        for (index, value) in arguments.readable[..arguments.actual_arg_count]
            .iter()
            .cloned()
            .enumerate()
        {
            let index = u32::try_from(index)
                .map_err(|_| RuntimeError::Invariant("Array.of index exceeded Uint32"))?;
            if let Some(value) = self.create_array_data_property(realm, &result, index, value)? {
                return Ok(Completion::Throw(value));
            }
        }
        if let Some(value) = self.set_array_like_length(realm, &result, u64::from(length))? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    fn native_array_like_object_and_length(
        &self,
        realm: ContextId,
        this_value: Value,
    ) -> Result<NativeConversion<(ObjectRef, u64)>, RuntimeError> {
        let object = match self.native_to_object(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let length_key = self.intern_property_key("length")?;
        let length_value = match self.get_property_in_realm(realm, &object, &length_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let length = match self.native_to_length(realm, &length_value)? {
            NativeConversion::Value(length) => length,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        Ok(NativeConversion::Value((object, length)))
    }

    pub(in crate::runtime) fn call_array_prototype_at(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype.at did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let index = match self.native_to_int64_sat(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Array.prototype.at index argv was not padded",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = i64::try_from(length)
            .map_err(|_| RuntimeError::Invariant("array-like length exceeded Int64"))?;
        let index = if index < 0 { length + index } else { index };
        if index < 0 || index >= length {
            return Ok(Completion::Return(Value::Undefined));
        }
        let key = self.intern_property_key(&index.to_string())?;
        let present = match self.has_property_in_realm(realm, &object, &key)? {
            Completion::Return(Value::Bool(value)) => value,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "array at HasProperty did not return a boolean",
                ));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if !present {
            return Ok(Completion::Return(Value::Undefined));
        }
        self.get_property_in_realm(realm, &object, &key)
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

    pub(in crate::runtime) fn call_array_prototype_with(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype.with did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let index = match self.native_to_int64_sat(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Array.prototype.with index argv was not padded",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length_i64 = i64::try_from(length)
            .map_err(|_| RuntimeError::Invariant("array-like length exceeded Int64"))?;
        let index = if index < 0 { length_i64 + index } else { index };
        if index < 0 || index >= length_i64 {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                &format!("invalid array index: {index}"),
            )?));
        }
        // QuickJS allocates and initializes the complete dense fast-array
        // storage before observing any indexed source property. Reserve and
        // initialize the equivalent Rust value buffer at the same boundary.
        let mut values = match self.native_allocate_fast_array_values(realm, length)? {
            NativeConversion::Value(values) => values,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let replacement_index = usize::try_from(index)
            .map_err(|_| RuntimeError::Invariant("validated Array.with index did not fit usize"))?;
        let replacement = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "Array.prototype.with replacement argv was not padded",
        ))?;

        for (position, slot) in values.iter_mut().enumerate() {
            if position == replacement_index {
                *slot = replacement.clone();
                continue;
            }
            let key = self.intern_property_key(&position.to_string())?;
            let present = match self.has_property_in_realm(realm, &object, &key)? {
                Completion::Return(Value::Bool(value)) => value,
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "Array.with HasProperty did not return a boolean",
                    ));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if present {
                *slot = match self.get_property_in_realm(realm, &object, &key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            }
        }
        Ok(Completion::Return(Value::Object(
            self.new_array_from_values(realm, values)?,
        )))
    }

    fn native_is_concat_spreadable(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Value(false));
        };
        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::IsConcatSpreadable));
        match self.get_property_in_realm(realm, object, &key)? {
            Completion::Return(Value::Undefined) => {
                Ok(NativeConversion::Value(self.is_array_object(object)?))
            }
            Completion::Return(value) => Ok(NativeConversion::Value(value.to_boolean())),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    pub(in crate::runtime) fn call_array_prototype_concat(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;

        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype.concat did not receive a generic invocation",
            ));
        };
        let source = match self.native_to_object(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = match self.array_species_create(realm, &source, 0)? {
            Completion::Return(Value::Object(object)) => object,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "ArraySpeciesCreate returned a primitive",
                ));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let mut next_index = 0_u64;

        for element in std::iter::once(Value::Object(source.clone())).chain(
            arguments.readable[..arguments.actual_arg_count]
                .iter()
                .cloned(),
        ) {
            let spreadable = match self.native_is_concat_spreadable(realm, &element)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if spreadable {
                let (element, length) =
                    match self.native_array_like_object_and_length(realm, element)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                let Some(end) = next_index.checked_add(length) else {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "Array loo long",
                    )?));
                };
                if end > MAX_SAFE_INTEGER {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "Array loo long",
                    )?));
                }
                for source_index in 0..length {
                    let key = self.intern_property_key(&source_index.to_string())?;
                    let present = match self.has_property_in_realm(realm, &element, &key)? {
                        Completion::Return(Value::Bool(value)) => value,
                        Completion::Return(_) => {
                            return Err(RuntimeError::Invariant(
                                "Array.concat HasProperty did not return a boolean",
                            ));
                        }
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if present {
                        let value = match self.get_property_in_realm(realm, &element, &key)? {
                            Completion::Return(value) => value,
                            Completion::Throw(value) => return Ok(Completion::Throw(value)),
                        };
                        if let Some(value) =
                            self.create_indexed_data_property(realm, &result, next_index, value)?
                        {
                            return Ok(Completion::Throw(value));
                        }
                    }
                    next_index += 1;
                }
            } else {
                if next_index >= MAX_SAFE_INTEGER {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "Array loo long",
                    )?));
                }
                if let Some(value) =
                    self.create_indexed_data_property(realm, &result, next_index, element)?
                {
                    return Ok(Completion::Throw(value));
                }
                next_index += 1;
            }
        }

        if let Some(value) = self.set_array_like_length(realm, &result, next_index)? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    pub(in crate::runtime) fn call_array_prototype_fill(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype.fill did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = i64::try_from(length)
            .map_err(|_| RuntimeError::Invariant("array-like length exceeded Int64"))?;

        // QuickJS deliberately skips conversion for an omitted or explicit
        // undefined bound. Every other start is converted before end, even
        // when the snapshot length is zero or the eventual range is empty.
        let mut start = if arguments.actual_arg_count > 1
            && !matches!(arguments.readable.get(1), Some(Value::Undefined))
        {
            match self.native_to_int64_clamp(
                realm,
                arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                    "Array.prototype.fill start argument was missing",
                ))?,
                0,
                length,
                length,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            0
        };
        let end = if arguments.actual_arg_count > 2
            && !matches!(arguments.readable.get(2), Some(Value::Undefined))
        {
            match self.native_to_int64_clamp(
                realm,
                arguments.readable.get(2).ok_or(RuntimeError::Invariant(
                    "Array.prototype.fill end argument was missing",
                ))?,
                0,
                length,
                length,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            length
        };
        let fill_value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Array.prototype.fill value argv was not padded",
        ))?;

        while start < end {
            let key = self.intern_property_key(&start.to_string())?;
            if let Some(value) =
                self.set_property_or_throw(realm, &object, &key, fill_value.clone())?
            {
                return Ok(Completion::Throw(value));
            }
            start += 1;
        }
        Ok(Completion::Return(Value::Object(object)))
    }

    pub(in crate::runtime) fn call_array_prototype_iteration(
        &self,
        realm: ContextId,
        kind: ArrayIterationKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype iteration method did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let callback = self.callable_from_value(
            arguments
                .readable
                .first()
                .ok_or(RuntimeError::Invariant(
                    "Array.prototype iteration callback argv was not padded",
                ))?
                .clone(),
        )?;
        let this_arg = if arguments.actual_arg_count > 1 {
            arguments
                .readable
                .get(1)
                .ok_or(RuntimeError::Invariant(
                    "Array.prototype iteration thisArg was missing",
                ))?
                .clone()
        } else {
            Value::Undefined
        };
        let result = match kind {
            ArrayIterationKind::Every => Value::Bool(true),
            ArrayIterationKind::Some => Value::Bool(false),
            ArrayIterationKind::ForEach => Value::Undefined,
            ArrayIterationKind::Map | ArrayIterationKind::Filter => {
                let result_length = if kind == ArrayIterationKind::Map {
                    length
                } else {
                    0
                };
                match self.array_species_create(realm, &object, result_length)? {
                    Completion::Return(value @ Value::Object(_)) => value,
                    Completion::Return(_) => {
                        return Err(RuntimeError::Invariant(
                            "ArraySpeciesCreate returned a primitive",
                        ));
                    }
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
        };
        let length = i64::try_from(length)
            .map_err(|_| RuntimeError::Invariant("array-like length exceeded Int64"))?;
        let callback_receiver = Value::Object(object.clone());
        let mut selected_count = 0_u64;

        for index in 0..length {
            let key = self.intern_property_key(&index.to_string())?;
            let present = match self.has_property_in_realm(realm, &object, &key)? {
                Completion::Return(Value::Bool(value)) => value,
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "Array iteration HasProperty did not return a boolean",
                    ));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if !present {
                continue;
            }
            let value = match self.get_property_in_realm(realm, &object, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let callback_arguments = [
                value.clone(),
                Value::number(index as f64),
                callback_receiver.clone(),
            ];
            let callback_result = match self.call_internal(
                realm,
                &callback,
                this_arg.clone(),
                &callback_arguments,
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            match kind {
                ArrayIterationKind::Every if !callback_result.to_boolean() => {
                    return Ok(Completion::Return(Value::Bool(false)));
                }
                ArrayIterationKind::Some if callback_result.to_boolean() => {
                    return Ok(Completion::Return(Value::Bool(true)));
                }
                ArrayIterationKind::Map => {
                    let Value::Object(result) = &result else {
                        return Err(RuntimeError::Invariant(
                            "Array.map result was not an object",
                        ));
                    };
                    let index = u64::try_from(index)
                        .map_err(|_| RuntimeError::Invariant("Array.map index was negative"))?;
                    if let Some(value) =
                        self.create_indexed_data_property(realm, result, index, callback_result)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                ArrayIterationKind::Filter if callback_result.to_boolean() => {
                    let Value::Object(result) = &result else {
                        return Err(RuntimeError::Invariant(
                            "Array.filter result was not an object",
                        ));
                    };
                    if let Some(value) =
                        self.create_indexed_data_property(realm, result, selected_count, value)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                    selected_count =
                        selected_count
                            .checked_add(1)
                            .ok_or(RuntimeError::Invariant(
                                "Array.filter result index overflowed u64",
                            ))?;
                }
                ArrayIterationKind::Every
                | ArrayIterationKind::Some
                | ArrayIterationKind::ForEach
                | ArrayIterationKind::Filter => {}
            }
        }
        Ok(Completion::Return(result))
    }

    /// QuickJS `JS_ArraySpeciesCreate`: generic receivers always allocate a
    /// defining-realm base Array, while genuine Arrays observe constructor and
    /// @@species with the cross-realm default-Array compatibility exception.
    fn array_species_create(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        length: u64,
    ) -> Result<Completion, RuntimeError> {
        if !self.is_array_object(source)? {
            return self.array_from_result(
                realm,
                Value::Undefined,
                Some(Value::number(length as f64)),
            );
        }

        let constructor_key = self.intern_property_key("constructor")?;
        let mut constructor = match self.get_property_in_realm(realm, source, &constructor_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        if let Value::Object(object) = &constructor
            && self.is_constructor(object)?
        {
            let callable = self.as_callable(object)?.ok_or(RuntimeError::Invariant(
                "constructable Array constructor value was not callable",
            ))?;
            let constructor_realm = self.callable_realm(&callable)?;
            let is_cross_realm_default = if constructor_realm != realm {
                self.0
                    .state
                    .borrow()
                    .heap
                    .context(constructor_realm)?
                    .array_constructor
                    .is_some_and(|default| default == object.object_id())
            } else {
                false
            };
            if is_cross_realm_default {
                constructor = Value::Undefined;
            }
        }

        if let Value::Object(object) = &constructor {
            let species_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Species));
            constructor = match self.get_property_in_realm(realm, object, &species_key)? {
                Completion::Return(Value::Null) => Value::Undefined,
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        }

        if matches!(constructor, Value::Undefined) {
            return self.array_from_result(
                realm,
                Value::Undefined,
                Some(Value::number(length as f64)),
            );
        }

        let callable = match self.constructor_from_value(realm, constructor)? {
            NativeConversion::Value(callable) => callable,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.construct_internal(realm, &callable, &callable, &[Value::number(length as f64)])
    }

    pub(in crate::runtime) fn call_array_prototype_reduce(
        &self,
        realm: ContextId,
        kind: ArrayReduceKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype reduce method did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let callback = self.callable_from_value(
            arguments
                .readable
                .first()
                .ok_or(RuntimeError::Invariant(
                    "Array.prototype reduce callback argv was not padded",
                ))?
                .clone(),
        )?;
        let length = i64::try_from(length)
            .map_err(|_| RuntimeError::Invariant("array-like length exceeded Int64"))?;
        let callback_receiver = Value::Object(object.clone());
        let mut step = 0_i64;

        let mut accumulator = if arguments.actual_arg_count > 1 {
            arguments
                .readable
                .get(1)
                .ok_or(RuntimeError::Invariant(
                    "Array.prototype reduce initial value was missing",
                ))?
                .clone()
        } else {
            let mut found = None;
            while step < length {
                let index = match kind {
                    ArrayReduceKind::Reduce => step,
                    ArrayReduceKind::ReduceRight => length - step - 1,
                };
                step += 1;
                let key = self.intern_property_key(&index.to_string())?;
                let present = match self.has_property_in_realm(realm, &object, &key)? {
                    Completion::Return(Value::Bool(value)) => value,
                    Completion::Return(_) => {
                        return Err(RuntimeError::Invariant(
                            "Array reduce HasProperty did not return a boolean",
                        ));
                    }
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if present {
                    found = Some(match self.get_property_in_realm(realm, &object, &key)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    });
                    break;
                }
            }
            let Some(value) = found else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "empty array",
                )?));
            };
            value
        };

        while step < length {
            let index = match kind {
                ArrayReduceKind::Reduce => step,
                ArrayReduceKind::ReduceRight => length - step - 1,
            };
            step += 1;
            let key = self.intern_property_key(&index.to_string())?;
            let present = match self.has_property_in_realm(realm, &object, &key)? {
                Completion::Return(Value::Bool(value)) => value,
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "Array reduce HasProperty did not return a boolean",
                    ));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if !present {
                continue;
            }
            let value = match self.get_property_in_realm(realm, &object, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            accumulator = match self.call_internal(
                realm,
                &callback,
                Value::Undefined,
                &[
                    accumulator,
                    value,
                    Value::number(index as f64),
                    callback_receiver.clone(),
                ],
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        }
        Ok(Completion::Return(accumulator))
    }

    pub(in crate::runtime) fn call_array_prototype_find(
        &self,
        realm: ContextId,
        kind: ArrayFindKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype find method did not receive a generic invocation",
            ));
        };
        let original_this = this_value.clone();
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let predicate = self.callable_from_value(
            arguments
                .readable
                .first()
                .ok_or(RuntimeError::Invariant(
                    "Array.prototype find predicate argv was not padded",
                ))?
                .clone(),
        )?;
        let this_arg = if arguments.actual_arg_count > 1 {
            arguments
                .readable
                .get(1)
                .ok_or(RuntimeError::Invariant(
                    "Array.prototype find thisArg was missing",
                ))?
                .clone()
        } else {
            Value::Undefined
        };
        let length = i64::try_from(length)
            .map_err(|_| RuntimeError::Invariant("array-like length exceeded Int64"))?;
        let (mut index, end, direction) = match kind {
            ArrayFindKind::Find | ArrayFindKind::FindIndex => (0, length, 1),
            ArrayFindKind::FindLast | ArrayFindKind::FindLastIndex => (length - 1, -1, -1),
        };

        while index != end {
            let key = self.intern_property_key(&index.to_string())?;
            let value = match self.get_property_in_realm(realm, &object, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let index_value = Value::number(index as f64);
            let callback_arguments = [value.clone(), index_value.clone(), original_this.clone()];
            let matches = match self.call_internal(
                realm,
                &predicate,
                this_arg.clone(),
                &callback_arguments,
            )? {
                Completion::Return(value) => value.to_boolean(),
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if matches {
                return Ok(Completion::Return(match kind {
                    ArrayFindKind::Find | ArrayFindKind::FindLast => value,
                    ArrayFindKind::FindIndex | ArrayFindKind::FindLastIndex => index_value,
                }));
            }
            index += direction;
        }
        Ok(Completion::Return(match kind {
            ArrayFindKind::Find | ArrayFindKind::FindLast => Value::Undefined,
            ArrayFindKind::FindIndex | ArrayFindKind::FindLastIndex => Value::Int(-1),
        }))
    }

    pub(in crate::runtime) fn call_array_prototype_copy_within(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype.copyWithin did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = i64::try_from(length)
            .map_err(|_| RuntimeError::Invariant("array-like length exceeded Int64"))?;
        let to = match self.native_to_int64_clamp(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Array.prototype.copyWithin target argv was not padded",
            ))?,
            0,
            length,
            length,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let from = match self.native_to_int64_clamp(
            realm,
            arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "Array.prototype.copyWithin start argv was not padded",
            ))?,
            0,
            length,
            length,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let final_index = if arguments.actual_arg_count > 2
            && !matches!(arguments.readable.get(2), Some(Value::Undefined))
        {
            match self.native_to_int64_clamp(
                realm,
                arguments.readable.get(2).ok_or(RuntimeError::Invariant(
                    "Array.prototype.copyWithin end argument was missing",
                ))?,
                0,
                length,
                length,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            length
        };
        let count = (final_index - from).min(length - to);
        if count > 0 {
            let backwards = from < to && to < from + count;
            let to = u64::try_from(to)
                .map_err(|_| RuntimeError::Invariant("Array.copyWithin target was negative"))?;
            let from = u64::try_from(from)
                .map_err(|_| RuntimeError::Invariant("Array.copyWithin source was negative"))?;
            let count = u64::try_from(count)
                .map_err(|_| RuntimeError::Invariant("Array.copyWithin count was negative"))?;
            if let Some(value) =
                self.copy_array_like_range(realm, &object, to, from, count, backwards)?
            {
                return Ok(Completion::Throw(value));
            }
        }
        Ok(Completion::Return(Value::Object(object)))
    }

    pub(in crate::runtime) fn call_array_prototype_flatten(
        &self,
        realm: ContextId,
        kind: ArrayFlattenKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype flatten method did not receive a generic invocation",
            ));
        };
        let (source, source_length) =
            match self.native_array_like_object_and_length(realm, this_value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };

        let (depth, mapper, mapper_this) = match kind {
            ArrayFlattenKind::FlatMap => {
                let mapper = self.callable_from_value(
                    arguments
                        .readable
                        .first()
                        .ok_or(RuntimeError::Invariant(
                            "Array.prototype.flatMap mapper argv was not padded",
                        ))?
                        .clone(),
                )?;
                let mapper_this = if arguments.actual_arg_count > 1 {
                    arguments
                        .readable
                        .get(1)
                        .ok_or(RuntimeError::Invariant(
                            "Array.prototype.flatMap thisArg was missing",
                        ))?
                        .clone()
                } else {
                    Value::Undefined
                };
                (1, Some(mapper), mapper_this)
            }
            ArrayFlattenKind::Flat => {
                let depth = if arguments.actual_arg_count > 0
                    && !matches!(arguments.readable.first(), Some(Value::Undefined))
                {
                    let depth = arguments.readable.first().ok_or(RuntimeError::Invariant(
                        "Array.prototype.flat depth argument was missing",
                    ))?;
                    match self.native_to_number(realm, depth)? {
                        NativeConversion::Value(value) => crate::number::to_int32_sat(value),
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                } else {
                    1
                };
                (depth, None, Value::Undefined)
            }
        };

        let target = match self.array_species_create(realm, &source, 0)? {
            Completion::Return(Value::Object(target)) => target,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "ArraySpeciesCreate returned a primitive for flatten",
                ));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;
        match self.flatten_into_array_with_limits(
            realm,
            &target,
            source,
            source_length,
            depth,
            mapper.as_ref(),
            &mapper_this,
            MAX_SAFE_INTEGER,
            ARRAY_FLATTEN_FRAME_LIMIT,
        )? {
            NativeConversion::Value(_) => Ok(Completion::Return(Value::Object(target))),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    #[allow(clippy::too_many_arguments)]
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
        if frame_limit == 0 {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "stack overflow",
            )?));
        }
        let mut frames = vec![ArrayFlattenFrame {
            source,
            length: source_length,
            next_index: 0,
            depth,
            apply_mapper: mapper.is_some(),
        }];
        let mut target_index = 0_u64;

        while !frames.is_empty() {
            let (source, source_index, depth, apply_mapper) = {
                let frame = frames
                    .last_mut()
                    .expect("non-empty flatten frame stack has a last frame");
                if frame.next_index >= frame.length {
                    frames.pop();
                    continue;
                }
                let source_index = frame.next_index;
                frame.next_index += 1;
                (
                    frame.source.clone(),
                    source_index,
                    frame.depth,
                    frame.apply_mapper,
                )
            };

            let Some(mut element) =
                (match self.try_get_array_like_index(realm, &source, source_index)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(NativeConversion::Throw(value));
                    }
                })
            else {
                continue;
            };

            if apply_mapper {
                let mapper = mapper.ok_or(RuntimeError::Invariant(
                    "flatten frame requested a missing mapper",
                ))?;
                element = match self.call_internal(
                    realm,
                    mapper,
                    mapper_this.clone(),
                    &[
                        element,
                        Value::number(source_index as f64),
                        Value::Object(source.clone()),
                    ],
                )? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
                };
            }

            if depth > 0
                && let Value::Object(element_object) = &element
                && self.is_array_object(element_object)?
            {
                let (nested_source, nested_length) = match self
                    .native_array_like_object_and_length(
                        realm,
                        Value::Object(element_object.clone()),
                    )? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(NativeConversion::Throw(value));
                    }
                };
                if frames.len() >= frame_limit {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Internal,
                        "stack overflow",
                    )?));
                }
                frames.push(ArrayFlattenFrame {
                    source: nested_source,
                    length: nested_length,
                    next_index: 0,
                    depth: depth - 1,
                    apply_mapper: false,
                });
                continue;
            }

            if target_index >= target_limit {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "Array too long",
                )?));
            }
            if let Some(value) =
                self.create_indexed_data_property(realm, target, target_index, element)?
            {
                return Ok(NativeConversion::Throw(value));
            }
            target_index += 1;
        }

        Ok(NativeConversion::Value(target_index))
    }

    pub(in crate::runtime) fn call_array_prototype_search(
        &self,
        realm: ContextId,
        kind: ArraySearchKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype search method did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let not_found = || match kind {
            ArraySearchKind::Includes => Value::Bool(false),
            ArraySearchKind::IndexOf | ArraySearchKind::LastIndexOf => Value::Int(-1),
        };
        if length == 0 {
            return Ok(Completion::Return(not_found()));
        }
        let length = i64::try_from(length)
            .map_err(|_| RuntimeError::Invariant("array-like length exceeded Int64"))?;
        let search = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Array.prototype search value argv was not padded",
        ))?;
        let from_index = if arguments.actual_arg_count > 1 {
            Some(arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "Array.prototype search fromIndex argv was missing",
            ))?)
        } else {
            None
        };

        let (mut index, end, step) = match kind {
            ArraySearchKind::Includes | ArraySearchKind::IndexOf => {
                let index = if let Some(from_index) = from_index {
                    match self.native_to_int64_clamp(realm, from_index, 0, length, length)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                } else {
                    0
                };
                (index, length, 1)
            }
            ArraySearchKind::LastIndexOf => {
                let index = if let Some(from_index) = from_index {
                    match self.native_to_int64_clamp(realm, from_index, -1, length - 1, length)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                } else {
                    length - 1
                };
                (index, -1, -1)
            }
        };

        while index != end {
            let key = self.intern_property_key(&index.to_string())?;
            let value = match kind {
                ArraySearchKind::Includes => {
                    match self.get_property_in_realm(realm, &object, &key)? {
                        Completion::Return(value) => Some(value),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                ArraySearchKind::IndexOf | ArraySearchKind::LastIndexOf => {
                    let present = match self.has_property_in_realm(realm, &object, &key)? {
                        Completion::Return(Value::Bool(value)) => value,
                        Completion::Return(_) => {
                            return Err(RuntimeError::Invariant(
                                "array search HasProperty did not return a boolean",
                            ));
                        }
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if !present {
                        None
                    } else {
                        match self.get_property_in_realm(realm, &object, &key)? {
                            Completion::Return(value) => Some(value),
                            Completion::Throw(value) => return Ok(Completion::Throw(value)),
                        }
                    }
                }
            };
            let matches = value.is_some_and(|value| match kind {
                ArraySearchKind::Includes => search.same_value_zero(&value),
                ArraySearchKind::IndexOf | ArraySearchKind::LastIndexOf => {
                    search.strict_equal(&value)
                }
            });
            if matches {
                return Ok(Completion::Return(match kind {
                    ArraySearchKind::Includes => Value::Bool(true),
                    ArraySearchKind::IndexOf | ArraySearchKind::LastIndexOf => {
                        Value::number(index as f64)
                    }
                }));
            }
            index += step;
        }
        Ok(Completion::Return(not_found()))
    }

    fn native_array_element_locale_value(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<Value>, RuntimeError> {
        let key = self.intern_property_key("toLocaleString")?;
        let method = match self.get_value_property_in_realm(realm, value.clone(), &key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Value::Object(method) = method else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(method) = self.as_callable(&method)? else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        match self.call_internal(realm, &method, value, &[])? {
            Completion::Return(value) => Ok(NativeConversion::Value(value)),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    pub(in crate::runtime) fn call_array_prototype_join(
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

    pub(in crate::runtime) fn call_array_prototype_join_with_string_limit(
        &self,
        realm: ContextId,
        kind: ArrayJoinKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype join method did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let separator = match kind {
            ArrayJoinKind::ToLocaleString => JsString::from_static(","),
            ArrayJoinKind::Join
                if arguments.actual_arg_count == 0
                    || matches!(arguments.readable.first(), Some(Value::Undefined)) =>
            {
                JsString::from_static(",")
            }
            ArrayJoinKind::Join => {
                let separator = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "Array.prototype.join separator argv was not padded",
                ))?;
                match self.native_to_js_string(realm, separator)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
        };

        let mut output = JsStringBuilder::with_limit(0, string_limit);
        let mut separator_error = None;
        for index in 0..length {
            if index != 0 {
                if let Err(error) = output.push_js_string(&separator) {
                    separator_error.get_or_insert(error);
                }
            }
            // Pinned QuickJS passes its Int64 loop index through
            // JS_GetPropertyUint32, including Uint32 wraparound above 2^32.
            let key = self.intern_property_key(&(index as u32).to_string())?;
            let element = match self.get_property_in_realm(realm, &object, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if matches!(element, Value::Null | Value::Undefined) {
                continue;
            }
            let element = match kind {
                ArrayJoinKind::Join => {
                    // concat_value_free observes the failed buffer before
                    // attempting ordinary ToString.
                    if let Some(error) = separator_error {
                        return Err(error.into());
                    }
                    match self.native_to_js_string(realm, &element)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                ArrayJoinKind::ToLocaleString => {
                    let value = match self.native_array_element_locale_value(realm, element)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    };
                    // QuickJS invokes toLocaleString before handing the return
                    // value to the already-failed StringBuffer. The return
                    // value's own ToString is therefore skipped.
                    if let Some(error) = separator_error {
                        return Err(error.into());
                    }
                    match self.native_to_js_string(realm, &value)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                }
            };
            output.push_js_string(&element)?;
        }
        if let Some(error) = separator_error {
            return Err(error.into());
        }
        Ok(Completion::Return(Value::String(output.finish()?)))
    }

    pub(in crate::runtime) fn call_array_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype.toString did not receive a generic invocation",
            ));
        };
        let object = match self.native_to_object(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let join_key = self.intern_property_key("join")?;
        let method = match self.get_property_in_realm(realm, &object, &join_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if let Value::Object(method) = method
            && let Some(method) = self.as_callable(&method)?
        {
            return self.call_internal(realm, &method, Value::Object(object), &[]);
        }
        self.call_object_prototype_to_string(
            realm,
            NativeInvocation::Call {
                this_value: Value::Object(object),
            },
        )
    }

    fn try_get_array_like_index(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u64,
    ) -> Result<NativeConversion<Option<Value>>, RuntimeError> {
        let key = self.intern_property_key(&index.to_string())?;
        let present = match self.has_property_in_realm(realm, object, &key)? {
            Completion::Return(Value::Bool(value)) => value,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "Array indexed HasProperty did not return a boolean",
                ));
            }
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !present {
            return Ok(NativeConversion::Value(None));
        }
        match self.get_property_in_realm(realm, object, &key)? {
            Completion::Return(value) => Ok(NativeConversion::Value(Some(value))),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    fn copy_array_like_range(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        to_start: u64,
        from_start: u64,
        count: u64,
        backwards: bool,
    ) -> Result<Option<Value>, RuntimeError> {
        for offset in 0..count {
            let relative = if backwards {
                count - offset - 1
            } else {
                offset
            };
            let from = from_start
                .checked_add(relative)
                .ok_or(RuntimeError::Invariant(
                    "Array copy source index overflowed",
                ))?;
            let to = to_start
                .checked_add(relative)
                .ok_or(RuntimeError::Invariant(
                    "Array copy target index overflowed",
                ))?;
            let value = match self.try_get_array_like_index(realm, object, from)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Some(value)),
            };
            let to_key = self.intern_property_key(&to.to_string())?;
            if let Some(value) = value {
                if let Some(value) = self.set_property_or_throw(realm, object, &to_key, value)? {
                    return Ok(Some(value));
                }
            } else if !self.delete_property(object, &to_key)? {
                return Ok(Some(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "could not delete property",
                )?));
            }
        }
        Ok(None)
    }

    fn delete_array_like_index_or_throw(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u64,
    ) -> Result<Option<Value>, RuntimeError> {
        let key = self.intern_property_key(&index.to_string())?;
        if self.delete_property(object, &key)? {
            Ok(None)
        } else {
            Ok(Some(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "could not delete property",
            )?))
        }
    }

    pub(in crate::runtime) fn call_array_prototype_pop(
        &self,
        realm: ContextId,
        kind: ArrayPopKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype pop/shift did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let new_length = length.saturating_sub(1);
        let mut result = Value::Undefined;
        if length != 0 {
            let result_key = self.intern_property_key(
                &(if kind == ArrayPopKind::Shift {
                    0
                } else {
                    new_length
                })
                .to_string(),
            )?;
            result = match self.get_property_in_realm(realm, &object, &result_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if kind == ArrayPopKind::Shift
                && let Some(value) =
                    self.copy_array_like_range(realm, &object, 0, 1, new_length, false)?
            {
                return Ok(Completion::Throw(value));
            }
            if let Some(value) =
                self.delete_array_like_index_or_throw(realm, &object, new_length)?
            {
                return Ok(Completion::Throw(value));
            }
        }
        if let Some(value) = self.set_array_like_length(realm, &object, new_length)? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(result))
    }

    pub(in crate::runtime) fn call_array_prototype_push(
        &self,
        realm: ContextId,
        kind: ArrayPushKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;

        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype push/unshift did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let argument_count = u64::try_from(arguments.actual_arg_count)
            .map_err(|_| RuntimeError::Invariant("Array push argument count exceeded Uint64"))?;
        let Some(new_length) = length.checked_add(argument_count) else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "Array loo long",
            )?));
        };
        if new_length > MAX_SAFE_INTEGER {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "Array loo long",
            )?));
        }

        let from = if kind == ArrayPushKind::Unshift && argument_count != 0 {
            if let Some(value) =
                self.copy_array_like_range(realm, &object, argument_count, 0, length, true)?
            {
                return Ok(Completion::Throw(value));
            }
            0
        } else {
            length
        };
        for (offset, value) in arguments.readable[..arguments.actual_arg_count]
            .iter()
            .cloned()
            .enumerate()
        {
            let offset = u64::try_from(offset)
                .map_err(|_| RuntimeError::Invariant("Array push offset exceeded Uint64"))?;
            let index = from
                .checked_add(offset)
                .ok_or(RuntimeError::Invariant("Array push index overflowed"))?;
            let key = self.intern_property_key(&index.to_string())?;
            if let Some(value) = self.set_property_or_throw(realm, &object, &key, value)? {
                return Ok(Completion::Throw(value));
            }
        }
        if let Some(value) = self.set_array_like_length(realm, &object, new_length)? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::number(new_length as f64)))
    }

    pub(in crate::runtime) fn call_array_prototype_reverse(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype.reverse did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let mut lower = 0_u64;
        let mut upper = length.saturating_sub(1);
        while lower < upper {
            let lower_value = match self.try_get_array_like_index(realm, &object, lower)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let upper_value = match self.try_get_array_like_index(realm, &object, upper)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };

            match (lower_value, upper_value) {
                (lower_value, Some(upper_value)) => {
                    let lower_key = self.intern_property_key(&lower.to_string())?;
                    if let Some(value) =
                        self.set_property_or_throw(realm, &object, &lower_key, upper_value)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                    if let Some(lower_value) = lower_value {
                        let upper_key = self.intern_property_key(&upper.to_string())?;
                        if let Some(value) =
                            self.set_property_or_throw(realm, &object, &upper_key, lower_value)?
                        {
                            return Ok(Completion::Throw(value));
                        }
                    } else if let Some(value) =
                        self.delete_array_like_index_or_throw(realm, &object, upper)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                (Some(lower_value), None) => {
                    if let Some(value) =
                        self.delete_array_like_index_or_throw(realm, &object, lower)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                    let upper_key = self.intern_property_key(&upper.to_string())?;
                    if let Some(value) =
                        self.set_property_or_throw(realm, &object, &upper_key, lower_value)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                (None, None) => {}
            }

            lower += 1;
            upper -= 1;
        }
        Ok(Completion::Return(Value::Object(object)))
    }

    pub(in crate::runtime) fn call_array_prototype_to_reversed(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype.toReversed did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let mut values = match self.native_allocate_fast_array_values(realm, length)? {
            NativeConversion::Value(values) => values,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        for (output_index, slot) in values.iter_mut().enumerate() {
            let output_index = u64::try_from(output_index)
                .map_err(|_| RuntimeError::Invariant("Array.toReversed index exceeded Uint64"))?;
            let source_index = length - output_index - 1;
            *slot = match self.try_get_array_like_index(realm, &object, source_index)? {
                NativeConversion::Value(Some(value)) => value,
                NativeConversion::Value(None) => Value::Undefined,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        }

        Ok(Completion::Return(Value::Object(
            self.new_array_from_values(realm, values)?,
        )))
    }

    fn native_array_sort_comparator(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<NativeConversion<Option<CallableRef>>, RuntimeError> {
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Array.prototype sort comparator argv was not padded",
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

    fn collect_array_sort_slots(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        length: u64,
    ) -> Result<NativeConversion<(Vec<ArraySortSlot>, u64)>, RuntimeError> {
        let mut slots = Vec::new();
        let mut logical_capacity = 0_usize;
        let mut undefined_count = 0_u64;
        for index in 0..length {
            // QuickJS grows its ValueSlot buffer before TryGet, including for
            // the first hole. Keep that resource-failure boundary distinct
            // from the later property query.
            Self::reserve_array_sort_slot_capacity(&mut slots, &mut logical_capacity)?;
            let value = match self.try_get_array_like_index(realm, object, index)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            let Some(value) = value else {
                continue;
            };
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
                original_position: index,
            });
        }
        Ok(NativeConversion::Value((slots, undefined_count)))
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

    fn compare_array_sort_slots(
        &self,
        realm: ContextId,
        comparator: Option<&CallableRef>,
        slots: &mut [ArraySortSlot],
        left: usize,
        right: usize,
    ) -> Result<NativeConversion<ComparisonOrdering>, RuntimeError> {
        let ordering = if let Some(comparator) = comparator {
            if slots[left]
                .value
                .same_quickjs_representation(&slots[right].value)
            {
                ComparisonOrdering::Equal
            } else {
                let result = match self.call_internal(
                    realm,
                    comparator,
                    Value::Undefined,
                    &[slots[left].value.clone(), slots[right].value.clone()],
                )? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
                };
                let number = if let Value::Int(value) = result {
                    f64::from(value)
                } else {
                    match self.native_to_number(realm, &result)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(NativeConversion::Throw(value));
                        }
                    }
                };
                if number > 0.0 {
                    ComparisonOrdering::Greater
                } else if number < 0.0 {
                    ComparisonOrdering::Less
                } else {
                    ComparisonOrdering::Equal
                }
            }
        } else {
            if slots[left].cached_string.is_none() {
                let string = match self.native_to_js_string(realm, &slots[left].value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(NativeConversion::Throw(value));
                    }
                };
                slots[left].cached_string = Some(string);
            }
            if slots[right].cached_string.is_none() {
                let string = match self.native_to_js_string(realm, &slots[right].value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(NativeConversion::Throw(value));
                    }
                };
                slots[right].cached_string = Some(string);
            }
            slots[left]
                .cached_string
                .as_ref()
                .ok_or(RuntimeError::Invariant(
                    "Array sort left string cache was missing",
                ))?
                .utf16_units()
                .cmp(
                    slots[right]
                        .cached_string
                        .as_ref()
                        .ok_or(RuntimeError::Invariant(
                            "Array sort right string cache was missing",
                        ))?
                        .utf16_units(),
                )
        };

        Ok(NativeConversion::Value(if ordering.is_eq() {
            slots[left]
                .original_position
                .cmp(&slots[right].original_position)
        } else {
            ordering
        }))
    }

    fn sort_array_slots(
        &self,
        realm: ContextId,
        comparator: Option<&CallableRef>,
        slots: &mut [ArraySortSlot],
    ) -> Result<NativeConversion<()>, RuntimeError> {
        let result = quickjs_rqsort_by(slots, |slots, left, right| {
            match self.compare_array_sort_slots(realm, comparator, slots, left, right) {
                Ok(NativeConversion::Value(ordering)) => Ok(ordering),
                Ok(NativeConversion::Throw(value)) => Err(ArraySortAbort::Throw(value)),
                Err(error) => Err(ArraySortAbort::Runtime(error)),
            }
        });
        match result {
            Ok(()) => Ok(NativeConversion::Value(())),
            Err(ArraySortAbort::Throw(value)) => Ok(NativeConversion::Throw(value)),
            Err(ArraySortAbort::Runtime(error)) => Err(error),
        }
    }

    fn write_array_sort_slots(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        length: u64,
        mut slots: Vec<ArraySortSlot>,
        undefined_count: u64,
    ) -> Result<Option<Value>, RuntimeError> {
        let defined_count = u64::try_from(slots.len())
            .map_err(|_| RuntimeError::Invariant("Array sort slot count exceeded Uint64"))?;
        for (position, slot) in slots.iter_mut().enumerate() {
            slot.cached_string.take();
            let position = u64::try_from(position)
                .map_err(|_| RuntimeError::Invariant("Array sort position exceeded Uint64"))?;
            if slot.original_position == position {
                slot.value = Value::Undefined;
                continue;
            }
            let key = self.intern_property_key(&position.to_string())?;
            let value = std::mem::replace(&mut slot.value, Value::Undefined);
            if let Some(value) = self.set_property_or_throw(realm, object, &key, value)? {
                return Ok(Some(value));
            }
        }
        drop(slots);

        let mut index = defined_count;
        for _ in 0..undefined_count {
            let key = self.intern_property_key(&index.to_string())?;
            if let Some(value) =
                self.set_property_or_throw(realm, object, &key, Value::Undefined)?
            {
                return Ok(Some(value));
            }
            index += 1;
        }
        while index < length {
            if let Some(value) = self.delete_array_like_index_or_throw(realm, object, index)? {
                return Ok(Some(value));
            }
            index += 1;
        }
        Ok(None)
    }

    pub(in crate::runtime) fn call_array_prototype_sort(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let comparator = match self.native_array_sort_comparator(realm, arguments)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype.sort did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let (mut slots, undefined_count) =
            match self.collect_array_sort_slots(realm, &object, length)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        match self.sort_array_slots(realm, comparator.as_ref(), &mut slots)? {
            NativeConversion::Value(()) => {}
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        }

        if let Some(value) =
            self.write_array_sort_slots(realm, &object, length, slots, undefined_count)?
        {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(object)))
    }

    pub(in crate::runtime) fn call_array_prototype_to_sorted(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let comparator = match self.native_array_sort_comparator(realm, arguments)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype.toSorted did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let mut values = match self.native_allocate_fast_array_values(realm, length)? {
            NativeConversion::Value(values) => values,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        for (index, slot) in values.iter_mut().enumerate() {
            let index = u64::try_from(index)
                .map_err(|_| RuntimeError::Invariant("Array.toSorted index exceeded Uint64"))?;
            *slot = match self.try_get_array_like_index(realm, &object, index)? {
                NativeConversion::Value(Some(value)) => value,
                NativeConversion::Value(None) => Value::Undefined,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        }

        let (mut slots, undefined_count) = Self::collect_dense_array_sort_slots(&values)?;
        let result = self.new_array_from_values(realm, values)?;
        match self.sort_array_slots(realm, comparator.as_ref(), &mut slots)? {
            NativeConversion::Value(()) => {}
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        }
        if let Some(value) =
            self.write_array_sort_slots(realm, &result, length, slots, undefined_count)?
        {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    /// Shared Rust port of QuickJS `js_array_slice`. The upstream `splice`
    /// selector first creates and fills the deleted-elements result through
    /// ArraySpeciesCreate, then moves/deletes/inserts on the receiver while
    /// retaining every completed mutation if a later operation throws.
    pub(in crate::runtime) fn call_array_prototype_slice(
        &self,
        realm: ContextId,
        kind: ArraySliceKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;

        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype slice/splice did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let signed_length = i64::try_from(length)
            .map_err(|_| RuntimeError::Invariant("array-like length exceeded Int64"))?;
        let start = match self.native_to_int64_clamp(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Array.prototype slice/splice start argv was not padded",
            ))?,
            0,
            signed_length,
            signed_length,
        )? {
            NativeConversion::Value(value) => u64::try_from(value).map_err(|_| {
                RuntimeError::Invariant("clamped Array slice/splice start was negative")
            })?,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let (copy_count, delete_count, item_count, new_length) = match kind {
            ArraySliceKind::Slice => {
                let final_index = if arguments.actual_arg_count > 1
                    && !matches!(arguments.readable.get(1), Some(Value::Undefined))
                {
                    match self.native_to_int64_clamp(
                        realm,
                        arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                            "Array.prototype.slice end argv was not padded",
                        ))?,
                        0,
                        signed_length,
                        signed_length,
                    )? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                } else {
                    signed_length
                };
                let start = i64::try_from(start)
                    .map_err(|_| RuntimeError::Invariant("Array.slice start exceeded Int64"))?;
                let count = u64::try_from((final_index - start).max(0))
                    .map_err(|_| RuntimeError::Invariant("Array.slice count was negative"))?;
                (count, 0, 0, None)
            }
            ArraySliceKind::Splice => {
                let item_count = u64::try_from(arguments.actual_arg_count.saturating_sub(2))
                    .map_err(|_| {
                        RuntimeError::Invariant("Array.splice argument count exceeded Uint64")
                    })?;
                let delete_count = match arguments.actual_arg_count {
                    0 => 0,
                    1 => length - start,
                    _ => {
                        let remaining = length - start;
                        let remaining = i64::try_from(remaining).map_err(|_| {
                            RuntimeError::Invariant("Array.splice remaining length exceeded Int64")
                        })?;
                        match self.native_to_int64_clamp(
                            realm,
                            arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                                "Array.prototype.splice deleteCount argv was not padded",
                            ))?,
                            0,
                            remaining,
                            0,
                        )? {
                            NativeConversion::Value(value) => {
                                u64::try_from(value).map_err(|_| {
                                    RuntimeError::Invariant(
                                        "clamped Array.splice deleteCount was negative",
                                    )
                                })?
                            }
                            NativeConversion::Throw(value) => {
                                return Ok(Completion::Throw(value));
                            }
                        }
                    }
                };
                let new_length = (length - delete_count)
                    .checked_add(item_count)
                    .filter(|value| *value <= MAX_SAFE_INTEGER);
                let Some(new_length) = new_length else {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "Array loo long",
                    )?));
                };
                (delete_count, delete_count, item_count, Some(new_length))
            }
        };

        let result = match self.array_species_create(realm, &object, copy_count)? {
            Completion::Return(Value::Object(object)) => object,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "ArraySpeciesCreate returned a primitive",
                ));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        for output_index in 0..copy_count {
            let source_index = start
                .checked_add(output_index)
                .ok_or(RuntimeError::Invariant(
                    "Array slice source index overflowed",
                ))?;
            let value = match self.try_get_array_like_index(realm, &object, source_index)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if let Some(value) = value
                && let Some(value) =
                    self.create_indexed_data_property(realm, &result, output_index, value)?
            {
                return Ok(Completion::Throw(value));
            }
        }
        if let Some(value) = self.set_array_like_length(realm, &result, copy_count)? {
            return Ok(Completion::Throw(value));
        }

        if matches!(kind, ArraySliceKind::Slice) {
            return Ok(Completion::Return(Value::Object(result)));
        }

        let new_length = new_length.ok_or(RuntimeError::Invariant(
            "Array.splice new length was not computed",
        ))?;
        if item_count != delete_count {
            let from = start
                .checked_add(delete_count)
                .ok_or(RuntimeError::Invariant(
                    "Array.splice tail start overflowed",
                ))?;
            let to = start
                .checked_add(item_count)
                .ok_or(RuntimeError::Invariant(
                    "Array.splice target start overflowed",
                ))?;
            let count = length - from;
            if let Some(value) = self.copy_array_like_range(
                realm,
                &object,
                to,
                from,
                count,
                item_count > delete_count,
            )? {
                return Ok(Completion::Throw(value));
            }

            for index in (new_length..length).rev() {
                if let Some(value) = self.delete_array_like_index_or_throw(realm, &object, index)? {
                    return Ok(Completion::Throw(value));
                }
            }
        }

        for (offset, value) in arguments.readable[..arguments.actual_arg_count]
            .iter()
            .skip(2)
            .cloned()
            .enumerate()
        {
            let offset = u64::try_from(offset)
                .map_err(|_| RuntimeError::Invariant("Array.splice item offset exceeded Uint64"))?;
            let index = start.checked_add(offset).ok_or(RuntimeError::Invariant(
                "Array.splice item index overflowed",
            ))?;
            let key = self.intern_property_key(&index.to_string())?;
            if let Some(value) = self.set_property_or_throw(realm, &object, &key, value)? {
                return Ok(Completion::Throw(value));
            }
        }
        if let Some(value) = self.set_array_like_length(realm, &object, new_length)? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    /// QuickJS `js_array_toSpliced`: allocate a defining-realm dense base
    /// Array, copy the prefix and suffix through conditional Has/Get queries,
    /// and place the supplied values between them without consulting species.
    pub(in crate::runtime) fn call_array_prototype_to_spliced(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;

        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array.prototype.toSpliced did not receive a generic invocation",
            ));
        };
        let (object, length) = match self.native_array_like_object_and_length(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let signed_length = i64::try_from(length)
            .map_err(|_| RuntimeError::Invariant("array-like length exceeded Int64"))?;
        let start = if arguments.actual_arg_count == 0 {
            0
        } else {
            match self.native_to_int64_clamp(
                realm,
                arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "Array.prototype.toSpliced start argv was not padded",
                ))?,
                0,
                signed_length,
                signed_length,
            )? {
                NativeConversion::Value(value) => u64::try_from(value).map_err(|_| {
                    RuntimeError::Invariant("clamped Array.toSpliced start was negative")
                })?,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        };
        let mut delete_count = if arguments.actual_arg_count == 0 {
            0
        } else {
            length - start
        };
        if arguments.actual_arg_count > 1 {
            let remaining = i64::try_from(delete_count).map_err(|_| {
                RuntimeError::Invariant("Array.toSpliced remaining length exceeded Int64")
            })?;
            delete_count = match self.native_to_int64_clamp(
                realm,
                arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                    "Array.prototype.toSpliced deleteCount argv was not padded",
                ))?,
                0,
                remaining,
                0,
            )? {
                NativeConversion::Value(value) => u64::try_from(value).map_err(|_| {
                    RuntimeError::Invariant("clamped Array.toSpliced deleteCount was negative")
                })?,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        }
        let item_count =
            u64::try_from(arguments.actual_arg_count.saturating_sub(2)).map_err(|_| {
                RuntimeError::Invariant("Array.toSpliced argument count exceeded Uint64")
            })?;
        let new_length = (length - delete_count)
            .checked_add(item_count)
            .filter(|value| *value <= MAX_SAFE_INTEGER);
        let Some(new_length) = new_length else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "invalid array length",
            )?));
        };
        let mut values = match self.native_allocate_fast_array_values(realm, new_length)? {
            NativeConversion::Value(values) => values,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let mut output_index = 0_usize;
        for source_index in 0..start {
            values[output_index] =
                match self.try_get_array_like_index(realm, &object, source_index)? {
                    NativeConversion::Value(Some(value)) => value,
                    NativeConversion::Value(None) => Value::Undefined,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            output_index += 1;
        }
        for value in arguments.readable[..arguments.actual_arg_count]
            .iter()
            .skip(2)
        {
            values[output_index] = value.clone();
            output_index += 1;
        }
        let suffix_start = start
            .checked_add(delete_count)
            .ok_or(RuntimeError::Invariant(
                "Array.toSpliced suffix start overflowed",
            ))?;
        for source_index in suffix_start..length {
            values[output_index] =
                match self.try_get_array_like_index(realm, &object, source_index)? {
                    NativeConversion::Value(Some(value)) => value,
                    NativeConversion::Value(None) => Value::Undefined,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            output_index += 1;
        }
        if output_index != values.len() {
            return Err(RuntimeError::Invariant(
                "Array.toSpliced did not fill its dense result",
            ));
        }

        Ok(Completion::Return(Value::Object(
            self.new_array_from_values(realm, values)?,
        )))
    }

    pub(in crate::runtime) fn call_array_prototype_iterator(
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

    pub(in crate::runtime) fn call_array_iterator_next(
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

    pub(in crate::runtime) fn call_array_iterator_next_raw(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array Iterator next did not receive an iterator-next invocation",
            ));
        };
        let Value::Object(iterator) = this_value else {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "Array Iterator object expected",
                )?,
            )));
        };
        let state = self
            .0
            .state
            .borrow()
            .heap
            .array_iterator_state(iterator.object_id());
        let (source, index, kind) = match state {
            Ok(state) => state,
            Err(HeapError::Invariant(_)) => {
                return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                    self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "Array Iterator object expected",
                    )?,
                )));
            }
            Err(error) => return Err(error.into()),
        };
        let Some(source) = source else {
            return Ok(NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Undefined,
                done: true,
            });
        };
        let source = ObjectRef::from_borrowed_handle(self.clone(), source)?;
        let length_key = self.intern_property_key("length")?;
        let length_value = match self.get_property_in_realm(realm, &source, &length_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
            }
        };
        let length = match self.native_to_number(realm, &length_value)? {
            NativeConversion::Value(value) => Self::to_uint32_number(value),
            NativeConversion::Throw(value) => {
                return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
            }
        };
        if index >= length {
            let mut state = self.0.state.borrow_mut();
            let cleanup = state.heap.finish_array_iterator(iterator.object_id())?;
            state.apply_cleanup(cleanup)?;
            return Ok(NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Undefined,
                done: true,
            });
        }
        self.0
            .state
            .borrow_mut()
            .heap
            .set_array_iterator_index(iterator.object_id(), index + 1)?;
        let key_value = Self::array_length_value(index);
        let value = match kind {
            ArrayIteratorKind::Key => key_value,
            ArrayIteratorKind::Value | ArrayIteratorKind::KeyAndValue => {
                let key = self.intern_property_key(&index.to_string())?;
                let value = match self.get_property_in_realm(realm, &source, &key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => {
                        return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
                    }
                };
                if kind == ArrayIteratorKind::KeyAndValue {
                    Value::Object(self.new_array_from_values(realm, vec![key_value, value])?)
                } else {
                    value
                }
            }
        };
        Ok(NativeInvokeOutcome::IteratorNextRaw { value, done: false })
    }
}

#[cfg(test)]
mod tests;
