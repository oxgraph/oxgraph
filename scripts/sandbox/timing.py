"""Shared SQL benchmark timing (pgGraph sandbox methodology)."""

from __future__ import annotations

import hashlib
import json
import subprocess
import time
from dataclasses import dataclass
from statistics import median


@dataclass(frozen=True)
class WorkloadQuery:
    """One measured SQL workload."""

    name: str
    question: str
    sql: str


def query_hash(sql: str) -> str:
    return hashlib.sha256(sql.encode("utf-8")).hexdigest()


def checksum_sql(sql: str) -> str:
    body = sql.rstrip(";")
    return f"""
SELECT
  count(*)::bigint AS row_count,
  md5(coalesce(string_agg(row_hash, '' ORDER BY row_hash), '')) AS result_checksum
FROM (
  SELECT md5(row_to_json(benchmark_query)::text) AS row_hash
  FROM ({body}) AS benchmark_query
) AS benchmark_rows
"""


def server_execution_ms(conn, sql: str) -> float | None:
    explain_sql = f"EXPLAIN (ANALYZE, FORMAT JSON, TIMING ON) {checksum_sql(sql)}"
    with conn.cursor() as cur:
        cur.execute(explain_sql)
        row = cur.fetchone()
    if not row:
        return None
    plan = row[0]
    if isinstance(plan, str):
        plan = json.loads(plan)
    return float(plan[0]["Execution Time"])


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    rank = (len(ordered) - 1) * pct
    lower = int(rank)
    upper = min(lower + 1, len(ordered) - 1)
    weight = rank - lower
    return ordered[lower] * (1 - weight) + ordered[upper] * weight


def run_query(conn, query: WorkloadQuery, phase: str, iteration: int) -> dict[str, object]:
    measured_sql = checksum_sql(query.sql)
    started = time.perf_counter()
    try:
        server_ms = server_execution_ms(conn, query.sql)
        with conn.cursor() as cur:
            cur.execute(measured_sql)
            row_count, checksum = cur.fetchone()
        elapsed = time.perf_counter() - started
        return {
            "name": query.name,
            "question": query.question,
            "sql": query.sql,
            "phase": phase,
            "iteration": iteration,
            "sql_sha256": query_hash(query.sql),
            "wall_ms": round(elapsed * 1000, 3),
            "server_execution_ms": round(server_ms, 3) if server_ms is not None else None,
            "row_count": int(row_count or 0),
            "result_checksum": checksum,
            "ok": True,
        }
    except Exception as exc:  # noqa: BLE001 — benchmark harness records failures
        elapsed = time.perf_counter() - started
        return {
            "name": query.name,
            "question": query.question,
            "sql": query.sql,
            "phase": phase,
            "iteration": iteration,
            "sql_sha256": query_hash(query.sql),
            "wall_ms": round(elapsed * 1000, 3),
            "server_execution_ms": None,
            "ok": False,
            "error": str(exc),
        }


def summarize(results: list[dict[str, object]]) -> dict[str, object]:
    summary: dict[str, object] = {}
    for result in results:
        if not result.get("ok"):
            continue
        key = f"{result['phase']}:{result['name']}"
        summary.setdefault(key, []).append(float(result["wall_ms"]))
    return {
        key: {
            "iterations": len(values),
            "min_ms": min(values),
            "median_ms": median(values),
            "p95_ms": percentile(values, 0.95),
            "max_ms": max(values),
        }
        for key, values in summary.items()
    }


def docker_restart(container: str) -> None:
    subprocess.run(["docker", "restart", container], check=True, capture_output=True)
    deadline = time.time() + 120
    while time.time() < deadline:
        proc = subprocess.run(
            [
                "docker",
                "exec",
                container,
                "pg_isready",
                "-U",
                "postgres",
                "-d",
                "postgres",
            ],
            capture_output=True,
            text=True,
        )
        if proc.returncode == 0:
            return
        time.sleep(1)
    raise RuntimeError(f"Postgres in {container} did not become ready")


def run_workloads(
    queries: list[WorkloadQuery],
    container: str,
    connect_fn,
    *,
    hot_iterations: int = 10,
    restart_fn=None,
) -> list[dict[str, object]]:
    restart = restart_fn or (lambda: docker_restart(container))
    results: list[dict[str, object]] = []
    for query in queries:
        restart()
        with connect_fn() as conn:
            results.append(run_query(conn, query, "cold", 1))

    with connect_fn() as hot_conn:
        for query in queries:
            run_query(hot_conn, query, "warmup", 0)

        for iteration in range(1, hot_iterations + 1):
            for query in queries:
                results.append(run_query(hot_conn, query, "hot", iteration))

    return results
