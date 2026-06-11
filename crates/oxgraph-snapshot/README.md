# oxgraph-snapshot

Topology-agnostic byte-level snapshot container.

[![crates.io](https://img.shields.io/crates/v/oxgraph-snapshot.svg)](https://crates.io/crates/oxgraph-snapshot)
[![docs.rs](https://docs.rs/oxgraph-snapshot/badge.svg)](https://docs.rs/oxgraph-snapshot)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The byte container of the [oxgraph](https://github.com/oxgraph/oxgraph)
crate family: validate a snapshot, then borrow its sections as typed slices.
`no_std`, `unsafe`-free, allocation-free by default.

## What it is

`oxgraph-snapshot` defines a sectioned, zero-copy checkpoint format that can
validate, package, and read immutable topology snapshots without knowing
whether their contents are graphs, hypergraphs, or anything else. Section
semantics belong to upper layers (`oxgraph-csr`, `oxgraph-hyper-bcsr`, the
engines); the container only validates and exposes byte-aligned section
payloads.

Every snapshot is a fixed-size header, one entry per section, and a payload
region. All multi-byte integers are little-endian and stored unaligned, so a
snapshot can be borrowed from any byte slice (`Vec<u8>`, an mmap'd file, a
sub-slice) with no alignment requirement on the base pointer. Two integrity
invariants are mandatory: every section entry carries a CRC-32C over its
payload and the header carries a CRC-32C over the section table. Section
kinds must be strictly ascending, which makes the table duplicate-free and
section lookup a binary search. `Section::try_as_slice` checks the payload
pointer's alignment before reinterpreting `&[u8]` as `&[T]`.

## Where it sits

```text
oxgraph-layout-util               index/word vocabulary, CRC-32C
└── oxgraph-snapshot            ← this crate (byte container)
    ├── oxgraph-csr / oxgraph-csc    graph sections
    ├── oxgraph-hyper-bcsr           hypergraph sections
    └── oxgraph-db / oxgraph-postgres  engine base files
```

## Example

Adapted from the runnable example
[`examples/pack_two_sections.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-snapshot/examples/pack_two_sections.rs)
(`cargo run -p oxgraph-snapshot --example pack_two_sections --features alloc`):

```rust
use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{Snapshot, SnapshotWriter};

// Write two sections; the writer needs the `alloc` feature.
let mut writer = SnapshotWriter::new(2, crc32c_append)?;
writer.section_typed(0x0100, 0, &[10u32, 20, 30, 40, 50])?;
writer.section_bytes(0x0101, 0, 0, b"raw payload")?;
let bytes = writer.finish()?;

// Open validates the header and section table; payload CRCs are checked
// on the verified read paths.
let snapshot = Snapshot::open(&bytes)?;
for section in snapshot.sections() {
    println!("kind={} bytes={}", section.kind(), section.bytes().len());
}
```

## Features

| Feature | Effect |
| --- | --- |
| none (default) | Reader-only API; `no_std`, no allocation. |
| `alloc` | Enables `SnapshotWriter`, the owning encoder that returns `Vec<u8>`. |
| `std` | Reserved for future mmap helpers; activates `alloc`. |

## Stability

The bytes are intentionally not yet promised as a stable ABI: a snapshot
whose format major does not match this library is rejected at open. Treat
persisted snapshots as coupled to the crate version that wrote them.

## Documentation

See [docs.rs/oxgraph-snapshot](https://docs.rs/oxgraph-snapshot) for the
full API and the
[oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for how
the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features snapshot` (or `snapshot-alloc` for writing).

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
