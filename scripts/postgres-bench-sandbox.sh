#!/usr/bin/env bash
set -euo pipefail

# Tier 2 — SQL sandbox benchmarks (pgGraph methodology: cold container restart + hot iterations).
#
# Compares:
#   - micro (10k, seed 42): pgGraph Docker vs oxgraph on pgrx-managed Postgres
#   - panama/ldbc (optional): pgGraph upstream sandbox only (oxgraph lacks text-node catalog)
#
# Env:
#   SANDBOX_DATASET=micro|panama|ldbc|all   (default: micro)
#   PGGRAPH_DIR=/tmp/pgGraph
#   SKIP_PGGRAPH=1 | SKIP_OXGRAPH=1
#   OXGRAPH_PGRX_PORT=28816

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${SANDBOX_OUT:-${BENCH_OUT:+${BENCH_OUT}/sandbox}}"
OUT="${OUT:-${ROOT}/target/postgres-bench/sandbox}"
SANDBOX="${ROOT}/scripts/sandbox"
DATASET="${SANDBOX_DATASET:-micro}"
PGGRAPH_DIR="${PGGRAPH_DIR:-/tmp/pgGraph}"

# shellcheck source=scripts/postgres-env.sh
source "${ROOT}/scripts/postgres-env.sh"

mkdir -p "${OUT}"

if ! python3 -c 'import psycopg' 2>/dev/null; then
  python3 -m pip install --user -q -r "${SANDBOX}/requirements.txt"
fi

PGGRAPH_CONTAINER="${PGGRAPH_CONTAINER_NAME:-pggraph-sandbox}"
PGGRAPH_PORT="${PGGRAPH_PG_PORT:-55432}"
PGGRAPH_IMAGE="${PGGRAPH_IMAGE_NAME:-pggraph-postgres:17}"
OXGRAPH_PORT="${OXGRAPH_PGRX_PORT:-28816}"
OXGRAPH_USER="${OXGRAPH_PGRX_USER:-${USER}}"

ensure_pggraph() {
  if [[ ! -d "${PGGRAPH_DIR}/.git" ]]; then
    git clone --depth 1 "https://github.com/Evokoa/pgGraph.git" "${PGGRAPH_DIR}"
  fi
  # shellcheck source=/dev/null
  source "${PGGRAPH_DIR}/sandbox/common/docker.sh"
  require_docker
  ensure_pggraph_image "${PGGRAPH_DIR}" "${PGGRAPH_IMAGE}"
  ensure_pggraph_container "${PGGRAPH_CONTAINER}" "${PGGRAPH_IMAGE}" "${PGGRAPH_PORT}"
}

ensure_oxgraph_pgrx() {
  cd "${ROOT}"
  cargo pgrx start pg16 2>/dev/null || true
  cargo pgrx install --package oxgraph-pgrx --features pg16 \
    --pg-config "${PGRX_HOME}/16.14/pgrx-install/bin/pg_config"
  psql -h 127.0.0.1 -p "${OXGRAPH_PORT}" -U "${OXGRAPH_USER}" -d postgres -v ON_ERROR_STOP=1 \
    -c "CREATE EXTENSION IF NOT EXISTS oxgraph_pgrx;" \
    -c "SELECT graph.graph_reset();" >/dev/null
}

run_pggraph_micro() {
  ensure_pggraph
  python3 "${SANDBOX}/run_micro.py" \
    --backend pggraph \
    --container "${PGGRAPH_CONTAINER}" \
    --port "${PGGRAPH_PORT}" \
    --image "${PGGRAPH_IMAGE}" \
    --out "${OUT}/pggraph-micro.json"
}

run_oxgraph_micro() {
  ensure_oxgraph_pgrx
  python3 "${SANDBOX}/run_micro.py" \
    --backend oxgraph \
    --container host-pgrx \
    --host-pgrx \
    --port "${OXGRAPH_PORT}" \
    --user "${OXGRAPH_USER}" \
    --out "${OUT}/oxgraph-micro.json"
}

run_pggraph_upstream() {
  local ds="$1"
  ensure_pggraph
  if [[ ! -d "${PGGRAPH_DIR}/sandbox/benchmark/.venv" ]]; then
    "${PGGRAPH_DIR}/sandbox/run_benchmarks.sh" "${ds}" --yes 2>/dev/null || \
      "${PGGRAPH_DIR}/sandbox/run_benchmarks.sh" "${ds}"
  else
    "${PGGRAPH_DIR}/sandbox/run_benchmarks.sh" "${ds}" --yes
  fi
  local latest
  latest="$(ls -td "${PGGRAPH_DIR}/sandbox/benchmark/results"/*/report.json 2>/dev/null | head -1)"
  if [[ -n "${latest}" ]]; then
    cp "${latest}" "${OUT}/pggraph-${ds}.json"
    echo "Copied upstream report to ${OUT}/pggraph-${ds}.json"
  fi
}

if [[ "${SKIP_PGGRAPH:-}" != "1" ]]; then
  case "${DATASET}" in
    micro)
      run_pggraph_micro
      ;;
    panama | ldbc)
      run_pggraph_upstream "${DATASET}"
      ;;
    all)
      run_pggraph_micro
      run_pggraph_upstream panama
      run_pggraph_upstream ldbc
      ;;
    *)
      echo "Unknown SANDBOX_DATASET=${DATASET}" >&2
      exit 1
      ;;
  esac
fi

if [[ "${SKIP_OXGRAPH:-}" != "1" ]]; then
  case "${DATASET}" in
    micro | all)
      run_oxgraph_micro
      ;;
    panama | ldbc)
      echo "oxgraph sandbox: skipping ${DATASET} (text-node catalog); use engine tier for 10k synthetic." >&2
      ;;
  esac
fi

if [[ -f "${OUT}/pggraph-micro.json" && -f "${OUT}/oxgraph-micro.json" ]]; then
  python3 "${SANDBOX}/compare_reports.py" \
    "${OUT}/pggraph-micro.json" \
    "${OUT}/oxgraph-micro.json" \
    --out "${OUT}/comparison-sandbox.txt"
fi

echo ""
echo "Sandbox artifacts: ${OUT}/"
