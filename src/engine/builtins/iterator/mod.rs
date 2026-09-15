//! `%Iterator%`, `Iterator.from`, `Iterator.concat`, and synchronous Iterator
//! Helpers.
//!
//! The implementation follows the pinned QuickJS iterator slice rather than
//! treating the proposal methods as Array conveniences.  In particular,
//! iterator records cache `next`, eager consumers preserve QuickJS's close
//! precedence, and lazy helpers keep their re-entrancy and completion state in
//! traced heap payloads.

use super::object::ObjectIteratorStep;
use super::quickjs_to_int64_free;
use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::builtins::native::NativeFunctionId;

use crate::engine::heap::{
    ContextId, IteratorConsumerKind, IteratorHelperData, IteratorHelperKind, IteratorRealmData,
    IteratorResumeKind, ObjectData,
};
use crate::engine::object::shape::PropertyFlags;
use crate::engine::object::{
    AccessorValue, CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor,
    PropertyKey, WellKnownSymbol,
};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation};

pub(super) mod array;
pub(crate) mod collection;
pub(super) mod concat;
pub(crate) mod constructor;
pub(super) mod consume;
pub(super) mod create;
pub(crate) mod entry;
pub(super) mod from;
pub(super) mod helper;
pub(super) mod step;
pub(super) mod wrap;

impl Runtime {
    pub(crate) fn initialize_iterator_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        iterator_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let concat_prototype = self.new_object(Some(iterator_prototype))?;
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
            NativeFunctionId::IteratorConcat,
            "concat",
            0,
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

        self.define_native_builtin_auto_init(
            &concat_prototype,
            realm,
            NativeFunctionId::IteratorConcatNext,
            "next",
            0,
            0,
        )?;
        self.define_native_builtin_auto_init(
            &concat_prototype,
            realm,
            NativeFunctionId::IteratorConcatReturn,
            "return",
            0,
            0,
        )?;
        self.define_iterator_data_tag(&concat_prototype, "Iterator Concat")?;

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
                concat_prototype: concat_prototype.object_id(),
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

    pub(crate) fn call_iterator_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        constructor::finish(
            self,
            realm,
            constructor::ConstructorStep::start(self, realm, &invocation)?,
        )
    }

    pub(crate) fn call_iterator_constructor_accessor(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        constructor::finish(
            self,
            realm,
            constructor::ConstructorStep::accessor(self, realm, callable, &invocation, arguments)?,
        )
    }

    pub(crate) fn call_iterator_from(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        from::finish(
            self,
            realm,
            from::FromStep::start(self, realm, &invocation, arguments)?,
        )
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

    pub(crate) fn call_iterator_wrap_resume(
        &self,
        realm: ContextId,
        kind: IteratorResumeKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        wrap::finish(
            self,
            realm,
            wrap::WrapStep::start(self, realm, kind, &invocation)?,
        )
    }

    pub(crate) fn call_iterator_create_helper(
        &self,
        realm: ContextId,
        kind: IteratorHelperKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        create::finish(
            self,
            realm,
            create::CreateStep::start(self, realm, kind, &invocation, arguments)?,
        )
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

    pub(crate) fn call_iterator_consumer(
        &self,
        realm: ContextId,
        kind: IteratorConsumerKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        consume::finish(
            self,
            realm,
            consume::ConsumeStep::start(
                self,
                realm,
                consume::ConsumeKind::Predicate(kind),
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_iterator_reduce(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        consume::finish(
            self,
            realm,
            consume::ConsumeStep::start(
                self,
                realm,
                consume::ConsumeKind::Reduce,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_iterator_to_array(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        consume::finish(
            self,
            realm,
            consume::ConsumeStep::start(
                self,
                realm,
                consume::ConsumeKind::Array,
                &invocation,
                &NativeArguments {
                    actual_arg_count: 0,
                    readable: Vec::new(),
                },
            )?,
        )
    }

    pub(crate) fn call_iterator_helper_resume(
        &self,
        realm: ContextId,
        mode: IteratorResumeKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        helper::finish(
            self,
            realm,
            helper::HelperResumeStep::start(self, realm, mode, &invocation)?,
        )
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
