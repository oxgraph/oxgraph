//! Graph builder proptests for freeze, identity, and snapshot sections.

use std::sync::Arc;

use arrow_array::Int32Array;
use arrow_schema::{DataType, Field};
use oxgraph_graph::{
    ContainsElement, ContainsRelation, GraphCounts, LocalElementIdentity, LocalRelationIdentity,
};
use oxgraph_graph_build::{GraphBuildError, GraphBuilder, GraphNodeId};
use oxgraph_property::{
    IdFamily, LayerId, LayerRole, PropertyLayer, PropertyLayerDescriptor, StorageMode,
    validate_identity_snapshot, validate_property_snapshot,
};
use oxgraph_snapshot::Snapshot;
use proptest::{prelude::*, test_runner::TestCaseError};

/// Converts graph build results into proptest failures.
fn prop_graph<T>(result: Result<T, GraphBuildError>) -> Result<T, TestCaseError> {
    result.map_err(|error| TestCaseError::fail(error.to_string()))
}

/// Converts property results into proptest failures.
fn prop_property<T>(
    result: Result<T, oxgraph_property::PropertyError>,
) -> Result<T, TestCaseError> {
    result.map_err(|error| TestCaseError::fail(error.to_string()))
}

proptest! {
    /// Random edge lists freeze into views with stable local/canonical IDs and validating snapshots.
    #[test]
    fn random_graph_freeze_preserves_identity(
        node_count in 1_u32..32,
        edges in prop::collection::vec((0_u32..32, 0_u32..32), 0..128),
    ) {
        let mut builder = GraphBuilder::new(0_i16, 1_u32);
        for _ in 0..node_count {
            prop_graph(builder.add_node())?;
        }
        for (source, target) in edges {
            if source < node_count && target < node_count {
                prop_graph(builder.add_edge(GraphNodeId(source), GraphNodeId(target)))?;
            }
        }
        let relation_values = vec![7_i32; builder.edge_count()];
        let descriptor = prop_property(PropertyLayerDescriptor::try_new(
            LayerId(10),
            "relation_marker",
            IdFamily::Relation,
            LayerRole::Property,
            StorageMode::Dense,
            Field::new("relation_marker", DataType::Int32, false),
        ))?;
        let layer = prop_property(PropertyLayer::try_new_dense(
            descriptor,
            Arc::new(Int32Array::from(relation_values)),
        ))?;
        prop_graph(builder.add_property_layer(layer))?;
        let frozen = prop_graph(builder.freeze())?;
        for node in 0..node_count {
            let id = GraphNodeId(node);
            prop_assert!(frozen.contains_element(id));
            prop_assert_eq!(frozen.local_element_id(id), Some(id));
        }
        let edge_count = u32::try_from(frozen.edge_count())
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        for edge in 0..edge_count {
            prop_assert!(frozen.contains_relation(oxgraph_graph_build::GraphEdgeId(edge)));
            prop_assert_eq!(
                frozen.local_relation_id(oxgraph_graph_build::GraphEdgeId(edge)),
                Some(oxgraph_graph_build::GraphEdgeId(edge))
            );
        }
        let bytes = prop_graph(frozen.to_csr_snapshot())?;
        let snapshot = Snapshot::open(&bytes)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let _identity = validate_identity_snapshot(&snapshot)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let _property = validate_property_snapshot(&snapshot)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
    }
}
