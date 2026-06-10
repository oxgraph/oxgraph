//! Section-kind registry uniqueness test.
//!
//! Asserts that every persisted OXGT section kind across the in-tree
//! subsystems — CSR, BCSR, property, Postgres, and the embedded database band
//! — is pairwise distinct, so two subsystems can never silently collide on a
//! kind value. The registry itself is documented in
//! `docs/section-kind-registry.md`.

#![cfg(feature = "full")]

use oxgraph::{
    csr::CsrSnapshotIndex,
    hyper_bcsr::BcsrSnapshotIndex,
    postgres::{
        SNAPSHOT_KIND_PG_CATALOG, SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32,
        SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32, SNAPSHOT_KIND_PG_METADATA,
    },
    property::PropertySnapshotMetaWord,
};

/// Expands one width-parameterized trait const into its three derived kinds.
macro_rules! widths {
    ($trait:ident :: $kind:ident) => {
        [
            <u16 as $trait>::$kind,
            <u32 as $trait>::$kind,
            <u64 as $trait>::$kind,
        ]
    };
}

/// Count of OXGDB section kinds in the database band.
///
/// `oxgraph-db`'s kinds are crate-private; they are assigned contiguously from
/// the band base in emission order and pinned to `DATABASE_BAND` by a
/// compile-time check in `crates/oxgraph-db/src/wire.rs` (`SECTION_DB_HEADER
/// 0x0300` through `SECTION_BASE_TRAILER 0x0316`). Mirror that count here when
/// the database band changes.
const DATABASE_KIND_COUNT: u32 = 23;

/// Returns every persisted section kind in the registry.
fn all_registry_kinds() -> Vec<u32> {
    let mut kinds = Vec::new();
    // CSR band (0x0001..0x0020).
    kinds.extend(widths!(CsrSnapshotIndex::OFFSETS_KIND));
    kinds.extend(widths!(CsrSnapshotIndex::TARGETS_KIND));
    // BCSR band (0x0020..0x0100).
    kinds.extend(widths!(BcsrSnapshotIndex::HEAD_OFFSETS_KIND));
    kinds.extend(widths!(BcsrSnapshotIndex::HEAD_PARTICIPANTS_KIND));
    kinds.extend(widths!(BcsrSnapshotIndex::TAIL_OFFSETS_KIND));
    kinds.extend(widths!(BcsrSnapshotIndex::TAIL_PARTICIPANTS_KIND));
    kinds.extend(widths!(BcsrSnapshotIndex::VERTEX_OUTGOING_OFFSETS_KIND));
    kinds.extend(widths!(BcsrSnapshotIndex::VERTEX_OUTGOING_HYPEREDGES_KIND));
    kinds.extend(widths!(BcsrSnapshotIndex::VERTEX_INCOMING_OFFSETS_KIND));
    kinds.extend(widths!(BcsrSnapshotIndex::VERTEX_INCOMING_HYPEREDGES_KIND));
    // Property band (0x0100..0x0200).
    kinds.extend(widths!(PropertySnapshotMetaWord::PROPERTY_DESCRIPTORS_KIND));
    kinds.extend(widths!(PropertySnapshotMetaWord::PROPERTY_DATA_KIND));
    kinds.extend(widths!(PropertySnapshotMetaWord::IDENTITY_MODES_KIND));
    kinds.extend(widths!(PropertySnapshotMetaWord::ELEMENT_IDENTITY_MAP_KIND));
    kinds.extend(widths!(
        PropertySnapshotMetaWord::RELATION_IDENTITY_MAP_KIND
    ));
    kinds.extend(widths!(
        PropertySnapshotMetaWord::INCIDENCE_IDENTITY_MAP_KIND
    ));
    // Postgres band (0x0200..0x0300); the engine pins the u32 width.
    kinds.extend([
        SNAPSHOT_KIND_PG_CATALOG,
        SNAPSHOT_KIND_PG_INBOUND_OFFSETS_U32,
        SNAPSHOT_KIND_PG_INBOUND_TARGETS_U32,
        SNAPSHOT_KIND_PG_METADATA,
    ]);
    // Database band (0x0300..0x0400): contiguous from the band base.
    kinds.extend((0..DATABASE_KIND_COUNT).map(|offset| 0x0300 + offset));
    kinds
}

/// Every registered section kind value is pairwise distinct.
#[test]
fn registry_kinds_are_pairwise_distinct() {
    let kinds = all_registry_kinds();
    let mut sorted = kinds.clone();
    sorted.sort_unstable();
    for window in sorted.windows(2) {
        assert_ne!(
            window[0], window[1],
            "duplicate section kind {:#06X} in the registry",
            window[0],
        );
    }
    // 6 CSR + 24 BCSR + 18 property + 4 Postgres + 23 database kinds.
    assert_eq!(kinds.len(), 75, "registry table lost or gained an entry");
}
