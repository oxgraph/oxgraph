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
    CheckpointPolicy, Db, DbError, ElementId, Int, Key, PropertyFamily, PropertyKeyId,
    PropertySubject, PropertyType, Text,
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
                i64::try_from(index)
                    .map_err(|_error| DbError::Query(oxgraph_db::QueryError::ValueOutOfRange))?,
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

/// Begins a write and immediately rolls it back by returning an error (the
/// measured unit of work for the rollback fast path).
fn rollback_empty_write(database: &mut Db) {
    let _ =
        database.write(|_writer| Err::<(), DbError>(DbError::Query(oxgraph_db::QueryError::Empty)));
}

/// Benchmarks creating and rolling back an empty writer over growing base sizes
/// (the `begin_write`/rollback fast path must not depend on base size).
fn bench_begin_write_rollback(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_begin_write_rollback_over_base");
    for size in BASE_SIZES {
        let mut database = database_or_panic("rollback", size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _size| {
            b.iter(|| rollback_empty_write(&mut database));
        });
    }
    group.finish();
}

/// Creates a store whose committed overlay holds `element_count` ranked
/// elements WITHOUT folding them into the base (manual policy, no compact),
/// so every subsequent `begin_write` seeds its writer from an unfolded
/// overlay of that size.
fn create_unfolded_database(path: &Path, element_count: usize) -> Result<Db, DbError> {
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
    Ok(database)
}

/// Benchmarks a single-element write over a committed-but-unfolded overlay of
/// growing size.
///
/// perf: this defends the honest `begin_write` contract — seeding the writer
/// clones the parent overlay's delta map STRUCTURE while label sets, text
/// values, per-subject property delta maps, and index posting sets are
/// `Arc`-shared copy-on-write, so a write over an unfolded overlay of `N`
/// entries is `O(N)` map entries at `O(1)` each. The residual `O(N)` term is
/// the outer map-node clone itself; `db_commit_single_element_over_base`
/// (folded bases) stays the `O(change)` commit contract.
fn bench_write_over_unfolded_overlay(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_write_over_unfolded_overlay");
    group.sample_size(20);
    for size in BASE_SIZES {
        let path = bench_path(&format!("unfolded-{size}"));
        let mut database = unwrap(
            create_unfolded_database(&path, size),
            "benchmark fixture should build",
        );
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _size| {
            b.iter(|| commit_one_element(&mut database));
        });
    }
    group.finish();
}

/// Creates a folded store with `element_count` elements and a text property
/// key, returning the handle, the element ids, and the key.
fn create_text_database(
    path: &Path,
    element_count: usize,
) -> Result<(Db, Vec<ElementId>, PropertyKeyId), DbError> {
    clean(path);
    let mut database = Db::create(path)?;
    database.set_checkpoint_policy(CheckpointPolicy::Manual);
    let (handles, _outcome) = database.write(|writer| {
        let name_key =
            writer.register_property_key("name", PropertyFamily::Element, PropertyType::Text)?;
        let mut elements = Vec::with_capacity(element_count);
        for _index in 0..element_count {
            elements.push(writer.create_element()?);
        }
        Ok((elements, name_key))
    })?;
    database.compact()?;
    database.set_checkpoint_policy(CheckpointPolicy::Manual);
    Ok((database, handles.0, handles.1))
}

/// Benchmarks one commit setting 1k fresh ~64-byte text values.
///
/// perf: this defends "a text property set is `O(text)` on the commit path".
/// Values change every iteration (a re-asserted equal value is an elided
/// no-op), so each measured commit logs and freezes all 1k texts.
fn bench_commit_text_properties(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_commit_text_properties");
    group.sample_size(20);
    let path = bench_path("text");
    let (mut database, elements, name_key) = unwrap(
        create_text_database(&path, 1_000),
        "benchmark fixture should build",
    );
    let mut round: u64 = 0;
    let padding = "x".repeat(47);
    group.bench_function("1k_values_64b", |b| {
        b.iter(|| {
            round += 1;
            commit_text_round(&mut database, &elements, name_key, round, &padding);
        });
    });
    group.finish();
}

/// Commits one round of fresh ~64-byte text values (the measured unit of
/// work); values embed the round so no set is elided as a no-op.
fn commit_text_round(
    database: &mut Db,
    elements: &[ElementId],
    name_key: PropertyKeyId,
    round: u64,
    padding: &str,
) {
    unwrap(
        database.write(|writer| {
            for (index, element) in elements.iter().enumerate() {
                writer.set(
                    PropertySubject::Element(*element),
                    Key::<Text>::from_id(name_key),
                    format!("{round:08}-{index:06}-{padding}"),
                )?;
            }
            Ok(())
        }),
        "text transaction should commit",
    );
}

criterion_group!(
    benches,
    bench_commit_single_element,
    bench_begin_write_rollback,
    bench_write_over_unfolded_overlay,
    bench_commit_text_properties
);
criterion_main!(benches);
