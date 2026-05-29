//! Focused tests for public helper surfaces and error branches.

use std::collections::BTreeMap;

use oxgraph_postgres::{
    CatalogError, Config, ConfigError, DualTopologySnapshot, EdgeId, EdgeRow, EngineStatusReport,
    NodeKey, PostgresGraphError, RegisteredEdge, SyncAction, SyncActionCodec, SyncActionWire,
    SyncHealthReport, SyncRow, TableId, edge_row_from_scan, estimate_build, resolve_sync_action,
    validate_sql_ident,
};

#[test]
fn sync_action_codec_decodes_wire_and_dense_actions() -> Result<(), PostgresGraphError> {
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
    let node_map = oxgraph_postgres::dense_node_map_from_edges(&edges)?;
    let source_key = NodeKey::registered(TableId(1), 1);
    let target_key = NodeKey::registered(TableId(1), 3);
    let wire = SyncActionCodec::decode_wire(
        1,
        Some(source_key.0.cast_signed()),
        Some(target_key.0.cast_signed()),
    )?;
    assert_eq!(
        wire,
        SyncActionWire::InsertEdge {
            source: source_key,
            target: target_key,
        }
    );
    let dense = SyncActionCodec::decode(
        1,
        Some(source_key.0.cast_signed()),
        Some(target_key.0.cast_signed()),
        &node_map,
    )?;
    assert_eq!(
        dense,
        SyncAction::InsertEdge {
            source: 0,
            target: 2
        }
    );
    let truncate = SyncActionCodec::decode(4, None, None, &node_map)?;
    assert_eq!(truncate, SyncAction::TruncateOverlays);
    Ok(())
}

#[test]
fn sync_codec_rejects_missing_node_key_args() -> Result<(), PostgresGraphError> {
    let node_map = oxgraph_postgres::dense_node_map_from_edges(&[])?;
    assert!(matches!(
        SyncActionCodec::decode(1, None, Some(0), &node_map),
        Err(oxgraph_postgres::SyncError::InvalidActionArgs { action_type: 1 })
    ));
    Ok(())
}

#[test]
fn truncate_overlays_clears_overlay_state() -> Result<(), PostgresGraphError> {
    let mut overlay = oxgraph_postgres::OverlayState::default();
    overlay.push_edge(oxgraph_postgres::OverlayEdge {
        source: 0,
        target: 1,
    });
    let rows = [SyncRow {
        sequence: 1,
        action: SyncAction::TruncateOverlays,
    }];
    SyncRow::apply_in_order(&rows, &mut overlay)?;
    assert!(overlay.added_edges().is_empty());
    Ok(())
}

#[test]
fn config_validate_rejects_zero_limits() {
    let traverse_zero = Config {
        traverse_limit: 0,
        ..Config::default()
    };
    assert!(matches!(
        traverse_zero.validate(),
        Err(PostgresGraphError::Config(ConfigError::ZeroTraverseLimit))
    ));
    let search_zero = Config {
        search_limit: 0,
        ..Config::default()
    };
    assert!(matches!(
        search_zero.validate(),
        Err(PostgresGraphError::Config(ConfigError::ZeroSearchLimit))
    ));
}

#[test]
fn validate_sql_ident_rejects_invalid_tokens() {
    assert_eq!(validate_sql_ident(""), Err(CatalogError::InvalidSqlIdent));
    assert_eq!(
        validate_sql_ident("1bad"),
        Err(CatalogError::InvalidSqlIdent)
    );
    assert_eq!(
        validate_sql_ident("bad-name"),
        Err(CatalogError::InvalidSqlIdent)
    );
}

#[test]
fn estimate_build_counts_distinct_nodes() {
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
    let estimate = estimate_build(&edges);
    assert_eq!(estimate.node_count, 3);
    assert_eq!(estimate.edge_count, 2);
}

#[test]
fn edge_row_from_scan_packs_registered_keys() {
    let edge = RegisteredEdge {
        id: EdgeId(1),
        source_table: TableId(1),
        target_table: TableId(2),
        source_column: "src".into(),
        target_column: "dst".into(),
        schema: "public".into(),
        name: "edges".into(),
    };
    let row = edge_row_from_scan(&edge, 10, 20);
    assert_eq!(row.source, NodeKey::registered(TableId(1), 10));
    assert_eq!(row.target, NodeKey::registered(TableId(2), 20));
}

#[test]
fn status_json_helpers_emit_expected_fields() {
    let unloaded = EngineStatusReport::unloaded().to_json();
    assert_eq!(unloaded, "{\"loaded\":false}");
    let health = SyncHealthReport {
        overlay_edges: 2,
        tombstoned_edges: 1,
        tombstoned_nodes: 0,
    }
    .to_json();
    assert!(health.contains("\"overlay_edges\":2"));
}

#[test]
fn resolve_sync_action_maps_dense_ids() -> Result<(), PostgresGraphError> {
    let mut node_map = BTreeMap::new();
    let key = NodeKey::registered(TableId(1), 5);
    node_map.insert(key, 7);
    let action = resolve_sync_action(SyncActionWire::DeleteNode { node_id: 3 }, &node_map)?;
    assert_eq!(action, SyncAction::DeleteNode { node_id: 3 });
    Ok(())
}

#[test]
fn dual_topology_snapshot_from_dense_edges_opens() -> Result<(), PostgresGraphError> {
    let bytes = DualTopologySnapshot::from_dense_u32_edges(&[(0, 1), (1, 2)], 0)?;
    assert!(!bytes.is_empty());
    Ok(())
}
