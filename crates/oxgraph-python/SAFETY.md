# oxgraph-python safety boundary

`oxgraph-python` is the only crate in this workspace that may use PyO3 FFI glue. Foundation crates continue to inherit `unsafe_code = "forbid"` from the workspace.

## Unsafe/generated glue

PyO3 generates the CPython type slots and module initialization for this crate. That glue is isolated to this crate through the crate-local `unsafe_code = "allow"` lint setting required by PyO3 extension modules. Rust code in topology, graph, hypergraph, layout, snapshot, property, builder, and algorithm crates remains safe Rust with `unsafe_code = "forbid"` inherited from the workspace.

## Packaging spike

The packaging decision for this slice is maturin with `pyproject.toml` in `crates/oxgraph-python`, `python-source = "python"`, and native module name `oxgraph._oxgraph`. The Rust library still builds as `_oxgraph` for Cargo tests/checks; maturin installs it under the Python package so `import oxgraph` re-exports the native classes and helpers.

## Ownership and lifetimes

Python-facing frozen graph and hypergraph objects own their Rust frozen views. They do not borrow from builders, so later builder edits cannot invalidate already-frozen Python objects.

## Stale views

The first Python API exposes owned frozen views only. Borrowed generation-checked caches are not exposed through Python in this slice.

## GIL and threads

PyO3 methods execute while called from Python under the GIL. The first API does not spawn Rust threads, release the GIL for long-running algorithms, or share mutable builders across Python threads.

## Panic and errors

Rust errors are converted to Python `ValueError` with the typed Rust error message. The binding does not intentionally panic across the FFI boundary.

## Snapshot/raw buffers

Snapshot helpers exposed later must validate raw bytes through `oxgraph-snapshot` before constructing topology views. Python buffers are not interpreted as typed topology storage without validation.
