#!/usr/bin/env python3
"""Run micro sandbox SQL benchmarks for one backend (pgGraph or oxgraph)."""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

SANDBOX_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SANDBOX_DIR))

from docker_util import (  # noqa: E402
    container_host_port,
    ensure_container,
    ensure_graph_extension,
    require_docker,
)
from fixture import build_benchmark_fixture, find_supernode  # noqa: E402
from micro_workloads import COMPARABLE_KEYS, oxgraph_micro_workloads, pggraph_micro_workloads  # noqa: E402
from seed_micro import seed_oxgraph_micro, seed_oxgraph_micro_container, seed_pggraph_micro  # noqa: E402
from timing import run_workloads, summarize  # noqa: E402


def benchmark_methodology_dict() -> dict[str, object]:
    return {
        "measurement_unit": "milliseconds",
        "phases": {
            "build": "Fixture load + graph.build()/graph.graph_build() measured in prepared.load_seconds and prepared.build_seconds.",
            "cold": "Docker container restart before each cold query; excludes build; graph artifact persists on disk.",
            "warmup": "One unrecorded pass per query in a persistent backend.",
            "hot": "Repeated measured iterations after warmup.",
        },
        "note": "Micro dataset: seed 42, 10k nodes, avg degree 3 — aligned with engine tier compare_pggraph.",
    }


def connect_port(port: int, user: str, password: str | None):
    try:
        import psycopg
    except ImportError as exc:
        raise RuntimeError(
            "Install psycopg: pip install -r scripts/sandbox/requirements.txt"
        ) from exc
    kwargs: dict[str, object] = {
        "host": "127.0.0.1",
        "port": port,
        "dbname": "postgres",
        "user": user,
        "autocommit": True,
    }
    if password is not None:
        kwargs["password"] = password
    return psycopg.connect(**kwargs)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", choices=("pggraph", "oxgraph"), required=True)
    parser.add_argument("--container", required=True)
    parser.add_argument("--port", type=int, default=0, help="Host port (0 = discover)")
    parser.add_argument("--image", default="")
    parser.add_argument("--build-context", type=Path, default=None)
    parser.add_argument("--dockerfile", type=Path, default=None)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--hot-iterations", type=int, default=10)
    parser.add_argument(
        "--host-pgrx",
        action="store_true",
        help="Use cargo pgrx-managed Postgres (oxgraph) instead of Docker cold restart",
    )
    parser.add_argument("--user", default="postgres")
    parser.add_argument("--password", default="postgres")
    args = parser.parse_args()

    restart_fn = None
    if args.host_pgrx:
        port = args.port or 28816
        user = args.user
        password = None
    else:
        require_docker()
        if args.build_context and args.dockerfile and args.image:
            ensure_container(
                name=args.container,
                image=args.image,
                host_port=args.port or (55432 if args.backend == "pggraph" else 55433),
                build_context=args.build_context,
                dockerfile=args.dockerfile,
            )
        port = args.port or container_host_port(args.container)
        user = args.user
        password = args.password
        if args.backend == "pggraph":
            ensure_graph_extension(args.container)
    fixture = build_benchmark_fixture()
    supernode = find_supernode(fixture)

    if args.backend == "pggraph":
        prepared = seed_pggraph_micro(args.container)
        queries = pggraph_micro_workloads(supernode)
    else:
        if args.host_pgrx:

            def execute(sql: str) -> None:
                with connect_port(port, user, password) as conn:
                    with conn.cursor() as cur:
                        cur.execute(sql)

            prepared = seed_oxgraph_micro(execute)
        else:
            prepared = seed_oxgraph_micro_container(args.container)
        queries = oxgraph_micro_workloads(supernode)

    if args.host_pgrx:

        def pgrx_restart() -> None:
            import subprocess

            root = Path(__file__).resolve().parents[2]
            subprocess.run(
                ["cargo", "pgrx", "stop", "pg16"],
                cwd=root,
                check=True,
                capture_output=True,
            )
            subprocess.run(
                ["cargo", "pgrx", "start", "pg16"],
                cwd=root,
                check=True,
                capture_output=True,
            )

        restart_fn = pgrx_restart
    else:
        restart_fn = None

    results = run_workloads(
        queries,
        args.container,
        lambda: connect_port(port, user, password),
        hot_iterations=args.hot_iterations,
        restart_fn=restart_fn,
    )

    report = {
        "backend": args.backend,
        "dataset": "micro",
        "captured_at": datetime.now(timezone.utc).isoformat(),
        "prepared": prepared,
        "supernode": supernode,
        "comparable_keys": COMPARABLE_KEYS,
        "methodology": benchmark_methodology_dict(),
        "results": results,
        "summary": summarize(results),
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
