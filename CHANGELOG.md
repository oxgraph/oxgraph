# Changelog

All notable changes to the oxgraph crate family are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the workspace adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
All workspace crates version together.

## [0.4.3] - 2026-06-12

### Added

- The `oxgraph` umbrella crate now re-exports the alloc-tier graph algorithms
  that `oxgraph-algo` already shipped but the umbrella never surfaced:
  `topological_sort` / `ToposortError`, `strongly_connected_components`,
  `connected_components`, `longest_path_dag` / `LongestPathError`, and
  `shortest_path_lengths` — plus the bounded-BFS API (`BfsBounds`, `BfsVisitor`,
  `breadth_first_search_bounded`, `breadth_first_search_bounded_both`,
  `reverse_breadth_first_search_bounded`) and `PageRankHypergraph`. A consumer
  depending on `oxgraph` with `algo-alloc` can now reach Kahn's topological sort
  directly instead of reimplementing it. The `oxgraph::algo` module doc gains an
  example covering the acyclic ordering and the `strongly_connected_components`
  cycle-reporting recipe.

## [0.4.2] - 2026-06-11

Documentation-only patch release; no code changes.

### Changed

- Repositioned the project framing: the root and umbrella READMEs and the
  architecture doc now lead with "the general-purpose, zero-copy graph and
  hypergraph engine for Rust" instead of the "Topology here. Meaning
  elsewhere. Storage anywhere." tagline, and the `oxgraph` crate description
  follows suit. Matches the oxgraph.io and docs.oxgraph.io repositioning.

## [0.4.1] - 2026-06-10

Documentation-only patch release; no code changes.

### Added

- Every crate now ships its own README: what it is, where it sits in the
  layering, an example lifted from compile-checked code where one exists,
  features, and pointers to docs.rs and the family README. Published crates
  now set `readme = "README.md"` instead of inheriting the workspace root
  README, so crates.io shows crate-specific pages rather than the same root
  README on all of them.

## [0.4.0] - 2026-06-09

Consolidation release for the 21-PR refactor series: one write path, typed
subsystem errors, a self-checking container format, and structural sharing on
the database write path.

### ⚠ Breaking — public API

- **`SnapshotBuilder` is deleted.** `SnapshotWriter` is the one owning write
  path (write-through encode: payloads stream to their final offsets, ~1x peak
  memory instead of the builder's ~2x own-then-copy). `SnapshotWriter::new`
  takes `(max_sections, checksum)`; the convenience methods `section_bytes`,
  `section_typed`, `section_little_endian`, and `section_widths` replace the
  builder's `add_section*` family one-for-one. The no-`alloc`
  `SnapshotPlan`/`PendingSection` planner is unchanged.
  `oxgraph_property::export::append_identity_and_property_sections` now takes
  `&mut SnapshotWriter`.
- **`DbError` is split into subsystem enums** (open/bind, write/commit,
  query/read, maintenance) so callers match on the failing subsystem instead
  of one flat enum; `IdFamily` moved alongside the identity vocabulary.
- **Dense-index trait renames** across `oxgraph-topology` (the `Dense*` family)
  with blanket count shadows; downstream code naming the old traits must
  rename imports.
- **`oxgraph-layout-util` is namespaced**: build-time helpers live under
  `oxgraph_layout_util::build` (including the shared `slice_to_le` and offset
  builders) and integrity checks under `oxgraph_layout_util::integrity`.
- **Borrowed read surface**: `Reader::value` / `Reader::text` return borrowed
  (`Cow`) data instead of owned copies; call sites that relied on owned
  returns must clone explicitly.
- **Property width unification** onto `LayoutIndex` / `SnapshotWidth`: the
  property crate's separate width family is gone.
- **BCSR generics bundle**: the six-parameter word generics collapse into the
  single `BcsrWords` bundle; BCSR frozen wrappers no longer expose derived
  arrays (they delegate to the borrowed view).
- **Umbrella feature cleanup** (`oxgraph` crate): empty default features; a
  feature is the only thing that pulls a layer in. `snapshot-alloc` now
  re-exports `SnapshotWriter`/`SectionSink` (previously `SnapshotBuilder`).

### On-disk formats

Both formats changed shape in this release; with nothing deployed, each is
simply format version 1 — there are no legacy readers and no migration paths.

- **OXGT container**: per-section CRC-32C plus a header table CRC are
  mandatory; section kinds must be strictly ascending (lookup is a binary
  search). Checksums are injected through the `Checksum32` seam; a
  pure-software CRC-32C lives in `oxgraph-layout-util`.
- **Section-kind scheme**: kinds encode their word width in the low bits
  (`BASE | WIDTH_CODE`); every in-tree layer's kinds live in registry bands
  (see `docs/section-kind-registry.md`).
- **OXGDB** (`oxgraph-db`): section kinds are contiguous in emission order,
  there is no whole-base trailer, and payload verification happens at bind
  time. A store whose header carries an unknown format version is rejected —
  there is no rebuild-from-records fallback.

### Added

- `SnapshotWriter` typed-width conveniences: `section_widths` (native index
  slices lowered via `slice_to_le`), `section_little_endian` (portable
  byteorder words), and `section_bytes` (raw payloads), keeping exporters
  one-liners.
- `PageRankHypergraph` bound bundle in `oxgraph-algo`, collapsing the
  hypergraph PageRank where-clause into one trait.
- Shared definition codec for projection/index defs in `oxgraph-db`'s
  `wire::defs` (one encode/decode path).
- Shared build helpers in `oxgraph-layout-util::build` (offset-index
  construction, dense-ID validation, width lowering), replacing per-crate
  copies in the CSR/BCSR builders.
- Property export helpers in `oxgraph_property::export` (layer-length
  validation plus the canonical identity/property section tail) shared by the
  CSR and BCSR exporters.
- Section-kind registry doc (`docs/section-kind-registry.md`) and the
  `kinds` band-allocation module in `oxgraph-snapshot`.

### Changed

- `oxgraph-db` decomposed into `overlay/` and `database/` module trees (pure
  moves; no behavior change).
- `Database::begin_write` uses Arc-based copy-on-write structural sharing:
  unchanged columns are shared, not cloned. Write-path benches improved from
  4.8/6.2/18 ms to 4.4/4.6/9.0 ms.
- CSR frozen graphs delegate traversal to the borrowed `as_view` zero-copy
  view, pinned by a build-vs-view equivalence law; BCSR frozen wrappers
  likewise delegate and their derived index arrays were deleted.
- Database freeze streams sections through `SnapshotWriter` instead of
  buffering whole snapshots.
- Internal write path no longer clones column data per commit.

### Removed

- `SnapshotBuilder` and its `add_section*` API (see breaking notes above).
- The OXGDB whole-base trailer and its post-encode patch path.
- BCSR derived adjacency arrays on frozen wrappers.
- Per-crate `*_slice_to_le` copies and duplicated def codecs.
