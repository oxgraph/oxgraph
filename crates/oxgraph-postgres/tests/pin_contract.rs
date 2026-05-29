//! Characterization pins for build, catalog, and sync before structural refactors.

use core::num::NonZeroUsize;

use oxgraph_postgres::{
    Catalog, CatalogError, EdgeId, EdgeRow, EngineBuilder, NodeKey, OverlayEdge, OverlayState,
    RegisteredEdge, RegisteredTable, SnapshotRebuild, SyncAction, SyncRow, TableId,
};

const LIMIT_10: NonZeroUsize = NonZeroUsize::new(10).unwrap();

#[test]
fn snapshot_rebuild_roundtrip_status() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut catalog = Catalog::new();
    catalog.add_table(RegisteredTable {
        id: TableId(1),
        schema: "public".into(),
        name: "nodes".into(),
        primary_key_column: "id".into(),
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
    let engine = EngineBuilder::new().snapshot_owned(bytes).build()?;
    assert_eq!(engine.status().node_count, 3);
    assert_eq!(engine.status().edge_count, 2);
    Ok(())
}

#[test]
fn catalog_rejects_duplicate_table_id() -> Result<(), CatalogError> {
    let mut catalog = Catalog::new();
    let table = RegisteredTable {
        id: TableId(1),
        schema: "public".into(),
        name: "a".into(),
        primary_key_column: "id".into(),
    };
    catalog.add_table(table.clone())?;
    assert_eq!(
        catalog.add_table(table),
        Err(CatalogError::DuplicateTableId(TableId(1)))
    );
    Ok(())
}

#[test]
fn catalog_rejects_duplicate_table_name() -> Result<(), CatalogError> {
    let mut catalog = Catalog::new();
    catalog.add_table(RegisteredTable {
        id: TableId(1),
        schema: "public".into(),
        name: "nodes".into(),
        primary_key_column: "id".into(),
    })?;
    assert_eq!(
        catalog.add_table(RegisteredTable {
            id: TableId(2),
            schema: "public".into(),
            name: "nodes".into(),
            primary_key_column: "id".into(),
        }),
        Err(CatalogError::DuplicateTableName {
            schema: "public".into(),
            name: "nodes".into(),
        })
    );
    Ok(())
}

#[test]
fn catalog_rejects_edge_with_missing_endpoint() -> Result<(), CatalogError> {
    let mut catalog = Catalog::new();
    catalog.add_table(RegisteredTable {
        id: TableId(1),
        schema: "public".into(),
        name: "nodes".into(),
        primary_key_column: "id".into(),
    })?;
    assert_eq!(
        catalog.add_edge(RegisteredEdge {
            id: EdgeId(1),
            source_table: TableId(9),
            target_table: TableId(1),
            source_column: "src".into(),
            target_column: "dst".into(),
            schema: "public".into(),
            name: "edges".into(),
        }),
        Err(CatalogError::MissingTable(TableId(9)))
    );
    Ok(())
}

#[test]
fn sync_apply_in_order_rejects_non_monotonic_sequence() {
    let mut overlay = OverlayState::default();
    let rows = [
        SyncRow {
            sequence: 2,
            action: SyncAction::InsertEdge {
                source: 0,
                target: 1,
            },
        },
        SyncRow {
            sequence: 2,
            action: SyncAction::DeleteNode { node_id: 0 },
        },
    ];
    assert!(matches!(
        SyncRow::apply_in_order(&rows, &mut overlay),
        Err(oxgraph_postgres::PostgresGraphError::Sync(
            oxgraph_postgres::SyncError::NonMonotonicSequence { .. }
        ))
    ));
}

#[test]
fn sync_overlay_tombstone_hides_node() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut overlay = OverlayState::default();
    overlay.push_edge(OverlayEdge {
        source: 0,
        target: 1,
    });
    let rows = [SyncRow {
        sequence: 1,
        action: SyncAction::DeleteNode { node_id: 0 },
    }];
    let applied = SyncRow::apply_in_order(&rows, &mut overlay)?;
    assert_eq!(applied, 1);
    assert!(!overlay.node_visible(0));
    Ok(())
}

#[test]
fn dual_topology_dense_edges_loads_engine() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    use oxgraph_postgres::DualTopologySnapshot;

    let bytes = DualTopologySnapshot::from_dense_u32_edges(&[(0, 1), (1, 2)], 0)?;
    let mut engine = EngineBuilder::new().snapshot_owned(bytes).build()?;
    assert_eq!(engine.status().node_count, 3);
    assert_eq!(engine.status().edge_count, 2);
    let visited = engine.visited_count(
        0,
        oxgraph_postgres::TraverseLimits::bounded(LIMIT_10),
        oxgraph_postgres::TraversalDirection::Out,
    )?;
    assert_eq!(visited, 3);
    Ok(())
}
