//! Query surface tests for search and multi-seed traversal.

use core::num::NonZeroUsize;

const LIMIT_10: NonZeroUsize = NonZeroUsize::new(10).unwrap();

use oxgraph_postgres::{
    Config, DualTopologySnapshot, EngineBuilder, QueryFreshness, SearchPredicate,
    TraversalDirection, TraverseLimits,
};

fn build_chain_engine(
    edges: &[(u32, u32)],
) -> Result<oxgraph_postgres::Engine, oxgraph_postgres::PostgresGraphError> {
    let bytes = DualTopologySnapshot::from_dense_u32_edges(edges, 0)?;
    EngineBuilder::new()
        .snapshot_owned(bytes)
        .config(Config {
            search_limit: 100,
            traverse_limit: 100,
            query_freshness: QueryFreshness::OverlayAware,
            maintenance_enabled: true,
        })
        .build()
}

#[test]
fn search_node_id_and_range() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let engine = build_chain_engine(&[(0, 1), (1, 2)])?;
    let limit = LIMIT_10;
    assert_eq!(engine.search(SearchPredicate::NodeId(1), limit)?, vec![1]);
    assert_eq!(
        engine.search(SearchPredicate::NodeIdRange { start: 0, end: 2 }, limit)?,
        vec![0, 1, 2]
    );
    Ok(())
}

#[test]
fn traverse_from_seeds_merges_distinct() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_chain_engine(&[(0, 1), (2, 3)])?;
    let limits = TraverseLimits::bounded(LIMIT_10);
    let merged = engine.traverse_from_seeds(&[0, 2], limits, TraversalDirection::Out)?;
    assert!(merged.contains(&0));
    assert!(merged.contains(&1));
    assert!(merged.contains(&2));
    assert!(merged.contains(&3));
    Ok(())
}
