//! Hypergraph builder proptests for participant normalization and snapshot validation.

use std::sync::Arc;

use arrow_array::Int32Array;
use arrow_schema::{DataType, Field};
use oxgraph_hyper::{ContainsElement, ContainsRelation, HypergraphCounts, LocalElementIdentity};
use oxgraph_hyper_bcsr::{BcsrHypergraph, BcsrValidation};
use oxgraph_hyper_build::{HyperBuildError, HyperVertexId, HypergraphBuilder};
use oxgraph_property::{
    IdFamily, LayerId, LayerRole, PropertyLayer, PropertyLayerDescriptor, StorageMode,
    validate_identity_snapshot, validate_property_snapshot,
};
use oxgraph_snapshot::Snapshot;
use proptest::{prelude::*, test_runner::TestCaseError};

/// Converts hypergraph build results into proptest failures.
fn prop_hyper<T>(result: Result<T, HyperBuildError>) -> Result<T, TestCaseError> {
    result.map_err(|error| TestCaseError::fail(error.to_string()))
}

/// Converts property results into proptest failures.
fn prop_property<T>(
    result: Result<T, oxgraph_property::PropertyError>,
) -> Result<T, TestCaseError> {
    result.map_err(|error| TestCaseError::fail(error.to_string()))
}

proptest! {
    /// Generated directed hyperedges freeze to strict BCSR snapshots.
    #[test]
    fn random_hypergraph_exports_strict_bcsr(
        vertex_count in 1_u32..24,
        pairs in prop::collection::vec((0_u32..24, 0_u32..24), 0..64),
    ) {
        let mut builder = HypergraphBuilder::new(0_i16, 1_u32, 2_i8);
        for _ in 0..vertex_count {
            prop_hyper(builder.add_vertex())?;
        }
        for (source, target) in pairs {
            if source < vertex_count && target < vertex_count && source != target {
                prop_hyper(builder.add_hyperedge(&[HyperVertexId(source)], &[HyperVertexId(target)]))?;
            }
        }
        let descriptor = prop_property(PropertyLayerDescriptor::try_new(
            LayerId(10),
            "vertex_marker",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            Field::new("vertex_marker", DataType::Int32, false),
        ))?;
        let layer = prop_property(PropertyLayer::try_new_dense(
            descriptor,
            Arc::new(Int32Array::from(vec![1_i32; vertex_count as usize])),
        ))?;
        builder.add_property_layer(layer);
        let frozen = prop_hyper(builder.freeze())?;
        for vertex in 0..vertex_count {
            let id = HyperVertexId(vertex);
            prop_assert!(frozen.contains_element(id));
            prop_assert_eq!(frozen.local_element_id(id), Some(id));
        }
        let relation_count = u32::try_from(frozen.hyperedge_count())
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        for relation in 0..relation_count {
            prop_assert!(frozen.contains_relation(oxgraph_hyper_build::HyperedgeId(relation)));
        }
        let bytes = prop_hyper(frozen.to_bcsr_snapshot())?;
        let snapshot = Snapshot::open(&bytes)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let _opened = BcsrHypergraph::from_snapshot_with(&snapshot, BcsrValidation::Strict)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let _identity = validate_identity_snapshot(&snapshot)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let _property = validate_property_snapshot(&snapshot)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
    }
}
