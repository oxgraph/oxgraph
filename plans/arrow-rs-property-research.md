# Arrow-rs fit research for `oxgraph-property`

## Question

Can `arrow-rs` fit OxGraph's architecture for a future `oxgraph-property` layer, especially around `no_std`, zero-copy, snapshots/mmap, and dependency boundaries?

## Context

OxGraph's deepest crates (`oxgraph-topology`, graph/hyper traits, CSR/BCSR layouts, snapshots) are designed as storage-agnostic, zero-copy-friendly, and `no_std` where possible. We are considering whether `oxgraph-property` should depend directly on `arrow-rs`, align with Arrow concepts without depending on it, or keep Arrow support in a separate optional interop crate.

## Research questions

- Does `arrow-rs` support `no_std`?
- Which `arrow-rs` crates are required for array/schema/property storage?
- Does `arrow-rs` support zero-copy arrays/views, slicing, and memory-mapped buffers?
- Does Arrow's memory model align with OxGraph snapshots and validation discipline?
- What dependency/feature impact would a direct dependency have?
- Should Arrow live in `oxgraph-property`, behind a feature, or in a separate `oxgraph-arrow` crate?

## Research lanes

The current interface does not expose a separate sub-agent invocation tool, so this research is split into sub-agent-style lanes and run with parallel source checks.

### Lane A — `no_std` and dependency boundary

Findings from `arrow-rs` `Cargo.toml` files and source markers:

- The Arrow workspace is currently edition 2024 with `rust-version = "1.85"`. OxGraph currently pins its own Rust toolchain, so MSRV/toolchain compatibility must be checked before adopting a direct dependency.
- `default-features = false` does **not** by itself mean `no_std`. It only disables optional Cargo features; the crate must also be authored with `#![no_std]` or conditional no-std support.
- A local probe showed a `#![no_std]` crate on the host target can depend on `arrow-array`/`arrow-buffer` with `default-features = false`, but that only proves the local crate can avoid directly importing `std`; Arrow dependencies can still link `std` on a std-capable target.
- `arrow-array`, `arrow-buffer`, `arrow-data`, and `arrow-schema` do not advertise `#![no_std]` in the inspected crate roots.
- Core Arrow crates use `std` concepts directly in source/docs, especially `std::sync::Arc`, `std::alloc`, `std::mem`, `std::slice`, and `std::fmt`.
- `arrow-buffer` uses `Arc<Bytes>` internally for `Buffer` and has unsafe `Send`/`Sync` impls around that buffer representation.
- `arrow-array` exposes `ArrayRef = Arc<dyn Array>` style APIs.
- The top-level Arrow README says Arrow can compile to WebAssembly with default features disabled, but this is not the same as `no_std` support.

Interpretation:

- A direct `arrow-rs` dependency should be treated as `std`-requiring unless upstream explicitly documents and tests no-std support for the specific crates/features we use. It does **not** fit the deepest `no_std` substrate crates (`oxgraph-topology`, graph/hyper traits, CSR/BCSR layout crates).
- It may fit a higher `std` layer such as `oxgraph-property`, `oxgraph-arrow`, or Python/data interop, if we accept the dependency boundary.

### Lane B — zero-copy and mmap/snapshot fit

Findings from Arrow docs/source:

- Arrow arrays are backed by one or more shared memory regions backed by `Buffer`.
- `Buffer`s can be sliced and cloned without copying underlying data and can be created from `Vec` or `bytes::Bytes` without copying.
- `ArrayData::slice` creates a zero-copy slice pointing at the same underlying buffers with different offset/length.
- Arrow arrays model primitive buffers, validity/null buffers, offset buffers for strings/lists, and child arrays for nested types.
- Arrow has validation APIs around `ArrayData`, including extra validation features such as `force_validate`.
- Arrow IPC has `memmap2` as a dev dependency in the inspected crate, and Arrow's buffer model can represent shared byte regions, but OxGraph would still need to validate exact mmap/open mechanics for our snapshot container.

Interpretation:

- Arrow's physical model is strongly aligned with OxGraph's property-layer needs: typed columns, shared buffers, zero-copy slicing, validity bitmaps, offsets for variable-width data, and nested arrays.
- Arrow's zero-copy story is mostly about sharing Arrow buffers/arrays, not automatically making OxGraph's custom snapshot sections Arrow IPC-compatible.
- If OxGraph wants Arrow-native property sections, we must decide whether snapshots embed Arrow IPC/FFI-compatible payloads, reference Arrow buffers directly, or store OxGraph-native sections that can be converted to Arrow.

### Lane C — safety and verification fit

Findings from Arrow README/source:

- Arrow uses unsafe internally and exposes some unsafe APIs for performance/validation opt-outs.
- Arrow explicitly documents a safety model and aims for no undefined behavior through safe APIs.
- Arrow uses strongly typed arrays/builders, validation logic for `ArrayData`, MIRI, and a `force_validate` feature.

Interpretation:

- A direct Arrow dependency would bring unsafe code into the dependency tree, but not into OxGraph's own foundation crates.
- This is probably acceptable for a `std` property/interop layer if our shapes document the boundary.
- It is not compatible with treating `oxgraph-property` as a tiny `no_std` substrate peer unless we feature-gate or split crates.

## Architectural options

### Option A — `oxgraph-property` depends directly on `arrow-rs`

Pros:

- Mature, well-supported typed array/schema system.
- Strong Python/data/Parquet ecosystem path.
- Handles generic values immediately: primitives, strings, binary, lists, structs, dictionaries, nulls.
- Good zero-copy slicing/buffer model.

Cons:

- Not `no_std` based on inspected sources.
- Higher MSRV/toolchain pressure (`arrow-rs` master currently uses Rust 1.85 / edition 2024).
- Larger dependency tree and compile time.
- Brings Arrow's API/versioning into a core OxGraph layer.
- OxGraph snapshots would need careful design to avoid conflating OxGraph snapshot format with Arrow IPC format.

### Option B — `oxgraph-property` defines a lightweight OxGraph-native descriptor model; `oxgraph-arrow` handles Arrow interop

Pros:

- Keeps property layer lighter and more controlled.
- Can preserve `no_std`/alloc-friendly pieces if desired.
- Avoids hard dependency in the core crate graph.
- Lets snapshots stay OxGraph-native while still permitting Arrow conversion.

Cons:

- Reinvents significant columnar/schema machinery.
- Risk of drifting from Arrow and making interop harder.
- More design and maintenance burden.

### Option C — split/feature-gated architecture

Possible shape:

```text
oxgraph-property-core
  no_std/alloc-friendly descriptors, layer IDs, ID-family metadata, validation traits

oxgraph-property-arrow or oxgraph-arrow
  depends on arrow-array/arrow-schema/arrow-buffer
  stores property data as Arrow arrays
  provides Arrow C Data / IPC / Parquet interop later

oxgraph-python
  can depend on the Arrow-backed layer for rich Python/data properties
```

Pros:

- Keeps deepest substrate clean.
- Allows Arrow where it is strongest.
- Lets users who need Python/data interop opt in.
- Avoids blocking property design on `no_std` concerns.

Cons:

- More crates/features.
- Need to define the seam carefully so algorithms can consume selected weight views from either native or Arrow-backed layers.

## Recommendation / decision

Accepted direction for OxGraph:

> Do not put `arrow-rs` in `oxgraph-topology` or any foundation/no_std traversal/layout crate. `oxgraph-property` is a higher-level `std` crate that depends on `arrow-rs` for named generic typed property layers. Umbrella features keep this optional so minimal users do not pay for Arrow.

Concrete next steps:

- Pin an actual released Arrow version during implementation.
- Prototype a tiny Arrow-backed relation property layer after shape approval: descriptor + `arrow_array::Float64Array` + selected `RelationWeight` view.
- Validate whether an Arrow array can be built from OxGraph snapshot-owned/mmap-owned buffers without copying and with acceptable lifetime/ownership semantics.
- Decide snapshot format boundary: OxGraph-native property sections convertible to Arrow vs embedded Arrow IPC/FFI-compatible sections.
