use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::code::function::metadata::{ClosureVariable, ClosureVariableKind};
use crate::engine::heap::{HeapError, RawValue, VarRefData, VarRefId};
use crate::engine::object::{ObjectRef, SymbolRef};
use crate::engine::value::Value;
use crate::engine::vm::host_bridge as vm_host;

impl Runtime {
    pub(crate) fn new_var_ref(
        &self,
        value: Value,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Result<VarRefRoot, RuntimeError> {
        let _operation = self.operation();
        self.validate_value_domain(&value, "captured variable")?;
        let raw = self.raw_property_value(&value)?;
        let mut state = self.0.state.borrow_mut();
        let retained_atom = if let RawValue::Symbol(atom) = &raw {
            state.atoms.retain(*atom)?;
            Some(*atom)
        } else {
            None
        };
        let data = VarRefData::captured(raw, is_lexical, is_const, kind);
        let id = match state.heap.allocate_var_ref(data) {
            Ok(id) => id,
            Err(error) => {
                if let Some(atom) = retained_atom {
                    state.atoms.release(atom)?;
                }
                return Err(error.into());
            }
        };
        drop(state);
        drop(value);
        Ok(VarRefRoot::from_owned_handle(self.clone(), id))
    }

    pub(crate) fn new_uninitialized_var_ref(&self) -> Result<VarRefRoot, RuntimeError> {
        let _operation = self.operation();
        let id = self
            .0
            .state
            .borrow_mut()
            .heap
            .allocate_var_ref(VarRefData::captured(
                RawValue::Uninitialized,
                false,
                false,
                ClosureVariableKind::Normal,
            ))?;
        Ok(VarRefRoot::from_owned_handle(self.clone(), id))
    }

    pub(crate) fn new_uninitialized_captured_var_ref(
        &self,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Result<VarRefRoot, RuntimeError> {
        let _operation = self.operation();
        let id = self
            .0
            .state
            .borrow_mut()
            .heap
            .allocate_var_ref(VarRefData::captured(
                RawValue::Uninitialized,
                is_lexical,
                is_const,
                kind,
            ))?;
        Ok(VarRefRoot::from_owned_handle(self.clone(), id))
    }

    pub(crate) fn set_var_ref_metadata(
        &self,
        root: &VarRefRoot,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Result<(), RuntimeError> {
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        self.0.state.borrow_mut().heap.set_var_ref_metadata(
            root.id(),
            is_lexical,
            is_const,
            kind,
        )?;
        Ok(())
    }

    pub(crate) fn reset_var_ref_uninitialized(
        &self,
        root: &VarRefRoot,
    ) -> Result<(), RuntimeError> {
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        let mut state = self.0.state.borrow_mut();
        let cleanup = state
            .heap
            .replace_var_ref_value(root.id(), RawValue::Uninitialized)?;
        state.apply_cleanup(cleanup)
    }

    pub(crate) fn read_var_ref(&self, root: &VarRefRoot) -> Result<Value, RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        let raw = {
            let state = self.0.state.borrow();
            let var_ref = state.heap.var_ref(root.id())?;
            if var_ref.kind.is_private() {
                return Err(RuntimeError::Invariant(
                    "ordinary VarRef read reached a private-element binding",
                ));
            }
            var_ref.value.clone()
        };
        self.root_raw_value(&raw)
    }

    pub(crate) fn raw_var_ref_value(&self, root: &VarRefRoot) -> Result<RawValue, RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        Ok(self.0.state.borrow().heap.var_ref(root.id())?.value.clone())
    }

    pub(crate) fn validate_var_ref_metadata(
        &self,
        root: &VarRefRoot,
        descriptor: ClosureVariable,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        let var_ref = self.0.state.borrow();
        let var_ref = var_ref.heap.var_ref(root.id())?;
        if !vm_host::closure_view_matches_cell(
            (var_ref.is_lexical, var_ref.is_const, var_ref.kind),
            descriptor,
        ) {
            return Err(RuntimeError::Invariant(
                "closure descriptor metadata does not match the shared variable cell",
            ));
        }
        Ok(())
    }

    pub(crate) fn write_var_ref(
        &self,
        root: &VarRefRoot,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        if self
            .0
            .state
            .borrow()
            .heap
            .var_ref(root.id())?
            .kind
            .is_private()
        {
            return Err(RuntimeError::Invariant(
                "ordinary VarRef write reached a private-element binding",
            ));
        }
        self.validate_value_domain(&value, "captured variable")?;
        let raw = self.raw_property_value(&value)?;
        let mut state = self.0.state.borrow_mut();
        let retained_atom = if let RawValue::Symbol(atom) = &raw {
            state.atoms.retain(*atom)?;
            Some(*atom)
        } else {
            None
        };
        let cleanup = match state.heap.replace_var_ref_value(root.id(), raw) {
            Ok(cleanup) => cleanup,
            Err(error) => {
                if let Some(atom) = retained_atom {
                    state.atoms.release(atom)?;
                }
                return Err(error.into());
            }
        };
        state.apply_cleanup(cleanup)?;
        drop(state);
        drop(value);
        Ok(())
    }

    pub(crate) fn take_owned_raw_value(&self, value: RawValue) -> Result<Value, RuntimeError> {
        Ok(match value {
            RawValue::Undefined => Value::Undefined,
            RawValue::Null => Value::Null,
            RawValue::Bool(value) => Value::Bool(value),
            RawValue::Int(value) => Value::Int(value),
            RawValue::Float(value) => Value::Float(value),
            RawValue::BigInt(value) => Value::BigInt(value),
            RawValue::String(value) => Value::String(value),
            RawValue::Symbol(atom) => Value::Symbol(SymbolRef::from_owned_atom(self.clone(), atom)),
            RawValue::Private(_) => {
                return Err(RuntimeError::Invariant(
                    "private-name identity occupied a public runtime root",
                ));
            }
            RawValue::Object(object) => {
                Value::Object(ObjectRef::from_owned_handle(self.clone(), object))
            }
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel occupied the pending exception slot",
                ));
            }
        })
    }

    pub(crate) fn root_raw_value(&self, value: &RawValue) -> Result<Value, RuntimeError> {
        Ok(match value {
            RawValue::Undefined => Value::Undefined,
            RawValue::Null => Value::Null,
            RawValue::Bool(value) => Value::Bool(*value),
            RawValue::Int(value) => Value::Int(*value),
            RawValue::Float(value) => Value::Float(*value),
            RawValue::BigInt(value) => Value::BigInt(value.clone()),
            RawValue::String(value) => Value::String(value.clone()),
            RawValue::Symbol(atom) => {
                Value::Symbol(SymbolRef::from_borrowed_atom(self.clone(), *atom)?)
            }
            RawValue::Private(_) => {
                return Err(RuntimeError::Invariant(
                    "private-name identity escaped into an ECMAScript Value",
                ));
            }
            RawValue::Object(object) => {
                Value::Object(ObjectRef::from_borrowed_handle(self.clone(), *object)?)
            }
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel escaped from an object property",
                ));
            }
        })
    }
}

pub(crate) struct VarRefRoot {
    pub(crate) runtime: Runtime,
    pub(crate) id: VarRefId,
}

impl VarRefRoot {
    pub(crate) fn from_owned_handle(runtime: Runtime, id: VarRefId) -> Self {
        Self { runtime, id }
    }

    pub(crate) fn from_borrowed_handle(runtime: Runtime, id: VarRefId) -> Result<Self, HeapError> {
        runtime.retain_var_ref_handle(id)?;
        Ok(Self { runtime, id })
    }

    pub(crate) const fn id(&self) -> VarRefId {
        self.id
    }

    pub(crate) fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.runtime.is_same_runtime(runtime)
    }
}

impl Clone for VarRefRoot {
    fn clone(&self) -> Self {
        self.runtime
            .retain_var_ref_handle(self.id)
            .expect("a live VarRef root must retain its cell");
        Self {
            runtime: self.runtime.clone(),
            id: self.id,
        }
    }
}

impl Drop for VarRefRoot {
    fn drop(&mut self) {
        self.runtime.release_var_ref_handle(self.id);
    }
}
