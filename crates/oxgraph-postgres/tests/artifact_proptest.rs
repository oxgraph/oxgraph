//! Property tests for snapshot artifact roundtrips.

use oxgraph_csr::build::{GraphBuilder, GraphNodeId, export_csr_snapshot};
use oxgraph_graph::OutgoingNeighborsGraph;
use oxgraph_postgres::{
    EngineBuilder, PostgresGraphError, PostgresMetadata, attach_postgres_sections,
    load_snapshot_bytes, validate_snapshot_bytes, write_snapshot_bytes,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn snapshot_bytes_roundtrip(node_count in 1_usize..32, edge_pairs in prop::collection::vec((0_u32..8, 0_u32..8), 1..64)) {
        let mut builder = GraphBuilder::<u32, u32>::new();
        for _ in 0..node_count {
            builder.add_node().map_err(|error| TestCaseError::fail(error.to_string()))?;
        }
        for (source, target) in edge_pairs {
            let source_usize = usize::try_from(source).map_err(|error| TestCaseError::fail(error.to_string()))?;
            let target_usize = usize::try_from(target).map_err(|error| TestCaseError::fail(error.to_string()))?;
            if source_usize >= node_count || target_usize >= node_count {
                continue;
            }
            builder
                .add_edge(GraphNodeId::new(source), GraphNodeId::new(target))
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
        }
        let frozen = builder.freeze().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let mut inbound_builder = GraphBuilder::<u32, u32>::new();
        for _ in 0..node_count {
            inbound_builder.add_node().map_err(|error| TestCaseError::fail(error.to_string()))?;
        }
        for source in 0..node_count {
            let source_id = GraphNodeId::new(
                u32::try_from(source).map_err(|error| TestCaseError::fail(error.to_string()))?,
            );
            for target in frozen.outgoing_neighbors(source_id) {
                inbound_builder
                    .add_edge(target, source_id)
                    .map_err(|error| TestCaseError::fail(error.to_string()))?;
            }
        }
        let inbound_frozen = inbound_builder.freeze().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let forward_bytes = export_csr_snapshot(&frozen).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let inbound_bytes = export_csr_snapshot(&inbound_frozen).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let edge_count = u32::try_from(frozen.edge_ids().len())
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let node_count_u32 = u32::try_from(node_count)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let metadata = PostgresMetadata::new(node_count_u32, edge_count, 1, true).with_reverse_index();
        let bytes = attach_postgres_sections(&forward_bytes, Some(&inbound_bytes), &metadata)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let validated = validate_snapshot_bytes(&bytes).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(&validated, &bytes);
        let snapshot = load_snapshot_bytes(&bytes).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert!(snapshot.section(oxgraph_postgres::SNAPSHOT_KIND_PG_METADATA).is_some());
        let engine = EngineBuilder::new().snapshot_owned(bytes).build().map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(engine.stats().node_count as usize, node_count);
    }

    #[test]
    fn malformed_bytes_rejected(payload in prop::collection::vec(any::<u8>(), 0..64)) {
        if payload.starts_with(b"OXGTOPO") {
            return Ok(());
        }
        let result = write_snapshot_bytes(&payload);
        prop_assert!(matches!(result, Err(PostgresGraphError::Snapshot(_))));
    }
}
