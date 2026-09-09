use crate::engine::api::error::ErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::Atom;
use crate::engine::code::function::metadata::ClosureVariableKind;
use crate::engine::heap::roots::VarRefRoot;

use crate::engine::heap::{ContextId, ObjectPayload, PropertySlot};
use crate::engine::object::shape::PropertyFlags;
use crate::engine::object::{ObjectRef, PropertyKey};
#[cfg(test)]
use crate::engine::value::JsString;
use crate::engine::value::Value;

impl Runtime {
    pub(crate) fn check_global_lexical_declaration(
        &self,
        realm: ContextId,
        key: &PropertyKey,
    ) -> Result<(), RuntimeError> {
        let conflicts = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let lexical = state.heap.object(context.global_var_object)?;
            let lexical_shape = state.heap.shape(lexical.shape)?;
            let global = state.heap.object(context.global_object)?;
            let global_shape = state.heap.shape(global.shape)?;
            let lexical_exists = lexical_shape.find(key.atom()).is_some();
            let fixed_global_exists = global_shape
                .find(key.atom())
                .and_then(|index| global_shape.entries().get(index as usize))
                .is_some_and(|entry| !entry.flags.configurable);
            lexical_exists || fixed_global_exists
        };
        if conflicts {
            let error =
                self.native_atom_error(ErrorKind::Syntax, "redeclaration of '", key, "'")?;
            return Err(RuntimeError::Engine(error));
        }
        Ok(())
    }

    pub(crate) fn check_global_var_declaration(
        &self,
        realm: ContextId,
        key: &PropertyKey,
    ) -> Result<(), RuntimeError> {
        let conflict = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let lexical = state.heap.object(context.global_var_object)?;
            let lexical_shape = state.heap.shape(lexical.shape)?;
            let global = state.heap.object(context.global_object)?;
            let global_shape = state.heap.shape(global.shape)?;
            if global_shape.find(key.atom()).is_none() && !global.extensible {
                Some(ErrorKind::Type)
            } else if lexical_shape.find(key.atom()).is_some() {
                Some(ErrorKind::Syntax)
            } else {
                None
            }
        };
        match conflict {
            Some(ErrorKind::Type) => {
                let error =
                    self.native_atom_error(ErrorKind::Type, "cannot define variable '", key, "'")?;
                Err(RuntimeError::Engine(error))
            }
            Some(ErrorKind::Syntax) => {
                let error =
                    self.native_atom_error(ErrorKind::Syntax, "redeclaration of '", key, "'")?;
                Err(RuntimeError::Engine(error))
            }
            Some(_) => Err(RuntimeError::Invariant(
                "global var preflight produced an impossible error kind",
            )),
            None => Ok(()),
        }
    }

    pub(crate) fn check_global_function_declaration(
        &self,
        realm: ContextId,
        key: &PropertyKey,
    ) -> Result<(), RuntimeError> {
        let conflict =
            {
                let state = self.0.state.borrow();
                let context = state.heap.context(realm)?;
                let lexical = state.heap.object(context.global_var_object)?;
                let lexical_shape = state.heap.shape(lexical.shape)?;
                let global = state.heap.object(context.global_object)?;
                let global_shape = state.heap.shape(global.shape)?;
                let cannot_define =
                    match global_shape.find(key.atom()) {
                        None => !global.extensible,
                        Some(index) => {
                            let index = usize::try_from(index).map_err(|_| {
                                RuntimeError::Invariant("shape index does not fit usize")
                            })?;
                            let entry = global_shape.entries().get(index).ok_or(
                                RuntimeError::Invariant("shape lookup index was out of bounds"),
                            )?;
                            let slot = global.slots.get(index).ok_or(RuntimeError::Invariant(
                                "shape property has no parallel object slot",
                            ))?;
                            !entry.flags.configurable
                                && (matches!(slot, PropertySlot::Accessor { .. })
                                    || !entry.flags.writable
                                    || !entry.flags.enumerable)
                        }
                    };
                if cannot_define {
                    Some(ErrorKind::Type)
                } else if lexical_shape.find(key.atom()).is_some() {
                    Some(ErrorKind::Syntax)
                } else {
                    None
                }
            };
        match conflict {
            Some(ErrorKind::Type) => {
                let error =
                    self.native_atom_error(ErrorKind::Type, "cannot define variable '", key, "'")?;
                Err(RuntimeError::Engine(error))
            }
            Some(ErrorKind::Syntax) => {
                let error =
                    self.native_atom_error(ErrorKind::Syntax, "redeclaration of '", key, "'")?;
                Err(RuntimeError::Engine(error))
            }
            Some(_) => Err(RuntimeError::Invariant(
                "global function preflight produced an impossible error kind",
            )),
            None => Ok(()),
        }
    }

    pub(crate) fn resolve_global_var(
        &self,
        realm: ContextId,
        name: Atom,
    ) -> Result<VarRefRoot, RuntimeError> {
        let key = PropertyKey::from_borrowed_atom(self.clone(), name)?;
        let global_var_object = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            context.global_var_object
        };
        let global_var_object = ObjectRef::from_borrowed_handle(self.clone(), global_var_object)?;
        if let Some(root) = self.own_var_ref_root(&global_var_object, &key)? {
            return Ok(root);
        }

        self.resolve_global_object_var(realm, &key)
    }

    /// Resolve only the object-environment half of a global name. Program
    /// function declarations use this after an earlier lexical declaration
    /// of the same name: QuickJS creates a distinct global-object binding even
    /// though ordinary identifier resolution still selects the lexical slot.
    pub(crate) fn resolve_global_object_var(
        &self,
        realm: ContextId,
        key: &PropertyKey,
    ) -> Result<VarRefRoot, RuntimeError> {
        let (global_object, hidden) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let global = state.heap.object(context.global_object)?;
            let ObjectPayload::GlobalObject { uninitialized_vars } = global.payload else {
                return Err(RuntimeError::Invariant(
                    "realm global object has no unresolved-name table",
                ));
            };
            (context.global_object, uninitialized_vars)
        };

        let global_object = ObjectRef::from_borrowed_handle(self.clone(), global_object)?;
        if self.is_auto_init_own_property(&global_object, key)? {
            self.materialize_auto_init_property(&global_object, key)?;
        }
        if let Some(root) = self.own_var_ref_root(&global_object, key)? {
            return Ok(root);
        }

        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
        if let Some(root) = self.own_var_ref_root(&hidden, key)? {
            return Ok(root);
        }
        let root = self.new_uninitialized_var_ref()?;
        self.store_property_slot(
            &hidden,
            key,
            PropertyFlags::data(true, true, true),
            PropertySlot::VarRef(root.id()),
        )?;
        Ok(root)
    }

    #[cfg(test)]
    pub(crate) fn create_global_lexical_for_test(
        &self,
        realm: ContextId,
        name: &str,
        is_const: bool,
        initial_value: Option<Value>,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        self.create_global_lexical_binding(realm, &key, is_const, initial_value)
            .map(drop)
    }

    #[cfg(test)]
    pub(crate) fn create_global_lexical_js_string_for_test(
        &self,
        realm: ContextId,
        name: &JsString,
        is_const: bool,
        initial_value: Option<Value>,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key_js_string(name)?;
        self.create_global_lexical_binding(realm, &key, is_const, initial_value)
            .map(drop)
    }

    pub(crate) fn create_global_lexical_binding(
        &self,
        realm: ContextId,
        key: &PropertyKey,
        is_const: bool,
        initial_value: Option<Value>,
    ) -> Result<VarRefRoot, RuntimeError> {
        let (global_var_object, global_object, hidden) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let global = state.heap.object(context.global_object)?;
            let ObjectPayload::GlobalObject { uninitialized_vars } = &global.payload else {
                return Err(RuntimeError::Invariant(
                    "realm global object has no unresolved-name table",
                ));
            };
            (
                context.global_var_object,
                context.global_object,
                *uninitialized_vars,
            )
        };
        let global_var_object = ObjectRef::from_borrowed_handle(self.clone(), global_var_object)?;
        if self.has_own_property(&global_var_object, key)? {
            return Err(RuntimeError::Invariant(
                "attempted to redeclare a global lexical binding after preflight",
            ));
        }
        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
        let global_object = ObjectRef::from_borrowed_handle(self.clone(), global_object)?;
        let root = if let Some(root) = self.own_var_ref_root(&global_object, key)? {
            let (flags, value) = {
                let state = self.0.state.borrow();
                let object = state.heap.object(global_object.object_id())?;
                let shape = state.heap.shape(object.shape)?;
                let index = shape.find(key.atom()).ok_or(RuntimeError::Invariant(
                    "global VarRef disappeared during lexical creation",
                ))? as usize;
                let flags = shape.entries()[index].flags;
                let value = state.heap.var_ref(root.id())?.value.clone();
                (flags, value)
            };
            let value = self.root_raw_value(&value)?;
            let replacement =
                self.new_var_ref(value, false, !flags.writable, ClosureVariableKind::Normal)?;
            self.store_property_slot(
                &global_object,
                key,
                flags,
                PropertySlot::VarRef(replacement.id()),
            )?;
            self.reset_var_ref_uninitialized(&root)?;
            root
        } else if let Some(root) = self.own_var_ref_root(&hidden, key)? {
            if !self.delete_property(&hidden, key)? {
                return Err(RuntimeError::Invariant(
                    "hidden global VarRef property was not configurable",
                ));
            }
            root
        } else {
            self.new_uninitialized_var_ref()?
        };
        self.set_var_ref_metadata(&root, true, is_const, ClosureVariableKind::Normal)?;
        if let Some(value) = initial_value {
            self.write_var_ref(&root, value)?;
        }
        self.store_property_slot(
            &global_var_object,
            key,
            PropertyFlags::data(!is_const, true, true),
            PropertySlot::VarRef(root.id()),
        )?;
        Ok(root)
    }

    pub(crate) fn create_global_var_binding(
        &self,
        realm: ContextId,
        key: &PropertyKey,
        mode: GlobalBindingCreationMode,
    ) -> Result<VarRefRoot, RuntimeError> {
        let (global_object, hidden) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let global = state.heap.object(context.global_object)?;
            let ObjectPayload::GlobalObject { uninitialized_vars } = global.payload else {
                return Err(RuntimeError::Invariant(
                    "realm global object has no unresolved-name table",
                ));
            };
            (context.global_object, uninitialized_vars)
        };
        let root = self.resolve_global_var(realm, key.atom())?;
        let global_object = ObjectRef::from_borrowed_handle(self.clone(), global_object)?;
        if self.has_own_property(&global_object, key)? {
            return Ok(root);
        }
        if !self.is_extensible(&global_object)? {
            return Err(RuntimeError::Invariant(
                "global object became non-extensible after var preflight",
            ));
        }

        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
        let Some(hidden_root) = self.own_var_ref_root(&hidden, key)? else {
            return Err(RuntimeError::Invariant(
                "new global var has no unresolved VarRef",
            ));
        };
        if hidden_root.id() != root.id() {
            return Err(RuntimeError::Invariant(
                "new global var resolved a different hidden VarRef",
            ));
        }
        if !self.delete_property(&hidden, key)? {
            return Err(RuntimeError::Invariant(
                "hidden global VarRef property was not configurable",
            ));
        }
        self.write_var_ref(&root, Value::Undefined)?;
        self.set_var_ref_metadata(&root, false, false, ClosureVariableKind::Normal)?;
        self.store_property_slot(
            &global_object,
            key,
            PropertyFlags::data(true, true, mode.configurable()),
            PropertySlot::VarRef(root.id()),
        )?;
        Ok(root)
    }

    pub(crate) fn create_global_function_binding(
        &self,
        realm: ContextId,
        key: &PropertyKey,
        mode: GlobalBindingCreationMode,
    ) -> Result<VarRefRoot, RuntimeError> {
        let (global_object, hidden) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let global = state.heap.object(context.global_object)?;
            let ObjectPayload::GlobalObject { uninitialized_vars } = global.payload else {
                return Err(RuntimeError::Invariant(
                    "realm global object has no unresolved-name table",
                ));
            };
            (context.global_object, uninitialized_vars)
        };
        // Deliberately skip the lexical environment here. QuickJS permits the
        // ordered `let f; function f(){}` descriptor pair and creates a
        // distinct global-object binding for the function descriptor; the
        // hoisted raw initialization still targets the first (lexical) slot.
        let root = self.resolve_global_object_var(realm, key)?;
        let global_object = ObjectRef::from_borrowed_handle(self.clone(), global_object)?;
        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
        if !self.has_own_property(&global_object, key)? {
            if !self.is_extensible(&global_object)? {
                return Err(RuntimeError::Invariant(
                    "global object became non-extensible after function preflight",
                ));
            }
            let Some(hidden_root) = self.own_var_ref_root(&hidden, key)? else {
                return Err(RuntimeError::Invariant(
                    "new global function has no unresolved VarRef",
                ));
            };
            if hidden_root.id() != root.id() {
                return Err(RuntimeError::Invariant(
                    "new global function resolved a different hidden VarRef",
                ));
            }
            if !self.delete_property(&hidden, key)? {
                return Err(RuntimeError::Invariant(
                    "hidden global VarRef property was not configurable",
                ));
            }
            self.write_var_ref(&root, Value::Undefined)?;
            self.set_var_ref_metadata(&root, false, false, ClosureVariableKind::Normal)?;
            self.store_property_slot(
                &global_object,
                key,
                PropertyFlags::data(true, true, mode.configurable()),
                PropertySlot::VarRef(root.id()),
            )?;
            return Ok(root);
        }

        // Existing configurable properties are replaced with ordinary
        // writable/enumerable data properties without invoking accessors.
        // Script makes the replacement permanent while eval preserves
        // configurability. Fixed W/E data properties keep their existing
        // non-configurable attributes and VarRef identity.
        let (flags, slot) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(global_object.object_id())?;
            let shape = state.heap.shape(object.shape)?;
            let index = usize::try_from(shape.find(key.atom()).ok_or(RuntimeError::Invariant(
                "global function property disappeared after declaration creation",
            ))?)
            .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
            let flags = shape
                .entries()
                .get(index)
                .ok_or(RuntimeError::Invariant(
                    "shape lookup index was out of bounds",
                ))?
                .flags;
            let slot = object
                .slots
                .get(index)
                .ok_or(RuntimeError::Invariant(
                    "shape property has no parallel object slot",
                ))?
                .clone();
            (flags, slot)
        };

        let hidden_root = self.own_var_ref_root(&hidden, key)?;
        match &slot {
            PropertySlot::VarRef(global_root) => {
                if *global_root != root.id() {
                    return Err(RuntimeError::Invariant(
                        "global function resolved a different property VarRef",
                    ));
                }
                if hidden_root.is_some() {
                    return Err(RuntimeError::Invariant(
                        "global function name exists in both property and hidden tables",
                    ));
                }
            }
            PropertySlot::Data(value) => {
                let value = self.root_raw_value(value)?;
                self.write_var_ref(&root, value)?;
                if hidden_root
                    .as_ref()
                    .is_none_or(|hidden_root| hidden_root.id() != root.id())
                {
                    return Err(RuntimeError::Invariant(
                        "global function data property has no matching hidden VarRef",
                    ));
                }
            }
            PropertySlot::Accessor { .. } => {
                if !flags.configurable {
                    return Err(RuntimeError::Invariant(
                        "fixed global accessor survived function preflight",
                    ));
                }
                if hidden_root
                    .as_ref()
                    .is_none_or(|hidden_root| hidden_root.id() != root.id())
                {
                    return Err(RuntimeError::Invariant(
                        "global function accessor has no matching hidden VarRef",
                    ));
                }
            }
            PropertySlot::AutoInit(_) => {
                return Err(RuntimeError::Invariant(
                    "global function autoinit property was not materialized",
                ));
            }
        }

        let function_flags = if flags.configurable {
            PropertyFlags::data(true, true, mode.configurable())
        } else {
            if !flags.writable || !flags.enumerable {
                return Err(RuntimeError::Invariant(
                    "fixed global data property survived function preflight",
                ));
            }
            PropertyFlags::data(true, true, false)
        };
        self.store_property_slot(
            &global_object,
            key,
            function_flags,
            PropertySlot::VarRef(root.id()),
        )?;
        if let Some(hidden_root) = hidden_root {
            if hidden_root.id() != root.id() {
                return Err(RuntimeError::Invariant(
                    "global function hidden VarRef changed during creation",
                ));
            }
            if !self.delete_property(&hidden, key)? {
                return Err(RuntimeError::Invariant(
                    "hidden global VarRef property was not configurable",
                ));
            }
        }
        if matches!(slot, PropertySlot::Accessor { .. }) {
            self.reset_var_ref_uninitialized(&root)?;
        }
        self.set_var_ref_metadata(&root, false, false, ClosureVariableKind::Normal)?;
        Ok(root)
    }

    #[cfg(test)]
    pub(crate) fn initialize_global_lexical_for_test(
        &self,
        realm: ContextId,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let global_var_object = {
            let state = self.0.state.borrow();
            state.heap.context(realm)?.global_var_object
        };
        let global_var_object = ObjectRef::from_borrowed_handle(self.clone(), global_var_object)?;
        let root =
            self.own_var_ref_root(&global_var_object, &key)?
                .ok_or(RuntimeError::Invariant(
                    "test initialized a missing global lexical binding",
                ))?;
        self.write_var_ref(&root, value)
    }
}

/// Controls the property attributes used while instantiating declarations on
/// a realm's global object. Ordinary Script declarations are permanent;
/// QuickJS keeps properties first created or configurably replaced by direct
/// or indirect eval configurable so a later `delete` can remove them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GlobalBindingCreationMode {
    Script,
    Eval,
}

impl GlobalBindingCreationMode {
    pub(crate) const fn configurable(self) -> bool {
        matches!(self, Self::Eval)
    }
}
