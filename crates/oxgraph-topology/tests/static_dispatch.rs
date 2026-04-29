//! Tests that `oxgraph-topology` traits support static-dispatch generic consumers.

use oxgraph_topology::{
    ContainsElement, ContainsIncidence, ContainsRelation, ElementIncidenceCount, ElementIndex,
    IncidenceBase, IncidenceCounts, IncidenceElement, IncidenceIndex, IncidenceRelation,
    IncidenceRole, IncidenceView, RelationIncidenceCount, RelationIncidences, RelationIndex,
    TopologyBase, TopologyCounts,
};
use proptest::prelude::*;

/// Element identifier used by the test fixture.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Element(usize);

/// Relation identifier used by the test fixture.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Relation(usize);

/// Incidence identifier used by the test fixture.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Incidence(usize);

/// Role used by the test fixture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FixtureRole {
    /// First endpoint or participant.
    First,
    /// Second endpoint or participant.
    Second,
    /// Additional endpoint or participant.
    Extra,
}

/// One incidence record in the fixture.
#[derive(Clone, Copy, Debug)]
struct IncidenceRecord {
    /// Element participating in the relation.
    element: Element,
    /// Relation containing the incidence.
    relation: Relation,
    /// Role of the participation.
    role: FixtureRole,
}

/// Tiny topology fixture with fixed relation offsets.
#[derive(Debug)]
struct FixtureTopology {
    /// Number of elements in the fixture.
    element_count: usize,
    /// Relation start offsets into `incidences`.
    relation_offsets: &'static [usize],
    /// Flat incidence records.
    incidences: &'static [IncidenceRecord],
}

impl TopologyBase for FixtureTopology {
    type ElementId = Element;
    type RelationId = Relation;
}

impl IncidenceBase for FixtureTopology {
    type IncidenceId = Incidence;
    type Role = FixtureRole;
}

impl TopologyCounts for FixtureTopology {
    fn element_count(&self) -> usize {
        self.element_count
    }

    fn relation_count(&self) -> usize {
        self.relation_offsets.len() - 1
    }
}

impl IncidenceCounts for FixtureTopology {
    fn incidence_count(&self) -> usize {
        self.incidences.len()
    }
}

impl ElementIndex for FixtureTopology {
    fn element_bound(&self) -> usize {
        self.element_count
    }

    fn element_index(&self, element: Element) -> usize {
        element.0
    }
}

impl ContainsElement for FixtureTopology {
    fn contains_element(&self, element: Element) -> bool {
        element.0 < self.element_count
    }
}

impl RelationIndex for FixtureTopology {
    fn relation_bound(&self) -> usize {
        self.relation_offsets.len() - 1
    }

    fn relation_index(&self, relation: Relation) -> usize {
        relation.0
    }
}

impl ContainsRelation for FixtureTopology {
    fn contains_relation(&self, relation: Relation) -> bool {
        relation.0 < self.relation_offsets.len() - 1
    }
}

impl IncidenceIndex for FixtureTopology {
    fn incidence_bound(&self) -> usize {
        self.incidences.len()
    }

    fn incidence_index(&self, incidence: Incidence) -> usize {
        incidence.0
    }
}

impl ContainsIncidence for FixtureTopology {
    fn contains_incidence(&self, incidence: Incidence) -> bool {
        incidence.0 < self.incidences.len()
    }
}

impl RelationIncidences for FixtureTopology {
    type Incidences<'view>
        = RelationIncidenceIter
    where
        Self: 'view;

    fn relation_incidences(&self, relation: Relation) -> Self::Incidences<'_> {
        let start = self.relation_offsets[relation.0];
        let end = self.relation_offsets[relation.0 + 1];
        RelationIncidenceIter { next: start, end }
    }
}

impl IncidenceElement for FixtureTopology {
    fn incidence_element(&self, incidence: Incidence) -> Element {
        self.incidences[incidence.0].element
    }
}

impl IncidenceRelation for FixtureTopology {
    fn incidence_relation(&self, incidence: Incidence) -> Relation {
        self.incidences[incidence.0].relation
    }
}

impl IncidenceRole for FixtureTopology {
    fn incidence_role(&self, incidence: Incidence) -> FixtureRole {
        self.incidences[incidence.0].role
    }
}

impl RelationIncidenceCount for FixtureTopology {
    fn relation_incidence_count(&self, relation: Relation) -> usize {
        self.relation_offsets[relation.0 + 1] - self.relation_offsets[relation.0]
    }
}

impl ElementIncidenceCount for FixtureTopology {
    fn element_incidence_count(&self, element: Element) -> usize {
        self.incidences
            .iter()
            .filter(|record| record.element == element)
            .count()
    }
}

/// Iterator over contiguous incidence IDs for one fixture relation.
#[derive(Debug)]
struct RelationIncidenceIter {
    /// Next flat incidence index to yield.
    next: usize,
    /// Exclusive end of the relation's incidence range.
    end: usize,
}

impl Iterator for RelationIncidenceIter {
    type Item = Incidence;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }

        let incidence = Incidence(self.next);
        self.next += 1;
        Some(incidence)
    }
}

/// Returns the elements participating in one relation through static dispatch.
fn relation_elements<T>(topology: &T, relation: T::RelationId) -> Vec<T::ElementId>
where
    T: RelationIncidences + IncidenceElement,
{
    topology
        .relation_incidences(relation)
        .map(|incidence| topology.incidence_element(incidence))
        .collect()
}

/// Returns relation roles through the complete incidence-view bundle.
fn relation_roles<T>(topology: &T, relation: T::RelationId) -> Vec<T::Role>
where
    T: IncidenceView,
{
    topology
        .relation_incidences(relation)
        .map(|incidence| topology.incidence_role(incidence))
        .collect()
}

/// Returns a fixture shaped like one binary edge and one ternary hyperedge.
fn fixture() -> FixtureTopology {
    static OFFSETS: &[usize] = &[0, 2, 5];
    static INCIDENCES: &[IncidenceRecord] = &[
        IncidenceRecord {
            element: Element(0),
            relation: Relation(0),
            role: FixtureRole::First,
        },
        IncidenceRecord {
            element: Element(1),
            relation: Relation(0),
            role: FixtureRole::Second,
        },
        IncidenceRecord {
            element: Element(0),
            relation: Relation(1),
            role: FixtureRole::First,
        },
        IncidenceRecord {
            element: Element(1),
            relation: Relation(1),
            role: FixtureRole::Second,
        },
        IncidenceRecord {
            element: Element(2),
            relation: Relation(1),
            role: FixtureRole::Extra,
        },
    ];

    FixtureTopology {
        element_count: 3,
        relation_offsets: OFFSETS,
        incidences: INCIDENCES,
    }
}

#[test]
fn generic_consumer_reads_relation_elements() {
    let topology = fixture();

    assert_eq!(
        relation_elements(&topology, Relation(0)),
        [Element(0), Element(1)]
    );
    assert_eq!(
        relation_elements(&topology, Relation(1)),
        [Element(0), Element(1), Element(2)]
    );
}

#[test]
fn incidence_records_resolve_both_sides_and_roles() {
    let topology = fixture();

    assert_eq!(topology.incidence_relation(Incidence(4)), Relation(1));
    assert_eq!(topology.incidence_element(Incidence(4)), Element(2));
    assert_eq!(topology.incidence_role(Incidence(4)), FixtureRole::Extra);
}

#[test]
fn dense_indexes_are_within_bounds() {
    let topology = fixture();

    assert_eq!(topology.element_bound(), 3);
    assert_eq!(topology.relation_bound(), 2);
    assert_eq!(topology.incidence_bound(), 5);

    for element in [Element(0), Element(1), Element(2)] {
        assert!(topology.element_index(element) < topology.element_bound());
    }

    for relation in [Relation(0), Relation(1)] {
        assert!(topology.relation_index(relation) < topology.relation_bound());
    }

    for incidence in [
        Incidence(0),
        Incidence(1),
        Incidence(2),
        Incidence(3),
        Incidence(4),
    ] {
        assert!(topology.incidence_index(incidence) < topology.incidence_bound());
    }
}

#[test]
fn dense_indexes_are_distinct_for_visible_ids() {
    let topology = fixture();

    assert_ne!(
        topology.element_index(Element(0)),
        topology.element_index(Element(1))
    );
    assert_ne!(
        topology.relation_index(Relation(0)),
        topology.relation_index(Relation(1))
    );
    assert_ne!(
        topology.incidence_index(Incidence(0)),
        topology.incidence_index(Incidence(1))
    );
}

#[test]
fn containment_reports_visible_ids() {
    let topology = fixture();

    assert!(topology.contains_element(Element(2)));
    assert!(!topology.contains_element(Element(3)));
    assert!(topology.contains_relation(Relation(1)));
    assert!(!topology.contains_relation(Relation(2)));
    assert!(topology.contains_incidence(Incidence(4)));
    assert!(!topology.contains_incidence(Incidence(5)));
}

#[test]
fn incidence_view_blanket_impl_resolves_roles() {
    let topology = fixture();

    assert_eq!(
        relation_roles(&topology, Relation(1)),
        [FixtureRole::First, FixtureRole::Second, FixtureRole::Extra]
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Relation incidence count matches traversal count for all fixture relations.
    #[test]
    fn relation_count_matches_iterator_count(index in 0usize..2) {
        let topology = fixture();
        let relation = Relation(index);

        prop_assert_eq!(
            topology.relation_incidence_count(relation),
            topology.relation_incidences(relation).count()
        );
    }

    /// Element incidence count matches scanning the fixture incidence records.
    #[test]
    fn element_count_matches_fixture_records(index in 0usize..3) {
        let topology = fixture();
        let element = Element(index);
        let expected = topology.incidences
            .iter()
            .filter(|record| record.element == element)
            .count();

        prop_assert_eq!(topology.element_incidence_count(element), expected);
    }
}
