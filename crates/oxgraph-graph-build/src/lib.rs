//! Append/update-only builders for directed graph topology.
//!
//! The builder assigns dense append-only canonical node and edge IDs, supports
//! isolated nodes and parallel edges, stores generic topology weights, attaches
//! named Arrow property layers, and freezes to an owned immutable view. Deletion,
//! tombstones, ID reuse, compaction, and overlay mutation are out of scope.
// kani-skip: builder freeze/export allocates variable-sized CSR/property snapshots; randomized
// proptests cover shape invariants here.

use std::{error::Error, fmt, vec, vec::Vec};

use oxgraph_csr::{SNAPSHOT_KIND_CSR_OFFSETS, SNAPSHOT_KIND_CSR_TARGETS};
use oxgraph_graph::{
    CanonicalElementIdentity, CanonicalRelationIdentity, ContainsElement, ContainsRelation,
    EdgeSourceGraph, EdgeTargetGraph, ElementIndex, ElementSuccessors, ElementWeight, GraphCounts,
    LocalElementIdentity, LocalRelationIdentity, OutgoingEdgeCount, OutgoingGraph, RelationIndex,
    RelationWeight, TopologyBase, TopologyCounts,
};
use oxgraph_property::{
    EncodedPropertySnapshot, IdFamily, IdentityModeRecord, PropertyError, PropertyLayer,
    SNAPSHOT_KIND_IDENTITY_MODES, SNAPSHOT_KIND_PROPERTY_DATA, SNAPSHOT_KIND_PROPERTY_DESCRIPTORS,
    SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U32, SNAPSHOT_PROPERTY_VERSION, encode_property_snapshot,
};
use oxgraph_snapshot::{PlanError, SnapshotBuilder};

/// Local/canonical node ID assigned by [`GraphBuilder`].
///
/// IDs are dense append-only `u32` handles and are not reused within one builder
/// generation.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GraphNodeId(pub u32);

/// Local/canonical edge ID assigned by [`GraphBuilder`].
///
/// Parallel edges are represented by distinct IDs. IDs are dense append-only
/// `u32` handles and are not reused within one builder generation.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GraphEdgeId(pub u32);

/// Errors raised by graph building and freeze/export operations.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Debug)]
#[non_exhaustive]
pub enum GraphBuildError {
    /// A node ID was not visible in the builder or frozen view.
    InvalidNode {
        /// Invalid node ID.
        node: GraphNodeId,
    },
    /// An edge ID was not visible in the builder or frozen view.
    InvalidEdge {
        /// Invalid edge ID.
        edge: GraphEdgeId,
    },
    /// A first-generation `u32` ID or offset overflowed.
    IdOverflow {
        /// Value that did not fit in `u32`.
        value: usize,
    },
    /// An attached property layer used an ID family unsupported by graphs.
    UnsupportedPropertyFamily {
        /// Unsupported family.
        id_family: IdFamily,
    },
    /// An attached property layer was too short for the frozen topology.
    PropertyLayerTooShort {
        /// Property layer ID family.
        id_family: IdFamily,
        /// Required logical length.
        required: usize,
        /// Actual logical length.
        actual: usize,
    },
    /// Property snapshot encoding failed.
    Property {
        /// Underlying property-layer error.
        source: PropertyError,
    },
    /// Snapshot export failed.
    SnapshotPlan {
        /// Underlying snapshot planning error.
        source: PlanError,
    },
}

impl fmt::Display for GraphBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNode { node } => write!(formatter, "invalid graph node ID {node:?}"),
            Self::InvalidEdge { edge } => write!(formatter, "invalid graph edge ID {edge:?}"),
            Self::IdOverflow { value } => write!(formatter, "graph builder ID overflow at {value}"),
            Self::UnsupportedPropertyFamily { id_family } => {
                write!(formatter, "unsupported graph property family {id_family:?}")
            }
            Self::PropertyLayerTooShort {
                id_family,
                required,
                actual,
            } => write!(
                formatter,
                "graph property layer for {id_family:?} too short: required {required}, got {actual}"
            ),
            Self::Property { source } => {
                write!(formatter, "property snapshot export failed: {source}")
            }
            Self::SnapshotPlan { source } => write!(formatter, "snapshot export failed: {source}"),
        }
    }
}

impl Error for GraphBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Property { source } => Some(source),
            Self::SnapshotPlan { source } => Some(source),
            Self::InvalidNode { .. }
            | Self::InvalidEdge { .. }
            | Self::IdOverflow { .. }
            | Self::UnsupportedPropertyFamily { .. }
            | Self::PropertyLayerTooShort { .. } => None,
        }
    }
}

impl From<PlanError> for GraphBuildError {
    fn from(source: PlanError) -> Self {
        Self::SnapshotPlan { source }
    }
}

impl From<PropertyError> for GraphBuildError {
    fn from(source: PropertyError) -> Self {
        Self::Property { source }
    }
}

/// Append/update-only directed graph builder.
///
/// `EW` is the element-weight type and `RW` is the relation-weight type. Callers
/// provide default weights at construction; the builder does not assume `f64`,
/// `Default`, or `One`.
///
/// # Performance
///
/// Adding a node or edge is `O(1)` amortized. Freezing is `O(n + m)` for `n`
/// nodes and `m` edges.
#[derive(Clone, Debug)]
#[must_use]
pub struct GraphBuilder<EW, RW> {
    /// Number of nodes assigned so far.
    node_count: u32,
    /// Default element weight for future nodes.
    default_element_weight: EW,
    /// Default relation weight for future edges.
    default_relation_weight: RW,
    /// Element weights by canonical node ID.
    element_weights: Vec<EW>,
    /// Edge source IDs by canonical edge ID.
    sources: Vec<u32>,
    /// Edge target IDs by canonical edge ID.
    targets: Vec<u32>,
    /// Relation weights by canonical edge ID.
    relation_weights: Vec<RW>,
    /// Named property layers attached before freeze/export.
    property_layers: Vec<PropertyLayer>,
    /// Builder generation bumped by edits.
    generation: u64,
}

impl<EW: Clone, RW: Clone> GraphBuilder<EW, RW> {
    /// Constructs an empty graph builder with caller-provided default weights.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub const fn new(default_element_weight: EW, default_relation_weight: RW) -> Self {
        Self {
            node_count: 0,
            default_element_weight,
            default_relation_weight,
            element_weights: Vec::new(),
            sources: Vec::new(),
            targets: Vec::new(),
            relation_weights: Vec::new(),
            property_layers: Vec::new(),
            generation: 0,
        }
    }

    /// Returns the current edit generation.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Adds one isolated node and returns its canonical ID.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::IdOverflow`] if the `u32` ID space is exhausted.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` amortized.
    pub fn add_node(&mut self) -> Result<GraphNodeId, GraphBuildError> {
        let id = self.node_count;
        self.node_count = self
            .node_count
            .checked_add(1)
            .ok_or(GraphBuildError::IdOverflow { value: usize::MAX })?;
        self.element_weights
            .push(self.default_element_weight.clone());
        self.generation = self.generation.wrapping_add(1);
        Ok(GraphNodeId(id))
    }

    /// Adds a directed edge and returns its canonical edge ID.
    ///
    /// Parallel edges are accepted and represented by distinct IDs.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::InvalidNode`] when either endpoint is not
    /// visible, or [`GraphBuildError::IdOverflow`] if the edge ID would not fit
    /// in `u32`.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` amortized.
    pub fn add_edge(
        &mut self,
        source: GraphNodeId,
        target: GraphNodeId,
    ) -> Result<GraphEdgeId, GraphBuildError> {
        self.ensure_node(source)?;
        self.ensure_node(target)?;
        let edge = usize_to_u32(self.sources.len())?;
        self.sources.push(source.0);
        self.targets.push(target.0);
        self.relation_weights
            .push(self.default_relation_weight.clone());
        self.generation = self.generation.wrapping_add(1);
        Ok(GraphEdgeId(edge))
    }

    /// Updates the element weight for an existing node.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::InvalidNode`] when `node` is not visible.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn set_element_weight(
        &mut self,
        node: GraphNodeId,
        weight: EW,
    ) -> Result<(), GraphBuildError> {
        self.ensure_node(node)?;
        self.element_weights[node.0 as usize] = weight;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    /// Updates the relation weight for an existing edge.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::InvalidEdge`] when `edge` is not visible.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn set_relation_weight(
        &mut self,
        edge: GraphEdgeId,
        weight: RW,
    ) -> Result<(), GraphBuildError> {
        let index = self.edge_index_checked(edge)?;
        self.relation_weights[index] = weight;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    /// Attaches a named Arrow property layer for snapshot export.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::UnsupportedPropertyFamily`] for incidence-keyed
    /// layers because ordinary graph builders expose element and relation IDs.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` plus descriptor inspection.
    pub fn add_property_layer(&mut self, layer: PropertyLayer) -> Result<(), GraphBuildError> {
        if layer.descriptor().id_family == IdFamily::Incidence {
            return Err(GraphBuildError::UnsupportedPropertyFamily {
                id_family: IdFamily::Incidence,
            });
        }
        self.property_layers.push(layer);
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    /// Returns the number of nodes assigned so far.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn node_count(&self) -> usize {
        self.node_count as usize
    }

    /// Returns the number of edges assigned so far.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn edge_count(&self) -> usize {
        self.sources.len()
    }

    /// Freezes the current builder contents into an owned immutable view.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::IdOverflow`] if an intermediate CSR offset
    /// does not fit the first implementation's `u32` storage.
    ///
    /// # Performance
    ///
    /// This function is `O(n + m)` for `n` nodes and `m` edges.
    pub fn freeze(&self) -> Result<FrozenGraph<EW, RW>, GraphBuildError> {
        self.validate_property_layers()?;
        let node_count = self.node_count();
        let mut offsets = vec![0_u32; node_count + 1];
        for &source in &self.sources {
            let slot = source as usize + 1;
            offsets[slot] = offsets[slot]
                .checked_add(1)
                .ok_or(GraphBuildError::IdOverflow { value: usize::MAX })?;
        }
        for index in 1..offsets.len() {
            let previous = offsets[index - 1];
            offsets[index] = offsets[index]
                .checked_add(previous)
                .ok_or(GraphBuildError::IdOverflow { value: usize::MAX })?;
        }
        // CLONE: freeze needs independent write cursors while preserving final
        // offsets for the owned immutable view.
        let mut cursor = offsets.clone();
        let mut edge_ids = vec![0_u32; self.sources.len()];
        let mut targets = vec![0_u32; self.targets.len()];
        for (edge_index, (&source, &target)) in self.sources.iter().zip(&self.targets).enumerate() {
            let source_slot = source as usize;
            let position = cursor[source_slot] as usize;
            cursor[source_slot] = cursor[source_slot]
                .checked_add(1)
                .ok_or(GraphBuildError::IdOverflow { value: usize::MAX })?;
            edge_ids[position] = usize_to_u32(edge_index)?;
            targets[position] = target;
        }
        Ok(FrozenGraph {
            node_count: self.node_count,
            offsets: offsets.into_boxed_slice(),
            edge_ids: edge_ids.into_boxed_slice(),
            targets: targets.into_boxed_slice(),
            element_weights: self.element_weights.clone().into_boxed_slice(),
            edge_targets: self.targets.clone().into_boxed_slice(),
            sources: self.sources.clone().into_boxed_slice(),
            relation_weights: self.relation_weights.clone().into_boxed_slice(),
            property_layers: self.property_layers.clone().into_boxed_slice(),
        })
    }

    /// Validates attached property layers against current topology counts.
    ///
    /// # Performance
    ///
    /// This function is `O(p)` for `p` property layers.
    fn validate_property_layers(&self) -> Result<(), GraphBuildError> {
        for layer in &self.property_layers {
            let required = match layer.descriptor().id_family {
                IdFamily::Element => self.node_count(),
                IdFamily::Relation => self.edge_count(),
                unsupported => {
                    return Err(GraphBuildError::UnsupportedPropertyFamily {
                        id_family: unsupported,
                    });
                }
            };
            if layer.len() < required {
                return Err(GraphBuildError::PropertyLayerTooShort {
                    id_family: layer.descriptor().id_family,
                    required,
                    actual: layer.len(),
                });
            }
        }
        Ok(())
    }

    /// Validates that `node` is visible in this builder.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn ensure_node(&self, node: GraphNodeId) -> Result<(), GraphBuildError> {
        if node.0 < self.node_count {
            Ok(())
        } else {
            Err(GraphBuildError::InvalidNode { node })
        }
    }

    /// Returns a checked edge index.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn edge_index_checked(&self, edge: GraphEdgeId) -> Result<usize, GraphBuildError> {
        let index = edge.0 as usize;
        if index < self.sources.len() {
            Ok(index)
        } else {
            Err(GraphBuildError::InvalidEdge { edge })
        }
    }
}

/// Owned immutable graph produced by [`GraphBuilder::freeze`].
///
/// Local IDs and canonical IDs are equal in this first builder generation.
///
/// # Performance
///
/// Count, containment, index, endpoint, identity, and weight lookup are `O(1)`;
/// outgoing traversal is `O(out_degree)`.
#[derive(Clone, Debug)]
#[must_use]
pub struct FrozenGraph<EW, RW> {
    /// Node count.
    node_count: u32,
    /// CSR offsets by source node.
    offsets: Box<[u32]>,
    /// Edge IDs grouped by source node.
    edge_ids: Box<[u32]>,
    /// Edge targets grouped by source node.
    targets: Box<[u32]>,
    /// Element weights by local/canonical node ID.
    element_weights: Box<[EW]>,
    /// Edge targets by canonical edge ID.
    edge_targets: Box<[u32]>,
    /// Edge sources by canonical edge ID.
    sources: Box<[u32]>,
    /// Relation weights by canonical edge ID.
    relation_weights: Box<[RW]>,
    /// Named property layers attached to the frozen view.
    property_layers: Box<[PropertyLayer]>,
}

impl<EW, RW> FrozenGraph<EW, RW> {
    /// Returns the CSR offsets array.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub fn offsets(&self) -> &[u32] {
        &self.offsets
    }

    /// Returns the CSR target array grouped by source.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub fn targets(&self) -> &[u32] {
        &self.targets
    }

    /// Returns attached property layers.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn property_layers(&self) -> &[PropertyLayer] {
        &self.property_layers
    }

    /// Exports CSR topology, identity, and attached property sections.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::SnapshotPlan`] if snapshot planning rejects a
    /// section or final byte size.
    ///
    /// # Performance
    ///
    /// This function is `O(n + m + property bytes)`.
    pub fn to_csr_snapshot(&self) -> Result<Vec<u8>, GraphBuildError> {
        let property = self.encode_property_snapshot()?;
        let identity_modes = [
            IdentityModeRecord::local_equals_canonical(IdFamily::Element, self.node_count()),
            IdentityModeRecord::explicit_u32_map(IdFamily::Relation, self.edge_ids.len()),
        ];
        let mut builder = SnapshotBuilder::new();
        builder.add_section_typed(SNAPSHOT_KIND_CSR_OFFSETS, 1, &self.offsets)?;
        builder.add_section_typed(SNAPSHOT_KIND_CSR_TARGETS, 1, &self.targets)?;
        builder.add_section_typed(
            SNAPSHOT_KIND_IDENTITY_MODES,
            SNAPSHOT_PROPERTY_VERSION,
            &identity_modes,
        )?;
        builder.add_section_typed(
            SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U32,
            SNAPSHOT_PROPERTY_VERSION,
            &self.edge_ids,
        )?;
        if let Some(property) = property {
            builder.add_section(
                SNAPSHOT_KIND_PROPERTY_DESCRIPTORS,
                SNAPSHOT_PROPERTY_VERSION,
                0,
                property.descriptors,
            )?;
            builder.add_section(
                SNAPSHOT_KIND_PROPERTY_DATA,
                SNAPSHOT_PROPERTY_VERSION,
                0,
                property.data,
            )?;
        }
        builder.finish().map_err(GraphBuildError::from)
    }

    /// Encodes attached property snapshot sections.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::Property`] if descriptor/layer validation fails.
    ///
    /// # Performance
    ///
    /// This function is `O(property bytes)`.
    fn encode_property_snapshot(&self) -> Result<Option<EncodedPropertySnapshot>, GraphBuildError> {
        if self.property_layers.is_empty() {
            Ok(None)
        } else {
            Ok(Some(encode_property_snapshot(&self.property_layers)?))
        }
    }

    /// Returns a checked node slot.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn node_slot(node: GraphNodeId) -> usize {
        node.0 as usize
    }

    /// Returns a checked edge slot.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn edge_slot(edge: GraphEdgeId) -> usize {
        edge.0 as usize
    }

    /// Returns the outgoing edge range for `node`.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn outgoing_range(&self, node: GraphNodeId) -> core::ops::Range<usize> {
        let slot = Self::node_slot(node);
        self.offsets[slot] as usize..self.offsets[slot + 1] as usize
    }
}

impl<EW, RW> TopologyBase for FrozenGraph<EW, RW> {
    type ElementId = GraphNodeId;
    type RelationId = GraphEdgeId;
}

impl<EW, RW> TopologyCounts for FrozenGraph<EW, RW> {
    fn element_count(&self) -> usize {
        self.node_count as usize
    }

    fn relation_count(&self) -> usize {
        self.sources.len()
    }
}

impl<EW, RW> GraphCounts for FrozenGraph<EW, RW> {}

impl<EW, RW> ElementIndex for FrozenGraph<EW, RW> {
    fn element_bound(&self) -> usize {
        self.node_count as usize
    }

    fn element_index(&self, element: GraphNodeId) -> usize {
        element.0 as usize
    }
}

impl<EW, RW> RelationIndex for FrozenGraph<EW, RW> {
    fn relation_bound(&self) -> usize {
        self.sources.len()
    }

    fn relation_index(&self, relation: GraphEdgeId) -> usize {
        relation.0 as usize
    }
}

impl<EW, RW> ContainsElement for FrozenGraph<EW, RW> {
    fn contains_element(&self, element: GraphNodeId) -> bool {
        element.0 < self.node_count
    }
}

impl<EW, RW> ContainsRelation for FrozenGraph<EW, RW> {
    fn contains_relation(&self, relation: GraphEdgeId) -> bool {
        (relation.0 as usize) < self.sources.len()
    }
}

impl<EW, RW> EdgeSourceGraph for FrozenGraph<EW, RW> {
    fn source(&self, edge: GraphEdgeId) -> GraphNodeId {
        GraphNodeId(self.sources[Self::edge_slot(edge)])
    }
}

impl<EW, RW> EdgeTargetGraph for FrozenGraph<EW, RW> {
    fn target(&self, edge: GraphEdgeId) -> GraphNodeId {
        GraphNodeId(self.edge_targets[Self::edge_slot(edge)])
    }
}

impl<EW, RW> OutgoingGraph for FrozenGraph<EW, RW> {
    type OutEdges<'view>
        = FrozenOutEdges<'view>
    where
        Self: 'view;

    fn outgoing_edges(&self, node: GraphNodeId) -> Self::OutEdges<'_> {
        FrozenOutEdges {
            inner: self.edge_ids[self.outgoing_range(node)].iter(),
        }
    }
}

impl<EW, RW> OutgoingEdgeCount for FrozenGraph<EW, RW> {
    fn out_degree(&self, node: GraphNodeId) -> usize {
        self.outgoing_range(node).len()
    }
}

impl<EW, RW> ElementSuccessors for FrozenGraph<EW, RW> {
    type Successors<'view>
        = FrozenSuccessors<'view>
    where
        Self: 'view;

    fn element_successors(&self, node: GraphNodeId) -> Self::Successors<'_> {
        FrozenSuccessors {
            edge_ids: self.edge_ids[self.outgoing_range(node)].iter(),
            targets_by_edge: &self.edge_targets,
        }
    }
}

impl<EW: Copy, RW> ElementWeight for FrozenGraph<EW, RW> {
    type Weight = EW;

    fn element_weight(&self, element: GraphNodeId) -> Self::Weight {
        self.element_weights[Self::node_slot(element)]
    }
}

impl<EW, RW: Copy> RelationWeight for FrozenGraph<EW, RW> {
    type Weight = RW;

    fn relation_weight(&self, relation: GraphEdgeId) -> Self::Weight {
        self.relation_weights[Self::edge_slot(relation)]
    }
}

impl<EW, RW> CanonicalElementIdentity for FrozenGraph<EW, RW> {
    type CanonicalElementId = GraphNodeId;

    fn canonical_element_id(&self, element: GraphNodeId) -> Self::CanonicalElementId {
        element
    }
}

impl<EW, RW> LocalElementIdentity for FrozenGraph<EW, RW> {
    fn local_element_id(&self, canonical: Self::CanonicalElementId) -> Option<Self::ElementId> {
        self.contains_element(canonical).then_some(canonical)
    }
}

impl<EW, RW> CanonicalRelationIdentity for FrozenGraph<EW, RW> {
    type CanonicalRelationId = GraphEdgeId;

    fn canonical_relation_id(&self, relation: GraphEdgeId) -> Self::CanonicalRelationId {
        relation
    }
}

impl<EW, RW> LocalRelationIdentity for FrozenGraph<EW, RW> {
    fn local_relation_id(&self, canonical: Self::CanonicalRelationId) -> Option<Self::RelationId> {
        self.contains_relation(canonical).then_some(canonical)
    }
}

/// Iterator over frozen outgoing edge IDs.
///
/// # Performance
///
/// Advancing is `O(1)`.
pub struct FrozenOutEdges<'view> {
    /// Borrowed edge IDs.
    inner: core::slice::Iter<'view, u32>,
}

impl Iterator for FrozenOutEdges<'_> {
    type Item = GraphEdgeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().copied().map(GraphEdgeId)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for FrozenOutEdges<'_> {}

/// Iterator over frozen outgoing successor nodes.
///
/// # Performance
///
/// Advancing is `O(1)` after iterator construction.
pub struct FrozenSuccessors<'view> {
    /// Borrowed outgoing edge IDs.
    edge_ids: core::slice::Iter<'view, u32>,
    /// Edge-ID-indexed target vector.
    targets_by_edge: &'view [u32],
}

impl Iterator for FrozenSuccessors<'_> {
    type Item = GraphNodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.edge_ids
            .next()
            .copied()
            .map(|edge| GraphNodeId(self.targets_by_edge[edge as usize]))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.edge_ids.size_hint()
    }
}

impl ExactSizeIterator for FrozenSuccessors<'_> {}

/// Converts `usize` to `u32` for first-generation builder IDs.
///
/// # Performance
///
/// This function is `O(1)`.
fn usize_to_u32(value: usize) -> Result<u32, GraphBuildError> {
    u32::try_from(value).map_err(|_error| GraphBuildError::IdOverflow { value })
}

#[cfg(test)]
mod tests {
    //! Tests for graph builder freeze semantics.

    use arrow_array::Int32Array;
    use arrow_schema::{DataType, Field};
    use oxgraph_property::{LayerId, LayerRole, PropertyLayerDescriptor, StorageMode};

    use super::*;

    /// Builder preserves isolated nodes, parallel edges, and generic weights.
    #[test]
    fn freeze_preserves_parallel_edges() -> Result<(), GraphBuildError> {
        let mut builder = GraphBuilder::new(1_i32, 1_u16);
        let a = builder.add_node()?;
        let b = builder.add_node()?;
        let first = builder.add_edge(a, b)?;
        let second = builder.add_edge(a, b)?;
        builder.set_element_weight(a, 4_i32)?;
        builder.set_relation_weight(second, 2_u16)?;
        let frozen = builder.freeze()?;
        assert_eq!(frozen.node_count(), 2);
        assert_eq!(frozen.edge_count(), 2);
        assert_eq!(
            frozen.outgoing_edges(a).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_eq!(frozen.element_weight(a), 4_i32);
        assert_eq!(frozen.relation_weight(second), 2_u16);
        Ok(())
    }

    /// Graph snapshots include identity and attached property sections that validate.
    #[test]
    fn snapshot_includes_identity_and_property_sections() -> Result<(), Box<dyn Error>> {
        use oxgraph_property::{validate_identity_snapshot, validate_property_snapshot};
        use oxgraph_snapshot::Snapshot;

        let mut builder = GraphBuilder::new((), ());
        let a = builder.add_node()?;
        let b = builder.add_node()?;
        let _edge = builder.add_edge(a, b)?;
        let descriptor = PropertyLayerDescriptor::try_new(
            LayerId(1),
            "relation_count",
            IdFamily::Relation,
            LayerRole::Property,
            StorageMode::Dense,
            Field::new("relation_count", DataType::Int32, false),
        )?;
        builder.add_property_layer(PropertyLayer::try_new_dense(
            descriptor,
            std::sync::Arc::new(Int32Array::from(vec![5_i32])),
        )?)?;
        let frozen = builder.freeze()?;
        let bytes = frozen.to_csr_snapshot()?;
        let snapshot = Snapshot::open(&bytes)?;
        assert_eq!(validate_identity_snapshot(&snapshot)?.records.len(), 2);
        assert_eq!(validate_property_snapshot(&snapshot)?.layer_count, 1);
        Ok(())
    }
}
