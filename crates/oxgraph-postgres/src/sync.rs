//! Sync log replay into overlay buffers.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

use crate::{
    build::EdgeRow,
    catalog::NodeKey,
    error::{BuildError, PostgresGraphError, SyncError},
    overlay::{OverlayEdge, OverlayState},
};

/// One durable sync row interpreted by the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncAction {
    /// Insert a new overlay edge between dense node ids.
    InsertEdge {
        /// Source node id.
        source: u32,
        /// Target node id.
        target: u32,
    },
    /// Remove a previously inserted overlay edge between dense node ids.
    RemoveOverlayEdge {
        /// Source node id.
        source: u32,
        /// Target node id.
        target: u32,
    },
    /// Tombstone a base edge id.
    DeleteEdge {
        /// Base CSR edge id.
        edge_id: u32,
    },
    /// Tombstone a node id from query results.
    DeleteNode {
        /// Dense node id.
        node_id: u32,
    },
    /// Truncate all overlays (maintenance prelude).
    TruncateOverlays,
}

impl SyncAction {
    /// Applies this action to `overlay`.
    ///
    /// # Performance
    ///
    /// This method is `O(log t)` for indexed overlay mutations.
    pub(crate) fn apply_to(self, overlay: &mut OverlayState) {
        match self {
            Self::InsertEdge { source, target } => {
                overlay.push_edge(OverlayEdge { source, target });
            }
            Self::RemoveOverlayEdge { source, target } => {
                overlay.remove_edge(source, target);
            }
            Self::DeleteEdge { edge_id } => {
                overlay.tombstone_edge(edge_id);
            }
            Self::DeleteNode { node_id } => {
                overlay.tombstone_node(node_id);
            }
            Self::TruncateOverlays => overlay.clear(),
        }
    }
}

/// Persisted sync-log action before dense-node resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncActionWire {
    /// Insert an overlay edge keyed by registered `(table_id, primary_key)` pairs.
    InsertEdge {
        /// Source node key from the sync log.
        source: NodeKey,
        /// Target node key from the sync log.
        target: NodeKey,
    },
    /// Remove an overlay edge keyed by registered node keys.
    RemoveOverlayEdge {
        /// Source node key from the sync log.
        source: NodeKey,
        /// Target node key from the sync log.
        target: NodeKey,
    },
    /// Tombstone a base CSR edge id (already dense).
    DeleteEdge {
        /// Base CSR edge id.
        edge_id: u32,
    },
    /// Tombstone a dense node id (already dense).
    DeleteNode {
        /// Dense node id.
        node_id: u32,
    },
    /// Truncate all overlay buffers.
    TruncateOverlays,
}

/// Persisted sync-log action type ids shared with trigger SQL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum SyncActionCodec {
    /// [`SyncActionWire::InsertEdge`]
    InsertEdge = 1,
    /// [`SyncActionWire::DeleteEdge`]
    DeleteEdge = 2,
    /// [`SyncActionWire::DeleteNode`]
    DeleteNode = 3,
    /// [`SyncActionWire::TruncateOverlays`]
    TruncateOverlays = 4,
    /// [`SyncActionWire::RemoveOverlayEdge`]
    RemoveOverlayEdge = 5,
}

impl SyncActionCodec {
    /// Returns whether this action carries registered node-key arguments.
    ///
    /// Insert and remove-overlay actions reference source/target node keys;
    /// the others carry dense ids or nothing. Both [`Self::decode_wire`] and the
    /// dense-map key harvest consult this so the keyed-action set lives in one
    /// place instead of being a magic literal pair.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn carries_node_keys(self) -> bool {
        matches!(self, Self::InsertEdge | Self::RemoveOverlayEdge)
    }

    /// Decodes a persisted sync row into a wire action.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when the action type or arguments are invalid.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub fn decode_wire(
        action_type: i16,
        arg0: Option<i64>,
        arg1: Option<i64>,
    ) -> Result<SyncActionWire, SyncError> {
        match Self::try_from(action_type)? {
            Self::InsertEdge => {
                let source = decode_node_key(arg0, action_type)?;
                let target = decode_node_key(arg1, action_type)?;
                Ok(SyncActionWire::InsertEdge { source, target })
            }
            Self::DeleteEdge => {
                let edge_id =
                    u32::try_from(arg0.ok_or(SyncError::InvalidActionArgs { action_type })?)
                        .map_err(|_| SyncError::InvalidActionArgs { action_type })?;
                Ok(SyncActionWire::DeleteEdge { edge_id })
            }
            Self::DeleteNode => {
                let node_id =
                    u32::try_from(arg0.ok_or(SyncError::InvalidActionArgs { action_type })?)
                        .map_err(|_| SyncError::InvalidActionArgs { action_type })?;
                Ok(SyncActionWire::DeleteNode { node_id })
            }
            Self::TruncateOverlays => Ok(SyncActionWire::TruncateOverlays),
            Self::RemoveOverlayEdge => {
                let source = decode_node_key(arg0, action_type)?;
                let target = decode_node_key(arg1, action_type)?;
                Ok(SyncActionWire::RemoveOverlayEdge { source, target })
            }
        }
    }

    /// Decodes a persisted sync row into a dense [`SyncAction`].
    ///
    /// Keyed insert/remove actions require `node_map` built from the current
    /// relational edge scan.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when decoding or dense resolution fails.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` per row excluding map lookup.
    pub fn decode(
        action_type: i16,
        arg0: Option<i64>,
        arg1: Option<i64>,
        node_map: &BTreeMap<NodeKey, u32>,
    ) -> Result<SyncAction, SyncError> {
        resolve_sync_action(Self::decode_wire(action_type, arg0, arg1)?, node_map)
    }
}

impl TryFrom<i16> for SyncActionCodec {
    type Error = SyncError;

    /// Maps a persisted action-type id to its codec variant.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::InvalidActionType`] for an unknown id.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn try_from(value: i16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::InsertEdge),
            2 => Ok(Self::DeleteEdge),
            3 => Ok(Self::DeleteNode),
            4 => Ok(Self::TruncateOverlays),
            5 => Ok(Self::RemoveOverlayEdge),
            action_type => Err(SyncError::InvalidActionType { action_type }),
        }
    }
}

/// Resolves one wire sync action into dense engine coordinates.
///
/// # Errors
///
/// Returns [`SyncError::UnknownNodeKey`] when a keyed action references an
/// unassigned node key.
///
/// # Performance
///
/// This function is `O(log n)` for keyed actions.
pub fn resolve_sync_action(
    action: SyncActionWire,
    node_map: &BTreeMap<NodeKey, u32>,
) -> Result<SyncAction, SyncError> {
    match action {
        SyncActionWire::InsertEdge { source, target } => Ok(SyncAction::InsertEdge {
            source: lookup_dense(source, node_map)?,
            target: lookup_dense(target, node_map)?,
        }),
        SyncActionWire::RemoveOverlayEdge { source, target } => Ok(SyncAction::RemoveOverlayEdge {
            source: lookup_dense(source, node_map)?,
            target: lookup_dense(target, node_map)?,
        }),
        SyncActionWire::DeleteEdge { edge_id } => Ok(SyncAction::DeleteEdge { edge_id }),
        SyncActionWire::DeleteNode { node_id } => Ok(SyncAction::DeleteNode { node_id }),
        SyncActionWire::TruncateOverlays => Ok(SyncAction::TruncateOverlays),
    }
}

/// Raw sync-log row scanned from SPI before dense resolution.
pub type RawSyncRow = (u64, i16, Option<i64>, Option<i64>);

/// Resolves persisted sync rows using the current relational edge scan.
///
/// # Errors
///
/// Returns [`PostgresGraphError::Build`] when dense assignment fails, or
/// [`PostgresGraphError::Sync`] when a row cannot be decoded or resolved.
///
/// # Performance
///
/// This function is `O(n log n + m + r log r)` for edge scan size and row count `r`.
pub fn resolve_sync_rows(
    edges: &[EdgeRow],
    raw_rows: &[RawSyncRow],
) -> Result<Vec<SyncRow>, PostgresGraphError> {
    let node_map = dense_node_map_for_sync_resolution(edges, raw_rows)?;
    let mut rows = Vec::with_capacity(raw_rows.len());
    for (sequence, action_type, arg0, arg1) in raw_rows {
        let action = SyncActionCodec::decode(*action_type, *arg0, *arg1, &node_map)?;
        rows.push(SyncRow {
            sequence: *sequence,
            action,
        });
    }
    Ok(rows)
}

/// Builds the dense node assignment used when replaying sync rows.
///
/// Keys come from the current relational edge scan plus any keyed sync-log
/// arguments so deletes remain resolvable after base edge rows disappear.
///
/// # Errors
///
/// Returns [`PostgresGraphError::Build`] when dense assignment fails.
///
/// # Performance
///
/// This function is `O(n log n + m + r)` for edge count `m` and sync row count `r`.
pub fn dense_node_map_for_sync_resolution(
    edges: &[EdgeRow],
    raw_rows: &[RawSyncRow],
) -> Result<BTreeMap<NodeKey, u32>, PostgresGraphError> {
    let mut keys = BTreeSet::new();
    for edge in edges {
        keys.insert(edge.source);
        keys.insert(edge.target);
    }
    for (_, action_type, arg0, arg1) in raw_rows {
        if SyncActionCodec::try_from(*action_type).is_ok_and(SyncActionCodec::carries_node_keys) {
            if let Some(key) = node_key_from_i64(*arg0) {
                keys.insert(key);
            }
            if let Some(key) = node_key_from_i64(*arg1) {
                keys.insert(key);
            }
        }
    }
    let mut map = BTreeMap::new();
    for (index, key) in keys.into_iter().enumerate() {
        let dense = u32::try_from(index).map_err(|_| BuildError::NodeCountOverflow)?;
        map.insert(key, dense);
    }
    Ok(map)
}

/// Parses a non-negative sync-log node key when present.
fn node_key_from_i64(value: Option<i64>) -> Option<NodeKey> {
    let value = value?;
    if value.is_negative() {
        return None;
    }
    u64::try_from(value).ok().map(NodeKey)
}

/// Decodes a persisted node-key argument from the sync log.
fn decode_node_key(value: Option<i64>, action_type: i16) -> Result<NodeKey, SyncError> {
    let value = value.ok_or(SyncError::InvalidActionArgs { action_type })?;
    if value.is_negative() {
        return Err(SyncError::InvalidActionArgs { action_type });
    }
    let key = u64::try_from(value).map_err(|_| SyncError::InvalidActionArgs { action_type })?;
    Ok(NodeKey(key))
}

/// Maps a registered node key to its dense id using the current edge scan.
fn lookup_dense(key: NodeKey, node_map: &BTreeMap<NodeKey, u32>) -> Result<u32, SyncError> {
    node_map
        .get(&key)
        .copied()
        .ok_or(SyncError::UnknownNodeKey { key })
}

/// One sync log row with monotonic sequence metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncRow {
    /// Monotonic sequence number assigned by the extension.
    pub sequence: u64,
    /// Action to replay.
    pub action: SyncAction,
}

impl SyncRow {
    /// Applies caller-ordered rows to `overlay`, rejecting any that are not in
    /// strictly increasing sequence order.
    ///
    /// Rows must arrive already sorted by `sequence`; this method validates that
    /// contract rather than silently normalizing it, so genuinely out-of-order
    /// input (e.g. `[5, 3, 1]`) is rejected, not just duplicates. The extension
    /// emits rows in sequence order, so this is a cheap defensive check.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Sync`] when a row's sequence is not strictly
    /// greater than its predecessor.
    ///
    /// # Performance
    ///
    /// This method is `O(r + a)` where `r` is row count and `a` is applied actions.
    pub fn apply_in_order(
        rows: &[Self],
        overlay: &mut OverlayState,
    ) -> Result<usize, PostgresGraphError> {
        let mut applied = 0_usize;
        let mut last_sequence = None;
        for row in rows {
            if let Some(previous) = last_sequence
                && row.sequence <= previous
            {
                return Err(SyncError::NonMonotonicSequence {
                    sequence: row.sequence,
                    previous,
                }
                .into());
            }
            last_sequence = Some(row.sequence);
            row.action.apply_to(overlay);
            applied += 1;
        }
        Ok(applied)
    }
}

/// Returns a coarse health summary for sync surfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncHealth {
    /// Overlay edge insertions currently buffered.
    pub overlay_edges: usize,
    /// Tombstoned base edges.
    pub tombstoned_edges: usize,
    /// Tombstoned nodes.
    pub tombstoned_nodes: usize,
}
