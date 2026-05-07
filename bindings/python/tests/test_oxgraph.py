"""Python smoke tests for the OxGraph facade.

Run with:
    cd bindings/python
    uv run maturin develop
    uv run pytest tests
"""

import oxgraph
import pytest


GRAPH_REMOVED_METHODS = (
    "node_for_label",
    "label_for_node",
    "node_ids",
    "edge_ids",
    "edge_endpoints",
    "outgoing_edges",
    "successor_nodes",
)

HYPERGRAPH_REMOVED_METHODS = (
    "vertex_for_label",
    "label_for_vertex",
    "vertex_ids",
    "hyperedge_ids",
    "outgoing_hyperedges",
    "successor_vertices",
)


def _assert_methods_absent(obj, names):
    for name in names:
        assert not hasattr(obj, name)


def test_import_and_graph_algorithms():
    builder = oxgraph.GraphBuilder()
    a = builder.add_node("a", weight=2.0)
    b = builder.add_node("b")
    edge = builder.add_edge(a, b, weight=3.0)
    assert builder.node("a") == a
    _assert_methods_absent(builder, ("node_for_label",))
    graph = builder.freeze()

    assert graph.node("a") == a
    assert graph.node_label(a) == "a"
    assert graph.canonical_node_id(a) == a
    assert graph.local_node_id(a) == a
    assert graph.element_weight(a) == 2.0
    assert graph.relation_weight(edge) == 3.0
    assert graph.bfs(a) == [a, b]

    ranks = graph.weighted_pagerank(max_iterations=100)
    assert len(ranks) == 2
    assert pytest.approx(sum(ranks)) == 1.0


def test_graph_topology_iteration_methods():
    builder = oxgraph.GraphBuilder()
    a = builder.add_node("a")
    b = builder.add_node("b")
    c = builder.add_node("c")
    ab = builder.add_edge(a, b, weight=2.0)
    ac = builder.add_edge(a, c, weight=3.0)
    cb = builder.add_edge(c, b, weight=4.0)
    graph = builder.freeze()

    assert graph.nodes() == [a, b, c]
    assert graph.edges() == [(ab, a, b), (ac, a, c), (cb, c, b)]
    assert graph.edge(ab) == (a, b)
    assert graph.edge(ac) == (a, c)
    assert graph.out_edges(a) == [(ab, b), (ac, c)]
    assert graph.out_edges(b) == []
    assert graph.successors(a) == [b, c]
    assert graph.successors(c) == [b]

    with pytest.raises(oxgraph.GraphError):
        graph.edge(99)
    with pytest.raises(oxgraph.GraphError):
        graph.out_edges(99)
    with pytest.raises(oxgraph.GraphError):
        graph.successors(99)
    _assert_methods_absent(graph, GRAPH_REMOVED_METHODS)


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
    assert builder.vertex("a") == a
    _assert_methods_absent(builder, ("vertex_for_label",))
    graph = builder.freeze()

    assert graph.vertex("a") == a
    assert graph.vertex_label(a) == "a"
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


def test_hypergraph_topology_iteration_methods():
    builder = oxgraph.HypergraphBuilder()
    a = builder.add_vertex("a")
    b = builder.add_vertex("b")
    c = builder.add_vertex("c")
    d = builder.add_vertex("d")
    first = builder.add_hyperedge([a, b], [c, d], weight=2.0)
    second = builder.add_hyperedge([c], [a], weight=3.0)
    graph = builder.freeze()

    assert graph.vertices() == [a, b, c, d]
    assert graph.hyperedges() == [
        (first, [a, b], [c, d]),
        (second, [c], [a]),
    ]
    assert graph.hyperedge(first) == ([a, b], [c, d])
    assert graph.hyperedge(second) == ([c], [a])
    assert graph.source_vertices(first) == [a, b]
    assert graph.target_vertices(first) == [c, d]
    assert graph.source_vertices(second) == [c]
    assert graph.target_vertices(second) == [a]
    assert graph.source_incidences(first) == [0, 1]
    assert graph.target_incidences(first) == [2, 3]
    assert graph.source_incidences(second) == [4]
    assert graph.target_incidences(second) == [5]
    assert graph.out_hyperedges(a) == [first]
    assert graph.out_hyperedges(c) == [second]
    assert graph.successors(a) == [c, d]
    assert graph.successors(c) == [a]

    with pytest.raises(oxgraph.HypergraphError):
        graph.hyperedge(99)
    with pytest.raises(oxgraph.HypergraphError):
        graph.source_vertices(99)
    with pytest.raises(oxgraph.HypergraphError):
        graph.source_incidences(99)
    with pytest.raises(oxgraph.HypergraphError):
        graph.out_hyperedges(99)
    with pytest.raises(oxgraph.HypergraphError):
        graph.successors(99)
    _assert_methods_absent(graph, HYPERGRAPH_REMOVED_METHODS)


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
