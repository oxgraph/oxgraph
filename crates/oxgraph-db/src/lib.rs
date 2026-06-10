//! Standalone OxGraph-native database engine.
//!
//! `oxgraph-db` is a product layer above the topology substrate. It owns
//! durable database identity, cataloged physical projections, properties,
//! indexes, native `OxQL` execution, and embedded
//! transaction semantics. Foundation crates remain storage- and
//! meaning-neutral.
#![expect(
    clippy::redundant_pub_crate,
    reason = "internal modules use crate visibility for sibling boundaries while staying unexported"
)]

pub(crate) mod backing;
pub(crate) mod catalog;
pub(crate) mod crc;
pub(crate) mod database;
pub(crate) mod error;
pub(crate) mod freeze;
pub(crate) mod id;
pub(crate) mod index;
pub(crate) mod lock;
pub(crate) mod overlay;
pub mod projection;
pub(crate) mod query;
pub(crate) mod read;
pub(crate) mod schema;
pub(crate) mod state;
pub(crate) mod storage;
pub(crate) mod traversal;
pub(crate) mod typed;
pub(crate) mod value;
pub(crate) mod wal;
pub(crate) mod wire;

// Re-exported so `db`-only consumers can configure `Reader::personalized_pagerank`
// without depending on `oxgraph-algo` directly.
pub use oxgraph_algo::PageRankConfig;

pub use crate::{
    catalog::{
        Catalog, GraphProjectionDefinition, HypergraphProjectionDefinition, IndexDefinition,
        IndexEntry, LabelDefinition, ProjectionDefinition, ProjectionEntry, PropertyFamily,
        PropertyKeyDefinition, RelationTypeDefinition, RoleDefinition,
    },
    database::{
        CatalogSummary, CheckpointPolicy, CommitOutcome, Db, IndexProbe, ReadPin, Reader, Stats,
        Writer,
    },
    error::{CatalogError, DbError, IdFamily, QueryError, StorageError, TxnError},
    id::{
        CheckpointGeneration, CommitSeq, ElementId, IncidenceId, IndexId, LabelId, ProjectionId,
        PropertyKeyId, RelationId, RelationTypeId, RoleId, TransactionId,
    },
    query::{PreparedQuery, QueryResult, QueryRow, QueryValue},
    read::{Element, Properties, Relation},
    schema::{Bound, GraphProjectionSpec, Schema},
    state::{ElementRecord, IncidenceRecord, LabelSet, PropertySubject, RelationRecord},
    traversal::{Direction, Subgraph, TraversedEdge, TraversedNode, Walk},
    typed::{Assignable, Bool, EqualityIndex, Int, Key, RangeIndex, Readable, Text, ValueType},
    value::{PropertyType, PropertyValue},
};

#[cfg(kani)]
mod proofs;
