//! Borrowed bipartite compressed-sparse-row hypergraph views.
//!
//! `oxgraph-hyper-bcsr` provides the first concrete hypergraph layout for the
//! substrate. A [`BcsrHypergraph`] borrows eight validated CSR slices — one
//! offset/value pair per direction per role — and implements the
//! storage-agnostic hypergraph traits from `oxgraph-hyper`.
//!
//! Bipartite CSR keeps both directions dense: head and tail participants are
//! stored under a hyperedge-major index, while outgoing and incoming
//! incidences are stored under a vertex-major index. This trades roughly four
//! times the participant storage for `O(degree)` traversal in either
//! direction, which is the only access pattern that scales for read-heavy
//! workloads.
//!
//! # Layout summary
//!
//! Eight sections compose the snapshot bytes consumed by [`BcsrHypergraph`]:
//!
//! | Section kind                            | Logical content                                                            |
//! | --------------------------------------- | -------------------------------------------------------------------------- |
//! Snapshot section kinds are width-specific. Vertex participant sections use
//! the selected vertex width, relation sections use the selected relation
//! width, and all offset sections use the selected incidence width.
//!
//! # Validation
//!
//! Section payloads are validated at open time. [`BcsrValidation::Layout`]
//! covers length, offset monotonicity, in-range IDs, and per-range
//! sorted-and-unique vertex / hyperedge sequences. [`BcsrValidation::Strict`]
//! additionally checks cross-CSR consistency — that hyperedge-major and
//! vertex-major arrays describe the same incidence set. `Layout` is the
//! default for trusted producers; `Strict` is required for end-to-end
//! semantic guarantees on untrusted inputs.
//!
//! # Optional builder
//!
//! The `build` feature enables the append-only [`build::HypergraphBuilder`]
//! and [`build::WeightedHypergraphBuilder`] types plus snapshot export
//! helpers in the [`build`] submodule. The additional `build-property-arrow`
//! feature enables property-snapshot export via `oxgraph-property`.
#![no_std]

#[cfg(feature = "build")]
extern crate alloc;

#[cfg(kani)]
extern crate kani;

#[cfg(feature = "build")]
pub mod build;
mod error;
mod id;
mod internal;
mod snapshot;
mod word;

#[cfg(kani)]
mod proofs;

pub use crate::{
    error::{BcsrError, BcsrRoleSide, BcsrSection, BcsrSnapshotError},
    id::{BcsrHyperedgeId, BcsrParticipantId, BcsrRole, BcsrVertexId},
    internal::{
        BcsrChainedHyperedges, BcsrChainedParticipants, BcsrChainedRelationIncidences,
        BcsrElementIncidences, BcsrHyperedgeSlice, BcsrHypergraph, BcsrParticipantSlice,
        BcsrPredecessorVertices, BcsrSections, BcsrSuccessorVertices, BcsrValidation,
        BcsrVertexSlice,
    },
    snapshot::{
        SNAPSHOT_KIND_BCSR_HEAD_OFFSETS_BASE, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS_BASE,
        SNAPSHOT_KIND_BCSR_TAIL_OFFSETS_BASE, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS_BASE,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES_BASE,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS_BASE,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES_BASE,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS_BASE,
    },
    word::{
        BcsrSnapshotIndex, BcsrWords, LayoutIndex, LayoutSnapshotWord, LayoutWord, LeWords,
        NativeWords, SNAPSHOT_BCSR_SECTION_VERSION,
    },
};

/// Native borrowed BCSR hypergraph alias.
///
/// # Performance
///
/// `perf: unspecified`; this alias carries no runtime cost.
pub type BcsrNativeHypergraph<'view, VertexIndex, RelationIndex, IncidenceIndex> =
    BcsrHypergraph<'view, NativeWords<VertexIndex, RelationIndex, IncidenceIndex>>;

/// Snapshot-backed BCSR hypergraph alias.
///
/// # Performance
///
/// `perf: unspecified`; this alias carries no runtime cost.
pub type BcsrSnapshotHypergraph<'view, VertexIndex, RelationIndex, IncidenceIndex> =
    BcsrHypergraph<'view, LeWords<VertexIndex, RelationIndex, IncidenceIndex>>;
