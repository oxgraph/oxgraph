//! Tests for borrowed CSR graph validation and traversal.

use oxgraph_csr::{CsrEdgeId, CsrError, CsrGraph, CsrNativeGraph, CsrNodeId};
use oxgraph_graph::{
    DenseElementIndex, DenseRelationIndex, EdgeTargetGraph, GraphCounts, OutgoingEdgeCount,
    OutgoingGraph, OutgoingNeighborsGraph,
};
use proptest::prelude::*;
use zerocopy::byteorder::{LE, U16, U32, U64};

/// Returns a valid graph shaped like `0 -> {1, 2}`, `1 -> {2}`, `2 -> {3}`.
fn fixture() -> Result<CsrNativeGraph<'static, u32, u32>, CsrError<u32, u32>> {
    static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
    static TARGETS: &[u32] = &[1, 2, 2, 3];

    CsrGraph::validate(4, OFFSETS, TARGETS)
}

#[test]
fn valid_csr_traverses_outgoing_edges() -> Result<(), CsrError<u32, u32>> {
    let graph = fixture()?;

    assert_eq!(graph.node_count(), 4);
    assert_eq!(graph.edge_count(), 4);
    assert_eq!(
        graph.outgoing_edges(CsrNodeId::new(0)).collect::<Vec<_>>(),
        [CsrEdgeId::new(0), CsrEdgeId::new(1)]
    );
    assert_eq!(graph.target(CsrEdgeId::new(0)), CsrNodeId::new(1));
    assert_eq!(graph.target(CsrEdgeId::new(1)), CsrNodeId::new(2));

    Ok(())
}

#[test]
fn for_each_out_target_matches_outgoing_neighbors() -> Result<(), CsrError<u32, u32>> {
    let graph = fixture()?;

    let mut collected = Vec::new();
    graph.for_each_out_target(CsrNodeId::new(0), |target| {
        collected.push(target);
        false
    });
    assert_eq!(
        collected,
        graph
            .outgoing_neighbors(CsrNodeId::new(0))
            .collect::<Vec<_>>()
    );

    let stopped = graph.for_each_out_target(CsrNodeId::new(0), |target| {
        assert_eq!(target, CsrNodeId::new(1));
        true
    });
    assert!(stopped);

    Ok(())
}

#[test]
fn valid_csr_traverses_outgoing_neighbors_directly() -> Result<(), CsrError<u32, u32>> {
    let graph = fixture()?;

    assert_eq!(
        graph
            .outgoing_neighbors(CsrNodeId::new(0))
            .collect::<Vec<_>>(),
        [CsrNodeId::new(1), CsrNodeId::new(2)]
    );
    assert_eq!(
        graph
            .outgoing_neighbors(CsrNodeId::new(3))
            .collect::<Vec<_>>(),
        []
    );

    Ok(())
}

#[test]
fn csr_exposes_dense_element_and_relation_indexes() -> Result<(), CsrError<u32, u32>> {
    let graph = fixture()?;

    assert_eq!(graph.element_bound(), graph.node_count());
    assert_eq!(graph.relation_bound(), graph.edge_count());
    assert_eq!(graph.element_index(CsrNodeId::new(2)), 2);
    assert_eq!(graph.relation_index(CsrEdgeId::new(3)), 3);

    Ok(())
}

#[test]
fn csr_reports_node_and_edge_containment() -> Result<(), CsrError<u32, u32>> {
    let graph = fixture()?;

    assert!(graph.contains_node(CsrNodeId::new(3)));
    assert!(!graph.contains_node(CsrNodeId::new(4)));
    assert!(graph.contains_edge(CsrEdgeId::new(3)));
    assert!(!graph.contains_edge(CsrEdgeId::new(4)));
    assert_eq!(graph.try_target(CsrEdgeId::new(1)), Some(CsrNodeId::new(2)));
    assert_eq!(graph.try_target(CsrEdgeId::new(4)), None);

    Ok(())
}

#[test]
fn empty_csr_graph_is_valid() -> Result<(), CsrError<u32, u32>> {
    static OFFSETS: &[u32] = &[0];
    static TARGETS: &[u32] = &[];

    let graph = CsrGraph::validate(0, OFFSETS, TARGETS)?;

    assert_eq!(graph.node_count(), 0);
    assert_eq!(graph.edge_count(), 0);
    assert!(!graph.contains_node(CsrNodeId::new(0)));
    assert!(!graph.contains_edge(CsrEdgeId::new(0)));

    Ok(())
}

#[test]
fn csr_supports_isolated_nodes_self_loops_and_parallel_edges() -> Result<(), CsrError<u32, u32>> {
    static OFFSETS: &[u32] = &[0, 3, 3, 4];
    static TARGETS: &[u32] = &[0, 1, 1, 2];

    let graph = CsrGraph::validate(3, OFFSETS, TARGETS)?;

    assert_eq!(graph.out_degree(CsrNodeId::new(1)), 0);
    assert_eq!(graph.target(CsrEdgeId::new(0)), CsrNodeId::new(0));
    assert_eq!(graph.target(CsrEdgeId::new(1)), CsrNodeId::new(1));
    assert_eq!(graph.target(CsrEdgeId::new(2)), CsrNodeId::new(1));
    assert_eq!(
        graph
            .outgoing_neighbors(CsrNodeId::new(0))
            .collect::<Vec<_>>(),
        [CsrNodeId::new(0), CsrNodeId::new(1), CsrNodeId::new(1)]
    );

    Ok(())
}

#[test]
fn outgoing_iterator_reports_exact_remaining_length() -> Result<(), CsrError<u32, u32>> {
    let graph = fixture()?;
    let mut edges = graph.outgoing_edges(CsrNodeId::new(0));

    assert_eq!(edges.len(), 2);
    assert_eq!(edges.next(), Some(CsrEdgeId::new(0)));
    assert_eq!(edges.len(), 1);
    assert_eq!(edges.next(), Some(CsrEdgeId::new(1)));
    assert_eq!(edges.len(), 0);
    assert_eq!(edges.next(), None);

    Ok(())
}

#[test]
fn outgoing_neighbor_iterator_reports_exact_remaining_length() -> Result<(), CsrError<u32, u32>> {
    let graph = fixture()?;
    let mut neighbors = graph.outgoing_neighbors(CsrNodeId::new(0));

    assert_eq!(neighbors.len(), 2);
    assert_eq!(neighbors.next(), Some(CsrNodeId::new(1)));
    assert_eq!(neighbors.len(), 1);
    assert_eq!(neighbors.next(), Some(CsrNodeId::new(2)));
    assert_eq!(neighbors.len(), 0);
    assert_eq!(neighbors.next(), None);

    Ok(())
}

#[test]
fn validates_u16_index_graph() -> Result<(), CsrError<u16, u16>> {
    static OFFSETS: &[u16] = &[0, 2, 2];
    static TARGETS: &[u16] = &[1, 0];

    let graph = CsrGraph::validate(2u16, OFFSETS, TARGETS)?;

    assert_eq!(graph.node_count(), 2);
    assert_eq!(
        graph
            .outgoing_edges(CsrNodeId::new(0u16))
            .collect::<Vec<_>>(),
        [CsrEdgeId::new(0u16), CsrEdgeId::new(1u16)]
    );
    assert_eq!(graph.target(CsrEdgeId::new(0u16)), CsrNodeId::new(1u16));

    Ok(())
}

#[test]
fn validates_u64_index_graph() -> Result<(), CsrError<u64, u64>> {
    static OFFSETS: &[u64] = &[0, 1, 1];
    static TARGETS: &[u64] = &[1];

    let graph = CsrGraph::validate(2u64, OFFSETS, TARGETS)?;

    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.out_degree(CsrNodeId::new(0u64)), 1);
    assert_eq!(graph.target(CsrEdgeId::new(0u64)), CsrNodeId::new(1u64));

    Ok(())
}

#[test]
fn validates_mixed_u32_nodes_u64_edges() -> Result<(), CsrError<u32, u64>> {
    static OFFSETS: &[u64] = &[0, 2, 3, 3];
    static TARGETS: &[u32] = &[1, 2, 0];

    let graph = CsrGraph::validate(3u32, OFFSETS, TARGETS)?;

    assert_eq!(graph.node_count(), 3);
    assert_eq!(graph.edge_count(), 3);
    assert_eq!(
        graph
            .outgoing_edges(CsrNodeId::new(0u32))
            .collect::<Vec<_>>(),
        [CsrEdgeId::new(0u64), CsrEdgeId::new(1u64)]
    );
    assert_eq!(graph.target(CsrEdgeId::new(2u64)), CsrNodeId::new(0u32));

    Ok(())
}

#[test]
fn validates_usize_index_graph() -> Result<(), CsrError<usize, usize>> {
    static OFFSETS: &[usize] = &[0, 1, 1];
    static TARGETS: &[usize] = &[1];

    let graph = CsrGraph::validate(2usize, OFFSETS, TARGETS)?;

    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.out_degree(CsrNodeId::new(0usize)), 1);
    assert_eq!(graph.target(CsrEdgeId::new(0usize)), CsrNodeId::new(1usize));

    Ok(())
}

#[test]
fn validates_zero_copy_little_endian_u16_words() -> Result<(), CsrError<u16, u16>> {
    static OFFSETS: &[U16<LE>] = &[U16::new(0), U16::new(1), U16::new(1)];
    static TARGETS: &[U16<LE>] = &[U16::new(1)];

    let graph = CsrGraph::validate(2u16, OFFSETS, TARGETS)?;

    assert_eq!(graph.target(CsrEdgeId::new(0u16)), CsrNodeId::new(1u16));
    assert_eq!(
        graph
            .outgoing_neighbors(CsrNodeId::new(0u16))
            .collect::<Vec<_>>(),
        [CsrNodeId::new(1u16)]
    );

    Ok(())
}

#[test]
fn validates_zero_copy_little_endian_u32_words() -> Result<(), CsrError<u32, u32>> {
    static OFFSETS: &[U32<LE>] = &[U32::new(0), U32::new(1), U32::new(1)];
    static TARGETS: &[U32<LE>] = &[U32::new(1)];

    let graph = CsrGraph::validate(2, OFFSETS, TARGETS)?;

    assert_eq!(graph.target(CsrEdgeId::new(0)), CsrNodeId::new(1));
    assert_eq!(
        graph
            .outgoing_neighbors(CsrNodeId::new(0))
            .collect::<Vec<_>>(),
        [CsrNodeId::new(1)]
    );

    Ok(())
}

#[test]
fn validates_zero_copy_little_endian_u64_words() -> Result<(), CsrError<u64, u64>> {
    static OFFSETS: &[U64<LE>] = &[U64::new(0), U64::new(1), U64::new(1)];
    static TARGETS: &[U64<LE>] = &[U64::new(1)];

    let graph = CsrGraph::validate(2u64, OFFSETS, TARGETS)?;

    assert_eq!(graph.target(CsrEdgeId::new(0u64)), CsrNodeId::new(1u64));
    assert_eq!(
        graph
            .outgoing_neighbors(CsrNodeId::new(0u64))
            .collect::<Vec<_>>(),
        [CsrNodeId::new(1u64)]
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
            let id = CsrNodeId::new(node);
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
