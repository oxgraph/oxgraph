//! Traversal semantics: depth limits, overlays, and inbound/outbound parity.

use core::num::{NonZeroU32, NonZeroUsize};

const LIMIT_100: NonZeroUsize = NonZeroUsize::new(100).unwrap();
const LIMIT_10: NonZeroUsize = NonZeroUsize::new(10).unwrap();
const LIMIT_1: NonZeroUsize = NonZeroUsize::new(1).unwrap();
const LIMIT_2: NonZeroUsize = NonZeroUsize::new(2).unwrap();

use oxgraph_postgres::{
    Config, DualTopologySnapshot, EngineBuilder, OverlayEdge, OverlayState, QueryFreshness,
    SyncAction, SyncRow, TraversalDirection, TraverseLimits,
};

const CHAIN_EDGES: &[(u32, u32)] = &[(0, 1), (1, 2), (2, 3)];

const fn limits(depth: u32) -> TraverseLimits {
    TraverseLimits {
        result_limit: LIMIT_100,
        max_depth: NonZeroU32::new(depth),
    }
}

fn build_transpose_engine(
    edges: &[(u32, u32)],
) -> Result<oxgraph_postgres::Engine, oxgraph_postgres::PostgresGraphError> {
    let bytes = DualTopologySnapshot::from_dense_u32_edges(edges, 0)?;
    EngineBuilder::new().snapshot_owned(bytes).build()
}

#[test]
fn depth_three_visits_more_than_depth_one() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(CHAIN_EDGES)?;
    let d1 = engine.traverse(0, limits(1), TraversalDirection::Out)?;
    let d3 = engine.traverse(0, limits(3), TraversalDirection::Out)?;
    assert!(d3.len() > d1.len());
    Ok(())
}

/// Pins depth-limited node output on a simple chain (seed depth 0, BFS discovery order).
#[test]
fn exact_depth_node_output_on_chain() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(CHAIN_EDGES)?;
    assert_eq!(
        engine.traverse(0, limits(1), TraversalDirection::Out)?,
        vec![0, 1]
    );
    assert_eq!(
        engine.traverse(0, limits(2), TraversalDirection::Out)?,
        vec![0, 1, 2]
    );
    assert_eq!(
        engine.traverse(0, limits(3), TraversalDirection::Out)?,
        vec![0, 1, 2, 3]
    );
    Ok(())
}

/// Collect cardinality matches count when `result_limit` covers the reachable set.
#[test]
fn collect_len_matches_visited_count_on_chain() -> Result<(), oxgraph_postgres::PostgresGraphError>
{
    let mut engine = build_transpose_engine(CHAIN_EDGES)?;
    for depth in [1_u32, 2, 3] {
        let lim = limits(depth);
        assert_eq!(
            engine.traverse(0, lim, TraversalDirection::Out)?.len(),
            engine.visited_count(0, lim, TraversalDirection::Out)?
        );
    }
    Ok(())
}

/// Count-only traversal discovers boundary nodes but does not enqueue past depth.
#[test]
fn exact_depth_visited_count_on_chain() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(CHAIN_EDGES)?;
    assert_eq!(
        engine.visited_count(0, limits(1), TraversalDirection::Out)?,
        2
    );
    assert_eq!(
        engine.visited_count(0, limits(2), TraversalDirection::Out)?,
        3
    );
    assert_eq!(
        engine.visited_count(0, limits(3), TraversalDirection::Out)?,
        4
    );
    Ok(())
}

/// Second query must not see neighbors staged by the first (epoch + no stale buffer).
#[test]
fn repeated_query_scratch_isolation() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    // Node 2 exists (self-loop) but is unreachable from 0; prior query must not leak `1`.
    let mut engine = build_transpose_engine(&[(0, 1), (2, 2)])?;
    let _ = engine.visited_count(0, limits(100), TraversalDirection::Out)?;
    let isolated = engine.visited_count(2, limits(100), TraversalDirection::Out)?;
    assert_eq!(isolated, 1);
    let out = engine.traverse(2, limits(100), TraversalDirection::Out)?;
    assert_eq!(out, vec![2]);
    Ok(())
}

#[test]
fn count_only_result_limit_stops_during_discovery()
-> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(&[(0, 1), (0, 2), (0, 3)])?;
    let limits = TraverseLimits {
        result_limit: LIMIT_2,
        max_depth: None,
    };
    assert_eq!(engine.visited_count(0, limits, TraversalDirection::Out)?, 2);
    Ok(())
}

#[test]
fn inbound_depth_limited_matches_outbound_on_chain()
-> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(CHAIN_EDGES)?;
    let expected_out = [vec![0, 1], vec![0, 1, 2], vec![0, 1, 2, 3]];
    let expected_in = [vec![3, 2], vec![3, 2, 1], vec![3, 2, 1, 0]];
    for (depth, (out_nodes, in_nodes)) in [1_u32, 2, 3]
        .into_iter()
        .zip(expected_out.into_iter().zip(expected_in))
    {
        let lim = limits(depth);
        assert_eq!(engine.traverse(0, lim, TraversalDirection::Out)?, out_nodes);
        assert_eq!(engine.traverse(3, lim, TraversalDirection::In)?, in_nodes);
        assert_eq!(
            engine.visited_count(0, lim, TraversalDirection::Out)?,
            engine.visited_count(3, lim, TraversalDirection::In)?
        );
    }
    Ok(())
}

#[test]
fn depth_limited_honors_overlay_under_base_only() -> Result<(), oxgraph_postgres::PostgresGraphError>
{
    let mut engine = build_transpose_engine(&[(0, 1)])?;
    engine.apply_sync_rows(&[SyncRow {
        sequence: 1,
        action: SyncAction::InsertEdge {
            source: 0,
            target: 2,
        },
    }])?;
    engine.set_config(Config {
        query_freshness: QueryFreshness::BaseOnly,
        ..Config::default()
    })?;
    assert_eq!(
        engine.traverse(0, limits(2), TraversalDirection::Out)?,
        vec![0, 1]
    );
    assert_eq!(
        engine.visited_count(0, limits(2), TraversalDirection::Out)?,
        2
    );
    Ok(())
}

#[test]
fn result_limit_stops_before_expanding_high_degree_node()
-> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(&[(0, 1), (0, 2), (0, 3), (0, 4)])?;
    let limits = TraverseLimits {
        result_limit: LIMIT_1,
        max_depth: None,
    };
    let out = engine.traverse(0, limits, TraversalDirection::Out)?;
    assert_eq!(out, vec![0]);
    Ok(())
}

#[test]
fn base_only_skips_overlay_edges_but_honors_node_tombstones()
-> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(&[(0, 1)])?;
    engine.apply_sync_rows(&[SyncRow {
        sequence: 1,
        action: SyncAction::InsertEdge {
            source: 0,
            target: 2,
        },
    }])?;
    engine.set_config(Config {
        query_freshness: QueryFreshness::BaseOnly,
        ..Config::default()
    })?;
    let with_overlay = engine.traverse(
        0,
        TraverseLimits::bounded(LIMIT_10),
        TraversalDirection::Out,
    )?;
    assert!(!with_overlay.contains(&2));

    engine.apply_sync_rows(&[SyncRow {
        sequence: 2,
        action: SyncAction::DeleteNode { node_id: 1 },
    }])?;
    let base_only = engine.traverse(
        0,
        TraverseLimits::bounded(LIMIT_10),
        TraversalDirection::Out,
    )?;
    assert!(!base_only.contains(&1));
    Ok(())
}

#[test]
fn inbound_outbound_parity_on_transpose_fixture() -> Result<(), oxgraph_postgres::PostgresGraphError>
{
    let mut engine = build_transpose_engine(&[(0, 1), (1, 2), (2, 3)])?;
    let limits = TraverseLimits {
        result_limit: LIMIT_100,
        max_depth: None,
    };
    let out = engine.traverse(0, limits, TraversalDirection::Out)?;
    let inbound = engine.traverse(3, limits, TraversalDirection::In)?;
    assert_eq!(out.len(), inbound.len());
    for node in &out {
        assert!(inbound.contains(node));
    }
    Ok(())
}

/// Tombstoned base edges must not appear on the inbound predecessor path.
#[test]
fn inbound_hides_tombstoned_base_edge() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(&[(0, 1), (1, 2)])?;
    engine.apply_sync_rows(&[SyncRow {
        sequence: 1,
        action: SyncAction::DeleteEdge { edge_id: 1 },
    }])?;
    assert_eq!(
        engine.traverse(0, limits(2), TraversalDirection::Out)?,
        vec![0, 1]
    );
    assert_eq!(
        engine.traverse(2, limits(2), TraversalDirection::In)?,
        vec![2]
    );
    assert_eq!(
        engine.visited_count(2, limits(2), TraversalDirection::In)?,
        1
    );
    Ok(())
}

#[test]
fn count_only_base_only_dedups_parallel_outgoing_edges()
-> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(&[(0, 1), (0, 1), (0, 2)])?;
    engine.set_config(Config {
        query_freshness: QueryFreshness::BaseOnly,
        ..Config::default()
    })?;
    assert_eq!(
        engine.visited_count(0, limits(1), TraversalDirection::Out)?,
        3
    );
    assert_eq!(
        engine.visited_count(0, limits(2), TraversalDirection::Out)?,
        3
    );
    Ok(())
}

#[test]
fn count_only_base_only_dedups_parallel_incoming_edges()
-> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(&[(0, 2), (0, 2), (1, 2)])?;
    engine.set_config(Config {
        query_freshness: QueryFreshness::BaseOnly,
        ..Config::default()
    })?;
    assert_eq!(
        engine.visited_count(2, limits(1), TraversalDirection::In)?,
        3
    );
    Ok(())
}

#[test]
fn count_only_depth_one_handles_self_loop_and_limit()
-> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(&[(0, 0), (0, 1), (0, 1)])?;
    engine.set_config(Config {
        query_freshness: QueryFreshness::BaseOnly,
        ..Config::default()
    })?;
    assert_eq!(
        engine.visited_count(0, limits(1), TraversalDirection::Out)?,
        2
    );
    assert_eq!(
        engine.visited_count(
            0,
            TraverseLimits {
                result_limit: LIMIT_1,
                max_depth: NonZeroU32::new(1),
            },
            TraversalDirection::Out,
        )?,
        1
    );
    assert_eq!(
        engine.visited_count(
            0,
            TraverseLimits {
                result_limit: LIMIT_2,
                max_depth: NonZeroU32::new(1),
            },
            TraversalDirection::Out,
        )?,
        2
    );
    Ok(())
}

#[test]
fn unique_count_path_honors_node_tombstones() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(&[(0, 1), (0, 1), (0, 2)])?;
    engine.set_config(Config {
        query_freshness: QueryFreshness::BaseOnly,
        ..Config::default()
    })?;
    engine.apply_sync_rows(&[SyncRow {
        sequence: 1,
        action: SyncAction::DeleteNode { node_id: 1 },
    }])?;
    assert_eq!(
        engine.visited_count(0, limits(1), TraversalDirection::Out)?,
        2
    );
    Ok(())
}

#[test]
fn edge_tombstones_use_parallel_profile() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_transpose_engine(&[(0, 1), (0, 1)])?;
    engine.set_config(Config {
        query_freshness: QueryFreshness::BaseOnly,
        ..Config::default()
    })?;
    engine.apply_sync_rows(&[
        SyncRow {
            sequence: 1,
            action: SyncAction::DeleteEdge { edge_id: 0 },
        },
        SyncRow {
            sequence: 2,
            action: SyncAction::DeleteEdge { edge_id: 1 },
        },
    ])?;
    assert_eq!(
        engine.visited_count(0, limits(1), TraversalDirection::Out)?,
        1
    );
    assert_eq!(
        engine.visited_count(1, limits(1), TraversalDirection::Out)?,
        1
    );
    Ok(())
}

#[test]
fn overlay_indexes_match_manual_edges() {
    let mut overlay = OverlayState::default();
    overlay.push_edge(OverlayEdge {
        source: 1,
        target: 9,
    });
    assert_eq!(overlay.overlay_targets(1), &[9]);
    overlay.clear();
    assert!(overlay.overlay_targets(1).is_empty());
}
