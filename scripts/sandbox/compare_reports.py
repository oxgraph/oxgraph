#!/usr/bin/env python3
"""Compare sandbox benchmark reports (micro SQL tier)."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

SANDBOX_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SANDBOX_DIR))

from micro_workloads import COMPARABLE_KEYS  # noqa: E402


def median_hot(report: dict[str, object], workload: str) -> float | None:
    summary = report.get("summary", {})
    key = f"hot:{workload}"
    entry = summary.get(key)
    if not entry:
        return None
    return float(entry["median_ms"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pggraph_report", type=Path)
    parser.add_argument("oxgraph_report", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    pg = json.loads(args.pggraph_report.read_text(encoding="utf-8"))
    ox = json.loads(args.oxgraph_report.read_text(encoding="utf-8"))

    lines: list[str] = []
    lines.append("=== Sandbox SQL tier (micro, hot median wall_ms) ===")
    lines.append(
        f"{'Workload':<24} {'pgGraph ms':>12} {'oxgraph ms':>12} {'ox/pg':>10}"
    )

    for key, (pg_name, ox_name) in COMPARABLE_KEYS.items():
        pg_ms = median_hot(pg, pg_name)
        ox_ms = median_hot(ox, ox_name)
        if pg_ms is None and ox_ms is None:
            continue
        if pg_ms is None or pg_ms == 0:
            ratio = "n/a"
        elif ox_ms is None:
            ratio = "ox n/a"
        else:
            ratio = f"{ox_ms / pg_ms:.2f}x"
        lines.append(
            f"{key:<24} {pg_ms or 0:>12.1f} {ox_ms or 0:>12.1f} {ratio:>10}"
        )

    pg_prep = pg.get("prepared", {})
    ox_prep = ox.get("prepared", {})
    lines.append("")
    lines.append("=== Prepared (load + build, seconds) ===")
    if isinstance(pg_prep.get("load_seconds"), (int, float)):
        lines.append(
            f"pgGraph load={pg_prep['load_seconds']:.2f} "
            f"build={pg_prep.get('build_seconds', 0):.2f}"
        )
    if isinstance(ox_prep.get("load_seconds"), (int, float)):
        lines.append(
            f"oxgraph load={ox_prep['load_seconds']:.2f} "
            f"build={ox_prep.get('build_seconds', 0):.2f}"
        )

    lines.append("")
    lines.append(
        "Panama/LDBC: run pgGraph sandbox/run_benchmarks.sh; oxgraph lacks text-node "
        "catalog ingest — compare engine tier for 10k synthetic only."
    )
    lines.append(
        "Extension tier (cargo pgrx bench): see target/postgres-bench/extension-bench.txt"
    )

    text = "\n".join(lines) + "\n"
    print(text)
    args.out.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
