# oxgraph

Storage-agnostic, zero-copy-friendly graph and hypergraph topology substrate for Rust.

**Topology here. Meaning elsewhere. Storage anywhere.**

[![crates.io](https://img.shields.io/crates/v/oxgraph.svg)](https://crates.io/crates/oxgraph)
[![docs.rs](https://docs.rs/oxgraph/badge.svg)](https://docs.rs/oxgraph)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## What it is

oxgraph is a family of small Rust crates that model graph and hypergraph
*topology*, the question of what connects to what, kept separate from what the
data means and where it is stored. The core is `no_std`. Algorithms bind to
capability traits, not to a concrete container, so the same BFS runs over an
in-memory layout, a memory-mapped file, or rows read from a database.

The pipeline it is built around is `bytes → validate → view → traverse`. You point
a view at a byte slice, it validates the layout, and you walk the graph. No
parse step, no heap graph rebuilt from the bytes, no copy of the edges.

## Why it exists

Graph-shaped data shows up everywhere: compilers, databases, knowledge graphs,
provenance, build systems. Most of those systems rebuild large topology into a
heap-owned graph before they can traverse it, and most reinvent node and edge
IDs, adjacency, serialization, and validation while doing it.

oxgraph splits those concerns apart. The foundation defines connectivity and
refuses to interpret it. Layouts decide the byte representation. Properties and
domain meaning live in layers above. One narrow goal drives the design. Open a
large graph from a snapshot, memory-map it, and start traversing without
rebuilding it in memory. See [`vision.md`](vision.md) for the full rationale and
design principles.

## The crate family

Foundation, `no_std`:

| Crate | Gives you |
| --- | --- |
| [`oxgraph-topology`](https://docs.rs/oxgraph-topology) | Capability traits for discrete topology: elements, relations, incidences, roles, identity. The shared vocabulary everything else builds on. |
| [`oxgraph-graph`](https://docs.rs/oxgraph-graph) | Binary graph vocabulary over topology: nodes, edges, directed traversal, neighbors. |
| [`oxgraph-hyper`](https://docs.rs/oxgraph-hyper) | Hypergraph vocabulary over topology: vertices, hyperedges, participants. |
| [`oxgraph-layout-util`](https://docs.rs/oxgraph-layout-util) | Shared index and word types plus offset-integrity checks that concrete layouts reuse. |
| [`oxgraph-snapshot`](https://docs.rs/oxgraph-snapshot) | Topology-agnostic byte container. Validate a snapshot and borrow its sections as typed slices. |

Borrowed layouts:

| Crate | Gives you |
| --- | --- |
| [`oxgraph-csr`](https://docs.rs/oxgraph-csr) | Compressed-sparse-row graph views over offset and target slices, native or from a snapshot. |
| [`oxgraph-csc`](https://docs.rs/oxgraph-csc) | Inbound (compressed-sparse-column) graph views for reverse traversal. |
| [`oxgraph-hyper-bcsr`](https://docs.rs/oxgraph-hyper-bcsr) | Directed bipartite-CSR hypergraph views, dense in both directions. |

Algorithms and properties:

| Crate | Gives you |
| --- | --- |
| [`oxgraph-algo`](https://docs.rs/oxgraph-algo) | BFS and PageRank over the capability traits, with `no_std`, `alloc`, and `std` tiers. |
| [`oxgraph-property`](https://docs.rs/oxgraph-property) | Arrow-backed named, typed property layers and snapshot identity maps. |

Entry point:

| Crate | Gives you |
| --- | --- |
| [`oxgraph`](https://docs.rs/oxgraph) | Curated, feature-gated re-exports of every crate above. Most users depend on this one. |

Built on top: [`oxgraph-db`](https://docs.rs/oxgraph-db) is an embedded
OxGraph-native database with catalogs, queries, and durable identity;
[`oxgraph-postgres`](https://docs.rs/oxgraph-postgres) is a Postgres-backed
engine. `oxgraphd` (server and CLI) and `oxgraph-pgrx` (Postgres extension) ship
in the workspace but are not published to crates.io.

For the dependency diagram, the ID width policy, and the snapshot wire contract,
see [`docs/architecture.md`](docs/architecture.md).

## Using it

Depend on the umbrella crate and turn on the features you need. Its default
feature set is empty, so a feature is the only thing that pulls a layer in:
`cargo add oxgraph --features csr,algo-std`. You can also depend on the
individual crates directly if you want a tighter dependency graph.

Feature names map to layers. The common ones:

| Feature | Use it for |
| --- | --- |
| `topology`, `graph`, `hyper` | The capability traits for graphs and hypergraphs. |
| `csr`, `csc`, `hyper-bcsr` | Borrowed layouts over slices. |
| `snapshot`, `snapshot-alloc` | Reading snapshots, and building them (`-alloc`). |
| `algo`, `algo-alloc`, `algo-std` | BFS and PageRank at the matching allocation tier. |
| `graph-build`, `hyper-build` | Append/freeze builders that produce a layout, then a snapshot. |
| `property-arrow` | Arrow-backed property layers. |
| `db`, `postgres` | The embedded database and the Postgres engine. |
| `full` | Everything. |

## How to use it, by task

Every row points at a runnable example or the API docs. The examples are real
programs you can run with `cargo run --example <name> -p <crate>`.

| You want to | Reach for | Start from |
| --- | --- | --- |
| Expose your own storage as a graph or hypergraph | the `topology` / `graph` / `hyper` traits | [`directed_graph.rs`](crates/oxgraph-topology/examples/directed_graph.rs), [`graph_directed.rs`](crates/oxgraph-graph/examples/graph_directed.rs), [`hyper_directed.rs`](crates/oxgraph-hyper/examples/hyper_directed.rs) |
| Borrow a CSR or bipartite-CSR layout over slices | `oxgraph-csr`, `oxgraph-hyper-bcsr` | [`csr_directed.rs`](crates/oxgraph-csr/examples/csr_directed.rs), [`bcsr_directed.rs`](crates/oxgraph-hyper-bcsr/examples/bcsr_directed.rs) |
| Open a validated snapshot from bytes, an mmap, a file, or DB rows and traverse it without rebuilding | `oxgraph-snapshot` plus a layout | [`open_snapshot.rs`](crates/oxgraph-csr/examples/open_snapshot.rs), [`open_bcsr_snapshot.rs`](crates/oxgraph-hyper-bcsr/examples/open_bcsr_snapshot.rs), [`pack_two_sections.rs`](crates/oxgraph-snapshot/examples/pack_two_sections.rs) |
| Run BFS or PageRank, picking a `no_std`, `alloc`, or `std` tier | `oxgraph-algo` | [`bfs.rs`](crates/oxgraph-algo/examples/bfs.rs), [docs.rs](https://docs.rs/oxgraph-algo) |
| Build a graph, freeze it, and write a snapshot | the `graph-build` / `hyper-build` features | [`yaml_to_hypergraph.rs`](crates/oxgraph-hyper-bcsr/examples/yaml_to_hypergraph.rs), [docs.rs](https://docs.rs/oxgraph-csr) |
| Attach Arrow-backed named properties | `oxgraph-property` | [docs.rs](https://docs.rs/oxgraph-property) |
| Run an embedded database or a Postgres engine | `oxgraph-db`, `oxgraph-postgres` | [docs.rs](https://docs.rs/oxgraph-db) |

## Documentation

- API reference, per crate: [docs.rs/oxgraph](https://docs.rs/oxgraph) and the linked crates above.
- [`docs/architecture.md`](docs/architecture.md): layer graph, width policy, snapshot wire contract.
- [`vision.md`](vision.md): the problem, the thesis, the design principles.
- [`docs/downstream-migration.md`](docs/downstream-migration.md): building a domain system on top.
- [`SECURITY.md`](SECURITY.md): safety model. The substrate is `unsafe`-free.
- Per-crate `examples/` directories: runnable programs for each layer.

## Claude Code plugins

This repository also hosts the **oxgraph Claude Code plugin marketplace**
([`.claude-plugin/marketplace.json`](.claude-plugin/marketplace.json)). Add it
once, then install any oxgraph-family plugin:

```sh
/plugin marketplace add oxgraph/oxgraph
/plugin install oxcode@oxgraph
```

Today it ships the [**oxcode**](https://github.com/oxgraph/oxcode) plugin, which
bundles oxcode's MCP server — PageRank-curated code intelligence for coding
agents. The plugin only adds the MCP wiring: it still needs the `oxcode` binary
on your `PATH` and an indexed project. See the
[oxcode README](https://github.com/oxgraph/oxcode#readme) for the full setup.

## Status

Pre-1.0 and still changing. The traits and crates are not stable yet. The
snapshot byte format is an internal ABI candidate, not a stable interchange
format, so treat persisted snapshots as coupled to the crate version that wrote
them.

## License

MIT. See [LICENSE](LICENSE).
