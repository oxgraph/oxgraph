//! Property-layer proptests for descriptor validation and selected sparse views.

use std::{fmt, sync::Arc};

use arrow_array::{Float32Array, Int32Array, UInt32Array, types::Float32Type};
use arrow_schema::{DataType, Field};
use oxgraph_property::{
    DecodedPropertyData, DecodedPropertyLayer, IdFamily, LayerId, LayerRole, MissingPolicy,
    PropertyError, PropertyLayer, PropertyLayerData, PropertyLayerDescriptor, RelationAxis,
    SparseWeights, StorageMode, encode_property_snapshot, rekey_layer_to_local,
    validate_property_sections, validate_unique_layer_ids, validate_unique_names,
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

/// Builds an Int32 Arrow field for generated descriptors.
fn i32_field(name: &str) -> Field {
    Field::new(name, DataType::Int32, false)
}

/// Converts a library result into a proptest failure.
fn prop_ok<T, E: fmt::Display>(result: Result<T, E>) -> Result<T, TestCaseError> {
    result.map_err(|error| TestCaseError::fail(error.to_string()))
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Non-empty generated names are accepted and duplicate family/name pairs are rejected.
    #[test]
    fn duplicate_name_validation_is_family_scoped(name in "[a-z][a-z0-9_]{0,12}") {
        let relation = prop_ok(PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(1_u32),
            &name,
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Dense,
            f32_field(&name),
        ))?;
        let element = prop_ok(PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(2_u32),
            &name,
            IdFamily::Element,
            LayerRole::Weight,
            StorageMode::Dense,
            f32_field(&name),
        ))?;
        prop_assert!(validate_unique_names([&relation, &element]).is_ok());
        let duplicate = prop_ok(PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(3_u32),
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

    /// Duplicate layer IDs are rejected independently of layer names.
    #[test]
    fn duplicate_layer_ids_are_rejected(
        layer_id in 0_u32..1_000,
        left_name in "[a-z][a-z0-9_]{0,8}",
        right_suffix in "[a-z][a-z0-9_]{0,8}",
    ) {
        let right_name = format!("{left_name}_{right_suffix}");
        let left = prop_ok(PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(layer_id),
            &left_name,
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            i32_field(&left_name),
        ))?;
        let right = prop_ok(PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(layer_id),
            &right_name,
            IdFamily::Relation,
            LayerRole::Property,
            StorageMode::Dense,
            i32_field(&right_name),
        ))?;
        let duplicate = matches!(
            validate_unique_layer_ids([&left, &right]),
            Err(PropertyError::DuplicateLayerId { .. })
        );
        prop_assert!(duplicate);
    }

    /// Sparse totalizing relation-weight selection returns explicit values and defaults.
    #[test]
    fn sparse_relation_selection_totalizes(
        len in 1_usize..64,
        default in -10.0_f32..10.0,
        first in -100.0_f32..100.0,
    ) {
        let descriptor = prop_ok(PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(10_u32),
            "weight",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Sparse {
                missing: MissingPolicy::Default,
            },
            f32_field("weight"),
        ))?;
        let explicit_index = prop_ok(u32::try_from(len - 1))?;
        let layer = prop_ok(PropertyLayer::try_new_sparse(
            descriptor,
            len,
            Arc::new(UInt32Array::from(vec![explicit_index])),
            Arc::new(Float32Array::from(vec![first])),
            Some(Arc::new(Float32Array::from(vec![default]))),
        ))?;
        let topology = Topology { relation_bound: len };
        let selected = prop_ok(SparseWeights::<RelationAxis, _, u32, u32, Float32Type>::new(&topology, &layer))?;
        prop_assert!((selected.relation_weight(explicit_index) - first).abs() < f32::EPSILON);
        let expected_zero = if len == 1 { first } else { default };
        prop_assert!((selected.relation_weight(0) - expected_zero).abs() < f32::EPSILON);
    }

    /// Sparse index validation rejects generated out-of-bounds explicit indexes.
    #[test]
    fn sparse_index_bounds_are_checked(
        len in 1_usize..64,
        index in 0_u32..96,
    ) {
        let descriptor = prop_ok(PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(20_u32),
            "sparse",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Sparse {
                missing: MissingPolicy::Null,
            },
            i32_field("sparse"),
        ))?;
        let result = PropertyLayer::try_new_sparse(
            descriptor,
            len,
            Arc::new(UInt32Array::from(vec![index])),
            Arc::new(Int32Array::from(vec![1_i32])),
            None,
        );
        if (index as usize) < len {
            prop_assert!(result.is_ok());
        } else {
            let out_of_bounds = matches!(result, Err(PropertyError::SparseIndexOutOfBounds { .. }));
            prop_assert!(out_of_bounds);
        }
    }

    /// Dense and sparse primitive property snapshots roundtrip through validation.
    #[test]
    fn property_snapshot_roundtrips_generated_primitive_layers(
        dense_values in proptest::collection::vec(-100_i32..100, 1..32),
        sparse_pairs in proptest::collection::btree_map(0_u32..32, -10.0_f32..10.0, 0..16),
        default in -10.0_f32..10.0,
    ) {
        let logical_len = dense_values.len().max(32);
        let dense_expected = dense_values.clone();
        let dense = prop_ok(PropertyLayer::try_new_dense(
            prop_ok(PropertyLayerDescriptor::<u32, u32>::try_new(
                LayerId(30_u32),
                "dense",
                IdFamily::Element,
                LayerRole::Property,
                StorageMode::Dense,
                i32_field("dense"),
            ))?,
            Arc::new(Int32Array::from(dense_values)),
        ))?;
        let sparse_indices_expected = sparse_pairs.keys().copied().collect::<Vec<_>>();
        let sparse_values_expected = sparse_pairs.values().copied().collect::<Vec<_>>();
        let sparse = prop_ok(PropertyLayer::try_new_sparse(
            prop_ok(PropertyLayerDescriptor::<u32, u32>::try_new(
                LayerId(31_u32),
                "sparse",
                IdFamily::Relation,
                LayerRole::Weight,
                StorageMode::Sparse {
                    missing: MissingPolicy::Default,
                },
                f32_field("sparse"),
            ))?,
            logical_len,
            Arc::new(UInt32Array::from(sparse_indices_expected.clone())),
            Arc::new(Float32Array::from(sparse_values_expected.clone())),
            Some(Arc::new(Float32Array::from(vec![default]))),
        ))?;
        let encoded = prop_ok(encode_property_snapshot::<u32, u32, u32>(&[dense, sparse]))?;
        let summary = prop_ok(validate_property_sections::<u32>(&encoded.descriptors, &encoded.data))?;
        prop_assert_eq!(summary.layer_count, 2);
        let decoded = prop_ok(DecodedPropertyLayer::decode_sections::<u32>(
            &encoded.descriptors,
            &encoded.data,
        ))?;
        prop_assert_eq!(decoded.len(), 2);
        let DecodedPropertyData::Dense { values: dense_values_array } = &decoded[0].data else {
            return Err(TestCaseError::fail("dense layer did not decode as dense"));
        };
        let dense_array = dense_values_array
            .as_any()
            .downcast_ref::<Int32Array>()
            .ok_or_else(|| TestCaseError::fail("decoded dense layer is not Int32Array"))?;
        prop_assert_eq!(dense_array.values(), dense_expected.as_slice());
        prop_assert_eq!(decoded[0].name.as_str(), "dense");
        prop_assert_eq!(decoded[0].id_family, IdFamily::Element);
        prop_assert_eq!(decoded[0].role, LayerRole::Property);
        prop_assert_eq!(decoded[0].logical_len, dense_expected.len());
        let DecodedPropertyData::Sparse {
            indices: sparse_indices_array,
            values: sparse_values_array,
            default: sparse_default_array,
        } = &decoded[1].data
        else {
            return Err(TestCaseError::fail("sparse layer did not decode as sparse"));
        };
        let sparse_indices_decoded = sparse_indices_array
            .as_any()
            .downcast_ref::<UInt32Array>()
            .ok_or_else(|| TestCaseError::fail("decoded sparse indices are not UInt32Array"))?;
        let sparse_values_decoded = sparse_values_array
            .as_any()
            .downcast_ref::<Float32Array>()
            .ok_or_else(|| TestCaseError::fail("decoded sparse values are not Float32Array"))?;
        prop_assert_eq!(sparse_indices_decoded.values(), sparse_indices_expected.as_slice());
        for (decoded_value, expected_value) in sparse_values_decoded
            .values()
            .iter()
            .copied()
            .zip(sparse_values_expected.iter().copied())
        {
            prop_assert!((decoded_value - expected_value).abs() < f32::EPSILON);
        }
        let default_array = sparse_default_array
            .as_ref()
            .ok_or_else(|| TestCaseError::fail("sparse-default layer lost its default array"))?
            .as_any()
            .downcast_ref::<Float32Array>()
            .ok_or_else(|| TestCaseError::fail("decoded sparse default is not Float32Array"))?;
        prop_assert_eq!(default_array.len(), 1);
        prop_assert!((default_array.value(0) - default).abs() < f32::EPSILON);
        let sparse_default_storage = matches!(
            decoded[1].storage,
            StorageMode::Sparse {
                missing: MissingPolicy::Default,
            }
        );
        prop_assert!(sparse_default_storage);
        prop_assert_eq!(decoded[1].name.as_str(), "sparse");
        prop_assert_eq!(decoded[1].id_family, IdFamily::Relation);
        prop_assert_eq!(decoded[1].role, LayerRole::Weight);
        prop_assert_eq!(decoded[1].logical_len, logical_len);
    }

    /// Dense rekeying over generated reverse permutations preserves canonical values.
    #[test]
    fn rekey_dense_reverse_permutation(values in proptest::collection::vec(-1_000_i32..1_000, 1..64)) {
        let layer = prop_ok(PropertyLayer::try_new_dense(
            prop_ok(PropertyLayerDescriptor::<u32, u32>::try_new(
                LayerId(40_u32),
                "dense",
                IdFamily::Relation,
                LayerRole::Property,
                StorageMode::Dense,
                i32_field("dense"),
            ))?,
            Arc::new(Int32Array::from(values.clone())),
        ))?;
        let local_to_canonical = (0..values.len())
            .rev()
            .map(|value| u32::try_from(value).map_err(|error| TestCaseError::fail(error.to_string())))
            .collect::<Result<Vec<_>, _>>()?;
        let rekeyed = prop_ok(rekey_layer_to_local(&layer, &local_to_canonical))?;
        let PropertyLayerData::Dense { values: array } = rekeyed.data() else {
            return Err(TestCaseError::fail("dense layer did not stay dense"));
        };
        let array = array
            .as_any()
            .downcast_ref::<Int32Array>()
            .ok_or_else(|| TestCaseError::fail("rekeyed dense layer type changed"))?;
        let expected = values.into_iter().rev().collect::<Vec<_>>();
        prop_assert_eq!(array.values(), expected.as_slice());
    }

    /// Sparse rekeying maps generated explicit canonical indexes into local order.
    #[test]
    fn rekey_sparse_reverse_permutation(
        len in 2_usize..64,
        first in -100.0_f32..100.0,
        last in -100.0_f32..100.0,
        default in -10.0_f32..10.0,
    ) {
        let last_index = prop_ok(u32::try_from(len - 1))?;
        let layer = prop_ok(PropertyLayer::try_new_sparse(
            prop_ok(PropertyLayerDescriptor::<u32, u32>::try_new(
                LayerId(41_u32),
                "sparse",
                IdFamily::Relation,
                LayerRole::Weight,
                StorageMode::Sparse {
                    missing: MissingPolicy::Default,
                },
                f32_field("sparse"),
            ))?,
            len,
            Arc::new(UInt32Array::from(vec![0_u32, last_index])),
            Arc::new(Float32Array::from(vec![first, last])),
            Some(Arc::new(Float32Array::from(vec![default]))),
        ))?;
        let local_to_canonical = (0..len)
            .rev()
            .map(|value| u32::try_from(value).map_err(|error| TestCaseError::fail(error.to_string())))
            .collect::<Result<Vec<_>, _>>()?;
        let rekeyed = prop_ok(rekey_layer_to_local(&layer, &local_to_canonical))?;
        let PropertyLayerData::Sparse { indices, values, .. } = rekeyed.data() else {
            return Err(TestCaseError::fail("sparse layer did not stay sparse"));
        };
        prop_assert_eq!(indices.values(), &[0, last_index]);
        let values = values
            .as_any()
            .downcast_ref::<Float32Array>()
            .ok_or_else(|| TestCaseError::fail("rekeyed sparse layer type changed"))?;
        prop_assert!((values.value(0) - last).abs() < f32::EPSILON);
        prop_assert!((values.value(1) - first).abs() < f32::EPSILON);
    }
}
