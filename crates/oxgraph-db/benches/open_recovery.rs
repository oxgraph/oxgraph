//! Criterion coverage for greenfield `oxgraph-db` public inputs.
//!
//! perf: each benchmark names the input size in its benchmark identifier.

use std::{collections::BTreeSet, path::Path};

use criterion::{Criterion, criterion_group, criterion_main};
use oxgraph_db::{
    Database, DbError, GraphProjectionDefinition, HypergraphProjectionDefinition, IndexDefinition,
    IndexLookup, ProjectionDefinition, PropertyFamily, PropertySubject, PropertyType,
    PropertyValue, QueryLanguage, TraversalDirection, TraversalOptions,
};
use oxgraph_graph::{ElementSuccessors, LocalElementIdentity};
use oxgraph_hyper::DirectedHyperedgeParticipants;

/// Builds a temporary benchmark path.
fn bench_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("oxgraph-db-{name}-{}", std::process::id()))
}

/// Removes an existing benchmark path.
fn clean(path: &Path) {
    match std::fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("remove benchmark fixture: {error}"),
    }
}

/// Creates a committed database fixture.
fn create_fixture(path: &Path, element_count: usize) -> Result<Fixture, DbError> {
    clean(path);
    let mut database = Database::create(path)?;
    let mut writer = database.begin_write()?;
    let source = writer.register_role("source")?;
    let target = writer.register_role("target")?;
    let edge_type = writer.register_relation_type("Edge")?;
    let rank_key =
        writer.register_property_key("rank", PropertyFamily::Element, PropertyType::Integer)?;
    let mut elements = Vec::with_capacity(element_count);
    for index in 0..element_count {
        let element = writer.create_element()?;
        writer.set_property(
            PropertySubject::Element(element),
            rank_key,
            PropertyValue::Integer(i64::try_from(index).map_err(|_error| DbError::IdOverflow)?),
        )?;
        elements.push(element);
    }
    for window in elements.windows(2) {
        let relation = writer.create_relation()?;
        writer.set_relation_type(relation, edge_type)?;
        writer.create_incidence(relation, window[0], source)?;
        writer.create_incidence(relation, window[1], target)?;
    }
    let graph_projection =
        writer.define_projection(ProjectionDefinition::Graph(GraphProjectionDefinition {
            name: "edge_graph".to_owned(),
            relation_types: BTreeSet::from([edge_type]),
            source_role: source,
            target_role: target,
        }))?;
    let hyper_projection = writer.define_projection(ProjectionDefinition::Hypergraph(
        HypergraphProjectionDefinition {
            name: "edge_hyper".to_owned(),
            relation_types: BTreeSet::from([edge_type]),
            source_roles: BTreeSet::from([source]),
            target_roles: BTreeSet::from([target]),
        },
    ))?;
    let rank_index = writer.define_index(
        "rank_eq",
        IndexDefinition::PropertyEquality { key: rank_key },
    )?;
    writer.commit()?;
    Ok(Fixture {
        graph_projection,
        hyper_projection,
        rank_key,
        rank_index,
        first: elements[0],
    })
}

/// Opens a fixture database.
fn open_fixture(name: &str, element_count: usize) -> Result<(Database, Fixture), DbError> {
    let path = bench_path(name);
    let fixture = create_fixture(&path, element_count)?;
    Database::open(&path).map(|database| (database, fixture))
}

/// Benchmarks opening a 1k-element store.
fn bench_open_recovery(c: &mut Criterion) {
    let path = bench_path("open-recovery");
    if let Err(error) = create_fixture(&path, 1_000) {
        panic!("benchmark fixture should build: {error}");
    }
    c.bench_function("db_open_recovery_1k", |b| {
        b.iter(|| {
            if let Err(error) = Database::open(&path) {
                panic!("database should open: {error}");
            }
        });
    });
}

/// Benchmarks preparing an `OxQL` query.
fn bench_prepare(c: &mut Criterion) {
    let (database, _fixture) = fixture_or_panic("prepare", 1_000);
    c.bench_function("db_prepare_oxql_1k_catalog", |b| {
        b.iter(|| {
            if let Err(error) = database.prepare(QueryLanguage::Oxql, "MATCH ELEMENTS") {
                panic!("query should prepare: {error}");
            }
        });
    });
}

/// Benchmarks creating a reader pin.
fn bench_begin_read(c: &mut Criterion) {
    let (database, _fixture) = fixture_or_panic("begin-read", 1_000);
    c.bench_function("db_begin_read_1k", |b| {
        b.iter(|| database.begin_read());
    });
}

/// Benchmarks creating and rolling back an empty writer.
fn bench_begin_write(c: &mut Criterion) {
    let (mut database, _fixture) = fixture_or_panic("begin-write", 1_000);
    c.bench_function("db_begin_write_rollback_1k", |b| {
        b.iter(|| match database.begin_write() {
            Ok(writer) => writer.rollback(),
            Err(error) => panic!("writer should start: {error}"),
        });
    });
}

/// Benchmarks executing an element scan.
fn bench_execute_scan(c: &mut Criterion) {
    let (database, _fixture) = fixture_or_panic("execute-scan", 1_000);
    let read = database.begin_read();
    let query = match database.prepare(QueryLanguage::Oxql, "MATCH ELEMENTS") {
        Ok(query) => query,
        Err(error) => panic!("query should prepare: {error}"),
    };
    c.bench_function("db_execute_elements_1k", |b| {
        b.iter(|| {
            if let Err(error) = read.execute(&query) {
                panic!("query should execute: {error}");
            }
        });
    });
}

/// Benchmarks committing single-element transactions.
fn bench_commit_single_element(c: &mut Criterion) {
    let path = bench_path("commit-single");
    clean(&path);
    let mut database = match Database::create(&path) {
        Ok(database) => database,
        Err(error) => panic!("database should create: {error}"),
    };
    c.bench_function("db_commit_single_element", |b| {
        b.iter(|| {
            let mut writer = match database.begin_write() {
                Ok(writer) => writer,
                Err(error) => panic!("writer should start: {error}"),
            };
            if let Err(error) = writer.create_element() {
                panic!("element should create: {error}");
            }
            if let Err(error) = writer.commit() {
                panic!("transaction should commit: {error}");
            }
        });
    });
}

/// Benchmarks graph CSR-style traversal.
fn bench_graph_traversal(c: &mut Criterion) {
    let (database, fixture) = fixture_or_panic("graph-traversal", 1_000);
    let read = database.begin_read();
    let graph = match read.graph_projection(fixture.graph_projection) {
        Ok(graph) => graph,
        Err(error) => panic!("graph projection should build: {error}"),
    };
    let Some(first) = graph.local_element_id(fixture.first) else {
        panic!("first element should be projected");
    };
    c.bench_function("db_graph_csr_successors_1k", |b| {
        b.iter(|| graph.element_successors(first).count());
    });
}

/// Benchmarks the public graph traversal API.
fn bench_graph_traversal_api(c: &mut Criterion) {
    let (database, fixture) = fixture_or_panic("graph-traversal-api", 1_000);
    let read = database.begin_read();
    c.bench_function("db_traverse_graph_api_1k_depth4_limit128", |b| {
        b.iter(|| {
            if let Err(error) = read.traverse_graph(
                fixture.graph_projection,
                &[fixture.first],
                TraversalOptions {
                    max_depth: 4,
                    direction: TraversalDirection::Outgoing,
                    limit: 128,
                    include_start: false,
                },
            ) {
                panic!("graph traversal should succeed: {error}");
            }
        });
    });
}

/// Benchmarks hypergraph BCSR-style traversal.
fn bench_hypergraph_traversal(c: &mut Criterion) {
    let (database, fixture) = fixture_or_panic("hyper-traversal", 1_000);
    let read = database.begin_read();
    let hyper = match read.hypergraph_projection(fixture.hyper_projection) {
        Ok(hyper) => hyper,
        Err(error) => panic!("hypergraph projection should build: {error}"),
    };
    c.bench_function("db_hyper_bcsr_targets_1k", |b| {
        b.iter(|| {
            hyper
                .target_participants(oxgraph_db::ProjectionRelationId::new(0))
                .count()
        });
    });
}

/// Benchmarks property equality lookup.
fn bench_property_lookup(c: &mut Criterion) {
    let (database, fixture) = fixture_or_panic("property-lookup", 1_000);
    let read = database.begin_read();
    c.bench_function("db_property_equality_lookup_1k", |b| {
        b.iter(|| {
            if let Err(error) =
                read.lookup_property_equal(fixture.rank_key, &PropertyValue::Integer(500))
            {
                panic!("property lookup should succeed: {error}");
            }
        });
    });
    c.bench_function("db_index_equality_lookup_1k", |b| {
        b.iter(|| {
            if let Err(error) = read.lookup_index(
                fixture.rank_index,
                IndexLookup::Equal(&PropertyValue::Integer(500)),
            ) {
                panic!("index lookup should succeed: {error}");
            }
        });
    });
}

/// Opens a fixture or panics for benchmark setup.
fn fixture_or_panic(name: &str, element_count: usize) -> (Database, Fixture) {
    match open_fixture(name, element_count) {
        Ok(fixture) => fixture,
        Err(error) => panic!("benchmark fixture should open: {error}"),
    }
}

/// Benchmark fixture IDs.
struct Fixture {
    /// Graph projection ID.
    graph_projection: oxgraph_db::ProjectionId,
    /// Hypergraph projection ID.
    hyper_projection: oxgraph_db::ProjectionId,
    /// Rank property key ID.
    rank_key: oxgraph_db::PropertyKeyId,
    /// Rank index ID.
    rank_index: oxgraph_db::IndexId,
    /// First element ID.
    first: oxgraph_db::ElementId,
}

criterion_group!(
    benches,
    bench_open_recovery,
    bench_prepare,
    bench_begin_read,
    bench_begin_write,
    bench_execute_scan,
    bench_commit_single_element,
    bench_graph_traversal,
    bench_graph_traversal_api,
    bench_hypergraph_traversal,
    bench_property_lookup
);
criterion_main!(benches);
