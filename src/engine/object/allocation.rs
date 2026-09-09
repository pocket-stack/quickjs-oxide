use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::Atom;

use crate::engine::builtins::native::{NativeFunctionId, PrimitiveKind};
use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariableKind, ClosureVariableName,
};
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::heap::roots::VarRefRoot;

use crate::engine::heap::{
    ContextId, ObjectData, ObjectPayload, PrimitiveObjectData, PropertySlot, RawValue, ShapeId,
};
use crate::engine::object::shape::{PropertyFlags, ShapeEntry};
use crate::engine::object::{
    CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
};
use crate::engine::realm::bindings::GlobalBindingCreationMode;
use crate::engine::value::{JsString, Value};
use std::collections::HashMap;

impl Runtime {
    /// Allocate an ordinary object whose prototype is `prototype` or null.
    pub fn new_object(&self, prototype: Option<&ObjectRef>) -> Result<ObjectRef, RuntimeError> {
        self.new_empty_object_with(prototype, ObjectData::ordinary)
    }

    pub(crate) fn new_iterator_object(
        &self,
        prototype: &ObjectRef,
    ) -> Result<ObjectRef, RuntimeError> {
        self.new_empty_object_with(Some(prototype), ObjectData::iterator)
    }

    pub(crate) fn new_empty_object_with(
        &self,
        prototype: Option<&ObjectRef>,
        build: fn(ShapeId, Vec<PropertySlot>) -> ObjectData,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if prototype.is_some_and(|prototype| !prototype.belongs_to(self)) {
            return Err(RuntimeError::WrongRuntime("prototype"));
        }
        let prototype = prototype.map(ObjectRef::object_id);

        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(prototype, &[])?;
        let object = match state.heap.allocate_object(build(shape, Vec::new())) {
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
    pub(crate) fn new_empty_array_with_prototype(
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

    /// Append one element to an Array that has not observed any mutation
    /// outside its consecutive construction path. This is the common
    /// QuickJS `add_fast_array_element` substrate used by VM literals,
    /// builtin result arrays, and JSON parsing; no decimal property atom is
    /// created for the dense index.
    pub(crate) fn append_fresh_array_value(
        &self,
        array: &ObjectRef,
        value: Value,
    ) -> Result<(), RuntimeError> {
        if !array.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Array"));
        }
        self.validate_value_domain(&value, "Array element")?;
        let raw = self.raw_property_value(&value)?;
        let mut state = self.0.state.borrow_mut();
        let retained_atoms = state.retain_raw_value_atoms(std::iter::once(&raw))?;
        match state
            .heap
            .append_fresh_array_dense_value(array.object_id(), raw)
        {
            Ok(()) => Ok(()),
            Err(error) => {
                state.release_atoms(retained_atoms)?;
                Err(error.into())
            }
        }
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
        for value in values {
            self.append_fresh_array_value(&array, value)?;
        }
        Ok(array)
    }

    pub(crate) fn new_string_iterator(
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

    pub(crate) fn new_iterator_result(
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

    pub(crate) fn new_primitive_object(
        &self,
        prototype: &ObjectRef,
        kind: PrimitiveKind,
        value: Value,
    ) -> Result<ObjectRef, RuntimeError> {
        self.new_primitive_object_with_string_length(prototype, kind, value, false)
    }

    pub(crate) fn new_string_object(
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

    pub(crate) fn new_primitive_object_with_string_length(
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

    pub(crate) fn new_global_object(
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

    pub(crate) fn new_native_function(
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
    pub(crate) fn new_bound_native_function(
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

    pub(crate) fn new_bound_function(
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
    pub(crate) fn new_native_builtin(
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
    pub(crate) fn set_constructor_bit(
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
                | ObjectPayload::Proxy(crate::engine::heap::ProxyData {
                    is_callable: true,
                    ..
                })
        );
        if !callable {
            return Ok(None);
        }
        Ok(Some(CallableRef::from_validated_object(object.clone())))
    }

    /// Instantiate one runtime-owned bytecode node as a callable object in the
    /// caller's realm, matching QuickJS's `js_closure` boundary.
    pub(crate) fn new_bytecode_closure(
        &self,
        caller_realm: ContextId,
        function: &FunctionBytecodeRef,
    ) -> Result<CallableRef, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        self.ensure_dynamic_import_bytecode_tree_authorized(function)?;
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
                    ClosureVariableKind::ModuleImportView
                    | ClosureVariableKind::FunctionName
                    | ClosureVariableKind::GlobalFunction
                    | ClosureVariableKind::EvalVariableObject
                    | ClosureVariableKind::ArgEvalVariableObject
                    | ClosureVariableKind::WithObject
                    | ClosureVariableKind::PrivateField
                    | ClosureVariableKind::PrivateMethod
                    | ClosureVariableKind::PrivateGetter
                    | ClosureVariableKind::PrivateSetter
                    | ClosureVariableKind::PrivateGetterSetter => {
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
                        ClosureVariableKind::ModuleImportView
                        | ClosureVariableKind::FunctionName
                        | ClosureVariableKind::GlobalFunction
                        | ClosureVariableKind::EvalVariableObject
                        | ClosureVariableKind::ArgEvalVariableObject
                        | ClosureVariableKind::WithObject
                        | ClosureVariableKind::PrivateField
                        | ClosureVariableKind::PrivateMethod
                        | ClosureVariableKind::PrivateGetter
                        | ClosureVariableKind::PrivateSetter
                        | ClosureVariableKind::PrivateGetterSetter => {
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
                ClosureSource::ModuleDeclaration
                | ClosureSource::ModuleImport
                | ClosureSource::ModuleImportCollision
                | ClosureSource::ModuleImportMeta => {
                    return Err(RuntimeError::Invariant(
                        "ordinary root publication received a module descriptor",
                    ));
                }
            };
            slots.push(root);
        }
        self.new_bytecode_closure_with_slots(caller_realm, function, &slots)
    }
}
