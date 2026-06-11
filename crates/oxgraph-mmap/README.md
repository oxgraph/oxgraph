# oxgraph-mmap

Read-only memory-map shim: the single audited unsafe island for OxGraph's zero-copy base files.

[![crates.io](https://img.shields.io/crates/v/oxgraph-mmap.svg)](https://crates.io/crates/oxgraph-mmap)
[![docs.rs](https://docs.rs/oxgraph-mmap/badge.svg)](https://docs.rs/oxgraph-mmap)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/oxgraph/oxgraph/blob/main/LICENSE)

The one place in the [oxgraph](https://github.com/oxgraph/oxgraph)
workspace where `unsafe` is allowed.

## What it is

Memory-mapping a file is inherently `unsafe` in Rust: the kernel can change
the mapped bytes under a live `&[u8]` if the file is modified or truncated,
which is undefined behavior, so `memmap2` exposes only `unsafe fn`
constructors for file-backed maps. Every other crate in the workspace keeps
`unsafe_code = "forbid"`. This crate exists so the one unavoidable `unsafe`
call lives in exactly one small, audited place behind a safe API, instead of
weakening the storage engine's `forbid` posture.

The whole public API is one function, `map_read_only`, plus a re-export of
`memmap2::Mmap`.

## Safety contract for callers

The mapped file MUST be immutable for the lifetime of the returned `Mmap`.
The engine only maps published, per-generation base files
(`base-{g}.oxgdb`), which are written once via atomic rename and never
modified in place or truncated while a reader holds them, so the contract
holds by construction.

## Example

```rust
use std::fs::File;

use oxgraph_mmap::map_read_only;

let file = File::open("base-1.oxgdb")?;
let map = map_read_only(&file)?; // one syscall; pages fault in lazily
let bytes: &[u8] = &map;
```

## Documentation

See [docs.rs/oxgraph-mmap](https://docs.rs/oxgraph-mmap) for the full API
and the [oxgraph family README](https://github.com/oxgraph/oxgraph#readme)
for how the layers fit together.

## License

MIT. See [LICENSE](https://github.com/oxgraph/oxgraph/blob/main/LICENSE).
