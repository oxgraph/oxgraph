//! Kani proofs for bounded Postgres engine contracts.
//!
//! Overlay/sync replay proofs are skipped — unbounded `Vec` growth in harnesses.

#![cfg(kani)]

use zerocopy::FromBytes;

use crate::{artifact::PostgresMetadata, role::GraphRole, sync::SyncAction};

/// `GraphRole` equality is reflexive on the two variants.
#[kani::proof]
fn graph_role_reflexive() {
    let role: GraphRole = if kani::any() {
        GraphRole::Reader
    } else {
        GraphRole::Admin
    };
    assert_eq!(role, role);
}

/// `SyncAction` equality is reflexive for bounded symbolic variants.
#[kani::proof]
fn sync_action_reflexive() {
    let action = if kani::any() {
        SyncAction::InsertEdge {
            source: kani::any(),
            target: kani::any(),
        }
    } else if kani::any() {
        SyncAction::DeleteEdge {
            edge_id: kani::any(),
        }
    } else if kani::any() {
        SyncAction::DeleteNode {
            node_id: kani::any(),
        }
    } else {
        SyncAction::TruncateOverlays
    };
    assert_eq!(action, action);
}

/// `PostgresMetadata::new` round-trips through zerocopy layout bytes.
#[kani::proof]
fn postgres_metadata_roundtrip() {
    let node_count: u32 = kani::any();
    let edge_count: u32 = kani::any();
    let built_at: u64 = kani::any();
    let read_only: bool = kani::any();
    let meta = PostgresMetadata::new(node_count, edge_count, built_at, read_only);
    let bytes = meta.as_bytes();
    let parsed = PostgresMetadata::read_from_bytes(bytes);
    assert_eq!(parsed.ok(), Some(meta));
}
