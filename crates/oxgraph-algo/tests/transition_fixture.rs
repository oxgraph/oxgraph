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
    PageRankConfig, breadth_first_search, hypergraph_pagerank_weighted, pagerank_weighted,
};
use oxgraph_graph::GraphCounts;
use oxgraph_graph_build::GraphBuilder;
use oxgraph_hyper::HypergraphCounts;
use oxgraph_hyper_build::HypergraphBuilder;
use oxgraph_property::{
    IdFamily, LayerId, LayerRole, PropertyLayer, PropertyLayerDescriptor, StorageMode,
    validate_identity_snapshot, validate_property_snapshot,
};
use oxgraph_snapshot::Snapshot;
use oxgraph_topology::{LocalElementIdentity, RelationWeight};

/// Convergence configuration for local transition fixtures.
const CONFIG: PageRankConfig = PageRankConfig::new(0.85, 1.0e-10, 200);

#[test]
fn weighted_graph_transition_fixture_covers_algorithms_identity_and_snapshot()
-> Result<(), Box<dyn Error>> {
    let mut builder = GraphBuilder::new(0.0_f64, 1.0_f64);
    let idle = builder.add_node()?;
    let queued = builder.add_node()?;
    let running = builder.add_node()?;
    let done = builder.add_node()?;
    let to_queued = builder.add_edge(idle, queued)?;
    let to_running = builder.add_edge(queued, running)?;
    let retry = builder.add_edge(running, queued)?;
    let finish = builder.add_edge(running, done)?;
    builder.set_relation_weight(to_queued, 3.0)?;
    builder.set_relation_weight(to_running, 5.0)?;
    builder.set_relation_weight(retry, 1.0)?;
    builder.set_relation_weight(finish, 4.0)?;
    builder.add_property_layer(PropertyLayer::try_new_dense(
        PropertyLayerDescriptor::try_new(
            LayerId(1),
            "edge_weight",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Dense,
            Field::new("edge_weight", DataType::Float64, false),
        )?,
        Arc::new(Float64Array::from(vec![3.0, 5.0, 1.0, 4.0])),
    )?)?;
    let graph = builder.freeze()?;

    let bfs: Vec<_> = breadth_first_search(&graph, idle)?.collect();
    assert_eq!(bfs, vec![idle, queued, running, done]);
    assert_eq!(graph.local_element_id(idle), Some(idle));
    assert_eq!(graph.edge_count(), 4);
    assert!((graph.relation_weight(finish) - 4.0_f64).abs() < f64::EPSILON);

    let elements = vec![idle, queued, running, done];
    let mut ranks = vec![0.0; graph.node_count()];
    pagerank_weighted(&graph, &graph, elements, CONFIG, None, &mut ranks)?;
    assert_probability_mass(&ranks);

    let bytes = graph.to_csr_snapshot()?;
    let snapshot = Snapshot::open(&bytes)?;
    assert_eq!(validate_identity_snapshot(&snapshot)?.records.len(), 2);
    assert_eq!(validate_property_snapshot(&snapshot)?.layer_count, 1);
    Ok(())
}

#[test]
fn weighted_hyper_transition_fixture_covers_bipartite_ranking_and_snapshot()
-> Result<(), Box<dyn Error>> {
    let mut builder = HypergraphBuilder::new(0.0_f64, 1.0_f64, 1.0_f64);
    let idle = builder.add_vertex()?;
    let queued = builder.add_vertex()?;
    let running = builder.add_vertex()?;
    let done = builder.add_vertex()?;
    let dispatch = builder.add_hyperedge(&[idle, queued], &[running])?;
    let complete_or_retry = builder.add_hyperedge(&[running], &[queued, done])?;
    builder.set_relation_weight(dispatch, 2.0)?;
    builder.set_relation_weight(complete_or_retry, 4.0)?;
    builder.set_target_incidence_weight(complete_or_retry, 0, 1.0)?;
    builder.set_target_incidence_weight(complete_or_retry, 1, 3.0)?;
    builder.add_property_layer(PropertyLayer::try_new_dense(
        PropertyLayerDescriptor::try_new(
            LayerId(2),
            "hyperedge_weight",
            IdFamily::Relation,
            LayerRole::Weight,
            StorageMode::Dense,
            Field::new("hyperedge_weight", DataType::Float64, false),
        )?,
        Arc::new(Float64Array::from(vec![2.0, 4.0])),
    )?);
    let graph = builder.freeze()?;

    let elements = vec![idle, queued, running, done];
    let relations = vec![dispatch, complete_or_retry];
    let mut element_ranks = vec![0.0; graph.vertex_count()];
    let mut relation_ranks = vec![0.0; graph.hyperedge_count()];
    hypergraph_pagerank_weighted(
        &graph,
        &graph,
        &graph,
        elements,
        relations,
        CONFIG,
        None,
        &mut element_ranks,
        &mut relation_ranks,
    )?;
    assert_probability_mass(&element_ranks);
    assert_probability_mass(&relation_ranks);

    let bytes = graph.to_bcsr_snapshot()?;
    let snapshot = Snapshot::open(&bytes)?;
    assert_eq!(validate_identity_snapshot(&snapshot)?.records.len(), 3);
    assert_eq!(validate_property_snapshot(&snapshot)?.layer_count, 1);
    Ok(())
}

/// Asserts finite non-negative rank mass.
fn assert_probability_mass(values: &[f64]) {
    let total: f64 = values.iter().sum();
    assert!(total.is_finite());
    assert!(values.iter().all(|value| *value >= 0.0));
}
