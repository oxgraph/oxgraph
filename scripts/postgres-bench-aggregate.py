#!/usr/bin/env python3
"""Aggregate repeated postgres benchmark runs into a pgGraph comparison report."""

from __future__ import annotations

import json
import re
import statistics
import sys
from pathlib import Path


def parse_criterion_us(path: Path) -> dict[str, float]:
    if not path.exists():
        return {}
    text = path.read_text(encoding="utf-8", errors="replace")
    results: dict[str, float] = {}
    current: str | None = None
    for line in text.splitlines():
        if line.startswith("Benchmarking "):
            current = line.split()[1].rstrip(":")
        m = re.search(r"time:\s+\[[^\]]+\s+([\d.]+)\s+([µu]s|ms|ns|s)\s+", line)
        if m and current:
            val = float(m.group(1))
            unit = m.group(2)
            if unit == "ms":
                val *= 1000.0
            elif unit in ("µs", "us"):
                pass
            elif unit == "ns":
                val /= 1000.0
            elif unit == "s":
                val *= 1_000_000.0
            results[current] = val
    return results


def parse_pgrx_bench_us(path: Path) -> dict[str, float]:
    if not path.exists():
        return {}
    text = path.read_text(encoding="utf-8", errors="replace")
    results: dict[str, float] = {}
    current: str | None = None
    for line in text.splitlines():
        if line.startswith("bench_") or "bench_" in line:
            m_name = re.search(r"\x1b\[1m(bench_[^\x1b]+)\x1b", line)
            if m_name:
                current = m_name.group(1)
        m = re.search(r"time:\s+\[([\d.]+)\s+us\s+([\d.]+)\s+us", line)
        if not m:
            m = re.search(r"time:\s+\[([\d.]+)\s+us\s+([\d.]+)\s+us\s+([\d.]+)\s+us\]", line)
        if m and current:
            # median is middle value in criterion pgrx output
            if m.lastindex and m.lastindex >= 3:
                results[current] = float(m.group(2))
            else:
                results[current] = float(m.group(1))
    return results


def parse_pgrx_bench_us_fallback(path: Path) -> dict[str, float]:
    """Parse pgrx bench lines like: time:   [48.858 us 49.174 us 49.506 us] (stored as µs)."""
    if not path.exists():
        return {}
    text = path.read_text(encoding="utf-8", errors="replace")
    results: dict[str, float] = {}
    current: str | None = None
    for line in text.splitlines():
        stripped = re.sub(r"\x1b\[[0-9;]*m", "", line)
        if stripped.startswith("bench_") and "time:" not in stripped:
            current = stripped.split()[0]
        for unit, scale in (("us", 1.0), ("ms", 1000.0)):
            m = re.search(
                rf"time:\s+\[([\d.]+)\s+{unit}\s+([\d.]+)\s+{unit}\s+([\d.]+)\s+{unit}\]",
                stripped,
            )
            if m and current:
                results[current] = float(m.group(2)) * scale
                break
    return results


def sandbox_hot_ms(path: Path) -> dict[str, float]:
    if not path.exists():
        return {}
    data = json.loads(path.read_text(encoding="utf-8"))
    out: dict[str, float] = {}
    for key, entry in data.get("summary", {}).items():
        if key.startswith("hot:"):
            out[key.removeprefix("hot:")] = float(entry["median_ms"])
    return out


def stats(values: list[float]) -> dict[str, float]:
    if not values:
        return {}
    return {
        "n": float(len(values)),
        "mean": statistics.mean(values),
        "stdev": statistics.stdev(values) if len(values) > 1 else 0.0,
        "min": min(values),
        "max": max(values),
    }


def fmt_us(v: float | None) -> str:
    if v is None:
        return "—"
    if v >= 1000:
        return f"{v / 1000:.2f} ms"
    return f"{v:.1f} µs"


def fmt_ms(v: float | None) -> str:
    if v is None:
        return "—"
    return f"{v:.2f} ms"


class Row:
    def __init__(
        self,
        tier: str,
        label: str,
        ox_key: str | None,
        pg_key: str | None,
        *,
        unit: str = "us",
        pg_na: str = "",
    ) -> None:
        self.tier = tier
        self.label = label
        self.ox_key = ox_key
        self.pg_key = pg_key
        self.unit = unit
        self.pg_na = pg_na


ENGINE_ROWS = [
    Row("0-engine", "CSR/index build (10k)", "engine_graph_construction/build/10k", "graph_construction/build/10k"),
    Row("0-engine", "Engine open (10k)", "engine_oxgraph_engine_open/10k", None),
    Row(
        "0-engine",
        "BFS d1 supernode collect (10k)",
        "engine_bfs_traverse/d1_supernode/10k",
        "bfs_traverse/d1_supernode/10k",
    ),
    Row(
        "0-engine",
        "BFS d3 supernode collect (10k)",
        "engine_bfs_traverse/d3_supernode/10k",
        "bfs_traverse/d3_supernode/10k",
    ),
]

EXTENSION_ROWS = [
    Row("1-extension", "SQL traverse d1 (10k)", "bench_sql_traverse_d1_10k", None, pg_na="no #[pg_bench]"),
    Row("1-extension", "SQL traverse d3 (10k)", "bench_sql_traverse_d3_10k", None, pg_na="no #[pg_bench]"),
    Row("1-extension", "Direct traverse d1 (10k)", "bench_direct_traverse_d1_10k", None, pg_na="no #[pg_bench]"),
    Row("1-extension", "SQL graph build (10k)", "bench_sql_graph_build_10k", None, pg_na="no #[pg_bench]"),
    Row("1-extension", "SPI catalog scan (10k)", "bench_spi_catalog_scan", None, pg_na="no #[pg_bench]"),
    Row("1-extension", "SQL sync reload", "bench_sql_sync_reload", None, pg_na="no #[pg_bench]"),
]

SANDBOX_ROWS = [
    Row("2-sandbox", "SQL status (micro)", "status", "status", unit="ms"),
    Row("2-sandbox", "SQL traverse d1 (micro)", "traverse_depth_1", "traverse_depth_1", unit="ms"),
    Row("2-sandbox", "SQL traverse d2 (micro)", "traverse_depth_2", "traverse_depth_2", unit="ms"),
]


def collect_runs(repeats_dir: Path) -> list[Path]:
    runs = sorted(repeats_dir.glob("run-*"))
    return [p for p in runs if p.is_dir()]


def main() -> int:
    repeats_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("target/postgres-bench/repeats")
    out_path = (
        Path(sys.argv[2])
        if len(sys.argv) > 2
        else Path("target/postgres-bench/comparison-all-runs.txt")
    )
    runs = collect_runs(repeats_dir)
    if not runs:
        print(f"No runs under {repeats_dir}", file=sys.stderr)
        return 1

    lines: list[str] = []
    lines.append(f"Postgres benchmark comparison vs pgGraph ({len(runs)} runs)")
    lines.append(f"Runs: {', '.join(p.name for p in runs)}")
    lines.append("")

    all_rows = ENGINE_ROWS + EXTENSION_ROWS + SANDBOX_ROWS
    current_tier = ""

    for row in all_rows:
        if row.tier != current_tier:
            current_tier = row.tier
            lines.append(f"## Tier {row.tier}")
            lines.append(
                f"{'Workload':<36} {'ox mean±σ':>18} {'pg mean±σ':>18} {'ox/pg':>8}  notes"
            )
            lines.append("-" * 95)

        ox_samples: list[float] = []
        pg_samples: list[float] = []

        for run_dir in runs:
            if row.tier.startswith("0"):
                ox = parse_criterion_us(run_dir / "oxgraph-engine.txt")
                pg = parse_criterion_us(run_dir / "pggraph-engine.txt")
            elif row.tier.startswith("1"):
                ox = parse_pgrx_bench_us_fallback(run_dir / "extension-bench.txt")
                if not ox:
                    ox = parse_pgrx_bench_us(run_dir / "extension-bench.txt")
                pg = {}
            else:
                ox = sandbox_hot_ms(run_dir / "sandbox" / "oxgraph-micro.json")
                pg = sandbox_hot_ms(run_dir / "sandbox" / "pggraph-micro.json")

            if row.ox_key and row.ox_key in ox:
                ox_samples.append(ox[row.ox_key])
            if row.pg_key and row.pg_key in pg:
                pg_samples.append(pg[row.pg_key])

        ox_s = stats(ox_samples)
        pg_s = stats(pg_samples)

        if row.unit == "ms":
            ox_disp = (
                f"{ox_s['mean']:.2f}±{ox_s['stdev']:.2f} ms"
                if ox_s
                else "—"
            )
            pg_disp = (
                f"{pg_s['mean']:.2f}±{pg_s['stdev']:.2f} ms"
                if pg_s
                else (row.pg_na or "—")
            )
            ratio = (
                f"{ox_s['mean'] / pg_s['mean']:.2f}x"
                if ox_s and pg_s and pg_s["mean"] > 0
                else ("ox only" if not pg_s else "n/a")
            )
        else:
            def fmt_ox_stats(s: dict[str, float]) -> str:
                if not s:
                    return "—"
                mean = s["mean"]
                stdev = s["stdev"]
                if mean >= 1000:
                    return f"{mean / 1000:.2f}±{stdev / 1000:.2f} ms"
                return f"{mean:.1f}±{stdev:.1f} µs"

            ox_disp = fmt_ox_stats(ox_s)
            pg_disp = (
                fmt_ox_stats(pg_s)
                if pg_s
                else (row.pg_na or "—")
            )
            ratio = (
                f"{ox_s['mean'] / pg_s['mean']:.2f}x"
                if ox_s and pg_s and pg_s["mean"] > 0
                else ("ox only" if not pg_s else "n/a")
            )

        note = row.pg_na if not pg_s and row.pg_na else ""
        lines.append(
            f"{row.label:<36} {ox_disp:>18} {pg_disp:>18} {ratio:>8}  {note}"
        )

    lines.append("")
    lines.append("Notes:")
    lines.append("- ox/pg < 1.0 means oxgraph faster (lower latency).")
    lines.append("- Engine: oxgraph traverse_core_out vs pgGraph bfs_execute; 10k seed-42 fixture.")
    lines.append("- Extension: oxgraph cargo pgrx bench only; pgGraph has no in-process bench tier.")
    lines.append("- Sandbox: hot-phase wall_ms median per run; micro 10k SQL methodology.")
    lines.append(f"- Raw runs: {repeats_dir}/run-*/")

    text = "\n".join(lines) + "\n"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(text, encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
