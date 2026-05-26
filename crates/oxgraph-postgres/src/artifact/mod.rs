//! OXGTOPO artifact storage helpers for Postgres-owned snapshot sections.

mod metadata;
mod sections;

pub use metadata::{
    PostgresMetadata, PostgresSectionError, SNAPSHOT_KIND_PG_CATALOG, SNAPSHOT_KIND_PG_METADATA,
};
use oxgraph_snapshot::Snapshot;
pub use sections::{attach_metadata, attach_postgres_sections};

use crate::error::{BuildError, PostgresGraphError};

/// Validates and borrows snapshot bytes without copying the payload region.
///
/// # Errors
///
/// Returns [`PostgresGraphError::Snapshot`] when header or section table validation fails.
///
/// # Performance
///
/// This function is `O(s)` where `s` is the section count.
pub fn load_snapshot_bytes(bytes: &[u8]) -> Result<Snapshot<'_>, PostgresGraphError> {
    Snapshot::open(bytes).map_err(PostgresGraphError::from)
}

/// Returns owned snapshot bytes unchanged after validation.
///
/// # Errors
///
/// Returns [`PostgresGraphError::Snapshot`] when validation fails.
///
/// # Performance
///
/// This function is `O(s + b)` where `s` is section count and `b` is byte length.
pub fn validate_snapshot_bytes(bytes: &[u8]) -> Result<Vec<u8>, PostgresGraphError> {
    let _ = Snapshot::open(bytes)?;
    Ok(bytes.to_vec())
}

/// Reads Postgres-owned metadata from a validated snapshot.
///
/// # Errors
///
/// Returns [`PostgresGraphError::Snapshot`] or [`PostgresGraphError::Build`] when metadata
/// is missing or malformed.
///
/// # Performance
///
/// This function is `O(s)`.
pub fn read_metadata(snapshot: &Snapshot<'_>) -> Result<PostgresMetadata, PostgresGraphError> {
    metadata::read_postgres_metadata(snapshot).map_err(|error| match error {
        PostgresSectionError::Snapshot(snapshot_error) => {
            PostgresGraphError::Snapshot(snapshot_error)
        }
        PostgresSectionError::MissingSection => {
            PostgresGraphError::Build(BuildError::MissingMetadataSection)
        }
        PostgresSectionError::Malformed(message) => {
            PostgresGraphError::Build(BuildError::MalformedMetadata(message))
        }
    })
}

/// Writes validated snapshot bytes suitable for BYTEA/LO persistence.
///
/// # Errors
///
/// Returns [`PostgresGraphError::Snapshot`] when validation fails.
///
/// # Performance
///
/// This function is `O(b)`.
pub fn write_snapshot_bytes(bytes: &[u8]) -> Result<Vec<u8>, PostgresGraphError> {
    validate_snapshot_bytes(bytes)
}
