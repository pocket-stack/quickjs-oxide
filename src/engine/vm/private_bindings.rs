//! Shared private identity initialization; these operations cannot invoke JS.
use super::bindings::FrameBinding;
use super::exception::runtime_error_to_vm_error;
use crate::engine::api::{error::Error, runtime::Runtime};
use crate::engine::atom::Atom;
use crate::engine::code::function::metadata::{ClosureVariableKind, VariableDefinition};
use crate::engine::value::Value;

pub(in crate::engine::vm) fn validate_definition(
    definition: VariableDefinition,
) -> Result<(Atom, ClosureVariableKind), Error> {
    if !matches!(
        definition.kind,
        ClosureVariableKind::PrivateField
            | ClosureVariableKind::PrivateMethod
            | ClosureVariableKind::PrivateGetter
            | ClosureVariableKind::PrivateSetter
            | ClosureVariableKind::PrivateGetterSetter
    ) || !definition.is_lexical
        || !definition.is_const
        || definition.is_parameter_initializer
    {
        return Err(Error::internal(
            "private-name opcode referenced a non-private local definition",
        ));
    }
    definition
        .name
        .map(|name| (name, definition.kind))
        .ok_or_else(|| Error::internal("private-element local has no source name"))
}

pub(in crate::engine::vm) fn initialize_name(
    runtime: &Runtime,
    definition: VariableDefinition,
    binding: &mut FrameBinding,
) -> Result<(), Error> {
    let (source_name, kind) = validate_definition(definition)?;
    if kind != ClosureVariableKind::PrivateField {
        return Err(Error::internal(
            "private-name initializer referenced a non-field binding",
        ));
    }
    let description = runtime
        .0
        .state
        .borrow()
        .atoms
        .to_js_string(source_name)
        .map_err(|error| Error::internal(error.to_string()))?;
    let name = runtime
        .new_private_name(description)
        .map_err(runtime_error_to_vm_error)?;
    match binding {
        FrameBinding::Uninitialized => {
            *binding = FrameBinding::Private(name);
            Ok(())
        }
        FrameBinding::Private(_) => Err(Error::internal(
            "private-name local was initialized more than once",
        )),
        FrameBinding::PrivateCallable(_) => Err(Error::internal(
            "private-name initializer reached a private-method frame cell",
        )),
        FrameBinding::Captured(root) => runtime
            .initialize_private_var_ref(&root, &name)
            .map_err(runtime_error_to_vm_error),
        FrameBinding::Direct(_) => Err(Error::internal(
            "private-name initializer reached an ordinary frame value",
        )),
    }
}

pub(in crate::engine::vm) fn initialize_callable(
    runtime: &Runtime,
    definition: VariableDefinition,
    binding: &mut FrameBinding,
    home_object: Value,
    callable_value: Value,
    infer_name: bool,
    accepts_kind: impl FnOnce(ClosureVariableKind) -> bool,
) -> Result<(), Error> {
    let (source_name, kind) = validate_definition(definition)?;
    if !accepts_kind(kind) {
        return Err(Error::internal(
            "private-callable initializer referenced an incompatible binding",
        ));
    }
    let Value::Object(home_object) = home_object else {
        return Err(Error::internal(
            "private-callable initializer did not receive a HomeObject",
        ));
    };
    let callable = runtime
        .callable_from_value(callable_value)
        .map_err(|error| Error::internal(error.to_string()))?;
    if infer_name {
        let name = runtime
            .0
            .state
            .borrow()
            .atoms
            .to_js_string(source_name)
            .map_err(|error| Error::internal(error.to_string()))?;
        runtime
            .define_object_name(&Value::Object(callable.as_object().clone()), &name)
            .map_err(runtime_error_to_vm_error)?;
    }
    runtime
        .install_object_literal_home_object(&callable, &home_object)
        .map_err(runtime_error_to_vm_error)?;

    match binding {
        FrameBinding::Uninitialized => {
            *binding = FrameBinding::PrivateCallable(callable);
            Ok(())
        }
        FrameBinding::Captured(root) => runtime
            .initialize_private_callable_var_ref(&root, &callable, kind)
            .map_err(runtime_error_to_vm_error),
        FrameBinding::PrivateCallable(_) => Err(Error::internal(
            "private-callable local was initialized more than once",
        )),
        FrameBinding::Private(_) => Err(Error::internal(
            "private-callable initializer reached a private-field frame cell",
        )),
        FrameBinding::Direct(_) => Err(Error::internal(
            "private-callable initializer reached an ordinary frame value",
        )),
    }
}

#[cfg(feature = "stack-vm")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Initialization {
    Name,
    Method,
    Accessor,
}

#[cfg(feature = "stack-vm")]
#[inline(never)]
pub(super) fn step(
    runtime: &Runtime,
    execution: &mut super::execution::RunningExecution,
    id: super::frame::FrameId,
    index: u16,
    kind: Initialization,
) -> Result<Option<super::Completion>, Error> {
    let frame = execution.frames.current_mut(id)?;
    let definition = frame.executable.local_definitions[usize::from(index)];
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    let mut retained_home = None;
    let result = if kind == Initialization::Name {
        initialize_name(
            runtime,
            definition,
            execution.slots.local_mut(&frame.window, index)?,
        )
    } else {
        let callable = execution.slots.pop(&mut frame.window)?;
        let home = execution.slots.pop(&mut frame.window)?;
        let result = initialize_callable(
            runtime,
            definition,
            execution.slots.local_mut(&frame.window, index)?,
            home.clone(),
            callable,
            kind == Initialization::Method,
            |binding_kind| match kind {
                Initialization::Method => binding_kind == ClosureVariableKind::PrivateMethod,
                Initialization::Accessor => matches!(
                    binding_kind,
                    ClosureVariableKind::PrivateGetter
                        | ClosureVariableKind::PrivateSetter
                        | ClosureVariableKind::PrivateGetterSetter
                ),
                Initialization::Name => false,
            },
        );
        retained_home = Some(home);
        result
    };
    match result {
        Ok(()) => {
            if let Some(home) = retained_home {
                execution.slots.push(&mut frame.window, home)?;
            }
            frame.resume_pc = frame
                .fault_pc
                .checked_add(1)
                .ok_or_else(|| Error::internal("private initialization resume PC overflow"))?;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_instruction(depth);
            Ok(None)
        }
        Err(error) => {
            let Some(kind) =
                crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
            else {
                return Err(error);
            };
            Ok(Some(super::Completion::Throw(
                runtime
                    .new_native_error_from_error(frame.executable.realm, kind, &error)
                    .map_err(runtime_error_to_vm_error)?,
            )))
        }
    }
}

use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariable, ClosureVariableName,
};
use crate::engine::heap::RawValue;
use crate::engine::object::PrivateNameRef;

pub(in crate::engine::vm) fn validate_descriptor(
    descriptor: ClosureVariable,
) -> Result<ClosureVariableKind, Error> {
    if !matches!(
        descriptor.kind,
        ClosureVariableKind::PrivateField
            | ClosureVariableKind::PrivateMethod
            | ClosureVariableKind::PrivateGetter
            | ClosureVariableKind::PrivateSetter
            | ClosureVariableKind::PrivateGetterSetter
    ) || !descriptor.is_lexical
        || !descriptor.is_const
        || !matches!(descriptor.name, ClosureVariableName::Atom(_))
        || !matches!(
            descriptor.source,
            ClosureSource::ParentLocal(_)
                | ClosureSource::ParentClosure(_)
                | ClosureSource::EvalEnvironment(_)
        )
    {
        return Err(Error::internal(
            "private-name opcode referenced a non-private closure descriptor",
        ));
    }
    Ok(descriptor.kind)
}

fn captured_name(
    runtime: &Runtime,
    root: &impl crate::engine::heap::roots::VarRefHandle,
) -> Result<Option<PrivateNameRef>, Error> {
    match runtime
        .raw_var_ref_value(&root)
        .map_err(runtime_error_to_vm_error)?
    {
        RawValue::Uninitialized => Ok(None),
        RawValue::Private(_) => runtime
            .private_name_from_raw_var_ref(&root)
            .map(Some)
            .map_err(runtime_error_to_vm_error),
        _ => Err(Error::internal(
            "private-name VarRef contains an ordinary value",
        )),
    }
}

pub(in crate::engine::vm) enum PrivateSource<'a> {
    Local(VariableDefinition, &'a FrameBinding),
    Closure(ClosureVariable, crate::engine::heap::roots::VarRefView<'a>),
}

pub(in crate::engine::vm) fn optional_field_name(
    runtime: &Runtime,
    source: PrivateSource<'_>,
) -> Result<Option<PrivateNameRef>, Error> {
    match source {
        PrivateSource::Local(definition, binding) => {
            if validate_definition(definition)?.1 != ClosureVariableKind::PrivateField {
                return Err(Error::internal(
                    "private-field operation referenced a non-field local",
                ));
            }
            match binding {
                FrameBinding::Private(name) => Ok(Some(name.clone())),
                FrameBinding::Captured(root) => captured_name(runtime, &root),
                FrameBinding::Uninitialized => Ok(None),
                FrameBinding::PrivateCallable(_) => Err(Error::internal(
                    "private-field local contains a private method",
                )),
                FrameBinding::Direct(_) => Err(Error::internal(
                    "private-name local contains an ordinary frame value",
                )),
            }
        }
        PrivateSource::Closure(descriptor, root) => {
            if validate_descriptor(descriptor)? != ClosureVariableKind::PrivateField {
                return Err(Error::internal(
                    "private-field operation referenced a non-field closure",
                ));
            }
            runtime
                .validate_var_ref_metadata(&root, descriptor)
                .map_err(runtime_error_to_vm_error)?;
            captured_name(runtime, &root)
        }
    }
}

use crate::engine::object::{CallableRef, ObjectRef};
fn captured_callable(
    runtime: &Runtime,
    root: &impl crate::engine::heap::roots::VarRefHandle,
    kind: ClosureVariableKind,
) -> Result<Option<CallableRef>, Error> {
    match runtime
        .raw_var_ref_value(&root)
        .map_err(runtime_error_to_vm_error)?
    {
        RawValue::Uninitialized => Ok(None),
        RawValue::Object(_) => runtime
            .private_callable_from_raw_var_ref(&root, kind)
            .map(Some)
            .map_err(runtime_error_to_vm_error),
        _ => Err(Error::internal(
            "private-callable VarRef contains an incompatible value",
        )),
    }
}

pub(in crate::engine::vm) fn optional_callable(
    runtime: &Runtime,
    source: PrivateSource<'_>,
    expected_kind: ClosureVariableKind,
) -> Result<Option<CallableRef>, Error> {
    match source {
        PrivateSource::Local(definition, binding) => {
            let kind = validate_definition(definition)?.1;
            if kind != expected_kind || !super::bindings::is_private_callable_kind(kind) {
                return Err(Error::internal(
                    "private-callable operation referenced an incompatible local",
                ));
            }
            match binding {
                FrameBinding::PrivateCallable(callable) => Ok(Some(callable.clone())),
                FrameBinding::Captured(root) => captured_callable(runtime, &root, expected_kind),
                FrameBinding::Uninitialized => Ok(None),
                FrameBinding::Private(_) => Err(Error::internal(
                    "private-callable local contains a private field identity",
                )),
                FrameBinding::Direct(_) => Err(Error::internal(
                    "private-callable local contains an ordinary frame value",
                )),
            }
        }
        PrivateSource::Closure(descriptor, root) => {
            let kind = validate_descriptor(descriptor)?;
            if kind != expected_kind || !super::bindings::is_private_callable_kind(kind) {
                return Err(Error::internal(
                    "private-callable operation referenced an incompatible closure",
                ));
            }
            runtime
                .validate_var_ref_metadata(&root, descriptor)
                .map_err(runtime_error_to_vm_error)?;
            captured_callable(runtime, &root, expected_kind)
        }
    }
}

pub(in crate::engine::vm) fn branded_receiver(
    runtime: &Runtime,
    callable: &CallableRef,
    kind: ClosureVariableKind,
    base: Value,
) -> Result<ObjectRef, Error> {
    use crate::engine::api::error::ErrorKind;
    // Resolve HomeObject's brand before validating the receiver, as QuickJS does.
    runtime
        .require_private_method_brand(callable, kind)
        .map_err(runtime_error_to_vm_error)?;
    let Value::Object(receiver) = base else {
        return Err(Error::new(ErrorKind::Type, "not an object"));
    };
    if !runtime
        .check_private_method_brand(callable, &receiver, kind)
        .map_err(runtime_error_to_vm_error)?
    {
        return Err(Error::new(ErrorKind::Type, "invalid brand on object"));
    }
    Ok(receiver)
}
