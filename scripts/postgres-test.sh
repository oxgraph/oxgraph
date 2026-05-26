#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

# shellcheck source=scripts/postgres-env.sh
source "${ROOT}/scripts/postgres-env.sh"

echo "Running oxgraph-pgrx extension integration tests (SQL/SPI via cargo pgrx test)"
cargo pgrx test --package oxgraph-pgrx --features pg16
