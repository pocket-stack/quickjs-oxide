//! Runtime substrate for class private data fields.
//!
//! The semantic anchors are QuickJS 2026-06-04 `quickjs.c` 8366-8456
//! (`JS_DefinePrivateField`, `JS_GetPrivateField`, and
//! `JS_SetPrivateField`) and 15964-15994 (`js_operator_private_in`).  In
//! particular, these operations inspect only the receiver's own shape,
//! private definition bypasses ordinary extensibility checks, and duplicate or
//! absent fields use QuickJS's native TypeError spelling.

use super::*;
use crate::heap::EvalKind;
use crate::object::PrivateNameRef;

impl Runtime {
    /// Allocate a fresh runtime-local identity for one evaluated private name.
    ///
    /// Equal descriptions deliberately produce unequal identities.  The
    /// returned handle cannot be converted to a public Symbol or Value.
    pub(in crate::runtime) fn new_private_name(
        &self,
        description: JsString,
    ) -> Result<PrivateNameRef, RuntimeError> {
        let _operation = self.operation();
        let atom = self
            .0
            .state
            .borrow_mut()
            .atoms
            .new_private_symbol_js_string(Some(description))?;
        Ok(PrivateNameRef::from_owned_atom(self.clone(), atom))
    }

    /// Define a private data field directly on `receiver`.
    ///
    /// This intentionally bypasses the object's extensibility bit, matching
    /// QuickJS's `JS_DefinePrivateField`; it still rejects duplicate identity.
    pub(in crate::runtime) fn define_private_field_own(
        &self,
        receiver: &ObjectRef,
        name: &PrivateNameRef,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        self.validate_private_receiver(receiver, name)?;
        self.validate_value_domain(&value, "private field value")?;
        let raw = self.raw_property_value(&value)?;
        let object_id = receiver.object_id();

        let duplicate = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object_id)?;
            state.heap.shape(object.shape)?.find(name.atom()).is_some()
        };
        if duplicate {
            return Err(RuntimeError::Engine(self.private_field_error(
                name,
                "private class field '",
                "' already exists",
            )?));
        }

        let mut state = self.0.state.borrow_mut();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            // Recheck under the mutable borrow so a future interior mutator
            // cannot turn the snapshot above into a duplicate transition.
            if shape.find(name.atom()).is_some() {
                drop(state);
                return Err(RuntimeError::Engine(self.private_field_error(
                    name,
                    "private class field '",
                    "' already exists",
                )?));
            }
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object.slots.clone(),
            )
        };
        entries.push(ShapeEntry {
            atom: name.atom(),
            flags: PropertyFlags::data(true, true, true),
        });
        slots.push(PropertySlot::Data(raw));
        state.replace_layout(object_id, prototype, &entries, slots)?;
        drop(state);
        // `replace_layout` retained the heap occurrence before this incoming
        // public root is released.
        drop(value);
        Ok(())
    }

    /// Read one private data field directly from `receiver`'s own shape.
    pub(in crate::runtime) fn get_private_field_own(
        &self,
        receiver: &ObjectRef,
        name: &PrivateNameRef,
    ) -> Result<Value, RuntimeError> {
        let _operation = self.operation();
        self.validate_private_receiver(receiver, name)?;
        let raw = {
            let state = self.0.state.borrow();
            let object = state.heap.object(receiver.object_id())?;
            let shape = state.heap.shape(object.shape)?;
            let Some(index) = shape.find(name.atom()) else {
                drop(state);
                return Err(RuntimeError::Engine(self.private_field_error(
                    name,
                    "private class field '",
                    "' does not exist",
                )?));
            };
            let index = usize::try_from(index)
                .map_err(|_| RuntimeError::Invariant("private field index does not fit usize"))?;
            match object.slots.get(index) {
                Some(PropertySlot::Data(value)) => value.clone(),
                Some(
                    PropertySlot::VarRef(_)
                    | PropertySlot::Accessor { .. }
                    | PropertySlot::AutoInit(_),
                ) => {
                    return Err(RuntimeError::Invariant(
                        "private data field used non-data storage",
                    ));
                }
                None => {
                    return Err(RuntimeError::Invariant(
                        "private field shape entry has no parallel slot",
                    ));
                }
            }
        };
        self.root_raw_value(&raw)
    }

    /// Replace one existing private data field directly on `receiver`.
    pub(in crate::runtime) fn set_private_field_own(
        &self,
        receiver: &ObjectRef,
        name: &PrivateNameRef,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        self.validate_private_receiver(receiver, name)?;
        self.validate_value_domain(&value, "private field value")?;
        let raw = self.raw_property_value(&value)?;
        let object_id = receiver.object_id();
        let index = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            let Some(index) = shape.find(name.atom()) else {
                drop(state);
                return Err(RuntimeError::Engine(self.private_field_error(
                    name,
                    "private class field '",
                    "' does not exist",
                )?));
            };
            let index = usize::try_from(index)
                .map_err(|_| RuntimeError::Invariant("private field index does not fit usize"))?;
            if !matches!(object.slots.get(index), Some(PropertySlot::Data(_))) {
                return Err(RuntimeError::Invariant(
                    "private data field used non-data storage",
                ));
            }
            index
        };

        let replacement = PropertySlot::Data(raw);
        let mut state = self.0.state.borrow_mut();
        let retained_atoms = state.retain_slot_atoms(std::slice::from_ref(&replacement))?;
        let cleanup = match state
            .heap
            .replace_object_slot(object_id, index, replacement)
        {
            Ok(cleanup) => cleanup,
            Err(error) => {
                state.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
        };
        state.apply_cleanup(cleanup)?;
        drop(state);
        // `replace_object_slot` retained the heap occurrence before this
        // incoming public root is released.
        drop(value);
        Ok(())
    }

    /// Test for one private data field on `receiver` without walking its
    /// prototype chain.
    pub(in crate::runtime) fn has_private_field_own(
        &self,
        receiver: &ObjectRef,
        name: &PrivateNameRef,
    ) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        self.validate_private_receiver(receiver, name)?;
        let state = self.0.state.borrow();
        let object = state.heap.object(receiver.object_id())?;
        Ok(state.heap.shape(object.shape)?.find(name.atom()).is_some())
    }

    /// Capture a private-name identity in its dedicated immutable lexical
    /// VarRef representation.
    pub(in crate::runtime) fn new_private_var_ref(
        &self,
        name: &PrivateNameRef,
    ) -> Result<VarRefRoot, RuntimeError> {
        let _operation = self.operation();
        self.validate_private_name(name)?;
        let atom = name.atom();
        let mut state = self.0.state.borrow_mut();
        state.atoms.retain(atom)?;
        let data = VarRefData::captured(
            RawValue::Private(atom),
            true,
            true,
            ClosureVariableKind::PrivateField,
        );
        let id = match state.heap.allocate_var_ref(data) {
            Ok(id) => id,
            Err(error) => {
                state.atoms.release(atom)?;
                return Err(error.into());
            }
        };
        drop(state);
        Ok(VarRefRoot::from_owned_handle(self.clone(), id))
    }

    /// Initialize an uninitialized private-name VarRef without routing its
    /// identity through ordinary value writes.
    pub(in crate::runtime) fn initialize_private_var_ref(
        &self,
        root: &VarRefRoot,
        name: &PrivateNameRef,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("private-name closure variable"));
        }
        self.validate_private_name(name)?;
        let atom = name.atom();
        let mut state = self.0.state.borrow_mut();
        {
            let var_ref = state.heap.var_ref(root.id())?;
            if var_ref.kind != ClosureVariableKind::PrivateField
                || !var_ref.is_lexical
                || !var_ref.is_const
            {
                return Err(RuntimeError::Invariant(
                    "private-name initialization reached an ordinary VarRef",
                ));
            }
            if !matches!(&var_ref.value, RawValue::Uninitialized) {
                return Err(RuntimeError::Invariant(
                    "private-name VarRef was initialized more than once",
                ));
            }
        }
        state.atoms.retain(atom)?;
        let cleanup = match state
            .heap
            .replace_var_ref_value(root.id(), RawValue::Private(atom))
        {
            Ok(cleanup) => cleanup,
            Err(error) => {
                state.atoms.release(atom)?;
                return Err(error.into());
            }
        };
        state.apply_cleanup(cleanup)
    }

    /// Root a private name held by an authenticated captured cell.
    ///
    /// Ordinary VarRef reads remain fail-closed: only this typed path may turn
    /// `RawValue::Private` into a `PrivateNameRef`.
    pub(in crate::runtime) fn private_name_from_raw_var_ref(
        &self,
        root: &VarRefRoot,
    ) -> Result<PrivateNameRef, RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("private-name closure variable"));
        }
        let atom = {
            let state = self.0.state.borrow();
            let var_ref = state.heap.var_ref(root.id())?;
            if var_ref.kind != ClosureVariableKind::PrivateField
                || !var_ref.is_lexical
                || !var_ref.is_const
            {
                return Err(RuntimeError::Invariant(
                    "private-name read reached an ordinary VarRef",
                ));
            }
            match &var_ref.value {
                RawValue::Private(atom) => {
                    if state.atoms.kind(*atom)? != AtomKind::Private {
                        return Err(RuntimeError::Invariant(
                            "private-name VarRef contains a non-private atom",
                        ));
                    }
                    *atom
                }
                RawValue::Uninitialized => {
                    return Err(RuntimeError::Invariant(
                        "private-name VarRef was read before initialization",
                    ));
                }
                _ => {
                    return Err(RuntimeError::Invariant(
                        "private-name VarRef contains an ordinary value",
                    ));
                }
            }
        };
        PrivateNameRef::from_borrowed_atom(self.clone(), atom).map_err(Into::into)
    }

    /// Capture one class-private method closure in its dedicated immutable
    /// lexical cell. The method's HomeObject must already be installed: it is
    /// the authority from which QuickJS derives the class-side brand.
    pub(in crate::runtime) fn new_private_method_var_ref(
        &self,
        method: &CallableRef,
    ) -> Result<VarRefRoot, RuntimeError> {
        let _operation = self.operation();
        let (method_id, home_object) = self.private_method_callable_parts(method)?;
        if home_object.is_none() {
            return Err(RuntimeError::Invariant(
                "private-method callable has no HomeObject",
            ));
        }
        let id = self
            .0
            .state
            .borrow_mut()
            .heap
            .allocate_var_ref(VarRefData::captured(
                RawValue::Object(method_id),
                true,
                true,
                ClosureVariableKind::PrivateMethod,
            ))?;
        Ok(VarRefRoot::from_owned_handle(self.clone(), id))
    }

    /// Initialize an uninitialized captured private-method cell exactly once.
    /// This bypasses the ordinary VarRef write path so the callable capability
    /// can never escape as a mutable source-visible lexical value.
    pub(in crate::runtime) fn initialize_private_method_var_ref(
        &self,
        root: &VarRefRoot,
        method: &CallableRef,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime(
                "private-method closure variable",
            ));
        }
        let (method_id, home_object) = self.private_method_callable_parts(method)?;
        if home_object.is_none() {
            return Err(RuntimeError::Invariant(
                "private-method callable has no HomeObject",
            ));
        }
        let mut state = self.0.state.borrow_mut();
        {
            let var_ref = state.heap.var_ref(root.id())?;
            if var_ref.kind != ClosureVariableKind::PrivateMethod
                || !var_ref.is_lexical
                || !var_ref.is_const
            {
                return Err(RuntimeError::Invariant(
                    "private-method initialization reached an ordinary VarRef",
                ));
            }
            if !matches!(var_ref.value, RawValue::Uninitialized) {
                return Err(RuntimeError::Invariant(
                    "private-method VarRef was initialized more than once",
                ));
            }
        }
        let cleanup = state
            .heap
            .replace_var_ref_value(root.id(), RawValue::Object(method_id))?;
        state.apply_cleanup(cleanup)
    }

    /// Root the callable held by an authenticated captured private-method
    /// cell. Generic VarRef reads deliberately reject this representation.
    pub(in crate::runtime) fn private_method_from_raw_var_ref(
        &self,
        root: &VarRefRoot,
    ) -> Result<CallableRef, RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime(
                "private-method closure variable",
            ));
        }
        let method_id = {
            let state = self.0.state.borrow();
            let var_ref = state.heap.var_ref(root.id())?;
            if var_ref.kind != ClosureVariableKind::PrivateMethod
                || !var_ref.is_lexical
                || !var_ref.is_const
            {
                return Err(RuntimeError::Invariant(
                    "private-method read reached an ordinary VarRef",
                ));
            }
            match var_ref.value {
                RawValue::Object(method) => method,
                RawValue::Uninitialized => {
                    return Err(RuntimeError::Invariant(
                        "private-method VarRef was read before initialization",
                    ));
                }
                _ => {
                    return Err(RuntimeError::Invariant(
                        "private-method VarRef contains an ordinary value",
                    ));
                }
            }
        };
        let object = ObjectRef::from_borrowed_handle(self.clone(), method_id)?;
        let method = CallableRef::from_validated_object(object);
        let (_, home_object) = self.private_method_callable_parts(&method)?;
        if home_object.is_none() {
            return Err(RuntimeError::Invariant(
                "private-method callable lost its HomeObject",
            ));
        }
        Ok(method)
    }

    /// Create the one hidden private brand owned by a class prototype or
    /// constructor. Repeated method declarations reuse the same class-side
    /// identity, matching QuickJS `JS_AddBrand`.
    pub(in crate::runtime) fn ensure_private_brand_home(
        &self,
        home_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !home_object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime(
                "private-method brand HomeObject",
            ));
        }
        let mut state = self.0.state.borrow_mut();
        if let Some(brand) = state
            .heap
            .object_private_brand_home(home_object.object_id())?
        {
            if state.atoms.kind(brand)? != AtomKind::Private {
                return Err(RuntimeError::Invariant(
                    "private-method HomeObject brand is not a private atom",
                ));
            }
            return Ok(());
        }
        let brand = state
            .atoms
            .new_private_symbol_js_string(Some(JsString::try_from_utf8("<brand>")?))?;
        if let Err(error) = state
            .heap
            .attach_object_private_brand_home(home_object.object_id(), brand)
        {
            state.atoms.release(brand)?;
            return Err(error.into());
        }
        Ok(())
    }

    /// Install an own-only hidden marker for one receiver. This bypasses
    /// `[[Extensible]]`; a repeated initialization reports QuickJS's native
    /// duplicate-private-method TypeError.
    pub(in crate::runtime) fn add_private_method_brand(
        &self,
        home_object: &ObjectRef,
        receiver: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !home_object.belongs_to(self) || !receiver.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("private-method brand object"));
        }
        let brand = self.private_brand_atom(home_object)?;
        let receiver_id = receiver.object_id();
        let duplicate = {
            let state = self.0.state.borrow();
            let object = state.heap.object(receiver_id)?;
            state.heap.shape(object.shape)?.find(brand).is_some()
        };
        if duplicate {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "private method is already present",
            )));
        }

        let mut state = self.0.state.borrow_mut();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(receiver_id)?;
            let shape = state.heap.shape(object.shape)?;
            if shape.find(brand).is_some() {
                drop(state);
                return Err(RuntimeError::Engine(Error::new(
                    ErrorKind::Type,
                    "private method is already present",
                )));
            }
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object.slots.clone(),
            )
        };
        entries.push(ShapeEntry {
            atom: brand,
            flags: PropertyFlags::data(true, true, true),
        });
        slots.push(PropertySlot::Data(RawValue::Undefined));
        state.replace_layout(receiver_id, prototype, &entries, slots)
    }

    /// Check a receiver against the brand derived from the method callable's
    /// HomeObject. A missing HomeObject brand has QuickJS's distinct error;
    /// an ordinary brand mismatch is returned as `false` for the VM opcode to
    /// map to either `#x in value` or `invalid brand on object`.
    pub(in crate::runtime) fn check_private_method_brand(
        &self,
        method: &CallableRef,
        receiver: &ObjectRef,
    ) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        if !receiver.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("private-method brand receiver"));
        }
        let brand = self.private_method_brand_atom(method)?;
        let state = self.0.state.borrow();
        let object = state.heap.object(receiver.object_id())?;
        Ok(state.heap.shape(object.shape)?.find(brand).is_some())
    }

    /// Validate that the method's HomeObject already owns a class-side brand.
    /// QuickJS performs this lookup before converting a private-get receiver,
    /// which makes the missing-brand error observable even for primitives.
    pub(in crate::runtime) fn require_private_method_brand(
        &self,
        method: &CallableRef,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        self.private_method_brand_atom(method).map(|_| ())
    }

    fn private_method_brand_atom(&self, method: &CallableRef) -> Result<Atom, RuntimeError> {
        let (_, home_object) = self.private_method_callable_parts(method)?;
        let Some(home_object) = home_object else {
            return Err(Self::missing_private_brand_error());
        };
        let home_object = ObjectRef::from_borrowed_handle(self.clone(), home_object)?;
        self.private_brand_atom(&home_object)
    }

    fn private_method_callable_parts(
        &self,
        method: &CallableRef,
    ) -> Result<(ObjectId, Option<ObjectId>), RuntimeError> {
        if !method.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("private-method callable"));
        }
        let method_id = method.as_object().object_id();
        let state = self.0.state.borrow();
        let object = state.heap.object(method_id)?;
        let ObjectPayload::BytecodeFunction {
            bytecode,
            home_object,
            ..
        } = &object.payload
        else {
            return Err(RuntimeError::Invariant(
                "private-method cell contains a non-bytecode callable",
            ));
        };
        let metadata = state.heap.function_bytecode(*bytecode)?.metadata;
        if object.is_constructor
            || metadata.has_prototype
            || metadata.constructor_kind != ConstructorKind::None
            || metadata.class_initializer_kind.is_some()
            || !metadata.strict
            || metadata.eval_kind != EvalKind::None
            || metadata.function_kind != FunctionKind::Normal
            || !metadata.needs_home_object
        {
            return Err(RuntimeError::Invariant(
                "private-method callable has invalid bytecode metadata",
            ));
        }
        Ok((method_id, *home_object))
    }

    fn private_brand_atom(&self, home_object: &ObjectRef) -> Result<Atom, RuntimeError> {
        if !home_object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime(
                "private-method brand HomeObject",
            ));
        }
        let state = self.0.state.borrow();
        let Some(brand) = state
            .heap
            .object_private_brand_home(home_object.object_id())?
        else {
            return Err(Self::missing_private_brand_error());
        };
        if state.atoms.kind(brand)? != AtomKind::Private {
            return Err(RuntimeError::Invariant(
                "private-method HomeObject brand is not a private atom",
            ));
        }
        Ok(brand)
    }

    fn missing_private_brand_error() -> RuntimeError {
        RuntimeError::Engine(Error::new(
            ErrorKind::Type,
            "expecting <brand> private field",
        ))
    }

    fn validate_private_receiver(
        &self,
        receiver: &ObjectRef,
        name: &PrivateNameRef,
    ) -> Result<(), RuntimeError> {
        if !receiver.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("private field receiver"));
        }
        self.validate_private_name(name)
    }

    fn validate_private_name(&self, name: &PrivateNameRef) -> Result<(), RuntimeError> {
        if !name.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("private name"));
        }
        if self.0.state.borrow().atoms.kind(name.atom())? != AtomKind::Private {
            return Err(RuntimeError::Invariant(
                "private-name handle contains a non-private atom",
            ));
        }
        Ok(())
    }

    fn private_field_error(
        &self,
        name: &PrivateNameRef,
        prefix: &str,
        suffix: &str,
    ) -> Result<Error, RuntimeError> {
        let mut message = NativeErrorMessage::new();
        message.push_utf8(prefix);
        self.0
            .state
            .borrow()
            .atoms
            .push_atom_get_str(name.atom(), &mut message)?;
        message.push_utf8(suffix);
        Ok(Error::from_native_message(ErrorKind::Type, message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instruction;
    use crate::function::UnlinkedFunction;

    fn private_name(runtime: &Runtime, text: &str) -> PrivateNameRef {
        runtime
            .new_private_name(JsString::try_from_utf8(text).unwrap())
            .unwrap()
    }

    fn private_atom_ref_count(runtime: &Runtime, name: &PrivateNameRef) -> u32 {
        runtime
            .0
            .state
            .borrow()
            .atoms
            .resolve(name.atom())
            .unwrap()
            .ref_count
            .expect("private names are not permanent atoms")
    }

    fn assert_type_error(error: RuntimeError, expected: &str) {
        assert!(
            matches!(error, RuntimeError::Engine(ref error)
                if error.kind() == ErrorKind::Type && error.message() == expected),
            "unexpected error: {error}"
        );
    }

    fn private_method_callable(runtime: &Runtime) -> (CallableRef, ObjectRef) {
        let context = runtime.new_context();
        let function = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new(
                    vec![Instruction::Undefined, Instruction::Return],
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 1,
                        strict: true,
                        needs_home_object: true,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap();
        let method = runtime
            .new_bytecode_closure(context.realm, &function)
            .unwrap();
        let home_object = runtime.new_object(None).unwrap();
        runtime
            .install_object_literal_home_object(&method, &home_object)
            .unwrap();
        (method, home_object)
    }

    #[test]
    fn equal_private_descriptions_have_unique_identity() {
        let runtime = Runtime::new();
        let first = private_name(&runtime, "#token");
        let second = private_name(&runtime, "#token");
        assert_ne!(first, second);

        let object = runtime.new_object(None).unwrap();
        runtime
            .define_private_field_own(&object, &first, Value::Int(1))
            .unwrap();
        assert!(runtime.has_private_field_own(&object, &first).unwrap());
        assert!(!runtime.has_private_field_own(&object, &second).unwrap());
    }

    #[test]
    fn private_access_is_own_only_and_uses_quickjs_errors() {
        let runtime = Runtime::new();
        let name = private_name(&runtime, "#value");
        let owner = runtime.new_object(None).unwrap();
        runtime
            .define_private_field_own(&owner, &name, Value::Int(7))
            .unwrap();
        let child = runtime.new_object(Some(&owner)).unwrap();

        assert!(!runtime.has_private_field_own(&child, &name).unwrap());
        assert_type_error(
            runtime.get_private_field_own(&child, &name).unwrap_err(),
            "private class field '#value' does not exist",
        );
        assert_type_error(
            runtime
                .set_private_field_own(&child, &name, Value::Int(8))
                .unwrap_err(),
            "private class field '#value' does not exist",
        );
        assert_type_error(
            runtime
                .define_private_field_own(&owner, &name, Value::Int(9))
                .unwrap_err(),
            "private class field '#value' already exists",
        );
        assert_eq!(
            runtime.get_private_field_own(&owner, &name).unwrap(),
            Value::Int(7)
        );
    }

    #[test]
    fn private_definition_bypasses_extensibility_and_stays_hidden() {
        let runtime = Runtime::new();
        let name = private_name(&runtime, "#hidden");
        let object = runtime.new_object(None).unwrap();
        let visible = runtime.intern_property_key("visible").unwrap();
        assert!(
            runtime
                .define_own_property(
                    &object,
                    &visible,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(1)),
                        writable: DescriptorField::Present(true),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        runtime.prevent_extensions(&object).unwrap();

        runtime
            .define_private_field_own(&object, &name, Value::Int(41))
            .unwrap();
        runtime
            .set_private_field_own(&object, &name, Value::Int(42))
            .unwrap();
        assert_eq!(
            runtime.get_private_field_own(&object, &name).unwrap(),
            Value::Int(42)
        );
        assert_eq!(runtime.own_property_keys(&object).unwrap(), [visible]);
    }

    #[test]
    fn private_handles_reject_cross_runtime_receivers_and_names() {
        let first = Runtime::new();
        let second = Runtime::new();
        let first_name = private_name(&first, "#first");
        let second_name = private_name(&second, "#second");
        let first_object = first.new_object(None).unwrap();
        let second_object = second.new_object(None).unwrap();

        assert_eq!(
            first
                .has_private_field_own(&second_object, &first_name)
                .unwrap_err(),
            RuntimeError::WrongRuntime("private field receiver")
        );
        assert_eq!(
            first
                .has_private_field_own(&first_object, &second_name)
                .unwrap_err(),
            RuntimeError::WrongRuntime("private name")
        );
    }

    #[test]
    fn private_shape_and_var_ref_edges_keep_atoms_alive_without_public_values() {
        let runtime = Runtime::new();
        let baseline_atoms = runtime.test_atom_count();
        let baseline_var_refs = runtime.heap_counts().var_ref_nodes;
        let name = private_name(&runtime, "#lifetime");
        assert_eq!(private_atom_ref_count(&runtime, &name), 1);
        let object = runtime.new_object(None).unwrap();
        runtime
            .define_private_field_own(&object, &name, Value::Int(1))
            .unwrap();
        assert_eq!(private_atom_ref_count(&runtime, &name), 2);
        let captured = runtime.new_private_var_ref(&name).unwrap();
        assert_eq!(private_atom_ref_count(&runtime, &name), 3);
        drop(name);

        assert_eq!(runtime.test_atom_count(), baseline_atoms + 1);
        assert_eq!(runtime.heap_counts().var_ref_nodes, baseline_var_refs + 1);
        let rooted_again = runtime.private_name_from_raw_var_ref(&captured).unwrap();
        assert_eq!(private_atom_ref_count(&runtime, &rooted_again), 3);
        assert!(
            runtime
                .has_private_field_own(&object, &rooted_again)
                .unwrap()
        );
        assert!(matches!(
            runtime.read_var_ref(&captured),
            Err(RuntimeError::Invariant(
                "ordinary VarRef read reached a private-element binding"
            ))
        ));

        drop(rooted_again);
        drop(captured);
        assert_eq!(runtime.heap_counts().var_ref_nodes, baseline_var_refs);
        assert_eq!(runtime.test_atom_count(), baseline_atoms + 1);
        drop(object);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }

    #[test]
    fn uninitialized_private_var_ref_has_one_typed_initialization_path() {
        let runtime = Runtime::new();
        let name = private_name(&runtime, "#captured");
        let root = runtime
            .new_uninitialized_captured_var_ref(true, true, ClosureVariableKind::PrivateField)
            .unwrap();
        runtime.initialize_private_var_ref(&root, &name).unwrap();
        assert_eq!(runtime.private_name_from_raw_var_ref(&root).unwrap(), name);
        assert!(matches!(
            runtime.initialize_private_var_ref(&root, &name),
            Err(RuntimeError::Invariant(
                "private-name VarRef was initialized more than once"
            ))
        ));
    }

    #[test]
    fn private_method_var_refs_keep_callable_capability_typed() {
        let runtime = Runtime::new();
        let (method, _) = private_method_callable(&runtime);
        let captured = runtime.new_private_method_var_ref(&method).unwrap();
        assert_eq!(
            runtime.private_method_from_raw_var_ref(&captured).unwrap(),
            method
        );
        assert!(matches!(
            runtime.read_var_ref(&captured),
            Err(RuntimeError::Invariant(
                "ordinary VarRef read reached a private-element binding"
            ))
        ));
        assert!(matches!(
            runtime.write_var_ref(&captured, Value::Int(1)),
            Err(RuntimeError::Invariant(
                "ordinary VarRef write reached a private-element binding"
            ))
        ));

        let uninitialized = runtime
            .new_uninitialized_captured_var_ref(true, true, ClosureVariableKind::PrivateMethod)
            .unwrap();
        runtime
            .initialize_private_method_var_ref(&uninitialized, &method)
            .unwrap();
        assert_eq!(
            runtime
                .private_method_from_raw_var_ref(&uninitialized)
                .unwrap(),
            method
        );
        assert!(matches!(
            runtime.initialize_private_method_var_ref(&uninitialized, &method),
            Err(RuntimeError::Invariant(
                "private-method VarRef was initialized more than once"
            ))
        ));
        assert!(matches!(
            runtime.private_name_from_raw_var_ref(&captured),
            Err(RuntimeError::Invariant(
                "private-name read reached an ordinary VarRef"
            ))
        ));
    }

    #[test]
    fn private_method_brands_are_hidden_own_only_and_ignore_extensibility() {
        let runtime = Runtime::new();
        let (method, home_object) = private_method_callable(&runtime);
        let receiver = runtime.new_object(None).unwrap();
        let inherited = runtime.new_object(Some(&receiver)).unwrap();

        assert_type_error(
            runtime
                .check_private_method_brand(&method, &receiver)
                .unwrap_err(),
            "expecting <brand> private field",
        );
        runtime.ensure_private_brand_home(&home_object).unwrap();
        let brand = runtime
            .0
            .state
            .borrow()
            .heap
            .object_private_brand_home(home_object.object_id())
            .unwrap()
            .unwrap();
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .atoms
                .resolve(brand)
                .unwrap()
                .ref_count,
            Some(1)
        );

        runtime.prevent_extensions(&receiver).unwrap();
        runtime
            .add_private_method_brand(&home_object, &receiver)
            .unwrap();
        assert!(
            runtime
                .check_private_method_brand(&method, &receiver)
                .unwrap()
        );
        assert!(
            !runtime
                .check_private_method_brand(&method, &inherited)
                .unwrap()
        );
        assert!(runtime.own_property_keys(&receiver).unwrap().is_empty());
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .atoms
                .resolve(brand)
                .unwrap()
                .ref_count,
            Some(2)
        );
        assert_type_error(
            runtime
                .add_private_method_brand(&home_object, &receiver)
                .unwrap_err(),
            "private method is already present",
        );
    }

    #[test]
    fn gc_reclaims_private_shape_atoms_and_private_value_cycles() {
        let runtime = Runtime::new();
        let baseline_atoms = runtime.test_atom_count();
        let baseline_objects = runtime.heap_counts().object_nodes;
        let name = private_name(&runtime, "#self");
        let object = runtime.new_object(None).unwrap();
        runtime
            .define_private_field_own(&object, &name, Value::Object(object.clone()))
            .unwrap();

        drop(name);
        drop(object);
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);
        assert_eq!(runtime.test_atom_count(), baseline_atoms + 1);

        let stats = runtime.run_gc().unwrap();
        assert_eq!(stats.cleanup.finalized_objects, 1);
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }

    #[test]
    fn compiled_private_fields_cover_instance_static_and_reference_operations() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let result = context
            .eval(
                r#"
                    class Box {
                        #value = 40;
                        #callback = function () { return "update"; };
                        static #offset = 2;

                        update() {
                            this.#value += Box.#offset;
                            return this.#value;
                        }
                        has(value) { return #value in value; }
                        readArrow() { return (() => this.#value)(); }
                        call() { return this.#callback(); }
                    }
                    var box = new Box();
                    [
                        box.update(),
                        box.has(box),
                        box.has({}),
                        box.readArrow(),
                        box.call()
                    ].join(",");
                "#,
            )
            .unwrap();

        assert_eq!(
            result,
            Value::String(JsString::from_static("42,true,false,42,update"))
        );
    }

    #[test]
    fn compiled_private_names_relay_through_nested_functions_eval_and_fresh_classes() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let result = context
            .eval(
                r#"
                    class A {
                        #value = 42;
                        arrow() { return (() => this.#value)(); }
                        nested() {
                            function read(receiver) { return receiver.#value; }
                            return read(this);
                        }
                        viaEval() { return eval("this.#value"); }
                        has(value) { return #value in value; }
                    }
                    class B {
                        #value = 42;
                        has(value) { return #value in value; }
                    }
                    var a = new A();
                    var b = new B();
                    [
                        a.arrow(),
                        a.nested(),
                        a.viaEval(),
                        a.has(b),
                        b.has(a),
                        a.has(a)
                    ].join(",");
                "#,
            )
            .unwrap();

        assert_eq!(
            result,
            Value::String(JsString::from_static("42,42,42,false,false,true"))
        );
    }

    #[test]
    fn compiled_forward_private_name_reads_match_quickjs_initialization_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let result = context
            .eval(
                r#"
                    var before;
                    eval(`
                        class C {
                            static before = #later in {};
                            static #later = 1;
                        }
                        before = C.before;
                    `);
                    var failure;
                    try {
                        eval(`
                            class D {
                                static failure = D.#later;
                                static #later = 1;
                            }
                        `);
                    } catch (error) {
                        failure = error.name + ":" + error.message;
                    }
                    before + "," + failure;
                "#,
            )
            .unwrap();

        assert_eq!(
            result,
            Value::String(JsString::from_static(
                "false,TypeError:private class field '#later' does not exist"
            ))
        );
    }

    #[test]
    fn abrupt_class_scope_reentry_reuses_captured_private_method_cell() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let result = context
            .eval(
                r#"
                    var checks = [], receiver;
                    function boom() { throw 0; }
                    for (var index = 0; index < 3; index++) {
                        try {
                            class C {
                                #field = 41;
                                #method() { return 42; }
                                [(
                                    checks.push(function (value) {
                                        return [
                                            #field in value,
                                            value.#field,
                                            #method in value,
                                            value.#method()
                                        ].join(":");
                                    }),
                                    index < 2 ? boom() : "ok"
                                )]() {}
                            }
                            receiver = new C();
                        } catch (error) {}
                    }
                    [
                        checks[0](receiver),
                        checks[1](receiver),
                        checks[2](receiver)
                    ].join("|");
                "#,
            )
            .unwrap();

        assert_eq!(
            result,
            Value::String(JsString::from_static(
                "true:41:true:42|true:41:true:42|true:41:true:42"
            ))
        );
    }

    #[test]
    fn private_method_get_checks_brand_home_before_primitive_receiver() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let result = context
            .eval(
                r#"
                    try {
                        class C {
                            #method() { return 42; }
                            [(1).#method] = 0;
                        }
                    } catch (error) {
                        error.name + ":" + error.message;
                    }
                "#,
            )
            .unwrap();

        assert_eq!(
            result,
            Value::String(JsString::from_static(
                "TypeError:expecting <brand> private field"
            ))
        );
    }

    #[test]
    fn uninitialized_private_in_preserves_quickjs_internal_tag_atom() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let result = context
            .eval(
                r#"
                    var receiver = {"[unsupported type]": 1};
                    var methodResult, fieldResult;
                    class Method {
                        [(methodResult = #later in receiver, "x")] = 0;
                        #later() {}
                    }
                    class Field {
                        [(fieldResult = #later in receiver, "x")] = 0;
                        #later = 1;
                    }
                    methodResult + "," + fieldResult;
                "#,
            )
            .unwrap();

        assert_eq!(result, Value::String(JsString::from_static("true,true")));
    }
}
