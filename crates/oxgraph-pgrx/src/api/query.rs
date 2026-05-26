//! SQL query entrypoints.

use core::num::{NonZeroU32, NonZeroUsize};

use oxgraph_postgres::{SearchPredicate, TraversalDirection, TraverseLimits};
use pgrx::prelude::*;

use crate::session::{SessionEngine, log_error};

/// Searches dense node ids in the active engine.
#[pg_extern(schema = "graph")]
fn graph_search(
    start: default!(i32, 0),
    end: default!(i32, -1),
    limit: default!(i32, 100),
) -> Vec<i32> {
    let start_u = u32::try_from(start.max(0)).unwrap_or(0);
    let end_u = if end < 0 {
        start_u
    } else {
        u32::try_from(end).unwrap_or(start_u)
    };
    let limit = NonZeroUsize::new(usize::try_from(limit.max(1)).unwrap_or(1));
    let Some(limit) = limit else {
        return Vec::new();
    };
    let predicate = if start_u == end_u {
        SearchPredicate::NodeId(start_u)
    } else {
        SearchPredicate::NodeIdRange {
            start: start_u.min(end_u),
            end: start_u.max(end_u),
        }
    };
    match SessionEngine::try_with(|engine| engine.search(predicate, limit)) {
        Ok(matches) => matches.into_iter().map(|value| value as i32).collect(),
        Err(error) => {
            log_error(&error);
            Vec::new()
        }
    }
}

/// Breadth-first traversal from a seed node id.
#[pg_extern(schema = "graph")]
fn graph_traverse(
    seed: default!(i32, 0),
    limit: default!(i32, 100),
    direction: default!(&str, "'out'"),
    max_depth: default!(i32, -1),
) -> Vec<i32> {
    let direction = match direction {
        "in" => TraversalDirection::In,
        _ => TraversalDirection::Out,
    };
    let seed = if seed < 0 { 0 } else { seed as u32 };
    let Some(result_limit) = NonZeroUsize::new(if limit < 0 { 1 } else { limit as usize }) else {
        return Vec::new();
    };
    let max_depth = if max_depth <= 0 {
        None
    } else {
        NonZeroU32::new(max_depth as u32)
    };
    let limits = TraverseLimits {
        result_limit,
        max_depth,
    };
    match SessionEngine::try_with_mut(|engine| engine.traverse(seed, limits, direction)) {
        Ok(nodes) => nodes.into_iter().map(|value| value as i32).collect(),
        Err(error) => {
            log_error(&error);
            Vec::new()
        }
    }
}

/// Multi-seed traversal from a Postgres integer array.
#[pg_extern(schema = "graph")]
fn graph_traverse_seeds(
    seeds: Vec<i32>,
    limit: default!(i32, 100),
    direction: default!(&str, "'out'"),
) -> Vec<i32> {
    let direction = match direction {
        "in" => TraversalDirection::In,
        _ => TraversalDirection::Out,
    };
    let Some(result_limit) = NonZeroUsize::new(usize::try_from(limit.max(1)).unwrap_or(1)) else {
        return Vec::new();
    };
    let limits = TraverseLimits::bounded(result_limit);
    let seeds: Vec<u32> = seeds
        .into_iter()
        .filter_map(|s| u32::try_from(s.max(0)).ok())
        .collect();
    match SessionEngine::try_with_mut(|engine| {
        engine.traverse_from_seeds(&seeds, limits, direction)
    }) {
        Ok(nodes) => nodes.into_iter().map(|value| value as i32).collect(),
        Err(error) => {
            log_error(&error);
            Vec::new()
        }
    }
}
