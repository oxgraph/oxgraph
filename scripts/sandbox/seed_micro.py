"""Load the shared 10k micro fixture into a backend."""

from __future__ import annotations

import time
from collections.abc import Callable

from docker_util import docker_psql
from fixture import RawEdge, build_benchmark_fixture


def _insert_edges(execute: Callable[[str], None], edges: list[RawEdge], *, batch: int = 2000) -> None:
    for offset in range(0, len(edges), batch):
        chunk = edges[offset : offset + batch]
        if not chunk:
            continue
        edge_values = ",".join(f"({edge.source},{edge.target})" for edge in chunk)
        execute(f"INSERT INTO micro.edges (start_id, end_id) VALUES {edge_values};")


def _load_micro_tables(execute: Callable[[str], None], fixture) -> float:
    started = time.perf_counter()
    execute(
        """
        DROP SCHEMA IF EXISTS micro CASCADE;
        CREATE SCHEMA micro;
        CREATE TABLE micro.nodes (node_id bigint PRIMARY KEY);
        CREATE TABLE micro.edges (
          edge_id bigserial PRIMARY KEY,
          start_id bigint NOT NULL,
          end_id bigint NOT NULL
        );
        """
    )
    node_values = ",".join(f"({node_id})" for node_id in range(fixture.node_count))
    execute(f"INSERT INTO micro.nodes (node_id) VALUES {node_values};")
    _insert_edges(execute, fixture.raw_edges)
    return time.perf_counter() - started


def seed_pggraph_micro(container: str) -> dict[str, float]:
    """Register and build pgGraph micro schema; returns load/build seconds."""
    fixture = build_benchmark_fixture()

    def execute(sql: str) -> None:
        docker_psql(container, sql, timeout=None)

    execute("SELECT graph.reset();")
    load_seconds = _load_micro_tables(execute, fixture)
    execute(
        """
        SELECT graph.add_table('micro.nodes'::regclass, id_column := 'node_id');
        SELECT graph.add_edge(
          'micro.edges'::regclass,
          'start_id',
          'micro.nodes'::regclass,
          'end_id',
          'micro_edge',
          bidirectional := true
        );
        """
    )
    build_started = time.perf_counter()
    execute("SELECT count(*) FROM graph.build();")
    build_seconds = time.perf_counter() - build_started
    return {"load_seconds": load_seconds, "build_seconds": build_seconds}


def seed_oxgraph_micro(execute: Callable[[str], None]) -> dict[str, float]:
    """Register catalog and build oxgraph micro schema."""
    fixture = build_benchmark_fixture()
    execute("SELECT graph.graph_reset();")
    execute(
        """
        TRUNCATE graph._registered_filter_columns,
                 graph._registered_edges,
                 graph._registered_tables;
        """
    )
    load_seconds = _load_micro_tables(execute, fixture)
    execute(
        """
        INSERT INTO graph._registered_tables (table_id, schema_name, table_name, primary_key_column)
        VALUES (1, 'micro', 'nodes', 'node_id');
        INSERT INTO graph._registered_edges (
          edge_id, source_table_id, target_table_id,
          source_column, target_column, schema_name, table_name
        ) VALUES (1, 1, 1, 'start_id', 'end_id', 'micro', 'edges');
        """
    )
    build_started = time.perf_counter()
    execute("SELECT graph.graph_build(0);")
    build_seconds = time.perf_counter() - build_started
    return {"load_seconds": load_seconds, "build_seconds": build_seconds}


def seed_oxgraph_micro_container(container: str) -> dict[str, float]:
    return seed_oxgraph_micro(
        lambda sql: docker_psql(container, sql, timeout=None),
    )
