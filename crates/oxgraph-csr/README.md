# oxgraph-csr

Borrowed CSR graph views implementing oxgraph-graph traits.

[![crates.io](https://img.shields.io/crates/v/oxgraph-csr.svg)](https://crates.io/crates/oxgraph-csr)
[![docs.rs](https://docs.rs/oxgraph-csr/badge.svg)](https://docs.rs/oxgraph-csr)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The first concrete graph layout of the
[oxgraph](https://github.com/oxgraph/oxgraph) crate family. `no_std`,
`unsafe`-free.

## What it is

A `CsrGraph` borrows validated compressed-sparse-row offset and target
slices, native or straight out of a snapshot section, and implements the
storage-agnostic graph traits from `oxgraph-graph`. Validation happens once
at construction; traversal after that is slice indexing with no parse step
and no copy of the edges.

CSR is optimized for outgoing traversal. Incoming traversal requires a
reverse index and is deliberately not implemented here; that is
`oxgraph-csc`, a separate crate so forward and inbound views cannot be mixed
at the type level.

## Where it sits

```text
oxgraph-graph                     node/edge capability traits
└── oxgraph-csr                 ← this crate (borrowed CSR layout)
    ├── reads oxgraph-snapshot sections (CSR_OFFSETS / CSR_TARGETS)
    ├── reuses oxgraph-layout-util widths and integrity checks
    └── traversed by oxgraph-algo (BFS, PageRank)
```

Snapshot section kinds are width-coded: each persisted kind is
`BASE | WIDTH_CODE`, where the low two bits select the little-endian word
width (`u16`/`u32`/`u64`).

## Example

Adapted from the runnable example
[`examples/csr_directed.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-csr/examples/csr_directed.rs)
(`cargo run -p oxgraph-csr --example csr_directed`):

```rust
use oxgraph_csr::{CsrGraph, CsrNodeId};
use oxgraph_graph::{EdgeTargetGraph, GraphCounts, OutgoingEdgeCount, OutgoingGraph};

static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
static TARGETS: &[u32] = &[1, 2, 2, 3];

let graph = CsrGraph::validate(4, OFFSETS, TARGETS)?;

println!("nodes={} edges={}", graph.node_count(), graph.edge_count());
println!("out_degree(0)={}", graph.out_degree(CsrNodeId::new(0)));

for edge in graph.outgoing_edges(CsrNodeId::new(0)) {
    println!("edge={edge:?} target={:?}", graph.target(edge));
}
```

Opening a view from a packed snapshot instead of native slices is
[`examples/open_snapshot.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-csr/examples/open_snapshot.rs).

## Features

| Feature | Effect |
| --- | --- |
| none (default) | Borrowed read views only. |
| `build` | Append-only `build::GraphBuilder` / `build::WeightedGraphBuilder` plus snapshot export helpers. |
| `build-property-arrow` | Property-snapshot export via `oxgraph-property`. |

## Documentation

See [docs.rs/oxgraph-csr](https://docs.rs/oxgraph-csr) for the full API and
the [oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for
how the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features csr` (or `graph-build` for the builders).

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
