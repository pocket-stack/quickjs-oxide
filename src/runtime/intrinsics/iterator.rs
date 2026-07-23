//! `%Iterator%`, `Iterator.from`, and synchronous Iterator Helpers.
//!
//! The implementation follows the pinned QuickJS iterator slice rather than
//! treating the proposal methods as Array conveniences.  In particular,
//! iterator records cache `next`, eager consumers preserve QuickJS's close
//! precedence, and lazy helpers keep their re-entrancy and completion state in
//! traced heap payloads.

use crate::heap::{
    IteratorConsumerKind, IteratorHelperData, IteratorHelperKind, IteratorRealmData,
    IteratorResumeKind,
};

use super::super::*;
use super::object::ObjectIteratorStep;

enum IteratorClose {
    Closed,
    Throw(Value),
}

enum HelperStep {
    Result { value: Value, done: bool },
    Throw { value: Value, close_outer: bool },
}

/// Pinned QuickJS `JS_ToInt64Free` for an already numeric value.
///
/// Rust's float-to-integer cast saturates outside the signed range, whereas
/// QuickJS preserves the low 64 bits for finite values whose binary exponent
/// is still close enough to the mantissa, and maps still larger magnitudes to
/// zero. Iterator `drop`/`take` expose that representation-level behavior
/// after `JS_ToIntegerFree`, so keep the bit-level conversion local and exact.
fn quickjs_to_int64_free(number: f64) -> i64 {
    const EXPONENT_BIAS: u64 = 1023;
    const MANTISSA_BITS: u64 = 52;
    const MANTISSA_MASK: u64 = (1_u64 << MANTISSA_BITS) - 1;

    let bits = number.to_bits();
    let exponent = (bits >> MANTISSA_BITS) & 0x7ff;
    if exponent <= EXPONENT_BIAS + 62 {
        // The magnitude is strictly below 2^63, so this cast cannot saturate.
        return number as i64;
    }
    if exponent <= EXPONENT_BIAS + 62 + 53 {
        let significand = (bits & MANTISSA_MASK) | (1_u64 << MANTISSA_BITS);
        let shift = u32::try_from(exponent - EXPONENT_BIAS - MANTISSA_BITS)
            .expect("QuickJS ToInt64 exponent shift fits u32");
        let low_bits = significand << shift;
        let signed = low_bits as i64;
        return if bits >> 63 == 0 {
            signed
        } else {
            signed.wrapping_neg()
        };
    }
    0
}

impl Runtime {
    pub(in crate::runtime) fn initialize_iterator_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        iterator_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let helper_prototype = self.new_object(Some(iterator_prototype))?;
        let wrap_prototype = self.new_object(Some(iterator_prototype))?;

        // This is the exact QuickJS table order.  The unusual constructor
        // accessor follows the string methods, while both symbols come last.
        for (kind, name) in [
            (IteratorHelperKind::Drop, "drop"),
            (IteratorHelperKind::Filter, "filter"),
            (IteratorHelperKind::FlatMap, "flatMap"),
            (IteratorHelperKind::Map, "map"),
            (IteratorHelperKind::Take, "take"),
        ] {
            self.define_native_builtin_auto_init(
                iterator_prototype,
                realm,
                NativeFunctionId::IteratorPrototypeCreateHelper(kind),
                name,
                1,
                1,
            )?;
        }
        for (kind, name) in [
            (IteratorConsumerKind::Every, "every"),
            (IteratorConsumerKind::Find, "find"),
            (IteratorConsumerKind::ForEach, "forEach"),
            (IteratorConsumerKind::Some, "some"),
        ] {
            self.define_native_builtin_auto_init(
                iterator_prototype,
                realm,
                NativeFunctionId::IteratorPrototypeConsume(kind),
                name,
                1,
                1,
            )?;
        }
        self.define_native_builtin_auto_init(
            iterator_prototype,
            realm,
            NativeFunctionId::IteratorPrototypeReduce,
            "reduce",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            iterator_prototype,
            realm,
            NativeFunctionId::IteratorPrototypeToArray,
            "toArray",
            0,
            0,
        )?;

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::IteratorConstructor,
            0,
            "Iterator",
            0,
        )?;
        self.define_native_builtin_auto_init(
            constructor.as_object(),
            realm,
            NativeFunctionId::IteratorFrom,
            "from",
            1,
            1,
        )?;
        self.define_function_data_property(
            constructor.as_object(),
            "prototype",
            Value::Object(iterator_prototype.clone()),
            false,
            false,
        )?;

        // QuickJS deliberately uses one nameless CFunctionData object as both
        // accessor halves.  Identity is observable.
        let constructor_accessor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::IteratorConstructorAccessor,
            0,
            "",
            0,
        )?;
        let constructor_key = self.intern_property_key("constructor")?;
        if !self.define_own_property(
            iterator_prototype,
            &constructor_key,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(
                    constructor_accessor.clone(),
                )),
                set: DescriptorField::Present(AccessorValue::Callable(constructor_accessor)),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype constructor accessor was rejected",
            ));
        }

        let iterator_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        self.define_native_builtin_auto_init_with_key(
            iterator_prototype,
            realm,
            &iterator_key,
            NativeFunctionId::IteratorPrototypeIterator,
            "[Symbol.iterator]",
            0,
            0,
            PropertyFlags::data(true, false, true),
        )?;

        // `%IteratorPrototype%[@@toStringTag]` is an accessor, not the data
        // property used by concrete iterator prototypes.
        let tag_getter = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::IteratorPrototypeToStringTagGetter,
            0,
            "get [Symbol.toStringTag]",
            0,
        )?;
        let tag_setter = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::IteratorPrototypeToStringTagSetter,
            1,
            "set [Symbol.toStringTag]",
            1,
        )?;
        let tag_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            iterator_prototype,
            &tag_key,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(tag_getter)),
                set: DescriptorField::Present(AccessorValue::Callable(tag_setter)),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype toStringTag definition was rejected",
            ));
        }

        for (kind, name) in [
            (IteratorResumeKind::Next, "next"),
            (IteratorResumeKind::Return, "return"),
        ] {
            self.define_native_builtin_auto_init(
                &helper_prototype,
                realm,
                NativeFunctionId::IteratorHelperResume(kind),
                name,
                0,
                0,
            )?;
        }
        self.define_iterator_data_tag(&helper_prototype, "Iterator Helper")?;

        for (kind, name) in [
            (IteratorResumeKind::Next, "next"),
            (IteratorResumeKind::Return, "return"),
        ] {
            self.define_native_builtin_auto_init(
                &wrap_prototype,
                realm,
                NativeFunctionId::IteratorWrapResume(kind),
                name,
                0,
                0,
            )?;
        }

        self.define_function_data_property(
            global_object,
            "Iterator",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.0.state.borrow_mut().heap.attach_iterator_intrinsics(
            realm,
            IteratorRealmData {
                constructor: constructor.as_object().object_id(),
                helper_prototype: helper_prototype.object_id(),
                wrap_prototype: wrap_prototype.object_id(),
            },
        )?;
        Ok(())
    }

    fn define_iterator_data_tag(
        &self,
        object: &ObjectRef,
        value: &'static str,
    ) -> Result<(), RuntimeError> {
        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static(value))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Iterator helper toStringTag definition was rejected",
            ));
        }
        Ok(())
    }

    fn iterator_realm_data(&self, realm: ContextId) -> Result<IteratorRealmData, RuntimeError> {
        self.0
            .state
            .borrow()
            .heap
            .context(realm)?
            .iterator
            .ok_or(RuntimeError::Invariant("realm has no Iterator intrinsics"))
    }

    fn iterator_receiver(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator prototype method did not receive a generic invocation",
            ));
        };
        let Value::Object(object) = this_value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        Ok(NativeConversion::Value(object))
    }

    fn iterator_prototype_from_new_target(
        &self,
        realm: ContextId,
        new_target: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(new_target) = new_target else {
            return Err(RuntimeError::Invariant(
                "Iterator constructor new.target was not an object",
            ));
        };
        let key = self.intern_property_key("prototype")?;
        match self.get_property_in_realm(realm, &new_target, &key)? {
            Completion::Return(Value::Object(prototype)) => Ok(NativeConversion::Value(prototype)),
            Completion::Return(_) => {
                let callable = self.callable_from_value(Value::Object(new_target))?;
                let fallback_realm = self.callable_realm(&callable)?;
                let prototype = self
                    .0
                    .state
                    .borrow()
                    .heap
                    .context(fallback_realm)?
                    .iterator_prototype;
                Ok(NativeConversion::Value(ObjectRef::from_borrowed_handle(
                    self.clone(),
                    prototype,
                )?))
            }
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    pub(in crate::runtime) fn call_iterator_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let new_target = match invocation {
            NativeInvocation::Construct {
                new_target: Value::Undefined,
            }
            | NativeInvocation::Call { .. } => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "constructor requires 'new'",
                )?));
            }
            NativeInvocation::Construct { new_target } => new_target,
            NativeInvocation::Getter { .. } | NativeInvocation::Setter { .. } => {
                return Err(RuntimeError::Invariant(
                    "Iterator constructor received an accessor invocation",
                ));
            }
        };
        let Value::Object(new_target_object) = &new_target else {
            return Err(RuntimeError::Invariant(
                "Iterator constructor new.target was not an object",
            ));
        };
        let new_target_is_native_iterator = {
            let state = self.0.state.borrow();
            matches!(
                &state.heap.object(new_target_object.object_id())?.payload,
                ObjectPayload::NativeFunction { data, .. }
                    if data.target == NativeFunctionId::IteratorConstructor
            )
        };
        if new_target_is_native_iterator {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "abstract class not constructable",
            )?));
        }
        let prototype = match self.iterator_prototype_from_new_target(realm, new_target)? {
            NativeConversion::Value(prototype) => prototype,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Object(
            self.new_object(Some(&prototype))?,
        )))
    }

    pub(in crate::runtime) fn call_iterator_constructor_accessor(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator constructor accessor did not receive a generic invocation",
            ));
        };
        let defining_realm = {
            let state = self.0.state.borrow();
            let ObjectPayload::NativeFunction { data, .. } =
                &state.heap.object(callable.as_object().object_id())?.payload
            else {
                return Err(RuntimeError::Invariant(
                    "Iterator constructor accessor callable lost its native payload",
                ));
            };
            if data.target != NativeFunctionId::IteratorConstructorAccessor {
                return Err(RuntimeError::Invariant(
                    "Iterator constructor accessor callable changed target",
                ));
            }
            data.realm.ok_or(RuntimeError::Invariant(
                "Iterator constructor accessor lost its defining realm",
            ))?
        };
        if arguments.actual_arg_count == 0 {
            let constructor = self.iterator_realm_data(defining_realm)?.constructor;
            return Ok(Completion::Return(Value::Object(
                ObjectRef::from_borrowed_handle(self.clone(), constructor)?,
            )));
        }
        let Some(Value::Object(value)) = arguments.readable.first() else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let Value::Object(receiver) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let key = self.intern_property_key("constructor")?;
        if !self.define_own_property(
            &receiver,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::Object(value.clone())),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot define property",
            )?));
        }
        Ok(Completion::Return(Value::Undefined))
    }

    pub(in crate::runtime) fn call_iterator_from(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator.from did not receive a generic invocation",
            ));
        };
        let input = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Iterator.from argument was not padded",
            ))?;
        if !matches!(input, Value::Object(_) | Value::String(_)) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "Iterator.from called on non-object",
            )?));
        }
        let iterator_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        let method = match self.get_value_property_in_realm(realm, input.clone(), &iterator_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let iterator = if matches!(method, Value::Undefined | Value::Null) {
            input
        } else {
            let method = match self.iterator_callable_value(realm, method)? {
                NativeConversion::Value(method) => method,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            match self.call_internal(realm, &method, input, &[])? {
                Completion::Return(Value::Object(iterator)) => Value::Object(iterator),
                Completion::Return(_) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not an object",
                    )?));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        };

        // QuickJS gets `next` before its OrdinaryIsInstanceOf check.
        let next_key = self.intern_property_key("next")?;
        let next = match self.get_value_property_in_realm(realm, iterator.clone(), &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let constructor = self.iterator_realm_data(realm)?.constructor;
        let constructor = ObjectRef::from_borrowed_handle(self.clone(), constructor)?;
        let constructor = CallableRef::from_validated_object(constructor);
        match self.ordinary_is_instance_of(realm, &constructor, iterator.clone())? {
            Completion::Return(value) if value.to_boolean() => Ok(Completion::Return(iterator)),
            Completion::Return(_) => Ok(Completion::Return(Value::Object(
                self.new_iterator_wrap(realm, &iterator, &next)?,
            ))),
            Completion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    fn iterator_callable_value(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<CallableRef>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(callable) = self.as_callable(&object)? else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        Ok(NativeConversion::Value(callable))
    }

    fn new_iterator_wrap(
        &self,
        realm: ContextId,
        source: &Value,
        next: &Value,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.iterator_realm_data(realm)?.wrap_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        let raw_source = self.raw_property_value(source)?;
        let raw_next = self.raw_property_value(next)?;
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let retained_atoms = match state.retain_raw_value_atoms([&raw_source, &raw_next]) {
            Ok(atoms) => atoms,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error);
            }
        };
        let object = match state.heap.allocate_object(ObjectData::iterator_wrap(
            shape,
            Vec::new(),
            raw_source,
            raw_next,
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
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    pub(in crate::runtime) fn call_iterator_wrap_resume(
        &self,
        realm: ContextId,
        kind: IteratorResumeKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let receiver = match self.iterator_receiver(realm, invocation)? {
            NativeConversion::Value(receiver) => receiver,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let wrap_state = {
            let state = self.0.state.borrow();
            state.heap.iterator_wrap_state(receiver.object_id())
        };
        let (source, next) = match wrap_state {
            Ok(state) => state,
            Err(HeapError::Invariant(_)) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an Iterator Wrap",
                )?));
            }
            Err(error) => return Err(error.into()),
        };
        let source = self.root_raw_value(&source)?;
        match kind {
            IteratorResumeKind::Next => {
                let method = self.root_raw_value(&next)?;
                let step = match &source {
                    Value::Object(source) => self.object_iterator_next(realm, source, method)?,
                    _ => self.iterator_wrap_primitive_next(realm, source.clone(), method)?,
                };
                match step {
                    ObjectIteratorStep::Yield(value) => Ok(Completion::Return(Value::Object(
                        self.new_iterator_result(realm, value, false)?,
                    ))),
                    ObjectIteratorStep::Done => Ok(Completion::Return(Value::Object(
                        self.new_iterator_result(realm, Value::Undefined, true)?,
                    ))),
                    ObjectIteratorStep::Throw(value) => Ok(Completion::Throw(value)),
                }
            }
            IteratorResumeKind::Return => {
                let key = self.intern_property_key("return")?;
                let method = match self.get_value_property_in_realm(realm, source.clone(), &key)? {
                    Completion::Return(Value::Undefined | Value::Null) => {
                        return Ok(Completion::Return(Value::Object(
                            self.new_iterator_result(realm, Value::Undefined, true)?,
                        )));
                    }
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let method = match self.iterator_callable_value(realm, method)? {
                    NativeConversion::Value(method) => method,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                match self.call_internal(realm, &method, source, &[])? {
                    Completion::Return(Value::Object(result)) => {
                        Ok(Completion::Return(Value::Object(result)))
                    }
                    Completion::Return(_) => Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "iterator must return an object",
                    )?)),
                    Completion::Throw(value) => Ok(Completion::Throw(value)),
                }
            }
        }
    }

    fn iterator_wrap_primitive_next(
        &self,
        realm: ContextId,
        source: Value,
        method: Value,
    ) -> Result<ObjectIteratorStep, RuntimeError> {
        let method = match self.iterator_callable_value(realm, method)? {
            NativeConversion::Value(method) => method,
            NativeConversion::Throw(value) => return Ok(ObjectIteratorStep::Throw(value)),
        };
        let result = match self.call_internal(realm, &method, source, &[])? {
            Completion::Return(result) => result,
            Completion::Throw(value) => return Ok(ObjectIteratorStep::Throw(value)),
        };
        let Value::Object(result) = result else {
            return Ok(ObjectIteratorStep::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "iterator must return an object",
            )?));
        };

        let done_key = self.intern_property_key("done")?;
        let done = match self.get_property_in_realm(realm, &result, &done_key)? {
            Completion::Return(value) => value.to_boolean(),
            Completion::Throw(value) => return Ok(ObjectIteratorStep::Throw(value)),
        };
        if done {
            return Ok(ObjectIteratorStep::Done);
        }

        let value_key = self.intern_property_key("value")?;
        match self.get_property_in_realm(realm, &result, &value_key)? {
            Completion::Return(value) => Ok(ObjectIteratorStep::Yield(value)),
            Completion::Throw(value) => Ok(ObjectIteratorStep::Throw(value)),
        }
    }
}

impl Runtime {
    pub(in crate::runtime) fn call_iterator_create_helper(
        &self,
        realm: ContextId,
        kind: IteratorHelperKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let source = match self.iterator_receiver(realm, invocation)? {
            NativeConversion::Value(source) => source,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Iterator helper argument was not padded",
            ))?;

        let (callback, count) = match kind {
            IteratorHelperKind::Drop | IteratorHelperKind::Take => {
                let number = match self.native_to_number(realm, &argument)? {
                    NativeConversion::Value(number) => number,
                    NativeConversion::Throw(value) => {
                        self.close_iterator_preserving_throw(realm, &source)?;
                        return Ok(Completion::Throw(value));
                    }
                };
                if number.is_nan() || number == f64::NEG_INFINITY {
                    self.close_iterator_preserving_throw(realm, &source)?;
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Range,
                        "must be positive",
                    )?));
                }
                let count = if number == f64::INFINITY {
                    (1_i64 << 53) - 1
                } else {
                    quickjs_to_int64_free(number.trunc())
                };
                if count < 0 {
                    self.close_iterator_preserving_throw(realm, &source)?;
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Range,
                        "must be positive",
                    )?));
                }
                (Value::Undefined, count)
            }
            IteratorHelperKind::Filter | IteratorHelperKind::FlatMap | IteratorHelperKind::Map => {
                if let NativeConversion::Throw(value) =
                    self.iterator_callable_value(realm, argument.clone())?
                {
                    self.close_iterator_preserving_throw(realm, &source)?;
                    return Ok(Completion::Throw(value));
                }
                (argument, 0)
            }
        };

        let next_key = self.intern_property_key("next")?;
        let next = match self.get_property_in_realm(realm, &source, &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                self.close_iterator_preserving_throw(realm, &source)?;
                return Ok(Completion::Throw(value));
            }
        };
        Ok(Completion::Return(Value::Object(
            self.new_iterator_helper(realm, &source, &next, &callback, count, kind)?,
        )))
    }

    fn new_iterator_helper(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        next: &Value,
        callback: &Value,
        count: i64,
        kind: IteratorHelperKind,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.iterator_realm_data(realm)?.helper_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        let data = IteratorHelperData {
            source: source.object_id(),
            next: self.raw_property_value(next)?,
            callback: self.raw_property_value(callback)?,
            inner: None,
            count,
            kind,
            executing: false,
            done: false,
        };
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let retained_atoms = match state.retain_raw_value_atoms([&data.next, &data.callback]) {
            Ok(atoms) => atoms,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error);
            }
        };
        let object =
            match state
                .heap
                .allocate_object(ObjectData::iterator_helper(shape, Vec::new(), data))
            {
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
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    pub(in crate::runtime) fn call_iterator_consumer(
        &self,
        realm: ContextId,
        kind: IteratorConsumerKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let source = match self.iterator_receiver(realm, invocation)? {
            NativeConversion::Value(source) => source,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let callback_value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Iterator consumer callback was not padded",
            ))?;
        let callback = match self.iterator_callable_value(realm, callback_value)? {
            NativeConversion::Value(callback) => callback,
            NativeConversion::Throw(value) => {
                self.close_iterator_preserving_throw(realm, &source)?;
                return Ok(Completion::Throw(value));
            }
        };
        let next_key = self.intern_property_key("next")?;
        let next = match self.get_property_in_realm(realm, &source, &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let mut index = 0_i64;
        loop {
            let item = match self.object_iterator_next(realm, &source, next.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
                    let value = match kind {
                        IteratorConsumerKind::Every => Value::Bool(true),
                        IteratorConsumerKind::Some => Value::Bool(false),
                        IteratorConsumerKind::Find | IteratorConsumerKind::ForEach => {
                            Value::Undefined
                        }
                    };
                    return Ok(Completion::Return(value));
                }
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let selected = match self.call_internal(
                realm,
                &callback,
                Value::Undefined,
                &[item.clone(), Value::number(index as f64)],
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &source)?;
                    return Ok(Completion::Throw(value));
                }
            };
            index = index.wrapping_add(1);

            let early = match kind {
                IteratorConsumerKind::Every if !selected.to_boolean() => Some(Value::Bool(false)),
                IteratorConsumerKind::Some if selected.to_boolean() => Some(Value::Bool(true)),
                IteratorConsumerKind::Find if selected.to_boolean() => Some(item),
                IteratorConsumerKind::Every
                | IteratorConsumerKind::Some
                | IteratorConsumerKind::Find
                | IteratorConsumerKind::ForEach => None,
            };
            if let Some(value) = early {
                return Ok(match self.iterator_close_normal(realm, &source)? {
                    IteratorClose::Closed => Completion::Return(value),
                    IteratorClose::Throw(value) => Completion::Throw(value),
                });
            }
        }
    }

    pub(in crate::runtime) fn call_iterator_reduce(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let source = match self.iterator_receiver(realm, invocation)? {
            NativeConversion::Value(source) => source,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let callback_value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Iterator reduce callback was not padded",
            ))?;
        let callback = match self.iterator_callable_value(realm, callback_value)? {
            NativeConversion::Value(callback) => callback,
            NativeConversion::Throw(value) => {
                self.close_iterator_preserving_throw(realm, &source)?;
                return Ok(Completion::Throw(value));
            }
        };
        let next_key = self.intern_property_key("next")?;
        let next = match self.get_property_in_realm(realm, &source, &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                self.close_iterator_preserving_throw(realm, &source)?;
                return Ok(Completion::Throw(value));
            }
        };

        let (mut accumulator, mut index) = if arguments.actual_arg_count > 1 {
            (
                arguments
                    .readable
                    .get(1)
                    .cloned()
                    .ok_or(RuntimeError::Invariant(
                        "Iterator reduce initial value disappeared",
                    ))?,
                0_i64,
            )
        } else {
            match self.object_iterator_next(realm, &source, next.clone())? {
                ObjectIteratorStep::Yield(value) => (value, 1),
                ObjectIteratorStep::Done => {
                    let value =
                        self.new_native_error(realm, NativeErrorKind::Type, "empty iterator")?;
                    self.close_iterator_preserving_throw(realm, &source)?;
                    return Ok(Completion::Throw(value));
                }
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            }
        };

        loop {
            let item = match self.object_iterator_next(realm, &source, next.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => return Ok(Completion::Return(accumulator)),
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            accumulator = match self.call_internal(
                realm,
                &callback,
                Value::Undefined,
                &[accumulator, item, Value::number(index as f64)],
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &source)?;
                    return Ok(Completion::Throw(value));
                }
            };
            index = index.wrapping_add(1);
        }
    }

    pub(in crate::runtime) fn call_iterator_to_array(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let source = match self.iterator_receiver(realm, invocation)? {
            NativeConversion::Value(source) => source,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let next_key = self.intern_property_key("next")?;
        let next = match self.get_property_in_realm(realm, &source, &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = self.new_array(realm)?;
        let mut index = 0_u32;
        loop {
            let item = match self.object_iterator_next(realm, &source, next.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Object(result)));
                }
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if let Some(value) = self.create_array_data_property(realm, &result, index, item)? {
                return Ok(Completion::Throw(value));
            }
            index = index.checked_add(1).ok_or_else(|| {
                RuntimeError::Engine(Error::new(ErrorKind::Range, "invalid array length"))
            })?;
        }
    }

    fn iterator_close_normal(
        &self,
        realm: ContextId,
        iterator: &ObjectRef,
    ) -> Result<IteratorClose, RuntimeError> {
        let key = self.intern_property_key("return")?;
        let method = match self.get_property_in_realm(realm, iterator, &key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(IteratorClose::Throw(value)),
        };
        if matches!(method, Value::Undefined | Value::Null) {
            return Ok(IteratorClose::Closed);
        }
        let method = match self.iterator_callable_value(realm, method)? {
            NativeConversion::Value(method) => method,
            NativeConversion::Throw(value) => return Ok(IteratorClose::Throw(value)),
        };
        match self.call_internal(realm, &method, Value::Object(iterator.clone()), &[])? {
            Completion::Return(Value::Object(_)) => Ok(IteratorClose::Closed),
            Completion::Return(_) => Ok(IteratorClose::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?)),
            Completion::Throw(value) => Ok(IteratorClose::Throw(value)),
        }
    }
}

impl Runtime {
    pub(in crate::runtime) fn call_iterator_helper_resume(
        &self,
        realm: ContextId,
        mode: IteratorResumeKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let helper = match self.iterator_receiver(realm, invocation)? {
            NativeConversion::Value(helper) => helper,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let state_result = {
            let state = self.0.state.borrow();
            state.heap.iterator_helper_state(helper.object_id())
        };
        let state = match state_result {
            Ok(state) => state,
            Err(HeapError::Invariant(_)) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an Iterator Helper",
                )?));
            }
            Err(error) => return Err(error.into()),
        };
        if state.executing {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot invoke a running iterator",
            )?));
        }
        if state.done {
            return Ok(Completion::Return(Value::Object(
                self.new_iterator_result(realm, Value::Undefined, true)?,
            )));
        }
        self.0
            .state
            .borrow_mut()
            .heap
            .set_iterator_helper_running(helper.object_id(), true)?;

        let source = ObjectRef::from_borrowed_handle(self.clone(), state.source)?;
        let step = match self.resume_iterator_helper(realm, &helper, &source, &state, mode) {
            Ok(step) => step,
            Err(error) => {
                self.0
                    .state
                    .borrow_mut()
                    .heap
                    .set_iterator_helper_running(helper.object_id(), false)?;
                return Err(error);
            }
        };
        let step = match step {
            HelperStep::Throw {
                value,
                close_outer: true,
            } => {
                if let Err(error) = self.close_iterator_preserving_throw(realm, &source) {
                    self.0
                        .state
                        .borrow_mut()
                        .heap
                        .set_iterator_helper_running(helper.object_id(), false)?;
                    return Err(error);
                }
                HelperStep::Throw {
                    value,
                    close_outer: false,
                }
            }
            step => step,
        };
        let done = match (&step, mode) {
            (_, IteratorResumeKind::Return) => true,
            (HelperStep::Result { done, .. }, IteratorResumeKind::Next) => *done,
            // QuickJS marks a `take` helper done before performing the
            // exhaustion close.  A throwing `return` therefore still
            // exhausts the helper and later `next()` calls do not close the
            // source again.
            (HelperStep::Throw { .. }, IteratorResumeKind::Next) => {
                state.kind == IteratorHelperKind::Take && state.count == 0
            }
        };
        self.0
            .state
            .borrow_mut()
            .heap
            .set_iterator_helper_done_and_running(helper.object_id(), done, false)?;

        match step {
            HelperStep::Result { value, done } => Ok(Completion::Return(Value::Object(
                self.new_iterator_result(realm, value, done)?,
            ))),
            HelperStep::Throw { value, .. } => Ok(Completion::Throw(value)),
        }
    }

    fn resume_iterator_helper(
        &self,
        realm: ContextId,
        helper: &ObjectRef,
        source: &ObjectRef,
        state: &IteratorHelperData,
        mode: IteratorResumeKind,
    ) -> Result<HelperStep, RuntimeError> {
        match state.kind {
            IteratorHelperKind::Drop => {
                self.resume_iterator_drop(realm, helper, source, state, mode)
            }
            IteratorHelperKind::Filter => {
                self.resume_iterator_filter(realm, helper, source, state, mode)
            }
            IteratorHelperKind::FlatMap => {
                self.resume_iterator_flat_map(realm, helper, source, state, mode)
            }
            IteratorHelperKind::Map => self.resume_iterator_map(realm, helper, source, state, mode),
            IteratorHelperKind::Take => {
                self.resume_iterator_take(realm, helper, source, state, mode)
            }
        }
    }

    fn helper_method(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        cached_next: &RawValue,
        mode: IteratorResumeKind,
    ) -> Result<Result<Value, Value>, RuntimeError> {
        if mode == IteratorResumeKind::Next {
            return Ok(Ok(self.root_raw_value(cached_next)?));
        }
        let key = self.intern_property_key("return")?;
        Ok(match self.get_property_in_realm(realm, source, &key)? {
            Completion::Return(value) => Ok(value),
            Completion::Throw(value) => Err(value),
        })
    }

    fn helper_step_with_method(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        method: Value,
    ) -> Result<HelperStep, RuntimeError> {
        Ok(match self.object_iterator_next(realm, source, method)? {
            ObjectIteratorStep::Yield(value) => HelperStep::Result { value, done: false },
            ObjectIteratorStep::Done => HelperStep::Result {
                value: Value::Undefined,
                done: true,
            },
            ObjectIteratorStep::Throw(value) => HelperStep::Throw {
                value,
                close_outer: false,
            },
        })
    }

    fn set_helper_count(&self, helper: &ObjectRef, count: i64) -> Result<(), RuntimeError> {
        self.0
            .state
            .borrow_mut()
            .heap
            .set_iterator_helper_count(helper.object_id(), count)?;
        Ok(())
    }

    fn set_helper_inner(
        &self,
        helper: &ObjectRef,
        inner: Option<&ObjectRef>,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let cleanup = state
            .heap
            .set_iterator_helper_inner(helper.object_id(), inner.map(ObjectRef::object_id))?;
        state.apply_cleanup(cleanup)
    }

    fn resume_iterator_drop(
        &self,
        realm: ContextId,
        helper: &ObjectRef,
        source: &ObjectRef,
        state: &IteratorHelperData,
        mode: IteratorResumeKind,
    ) -> Result<HelperStep, RuntimeError> {
        let method = match self.helper_method(realm, source, &state.next, mode)? {
            Ok(method) => method,
            Err(value) => {
                return Ok(HelperStep::Throw {
                    value,
                    close_outer: true,
                });
            }
        };
        let mut count = state.count;
        while count > 0 {
            count -= 1;
            self.set_helper_count(helper, count)?;
            let step = self.helper_step_with_method(realm, source, method.clone())?;
            match step {
                HelperStep::Throw { value, .. } => {
                    return Ok(HelperStep::Throw {
                        value,
                        close_outer: false,
                    });
                }
                HelperStep::Result { done: true, .. } => {
                    return Ok(HelperStep::Result {
                        value: Value::Undefined,
                        done: true,
                    });
                }
                HelperStep::Result { done: false, .. } if mode == IteratorResumeKind::Return => {
                    return Ok(HelperStep::Result {
                        value: Value::Undefined,
                        done: true,
                    });
                }
                HelperStep::Result { done: false, .. } => {}
            }
        }
        self.helper_step_with_method(realm, source, method)
    }

    fn helper_callback(
        &self,
        realm: ContextId,
        callback: &RawValue,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let callback = match self.iterator_callable_value(realm, self.root_raw_value(callback)?)? {
            NativeConversion::Value(callback) => callback,
            NativeConversion::Throw(_) => {
                return Err(RuntimeError::Invariant(
                    "Iterator Helper callback lost its callable brand",
                ));
            }
        };
        self.call_internal(realm, &callback, Value::Undefined, arguments)
    }

    fn resume_iterator_filter(
        &self,
        realm: ContextId,
        helper: &ObjectRef,
        source: &ObjectRef,
        state: &IteratorHelperData,
        mode: IteratorResumeKind,
    ) -> Result<HelperStep, RuntimeError> {
        let method = match self.helper_method(realm, source, &state.next, mode)? {
            Ok(method) => method,
            Err(value) => {
                return Ok(HelperStep::Throw {
                    value,
                    close_outer: true,
                });
            }
        };
        let mut index = state.count;
        loop {
            let step = self.helper_step_with_method(realm, source, method.clone())?;
            let HelperStep::Result { value, done: false } = step else {
                return Ok(step);
            };
            if mode == IteratorResumeKind::Return {
                return Ok(HelperStep::Result { value, done: false });
            }
            self.set_helper_count(helper, index.wrapping_add(1))?;
            let selected = match self.helper_callback(
                realm,
                &state.callback,
                &[value.clone(), Value::number(index as f64)],
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => {
                    return Ok(HelperStep::Throw {
                        value,
                        close_outer: true,
                    });
                }
            };
            index = index.wrapping_add(1);
            if selected.to_boolean() {
                return Ok(HelperStep::Result { value, done: false });
            }
        }
    }

    fn resume_iterator_map(
        &self,
        realm: ContextId,
        helper: &ObjectRef,
        source: &ObjectRef,
        state: &IteratorHelperData,
        mode: IteratorResumeKind,
    ) -> Result<HelperStep, RuntimeError> {
        let method = match self.helper_method(realm, source, &state.next, mode)? {
            Ok(method) => method,
            Err(value) => {
                return Ok(HelperStep::Throw {
                    value,
                    close_outer: true,
                });
            }
        };
        let step = self.helper_step_with_method(realm, source, method)?;
        let HelperStep::Result { value, done: false } = step else {
            return Ok(step);
        };
        if mode == IteratorResumeKind::Return {
            return Ok(HelperStep::Result { value, done: false });
        }
        self.set_helper_count(helper, state.count.wrapping_add(1))?;
        match self.helper_callback(
            realm,
            &state.callback,
            &[value, Value::number(state.count as f64)],
        )? {
            Completion::Return(value) => Ok(HelperStep::Result { value, done: false }),
            Completion::Throw(value) => Ok(HelperStep::Throw {
                value,
                close_outer: true,
            }),
        }
    }

    fn resume_iterator_take(
        &self,
        realm: ContextId,
        helper: &ObjectRef,
        source: &ObjectRef,
        state: &IteratorHelperData,
        mode: IteratorResumeKind,
    ) -> Result<HelperStep, RuntimeError> {
        if state.count > 0 {
            let method = match self.helper_method(realm, source, &state.next, mode)? {
                Ok(method) => method,
                Err(value) => {
                    return Ok(HelperStep::Throw {
                        value,
                        close_outer: true,
                    });
                }
            };
            self.set_helper_count(helper, state.count - 1)?;
            return self.helper_step_with_method(realm, source, method);
        }
        Ok(match self.iterator_close_normal(realm, source)? {
            IteratorClose::Closed => HelperStep::Result {
                value: Value::Undefined,
                done: true,
            },
            IteratorClose::Throw(value) => HelperStep::Throw {
                value,
                close_outer: false,
            },
        })
    }

    fn flat_map_inner_failure(
        &self,
        realm: ContextId,
        helper: &ObjectRef,
        inner: &ObjectRef,
        original: Value,
    ) -> Result<HelperStep, RuntimeError> {
        // QuickJS's `inner_fail` deliberately performs a normal IteratorClose
        // even though an exception is already pending. A close failure
        // therefore replaces the original inner-step failure; a successful
        // close leaves the original failure in place. The selected exception
        // is subsequently preserved while closing the outer iterator.
        let value = match self.iterator_close_normal(realm, inner)? {
            IteratorClose::Closed => original,
            IteratorClose::Throw(value) => value,
        };
        self.set_helper_inner(helper, None)?;
        Ok(HelperStep::Throw {
            value,
            close_outer: true,
        })
    }

    fn resume_iterator_flat_map(
        &self,
        realm: ContextId,
        helper: &ObjectRef,
        source: &ObjectRef,
        state: &IteratorHelperData,
        mode: IteratorResumeKind,
    ) -> Result<HelperStep, RuntimeError> {
        let mut inner = state
            .inner
            .map(|inner| ObjectRef::from_borrowed_handle(self.clone(), inner))
            .transpose()?;
        let mut index = state.count;
        loop {
            if inner.is_none() {
                let method = match self.helper_method(realm, source, &state.next, mode)? {
                    Ok(method) => method,
                    Err(value) => {
                        return Ok(HelperStep::Throw {
                            value,
                            close_outer: true,
                        });
                    }
                };
                let step = self.helper_step_with_method(realm, source, method)?;
                let HelperStep::Result { value, done: false } = step else {
                    return Ok(step);
                };
                if mode == IteratorResumeKind::Return {
                    return Ok(HelperStep::Result { value, done: false });
                }
                self.set_helper_count(helper, index.wrapping_add(1))?;
                let mapped = match self.helper_callback(
                    realm,
                    &state.callback,
                    &[value, Value::number(index as f64)],
                )? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => {
                        return Ok(HelperStep::Throw {
                            value,
                            close_outer: true,
                        });
                    }
                };
                index = index.wrapping_add(1);
                let Value::Object(mapped_object) = mapped.clone() else {
                    return Ok(HelperStep::Throw {
                        value: self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "not an object",
                        )?,
                        close_outer: true,
                    });
                };
                let iterator_key =
                    PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
                let method =
                    match self.get_property_in_realm(realm, &mapped_object, &iterator_key)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => {
                            return Ok(HelperStep::Throw {
                                value,
                                close_outer: true,
                            });
                        }
                    };
                let mapped_iterator = if matches!(method, Value::Undefined | Value::Null) {
                    mapped_object
                } else {
                    let method = match self.iterator_callable_value(realm, method)? {
                        NativeConversion::Value(method) => method,
                        NativeConversion::Throw(value) => {
                            return Ok(HelperStep::Throw {
                                value,
                                close_outer: true,
                            });
                        }
                    };
                    match self.call_internal(realm, &method, mapped, &[])? {
                        Completion::Return(Value::Object(iterator)) => iterator,
                        Completion::Return(_) => {
                            return Ok(HelperStep::Throw {
                                value: self.new_native_error(
                                    realm,
                                    NativeErrorKind::Type,
                                    "not an object",
                                )?,
                                close_outer: true,
                            });
                        }
                        Completion::Throw(value) => {
                            return Ok(HelperStep::Throw {
                                value,
                                close_outer: true,
                            });
                        }
                    }
                };
                self.set_helper_inner(helper, Some(&mapped_iterator))?;
                inner = Some(mapped_iterator);
            }

            let inner_iterator = inner
                .as_ref()
                .expect("flatMap loop materialized an inner iterator");
            let key_name = match mode {
                IteratorResumeKind::Next => "next",
                IteratorResumeKind::Return => "return",
            };
            let key = self.intern_property_key(key_name)?;
            let method = match self.get_property_in_realm(realm, inner_iterator, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => {
                    return self.flat_map_inner_failure(realm, helper, inner_iterator, value);
                }
            };
            if mode == IteratorResumeKind::Return
                && matches!(method, Value::Undefined | Value::Null)
            {
                let _ = self.iterator_close_normal(realm, inner_iterator)?;
                self.set_helper_inner(helper, None)?;
                inner = None;
                continue;
            }
            let step = self.helper_step_with_method(realm, inner_iterator, method)?;
            match step {
                HelperStep::Result { done: false, value } => {
                    return Ok(HelperStep::Result { value, done: false });
                }
                HelperStep::Result { done: true, .. } => {
                    let _ = self.iterator_close_normal(realm, inner_iterator)?;
                    self.set_helper_inner(helper, None)?;
                    inner = None;
                }
                HelperStep::Throw { value, .. } => {
                    return self.flat_map_inner_failure(realm, helper, inner_iterator, value);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_finite_helper_limits_use_quickjs_low_sixty_four_bits() {
        assert_eq!(quickjs_to_int64_free(2_f64.powi(63)), i64::MIN);
        assert_eq!(quickjs_to_int64_free(-2_f64.powi(63)), i64::MIN);
        assert_eq!(
            quickjs_to_int64_free(100_000_000_000_000_000_000_f64),
            7_766_279_631_452_241_920
        );
        assert_eq!(
            quickjs_to_int64_free(-100_000_000_000_000_000_000_f64),
            -7_766_279_631_452_241_920
        );
        assert_eq!(quickjs_to_int64_free(1e100), 0);
        assert_eq!(quickjs_to_int64_free(-1e100), 0);
        assert_eq!(quickjs_to_int64_free(f64::MAX), 0);

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"
(function () {
    var rangeError = false;
    try {
        [1].values().take(2 ** 63);
    } catch (error) {
        rangeError = error instanceof RangeError;
    }
    var dropped = [7].values().drop(1e100).next();
    var taken = [7].values().take(1e100).next();
    return rangeError &&
        dropped.value === 7 && dropped.done === false &&
        taken.value === undefined && taken.done === true;
})()
"#,
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn flat_map_inner_normal_close_error_replaces_the_step_error() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"
(function () {
    var firstLog = [];
    var firstOuter = {
        next: function () { return { done: false, value: 0 }; },
        return: function () {
            firstLog.push("outer-return");
            return {};
        },
    };
    var firstInner = {
        next: function () {
            firstLog.push("inner-next");
            throw "inner-next";
        },
        return: function () {
            firstLog.push("inner-return");
            throw "inner-return";
        },
    };
    var firstHelper = Iterator.prototype.flatMap.call(
        firstOuter,
        function () { return firstInner; }
    );
    var firstError;
    try {
        firstHelper.next();
    } catch (error) {
        firstError = error;
    }

    var secondLog = [];
    var returnGets = 0;
    var secondOuter = {
        next: function () { return { done: false, value: 0 }; },
        return: function () {
            secondLog.push("outer-return");
            return {};
        },
    };
    var secondInner = {
        next: function () { return { done: false, value: 1 }; },
        get return() {
            returnGets++;
            secondLog.push("get-return-" + returnGets);
            throw returnGets === 1 ? "first-return-get" : "second-return-get";
        },
    };
    var secondHelper = Iterator.prototype.flatMap.call(
        secondOuter,
        function () { return secondInner; }
    );
    secondHelper.next();
    var secondError;
    try {
        secondHelper.return();
    } catch (error) {
        secondError = error;
    }

    return firstError === "inner-return" &&
        firstLog.join(",") === "inner-next,inner-return,outer-return" &&
        secondError === "second-return-get" &&
        secondLog.join(",") === "get-return-1,get-return-2,outer-return";
})()
"#,
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn every_realms_native_iterator_constructor_is_abstract_new_target() {
        let runtime = Runtime::new();
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let foreign_iterator = second.eval("Iterator").unwrap();
        let foreign_bound_iterator = second.eval("Iterator.bind(null)").unwrap();
        let foreign_iterator_constructor_accessor = second
            .eval("Object.getOwnPropertyDescriptor(Iterator.prototype, 'constructor').get")
            .unwrap();
        let foreign_type_error = second.eval("TypeError").unwrap();
        let global = first.global_object().unwrap();

        for (name, value) in [
            ("foreignIterator", foreign_iterator),
            ("foreignBoundIterator", foreign_bound_iterator),
            (
                "foreignIteratorConstructorAccessor",
                foreign_iterator_constructor_accessor,
            ),
            ("foreignTypeError", foreign_type_error),
        ] {
            let key = runtime.intern_property_key(name).unwrap();
            assert!(
                first
                    .define_own_property(
                        &global,
                        &key,
                        &OrdinaryPropertyDescriptor {
                            value: DescriptorField::Present(value),
                            writable: DescriptorField::Present(true),
                            enumerable: DescriptorField::Present(false),
                            configurable: DescriptorField::Present(true),
                            ..OrdinaryPropertyDescriptor::new()
                        },
                    )
                    .unwrap()
            );
        }

        assert_eq!(
            first
                .eval(
                    r#"
(function () {
    var threw = false;
    try {
        Reflect.construct(Iterator, [], foreignIterator);
    } catch (error) {
        threw = error instanceof TypeError;
    }
    var constructed = Reflect.construct(Iterator, [], foreignBoundIterator);
    var getterResult = foreignIteratorConstructorAccessor();
    var setterError;
    try {
        foreignIteratorConstructorAccessor({}, 1);
    } catch (error) {
        setterError = error;
    }
    return threw && typeof constructed === "object" &&
        getterResult === foreignIterator &&
        setterError instanceof TypeError &&
        !(setterError instanceof foreignTypeError);
})()
"#,
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn iterator_hidden_edges_survive_gc_and_unrooted_cycles_collect() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"
(function () {
    globalThis.helper = [1, 2].values().map(function (value) {
        return value + 40;
    });
    var source = {
        index: 0,
        next: function () {
            this.index++;
            return { done: false, value: this.index + 6 };
        },
    };
    globalThis.wrapper = Iterator.from(source);
    delete source.next;
})()
"#,
                )
                .unwrap(),
            Value::Undefined
        );
        runtime.run_gc().unwrap();
        assert_eq!(
            context
                .eval(
                    r#"
(function () {
    var valid = helper.next().value === 41 &&
        wrapper.next().value === 7;
    delete globalThis.helper;
    delete globalThis.wrapper;
    return valid;
})()
"#,
                )
                .unwrap(),
            Value::Bool(true)
        );
        runtime.run_gc().unwrap();

        let Value::Object(cycle) = context
            .eval(
                r#"
(function () {
    var helper;
    var source = {
        next: function () {
            return { done: true, value: undefined };
        },
    };
    var callback = function (value) {
        source.helper;
        return value;
    };
    helper = Iterator.prototype.map.call(source, callback);
    source.helper = helper;
    return helper;
})()
"#,
            )
            .unwrap()
        else {
            panic!("Iterator helper cycle did not produce an object");
        };
        let cycle_id = cycle.object_id();
        drop(cycle);
        assert!(runtime.run_gc().unwrap().cleanup.finalized_objects >= 3);
        assert!(
            runtime.0.state.borrow().heap.object(cycle_id).is_err(),
            "unrooted Iterator helper/callback/source cycle survived collection"
        );
    }
}
