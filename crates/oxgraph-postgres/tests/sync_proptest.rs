//! Property tests for sync replay ordering and overlay effects.

use oxgraph_postgres::{
    NodeKey, SyncAction, SyncActionWire, SyncRow, TableId, resolve_sync_action,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn strictly_increasing_sequences_apply(
        actions in prop::collection::vec(1_u64..1_000, 1..32),
    ) {
        let mut overlay = oxgraph_postgres::OverlayState::default();
        let rows: Vec<SyncRow> = actions
            .into_iter()
            .enumerate()
            .map(|(index, _)| SyncRow {
                sequence: (index as u64) + 1,
                action: SyncAction::InsertEdge {
                    source: 0,
                    target: 1,
                },
            })
            .collect();
        let applied = SyncRow::apply_in_order(&rows, &mut overlay)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(applied, rows.len());
        prop_assert_eq!(overlay.added_edges.len(), rows.len());
    }

    #[test]
    fn duplicate_sequence_rejected(sequence in 2_u64..1_000) {
        let mut overlay = oxgraph_postgres::OverlayState::default();
        let rows = [
            SyncRow {
                sequence,
                action: SyncAction::InsertEdge {
                    source: 0,
                    target: 1,
                },
            },
            SyncRow {
                sequence,
                action: SyncAction::DeleteNode { node_id: 0 },
            },
        ];
        let is_non_monotonic = matches!(
            SyncRow::apply_in_order(&rows, &mut overlay),
            Err(oxgraph_postgres::PostgresGraphError::Sync(
                oxgraph_postgres::SyncError::NonMonotonicSequence { .. }
            ))
        );
        prop_assert!(is_non_monotonic);
    }

    #[test]
    fn insert_then_remove_overlay_roundtrip(
        source in 0_u32..8,
        target in 0_u32..8,
    ) {
        prop_assume!(source != target);
        let mut overlay = oxgraph_postgres::OverlayState::default();
        let rows = [
            SyncRow {
                sequence: 1,
                action: SyncAction::InsertEdge { source, target },
            },
            SyncRow {
                sequence: 2,
                action: SyncAction::RemoveOverlayEdge { source, target },
            },
        ];
        SyncRow::apply_in_order(&rows, &mut overlay)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert!(overlay.added_edges.is_empty());
    }

    #[test]
    fn keyed_wire_actions_resolve_for_registered_keys(
        source_pk in 1_u64..32,
        target_pk in 1_u64..32,
    ) {
        prop_assume!(source_pk != target_pk);
        let edges = [oxgraph_postgres::EdgeRow {
            source: NodeKey::registered(TableId(1), source_pk),
            target: NodeKey::registered(TableId(1), target_pk),
        }];
        let node_map = oxgraph_postgres::dense_node_map_from_edges(&edges)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let wire = SyncActionWire::InsertEdge {
            source: NodeKey::registered(TableId(1), source_pk),
            target: NodeKey::registered(TableId(1), target_pk),
        };
        let dense = resolve_sync_action(wire, &node_map)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        match dense {
            SyncAction::InsertEdge { source, target } => {
                prop_assert_eq!(source, node_map[&NodeKey::registered(TableId(1), source_pk)]);
                prop_assert_eq!(target, node_map[&NodeKey::registered(TableId(1), target_pk)]);
            }
            other => prop_assert!(false, "unexpected action: {other:?}"),
        }
    }
}
