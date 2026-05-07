# Petgraph architecture research for oxgraph Python/build design

## Sources inspected

- `petgraph` latest docs on docs.rs (`0.8.3`).
- Petgraph master source on GitHub under `crates/petgraph/src/`:
  - `graph_impl/mod.rs`
  - `graph_impl/stable_graph/mod.rs`
  - `graphmap.rs`
  - `csr.rs`
  - `matrix_graph.rs`
  - `data.rs`
- Local oxgraph `vision.md` sections on layouts, builders, mutation, and Python/interop.

## Findings

- Petgraph has multiple concrete graph storage types rather than one universal mutable/frozen representation:
  - `Graph`: adjacency-list graph with arbitrary node/edge weights; mutable and fast for insert/search.
  - `StableGraph`: adjacency-list graph that preserves unrelated indices across removals by storing `Option` weights and free lists.
  - `GraphMap`: map-keyed graph, where node values are the keys; combined adjacency-list/sparse-adjacency-matrix representation.
  - `Csr`: compressed sparse row sparse adjacency matrix graph.
  - `MatrixGraph`: adjacency-matrix-backed graph.
- `Graph` is the ergonomic mutable default. It stores `nodes: Vec<Node<...>>` and `edges: Vec<Edge<...>>`, allows parallel edges, and supports `add_node`/`add_edge` in `O(1)`. Removals use `swap_remove`, so removing a node/edge invalidates the removed id and the last node/edge id that moves into its slot.
- `StableGraph` is the stable-id mutable variant. It wraps `Graph<Option<N>, Option<E>, ...>`, keeps `free_node`/`free_edge` free lists, allows parallel edges, and removals invalidate only the removed id (plus incident edge ids for removed nodes), leaving gaps in the id space.
- `GraphMap` is label-keyed and simple-graph oriented: it stores `nodes: IndexMap<N, Vec<(N, CompactDirection)>>` plus `edges: IndexMap<(N, N), E>`, does not allow parallel edges, and `add_edge` updates and returns the old weight if the edge already existed.
- `Csr` stores `column`, `edges`, `row`, and `node_weights`; self loops are allowed but parallel edges are not. It is compact and fast for outgoing traversal. It has `from_sorted_edges` for `O(|V| + |E|)` construction when edges are sorted and unique. It also has incremental `add_edge`, but even row-major insertion is documented as `O(|V|·|E|)` for the whole operation because insertion shifts arrays and updates row offsets. It does not implement petgraph's `data::Build` trait in `data.rs`.
- `MatrixGraph` is an adjacency matrix; it does not allow parallel edges and supports `update_edge`/`add_or_update_edge` with `O(1)` best case and `O(|V|^2)` worst case when the matrix reallocates.
- Petgraph separates dynamic graph ergonomics from compact CSR. This supports oxgraph's direction: a construction/build layer can use edge tables/adjacency indexes and then freeze/export to CSR/CSC.
- Petgraph's `data::Build` trait abstracts graph construction at the trait level (`add_node`, `add_edge`, `update_edge`), but implementations still choose concrete storage layouts. `Graph`, `StableGraph`, and `GraphMap` implement this construction trait; `Csr` has its own construction methods instead.
- Petgraph uses node/edge weights as associated data in core graph types. For oxgraph, arbitrary Python metadata should stay out of foundational topology, but typed payload/weight layers or binding-level maps can provide equivalent ergonomics above the substrate.

## Implications for oxgraph

- A graph being built always has a physical storage strategy, but a builder can use a construction-friendly representation distinct from the final traversal layout.
- The proposed oxgraph layer should be graph-family-specific first (`graph-build`) rather than a fully generic graph/hypergraph builder, because graph edges and hypergraph participant sets have different input contracts.
- Petgraph suggests users are accustomed to choosing between mutable adjacency-style graphs, stable-id mutable graphs, label-keyed simple graphs, and compact CSR/matrix representations based on workload.
- Per-layout builders can still exist as lower-level helpers (`CsrBuilder`), but a user-facing graph builder should be able to ingest an edge stream once and freeze/export to CSR/CSC/COO/snapshot.
- Oxgraph should not copy petgraph's exact `weights in core graph` design into the foundation; instead, it can provide payload/weight layers above topology and use typed slices for algorithms.
