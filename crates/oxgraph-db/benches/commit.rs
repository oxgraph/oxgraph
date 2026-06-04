//! Criterion coverage for the commit-cost contract.
//!
//! perf: these benchmarks defend "commit is `O(change)` — flat as the base
//! grows". A single-element dirty commit is benchmarked over stores whose base
//! holds 1k / 10k / 100k committed elements; the per-commit time must stay
//! roughly flat across those base sizes, since a commit only freezes the delta
//! it accumulated (overlay seeded `O(parent change)`, frame `O(change)`),
//! independent of the base record count. The base size is named in every
//! benchmark identifier. Auto-checkpointing is disabled so the commit cost is
//! measured in isolation from any fold.

use std::{fmt::Display, path::Path};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oxgraph_db::{
    CheckpointPolicy, Db, DbError, Int, Key, PropertyFamily, PropertySubject, PropertyType,
};

/// Base sizes the commit benches sweep, spanning two decades so a flat
/// per-commit time across them shows commit is `O(change)`, not `O(base)`.
const BASE_SIZES: [usize; 3] = [1_000, 10_000, 100_000];

/// Unwraps a benchmark `Result`, panicking with `context` on error (benches must
/// not use `expect`, which the workspace lint table denies outside `#[test]`).
fn unwrap<T, E: Display>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error}"),
    }
}

/// Builds a temporary benchmark path.
fn bench_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("oxgraph-db-commit-{name}-{}", std::process::id()))
}

/// Removes an existing benchmark path.
fn clean(path: &Path) {
    match std::fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("remove benchmark fixture: {error}"),
    }
}

/// Creates a store whose base holds `element_count` ranked elements, folded into
/// the base so subsequent commits layer over a large base. Auto-checkpointing is
/// disabled so the measured commits never trigger a fold.
fn create_database(path: &Path, element_count: usize) -> Result<Db, DbError> {
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
                i64::try_from(index).map_err(|_error| DbError::IdOverflow)?,
            )?;
        }
        Ok(())
    })?;
    database.compact()?;
    // The reopen inside `checkpoint` resets the policy; disable it again.
    database.set_checkpoint_policy(CheckpointPolicy::Manual);
    Ok(database)
}

/// Opens a fixture database, panicking on setup failure.
fn database_or_panic(name: &str, element_count: usize) -> Db {
    let path = bench_path(name);
    unwrap(
        create_database(&path, element_count),
        "benchmark fixture should build",
    )
}

/// Commits one fresh element against `database` (the measured unit of work).
fn commit_one_element(database: &mut Db) {
    unwrap(
        database.write(|writer| {
            writer.create_element()?;
            Ok(())
        }),
        "transaction should commit",
    );
}

/// Benchmarks a single-element dirty commit over growing base sizes; the
/// per-commit time must stay roughly flat (commit is `O(change)`).
fn bench_commit_single_element(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_commit_single_element_over_base");
    for size in BASE_SIZES {
        let mut database = database_or_panic("single", size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _size| {
            b.iter(|| commit_one_element(&mut database));
        });
    }
    group.finish();
}

/// Benchmarks creating and rolling back an empty writer over growing base sizes
/// (the `begin_write`/rollback fast path must not depend on base size).
fn bench_begin_write_rollback(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_begin_write_rollback_over_base");
    for size in BASE_SIZES {
        let mut database = database_or_panic("rollback", size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _size| {
            b.iter(|| {
                let _ = database.write(|_writer| Err::<(), DbError>(DbError::EmptyQuery));
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_commit_single_element,
    bench_begin_write_rollback
);
criterion_main!(benches);
