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
        LayoutWord, LocalId, NodeAxis, SnapshotWidth, VertexAxis, ZerocopyWord,
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
        ContainsElement, ContainsIncidence, ContainsRelation, ElementId, ElementIncidenceCount,
        ElementIncidences, ElementIndex, ElementPredecessors, ElementSuccessors, ElementWeight,
        IncidenceBase, IncidenceCounts, IncidenceElement, IncidenceIndex, IncidenceRelation,
        IncidenceRole, IncidenceView, IncidenceWeight, LocalElementIdentity,
        LocalIncidenceIdentity, LocalRelationIdentity, RelationId, RelationIncidenceCount,
        RelationIncidences, RelationIndex, RelationWeight, TopologyBase, TopologyCounts,
        TopologyId,
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
        ContainsEdge, ContainsEndpoint, ContainsNode, DirectedGraph, EdgeEndpointGraph, EdgeId,
        EdgeIndex, EdgeSourceGraph, EdgeTargetGraph, EndpointId, EndpointIndex, EndpointRole,
        ForwardGraph, GraphBase, GraphCounts, IncomingEdgeCount, IncomingGraph,
        IncomingNeighborsGraph, NodeId, NodeIndex, OutgoingEdgeCount, OutgoingGraph,
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
        ContainsHyperedge, ContainsParticipant, ContainsVertex, DirectedHyperedgeIncidences,
        DirectedHyperedgeParticipants, DirectedHypergraph, DirectedVertexHyperedges,
        DirectedVertexPredecessors, DirectedVertexSuccessors, HyperedgeId, HyperedgeIncidences,
        HyperedgeIndex, HyperedgeParticipantCount, HyperedgeParticipants, Hypergraph,
        HypergraphBase, HypergraphCounts, IncidentHyperedgeCount, IncidentHyperedges,
        ParticipantBase, ParticipantCounts, ParticipantHyperedge, ParticipantId, ParticipantIndex,
        ParticipantRole, ParticipantRoleOf, ParticipantVertex, VertexId, VertexIncidences,
        VertexIndex,
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
        SNAPSHOT_KIND_CSR_OFFSETS_U16, SNAPSHOT_KIND_CSR_OFFSETS_U32,
        SNAPSHOT_KIND_CSR_OFFSETS_U64, SNAPSHOT_KIND_CSR_TARGETS_U16,
        SNAPSHOT_KIND_CSR_TARGETS_U32, SNAPSHOT_KIND_CSR_TARGETS_U64,
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
        SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U16, SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U32,
        SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_U64, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U16,
        SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U32, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_U64,
        SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U16, SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U32,
        SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_U64, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U16,
        SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U32, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_U64,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U16,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U32,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_U64,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U16,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U32,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_U64,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U16,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U32,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_U64,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U16,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U32,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_U64,
    };
}

/// Topology-agnostic snapshot container.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "snapshot")]
pub mod snapshot {
    #[cfg(feature = "snapshot-alloc")]
    pub use oxgraph_snapshot::SnapshotBuilder;
    pub use oxgraph_snapshot::{
        Checksum32, FORMAT_MAGIC, FORMAT_MAJOR, FORMAT_MINOR, HEADER_SIZE, HeaderOnlySnapshot,
        MAX_ALIGNMENT_LOG2, MAX_SECTION_COUNT, MAX_SUPPORTED_MINOR, PendingSection, PlanError,
        SECTION_ENTRY_SIZE, Section, SectionIter, SectionViewError, Snapshot, SnapshotError,
        SnapshotPlan, ValidationLevel,
    };
}

/// Substrate-agnostic algorithms.
///
/// # Performance
///
/// `perf: unspecified`; this module re-exports another crate.
#[cfg(feature = "algo")]
pub mod algo {
    pub use oxgraph_algo::{
        BfsEpochScratch, BfsError, BreadthFirstSearchEpochScratch, BreadthFirstSearchScratch,
        HyperWeighted, HypergraphOutgoingDistribution, HypergraphPageRankScratch,
        IntoPageRankScalar, OutgoingDistribution, PageRankConfig, PageRankError, PageRankReport,
        PageRankScalar, PageRankScratch, ReverseBreadthFirstSearchEpochScratch,
        ReverseBreadthFirstSearchScratch, Uniform, Weighted,
        breadth_first_search_with_epoch_scratch, breadth_first_search_with_scratch,
        pagerank_graph_with_scratch, pagerank_hypergraph_with_scratch,
        reverse_breadth_first_search_with_epoch_scratch, reverse_breadth_first_search_with_scratch,
    };
    #[cfg(feature = "algo-alloc")]
    pub use oxgraph_algo::{
        BfsWorkspace, BreadthFirstSearch, BreadthFirstSearchWorkspace, GenericBreadthFirstSearch,
        GenericReverseBreadthFirstSearch, HypergraphPageRankWorkspace, PageRankWorkspace,
        ReverseBreadthFirstSearch, ReverseBreadthFirstSearchWorkspace, breadth_first_search,
        breadth_first_search_generic, breadth_first_search_with_workspace, pagerank_graph,
        pagerank_graph_with_workspace, pagerank_hypergraph, pagerank_hypergraph_with_workspace,
        reverse_breadth_first_search, reverse_breadth_first_search_generic,
        reverse_breadth_first_search_with_workspace,
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
        PropertySnapshotSummary, RelationAxis, SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U16,
        SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U32, SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U64,
        SNAPSHOT_KIND_IDENTITY_MODES_U16, SNAPSHOT_KIND_IDENTITY_MODES_U32,
        SNAPSHOT_KIND_IDENTITY_MODES_U64, SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U16,
        SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U32, SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U64,
        SNAPSHOT_KIND_PROPERTY_DATA_U16, SNAPSHOT_KIND_PROPERTY_DATA_U32,
        SNAPSHOT_KIND_PROPERTY_DATA_U64, SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U16,
        SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U32, SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U64,
        SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U16, SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U32,
        SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U64, SNAPSHOT_PROPERTY_VERSION, SparseWeights,
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
        GraphBuilder, GraphEdgeId, GraphNodeId, WeightedGraphBuilder, csr_slice_to_le,
        export_csr_snapshot, export_weighted_csr_snapshot, identity_slice_to_le,
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
        identity_slice_to_le,
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
        RegisteredTable, SNAPSHOT_KIND_PG_CATALOG, SNAPSHOT_KIND_PG_METADATA, SearchPredicate,
        SnapshotRebuild, SyncAction, SyncActionCodec, SyncError, SyncHealth, SyncRow, TableId,
        TraversalDirection, TraverseLimits, estimate_build,
    };
}
