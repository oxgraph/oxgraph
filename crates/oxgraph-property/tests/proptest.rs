//! Property-layer proptests for descriptor validation and selected sparse views.

use std::{fmt, sync::Arc};

use arrow_array::{Float32Array, UInt64Array, types::Float32Type};
use arrow_schema::{DataType, Field};
use oxgraph_property::{
    IdFamily, LayerId, LayerRole, MissingPolicy, PropertyError, PropertyLayer,
    PropertyLayerDescriptor, SparseRelationWeights, StorageMode, validate_unique_names,
};
use oxgraph_topology::{RelationIndex, RelationWeight, TopologyBase};
use proptest::{prelude::*, test_runner::TestCaseError};

/// Test topology with dense relation IDs.
#[derive(Clone, Copy, Debug)]
struct Topology {
    /// Relation bound for generated layers.
    relation_bound: usize,
}

impl TopologyBase for Topology {
    type ElementId = u32;
    type RelationId = u32;
}

impl RelationIndex for Topology {
    fn relation_bound(&self) -> usize {
        self.relation_bound
    }

    fn relation_index(&self, relation: Self::RelationId) -> usize {
        relation as usize
    }
}

/// Builds a Float32 Arrow field for generated descriptors.
fn f32_field(name: &str) -> Field {
    Field::new(name, DataType::Float32, false)
}

/// Converts a library result into a proptest failure.
fn prop_ok<T, E: fmt::Display>(result: Result<T, E>) -> Result<T, TestCaseError> {
    result.map_err(|error| TestCaseError::fail(error.to_string()))
}

proptest! {
    /// Non-empty generated names are accepted and duplicate family/name pairs are rejected.
    #[test]
    fn duplicate_name_validation_is_family_scoped(name in "[a-z][a-z0-9_]{0,12}") {
        let relation = prop_ok(PropertyLayerDescriptor::try_new(
            LayerId(1),
            &name,
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Dense,
            f32_field(&name),
        ))?;
        let element = prop_ok(PropertyLayerDescriptor::try_new(
            LayerId(2),
            &name,
            IdFamily::Element,
            LayerRole::Weight,
            StorageMode::Dense,
            f32_field(&name),
        ))?;
        prop_assert!(validate_unique_names([&relation, &element]).is_ok());
        let duplicate = prop_ok(PropertyLayerDescriptor::try_new(
            LayerId(3),
            &name,
            IdFamily::Relation,
            LayerRole::Property,
            StorageMode::Dense,
            f32_field(&name),
        ))?;
        let duplicate_result = validate_unique_names([&relation, &duplicate]);
        if !matches!(duplicate_result, Err(PropertyError::DuplicateName { .. })) {
            return Err(TestCaseError::fail("duplicate relation name was accepted"));
        }
    }

    /// Sparse totalizing relation-weight selection returns explicit values and defaults.
    #[test]
    fn sparse_relation_selection_totalizes(
        len in 1_usize..64,
        default in -10.0_f32..10.0,
        first in -100.0_f32..100.0,
    ) {
        let descriptor = prop_ok(PropertyLayerDescriptor::try_new(
            LayerId(10),
            "weight",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Sparse {
                missing: MissingPolicy::Default,
            },
            f32_field("weight"),
        ))?;
        let explicit_index = (len - 1) as u64;
        let layer = prop_ok(PropertyLayer::try_new_sparse(
            descriptor,
            len,
            Arc::new(UInt64Array::from(vec![explicit_index])),
            Arc::new(Float32Array::from(vec![first])),
            Some(Arc::new(Float32Array::from(vec![default]))),
        ))?;
        let topology = Topology { relation_bound: len };
        let selected = prop_ok(SparseRelationWeights::<_, Float32Type>::new(&topology, &layer))?;
        let explicit_relation = prop_ok(u32::try_from(explicit_index))?;
        prop_assert!((selected.relation_weight(explicit_relation) - first).abs() < f32::EPSILON);
        let expected_zero = if len == 1 { first } else { default };
        prop_assert!((selected.relation_weight(0) - expected_zero).abs() < f32::EPSILON);
    }
}
