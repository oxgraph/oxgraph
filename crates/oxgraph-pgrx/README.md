# oxgraph-pgrx

PostgreSQL extension facade for OxGraph (pgrx SQL/SPI glue).

Part of the [oxgraph](https://github.com/oxgraph/oxgraph) workspace. Not
published to crates.io (`publish = false`); it builds as a Postgres
extension, not a library dependency.

## What it is

`oxgraph-pgrx` is the thin extension shell around `oxgraph-postgres`, which
owns all engine logic. This crate contributes only what must run inside a
Postgres backend: the SQL API and SPI calls, GUC registration, the
background maintenance worker, ACLs, and the `graph` schema bootstrap
(`sql/bootstrap.sql`).

Each backend process holds at most one in-memory engine in a thread-local
session slot. Other backends observe persisted snapshot bytes in
`graph._snapshot_store` after a rebuild or maintenance pass.

## Where it sits

```text
oxgraph-postgres                  engine library (catalog, build, query, sync)
└── oxgraph-pgrx                ← this crate (SQL/SPI facade, cdylib)
        loaded by PostgreSQL as the oxgraph extension
```

## Building and testing

The extension is excluded from the workspace's normal `ci`/`miri`/`kani`
aggregates and has its own `just` recipes (run from the repo root):

```sh
just postgres-init    # one-time pgrx setup (macOS needs `brew install pkgconf` first)
just postgres-test    # cargo pgrx test
just postgres-check   # clippy for the extension crate
just postgres-bench   # engine + extension benchmarks
```

Postgres major versions are selected by feature: `pg14` through `pg18`,
default `pg16`.

## Documentation

This crate is not on docs.rs. See the
[oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for how
the layers fit together and
[docs.rs/oxgraph-postgres](https://docs.rs/oxgraph-postgres) for the engine
it wraps.

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
