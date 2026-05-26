#!/usr/bin/env bash
set -euo pipefail

# Tier 0 — engine-only Criterion (no SQL/SPI/pgrx). Compares oxgraph-postgres to pgGraph.
# Fixture: seed 42, 10k nodes, avg degree 3 (shared bench_fixture module).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${BENCH_OUT:-${ROOT}/target/postgres-bench}"
PGGRAPH_DIR="${PGGRAPH_DIR:-/tmp/pgGraph}"
# shellcheck source=scripts/postgres-env.sh
source "${ROOT}/scripts/postgres-env.sh"

mkdir -p "${OUT}"

if [[ ! -d "${PGGRAPH_DIR}/.git" ]]; then
  git clone --depth 1 "https://github.com/Evokoa/pgGraph.git" "${PGGRAPH_DIR}"
fi

cd "${ROOT}"

echo "=== oxgraph-postgres (engine compare_pggraph) ===" | tee "${OUT}/oxgraph-engine.txt"
cargo bench -p oxgraph-postgres --features bench-fixture --bench compare_pggraph -- --noplot 2>&1 \
  | tee -a "${OUT}/oxgraph-engine.txt"

echo "=== pgGraph graph crate (bfs_bench + graph_construction, pg16, 10k) ===" | tee "${OUT}/pggraph-engine.txt"
cd "${PGGRAPH_DIR}/graph"
if [[ ! -f "${PGRX_HOME}/config.toml" ]]; then
  echo "Run: just postgres-init  (or cargo pgrx init --pg16 download)" >&2
  exit 1
fi
cargo bench -p graph --no-default-features --features pg16 --bench bfs_bench -- "10k" --noplot 2>&1 \
  | tee "${OUT}/pggraph-engine.txt" || {
  echo "pgGraph bench failed; retry with:" >&2
  echo "  cd ${PGGRAPH_DIR}/graph && cargo bench -p graph --no-default-features --features pg16 --bench bfs_bench -- 10k" >&2
  exit 1
}

python3 - "${OUT}" <<'PY'
import re
import sys
from pathlib import Path

out_dir = Path(sys.argv[1])


def parse_times(path: Path) -> dict[str, float]:
    text = path.read_text()
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


ox = parse_times(out_dir / "oxgraph-engine.txt")
pg = parse_times(out_dir / "pggraph-engine.txt")

pairs = [
    (
        "CSR/index build (10k)",
        "engine_graph_construction/build/10k",
        "graph_construction/build/10k",
    ),
    (
        "Engine open (10k)",
        "engine_oxgraph_engine_open/10k",
        None,
    ),
    (
        "BFS depth-1 supernode (10k)",
        "engine_bfs_traverse/d1_supernode/10k",
        "bfs_traverse/d1_supernode/10k",
    ),
    (
        "BFS depth-3 supernode (10k)",
        "engine_bfs_traverse/d3_supernode/10k",
        "bfs_traverse/d3_supernode/10k",
    ),
]

print("\n=== Engine tier comparison (median µs; ox/pg < 1 means oxgraph faster) ===")
print(f"{'Workload':<32} {'oxgraph µs':>12} {'pgGraph µs':>12} {'ox/pg':>10}")
rows: list[str] = []
for label, ox_key, pg_key in pairs:
    ox_us = ox.get(ox_key)
    pg_us = pg.get(pg_key) if pg_key else None
    if ox_us is None and pg_us is None:
        continue
    if pg_us is None:
        ratio = "ox only"
        pg_display = "—"
    elif ox_us is None:
        ratio = "n/a"
        pg_display = f"{pg_us:.1f}"
    else:
        ratio = f"{ox_us / pg_us:.2f}x"
        pg_display = f"{pg_us:.1f}"
    line = f"{label:<32} {ox_us or 0:>12.1f} {pg_display:>12} {ratio:>10}"
    print(line)
    rows.append(line)

notes = """
Engine tier notes:
- Fixture: pgGraph graph_gen (seed 42, power-law raw edges, bidirectional pairs).
- BFS pairing: oxgraph `traverse_core_out` (collect node ids) vs pgGraph `bfs_execute` (visited/parent/depth).
  Count and collect share one kernel; no separate count-only bench lane.
- pgGraph bench reallocates `BfsConfig` each iteration; oxgraph reuses the open engine.
- Not measured: SQL, SPI, CREATE EXTENSION, catalog scans, sync triggers, GUC paths, libpq client.
"""
print(notes)

summary = out_dir / "comparison-engine.txt"
summary.write_text("\n".join(rows) + notes + "\n\nRaw: oxgraph-engine.txt, pggraph-engine.txt\n")
print(f"\nRaw logs: {out_dir}/oxgraph-engine.txt, {out_dir}/pggraph-engine.txt")
PY
