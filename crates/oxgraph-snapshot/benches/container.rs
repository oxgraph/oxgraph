//! Criterion benches for the topology-agnostic snapshot container.
//!
//! Defends three perf contracts (the corresponding doc claims live next to each item):
//! - `Snapshot::open` at `ValidationLevel::Layout` is `O(s^2)` in the section count, dominated by
//!   the duplicate-kind walk (see [`Snapshot::open_with`](oxgraph_snapshot::Snapshot::open_with)
//!   doc). The bench parameterises across `s ∈ {1, 16, 256, 1024 = MAX_SECTION_COUNT}` and uses
//!   `Throughput::Elements(s)` so a contract regression shows up as a sub-linear elements/sec curve
//!   rather than a hidden constant-factor blowup.
//! - `Snapshot::section` linear scan cost is `O(s)` per lookup; benched at the worst case (last
//!   kind, forcing the full table walk).
//! - `SnapshotBuilder::finish` is `O(s + total payload bytes)`; benched with payload size held
//!   constant so the bytes-per-second number tracks the linear-in-bytes term directly.

use std::hint::black_box;

use criterion::{
    BenchmarkGroup, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main,
    measurement::WallTime,
};
use oxgraph_snapshot::{Snapshot, SnapshotBuilder};

/// Builds a snapshot with `count` distinct sections, each carrying
/// `bytes_per_section` bytes. Used to size both `open` and `section`
/// lookup benches.
fn build_snapshot(count: u32, bytes_per_section: usize) -> Vec<u8> {
    let mut builder = SnapshotBuilder::new();
    for kind in 0..count {
        let payload = vec![(kind & 0xFF) as u8; bytes_per_section];
        match builder.add_section(kind, 0, 0, payload) {
            Ok(_) => {}
            Err(error) => panic!("bench builder: {error:?}"),
        }
    }
    match builder.finish() {
        Ok(bytes) => bytes,
        Err(error) => panic!("bench builder finish: {error:?}"),
    }
}

/// Benchmarks `Snapshot::open` across a range of section counts to
/// document the validation cost contract.
fn bench_open(group: &mut BenchmarkGroup<'_, WallTime>) {
    for &count in &[1u32, 16, 256, 1024] {
        let bytes = build_snapshot(count, 16);
        group.throughput(Throughput::Elements(u64::from(count)));
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &bytes,
            |bencher, snapshot_bytes| {
                bencher.iter(|| {
                    let snapshot = match Snapshot::open(black_box(snapshot_bytes)) {
                        Ok(value) => value,
                        Err(error) => panic!("bench open: {error:?}"),
                    };
                    black_box(snapshot);
                });
            },
        );
    }
}

/// Benchmarks `Snapshot::section` for the worst-case linear scan
/// (looking up the last section).
fn bench_section_lookup_last(group: &mut BenchmarkGroup<'_, WallTime>) {
    for &count in &[16u32, 256, 1024] {
        let bytes = build_snapshot(count, 16);
        let snapshot = match Snapshot::open(&bytes) {
            Ok(value) => value,
            Err(error) => panic!("bench open: {error:?}"),
        };
        let last_kind = count - 1;
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &(snapshot, last_kind),
            |bencher, (snap, kind)| {
                bencher.iter(|| {
                    let section = snap.section(black_box(*kind));
                    black_box(section);
                });
            },
        );
    }
}

/// Benchmarks `SnapshotBuilder::finish` across a range of total payload
/// sizes to document the linear-in-bytes contract.
fn bench_builder_finish(group: &mut BenchmarkGroup<'_, WallTime>) {
    for &(count, payload_size) in &[(16u32, 1024usize), (256, 1024), (1024, 1024)] {
        group.throughput(Throughput::Bytes(u64::from(count) * payload_size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{count}x{payload_size}")),
            &(count, payload_size),
            |bencher, &(c, p)| {
                bencher.iter(|| {
                    let snapshot = build_snapshot(c, p);
                    black_box(snapshot);
                });
            },
        );
    }
}

/// Top-level criterion entry: stitches together the three benchmark groups.
fn bench_container(criterion: &mut Criterion) {
    {
        let mut open_group = criterion.benchmark_group("snapshot_open");
        bench_open(&mut open_group);
        open_group.finish();
    }
    {
        let mut lookup_group = criterion.benchmark_group("section_lookup_last");
        bench_section_lookup_last(&mut lookup_group);
        lookup_group.finish();
    }
    {
        let mut finish_group = criterion.benchmark_group("builder_finish");
        bench_builder_finish(&mut finish_group);
        finish_group.finish();
    }
}

criterion_group!(benches, bench_container);
criterion_main!(benches);
