# oxgraph-layout-util

Shared layout primitives for oxgraph: builder helpers + offset-integrity validation.

[![crates.io](https://img.shields.io/crates/v/oxgraph-layout-util.svg)](https://crates.io/crates/oxgraph-layout-util)
[![docs.rs](https://docs.rs/oxgraph-layout-util/badge.svg)](https://docs.rs/oxgraph-layout-util)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The support crate every concrete [oxgraph](https://github.com/oxgraph/oxgraph)
layout reuses. `no_std` (+ `alloc` for build-time helpers), `unsafe`-free, no
dependency on any other oxgraph crate.

## What it is

Three responsibilities live here, one namespace each:

- **Index/word vocabulary** (crate root): `LayoutIndex`, `LayoutWord`,
  `LayoutSnapshotWord`, and `SnapshotWidth`, the single sealed set of dense
  index widths, native and little-endian storage words, and the
  width-to-LE-word bijection shared by every layout and snapshot crate.
  `LocalId` is the one generic local-handle newtype layout crates alias for
  their node/edge/vertex/hyperedge/incidence identities, and `IdSlice` is the
  one slice-to-handle iterator they all reuse.
- **Build-time** (`build` module): validate dense IDs against a known count,
  flatten per-bucket payloads into CSR-style `(offsets, items)` pairs, and
  lower native index slices into explicit little-endian words.
- **Read-time** (`integrity` module): walk borrowed offset arrays at
  view-open time and convert already-validated indexes infallibly.

The crate also provides the CRC-32C (Castagnoli) checksum function
(`crc32c_append`) that `oxgraph-snapshot` writers and verified readers plug
in. No public domain semantics: this crate knows nothing about graphs or
hypergraphs.

## Where it sits

```text
oxgraph-layout-util             ← this crate
├── oxgraph-snapshot              byte container (no-alloc dependency)
├── oxgraph-csr / oxgraph-csc     borrowed graph layouts
└── oxgraph-hyper-bcsr            borrowed hypergraph layout
```

## Features

| Feature | Effect |
| --- | --- |
| `alloc` (default) | Enables the build-time primitives that allocate (`build_offset_index`, `slice_to_le`). |
| none (`default-features = false`) | Keeps the index/word vocabulary, `LocalId`, `IdSlice`, and the offset-integrity checks, with no allocation. |

## Documentation

See [docs.rs/oxgraph-layout-util](https://docs.rs/oxgraph-layout-util) for
the full API and the
[oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for how
the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features layout`.

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
