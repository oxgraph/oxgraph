//! Tests for the database engine: open/recovery latency contracts, the
//! checkpoint crash matrix, transaction semantics, and the read surface.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{maintenance::CheckpointStop, *};
use crate::{
    ElementId, EqualityIndex, IndexDefinition, Key, LabelId, PropertyFamily, PropertyKeyId,
    PropertySubject, PropertyType,
};

/// Per-process path counter for unique temporary store directories.
static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

/// Returns a unique temporary store path and removes any prior contents.
fn temp_store(name: &str) -> PathBuf {
    let id = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("oxgraph-db-cp-{name}-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// Manual measurement harness (run with
/// `cargo test -p oxgraph-db --release -- --ignored open_latency_large_base
/// --nocapture`): builds a folded base at roughly the measured-problem scale
/// (>=100k elements, >=300k relations, properties), then times `Db::open`.
/// Open must be dominated by the record decode + page faults, NOT the
/// `O(base)` index rebuild the prior design paid — the index is borrowed.
/// Number of element/relation records the open-latency harness builds and the
/// number of timed open runs it averages.
#[cfg(not(debug_assertions))]
const OPEN_LATENCY_ELEMENTS: usize = 100_000;
/// Relations the open-latency harness builds (each with two incidences).
#[cfg(not(debug_assertions))]
const OPEN_LATENCY_RELATIONS: usize = 320_000;
/// Timed open runs the open-latency harness averages.
#[cfg(not(debug_assertions))]
const OPEN_LATENCY_RUNS: u32 = 5;

/// Populates `database` with `OPEN_LATENCY_ELEMENTS` ranked elements and
/// `OPEN_LATENCY_RELATIONS` weighted relations (two incidences each).
#[cfg(not(debug_assertions))]
fn populate_large_store(database: &mut Db) {
    database.set_checkpoint_policy(CheckpointPolicy::Manual);
    database
        .write(|writer| {
            let rank = writer.register_property_key(
                "rank",
                PropertyFamily::Element,
                PropertyType::Integer,
            )?;
            let weight = writer.register_property_key(
                "weight",
                PropertyFamily::Relation,
                PropertyType::Integer,
            )?;
            let role = writer.register_role("party")?;
            let mut elements = Vec::with_capacity(OPEN_LATENCY_ELEMENTS);
            for index in 0..OPEN_LATENCY_ELEMENTS {
                let element = writer.create_element()?;
                writer.set_property(
                    PropertySubject::Element(element),
                    rank,
                    PropertyValue::Integer(i64::try_from(index % 997).unwrap_or(0)),
                )?;
                elements.push(element);
            }
            for index in 0..OPEN_LATENCY_RELATIONS {
                let relation = writer.create_relation()?;
                writer.set_property(
                    PropertySubject::Relation(relation),
                    weight,
                    PropertyValue::Integer(i64::try_from(index % 503).unwrap_or(0)),
                )?;
                let source = elements[index % OPEN_LATENCY_ELEMENTS];
                let target = elements[(index + 1) % OPEN_LATENCY_ELEMENTS];
                writer.create_incidence(relation, source, role)?;
                writer.create_incidence(relation, target, role)?;
            }
            Ok(())
        })
        .expect("populate");
    // Fold everything into the base so open pays the base term, not log replay.
    database.compact().expect("compact");
}

/// Mean elapsed time of `OPEN_LATENCY_RUNS` full `Db::open` calls on `path`.
#[cfg(not(debug_assertions))]
fn mean_open_ms(path: &std::path::Path) -> f64 {
    let mut total = std::time::Duration::ZERO;
    for _run in 0..OPEN_LATENCY_RUNS {
        let start = std::time::Instant::now();
        let opened = Db::open(path).expect("timed open");
        total += start.elapsed();
        drop(opened);
    }
    total.as_secs_f64() * 1000.0 / f64::from(OPEN_LATENCY_RUNS)
}

/// Mean elapsed time of the prior design's open-time heavy work — record
/// decode + `from_records` index rebuild (`BaseRecords::from_view`) — over
/// `OPEN_LATENCY_RUNS` runs, the BEFORE proxy for the borrowed open.
#[cfg(not(debug_assertions))]
fn mean_old_from_view_ms(path: &std::path::Path) -> f64 {
    let superblock = crate::wal::read_superblock(path).expect("superblock");
    let base_path = path.join(base_file(superblock.base_generation.get()));
    let mut total = std::time::Duration::ZERO;
    for _run in 0..OPEN_LATENCY_RUNS {
        let base = crate::backing::Base::open(&base_path, false).expect("base open");
        let start = std::time::Instant::now();
        let records = crate::overlay::BaseRecords::from_view(base.get()).expect("old from_view");
        total += start.elapsed();
        drop(records);
        drop(base);
    }
    total.as_secs_f64() * 1000.0 / f64::from(OPEN_LATENCY_RUNS)
}

/// Manual measurement harness (run with
/// `cargo test -p oxgraph-db --release -- --ignored open_latency_large_base
/// --nocapture`): builds a folded base at roughly the measured-problem scale
/// (>=100k elements, >=300k relations, properties), then times `Db::open`.
/// Open must be dominated by the record decode + page faults, NOT the
/// `O(base)` index rebuild the prior design paid — the index is borrowed.
/// Debug builds skip it (the open-time `debug_assert!` differential check
/// would itself rebuild the index and skew the timing); run in `--release`.
#[test]
#[ignore = "manual perf measurement; run explicitly with --release --ignored --nocapture"]
#[cfg(not(debug_assertions))]
fn open_latency_large_base() {
    let path = temp_store("open-latency");
    let mut database = Db::create(&path).expect("create");
    populate_large_store(&mut database);
    drop(database);

    let _warm = Db::open(&path).expect("warm open");
    let after_ms = mean_open_ms(&path);
    let before_ms = mean_old_from_view_ms(&path);

    println!(
        "open_latency_large_base: {OPEN_LATENCY_ELEMENTS} elements, \
         {OPEN_LATENCY_RELATIONS} relations, {} incidences, {} properties",
        OPEN_LATENCY_RELATIONS * 2,
        OPEN_LATENCY_ELEMENTS + OPEN_LATENCY_RELATIONS,
    );
    println!("  BEFORE open work (decode + from_records rebuild): {before_ms:.1} ms / open");
    println!("  AFTER  full Db::open (decode + BORROWED index):   {after_ms:.1} ms / open");

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn reconcile_upserts_reuse_or_mint_and_retain_prunes_the_complement() {
    let path = temp_store("reconcile");
    let mut database = Db::create(&path).expect("create");
    let index = {
        let mut writer = database.begin_write().expect("begin write");
        let key = writer
            .register_property_key("stable_key", PropertyFamily::Element, PropertyType::Text)
            .expect("key");
        let index = writer
            .define_index(
                "element_stable_key_eq",
                IndexDefinition::PropertyEquality { key },
            )
            .expect("index");
        writer.commit().expect("commit schema");
        index
    };
    let eq = EqualityIndex::<crate::Text>::from_id(index);

    let (a1, b1) = {
        let mut writer = database.begin_write().expect("begin write");
        let a = writer.upsert_element(eq, "a").expect("upsert a");
        let b = writer.upsert_element(eq, "b").expect("upsert b");
        writer.commit().expect("commit");
        (a, b)
    };

    let (a2, c1) = {
        let mut writer = database.begin_write().expect("begin write");
        let a = writer.upsert_element(eq, "a").expect("re-upsert a");
        let c = writer.upsert_element(eq, "c").expect("upsert c");
        writer.retain(eq, &["a", "c"]).expect("retain");
        writer.commit().expect("commit");
        (a, c)
    };

    assert_eq!(a1, a2, "an unchanged identity reuses its element id");
    assert_ne!(c1, a1);
    assert_ne!(c1, b1);

    let read = database.reader();
    assert!(read.contains_element(a1), "kept a");
    assert!(read.contains_element(c1), "kept c");
    assert!(!read.contains_element(b1), "retain tombstoned b");
    assert_eq!(
        read.element_by_key(eq, "a")
            .expect("lookup a")
            .map(|element| element.id),
        Some(a1)
    );
    assert!(
        read.element_by_key(eq, "b").expect("lookup b").is_none(),
        "b is not resolvable after the prune"
    );
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn write_closure_commits_on_ok_rolls_back_on_err_and_reports_outcome() {
    let path = temp_store("write-closure");
    let mut database = Db::create(&path).expect("create");

    // Ok with a change → committed; the read closure observes it.
    let (id, outcome) = database
        .write(|writer| {
            let id = writer.create_element()?;
            Ok(id)
        })
        .expect("write");
    assert!(matches!(outcome, CommitOutcome::Committed(_)));
    database
        .read(|read| {
            assert!(read.contains_element(id));
            Ok(())
        })
        .expect("read");

    // A no-op write reports Empty (no frame appended).
    let ((), outcome) = database.write(|_writer| Ok(())).expect("empty write");
    assert_eq!(outcome, CommitOutcome::Empty);

    // An Err from the closure rolls back the staged delta.
    let before = database
        .read(|read| Ok(read.element_count()))
        .expect("count");
    let result = database.write(|writer| {
        writer.create_element()?;
        Err::<(), DbError>(DbError::Query(crate::error::QueryError::Empty))
    });
    assert!(result.is_err());
    let after = database
        .read(|read| Ok(read.element_count()))
        .expect("count");
    assert_eq!(before, after, "the failed write staged nothing durable");

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn re_setting_an_unchanged_property_value_is_a_no_op_commit() {
    // The reconcile/reindex contract: re-asserting a property's existing value
    // must log NO mutation, so an incremental reconcile that re-sets every
    // property of every unchanged subject stays O(change). Without the no-op
    // gate the commit logs the whole graph every reindex.
    let path = temp_store("set-noop");
    let mut database = Db::create(&path).expect("create");
    let schema = Schema::new().key::<crate::Text>("name", PropertyFamily::Element);

    // Create an element and set its name (a real change → committed).
    let id = database
        .write(|writer| {
            let bound = writer.apply_schema(&schema)?;
            let name = bound.key::<crate::Text>("name")?;
            let id = writer.create_element()?;
            writer.set(id, name, "alpha")?;
            Ok(id)
        })
        .expect("first write")
        .0;

    // Re-asserting the SAME value mutates nothing → the commit is Empty.
    let ((), outcome) = database
        .write(|writer| {
            let bound = writer.apply_schema(&schema)?;
            let name = bound.key::<crate::Text>("name")?;
            writer.set(id, name, "alpha")?;
            Ok(())
        })
        .expect("idempotent set");
    assert_eq!(
        outcome,
        CommitOutcome::Empty,
        "re-setting the same property value must log no mutation"
    );

    // Setting a DIFFERENT value is a real change → committed, and visible.
    let ((), outcome) = database
        .write(|writer| {
            let bound = writer.apply_schema(&schema)?;
            let name = bound.key::<crate::Text>("name")?;
            writer.set(id, name, "beta")?;
            Ok(())
        })
        .expect("changed set");
    assert!(matches!(outcome, CommitOutcome::Committed(_)));
    let name = database
        .bind(&schema)
        .expect("bind")
        .key::<crate::Text>("name")
        .expect("name key");
    let value = database
        .read(|read| {
            Ok(read
                .element(id)
                .and_then(|element| element.properties().get::<crate::Text, String>(name)))
        })
        .expect("read");
    assert_eq!(value.as_deref(), Some("beta"));

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn schema_apply_is_idempotent_and_bind_resolves_typed_handles() {
    let path = temp_store("schema");
    let mut database = Db::create(&path).expect("create");
    let schema = Schema::new()
        .label("function")
        .key::<crate::Text>("name", PropertyFamily::Element)
        .equality_index("name_eq", "name");

    // First apply registers the catalog and upserts two elements by identity.
    let (alpha, beta) = database
        .write(|writer| {
            let bound = writer.apply_schema(&schema)?;
            let name_eq = bound.equality_index::<crate::Text>("name_eq")?;
            let function = bound.label("function")?;
            let alpha = writer.upsert_element(name_eq, "alpha")?;
            writer.add_label(alpha, function)?;
            let beta = writer.upsert_element(name_eq, "beta")?;
            Ok((alpha, beta))
        })
        .expect("apply + write")
        .0;
    assert_ne!(alpha, beta);

    // Re-applying the same schema is idempotent: nothing new registers, so the
    // commit is empty.
    let (_bound, outcome) = database
        .write(|writer| writer.apply_schema(&schema))
        .expect("re-apply");
    assert_eq!(
        outcome,
        CommitOutcome::Empty,
        "re-applying a schema registers nothing new"
    );

    // bind() resolves the schema read-only on a reopened store; the typed
    // handle round-trips, and a wrong value type is rejected.
    let reopened = Db::open(&path).expect("open");
    let bound = reopened.bind(&schema).expect("bind");
    let name_eq = bound
        .equality_index::<crate::Text>("name_eq")
        .expect("typed index");
    assert!(
        bound.equality_index::<crate::Int>("name_eq").is_err(),
        "a wrong-value-type handle request is a SchemaConflict"
    );
    let found = reopened
        .read(|read| read.element_by_key(name_eq, "alpha"))
        .expect("read")
        .expect("alpha present");
    assert_eq!(found.id, alpha);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn hypergraph_projection_schema_round_trips_and_ranks() {
    let path = temp_store("hyper-schema");
    let mut database = Db::create(&path).expect("create");
    let schema = Schema::new()
        .role("source")
        .role("target")
        .relation_type("Meeting")
        .key::<crate::Text>("meeting_key", PropertyFamily::Relation)
        .equality_index("meeting_eq", "meeting_key")
        .hypergraph_projection("meetings", &["Meeting"], &["source"], &["target"]);

    // The builder declares the hypergraph projection; applying it registers the
    // catalog and an n-ary relation {alice -> bob, carol} upserts in one verb.
    let (projection, alice) = database
        .write(|writer| {
            let bound = writer.apply_schema(&schema)?;
            let meeting_eq = bound.equality_index::<crate::Text>("meeting_eq")?;
            let meeting_type = bound.relation_type("Meeting")?;
            let source = bound.role("source")?;
            let target = bound.role("target")?;
            let alice = writer.create_element()?;
            let bob = writer.create_element()?;
            let carol = writer.create_element()?;
            writer.upsert_relation(
                meeting_eq,
                "m1",
                meeting_type,
                &[(alice, source), (bob, target), (carol, target)],
            )?;
            Ok((bound.projection("meetings")?, alice))
        })
        .expect("apply + write")
        .0;

    // Re-applying the same schema registers nothing new (idempotent).
    let (_bound, outcome) = database
        .write(|writer| writer.apply_schema(&schema))
        .expect("re-apply");
    assert_eq!(outcome, CommitOutcome::Empty);

    // The builder-declared hypergraph projection materializes and ranks: three
    // participants and the single hyperedge.
    let ranked = database
        .read(|read| {
            read.personalized_hypergraph_pagerank(
                projection,
                &[alice],
                crate::PageRankConfig::new(0.85_f64, 1e-6_f64, 100),
            )
        })
        .expect("hyper pagerank");
    assert_eq!(ranked.elements.len(), 3);
    assert_eq!(ranked.relations.len(), 1);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn schema_fingerprint_is_order_independent_and_change_sensitive() {
    let base = Schema::new()
        .role("source")
        .role("target")
        .label("function");
    // Declaration order does not change the fingerprint.
    let reordered = Schema::new()
        .label("function")
        .role("target")
        .role("source");
    assert_eq!(base.fingerprint(), reordered.fingerprint());
    // Adding a declared item changes it.
    assert_ne!(base.fingerprint(), base.label("module").fingerprint());
}

/// The exact logical state the crash-matrix asserts recovery preserves: the
/// visible element ids, the rank-keyed property values, and the `Person`
/// label membership.
#[derive(Debug, Eq, PartialEq)]
struct LogicalState {
    /// Visible element ids in ascending order.
    elements: Vec<ElementId>,
    /// Subjects whose `rank` equals each probed value, by value.
    rank_eq_500: Vec<PropertySubject>,
    /// Element ids carrying the `Person` label.
    person_members: Vec<ElementId>,
}

/// Catalog/topology fixture ids returned by [`build_fixture`].
struct Fixture {
    /// `rank` integer property key.
    rank: PropertyKeyId,
    /// `Person` label.
    person: LabelId,
}

/// Builds a committed fixture: 8 elements, each ranked `index * 100`, the
/// even-indexed ones labelled `Person`. Returns the fixture ids.
fn build_fixture(database: &mut Db) -> Fixture {
    let mut writer = database.begin_write().expect("begin write");
    let rank = writer
        .register_property_key("rank", PropertyFamily::Element, PropertyType::Integer)
        .expect("rank key");
    let person = writer.register_label("Person").expect("person label");
    for index in 0..8u64 {
        let element = writer.create_element().expect("element");
        writer
            .set(
                element,
                Key::<crate::Int>::from_id(rank),
                i64::try_from(index).expect("index") * 100,
            )
            .expect("set rank");
        if index % 2 == 0 {
            writer.add_label(element, person).expect("add label");
        }
    }
    writer.commit().expect("commit fixture");
    Fixture { rank, person }
}

/// Reads the logical state through the index-backed read surface.
fn read_logical(database: &Db, fixture: &Fixture) -> LogicalState {
    let read = database.reader();
    let elements = read.element_ids();
    let rank_eq_500 = read
        .lookup_property_equal(fixture.rank, &PropertyValue::Integer(500))
        .expect("rank lookup");
    let person_members = read.snapshot.view().elements_with_label(fixture.person);
    LogicalState {
        elements,
        rank_eq_500,
        person_members,
    }
}

/// Asserts ids are never reused across a fold BEHAVIORALLY: the next element
/// `database` mints must take the id one past the current maximum visible
/// element id, i.e. the recovered watermark survived the fold. A regression
/// that dropped the watermark on fold (so the recovered record set is
/// unchanged but the next-id counter reset) would reuse an existing id and
/// fail this assertion — which the unchanged-record-set checks alone miss.
///
/// The probe element is rolled back, so it does not perturb the logical state
/// the surrounding test re-reads.
fn assert_no_id_reuse_across_fold(database: &mut Db) {
    let max_existing = database
        .reader()
        .element_ids()
        .into_iter()
        .map(ElementId::get)
        .max()
        .unwrap_or(0);
    let expected = ElementId::new(max_existing + 1);
    let mut writer = database.begin_write().expect("watermark probe writer");
    let minted = writer.create_element().expect("watermark probe element");
    assert_eq!(
        minted, expected,
        "the next minted id must be one past the max existing id (watermark \
         survived the fold; ids are never reused)",
    );
    // Drop the probe writer so it leaves no trace in the logical state.
    drop(writer);
}

/// CHECKPOINT-CRASH-MATRIX: a crash after each fsync point in `checkpoint`
/// recovers EXACTLY the correct logical state. After a crash before the
/// superblock lands, the OLD generation stays authoritative (the orphan new
/// base is ignored); after a crash once the superblock names the new
/// generation, the NEW base is authoritative. The completed checkpoint
/// recovers the same logical state from the folded base. In every case the
/// index-backed lookups return the same answers as before the (attempted)
/// fold.
#[test]
fn checkpoint_crash_matrix_recovers_exact_state() {
    for stop in [
        CheckpointStop::BeforeSuperblock,
        CheckpointStop::BeforeRotate,
        CheckpointStop::Complete,
    ] {
        let path = temp_store(&format!("crash-{stop:?}"));
        let mut database = Db::create(&path).expect("create");
        let fixture = build_fixture(&mut database);
        let before = read_logical(&database, &fixture);
        let before_generation = database.base_generation;

        // Simulate a crash at `stop`: the checkpoint returns right after the
        // chosen fsync, leaving the intermediate files in place. We then drop
        // the handle (as a crash would) and reopen from disk.
        database
            .checkpoint_inner(stop)
            .expect("checkpoint stop returns ok");
        drop(database);

        let mut recovered = Db::open(&path).expect("reopen after crash");
        let after = read_logical(&recovered, &fixture);
        assert_eq!(
            after, before,
            "crash at {stop:?} must recover the exact logical state",
        );

        // The recovered watermark survives every crash window: the next minted
        // id is one past the max recovered element id, so ids are never reused
        // across the (attempted) fold — asserted behaviorally, not merely
        // inferred from the unchanged record set.
        assert_no_id_reuse_across_fold(&mut recovered);

        // Generation expectation per crash window.
        match stop {
            CheckpointStop::BeforeSuperblock => assert_eq!(
                recovered.base_generation, before_generation,
                "old superblock stays authoritative before the new one lands",
            ),
            CheckpointStop::BeforeRotate | CheckpointStop::Complete => assert_eq!(
                recovered.base_generation,
                before_generation + 1,
                "the new superblock names the folded generation",
            ),
        }

        // A second open is idempotent (orphan files from a partial crash do
        // not derail a repeat recovery).
        let reopened = Db::open(&path).expect("second reopen");
        assert_eq!(read_logical(&reopened, &fixture), before);

        drop(reopened);
        let _ = std::fs::remove_dir_all(&path);
    }
}

/// The auto-checkpoint policy folds the delta-log into a fresh base once the
/// log outgrows the base by the configured factor: under a tiny factor, a
/// run of dirty commits advances the live generation (the log was folded),
/// and the logical state is preserved across the fold. The manual policy
/// never auto-folds.
#[test]
fn auto_checkpoint_policy_folds_when_log_outgrows_base() {
    // Manual policy: many commits, generation never advances on its own.
    let manual_path = temp_store("auto-manual");
    let mut manual = Db::create(&manual_path).expect("create manual");
    manual.set_checkpoint_policy(CheckpointPolicy::Manual);
    let _fixture = build_fixture(&mut manual);
    for _ in 0..200 {
        let mut writer = manual.begin_write().expect("writer");
        writer.create_element().expect("element");
        writer.commit().expect("commit");
    }
    assert_eq!(
        manual.live_generation(),
        CheckpointGeneration::new(0),
        "manual policy must never auto-fold",
    );
    drop(manual);
    let _ = std::fs::remove_dir_all(&manual_path);

    // Size-ratio policy with the smallest factor: the log soon outgrows the
    // tiny base floor, so a run of commits triggers at least one fold.
    let auto_path = temp_store("auto-ratio");
    let mut auto = Db::create(&auto_path).expect("create auto");
    auto.set_checkpoint_policy(CheckpointPolicy::SizeRatio { factor: 1 });
    let fixture = build_fixture(&mut auto);
    let before = read_logical(&auto, &fixture);
    for _ in 0..400 {
        let mut writer = auto.begin_write().expect("writer");
        writer.create_element().expect("element");
        writer.commit().expect("commit");
    }
    assert!(
        auto.live_generation() > CheckpointGeneration::new(0),
        "size-ratio policy must auto-fold once the log outgrows the base",
    );
    // The pre-existing logical state survives every fold; the policy is also
    // surfaced in status and preserved across the fold.
    let after = read_logical(&auto, &fixture);
    assert_eq!(after.rank_eq_500, before.rank_eq_500);
    assert_eq!(after.person_members, before.person_members);
    // Ids are never reused across the auto-fold: the next minted id is one
    // past the max existing id (the watermark folded into the new base).
    assert_no_id_reuse_across_fold(&mut auto);
    assert_eq!(
        auto.checkpoint_policy(),
        CheckpointPolicy::SizeRatio { factor: 1 },
        "the auto-fold reopen must preserve the configured policy",
    );
    // Status surfaces the live generation and the (now small) log size.
    let status = auto.stats();
    assert_eq!(status.live_generation, auto.live_generation());
    assert!(status.base_byte_size > 0, "live base has bytes");
    drop(auto);
    let _ = std::fs::remove_dir_all(&auto_path);
}
