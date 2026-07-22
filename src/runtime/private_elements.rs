//! Runtime substrate for class private data fields.
//!
//! The semantic anchors are QuickJS 2026-06-04 `quickjs.c` 8366-8456
//! (`JS_DefinePrivateField`, `JS_GetPrivateField`, and
//! `JS_SetPrivateField`) and 15964-15994 (`js_operator_private_in`).  In
//! particular, these operations inspect only the receiver's own shape,
//! private definition bypasses ordinary extensibility checks, and duplicate or
//! absent fields use QuickJS's native TypeError spelling.

use super::*;
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
            if !var_ref.kind.is_private() || !var_ref.is_lexical || !var_ref.is_const {
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
            if !var_ref.kind.is_private() || !var_ref.is_lexical || !var_ref.is_const {
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
                "private-name identity escaped into an ECMAScript Value"
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
}
