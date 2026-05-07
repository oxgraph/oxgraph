//! Hypergraph builder proptests for participant normalization and snapshot validation.

use std::sync::Arc;

use arrow_array::Int32Array;
use arrow_schema::{DataType, Field};
use oxgraph_hyper::{ContainsElement, ContainsRelation, HypergraphCounts, LocalElementIdentity};
use oxgraph_hyper_bcsr::{BcsrSnapshotHypergraph, BcsrValidation};
use oxgraph_hyper_build::{
    HyperBuildError, HyperVertexId, HyperedgeId, HypergraphBuilder,
    export_bcsr_snapshot_with_properties,
};
use oxgraph_property::{
    HyperPropertyLayers, IdFamily, LayerId, LayerRole, PropertyLayer, PropertyLayerDescriptor,
    StorageMode, validate_identity_snapshot, validate_property_snapshot,
};
use oxgraph_snapshot::Snapshot;
use proptest::{prelude::*, test_runner::TestCaseError};

/// Converts hypergraph build results into proptest failures.
fn prop_hyper<T>(result: Result<T, HyperBuildError<u32, u32, u32>>) -> Result<T, TestCaseError> {
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
        let mut builder = HypergraphBuilder::<u32, u32, u32>::new();
        for _ in 0..vertex_count {
            prop_hyper(builder.add_vertex())?;
        }
        for (source, target) in pairs {
            if source < vertex_count && target < vertex_count && source != target {
                prop_hyper(builder.add_hyperedge(&[HyperVertexId(source)], &[HyperVertexId(target)]))?;
            }
        }
        let descriptor = prop_property(PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(10_u32),
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
        let frozen = prop_hyper(builder.freeze())?;
        for vertex in 0..vertex_count {
            let id = HyperVertexId(vertex);
            prop_assert!(frozen.contains_element(id));
            prop_assert_eq!(frozen.local_element_id(id), Some(id));
        }
        let relation_count = u32::try_from(frozen.hyperedge_count())
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        for relation in 0..relation_count {
            prop_assert!(frozen.contains_relation(HyperedgeId(relation)));
        }
        let element_layers = [layer];
        let bytes = prop_hyper(export_bcsr_snapshot_with_properties(
            &frozen,
            HyperPropertyLayers {
                element: &element_layers,
                relation: &[],
                incidence: &[],
            },
        ))?;
        let snapshot = Snapshot::open(&bytes)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let _opened = BcsrSnapshotHypergraph::<u32, u32, u32>::from_snapshot_with(&snapshot, BcsrValidation::Strict)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let _identity = validate_identity_snapshot::<u32>(&snapshot)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let _property = validate_property_snapshot::<u32>(&snapshot)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
    }
}
