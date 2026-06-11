# oxgraph-hyper-bcsr

Directed bipartite-CSR hypergraph views implementing oxgraph-hyper traits.

[![crates.io](https://img.shields.io/crates/v/oxgraph-hyper-bcsr.svg)](https://crates.io/crates/oxgraph-hyper-bcsr)
[![docs.rs](https://docs.rs/oxgraph-hyper-bcsr/badge.svg)](https://docs.rs/oxgraph-hyper-bcsr)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The first concrete hypergraph layout of the
[oxgraph](https://github.com/oxgraph/oxgraph) crate family. `no_std`,
`unsafe`-free.

## What it is

A `BcsrHypergraph` borrows eight validated CSR slices, one offset/value pair
per direction per role, and implements the storage-agnostic hypergraph
traits from `oxgraph-hyper`. Bipartite CSR keeps both directions dense: head
and tail participants are stored under a hyperedge-major index, while
outgoing and incoming incidences are stored under a vertex-major index. This
trades roughly four times the participant storage for `O(degree)` traversal
in either direction, the only access pattern that scales for read-heavy
workloads.

Section payloads are validated at open time. `BcsrValidation::Layout` covers
lengths, offset monotonicity, in-range IDs, and per-range sorted-and-unique
sequences; it is the default for trusted producers. `BcsrValidation::Strict`
additionally checks that the hyperedge-major and vertex-major arrays
describe the same incidence set, which is required for end-to-end semantic
guarantees on untrusted inputs.

## Where it sits

```text
oxgraph-hyper                     vertex/hyperedge capability traits
└── oxgraph-hyper-bcsr          ← this crate (borrowed bipartite-CSR layout)
    ├── reads oxgraph-snapshot sections (width-coded kinds)
    ├── reuses oxgraph-layout-util widths and integrity checks
    └── traversed by oxgraph-algo (BFS, PageRank)
```

## Example

Adapted from the runnable example
[`examples/bcsr_directed.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-hyper-bcsr/examples/bcsr_directed.rs)
(`cargo run -p oxgraph-hyper-bcsr --example bcsr_directed`):

```rust
use oxgraph_hyper::{DirectedHyperedgeParticipants, IncidentHyperedges};
use oxgraph_hyper_bcsr::{BcsrHyperedgeId, BcsrNativeHypergraph, BcsrSections, BcsrVertexId};

// Three vertices, two directed hyperedges:
//   h0: head={v0} tail={v1, v2}
//   h1: head={v1} tail={v2}
let view = BcsrNativeHypergraph::<u32, u32, u32>::open(BcsrSections {
    head_offsets: &[0, 1, 2],
    head_participants: &[0, 1],
    tail_offsets: &[0, 2, 3],
    tail_participants: &[1, 2, 2],
    vertex_outgoing_offsets: &[0, 1, 2, 2],
    vertex_outgoing_hyperedges: &[0, 1],
    vertex_incoming_offsets: &[0, 0, 1, 3],
    vertex_incoming_hyperedges: &[0, 0, 1],
})?;

let h0 = BcsrHyperedgeId::new(0);
let heads: Vec<_> = view.source_participants(h0).collect();
let incident: Vec<_> = view.incident_hyperedges(BcsrVertexId::new(2)).collect();
```

Opening the same view from a packed snapshot is
[`examples/open_bcsr_snapshot.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-hyper-bcsr/examples/open_bcsr_snapshot.rs),
and building one from data is
[`examples/yaml_to_hypergraph.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-hyper-bcsr/examples/yaml_to_hypergraph.rs).

## Features

| Feature | Effect |
| --- | --- |
| none (default) | Borrowed read views only. |
| `build` | Append-only `build::HypergraphBuilder` / `build::WeightedHypergraphBuilder` plus snapshot export helpers. |
| `build-property-arrow` | Property-snapshot export via `oxgraph-property`. |

## Documentation

See [docs.rs/oxgraph-hyper-bcsr](https://docs.rs/oxgraph-hyper-bcsr) for the
full API and the
[oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for how
the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features hyper-bcsr` (or `hyper-build` for the
builders).

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
