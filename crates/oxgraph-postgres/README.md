# oxgraph-postgres

Postgres-backed OxGraph engine: catalog, build, artifact I/O, query, sync.

[![crates.io](https://img.shields.io/crates/v/oxgraph-postgres.svg)](https://crates.io/crates/oxgraph-postgres)
[![docs.rs](https://docs.rs/oxgraph-postgres/badge.svg)](https://docs.rs/oxgraph-postgres)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The Postgres engine of the [oxgraph](https://github.com/oxgraph/oxgraph)
crate family, as a plain Rust library.

## What it is

`oxgraph-postgres` turns relational rows into traversable OxGraph
topology. It owns catalog modeling (which tables and foreign keys define the
graph), the relational build into OXGTOPO snapshot artifacts, artifact I/O
and validation, active engine state, overlays for not-yet-rebuilt changes,
sync replay, and the query algorithms (search, traversal) that run over the
forward CSR and inbound CSC views.

This crate contains no Postgres extension machinery: it is deliberately a
library so the engine can be tested and benchmarked without a running
server. SPI calls, triggers, and the SQL facade live in `oxgraph-pgrx`, the
extension crate that embeds this engine into a Postgres backend.

## Where it sits

```text
oxgraph-csr / oxgraph-csc          borrowed forward/inbound layouts
oxgraph-snapshot                   OXGTOPO artifact container
oxgraph-algo                       traversal algorithms
└── oxgraph-postgres             ← this crate (engine library)
    └── oxgraph-pgrx               Postgres extension facade (unpublished)
```

## Features

| Feature | Effect |
| --- | --- |
| `std` (default) | The full engine; the crate is empty without it. |
| `serde` (default via `std`) | Serialization for config and status types. |
| `bench-fixture` | Deterministic fixture generation for benchmarks and extension tests. |

## Documentation

See [docs.rs/oxgraph-postgres](https://docs.rs/oxgraph-postgres) for the
full API and the
[oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for how
the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features postgres`.

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
