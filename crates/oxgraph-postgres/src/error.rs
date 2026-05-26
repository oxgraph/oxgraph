//! Shared error surface for the Postgres graph engine.

use core::fmt;

use oxgraph_csc::CscSnapshotError;
use oxgraph_csr::CsrSnapshotError;
use oxgraph_snapshot::{PlanError, SnapshotError};

use crate::{catalog::CatalogError, role::GraphRole};

/// Relational build or snapshot export failures.
#[derive(Debug)]
pub enum BuildError {
    /// Snapshot planning failed.
    Plan(PlanError),
    /// CSR construction failed.
    Graph(oxgraph_csr::build::GraphBuildError<u32, u32>),
    /// Engine build was invoked without snapshot bytes.
    MissingSnapshotBytes,
    /// Build received no edge rows.
    EmptyEdges,
    /// Distinct node count does not fit in `u32` metadata.
    NodeCountOverflow,
    /// Edge count does not fit in `u32` metadata.
    EdgeCountOverflow,
    /// Forward and inbound edge counts diverged after transpose.
    EdgeCountMismatch,
    /// An edge row referenced a node key that was not assigned.
    MissingNodeKey,
    /// Required CSR section was absent during snapshot assembly.
    MissingCsrSection {
        /// Section kind that was expected.
        kind: u32,
    },
    /// Postgres metadata section was missing at engine open.
    MissingMetadataSection,
    /// Section payload could not be interpreted.
    MalformedMetadata(alloc::string::String),
    /// Artifact metadata lacks the inbound CSC flag.
    MissingReverseIndex,
    /// Forward and inbound node counts disagree at engine open.
    TopologyNodeCountMismatch,
    /// Forward and inbound edge counts disagree at engine open.
    TopologyEdgeCountMismatch,
    /// Forward node count disagrees with metadata.
    MetadataNodeCountMismatch,
    /// Forward edge count disagrees with metadata.
    MetadataEdgeCountMismatch,
    /// SPI or extension boundary failure.
    Spi(alloc::string::String),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(error) => write!(f, "plan error: {error}"),
            Self::Graph(error) => write!(f, "graph build error: {error}"),
            Self::MissingSnapshotBytes => f.write_str("snapshot bytes required"),
            Self::EmptyEdges => f.write_str("build requires at least one edge row"),
            Self::NodeCountOverflow => f.write_str("node count does not fit in u32 metadata"),
            Self::EdgeCountOverflow => f.write_str("edge count does not fit in u32 metadata"),
            Self::EdgeCountMismatch | Self::TopologyEdgeCountMismatch => {
                f.write_str("forward and inbound edge counts diverged")
            }
            Self::MissingNodeKey => f.write_str("edge row references missing node key"),
            Self::MissingCsrSection { kind } => {
                write!(f, "missing CSR section {kind:#06x}")
            }
            Self::MissingMetadataSection => f.write_str("postgres metadata section missing"),
            Self::MalformedMetadata(message) => write!(f, "postgres metadata layout: {message}"),
            Self::MissingReverseIndex => {
                f.write_str("artifact missing HAS_REVERSE_INDEX metadata flag")
            }
            Self::TopologyNodeCountMismatch => {
                f.write_str("forward and inbound node counts diverged")
            }
            Self::MetadataNodeCountMismatch => {
                f.write_str("forward node count disagrees with metadata")
            }
            Self::MetadataEdgeCountMismatch => {
                f.write_str("forward edge count disagrees with metadata")
            }
            Self::Spi(message) => write!(f, "spi: {message}"),
        }
    }
}

impl core::error::Error for BuildError {}

/// Query input or limit failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    /// BFS seed is outside the canonical node bound.
    SeedOutOfBounds {
        /// Requested seed id.
        seed: u32,
        /// Canonical node count.
        node_count: u32,
    },
    /// A required limit was zero.
    LimitZero,
    /// Dense node iteration overflowed `u32`.
    NodeIndexOverflow,
    /// Internal kernel invariant violated.
    InternalInvariant(&'static str),
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SeedOutOfBounds { seed, node_count } => {
                write!(f, "seed {seed} out of bounds (node_count {node_count})")
            }
            Self::LimitZero => f.write_str("limit must be > 0"),
            Self::NodeIndexOverflow => f.write_str("node index does not fit in u32"),
            Self::InternalInvariant(message) => write!(f, "internal query invariant: {message}"),
        }
    }
}

impl core::error::Error for QueryError {}

/// Sync replay ordering failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncError {
    /// Sequence numbers were not strictly increasing.
    NonMonotonicSequence {
        /// Offending sequence.
        sequence: u64,
        /// Previous sequence in the batch.
        previous: u64,
    },
    /// Persisted action type id is not recognized.
    InvalidActionType {
        /// Raw action type from the sync log.
        action_type: i16,
    },
    /// Action arguments could not be decoded for a known action type.
    InvalidActionArgs {
        /// Raw action type from the sync log.
        action_type: i16,
    },
    /// A keyed sync action referenced a node key absent from the current scan.
    UnknownNodeKey {
        /// Unassigned registered node key.
        key: crate::catalog::NodeKey,
    },
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonMonotonicSequence { sequence, previous } => {
                write!(f, "non-monotonic sync sequence {sequence} after {previous}")
            }
            Self::InvalidActionType { action_type } => {
                write!(f, "invalid sync action type {action_type}")
            }
            Self::InvalidActionArgs { action_type } => {
                write!(f, "invalid sync action arguments for type {action_type}")
            }
            Self::UnknownNodeKey { key } => write!(f, "unknown sync node key {key}"),
        }
    }
}

impl core::error::Error for SyncError {}

/// Operational configuration failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// Traverse limit GUC was zero.
    ZeroTraverseLimit,
    /// Search limit GUC was zero.
    ZeroSearchLimit,
    /// Maintenance rebuild is disabled by policy.
    MaintenanceDisabled,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTraverseLimit => f.write_str("traverse_limit must be > 0"),
            Self::ZeroSearchLimit => f.write_str("search_limit must be > 0"),
            Self::MaintenanceDisabled => f.write_str("maintenance rebuild disabled by config"),
        }
    }
}

impl core::error::Error for ConfigError {}

/// Errors returned by the Postgres graph engine library.
///
/// Concrete variants only — no boxed or dynamic error types on the public surface.
#[derive(Debug)]
pub enum PostgresGraphError {
    /// Snapshot bytes failed container validation.
    Snapshot(SnapshotError),
    /// Forward CSR topology sections could not be opened.
    ForwardSnapshot(CsrSnapshotError<u32, u32>),
    /// Inbound CSC topology sections could not be opened.
    InboundSnapshot(CscSnapshotError),
    /// No graph engine is loaded in this session.
    NotLoaded,
    /// Relational build or export failed.
    Build(BuildError),
    /// Catalog registration is inconsistent or incomplete.
    Catalog(CatalogError),
    /// Query limits or inputs were invalid.
    Query(QueryError),
    /// Sync replay rejected a row or batch.
    Sync(SyncError),
    /// Operational configuration was invalid.
    Config(ConfigError),
    /// Access control policy denied an operation.
    AccessDenied {
        /// Role required for the operation.
        required: GraphRole,
        /// Role observed in the session.
        actual: GraphRole,
    },
}

impl fmt::Display for PostgresGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot(error) => write!(f, "snapshot error: {error}"),
            Self::ForwardSnapshot(error) => write!(f, "forward snapshot error: {error}"),
            Self::InboundSnapshot(error) => write!(f, "inbound snapshot error: {error}"),
            Self::NotLoaded => f.write_str("graph engine not loaded"),
            Self::Build(error) => write!(f, "build error: {error}"),
            Self::Catalog(error) => write!(f, "catalog error: {error}"),
            Self::Query(error) => write!(f, "query error: {error}"),
            Self::Sync(error) => write!(f, "sync error: {error}"),
            Self::Config(error) => write!(f, "config error: {error}"),
            Self::AccessDenied { required, actual } => {
                write!(f, "access denied: {actual:?} cannot satisfy {required:?}")
            }
        }
    }
}

impl core::error::Error for PostgresGraphError {}

impl From<SnapshotError> for PostgresGraphError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

impl From<CsrSnapshotError<u32, u32>> for PostgresGraphError {
    fn from(error: CsrSnapshotError<u32, u32>) -> Self {
        Self::ForwardSnapshot(error)
    }
}

impl From<CscSnapshotError> for PostgresGraphError {
    fn from(error: CscSnapshotError) -> Self {
        Self::InboundSnapshot(error)
    }
}

impl From<PlanError> for PostgresGraphError {
    fn from(error: PlanError) -> Self {
        Self::Build(BuildError::Plan(error))
    }
}

impl From<oxgraph_csr::build::GraphBuildError<u32, u32>> for PostgresGraphError {
    fn from(error: oxgraph_csr::build::GraphBuildError<u32, u32>) -> Self {
        Self::Build(BuildError::Graph(error))
    }
}

impl From<CatalogError> for PostgresGraphError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

impl From<BuildError> for PostgresGraphError {
    fn from(error: BuildError) -> Self {
        Self::Build(error)
    }
}

impl From<QueryError> for PostgresGraphError {
    fn from(error: QueryError) -> Self {
        Self::Query(error)
    }
}

impl From<SyncError> for PostgresGraphError {
    fn from(error: SyncError) -> Self {
        Self::Sync(error)
    }
}

impl From<ConfigError> for PostgresGraphError {
    fn from(error: ConfigError) -> Self {
        Self::Config(error)
    }
}
