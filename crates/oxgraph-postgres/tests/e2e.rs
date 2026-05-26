//! End-to-end relational build → OXGTOPO persist → engine reload → query parity.

use core::num::{NonZeroU32, NonZeroUsize};

const LIMIT_10: NonZeroUsize = NonZeroUsize::new(10).unwrap();

use oxgraph_postgres::{
    Catalog, Config, EdgeId, EdgeRow, EngineBuilder, GraphRole, NodeKey, PostgresGraphError,
    QueryFreshness, RegisteredEdge, RegisteredTable, SnapshotRebuild, SyncAction, SyncRow, TableId,
    TraversalDirection, TraverseLimits,
    bench_fixture::{chain_catalog, chain_edges},
};

fn chain_engine() -> Result<oxgraph_postgres::Engine, PostgresGraphError> {
    let catalog = chain_catalog()?;
    let edges = chain_edges();
    let bytes = SnapshotRebuild::from_catalog_and_edges(&catalog, &edges, 1_700_000_000)?;
    EngineBuilder::new().snapshot_owned(bytes).build()
}

#[test]
fn relational_build_reload_and_queries() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut catalog = Catalog::new();
    catalog.add_table(RegisteredTable {
        id: TableId(1),
        schema: "public".into(),
        name: "nodes".into(),
        primary_key_column: "id".into(),
    })?;
    catalog.add_edge(RegisteredEdge {
        id: EdgeId(1),
        source_table: TableId(1),
        target_table: TableId(1),
        source_column: "src".into(),
        target_column: "dst".into(),
        schema: "public".into(),
        name: "edges".into(),
    })?;

    let edges = [
        EdgeRow {
            source: NodeKey::registered(TableId(1), 1),
            target: NodeKey::registered(TableId(1), 2),
        },
        EdgeRow {
            source: NodeKey::registered(TableId(1), 2),
            target: NodeKey::registered(TableId(1), 3),
        },
    ];
    let bytes = SnapshotRebuild::from_catalog_and_edges(&catalog, &edges, 1_700_000_000)?;
    let mut engine = EngineBuilder::new().snapshot_owned(bytes).build()?;
    let limits = TraverseLimits::bounded(LIMIT_10);
    let forward = engine.traverse(0, limits, TraversalDirection::Out)?;
    assert_eq!(forward, vec![0, 1, 2]);
    let reverse = engine.traverse(2, limits, TraversalDirection::In)?;
    assert_eq!(reverse, vec![2, 1, 0]);

    engine.apply_sync_rows(&[SyncRow {
        sequence: 1,
        action: SyncAction::InsertEdge {
            source: 0,
            target: 2,
        },
    }])?;
    let with_overlay = engine.traverse(0, limits, TraversalDirection::Out)?;
    assert!(with_overlay.contains(&2));

    engine.rebuild_from_catalog(&catalog, &edges, 1_700_000_001)?;
    let reloaded = engine.traverse(0, limits, TraversalDirection::Out)?;
    assert_eq!(reloaded, vec![0, 1, 2]);
    Ok(())
}

/// Sync rows do not affect traverse until explicitly applied.
#[test]
fn sync_overlay_invisible_until_applied() -> Result<(), PostgresGraphError> {
    let mut engine = chain_engine()?;
    let limits = TraverseLimits::bounded(LIMIT_10);
    let before = engine.traverse(0, limits, TraversalDirection::Out)?;
    assert_eq!(before, vec![0, 1, 2]);
    assert_eq!(engine.sync_health().overlay_edges, 0);
    Ok(())
}

/// Rebuild clears overlay and reflects newly scanned relational edges in the base artifact.
#[test]
fn rebuild_clears_overlay_and_matches_relational() -> Result<(), PostgresGraphError> {
    let catalog = chain_catalog()?;
    let mut edges = chain_edges().to_vec();
    let bytes = SnapshotRebuild::from_catalog_and_edges(&catalog, &edges, 1_700_000_000)?;
    let mut engine = EngineBuilder::new().snapshot_owned(bytes).build()?;

    engine.apply_sync_rows(&[SyncRow {
        sequence: 1,
        action: SyncAction::InsertEdge {
            source: 0,
            target: 2,
        },
    }])?;
    assert_eq!(engine.sync_health().overlay_edges, 1);

    edges.push(EdgeRow {
        source: NodeKey::registered(TableId(1), 1),
        target: NodeKey::registered(TableId(1), 3),
    });
    engine.rebuild_from_catalog(&catalog, &edges, 1_700_000_001)?;
    assert_eq!(engine.sync_health().overlay_edges, 0);

    engine.set_config(Config {
        query_freshness: QueryFreshness::BaseOnly,
        ..Config::default()
    })?;
    let depth_one = engine.traverse(
        0,
        TraverseLimits {
            result_limit: LIMIT_10,
            max_depth: NonZeroU32::new(1),
        },
        TraversalDirection::Out,
    )?;
    assert!(
        depth_one.contains(&2),
        "rebuilt base should include relational shortcut edge"
    );
    Ok(())
}

/// Overlay-aware and base-only freshness diverge on the same overlay insert.
#[test]
fn base_only_vs_overlay_aware_on_same_overlay() -> Result<(), PostgresGraphError> {
    let mut engine = chain_engine()?;
    engine.apply_sync_rows(&[SyncRow {
        sequence: 1,
        action: SyncAction::InsertEdge {
            source: 0,
            target: 2,
        },
    }])?;
    let limits = TraverseLimits {
        result_limit: LIMIT_10,
        max_depth: NonZeroU32::new(1),
    };

    engine.set_config(Config {
        query_freshness: QueryFreshness::OverlayAware,
        ..Config::default()
    })?;
    let overlay_aware = engine.traverse(0, limits, TraversalDirection::Out)?;
    assert!(overlay_aware.contains(&2));

    engine.set_config(Config {
        query_freshness: QueryFreshness::BaseOnly,
        ..Config::default()
    })?;
    let base_only = engine.traverse(0, limits, TraversalDirection::Out)?;
    assert_eq!(base_only, vec![0, 1]);
    Ok(())
}

/// Maintenance-disabled config rejects catalog rebuild.
#[test]
fn maintenance_disabled_blocks_rebuild() -> Result<(), PostgresGraphError> {
    let catalog = chain_catalog()?;
    let edges = chain_edges();
    let mut engine = chain_engine()?;
    engine.set_config(Config {
        maintenance_enabled: false,
        ..Config::default()
    })?;
    assert!(matches!(
        engine.rebuild_from_catalog(&catalog, &edges, 1_700_000_001),
        Err(PostgresGraphError::Config(
            oxgraph_postgres::ConfigError::MaintenanceDisabled
        ))
    ));
    Ok(())
}

#[test]
/// Verifies ACL helper denies reader sessions for admin-only operations.
fn acl_denial_helper() {
    assert!(GraphRole::Reader.satisfies(GraphRole::Reader).is_ok());
    assert!(matches!(
        GraphRole::Reader.satisfies(GraphRole::Admin),
        Err(PostgresGraphError::AccessDenied { .. })
    ));
}
