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
//! | [`SNAPSHOT_KIND_BCSR_HEAD_OFFSETS`]     | hyperedge-major head offsets, length `hyperedge_count + 1`                 |
//! | [`SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS`]| flat vertex IDs in head sets, length `P_head`                              |
//! | [`SNAPSHOT_KIND_BCSR_TAIL_OFFSETS`]     | hyperedge-major tail offsets, length `hyperedge_count + 1`                 |
//! | [`SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS`]| flat vertex IDs in tail sets, length `P_tail`                              |
//! | [`SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS`] | vertex-major outgoing offsets, length `vertex_count + 1`            |
//! | [`SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES`] | flat hyperedge IDs where v is in head, length `P_outgoing`        |
//! | [`SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS`] | vertex-major incoming offsets, length `vertex_count + 1`            |
//! | [`SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES`] | flat hyperedge IDs where v is in tail, length `P_incoming`        |
//!
//! # v1.0 caps
//!
//! Vertex IDs, hyperedge IDs, and offsets are 32-bit. The container therefore
//! supports at most `2^32 − 1` vertices, `2^32 − 1` hyperedges, and `2^32 − 1`
//! participants in each of `P_head`, `P_tail`, `P_outgoing`, and `P_incoming`.
//! `u64`-offset variants are reserved at section kinds `0x0018..=0x001F` and
//! will land as a separate slice if a producer needs them.
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
#![no_std]

#[cfg(kani)]
extern crate kani;

mod error;
mod id;
mod internal;
mod role;
mod sections;
mod snapshot;
mod word;

#[cfg(kani)]
mod proofs;

pub use crate::{
    error::{BcsrError, BcsrRoleSide, BcsrSection, BcsrSnapshotError},
    id::{BcsrHyperedgeId, BcsrParticipantId, BcsrVertexId},
    internal::{
        BcsrChainedHyperedges, BcsrChainedParticipants, BcsrChainedRelationIncidences,
        BcsrElementIncidences, BcsrHyperedgeSlice, BcsrHypergraph, BcsrParticipantSlice,
        BcsrPredecessorVertices, BcsrSuccessorVertices, BcsrValidation, BcsrVertexSlice,
    },
    role::BcsrRole,
    sections::BcsrSections,
    snapshot::{
        SNAPSHOT_KIND_BCSR_HEAD_OFFSETS, SNAPSHOT_KIND_BCSR_HEAD_PARTICIPANTS,
        SNAPSHOT_KIND_BCSR_TAIL_OFFSETS, SNAPSHOT_KIND_BCSR_TAIL_PARTICIPANTS,
        SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_HYPEREDGES, SNAPSHOT_KIND_BCSR_VERTEX_INCOMING_OFFSETS,
        SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_HYPEREDGES, SNAPSHOT_KIND_BCSR_VERTEX_OUTGOING_OFFSETS,
    },
    word::BcsrWord,
};
