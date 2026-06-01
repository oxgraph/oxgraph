//! Generates the Kani fixture snapshot once via `cargo test -p oxgraph-csc --features build
//! write_tiny_dual_fixture -- --ignored --nocapture`.
//!
//! The fixture holds the inbound (transposed) CSC offsets/targets sections under
//! the Postgres inbound section kinds (`0x0201` / `0x0202`) that the Kani proof
//! opens with `from_snapshot_with_kinds`. It is built directly from the CSR
//! builder + transpose so this crate carries no storage-engine dependency.

use oxgraph_csr::build::{GraphBuilder, export_csr_snapshot};
use oxgraph_snapshot::Snapshot;

/// Inbound offsets section kind (Postgres band) the proof opens with.
const INBOUND_OFFSETS_KIND: u32 = 0x0201;
/// Inbound targets section kind (Postgres band) the proof opens with.
const INBOUND_TARGETS_KIND: u32 = 0x0202;

#[test]
#[ignore = "run manually to refresh proofs/tiny_dual.oxgtopo"]
fn write_tiny_dual_fixture() -> Result<(), Box<dyn std::error::Error>> {
    // Two nodes, one edge 0 -> 1. The transpose gives node 1 a single
    // predecessor (node 0), matching the proof's node_count == 2, relation == 1.
    let mut builder = GraphBuilder::<u32, u32>::new();
    let a = builder.add_node()?;
    let b = builder.add_node()?;
    builder.add_edge(a, b)?;
    let inbound = builder.freeze()?.transpose()?;

    // Export the transpose under the default CSR kinds, then re-band the two
    // sections into the Postgres inbound kinds the proof reads.
    let csr_bytes = export_csr_snapshot(&inbound)?;
    let csr = Snapshot::open(&csr_bytes)?;
    let mut out = oxgraph_snapshot::SnapshotBuilder::new();
    for section in csr.sections() {
        let dest_kind = match section.kind() {
            oxgraph_csr::SNAPSHOT_KIND_CSR_OFFSETS_U32 => INBOUND_OFFSETS_KIND,
            oxgraph_csr::SNAPSHOT_KIND_CSR_TARGETS_U32 => INBOUND_TARGETS_KIND,
            other => other,
        };
        let alignment = section.declared_alignment();
        let alignment_log2 = if alignment <= 1 {
            0
        } else {
            u8::try_from(alignment.trailing_zeros())?
        };
        out.add_section(
            dest_kind,
            section.version(),
            alignment_log2,
            section.bytes().to_vec(),
        )?;
    }
    let bytes = out.finish()?;

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/proofs/tiny_dual.oxgtopo");
    std::fs::write(path, bytes)?;
    Ok(())
}
