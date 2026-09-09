use crate::engine::api::error::{Error, ErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::builtins::native::NativeFunctionId;

use crate::engine::code::function::metadata::{ConstructorKind, FunctionKind, FunctionMetadata};
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::heap::roots::VarRefRoot;

use crate::engine::heap::{AutoInitProperty, ContextId, ObjectData, PropertySlot};
use crate::engine::object::shape::{PropertyFlags, ShapeEntry};
use crate::engine::object::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, DescriptorField, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey,
};
use crate::engine::value::{JsString, Value};

impl Runtime {
    pub(crate) fn new_bytecode_closure_with_slots(
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
        self.ensure_dynamic_import_bytecode_tree_authorized(function)?;

        let mut state = self.0.state.borrow_mut();
        let (metadata, func_name) = {
            let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
            (bytecode.metadata, bytecode.func_name.clone())
        };
        let context = state.heap.context(caller_realm)?;
        // QuickJS stores a module's async bytecode in an ordinary hidden
        // bytecode-function object.  The evaluator enters the async driver
        // explicitly, while the link-only `this = true` call uses the same
        // object as a synchronous declaration-instantiation entry point.
        // Preserve that object shape instead of exposing an AsyncFunction
        // prototype merely because every module root has async bytecode.
        let function_prototype = if metadata.is_module {
            context.function_prototype
        } else {
            match metadata.function_kind {
                FunctionKind::Normal => context.function_prototype,
                FunctionKind::Generator => {
                    context
                        .generator
                        .ok_or(RuntimeError::Invariant(
                            "generator closure realm has no Generator intrinsics",
                        ))?
                        .function_prototype
                }
                FunctionKind::Async => {
                    context
                        .async_function
                        .ok_or(RuntimeError::Invariant(
                            "async closure realm has no AsyncFunction intrinsics",
                        ))?
                        .function_prototype
                }
                FunctionKind::AsyncGenerator => {
                    context
                        .async_generator
                        .ok_or(RuntimeError::Invariant(
                            "async-generator closure realm has no AsyncGenerator intrinsics",
                        ))?
                        .function_prototype
                }
            }
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

    pub(crate) fn initialize_bytecode_function_properties(
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
        if matches!(
            metadata.function_kind,
            FunctionKind::Generator | FunctionKind::AsyncGenerator
        ) {
            if !metadata.has_prototype || metadata.constructor_kind != ConstructorKind::None {
                return Err(RuntimeError::Invariant(
                    "generator-family bytecode has invalid prototype/constructor metadata",
                ));
            }
            return self.define_generator_function_prototype(
                callable.as_object(),
                realm,
                metadata.function_kind,
            );
        }
        if !metadata.has_prototype {
            return Ok(());
        }
        self.define_function_auto_init_prototype(callable.as_object(), realm)
    }

    pub(crate) fn define_generator_function_prototype(
        &self,
        function: &ObjectRef,
        realm: ContextId,
        function_kind: FunctionKind,
    ) -> Result<(), RuntimeError> {
        let generator_prototype = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            match function_kind {
                FunctionKind::Generator => {
                    context
                        .generator
                        .ok_or(RuntimeError::Invariant(
                            "generator function realm has no Generator intrinsics",
                        ))?
                        .prototype
                }
                FunctionKind::AsyncGenerator => {
                    context
                        .async_generator
                        .ok_or(RuntimeError::Invariant(
                            "async-generator function realm has no AsyncGenerator intrinsics",
                        ))?
                        .prototype
                }
                FunctionKind::Normal | FunctionKind::Async => {
                    return Err(RuntimeError::Invariant(
                        "ordinary function requested a generator prototype",
                    ));
                }
            }
        };
        let generator_prototype =
            ObjectRef::from_borrowed_handle(self.clone(), generator_prototype)?;
        let prototype = self.new_object(Some(&generator_prototype))?;
        self.define_function_data_property(
            function,
            "prototype",
            Value::Object(prototype),
            true,
            false,
        )
    }

    pub(crate) fn define_function_auto_init_prototype(
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

    pub(crate) fn define_native_builtin_auto_init(
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
    pub(crate) fn define_native_builtin_auto_init_with_key(
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

    pub(crate) fn define_string_auto_init(
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
    pub(crate) fn define_failure_auto_init(
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

    pub(crate) fn define_function_data_property(
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
    pub(crate) fn define_object_name(
        &self,
        value: &Value,
        name: &JsString,
    ) -> Result<(), RuntimeError> {
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
}
