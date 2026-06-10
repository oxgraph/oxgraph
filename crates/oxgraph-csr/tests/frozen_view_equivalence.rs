//! Equivalence law between the three CSR shapes: for any built graph, the
//! frozen graph, its zero-copy [`FrozenGraph::as_view`] view, and the view
//! over its exported snapshot must agree on every shared observable — counts,
//! containment, out-degrees, and successor sequences — and the frozen graph's
//! canonical edge ids must resolve to the same targets the view yields
//! positionally.

use oxgraph_csr::{
    CsrNodeId, CsrSnapshotGraph,
    build::{GraphBuilder, GraphNodeId, export_csr_snapshot},
};
use oxgraph_graph::{
    ContainsElement, ContainsRelation, EdgeTargetGraph, ElementSuccessors, OutgoingEdgeCount,
    OutgoingGraph, TopologyCounts,
};
use oxgraph_snapshot::Snapshot;
use proptest::prelude::*;

/// Strategy producing `(node_count, edges)` with edges within bounds.
fn graph_strategy() -> impl Strategy<Value = (usize, Vec<(u32, u32)>)> {
    (1u32..32).prop_flat_map(|node_count| {
        let edge = (0u32..node_count, 0u32..node_count);
        let node_count = node_count as usize;
        (Just(node_count), proptest::collection::vec(edge, 0..64))
    })
}

proptest! {
    /// Frozen, as_view, and snapshot views agree on every shared observable.
    #[test]
    fn frozen_view_snapshot_agree((node_count, edges) in graph_strategy()) {
        let mut builder = GraphBuilder::<u32, u32>::new();
        let mut nodes = Vec::with_capacity(node_count);
        for _index in 0..node_count {
            nodes.push(builder.add_node().expect("node allocates"));
        }
        for (source, target) in &edges {
            builder
                .add_edge(nodes[*source as usize], nodes[*target as usize])
                .expect("edge endpoints are visible");
        }
        let frozen = builder.freeze().expect("builder freezes");
        let view = frozen.as_view().expect("count fits the width");
        let bytes = export_csr_snapshot(&frozen).expect("snapshot exports");
        let snapshot = Snapshot::open(&bytes).expect("snapshot opens");
        let opened =
            CsrSnapshotGraph::<u32, u32>::from_snapshot(&snapshot).expect("sections bind");

        // Counts.
        prop_assert_eq!(frozen.element_count(), node_count);
        prop_assert_eq!(view.element_count(), node_count);
        prop_assert_eq!(opened.element_count(), node_count);
        prop_assert_eq!(frozen.relation_count(), edges.len());
        prop_assert_eq!(view.relation_count(), edges.len());
        prop_assert_eq!(opened.relation_count(), edges.len());

        // Containment bounds.
        let beyond = u32::try_from(node_count).expect("small count");
        prop_assert!(!frozen.contains_element(GraphNodeId::new(beyond)));
        prop_assert!(!view.contains_element(CsrNodeId::new(beyond)));
        prop_assert!(!opened.contains_element(CsrNodeId::new(beyond)));

        for index in 0..node_count {
            let id = u32::try_from(index).expect("small index");
            let frozen_node = GraphNodeId::new(id);
            let view_node = CsrNodeId::new(id);

            prop_assert!(frozen.contains_element(frozen_node));
            prop_assert!(view.contains_element(view_node));
            prop_assert!(opened.contains_element(view_node));

            // Out-degrees.
            let degree = frozen.out_degree(frozen_node);
            prop_assert_eq!(view.out_degree(view_node), degree);
            prop_assert_eq!(opened.out_degree(view_node), degree);

            // Successor sequences (CSR traversal order).
            let frozen_succ: Vec<u32> = frozen
                .element_successors(frozen_node)
                .map(oxgraph_layout_util::LocalId::get)
                .collect();
            let view_succ: Vec<u32> = view
                .element_successors(view_node)
                .map(oxgraph_layout_util::LocalId::get)
                .collect();
            let opened_succ: Vec<u32> = opened
                .element_successors(view_node)
                .map(oxgraph_layout_util::LocalId::get)
                .collect();
            prop_assert_eq!(&frozen_succ, &view_succ);
            prop_assert_eq!(&frozen_succ, &opened_succ);

            // Canonical edge ids resolve to the same targets the view yields
            // positionally: outgoing edges are canonical ids whose
            // EdgeTargetGraph targets must equal the successor sequence.
            let canonical_targets: Vec<u32> = frozen
                .outgoing_edges(frozen_node)
                .map(|edge| frozen.target(edge).get())
                .collect();
            prop_assert_eq!(&canonical_targets, &frozen_succ);

            for edge in frozen.outgoing_edges(frozen_node) {
                prop_assert!(frozen.contains_relation(edge));
            }
        }
    }
}
