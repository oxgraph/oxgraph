#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

# shellcheck source=scripts/postgres-env.sh
source "${ROOT}/scripts/postgres-env.sh"

echo "Initializing pgrx Postgres 16 (download + build) — one-time setup"
cargo pgrx init --pg16 download

if [[ -x "${PGRX_HOME}/16.14/pgrx-install/bin/pg_config" ]]; then
  cat >"${PGRX_HOME}/config.toml" <<EOF
[configs]
pg16 = "${PGRX_HOME}/16.14/pgrx-install/bin/pg_config"
EOF
  echo "Wrote ${PGRX_HOME}/config.toml pointing at pgrx-managed pg16"
fi
