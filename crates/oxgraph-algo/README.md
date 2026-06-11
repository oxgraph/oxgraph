# oxgraph-algo

Substrate-agnostic graph algorithms over oxgraph-topology traits.

[![crates.io](https://img.shields.io/crates/v/oxgraph-algo.svg)](https://crates.io/crates/oxgraph-algo)
[![docs.rs](https://docs.rs/oxgraph-algo/badge.svg)](https://docs.rs/oxgraph-algo)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The algorithms layer of the [oxgraph](https://github.com/oxgraph/oxgraph)
crate family. `no_std`, `unsafe`-free, allocation-free by default.

## What it is

`oxgraph-algo` contains algorithms whose semantics generalize across
ordinary binary graphs and hypergraphs. BFS binds only to capability traits
from `oxgraph-topology`; PageRank additionally uses graph and hypergraph
vocabulary traits, while still avoiding concrete storage and Arrow/property
dependencies. The same compiled algorithm runs on `oxgraph_csr::CsrGraph`,
`oxgraph_hyper_bcsr::BcsrHypergraph`, and any future view that exposes the
required capabilities.

Every traversal ships in two directions: forward entry points expand through
`ElementSuccessors`, reverse entry points through `ElementPredecessors`, and
both reuse the same scratch storage. The `bfs` and `pagerank` modules are
always available; `components`, `longest_path_dag`, `scc`, `sssp`, and
`toposort` sit behind the `alloc` feature.

## Where it sits

```text
oxgraph-topology / -graph / -hyper   capability traits
└── oxgraph-algo                   ← this crate (BFS, PageRank, …)
        runs over oxgraph-csr, oxgraph-csc, oxgraph-hyper-bcsr,
        or any view implementing the required traits
```

## Allocation tiers

| Feature | You get | Cost model |
| --- | --- | --- |
| none (default) | `*_with_scratch` / `*_with_epoch_scratch` entry points; the caller provides reusable scratch storage | `O(n + m)` traversal, no allocation |
| `alloc` | Owning convenience wrappers (`breadth_first_search`, workspaces) and `BTreeSet`-backed generic variants | `O(n + m)` (`O((n + m) log n)` generic) |
| `std` | `HashSet`-backed generic variants | expected `O(n + m)` |

## Example

Adapted from the runnable example
[`examples/bfs.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-algo/examples/bfs.rs)
(`cargo run -p oxgraph-algo --example bfs`): scratch-backed BFS over a
borrowed CSR graph, with no allocation.

```rust
use oxgraph_algo::breadth_first_search_with_scratch;
use oxgraph_csr::{CsrNativeGraph, CsrNodeId};

static OFFSETS: &[u32] = &[0, 2, 3, 4, 4];
static TARGETS: &[u32] = &[1, 2, 2, 3];

let graph = CsrNativeGraph::<u32, u32>::validate(4, OFFSETS, TARGETS)?;
let mut visited = [0; 4];
let mut queue = [CsrNodeId::new(0); 4];

let start = CsrNodeId::new(0);
for node in breadth_first_search_with_scratch(&graph, start, &mut visited, &mut queue)? {
    println!("bfs_node={node:?}");
}
```

## Documentation

See [docs.rs/oxgraph-algo](https://docs.rs/oxgraph-algo) for the full API,
including the per-tier performance contracts on every entry point, and the
[oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for how
the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features algo` (or `algo-alloc` / `algo-std`).

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
