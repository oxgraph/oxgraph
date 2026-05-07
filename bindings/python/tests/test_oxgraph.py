"""Python smoke tests for the OxGraph facade.

Run with:
    cd bindings/python
    uv run maturin develop
    uv run pytest tests
"""

import pytest

import oxgraph


def test_import_and_graph_algorithms():
    builder = oxgraph.GraphBuilder()
    a = builder.add_node("a", weight=2.0)
    b = builder.add_node("b")
    edge = builder.add_edge(a, b, weight=3.0)
    graph = builder.freeze()

    assert graph.node_for_label("a") == a
    assert graph.label_for_node(a) == "a"
    assert graph.canonical_node_id(a) == a
    assert graph.local_node_id(a) == a
    assert graph.element_weight(a) == 2.0
    assert graph.relation_weight(edge) == 3.0
    assert graph.bfs(a) == [a, b]

    ranks = graph.weighted_pagerank(max_iterations=100)
    assert len(ranks) == 2
    assert pytest.approx(sum(ranks)) == 1.0


def test_graph_snapshot_helpers_and_errors():
    builder = oxgraph.GraphBuilder()
    a = builder.add_node("a")
    b = builder.add_node("b")
    builder.add_edge(a, b)
    graph = builder.freeze()

    info = oxgraph.open_csr_snapshot(graph.to_csr_snapshot())
    assert info.section_count() >= 2
    assert len(info.section_kinds()) == info.section_count()

    with pytest.raises(oxgraph.GraphError):
        graph.relation_weight(99)
    with pytest.raises(oxgraph.SnapshotError):
        oxgraph.open_snapshot(b"not a snapshot")


def test_hypergraph_algorithms_and_snapshot_helpers():
    builder = oxgraph.HypergraphBuilder()
    a = builder.add_vertex("a")
    b = builder.add_vertex("b")
    hyperedge = builder.add_hyperedge([a], [b], weight=4.0, target_weights=[2.0])
    graph = builder.freeze()

    assert graph.vertex_for_label("a") == a
    assert graph.canonical_hyperedge_id(hyperedge) == hyperedge
    assert graph.relation_weight(hyperedge) == 4.0
    assert graph.incidence_count() == 2
    assert graph.bfs(a) == [a, b]

    element_ranks, relation_ranks = graph.weighted_pagerank(max_iterations=100)
    assert len(element_ranks) == 2
    assert len(relation_ranks) == 1
    assert pytest.approx(sum(element_ranks) + sum(relation_ranks)) == 1.0

    info = oxgraph.open_bcsr_snapshot(graph.to_bcsr_snapshot())
    assert info.section_count() >= 8


def test_duplicate_labels_and_weight_length_errors_do_not_mutate_builders():
    graph_builder = oxgraph.GraphBuilder()
    graph_builder.add_node("dup")
    before = graph_builder.node_count()
    with pytest.raises(oxgraph.GraphError):
        graph_builder.add_node("dup")
    assert graph_builder.node_count() == before

    hyper_builder = oxgraph.HypergraphBuilder()
    a = hyper_builder.add_vertex("a")
    b = hyper_builder.add_vertex("b")
    before = hyper_builder.hyperedge_count()
    with pytest.raises(oxgraph.HypergraphError):
        hyper_builder.add_hyperedge([a], [b], source_weights=[1.0, 2.0])
    assert hyper_builder.hyperedge_count() == before
