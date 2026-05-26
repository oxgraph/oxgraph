//! Postgres-owned snapshot metadata section (`0x0203`).

use oxgraph_snapshot::{PlanError, Snapshot, SnapshotBuilder, SnapshotError};
use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout,
    byteorder::{LE, U32, U64},
};

/// Section kind for serialized catalog blobs owned by Postgres.
pub const SNAPSHOT_KIND_PG_CATALOG: u32 = 0x0200;

/// Section kind for Postgres engine metadata (`0x0200` reserved range).
pub const SNAPSHOT_KIND_PG_METADATA: u32 = 0x0203;

/// Fixed-layout Postgres metadata stored in snapshot section payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, FromBytes, Immutable, IntoBytes, KnownLayout)]
#[repr(C)]
pub struct PostgresMetadata {
    /// Metadata schema version.
    pub version: U32<LE>,
    /// Bit flags (`READ_ONLY`, `HAS_REVERSE_INDEX`, …).
    pub flags: U32<LE>,
    /// Build timestamp in Unix seconds (semantic-free).
    pub built_at_unix: U64<LE>,
    /// Node count at build time.
    pub node_count: U32<LE>,
    /// Edge count at build time.
    pub edge_count: U32<LE>,
}

impl PostgresMetadata {
    /// Metadata schema version written by this library.
    pub const VERSION: u32 = 1;

    /// Flag indicating the artifact was published read-only.
    pub const FLAG_READ_ONLY: u32 = 1;

    /// Flag indicating inbound CSC sections (`0x0201` / `0x0202`) are present.
    pub const FLAG_HAS_REVERSE_INDEX: u32 = 2;

    /// Creates metadata for a freshly built artifact.
    #[must_use]
    pub const fn new(
        node_count: u32,
        edge_count: u32,
        built_at_unix: u64,
        read_only: bool,
    ) -> Self {
        let mut flags = 0_u32;
        if read_only {
            flags |= Self::FLAG_READ_ONLY;
        }
        Self {
            version: U32::new(Self::VERSION),
            flags: U32::new(flags),
            built_at_unix: U64::new(built_at_unix),
            node_count: U32::new(node_count),
            edge_count: U32::new(edge_count),
        }
    }

    /// Returns metadata with [`Self::FLAG_HAS_REVERSE_INDEX`] set.
    #[must_use]
    pub const fn with_reverse_index(mut self) -> Self {
        self.flags = U32::new(self.flags.get() | Self::FLAG_HAS_REVERSE_INDEX);
        self
    }

    /// Returns whether the read-only flag is set.
    #[must_use]
    pub const fn is_read_only(self) -> bool {
        self.flags.get() & Self::FLAG_READ_ONLY != 0
    }

    /// Returns whether inbound CSC sections are required at engine open.
    #[must_use]
    pub const fn has_reverse_index(self) -> bool {
        self.flags.get() & Self::FLAG_HAS_REVERSE_INDEX != 0
    }
}

/// Errors while reading Postgres-owned snapshot sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostgresSectionError {
    /// Underlying snapshot validation failed.
    Snapshot(SnapshotError),
    /// Expected Postgres section was absent.
    MissingSection,
    /// Section payload could not be interpreted.
    Malformed(alloc::string::String),
}

/// Reads [`PostgresMetadata`] from a snapshot's Postgres section.
///
/// # Errors
///
/// Returns [`PostgresSectionError`] when the section is missing or malformed.
///
/// # Performance
///
/// This function is `O(s)`.
pub(super) fn read_postgres_metadata(
    snapshot: &Snapshot<'_>,
) -> Result<PostgresMetadata, PostgresSectionError> {
    let section = snapshot
        .section(SNAPSHOT_KIND_PG_METADATA)
        .ok_or(PostgresSectionError::MissingSection)?;
    PostgresMetadata::ref_from_bytes(section.bytes())
        .map_err(|error| {
            PostgresSectionError::Malformed(alloc::format!("postgres metadata layout: {error}"))
        })
        .copied()
}

/// Writes the Postgres metadata section into a snapshot builder.
///
/// # Errors
///
/// Returns [`SnapshotError`] when planning or encoding fails.
///
/// # Performance
///
/// This function is `O(1)`.
pub(super) fn write_postgres_metadata_section(
    builder: &mut SnapshotBuilder,
    metadata: &PostgresMetadata,
) -> Result<(), PlanError> {
    builder.add_section_typed(
        SNAPSHOT_KIND_PG_METADATA,
        PostgresMetadata::VERSION,
        core::slice::from_ref(metadata),
    )?;
    Ok(())
}
