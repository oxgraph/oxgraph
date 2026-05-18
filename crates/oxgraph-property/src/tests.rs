//! Unit tests for generic property descriptor and selected weight validation.

use arrow_array::{
    BooleanArray, Float32Array, Float64Array, Int32Array, StringArray, UInt32Array, UInt64Array,
    types::{Float32Type, Int32Type},
};
use oxgraph_snapshot::{Snapshot, SnapshotBuilder};

use super::*;

/// Test topology with dense relation IDs.
#[derive(Clone, Copy)]
struct Topology;

impl TopologyBase for Topology {
    type ElementId = u32;
    type RelationId = u32;
}

impl RelationIndex for Topology {
    fn relation_bound(&self) -> usize {
        2
    }

    fn relation_index(&self, relation: Self::RelationId) -> usize {
        relation as usize
    }
}

/// Builds an Arrow field for test descriptors.
///
/// # Performance
///
/// This function is `O(name.len())`.
fn field(name: &str, data_type: DataType) -> Field {
    Field::new(name, data_type, false)
}

/// Dense relation layers can be selected by relation index without f64.
#[test]
fn dense_relation_layer_selects_i32_weight() -> Result<(), PropertyError> {
    let descriptor = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(1_u32),
        "count",
        IdFamily::Relation,
        LayerRole::Weight,
        StorageMode::Dense,
        field("count", DataType::Int32),
    )?;
    let layer = PropertyLayer::try_new_dense(descriptor, Arc::new(Int32Array::from(vec![2, 7])))?;
    let selected = DenseWeights::<RelationAxis, _, u32, u32, Int32Type>::new(&Topology, &layer)?;
    assert_eq!(selected.relation_weight(1), 7);
    Ok(())
}

/// Sparse relation layers can totalize with an Arrow default scalar.
#[test]
fn sparse_relation_layer_totalizes_with_default() -> Result<(), PropertyError> {
    let descriptor = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(2_u32),
        "capacity",
        IdFamily::Relation,
        LayerRole::Weight,
        StorageMode::Sparse {
            missing: MissingPolicy::Default,
        },
        field("capacity", DataType::Float32),
    )?;
    let layer = PropertyLayer::try_new_sparse(
        descriptor,
        4,
        Arc::new(UInt32Array::from(vec![1_u32])),
        Arc::new(Float32Array::from(vec![3.5_f32])),
        Some(Arc::new(Float32Array::from(vec![1.25_f32]))),
    )?;
    let selected = SparseWeights::<RelationAxis, _, u32, u32, Float32Type>::new(&Topology, &layer)?;
    assert!((selected.relation_weight(0) - 1.25).abs() < f32::EPSILON);
    assert!((selected.relation_weight(1) - 3.5).abs() < f32::EPSILON);
    Ok(())
}

/// Dense and sparse property layers support every explicit sparse-index width.
#[test]
fn property_layers_support_u16_u32_and_u64_indexes() -> Result<(), PropertyError> {
    let dense_u16 = PropertyLayer::try_new_dense(
        PropertyLayerDescriptor::<u16, u16>::try_new(
            LayerId(1_u16),
            "dense_u16",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            field("dense_u16", DataType::Int32),
        )?,
        Arc::new(Int32Array::from(vec![1, 2])),
    )?;
    assert_eq!(dense_u16.len(), 2);

    let sparse_u32 = PropertyLayer::try_new_sparse(
        PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(2_u32),
            "sparse_u32",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Sparse {
                missing: MissingPolicy::Default,
            },
            field("sparse_u32", DataType::Float32),
        )?,
        8,
        Arc::new(UInt32Array::from(vec![1_u32, 7])),
        Arc::new(Float32Array::from(vec![2.0_f32, 4.0])),
        Some(Arc::new(Float32Array::from(vec![1.0_f32]))),
    )?;
    assert_eq!(sparse_u32.len(), 8);

    let sparse_u64 = PropertyLayer::try_new_sparse(
        PropertyLayerDescriptor::<u64, u64>::try_new(
            LayerId(3_u64),
            "sparse_u64",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Sparse {
                missing: MissingPolicy::Null,
            },
            field("sparse_u64", DataType::Int32),
        )?,
        9,
        Arc::new(UInt64Array::from(vec![0_u64, 8])),
        Arc::new(Int32Array::from(vec![5_i32, 6])),
        None,
    )?;
    assert_eq!(sparse_u64.len(), 9);
    Ok(())
}

/// Nullable sparse properties can be valid properties but invalid total weights.
#[test]
fn sparse_selected_weights_reject_explicit_nulls() -> Result<(), PropertyError> {
    let descriptor = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(4_u32),
        "nullable_weight",
        IdFamily::Relation,
        LayerRole::Weight,
        StorageMode::Sparse {
            missing: MissingPolicy::Default,
        },
        Field::new("nullable_weight", DataType::Float32, true),
    )?;
    let values = Float32Array::from(vec![Some(3.0_f32), None]);
    let layer = PropertyLayer::try_new_sparse(
        descriptor,
        3,
        Arc::new(UInt32Array::from(vec![0_u32, 2])),
        Arc::new(values),
        Some(Arc::new(Float32Array::from(vec![1.0_f32]))),
    )?;

    assert!(matches!(
        SparseWeights::<RelationAxis, _, u32, u32, Float32Type>::new(&Topology, &layer),
        Err(PropertyError::UnexpectedNull { index: 1 })
    ));
    Ok(())
}

/// Duplicate names are rejected only within the same ID family.
#[test]
fn duplicate_names_are_family_scoped() -> Result<(), PropertyError> {
    let first = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(1_u32),
        "weight",
        IdFamily::Relation,
        LayerRole::Weight,
        StorageMode::Dense,
        field("weight", DataType::UInt32),
    )?;
    let second = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(2_u32),
        "weight",
        IdFamily::Element,
        LayerRole::Weight,
        StorageMode::Dense,
        field("weight", DataType::UInt32),
    )?;
    let duplicate = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(3_u32),
        "weight",
        IdFamily::Relation,
        LayerRole::Property,
        StorageMode::Dense,
        field("weight", DataType::UInt32),
    )?;
    assert!(validate_unique_names([&first, &second]).is_ok());
    assert!(matches!(
        validate_unique_names([&first, &duplicate]),
        Err(PropertyError::DuplicateName { .. })
    ));
    Ok(())
}

/// Descriptors preserve exact Arrow fields for non-floating Arrow properties.
#[test]
fn descriptor_preserves_generic_arrow_fields() -> Result<(), PropertyError> {
    let boolean = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(1_u32),
        "flag",
        IdFamily::Element,
        LayerRole::Property,
        StorageMode::Dense,
        Field::new("flag", DataType::Boolean, false),
    )?;
    let utf8 = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(2_u32),
        "label",
        IdFamily::Element,
        LayerRole::Property,
        StorageMode::Dense,
        Field::new("label", DataType::Utf8, true),
    )?;
    assert_eq!(boolean.arrow_field.data_type(), &DataType::Boolean);
    assert_eq!(utf8.arrow_field.data_type(), &DataType::Utf8);
    Ok(())
}

/// Property descriptor/data sections roundtrip through Arrow IPC payloads.
#[test]
fn property_snapshot_sections_validate() -> Result<(), Box<dyn Error>> {
    let dense_descriptor = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(10_u32),
        "count",
        IdFamily::Relation,
        LayerRole::Weight,
        StorageMode::Dense,
        field("count", DataType::UInt32),
    )?;
    let dense = PropertyLayer::try_new_dense(
        dense_descriptor,
        Arc::new(UInt32Array::from(vec![4_u32, 9_u32])),
    )?;
    let sparse_descriptor = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(11_u32),
        "score",
        IdFamily::Element,
        LayerRole::Property,
        StorageMode::Sparse {
            missing: MissingPolicy::Default,
        },
        field("score", DataType::Float32),
    )?;
    let sparse = PropertyLayer::try_new_sparse(
        sparse_descriptor,
        3,
        Arc::new(UInt32Array::from(vec![2_u32])),
        Arc::new(Float32Array::from(vec![8.0_f32])),
        Some(Arc::new(Float32Array::from(vec![1.0_f32]))),
    )?;
    let encoded = encode_property_snapshot::<u32, u32, u32>(&[dense, sparse])?;
    let mut builder = SnapshotBuilder::new();
    builder.add_section(
        SNAPSHOT_KIND_PROPERTY_DESCRIPTORS_U32,
        SNAPSHOT_PROPERTY_VERSION,
        0,
        encoded.descriptors,
    )?;
    builder.add_section(
        SNAPSHOT_KIND_PROPERTY_DATA_U32,
        SNAPSHOT_PROPERTY_VERSION,
        0,
        encoded.data,
    )?;
    let bytes = builder.finish()?;
    let snapshot = Snapshot::open(&bytes)?;
    let summary = validate_property_snapshot::<u32>(&snapshot)?;
    assert_eq!(summary.layer_count, 2);
    assert_eq!(summary.total_logical_values, 5);
    Ok(())
}

/// Snapshot validation accepts exact Arrow IPC payloads for bool/int/float/string layers.
#[test]
fn property_snapshot_validates_exact_arrow_payload_shapes() -> Result<(), Box<dyn Error>> {
    let bool_layer = PropertyLayer::try_new_dense(
        PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(20_u32),
            "flag",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            field("flag", DataType::Boolean),
        )?,
        Arc::new(BooleanArray::from(vec![true, false])),
    )?;
    let int_layer = PropertyLayer::try_new_dense(
        PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(21_u32),
            "count",
            IdFamily::Relation,
            LayerRole::Property,
            StorageMode::Dense,
            field("count", DataType::Int32),
        )?,
        Arc::new(Int32Array::from(vec![1_i32, 2])),
    )?;
    let float_layer = PropertyLayer::try_new_dense(
        PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(22_u32),
            "score",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            field("score", DataType::Float64),
        )?,
        Arc::new(Float64Array::from(vec![1.5_f64, 2.5])),
    )?;
    let string_layer = PropertyLayer::try_new_dense(
        PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(23_u32),
            "label",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            field("label", DataType::Utf8),
        )?,
        Arc::new(StringArray::from(vec!["a", "b"])),
    )?;

    let encoded = encode_property_snapshot::<u32, u32, u32>(&[
        bool_layer,
        int_layer,
        float_layer,
        string_layer,
    ])?;
    let summary = validate_property_sections::<u32>(&encoded.descriptors, &encoded.data)?;
    assert_eq!(summary.layer_count, 4);
    assert_eq!(summary.total_logical_values, 8);
    Ok(())
}

/// Sparse-default snapshots store explicit values and default scalar as separate streams.
#[test]
fn sparse_default_snapshot_accepts_zero_one_and_many_explicit_values() -> Result<(), PropertyError>
{
    for (layer_id, indices, values) in [
        (30_u32, Vec::new(), Vec::new()),
        (31_u32, vec![1_u32], vec![4.0_f32]),
        (32_u32, vec![0_u32, 2, 4], vec![2.0_f32, 3.0, 5.0]),
    ] {
        let layer = PropertyLayer::try_new_sparse(
            PropertyLayerDescriptor::<u32, u32>::try_new(
                LayerId(layer_id),
                "score",
                IdFamily::Element,
                LayerRole::Property,
                StorageMode::Sparse {
                    missing: MissingPolicy::Default,
                },
                field("score", DataType::Float32),
            )?,
            5,
            Arc::new(UInt32Array::from(indices)),
            Arc::new(Float32Array::from(values)),
            Some(Arc::new(Float32Array::from(vec![1.0_f32]))),
        )?;
        let encoded = encode_property_snapshot::<u32, u32, u32>(&[layer])?;
        let summary = validate_property_sections::<u32>(&encoded.descriptors, &encoded.data)?;
        assert_eq!(summary.layer_count, 1);
        assert_eq!(summary.total_logical_values, 5);
    }
    Ok(())
}

/// Invalid property data sections are rejected structurally.
#[test]
fn property_snapshot_rejects_trailing_data() -> Result<(), PropertyError> {
    let descriptor = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(1_u32),
        "count",
        IdFamily::Relation,
        LayerRole::Weight,
        StorageMode::Dense,
        field("count", DataType::Int32),
    )?;
    let dense = PropertyLayer::try_new_dense(descriptor, Arc::new(Int32Array::from(vec![1_i32])))?;
    let mut encoded = encode_property_snapshot::<u32, u32, u32>(&[dense])?;
    encoded.data.push(0);
    assert!(matches!(
        validate_property_sections::<u32>(&encoded.descriptors, &encoded.data),
        Err(PropertyError::SnapshotDescriptorMismatch { .. })
    ));
    Ok(())
}

/// Sparse-default descriptor corruption is rejected before consumers see payloads.
#[test]
fn sparse_default_snapshot_rejects_malformed_default_ranges() -> Result<(), PropertyError> {
    let descriptor = PropertyLayerDescriptor::<u32, u32>::try_new(
        LayerId(40_u32),
        "score",
        IdFamily::Element,
        LayerRole::Property,
        StorageMode::Sparse {
            missing: MissingPolicy::Default,
        },
        field("score", DataType::Float32),
    )?;
    let layer = PropertyLayer::try_new_sparse(
        descriptor,
        3,
        Arc::new(UInt32Array::from(vec![1_u32])),
        Arc::new(Float32Array::from(vec![8.0_f32])),
        Some(Arc::new(Float32Array::from(vec![1.0_f32]))),
    )?;
    let encoded = encode_property_snapshot::<u32, u32, u32>(&[layer])?;
    let record_start = core::mem::size_of::<PropertySnapshotHeader>();
    let word_len = core::mem::size_of::<U32<LE>>();
    let default_offset_start = record_start + (11 * word_len);
    let default_len_start = record_start + (12 * word_len);

    let mut missing_default = encoded.descriptors.clone();
    missing_default[default_len_start..default_len_start + word_len]
        .copy_from_slice(&0_u32.to_le_bytes());
    assert!(matches!(
        validate_property_sections::<u32>(&missing_default, &encoded.data),
        Err(PropertyError::SnapshotDescriptorMismatch { .. })
    ));

    let mut overlapping_default = encoded.descriptors;
    let value_offset =
        overlapping_default[record_start + (9 * word_len)..record_start + (10 * word_len)].to_vec();
    overlapping_default[default_offset_start..default_offset_start + word_len]
        .copy_from_slice(&value_offset);
    assert!(matches!(
        validate_property_sections::<u32>(&overlapping_default, &encoded.data),
        Err(PropertyError::SnapshotDescriptorMismatch { .. } | PropertyError::Arrow { .. })
    ));
    Ok(())
}

/// Rekeying dense layers uses Arrow take and works for primitive and string values.
#[test]
fn rekey_dense_layers_to_snapshot_local_order() -> Result<(), PropertyError> {
    let int_layer = PropertyLayer::try_new_dense(
        PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(50_u32),
            "count",
            IdFamily::Relation,
            LayerRole::Property,
            StorageMode::Dense,
            field("count", DataType::Int32),
        )?,
        Arc::new(Int32Array::from(vec![10_i32, 20, 30])),
    )?;
    let rekeyed_int = rekey_layer_to_local(&int_layer, &[2_u32, 0, 1])?;
    let PropertyLayerData::Dense { values } = rekeyed_int.data() else {
        unreachable!("rekeyed dense layer stays dense");
    };
    let values = values.as_any().downcast_ref::<Int32Array>().ok_or(
        PropertyError::SnapshotDescriptorMismatch {
            reason: "rekeyed int layer has wrong type",
        },
    )?;
    assert_eq!(values.values(), &[30, 10, 20]);

    let string_layer = PropertyLayer::try_new_dense(
        PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(51_u32),
            "label",
            IdFamily::Element,
            LayerRole::Property,
            StorageMode::Dense,
            field("label", DataType::Utf8),
        )?,
        Arc::new(StringArray::from(vec!["a", "b", "c"])),
    )?;
    let rekeyed_string = rekey_layer_to_local(&string_layer, &[1_u32, 2])?;
    let PropertyLayerData::Dense { values } = rekeyed_string.data() else {
        unreachable!("rekeyed dense layer stays dense");
    };
    let values = values.as_any().downcast_ref::<StringArray>().ok_or(
        PropertyError::SnapshotDescriptorMismatch {
            reason: "rekeyed string layer has wrong type",
        },
    )?;
    assert_eq!(values.value(0), "b");
    assert_eq!(values.value(1), "c");
    assert_eq!(rekeyed_string.len(), 2);
    Ok(())
}

/// Rekeying sparse layers remaps explicit indexes and preserves defaults.
#[test]
fn rekey_sparse_layers_to_snapshot_local_order() -> Result<(), PropertyError> {
    let layer = PropertyLayer::try_new_sparse(
        PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(52_u32),
            "score",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Sparse {
                missing: MissingPolicy::Default,
            },
            field("score", DataType::Float64),
        )?,
        4,
        Arc::new(UInt32Array::from(vec![0_u32, 3])),
        Arc::new(Float64Array::from(vec![1.5_f64, 9.5])),
        Some(Arc::new(Float64Array::from(vec![2.5_f64]))),
    )?;
    let rekeyed = rekey_layer_to_local(&layer, &[3_u32, 0])?;
    let PropertyLayerData::Sparse {
        indices,
        values,
        default,
    } = rekeyed.data()
    else {
        unreachable!("rekeyed sparse layer stays sparse");
    };
    assert_eq!(indices.values(), &[0, 1]);
    let values = values.as_any().downcast_ref::<Float64Array>().ok_or(
        PropertyError::SnapshotDescriptorMismatch {
            reason: "rekeyed float layer has wrong type",
        },
    )?;
    assert_eq!(values.values(), &[9.5, 1.5]);
    let default = default
        .as_ref()
        .and_then(|array| array.as_any().downcast_ref::<Float64Array>())
        .ok_or(PropertyError::SnapshotDescriptorMismatch {
            reason: "rekeyed sparse default has wrong type",
        })?;
    assert!((default.value(0) - 2.5).abs() <= f64::EPSILON);
    assert_eq!(rekeyed.len(), 2);
    Ok(())
}

/// Identity mode sections validate local-equals and explicit map modes.
#[test]
fn identity_snapshot_sections_validate() -> Result<(), Box<dyn Error>> {
    let modes = [
        IdentityModeRecord::<u32>::local_equals_canonical(IdFamily::Element, 3)?,
        IdentityModeRecord::<u32>::explicit_map(IdFamily::Relation, 2)?,
    ];
    let maps = [U32::<LE>::new(10), U32::<LE>::new(12)];
    let mut builder = SnapshotBuilder::new();
    builder.add_section_typed(
        SNAPSHOT_KIND_IDENTITY_MODES_U32,
        SNAPSHOT_PROPERTY_VERSION,
        &modes,
    )?;
    builder.add_section_typed(
        SNAPSHOT_KIND_RELATION_IDENTITY_MAP_U32,
        SNAPSHOT_PROPERTY_VERSION,
        &maps,
    )?;
    let bytes = builder.finish()?;
    let snapshot = Snapshot::open(&bytes)?;
    let summary = validate_identity_snapshot::<u32>(&snapshot)?;
    assert_eq!(summary.records.len(), 2);
    Ok(())
}

/// Identity map section validation follows the selected canonical ID width.
#[test]
fn identity_snapshot_sections_validate_u16_u32_and_u64() -> Result<(), Box<dyn Error>> {
    let modes_u16 = [IdentityModeRecord::<u16>::explicit_map(
        IdFamily::Element,
        2,
    )?];
    let maps_u16 = [U16::<LE>::new(1), U16::<LE>::new(3)];
    let mut builder = SnapshotBuilder::new();
    builder.add_section_typed(
        SNAPSHOT_KIND_IDENTITY_MODES_U16,
        SNAPSHOT_PROPERTY_VERSION,
        &modes_u16,
    )?;
    builder.add_section_typed(
        SNAPSHOT_KIND_ELEMENT_IDENTITY_MAP_U16,
        SNAPSHOT_PROPERTY_VERSION,
        &maps_u16,
    )?;
    let bytes = builder.finish()?;
    let snapshot = Snapshot::open(&bytes)?;
    assert_eq!(
        validate_identity_snapshot::<u16>(&snapshot)?.records.len(),
        1
    );

    let modes_u64 = [IdentityModeRecord::<u64>::explicit_map(
        IdFamily::Incidence,
        2,
    )?];
    let maps_u64 = [U64::<LE>::new(10), U64::<LE>::new(12)];
    let mut builder = SnapshotBuilder::new();
    builder.add_section_typed(
        SNAPSHOT_KIND_IDENTITY_MODES_U64,
        SNAPSHOT_PROPERTY_VERSION,
        &modes_u64,
    )?;
    builder.add_section_typed(
        SNAPSHOT_KIND_INCIDENCE_IDENTITY_MAP_U64,
        SNAPSHOT_PROPERTY_VERSION,
        &maps_u64,
    )?;
    let bytes = builder.finish()?;
    let snapshot = Snapshot::open(&bytes)?;
    assert_eq!(
        validate_identity_snapshot::<u64>(&snapshot)?.records.len(),
        1
    );
    Ok(())
}
