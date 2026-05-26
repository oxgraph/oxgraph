"""Comparable SQL workloads for the 10k micro sandbox dataset."""

from __future__ import annotations

from timing import WorkloadQuery


def pggraph_micro_workloads(supernode: int) -> list[WorkloadQuery]:
    """pgGraph SQL workloads on `micro` schema (bigint node ids as text seeds)."""
    seed = str(supernode)
    status_sql = f"""
WITH warm AS MATERIALIZED (
  SELECT count(*) AS rows_seen
  FROM graph.traverse('micro.nodes'::regclass, '{seed}', 1, hydrate := false, max_rows := 1)
)
SELECT s.*
FROM warm, graph.status() s
"""
    traverse_d1 = f"""
SELECT *
FROM graph.traverse('micro.nodes'::regclass, '{seed}', 1, hydrate := false, max_rows := 500)
"""
    traverse_d2 = f"""
SELECT *
FROM graph.traverse('micro.nodes'::regclass, '{seed}', 2, hydrate := false, max_rows := 500)
"""
    return [
        WorkloadQuery(
            "status",
            "Is the micro benchmark graph loaded, and how large is it?",
            status_sql,
        ),
        WorkloadQuery(
            "traverse_depth_1",
            "One-hop neighborhood around the fixture supernode.",
            traverse_d1,
        ),
        WorkloadQuery(
            "traverse_depth_2",
            "Two-hop neighborhood around the fixture supernode.",
            traverse_d2,
        ),
    ]


def oxgraph_micro_workloads(supernode: int) -> list[WorkloadQuery]:
    """oxgraph SQL workloads (dense node ids, same fixture supernode)."""
    status_sql = f"""
WITH warm AS MATERIALIZED (
  SELECT length(graph.graph_traverse({supernode}, 1, 'out', 1)::text) AS _
)
SELECT graph.graph_status() AS status
FROM warm
"""
    traverse_d1 = f"""
SELECT node_id
FROM unnest(graph.graph_traverse({supernode}, 500, 'out', 1)) AS node_id
"""
    traverse_d2 = f"""
SELECT node_id
FROM unnest(graph.graph_traverse({supernode}, 500, 'out', 2)) AS node_id
"""
    search_sql = f"""
SELECT node_id
FROM unnest(graph.graph_search({supernode}, {supernode}, 25)) AS node_id
"""
    return [
        WorkloadQuery(
            "status",
            "Is the micro benchmark graph loaded, and how large is it?",
            status_sql,
        ),
        WorkloadQuery(
            "traverse_depth_1",
            "One-hop neighborhood around the fixture supernode.",
            traverse_d1,
        ),
        WorkloadQuery(
            "traverse_depth_2",
            "Two-hop neighborhood around the fixture supernode.",
            traverse_d2,
        ),
        WorkloadQuery(
            "node_search",
            "Dense node-id search at the supernode (oxgraph-only API).",
            search_sql,
        ),
    ]


# Cross-backend keys for comparison (hot:median_ms).
COMPARABLE_KEYS: dict[str, tuple[str, str]] = {
    "status": ("status", "status"),
    "traverse_d1": ("traverse_depth_1", "traverse_depth_1"),
    "traverse_d2": ("traverse_depth_2", "traverse_depth_2"),
}
