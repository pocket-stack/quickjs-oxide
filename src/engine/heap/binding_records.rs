use super::*;

/// Mutable storage shared by an active frame and all closures capturing one
/// argument or local.
///
/// `is_lexical` and `is_const` mirror the metadata carried by QuickJS
/// `JSVarRef`.  Enforcement belongs to the VM; the heap owns and traces the
/// current value.  The root returned by [`Heap::allocate_var_ref`] is intended
/// to be the active frame's ownership.  Function-object closure slots retain
/// the same identity and therefore keep the cell alive after frame teardown.
#[derive(Clone, Debug, PartialEq)]
pub struct VarRefData {
    pub value: RawValue,
    pub is_lexical: bool,
    pub is_const: bool,
    pub kind: ClosureVariableKind,
}

impl VarRefData {
    /// Construct a captured argument/local cell.
    #[must_use]
    #[cfg(test)]
    pub const fn local(value: RawValue) -> Self {
        Self {
            value,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }
    }

    /// Construct a cell from one compiler-produced closure descriptor.
    #[must_use]
    pub const fn captured(
        value: RawValue,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Self {
        Self {
            value,
            is_lexical,
            is_const,
            kind,
        }
    }
}

pub(in crate::engine::heap) fn validate_var_ref_payload(
    var_ref: &VarRefData,
) -> Result<(), HeapError> {
    validate_var_ref_value(
        var_ref.kind,
        var_ref.is_lexical,
        var_ref.is_const,
        &var_ref.value,
    )
}

pub(in crate::engine::heap) fn validate_var_ref_value(
    kind: ClosureVariableKind,
    is_lexical: bool,
    is_const: bool,
    value: &RawValue,
) -> Result<(), HeapError> {
    if kind == ClosureVariableKind::ModuleImportView {
        return Err(HeapError::Invariant(
            "module-import view escaped into a VarRef cell",
        ));
    }
    if kind.is_private() && (!is_lexical || !is_const) {
        return Err(HeapError::Invariant(
            "private-element VarRef is not an immutable lexical binding",
        ));
    }
    match kind {
        ClosureVariableKind::PrivateField
            if !matches!(value, RawValue::Private(_) | RawValue::Uninitialized) =>
        {
            return Err(HeapError::Invariant(
                "private-name VarRef contains an ordinary ECMAScript value",
            ));
        }
        ClosureVariableKind::PrivateMethod
        | ClosureVariableKind::PrivateGetter
        | ClosureVariableKind::PrivateSetter
        | ClosureVariableKind::PrivateGetterSetter
            if !matches!(value, RawValue::Object(_) | RawValue::Uninitialized) =>
        {
            return Err(HeapError::Invariant(
                "private-method VarRef contains a non-callable representation",
            ));
        }
        _ => {}
    }
    if !kind.is_private() && matches!(value, RawValue::Private(_)) {
        return Err(HeapError::Invariant(
            "private-name identity escaped into an ordinary VarRef",
        ));
    }
    Ok(())
}
