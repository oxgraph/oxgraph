# oxgraph

A storage-agnostic, zero-copy-friendly topology access layer for Rust.

The current foundation includes six `no_std` crates: `oxgraph-topology` for discrete topology
identity/read-view capability traits, `oxgraph-graph` for binary graph read-view capability traits,
`oxgraph-hyper` for hypergraph read-view capability traits, `oxgraph-csr` for borrowed CSR graph views,
`oxgraph-algo` for graph algorithms with explicit `no_std`, `alloc`, and `std` traversal tiers, and
`oxgraph-snapshot` for validated byte-level snapshot containers. The core crates intentionally avoid concrete storage,
snapshot, mutation, payload, and domain semantics.

`oxgraph-graph` is anchored to `oxgraph-topology`: graph nodes are topology elements and graph edges are
topology relations. It provides graph-facing aliases and traversal capabilities over those IDs,
including direct-neighbor capabilities for layouts that can avoid edge-ID materialization. Graph-specific
traversal remains the hot path for ordinary graph users. Incidence IDs and roles are optional
capabilities, not requirements for graph-only views. `oxgraph-hyper` exposes the sibling directed
vertex-expansion primitives, but concrete hypergraph layouts and algorithms remain intentionally deferred.

Every workspace crate should include executable examples under `examples/` that show how the crate is
used. The core examples demonstrate topology, graph, and hypergraph vocabulary; layout, algorithm,
and snapshot examples demonstrate CSR traversal, BFS, and opening CSR views from neutral snapshot
sections.

See [`docs/architecture.md`](docs/architecture.md) for the crate dependency diagram, layer-by-layer
use cases, and examples of opening graph snapshots from static bytes, files, memory-mapped bytes,
and database row bytes.

## Tooling matrix

| Concern             | Tool                                           | Entry point         |
| ------------------- | ---------------------------------------------- | ------------------- |
| Format (Rust)       | `rustfmt` nightly (auto-grouped imports)       | `just fmt`          |
| Format (TOML)       | `taplo` (alphabetised dep tables)              | `just fmt-toml`     |
| Lint                | `clippy` with strict complexity thresholds     | `just lint`         |
| Supply-chain        | `cargo-deny` (advisories, bans, sources)       | `just deny`         |
| Tests               | `cargo test`                                   | `just test`         |
| UB detection        | `miri` (nightly)                               | `just miri`         |
| Formal verification | `kani`                                         | `just kani`         |
| Benchmarks          | `criterion`                                    | `just bench`        |
| Pre-commit          | `prek`                                         | `just hooks-install`|
| Agent guard rails   | Claude Code `PreToolUse` hook                  | see below           |

`just ci` runs the gating subset (fmt-check → fmt-toml-check → clippy → deny → test).

## Getting started

```sh
# 1. Toolchain — rustup picks up rust-toolchain.toml (1.95.0 stable).
rustup show

# 2. Nightly rustfmt (for auto-import-grouping) and miri.
rustup toolchain install nightly --component rustfmt,miri

# 3. Dev tools.
cargo install prek cargo-deny taplo-cli
cargo install --locked kani-verifier && cargo kani setup

# 4. Git hooks.
just hooks-install

# 5. First run.
just ci
```

`.claude/hooks/block-bypass.sh` denies agent-side bypass attempts
(`--no-verify`, clippy `-A`, `--cap-lints`, `RUSTFLAGS=…-A`, `SKIP=`,
hooks-path overrides). Wire it into your local (gitignored) settings:

```sh
cat > .claude/settings.local.json <<'JSON'
{
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [
        { "type": "command",
          "command": "${CLAUDE_PROJECT_DIR}/.claude/hooks/block-bypass.sh" } ] }
    ]
  }
}
JSON
```

## Strictness

All clippy lint groups (`all`, `pedantic`, `nursery`, `cargo`) deny by
default. `#[allow(...)]` is itself denied — silencing a lint requires
`#[expect(..., reason = "...")]` (which errors if the lint no longer fires)
or an explicit reasoned entry in the workspace `[lints.clippy]` table. Every
public **and** private item must be documented; `missing_docs` and
`clippy::missing_docs_in_private_items` both deny.

`Cargo.lock` is committed so miri and kani runs stay reproducible.
