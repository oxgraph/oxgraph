# oxgraph-topology

Storage-agnostic core traits for discrete topology views.

[![crates.io](https://img.shields.io/crates/v/oxgraph-topology.svg)](https://crates.io/crates/oxgraph-topology)
[![docs.rs](https://docs.rs/oxgraph-topology/badge.svg)](https://docs.rs/oxgraph-topology)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The capability-trait foundation of the
[oxgraph](https://github.com/oxgraph/oxgraph) crate family. `no_std`,
`unsafe`-free, no dependencies.

## What it is

`oxgraph-topology` defines the minimal vocabulary shared by the graph,
hypergraph, snapshot, and layout crates: elements, relations, incidences,
roles, counts, and identity. It does not define concrete node, edge, vertex,
hyperedge, incidence, storage, or role types. Implementations provide those
through associated types.

The traits are read-view capabilities. A topology view is any value that
exposes topology through them, and the view decides its own boundary: an
entire snapshot, one layout section, a generated projection, a page-sized
window, or an overlay can all be views if they provide the requested
capabilities. Mutation is out of scope here; it belongs in explicit
capability traits that define identity stability, deletion, compaction, and
stale-handle semantics.

## Where it sits

Everything in the family builds on this crate:

```text
oxgraph-topology                ← this crate
├── oxgraph-graph                 binary graph vocabulary (nodes, edges)
├── oxgraph-hyper                 hypergraph vocabulary (vertices, hyperedges)
└── oxgraph-algo                  BFS and PageRank bound to these traits
```

Concrete layouts (`oxgraph-csr`, `oxgraph-csc`, `oxgraph-hyper-bcsr`)
implement the traits; algorithms consume them. The same compiled BFS runs
over an in-memory layout, a memory-mapped file, or rows read from a database.

## Example

Expose your own storage as a topology by implementing the capability traits
it can support. IDs are compact `Copy` handles; any `Copy + Eq + Ord + Debug
+ Hash` type qualifies through a blanket impl:

```rust
use oxgraph_topology::{TopologyBase, TopologyCounts};

/// Three nodes and two directed edges, stored however you like.
struct TinyGraph;

impl TopologyBase for TinyGraph {
    type ElementId = u32;  // nodes are topology elements
    type RelationId = u32; // edges are topology relations
}

impl TopologyCounts for TinyGraph {
    fn element_count(&self) -> usize {
        3
    }

    fn relation_count(&self) -> usize {
        2
    }
}
```

Complete implementations, including incidence and role capabilities, are
runnable examples:
[`examples/directed_graph.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-topology/examples/directed_graph.rs)
and
[`examples/hypergraph.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-topology/examples/hypergraph.rs),
run with `cargo run -p oxgraph-topology --example directed_graph`.

## Documentation

See [docs.rs/oxgraph-topology](https://docs.rs/oxgraph-topology) for the full
API and the
[oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for how
the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features topology`.

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
