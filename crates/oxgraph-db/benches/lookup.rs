//! Criterion coverage for the P5 index-backed lookup contract.
//!
//! perf: these benchmarks defend "index lookup is `O(log n + matches)` —
//! sublinear in base size WHEN the match count is held fixed". Each lookup
//! (`property_equal`, `property_range`, composite, label / relation-type
//! membership) is benchmarked at growing base sizes; the per-call time must stay
//! roughly flat as the base grows by 10x, which is the observable signature of an
//! `O(log n + matches)` index rather than an `O(n)` scan. The base size is named
//! in every benchmark identifier.
//!
//! To isolate the `O(log n)` posting-PROBE cost from result materialization, the
//! match count is held CONSTANT across base sizes:
//!
//! * point equality / composite probe a single `rank`/`(rank, name)` value, which matches exactly
//!   one element regardless of base size;
//! * range probes a fixed width-100 window, so it materializes ~100 matches at every base size;
//! * the membership benches probe a RARE label and a RARE relation type ([`ANCHOR_COUNT`] carriers,
//!   fixed regardless of base) rather than a dense one — so they too measure the probe, not
//!   `Theta(base)` materialization.
//!
//! A membership probe over a DENSE label/type would instead be `O(matches)` with
//! `matches = Theta(base)`, which is genuinely linear and defends no sublinearity
//! claim; that is intentionally not what these benches measure.

use std::{fmt::Display, path::Path};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oxgraph_db::{
    Database, DbError, IndexDefinition, IndexId, IndexLookup, PropertyFamily, PropertyKeyId,
    PropertySubject, PropertyType, PropertyValue,
};

/// Base sizes the lookup benches sweep, spanning two decades so a flat per-call
/// time across them demonstrates sublinearity.
const BASE_SIZES: [usize; 3] = [1_000, 10_000, 100_000];

/// Number of elements/relations that carry the RARE membership anchor label and
/// relation type. Fixed regardless of base size so the membership benches probe a
/// posting whose match count never grows with the base — isolating the `O(log n)`
/// posting-probe cost from `O(matches)` materialization.
const ANCHOR_COUNT: usize = 8;

/// Unwraps a benchmark `Result`, panicking with `context` on error (benches must
/// not use `expect`, which the workspace lint table denies outside `#[test]`).
fn unwrap<T, E: Display>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error}"),
    }
}

/// Unwraps a benchmark `Option`, panicking with `context` when absent.
fn unwrap_some<T>(value: Option<T>, context: &str) -> T {
    value.unwrap_or_else(|| panic!("{context}"))
}

/// Builds a temporary benchmark path.
fn bench_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("oxgraph-db-lookup-{name}-{}", std::process::id()))
}

/// Removes an existing benchmark path.
fn clean(path: &Path) {
    match std::fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("remove benchmark fixture: {error}"),
    }
}

/// Cataloged ids the lookup benches probe.
struct Fixture {
    /// `rank` integer property key.
    rank_key: PropertyKeyId,
    /// Equality index over `rank`.
    rank_index: IndexId,
}

/// Creates a committed fixture with `element_count` ranked elements and a chain
/// of `Edge` relations, folded into the base so the lookups exercise the base
/// index (the `O(log n)` posting path).
///
/// A RARE `Anchor` label and a RARE `Anchor` relation type are carried by exactly
/// [`ANCHOR_COUNT`] elements/relations regardless of base size, so the membership
/// benches probe a posting whose match count is fixed — isolating the `O(log n)`
/// posting-probe cost from result materialization. The dense `Edge` type still
/// types every relation (it backs no membership bench here), so the relation
/// chain keeps the same shape across sizes.
fn create_fixture(path: &Path, element_count: usize) -> Result<Fixture, DbError> {
    clean(path);
    let mut database = Database::create(path)?;
    let mut writer = database.begin_write()?;
    let source = writer.register_role("source")?;
    let target = writer.register_role("target")?;
    let edge_type = writer.register_relation_type("Edge")?;
    let anchor_type = writer.register_relation_type("Anchor")?;
    let anchor_label = writer.register_label("Anchor")?;
    let rank_key =
        writer.register_property_key("rank", PropertyFamily::Element, PropertyType::Integer)?;
    let name_key =
        writer.register_property_key("name", PropertyFamily::Element, PropertyType::Text)?;
    let rank_index = writer.define_index(
        "rank_eq",
        IndexDefinition::PropertyEquality { key: rank_key },
    )?;
    writer.define_index(
        "anchor_type_idx",
        IndexDefinition::RelationType {
            relation_type: anchor_type,
        },
    )?;
    writer.define_index(
        "anchor_label_idx",
        IndexDefinition::Label {
            label: anchor_label,
        },
    )?;
    writer.define_index(
        "rank_name",
        IndexDefinition::CompositeEquality {
            keys: vec![rank_key, name_key],
        },
    )?;
    let mut elements = Vec::with_capacity(element_count);
    for index in 0..element_count {
        let element = writer.create_element()?;
        let rank = i64::try_from(index).map_err(|_error| DbError::IdOverflow)?;
        writer.set_property(
            PropertySubject::Element(element),
            rank_key,
            PropertyValue::Integer(rank),
        )?;
        writer.set_property(
            PropertySubject::Element(element),
            name_key,
            PropertyValue::Text(format!("e{index}")),
        )?;
        // Only the first `ANCHOR_COUNT` elements carry the rare label, so the
        // label posting matches a fixed number of elements at every base size.
        if index < ANCHOR_COUNT {
            writer.add_element_label(element, anchor_label)?;
        }
        elements.push(element);
    }
    for (index, window) in elements.windows(2).enumerate() {
        let relation = writer.create_relation()?;
        // Every relation is an `Edge` (dense, keeps the chain shape); only the
        // first `ANCHOR_COUNT` additionally carry the rare `Anchor` type the
        // membership bench probes.
        let relation_type = if index < ANCHOR_COUNT {
            anchor_type
        } else {
            edge_type
        };
        writer.set_relation_type(relation, relation_type)?;
        writer.create_incidence(relation, window[0], source)?;
        writer.create_incidence(relation, window[1], target)?;
    }
    writer.commit()?;
    // Fold the committed delta into the base so lookups hit the base index.
    database.checkpoint()?;
    Ok(Fixture {
        rank_key,
        rank_index,
    })
}

/// Opens a fixture database, panicking on setup failure.
fn fixture_or_panic(name: &str, element_count: usize) -> (Database, Fixture) {
    let path = bench_path(name);
    let fixture = unwrap(
        create_fixture(&path, element_count),
        "benchmark fixture should build",
    );
    let database = unwrap(Database::open(&path), "benchmark fixture should open");
    (database, fixture)
}

/// Resolves a named index id from the fixture catalog.
fn index_named(database: &Database, name: &str) -> IndexId {
    unwrap_some(
        database.begin_read().catalog().index_id(name),
        "index defined",
    )
}

/// Benchmarks property equality lookup across growing base sizes; the per-call
/// time must stay roughly flat (sublinear in base size).
fn bench_property_equal(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_property_equal_lookup");
    for size in BASE_SIZES {
        let (database, fixture) = fixture_or_panic("equal", size);
        let read = database.begin_read();
        let probe = PropertyValue::Integer(half(size));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _size| {
            b.iter(|| {
                unwrap(
                    read.lookup_property_equal(fixture.rank_key, &probe),
                    "equality lookup",
                )
            });
        });
    }
    group.finish();
}

/// Benchmarks the equality INDEX path across growing base sizes.
fn bench_index_equal(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_index_equal_lookup");
    for size in BASE_SIZES {
        let (database, fixture) = fixture_or_panic("index-equal", size);
        let read = database.begin_read();
        let probe = PropertyValue::Integer(half(size));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _size| {
            b.iter(|| {
                unwrap(
                    read.lookup_index(fixture.rank_index, IndexLookup::Equal(&probe)),
                    "index lookup",
                )
            });
        });
    }
    group.finish();
}

/// Benchmarks a fixed-width property RANGE lookup across growing base sizes; a
/// flat per-call time shows the range walks the ordered postings, not the base.
fn bench_property_range(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_property_range_lookup_width100");
    for size in BASE_SIZES {
        let (database, fixture) = fixture_or_panic("range", size);
        let read = database.begin_read();
        let lo = PropertyValue::Integer(half(size));
        let hi = PropertyValue::Integer(half(size) + 100);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _size| {
            b.iter(|| {
                unwrap(
                    read.lookup_property_range(fixture.rank_key, &lo, &hi),
                    "range lookup",
                )
            });
        });
    }
    group.finish();
}

/// Benchmarks an `IndexLookup::All` membership lookup (label / relation-type)
/// resolved by the named index, across growing base sizes. The probed index is a
/// RARE one ([`ANCHOR_COUNT`] carriers, fixed regardless of base), so the per-call
/// time stays flat (`O(log n)` posting probe) rather than growing with the base.
fn bench_membership_all(c: &mut Criterion, group_name: &str, fixture_tag: &str, index_name: &str) {
    let mut group = c.benchmark_group(group_name);
    for size in BASE_SIZES {
        let (database, _fixture) = fixture_or_panic(fixture_tag, size);
        let read = database.begin_read();
        let index = index_named(&database, index_name);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _size| {
            b.iter(|| {
                unwrap(
                    read.lookup_index(index, IndexLookup::All),
                    "membership lookup",
                )
            });
        });
    }
    group.finish();
}

/// Benchmarks rare-label membership across growing base sizes (fixed match count,
/// so the per-call time isolates the `O(log n)` posting probe).
fn bench_label_membership(c: &mut Criterion) {
    bench_membership_all(c, "db_label_membership_lookup", "label", "anchor_label_idx");
}

/// Benchmarks rare-relation-type membership across growing base sizes (fixed
/// match count, so the per-call time isolates the `O(log n)` posting probe).
fn bench_relation_type_membership(c: &mut Criterion) {
    bench_membership_all(
        c,
        "db_relation_type_membership_lookup",
        "rtype",
        "anchor_type_idx",
    );
}

/// Benchmarks composite (rank, name) equality lookup across growing base sizes.
fn bench_composite_equal(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_composite_equal_lookup");
    for size in BASE_SIZES {
        let (database, _fixture) = fixture_or_panic("composite", size);
        let read = database.begin_read();
        let composite_index = index_named(&database, "rank_name");
        let target = half(size);
        let values = [
            PropertyValue::Integer(target),
            PropertyValue::Text(format!("e{target}")),
        ];
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _size| {
            b.iter(|| {
                unwrap(
                    read.lookup_index(composite_index, IndexLookup::CompositeEqual(&values)),
                    "composite lookup",
                )
            });
        });
    }
    group.finish();
}

/// Returns the mid-range probe value for a base of `size` elements.
fn half(size: usize) -> i64 {
    i64::try_from(size / 2).unwrap_or(i64::MAX)
}

criterion_group!(
    benches,
    bench_property_equal,
    bench_index_equal,
    bench_property_range,
    bench_label_membership,
    bench_relation_type_membership,
    bench_composite_equal,
);
criterion_main!(benches);
