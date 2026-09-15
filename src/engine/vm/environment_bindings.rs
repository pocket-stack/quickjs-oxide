//! Shared authentication of hidden eval/with local and closure owners.
pub(super) mod operation;
use super::{
    bindings::{FrameBinding, read_frame_binding},
    exception::runtime_error_to_vm_error,
};
use crate::engine::api::{error::Error, runtime::Runtime};
#[cfg(feature = "stack-vm")]
use crate::engine::code::bytecode::DynamicEnvironmentSource;
use crate::engine::code::bytecode::{EvalVariableSource, WithObjectSource};
use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName,
};
use crate::engine::code::runtime::PublishedFunctionSnapshot;
use crate::engine::heap::{ObjectPayload, roots::VarRefRoot};
use crate::engine::object::ObjectRef;
use crate::engine::value::Value;

fn eval_local_kind(
    executable: &PublishedFunctionSnapshot,
    index: u16,
) -> Option<ClosureVariableKind> {
    if executable.metadata.eval_variable_object_local == Some(index) {
        Some(ClosureVariableKind::EvalVariableObject)
    } else if executable.arg_eval_variable_object_local == Some(index) {
        Some(ClosureVariableKind::ArgEvalVariableObject)
    } else {
        None
    }
}
pub(super) fn eval_variable_object<'a>(
    runtime: &Runtime,
    executable: &PublishedFunctionSnapshot,
    source: EvalVariableSource,
    local: impl Fn(u16) -> Option<&'a FrameBinding>,
    closure_slots: &super::closure::ClosureSlots,
) -> Result<ObjectRef, Error> {
    let value = match source {
        EvalVariableSource::Local(index) => {
            let Some(expected_kind) = eval_local_kind(executable, index) else {
                return Err(Error::internal(
                    "eval variable opcode referenced an unauthenticated local",
                ));
            };
            let definition = *executable
                .local_definitions
                .get(usize::from(index))
                .ok_or_else(|| Error::internal("local definition index is out of bounds"))?;
            if definition.kind != expected_kind {
                return Err(Error::internal(
                    "eval variable opcode referenced a non-variable-object local",
                ));
            }
            let binding = local(index).ok_or_else(|| {
                Error::internal("eval variable-object local index is out of bounds")
            })?;
            if let FrameBinding::Captured(root) = binding {
                runtime
                    .validate_var_ref_metadata(
                        &root,
                        ClosureVariable {
                            source: ClosureSource::ParentLocal(index),
                            name: definition
                                .name
                                .map_or(ClosureVariableName::None, ClosureVariableName::Atom),
                            is_lexical: definition.is_lexical,
                            is_const: definition.is_const,
                            kind: definition.kind,
                        },
                    )
                    .map_err(runtime_error_to_vm_error)?;
            }
            read_frame_binding(runtime, binding)?
        }
        EvalVariableSource::Closure(index) => {
            let descriptor = executable
                .closure_variables
                .get(usize::from(index))
                .copied()
                .ok_or_else(|| {
                    Error::internal("eval variable-object closure index is out of bounds")
                })?;
            if !descriptor.kind.is_eval_variable_object() {
                return Err(Error::internal(
                    "eval variable opcode referenced a non-variable-object closure",
                ));
            }
            let root = closure_slots.get(usize::from(index)).ok_or_else(|| {
                Error::internal("eval variable-object closure slot is out of bounds")
            })?;
            runtime
                .validate_var_ref_metadata(&root, descriptor)
                .map_err(runtime_error_to_vm_error)?;
            runtime
                .read_var_ref(&root)
                .map_err(runtime_error_to_vm_error)?
        }
    };
    let Value::Object(object) = value else {
        return Err(Error::internal(
            "eval variable-object binding did not contain an Object",
        ));
    };
    if !object.belongs_to(runtime) {
        return Err(Error::internal(
            "eval variable object belongs to another runtime",
        ));
    }
    let state = runtime.0.state.borrow();
    let object_data = state
        .heap
        .object(object.object_id())
        .map_err(|error| Error::internal(error.to_string()))?;
    // Creation and publication authenticate an ordinary null-prototype
    // object. Once a syntactic-with method call exposes that receiver,
    // QuickJS lets user code mutate its prototype; later eval lookup must
    // therefore retain Ordinary branding without reasserting the initial
    // prototype shape.
    if !matches!(&object_data.payload, ObjectPayload::Ordinary) {
        return Err(Error::internal(
            "eval variable-object binding did not contain an ordinary Object",
        ));
    }
    drop(state);
    Ok(object)
}

pub(super) fn with_object<'a>(
    runtime: &Runtime,
    executable: &PublishedFunctionSnapshot,
    source: WithObjectSource,
    local: impl Fn(u16) -> Option<&'a FrameBinding>,
    closure_slots: &super::closure::ClosureSlots,
) -> Result<ObjectRef, Error> {
    let value = match source {
        WithObjectSource::Local(index) => {
            let definition = *executable
                .local_definitions
                .get(usize::from(index))
                .ok_or_else(|| Error::internal("local definition index is out of bounds"))?;
            if definition.kind != ClosureVariableKind::WithObject
                || definition.is_lexical
                || definition.is_const
            {
                return Err(Error::internal(
                    "dynamic with opcode referenced a non-with local",
                ));
            }
            let binding = local(index)
                .ok_or_else(|| Error::internal("with-object local index is out of bounds"))?;
            if let FrameBinding::Captured(root) = binding {
                runtime
                    .validate_var_ref_metadata(
                        &root,
                        ClosureVariable {
                            source: ClosureSource::ParentLocal(index),
                            name: definition
                                .name
                                .map_or(ClosureVariableName::None, ClosureVariableName::Atom),
                            is_lexical: definition.is_lexical,
                            is_const: definition.is_const,
                            kind: definition.kind,
                        },
                    )
                    .map_err(runtime_error_to_vm_error)?;
            }
            read_frame_binding(runtime, binding)?
        }
        WithObjectSource::Closure(index) => {
            let descriptor = executable
                .closure_variables
                .get(usize::from(index))
                .copied()
                .ok_or_else(|| Error::internal("with-object closure index is out of bounds"))?;
            if descriptor.kind != ClosureVariableKind::WithObject
                || descriptor.is_lexical
                || descriptor.is_const
            {
                return Err(Error::internal(
                    "dynamic with opcode referenced a non-with closure",
                ));
            }
            let root = closure_slots
                .get(usize::from(index))
                .ok_or_else(|| Error::internal("with-object closure slot is out of bounds"))?;
            runtime
                .validate_var_ref_metadata(&root, descriptor)
                .map_err(runtime_error_to_vm_error)?;
            runtime
                .read_var_ref(&root)
                .map_err(runtime_error_to_vm_error)?
        }
    };
    let Value::Object(object) = value else {
        return Err(Error::internal(
            "with-object binding did not contain an Object",
        ));
    };
    if !object.belongs_to(runtime) {
        return Err(Error::internal("with object belongs to another runtime"));
    }
    Ok(object)
}

#[cfg(feature = "stack-vm")]
pub(super) fn dynamic_object<'a>(
    runtime: &Runtime,
    executable: &PublishedFunctionSnapshot,
    source: DynamicEnvironmentSource,
    local: impl Fn(u16) -> Option<&'a FrameBinding>,
    closure_slots: &super::closure::ClosureSlots,
) -> Result<ObjectRef, Error> {
    match source {
        DynamicEnvironmentSource::Eval(source) => {
            eval_variable_object(runtime, executable, source, local, closure_slots)
        }
        DynamicEnvironmentSource::With(source) => {
            with_object(runtime, executable, source, local, closure_slots)
        }
    }
}

/// A lexical reference is already resolved; the global object still needs HasProperty.
pub(super) enum GlobalReference {
    Lexical(ObjectRef),
    Object {
        object: ObjectRef,
        key: crate::engine::object::PropertyKey,
    },
}

pub(super) fn global_reference(
    runtime: &Runtime,
    realm: crate::engine::heap::ContextId,
    executable: &PublishedFunctionSnapshot,
    closure_slots: &super::closure::ClosureSlots,
    index: u16,
) -> Result<GlobalReference, Error> {
    use crate::engine::{heap::RawValue, object::PropertyKey};
    let descriptor = executable
        .closure_variables
        .get(usize::from(index))
        .copied()
        .ok_or_else(|| Error::internal("global reference closure index is out of bounds"))?;
    if !matches!(
        descriptor.source,
        ClosureSource::GlobalDeclaration | ClosureSource::Global | ClosureSource::ParentGlobal(_)
    ) || !matches!(
        descriptor.kind,
        ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
    ) {
        return Err(Error::internal(
            "global reference opcode referenced a non-global closure",
        ));
    }
    let ClosureVariableName::Atom(atom) = descriptor.name else {
        return Err(Error::internal(
            "published global reference descriptor has no name atom",
        ));
    };
    let root = closure_slots
        .get(usize::from(index))
        .ok_or_else(|| Error::internal("global reference closure slot is out of bounds"))?;
    if !root.belongs_to(runtime) {
        return Err(Error::internal(
            "global reference closure belongs to another runtime",
        ));
    }

    let key = PropertyKey::from_borrowed_atom(runtime.clone(), atom)
        .map_err(|error| Error::internal(error.to_string()))?;
    let global_var_object = {
        let state = runtime.0.state.borrow();
        state
            .heap
            .context(realm)
            .map_err(|error| Error::internal(error.to_string()))?
            .global_var_object
    };
    let global_var_object = ObjectRef::from_borrowed_handle(runtime.clone(), global_var_object)
        .map_err(|error| Error::internal(error.to_string()))?;
    if let Some(root) = runtime
        .own_var_ref_root(&global_var_object, &key)
        .map_err(runtime_error_to_vm_error)?
    {
        let cell = runtime
            .0
            .state
            .borrow()
            .heap
            .var_ref(root.id())
            .map_err(|error| Error::internal(error.to_string()))?
            .clone();
        if !cell.is_lexical || cell.kind != ClosureVariableKind::Normal {
            return Err(Error::internal(
                "global lexical object contained a non-lexical VarRef",
            ));
        }
        if matches!(cell.value, RawValue::Uninitialized) {
            return Err(super::bindings::lexical_uninitialized_error(
                runtime,
                Some(atom),
                true,
            )?);
        }
        if cell.is_const {
            return Err(super::bindings::lexical_read_only_error(
                runtime,
                Some(atom),
            )?);
        }
        return Ok(GlobalReference::Lexical(global_var_object));
    }

    let global_object = runtime
        .global_object_for_realm(realm)
        .map_err(runtime_error_to_vm_error)?;
    Ok(GlobalReference::Object {
        object: global_object,
        key,
    })
}

pub(super) enum GlobalWrite {
    Cell(VarRefRoot),
    Property(crate::engine::object::PropertyKey),
}

fn global_write_binding<'a>(
    runtime: &Runtime,
    executable: &PublishedFunctionSnapshot,
    slots: &'a super::closure::ClosureSlots,
    index: u16,
    operation: &str,
) -> Result<
    (
        crate::engine::atom::Atom,
        crate::engine::heap::roots::VarRefView<'a>,
    ),
    Error,
> {
    let descriptor = executable
        .closure_variables
        .get(usize::from(index))
        .ok_or_else(|| Error::internal("global closure index is out of bounds"))?;
    if descriptor.kind.is_private() {
        return Err(Error::internal(format!(
            "global {operation} referenced a private-name binding"
        )));
    }
    let ClosureVariableName::Atom(atom) = descriptor.name else {
        return Err(Error::internal(
            "published global closure descriptor has no name atom",
        ));
    };
    let root = slots
        .get(usize::from(index))
        .ok_or_else(|| Error::internal("global closure slot is out of bounds"))?;
    if !root.belongs_to(runtime) {
        return Err(Error::internal("global closure belongs to another runtime"));
    }
    Ok((atom, root))
}

/// Classify against the live cell, including Program declaration promotion.
/// Initialization deliberately bypasses lexical/const checks just as the verified prologue does.
pub(super) fn prepare_global_write(
    runtime: &Runtime,
    executable: &PublishedFunctionSnapshot,
    slots: &super::closure::ClosureSlots,
    index: u16,
    initialize: bool,
) -> Result<GlobalWrite, Error> {
    let (atom, root) = global_write_binding(runtime, executable, slots, index, "write")?;
    let cell = runtime
        .0
        .state
        .borrow()
        .heap
        .var_ref(root.id())
        .map_err(|e| Error::internal(e.to_string()))?
        .clone();
    if initialize {
        return Ok(GlobalWrite::Cell(root.clone()));
    }
    let key = crate::engine::object::PropertyKey::from_borrowed_atom(runtime.clone(), atom)
        .map_err(|e| Error::internal(e.to_string()))?;
    if cell.is_lexical {
        if matches!(cell.value, crate::engine::heap::RawValue::Uninitialized) {
            return Err(runtime
                .native_atom_error(
                    crate::engine::api::ErrorKind::Reference,
                    "",
                    &key,
                    " is not initialized",
                )
                .map_err(runtime_error_to_vm_error)?);
        }
        if cell.is_const {
            return Err(runtime
                .native_atom_error(
                    crate::engine::api::ErrorKind::Type,
                    "'",
                    &key,
                    "' is read-only",
                )
                .map_err(runtime_error_to_vm_error)?);
        }
        return Ok(GlobalWrite::Cell(root.clone()));
    }
    if !matches!(cell.value, crate::engine::heap::RawValue::Uninitialized) && !cell.is_const {
        return Ok(GlobalWrite::Cell(root.clone()));
    }
    Ok(GlobalWrite::Property(key))
}

pub(super) fn prepare_global_delete(
    runtime: &Runtime,
    executable: &PublishedFunctionSnapshot,
    slots: &super::closure::ClosureSlots,
    index: u16,
) -> Result<Option<crate::engine::object::PropertyKey>, Error> {
    let (atom, root) = global_write_binding(runtime, executable, slots, index, "delete")?;
    if runtime
        .0
        .state
        .borrow()
        .heap
        .var_ref(root.id())
        .map_err(|e| Error::internal(e.to_string()))?
        .is_lexical
    {
        return Ok(None);
    }
    crate::engine::object::PropertyKey::from_borrowed_atom(runtime.clone(), atom)
        .map(Some)
        .map_err(|e| Error::internal(e.to_string()))
}
