//! Shared traversal accounting for canonical BC5 data writers.
//!
//! This state deliberately knows nothing about values, keys, tasks, or output
//! bytes. It only mirrors the parts of QuickJS data writing that must remain
//! consistent across validation and emission: object-reference numbering,
//! active-cycle detection, unique reachable nodes, and aggregate graph limits.

use std::collections::{HashMap, HashSet};

use super::model::{GraphError, GraphLimits, GraphResourceKind, NodeId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum DataNodeWrite {
    /// The emission pass must write this QuickJS object-reference index.
    Reference(u32),
    Traverse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum DataPlanNodeWrite {
    Reference,
    Traverse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum DataWriteStateError {
    Graph(GraphError),
    CircularReference { node: NodeId },
    AllocationFailed,
}

impl From<GraphError> for DataWriteStateError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

enum ObjectReferenceState {
    Disabled,
    /// The planning pass needs the semantic budget but no wire indices.
    Planning {
        emitted: usize,
    },
    /// The emission pass assigns the exact preorder indices written on wire.
    Emitting {
        indices: HashMap<NodeId, u32>,
        next_index: u32,
    },
}

pub(in crate::runtime::binary_object) struct DataWriteState {
    limits: GraphLimits,
    references: ObjectReferenceState,
    active_nodes: HashSet<NodeId>,
    unique_nodes: HashSet<NodeId>,
    total_container_entries: usize,
    total_bigint_bytes: usize,
    total_array_buffer_bytes: usize,
}

/// Capacity reserved for one unique node, but not published until validation.
#[must_use = "commit the unique node after validating it"]
pub(in crate::runtime::binary_object) struct UniqueNodeReservation<'state> {
    unique_nodes: &'state mut HashSet<NodeId>,
    node: NodeId,
}

impl UniqueNodeReservation<'_> {
    pub(in crate::runtime::binary_object) fn commit(self) {
        let inserted = self.unique_nodes.insert(self.node);
        debug_assert!(inserted);
    }
}

impl DataWriteState {
    #[must_use]
    pub(in crate::runtime::binary_object) fn new(
        limits: GraphLimits,
        allow_object_references: bool,
    ) -> Self {
        Self::with_references(
            limits,
            if allow_object_references {
                ObjectReferenceState::Emitting {
                    indices: HashMap::new(),
                    next_index: 0,
                }
            } else {
                ObjectReferenceState::Disabled
            },
        )
    }

    #[must_use]
    pub(in crate::runtime::binary_object) fn for_plan(
        limits: GraphLimits,
        allow_object_references: bool,
    ) -> Self {
        Self::with_references(
            limits,
            if allow_object_references {
                ObjectReferenceState::Planning { emitted: 0 }
            } else {
                ObjectReferenceState::Disabled
            },
        )
    }

    #[must_use]
    pub(in crate::runtime::binary_object) fn for_emission(
        limits: GraphLimits,
        allow_object_references: bool,
    ) -> Self {
        Self::new(limits, allow_object_references)
    }

    fn with_references(limits: GraphLimits, references: ObjectReferenceState) -> Self {
        Self {
            limits,
            references,
            active_nodes: HashSet::new(),
            unique_nodes: HashSet::new(),
            total_container_entries: 0,
            total_bigint_bytes: 0,
            total_array_buffer_bytes: 0,
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) fn allows_object_references(&self) -> bool {
        !matches!(self.references, ObjectReferenceState::Disabled)
    }

    pub(in crate::runtime::binary_object) fn enter_node(
        &mut self,
        node: NodeId,
    ) -> Result<DataNodeWrite, DataWriteStateError> {
        match &mut self.references {
            ObjectReferenceState::Emitting {
                indices,
                next_index,
            } => {
                if let Some(index) = indices.get(&node).copied() {
                    return Ok(DataNodeWrite::Reference(index));
                }
                let requested_references = usize::try_from(*next_index)
                    .ok()
                    .and_then(|count| count.checked_add(1))
                    .ok_or(GraphError::CountOverflow {
                        kind: GraphResourceKind::ObjectReferences,
                    })?;
                self.limits
                    .check(GraphResourceKind::ObjectReferences, requested_references)?;
                indices
                    .try_reserve(1)
                    .map_err(|_| DataWriteStateError::AllocationFailed)?;
                indices.insert(node, *next_index);
                *next_index = next_index.checked_add(1).ok_or(GraphError::CountOverflow {
                    kind: GraphResourceKind::ObjectReferences,
                })?;
                Ok(DataNodeWrite::Traverse)
            }
            ObjectReferenceState::Disabled => self
                .enter_active_node(node)
                .map(|()| DataNodeWrite::Traverse),
            ObjectReferenceState::Planning { .. } => {
                unreachable!("planning state requires enter_plan_node")
            }
        }
    }

    pub(in crate::runtime::binary_object) fn enter_plan_node(
        &mut self,
        node: NodeId,
    ) -> Result<DataPlanNodeWrite, DataWriteStateError> {
        match &mut self.references {
            ObjectReferenceState::Planning { emitted } => {
                if self.unique_nodes.contains(&node) {
                    return Ok(DataPlanNodeWrite::Reference);
                }
                *emitted = emitted.checked_add(1).ok_or(GraphError::CountOverflow {
                    kind: GraphResourceKind::ObjectReferences,
                })?;
                self.limits
                    .check(GraphResourceKind::ObjectReferences, *emitted)?;
                Ok(DataPlanNodeWrite::Traverse)
            }
            ObjectReferenceState::Disabled => self
                .enter_active_node(node)
                .map(|()| DataPlanNodeWrite::Traverse),
            ObjectReferenceState::Emitting { .. } => {
                unreachable!("emission state requires enter_node")
            }
        }
    }

    fn enter_active_node(&mut self, node: NodeId) -> Result<(), DataWriteStateError> {
        if self.active_nodes.contains(&node) {
            return Err(DataWriteStateError::CircularReference { node });
        }
        self.active_nodes
            .try_reserve(1)
            .map_err(|_| DataWriteStateError::AllocationFailed)?;
        self.active_nodes.insert(node);
        Ok(())
    }

    pub(in crate::runtime::binary_object) fn leave_node(&mut self, node: NodeId) {
        let was_active = self.active_nodes.remove(&node);
        debug_assert!(was_active);
    }

    pub(in crate::runtime::binary_object) fn child_depth(
        &self,
        parent_depth: usize,
    ) -> Result<usize, DataWriteStateError> {
        let depth = parent_depth
            .checked_add(1)
            .ok_or(GraphError::CountOverflow {
                kind: GraphResourceKind::NestingDepth,
            })?;
        self.limits.check(GraphResourceKind::NestingDepth, depth)?;
        Ok(depth)
    }

    pub(in crate::runtime::binary_object) fn reserve_unique_node(
        &mut self,
        node: NodeId,
    ) -> Result<Option<UniqueNodeReservation<'_>>, DataWriteStateError> {
        if self.unique_nodes.contains(&node) {
            return Ok(None);
        }
        let requested_nodes =
            self.unique_nodes
                .len()
                .checked_add(1)
                .ok_or(GraphError::CountOverflow {
                    kind: GraphResourceKind::Nodes,
                })?;
        self.limits
            .check(GraphResourceKind::Nodes, requested_nodes)?;
        self.unique_nodes
            .try_reserve(1)
            .map_err(|_| DataWriteStateError::AllocationFailed)?;
        Ok(Some(UniqueNodeReservation {
            unique_nodes: &mut self.unique_nodes,
            node,
        }))
    }

    pub(in crate::runtime::binary_object) fn record_unique_node(
        &mut self,
        node: NodeId,
    ) -> Result<bool, DataWriteStateError> {
        let Some(reservation) = self.reserve_unique_node(node)? else {
            return Ok(false);
        };
        reservation.commit();
        Ok(true)
    }

    pub(in crate::runtime::binary_object) fn check_container_entries(
        &self,
        entry_count: usize,
    ) -> Result<(), DataWriteStateError> {
        self.limits
            .check(GraphResourceKind::ContainerEntries, entry_count)?;
        Ok(())
    }

    pub(in crate::runtime::binary_object) fn charge_container_entries(
        &mut self,
        entry_count: usize,
    ) -> Result<(), DataWriteStateError> {
        Self::charge_aggregate(
            self.limits,
            &mut self.total_container_entries,
            GraphResourceKind::TotalContainerEntries,
            entry_count,
        )
    }

    pub(in crate::runtime::binary_object) fn check_bigint_bytes(
        &self,
        byte_length: usize,
    ) -> Result<(), DataWriteStateError> {
        self.limits
            .check(GraphResourceKind::BigIntBytes, byte_length)?;
        Ok(())
    }

    pub(in crate::runtime::binary_object) fn charge_bigint_bytes(
        &mut self,
        byte_length: usize,
    ) -> Result<(), DataWriteStateError> {
        Self::charge_aggregate(
            self.limits,
            &mut self.total_bigint_bytes,
            GraphResourceKind::TotalBigIntBytes,
            byte_length,
        )
    }

    pub(in crate::runtime::binary_object) fn charge_array_buffer_bytes(
        &mut self,
        byte_length: usize,
    ) -> Result<(), DataWriteStateError> {
        self.limits
            .check(GraphResourceKind::ArrayBufferBytes, byte_length)?;
        Self::charge_aggregate(
            self.limits,
            &mut self.total_array_buffer_bytes,
            GraphResourceKind::TotalArrayBufferBytes,
            byte_length,
        )
    }

    fn charge_aggregate(
        limits: GraphLimits,
        total: &mut usize,
        kind: GraphResourceKind,
        amount: usize,
    ) -> Result<(), DataWriteStateError> {
        *total = total
            .checked_add(amount)
            .ok_or(GraphError::CountOverflow { kind })?;
        limits.check(kind, *total)?;
        Ok(())
    }
}
