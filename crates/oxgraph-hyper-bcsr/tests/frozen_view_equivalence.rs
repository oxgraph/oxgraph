//! Equivalence law between the three BCSR shapes: for any built hypergraph,
//! the frozen wrapper, its zero-copy `as_view` view, and the view over its
//! exported snapshot must agree on every shared observable — vertex /
//! hyperedge / participant counts, per-vertex outgoing/incoming hyperedge
//! sequences, per-hyperedge source/target participant sequences, and
//! per-participant element/relation/role resolution.

use std::collections::BTreeSet;

use oxgraph_hyper::{
    DirectedHyperedgeParticipants, DirectedVertexHyperedges, IncidenceCounts, IncidenceElement,
    IncidenceRelation, IncidenceRole, RelationIncidences, TopologyCounts,
};
use oxgraph_hyper_bcsr::{
    BcsrHyperedgeId, BcsrNativeHypergraph, BcsrParticipantId, BcsrRole, BcsrSnapshotHypergraph,
    BcsrVertexId,
    build::{FrozenHypergraph, HyperParticipantRole, HypergraphBuilder, export_bcsr_snapshot},
};
use oxgraph_snapshot::Snapshot;
use proptest::{prelude::*, test_runner::TestCaseError};

/// Frozen build-path hypergraph under test.
type Frozen = FrozenHypergraph<u32, u32, u32>;

/// Zero-copy native view borrowed from the frozen wrapper.
type NativeView<'view> = BcsrNativeHypergraph<'view, u32, u32, u32>;

/// View bound over the frozen wrapper's exported snapshot bytes.
type SnapshotView<'view> = BcsrSnapshotHypergraph<'view, u32, u32, u32>;

/// Source/target vertex pick lists for one generated hyperedge.
type HyperedgePicks = (Vec<u32>, Vec<u32>);

/// Strategy producing `(vertex_count, hyperedges)` where each hyperedge is a
/// pair of source/target vertex pick lists within bounds (deduplicated into
/// participant sets by the test body).
fn hypergraph_strategy() -> impl Strategy<Value = (u32, Vec<HyperedgePicks>)> {
    (1_u32..12).prop_flat_map(|vertex_count| {
        let side = proptest::collection::vec(0..vertex_count, 0..=4);
        let hyperedge = (side.clone(), side);
        (
            Just(vertex_count),
            proptest::collection::vec(hyperedge, 0..12),
        )
    })
}

/// Maps the frozen wrapper's role vocabulary onto the view's.
fn view_role(role: HyperParticipantRole) -> BcsrRole {
    match role {
        HyperParticipantRole::Source => BcsrRole::Head,
        HyperParticipantRole::Target => BcsrRole::Tail,
        _ => unreachable!("hypergraph builders only assign source/target roles"),
    }
}

/// Asserts the three shapes yield identical outgoing and incoming hyperedge
/// sequences for `vertex`.
fn assert_vertex_adjacency(
    frozen: &Frozen,
    view: &NativeView<'_>,
    opened: &SnapshotView<'_>,
    vertex: BcsrVertexId<u32>,
) -> Result<(), TestCaseError> {
    let frozen_out: Vec<BcsrHyperedgeId<u32>> = frozen.outgoing_hyperedges(vertex).collect();
    let view_out: Vec<BcsrHyperedgeId<u32>> = view.outgoing_hyperedges(vertex).collect();
    let opened_out: Vec<BcsrHyperedgeId<u32>> = opened.outgoing_hyperedges(vertex).collect();
    prop_assert_eq!(&frozen_out, &view_out);
    prop_assert_eq!(&frozen_out, &opened_out);

    let frozen_in: Vec<BcsrHyperedgeId<u32>> = frozen.incoming_hyperedges(vertex).collect();
    let view_in: Vec<BcsrHyperedgeId<u32>> = view.incoming_hyperedges(vertex).collect();
    let opened_in: Vec<BcsrHyperedgeId<u32>> = opened.incoming_hyperedges(vertex).collect();
    prop_assert_eq!(&frozen_in, &view_in);
    prop_assert_eq!(&frozen_in, &opened_in);
    Ok(())
}

/// Asserts the three shapes yield identical source/target participant and
/// incidence-ID sequences for `hyperedge`.
fn assert_hyperedge_participants(
    frozen: &Frozen,
    view: &NativeView<'_>,
    opened: &SnapshotView<'_>,
    hyperedge: BcsrHyperedgeId<u32>,
) -> Result<(), TestCaseError> {
    let frozen_sources: Vec<BcsrVertexId<u32>> = frozen.source_participants(hyperedge).collect();
    let view_sources: Vec<BcsrVertexId<u32>> = view.source_participants(hyperedge).collect();
    let opened_sources: Vec<BcsrVertexId<u32>> = opened.source_participants(hyperedge).collect();
    prop_assert_eq!(&frozen_sources, &view_sources);
    prop_assert_eq!(&frozen_sources, &opened_sources);

    let frozen_targets: Vec<BcsrVertexId<u32>> = frozen.target_participants(hyperedge).collect();
    let view_targets: Vec<BcsrVertexId<u32>> = view.target_participants(hyperedge).collect();
    let opened_targets: Vec<BcsrVertexId<u32>> = opened.target_participants(hyperedge).collect();
    prop_assert_eq!(&frozen_targets, &view_targets);
    prop_assert_eq!(&frozen_targets, &opened_targets);

    let frozen_ids: Vec<BcsrParticipantId<u32>> = frozen.relation_incidences(hyperedge).collect();
    let view_ids: Vec<BcsrParticipantId<u32>> = view.relation_incidences(hyperedge).collect();
    let opened_ids: Vec<BcsrParticipantId<u32>> = opened.relation_incidences(hyperedge).collect();
    prop_assert_eq!(&frozen_ids, &view_ids);
    prop_assert_eq!(&frozen_ids, &opened_ids);
    Ok(())
}

/// Asserts the three shapes resolve `incidence` to the same vertex,
/// hyperedge, and role.
fn assert_incidence_resolution(
    frozen: &Frozen,
    view: &NativeView<'_>,
    opened: &SnapshotView<'_>,
    incidence: BcsrParticipantId<u32>,
) -> Result<(), TestCaseError> {
    let element = frozen.incidence_element(incidence);
    prop_assert_eq!(view.incidence_element(incidence), element);
    prop_assert_eq!(opened.incidence_element(incidence), element);

    let relation = frozen.incidence_relation(incidence);
    prop_assert_eq!(view.incidence_relation(incidence), relation);
    prop_assert_eq!(opened.incidence_relation(incidence), relation);

    let role = view_role(frozen.incidence_role(incidence));
    prop_assert_eq!(view.incidence_role(incidence), role);
    prop_assert_eq!(opened.incidence_role(incidence), role);
    Ok(())
}

proptest! {
    /// Frozen, `as_view`, and snapshot views agree on every shared observable.
    #[test]
    fn frozen_view_snapshot_agree((vertex_count, hyperedges) in hypergraph_strategy()) {
        let mut builder = HypergraphBuilder::<u32, u32, u32>::new();
        let mut vertices = Vec::new();
        for _ in 0..vertex_count {
            vertices.push(builder.add_vertex().expect("vertex allocates"));
        }
        let mut edges = Vec::with_capacity(hyperedges.len());
        let mut participant_count = 0_usize;
        for (source_picks, target_picks) in &hyperedges {
            let sources: Vec<BcsrVertexId<u32>> = source_picks
                .iter()
                .copied()
                .collect::<BTreeSet<u32>>()
                .into_iter()
                .map(|vertex| vertices[vertex as usize])
                .collect();
            let targets: Vec<BcsrVertexId<u32>> = target_picks
                .iter()
                .copied()
                .collect::<BTreeSet<u32>>()
                .into_iter()
                .map(|vertex| vertices[vertex as usize])
                .collect();
            participant_count += sources.len() + targets.len();
            edges.push(
                builder
                    .add_hyperedge(&sources, &targets)
                    .expect("participants are visible and duplicate-free"),
            );
        }
        let frozen = builder.freeze().expect("builder freezes");
        let view = frozen.as_view();
        let bytes = export_bcsr_snapshot(&frozen).expect("snapshot exports");
        let snapshot = Snapshot::open(&bytes).expect("snapshot opens");
        let opened = SnapshotView::from_snapshot(&snapshot).expect("sections bind");

        // Counts.
        prop_assert_eq!(frozen.element_count(), vertex_count as usize);
        prop_assert_eq!(view.element_count(), vertex_count as usize);
        prop_assert_eq!(opened.element_count(), vertex_count as usize);
        prop_assert_eq!(frozen.relation_count(), edges.len());
        prop_assert_eq!(view.relation_count(), edges.len());
        prop_assert_eq!(opened.relation_count(), edges.len());
        prop_assert_eq!(frozen.incidence_count(), participant_count);
        prop_assert_eq!(view.incidence_count(), participant_count);
        prop_assert_eq!(opened.incidence_count(), participant_count);

        // Per-vertex outgoing/incoming hyperedge sequences.
        for vertex in vertices.iter().copied() {
            assert_vertex_adjacency(&frozen, &view, &opened, vertex)?;
        }

        // Per-hyperedge source/target participant and incidence sequences.
        for hyperedge in edges.iter().copied() {
            assert_hyperedge_participants(&frozen, &view, &opened, hyperedge)?;
        }

        // Per-participant vertex/hyperedge/role resolution.
        for incidence in 0..participant_count {
            let id = BcsrParticipantId::new(u32::try_from(incidence).expect("small index"));
            assert_incidence_resolution(&frozen, &view, &opened, id)?;
        }
    }
}
