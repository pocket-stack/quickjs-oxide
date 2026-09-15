//! Running binding ownership and shared-cell lifetime rules.
//!
//! Direct values, private identities, TDZ and captured cells remain distinct.
//! Capturing installs the rooted cell before the direct owner is released;
//! closing roots the detached value before dropping the frame's cell handle.
//! Long-lived suspension storage must encode these owners as managed raw edges.

use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::code::function::metadata::{ClosureVariable, ClosureVariableKind};
use crate::engine::heap::RawValue;
use crate::engine::heap::roots::VarRefRoot;
use crate::engine::object::{CallableRef, PrivateNameRef};
use crate::engine::value::Value;
use crate::engine::vm::exception::runtime_error_to_vm_error;

pub(in crate::engine::vm) enum FrameBinding {
    Direct(Value),
    Private(PrivateNameRef),
    PrivateCallable(CallableRef),
    Uninitialized,
    Captured(VarRefRoot),
}

pub(in crate::engine::vm) const fn is_private_callable_kind(kind: ClosureVariableKind) -> bool {
    matches!(
        kind,
        ClosureVariableKind::PrivateMethod
            | ClosureVariableKind::PrivateGetter
            | ClosureVariableKind::PrivateSetter
            | ClosureVariableKind::PrivateGetterSetter
    )
}

/// Read a freshly authenticated shared cell without creating any owner or
/// operation boundary. Pending releases must take the canonical path because
/// its RuntimeOperation drains them before observing the cell.
#[cfg(feature = "stack-vm")]
#[inline]
pub(in crate::engine::vm) fn read_immediate_cell(
    runtime: &Runtime,
    root: &impl crate::engine::heap::roots::VarRefHandle,
) -> Option<Value> {
    if !root.belongs_to(runtime) || runtime.0.deferred_references.has_pending() {
        return None;
    }
    let state = runtime.0.state.try_borrow().ok()?;
    let cell = state.heap.var_ref(root.id()).ok()?;
    if cell.kind.is_private() {
        return None;
    }
    match &cell.value {
        RawValue::Undefined => Some(Value::Undefined),
        RawValue::Null => Some(Value::Null),
        RawValue::Bool(value) => Some(Value::Bool(*value)),
        RawValue::Int(value) => Some(Value::Int(*value)),
        RawValue::Float(value) => Some(Value::Float(*value)),
        _ => None,
    }
}

/// Keep the scalar read cheap; only a non-immediate miss attempts an owned
/// read under the shared heap guard. The flag distinguishes profiling events.
#[cfg(feature = "stack-vm")]
#[inline]
pub(in crate::engine::vm) fn read_run_cell(
    runtime: &Runtime,
    root: &impl crate::engine::heap::roots::VarRefHandle,
) -> Result<Option<(Value, bool)>, Error> {
    if let Some(value) = read_immediate_cell(runtime, root) {
        return Ok(Some((value, false)));
    }
    runtime
        .try_read_owned_var_ref(root)
        .map(|value| value.map(|value| (value, true)))
        .map_err(runtime_error_to_vm_error)
}

/// Commit only a no-owner immediate replacement. The caller first proves its
/// operand exists; no stack/heap mutation can occur between that peek and the
/// successful write, so consuming that same immediate operand cannot fail.
#[cfg(feature = "stack-vm")]
#[inline]
pub(in crate::engine::vm) fn try_write_immediate_cell(
    runtime: &Runtime,
    root: &impl crate::engine::heap::roots::VarRefHandle,
    value: &Value,
    expected: Option<(bool, bool, ClosureVariableKind)>,
) -> bool {
    let replacement = match value {
        Value::Undefined => RawValue::Undefined,
        Value::Null => RawValue::Null,
        Value::Bool(value) => RawValue::Bool(*value),
        Value::Int(value) => RawValue::Int(*value),
        Value::Float(value) => RawValue::Float(*value),
        _ => return false,
    };
    if !root.belongs_to(runtime) || runtime.0.deferred_references.has_pending() {
        return false;
    }
    let Ok(mut state) = runtime.0.state.try_borrow_mut() else {
        return false;
    };
    state
        .heap
        .try_replace_immediate_var_ref_value(root.id(), replacement, expected)
}

/// QuickJS keeps access flags on each closure descriptor rather than on the
/// shared VarRef. Its ordinary direct-eval prepass may therefore expose one
/// FunctionName cell through a mutable Normal descriptor. A module import is
/// likewise an immutable lexical view of the exporter's original mutable or
/// immutable ordinary cell, and nested closures/eval relay that immutable
/// view after the original `ModuleImport` source tag is no longer present.
/// Publication authenticates where these view-only metadata differences enter
/// the closure chain.
pub(crate) fn closure_view_matches_cell(
    cell: (bool, bool, ClosureVariableKind),
    descriptor: ClosureVariable,
) -> bool {
    cell == (descriptor.is_lexical, descriptor.is_const, descriptor.kind)
        || (descriptor.is_lexical
            && descriptor.is_const
            && descriptor.kind == ClosureVariableKind::ModuleImportView
            && cell.2 == ClosureVariableKind::Normal)
        || (cell.0 == descriptor.is_lexical
            && !cell.0
            && cell.2 == ClosureVariableKind::FunctionName
            && !descriptor.is_const
            && descriptor.kind == ClosureVariableKind::Normal)
}

#[inline]
pub(in crate::engine::vm) fn read_frame_binding(
    runtime: &Runtime,
    binding: &FrameBinding,
) -> Result<Value, Error> {
    match binding {
        FrameBinding::Direct(value) => Ok(value.clone()),
        FrameBinding::Private(_) | FrameBinding::PrivateCallable(_) => Err(Error::internal(
            "ordinary local read reached a private-element binding",
        )),
        FrameBinding::Uninitialized => Err(Error::internal(
            "unchecked local read reached an uninitialized lexical binding",
        )),
        FrameBinding::Captured(root) => runtime
            .read_var_ref(root)
            .map_err(|error| Error::internal(error.to_string())),
    }
}

#[inline]
pub(in crate::engine::vm) fn write_frame_binding(
    runtime: &Runtime,
    binding: &mut FrameBinding,
    value: Value,
) -> Result<(), Error> {
    match binding {
        FrameBinding::Direct(slot) => {
            *slot = value;
            Ok(())
        }
        FrameBinding::Private(_) | FrameBinding::PrivateCallable(_) => Err(Error::internal(
            "ordinary local write reached a private-element binding",
        )),
        FrameBinding::Uninitialized => Err(Error::internal(
            "unchecked local write reached an uninitialized lexical binding",
        )),
        FrameBinding::Captured(root) => runtime
            .write_var_ref(root, value)
            .map_err(|error| Error::internal(error.to_string())),
    }
}

pub(in crate::engine::vm) fn capture_frame_binding(
    runtime: &Runtime,
    binding: &mut FrameBinding,
    descriptor: ClosureVariable,
) -> Result<VarRefRoot, Error> {
    match binding {
        FrameBinding::Direct(value) => {
            if descriptor.kind.is_private() {
                return Err(Error::internal(
                    "private-name capture reached an ordinary frame value",
                ));
            }
            let root = runtime
                .new_var_ref(
                    value.clone(),
                    descriptor.is_lexical,
                    descriptor.is_const,
                    descriptor.kind,
                )
                .map_err(|error| Error::internal(error.to_string()))?;
            *binding = FrameBinding::Captured(root.clone());
            Ok(root)
        }
        FrameBinding::Private(name) => {
            if descriptor.kind != ClosureVariableKind::PrivateField
                || !descriptor.is_lexical
                || !descriptor.is_const
            {
                return Err(Error::internal(
                    "private-field frame cell used an incompatible closure descriptor",
                ));
            }
            let root = runtime
                .new_private_var_ref(name)
                .map_err(|error| Error::internal(error.to_string()))?;
            *binding = FrameBinding::Captured(root.clone());
            Ok(root)
        }
        FrameBinding::PrivateCallable(callable) => {
            if !is_private_callable_kind(descriptor.kind)
                || !descriptor.is_lexical
                || !descriptor.is_const
            {
                return Err(Error::internal(
                    "private-callable frame cell used an incompatible closure descriptor",
                ));
            }
            let root = runtime
                .new_private_callable_var_ref(callable, descriptor.kind)
                .map_err(|error| Error::internal(error.to_string()))?;
            *binding = FrameBinding::Captured(root.clone());
            Ok(root)
        }
        FrameBinding::Uninitialized => {
            let root = runtime
                .new_uninitialized_captured_var_ref(
                    descriptor.is_lexical,
                    descriptor.is_const,
                    descriptor.kind,
                )
                .map_err(|error| Error::internal(error.to_string()))?;
            *binding = FrameBinding::Captured(root.clone());
            Ok(root)
        }
        FrameBinding::Captured(root) => reuse_frame_capture(runtime, root, descriptor),
    }
}

/// Reuse a live cell through a publication-authenticated descriptor view.
/// This checks actual cell metadata without redispatching its frame storage.
pub(in crate::engine::vm) fn reuse_frame_capture(
    runtime: &Runtime,
    root: &impl crate::engine::heap::roots::VarRefHandle,
    descriptor: ClosureVariable,
) -> Result<VarRefRoot, Error> {
    runtime
        .validate_var_ref_metadata(root, descriptor)
        .map_err(|error| Error::internal(error.to_string()))?;
    Ok(root.to_root())
}

pub(in crate::engine::vm) fn close_frame_binding(
    runtime: &Runtime,
    binding: &mut FrameBinding,
    kind: ClosureVariableKind,
) -> Result<(), Error> {
    let FrameBinding::Captured(root) = binding else {
        return Ok(());
    };
    let raw = runtime
        .raw_var_ref_value(root)
        .map_err(|error| Error::internal(error.to_string()))?;
    let detached = match raw {
        RawValue::Uninitialized => FrameBinding::Uninitialized,
        RawValue::Private(_) if kind == ClosureVariableKind::PrivateField => FrameBinding::Private(
            runtime
                .private_name_from_raw_var_ref(root)
                .map_err(runtime_error_to_vm_error)?,
        ),
        RawValue::Object(_) if is_private_callable_kind(kind) => FrameBinding::PrivateCallable(
            runtime
                .private_callable_from_raw_var_ref(root, kind)
                .map_err(runtime_error_to_vm_error)?,
        ),
        _ if kind.is_private() => {
            return Err(Error::internal(
                "captured private-element cell contains an incompatible value",
            ));
        }
        raw => FrameBinding::Direct(
            runtime
                .root_raw_value(&raw)
                .map_err(runtime_error_to_vm_error)?,
        ),
    };
    *binding = detached;
    Ok(())
}

/// Validate a derived constructor's explicit return against its lexical this.
/// No user code runs while reading the binding or materializing these errors.
pub(in crate::engine::vm) fn finish_derived_return(
    runtime: &Runtime,
    caller_realm: crate::engine::heap::ContextId,
    definition: crate::engine::code::function::metadata::VariableDefinition,
    binding: Option<&FrameBinding>,
    value: Value,
) -> Result<crate::engine::vm::Completion, Error> {
    use crate::engine::api::error::NativeErrorKind;
    use crate::engine::vm::Completion;
    if !definition.is_lexical
        || definition.is_const
        || definition.kind != ClosureVariableKind::Normal
    {
        return Err(Error::internal(
            "derived return referenced a non-mutable lexical this local",
        ));
    }
    match value {
        value @ Value::Object(_) => Ok(Completion::Return(value)),
        Value::Undefined => {
            let binding = binding.ok_or_else(|| Error::internal("local index is out of bounds"))?;
            let this_value = match binding {
                FrameBinding::Direct(value) => value.clone(),
                FrameBinding::Private(_) | FrameBinding::PrivateCallable(_) => {
                    return Err(Error::internal(
                        "derived this local contains a private-element identity",
                    ));
                }
                FrameBinding::Uninitialized => {
                    return runtime
                        .new_native_error(
                            caller_realm,
                            NativeErrorKind::Reference,
                            "this is not initialized",
                        )
                        .map(Completion::Throw)
                        .map_err(runtime_error_to_vm_error);
                }
                FrameBinding::Captured(root) => {
                    let raw = runtime
                        .raw_var_ref_value(root)
                        .map_err(runtime_error_to_vm_error)?;
                    if matches!(raw, RawValue::Uninitialized) {
                        return runtime
                            .new_native_error(
                                caller_realm,
                                NativeErrorKind::Reference,
                                "this is not initialized",
                            )
                            .map(Completion::Throw)
                            .map_err(runtime_error_to_vm_error);
                    }
                    runtime
                        .root_raw_value(&raw)
                        .map_err(runtime_error_to_vm_error)?
                }
            };
            if !matches!(this_value, Value::Object(_)) {
                return Err(Error::internal(
                    "initialized derived this binding did not contain an Object",
                ));
            }
            Ok(Completion::Return(this_value))
        }
        _ => runtime
            .new_native_error(
                caller_realm,
                NativeErrorKind::Type,
                "derived class constructor must return an object or undefined",
            )
            .map(Completion::Throw)
            .map_err(runtime_error_to_vm_error),
    }
}

/// Return a direct replacement only for a fresh this slot; captured this is
/// updated in place, and a previously initialized binding is never overwritten.
pub(in crate::engine::vm) fn initialize_derived_binding(
    runtime: &Runtime,
    definition: crate::engine::code::function::metadata::VariableDefinition,
    binding: Option<&FrameBinding>,
    value: Value,
) -> Result<Option<FrameBinding>, Error> {
    use crate::engine::api::error::ErrorKind;
    if !definition.is_lexical
        || definition.is_const
        || definition.kind != ClosureVariableKind::Normal
    {
        return Err(Error::internal(
            "derived this initialization referenced a non-mutable lexical local",
        ));
    }
    if !matches!(value, Value::Object(_)) {
        return Err(Error::internal(
            "derived this initialization did not receive an Object",
        ));
    }

    let captured = match binding.ok_or_else(|| Error::internal("local index is out of bounds"))? {
        FrameBinding::Uninitialized => None,
        FrameBinding::Captured(root) => Some(root.clone()),
        FrameBinding::Direct(_) | FrameBinding::Private(_) | FrameBinding::PrivateCallable(_) => {
            return Err(Error::new(
                ErrorKind::Reference,
                "'this' can be initialized only once",
            ));
        }
    };
    if let Some(root) = captured {
        let raw = runtime
            .raw_var_ref_value(&root)
            .map_err(runtime_error_to_vm_error)?;
        if !matches!(raw, RawValue::Uninitialized) {
            return Err(Error::new(
                ErrorKind::Reference,
                "'this' can be initialized only once",
            ));
        }
        return runtime
            .write_var_ref(&root, value)
            .map(|()| None)
            .map_err(runtime_error_to_vm_error);
    }
    Ok(Some(FrameBinding::Direct(value)))
}

/// Shared diagnostic policy for local, closure and dynamic TDZ reads.
pub(in crate::engine::vm) fn lexical_uninitialized_error(
    runtime: &Runtime,
    name: Option<crate::engine::atom::Atom>,
    name_visible: bool,
) -> Result<Error, Error> {
    use crate::engine::api::error::ErrorKind;
    use crate::engine::object::PropertyKey;
    let Some(name) = name else {
        return Ok(Error::new(
            ErrorKind::Reference,
            "lexical variable is not initialized",
        ));
    };
    if !name_visible {
        return Ok(Error::new(
            ErrorKind::Reference,
            "lexical variable is not initialized",
        ));
    }
    // Compiler-only pseudo names must not leak into observable diagnostics.
    // QuickJS stores this identity as JS_ATOM_this and therefore reports
    // `this`, while this typed compiler uses the unspellable `<this>` name
    // to keep it distinct from authored bindings.
    let hidden_this = runtime
        .intern_property_key("<this>")
        .map_err(|error| Error::internal(error.to_string()))?;
    if hidden_this.atom() == name {
        return Ok(Error::new(ErrorKind::Reference, "this is not initialized"));
    }
    let key = PropertyKey::from_borrowed_atom(runtime.clone(), name)
        .map_err(|error| Error::internal(error.to_string()))?;
    runtime
        .native_atom_error(ErrorKind::Reference, "", &key, " is not initialized")
        .map_err(runtime_error_to_vm_error)
}

pub(in crate::engine::vm) fn closure_lexical_uninitialized_error(
    runtime: &Runtime,
    source: crate::engine::code::function::metadata::ClosureSource,
    name: Option<crate::engine::atom::Atom>,
    strip_variable_debug: bool,
) -> Result<Error, Error> {
    use crate::engine::code::function::metadata::ClosureSource;
    let semantic_name = matches!(
        source,
        ClosureSource::GlobalDeclaration
            | ClosureSource::Global
            | ClosureSource::ParentGlobal(_)
            | ClosureSource::ModuleDeclaration
            | ClosureSource::ModuleImport
            | ClosureSource::ModuleImportCollision
            | ClosureSource::ModuleImportMeta
    );
    lexical_uninitialized_error(runtime, name, semantic_name || !strip_variable_debug)
}

pub(in crate::engine::vm) fn lexical_read_only_error(
    runtime: &Runtime,
    name: Option<crate::engine::atom::Atom>,
) -> Result<Error, Error> {
    use crate::engine::api::error::ErrorKind;
    use crate::engine::object::PropertyKey;
    let Some(name) = name else {
        return Ok(Error::new(ErrorKind::Type, "lexical variable is read-only"));
    };
    let key = PropertyKey::from_borrowed_atom(runtime.clone(), name)
        .map_err(|error| Error::internal(error.to_string()))?;
    runtime
        .native_atom_error(ErrorKind::Type, "'", &key, "' is read-only")
        .map_err(runtime_error_to_vm_error)
}

pub(in crate::engine::vm) fn closure_name(
    descriptor: ClosureVariable,
) -> Result<Option<crate::engine::atom::Atom>, Error> {
    use crate::engine::code::function::metadata::ClosureVariableName;
    match descriptor.name {
        ClosureVariableName::Atom(name) => Ok(Some(name)),
        ClosureVariableName::None => Ok(None),
        ClosureVariableName::Constant(_) => Err(Error::internal(
            "published closure descriptor retained an unlinked name constant",
        )),
    }
}

pub(in crate::engine::vm) fn read_checked_closure(
    runtime: &Runtime,
    root: &impl crate::engine::heap::roots::VarRefHandle,
    descriptor: ClosureVariable,
    strip_variable_debug: bool,
) -> Result<Value, Error> {
    let raw = runtime
        .raw_var_ref_value(root)
        .map_err(runtime_error_to_vm_error)?;
    if matches!(raw, RawValue::Uninitialized) {
        return Err(closure_lexical_uninitialized_error(
            runtime,
            descriptor.source,
            closure_name(descriptor)?,
            strip_variable_debug,
        )?);
    }
    runtime
        .root_raw_value(&raw)
        .map_err(runtime_error_to_vm_error)
}

pub(in crate::engine::vm) fn write_checked_closure(
    runtime: &Runtime,
    root: &impl crate::engine::heap::roots::VarRefHandle,
    descriptor: ClosureVariable,
    strip_variable_debug: bool,
    value: Value,
) -> Result<(), Error> {
    let (uninitialized, is_const) = {
        let state = runtime.0.state.borrow();
        let cell = state
            .heap
            .var_ref(root.id())
            .map_err(|error| Error::internal(error.to_string()))?;
        (matches!(cell.value, RawValue::Uninitialized), cell.is_const)
    };
    if uninitialized {
        return Err(closure_lexical_uninitialized_error(
            runtime,
            descriptor.source,
            closure_name(descriptor)?,
            strip_variable_debug,
        )?);
    }
    if is_const {
        return Err(lexical_read_only_error(runtime, closure_name(descriptor)?)?);
    }
    runtime
        .write_var_ref(root, value)
        .map_err(runtime_error_to_vm_error)
}

/// New local cells take the parent's canonical metadata; existing cells validate
/// the child's authenticated view without changing the cell's identity.
pub(in crate::engine::vm) fn capture_local_binding(
    runtime: &Runtime,
    binding: &mut FrameBinding,
    definition: crate::engine::code::function::metadata::VariableDefinition,
    descriptor: ClosureVariable,
) -> Result<VarRefRoot, Error> {
    if let FrameBinding::Captured(root) = binding {
        reuse_frame_capture(runtime, root, descriptor)
    } else {
        capture_frame_binding(
            runtime,
            binding,
            ClosureVariable {
                is_lexical: definition.is_lexical,
                is_const: definition.is_const,
                kind: definition.kind,
                ..descriptor
            },
        )
    }
}

/// Preserve the initial TDZ cell or reset the same cell after an abrupt scope
/// exit marked it reusable; normal iteration must detach it with CloseLocal.
pub(in crate::engine::vm) fn reset_captured_binding(
    runtime: &Runtime,
    root: &impl crate::engine::heap::roots::VarRefHandle,
    reusable: bool,
) -> Result<(), Error> {
    let raw = runtime
        .raw_var_ref_value(root)
        .map_err(runtime_error_to_vm_error)?;
    if matches!(raw, RawValue::Uninitialized) {
        // QuickJS creates direct FunctionBody declaration closures
        // before expanding the body scope's lexical TDZ entries. A
        // child may therefore capture this first uninitialized cell
        // before SetLocalUninitialized reaches it; entering that same
        // initial lifetime is a no-op. A live initialized capture still
        // proves that a later lifetime skipped CloseLocal.
        return Ok(());
    }
    if reusable {
        // QuickJS resets the existing VarRef in place when an abrupt
        // completion skipped CloseLocal. Escaped closures therefore
        // observe the next lifetime initialized at this same scope
        // site, including its next private field/method identity.
        runtime
            .reset_var_ref_uninitialized(root)
            .map_err(runtime_error_to_vm_error)?;
        return Ok(());
    }
    return Err(Error::internal(
        "captured local entered a new lexical lifetime before CloseLocal",
    ));
}

/// Initialize a published lexical/with binding while preserving captured cells.
pub(in crate::engine::vm) fn initialize_local_binding(
    runtime: &Runtime,
    kind: ClosureVariableKind,
    binding: &mut FrameBinding,
    value: Value,
) -> Result<(), Error> {
    if kind == ClosureVariableKind::WithObject {
        let Value::Object(object) = &value else {
            return Err(Error::internal(
                "with-object initialization did not receive an Object",
            ));
        };
        if !object.belongs_to(runtime) {
            return Err(Error::internal(
                "with-object initialization received a cross-runtime Object",
            ));
        }
    }
    match binding {
        FrameBinding::Direct(slot) => {
            *slot = value;
            Ok(())
        }
        FrameBinding::Private(_) | FrameBinding::PrivateCallable(_) => Err(Error::internal(
            "ordinary lexical initialization reached a private-element frame cell",
        )),
        FrameBinding::Uninitialized => {
            *binding = FrameBinding::Direct(value);
            Ok(())
        }
        FrameBinding::Captured(root) => runtime
            .write_var_ref(root, value)
            .map_err(runtime_error_to_vm_error),
    }
}

pub(in crate::engine::vm) fn initialize_derived_closure(
    runtime: &Runtime,
    root: &impl crate::engine::heap::roots::VarRefHandle,
    descriptor: ClosureVariable,
    value: Value,
) -> Result<(), Error> {
    use crate::engine::api::error::ErrorKind;
    if !descriptor.is_lexical
        || descriptor.is_const
        || descriptor.kind != ClosureVariableKind::Normal
    {
        return Err(Error::internal(
            "derived this initialization referenced a non-mutable lexical closure",
        ));
    }
    if !matches!(value, Value::Object(_)) {
        return Err(Error::internal(
            "derived this initialization did not receive an Object",
        ));
    }
    let raw = runtime
        .raw_var_ref_value(root)
        .map_err(runtime_error_to_vm_error)?;
    if !matches!(raw, RawValue::Uninitialized) {
        // Pinned QuickJS's captured form (`put_var_ref_check_init`) uses
        // the ordinary uninitialized-binding diagnostic here. This
        // intentionally differs from the owning-local opcode's explicit
        // "initialized only once" message.
        return Err(Error::new(ErrorKind::Reference, "this is not initialized"));
    }
    runtime
        .write_var_ref(root, value)
        .map_err(runtime_error_to_vm_error)
}

/// Preserve the dedicated import-collision authority at both VM consumers.
pub(super) fn validate_module_import_collision(descriptor: ClosureVariable) -> Result<(), Error> {
    if descriptor.source
        != crate::engine::code::function::metadata::ClosureSource::ModuleImportCollision
        || !descriptor.is_lexical
        || !descriptor.is_const
        || !matches!(
            descriptor.kind,
            ClosureVariableKind::Normal | ClosureVariableKind::ModuleImportView
        )
    {
        return Err(Error::internal(
            "module import collision initialization targeted a non-import binding",
        ));
    }
    Ok(())
}

#[cfg(all(test, feature = "stack-vm"))]
mod immediate_cell_tests {
    use super::*;

    #[test]
    #[cfg(feature = "profiling")]
    fn owned_cell_reads_keep_global_and_captured_function_identity() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context
            .eval("let ownedCellGlobal = function() { return 7; };")
            .unwrap();
        let profile = crate::engine::api::profiling::CostProfile::start();
        assert_eq!(
            context
                .eval(
                    r#"
            (() => {
                let f = ownedCellGlobal;
                function get() { return f; }
                if (get() !== ownedCellGlobal) return false;
                f = function() { return 9; };
                return get()() === 9 && ownedCellGlobal() === 7;
            })()
        "#
                )
                .unwrap(),
            Value::Bool(true)
        );
        let cost = profile.snapshot();
        for name in ["global_owned_cell_read", "captured_owned_cell_read"] {
            assert!(
                cost.owned_execution_events.get(name).copied().unwrap_or(0) > 0,
                "{name}"
            );
        }
    }

    #[test]
    fn immediate_cell_writes_commit_only_mutable_initialized_owners() {
        let runtime = Runtime::new();
        let root = runtime
            .new_var_ref(Value::Int(1), true, false, ClosureVariableKind::Normal)
            .unwrap();
        let metadata = Some((true, false, ClosureVariableKind::Normal));
        assert!(try_write_immediate_cell(
            &runtime,
            &root,
            &Value::Int(2),
            metadata
        ));
        assert_eq!(runtime.read_var_ref(&root).unwrap(), Value::Int(2));
        assert!(!try_write_immediate_cell(
            &runtime,
            &root,
            &Value::Int(3),
            Some((false, false, ClosureVariableKind::Normal))
        ));
        assert!(!try_write_immediate_cell(
            &Runtime::new(),
            &root,
            &Value::Int(3),
            metadata
        ));
        {
            let _state = runtime.0.state.borrow();
            assert!(!try_write_immediate_cell(
                &runtime,
                &root,
                &Value::Int(3),
                metadata
            ));
        }
        assert!(!try_write_immediate_cell(
            &runtime,
            &root,
            &Value::Object(runtime.new_object(None).unwrap()),
            metadata
        ));
        assert_eq!(runtime.read_var_ref(&root).unwrap(), Value::Int(2));
        assert!(try_write_immediate_cell(
            &runtime,
            &root,
            &Value::Float(-0.0),
            metadata
        ));
        let Value::Float(value) = runtime.read_var_ref(&root).unwrap() else {
            panic!("expected float");
        };
        assert!(value.is_sign_negative());
        runtime.reset_var_ref_uninitialized(&root).unwrap();
        assert!(!try_write_immediate_cell(
            &runtime,
            &root,
            &Value::Int(3),
            metadata
        ));
        runtime
            .write_var_ref(&root, Value::Object(runtime.new_object(None).unwrap()))
            .unwrap();
        assert!(!try_write_immediate_cell(
            &runtime,
            &root,
            &Value::Int(3),
            metadata
        ));
        let constant = runtime
            .new_var_ref(Value::Int(1), true, true, ClosureVariableKind::Normal)
            .unwrap();
        assert!(!try_write_immediate_cell(
            &runtime,
            &constant,
            &Value::Int(3),
            None
        ));
        assert_eq!(runtime.read_var_ref(&constant).unwrap(), Value::Int(1));
    }

    #[test]
    fn immediate_cell_writes_preserve_deferred_release_boundary() {
        let runtime = Runtime::new();
        let root = runtime
            .new_var_ref(Value::Int(1), false, false, ClosureVariableKind::Normal)
            .unwrap();
        let object = runtime.new_object(None).unwrap();
        {
            let _state = runtime.0.state.borrow();
            drop(object);
        }
        assert!(!try_write_immediate_cell(
            &runtime,
            &root,
            &Value::Int(2),
            None
        ));
        assert!(runtime.0.deferred_references.has_pending());
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .var_ref(root.id())
                .unwrap()
                .value,
            RawValue::Int(1)
        );
        runtime.drain_deferred_references().unwrap();
        assert!(try_write_immediate_cell(
            &runtime,
            &root,
            &Value::Int(2),
            None
        ));
    }

    #[test]
    fn immediate_cell_writes_preserve_assignment_results_and_error_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let x=1; function read(){return x;} function set(v){return x=v;}
            if(set(2)!==2 || read()!==2)return false;
            if((x=3)!==3 || read()!==3)return false;
            set({answer:4}); if(read().answer!==4)return false;
            set(5); if(read()!==5)return false;
            set(null); if(read()!==null)return false;
            set(undefined); if(read()!==undefined)return false;
            set(true); if(read()!==true)return false;
            function mapped(arg){function read(){return arg;} arg=7; return arguments[0]===7 && read()===7;}
            function strict(arg){'use strict'; function read(){return arg;} arg=7; return arguments[0]===1 && read()===7;}
            let trace='';
            try { (()=>later=2)(); let later=1; } catch(e){if(e instanceof ReferenceError)trace+='tdz';}
            const c=1; try { (()=>c=2)(); } catch(e){if(e instanceof TypeError)trace+='const';}
            return mapped(1) && strict(1) && trace==='tdzconst' && c===1;
        })()"#).unwrap(), Value::Bool(true));
        context.eval("let immediateWriteGlobal=1;").unwrap();
        assert_eq!(context.eval(r#"(()=>{
            immediateWriteGlobal=2; let a=immediateWriteGlobal;
            immediateWriteGlobal={answer:3}; let b=immediateWriteGlobal.answer;
            immediateWriteGlobal=4; let c=immediateWriteGlobal;
            let calls=0,last=0;
            Object.defineProperty(globalThis,'cellSetterProbe',{configurable:true,set(v){calls++;last=v;}});
            cellSetterProbe=8; cellSetterProbe=9; delete globalThis.cellSetterProbe;
            return a===2 && b===3 && c===4 && calls===2 && last===9;
        })()"#).unwrap(), Value::Bool(true));
    }

    #[test]
    #[cfg(feature = "profiling")]
    fn immediate_cell_write_profiles_prove_both_run_paths() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.eval("let profileWriteGlobal=1;").unwrap();
        let profile = crate::engine::api::profiling::CostProfile::start();
        context
            .eval("profileWriteGlobal=2;profileWriteGlobal=3;")
            .unwrap();
        assert_eq!(
            context
                .eval("(()=>{let x=1;function set(v){x=v;}set(2);set(3);return x;})()")
                .unwrap(),
            Value::Int(3)
        );
        let cost = profile.snapshot();
        assert!(
            cost.owned_execution_events
                .get("global_immediate_cell_write")
                .copied()
                .unwrap_or(0)
                >= 2
        );
        assert!(
            cost.owned_execution_events
                .get("captured_immediate_cell_write")
                .copied()
                .unwrap_or(0)
                >= 2
        );
    }

    #[test]
    fn immediate_cell_reads_are_fresh_and_preserve_fallback_boundaries() {
        let runtime = Runtime::new();
        let root = runtime
            .new_var_ref(Value::Int(1), false, false, ClosureVariableKind::Normal)
            .unwrap();
        assert_eq!(read_immediate_cell(&runtime, &root), Some(Value::Int(1)));
        for value in [
            Value::Null,
            Value::Undefined,
            Value::Bool(true),
            Value::Float(-0.0),
            Value::Int(7),
        ] {
            runtime.write_var_ref(&root, value.clone()).unwrap();
            assert_eq!(read_immediate_cell(&runtime, &root), Some(value));
        }
        let foreign = Runtime::new();
        assert!(read_immediate_cell(&foreign, &root).is_none());
        {
            let _state = runtime.0.state.borrow_mut();
            assert!(read_immediate_cell(&runtime, &root).is_none());
        }
        runtime
            .write_var_ref(&root, Value::Object(runtime.new_object(None).unwrap()))
            .unwrap();
        assert!(read_immediate_cell(&runtime, &root).is_none());
        runtime.reset_var_ref_uninitialized(&root).unwrap();
        assert!(read_immediate_cell(&runtime, &root).is_none());
        let constant = runtime
            .new_var_ref(Value::Int(9), true, true, ClosureVariableKind::Normal)
            .unwrap();
        assert_eq!(
            read_immediate_cell(&runtime, &constant),
            Some(Value::Int(9))
        );
    }

    #[test]
    fn immediate_cell_reads_never_drain_deferred_owners() {
        let runtime = Runtime::new();
        let root = runtime
            .new_var_ref(Value::Int(1), false, false, ClosureVariableKind::Normal)
            .unwrap();
        let object = runtime.new_object(None).unwrap();
        {
            let _state = runtime.0.state.borrow();
            drop(object);
        }
        assert!(runtime.0.deferred_references.has_pending());
        assert!(read_immediate_cell(&runtime, &root).is_none());
        assert!(runtime.0.deferred_references.has_pending());
        runtime.drain_deferred_references().unwrap();
        assert_eq!(read_immediate_cell(&runtime, &root), Some(Value::Int(1)));
    }

    #[test]
    fn captured_reads_observe_callbacks_eval_arguments_and_tdz() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let value=1;
            function read(){return value;}
            function mutate(next){value=next;}
            if(read()!==1)return false;
            mutate(2); if(read()!==2)return false;
            eval('value=3'); if(read()!==3)return false;
            mutate({answer:4}); if(read().answer!==4)return false;
            mutate(-0); if(!Object.is(read(),-0))return false;
            mutate(NaN); if(!Number.isNaN(read()))return false;
            function mapped(arg){function inner(){return arg;} arguments[0]=8; return arg===8 && inner()===8;}
            function strict(arg){'use strict'; function inner(){return arg;} arguments[0]=8; return arg===1 && inner()===1;}
            let tdz=false; try { (()=>later)(); let later=1; } catch(e){tdz=e instanceof ReferenceError;}
            const constant=9; function constantRead(){return constant;}
            return mapped(1) && strict(1) && tdz && constantRead()===9;
        })()"#).unwrap(), Value::Bool(true));
        context.eval("let immediateGlobal=1;").unwrap();
        assert_eq!(context.eval(r#"(()=>{
            let first=immediateGlobal;
            immediateGlobal=2;
            let second=immediateGlobal;
            immediateGlobal={answer:3};
            let third=immediateGlobal.answer;
            let calls=0;
            Object.defineProperty(globalThis,'cellGetterProbe',{configurable:true,get(){calls++;return calls;}});
            let a=cellGetterProbe,b=cellGetterProbe;
            delete globalThis.cellGetterProbe;
            return first===1 && second===2 && third===3 && a===1 && b===2;
        })()"#).unwrap(), Value::Bool(true));
    }

    #[test]
    #[cfg(feature = "profiling")]
    fn captured_immediate_reads_stay_in_the_authenticated_run() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.eval("let immediateProfileGlobal=7;").unwrap();
        let profile = crate::engine::api::profiling::CostProfile::start();
        assert_eq!(
            context.eval("immediateProfileGlobal").unwrap(),
            Value::Int(7)
        );
        assert_eq!(
            context
                .eval("(()=>{let x=2;function get(){return x;}return get()+get();})()")
                .unwrap(),
            Value::Int(4)
        );
        let cost = profile.snapshot();
        assert!(
            cost.owned_execution_events
                .get("global_immediate_cell_read")
                .copied()
                .unwrap_or(0)
                > 0
        );
        assert!(
            cost.owned_execution_events
                .get("captured_immediate_cell_read")
                .copied()
                .unwrap_or(0)
                >= 2
        );
    }
}
