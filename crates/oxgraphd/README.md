# oxgraphd

HTTP server and CLI package for the OxGraph database.

Part of the [oxgraph](https://github.com/oxgraph/oxgraph) workspace. Not
published to crates.io (`publish = false`).

## What it is

`oxgraphd` is a thin facade over `oxgraph-db`, the embedded engine. It adds
no database logic; it packages the engine behind two binaries:

- **`oxgraph`**, a CLI for working with a database directory:
  `oxgraph db <create|status|query|explain|compact|validate|catalog|projections|indexes> ...`
- **`oxgraphd`**, an HTTP server that serves one database:
  `oxgraphd <database-path> <address>`

## Where it sits

```text
oxgraph-db                        embedded engine (catalog, OxQL, transactions)
└── oxgraphd                    ← this crate (HTTP server + CLI facade)
```

## Running it

From the workspace root:

```sh
cargo run -p oxgraphd --bin oxgraph -- db create ./social.oxgdb
cargo run -p oxgraphd --bin oxgraph -- db status ./social.oxgdb
cargo run -p oxgraphd --bin oxgraphd -- ./social.oxgdb 127.0.0.1:7474
```

## Documentation

This crate is not on docs.rs. See
[docs.rs/oxgraph-db](https://docs.rs/oxgraph-db) for the engine it wraps and
the [oxgraph family README](https://github.com/oxgraph/oxgraph#readme) for
how the layers fit together.

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
