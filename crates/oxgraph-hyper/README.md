# oxgraph-hyper

Storage-agnostic core traits for hypergraph views.

[![crates.io](https://img.shields.io/crates/v/oxgraph-hyper.svg)](https://crates.io/crates/oxgraph-hyper)
[![docs.rs](https://docs.rs/oxgraph-hyper/badge.svg)](https://docs.rs/oxgraph-hyper)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The hypergraph specialization of the
[oxgraph](https://github.com/oxgraph/oxgraph) crate family. `no_std`,
`unsafe`-free.

## What it is

`oxgraph-hyper` sits directly above `oxgraph-topology` and renames its
vocabulary for hypergraphs: topology elements become vertices, topology
relations become hyperedges, and topology incidences become participant
records. Use it to write generic hypergraph consumers over vertex,
hyperedge, participant, incident-hyperedge, and directed participant-set
vocabulary.

Most traits and aliases in this crate are hypergraph-vocabulary shadows of
topology traits, and they inherit their performance contracts from the
traits they shadow: `O(1)` for accessors, `O(1)` to construct an iterator
plus `O(k)` to yield `k` items. Concrete layouts, snapshots, builders,
mutation systems, payloads, and algorithms live in higher-level crates.

## Where it sits

```text
oxgraph-topology                  capability traits
└── oxgraph-hyper               ← this crate (vertex/hyperedge vocabulary)
    ├── oxgraph-hyper-bcsr        borrowed bipartite-CSR layout
    └── oxgraph-algo              BFS and PageRank over these traits
```

## Example

Generic code asks for the capabilities it needs and works with any view that
provides them:

```rust
use oxgraph_hyper::{HyperedgeParticipants, HypergraphCounts};

// `hypergraph` is any view implementing the hypergraph capability traits.
println!("vertices={}", hypergraph.vertex_count());
println!("hyperedges={}", hypergraph.hyperedge_count());

// A hyperedge connects any number of vertices; each participant record
// carries its vertex, hyperedge, and role.
for participant in hypergraph.hyperedge_participants(hyperedge) {
    println!("participant={participant:?}");
}
```

A complete implementation of these traits for a tiny directed hypergraph is
the runnable example
[`examples/hyper_directed.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-hyper/examples/hyper_directed.rs):
`cargo run -p oxgraph-hyper --example hyper_directed`.

## Documentation

See [docs.rs/oxgraph-hyper](https://docs.rs/oxgraph-hyper) for the full API
and the [oxgraph family README](https://github.com/oxgraph/oxgraph#readme)
for how the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features hyper`.

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
