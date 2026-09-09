//! Allocate a realm and initialize its intrinsic roots in publication order.

use crate::engine::api::context::Context;
use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{NativeFunctionId, PrimitiveKind};
use crate::engine::heap::ContextData;
use crate::engine::object::{DescriptorField, ObjectRef, OrdinaryPropertyDescriptor};
use crate::engine::value::{JsString, Value};

impl Runtime {
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
                    .with_public_id(id)
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
        // QuickJS installs `%Iterator%` after Function and before the global
        // function-list prefix. This is observable in global own-key order.
        self.initialize_iterator_intrinsic(
            realm,
            &function_prototype,
            &iterator_prototype,
            &global_object,
        )
        .expect("Iterator intrinsic initialization must succeed");
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
        self.initialize_string_normalize_intrinsic(realm, &string_prototype)
            .expect("String Unicode intrinsic initialization must succeed");
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
        self.initialize_generator_intrinsic(realm, &function_prototype, &iterator_prototype)
            .expect("Generator intrinsic initialization must succeed");
        self.initialize_async_function_intrinsic(realm, &function_prototype)
            .expect("AsyncFunction intrinsic initialization must succeed");
        // Upstream installs `globalThis` after String/Math/Reflect/Symbol and
        // generator setup, then installs BigInt. The remaining intervening
        // intrinsics are absent here, but this boundary preserves the
        // implemented `Boolean, String, Math, Reflect, Symbol, globalThis,
        // BigInt, Date, RegExp, JSON` relative order.
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
        self.initialize_json_intrinsic(realm, &global_object)
            .expect("JSON intrinsic initialization must succeed");
        self.initialize_proxy_intrinsic(realm, &function_prototype, &global_object)
            .expect("Proxy intrinsic initialization must succeed");
        // Pinned QuickJS installs strong/weak collection intrinsics after
        // Proxy.
        self.initialize_map_intrinsic(
            realm,
            &function_prototype,
            &object_prototype,
            &iterator_prototype,
            &global_object,
        )
        .expect("Map intrinsic initialization must succeed");
        self.initialize_set_intrinsic(
            realm,
            &function_prototype,
            &object_prototype,
            &iterator_prototype,
            &global_object,
        )
        .expect("Set intrinsic initialization must succeed");
        self.initialize_weak_collection_intrinsics(
            realm,
            &function_prototype,
            &object_prototype,
            &global_object,
        )
        .expect("WeakMap/WeakSet intrinsic initialization must succeed");
        self.initialize_array_buffer_intrinsic(
            realm,
            &function_prototype,
            &object_prototype,
            &global_object,
        )
        .expect("ArrayBuffer intrinsic initialization must succeed");
        self.initialize_shared_array_buffer_intrinsic(
            realm,
            &function_prototype,
            &object_prototype,
            &global_object,
        )
        .expect("SharedArrayBuffer intrinsic initialization must succeed");
        self.initialize_typed_array_intrinsics(
            realm,
            &function_prototype,
            &object_prototype,
            &global_object,
        )
        .expect("TypedArray intrinsic initialization must succeed");
        self.initialize_data_view_intrinsic(
            realm,
            &function_prototype,
            &object_prototype,
            &global_object,
        )
        .expect("DataView intrinsic initialization must succeed");
        self.initialize_atomics_intrinsic(realm, &global_object)
            .expect("Atomics intrinsic initialization must succeed");
        self.initialize_promise_intrinsic(
            realm,
            &function_prototype,
            &object_prototype,
            &global_object,
        )
        .expect("Promise intrinsic initialization must succeed");
        self.initialize_async_generator_intrinsic(realm, &function_prototype, &object_prototype)
            .expect("AsyncGenerator intrinsic initialization must succeed");
        self.initialize_weak_ref_intrinsics(
            realm,
            &function_prototype,
            &object_prototype,
            &global_object,
        )
        .expect("WeakRef intrinsic initialization must succeed");
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
}
