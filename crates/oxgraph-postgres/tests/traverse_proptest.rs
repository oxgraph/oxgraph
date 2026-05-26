//! Property tests: production BFS matches reference cardinality and discovery order.

use core::num::{NonZeroU32, NonZeroUsize};
use std::{collections::VecDeque, mem, vec::Vec};

use oxgraph_postgres::{
    Config, DualTopologySnapshot, EngineBuilder, QueryFreshness, TraversalDirection, TraverseLimits,
};
use proptest::prelude::*;

const LIMIT_MAX: NonZeroUsize = NonZeroUsize::new(256).unwrap();

fn build_engine(
    node_count: usize,
    edges: &[(u32, u32)],
) -> Result<oxgraph_postgres::Engine, oxgraph_postgres::PostgresGraphError> {
    let node_count_u32 = u32::try_from(node_count).map_err(|_| {
        oxgraph_postgres::PostgresGraphError::Build(oxgraph_postgres::BuildError::NodeCountOverflow)
    })?;
    let bytes =
        DualTopologySnapshot::from_dense_u32_edges_with_node_count(node_count_u32, edges, 0)?;
    EngineBuilder::new().snapshot_owned(bytes).build()
}

fn reference_adjacency(node_count: usize, edges: &[(u32, u32)]) -> Vec<Vec<u32>> {
    let mut adjacency = Vec::with_capacity(node_count);
    adjacency.resize(node_count, Vec::new());
    for &(source, target) in edges {
        let source_usize = source as usize;
        let target_usize = target as usize;
        if source_usize < node_count && target_usize < node_count {
            adjacency[source_usize].push(target);
        }
    }
    adjacency
}

/// Node-unique rows with sorted targets (matches `UniqueAdjacency` / `BaseOnly` kernel order).
fn reference_adjacency_unique(node_count: usize, edges: &[(u32, u32)]) -> Vec<Vec<u32>> {
    let mut adjacency = reference_adjacency(node_count, edges);
    for row in &mut adjacency {
        row.sort_unstable();
        row.dedup();
    }
    adjacency
}

fn reference_visited_count(
    node_count: usize,
    edges: &[(u32, u32)],
    seed: u32,
    max_depth: u32,
) -> usize {
    let adjacency = reference_adjacency(node_count, edges);
    let seed_usize = seed as usize;
    if seed_usize >= node_count {
        return 0;
    }

    let mut visited = vec![false; node_count];
    let mut queue = VecDeque::new();
    let mut next_queue = VecDeque::new();
    visited[seed_usize] = true;
    queue.push_back(seed);
    let mut count = 1_usize;
    let mut wave_depth = 0_u32;

    while !queue.is_empty() {
        if wave_depth >= max_depth {
            break;
        }
        let mut wave = ReferenceWave {
            adjacency: &adjacency,
            visited: &mut visited,
            wave_depth,
            max_depth,
            next_queue: &mut next_queue,
            count: &mut count,
            order: None,
            result_limit: usize::MAX,
        };
        for &node in &queue {
            wave.discover_neighbors(node);
        }
        wave_depth = wave_depth.saturating_add(1);
        queue.clear();
        mem::swap(&mut queue, &mut next_queue);
    }
    count
}

fn reference_bfs_collect(
    node_count: usize,
    edges: &[(u32, u32)],
    seed: u32,
    max_depth: u32,
    result_limit: usize,
) -> Vec<u32> {
    let adjacency = reference_adjacency_unique(node_count, edges);
    let seed_usize = seed as usize;
    if seed_usize >= node_count {
        return Vec::new();
    }

    let mut visited = vec![false; node_count];
    let mut queue = VecDeque::new();
    let mut next_queue = VecDeque::new();
    visited[seed_usize] = true;
    queue.push_back(seed);
    let mut order = vec![seed];
    let mut count = 1_usize;
    let mut wave_depth = 0_u32;

    while !queue.is_empty() {
        if wave_depth >= max_depth || count >= result_limit {
            break;
        }
        let mut wave = ReferenceWave {
            adjacency: &adjacency,
            visited: &mut visited,
            wave_depth,
            max_depth,
            next_queue: &mut next_queue,
            count: &mut count,
            order: Some(&mut order),
            result_limit,
        };
        for &node in &queue {
            if wave.at_result_limit() {
                break;
            }
            wave.discover_neighbors(node);
        }
        wave_depth = wave_depth.saturating_add(1);
        queue.clear();
        mem::swap(&mut queue, &mut next_queue);
    }
    order
}

struct ReferenceWave<'a> {
    adjacency: &'a [Vec<u32>],
    visited: &'a mut [bool],
    wave_depth: u32,
    max_depth: u32,
    next_queue: &'a mut VecDeque<u32>,
    count: &'a mut usize,
    order: Option<&'a mut Vec<u32>>,
    result_limit: usize,
}

impl ReferenceWave<'_> {
    const fn at_result_limit(&self) -> bool {
        *self.count >= self.result_limit
    }

    fn discover_neighbors(&mut self, node: u32) {
        for &neighbor in &self.adjacency[node as usize] {
            if self.at_result_limit() {
                return;
            }
            let next_usize = neighbor as usize;
            if self.visited[next_usize] {
                continue;
            }
            self.visited[next_usize] = true;
            *self.count += 1;
            if let Some(order) = self.order.as_mut() {
                order.push(neighbor);
            }
            let next_depth = self.wave_depth.saturating_add(1);
            if next_depth < self.max_depth {
                self.next_queue.push_back(neighbor);
            }
        }
    }
}

proptest! {
    #[test]
    fn visited_count_matches_reference(
        node_count in 2_usize..16,
        edge_pairs in prop::collection::vec((0_u32..16, 0_u32..16), 0..48),
        seed in 0_u32..16,
        max_depth in 1_u32..8,
    ) {
        let edges: Vec<(u32, u32)> = edge_pairs
            .into_iter()
            .filter(|&(source, target)| {
                (source as usize) < node_count && (target as usize) < node_count
            })
            .collect();
        let mut engine = build_engine(node_count, &edges)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        engine
            .set_config(Config {
                query_freshness: QueryFreshness::BaseOnly,
                ..Config::default()
            })
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let limits = TraverseLimits {
            result_limit: LIMIT_MAX,
            max_depth: NonZeroU32::new(max_depth),
        };
        if seed as usize >= node_count {
            return Ok(());
        }
        let production = engine.visited_count(seed, limits, TraversalDirection::Out)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let reference = reference_visited_count(node_count, &edges, seed, max_depth);
        prop_assert_eq!(production, reference);
    }

    #[test]
    fn collect_matches_reference_order_and_count(
        node_count in 2_usize..16,
        edge_pairs in prop::collection::vec((0_u32..16, 0_u32..16), 0..48),
        seed in 0_u32..16,
        max_depth in 1_u32..8,
        result_limit in 1_usize..64,
    ) {
        let edges: Vec<(u32, u32)> = edge_pairs
            .into_iter()
            .filter(|&(source, target)| {
                (source as usize) < node_count && (target as usize) < node_count
            })
            .collect();
        let mut engine = build_engine(node_count, &edges)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        engine
            .set_config(Config {
                query_freshness: QueryFreshness::BaseOnly,
                ..Config::default()
            })
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let result_limit = NonZeroUsize::new(result_limit).unwrap_or(LIMIT_MAX);
        let limits = TraverseLimits {
            result_limit,
            max_depth: NonZeroU32::new(max_depth),
        };
        if seed as usize >= node_count {
            return Ok(());
        }
        let production = engine
            .traverse(seed, limits, TraversalDirection::Out)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let reference =
            reference_bfs_collect(node_count, &edges, seed, max_depth, result_limit.get());
        prop_assert_eq!(&production, &reference);
        let production_len = production.len();
        let count = engine.visited_count(seed, limits, TraversalDirection::Out)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        if result_limit.get() >= reference_visited_count(node_count, &edges, seed, max_depth) {
            prop_assert_eq!(production_len, count);
        }
    }

    #[test]
    fn inbound_visited_count_matches_reference(
        node_count in 2_usize..16,
        edge_pairs in prop::collection::vec((0_u32..16, 0_u32..16), 0..48),
        seed in 0_u32..16,
        max_depth in 1_u32..8,
    ) {
        let edges: Vec<(u32, u32)> = edge_pairs
            .into_iter()
            .filter(|&(source, target)| {
                (source as usize) < node_count && (target as usize) < node_count
            })
            .collect();
        let mut engine = build_engine(node_count, &edges)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        engine
            .set_config(Config {
                query_freshness: QueryFreshness::BaseOnly,
                ..Config::default()
            })
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let limits = TraverseLimits {
            result_limit: LIMIT_MAX,
            max_depth: NonZeroU32::new(max_depth),
        };
        if seed as usize >= node_count {
            return Ok(());
        }
        let production = engine.visited_count(seed, limits, TraversalDirection::In)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let reference = reference_inbound_visited_count(node_count, &edges, seed, max_depth);
        prop_assert_eq!(production, reference);
    }
}

fn reference_inbound_adjacency(node_count: usize, edges: &[(u32, u32)]) -> Vec<Vec<u32>> {
    let mut adjacency = Vec::with_capacity(node_count);
    adjacency.resize(node_count, Vec::new());
    for &(source, target) in edges {
        let source_usize = source as usize;
        let target_usize = target as usize;
        if source_usize < node_count && target_usize < node_count {
            adjacency[target_usize].push(source);
        }
    }
    adjacency
}

fn reference_inbound_visited_count(
    node_count: usize,
    edges: &[(u32, u32)],
    seed: u32,
    max_depth: u32,
) -> usize {
    let adjacency = reference_inbound_adjacency(node_count, edges);
    let seed_usize = seed as usize;
    if seed_usize >= node_count {
        return 0;
    }

    let mut visited = vec![false; node_count];
    let mut queue = VecDeque::new();
    let mut next_queue = VecDeque::new();
    visited[seed_usize] = true;
    queue.push_back(seed);
    let mut count = 1_usize;
    let mut wave_depth = 0_u32;

    while !queue.is_empty() {
        if wave_depth >= max_depth {
            break;
        }
        let mut wave = ReferenceWave {
            adjacency: &adjacency,
            visited: &mut visited,
            wave_depth,
            max_depth,
            next_queue: &mut next_queue,
            count: &mut count,
            order: None,
            result_limit: usize::MAX,
        };
        for &node in &queue {
            wave.discover_neighbors(node);
        }
        wave_depth = wave_depth.saturating_add(1);
        queue.clear();
        mem::swap(&mut queue, &mut next_queue);
    }
    count
}
