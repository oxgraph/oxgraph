//! Cross-module internals for [`BcsrHypergraph`](crate::BcsrHypergraph).
//!
//! Public-facing types are re-exported here so the crate root only references
//! `crate::internal::Foo`, never `crate::internal::iter::Foo`. Submodules
//! stay private; cross-sibling helpers use `pub(in crate::internal)`.

mod iter;
mod snapshot_open;
mod traits;
mod validation;
mod view;

pub use self::{
    iter::{
        BcsrChainedHyperedges, BcsrChainedParticipants, BcsrChainedRelationIncidences,
        BcsrElementIncidences, BcsrHyperedgeSlice, BcsrParticipantSlice, BcsrPredecessorVertices,
        BcsrSuccessorVertices, BcsrVertexSlice,
    },
    validation::BcsrValidation,
    view::{BcsrHypergraph, BcsrSections},
};
