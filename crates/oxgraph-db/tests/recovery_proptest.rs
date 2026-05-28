//! Property tests for OXGDB recovery, projections, properties, and queries.

use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use oxgraph_db::{
    Database, DbError, GraphProjectionDefinition, HypergraphProjectionDefinition, IndexDefinition,
    IndexLookup, ProjectionDefinition, PropertyFamily, PropertySubject, PropertyType,
    PropertyValue, QueryLanguage, RelationId, RoleId, TraversalDirection, TraversalOptions,
    TraversalRow,
};
use oxgraph_graph::{ElementSuccessors, LocalElementIdentity, TopologyCounts};
use oxgraph_hyper::DirectedHyperedgeParticipants;
use proptest::prelude::*;

/// Per-process path counter.
static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

/// Builds a unique temporary database path.
fn temp_path(name: &str) -> PathBuf {
    let id = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("oxgraph-db-{name}-{}-{id}", std::process::id()))
}

/// Removes `path` when it exists.
fn clean(path: &Path) -> Result<(), std::io::Error> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Converts a database error into a proptest failure.
fn prop_db<T>(result: Result<T, DbError>) -> Result<T, TestCaseError> {
    result.map_err(|error| TestCaseError::fail(error.to_string()))
}

/// Converts an IO error into a proptest failure.
fn prop_io(result: Result<(), std::io::Error>) -> Result<(), TestCaseError> {
    result.map_err(|error| TestCaseError::fail(error.to_string()))
}

proptest! {
    #[test]
    fn committed_topology_properties_and_catalog_recover(
        element_count in 1_usize..8,
        edge_specs in prop::collection::vec((0_usize..8, 0_usize..8), 0..16),
    ) {
        let path = temp_path("recovery");
        prop_io(clean(&path))?;

        let mut database = prop_db(Database::create(&path))?;
        let expected = prop_db(load_generated(&mut database, element_count, &edge_specs))?;
        let reopened = prop_db(Database::open(&path))?;
        let read = reopened.begin_read();
        prop_assert_eq!(read.element_count(), element_count);
        prop_assert_eq!(read.relation_count(), edge_specs.len());
        prop_assert_eq!(read.incidence_count(), edge_specs.len() * 2);
        prop_assert_eq!(read.catalog().projections().count(), 2);

        let graph = prop_db(read.graph_projection(expected.graph_projection))?;
        prop_assert_eq!(graph.relation_count(), edge_specs.len());
        let projected_endpoint_count = edge_specs
            .iter()
            .flat_map(|(source, target)| [source % element_count, target % element_count])
            .collect::<BTreeSet<_>>()
            .len();
        prop_assert_eq!(graph.element_count(), projected_endpoint_count);
        prop_io(clean(&path))?;
    }

    #[test]
    fn graph_projection_matches_reference_successors(
        element_count in 1_usize..8,
        edge_specs in prop::collection::vec((0_usize..8, 0_usize..8), 0..16),
    ) {
        let path = temp_path("graph-projection");
        prop_io(clean(&path))?;

        let mut database = prop_db(Database::create(&path))?;
        let expected = prop_db(load_generated(&mut database, element_count, &edge_specs))?;
        let read = database.begin_read();
        let graph = prop_db(read.graph_projection(expected.graph_projection))?;

        for (slot, element) in expected.elements.iter().copied().enumerate() {
            let reference = edge_specs
                .iter()
                .filter(|(source, _target)| *source % element_count == slot)
                .count();
            let Some(local) = graph.local_element_id(element) else {
                prop_assert_eq!(reference, 0);
                continue;
            };
            let observed = graph.element_successors(local).count();
            prop_assert_eq!(observed, reference);
        }
        prop_io(clean(&path))?;
    }

    #[test]
    fn hypergraph_projection_matches_reference_targets(
        element_count in 2_usize..8,
        target_count in 1_usize..8,
    ) {
        let path = temp_path("hyper-projection");
        prop_io(clean(&path))?;

        let mut database = prop_db(Database::create(&path))?;
        let expected = prop_db(load_one_hyperedge(&mut database, element_count, target_count))?;
        let read = database.begin_read();
        let hyper = prop_db(read.hypergraph_projection(expected.hyper_projection))?;
        prop_assert_eq!(hyper.relation_count(), 1);
        let target_seen = hyper
            .target_participants(oxgraph_db::ProjectionRelationId::new(0))
            .count();
        prop_assert_eq!(target_seen, target_count.min(element_count - 1));
        prop_io(clean(&path))?;
    }

    #[test]
    fn property_index_lookup_matches_reference(values in prop::collection::vec(0_i64..64, 1..16)) {
        let path = temp_path("property-index");
        prop_io(clean(&path))?;

        let mut database = prop_db(Database::create(&path))?;
        let mut writer = prop_db(database.begin_write())?;
        let key = prop_db(writer.register_property_key(
            "rank",
            PropertyFamily::Element,
            PropertyType::Integer,
        ))?;
        let index = prop_db(writer.define_index(
            "rank_eq",
            IndexDefinition::PropertyEquality { key },
        ))?;
        let mut expected = Vec::new();
        for value in &values {
            let element = prop_db(writer.create_element())?;
            prop_db(writer.set_property(
                PropertySubject::Element(element),
                key,
                PropertyValue::Integer(*value),
            ))?;
            if *value == values[0] {
                expected.push(PropertySubject::Element(element));
            }
        }
        prop_db(writer.commit())?;

        let read = database.begin_read();
        let observed = prop_db(read.lookup_index(
            index,
            IndexLookup::Equal(&PropertyValue::Integer(values[0])),
        ))?;
        prop_assert_eq!(observed, expected);
        prop_io(clean(&path))?;
    }

    #[test]
    fn composite_index_lookup_matches_reference(
        values in prop::collection::vec((0_i64..8, 0_i64..8), 1..16),
        target_left in 0_i64..8,
        target_right in 0_i64..8,
    ) {
        let path = temp_path("composite-index");
        prop_io(clean(&path))?;

        let mut database = prop_db(Database::create(&path))?;
        let mut writer = prop_db(database.begin_write())?;
        let left_key = prop_db(writer.register_property_key(
            "left",
            PropertyFamily::Element,
            PropertyType::Integer,
        ))?;
        let right_key = prop_db(writer.register_property_key(
            "right",
            PropertyFamily::Element,
            PropertyType::Integer,
        ))?;
        let index = prop_db(writer.define_index(
            "left_right",
            IndexDefinition::CompositeEquality {
                keys: vec![left_key, right_key],
            },
        ))?;
        let mut expected = Vec::new();
        for (left, right) in &values {
            let element = prop_db(writer.create_element())?;
            prop_db(writer.set_property(
                PropertySubject::Element(element),
                left_key,
                PropertyValue::Integer(*left),
            ))?;
            prop_db(writer.set_property(
                PropertySubject::Element(element),
                right_key,
                PropertyValue::Integer(*right),
            ))?;
            if *left == target_left && *right == target_right {
                expected.push(PropertySubject::Element(element));
            }
        }
        prop_db(writer.commit())?;

        let lookup_values = [
            PropertyValue::Integer(target_left),
            PropertyValue::Integer(target_right),
        ];
        let read = database.begin_read();
        let observed = prop_db(read.lookup_index(
            index,
            IndexLookup::CompositeEqual(&lookup_values),
        ))?;
        prop_assert_eq!(observed, expected);
        prop_io(clean(&path))?;
    }

    #[test]
    fn property_lookup_type_mismatches_error(value in 0_i64..64) {
        let path = temp_path("property-type-errors");
        prop_io(clean(&path))?;

        let mut database = prop_db(Database::create(&path))?;
        let mut writer = prop_db(database.begin_write())?;
        let key = prop_db(writer.register_property_key(
            "rank",
            PropertyFamily::Element,
            PropertyType::Integer,
        ))?;
        let index = prop_db(writer.define_index(
            "rank_eq",
            IndexDefinition::PropertyEquality { key },
        ))?;
        let element = prop_db(writer.create_element())?;
        prop_db(writer.set_property(
            PropertySubject::Element(element),
            key,
            PropertyValue::Integer(value),
        ))?;
        prop_db(writer.commit())?;

        let read = database.begin_read();
        let equal_type_error = matches!(
            read.lookup_property_equal(key, &PropertyValue::Text("wrong".to_owned())),
            Err(DbError::PropertyTypeMismatch { .. }),
        );
        prop_assert!(equal_type_error);
        let range_type_error = matches!(
            read.lookup_property_range(
                key,
                &PropertyValue::Text("0".to_owned()),
                &PropertyValue::Text("9".to_owned()),
            ),
            Err(DbError::PropertyTypeMismatch { .. }),
        );
        prop_assert!(range_type_error);
        let index_type_error = matches!(
            read.lookup_index(
                index,
                IndexLookup::Equal(&PropertyValue::Text("wrong".to_owned())),
            ),
            Err(DbError::PropertyTypeMismatch { .. }),
        );
        prop_assert!(index_type_error);
        prop_io(clean(&path))?;
    }

    #[test]
    fn projection_index_lookup_matches_reference_membership(
        element_count in 1_usize..8,
        edge_specs in prop::collection::vec((0_usize..8, 0_usize..8), 0..16),
    ) {
        let path = temp_path("projection-index");
        prop_io(clean(&path))?;

        let mut database = prop_db(Database::create(&path))?;
        let expected = prop_db(load_generated(&mut database, element_count, &edge_specs))?;
        let read = database.begin_read();
        let mut graph_expected = BTreeSet::new();
        let mut hyper_expected = BTreeSet::new();
        for (source, target) in &edge_specs {
            let source = expected.elements[*source % element_count];
            let target = expected.elements[*target % element_count];
            graph_expected.insert(PropertySubject::Element(source));
            graph_expected.insert(PropertySubject::Element(target));
            hyper_expected.insert(PropertySubject::Element(source));
            hyper_expected.insert(PropertySubject::Element(target));
        }
        for relation in &expected.relations {
            graph_expected.insert(PropertySubject::Relation(*relation));
            hyper_expected.insert(PropertySubject::Relation(*relation));
        }
        for incidence in &expected.incidences {
            hyper_expected.insert(PropertySubject::Incidence(*incidence));
        }

        let graph_observed = prop_db(read.lookup_index(
            expected.graph_projection_index,
            IndexLookup::All,
        ))?;
        let hyper_observed = prop_db(read.lookup_index(
            expected.hyper_projection_index,
            IndexLookup::All,
        ))?;
        prop_assert_eq!(
            graph_observed,
            graph_expected.into_iter().collect::<Vec<_>>()
        );
        prop_assert_eq!(
            hyper_observed,
            hyper_expected.into_iter().collect::<Vec<_>>()
        );
        prop_io(clean(&path))?;
    }

    #[test]
    fn graph_traversal_matches_reference_bfs(
        element_count in 1_usize..8,
        edge_specs in prop::collection::vec((0_usize..8, 0_usize..8), 1..16),
        max_depth in 0_usize..5,
        limit in 0_usize..16,
        direction in traversal_direction_strategy(),
        include_start in any::<bool>(),
    ) {
        let path = temp_path("graph-traversal");
        prop_io(clean(&path))?;

        let mut database = prop_db(Database::create(&path))?;
        let expected = prop_db(load_generated(&mut database, element_count, &edge_specs))?;
        let read = database.begin_read();
        let seed_slot = edge_specs[0].0 % element_count;
        let options = TraversalOptions {
            max_depth,
            direction,
            limit,
            include_start,
        };
        let observed = prop_db(read.traverse_graph(
            expected.graph_projection,
            &[expected.elements[seed_slot]],
            options,
        ))?
        .rows()
        .to_vec();
        let reference = reference_graph_traversal(
            element_count,
            &edge_specs,
            &expected.elements,
            seed_slot,
            options,
        );
        prop_assert_eq!(&observed, &reference);
        let mut unique = BTreeSet::new();
        for row in &observed {
            prop_assert!(unique.insert(row.element));
            prop_assert!(row.depth <= max_depth);
        }
        for rows in observed.windows(2) {
            prop_assert!(rows[0].depth <= rows[1].depth);
        }
        prop_assert!(observed.len() <= limit);
        prop_io(clean(&path))?;
    }

    #[test]
    fn tombstone_element_cascades_incidence_visibility(
        element_count in 1_usize..8,
        edge_specs in prop::collection::vec((0_usize..8, 0_usize..8), 0..16),
        tombstone_slot in 0_usize..8,
    ) {
        let path = temp_path("tombstone");
        prop_io(clean(&path))?;

        let mut database = prop_db(Database::create(&path))?;
        let expected = prop_db(load_generated(&mut database, element_count, &edge_specs))?;
        let removed_slot = tombstone_slot % element_count;
        let removed = expected.elements[removed_slot];
        let mut writer = prop_db(database.begin_write())?;
        prop_db(writer.tombstone_element(removed))?;
        prop_db(writer.commit())?;

        let read = database.begin_read();
        let reference = edge_specs
            .iter()
            .map(|(source, target)| {
                usize::from(*source % element_count != removed_slot)
                    + usize::from(*target % element_count != removed_slot)
            })
            .sum::<usize>();
        prop_assert!(!read.contains_element(removed));
        prop_assert_eq!(read.incidence_count(), reference);
        prop_io(clean(&path))?;
    }

    #[test]
    fn invalid_incidence_references_are_typed(
        missing_relation in 1_u64..u64::MAX,
        missing_role in 1_u64..u64::MAX,
    ) {
        let path = temp_path("invalid-incidence");
        prop_io(clean(&path))?;

        let mut database = prop_db(Database::create(&path))?;
        let mut writer = prop_db(database.begin_write())?;
        let element = prop_db(writer.create_element())?;
        let result = writer.create_incidence(
            RelationId::new(missing_relation),
            element,
            RoleId::new(missing_role),
        );
        prop_assert!(
            matches!(result, Err(DbError::UnknownRelation { .. })),
            "expected unknown relation"
        );
        writer.rollback();
        prop_io(clean(&path))?;
    }

    #[test]
    fn rollback_and_empty_commits_do_not_become_visible(empty_commits in 0_usize..8) {
        let path = temp_path("empty-rollback");
        prop_io(clean(&path))?;

        let mut database = prop_db(Database::create(&path))?;
        for _index in 0..empty_commits {
            let writer = prop_db(database.begin_write())?;
            prop_assert_eq!(prop_db(writer.commit())?.get(), 0);
        }
        let mut writer = prop_db(database.begin_write())?;
        prop_db(writer.create_element())?;
        writer.rollback();

        let reopened = prop_db(Database::open(&path))?;
        let read = reopened.begin_read();
        prop_assert_eq!(read.element_count(), 0);
        prop_assert_eq!(read.relation_count(), 0);
        prop_assert_eq!(read.incidence_count(), 0);
        prop_io(clean(&path))?;
    }

    #[test]
    fn query_prepare_rejects_unknown_profiles(query in ".{0,32}") {
        let path = temp_path("query-prepare");
        prop_io(clean(&path))?;

        let database = prop_db(Database::create(&path))?;
        let result = database.prepare(QueryLanguage::Oxql, &query);
        match query.trim() {
            "" => prop_assert!(matches!(result, Err(DbError::EmptyQuery))),
            "MATCH ELEMENTS" | "MATCH RELATIONS" | "MATCH INCIDENCES" | "CATALOG" => {
                let prepared = prop_db(result)?;
                prop_assert!(!prepared.explain().is_empty());
            }
            _query => prop_assert!(
                matches!(result, Err(DbError::UnsupportedQuery { .. })),
                "expected unsupported query"
            ),
        }
        prop_io(clean(&path))?;
    }
}

/// Generates traversal directions.
fn traversal_direction_strategy() -> impl Strategy<Value = TraversalDirection> {
    prop_oneof![
        Just(TraversalDirection::Outgoing),
        Just(TraversalDirection::Incoming),
        Just(TraversalDirection::Both),
    ]
}

/// Computes reference BFS traversal over generated edge slots.
fn reference_graph_traversal(
    element_count: usize,
    edge_specs: &[(usize, usize)],
    elements: &[oxgraph_db::ElementId],
    seed_slot: usize,
    options: TraversalOptions,
) -> Vec<TraversalRow> {
    if options.limit == 0 {
        return Vec::new();
    }
    let mut outgoing = vec![Vec::new(); element_count];
    let mut incoming = vec![Vec::new(); element_count];
    for (source, target) in edge_specs {
        let source = source % element_count;
        let target = target % element_count;
        outgoing[source].push(target);
        incoming[target].push(source);
    }
    let mut traversal = ReferenceTraversal::new(elements, seed_slot, options.limit);
    if options.include_start {
        traversal.push_start();
        if traversal.at_limit() {
            return traversal.finish();
        }
    }
    while let Some((slot, depth)) = traversal.pop_frontier() {
        if depth >= options.max_depth {
            continue;
        }
        let next_depth = depth + 1;
        let reached_limit = match options.direction {
            TraversalDirection::Outgoing => traversal.visit_neighbors(&outgoing[slot], next_depth),
            TraversalDirection::Incoming => traversal.visit_neighbors(&incoming[slot], next_depth),
            TraversalDirection::Both => {
                traversal.visit_neighbors(&outgoing[slot], next_depth)
                    || traversal.visit_neighbors(&incoming[slot], next_depth)
            }
        };
        if reached_limit {
            return traversal.finish();
        }
    }
    traversal.finish()
}

/// Mutable state for reference traversal.
struct ReferenceTraversal<'elements> {
    /// Canonical element IDs by generated slot.
    elements: &'elements [oxgraph_db::ElementId],
    /// Maximum emitted row count.
    limit: usize,
    /// Generated slots already discovered.
    visited: BTreeSet<usize>,
    /// FIFO frontier of generated slots and depths.
    queue: VecDeque<(usize, usize)>,
    /// Emitted rows.
    rows: Vec<TraversalRow>,
    /// Seed slot.
    seed_slot: usize,
}

impl<'elements> ReferenceTraversal<'elements> {
    /// Creates reference traversal state.
    fn new(elements: &'elements [oxgraph_db::ElementId], seed_slot: usize, limit: usize) -> Self {
        let mut visited = BTreeSet::new();
        visited.insert(seed_slot);
        let mut queue = VecDeque::new();
        queue.push_back((seed_slot, 0));
        Self {
            elements,
            limit,
            visited,
            queue,
            rows: Vec::new(),
            seed_slot,
        }
    }

    /// Emits the seed row.
    fn push_start(&mut self) {
        self.rows.push(TraversalRow {
            element: self.elements[self.seed_slot],
            depth: 0,
        });
    }

    /// Pops one frontier item.
    fn pop_frontier(&mut self) -> Option<(usize, usize)> {
        self.queue.pop_front()
    }

    /// Returns whether the emitted row limit has been reached.
    const fn at_limit(&self) -> bool {
        self.rows.len() == self.limit
    }

    /// Visits reference neighbor slots.
    fn visit_neighbors(&mut self, neighbors: &[usize], depth: usize) -> bool {
        for neighbor in neighbors {
            if !self.visited.insert(*neighbor) {
                continue;
            }
            self.queue.push_back((*neighbor, depth));
            self.rows.push(TraversalRow {
                element: self.elements[*neighbor],
                depth,
            });
            if self.at_limit() {
                return true;
            }
        }
        false
    }

    /// Finishes traversal into rows.
    fn finish(self) -> Vec<TraversalRow> {
        self.rows
    }
}

/// Expected IDs produced by generated fixtures.
struct ExpectedIds {
    /// Element IDs in creation order.
    elements: Vec<oxgraph_db::ElementId>,
    /// Relation IDs in creation order.
    relations: Vec<oxgraph_db::RelationId>,
    /// Incidence IDs in creation order.
    incidences: Vec<oxgraph_db::IncidenceId>,
    /// Graph projection ID.
    graph_projection: oxgraph_db::ProjectionId,
    /// Hypergraph projection ID.
    hyper_projection: oxgraph_db::ProjectionId,
    /// Graph projection index ID.
    graph_projection_index: oxgraph_db::IndexId,
    /// Hypergraph projection index ID.
    hyper_projection_index: oxgraph_db::IndexId,
}

/// Loads a generated binary edge fixture.
fn load_generated(
    database: &mut Database,
    element_count: usize,
    edge_specs: &[(usize, usize)],
) -> Result<ExpectedIds, DbError> {
    let mut writer = database.begin_write()?;
    let source = writer.register_role("source")?;
    let target = writer.register_role("target")?;
    let edge_type = writer.register_relation_type("Edge")?;
    let mut elements = Vec::with_capacity(element_count);
    for _index in 0..element_count {
        elements.push(writer.create_element()?);
    }
    let mut relations = Vec::with_capacity(edge_specs.len());
    let mut incidences = Vec::with_capacity(edge_specs.len() * 2);
    for (source_slot, target_slot) in edge_specs {
        let relation = writer.create_relation()?;
        writer.set_relation_type(relation, edge_type)?;
        let source_incidence =
            writer.create_incidence(relation, elements[*source_slot % element_count], source)?;
        let target_incidence =
            writer.create_incidence(relation, elements[*target_slot % element_count], target)?;
        relations.push(relation);
        incidences.push(source_incidence);
        incidences.push(target_incidence);
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
    let graph_projection_index = writer.define_index(
        "edge_graph_projection",
        IndexDefinition::Projection {
            projection: graph_projection,
        },
    )?;
    let hyper_projection_index = writer.define_index(
        "edge_hyper_projection",
        IndexDefinition::Projection {
            projection: hyper_projection,
        },
    )?;
    writer.commit()?;
    Ok(ExpectedIds {
        elements,
        relations,
        incidences,
        graph_projection,
        hyper_projection,
        graph_projection_index,
        hyper_projection_index,
    })
}

/// Loads one directed hyperedge fixture.
fn load_one_hyperedge(
    database: &mut Database,
    element_count: usize,
    target_count: usize,
) -> Result<ExpectedIds, DbError> {
    let mut writer = database.begin_write()?;
    let source = writer.register_role("source")?;
    let target = writer.register_role("target")?;
    let event_type = writer.register_relation_type("Event")?;
    let mut elements = Vec::with_capacity(element_count);
    for _index in 0..element_count {
        elements.push(writer.create_element()?);
    }
    let relation = writer.create_relation()?;
    writer.set_relation_type(relation, event_type)?;
    let mut incidences = Vec::with_capacity(target_count.min(element_count - 1) + 1);
    incidences.push(writer.create_incidence(relation, elements[0], source)?);
    for element in elements.iter().skip(1).take(target_count) {
        incidences.push(writer.create_incidence(relation, *element, target)?);
    }
    let graph_projection =
        writer.define_projection(ProjectionDefinition::Graph(GraphProjectionDefinition {
            name: "event_graph".to_owned(),
            relation_types: BTreeSet::new(),
            source_role: source,
            target_role: target,
        }))?;
    let hyper_projection = writer.define_projection(ProjectionDefinition::Hypergraph(
        HypergraphProjectionDefinition {
            name: "event_hyper".to_owned(),
            relation_types: BTreeSet::from([event_type]),
            source_roles: BTreeSet::from([source]),
            target_roles: BTreeSet::from([target]),
        },
    ))?;
    let graph_projection_index = writer.define_index(
        "event_graph_projection",
        IndexDefinition::Projection {
            projection: graph_projection,
        },
    )?;
    let hyper_projection_index = writer.define_index(
        "event_hyper_projection",
        IndexDefinition::Projection {
            projection: hyper_projection,
        },
    )?;
    writer.commit()?;
    Ok(ExpectedIds {
        elements,
        relations: vec![relation],
        incidences,
        graph_projection,
        hyper_projection,
        graph_projection_index,
        hyper_projection_index,
    })
}
