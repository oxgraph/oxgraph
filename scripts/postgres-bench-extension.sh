#!/usr/bin/env bash
set -euo pipefail

# Tier 1 — in-Postgres extension benches via `cargo pgrx bench` (SQL/SPI/pgrx paths).
# Measures Spi `SELECT graph.*`, catalog build, sync reload, and direct-call overhead baselines.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${BENCH_OUT:-${ROOT}/target/postgres-bench}"
# shellcheck source=scripts/postgres-env.sh
source "${ROOT}/scripts/postgres-env.sh"

mkdir -p "${OUT}"

cd "${ROOT}"

echo "=== oxgraph-pgrx (cargo pgrx bench, pg16) ===" | tee "${OUT}/extension-bench.txt"
cargo pgrx bench --package oxgraph-pgrx --features pg16 2>&1 | tee -a "${OUT}/extension-bench.txt"

echo ""
echo "Extension bench history is stored in the oxgraph_pgrx_benches database (pgrx_bench schema)."
echo "Re-run with: cargo pgrx bench --report"
echo "Raw log: ${OUT}/extension-bench.txt"
