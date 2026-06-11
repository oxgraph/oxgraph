# oxgraph-csc

Borrowed compressed-sparse-column (inbound) graph views over snapshot sections.

[![crates.io](https://img.shields.io/crates/v/oxgraph-csc.svg)](https://crates.io/crates/oxgraph-csc)
[![docs.rs](https://docs.rs/oxgraph-csc/badge.svg)](https://docs.rs/oxgraph-csc)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The reverse-traversal counterpart to `oxgraph-csr` in the
[oxgraph](https://github.com/oxgraph/oxgraph) crate family. `no_std`,
`unsafe`-free.

## What it is

`oxgraph-csc` provides inbound adjacency: predecessor traversal over a graph
whose incoming edges are stored as CSR-on-transposed-edges in snapshot
sections. It is a separate crate from `oxgraph-csr` on purpose, so forward
and inbound views cannot be mixed at the type level.

The view is storage-agnostic. It owns no section-kind constants and reads
whatever offsets/targets kinds the caller supplies through
`CscSnapshotGraph::from_snapshot_with_kinds`; the storage layer that
persists the inbound index (for example the Postgres engine) owns the
section-kind constants and the section version. Opening a snapshot-backed
view validates the layout once, `O(s + n + m)` per engine load; traversal
after that is slice indexing.

## Where it sits

```text
oxgraph-graph                     node/edge capability traits
└── oxgraph-csc                 ← this crate (borrowed inbound layout)
    ├── wraps oxgraph-csr over transposed edges
    ├── reads caller-supplied oxgraph-snapshot sections
    └── consumed by oxgraph-postgres for reverse traversal
```

## Documentation

See [docs.rs/oxgraph-csc](https://docs.rs/oxgraph-csc) for the full API
(`oxgraph-postgres` is the reference consumer) and the
[oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for how
the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features csc`.

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
