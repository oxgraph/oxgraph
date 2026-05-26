//! Boundary tests for catalog keys, sync wire format, and dense resolution.

use std::collections::BTreeMap;

use oxgraph_postgres::{
    CatalogError, EdgeRow, NodeKey, PostgresGraphError, SyncAction, SyncActionCodec,
    SyncActionWire, SyncRow, TableId, dense_node_map_from_edges, edge_id_from_i32,
    resolve_sync_action, resolve_sync_rows, table_id_from_i32, validate_primary_key,
};

#[test]
fn validate_primary_key_accepts_zero_and_u32_max() -> Result<(), CatalogError> {
    assert_eq!(validate_primary_key(0)?, 0);
    assert_eq!(
        validate_primary_key(i64::from(u32::MAX))?,
        u64::from(u32::MAX)
    );
    Ok(())
}

#[test]
fn validate_primary_key_rejects_negative_and_overflow() {
    assert_eq!(
        validate_primary_key(-1),
        Err(CatalogError::InvalidPrimaryKey)
    );
    assert_eq!(
        validate_primary_key(i64::from(u32::MAX) + 1),
        Err(CatalogError::PrimaryKeyOutOfRange)
    );
    assert_eq!(
        validate_primary_key(i64::MAX),
        Err(CatalogError::PrimaryKeyOutOfRange)
    );
}

#[test]
fn table_and_edge_id_parsing_rejects_invalid_values() {
    assert_eq!(table_id_from_i32(-1), Err(CatalogError::InvalidTableId));
    assert_eq!(edge_id_from_i32(-1), Err(CatalogError::InvalidEdgeId));
}

#[test]
fn node_keys_from_different_tables_do_not_collide() {
    let left = NodeKey::registered(TableId(1), 42);
    let right = NodeKey::registered(TableId(2), 42);
    assert_ne!(left, right);
    assert_eq!(left.table_id(), TableId(1));
    assert_eq!(right.table_id(), TableId(2));
    assert_eq!(left.primary_key(), 42);
}

#[test]
fn dense_node_map_assigns_sorted_keys() -> Result<(), PostgresGraphError> {
    let edges = [
        EdgeRow {
            source: NodeKey::registered(TableId(1), 3),
            target: NodeKey::registered(TableId(1), 1),
        },
        EdgeRow {
            source: NodeKey::registered(TableId(2), 1),
            target: NodeKey::registered(TableId(1), 2),
        },
    ];
    let map = dense_node_map_from_edges(&edges)?;
    assert_eq!(map.len(), 4);
    assert_eq!(map[&NodeKey::registered(TableId(1), 1)], 0);
    assert_eq!(map[&NodeKey::registered(TableId(1), 2)], 1);
    assert_eq!(map[&NodeKey::registered(TableId(1), 3)], 2);
    assert_eq!(map[&NodeKey::registered(TableId(2), 1)], 3);
    Ok(())
}

#[test]
fn sync_codec_decodes_remove_overlay_wire_action() -> Result<(), oxgraph_postgres::SyncError> {
    let source = NodeKey::registered(TableId(1), 1);
    let target = NodeKey::registered(TableId(1), 3);
    let wire = SyncActionCodec::decode_wire(
        5,
        Some(source.0.cast_signed()),
        Some(target.0.cast_signed()),
    )?;
    assert_eq!(wire, SyncActionWire::RemoveOverlayEdge { source, target });
    Ok(())
}

#[test]
fn sync_codec_rejects_invalid_action_type() {
    assert!(matches!(
        SyncActionCodec::decode_wire(99, None, None),
        Err(oxgraph_postgres::SyncError::InvalidActionType { action_type: 99 })
    ));
}

#[test]
fn remove_overlay_edge_resolves_to_dense_ids() -> Result<(), PostgresGraphError> {
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
    let node_map = dense_node_map_from_edges(&edges)?;
    let action = resolve_sync_action(
        SyncActionWire::RemoveOverlayEdge {
            source: NodeKey::registered(TableId(1), 1),
            target: NodeKey::registered(TableId(1), 3),
        },
        &node_map,
    )?;
    assert_eq!(
        action,
        SyncAction::RemoveOverlayEdge {
            source: 0,
            target: 2
        }
    );
    Ok(())
}

#[test]
fn remove_overlay_edge_clears_matching_overlay() -> Result<(), PostgresGraphError> {
    let mut overlay = oxgraph_postgres::OverlayState::default();
    overlay.push_edge(oxgraph_postgres::OverlayEdge {
        source: 0,
        target: 2,
    });
    overlay.push_edge(oxgraph_postgres::OverlayEdge {
        source: 1,
        target: 2,
    });
    let rows = [SyncRow {
        sequence: 1,
        action: SyncAction::RemoveOverlayEdge {
            source: 0,
            target: 2,
        },
    }];
    SyncRow::apply_in_order(&rows, &mut overlay)?;
    assert_eq!(overlay.added_edges.len(), 1);
    assert_eq!(
        overlay.added_edges[0],
        oxgraph_postgres::OverlayEdge {
            source: 1,
            target: 2
        }
    );
    Ok(())
}

#[test]
fn resolve_sync_rows_preserves_sequence_order() -> Result<(), PostgresGraphError> {
    let edges = [EdgeRow {
        source: NodeKey::registered(TableId(1), 1),
        target: NodeKey::registered(TableId(1), 2),
    }];
    let source = NodeKey::registered(TableId(1), 1);
    let target = NodeKey::registered(TableId(1), 2);
    let raw_rows = [
        (
            2_u64,
            1_i16,
            Some(source.0.cast_signed()),
            Some(target.0.cast_signed()),
        ),
        (1_u64, 4_i16, None, None),
    ];
    let rows = resolve_sync_rows(&edges, &raw_rows)?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].sequence, 2);
    assert_eq!(rows[1].sequence, 1);
    assert_eq!(rows[1].action, SyncAction::TruncateOverlays);
    Ok(())
}

#[test]
fn keyed_actions_require_present_node_map_entry() {
    let mut node_map = BTreeMap::new();
    node_map.insert(NodeKey::registered(TableId(1), 1), 0);
    assert!(matches!(
        resolve_sync_action(
            SyncActionWire::InsertEdge {
                source: NodeKey::registered(TableId(1), 1),
                target: NodeKey::registered(TableId(1), 99),
            },
            &node_map,
        ),
        Err(oxgraph_postgres::SyncError::UnknownNodeKey { .. })
    ));
}

#[test]
fn sync_resolution_includes_keys_from_pending_log_rows() -> Result<(), PostgresGraphError> {
    let edges = [EdgeRow {
        source: NodeKey::registered(TableId(1), 1),
        target: NodeKey::registered(TableId(1), 2),
    }];
    let source = NodeKey::registered(TableId(1), 1);
    let target = NodeKey::registered(TableId(1), 3);
    let raw_rows = [(
        1_u64,
        5_i16,
        Some(source.0.cast_signed()),
        Some(target.0.cast_signed()),
    )];
    let rows = resolve_sync_rows(&edges, &raw_rows)?;
    assert_eq!(
        rows[0].action,
        SyncAction::RemoveOverlayEdge {
            source: 0,
            target: 2
        }
    );
    Ok(())
}
