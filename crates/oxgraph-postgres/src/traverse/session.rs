//! Single-query traversal parameters assembled at the public API boundary.

use alloc::vec::Vec;
use core::num::NonZeroU32;

use super::{
    TraversalDirection, TraverseLimits,
    profile::{TraverseMode, TraverseProfile},
};
use crate::{
    engine::Engine,
    error::{PostgresGraphError, QueryError},
};

/// Query parameters for one BFS (no cross-query caching; borrows engine only in the kernel).
pub(super) struct TraverseSession {
    /// Canonical node bound from engine metadata.
    pub node_count: u32,
    /// Resolved neighbor iteration profile.
    pub profile: TraverseProfile,
    /// Collect or count-only mode.
    pub mode: TraverseMode,
    /// Maximum nodes to return or count (seeds count toward the cap).
    pub result_limit: usize,
    /// Optional hop bound (`None` = unlimited).
    pub max_depth: Option<u32>,
    /// Visible seeds in caller order.
    pub seeds: Vec<u32>,
    /// Whether node tombstones must be checked during expansion.
    pub check_nodes: bool,
}

impl TraverseSession {
    /// Assembles session parameters for one traverse query.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresGraphError::Query`] when every seed is out of bounds.
    ///
    /// # Performance
    ///
    /// This function is `O(s)` for `s` seeds.
    pub(super) fn open(
        engine: &Engine,
        seeds: &[u32],
        limits: TraverseLimits,
        direction: TraversalDirection,
        mode: TraverseMode,
    ) -> Result<Self, PostgresGraphError> {
        let node_count = engine.node_count();
        if seeds.is_empty() {
            return Ok(Self {
                node_count,
                profile: TraverseProfile::resolve(
                    direction,
                    engine.config().query_freshness,
                    engine.overlay(),
                ),
                mode,
                result_limit: limits.result_limit.get(),
                max_depth: limits.max_depth.map(NonZeroU32::get),
                seeds: Vec::new(),
                check_nodes: engine.overlay().has_node_tombstones(),
            });
        }
        for seed in seeds {
            if *seed >= node_count {
                return Err(QueryError::SeedOutOfBounds {
                    seed: *seed,
                    node_count,
                }
                .into());
            }
        }
        let freshness = engine.config().query_freshness;
        let profile = TraverseProfile::resolve(direction, freshness, engine.overlay());
        let check_nodes = engine.overlay().has_node_tombstones();
        let visible_seeds = seeds
            .iter()
            .copied()
            .filter(|seed| match direction {
                TraversalDirection::Out => engine.forward().node_visible(*seed, engine.overlay()),
                TraversalDirection::In => engine.inbound().node_visible(*seed, engine.overlay()),
            })
            .collect();
        Ok(Self {
            node_count,
            profile,
            mode,
            result_limit: limits.result_limit.get(),
            max_depth: limits.max_depth.map(NonZeroU32::get),
            seeds: visible_seeds,
            check_nodes,
        })
    }
}
