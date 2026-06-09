//! Criterion coverage for the database open/recovery cost contract.
//!
//! perf: these benchmarks defend "open is `O(base + log-tail)`". Two sweeps
//! isolate the two terms:
//!
//! * `db_open_over_base` opens stores whose data is fully FOLDED into the base (empty log tail) at
//!   growing base sizes — the open cost rises with the base (the `O(base)` term: mmap/CRC verify +
//!   `BaseRecords`/`BaseIndex` build).
//! * `db_open_over_log_tail` opens a small base with a growing UNFOLDED delta-log tail of committed
//!   single-element frames — the open cost rises with the log tail (the `O(log-tail)` term: WAL
//!   replay folding the frames into the overlay).
//!
//! The base/log-tail size is named in every benchmark identifier. Index/commit
//! costs live in `benches/lookup.rs` and `benches/commit.rs`.

use std::{fmt::Display, path::Path};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oxgraph_db::{
    CheckpointPolicy, Db, DbError, Int, Key, PropertyFamily, PropertySubject, PropertyType,
};

/// Unwraps a benchmark `Result`, panicking with `context` on error (benches must
/// not use `expect`, which the workspace lint table denies outside `#[test]`).
fn unwrap<T, E: Display>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error}"),
    }
}

/// Folded-base sizes the open-cost sweep covers (data folded into the base, no
/// log tail).
const BASE_SIZES: [usize; 3] = [1_000, 10_000, 100_000];

/// Unfolded log-tail lengths (committed single-element frames) the recovery-cost
/// sweep replays over a small base.
const LOG_TAIL_LENGTHS: [usize; 3] = [100, 1_000, 10_000];

/// Builds a temporary benchmark path.
fn bench_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("oxgraph-db-open-{name}-{}", std::process::id()))
}

/// Removes an existing benchmark path.
fn clean(path: &Path) {
    match std::fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("remove benchmark fixture: {error}"),
    }
}

/// Creates a store whose `element_count` ranked elements are FOLDED into the base
/// (so the delta-log is empty and open cost is dominated by the base term).
fn create_folded_base(path: &Path, element_count: usize) -> Result<(), DbError> {
    clean(path);
    let mut database = Db::create(path)?;
    database.set_checkpoint_policy(CheckpointPolicy::Manual);
    database.write(|writer| {
        let rank_key =
            writer.register_property_key("rank", PropertyFamily::Element, PropertyType::Integer)?;
        for index in 0..element_count {
            let element = writer.create_element()?;
            writer.set(
                PropertySubject::Element(element),
                Key::<Int>::from_id(rank_key),
                i64::try_from(index)
                    .map_err(|_error| DbError::Query(oxgraph_db::QueryError::ValueOutOfRange))?,
            )?;
        }
        Ok(())
    })?;
    database.compact()?;
    Ok(())
}

/// Creates a store with a small base and `frame_count` UNFOLDED committed
/// single-element frames in the delta-log (so open cost is dominated by the
/// log-tail replay term). Auto-checkpointing is disabled so the frames are not
/// folded away.
fn create_log_tail(path: &Path, frame_count: usize) -> Result<(), DbError> {
    clean(path);
    let mut database = Db::create(path)?;
    database.set_checkpoint_policy(CheckpointPolicy::Manual);
    for _ in 0..frame_count {
        database.write(|writer| {
            writer.create_element()?;
            Ok(())
        })?;
    }
    Ok(())
}

/// Benchmarks opening a folded-base store across growing base sizes (the
/// `O(base)` open term).
fn bench_open_over_base(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_open_over_base");
    for size in BASE_SIZES {
        let path = bench_path(&format!("base-{size}"));
        unwrap(
            create_folded_base(&path, size),
            "benchmark fixture should build",
        );
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _size| {
            b.iter(|| unwrap(Db::open(&path), "database should open"));
        });
    }
    group.finish();
}

/// Benchmarks opening a small-base store across growing UNFOLDED log-tail lengths
/// (the `O(log-tail)` recovery term).
fn bench_open_over_log_tail(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_open_over_log_tail");
    for frames in LOG_TAIL_LENGTHS {
        let path = bench_path(&format!("tail-{frames}"));
        unwrap(
            create_log_tail(&path, frames),
            "benchmark fixture should build",
        );
        group.bench_with_input(
            BenchmarkId::from_parameter(frames),
            &frames,
            |b, _frames| {
                b.iter(|| unwrap(Db::open(&path), "database should open"));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_open_over_base, bench_open_over_log_tail);
criterion_main!(benches);
