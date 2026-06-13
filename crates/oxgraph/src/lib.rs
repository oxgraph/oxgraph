//! Curated feature-gated re-exports for `OxGraph`.
//!
//! The umbrella crate does not add helper APIs that kernel crates lack. It only
//! collects the workspace surface behind explicit feature flags so consumers can
//! choose the layers they need.
#![no_std]

#[cfg(kani)]
extern crate kani;

/// Shared layout index/word vocabulary.
///
/// Re-exports the canonical dense-index, storage-word, and local-handle
/// vocabulary every concrete layout and build crate is parameterized over.
/// Concrete layout features (`csr`, `hyper-bcsr`, `graph-build`, `hyper-build`,
/// `topology`) enable this feature transitively, so the index/word traits stay
/// reachable wherever a layout type is.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "layout")]
pub mod layout {
    pub use oxgraph_layout_util::{
        Axis, EdgeAxis, HyperedgeAxis, IdSlice, IncidenceAxis, LayoutIndex, LayoutSnapshotWord,
        LayoutWord, LocalId, NodeAxis, SnapshotWidth, VertexAxis, ZerocopyWord, build, integrity,
    };
}

/// Substrate-neutral topology capability traits.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "topology")]
pub mod topology {
    pub use oxgraph_topology::{
        CanonicalElementIdentity, CanonicalIncidenceIdentity, CanonicalRelationIdentity,
        ContainsElement, ContainsIncidence, ContainsRelation, DenseElementIndex,
        DenseIncidenceIndex, DenseRelationIndex, ElementId, ElementIncidenceCount,
        ElementIncidences, ElementPredecessors, ElementSuccessors, ElementWeight, IncidenceBase,
        IncidenceCounts, IncidenceElement, IncidenceRelation, IncidenceRole, IncidenceView,
        IncidenceWeight, LocalElementIdentity, LocalIncidenceIdentity, LocalRelationIdentity,
        RelationId, RelationIncidenceCount, RelationIncidences, RelationWeight, TopologyBase,
        TopologyCounts, TopologyId,
    };
}

/// Binary graph vocabulary traits.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "graph")]
pub mod graph {
    pub use oxgraph_graph::{
        ContainsEdge, ContainsEndpoint, ContainsNode, DenseEdgeIndex, DenseEndpointIndex,
        DenseNodeIndex, DirectedGraph, EdgeEndpointGraph, EdgeId, EdgeSourceGraph, EdgeTargetGraph,
        EndpointId, EndpointRole, ForwardGraph, GraphBase, GraphCounts, IncomingEdgeCount,
        IncomingGraph, IncomingNeighborsGraph, NodeId, OutgoingEdgeCount, OutgoingGraph,
        OutgoingNeighborsGraph, ReverseGraph,
    };
}

/// Hypergraph vocabulary traits.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "hyper")]
pub mod hyper {
    pub use oxgraph_hyper::{
        ContainsHyperedge, ContainsParticipant, ContainsVertex, DenseHyperedgeIndex,
        DenseParticipantIndex, DenseVertexIndex, DirectedHyperedgeIncidences,
        DirectedHyperedgeParticipants, DirectedHypergraph, DirectedVertexHyperedges,
        DirectedVertexPredecessors, DirectedVertexSuccessors, HyperedgeId, HyperedgeIncidences,
        HyperedgeParticipantCount, HyperedgeParticipants, Hypergraph, HypergraphBase,
        HypergraphCounts, IncidentHyperedgeCount, IncidentHyperedges, ParticipantBase,
        ParticipantCounts, ParticipantHyperedge, ParticipantId, ParticipantRole, ParticipantRoleOf,
        ParticipantVertex, VertexId, VertexIncidences,
    };
}

/// Borrowed compressed-sparse-column (inbound) graph views.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "csc")]
pub mod csc {
    pub use oxgraph_csc::{CscNodeId, CscSnapshotError, CscSnapshotGraph};
}

/// Borrowed CSR graph layout.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "csr")]
pub mod csr {
    // The shared index/word vocabulary (`LayoutIndex`, `LayoutWord`,
    // `LayoutSnapshotWord`) lives at [`crate::layout`]; the `csr` feature enables
    // it. This module re-exports only CSR-specific items.
    pub use oxgraph_csr::{
        CsrEdgeId, CsrError, CsrGraph, CsrNativeGraph, CsrNodeId, CsrOutEdges, CsrSnapshotError,
        CsrSnapshotGraph, CsrSnapshotIndex, SNAPSHOT_CSR_SECTION_VERSION,
        SNAPSHOT_KIND_CSR_OFFSETS_BASE, SNAPSHOT_KIND_CSR_TARGETS_BASE,
    };
}

/// Borrowed bipartite-CSR hypergraph layout.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "hyper-bcsr")]
pub mod hyper_bcsr {
    // The shared index/word vocabulary (`LayoutIndex`, `LayoutWord`,
    // `LayoutSnapshotWord`) lives at [`crate::layout`]; the `hyper-bcsr` feature
    // enables it. This module re-exports only BCSR-specific items.
    pub use oxgraph_hyper_bcsr::{
        BcsrChainedHyperedges, BcsrChainedParticipants, BcsrChainedRelationIncidences,
        BcsrElementIncidences, BcsrError, BcsrHyperedgeId, BcsrHyperedgeSlice, BcsrHypergraph,
        BcsrNativeHypergraph, BcsrParticipantId, BcsrParticipantSlice, BcsrPredecessorVertices,
        BcsrRole, BcsrRoleSide, BcsrSection, BcsrSections, BcsrSnapshotError,
        BcsrSnapshotHypergraph, BcsrSnapshotIndex, BcsrSuccessorVertices, BcsrValidation,
        BcsrVertexId, BcsrVertexSlice, BcsrWords, LeWords, NativeWords,
        SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_BASE, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_BASE,
        SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_BASE, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_BASE,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_BASE,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_BASE,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_BASE,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_BASE,
    };
}

/// Topology-agnostic snapshot container.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "snapshot")]
pub mod snapshot {
    pub use oxgraph_snapshot::{
        Checksum32, FORMAT_MAGIC, FORMAT_MAJOR, FORMAT_MINOR, HEADER_SIZE, HeaderOnlySnapshot,
        MAX_ALIGNMENT_LOG2, MAX_SECTION_COUNT, MAX_SUPPORTED_MINOR, PendingSection, PlanError,
        SECTION_ENTRY_SIZE, Section, SectionIter, SectionViewError, Snapshot, SnapshotError,
        SnapshotPlan, ValidationLevel,
    };
    #[cfg(feature = "snapshot-alloc")]
    pub use oxgraph_snapshot::{SectionSink, SnapshotWriter};
}

/// Substrate-agnostic algorithms.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
///
/// # Examples
///
/// Build a graph with `graph_build::GraphBuilder`, then order it with
/// `topological_sort`. A frozen graph implements the topology capability traits
/// directly, so it feeds straight into the algorithm — there is no need to
/// assemble CSR offset/target arrays by hand. When the graph has a cycle,
/// `topological_sort` reports only that one exists; recover its members by
/// running `strongly_connected_components` over the same elements.
///
/// ```
/// use oxgraph::{
///     algo::{ToposortError, strongly_connected_components, topological_sort},
///     graph_build::GraphBuilder,
/// };
///
/// // A diamond DAG: 0 -> 1 -> 3 and 0 -> 2 -> 3.
/// let mut builder = GraphBuilder::<u32, u32>::new();
/// let nodes: Vec<_> = (0..4)
///     .map(|_| builder.add_node())
///     .collect::<Result<_, _>>()?;
/// builder.add_edge(nodes[0], nodes[1])?;
/// builder.add_edge(nodes[0], nodes[2])?;
/// builder.add_edge(nodes[1], nodes[3])?;
/// builder.add_edge(nodes[2], nodes[3])?;
/// let dag = builder.freeze()?;
///
/// let order = topological_sort(&dag, &nodes)?;
/// assert_eq!(order.first(), Some(&nodes[0])); // dependency first
/// assert_eq!(order.last(), Some(&nodes[3])); // dependent last
///
/// // 0 -> 1 -> 2 -> 1: nodes 1 and 2 form a cycle.
/// let mut builder = GraphBuilder::<u32, u32>::new();
/// let cycle: Vec<_> = (0..3)
///     .map(|_| builder.add_node())
///     .collect::<Result<_, _>>()?;
/// builder.add_edge(cycle[0], cycle[1])?;
/// builder.add_edge(cycle[1], cycle[2])?;
/// builder.add_edge(cycle[2], cycle[1])?;
/// let graph = builder.freeze()?;
///
/// assert_eq!(topological_sort(&graph, &cycle), Err(ToposortError::Cycle));
///
/// // Keep components larger than one (a self-loop is a singleton with a
/// // self-edge) to recover the precise cycle members.
/// let cycles: Vec<_> = strongly_connected_components(&graph, &cycle)
///     .into_iter()
///     .filter(|component| component.len() > 1)
///     .collect();
/// assert_eq!(cycles.len(), 1);
/// assert_eq!(cycles[0].len(), 2); // nodes 1 and 2
///
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[cfg(feature = "algo")]
pub mod algo {
    pub use oxgraph_algo::{
        BfsBounds, BfsEpochScratch, BfsError, BfsVisitor, BreadthFirstSearchEpochScratch,
        BreadthFirstSearchScratch, HyperWeighted, HypergraphOutgoingDistribution,
        HypergraphPageRankScratch, IntoPageRankScalar, OutgoingDistribution, PageRankConfig,
        PageRankError, PageRankHypergraph, PageRankReport, PageRankScalar, PageRankScratch,
        ReverseBreadthFirstSearchEpochScratch, ReverseBreadthFirstSearchScratch, Uniform, Weighted,
        breadth_first_search_bounded, breadth_first_search_bounded_both,
        breadth_first_search_with_epoch_scratch, breadth_first_search_with_scratch,
        pagerank_graph_with_scratch, pagerank_hypergraph_with_scratch,
        reverse_breadth_first_search_bounded, reverse_breadth_first_search_with_epoch_scratch,
        reverse_breadth_first_search_with_scratch,
    };
    #[cfg(feature = "algo-alloc")]
    pub use oxgraph_algo::{
        BfsWorkspace, BreadthFirstSearch, BreadthFirstSearchWorkspace, GenericBreadthFirstSearch,
        GenericReverseBreadthFirstSearch, HypergraphPageRankWorkspace, LongestPathError,
        PageRankWorkspace, ReverseBreadthFirstSearch, ReverseBreadthFirstSearchWorkspace,
        ToposortError, breadth_first_search, breadth_first_search_generic,
        breadth_first_search_with_workspace, connected_components, longest_path_dag,
        pagerank_graph, pagerank_graph_with_workspace, pagerank_hypergraph,
        pagerank_hypergraph_with_workspace, reverse_breadth_first_search,
        reverse_breadth_first_search_generic, reverse_breadth_first_search_with_workspace,
        shortest_path_lengths, strongly_connected_components, topological_sort,
    };
    #[cfg(feature = "algo-std")]
    pub use oxgraph_algo::{
        HashBreadthFirstSearch, HashReverseBreadthFirstSearch, breadth_first_search_generic_hash,
        reverse_breadth_first_search_generic_hash,
    };
}

/// Arrow-backed property layers.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "property-arrow")]
pub mod property {
    pub use oxgraph_property::{
        DenseWeights, ElementAxis, EncodedPropertySnapshot, GraphPropertyLayers,
        HyperPropertyLayers, IdFamily, IdentityMapMode, IdentityModeRecord, IdentityModeSummary,
        IdentitySnapshotSummary, IncidenceAxis, LayerId, LayerName, LayerRole, MissingPolicy,
        PropertyAxis, PropertyError, PropertyIndex, PropertyLayer, PropertyLayerData,
        PropertyLayerDescriptor, PropertySnapshotMetaWord, PropertySnapshotRecord,
        PropertySnapshotSummary, RelationAxis, SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_BASE,
        SNAPSHOT_KIND_IDENTITY_MODES_BASE, SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_BASE,
        SNAPSHOT_KIND_PROPERTY_DATA_BASE, SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_BASE,
        SNAPSHOT_KIND_RELATION_IDENTITY_MAP_BASE, SNAPSHOT_PROPERTY_VERSION, SparseWeights,
        StorageMode, encode_graph_property_snapshot, encode_hyper_property_snapshot,
        encode_property_snapshot, rekey_layer_to_local, validate_identity_snapshot,
        validate_property_sections, validate_property_snapshot, validate_unique_layer_ids,
        validate_unique_names,
    };
}

/// Embedded OxGraph-native database primitives.
///
/// This module is the ergonomic library entry point for applications that build
/// databases directly from the umbrella crate. It re-exports the embedded engine
/// and its catalog, query, projection, property, and identity types; server and
/// CLI concerns remain in the separate `oxgraphd` package.
///
/// # Examples
///
/// ```no_run
/// use oxgraph::db::Db;
///
/// fn main() -> Result<(), oxgraph::db::DbError> {
///     let database = Db::create("target/example.oxgdb")?;
///     let prepared = database.prepare("MATCH ELEMENTS")?;
///     let _rows = database.reader().run(&prepared)?;
///     Ok(())
/// }
/// ```
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "db")]
pub mod db {
    pub use oxgraph_db::*;
}

/// Append/update graph builders.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "graph-build")]
pub mod graph_build {
    // The builder index-width contract is the shared `LayoutIndex`, re-exported
    // at [`crate::layout`]; the `graph-build` feature enables it.
    pub use oxgraph_csr::build::{
        FrozenGraph, FrozenOutEdges, FrozenSuccessors, FrozenWeightedGraph, GraphBuildError,
        GraphBuilder, GraphEdgeId, GraphNodeId, WeightedGraphBuilder, export_csr_snapshot,
        export_weighted_csr_snapshot,
    };
    #[cfg(feature = "graph-property-arrow")]
    pub use oxgraph_csr::build::{
        export_csr_snapshot_with_properties, export_weighted_csr_snapshot_with_properties,
    };
}

/// Append/update hypergraph builders.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "hyper-build")]
pub mod hyper_build {
    // The builder index-width contract is the shared `LayoutIndex`, re-exported
    // at [`crate::layout`]; the `hyper-build` feature enables it. Frozen
    // hypergraph traversal yields the borrowed view's `Bcsr*` iterator family,
    // re-exported at [`crate::hyper_bcsr`]; the build module no longer defines
    // duplicate iterator types.
    pub use oxgraph_hyper_bcsr::build::{
        FrozenHypergraph, FrozenWeightedHypergraph, HyperBuildError, HyperParticipantId,
        HyperParticipantRole, HyperVertexId, HyperedgeId, HypergraphBuilder,
        WeightedHypergraphBuilder, export_bcsr_snapshot, export_weighted_bcsr_snapshot,
    };
    #[cfg(feature = "hyper-property-arrow")]
    pub use oxgraph_hyper_bcsr::build::{
        export_bcsr_snapshot_with_properties, export_weighted_bcsr_snapshot_with_properties,
    };
}

/// `PostgreSQL` graph engine library.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "postgres")]
pub mod postgres {
    // The borrowed inbound CSC view and its section kinds are re-exported once,
    // at [`crate::csc`] (the `postgres` feature enables `csc`). This module does
    // not duplicate them.
    pub use oxgraph_postgres::{
        BuildError, BuildEstimate, Catalog, CatalogError, Config, ConfigError,
        DualTopologySnapshot, EdgeId, EdgeRow, Engine, EngineBuilder, EngineStatus, FilterColumn,
        ForwardCsr, GraphRole, GraphTopology, InboundCsc, NodeKey, OverlayEdge, OverlayState,
        PostgresGraphError, PostgresMetadata, QueryError, QueryFreshness, RegisteredEdge,
        RegisteredTable, SNAPSHOT_KIND_PG_CATALOG, SNAPSHOT_KIND_PG_INBOUND_OFFSETS_BASE,
        SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32, SNAPSHOT_KIND_PG_INBOUND_TARGETS_BASE,
        SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32, SNAPSHOT_KIND_PG_METADATA, SearchPredicate,
        SnapshotRebuild, SyncAction, SyncActionCodec, SyncError, SyncHealth, SyncRow, TableId,
        TraversalDirection, TraverseLimits, estimate_build,
    };
}
