# oxgraph — project guidance for Claude

Storage-agnostic, zero-copy-friendly topology substrate. Those words are
load-bearing invariants. Respect the enforcement gates; don't route around
them.

## Posture

Agent write-cost ~0; CI compute and signal quality are the real
constraints. Don't bench or prove things that defend no contract —
noise is noise regardless of who produced it.

## Verification defaults

| Construct                            | Required verification                                              |
| ------------------------------------ | ------------------------------------------------------------------ |
| Public fn taking data                | `proptest` strategy exercising its invariants                      |
| Data type with algebraic contract    | `#[kani::proof]` per property (symmetry, totality, roundtrip, …)   |
| Public API taking a measurable input | `criterion` bench defending a stated perf contract                 |
| Every pub item's doc                 | `O(n)` / `≤ 1ms for n ≤ 10k` perf contract, or `perf: unspecified` |

`PartialEq` symmetry, `Hash ↔ Eq`, `Ord` totality, serde roundtrip,
merge laws — default to kani. Skip only where kani can't reach
(unbounded loops, IO, async); mark with `// kani-skip: <reason>`.

## Data layout: zero-copy

Wire formats, stored records, audit entries, and any struct that crosses
an IO boundary are `#[derive(FromBytes, IntoBytes, Immutable, KnownLayout)]`
via the `zerocopy` crate. Borrow, don't parse into owned intermediaries.
Reach for zerocopy before `String` / `Vec` — allocations on the data path
need a contract-level justification.

## CI split

- `just ci` — fast gate (fmt, taplo, clippy, deny, test). Runs in prek.
- `just verify` — heavy gate (miri, kani). Pre-PR, on demand.

## Library discipline

- `unsafe_code = "forbid"` stays forbid. Lifting: `// SAFETY:` per use.
- Every `pub` is a year-long support commitment.
- Errors are concrete enums implementing `Display` and `core::error::Error`.
  No `Box<dyn Error>` / `anyhow` on the public surface.
- No `unwrap` / `expect` in library code.
- `#[expect(..., reason = "…")]`, never `#[allow]`.
- Feature-flag optional integrations (serde, tracing, …).
- Newtypes over `String` / `HashMap` for domain IDs, timestamps, versions.
- Docs on every item, public and private. Lint-enforced.

## Enforcement

- `.claude/hooks/block-bypass.sh` (wired via gitignored
  `.claude/settings.local.json`) denies `--no-verify`, clippy `-A` /
  `--allow` / `--cap-lints`, `RUSTFLAGS=…-A`, `SKIP=`, `core.hooksPath`,
  `prek uninstall`. Tighten regex on false positives; don't sidestep.
- prek pre-commit is the floor. Fix failures; don't skip.
- No `Co-Authored-By: Claude …` on commits.

## Anti-patterns

- Benchmarks defending no contract.
- Proving things the type system already guarantees.
- `#[allow(clippy::…)]` with a `TODO`.
- Coverage chasing.
- Lint softening to unblock a PR — use a reasoned
  `[workspace.lints.clippy]` entry instead.

## Pause and ask before

- Public API reshuffles.
- Adding a workspace crate.
- Lifting `unsafe_code = "forbid"`.
- Touching `block-bypass.sh`, `.pre-commit-config.yaml`, or the
  workspace lint table.
