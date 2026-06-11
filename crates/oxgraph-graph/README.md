# oxgraph-graph

Storage-agnostic core traits for binary graph views.

[![crates.io](https://img.shields.io/crates/v/oxgraph-graph.svg)](https://crates.io/crates/oxgraph-graph)
[![docs.rs](https://docs.rs/oxgraph-graph/badge.svg)](https://docs.rs/oxgraph-graph)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The binary graph specialization of the
[oxgraph](https://github.com/oxgraph/oxgraph) crate family. `no_std`,
`unsafe`-free.

## What it is

`oxgraph-graph` sits directly above `oxgraph-topology` and renames its
vocabulary for ordinary directed graphs: topology elements become nodes,
topology relations become edges. Use it to write generic graph consumers over
node/edge vocabulary: endpoint lookup, outgoing traversal, incoming
traversal, and degree queries.

Most traits and aliases in this crate are graph-vocabulary shadows of
topology traits, and they inherit their performance contracts from the
traits they shadow: `O(1)` for accessors, `O(1)` to construct an iterator
plus `O(k)` to yield `k` items. Concrete layouts, snapshots, builders,
mutation systems, payloads, and algorithms live in higher-level crates that
implement or consume these read-view capabilities.

## Where it sits

```text
oxgraph-topology                  capability traits
└── oxgraph-graph               ← this crate (node/edge vocabulary)
    ├── oxgraph-csr               borrowed CSR layout (outgoing)
    ├── oxgraph-csc               borrowed CSC layout (inbound)
    └── oxgraph-algo              BFS and PageRank over these traits
```

## Example

Generic code asks for the capabilities it needs and works with any view that
provides them:

```rust
use oxgraph_graph::{EdgeTargetGraph, GraphCounts, OutgoingEdgeCount, OutgoingGraph};

// `graph` is any view implementing the graph capability traits.
println!("nodes={} edges={}", graph.node_count(), graph.edge_count());
println!("out_degree={}", graph.out_degree(node));

for edge in graph.outgoing_edges(node) {
    println!("edge={edge:?} target={:?}", graph.target(edge));
}
```

A complete implementation of these traits for a tiny directed graph is the
runnable example
[`examples/graph_directed.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-graph/examples/graph_directed.rs):
`cargo run -p oxgraph-graph --example graph_directed`.

## Documentation

See [docs.rs/oxgraph-graph](https://docs.rs/oxgraph-graph) for the full API
and the [oxgraph family README](https://github.com/oxgraph/oxgraph#readme)
for how the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features graph`.

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
