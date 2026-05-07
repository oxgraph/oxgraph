//! Criterion benchmarks for generic property selected-weight lookup.

use std::sync::Arc;

use arrow_array::{Float32Array, UInt64Array, types::Float32Type};
use arrow_schema::{DataType, Field};
use criterion::{Criterion, criterion_group, criterion_main};
use oxgraph_property::{
    DenseRelationWeights, IdFamily, LayerId, LayerRole, MissingPolicy, PropertyLayer,
    PropertyLayerDescriptor, SparseRelationWeights, StorageMode,
};
use oxgraph_topology::{RelationIndex, RelationWeight, TopologyBase};

/// Benchmark topology with dense relation IDs.
struct BenchTopology {
    /// Relation count.
    relations: usize,
}

impl TopologyBase for BenchTopology {
    type ElementId = u32;
    type RelationId = u32;
}

impl RelationIndex for BenchTopology {
    fn relation_bound(&self) -> usize {
        self.relations
    }

    fn relation_index(&self, relation: Self::RelationId) -> usize {
        relation as usize
    }
}

/// Builds a Float32 field for benchmark descriptors.
fn f32_field(name: &str) -> Field {
    Field::new(name, DataType::Float32, false)
}

/// Converts a benchmark index to `f32`.
#[expect(
    clippy::cast_precision_loss,
    reason = "benchmark indexes are capped at 10k and exactly representable as f32"
)]
const fn index_to_f32(index: usize) -> f32 {
    index as f32
}

/// Converts a benchmark index to `u32`.
fn index_to_u32(index: usize) -> u32 {
    u32::try_from(index)
        .unwrap_or_else(|error| panic!("benchmark relation count should fit u32: {error}"))
}

/// Converts a benchmark index to `u64`.
fn index_to_u64(index: usize) -> u64 {
    u64::try_from(index)
        .unwrap_or_else(|error| panic!("benchmark relation count should fit u64: {error}"))
}

/// Benchmarks dense and sparse selected relation-weight lookups.
fn property_lookup(c: &mut Criterion) {
    let topology = BenchTopology { relations: 10_000 };
    let dense_descriptor = PropertyLayerDescriptor::try_new(
        LayerId(1_u64),
        "dense_weight",
        IdFamily::Relation,
        LayerRole::Weight,
        StorageMode::Dense,
        f32_field("dense_weight"),
    )
    .unwrap_or_else(|error| panic!("benchmark descriptor should be valid: {error}"));
    let dense_values = Arc::new(Float32Array::from_iter_values(
        (0..topology.relations).map(index_to_f32),
    ));
    let dense = PropertyLayer::try_new_dense(dense_descriptor, dense_values)
        .unwrap_or_else(|error| panic!("benchmark layer should be valid: {error}"));
    let dense_weights = DenseRelationWeights::<_, u64, u64, Float32Type>::new(&topology, &dense)
        .unwrap_or_else(|error| panic!("benchmark selection should be valid: {error}"));

    let sparse_descriptor = PropertyLayerDescriptor::try_new(
        LayerId(2_u64),
        "sparse_weight",
        IdFamily::Relation,
        LayerRole::Weight,
        StorageMode::Sparse {
            missing: MissingPolicy::Default,
        },
        f32_field("sparse_weight"),
    )
    .unwrap_or_else(|error| panic!("benchmark descriptor should be valid: {error}"));
    let sparse = PropertyLayer::try_new_sparse(
        sparse_descriptor,
        topology.relations,
        Arc::new(UInt64Array::from_iter_values(
            (0..topology.relations).step_by(10).map(index_to_u64),
        )),
        Arc::new(Float32Array::from_iter_values(
            (0..topology.relations).step_by(10).map(index_to_f32),
        )),
        Some(Arc::new(Float32Array::from(vec![1.0_f32]))),
    )
    .unwrap_or_else(|error| panic!("benchmark sparse layer should be valid: {error}"));
    let sparse_weights = SparseRelationWeights::<_, u64, u64, Float32Type>::new(&topology, &sparse)
        .unwrap_or_else(|error| panic!("benchmark sparse selection should be valid: {error}"));

    c.bench_function("dense_relation_weight_lookup_f32", |b| {
        b.iter(|| {
            let mut total = 0.0_f32;
            for relation in 0..topology.relations {
                total += dense_weights.relation_weight(index_to_u32(relation));
            }
            total
        });
    });
    c.bench_function("sparse_relation_weight_lookup_f32", |b| {
        b.iter(|| {
            let mut total = 0.0_f32;
            for relation in 0..topology.relations {
                total += sparse_weights.relation_weight(index_to_u32(relation));
            }
            total
        });
    });
}

criterion_group!(benches, property_lookup);
criterion_main!(benches);
