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
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed var-ref lookup reached another node payload",
            )),
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
            | NodeData::Context(_) => Err(HeapError::Invariant(
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
