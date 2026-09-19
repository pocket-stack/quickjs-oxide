use super::*;

impl Heap {
    /// Read one captured-variable cell. All functions holding the same
    /// `VarRefId` observe this shared value.
    pub fn var_ref(&self, id: VarRefId) -> Result<&VarRefData, HeapError> {
        match self.live_node(RawId::VarRef(id))?.data {
            NodeData::VarRef(ref var_ref) => Ok(var_ref),
            NodeData::Object(_)
            | NodeData::Shape(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_)
            | NodeData::String(_)
            | NodeData::BigInt(_) => Err(HeapError::Invariant(
                "typed var-ref lookup reached another node payload",
            )),
        }
    }

    /// Trusted shared read for a live `VarRefId` held by an owning root.
    #[inline]
    pub(in crate::engine::heap) fn var_ref_fast(&self, id: VarRefId) -> &VarRefData {
        match &self.live_node_fast(RawId::VarRef(id)).data {
            NodeData::VarRef(var_ref) => var_ref,
            _ => unreachable!("trusted var-ref handle reached another node payload"),
        }
    }

    /// Trusted mutable read for a live `VarRefId` held by an owning root.
    #[inline]
    pub(in crate::engine::heap) fn var_ref_fast_mut(&mut self, id: VarRefId) -> &mut VarRefData {
        match &mut self.live_node_fast_mut(RawId::VarRef(id)).data {
            NodeData::VarRef(var_ref) => var_ref,
            _ => unreachable!("trusted var-ref handle reached another node payload"),
        }
    }

    /// Read immutable executable data without promoting any raw cpool edges.
    pub fn function_bytecode(
        &self,
        id: FunctionBytecodeId,
    ) -> Result<&FunctionBytecodeData, HeapError> {
        match self.live_node(RawId::FunctionBytecode(id))?.data {
            NodeData::FunctionBytecode(ref bytecode) => Ok(bytecode),
            NodeData::Object(_)
            | NodeData::Shape(_)
            | NodeData::VarRef(_)
            | NodeData::Context(_)
            | NodeData::String(_)
            | NodeData::BigInt(_) => Err(HeapError::Invariant(
                "typed bytecode lookup reached another node payload",
            )),
        }
    }

    /// Replace the value stored in a captured-variable cell transactionally.
    ///
    /// The new value's GC edge is retained before the old edge is detached.
    /// A symbol atom transfers to the heap on success; any atom owned by the
    /// previous value is returned in the cleanup.
    pub fn replace_var_ref_value(
        &mut self,
        id: VarRefId,
        replacement: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        let current = self.var_ref(id)?;
        validate_var_ref_value(
            current.kind,
            current.is_lexical,
            current.is_const,
            &replacement,
        )?;
        let new_edges = raw_value_edges(&replacement);
        self.retain_edges_transactionally(&new_edges)?;

        let previous = {
            let var_ref = self.var_ref_mut(id)?;
            std::mem::replace(&mut var_ref.value, replacement)
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&previous));
        for edge in raw_value_edges(&previous) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Restricted equivalent of replacement for a mutable, initialized cell
    /// whose old and new values own no heap/atom/primitive-storage edge.
    /// Declining leaves both the cell and all pending cleanup untouched.
    pub(crate) fn try_replace_immediate_var_ref_value(
        &mut self,
        id: VarRefId,
        replacement: RawValue,
        expected: Option<(bool, bool, ClosureVariableKind)>,
    ) -> bool {
        fn immediate(value: &RawValue) -> bool {
            matches!(
                value,
                RawValue::Undefined
                    | RawValue::Null
                    | RawValue::Bool(_)
                    | RawValue::Int(_)
                    | RawValue::Float(_)
            )
        }
        if !self.zero_queue.is_empty() || !immediate(&replacement) {
            return false;
        }
        // The caller holds an owning VarRef root, so the cell is live; a stale
        // id is a heap invariant violation at this trusted boundary. The
        // replacement is an immediate, so the only `validate_var_ref_value`
        // rejection still reachable here is a module-import view, checked
        // explicitly instead of running the full validator on every write.
        let cell = self.var_ref_fast_mut(id);
        if cell.is_const
            || cell.kind.is_private()
            || cell.kind == ClosureVariableKind::ModuleImportView
            || !immediate(&cell.value)
            || expected
                .is_some_and(|metadata| metadata != (cell.is_lexical, cell.is_const, cell.kind))
        {
            return false;
        }
        // Both edge sets and atom cleanup are empty, and the zero queue was
        // empty before mutation.
        cell.value = replacement;
        true
    }

    /// Update binding-mode metadata without disturbing the shared value or
    /// any of its retained GC edges.
    pub fn set_var_ref_metadata(
        &mut self,
        id: VarRefId,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Result<(), HeapError> {
        let var_ref = self.var_ref_mut(id)?;
        validate_var_ref_value(kind, is_lexical, is_const, &var_ref.value)?;
        var_ref.is_lexical = is_lexical;
        var_ref.is_const = is_const;
        var_ref.kind = kind;
        Ok(())
    }
}

#[cfg(test)]
mod immediate_write_tests {
    use super::*;

    #[test]
    fn immediate_replacement_does_not_drain_an_existing_zero_queue() {
        let runtime = crate::engine::api::runtime::Runtime::new();
        let root = runtime
            .new_var_ref(
                crate::engine::value::Value::Int(1),
                false,
                false,
                ClosureVariableKind::Normal,
            )
            .unwrap();
        let object = runtime.new_object(None).unwrap();
        let id = object.object_id();
        runtime.0.state.borrow_mut().heap.retain_object(id).unwrap();
        drop(object);
        let mut state = runtime.0.state.borrow_mut();
        state.heap.release_raw_no_drain(RawId::Object(id)).unwrap();
        assert!(
            !state
                .heap
                .try_replace_immediate_var_ref_value(root.id(), RawValue::Int(2), None)
        );
        assert_eq!(state.heap.zero_queue.len(), 1);
        assert!(
            matches!(state.heap.var_ref(root.id()).unwrap().value, RawValue::Int(1))
        );
        let cleanup = state.heap.drain_zero_queue().unwrap();
        state.apply_cleanup(cleanup).unwrap();
        assert!(
            state
                .heap
                .try_replace_immediate_var_ref_value(root.id(), RawValue::Int(2), None)
        );
        assert!(
            matches!(state.heap.var_ref(root.id()).unwrap().value, RawValue::Int(2))
        );
    }
}
