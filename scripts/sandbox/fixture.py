"""10k benchmark fixture aligned with oxgraph-postgres bench_fixture/graph.rs."""

from __future__ import annotations

from dataclasses import dataclass

BENCH_SEED = 42
BENCH_NODE_COUNT = 10_000
BENCH_AVG_DEGREE = 3


@dataclass(frozen=True)
class RawEdge:
    source: int
    target: int


@dataclass(frozen=True)
class GeneratedBenchmarkGraph:
    node_count: int
    raw_edges: list[RawEdge]


class Rng:
    def __init__(self, seed: int) -> None:
        self._state = 1 if seed == 0 else seed

    def next_u64(self) -> int:
        x = self._state
        x ^= (x << 13) & ((1 << 64) - 1)
        x ^= x >> 7
        x ^= (x << 17) & ((1 << 64) - 1)
        self._state = x
        return x

    def next_u32(self) -> int:
        return (self.next_u64() >> 16) & 0xFFFF_FFFF

    def next_bounded(self, n: int) -> int:
        return (self.next_u32() * n) >> 32


def build_benchmark_fixture(
    node_count: int = BENCH_NODE_COUNT,
    avg_degree: int = BENCH_AVG_DEGREE,
    seed: int = BENCH_SEED,
) -> GeneratedBenchmarkGraph:
    rng = Rng(seed)
    total_edges = node_count * avg_degree
    raw_edges: list[RawEdge] = []
    edge_list = list(range(node_count))

    for _ in range(total_edges):
        source = rng.next_bounded(node_count)
        target_idx = rng.next_u64() % len(edge_list)
        target = edge_list[target_idx]
        if source == target:
            continue
        raw_edges.append(RawEdge(source, target))
        raw_edges.append(RawEdge(target, source))
        edge_list.append(source)
        edge_list.append(target)

    return GeneratedBenchmarkGraph(node_count, raw_edges)


def out_degrees(fixture: GeneratedBenchmarkGraph) -> list[int]:
    degree = [0] * fixture.node_count
    for edge in fixture.raw_edges:
        if edge.source < len(degree):
            degree[edge.source] += 1
    return degree


def find_supernode(fixture: GeneratedBenchmarkGraph) -> int:
    best_idx = 0
    best_degree = 0
    for idx, deg in enumerate(out_degrees(fixture)):
        if deg > best_degree:
            best_degree = deg
            best_idx = idx
    return best_idx
