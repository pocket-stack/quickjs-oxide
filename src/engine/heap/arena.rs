use super::*;

impl Heap {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            #[cfg(not(feature = "profiling"))]
            slots: Vec::new(),
            #[cfg(feature = "profiling")]
            slots: profiling::ArenaStorage::new(),
            free: Vec::new(),
            zero_queue: VecDeque::new(),
            weak_head: None,
            weak_tail: None,
        }
    }

    /// Strong count for diagnostics.  A zombie remains queryable until all
    /// candidate incoming edges have been detached.
    pub fn object_strong_count(&self, id: ObjectId) -> Result<u32, HeapError> {
        self.strong_count(RawId::Object(id))
    }

    /// Strong count for diagnostics.
    pub fn shape_strong_count(&self, id: ShapeId) -> Result<u32, HeapError> {
        self.strong_count(RawId::Shape(id))
    }

    /// Strong count for captured-variable diagnostics.
    pub fn var_ref_strong_count(&self, id: VarRefId) -> Result<u32, HeapError> {
        self.strong_count(RawId::VarRef(id))
    }

    /// Strong count for context diagnostics.
    pub fn context_strong_count(&self, id: ContextId) -> Result<u32, HeapError> {
        self.strong_count(RawId::Context(id))
    }

    /// Strong count for function-bytecode diagnostics.
    #[cfg(test)]
    pub fn function_bytecode_strong_count(&self, id: FunctionBytecodeId) -> Result<u32, HeapError> {
        self.strong_count(RawId::FunctionBytecode(id))
    }

    /// Snapshot aggregate arena counts for tests and runtime diagnostics.
    #[must_use]
    pub fn counts(&self) -> HeapCounts {
        let mut counts = HeapCounts::default();
        for slot in &self.slots {
            match &slot.state {
                SlotState::Initializing { kind, .. } => {
                    counts.initializing = counts.initializing.saturating_add(1);
                    increment_kind_count(&mut counts, *kind);
                }
                SlotState::Live(node) => {
                    counts.live = counts.live.saturating_add(1);
                    increment_kind_count(&mut counts, node.data.kind());
                }
                SlotState::ZeroQueued(node) => {
                    counts.zero_queued = counts.zero_queued.saturating_add(1);
                    increment_kind_count(&mut counts, node.data.kind());
                }
                SlotState::Finalizing(node) => {
                    counts.finalizing = counts.finalizing.saturating_add(1);
                    increment_kind_count(&mut counts, node.data.kind());
                }
                SlotState::Zombie { kind, .. } => {
                    counts.zombies = counts.zombies.saturating_add(1);
                    increment_kind_count(&mut counts, *kind);
                }
                SlotState::Vacant => counts.vacant = counts.vacant.saturating_add(1),
                SlotState::Retired => counts.retired = counts.retired.saturating_add(1),
            }
        }
        counts
    }

    pub(in crate::engine::heap) fn reserve(
        &mut self,
        kind: HeapNodeKind,
    ) -> Result<(u32, u32), HeapError> {
        let index = if let Some(index) = self.free.pop() {
            index
        } else {
            let index = u32::try_from(self.slots.len()).map_err(|_| HeapError::Overflow {
                operation: "allocating an arena slot",
            })?;
            self.slots.push(ArenaSlot {
                generation: 1,
                state: SlotState::Vacant,
                weak_prev: None,
                weak_next: None,
            });
            index
        };
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(HeapError::Invariant("free list referenced a missing slot"))?;
        if !matches!(slot.state, SlotState::Vacant) {
            return Err(HeapError::Invariant(
                "free list referenced an occupied slot",
            ));
        }
        if slot.weak_prev.is_some() || slot.weak_next.is_some() {
            return Err(HeapError::Invariant(
                "free list referenced a linked weak-collection slot",
            ));
        }
        slot.state = SlotState::Initializing { kind, strong: 1 };
        Ok((index, slot.generation))
    }

    pub(in crate::engine::heap) fn abort_initializing(
        &mut self,
        index: u32,
    ) -> Result<(), HeapError> {
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(HeapError::Invariant("initializing slot disappeared"))?;
        if !matches!(slot.state, SlotState::Initializing { .. }) {
            return Err(HeapError::Invariant(
                "attempted to abort a published arena slot",
            ));
        }
        slot.state = SlotState::Vacant;
        self.free.push(index);
        Ok(())
    }

    pub(in crate::engine::heap) fn publish(
        &mut self,
        index: u32,
        data: NodeData,
    ) -> Result<(), HeapError> {
        let expected = data.kind();
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(HeapError::Invariant("initializing slot disappeared"))?;
        let (kind, strong) = match &slot.state {
            SlotState::Initializing { kind, strong } => (*kind, *strong),
            _ => {
                return Err(HeapError::Invariant(
                    "attempted to publish a non-initializing slot",
                ));
            }
        };
        if kind != expected || strong != 1 {
            return Err(HeapError::Invariant(
                "initializing slot metadata did not match its payload",
            ));
        }
        slot.state = SlotState::Live(Node { strong, data });
        Ok(())
    }

    pub(in crate::engine::heap) fn live_index(&self, id: RawId) -> Result<usize, HeapError> {
        let index = self.validate_slot_identity(id)?;
        if !matches!(self.slots[index].state, SlotState::Live(_)) {
            return Err(HeapError::Invariant(
                "heap edge targeted a node outside Live state",
            ));
        }
        Ok(index)
    }

    pub(in crate::engine::heap) fn live_node(&self, id: RawId) -> Result<&Node, HeapError> {
        let index = self.validate_slot_identity(id)?;
        match &self.slots[index].state {
            SlotState::Live(node) => Ok(node),
            _ => Err(HeapError::Stale {
                index: id.index(),
                generation: id.generation(),
            }),
        }
    }

    pub(in crate::engine::heap) fn live_node_mut(
        &mut self,
        id: RawId,
    ) -> Result<&mut Node, HeapError> {
        let index = self.validate_slot_identity(id)?;
        match &mut self.slots[index].state {
            SlotState::Live(node) => Ok(node),
            _ => Err(HeapError::Stale {
                index: id.index(),
                generation: id.generation(),
            }),
        }
    }

    pub(in crate::engine::heap) fn object_mut(
        &mut self,
        id: ObjectId,
    ) -> Result<&mut ObjectData, HeapError> {
        match &mut self.live_node_mut(RawId::Object(id))?.data {
            NodeData::Object(object) => Ok(object),
            NodeData::Shape(_)
            | NodeData::VarRef(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed object lookup reached another node payload",
            )),
        }
    }

    pub(in crate::engine::heap) fn var_ref_mut(
        &mut self,
        id: VarRefId,
    ) -> Result<&mut VarRefData, HeapError> {
        match &mut self.live_node_mut(RawId::VarRef(id))?.data {
            NodeData::VarRef(var_ref) => Ok(var_ref),
            NodeData::Object(_)
            | NodeData::Shape(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed var-ref lookup reached another node payload",
            )),
        }
    }

    pub(in crate::engine::heap) fn strong_count(&self, id: RawId) -> Result<u32, HeapError> {
        let index = self.validate_slot_identity(id)?;
        self.slots[index].state.strong().ok_or(HeapError::Stale {
            index: id.index(),
            generation: id.generation(),
        })
    }

    pub(in crate::engine::heap) fn is_live(&self, id: RawId) -> bool {
        self.validate_slot_identity(id)
            .is_ok_and(|index| matches!(self.slots[index].state, SlotState::Live(_)))
    }
}
