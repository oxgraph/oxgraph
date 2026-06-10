//! End-to-end tests for the greenfield OXGDB product engine.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use oxgraph_algo::breadth_first_search;
use oxgraph_db::{
    CommitOutcome, Db, DbError, Direction, GraphProjectionDefinition,
    HypergraphProjectionDefinition, IndexDefinition, IndexProbe, Int, Key, PageRankConfig,
    ProjectionDefinition, PropertyFamily, PropertySubject, PropertyType, PropertyValue, QueryValue,
    Text, TraversedNode, Walk,
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
    /// Db error.
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

    let mut database = Db::create(&path)?;
    let fixture = load_fixture(&mut database)?;
    database.compact()?;
    database.validate()?;

    let reopened = Db::open(&path)?;
    let read = reopened.reader();
    assert_eq!(read.element_count(), 3);
    assert_eq!(read.relation_count(), 2);
    assert_eq!(read.incidence_count(), 5);
    assert_eq!(
        read.property(PropertySubject::Element(fixture.alice), fixture.name_key),
        Some(PropertyValue::from("Alice"))
    );

    let graph = read.graph_projection(fixture.graph_projection)?;
    let alice_local = graph.local_element_id(fixture.alice).ok_or_else(|| {
        DbError::Catalog(oxgraph_db::CatalogError::UnknownId {
            family: oxgraph_db::IdFamily::Element,
            id: fixture.alice.get(),
        })
    })?;
    let graph_neighbors = graph
        .element_successors(alice_local)
        .map(|local| graph.canonical_element_id(local))
        .collect::<Vec<_>>();
    assert_eq!(graph_neighbors, vec![fixture.bob]);

    assert_eq!(breadth_first_search(&graph, alice_local)?.count(), 2);

    let hyper = read.hypergraph_projection(fixture.hyper_projection)?;
    assert_eq!(hyper.relation_count(), 1);
    let meeting_local = oxgraph_db::projection::ProjectionRelationId::new(0);
    let targets = hyper
        .target_participants(meeting_local)
        .map(|local| hyper.canonical_element_id(local))
        .collect::<Vec<_>>();
    assert_eq!(targets, vec![fixture.bob, fixture.carol]);
    let outgoing = hyper
        .outgoing_hyperedges(hyper.local_element_id(fixture.alice).ok_or_else(|| {
            DbError::Catalog(oxgraph_db::CatalogError::UnknownId {
                family: oxgraph_db::IdFamily::Element,
                id: fixture.alice.get(),
            })
        })?)
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

    let mut database = Db::create(&path)?;
    let _ = database.write(|rolled_back| {
        rolled_back.register_role("source")?;
        Err::<(), DbError>(DbError::Query(oxgraph_db::QueryError::Empty))
    });

    let ((), outcome) = database.write(|_empty| Ok(()))?;
    assert!(matches!(outcome, CommitOutcome::Empty));

    let (element, _outcome) = database.write(|writer| {
        let role = writer.register_role("source")?;
        let element = writer.create_element()?;
        let relation = writer.create_relation()?;
        writer.create_incidence(relation, element, role)?;
        Ok(element)
    })?;

    let mut reopened = Db::open(&path)?;
    let status = reopened.stats();
    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "the method path cannot satisfy write's for<'a> Fn bound over Writer<'a>"
    )]
    let (second, _outcome) = reopened.write(|writer| writer.create_element())?;
    assert!(reopened.stats().last_transaction_id > status.last_transaction_id);

    let read = Db::open(&path)?.reader();
    assert!(read.contains_element(element));
    assert!(read.contains_element(second));
    clean(&path)?;
    Ok(())
}

#[test]
fn rollback_only_transaction_id_burns_are_session_local() -> Result<(), TestError> {
    let path = temp_path("rollback-session-local");
    clean(&path)?;

    let mut database = Db::create(&path)?;
    let durable_transaction_id = database.stats().last_transaction_id;
    let _ = database.write(|rolled_back| {
        rolled_back.create_element()?;
        Err::<(), DbError>(DbError::Query(oxgraph_db::QueryError::Empty))
    });
    assert!(database.stats().last_transaction_id > durable_transaction_id);

    let reopened = Db::open(&path)?;
    assert_eq!(reopened.stats().last_transaction_id, durable_transaction_id);
    clean(&path)?;
    Ok(())
}

#[test]
fn index_lookup_uses_typed_composite_and_projection_semantics() -> Result<(), TestError> {
    let path = temp_path("index-lookup-semantics");
    clean(&path)?;

    let mut database = Db::create(&path)?;
    let fixture = load_fixture(&mut database)?;
    let read = database.reader();

    let tuple = [PropertyValue::from("Alice"), PropertyValue::Integer(42)];
    assert_eq!(
        read.lookup(fixture.person_identity_index, IndexProbe::Composite(&tuple),)?,
        vec![PropertySubject::Element(fixture.alice)]
    );
    let wrong_arity = [PropertyValue::from("Alice")];
    assert!(matches!(
        read.lookup(
            fixture.person_identity_index,
            IndexProbe::Composite(&wrong_arity),
        ),
        Err(DbError::Query(oxgraph_db::QueryError::Unsupported { .. }))
    ));
    let wrong_type = [PropertyValue::from("Alice"), PropertyValue::from("42")];
    assert!(matches!(
        read.lookup(
            fixture.person_identity_index,
            IndexProbe::Composite(&wrong_type),
        ),
        Err(DbError::Query(
            oxgraph_db::QueryError::PropertyTypeMismatch {
                expected: PropertyType::Integer,
                actual: PropertyType::Text,
            }
        ))
    ));

    assert_eq!(
        read.lookup(fixture.graph_projection_index, IndexProbe::All)?,
        vec![
            PropertySubject::Element(fixture.alice),
            PropertySubject::Element(fixture.bob),
            PropertySubject::Relation(fixture.knows),
        ]
    );
    assert_eq!(
        read.lookup(fixture.hyper_projection_index, IndexProbe::All)?,
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
        read.lookup(
            fixture.graph_projection_index,
            IndexProbe::Equal(&PropertyValue::Integer(1)),
        ),
        Err(DbError::Query(oxgraph_db::QueryError::Unsupported { .. }))
    ));

    clean(&path)?;
    Ok(())
}

#[test]
fn property_lookup_values_are_schema_checked() -> Result<(), TestError> {
    let path = temp_path("typed-property-lookup");
    clean(&path)?;

    let mut database = Db::create(&path)?;
    let fixture = load_fixture(&mut database)?;
    let read = database.reader();

    assert!(matches!(
        read.lookup_property_equal(fixture.age_key, &PropertyValue::from("42")),
        Err(DbError::Query(
            oxgraph_db::QueryError::PropertyTypeMismatch {
                expected: PropertyType::Integer,
                actual: PropertyType::Text,
            }
        ))
    ));
    assert!(matches!(
        read.lookup_property_range(
            fixture.age_key,
            &PropertyValue::Integer(0),
            &PropertyValue::from("99"),
        ),
        Err(DbError::Query(
            oxgraph_db::QueryError::PropertyTypeMismatch {
                expected: PropertyType::Integer,
                actual: PropertyType::Text,
            }
        ))
    ));
    assert!(matches!(
        read.lookup(
            fixture.age_index,
            IndexProbe::Range {
                min: &PropertyValue::from("0"),
                max: &PropertyValue::from("99"),
            },
        ),
        Err(DbError::Query(
            oxgraph_db::QueryError::PropertyTypeMismatch {
                expected: PropertyType::Integer,
                actual: PropertyType::Text,
            }
        ))
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
    let read = database.reader();

    let graph = read.graph_projection_by_name("calls")?;
    assert_eq!(graph.relation_count(), 3);
    assert!(matches!(
        read.graph_projection_by_name("missing"),
        Err(DbError::Query(oxgraph_db::QueryError::Unsupported { .. }))
    ));
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.alice],
        Walk::default(),
        &[TraversedNode {
            element: fixture.bob,
            depth: 1,
        }],
    )?;
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.bob],
        Walk {
            direction: Direction::Incoming,
            ..Walk::default()
        },
        &[TraversedNode {
            element: fixture.alice,
            depth: 1,
        }],
    )?;
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.bob],
        Walk {
            direction: Direction::Both,
            ..Walk::default()
        },
        &[
            TraversedNode {
                element: fixture.carol,
                depth: 1,
            },
            TraversedNode {
                element: fixture.alice,
                depth: 1,
            },
        ],
    )?;
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.alice],
        Walk {
            max_depth: 2,
            ..Walk::default()
        },
        &[
            TraversedNode {
                element: fixture.bob,
                depth: 1,
            },
            TraversedNode {
                element: fixture.carol,
                depth: 2,
            },
        ],
    )?;

    clean(&path)?;
    Ok(())
}

#[test]
fn personalized_pagerank_ranks_and_responds_to_seeds() -> Result<(), TestError> {
    let (path, database, fixture) = create_traversal_database("graph-pagerank")?;
    let read = database.reader();

    let seeded = read.personalized_pagerank(
        fixture.graph_projection,
        &[fixture.alice],
        PageRankConfig::new(0.85_f64, 1e-6_f64, 100),
    )?;

    // One score per projection element, ordered from highest rank to lowest.
    assert!(!seeded.is_empty());
    for window in seeded.windows(2) {
        assert!(window[0].1 >= window[1].1);
    }
    // PageRank is a probability distribution over the visible elements.
    let total: f64 = seeded.iter().map(|(_, rank)| rank).sum();
    assert!(
        (total - 1.0).abs() < 1e-6,
        "ranks should sum to 1, got {total}"
    );

    // Seeding alice with the restart mass raises her rank above the
    // uniform-teleport rank: personalization has an effect.
    let uniform = read.personalized_pagerank(
        fixture.graph_projection,
        &[],
        PageRankConfig::new(0.85_f64, 1e-6_f64, 100),
    )?;
    let rank_of = |ranks: &[(oxgraph_db::ElementId, f64)], element| {
        ranks
            .iter()
            .find(|(candidate, _)| *candidate == element)
            .map_or(0.0, |(_, rank)| *rank)
    };
    assert!(rank_of(&seeded, fixture.alice) > rank_of(&uniform, fixture.alice));

    clean(&path)?;
    Ok(())
}

#[test]
fn longest_path_finds_the_longest_chain() -> Result<(), TestError> {
    let path = temp_path("graph-longest-path");
    clean(&path)?;
    let mut database = Db::create(&path)?;
    let (elements, projection) = build_chain_dag(&mut database)?;
    let read = database.reader();

    // Edges 0->1->2->3 plus a shortcut 0->2: the longest simple chain runs the
    // full four-element path rather than the shortcut.
    let chain = read.longest_path(projection, &elements)?;
    assert_eq!(
        chain,
        vec![elements[0], elements[1], elements[2], elements[3]]
    );

    // An empty element set yields an empty path.
    assert!(read.longest_path(projection, &[])?.is_empty());

    clean(&path)?;
    Ok(())
}

#[test]
fn longest_path_rejects_cycles_and_ignores_unknown_elements() -> Result<(), TestError> {
    let (path, database, fixture) = create_traversal_database("graph-longest-path-errors")?;
    let read = database.reader();

    // The calls projection is the cycle alice -> bob -> carol -> alice, so the
    // induced subgraph over all three is cyclic; the algorithm error surfaces as
    // a concrete DbError, never the cross-crate LongestPathError.
    assert!(matches!(
        read.longest_path(
            fixture.graph_projection,
            &[fixture.alice, fixture.bob, fixture.carol],
        ),
        Err(DbError::Query(oxgraph_db::QueryError::Traversal { .. }))
    ));

    // `dave` participates in no Calls relation, so he is absent from the
    // projection; absent elements are ignored rather than erroring. With only an
    // absent element the longest path is empty and the rank falls back to uniform.
    assert_eq!(
        read.longest_path(fixture.graph_projection, &[fixture.dave])?,
        Vec::new()
    );
    assert!(
        !read
            .personalized_pagerank(
                fixture.graph_projection,
                &[fixture.dave],
                PageRankConfig::new(0.85_f64, 1e-6_f64, 100),
            )?
            .is_empty()
    );

    clean(&path)?;
    Ok(())
}

#[test]
fn personalized_pagerank_reflects_graph_structure() -> Result<(), TestError> {
    let path = temp_path("graph-pagerank-structure");
    clean(&path)?;
    let mut database = Db::create(&path)?;
    let (elements, projection) = build_chain_dag(&mut database)?;
    let read = database.reader();

    let rank_of = |ranks: &[(oxgraph_db::ElementId, f64)], element| {
        ranks
            .iter()
            .find(|(candidate, _)| *candidate == element)
            .map_or(0.0, |(_, rank)| *rank)
    };

    // In 0->{1,2}, 1->2, 2->3 the hub 2 (two incoming edges) outranks the source
    // 0 (no incoming edges): rank propagates along edges and the element<->rank
    // round-trip is pinned to specific nodes, not insertion order.
    let ranks =
        read.personalized_pagerank(projection, &[], PageRankConfig::new(0.85, 1e-6, 100))?;
    assert!(rank_of(&ranks, elements[2]) > rank_of(&ranks, elements[0]));

    // Seeding the source lifts it above its unseeded rank.
    let seeded = read.personalized_pagerank(
        projection,
        &[elements[0]],
        PageRankConfig::new(0.85, 1e-6, 100),
    )?;
    assert!(rank_of(&seeded, elements[0]) > rank_of(&ranks, elements[0]));

    clean(&path)?;
    Ok(())
}

#[test]
fn graph_traversal_api_handles_seeds_and_limits() -> Result<(), TestError> {
    let (path, database, fixture) = create_traversal_database("graph-traversal-limits")?;
    let read = database.reader();

    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.alice, fixture.bob],
        Walk {
            max_depth: 2,
            include_start: true,
            ..Walk::default()
        },
        &[
            TraversedNode {
                element: fixture.alice,
                depth: 0,
            },
            TraversedNode {
                element: fixture.bob,
                depth: 0,
            },
            TraversedNode {
                element: fixture.carol,
                depth: 1,
            },
        ],
    )?;
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.alice],
        Walk {
            include_start: true,
            ..Walk::default()
        },
        &[
            TraversedNode {
                element: fixture.alice,
                depth: 0,
            },
            TraversedNode {
                element: fixture.bob,
                depth: 1,
            },
        ],
    )?;
    assert_traversal(
        &read,
        fixture.graph_projection,
        &[fixture.alice],
        Walk {
            max_depth: 2,
            limit: 1,
            ..Walk::default()
        },
        &[TraversedNode {
            element: fixture.bob,
            depth: 1,
        }],
    )?;
    assert!(
        read.walk(fixture.graph_projection, &[], Walk::default(),)?
            .nodes()
            .is_empty()
    );

    clean(&path)?;
    Ok(())
}

#[test]
fn graph_traversal_api_rejects_invalid_inputs() -> Result<(), TestError> {
    let (path, database, fixture) = create_traversal_database("graph-traversal-errors")?;
    let read = database.reader();

    assert!(matches!(
        read.walk(
            fixture.graph_projection,
            &[fixture.dave],
            Walk::default(),
        ),
        Err(DbError::Catalog(oxgraph_db::CatalogError::UnknownId {
            family: oxgraph_db::IdFamily::Element,
            id,
        })) if id == fixture.dave.get()
    ));
    assert!(matches!(
        read.walk(
            oxgraph_db::ProjectionId::new(999),
            &[fixture.alice],
            Walk::default(),
        ),
        Err(DbError::Catalog(oxgraph_db::CatalogError::UnknownId {
            family: oxgraph_db::IdFamily::Projection,
            ..
        }))
    ));
    assert!(matches!(
        read.walk(fixture.hyper_projection, &[fixture.alice], Walk::default(),),
        Err(DbError::Query(
            oxgraph_db::QueryError::InvalidProjection { .. }
        ))
    ));

    clean(&path)?;
    Ok(())
}

#[test]
fn walk_returns_discovered_nodes_and_edges() -> Result<(), TestError> {
    // The fixture is a cycle alice -> bob -> carol -> alice over `Calls`.
    let (path, database, fixture) = create_traversal_database("walk-nodes-and-edges")?;
    let read = database.reader();

    let subgraph = read.walk(
        fixture.graph_projection,
        &[fixture.alice],
        Walk {
            max_depth: 2,
            include_start: true,
            ..Walk::default()
        },
    )?;

    // Depth-2 outgoing from alice discovers the whole cycle.
    let discovered = subgraph
        .nodes()
        .iter()
        .map(|node| node.element)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        discovered,
        BTreeSet::from([fixture.alice, fixture.bob, fixture.carol])
    );

    // Every edge connects two discovered nodes; the carol -> alice closing edge
    // is included because both endpoints are in the node set.
    let edge_pairs = subgraph
        .edges()
        .iter()
        .map(|edge| (edge.source, edge.target))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        edge_pairs,
        BTreeSet::from([
            (fixture.alice, fixture.bob),
            (fixture.bob, fixture.carol),
            (fixture.carol, fixture.alice),
        ])
    );

    // Excluding the seed from the result nodes must still report a seed-rooted
    // edge whose target is discovered (alice -> bob at depth 1).
    let excluded = read.walk(
        fixture.graph_projection,
        &[fixture.alice],
        Walk {
            max_depth: 1,
            include_start: false,
            ..Walk::default()
        },
    )?;
    assert!(
        !excluded
            .nodes()
            .iter()
            .any(|node| node.element == fixture.alice)
    );
    assert_eq!(
        excluded
            .edges()
            .iter()
            .map(|edge| (edge.source, edge.target))
            .collect::<Vec<_>>(),
        vec![(fixture.alice, fixture.bob)]
    );

    clean(&path)?;
    Ok(())
}

#[test]
fn neighbors_resolves_role_aware_adjacency() -> Result<(), TestError> {
    // The fixture is a cycle alice -> bob -> carol -> alice over `Calls`.
    let (path, database, fixture) = create_traversal_database("neighbors-role-aware")?;
    let read = database.reader();
    let calls = read
        .catalog()
        .relation_type_id("Calls")
        .ok_or(DbError::Query(oxgraph_db::QueryError::Empty))?;

    // Outgoing from bob follows bob -> carol.
    assert_eq!(
        read.neighbors(fixture.bob, calls, Direction::Outgoing),
        vec![fixture.carol]
    );
    // Incoming to bob follows alice -> bob.
    assert_eq!(
        read.neighbors(fixture.bob, calls, Direction::Incoming),
        vec![fixture.alice]
    );
    // Both yields each side, ascending by element id.
    let mut both = vec![fixture.alice, fixture.carol];
    both.sort_unstable();
    assert_eq!(read.neighbors(fixture.bob, calls, Direction::Both), both);

    // Dave participates in no `Calls` relation.
    assert!(
        read.neighbors(fixture.dave, calls, Direction::Both)
            .is_empty()
    );

    clean(&path)?;
    Ok(())
}

#[test]
fn endpoints_returns_binary_relation_endpoints() -> Result<(), TestError> {
    // The fixture is a cycle alice -> bob -> carol -> alice over `Calls`.
    let (path, database, fixture) = create_traversal_database("endpoints-binary")?;
    let read = database.reader();
    let calls = read
        .catalog()
        .relation_type_id("Calls")
        .ok_or(DbError::Query(oxgraph_db::QueryError::Empty))?;

    // Find the alice -> bob `Calls` relation and confirm its endpoints read back
    // from incidence storage in source, target order.
    let alice_bob = read
        .relation_ids()
        .into_iter()
        .find(|id| {
            read.relation(*id).is_some_and(|relation| {
                relation.relation_type == Some(calls)
                    && read.endpoints(*id) == Some((fixture.alice, fixture.bob))
            })
        })
        .ok_or(DbError::Query(oxgraph_db::QueryError::Empty))?;
    assert_eq!(
        read.endpoints(alice_bob),
        Some((fixture.alice, fixture.bob))
    );

    clean(&path)?;
    Ok(())
}

#[test]
fn oxql_graph_walk_executes_valid_queries() -> Result<(), TestError> {
    let (path, database, fixture) = create_traversal_database("oxql-graph-walk-valid")?;
    let read = database.reader();

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
        database.prepare(&format!(
            "GRAPH calls WALK FROM {} DEPTH 1 DIRECTION sideways",
            fixture.alice.get()
        ),),
        Err(DbError::Query(oxgraph_db::QueryError::Unsupported { .. }))
    ));
    assert!(matches!(
        database.prepare(&format!(
            "GRAPH calls WALK FROM {} DEPTH nope",
            fixture.alice.get()
        ),),
        Err(DbError::Query(oxgraph_db::QueryError::Unsupported { .. }))
    ));
    assert!(matches!(
        database.prepare(&format!(
            "GRAPH calls WALK FROM {} DEPTH 1 LIMIT nope",
            fixture.alice.get()
        ),),
        Err(DbError::Query(oxgraph_db::QueryError::Unsupported { .. }))
    ));
    assert!(matches!(
        database.prepare(&format!(
            "GRAPH missing WALK FROM {} DEPTH 1",
            fixture.alice.get()
        ),),
        Err(DbError::Query(oxgraph_db::QueryError::Unsupported { .. }))
    ));
    assert!(matches!(
        database.prepare(&format!(
            "GRAPH calls_hyper WALK FROM {} DEPTH 1",
            fixture.alice.get()
        ),),
        Err(DbError::Query(
            oxgraph_db::QueryError::InvalidProjection { .. }
        ))
    ));

    clean(&path)?;
    Ok(())
}

/// Three-way durability contract (a): flipping ANY payload byte of ANY base
/// section fails the next open with `InvalidStore` — every bound section's
/// CRC-32C is verified once at bind, before any borrow — and flipping a
/// section-TABLE byte fails the checked open via the header's `table_crc32c`.
#[test]
fn corrupt_base_bytes_fail_open() -> Result<(), TestError> {
    let path = temp_path("corrupt-base");
    clean(&path)?;

    let mut database = Db::create(&path)?;
    load_fixture(&mut database)?;
    database.compact()?; // fold the data into base-1 so the base is non-empty
    drop(database);

    // The live base is named by the superblock; after one checkpoint it is base-1.
    let base_path = path.join("base-1.oxgdb");
    let pristine = std::fs::read(&base_path)?;

    // Enumerate every section's payload range via the container; the snapshot
    // borrow must end before the file is rewritten.
    let sections: Vec<(u32, usize, usize)> = {
        let snapshot =
            oxgraph_snapshot::Snapshot::open(&pristine).expect("reopen frozen base container");
        snapshot
            .sections()
            .map(|section| {
                (
                    section.kind(),
                    section.bytes().as_ptr().addr() - pristine.as_ptr().addr(),
                    section.bytes().len(),
                )
            })
            .collect()
    };
    assert!(!sections.is_empty(), "frozen base has sections");

    for (kind, offset, len) in sections {
        // Flip the first and last byte of the section's payload.
        for position in [offset, offset + len - 1] {
            let mut bytes = pristine.clone();
            bytes[position] ^= 0xFF;
            std::fs::write(&base_path, &bytes)?;
            assert!(
                matches!(
                    Db::open(&path),
                    Err(DbError::Storage(
                        oxgraph_db::StorageError::InvalidStore { .. }
                    ))
                ),
                "corrupt payload byte at {position} in section {kind:#06X} must fail open",
            );
        }
    }

    // Flip a byte inside the section TABLE: the first entry's `version` word
    // (offset 8 + length 8 + kind 4 = 20 bytes into the entry), which the
    // structural open does not interpret, so the table checksum is the check
    // that rejects it.
    let mut bytes = pristine;
    bytes[oxgraph_snapshot::HEADER_SIZE + 20] ^= 0xFF;
    std::fs::write(&base_path, &bytes)?;
    assert!(
        matches!(
            Db::open(&path),
            Err(DbError::Storage(
                oxgraph_db::StorageError::InvalidStore { .. }
            ))
        ),
        "corrupt section-table byte must fail open via the table checksum",
    );

    clean(&path)?;
    Ok(())
}

/// Three-way durability contract (b): a torn final log frame (a partial trailing
/// record) truncates-and-accepts on open — the prior committed frame is the
/// recovered frontier, so recovery succeeds and the durable data survives.
#[test]
fn torn_log_tail_truncates_and_recovers() -> Result<(), TestError> {
    let path = temp_path("torn-log-tail");
    clean(&path)?;

    let mut database = Db::create(&path)?;
    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "the method path cannot satisfy write's for<'a> Fn bound over Writer<'a>"
    )]
    let (kept, _outcome) = database.write(|writer| writer.create_element())?;
    drop(database);

    // Append a partial (torn) trailing record to the live delta-0.log.
    let log_path = path.join("delta-0.log");
    let mut log = std::fs::OpenOptions::new().append(true).open(&log_path)?;
    std::io::Write::write_all(&mut log, &[0xAB, 0xCD, 0xEF])?; // < a header: torn tail
    drop(log);

    // Recovery truncates the torn tail and accepts the prior committed frame.
    let reopened = Db::open(&path)?;
    let read = reopened.reader();
    assert!(read.contains_element(kept), "committed element survives");
    assert_eq!(read.element_count(), 1);
    clean(&path)?;
    Ok(())
}

/// Three-way durability contract (c): a bit-flip inside a NON-final committed log
/// frame is loud `LogCorrupt`, never silently skipped.
#[test]
fn interior_log_corruption_is_loud() -> Result<(), TestError> {
    let path = temp_path("interior-log-corruption");
    clean(&path)?;

    let mut database = Db::create(&path)?;
    database.write(|first| {
        first.create_element()?;
        Ok(())
    })?;
    database.write(|second| {
        second.create_element()?;
        Ok(())
    })?;
    drop(database);

    // Flip a payload byte inside the FIRST (non-final) frame's body. The
    // LogRecordHeader is 40 bytes; offset 48 lands in the first frame's ops.
    let log_path = path.join("delta-0.log");
    let mut bytes = std::fs::read(&log_path)?;
    bytes[48] ^= 0xFF;
    std::fs::write(&log_path, &bytes)?;

    assert!(
        matches!(
            Db::open(&path),
            Err(DbError::Storage(
                oxgraph_db::StorageError::LogCorrupt { .. }
            ))
        ),
        "interior log corruption must be a loud LogCorrupt",
    );
    clean(&path)?;
    Ok(())
}

#[test]
fn concurrent_writers_are_rejected_until_release() -> Result<(), TestError> {
    let path = temp_path("writer-lock");
    clean(&path)?;

    let mut first = Db::create(&path)?;
    let mut second = Db::open(&path)?;

    // While `first` holds the single-writer lock inside its write scope, a second
    // handle's write is rejected with `WriterLockHeld`.
    first.write(|_writer| {
        let blocked = second.write(|_writer| Ok(()));
        assert!(matches!(
            blocked,
            Err(DbError::Txn(oxgraph_db::TxnError::WriterLockHeld))
        ));
        Ok(())
    })?;

    // Once the first write commits, the lock is free and the second handle writes.
    second.write(|_writer| Ok(()))?;

    clean(&path)?;
    Ok(())
}

/// MVCC isolation on the LIVE commit path: a `Reader` pins its snapshot
/// before a commit on the same `Db` handle, so it keeps observing the
/// pre-commit state (count and pin unchanged) while a fresh reader observes the
/// post-commit state. This guards the P4 publish-new-`Arc` invariant on the real
/// `begin_write` + `commit` path (not the `with_applied` proof helper).
#[test]
fn pinned_reader_is_isolated_from_a_later_commit() -> Result<(), TestError> {
    let path = temp_path("mvcc-reader-isolation");
    clean(&path)?;

    let mut database = Db::create(&path)?;
    database.write(|seed| {
        seed.create_element()?;
        Ok(())
    })?;

    // Pin a reader at count N = 1, recording its visible generation/lsn.
    let pinned = database.reader();
    let n = pinned.element_count();
    assert_eq!(n, 1);
    let pin_before = pinned.pin();

    // Commit a new element on the SAME handle while the reader is pinned.
    database.write(|writer| {
        writer.create_element()?;
        Ok(())
    })?;

    // The pinned reader still observes the pre-commit state and an unchanged pin.
    assert_eq!(
        pinned.element_count(),
        n,
        "pinned reader must not see the post-commit element",
    );
    assert_eq!(
        pinned.pin(),
        pin_before,
        "pinned reader's generation/lsn must be unchanged across the commit",
    );

    // A fresh reader observes the post-commit state (N + 1) and a newer pin.
    let fresh = database.reader();
    assert_eq!(
        fresh.element_count(),
        n + 1,
        "a reader begun after the commit must see the new element",
    );
    assert!(
        fresh.pin().visible_commit_seq > pin_before.visible_commit_seq,
        "the fresh reader's commit sequence must advance past the pinned reader's",
    );

    clean(&path)?;
    Ok(())
}

/// Loads a traversal-focused graph fixture.
fn load_traversal_fixture(database: &mut Db) -> Result<TraversalFixtureIds, DbError> {
    let (fixture, _outcome) = database.write(|writer| {
        let source_role = writer.register_role("source")?;
        let target_role = writer.register_role("target")?;
        let calls_type = writer.register_relation_type("Calls")?;
        let meeting_type = writer.register_relation_type("Meeting")?;
        let alice = writer.create_element()?;
        let bob = writer.create_element()?;
        let carol = writer.create_element()?;
        let dave = writer.create_element()?;
        let roles = (source_role, target_role);
        create_directed_relation(writer, calls_type, roles, (alice, bob))?;
        create_directed_relation(writer, calls_type, roles, (bob, carol))?;
        create_directed_relation(writer, calls_type, roles, (carol, alice))?;
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
        Ok(TraversalFixtureIds {
            alice,
            bob,
            carol,
            dave,
            graph_projection,
            hyper_projection,
        })
    })?;
    Ok(fixture)
}

/// Creates a traversal test database.
fn create_traversal_database(name: &str) -> Result<(PathBuf, Db, TraversalFixtureIds), TestError> {
    let path = temp_path(name);
    clean(&path)?;
    let mut database = Db::create(&path)?;
    let fixture = load_traversal_fixture(&mut database)?;
    Ok((path, database, fixture))
}

/// Creates one directed binary relation.
fn create_directed_relation(
    writer: &mut oxgraph_db::Writer<'_>,
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

/// Builds a small DAG `0 -> 1 -> 2 -> 3` with a `0 -> 2` shortcut over a `calls`
/// graph projection, returning the element ids and the projection id.
fn build_chain_dag(
    database: &mut Db,
) -> Result<(Vec<oxgraph_db::ElementId>, oxgraph_db::ProjectionId), TestError> {
    let (out, _outcome) = database.write(|writer| {
        let source = writer.register_role("source")?;
        let target = writer.register_role("target")?;
        let edge_type = writer.register_relation_type("Calls")?;
        let mut elements = Vec::with_capacity(4);
        for _ in 0..4 {
            elements.push(writer.create_element()?);
        }
        for (from, to) in [(0, 1), (1, 2), (2, 3), (0, 2)] {
            create_directed_relation(
                writer,
                edge_type,
                (source, target),
                (elements[from], elements[to]),
            )?;
        }
        let projection =
            writer.define_projection(ProjectionDefinition::Graph(GraphProjectionDefinition {
                name: "calls".to_owned(),
                relation_types: BTreeSet::from([edge_type]),
                source_role: source,
                target_role: target,
            }))?;
        Ok((elements, projection))
    })?;
    Ok(out)
}

/// Asserts graph traversal rows.
fn assert_traversal(
    read: &oxgraph_db::Reader,
    projection: oxgraph_db::ProjectionId,
    seeds: &[oxgraph_db::ElementId],
    options: Walk,
    expected: &[TraversedNode],
) -> Result<(), DbError> {
    assert_eq!(read.walk(projection, seeds, options)?.nodes(), expected);
    Ok(())
}

/// Executes an `OxQL` query returning one element value per row.
fn execute_element_query(
    database: &Db,
    read: &oxgraph_db::Reader,
    query: &str,
) -> Result<Vec<oxgraph_db::ElementId>, DbError> {
    let prepared = database.prepare(query)?;
    Ok(read
        .run(&prepared)?
        .rows()
        .iter()
        .filter_map(|row| match row.values.as_slice() {
            [QueryValue::Element(element)] => Some(*element),
            _values => None,
        })
        .collect())
}

/// Loads a graph and hypergraph fixture.
fn load_fixture(database: &mut Db) -> Result<FixtureIds, DbError> {
    let (fixture, _outcome) = database.write(|writer| {
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

        let (alice, bob, carol) = create_people(writer, person_label, name_key, age_key)?;

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
            writer,
            FixtureIndexInputs {
                person_label,
                name_key,
                age_key,
                graph_projection,
                hyper_projection,
            },
        )?;

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
    })?;
    Ok(fixture)
}

/// Defines the fixture indexes.
fn define_fixture_indexes(
    writer: &mut oxgraph_db::Writer<'_>,
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
    writer: &mut oxgraph_db::Writer<'_>,
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
        writer.add_label(element, person_label)?;
    }
    writer.set(
        PropertySubject::Element(alice),
        Key::<Text>::from_id(name_key),
        "Alice".to_owned(),
    )?;
    writer.set(
        PropertySubject::Element(bob),
        Key::<Text>::from_id(name_key),
        "Bob".to_owned(),
    )?;
    writer.set(
        PropertySubject::Element(alice),
        Key::<Int>::from_id(age_key),
        42_i64,
    )?;
    Ok((alice, bob, carol))
}

/// Asserts compound `WHERE` predicate coverage over the fixture.
///
/// Verifies that `OR` unions, `AND` intersects, ordered comparisons work, `AND`
/// binds tighter than `OR`, parentheses override that precedence, and malformed
/// predicates are rejected at prepare time.
fn assert_compound_where(
    database: &Db,
    read: &oxgraph_db::Reader,
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
        database.prepare("MATCH ELEMENTS WHERE name ="),
        Err(DbError::Query(oxgraph_db::QueryError::Unsupported { .. }))
    ));
    assert!(matches!(
        database.prepare("MATCH ELEMENTS WHERE ( name = 'Alice'"),
        Err(DbError::Query(oxgraph_db::QueryError::Unsupported { .. }))
    ));
    Ok(())
}

/// Asserts query-language coverage over the fixture.
fn assert_query_counts(database: &Db, fixture: &FixtureIds) -> Result<(), DbError> {
    let read = database.reader();
    let elements = database.prepare("MATCH ELEMENTS")?;
    assert_eq!(read.run(&elements)?.rows().len(), 3);

    let people = database.prepare("MATCH ELEMENTS HAS LABEL Person")?;
    assert_eq!(read.run(&people)?.rows().len(), 3);

    let alice = database.prepare("MATCH ELEMENTS WHERE name = 'Alice'")?;
    let rows = read.run(&alice)?;
    assert_eq!(rows.rows().len(), 1);
    assert_eq!(
        rows.rows()[0].values,
        vec![QueryValue::Element(fixture.alice)]
    );
    assert!(matches!(
        database.prepare("MATCH ELEMENTS WHERE age = '42'"),
        Err(DbError::Query(
            oxgraph_db::QueryError::PropertyTypeMismatch {
                expected: PropertyType::Integer,
                actual: PropertyType::Text,
            }
        ))
    ));
    assert!(matches!(
        database.prepare("MATCH ELEMENTS WHERE relation_weight = 1",),
        Err(DbError::Query(
            oxgraph_db::QueryError::WrongPropertyFamily {
                expected: PropertyFamily::Relation,
                actual: PropertyFamily::Element,
            }
        ))
    ));
    assert!(matches!(
        database.prepare("MATCH ELEMENTS WHERE incidence_note = 'source'",),
        Err(DbError::Query(
            oxgraph_db::QueryError::WrongPropertyFamily {
                expected: PropertyFamily::Incidence,
                actual: PropertyFamily::Element,
            }
        ))
    ));

    assert_compound_where(database, &read, fixture)?;

    let knows = database.prepare("MATCH RELATIONS TYPE Knows")?;
    assert_eq!(read.run(&knows)?.rows().len(), 1);

    let neighbors = database.prepare(&format!(
        "GRAPH knows_graph NEIGHBORS {}",
        fixture.alice.get()
    ))?;
    assert_eq!(
        read.run(&neighbors)?.rows()[0].values,
        vec![QueryValue::Element(fixture.bob)]
    );

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

/// The borrowed read surface returns the same values as the owned one, with
/// `Cow::Borrowed` for folded values and shared text payloads either way.
#[test]
fn borrowed_reads_match_owned() -> Result<(), DbError> {
    let path = temp_path("borrowed-reads");
    let mut database = Db::create(&path)?;
    let ((element, name_key), _outcome) = database.write(|writer| {
        let name_key = writer.register_property_key(
            "name",
            oxgraph_db::PropertyFamily::Element,
            PropertyType::Text,
        )?;
        let element = writer.create_element()?;
        writer.set(
            PropertySubject::Element(element),
            oxgraph_db::Key::<oxgraph_db::Text>::from_id(name_key),
            "Alice",
        )?;
        Ok((element, name_key))
    })?;

    // Overlay-resident (unfolded) value: owned and borrowed agree.
    let reader = database.reader();
    let subject = PropertySubject::Element(element);
    assert_eq!(
        reader.property(subject, name_key),
        reader
            .value(subject, name_key)
            .map(std::borrow::Cow::into_owned)
    );
    assert_eq!(reader.text(subject, name_key).as_deref(), Some("Alice"));

    // Folded value: the borrowed arm borrows from the base.
    database.compact()?;
    let reader = database.reader();
    assert!(matches!(
        reader.value(subject, name_key),
        Some(std::borrow::Cow::Borrowed(_))
    ));
    assert_eq!(reader.text(subject, name_key).as_deref(), Some("Alice"));
    let _ = std::fs::remove_dir_all(&path);
    Ok(())
}
