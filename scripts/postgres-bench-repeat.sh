#!/usr/bin/env bash
set -euo pipefail

# Run all postgres benchmark tiers N times and aggregate means vs pgGraph.
#
# Env:
#   BENCH_RUNS=8          number of full repetitions (default 8)
#   SKIP_EXTENSION=1      skip cargo pgrx bench tier
#   SKIP_SANDBOX=1        skip sandbox SQL tier
#   PGGRAPH_DIR=/tmp/pgGraph

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNS="${BENCH_RUNS:-8}"
REPEATS="${ROOT}/target/postgres-bench/repeats"
# shellcheck source=scripts/postgres-env.sh
source "${ROOT}/scripts/postgres-env.sh"

mkdir -p "${REPEATS}"

echo "=== postgres-bench-repeat: ${RUNS} runs ==="
echo "Output: ${REPEATS}/run-*"

for i in $(seq 1 "${RUNS}"); do
  run_dir="${REPEATS}/run-$(printf '%02d' "${i}")"
  mkdir -p "${run_dir}/sandbox"
  echo ""
  echo ">>> Run ${i}/${RUNS} -> ${run_dir}"

  BENCH_OUT="${run_dir}" "${ROOT}/scripts/postgres-bench-engine.sh" \
    2>&1 | tee "${run_dir}/engine.log"

  if [[ "${SKIP_EXTENSION:-}" != "1" ]]; then
    BENCH_OUT="${run_dir}" "${ROOT}/scripts/postgres-bench-extension.sh" \
      2>&1 | tee "${run_dir}/extension.log"
  fi

  if [[ "${SKIP_SANDBOX:-}" != "1" ]]; then
    BENCH_OUT="${run_dir}" SANDBOX_OUT="${run_dir}/sandbox" \
      "${ROOT}/scripts/postgres-bench-sandbox.sh" \
      2>&1 | tee "${run_dir}/sandbox.log"
  fi
done

python3 "${ROOT}/scripts/postgres-bench-aggregate.py" \
  "${REPEATS}" \
  "${ROOT}/target/postgres-bench/comparison-all-runs.txt"

echo ""
echo "Done. Summary: ${ROOT}/target/postgres-bench/comparison-all-runs.txt"
