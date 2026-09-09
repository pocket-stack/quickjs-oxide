//! Install primitive and function intrinsics and their initial property relationships.

mod construction;

use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{
    BigIntAsNKind, DynamicFunctionKind, FunctionDebugPosition, GlobalNumberPredicateKind,
    GlobalUriCodecKind, NativeFunctionId, NumberFormatKind, NumberParseKind, NumberPredicateKind,
    PrimitiveKind, SymbolRegistryKind,
};
use crate::engine::heap::ContextId;
use crate::engine::object::shape::PropertyFlags;

use crate::engine::object::{
    AccessorValue, CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor,
    PropertyKey, WellKnownSymbol,
};
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;

impl Runtime {
    pub(crate) fn initialize_function_constructor(
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

    pub(crate) fn initialize_number_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        number_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let kind = PrimitiveKind::Number;
        for (target, name, arity) in [
            (
                NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::Exponential),
                "toExponential",
                1,
            ),
            (
                NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::Fixed),
                "toFixed",
                1,
            ),
            (
                NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::Precision),
                "toPrecision",
                1,
            ),
            (
                NativeFunctionId::PrimitivePrototypeToString(kind),
                "toString",
                1,
            ),
            (
                NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::LocaleString),
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

    pub(crate) fn initialize_boolean_intrinsic(
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
    pub(crate) fn initialize_string_conversion_core(
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

    /// Complete the String iterator class slice: the generic String method,
    /// branded iterator prototype, native-next ABI and configurable tag.
    pub(crate) fn initialize_string_iterator_intrinsics(
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

    pub(crate) fn initialize_symbol_intrinsic(
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

    pub(crate) fn initialize_bigint_intrinsic(
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

    pub(crate) fn initialize_global_functions_prefix(
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

    pub(crate) fn initialize_global_primitive_constants(
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

    pub(crate) fn initialize_global_to_string_tag(
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

    pub(crate) fn initialize_global_this(
        &self,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
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

    pub(crate) fn initialize_function_restricted_properties(
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

    pub(crate) fn initialize_function_prototype_methods(
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

    pub(crate) fn define_native_builtin_getter(
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

    pub(crate) fn define_native_builtin_getter_on(
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

    pub(crate) fn define_constructor_relationship(
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
}

pub(crate) mod bindings;

pub(crate) mod prototypes;
