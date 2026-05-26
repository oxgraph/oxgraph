#!/usr/bin/env bash
set -euo pipefail

# Postgres verification benches — three tiers:
#   Tier 0 (engine):     scripts/postgres-bench-engine.sh — Criterion, no SQL
#   Tier 1 (extension):  scripts/postgres-bench-extension.sh — cargo pgrx bench
#   Tier 2 (sandbox SQL): scripts/postgres-bench-sandbox.sh — pgGraph methodology vs oxgraph
#
# Not measured: libpq wire protocol, steady-state background worker, Docker image CI.
# One-time setup: brew install pkgconf && just postgres-init

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

"${ROOT}/scripts/postgres-bench-engine.sh"
"${ROOT}/scripts/postgres-bench-extension.sh"
"${ROOT}/scripts/postgres-bench-sandbox.sh"
