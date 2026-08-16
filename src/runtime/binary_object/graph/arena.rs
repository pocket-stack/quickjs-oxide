//! Shared, bounded object identity and reference-table state for BC5 readers.
//!
//! QuickJS carries one `BCReaderState.objects` table through every recursive
//! constant-pool value. Object-producing tags reserve entries here; function
//! records deliberately do not. Keeping this layer independent from the data
//! decoder lets a future whole-function image use the same registration order
//! without inheriting data-only error or frame types.

use std::fmt;

use super::model::{GraphError, GraphLimits, GraphResourceKind, NodeId, WireNodeCarrier};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum ArenaError {
    Graph(GraphError),
    InvalidNodeState { node: NodeId },
}

impl fmt::Display for ArenaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graph(error) => fmt::Display::fmt(error, formatter),
            Self::InvalidNodeState { node } => {
                write!(
                    formatter,
                    "invalid pending node state at {}",
                    node.zero_based()
                )
            }
        }
    }
}

impl std::error::Error for ArenaError {}

impl From<GraphError> for ArenaError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum PendingNodeKind {
    Ordinary,
    Array,
    TemplateObject,
    TypedArray,
}

impl PendingNodeKind {
    fn accepts<V, K>(self, node: &WireNodeCarrier<V, K>) -> bool {
        matches!(
            (self, node),
            (Self::Ordinary, WireNodeCarrier::Ordinary { .. })
                | (Self::Array, WireNodeCarrier::Array { .. })
                | (Self::TemplateObject, WireNodeCarrier::TemplateObject { .. })
                | (Self::TypedArray, WireNodeCarrier::TypedArray { .. })
        )
    }
}

enum NodeSlot<V, K> {
    Pending(PendingNodeKind),
    Ready(WireNodeCarrier<V, K>),
}

#[derive(Debug)]
pub(in crate::runtime::binary_object) enum NodeState<'a, V, K> {
    Pending(PendingNodeKind),
    Ready(&'a WireNodeCarrier<V, K>),
}

/// A node/reference preflight tied to one arena by an exclusive borrow.
///
/// Installation has no target-arena parameter, so a reservation cannot cross
/// arenas or race an alias insertion. Dropping it is a clean decode abort: no
/// identity or reference-table entry has been published yet.
#[must_use = "install the reserved node before continuing arena decoding"]
pub(in crate::runtime::binary_object) struct NodeReservation<'arena, V, K> {
    arena: &'arena mut ObjectArena<V, K>,
    node: NodeId,
    register_reference: bool,
}

impl<V, K> NodeReservation<'_, V, K> {
    pub(in crate::runtime::binary_object) fn install_pending_node(
        self,
        kind: PendingNodeKind,
    ) -> Result<NodeId, ArenaError> {
        self.install_node(NodeSlot::Pending(kind))
    }

    pub(in crate::runtime::binary_object) fn install_ready_node(
        self,
        node: WireNodeCarrier<V, K>,
    ) -> Result<NodeId, ArenaError> {
        self.install_node(NodeSlot::Ready(node))
    }

    fn install_node(self, slot: NodeSlot<V, K>) -> Result<NodeId, ArenaError> {
        let Self {
            arena,
            node,
            register_reference,
        } = self;
        arena.install_node(node, register_reference, slot)
    }
}

pub(in crate::runtime::binary_object) struct ObjectArenaParts<V, K> {
    pub(in crate::runtime::binary_object) nodes: Box<[WireNodeCarrier<V, K>]>,
    pub(in crate::runtime::binary_object) ref_table: Box<[NodeId]>,
}

pub(in crate::runtime::binary_object) struct ObjectArena<V, K> {
    limits: GraphLimits,
    allow_references: bool,
    slots: Vec<NodeSlot<V, K>>,
    references: Vec<NodeId>,
}

impl<V, K> ObjectArena<V, K> {
    #[must_use]
    pub(in crate::runtime::binary_object) const fn new(
        limits: GraphLimits,
        allow_references: bool,
    ) -> Self {
        Self {
            limits,
            allow_references,
            slots: Vec::new(),
            references: Vec::new(),
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn allows_references(&self) -> bool {
        self.allow_references
    }

    pub(in crate::runtime::binary_object) fn resolve_reference(
        &self,
        index: u32,
    ) -> Result<NodeId, ArenaError> {
        self.references.get(index as usize).copied().ok_or_else(|| {
            GraphError::InvalidReferenceIndex {
                index,
                reference_count: self.references.len(),
            }
            .into()
        })
    }

    pub(in crate::runtime::binary_object) fn node_state(
        &self,
        node: NodeId,
    ) -> Result<NodeState<'_, V, K>, ArenaError> {
        let node_count = self.slots.len();
        match self
            .slots
            .get(node.as_usize())
            .ok_or(GraphError::InvalidNodeIndex {
                index: node.zero_based(),
                node_count,
            })? {
            NodeSlot::Pending(kind) => Ok(NodeState::Pending(*kind)),
            NodeSlot::Ready(value) => Ok(NodeState::Ready(value)),
        }
    }

    pub(in crate::runtime::binary_object) fn reserve_node(
        &mut self,
    ) -> Result<NodeReservation<'_, V, K>, ArenaError> {
        let raw_index = u32::try_from(self.slots.len()).map_err(|_| GraphError::CountOverflow {
            kind: GraphResourceKind::Nodes,
        })?;
        let requested_nodes = self
            .slots
            .len()
            .checked_add(1)
            .ok_or(GraphError::CountOverflow {
                kind: GraphResourceKind::Nodes,
            })?;
        self.limits
            .check(GraphResourceKind::Nodes, requested_nodes)?;
        self.slots
            .try_reserve(1)
            .map_err(|_| GraphError::AllocationFailed)?;

        let register_reference = self.preflight_reference_entry()?;
        Ok(NodeReservation {
            arena: self,
            node: NodeId::from_zero_based(raw_index),
            register_reference,
        })
    }

    pub(in crate::runtime::binary_object) fn append_reference_alias(
        &mut self,
        node: NodeId,
    ) -> Result<(), ArenaError> {
        self.validate_node_index(node)?;
        if self.preflight_reference_entry()? {
            self.references.push(node);
        }
        Ok(())
    }

    pub(in crate::runtime::binary_object) fn complete_node(
        &mut self,
        node: NodeId,
        value: WireNodeCarrier<V, K>,
    ) -> Result<(), ArenaError> {
        let node_count = self.slots.len();
        let slot = self
            .slots
            .get_mut(node.as_usize())
            .ok_or(GraphError::InvalidNodeIndex {
                index: node.zero_based(),
                node_count,
            })?;
        let NodeSlot::Pending(expected) = slot else {
            return Err(ArenaError::InvalidNodeState { node });
        };
        if !expected.accepts(&value) {
            return Err(ArenaError::InvalidNodeState { node });
        }
        *slot = NodeSlot::Ready(value);
        Ok(())
    }

    pub(in crate::runtime::binary_object) fn finish(
        self,
    ) -> Result<ObjectArenaParts<V, K>, ArenaError> {
        let mut ready_nodes = Vec::new();
        ready_nodes
            .try_reserve_exact(self.slots.len())
            .map_err(|_| GraphError::AllocationFailed)?;
        for (index, slot) in self.slots.into_iter().enumerate() {
            match slot {
                NodeSlot::Ready(node) => ready_nodes.push(node),
                NodeSlot::Pending(_) => {
                    let index = u32::try_from(index).map_err(|_| GraphError::CountOverflow {
                        kind: GraphResourceKind::Nodes,
                    })?;
                    return Err(ArenaError::InvalidNodeState {
                        node: NodeId::from_zero_based(index),
                    });
                }
            }
        }
        Ok(ObjectArenaParts {
            nodes: ready_nodes.into_boxed_slice(),
            ref_table: self.references.into_boxed_slice(),
        })
    }

    fn preflight_reference_entry(&mut self) -> Result<bool, ArenaError> {
        if !self.allow_references {
            return Ok(false);
        }

        let requested_references =
            self.references
                .len()
                .checked_add(1)
                .ok_or(GraphError::CountOverflow {
                    kind: GraphResourceKind::ObjectReferences,
                })?;
        self.limits
            .check(GraphResourceKind::ObjectReferences, requested_references)?;
        self.references
            .try_reserve(1)
            .map_err(|_| GraphError::AllocationFailed)?;
        Ok(true)
    }

    fn install_node(
        &mut self,
        node: NodeId,
        register_reference: bool,
        slot: NodeSlot<V, K>,
    ) -> Result<NodeId, ArenaError> {
        if node.as_usize() != self.slots.len() || register_reference != self.allow_references {
            return Err(ArenaError::InvalidNodeState { node });
        }

        self.slots.push(slot);
        if register_reference {
            self.references.push(node);
        }
        Ok(node)
    }

    fn validate_node_index(&self, node: NodeId) -> Result<(), ArenaError> {
        let node_count = self.slots.len();
        if node.as_usize() >= node_count {
            return Err(GraphError::InvalidNodeIndex {
                index: node.zero_based(),
                node_count,
            }
            .into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::binary_object::graph::model::{WireKey, WireNode, WireValue};

    const LIMITS: GraphLimits = GraphLimits::new(64, 64, 32, 128, 256, 1024, 2048, 1024, 2048);

    fn new_arena(references: bool) -> ObjectArena<WireValue, WireKey> {
        ObjectArena::new(LIMITS, references)
    }

    #[test]
    fn pending_nodes_are_referenceable_and_complete_exactly_once() {
        let mut arena = new_arena(true);
        assert!(arena.slots.is_empty());
        assert!(arena.references.is_empty());

        let node = arena
            .reserve_node()
            .unwrap()
            .install_pending_node(PendingNodeKind::Array)
            .unwrap();
        assert!(matches!(
            arena.slots.as_slice(),
            [NodeSlot::Pending(PendingNodeKind::Array)]
        ));
        assert_eq!(arena.references.as_slice(), &[node]);

        arena.append_reference_alias(node).unwrap();
        assert_eq!(arena.references.as_slice(), &[node, node]);
        assert_eq!(arena.slots.len(), 1);

        let completed = WireNode::Array {
            elements: Box::from([WireValue::Node(node)]),
        };
        arena.complete_node(node, completed.clone()).unwrap();
        assert!(matches!(
            arena.slots.as_slice(),
            [NodeSlot::Ready(WireNode::Array { .. })]
        ));
        assert_eq!(
            arena.complete_node(
                node,
                WireNode::Array {
                    elements: Box::default(),
                },
            ),
            Err(ArenaError::InvalidNodeState { node })
        );

        let parts = arena.finish().unwrap();
        assert_eq!(parts.nodes.as_ref(), &[completed]);
        assert_eq!(parts.ref_table.as_ref(), &[node, node]);
    }

    #[test]
    fn pending_nodes_cannot_escape_finalization() {
        let mut arena = new_arena(false);
        let node = arena
            .reserve_node()
            .unwrap()
            .install_pending_node(PendingNodeKind::Array)
            .unwrap();
        assert!(matches!(
            arena.finish(),
            Err(ArenaError::InvalidNodeState { node: failed }) if failed == node
        ));
    }

    #[test]
    fn pending_node_kinds_reject_cross_kind_completion_without_mutation() {
        let mut arena = new_arena(false);
        let node = arena
            .reserve_node()
            .unwrap()
            .install_pending_node(PendingNodeKind::Ordinary)
            .unwrap();
        assert_eq!(
            arena.complete_node(
                node,
                WireNode::Array {
                    elements: Box::default(),
                },
            ),
            Err(ArenaError::InvalidNodeState { node })
        );
        assert!(matches!(
            arena.slots.as_slice(),
            [NodeSlot::Pending(PendingNodeKind::Ordinary)]
        ));
        arena
            .complete_node(
                node,
                WireNode::Ordinary {
                    properties: Box::default(),
                },
            )
            .unwrap();
    }

    #[test]
    fn source_bound_reservations_cancel_cleanly_and_enforce_budgets() {
        let mut arena = new_arena(true);
        drop(arena.reserve_node().unwrap());
        assert!(arena.slots.is_empty());
        assert!(arena.references.is_empty());

        let node = arena
            .reserve_node()
            .unwrap()
            .install_ready_node(WireNode::Array {
                elements: Box::default(),
            })
            .unwrap();

        arena.append_reference_alias(node).unwrap();
        assert_eq!(arena.references.as_slice(), &[node, node]);

        let one_reference = GraphLimits::new(4, 1, 4, 4, 4, 4, 4, 4, 4);
        let mut bounded: ObjectArena<WireValue, WireKey> = ObjectArena::new(one_reference, true);
        let bounded_node = bounded
            .reserve_node()
            .unwrap()
            .install_pending_node(PendingNodeKind::Array)
            .unwrap();
        assert!(matches!(
            bounded.append_reference_alias(bounded_node),
            Err(ArenaError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::ObjectReferences,
                requested: 2,
                limit: 1,
            }))
        ));
        assert_eq!(bounded.references.as_slice(), &[bounded_node]);

        let no_references = GraphLimits::new(4, 0, 4, 4, 4, 4, 4, 4, 4);
        let mut disabled: ObjectArena<WireValue, WireKey> = ObjectArena::new(no_references, false);
        let ready = disabled
            .reserve_node()
            .unwrap()
            .install_ready_node(WireNode::Array {
                elements: Box::default(),
            })
            .unwrap();
        disabled.append_reference_alias(ready).unwrap();
        assert!(disabled.references.is_empty());

        let invalid = NodeId::from_zero_based(1);
        assert_eq!(
            disabled.append_reference_alias(invalid),
            Err(ArenaError::Graph(GraphError::InvalidNodeIndex {
                index: 1,
                node_count: 1,
            }))
        );

        let no_nodes = GraphLimits::new(0, 4, 4, 4, 4, 4, 4, 4, 4);
        let mut node_bounded: ObjectArena<WireValue, WireKey> = ObjectArena::new(no_nodes, true);
        assert!(matches!(
            node_bounded.reserve_node(),
            Err(ArenaError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::Nodes,
                requested: 1,
                limit: 0,
            }))
        ));
    }
}
