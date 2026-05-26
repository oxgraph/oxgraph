//! Tests that engine config GUC mirrors cap search and traverse output.

use core::num::{NonZeroU32, NonZeroUsize};

use oxgraph_postgres::{
    Config, DualTopologySnapshot, EngineBuilder, EngineStatusReport, SearchPredicate,
    TraversalDirection, TraverseLimits,
};

const LIMIT_100: NonZeroUsize = NonZeroUsize::new(100).unwrap();

fn build_chain_engine(
    config: Config,
) -> Result<oxgraph_postgres::Engine, oxgraph_postgres::PostgresGraphError> {
    let bytes = DualTopologySnapshot::from_dense_u32_edges(&[(0, 1), (1, 2), (2, 3)], 0)?;
    EngineBuilder::new()
        .snapshot_owned(bytes)
        .config(config)
        .build()
}

#[test]
fn search_limit_guc_caps_results() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let engine = build_chain_engine(Config {
        search_limit: 2,
        ..Config::default()
    })?;
    let matches = engine.search(SearchPredicate::NodeIdRange { start: 0, end: 3 }, LIMIT_100)?;
    assert_eq!(matches, vec![0, 1]);
    Ok(())
}

#[test]
fn traverse_limit_guc_caps_output() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_chain_engine(Config {
        traverse_limit: 2,
        ..Config::default()
    })?;
    let limits = TraverseLimits {
        result_limit: LIMIT_100,
        max_depth: None,
    };
    let nodes = engine.traverse(0, limits, TraversalDirection::Out)?;
    assert_eq!(nodes.len(), 2);
    Ok(())
}

#[test]
fn status_report_reflects_loaded_engine() -> Result<(), oxgraph_postgres::PostgresGraphError> {
    let engine = build_chain_engine(Config::default())?;
    let report = EngineStatusReport::from_engine(&engine);
    assert!(report.loaded);
    assert_eq!(report.node_count, 4);
    assert_eq!(report.edge_count, 3);
    assert_eq!(report.overlay_edge_count, 0);
    let json = report.to_json();
    assert!(json.contains("\"loaded\":true"));
    assert!(json.contains("\"node_count\":4"));
    assert!(json.contains("\"edge_count\":3"));
    Ok(())
}

#[test]
fn traverse_depth_limit_still_applies_with_high_guc_cap()
-> Result<(), oxgraph_postgres::PostgresGraphError> {
    let mut engine = build_chain_engine(Config::default())?;
    let limits = TraverseLimits {
        result_limit: LIMIT_100,
        max_depth: NonZeroU32::new(1),
    };
    let nodes = engine.traverse(0, limits, TraversalDirection::Out)?;
    assert_eq!(nodes, vec![0, 1]);
    Ok(())
}
