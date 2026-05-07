//! Append/freeze builders for directed graph topology.
//!
//! Core builders are `no_std + alloc` and do not know about Arrow, named
//! properties, snapshots, or Python. Snapshot and property export helpers live
//! behind explicit features.
// kani-skip: builders allocate variable-sized topology and snapshot buffers; proptests cover
// shape invariants and export roundtrips.
#![no_std]

extern crate alloc;

use alloc::{boxed::Box, vec, vec::Vec};
use core::{error::Error, fmt, hash::Hash};

#[cfg(feature = "snapshot")]
use oxgraph_csr::CsrSnapshotIndex;
use oxgraph_graph::{
    CanonicalElementIdentity, CanonicalRelationIdentity, ContainsElement, ContainsRelation,
    EdgeSourceGraph, EdgeTargetGraph, ElementIndex, ElementSuccessors, ElementWeight, GraphCounts,
    LocalElementIdentity, LocalRelationIdentity, OutgoingEdgeCount, OutgoingGraph, RelationIndex,
    RelationWeight, TopologyBase, TopologyCounts,
};
#[cfg(feature = "property-arrow")]
use oxgraph_property::{
    EncodedPropertySnapshot, GraphPropertyLayers, IdFamily, IdentityModeRecord,
    PropertySnapshotMetaWord, SNAPSHOT_PROPERTY_VERSION, encode_graph_property_snapshot,
    rekey_layer_to_local,
};
#[cfg(feature = "property-arrow")]
use oxgraph_property::{PropertyError, PropertyLayer};
#[cfg(feature = "snapshot")]
use oxgraph_snapshot::{PlanError, SnapshotBuilder};
#[cfg(feature = "snapshot")]
use zerocopy::byteorder::{LE, U16, U32, U64};

/// Result returned by weighted graph freeze operations.
type FrozenWeightedGraphResult<NodeIndex, EdgeIndex, EW, RW> = Result<
    FrozenWeightedGraph<NodeIndex, EdgeIndex, EW, RW>,
    GraphBuildError<NodeIndex, EdgeIndex>,
>;

/// Sealed trait module for builder index widths.
mod sealed {
    /// Seals [`super::BuildIndex`] to supported unsigned index widths.
    pub trait BuildIndex {}
}

/// Unsigned dense ID width usable by graph builders.
///
/// # Performance
///
/// Implementations perform checked conversions in `O(1)`.
pub trait BuildIndex: sealed::BuildIndex + Copy + Eq + Ord + fmt::Debug + Hash {
    /// Converts this ID to `usize` when representable on the current target.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn to_usize(self) -> Option<usize>;

    /// Converts a `usize` into this ID width when representable.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_usize(value: usize) -> Option<Self>;
}

/// Implements [`BuildIndex`] for one unsigned width.
macro_rules! impl_build_index {
    ($index:ty) => {
        impl sealed::BuildIndex for $index {}

        impl BuildIndex for $index {
            fn to_usize(self) -> Option<usize> {
                usize::try_from(self).ok()
            }

            fn from_usize(value: usize) -> Option<Self> {
                Self::try_from(value).ok()
            }
        }
    };
}

impl_build_index!(u16);
impl_build_index!(u32);
impl_build_index!(u64);

impl sealed::BuildIndex for usize {}

impl BuildIndex for usize {
    fn to_usize(self) -> Option<usize> {
        Some(self)
    }

    fn from_usize(value: usize) -> Option<Self> {
        Some(value)
    }
}

/// Local/canonical node ID assigned by graph builders.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GraphNodeId<NodeIndex>(pub NodeIndex);

/// Local/canonical edge ID assigned by graph builders.
///
/// # Performance
///
/// Copying, comparing, ordering, hashing, and debug-formatting are `O(1)` when
/// the underlying index type provides those operations in `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GraphEdgeId<EdgeIndex>(pub EdgeIndex);

/// Errors raised by graph building and export operations.
///
/// # Performance
///
/// Formatting is `O(message length)`.
#[derive(Debug)]
#[non_exhaustive]
pub enum GraphBuildError<NodeIndex, EdgeIndex> {
    /// A node ID was not visible in the builder or frozen view.
    InvalidNode {
        /// Invalid node ID.
        node: GraphNodeId<NodeIndex>,
    },
    /// An edge ID was not visible in the builder or frozen view.
    InvalidEdge {
        /// Invalid edge ID.
        edge: GraphEdgeId<EdgeIndex>,
    },
    /// A dense ID or offset overflowed the selected builder width.
    IdOverflow {
        /// Value that did not fit.
        value: usize,
    },
    /// A property layer was too short for the frozen topology.
    #[cfg(feature = "property-arrow")]
    PropertyLayerTooShort {
        /// Property layer ID family.
        id_family: IdFamily,
        /// Required logical length.
        required: usize,
        /// Actual logical length.
        actual: usize,
    },
    /// Property snapshot encoding failed.
    #[cfg(feature = "property-arrow")]
    Property {
        /// Underlying property-layer error.
        source: PropertyError,
    },
    /// Snapshot export failed.
    #[cfg(feature = "snapshot")]
    SnapshotPlan {
        /// Underlying snapshot planning error.
        source: PlanError,
    },
}

impl<NodeIndex, EdgeIndex> fmt::Display for GraphBuildError<NodeIndex, EdgeIndex>
where
    NodeIndex: fmt::Debug,
    EdgeIndex: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNode { node } => write!(formatter, "invalid graph node ID {node:?}"),
            Self::InvalidEdge { edge } => write!(formatter, "invalid graph edge ID {edge:?}"),
            Self::IdOverflow { value } => write!(formatter, "graph builder ID overflow at {value}"),
            #[cfg(feature = "property-arrow")]
            Self::PropertyLayerTooShort {
                id_family,
                required,
                actual,
            } => write!(
                formatter,
                "graph property layer for {id_family:?} too short: required {required}, got {actual}"
            ),
            #[cfg(feature = "property-arrow")]
            Self::Property { source } => {
                write!(formatter, "property snapshot export failed: {source}")
            }
            #[cfg(feature = "snapshot")]
            Self::SnapshotPlan { source } => write!(formatter, "snapshot export failed: {source}"),
        }
    }
}

impl<NodeIndex, EdgeIndex> Error for GraphBuildError<NodeIndex, EdgeIndex>
where
    NodeIndex: fmt::Debug,
    EdgeIndex: fmt::Debug,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(feature = "property-arrow")]
            Self::Property { source } => Some(source),
            #[cfg(feature = "snapshot")]
            Self::SnapshotPlan { source } => Some(source),
            Self::InvalidNode { .. } | Self::InvalidEdge { .. } | Self::IdOverflow { .. } => None,
            #[cfg(feature = "property-arrow")]
            Self::PropertyLayerTooShort { .. } => None,
        }
    }
}

#[cfg(feature = "snapshot")]
impl<NodeIndex, EdgeIndex> From<PlanError> for GraphBuildError<NodeIndex, EdgeIndex> {
    fn from(source: PlanError) -> Self {
        Self::SnapshotPlan { source }
    }
}

#[cfg(feature = "property-arrow")]
impl<NodeIndex, EdgeIndex> From<PropertyError> for GraphBuildError<NodeIndex, EdgeIndex> {
    fn from(source: PropertyError) -> Self {
        Self::Property { source }
    }
}

/// Append-only unweighted directed graph builder.
///
/// # Performance
///
/// Adding a node or edge is `O(1)` amortized. Freezing is `O(n + m)`.
#[derive(Clone, Debug)]
#[must_use]
pub struct GraphBuilder<NodeIndex, EdgeIndex>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    /// Number of nodes assigned so far.
    node_count: usize,
    /// Edge source IDs by canonical edge ID.
    sources: Vec<NodeIndex>,
    /// Edge target IDs by canonical edge ID.
    targets: Vec<NodeIndex>,
    /// Edge ID marker.
    edge: core::marker::PhantomData<EdgeIndex>,
}

impl<NodeIndex, EdgeIndex> GraphBuilder<NodeIndex, EdgeIndex>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    /// Constructs an empty unweighted graph builder.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub const fn new() -> Self {
        Self {
            node_count: 0,
            sources: Vec::new(),
            targets: Vec::new(),
            edge: core::marker::PhantomData,
        }
    }

    /// Adds one isolated node and returns its canonical ID.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::IdOverflow`] if `NodeIndex` cannot represent
    /// the next ID.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` amortized.
    pub fn add_node(
        &mut self,
    ) -> Result<GraphNodeId<NodeIndex>, GraphBuildError<NodeIndex, EdgeIndex>> {
        let id = NodeIndex::from_usize(self.node_count).ok_or(GraphBuildError::IdOverflow {
            value: self.node_count,
        })?;
        self.node_count = self
            .node_count
            .checked_add(1)
            .ok_or(GraphBuildError::IdOverflow { value: usize::MAX })?;
        Ok(GraphNodeId(id))
    }

    /// Adds a directed edge and returns its canonical edge ID.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::InvalidNode`] when either endpoint is not
    /// visible, or [`GraphBuildError::IdOverflow`] if `EdgeIndex` cannot
    /// represent the next edge ID.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` amortized.
    pub fn add_edge(
        &mut self,
        source: GraphNodeId<NodeIndex>,
        target: GraphNodeId<NodeIndex>,
    ) -> Result<GraphEdgeId<EdgeIndex>, GraphBuildError<NodeIndex, EdgeIndex>> {
        ensure_node(source, self.node_count)?;
        ensure_node(target, self.node_count)?;
        let edge =
            EdgeIndex::from_usize(self.sources.len()).ok_or(GraphBuildError::IdOverflow {
                value: self.sources.len(),
            })?;
        self.sources.push(source.0);
        self.targets.push(target.0);
        Ok(GraphEdgeId(edge))
    }

    /// Returns the number of nodes assigned so far.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn node_count(&self) -> usize {
        self.node_count
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

    /// Freezes the current builder contents into an immutable graph view.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::IdOverflow`] if a CSR offset or edge ID does
    /// not fit the selected `EdgeIndex` width.
    ///
    /// # Performance
    ///
    /// This function is `O(n + m)`.
    pub fn freeze(
        &self,
    ) -> Result<FrozenGraph<NodeIndex, EdgeIndex>, GraphBuildError<NodeIndex, EdgeIndex>> {
        Ok(FrozenGraph {
            topology: freeze_topology(self.node_count, &self.sources, &self.targets)?,
        })
    }
}

impl<NodeIndex, EdgeIndex> Default for GraphBuilder<NodeIndex, EdgeIndex>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Append-only directed graph builder with explicit element and relation weights.
///
/// # Performance
///
/// Adding a node or edge is `O(1)` amortized. Freezing is `O(n + m)`.
#[derive(Clone, Debug)]
#[must_use]
pub struct WeightedGraphBuilder<NodeIndex, EdgeIndex, EW, RW>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    /// Number of nodes assigned so far.
    node_count: usize,
    /// Element weights by canonical node ID.
    element_weights: Vec<EW>,
    /// Edge source IDs by canonical edge ID.
    sources: Vec<NodeIndex>,
    /// Edge target IDs by canonical edge ID.
    targets: Vec<NodeIndex>,
    /// Relation weights by canonical edge ID.
    relation_weights: Vec<RW>,
    /// Edge ID marker.
    edge: core::marker::PhantomData<EdgeIndex>,
}

impl<NodeIndex, EdgeIndex, EW, RW> WeightedGraphBuilder<NodeIndex, EdgeIndex, EW, RW>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    /// Constructs an empty weighted graph builder.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub const fn new() -> Self {
        Self {
            node_count: 0,
            element_weights: Vec::new(),
            sources: Vec::new(),
            targets: Vec::new(),
            relation_weights: Vec::new(),
            edge: core::marker::PhantomData,
        }
    }

    /// Adds one node with an explicit element weight.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::IdOverflow`] if `NodeIndex` cannot represent
    /// the next ID.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` amortized.
    pub fn add_node(
        &mut self,
        weight: EW,
    ) -> Result<GraphNodeId<NodeIndex>, GraphBuildError<NodeIndex, EdgeIndex>> {
        let id = NodeIndex::from_usize(self.node_count).ok_or(GraphBuildError::IdOverflow {
            value: self.node_count,
        })?;
        self.node_count = self
            .node_count
            .checked_add(1)
            .ok_or(GraphBuildError::IdOverflow { value: usize::MAX })?;
        self.element_weights.push(weight);
        Ok(GraphNodeId(id))
    }

    /// Adds a directed edge with an explicit relation weight.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::InvalidNode`] when either endpoint is not
    /// visible, or [`GraphBuildError::IdOverflow`] if `EdgeIndex` cannot
    /// represent the next edge ID.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` amortized.
    pub fn add_edge(
        &mut self,
        source: GraphNodeId<NodeIndex>,
        target: GraphNodeId<NodeIndex>,
        weight: RW,
    ) -> Result<GraphEdgeId<EdgeIndex>, GraphBuildError<NodeIndex, EdgeIndex>> {
        ensure_node(source, self.node_count)?;
        ensure_node(target, self.node_count)?;
        let edge =
            EdgeIndex::from_usize(self.sources.len()).ok_or(GraphBuildError::IdOverflow {
                value: self.sources.len(),
            })?;
        self.sources.push(source.0);
        self.targets.push(target.0);
        self.relation_weights.push(weight);
        Ok(GraphEdgeId(edge))
    }

    /// Updates an existing element weight.
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
        node: GraphNodeId<NodeIndex>,
        weight: EW,
    ) -> Result<(), GraphBuildError<NodeIndex, EdgeIndex>> {
        let slot = ensure_node(node, self.node_count)?;
        self.element_weights[slot] = weight;
        Ok(())
    }

    /// Updates an existing relation weight.
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
        edge: GraphEdgeId<EdgeIndex>,
        weight: RW,
    ) -> Result<(), GraphBuildError<NodeIndex, EdgeIndex>> {
        let slot = ensure_edge(edge, self.sources.len())?;
        self.relation_weights[slot] = weight;
        Ok(())
    }

    /// Returns the number of nodes assigned so far.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn node_count(&self) -> usize {
        self.node_count
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

    /// Freezes the current builder contents into an immutable weighted graph.
    ///
    /// # Errors
    ///
    /// Returns [`GraphBuildError::IdOverflow`] if a CSR offset or edge ID does
    /// not fit the selected `EdgeIndex` width.
    ///
    /// # Performance
    ///
    /// This function is `O(n + m)`.
    pub fn freeze(&self) -> FrozenWeightedGraphResult<NodeIndex, EdgeIndex, EW, RW>
    where
        EW: Clone,
        RW: Clone,
    {
        Ok(FrozenWeightedGraph {
            topology: freeze_topology(self.node_count, &self.sources, &self.targets)?,
            element_weights: self.element_weights.clone().into_boxed_slice(),
            relation_weights: self.relation_weights.clone().into_boxed_slice(),
        })
    }
}

impl<NodeIndex, EdgeIndex, EW, RW> Default for WeightedGraphBuilder<NodeIndex, EdgeIndex, EW, RW>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Owned immutable graph topology produced by [`GraphBuilder::freeze`].
///
/// # Performance
///
/// Count, containment, index, endpoint, and traversal methods are `O(1)` or
/// `O(out_degree)` for outgoing iteration.
#[derive(Clone, Debug)]
#[must_use]
pub struct FrozenGraph<NodeIndex, EdgeIndex>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    /// Shared frozen topology.
    topology: FrozenTopology<NodeIndex, EdgeIndex>,
}

/// Owned immutable weighted graph topology.
///
/// # Performance
///
/// Weight lookups are `O(1)`.
#[derive(Clone, Debug)]
#[must_use]
pub struct FrozenWeightedGraph<NodeIndex, EdgeIndex, EW, RW>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    /// Shared frozen topology.
    topology: FrozenTopology<NodeIndex, EdgeIndex>,
    /// Element weights by canonical node ID.
    element_weights: Box<[EW]>,
    /// Relation weights by canonical edge ID.
    relation_weights: Box<[RW]>,
}

/// Shared frozen graph topology storage.
#[derive(Clone, Debug)]
struct FrozenTopology<NodeIndex, EdgeIndex>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    /// Node count.
    node_count: usize,
    /// CSR offsets by source node.
    offsets: Box<[EdgeIndex]>,
    /// Edge IDs grouped by source node.
    edge_ids: Box<[EdgeIndex]>,
    /// Edge targets grouped by source node.
    targets: Box<[NodeIndex]>,
    /// Edge sources by canonical edge ID.
    sources: Box<[NodeIndex]>,
    /// Edge targets by canonical edge ID.
    edge_targets: Box<[NodeIndex]>,
}

impl<NodeIndex, EdgeIndex> FrozenGraph<NodeIndex, EdgeIndex>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    /// Returns the CSR offsets array.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub fn offsets(&self) -> &[EdgeIndex] {
        &self.topology.offsets
    }

    /// Returns the CSR target array grouped by source.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub fn targets(&self) -> &[NodeIndex] {
        &self.topology.targets
    }

    /// Returns the local-to-canonical edge map in CSR order.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub fn edge_ids(&self) -> &[EdgeIndex] {
        &self.topology.edge_ids
    }
}

impl<NodeIndex, EdgeIndex, EW, RW> FrozenWeightedGraph<NodeIndex, EdgeIndex, EW, RW>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    /// Returns the CSR offsets array.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub fn offsets(&self) -> &[EdgeIndex] {
        &self.topology.offsets
    }

    /// Returns the CSR target array grouped by source.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub fn targets(&self) -> &[NodeIndex] {
        &self.topology.targets
    }

    /// Returns the local-to-canonical edge map in CSR order.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub fn edge_ids(&self) -> &[EdgeIndex] {
        &self.topology.edge_ids
    }
}

/// Implements graph topology traits for one frozen graph wrapper.
macro_rules! impl_topology_for {
    ($name:ident, [$($extra:ident),*], $topology:ident) => {
        impl<NodeIndex, EdgeIndex$(, $extra)*> TopologyBase for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            type ElementId = GraphNodeId<NodeIndex>;
            type RelationId = GraphEdgeId<EdgeIndex>;
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> TopologyCounts for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            fn element_count(&self) -> usize {
                self.$topology.node_count
            }

            fn relation_count(&self) -> usize {
                self.$topology.sources.len()
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> GraphCounts for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> ElementIndex for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            fn element_bound(&self) -> usize {
                self.$topology.node_count
            }

            fn element_index(&self, element: GraphNodeId<NodeIndex>) -> usize {
                element.0.to_usize().unwrap_or(usize::MAX)
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> RelationIndex for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            fn relation_bound(&self) -> usize {
                self.$topology.sources.len()
            }

            fn relation_index(&self, relation: GraphEdgeId<EdgeIndex>) -> usize {
                relation.0.to_usize().unwrap_or(usize::MAX)
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> ContainsElement for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            fn contains_element(&self, element: GraphNodeId<NodeIndex>) -> bool {
                element
                    .0
                    .to_usize()
                    .is_some_and(|slot| slot < self.$topology.node_count)
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> ContainsRelation for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            fn contains_relation(&self, relation: GraphEdgeId<EdgeIndex>) -> bool {
                relation
                    .0
                    .to_usize()
                    .is_some_and(|slot| slot < self.$topology.sources.len())
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> EdgeSourceGraph for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            fn source(&self, edge: GraphEdgeId<EdgeIndex>) -> GraphNodeId<NodeIndex> {
                GraphNodeId(self.$topology.sources[edge_slot(edge)])
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> EdgeTargetGraph for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            fn target(&self, edge: GraphEdgeId<EdgeIndex>) -> GraphNodeId<NodeIndex> {
                GraphNodeId(self.$topology.edge_targets[edge_slot(edge)])
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> OutgoingGraph for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            type OutEdges<'view>
                = FrozenOutEdges<'view, EdgeIndex>
            where
                Self: 'view;

            fn outgoing_edges(&self, node: GraphNodeId<NodeIndex>) -> Self::OutEdges<'_> {
                FrozenOutEdges {
                    inner: self.$topology.edge_ids[outgoing_range(&self.$topology, node)].iter(),
                }
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> OutgoingEdgeCount for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            fn out_degree(&self, node: GraphNodeId<NodeIndex>) -> usize {
                outgoing_range(&self.$topology, node).len()
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> ElementSuccessors for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            type Successors<'view>
                = FrozenSuccessors<'view, EdgeIndex, NodeIndex>
            where
                Self: 'view;

            fn element_successors(&self, node: GraphNodeId<NodeIndex>) -> Self::Successors<'_> {
                FrozenSuccessors {
                    edge_ids: self.$topology.edge_ids[outgoing_range(&self.$topology, node)].iter(),
                    targets_by_edge: &self.$topology.edge_targets,
                }
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> CanonicalElementIdentity for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            type CanonicalElementId = GraphNodeId<NodeIndex>;

            fn canonical_element_id(
                &self,
                element: GraphNodeId<NodeIndex>,
            ) -> Self::CanonicalElementId {
                element
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> LocalElementIdentity for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            fn local_element_id(
                &self,
                canonical: Self::CanonicalElementId,
            ) -> Option<Self::ElementId> {
                self.contains_element(canonical).then_some(canonical)
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> CanonicalRelationIdentity for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            type CanonicalRelationId = GraphEdgeId<EdgeIndex>;

            fn canonical_relation_id(
                &self,
                relation: GraphEdgeId<EdgeIndex>,
            ) -> Self::CanonicalRelationId {
                relation
            }
        }

        impl<NodeIndex, EdgeIndex$(, $extra)*> LocalRelationIdentity for $name<NodeIndex, EdgeIndex$(, $extra)*>
        where
            NodeIndex: BuildIndex,
            EdgeIndex: BuildIndex,
        {
            fn local_relation_id(
                &self,
                canonical: Self::CanonicalRelationId,
            ) -> Option<Self::RelationId> {
                self.contains_relation(canonical).then_some(canonical)
            }
        }
    };
}

impl_topology_for!(FrozenGraph, [], topology);
impl_topology_for!(FrozenWeightedGraph, [EW, RW], topology);

impl<NodeIndex, EdgeIndex, EW: Copy, RW> ElementWeight
    for FrozenWeightedGraph<NodeIndex, EdgeIndex, EW, RW>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    type Weight = EW;

    fn element_weight(&self, element: GraphNodeId<NodeIndex>) -> Self::Weight {
        self.element_weights[node_slot(element)]
    }
}

impl<NodeIndex, EdgeIndex, EW, RW: Copy> RelationWeight
    for FrozenWeightedGraph<NodeIndex, EdgeIndex, EW, RW>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    type Weight = RW;

    fn relation_weight(&self, relation: GraphEdgeId<EdgeIndex>) -> Self::Weight {
        self.relation_weights[edge_slot(relation)]
    }
}

/// Iterator over frozen outgoing edge IDs.
///
/// # Performance
///
/// Advancing is `O(1)`.
pub struct FrozenOutEdges<'view, EdgeIndex> {
    /// Borrowed edge IDs.
    inner: core::slice::Iter<'view, EdgeIndex>,
}

impl<EdgeIndex: Copy> Iterator for FrozenOutEdges<'_, EdgeIndex> {
    type Item = GraphEdgeId<EdgeIndex>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().copied().map(GraphEdgeId)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<EdgeIndex: Copy> ExactSizeIterator for FrozenOutEdges<'_, EdgeIndex> {}

/// Iterator over frozen outgoing successor nodes.
///
/// # Performance
///
/// Advancing is `O(1)` after iterator construction.
pub struct FrozenSuccessors<'view, EdgeIndex, NodeIndex> {
    /// Borrowed outgoing edge IDs.
    edge_ids: core::slice::Iter<'view, EdgeIndex>,
    /// Edge-ID-indexed target vector.
    targets_by_edge: &'view [NodeIndex],
}

impl<EdgeIndex, NodeIndex> Iterator for FrozenSuccessors<'_, EdgeIndex, NodeIndex>
where
    EdgeIndex: BuildIndex,
    NodeIndex: Copy,
{
    type Item = GraphNodeId<NodeIndex>;

    fn next(&mut self) -> Option<Self::Item> {
        self.edge_ids
            .next()
            .copied()
            .map(|edge| GraphNodeId(self.targets_by_edge[edge.to_usize().unwrap_or(usize::MAX)]))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.edge_ids.size_hint()
    }
}

impl<EdgeIndex, NodeIndex> ExactSizeIterator for FrozenSuccessors<'_, EdgeIndex, NodeIndex>
where
    EdgeIndex: BuildIndex,
    NodeIndex: Copy,
{
}

/// Freezes source/target vectors into CSR order.
fn freeze_topology<NodeIndex, EdgeIndex>(
    node_count: usize,
    sources: &[NodeIndex],
    targets: &[NodeIndex],
) -> Result<FrozenTopology<NodeIndex, EdgeIndex>, GraphBuildError<NodeIndex, EdgeIndex>>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    let mut counts = vec![0_usize; node_count + 1];
    for &source in sources {
        let slot = source
            .to_usize()
            .ok_or(GraphBuildError::IdOverflow { value: usize::MAX })?;
        counts[slot + 1] = counts[slot + 1]
            .checked_add(1)
            .ok_or(GraphBuildError::IdOverflow { value: usize::MAX })?;
    }
    for index in 1..counts.len() {
        counts[index] = counts[index]
            .checked_add(counts[index - 1])
            .ok_or(GraphBuildError::IdOverflow { value: usize::MAX })?;
    }
    let offsets = counts
        .iter()
        .copied()
        .map(|value| EdgeIndex::from_usize(value).ok_or(GraphBuildError::IdOverflow { value }))
        .collect::<Result<Vec<_>, _>>()?;
    let mut cursor = counts;
    let mut edge_ids = vec![edge_zero::<NodeIndex, EdgeIndex>()?; sources.len()];
    let mut grouped_targets = vec![node_zero::<NodeIndex, EdgeIndex>()?; targets.len()];
    for (edge_index, (&source, &target)) in sources.iter().zip(targets).enumerate() {
        let source_slot = source
            .to_usize()
            .ok_or(GraphBuildError::IdOverflow { value: usize::MAX })?;
        let position = cursor[source_slot];
        cursor[source_slot] = cursor[source_slot]
            .checked_add(1)
            .ok_or(GraphBuildError::IdOverflow { value: usize::MAX })?;
        edge_ids[position] = EdgeIndex::from_usize(edge_index)
            .ok_or(GraphBuildError::IdOverflow { value: edge_index })?;
        grouped_targets[position] = target;
    }
    Ok(FrozenTopology {
        node_count,
        offsets: offsets.into_boxed_slice(),
        edge_ids: edge_ids.into_boxed_slice(),
        targets: grouped_targets.into_boxed_slice(),
        sources: sources.to_vec().into_boxed_slice(),
        edge_targets: targets.to_vec().into_boxed_slice(),
    })
}

/// Returns zero for an edge index width.
fn edge_zero<NodeIndex, EdgeIndex>() -> Result<EdgeIndex, GraphBuildError<NodeIndex, EdgeIndex>>
where
    EdgeIndex: BuildIndex,
{
    EdgeIndex::from_usize(0).ok_or(GraphBuildError::IdOverflow { value: 0 })
}

/// Returns zero for a node index width.
fn node_zero<NodeIndex, EdgeIndex>() -> Result<NodeIndex, GraphBuildError<NodeIndex, EdgeIndex>>
where
    NodeIndex: BuildIndex,
{
    NodeIndex::from_usize(0).ok_or(GraphBuildError::IdOverflow { value: 0 })
}

/// Validates a node ID and returns its dense slot.
fn ensure_node<NodeIndex, EdgeIndex>(
    node: GraphNodeId<NodeIndex>,
    node_count: usize,
) -> Result<usize, GraphBuildError<NodeIndex, EdgeIndex>>
where
    NodeIndex: BuildIndex,
{
    let Some(slot) = node.0.to_usize() else {
        return Err(GraphBuildError::InvalidNode { node });
    };
    if slot < node_count {
        Ok(slot)
    } else {
        Err(GraphBuildError::InvalidNode { node })
    }
}

/// Validates an edge ID and returns its dense slot.
fn ensure_edge<NodeIndex, EdgeIndex>(
    edge: GraphEdgeId<EdgeIndex>,
    edge_count: usize,
) -> Result<usize, GraphBuildError<NodeIndex, EdgeIndex>>
where
    EdgeIndex: BuildIndex,
{
    let Some(slot) = edge.0.to_usize() else {
        return Err(GraphBuildError::InvalidEdge { edge });
    };
    if slot < edge_count {
        Ok(slot)
    } else {
        Err(GraphBuildError::InvalidEdge { edge })
    }
}

/// Returns a node slot without panicking.
fn node_slot<NodeIndex: BuildIndex>(node: GraphNodeId<NodeIndex>) -> usize {
    node.0.to_usize().unwrap_or(usize::MAX)
}

/// Returns an edge slot without panicking.
fn edge_slot<EdgeIndex: BuildIndex>(edge: GraphEdgeId<EdgeIndex>) -> usize {
    edge.0.to_usize().unwrap_or(usize::MAX)
}

/// Returns the outgoing edge range for `node`.
fn outgoing_range<NodeIndex, EdgeIndex>(
    topology: &FrozenTopology<NodeIndex, EdgeIndex>,
    node: GraphNodeId<NodeIndex>,
) -> core::ops::Range<usize>
where
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    let slot = node_slot(node);
    let start = topology.offsets[slot]
        .to_usize()
        .unwrap_or(topology.edge_ids.len());
    let end = topology.offsets[slot + 1]
        .to_usize()
        .unwrap_or(topology.edge_ids.len());
    start..end
}

/// Snapshot export index with local little-endian conversion.
#[cfg(feature = "snapshot")]
pub trait GraphSnapshotIndex: BuildIndex + CsrSnapshotIndex {
    /// Converts this value into its little-endian CSR/property word.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn to_le_word(self) -> Self::LittleEndianWord;
}

#[cfg(feature = "snapshot")]
impl GraphSnapshotIndex for u16 {
    fn to_le_word(self) -> Self::LittleEndianWord {
        U16::<LE>::new(self)
    }
}

#[cfg(feature = "snapshot")]
impl GraphSnapshotIndex for u32 {
    fn to_le_word(self) -> Self::LittleEndianWord {
        U32::<LE>::new(self)
    }
}

#[cfg(feature = "snapshot")]
impl GraphSnapshotIndex for u64 {
    fn to_le_word(self) -> Self::LittleEndianWord {
        U64::<LE>::new(self)
    }
}

/// Converts a CSR payload slice into explicit little-endian words.
///
/// # Performance
///
/// This function is `O(values.len())`.
#[cfg(feature = "snapshot")]
pub fn csr_slice_to_le<I>(values: &[I]) -> Vec<I::LittleEndianWord>
where
    I: GraphSnapshotIndex,
{
    values
        .iter()
        .copied()
        .map(GraphSnapshotIndex::to_le_word)
        .collect()
}

/// Converts an identity-map payload slice into explicit little-endian words.
///
/// # Performance
///
/// This function is `O(values.len())`.
#[cfg(feature = "snapshot")]
pub fn identity_slice_to_le<I>(values: &[I]) -> Vec<I::LittleEndianWord>
where
    I: GraphSnapshotIndex,
{
    csr_slice_to_le(values)
}

/// Exports a frozen graph topology and identity metadata as a CSR snapshot.
///
/// # Errors
///
/// Returns [`GraphBuildError`] if snapshot planning fails or the identity
/// metadata width cannot represent the graph counts.
///
/// # Performance
///
/// This function is `O(n + m)`.
#[cfg(feature = "snapshot")]
pub fn export_csr_snapshot<NodeIndex, EdgeIndex>(
    graph: &FrozenGraph<NodeIndex, EdgeIndex>,
) -> Result<Vec<u8>, GraphBuildError<NodeIndex, EdgeIndex>>
where
    NodeIndex: GraphSnapshotIndex,
    EdgeIndex: GraphSnapshotIndex,
{
    export_topology_snapshot(&graph.topology)
}

/// Exports a frozen weighted graph topology and identity metadata as a CSR snapshot.
///
/// Weights are generic Rust values and are not serialized by the topology-only
/// snapshot feature; attach Arrow property layers through `property-arrow` when
/// weights need a wire representation.
///
/// # Errors
///
/// Returns [`GraphBuildError`] if snapshot planning fails.
///
/// # Performance
///
/// This function is `O(n + m)`.
#[cfg(feature = "snapshot")]
pub fn export_weighted_csr_snapshot<NodeIndex, EdgeIndex, EW, RW>(
    graph: &FrozenWeightedGraph<NodeIndex, EdgeIndex, EW, RW>,
) -> Result<Vec<u8>, GraphBuildError<NodeIndex, EdgeIndex>>
where
    NodeIndex: GraphSnapshotIndex,
    EdgeIndex: GraphSnapshotIndex,
{
    export_topology_snapshot(&graph.topology)
}

/// Exports a frozen graph with external property layers.
///
/// # Errors
///
/// Returns [`GraphBuildError`] if property validation, rekeying, encoding, or
/// snapshot planning fails.
///
/// # Performance
///
/// This function is `O(n + m + property bytes)`.
#[cfg(feature = "property-arrow")]
pub fn export_csr_snapshot_with_properties<NodeIndex, EdgeIndex, Id>(
    graph: &FrozenGraph<NodeIndex, EdgeIndex>,
    layers: GraphPropertyLayers<'_, Id, NodeIndex, EdgeIndex>,
) -> Result<Vec<u8>, GraphBuildError<NodeIndex, EdgeIndex>>
where
    NodeIndex: GraphSnapshotIndex + oxgraph_property::PropertyIndex,
    EdgeIndex: GraphSnapshotIndex + PropertySnapshotMetaWord,
    Id: Clone + Copy + Into<u64> + Ord + TryInto<EdgeIndex>,
{
    let property = encode_graph_properties(&graph.topology, layers)?;
    export_topology_property_snapshot(&graph.topology, property)
}

/// Exports a frozen weighted graph with external property layers.
///
/// # Errors
///
/// Returns [`GraphBuildError`] if property validation, rekeying, encoding, or
/// snapshot planning fails.
///
/// # Performance
///
/// This function is `O(n + m + property bytes)`.
#[cfg(feature = "property-arrow")]
pub fn export_weighted_csr_snapshot_with_properties<NodeIndex, EdgeIndex, EW, RW, Id>(
    graph: &FrozenWeightedGraph<NodeIndex, EdgeIndex, EW, RW>,
    layers: GraphPropertyLayers<'_, Id, NodeIndex, EdgeIndex>,
) -> Result<Vec<u8>, GraphBuildError<NodeIndex, EdgeIndex>>
where
    NodeIndex: GraphSnapshotIndex + oxgraph_property::PropertyIndex,
    EdgeIndex: GraphSnapshotIndex + PropertySnapshotMetaWord,
    Id: Clone + Copy + Into<u64> + Ord + TryInto<EdgeIndex>,
{
    let property = encode_graph_properties(&graph.topology, layers)?;
    export_topology_property_snapshot(&graph.topology, property)
}

/// Encodes graph property layers after relation rekeying.
#[cfg(feature = "property-arrow")]
fn encode_graph_properties<NodeIndex, EdgeIndex, Id>(
    topology: &FrozenTopology<NodeIndex, EdgeIndex>,
    layers: GraphPropertyLayers<'_, Id, NodeIndex, EdgeIndex>,
) -> Result<EncodedPropertySnapshot, GraphBuildError<NodeIndex, EdgeIndex>>
where
    NodeIndex: GraphSnapshotIndex + oxgraph_property::PropertyIndex,
    EdgeIndex: GraphSnapshotIndex + PropertySnapshotMetaWord,
    Id: Clone + Copy + Into<u64> + Ord + TryInto<EdgeIndex>,
{
    validate_property_lengths(topology, layers.element, IdFamily::Element)?;
    validate_property_lengths(topology, layers.relation, IdFamily::Relation)?;
    let relation = layers
        .relation
        .iter()
        .map(|layer| rekey_layer_to_local(layer, &topology.edge_ids).map_err(GraphBuildError::from))
        .collect::<Result<Vec<PropertyLayer<Id, EdgeIndex>>, _>>()?;
    Ok(encode_graph_property_snapshot::<
        EdgeIndex,
        Id,
        NodeIndex,
        EdgeIndex,
    >(GraphPropertyLayers {
        element: layers.element,
        relation: &relation,
    })?)
}

/// Validates property layers against topology counts.
#[cfg(feature = "property-arrow")]
#[expect(
    clippy::match_same_arms,
    reason = "non-exhaustive IdFamily requires a wildcard; graph snapshots treat unsupported families as empty"
)]
fn validate_property_lengths<Id, I, NodeIndex, EdgeIndex>(
    topology: &FrozenTopology<NodeIndex, EdgeIndex>,
    layers: &[PropertyLayer<Id, I>],
    id_family: IdFamily,
) -> Result<(), GraphBuildError<NodeIndex, EdgeIndex>>
where
    I: oxgraph_property::PropertyIndex,
    NodeIndex: BuildIndex,
    EdgeIndex: BuildIndex,
{
    let required = match id_family {
        IdFamily::Element => topology.node_count,
        IdFamily::Relation => topology.sources.len(),
        IdFamily::Incidence => 0,
        _ => 0,
    };
    for layer in layers {
        if layer.len() < required {
            return Err(GraphBuildError::PropertyLayerTooShort {
                id_family,
                required,
                actual: layer.len(),
            });
        }
    }
    Ok(())
}

/// Exports topology.
#[cfg(feature = "snapshot")]
fn export_topology_snapshot<NodeIndex, EdgeIndex>(
    topology: &FrozenTopology<NodeIndex, EdgeIndex>,
) -> Result<Vec<u8>, GraphBuildError<NodeIndex, EdgeIndex>>
where
    NodeIndex: GraphSnapshotIndex,
    EdgeIndex: GraphSnapshotIndex,
{
    let offsets = csr_slice_to_le(&topology.offsets);
    let targets = csr_slice_to_le(&topology.targets);
    let mut builder = SnapshotBuilder::new();
    builder.add_section_little_endian(EdgeIndex::OFFSETS_KIND, 1, &offsets)?;
    builder.add_section_little_endian(NodeIndex::TARGETS_KIND, 1, &targets)?;
    builder.finish().map_err(GraphBuildError::from)
}

/// Exports topology, identity, and encoded property payloads.
#[cfg(feature = "property-arrow")]
fn export_topology_property_snapshot<NodeIndex, EdgeIndex>(
    topology: &FrozenTopology<NodeIndex, EdgeIndex>,
    property: EncodedPropertySnapshot,
) -> Result<Vec<u8>, GraphBuildError<NodeIndex, EdgeIndex>>
where
    NodeIndex: GraphSnapshotIndex,
    EdgeIndex: GraphSnapshotIndex + PropertySnapshotMetaWord,
{
    let offsets = csr_slice_to_le(&topology.offsets);
    let targets = csr_slice_to_le(&topology.targets);
    let edge_ids = property_identity_slice_to_le::<EdgeIndex>(&topology.edge_ids);
    let identity_modes = [
        IdentityModeRecord::<EdgeIndex>::local_equals_canonical(
            IdFamily::Element,
            topology.node_count,
        )?,
        IdentityModeRecord::<EdgeIndex>::explicit_map(IdFamily::Relation, topology.edge_ids.len())?,
    ];
    let mut builder = SnapshotBuilder::new();
    builder.add_section_little_endian(EdgeIndex::OFFSETS_KIND, 1, &offsets)?;
    builder.add_section_little_endian(NodeIndex::TARGETS_KIND, 1, &targets)?;
    builder.add_section_little_endian(
        EdgeIndex::IDENTITY_MODES_KIND,
        SNAPSHOT_PROPERTY_VERSION,
        &identity_modes,
    )?;
    builder.add_section_little_endian(
        EdgeIndex::RELATION_IDENTITY_MAP_KIND,
        SNAPSHOT_PROPERTY_VERSION,
        &edge_ids,
    )?;
    builder.add_section(
        EdgeIndex::PROPERTY_DESCRIPTORS_KIND,
        SNAPSHOT_PROPERTY_VERSION,
        0,
        property.descriptors,
    )?;
    builder.add_section(
        EdgeIndex::PROPERTY_DATA_KIND,
        SNAPSHOT_PROPERTY_VERSION,
        0,
        property.data,
    )?;
    builder.finish().map_err(GraphBuildError::from)
}

/// Converts an identity-map payload slice into explicit little-endian words.
#[cfg(feature = "property-arrow")]
fn property_identity_slice_to_le<I>(
    values: &[I],
) -> Vec<<I as oxgraph_property::PropertyIndex>::LittleEndianWord>
where
    I: PropertySnapshotMetaWord,
{
    values
        .iter()
        .copied()
        .map(oxgraph_property::PropertyIndex::to_le_word)
        .collect()
}

#[cfg(test)]
mod tests {
    //! Tests for graph builder freeze semantics.

    use alloc::vec;

    use super::*;

    /// Builder preserves isolated nodes and parallel edges.
    #[test]
    fn freeze_preserves_parallel_edges() -> Result<(), GraphBuildError<u32, u32>> {
        let mut builder = GraphBuilder::<u32, u32>::new();
        let a = builder.add_node()?;
        let b = builder.add_node()?;
        let first = builder.add_edge(a, b)?;
        let second = builder.add_edge(a, b)?;
        let frozen = builder.freeze()?;
        assert_eq!(frozen.element_count(), 2);
        assert_eq!(frozen.relation_count(), 2);
        assert_eq!(
            frozen.outgoing_edges(a).collect::<Vec<_>>(),
            vec![first, second]
        );
        Ok(())
    }

    /// Weighted builders require explicit weights and preserve updates.
    #[test]
    fn weighted_builder_preserves_weights() -> Result<(), GraphBuildError<u32, u32>> {
        let mut builder = WeightedGraphBuilder::<u32, u32, i32, u16>::new();
        let a = builder.add_node(1_i32)?;
        let b = builder.add_node(2_i32)?;
        let edge = builder.add_edge(a, b, 3_u16)?;
        builder.set_element_weight(a, 4_i32)?;
        builder.set_relation_weight(edge, 5_u16)?;
        let frozen = builder.freeze()?;
        assert_eq!(frozen.element_weight(a), 4_i32);
        assert_eq!(frozen.relation_weight(edge), 5_u16);
        Ok(())
    }

    /// Mixed node/edge widths compile and freeze.
    #[test]
    fn mixed_width_builder_freezes() -> Result<(), GraphBuildError<u32, u64>> {
        let mut builder = GraphBuilder::<u32, u64>::new();
        let a = builder.add_node()?;
        let b = builder.add_node()?;
        let edge = builder.add_edge(a, b)?;
        let frozen = builder.freeze()?;
        assert_eq!(frozen.relation_index(edge), 0);
        assert_eq!(frozen.element_successors(a).collect::<Vec<_>>(), vec![b]);
        Ok(())
    }

    /// Snapshot export writes little-endian CSR sections.
    #[cfg(feature = "snapshot")]
    #[test]
    fn snapshot_export_roundtrips_topology() -> Result<(), Box<dyn Error>> {
        use oxgraph_csr::CsrSnapshotGraph;
        use oxgraph_snapshot::Snapshot;

        let mut builder = GraphBuilder::<u32, u32>::new();
        let a = builder.add_node()?;
        let b = builder.add_node()?;
        builder.add_edge(a, b)?;
        let frozen = builder.freeze()?;
        let bytes = export_csr_snapshot(&frozen)?;
        let snapshot = Snapshot::open(&bytes)?;
        let graph = CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot)?;
        assert_eq!(
            graph
                .element_successors(oxgraph_csr::CsrNodeId(0))
                .collect::<Vec<_>>(),
            vec![oxgraph_csr::CsrNodeId(1)]
        );
        Ok(())
    }
}
