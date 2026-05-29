//! Standalone OxGraph-native database engine.
//!
//! `oxgraph-db` is a product layer above the topology substrate. It owns
//! durable database identity, cataloged physical projections, properties,
//! indexes, native `OxQL` execution, the pinned Cypher profile, and embedded
//! transaction semantics. Foundation crates remain storage- and
//! meaning-neutral.
#![expect(
    clippy::redundant_pub_crate,
    reason = "internal modules use crate visibility for sibling boundaries while staying unexported"
)]

pub(crate) mod catalog;
pub(crate) mod database;
pub(crate) mod error;
pub(crate) mod id;
pub(crate) mod projection;
pub(crate) mod query;
pub(crate) mod serde_map;
pub(crate) mod state;
pub(crate) mod storage;
pub(crate) mod traversal;
pub(crate) mod value;

pub use crate::{
    catalog::{
        Catalog, GraphProjectionDefinition, HypergraphProjectionDefinition, IndexDefinition,
        IndexEntry, LabelDefinition, ProjectionDefinition, ProjectionEntry, PropertyFamily,
        PropertyKeyDefinition, RelationTypeDefinition, RoleDefinition,
    },
    database::{
        CatalogSummary, Database, DatabaseStatus, IndexLookup, ReadPin, ReadTransaction,
        WriteTransaction,
    },
    error::DbError,
    id::{
        CheckpointGeneration, CommitSeq, ElementId, IncidenceId, IndexId, LabelId, ProjectionId,
        PropertyKeyId, RelationId, RelationTypeId, RoleId, TransactionId,
    },
    projection::{
        GraphProjection, HypergraphProjection, ProjectionElementId, ProjectionIncidenceId,
        ProjectionRelationId,
    },
    query::{PreparedQuery, QueryLanguage, QueryResult, QueryRow, QueryValue},
    state::{ElementRecord, IncidenceRecord, PropertySubject, RelationRecord},
    traversal::{TraversalDirection, TraversalOptions, TraversalResult, TraversalRow},
    value::{PropertyType, PropertyValue},
};

#[cfg(kani)]
mod proofs;
