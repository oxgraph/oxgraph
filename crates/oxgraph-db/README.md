# oxgraph-db

Standalone OxGraph-native database engine above the topology substrate.

[![crates.io](https://img.shields.io/crates/v/oxgraph-db.svg)](https://crates.io/crates/oxgraph-db)
[![docs.rs](https://docs.rs/oxgraph-db/badge.svg)](https://docs.rs/oxgraph-db)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The embedded database of the [oxgraph](https://github.com/oxgraph/oxgraph)
crate family.

## What it is

`oxgraph-db` is the product layer above the topology substrate. It owns what
the foundation crates refuse to: durable database identity, a catalog of
labels, relation types, roles, and property keys, cataloged physical
projections (graph and hypergraph), typed properties, indexes, native OxQL
execution, and embedded transaction semantics.

Storage is a per-generation base snapshot plus a delta log. Writes go
through a transaction closure and commit to the log; opening a database
replays the valid log prefix over the memory-mapped base, truncating a torn
tail back to its last good byte. Compaction freezes the current state into
the next base generation. Reads run against a consistent pin and include
OxQL queries, graph/hypergraph walks, and PageRank over the cataloged
projections (`PageRankConfig` is re-exported so `db`-only consumers can
configure it without depending on `oxgraph-algo`).

## Where it sits

```text
oxgraph-topology / -graph / -hyper   capability traits
oxgraph-csr / -hyper-bcsr / -csc     borrowed layouts
oxgraph-snapshot / -property / -mmap base-file machinery
oxgraph-algo                         BFS / PageRank
└── oxgraph-db                     ← this crate (embedded engine)
    └── oxgraphd                     HTTP server / CLI facade (unpublished)
```

## Example

Adapted from the engine's end-to-end tests
([`tests/e2e.rs`](https://github.com/oxgraph/oxgraph/blob/main/crates/oxgraph-db/tests/e2e.rs)):

```rust
use oxgraph_db::{Db, Key, PropertyFamily, PropertySubject, PropertyType, Text};

let mut db = Db::create("./social.oxgdb")?;

let (alice, _outcome) = db.write(|writer| {
    let person = writer.register_label("Person")?;
    let knows = writer.register_relation_type("Knows")?;
    let source = writer.register_role("source")?;
    let target = writer.register_role("target")?;
    let name =
        writer.register_property_key("name", PropertyFamily::Element, PropertyType::Text)?;

    let alice = writer.create_element()?;
    let bob = writer.create_element()?;
    writer.add_label(alice, person)?;
    writer.set(
        PropertySubject::Element(alice),
        Key::<Text>::from_id(name),
        "Alice".to_owned(),
    )?;

    let edge = writer.create_relation()?;
    writer.set_relation_type(edge, knows)?;
    writer.create_incidence(edge, alice, source)?;
    writer.create_incidence(edge, bob, target)?;
    Ok(alice)
})?;

let prepared = db.prepare("MATCH ELEMENTS WHERE name = 'Alice' OR name = 'Bob'")?;
let rows = db.reader().run(&prepared)?;
```

## Documentation

See [docs.rs/oxgraph-db](https://docs.rs/oxgraph-db) for the full API and
the [oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for
how the layers fit together. Also available through the umbrella crate:
`cargo add oxgraph --features db`.

## Status

Pre-1.0. The on-disk format is coupled to the crate version that wrote it;
treat persisted databases as not yet portable across versions.

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
