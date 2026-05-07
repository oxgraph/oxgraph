# OxGraph architecture

OxGraph is a storage-agnostic topology substrate: **Topology here. Meaning
elsewhere. Storage anywhere.** Foundation crates stay domain-neutral, no-std
where possible, and independent of Arrow, properties, PyO3, Python labels, and
algorithm semantics. Higher layers adapt selected data into explicit capability
views.

## Active layer graph

```text
oxgraph-topology        no_std shared IDs, traversal, weights, identity traits
  |- oxgraph-graph      no_std binary graph vocabulary wrappers
  `- oxgraph-hyper      no_std hypergraph vocabulary wrappers

oxgraph-csr            borrowed CSR graph layout
oxgraph-hyper-bcsr     borrowed directed bipartite-CSR hypergraph layout
oxgraph-snapshot       topology-agnostic section container

oxgraph-property       std + Arrow named typed property layers and selections
oxgraph-graph-build    no_std + alloc append/freeze graph construction core
oxgraph-hyper-build    no_std + alloc append/freeze hypergraph construction core
oxgraph-algo           BFS and PageRank over capability bounds
oxgraph                feature-gated curated umbrella re-exports
```

The Python facade is not part of the active Rust merge stack. It is a blocked
follow-up after the Rust contracts in this document are implemented and
reviewed.

Foundation crates (`topology`, `graph`, `hyper`, `csr`, and `hyper-bcsr`) must
not depend on Arrow, `oxgraph-property`, PyO3, Python packaging, or builder
crates. Builders may depend on snapshot/layout crates only behind explicit
export features.

## Width policy

Public Rust APIs in this stack do not use default generic type parameters to
choose ID widths, index widths, numeric scalars, damping policy, or tolerance
policy for users. Callers spell the width or scalar type they want.

CSR memory views use two logical widths:

- `NodeIndex` for node IDs and target entries.
- `EdgeIndex` for edge IDs and offset entries.

BCSR memory views use three logical widths:

- `VertexIndex` for vertex IDs and participant arrays.
- `RelationIndex` for hyperedge/relation IDs and vertex relation arrays.
- `IncidenceIndex` for participant/incidence IDs and offset arrays.

Native memory views may use `u16`, `u32`, `u64`, or `usize`. Persisted snapshot
wire widths are only `u16`, `u32`, and `u64`; `usize` is native-memory-only.

## Snapshot wire contract

`oxgraph-snapshot` remains a topology-agnostic section container. Layout and
property crates register section kinds and validate their own payloads.

Persisted integer payloads are explicitly little-endian. CSR and BCSR topology
sections use width-specific section kinds, so a generic tool can identify
payload width from section kind alone. Snapshot readers request a typed view;
wrong-width opens fail instead of reinterpreting bytes.

Property and identity sections are also width-specific. Descriptor records and
identity mode/map records are generic over a selected metadata/canonical ID word
width. Snapshot bytes remain an internal ABI candidate, not a stable ABI.

## Properties and identity

Topology weights are semantic-free optional capabilities. A weight view is one
selected total view at a time; it is not a named property registry.

Named values live in `oxgraph-property`. Property layer IDs, sparse indexes,
descriptor metadata words, and identity-map canonical IDs are generic over
explicit unsigned widths. Arrow IPC schema is the only stored Arrow type/schema
source of truth; coarse duplicate Arrow family metadata is not part of the
contract.

Property arrays in snapshots are keyed by snapshot-local IDs. If a snapshot
layout reorders IDs relative to canonical builder order, the exporter must:

- emit the matching local-to-canonical identity map;
- rekey affected property layers into snapshot-local order;
- reject property layers that do not cover the visible local ID range.

For CSR graph snapshots, relation/edge properties are rekeyed when CSR local
edge order differs from canonical edge insertion order. For BCSR hypergraph
snapshots, incidence properties are rekeyed because snapshot-local participant
order is head/source incidences followed by tail/target incidences in BCSR
order.

Sparse-default property snapshots store two explicit Arrow IPC ranges: sparse
index/value data and a length-one non-null default stream.

## Builders

Builders are append/freeze construction systems, not mutation frameworks. The
core graph and hypergraph builder crates are `no_std + alloc`, generic over
explicit builder ID widths, and free of Arrow and property dependencies in their
base feature set.

Unweighted builders do not carry weight fields. Weighted builders require
explicit element, relation, and incidence weights on add operations. Builder
generation counters and cache invalidation APIs are not part of this stack.

Snapshot export lives behind `snapshot` features. Property Arrow export lives
behind `property-arrow` features and uses `oxgraph-property` rekey helpers
rather than duplicating Arrow reorder logic in builder crates.

The base `snapshot` feature exports topology sections only. Canonical identity
mode/map sections and weight/property payloads are emitted by `property-arrow`
export helpers, because the identity records and typed value payloads are part
of the property/identity snapshot contract. Generic Rust weight fields have no
wire format unless the caller supplies explicit Arrow property layers.

## PageRank

`oxgraph-algo::pagerank` uses induced visible-state semantics:

- caller-provided elements and relations define the visible state set;
- duplicate visible elements or relations are typed errors;
- transitions to invisible states are ignored;
- a row with no visible outgoing targets is dangling;
- weighted row totals sum only visible outgoing targets.

Rank configuration, reports, errors, scratch storage, workspaces, and output
ranks are generic over a public `PageRankScalar`. Rust callers pass explicit
damping, tolerance, max iteration, and scalar choices; the API does not default
to `f64`.

Borrowed scratch APIs compile without `alloc`. Allocating convenience functions
and reusable workspace types are behind the `alloc` feature. PageRank does not
depend on Arrow or named properties; callers pass selected topology weight
views.

## Umbrella crate

`oxgraph` is a curated entry point, not a wildcard mirror of every internal
crate root. Its default feature set is empty. Feature names expose dependency
costs explicitly, including:

- `property-arrow`;
- `graph-snapshot`;
- `hyper-snapshot`;
- `graph-property-arrow`;
- `hyper-property-arrow`.

Enabling `graph-build` or `hyper-build` pulls only the core builder crates.
Snapshot and property export costs require explicit snapshot/property features.

## Python follow-up

Python bindings move to a future follow-up under `bindings/python`, outside the
Rust workspace default member set. That package may choose facade-owned string
labels and `f64` convenience APIs, but those choices must not leak into Rust
substrate APIs.

The follow-up must keep any PyO3 unsafe allowance local to the Python crate and
document it in `SAFETY.md`. Its first public surface is limited to builders,
frozen views, snapshot open helpers, typed exceptions, BFS, and PageRank.
Property-layer Python classes wait until property snapshot/export contracts are
landed and tested in Rust.
