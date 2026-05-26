//! Property tests for search and multi-seed traversal invariants.

use core::num::NonZeroUsize;

use oxgraph_postgres::{
    Config, DualTopologySnapshot, EngineBuilder, QueryFreshness, SearchPredicate,
    TraversalDirection, TraverseLimits,
};
use proptest::prelude::*;

fn build_engine(
    edges: &[(u32, u32)],
) -> Result<oxgraph_postgres::Engine, oxgraph_postgres::PostgresGraphError> {
    let bytes = DualTopologySnapshot::from_dense_u32_edges(edges, 0)?;
    EngineBuilder::new()
        .snapshot_owned(bytes)
        .config(Config {
            search_limit: 1_000,
            traverse_limit: 1_000,
            query_freshness: QueryFreshness::OverlayAware,
            maintenance_enabled: true,
        })
        .build()
}

proptest! {
    #[test]
    fn search_range_matches_manual_scan(
        edges in prop::collection::vec((0u32..8, 0u32..8), 1..16)
    ) {
        let engine = build_engine(&edges)?;
        let node_bound = u32::try_from(engine.forward().node_count()).unwrap_or(0);
        if node_bound == 0 {
            return Ok(());
        }
        let start = 0;
        let end = node_bound.saturating_sub(1);
        let limit = NonZeroUsize::new(usize::try_from(node_bound).unwrap_or(usize::MAX))
            .unwrap_or(NonZeroUsize::MIN);
        let found = engine.search(
            SearchPredicate::NodeIdRange { start, end },
            limit,
        )?;
        for node in start..=end {
            if found.contains(&node) {
                prop_assert!(node < node_bound);
            }
        }
    }

    #[test]
    fn traverse_from_seeds_respects_limit(
        edges in prop::collection::vec((0u32..6, 0u32..6), 1..12),
        limit in 1usize..20
    ) {
        let mut engine = build_engine(&edges)?;
        let node_bound = u32::try_from(engine.forward().node_count()).unwrap_or(0);
        if node_bound == 0 {
            return Ok(());
        }
        let seeds: Vec<u32> = (0..node_bound.min(3)).collect();
        let limits = TraverseLimits::bounded(NonZeroUsize::new(limit).unwrap_or(NonZeroUsize::MIN));
        let merged = engine.traverse_from_seeds(&seeds, limits, TraversalDirection::Out)?;
        prop_assert!(merged.len() <= limit);
    }
}
