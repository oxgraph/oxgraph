# oxgraph engine benchmarks — 0.3.0

These numbers track the performance impact of the 0.3.0 engine overhaul
(O(change) identity-reconcile writes, intrinsic reverse-adjacency index, and
zero-copy / non-O(base) index open). They are measured **end to end through the
downstream consumer [oxcode](https://github.com/oxgraph/oxcode)** (tree-sitter
code indexing on top of `oxgraph-db`), which is the realistic workload the engine
is designed for.

## Method

- **baseline**: oxcode at its pre-overhaul commit on published **oxgraph 0.2.4**
  (`apply_delta` wholesale rewrite + O(total-incidences) tombstone).
- **current**: oxcode on **oxgraph 0.3.0** (identity reconcile + zero-copy open).
- Identical source corpora; release builds; sequential runs on an otherwise-idle
  16-core machine. Incremental reindex = append one function to a source file and
  re-index, verified by confirming the new symbol is queryable afterward.
- Measured 2026-06-03.

## Results

### corpus: storage-hub — 328 files · 40,749 elements · 527,999 relations · 1,055,998 incidences

| metric | 0.2.4 baseline | 0.3.0 | change |
| --- | ---: | ---: | ---: |
| incremental reindex (1-file edit) | **> 150 s** (~62 min, O(n²)) | **4,842 ms** | **≈ 770× faster** |
| symbol query (p50) | 3,902 ms | 988 ms | 3.9× faster |
| cold index | 15,165 ms | 11,968 ms | 1.3× faster |
| reindex, no change | 1,349 ms | 834 ms | 1.6× faster |
| WAL written per reindex | 953 MB | 5.2 MB | O(change) |
| database on disk | 805 MB | 1.18 GB | 1.5× larger* |

### corpus: harnessing — 76 files · 11,091 elements · 45,901 relations

| metric | 0.2.4 baseline | 0.3.0 | change |
| --- | ---: | ---: | ---: |
| incremental reindex (1-file edit) | **27,648 ms** | **444 ms** | **62× faster** |
| symbol query (p50) | 457 ms | 154 ms | 3.0× faster |
| cold index | 1,797 ms | 1,313 ms | 1.4× faster |

\* The larger on-disk size is the deliberate tradeoff for O(1)-style index open:
0.3.0 persists the derived index (equality / label / adjacency postings) as
zero-copy sections that are borrowed at open instead of rebuilt in RAM.

## What changed (engine)

- **Incremental reindex** went from O(n²) to O(change). The 0.2.4 `tombstone_*`
  primitives were O(total incidences) per call (no reverse adjacency), so a bulk
  edge replacement was O(n²) — ~62 min on a 528 K-relation graph. 0.3.0 adds an
  intrinsic reverse-adjacency index (cascade is O(log n + degree)) and
  identity-keyed reconcile verbs (`upsert_element` / `upsert_relation` / `retain`)
  whose unchanged subjects emit zero mutations (`set` no-ops on an equal value).
- **Query latency** dropped 3–4× from zero-copy index open: the base index is
  persisted at freeze and borrowed from the memory map at open, rather than
  decoded and rebuilt from records on every command.

Verification: `just ci` (fmt, taplo, clippy, deny, workspace tests), `just verify`
(miri on the zero-copy borrow path; `cargo kani` algebraic proofs — 68 verified,
0 failures), plus a freeze→open differential proptest against the owned-index
oracle.
