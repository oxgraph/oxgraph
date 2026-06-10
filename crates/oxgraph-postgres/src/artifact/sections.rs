//! Postgres-owned snapshot section assembly (inbound CSC + metadata).

use oxgraph_csr::CsrSnapshotIndex;
use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{Snapshot, SnapshotWriter};

use super::metadata::{
    PostgresMetadata, SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32, SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32,
    write_postgres_metadata_section,
};
use crate::error::{BuildError, PostgresGraphError};

/// Appends or replaces Postgres-owned sections on CSR topology bytes.
///
/// Forward topology sections are preserved. When `inbound_topology_bytes` is
/// [`Some`], CSR offsets/targets are copied into the inbound section kinds
/// [`SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32`] /
/// [`SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32`].
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
    let copied = forward
        .sections()
        .filter(|section| !is_postgres_owned_kind(section.kind()))
        .count();
    let inbound_count = if inbound.is_some() { 2 } else { 0 };
    let mut writer = SnapshotWriter::new(copied + inbound_count + 1, crc32c_append)?;
    for section in forward.sections() {
        if is_postgres_owned_kind(section.kind()) {
            continue;
        }
        writer.section_bytes(
            section.kind(),
            section.version(),
            alignment_log2_from_section(&section),
            section.bytes(),
        )?;
    }
    if let Some(inbound_snapshot) = inbound.as_ref() {
        copy_csr_section(
            inbound_snapshot,
            u32::OFFSETS_KIND,
            SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32,
            &mut writer,
        )?;
        copy_csr_section(
            inbound_snapshot,
            u32::TARGETS_KIND,
            SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32,
            &mut writer,
        )?;
    }
    write_postgres_metadata_section(&mut writer, metadata)?;
    writer.finish().map_err(PostgresGraphError::from)
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
    writer: &mut SnapshotWriter,
) -> Result<(), PostgresGraphError> {
    let section = snapshot
        .section(source_kind)
        .ok_or(PostgresGraphError::Build(BuildError::MissingCsrSection {
            kind: source_kind,
        }))?;
    writer.section_bytes(
        dest_kind,
        section.version(),
        alignment_log2_from_section(&section),
        section.bytes(),
    )?;
    Ok(())
}

/// Returns whether `kind` is in the Postgres-owned reserved section range.
fn is_postgres_owned_kind(kind: u32) -> bool {
    (0x0200..0x0300).contains(&kind)
}

/// Derives snapshot writer alignment metadata from a borrowed section view.
fn alignment_log2_from_section(section: &oxgraph_snapshot::Section<'_>) -> u8 {
    let alignment = section.declared_alignment();
    if alignment <= 1 {
        0
    } else {
        u8::try_from(alignment.trailing_zeros()).unwrap_or(u8::MAX)
    }
}
