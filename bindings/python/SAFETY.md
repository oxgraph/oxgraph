# OxGraph Python safety boundary

The Python facade lives under `bindings/python` as a standalone maturin crate.
It is not a member of the root Rust workspace, so PyO3 build requirements do
not affect the Rust PR #1-#8 merge path.

## Unsafe/generated glue

PyO3 generates the CPython type slots and module initialization for this crate.
That generated glue is isolated to this crate through the crate-local
`unsafe_code = "allow"` lint setting required by PyO3 extension modules. Rust
code in topology, graph, hypergraph, layout, snapshot, builder, and algorithm
crates remains safe Rust with `unsafe_code = "forbid"` inherited from the root
workspace.

## Packaging

The package is built with maturin from `bindings/python`, `python-source =
"python"`, and native module name `oxgraph._oxgraph`. The Rust library builds
as `_oxgraph`; maturin installs it under the Python package so `import oxgraph`
re-exports the native classes and helpers.

## Ownership and lifetimes

Python-facing frozen graph and hypergraph objects own their Rust frozen views. They do not borrow from builders, so later builder edits cannot invalidate already-frozen Python objects.

## Topology inspection

Frozen graph and hypergraph inspection methods return local integer IDs and
materialized Python lists or tuples through concise graph-family APIs such as
`nodes`, `edges`, `out_edges`, `vertices`, `hyperedges`, and `out_hyperedges`.
They wrap OxGraph traversal traits and do not expose borrowed Rust iterators,
raw storage slices, legacy aliases, or third-party graph library objects.

## Stale views

The first Python API exposes owned frozen views only. Borrowed generation-checked caches are not exposed through Python in this slice.

## GIL and threads

PyO3 methods execute while called from Python under the GIL. The first API does not spawn Rust threads, release the GIL for long-running algorithms, or share mutable builders across Python threads.

## Panic and errors

Rust errors are converted to Python `ValueError` with the typed Rust error message. The binding does not intentionally panic across the FFI boundary.

## Snapshot/raw buffers

Snapshot helpers validate raw bytes through `oxgraph-snapshot` before
constructing topology views. Python buffers are not interpreted as typed
topology storage without validation.

## Properties

The first reviewable facade intentionally does not expose property-layer
classes. Properties and property-backed weights can be added later with Python
tests after the Rust property snapshot contracts have landed.
