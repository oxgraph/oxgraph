"""Python smoke tests for the OxGraph facade.

Run with:
    cd crates/oxgraph-python
    python -m pip install -e .
    python -m pytest tests
"""

import pytest

import oxgraph


def test_import_and_graph_algorithms():
    builder = oxgraph.GraphBuilder()
    a = builder.add_node("a")
    b = builder.add_node("b")
    edge = builder.add_edge(a, b, 2.0)
    graph = builder.freeze()

    assert graph.node_for_label("a") == a
    assert graph.label_for_node(a) == "a"
    assert graph.canonical_node_id(a) == a
    assert graph.local_node_id(a) == a
    assert graph.relation_weight(edge) == 2.0
    assert graph.bfs(a) == [a, b]
    ranks = graph.weighted_pagerank(max_iterations=100)
    assert len(ranks) == 2
    assert pytest.approx(sum(ranks)) == 1.0


def test_property_layer_selected_pagerank_and_errors():
    builder = oxgraph.GraphBuilder()
    a = builder.add_node("a")
    b = builder.add_node("b")
    builder.add_edge(a, b)
    graph = builder.freeze()
    layer = oxgraph.DenseF64Layer(1, "weight", "relation", "weight", [3.0])

    ranks = graph.pagerank_with_dense_relation_weights(layer, max_iterations=100)
    assert len(ranks) == 2
    with pytest.raises(oxgraph.GraphError):
        graph.relation_weight(99)


def test_hypergraph_snapshot_helpers():
    builder = oxgraph.HypergraphBuilder()
    a = builder.add_vertex("a")
    b = builder.add_vertex("b")
    hyperedge = builder.add_hyperedge([a], [b], 4.0)
    graph = builder.freeze()

    assert graph.vertex_for_label("a") == a
    assert graph.canonical_hyperedge_id(hyperedge) == hyperedge
    element_ranks, relation_ranks = graph.weighted_pagerank(max_iterations=100)
    assert len(element_ranks) == 2
    assert len(relation_ranks) == 1
    snapshot = graph.to_bcsr_snapshot()
    info = oxgraph.open_bcsr_snapshot(snapshot)
    assert info.identity_family_count() == 3
    assert info.property_layer_count() is None


def test_duplicate_labels_do_not_mutate_builders():
    graph_builder = oxgraph.GraphBuilder()
    graph_builder.add_node("dup")
    before = graph_builder.node_count()
    with pytest.raises(oxgraph.GraphError):
        graph_builder.add_node("dup")
    assert graph_builder.node_count() == before

    hyper_builder = oxgraph.HypergraphBuilder()
    hyper_builder.add_vertex("dup")
    before = hyper_builder.vertex_count()
    with pytest.raises(oxgraph.HypergraphError):
        hyper_builder.add_vertex("dup")
    assert hyper_builder.vertex_count() == before
