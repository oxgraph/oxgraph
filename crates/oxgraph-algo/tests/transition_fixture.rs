//! OxGraph-local transition fixtures inspired by downstream state-transition graphs.
//!
//! The fixture stays entirely inside this repository: it builds a small weighted
//! transition topology, exercises BFS/PageRank/identity/snapshots, and documents
//! the Rust behavior that a downstream Python project can later bind to.

#![cfg(feature = "alloc")]

use std::{error::Error, sync::Arc};

use arrow_array::Float64Array;
use arrow_schema::{DataType, Field};
use oxgraph_algo::{
    HyperWeighted, PageRankConfig, Weighted, breadth_first_search, pagerank_graph,
    pagerank_hypergraph,
};
use oxgraph_graph_build::{WeightedGraphBuilder, export_weighted_csr_snapshot_with_properties};
use oxgraph_hyper::HypergraphCounts;
use oxgraph_hyper_build::{
    WeightedHypergraphBuilder, export_weighted_bcsr_snapshot_with_properties,
};
use oxgraph_property::{
    GraphPropertyLayers, HyperPropertyLayers, IdFamily, LayerId, LayerRole, PropertyLayer,
    PropertyLayerDescriptor, StorageMode, validate_identity_snapshot, validate_property_snapshot,
};
use oxgraph_snapshot::Snapshot;
use oxgraph_topology::{LocalElementIdentity, RelationWeight, TopologyCounts};

/// Convergence configuration for local transition fixtures.
const CONFIG: PageRankConfig<f64> = PageRankConfig::new(0.85, 1.0e-10, 200);

#[test]
fn weighted_graph_transition_fixture_covers_algorithms_identity_and_snapshot()
-> Result<(), Box<dyn Error>> {
    let mut builder = WeightedGraphBuilder::<u32, u32, f64, f64>::new();
    let idle = builder.add_node(0.0)?;
    let queued = builder.add_node(0.0)?;
    let running = builder.add_node(0.0)?;
    let done = builder.add_node(0.0)?;
    builder.add_edge(idle, queued, 3.0)?;
    builder.add_edge(queued, running, 5.0)?;
    builder.add_edge(running, queued, 1.0)?;
    let finish = builder.add_edge(running, done, 4.0)?;
    let graph = builder.freeze()?;

    let relation_weight_layers = [PropertyLayer::try_new_dense(
        PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(1_u32),
            "edge_weight",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Dense,
            Field::new("edge_weight", DataType::Float64, false),
        )?,
        Arc::new(Float64Array::from(vec![3.0, 5.0, 1.0, 4.0])),
    )?];

    let bfs: Vec<_> = breadth_first_search(&graph, idle)?.collect();
    assert_eq!(bfs, vec![idle, queued, running, done]);
    assert_eq!(graph.local_element_id(idle), Some(idle));
    assert_eq!(graph.relation_count(), 4);
    assert!((graph.relation_weight(finish) - 4.0_f64).abs() < f64::EPSILON);

    let elements = vec![idle, queued, running, done];
    let mut ranks = vec![0.0; graph.element_count()];
    pagerank_graph(
        &graph,
        &Weighted::new(&graph),
        elements,
        CONFIG,
        None,
        &mut ranks,
    )?;
    assert_probability_mass(&ranks);

    let bytes = export_weighted_csr_snapshot_with_properties(
        &graph,
        GraphPropertyLayers {
            element: &[],
            relation: &relation_weight_layers,
        },
    )?;
    let snapshot = Snapshot::open(&bytes)?;
    assert_eq!(
        validate_identity_snapshot::<u32>(&snapshot)?.records.len(),
        2
    );
    assert_eq!(validate_property_snapshot::<u32>(&snapshot)?.layer_count, 1);
    Ok(())
}

#[test]
fn weighted_hyper_transition_fixture_covers_bipartite_ranking_and_snapshot()
-> Result<(), Box<dyn Error>> {
    let mut builder = WeightedHypergraphBuilder::<u32, u32, u32, f64, f64, f64>::new();
    let idle = builder.add_vertex(0.0)?;
    let queued = builder.add_vertex(0.0)?;
    let running = builder.add_vertex(0.0)?;
    let done = builder.add_vertex(0.0)?;
    let dispatch = builder.add_hyperedge(&[(idle, 1.0), (queued, 1.0)], &[(running, 1.0)], 2.0)?;
    let complete_or_retry =
        builder.add_hyperedge(&[(running, 1.0)], &[(queued, 1.0), (done, 3.0)], 4.0)?;
    let relation_weight_layers = [PropertyLayer::try_new_dense(
        PropertyLayerDescriptor::<u32, u32>::try_new(
            LayerId(2_u32),
            "hyperedge_weight",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Dense,
            Field::new("hyperedge_weight", DataType::Float64, false),
        )?,
        Arc::new(Float64Array::from(vec![2.0, 4.0])),
    )?];
    let graph = builder.freeze()?;

    let elements = vec![idle, queued, running, done];
    let relations = vec![dispatch, complete_or_retry];
    let mut element_ranks = vec![0.0; graph.vertex_count()];
    let mut relation_ranks = vec![0.0; graph.hyperedge_count()];
    pagerank_hypergraph(
        &graph,
        &HyperWeighted::new(&graph, &graph),
        elements,
        relations,
        CONFIG,
        None,
        &mut element_ranks,
        &mut relation_ranks,
    )?;
    assert_probability_mass(&element_ranks);
    assert_probability_mass(&relation_ranks);

    let bytes = export_weighted_bcsr_snapshot_with_properties(
        &graph,
        HyperPropertyLayers {
            element: &[],
            relation: &relation_weight_layers,
            incidence: &[],
        },
    )?;
    let snapshot = Snapshot::open(&bytes)?;
    assert_eq!(
        validate_identity_snapshot::<u32>(&snapshot)?.records.len(),
        3
    );
    assert_eq!(validate_property_snapshot::<u32>(&snapshot)?.layer_count, 1);
    Ok(())
}

/// Asserts finite non-negative rank mass.
fn assert_probability_mass(values: &[f64]) {
    let total: f64 = values.iter().sum();
    assert!(total.is_finite());
    assert!(values.iter().all(|value| *value >= 0.0));
}
