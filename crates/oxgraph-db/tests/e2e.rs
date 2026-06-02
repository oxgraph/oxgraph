//! End-to-end tests for the greenfield OXGDB product engine.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use oxgraph_algo::breadth_first_search;
use oxgraph_db::{
    Database, DbError, GraphProjectionDefinition, HypergraphProjectionDefinition, IndexDefinition,
    IndexLookup, ProjectionDefinition, PropertyFamily, PropertySubject, PropertyType,
    PropertyValue, QueryLanguage, QueryValue, TraversalDirection, TraversalOptions, TraversalRow,
};
use oxgraph_graph::{
    CanonicalElementIdentity, ElementSuccessors, LocalElementIdentity, TopologyCounts,
};
use oxgraph_hyper::{DirectedHyperedgeParticipants, DirectedVertexHyperedges};

/// Per-process path counter.
static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

/// Test error type.
#[derive(Debug)]
#[expect(
    dead_code,
    reason = "test harness reads error fields through derived Debug when a Result test fails"
)]
enum TestError {
    /// Database error.
    Db(DbError),
    /// Filesystem error.
    Io(std::io::Error),
    /// BFS error.
    Bfs(oxgraph_algo::BfsError),
}

impl From<DbError> for TestError {
    fn from(error: DbError) -> Self {
        Self::Db(error)
    }
}

impl From<std::io::Error> for TestError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<oxgraph_algo::BfsError> for TestError {
    fn from(error: oxgraph_algo::BfsError) -> Self {
        Self::Bfs(error)
    }
}

/// Builds a unique temporary database path.
fn temp_path(name: &str) -> PathBuf {
    let id = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("oxgraph-db-{name}-{}-{id}", std::process::id()))
}

/// Removes `path` when it exists.
fn clean(path: &Path) -> Result<(), TestError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[test]
fn greenfield_database_supports_topology_properties_queries_and_recovery() -> Result<(), TestError>
{
    let path = temp_path("complete-product");
    clean(&path)?;

    let mut database = Database::create(&path)?;
    let fixture = load_fixture(&mut database)?;
    database.compact()?;
    database.validate()?;

    let reopened = Database::open(&path)?;
    let read = reopened.begin_read();
    assert_eq!(read.element_count(), 3);
    assert_eq!(read.relation_count(), 2);
    assert_eq!(read.incidence_count(), 5);
    assert_eq!(
        read.property(PropertySubject::Element(fixture.alice), fixture.name_key),
        Some(&PropertyValue::Text("Alice".to_owned()))
    );

    let graph = read.graph_projection(fixture.graph_projection)?;
    let alice_local = graph
        .local_element_id(fixture.alice)
        .ok_or(DbError::UnknownElement { id: fixture.alice })?;
    let graph_neighbors = graph
        .element_successors(alice_local)
        .map(|local| graph.canonical_element_id(local))
        .collect::<Vec<_>>();
    assert_eq!(graph_neighbors, vec![fixture.bob]);

    assert_eq!(breadth_first_search(&graph, alice_local)?.count(), 2);

    let hyper = read.hypergraph_projection(fixture.hyper_projection)?;
    assert_eq!(hyper.relation_count(), 1);
    let meeting_local = oxgraph_db::ProjectionRelationId::new(0);
    let targets = hyper
        .target_participants(meeting_local)
        .map(|local| hyper.canonical_element_id(local))
        .collect::<Vec<_>>();
    assert_eq!(targets, vec![fixture.bob, fixture.carol]);
    let outgoing = hyper
        .outgoing_hyperedges(
            hyper
                .local_element_id(fixture.alice)
                .ok_or(DbError::UnknownElement { id: fixture.alice })?,
        )
        .count();
    assert_eq!(outgoing, 1);

    assert_query_counts(&reopened, &fixture)?;
    clean(&path)?;
    Ok(())
}

#[test]
fn rollback_and_empty_commits_do_not_reuse_committed_transaction_ids() -> Result<(), TestError> {
    let path = temp_path("transaction-id-burns");
    clean(&path)?;

    let mut database = Database::create(&path)?;
    let mut rolled_back = database.begin_write()?;
    rolled_back.register_role("source")?;
    rolled_back.rollback();

    let empty = database.begin_write()?;
    assert_eq!(empty.commit()?.get(), 0);

    let mut writer = database.begin_write()?;
    let role = writer.register_role("source")?;
    let element = writer.create_element()?;
    let relation = writer.create_relation()?;
    writer.create_incidence(relation, element, role)?;
    writer.commit()?;

    let mut reopened = Database::open(&path)?;
    let status = reopened.status();
    let mut writer = reopened.begin_write()?;
    let second = writer.create_element()?;
    writer.commit()?;
    assert!(reopened.status().last_transaction_id > status.last_transaction_id);

    let read = Database::open(&path)?.begin_read();
    assert!(read.contains_element(element));
    assert!(read.contains_element(second));
    clean(&path)?;
    Ok(())
}

#[test]
fn rollback_only_transaction_id_burns_are_session_local() -> Result<(), TestError> {
    let path = temp_path("rollback-session-local");
    clean(&path)?;

    let mut database = Database::create(&path)?;
    let durable_transaction_id = database.status().last_transaction_id;
    let mut rolled_back = database.begin_write()?;
    rolled_back.create_element()?;
    rolled_back.rollback();
    assert!(database.status().last_transaction_id > durable_transaction_id);

    let reopened = Database::open(&path)?;
    assert_eq!(
        reopened.status().last_transaction_id,
        durable_transaction_id
    );
    clean(&path)?;
    Ok(())
}

#[test]
fn index_lookup_uses_typed_composite_and_projection_semantics() -> Result<(), TestError> {
    let path = temp_path("index-lookup-semantics");
    clean(&path)?;

    let mut database = Database::create(&path)?;
    let fixture = load_fixture(&mut database)?;
    let read = database.begin_read();

    let tuple = [
        PropertyValue::Text("Alice".to_owned()),
        PropertyValue::Integer(42),
    ];
    assert_eq!(
        read.lookup_index(
            fixture.person_identity_index,
            IndexLookup::CompositeEqual(&tuple),
        )?,
        vec![PropertySubject::Element(fixture.alice)]
    );
    let wrong_arity = [PropertyValue::Text("Alice".to_owned())];
    assert!(matches!(
        read.lookup_index(
            fixture.person_identity_index,
            IndexLookup::CompositeEqual(&wrong_arity),
        ),
        Err(DbError::UnsupportedQuery { .. })
    ));
    let wrong_type = [
        PropertyValue::Text("Alice".to_owned()),
        PropertyValue::Text("42".to_owned()),
    ];
    assert!(matches!(
        read.lookup_index(
            fixture.person_identity_index,
            IndexLookup::CompositeEqual(&wrong_type),
        ),
        Err(DbError::PropertyTypeMismatch {
            expected: PropertyType::Integer,
            actual: PropertyType::Text,
        })
    ));

    assert_eq!(
        read.lookup_index(fixture.graph_projection_index, IndexLookup::All)?,
        vec![
            PropertySubject::Element(fixture.alice),
            PropertySubject::Element(fixture.bob),
            PropertySubject::Relation(fixture.knows),
        ]
    );
    assert_eq!(
        read.lookup_index(fixture.hyper_projection_index, IndexLookup::All)?,
        vec![
            PropertySubject::Element(fixture.alice),
            PropertySubject::Element(fixture.bob),
            PropertySubject::Element(fixture.carol),
            PropertySubject::Relation(fixture.meeting),
            PropertySubject::Incidence(fixture.meeting_source),
            PropertySubject::Incidence(fixture.meeting_bob),
            PropertySubject::Incidence(fixture.meeting_carol),
        ]
    );
    assert!(matches!(
        read.lookup_index(
            fixture.graph_projection_index,
            IndexLookup::Equal(&PropertyValue::Integer(1)),
        ),
        Err(DbError::UnsupportedQuery { .. })
    ));

    clean(&path)?;
    Ok(())
}

#[test]
fn property_lookup_values_are_schema_checked() -> Result<(), TestError> {
    let path = temp_path("typed-property-lookup");
    clean(&path)?;

    let mut database = Database::create(&path)?;
    let fixture = load_fixture(&mut database)?;
    let read = database.begin_read();

    assert!(matches!(
        read.lookup_property_equal(fixture.age_key, &PropertyValue::Text("42".to_owned())),
        Err(DbError::PropertyTypeMismatch {
            expected: PropertyType::Integer,
            actual: PropertyType::Text,
        })
    ));
    assert!(matches!(
        read.lookup_property_range(
            fixture.age_key,
            &PropertyValue::Integer(0),
            &PropertyValue::Text("99".to_owned()),
        ),
        Err(DbError::PropertyTypeMismatch {
            expected: PropertyType::Integer,
            actual: PropertyType::Text,
        })
    ));
    assert!(matches!(
        read.lookup_index(
            fixture.age_index,
            IndexLookup::Range {
                min: &PropertyValue::Text("0".to_owned()),
                max: &PropertyValue::Text("99".to_owned()),
            },
        ),
        Err(DbError::PropertyTypeMismatch {
            expected: PropertyType::Integer,
            actual: PropertyType::Text,
        })
    ));
    assert!(
        read.lookup_property_range(
            fixture.age_key,
            &PropertyValue::Integer(100),
            &PropertyValue::Integer(0),
        )?
        .is_empty()
    );

    clean(&path)?;
    Ok(())
}

#[test]
fn graph_traversal_api_walks_directions_and_depth() -> Result<(), TestError> {
    let (path, database, fixture) = create_traversal_database("graph-traversal-directions")?;
    let read = database.begin_read();

    let graph = read.graph_projection_by_name("calls")?;
    assert_eq!(graph.relation_count(), 3);
    assert!(matches!(
        read.graph_projection_by_name("missing"),
        Err(DbError::UnsupportedQuery { .. })
    ));
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.alice],
        TraversalOptions::default(),
        &[TraversalRow {
            element: fixture.bob,
            depth: 1,
        }],
    )?;
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.bob],
        TraversalOptions {
            direction: TraversalDirection::Incoming,
            ..TraversalOptions::default()
        },
        &[TraversalRow {
            element: fixture.alice,
            depth: 1,
        }],
    )?;
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.bob],
        TraversalOptions {
            direction: TraversalDirection::Both,
            ..TraversalOptions::default()
        },
        &[
            TraversalRow {
                element: fixture.carol,
                depth: 1,
            },
            TraversalRow {
                element: fixture.alice,
                depth: 1,
            },
        ],
    )?;
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.alice],
        TraversalOptions {
            max_depth: 2,
            ..TraversalOptions::default()
        },
        &[
            TraversalRow {
                element: fixture.bob,
                depth: 1,
            },
            TraversalRow {
                element: fixture.carol,
                depth: 2,
            },
        ],
    )?;

    clean(&path)?;
    Ok(())
}

#[test]
fn graph_traversal_api_handles_seeds_and_limits() -> Result<(), TestError> {
    let (path, database, fixture) = create_traversal_database("graph-traversal-limits")?;
    let read = database.begin_read();

    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.alice, fixture.bob],
        TraversalOptions {
            max_depth: 2,
            include_start: true,
            ..TraversalOptions::default()
        },
        &[
            TraversalRow {
                element: fixture.alice,
                depth: 0,
            },
            TraversalRow {
                element: fixture.bob,
                depth: 0,
            },
            TraversalRow {
                element: fixture.carol,
                depth: 1,
            },
        ],
    )?;
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.alice],
        TraversalOptions {
            include_start: true,
            ..TraversalOptions::default()
        },
        &[
            TraversalRow {
                element: fixture.alice,
                depth: 0,
            },
            TraversalRow {
                element: fixture.bob,
                depth: 1,
            },
        ],
    )?;
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.alice],
        TraversalOptions {
            max_depth: 2,
            limit: 1,
            ..TraversalOptions::default()
        },
        &[TraversalRow {
            element: fixture.bob,
            depth: 1,
        }],
    )?;
    assert!(
        read.traverse_graph(fixture.graph_projection, &[], TraversalOptions::default(),)?
            .rows()
            .is_empty()
    );

    clean(&path)?;
    Ok(())
}

#[test]
fn graph_traversal_api_rejects_invalid_inputs() -> Result<(), TestError> {
    let (path, database, fixture) = create_traversal_database("graph-traversal-errors")?;
    let read = database.begin_read();

    assert!(matches!(
        read.traverse_graph(
            fixture.graph_projection,
            &[fixture.dave],
            TraversalOptions::default(),
        ),
        Err(DbError::UnknownElement { id }) if id == fixture.dave
    ));
    assert!(matches!(
        read.traverse_graph(
            oxgraph_db::ProjectionId::new(999),
            &[fixture.alice],
            TraversalOptions::default(),
        ),
        Err(DbError::UnknownProjection { .. })
    ));
    assert!(matches!(
        read.traverse_graph(
            fixture.hyper_projection,
            &[fixture.alice],
            TraversalOptions::default(),
        ),
        Err(DbError::InvalidProjection { .. })
    ));

    clean(&path)?;
    Ok(())
}

#[test]
fn oxql_graph_walk_executes_valid_queries() -> Result<(), TestError> {
    let (path, database, fixture) = create_traversal_database("oxql-graph-walk-valid")?;
    let read = database.begin_read();

    assert_eq!(
        execute_element_query(
            &database,
            &read,
            &format!("GRAPH calls WALK FROM {} DEPTH 2", fixture.alice.get()),
        )?,
        vec![fixture.bob, fixture.carol]
    );
    assert_eq!(
        execute_element_query(
            &database,
            &read,
            &format!(
                "GRAPH calls WALK FROM {} DEPTH 1 DIRECTION incoming",
                fixture.bob.get()
            ),
        )?,
        vec![fixture.alice]
    );
    assert_eq!(
        execute_element_query(
            &database,
            &read,
            &format!(
                "GRAPH calls WALK FROM {} DEPTH 1 DIRECTION both LIMIT 100",
                fixture.bob.get()
            ),
        )?,
        vec![fixture.carol, fixture.alice]
    );
    assert_eq!(
        execute_element_query(
            &database,
            &read,
            &format!("GRAPH calls NEIGHBORS {}", fixture.alice.get()),
        )?,
        vec![fixture.bob]
    );

    clean(&path)?;
    Ok(())
}

#[test]
fn oxql_graph_walk_rejects_invalid_queries() -> Result<(), TestError> {
    let (path, database, fixture) = create_traversal_database("oxql-graph-walk-invalid")?;

    assert!(matches!(
        database.prepare(
            QueryLanguage::Oxql,
            &format!(
                "GRAPH calls WALK FROM {} DEPTH 1 DIRECTION sideways",
                fixture.alice.get()
            ),
        ),
        Err(DbError::UnsupportedQuery { .. })
    ));
    assert!(matches!(
        database.prepare(
            QueryLanguage::Oxql,
            &format!("GRAPH calls WALK FROM {} DEPTH nope", fixture.alice.get()),
        ),
        Err(DbError::UnsupportedQuery { .. })
    ));
    assert!(matches!(
        database.prepare(
            QueryLanguage::Oxql,
            &format!(
                "GRAPH calls WALK FROM {} DEPTH 1 LIMIT nope",
                fixture.alice.get()
            ),
        ),
        Err(DbError::UnsupportedQuery { .. })
    ));
    assert!(matches!(
        database.prepare(
            QueryLanguage::Oxql,
            &format!("GRAPH missing WALK FROM {} DEPTH 1", fixture.alice.get()),
        ),
        Err(DbError::UnsupportedQuery { .. })
    ));
    assert!(matches!(
        database.prepare(
            QueryLanguage::Oxql,
            &format!(
                "GRAPH calls_hyper WALK FROM {} DEPTH 1",
                fixture.alice.get()
            ),
        ),
        Err(DbError::InvalidProjection { .. })
    ));

    clean(&path)?;
    Ok(())
}

#[test]
fn corrupt_store_bytes_fail_validation() -> Result<(), TestError> {
    let path = temp_path("corrupt-store");
    clean(&path)?;

    let mut database = Database::create(&path)?;
    load_fixture(&mut database)?;
    let store_path = path.join("store.oxgdb");
    let file = std::fs::OpenOptions::new().append(true).open(store_path)?;
    file.set_len(file.metadata()?.len() + 1)?;

    assert!(Database::open(&path).is_err());
    clean(&path)?;
    Ok(())
}

#[test]
fn concurrent_writers_are_rejected_until_release() -> Result<(), TestError> {
    let path = temp_path("writer-lock");
    clean(&path)?;

    let mut first = Database::create(&path)?;
    let writer = first.begin_write()?;

    let mut second = Database::open(&path)?;
    assert!(matches!(second.begin_write(), Err(DbError::WriterLockHeld)));

    // Releasing the held writer frees the single-writer lock.
    drop(writer);
    assert!(second.begin_write().is_ok());

    clean(&path)?;
    Ok(())
}

/// Loads a traversal-focused graph fixture.
fn load_traversal_fixture(database: &mut Database) -> Result<TraversalFixtureIds, DbError> {
    let mut writer = database.begin_write()?;
    let source_role = writer.register_role("source")?;
    let target_role = writer.register_role("target")?;
    let calls_type = writer.register_relation_type("Calls")?;
    let meeting_type = writer.register_relation_type("Meeting")?;
    let alice = writer.create_element()?;
    let bob = writer.create_element()?;
    let carol = writer.create_element()?;
    let dave = writer.create_element()?;
    let roles = (source_role, target_role);
    create_directed_relation(&mut writer, calls_type, roles, (alice, bob))?;
    create_directed_relation(&mut writer, calls_type, roles, (bob, carol))?;
    create_directed_relation(&mut writer, calls_type, roles, (carol, alice))?;
    let meeting = writer.create_relation()?;
    writer.set_relation_type(meeting, meeting_type)?;
    writer.create_incidence(meeting, alice, source_role)?;
    writer.create_incidence(meeting, bob, target_role)?;
    writer.create_incidence(meeting, carol, target_role)?;
    let graph_projection =
        writer.define_projection(ProjectionDefinition::Graph(GraphProjectionDefinition {
            name: "calls".to_owned(),
            relation_types: BTreeSet::from([calls_type]),
            source_role,
            target_role,
        }))?;
    let hyper_projection = writer.define_projection(ProjectionDefinition::Hypergraph(
        HypergraphProjectionDefinition {
            name: "calls_hyper".to_owned(),
            relation_types: BTreeSet::from([meeting_type]),
            source_roles: BTreeSet::from([source_role]),
            target_roles: BTreeSet::from([target_role]),
        },
    ))?;
    writer.commit()?;
    Ok(TraversalFixtureIds {
        alice,
        bob,
        carol,
        dave,
        graph_projection,
        hyper_projection,
    })
}

/// Creates a traversal test database.
fn create_traversal_database(
    name: &str,
) -> Result<(PathBuf, Database, TraversalFixtureIds), TestError> {
    let path = temp_path(name);
    clean(&path)?;
    let mut database = Database::create(&path)?;
    let fixture = load_traversal_fixture(&mut database)?;
    Ok((path, database, fixture))
}

/// Creates one directed binary relation.
fn create_directed_relation(
    writer: &mut oxgraph_db::WriteTransaction<'_>,
    relation_type: oxgraph_db::RelationTypeId,
    roles: (oxgraph_db::RoleId, oxgraph_db::RoleId),
    endpoints: (oxgraph_db::ElementId, oxgraph_db::ElementId),
) -> Result<(), DbError> {
    let relation = writer.create_relation()?;
    writer.set_relation_type(relation, relation_type)?;
    let (source_role, target_role) = roles;
    let (source, target) = endpoints;
    writer.create_incidence(relation, source, source_role)?;
    writer.create_incidence(relation, target, target_role)?;
    Ok(())
}

/// Asserts graph traversal rows.
fn assert_traversal(
    read: &oxgraph_db::ReadTransaction,
    projection: oxgraph_db::ProjectionId,
    seeds: &[oxgraph_db::ElementId],
    options: TraversalOptions,
    expected: &[TraversalRow],
) -> Result<(), DbError> {
    assert_eq!(
        read.traverse_graph(projection, seeds, options)?.rows(),
        expected
    );
    Ok(())
}

/// Executes an `OxQL` query returning one element value per row.
fn execute_element_query(
    database: &Database,
    read: &oxgraph_db::ReadTransaction,
    query: &str,
) -> Result<Vec<oxgraph_db::ElementId>, DbError> {
    let prepared = database.prepare(QueryLanguage::Oxql, query)?;
    Ok(read
        .execute(&prepared)?
        .rows()
        .iter()
        .filter_map(|row| match row.values.as_slice() {
            [QueryValue::Element(element)] => Some(*element),
            _values => None,
        })
        .collect())
}

/// Loads a graph and hypergraph fixture.
fn load_fixture(database: &mut Database) -> Result<FixtureIds, DbError> {
    let mut writer = database.begin_write()?;
    let source_role = writer.register_role("source")?;
    let target_role = writer.register_role("target")?;
    let person_label = writer.register_label("Person")?;
    let knows_type = writer.register_relation_type("Knows")?;
    let meeting_type = writer.register_relation_type("Meeting")?;
    let name_key =
        writer.register_property_key("name", PropertyFamily::Element, PropertyType::Text)?;
    let age_key =
        writer.register_property_key("age", PropertyFamily::Element, PropertyType::Integer)?;
    writer.register_property_key(
        "relation_weight",
        PropertyFamily::Relation,
        PropertyType::Integer,
    )?;
    writer.register_property_key(
        "incidence_note",
        PropertyFamily::Incidence,
        PropertyType::Text,
    )?;

    let (alice, bob, carol) = create_people(&mut writer, person_label, name_key, age_key)?;

    let knows = writer.create_relation()?;
    writer.set_relation_type(knows, knows_type)?;
    writer.create_incidence(knows, alice, source_role)?;
    writer.create_incidence(knows, bob, target_role)?;

    let meeting = writer.create_relation()?;
    writer.set_relation_type(meeting, meeting_type)?;
    let meeting_source = writer.create_incidence(meeting, alice, source_role)?;
    let meeting_bob = writer.create_incidence(meeting, bob, target_role)?;
    let meeting_carol = writer.create_incidence(meeting, carol, target_role)?;

    let graph_projection =
        writer.define_projection(ProjectionDefinition::Graph(GraphProjectionDefinition {
            name: "knows_graph".to_owned(),
            relation_types: BTreeSet::from([knows_type]),
            source_role,
            target_role,
        }))?;
    let hyper_projection = writer.define_projection(ProjectionDefinition::Hypergraph(
        HypergraphProjectionDefinition {
            name: "meeting_hyper".to_owned(),
            relation_types: BTreeSet::from([meeting_type]),
            source_roles: BTreeSet::from([source_role]),
            target_roles: BTreeSet::from([target_role]),
        },
    ))?;
    let indexes = define_fixture_indexes(
        &mut writer,
        FixtureIndexInputs {
            person_label,
            name_key,
            age_key,
            graph_projection,
            hyper_projection,
        },
    )?;
    writer.commit()?;

    Ok(FixtureIds {
        alice,
        bob,
        carol,
        knows,
        meeting,
        meeting_source,
        meeting_bob,
        meeting_carol,
        name_key,
        age_key,
        graph_projection,
        hyper_projection,
        age_index: indexes.age,
        person_identity_index: indexes.person_identity,
        graph_projection_index: indexes.graph_projection,
        hyper_projection_index: indexes.hyper_projection,
    })
}

/// Defines the fixture indexes.
fn define_fixture_indexes(
    writer: &mut oxgraph_db::WriteTransaction<'_>,
    inputs: FixtureIndexInputs,
) -> Result<FixtureIndexIds, DbError> {
    writer.define_index(
        "person_label",
        IndexDefinition::Label {
            label: inputs.person_label,
        },
    )?;
    writer.define_index(
        "name_eq",
        IndexDefinition::PropertyEquality {
            key: inputs.name_key,
        },
    )?;
    Ok(FixtureIndexIds {
        age: writer.define_index(
            "age_range",
            IndexDefinition::PropertyRange {
                key: inputs.age_key,
            },
        )?,
        person_identity: writer.define_index(
            "person_identity",
            IndexDefinition::CompositeEquality {
                keys: vec![inputs.name_key, inputs.age_key],
            },
        )?,
        graph_projection: writer.define_index(
            "knows_projection",
            IndexDefinition::Projection {
                projection: inputs.graph_projection,
            },
        )?,
        hyper_projection: writer.define_index(
            "meeting_projection",
            IndexDefinition::Projection {
                projection: inputs.hyper_projection,
            },
        )?,
    })
}

/// Creates the fixture people and their properties.
fn create_people(
    writer: &mut oxgraph_db::WriteTransaction<'_>,
    person_label: oxgraph_db::LabelId,
    name_key: oxgraph_db::PropertyKeyId,
    age_key: oxgraph_db::PropertyKeyId,
) -> Result<
    (
        oxgraph_db::ElementId,
        oxgraph_db::ElementId,
        oxgraph_db::ElementId,
    ),
    DbError,
> {
    let alice = writer.create_element()?;
    let bob = writer.create_element()?;
    let carol = writer.create_element()?;
    for element in [alice, bob, carol] {
        writer.add_element_label(element, person_label)?;
    }
    writer.set_property(
        PropertySubject::Element(alice),
        name_key,
        PropertyValue::Text("Alice".to_owned()),
    )?;
    writer.set_property(
        PropertySubject::Element(bob),
        name_key,
        PropertyValue::Text("Bob".to_owned()),
    )?;
    writer.set_property(
        PropertySubject::Element(alice),
        age_key,
        PropertyValue::Integer(42),
    )?;
    Ok((alice, bob, carol))
}

/// Asserts compound `WHERE` predicate coverage over the fixture.
///
/// Verifies that `OR` unions, `AND` intersects, ordered comparisons work, `AND`
/// binds tighter than `OR`, parentheses override that precedence, and malformed
/// predicates are rejected at prepare time.
fn assert_compound_where(
    database: &Database,
    read: &oxgraph_db::ReadTransaction,
    fixture: &FixtureIds,
) -> Result<(), DbError> {
    assert_eq!(
        execute_element_query(
            database,
            read,
            "MATCH ELEMENTS WHERE name = 'Alice' OR name = 'Bob'",
        )?
        .len(),
        2,
    );
    assert_eq!(
        execute_element_query(
            database,
            read,
            "MATCH ELEMENTS WHERE name = 'Alice' AND age = 42"
        )?,
        vec![fixture.alice],
    );
    assert_eq!(
        execute_element_query(database, read, "MATCH ELEMENTS WHERE age >= 42")?,
        vec![fixture.alice],
    );
    assert!(execute_element_query(database, read, "MATCH ELEMENTS WHERE age > 42")?.is_empty());
    // AND binds tighter: `Bob AND age=42` is false, so only Alice matches.
    assert_eq!(
        execute_element_query(
            database,
            read,
            "MATCH ELEMENTS WHERE name = 'Bob' AND age = 42 OR name = 'Alice'",
        )?,
        vec![fixture.alice],
    );
    // Parentheses override precedence: `(Alice OR Bob) AND age=42` is just Alice.
    assert_eq!(
        execute_element_query(
            database,
            read,
            "MATCH ELEMENTS WHERE ( name = 'Alice' OR name = 'Bob' ) AND age = 42",
        )?,
        vec![fixture.alice],
    );
    assert!(matches!(
        database.prepare(QueryLanguage::Oxql, "MATCH ELEMENTS WHERE name ="),
        Err(DbError::UnsupportedQuery { .. })
    ));
    assert!(matches!(
        database.prepare(QueryLanguage::Oxql, "MATCH ELEMENTS WHERE ( name = 'Alice'"),
        Err(DbError::UnsupportedQuery { .. })
    ));
    Ok(())
}

/// Asserts query-language coverage over the fixture.
fn assert_query_counts(database: &Database, fixture: &FixtureIds) -> Result<(), DbError> {
    let read = database.begin_read();
    let elements = database.prepare(QueryLanguage::Oxql, "MATCH ELEMENTS")?;
    assert_eq!(read.execute(&elements)?.rows().len(), 3);

    let people = database.prepare(QueryLanguage::Oxql, "MATCH ELEMENTS HAS LABEL Person")?;
    assert_eq!(read.execute(&people)?.rows().len(), 3);

    let alice = database.prepare(QueryLanguage::Oxql, "MATCH ELEMENTS WHERE name = 'Alice'")?;
    let rows = read.execute(&alice)?;
    assert_eq!(rows.rows().len(), 1);
    assert_eq!(
        rows.rows()[0].values,
        vec![QueryValue::Element(fixture.alice)]
    );
    assert!(matches!(
        database.prepare(QueryLanguage::Oxql, "MATCH ELEMENTS WHERE age = '42'"),
        Err(DbError::PropertyTypeMismatch {
            expected: PropertyType::Integer,
            actual: PropertyType::Text,
        })
    ));
    assert!(matches!(
        database.prepare(
            QueryLanguage::Oxql,
            "MATCH ELEMENTS WHERE relation_weight = 1",
        ),
        Err(DbError::WrongPropertyFamily {
            expected: PropertyFamily::Relation,
            actual: PropertyFamily::Element,
        })
    ));
    assert!(matches!(
        database.prepare(
            QueryLanguage::Oxql,
            "MATCH ELEMENTS WHERE incidence_note = 'source'",
        ),
        Err(DbError::WrongPropertyFamily {
            expected: PropertyFamily::Incidence,
            actual: PropertyFamily::Element,
        })
    ));

    assert_compound_where(database, &read, fixture)?;

    let knows = database.prepare(QueryLanguage::Oxql, "MATCH RELATIONS TYPE Knows")?;
    assert_eq!(read.execute(&knows)?.rows().len(), 1);

    let neighbors = database.prepare(
        QueryLanguage::Oxql,
        &format!("GRAPH knows_graph NEIGHBORS {}", fixture.alice.get()),
    )?;
    assert_eq!(
        read.execute(&neighbors)?.rows()[0].values,
        vec![QueryValue::Element(fixture.bob)]
    );

    let cypher_nodes = database.prepare(QueryLanguage::Cypher, "MATCH (n:Person) RETURN n")?;
    assert_eq!(read.execute(&cypher_nodes)?.rows().len(), 3);

    let cypher_edges =
        database.prepare(QueryLanguage::Cypher, "MATCH (n)-[r]->(m) RETURN n,r,m")?;
    assert_eq!(read.execute(&cypher_edges)?.rows().len(), 1);
    Ok(())
}

/// Fixture index definition inputs.
#[derive(Clone, Copy)]
struct FixtureIndexInputs {
    /// Person label ID.
    person_label: oxgraph_db::LabelId,
    /// Name property key ID.
    name_key: oxgraph_db::PropertyKeyId,
    /// Age property key ID.
    age_key: oxgraph_db::PropertyKeyId,
    /// Graph projection ID.
    graph_projection: oxgraph_db::ProjectionId,
    /// Hypergraph projection ID.
    hyper_projection: oxgraph_db::ProjectionId,
}

/// Fixture index ID bundle.
struct FixtureIndexIds {
    /// Age range index ID.
    age: oxgraph_db::IndexId,
    /// Composite identity index ID.
    person_identity: oxgraph_db::IndexId,
    /// Graph projection index ID.
    graph_projection: oxgraph_db::IndexId,
    /// Hypergraph projection index ID.
    hyper_projection: oxgraph_db::IndexId,
}

/// Traversal fixture ID bundle.
struct TraversalFixtureIds {
    /// Alice element ID.
    alice: oxgraph_db::ElementId,
    /// Bob element ID.
    bob: oxgraph_db::ElementId,
    /// Carol element ID.
    carol: oxgraph_db::ElementId,
    /// Dave element ID outside the graph projection.
    dave: oxgraph_db::ElementId,
    /// Graph projection ID.
    graph_projection: oxgraph_db::ProjectionId,
    /// Hypergraph projection ID.
    hyper_projection: oxgraph_db::ProjectionId,
}

/// Fixture ID bundle.
struct FixtureIds {
    /// Alice element ID.
    alice: oxgraph_db::ElementId,
    /// Bob element ID.
    bob: oxgraph_db::ElementId,
    /// Carol element ID.
    carol: oxgraph_db::ElementId,
    /// Knows relation ID.
    knows: oxgraph_db::RelationId,
    /// Meeting relation ID.
    meeting: oxgraph_db::RelationId,
    /// Meeting source incidence ID.
    meeting_source: oxgraph_db::IncidenceId,
    /// Meeting Bob incidence ID.
    meeting_bob: oxgraph_db::IncidenceId,
    /// Meeting Carol incidence ID.
    meeting_carol: oxgraph_db::IncidenceId,
    /// Name property key ID.
    name_key: oxgraph_db::PropertyKeyId,
    /// Age property key ID.
    age_key: oxgraph_db::PropertyKeyId,
    /// Graph projection ID.
    graph_projection: oxgraph_db::ProjectionId,
    /// Hypergraph projection ID.
    hyper_projection: oxgraph_db::ProjectionId,
    /// Age range index ID.
    age_index: oxgraph_db::IndexId,
    /// Composite identity index ID.
    person_identity_index: oxgraph_db::IndexId,
    /// Graph projection index ID.
    graph_projection_index: oxgraph_db::IndexId,
    /// Hypergraph projection index ID.
    hyper_projection_index: oxgraph_db::IndexId,
}
