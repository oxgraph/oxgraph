//! Generates the Kani fixture snapshot once via `cargo test -p oxgraph-csc --features build
//! write_tiny_dual_fixture -- --ignored --nocapture`.

use oxgraph_postgres::DualTopologySnapshot;

#[test]
#[ignore = "run manually to refresh proofs/tiny_dual.oxgtopo"]
fn write_tiny_dual_fixture() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = DualTopologySnapshot::from_dense_u32_edges(&[(0, 1)], 0)?;
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/proofs/tiny_dual.oxgtopo");
    std::fs::write(path, bytes)?;
    Ok(())
}
