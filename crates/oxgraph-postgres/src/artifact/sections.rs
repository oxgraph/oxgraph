//! Postgres-owned snapshot section assembly (inbound CSC + metadata).

use oxgraph_csr::CsrSnapshotIndex;
use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{Snapshot, SnapshotBuilder};

use super::metadata::{
    PostgresMetadata, SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32, SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32,
    write_postgres_metadata_section,
};
use crate::error::{BuildError, PostgresGraphError};

/// Appends or replaces Postgres-owned sections on CSR topology bytes.
///
/// Forward topology sections are preserved. When `inbound_topology_bytes` is
/// [`Some`], CSR offsets/targets are copied into inbound section kinds
/// `0x0201` / `0x0202`.
///
/// # Errors
///
/// Returns [`PostgresGraphError::Snapshot`] when input bytes are invalid, or
/// [`PostgresGraphError::Build`] when re-encoding fails.
///
/// # Performance
///
/// This function is `O(b + s)`.
pub fn attach_postgres_sections(
    forward_topology_bytes: &[u8],
    inbound_topology_bytes: Option<&[u8]>,
    metadata: &PostgresMetadata,
) -> Result<Vec<u8>, PostgresGraphError> {
    let forward = Snapshot::open(forward_topology_bytes)?;
    let inbound = inbound_topology_bytes.map(Snapshot::open).transpose()?;
    let mut builder = SnapshotBuilder::new(crc32c_append);
    for section in forward.sections() {
        if is_postgres_owned_kind(section.kind()) {
            continue;
        }
        builder.add_section(
            section.kind(),
            section.version(),
            alignment_log2_from_section(&section),
            section.bytes().to_vec(),
        )?;
    }
    if let Some(inbound_snapshot) = inbound.as_ref() {
        copy_csr_section(
            inbound_snapshot,
            u32::OFFSETS_KIND,
            SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32,
            &mut builder,
        )?;
        copy_csr_section(
            inbound_snapshot,
            u32::TARGETS_KIND,
            SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32,
            &mut builder,
        )?;
    }
    write_postgres_metadata_section(&mut builder, metadata)?;
    builder.finish().map_err(PostgresGraphError::from)
}

/// Appends or replaces the Postgres metadata section on CSR topology bytes.
///
/// Prefer [`attach_postgres_sections`] when inbound CSC bytes are available.
///
/// # Errors
///
/// Returns [`PostgresGraphError::Snapshot`] when input bytes are invalid, or
/// [`PostgresGraphError::Build`] when re-encoding fails.
///
/// # Performance
///
/// This function is `O(b + s)`.
pub fn attach_metadata(
    base_bytes: &[u8],
    metadata: &PostgresMetadata,
) -> Result<Vec<u8>, PostgresGraphError> {
    attach_postgres_sections(base_bytes, None, metadata)
}

/// Copies one CSR topology section into a Postgres-owned section kind.
fn copy_csr_section(
    snapshot: &Snapshot<'_>,
    source_kind: u32,
    dest_kind: u32,
    builder: &mut SnapshotBuilder,
) -> Result<(), PostgresGraphError> {
    let section = snapshot
        .section(source_kind)
        .ok_or(PostgresGraphError::Build(BuildError::MissingCsrSection {
            kind: source_kind,
        }))?;
    builder.add_section(
        dest_kind,
        section.version(),
        alignment_log2_from_section(&section),
        section.bytes().to_vec(),
    )?;
    Ok(())
}

/// Returns whether `kind` is in the Postgres-owned reserved section range.
fn is_postgres_owned_kind(kind: u32) -> bool {
    (0x0200..0x0300).contains(&kind)
}

/// Derives snapshot builder alignment metadata from a borrowed section view.
fn alignment_log2_from_section(section: &oxgraph_snapshot::Section<'_>) -> u8 {
    let alignment = section.declared_alignment();
    if alignment <= 1 {
        0
    } else {
        u8::try_from(alignment.trailing_zeros()).unwrap_or(u8::MAX)
    }
}
