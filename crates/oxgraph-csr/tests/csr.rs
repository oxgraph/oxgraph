//! Tests for borrowed CSR graph validation and traversal.

use oxgraph_csr::{CsrEdgeId, CsrError, CsrGraph, CsrNodeId};
use oxgraph_graph::{
    EdgeTargetGraph, ElementIndex, GraphCounts, OutgoingEdgeCount, OutgoingGraph,
    OutgoingNeighborsGraph, RelationIndex,
};
use proptest::prelude::*;
use zerocopy::byteorder::{LE, U32};

/// Returns a valid graph shaped like `0 -> {1, 2}`, `1 -> {2}`, `2 -> {3}`.
fn fixture() -> Result<CsrGraph<'static>, CsrError> {
    static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
    static TARGETS: &[u32] = &[1, 2, 2, 3];

    CsrGraph::validate(4, OFFSETS, TARGETS)
}

#[test]
fn valid_csr_traverses_outgoing_edges() -> Result<(), CsrError> {
    let graph = fixture()?;

    assert_eq!(graph.node_count(), 4);
    assert_eq!(graph.edge_count(), 4);
    assert_eq!(
        graph.outgoing_edges(CsrNodeId(0)).collect::<Vec<_>>(),
        [CsrEdgeId(0), CsrEdgeId(1)]
    );
    assert_eq!(graph.target(CsrEdgeId(0)), CsrNodeId(1));
    assert_eq!(graph.target(CsrEdgeId(1)), CsrNodeId(2));

    Ok(())
}

#[test]
fn valid_csr_traverses_outgoing_neighbors_directly() -> Result<(), CsrError> {
    let graph = fixture()?;

    assert_eq!(
        graph.outgoing_neighbors(CsrNodeId(0)).collect::<Vec<_>>(),
        [CsrNodeId(1), CsrNodeId(2)]
    );
    assert_eq!(
        graph.outgoing_neighbors(CsrNodeId(3)).collect::<Vec<_>>(),
        []
    );

    Ok(())
}

#[test]
fn csr_exposes_dense_element_and_relation_indexes() -> Result<(), CsrError> {
    let graph = fixture()?;

    assert_eq!(graph.element_bound(), graph.node_count());
    assert_eq!(graph.relation_bound(), graph.edge_count());
    assert_eq!(graph.element_index(CsrNodeId(2)), 2);
    assert_eq!(graph.relation_index(CsrEdgeId(3)), 3);

    Ok(())
}

#[test]
fn csr_reports_node_and_edge_containment() -> Result<(), CsrError> {
    let graph = fixture()?;

    assert!(graph.contains_node(CsrNodeId(3)));
    assert!(!graph.contains_node(CsrNodeId(4)));
    assert!(graph.contains_edge(CsrEdgeId(3)));
    assert!(!graph.contains_edge(CsrEdgeId(4)));
    assert_eq!(graph.try_target(CsrEdgeId(1)), Some(CsrNodeId(2)));
    assert_eq!(graph.try_target(CsrEdgeId(4)), None);

    Ok(())
}

#[test]
fn empty_csr_graph_is_valid() -> Result<(), CsrError> {
    static OFFSETS: &[u32] = &[0];
    static TARGETS: &[u32] = &[];

    let graph = CsrGraph::validate(0, OFFSETS, TARGETS)?;

    assert_eq!(graph.node_count(), 0);
    assert_eq!(graph.edge_count(), 0);
    assert!(!graph.contains_node(CsrNodeId(0)));
    assert!(!graph.contains_edge(CsrEdgeId(0)));

    Ok(())
}

#[test]
fn csr_supports_isolated_nodes_self_loops_and_parallel_edges() -> Result<(), CsrError> {
    static OFFSETS: &[u32] = &[0, 3, 3, 4];
    static TARGETS: &[u32] = &[0, 1, 1, 2];

    let graph = CsrGraph::validate(3, OFFSETS, TARGETS)?;

    assert_eq!(graph.out_degree(CsrNodeId(1)), 0);
    assert_eq!(graph.target(CsrEdgeId(0)), CsrNodeId(0));
    assert_eq!(graph.target(CsrEdgeId(1)), CsrNodeId(1));
    assert_eq!(graph.target(CsrEdgeId(2)), CsrNodeId(1));
    assert_eq!(
        graph.outgoing_neighbors(CsrNodeId(0)).collect::<Vec<_>>(),
        [CsrNodeId(0), CsrNodeId(1), CsrNodeId(1)]
    );

    Ok(())
}

#[test]
fn outgoing_iterator_reports_exact_remaining_length() -> Result<(), CsrError> {
    let graph = fixture()?;
    let mut edges = graph.outgoing_edges(CsrNodeId(0));

    assert_eq!(edges.len(), 2);
    assert_eq!(edges.next(), Some(CsrEdgeId(0)));
    assert_eq!(edges.len(), 1);
    assert_eq!(edges.next(), Some(CsrEdgeId(1)));
    assert_eq!(edges.len(), 0);
    assert_eq!(edges.next(), None);

    Ok(())
}

#[test]
fn outgoing_neighbor_iterator_reports_exact_remaining_length() -> Result<(), CsrError> {
    let graph = fixture()?;
    let mut neighbors = graph.outgoing_neighbors(CsrNodeId(0));

    assert_eq!(neighbors.len(), 2);
    assert_eq!(neighbors.next(), Some(CsrNodeId(1)));
    assert_eq!(neighbors.len(), 1);
    assert_eq!(neighbors.next(), Some(CsrNodeId(2)));
    assert_eq!(neighbors.len(), 0);
    assert_eq!(neighbors.next(), None);

    Ok(())
}

#[test]
fn validates_zero_copy_little_endian_words() -> Result<(), CsrError> {
    static OFFSETS: &[U32<LE>] = &[U32::new(0), U32::new(1), U32::new(1)];
    static TARGETS: &[U32<LE>] = &[U32::new(1)];

    let graph = CsrGraph::validate(2, OFFSETS, TARGETS)?;

    assert_eq!(graph.target(CsrEdgeId(0)), CsrNodeId(1));
    assert_eq!(
        graph.outgoing_neighbors(CsrNodeId(0)).collect::<Vec<_>>(),
        [CsrNodeId(1)]
    );

    Ok(())
}

#[test]
fn rejects_wrong_offset_length() {
    static OFFSETS: &[u32] = &[0, 1];
    static TARGETS: &[u32] = &[0];

    assert_eq!(
        CsrGraph::validate(2, OFFSETS, TARGETS).err(),
        Some(CsrError::OffsetLength {
            expected: 3,
            actual: 2,
        })
    );
}

#[test]
fn rejects_nonzero_first_offset() {
    static OFFSETS: &[u32] = &[1, 1];
    static TARGETS: &[u32] = &[];

    assert_eq!(
        CsrGraph::validate(1, OFFSETS, TARGETS).err(),
        Some(CsrError::FirstOffset { actual: 1 })
    );
}

#[test]
fn rejects_nonmonotonic_offsets() {
    static OFFSETS: &[u32] = &[0, 2, 1];
    static TARGETS: &[u32] = &[0];

    assert_eq!(
        CsrGraph::validate(2, OFFSETS, TARGETS).err(),
        Some(CsrError::NonMonotonicOffset {
            index: 2,
            previous: 2,
            actual: 1,
        })
    );
}

#[test]
fn rejects_bad_final_offset() {
    static OFFSETS: &[u32] = &[0, 2];
    static TARGETS: &[u32] = &[0];

    assert_eq!(
        CsrGraph::validate(1, OFFSETS, TARGETS).err(),
        Some(CsrError::FinalOffset {
            final_offset: 2,
            target_len: 1,
        })
    );
}

#[test]
fn rejects_final_offset_shorter_than_targets() {
    static OFFSETS: &[u32] = &[0, 0];
    static TARGETS: &[u32] = &[0];

    assert_eq!(
        CsrGraph::validate(1, OFFSETS, TARGETS).err(),
        Some(CsrError::FinalOffset {
            final_offset: 0,
            target_len: 1,
        })
    );
}

#[test]
fn rejects_out_of_range_target() {
    static OFFSETS: &[u32] = &[0, 1];
    static TARGETS: &[u32] = &[1];

    assert_eq!(
        CsrGraph::validate(1, OFFSETS, TARGETS).err(),
        Some(CsrError::TargetOutOfRange {
            index: 0,
            target: 1,
            node_count: 1,
        })
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Out-degree matches outgoing traversal length for generated valid CSR graphs.
    #[test]
    fn out_degree_matches_traversal(
        degrees in proptest::collection::vec(0u32..4, 1..8),
        target_seed in proptest::collection::vec(0u32..32, 0..32),
    ) {
        let node_count = match u32::try_from(degrees.len()) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("node count conversion failed: {error:?}"))),
        };

        let mut offsets = Vec::with_capacity(degrees.len() + 1);
        offsets.push(0);
        let mut total = 0u32;
        for degree in &degrees {
            total += *degree;
            offsets.push(total);
        }

        let edge_count = match usize::try_from(total) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("edge count conversion failed: {error:?}"))),
        };
        let mut targets = Vec::with_capacity(edge_count);
        for index in 0..edge_count {
            let seed = target_seed.get(index).copied().unwrap_or(0);
            targets.push(seed % node_count);
        }

        let graph = match CsrGraph::validate(node_count, &offsets, &targets) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!("valid CSR rejected: {error:?}"))),
        };

        for node in 0..node_count {
            let id = CsrNodeId(node);
            prop_assert_eq!(graph.out_degree(id), graph.outgoing_edges(id).count());
            prop_assert_eq!(
                graph.outgoing_neighbors(id).collect::<Vec<_>>(),
                graph
                    .outgoing_edges(id)
                    .map(|edge| graph.target(edge))
                    .collect::<Vec<_>>()
            );
            for edge in graph.outgoing_edges(id) {
                prop_assert!(graph.contains_edge(edge));
                prop_assert!(graph.contains_node(graph.target(edge)));
            }
        }
    }
}
