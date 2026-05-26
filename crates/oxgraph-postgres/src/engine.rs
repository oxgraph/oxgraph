//! Active graph engine: snapshot backing, dual topology, overlays, and config.

use alloc::{boxed::Box, vec::Vec};
use core::{
    cell::{Cell, RefCell},
    num::NonZeroUsize,
};

use yoke::Yoke;

use crate::{
    artifact::PostgresMetadata,
    build::EdgeRow,
    builder::EngineBuilder,
    catalog::Catalog,
    config::Config,
    error::{ConfigError, PostgresGraphError, QueryError},
    overlay::OverlayState,
    rebuild::SnapshotRebuild,
    search::SearchPredicate,
    sync::{SyncHealth, SyncRow},
    topology::{GraphTopology, TopologyHot, UniqueAdjacency},
    traverse::{TraversalDirection, TraverseLimits, traverse_core_collect, traverse_core_count},
};

/// Runtime status returned by admin/discovery surfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineStatus {
    /// Node count recorded in Postgres metadata.
    pub node_count: u32,
    /// Edge count recorded in Postgres metadata.
    pub edge_count: u32,
    /// Whether the artifact is marked read-only.
    pub read_only: bool,
    /// Number of overlay edge insertions not yet compacted.
    pub overlay_edge_count: usize,
    /// Number of tombstoned base edges.
    pub tombstoned_edges: usize,
}

/// Yoke cart owning snapshot bytes and parsed metadata.
#[expect(
    clippy::redundant_pub_crate,
    reason = "shared with private builder module"
)]
pub(crate) struct EngineCart {
    /// Owned OXGTOPO bytes backing both topology views.
    pub backing: Vec<u8>,
    /// Parsed Postgres metadata section.
    pub metadata: PostgresMetadata,
}

/// Topology views borrowing the cart backing (lifetime-erased in [`Engine`]).
#[derive(yoke::Yokeable)]
#[yoke(prove_covariant)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "shared with private builder module"
)]
pub(crate) struct EngineState<'a> {
    /// Forward CSR and inbound CSC opened once at build.
    pub topology: GraphTopology<'a>,
}

/// Active [`OxGraph`] backend state loaded from OXGTOPO bytes.
pub struct Engine {
    /// Yoke-attached topology views and owned snapshot cart.
    inner: Yoke<EngineState<'static>, Box<EngineCart>>,
    /// Overlay buffers applied on top of the base artifact.
    overlay: OverlayState,
    /// Operational config mirrored from extension GUCs.
    config: Config,
    /// Reused BFS scratch (epoch marks, dual frontier waves, collect buffer).
    traverse_scratch: crate::traverse::TraverseScratch,
    /// Node-unique adjacency (placeholder until first traverse builds it).
    unique_adjacency: RefCell<UniqueAdjacency>,
    /// Whether [`Self::unique_adjacency`] has been populated from topology.
    unique_cache_built: Cell<bool>,
}

impl Engine {
    /// Constructs an engine from validated yoke state and runtime buffers.
    #[expect(clippy::missing_const_for_fn, reason = "Yoke attachment is not const")]
    #[expect(
        clippy::too_many_arguments,
        reason = "constructs the validated Engine field set without intermediate mutation"
    )]
    pub(crate) fn from_parts(
        inner: Yoke<EngineState<'static>, Box<EngineCart>>,
        overlay: OverlayState,
        config: Config,
        traverse_scratch: crate::traverse::TraverseScratch,
        unique_adjacency: RefCell<UniqueAdjacency>,
        unique_cache_built: Cell<bool>,
    ) -> Self {
        Self {
            inner,
            overlay,
            config,
            traverse_scratch,
            unique_adjacency,
            unique_cache_built,
        }
    }

    /// Returns the canonical node count from artifact metadata.
    #[must_use]
    pub fn node_count(&self) -> u32 {
        self.inner.backing_cart().metadata.node_count.get()
    }

    /// Disjoint hot topology, unique cache, overlay, and scratch for one BFS query.
    ///
    /// Unique adjacency is built lazily on the first traverse (`O(n + m log d)`); engine open
    /// stays `O(n + m)` for topology attach only.
    pub(crate) fn traverse_workspace_mut(
        &mut self,
    ) -> (
        TopologyHot<'_>,
        core::cell::Ref<'_, UniqueAdjacency>,
        &OverlayState,
        &mut crate::traverse::TraverseScratch,
    ) {
        if !self.unique_cache_built.get() {
            let forward = self.inner.get().topology.forward;
            let inbound = self.inner.get().topology.inbound;
            *self.unique_adjacency.borrow_mut() =
                UniqueAdjacency::from_topology(&forward, &inbound);
            self.unique_cache_built.set(true);
        }
        let unique = self.unique_adjacency.borrow();
        let topology = &self.inner.get().topology;
        let hot = TopologyHot::from_topology(topology);
        (hot, unique, &self.overlay, &mut self.traverse_scratch)
    }

    /// Loads an engine via [`EngineBuilder`].
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError`] when snapshot validation or topology attach fails.
    pub fn from_snapshot_bytes(bytes: &[u8]) -> Result<Self, PostgresGraphError> {
        EngineBuilder::new().snapshot_owned(bytes.to_vec()).build()
    }

    /// Returns operational status for admin surfaces.
    #[must_use]
    pub fn status(&self) -> EngineStatus {
        let metadata = &self.inner.backing_cart().metadata;
        EngineStatus {
            node_count: metadata.node_count.get(),
            edge_count: metadata.edge_count.get(),
            read_only: metadata.is_read_only(),
            overlay_edge_count: self.overlay.added_edges.len(),
            tombstoned_edges: self.overlay.tombstoned_edges.len(),
        }
    }

    /// Returns the active configuration mirror.
    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Updates configuration after validation.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Config`] when limits or freshness settings are invalid.
    pub fn set_config(&mut self, config: Config) -> Result<(), PostgresGraphError> {
        config.validate()?;
        self.config = config;
        Ok(())
    }

    /// Borrows overlay state for read-only query visibility checks.
    #[must_use]
    pub const fn overlay(&self) -> &OverlayState {
        &self.overlay
    }

    /// Borrows the overlay buffer for sync replay.
    pub const fn overlay_mut(&mut self) -> &mut OverlayState {
        &mut self.overlay
    }

    /// Returns immutable snapshot bytes backing the engine.
    #[must_use]
    pub fn snapshot_bytes(&self) -> &[u8] {
        self.inner.backing_cart().backing.as_slice()
    }

    /// Returns both topology views opened at engine build.
    #[must_use]
    pub fn topology(&self) -> &GraphTopology<'_> {
        &self.inner.get().topology
    }

    /// Returns the forward CSR view (outgoing adjacency only).
    #[must_use]
    pub fn forward(&self) -> &crate::topology::ForwardCsr<'_> {
        &self.topology().forward
    }

    /// Returns the inbound CSC view (incoming adjacency only).
    #[must_use]
    pub fn inbound(&self) -> &crate::topology::InboundCsc<'_> {
        &self.topology().inbound
    }

    /// Replaces artifact bytes after maintenance rebuild.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError`] when the replacement snapshot fails validation.
    pub fn replace_snapshot_bytes(&mut self, bytes: &[u8]) -> Result<(), PostgresGraphError> {
        let engine = EngineBuilder::new()
            .snapshot_owned(bytes.to_vec())
            .config(self.config.clone())
            .overlay(OverlayState::default())
            .build()?;
        self.inner = engine.inner;
        self.overlay.clear();
        *self.unique_adjacency.borrow_mut() = UniqueAdjacency::default();
        self.unique_cache_built.set(false);
        self.traverse_scratch
            .reset_after_snapshot(self.node_count() as usize);
        Ok(())
    }

    /// Breadth-first traversal from one seed node.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Query`] when the seed is out of bounds or limits are zero.
    ///
    /// # Performance
    ///
    /// This method is `O(r + e)` where `r` is nodes discovered up to `result_limit` and `e` is
    /// edges examined along the chosen profile; `≤ 1ms` for `n ≤ 10k` on typical chain fixtures.
    pub fn traverse(
        &mut self,
        seed: u32,
        limits: TraverseLimits,
        direction: TraversalDirection,
    ) -> Result<Vec<u32>, PostgresGraphError> {
        let limits = limits.capped_by(self.config())?;
        traverse_core_collect(self, &[seed], limits, direction)
    }

    /// Breadth-first traversal from multiple seed nodes in one kernel run.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Query`] when every seed is out of bounds or limits are zero.
    ///
    /// # Performance
    ///
    /// Same as [`Self::traverse`] with multiple seeds; one kernel run.
    pub fn traverse_from_seeds(
        &mut self,
        seeds: &[u32],
        limits: TraverseLimits,
        direction: TraversalDirection,
    ) -> Result<Vec<u32>, PostgresGraphError> {
        let limits = limits.capped_by(self.config())?;
        traverse_core_collect(self, seeds, limits, direction)
    }

    /// Returns visited-node count for one seed without collecting ids.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Query`] when the seed is out of bounds or limits are zero.
    ///
    /// # Performance
    ///
    /// This method is `O(r + e)` without output allocation; matches collect cardinality.
    pub fn visited_count(
        &mut self,
        seed: u32,
        limits: TraverseLimits,
        direction: TraversalDirection,
    ) -> Result<usize, PostgresGraphError> {
        let limits = limits.capped_by(self.config())?;
        traverse_core_count(self, &[seed], limits, direction)
    }

    /// Searches dense node ids using a simple predicate.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Query`] when the effective limit is zero.
    ///
    /// # Performance
    ///
    /// This method is `O(n)` for `n` canonical nodes until the effective limit is reached.
    pub fn search(
        &self,
        predicate: SearchPredicate,
        limit: NonZeroUsize,
    ) -> Result<Vec<u32>, PostgresGraphError> {
        let effective_limit = core::cmp::min(limit.get(), self.config().search_limit as usize);
        let effective_limit = NonZeroUsize::new(effective_limit).ok_or(QueryError::LimitZero)?;
        let node_bound = self.forward().node_count();
        let mut matches = Vec::new();
        for node in 0..node_bound {
            let node_u32 = u32::try_from(node).map_err(|_| QueryError::NodeIndexOverflow)?;
            if !self.node_visible(node_u32) || !predicate.matches(node_u32) {
                continue;
            }
            matches.push(node_u32);
            if matches.len() >= effective_limit.get() {
                break;
            }
        }
        Ok(matches)
    }

    /// Applies sync rows to the overlay in sequence order.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Sync`] when row sequence numbers are not monotonic.
    pub fn apply_sync_rows(&mut self, rows: &[SyncRow]) -> Result<usize, PostgresGraphError> {
        SyncRow::apply_in_order(rows, self.overlay_mut())
    }

    /// Returns sync overlay health for admin surfaces.
    #[must_use]
    pub fn sync_health(&self) -> SyncHealth {
        let status = self.status();
        SyncHealth {
            overlay_edges: status.overlay_edge_count,
            tombstoned_edges: status.tombstoned_edges,
            tombstoned_nodes: self.overlay().tombstoned_nodes.len(),
        }
    }

    /// Rebuilds the base artifact from catalog metadata and freshly scanned edge rows.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Config`] when maintenance is disabled, or build/validation
    /// errors from catalog planning and snapshot encoding.
    pub fn rebuild_from_catalog(
        &mut self,
        catalog: &Catalog,
        edges: &[EdgeRow],
        built_at_unix: u64,
    ) -> Result<(), PostgresGraphError> {
        if !self.config().maintenance_enabled {
            return Err(ConfigError::MaintenanceDisabled.into());
        }
        let bytes = SnapshotRebuild::from_catalog_and_edges(catalog, edges, built_at_unix)?;
        self.replace_snapshot_bytes(&bytes)
    }

    /// Returns whether `node` is visible under the active freshness policy.
    fn node_visible(&self, node: u32) -> bool {
        self.inner
            .get()
            .topology
            .node_visible(node, TraversalDirection::Out, self.overlay())
    }
}
