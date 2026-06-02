//! Tests for the overlay state model: the Cow merge, k-way iteration, the
//! differential proptest against a `HashMap` oracle, and the published-overlay
//! immutability/clone-and-apply contract.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

use proptest::prelude::*;

use super::{
    BaseRecords, MergedState, Overlay, OverlayLayer, Snapshot, StateView, WriteOverlay,
    test_support::small_base,
};
use crate::{
    ElementId, IncidenceId, PropertyKeyId, RelationId, RoleId,
    backing::Base,
    id::{CheckpointGeneration, CommitSeq},
    state::{ElementRecord, IncidenceRecord, NextIds, PropertySubject, RelationRecord},
    value::PropertyValue,
};

/// Reads the base's watermark out of its header.
fn base_next_ids(base: &Base) -> NextIds {
    let header = *base.get().header();
    NextIds {
        element: ElementId::new(header.next_element),
        relation: RelationId::new(header.next_relation),
        incidence: IncidenceId::new(header.next_incidence),
        role: RoleId::new(header.next_role),
        label: crate::LabelId::new(header.next_label),
        relation_type: crate::RelationTypeId::new(header.next_relation_type),
        property_key: PropertyKeyId::new(header.next_property_key),
        projection: crate::ProjectionId::new(header.next_projection),
        index: crate::IndexId::new(header.next_index),
    }
}

/// An empty overlay over a base returns every base record borrowed (zero clone)
/// and matches the base exactly on counts.
#[test]
fn empty_overlay_borrows_base() {
    let base = small_base();
    let records = BaseRecords::from_view(base.get()).expect("base records");
    let overlay = Overlay::empty(base_next_ids(&base), base.get().catalog().clone());
    let view = MergedState::new(&records, &overlay);

    // Three elements, all borrowed.
    assert_eq!(view.element_count(), 3);
    for raw in 1u64..=3 {
        let record = view.element(ElementId::new(raw)).expect("element present");
        assert!(matches!(record, Cow::Borrowed(_)), "base read must borrow");
        assert_eq!(record.id, ElementId::new(raw));
    }
    assert_eq!(view.relation_count(), 2);
    assert_eq!(view.incidence_count(), 2);
}

/// A base-only id reads `Cow::Borrowed` (no clone); an overlay-supplied or
/// overlay-overridden id reads `Cow::Owned`; a tombstoned id reads `None`.
#[test]
fn cow_borrowed_fast_path_owned_override() {
    let base = small_base();
    let records = BaseRecords::from_view(base.get()).expect("base records");

    let mut write = WriteOverlay::new(base_next_ids(&base), base.get().catalog().clone());
    // Override element 2's labels (forces an owned overlay record).
    let robot = base.get().catalog().label_id("Robot").expect("robot");
    write.add_element_label(&records, ElementId::new(2), robot);
    // Tombstone element 3.
    write.tombstone_element(&records, ElementId::new(3));
    // Create a brand-new element (overlay-only).
    let fresh = write.create_element().expect("fresh element");
    let overlay = write.freeze();
    let view = MergedState::new(&records, &overlay);

    // Base-only id: borrowed.
    let e1 = view.element(ElementId::new(1)).expect("e1");
    assert!(matches!(e1, Cow::Borrowed(_)), "base-only id must borrow");

    // Overlay-overridden id: owned.
    let e2 = view.element(ElementId::new(2)).expect("e2");
    assert!(matches!(e2, Cow::Owned(_)), "overridden id must be owned");

    // Tombstoned id: absent.
    assert!(view.element(ElementId::new(3)).is_none(), "tombstone hides");

    // Overlay-only id: owned and present.
    let fresh_read = view.element(fresh).expect("fresh present");
    assert!(
        matches!(fresh_read, Cow::Owned(_)),
        "fresh id must be owned"
    );
}

/// A property base read borrows; an overlay-set property is owned; an
/// overlay-removed property is absent.
#[test]
fn cow_property_fast_path() {
    let base = small_base();
    let records = BaseRecords::from_view(base.get()).expect("base records");
    let name = base.get().catalog().property_key_id("name").expect("name");
    let rank = base.get().catalog().property_key_id("rank").expect("rank");

    let mut write = WriteOverlay::new(base_next_ids(&base), base.get().catalog().clone());
    // Override element 1's name.
    write.set_property(
        PropertySubject::Element(ElementId::new(1)),
        name,
        PropertyValue::Text("Alicia".to_owned()),
    );
    // Remove element 1's rank.
    write.remove_property(PropertySubject::Element(ElementId::new(1)), rank);
    let overlay = write.freeze();
    let view = MergedState::new(&records, &overlay);

    // Base-only property (element 2's name): borrowed.
    let bob = view
        .property(PropertySubject::Element(ElementId::new(2)), name)
        .expect("bob name");
    assert!(matches!(bob, Cow::Borrowed(_)), "base property must borrow");
    assert_eq!(bob.into_owned(), PropertyValue::Text("Bob".to_owned()));

    // Overlay-set property: owned.
    let alicia = view
        .property(PropertySubject::Element(ElementId::new(1)), name)
        .expect("alicia name");
    assert!(matches!(alicia, Cow::Owned(_)), "overlay property is owned");
    assert_eq!(
        alicia.into_owned(),
        PropertyValue::Text("Alicia".to_owned())
    );

    // Overlay-removed property: absent.
    assert!(
        view.property(PropertySubject::Element(ElementId::new(1)), rank)
            .is_none(),
        "removed property is hidden"
    );
}

/// Tombstoning is idempotent: tombstoning an absent id is a no-op (still
/// absent), and double-tombstoning collapses to a single tombstone with the same
/// visible result.
#[test]
fn tombstone_idempotent() {
    let base = small_base();
    let records = BaseRecords::from_view(base.get()).expect("base records");

    let mut write = WriteOverlay::new(base_next_ids(&base), base.get().catalog().clone());
    // Tombstone an id that does not exist in the base.
    let absent = ElementId::new(999);
    write.tombstone_element(&records, absent);
    // Double-tombstone a present base id.
    write.tombstone_element(&records, ElementId::new(1));
    write.tombstone_element(&records, ElementId::new(1));
    let overlay = write.freeze();
    let view = MergedState::new(&records, &overlay);

    assert!(
        view.element(absent).is_none(),
        "absent tombstone stays absent"
    );
    assert!(
        view.element(ElementId::new(1)).is_none(),
        "double tombstone hides"
    );
    // Element 2 and 3 remain.
    assert_eq!(view.element_count(), 2);
}

/// Characterizes the overlay's permissiveness: the overlay is an UNVALIDATED
/// delta whose property layer and record layer are independent, so a
/// `set_property` recorded AFTER the subject is tombstoned in the SAME writer
/// surfaces an ORPHAN property — visible through `property` / `properties` /
/// `property_equal` even though the subject element reads back absent.
///
/// Referential integrity is enforced one layer up, at the `WriteTransaction`
/// boundary, which rejects a `set_property` against an absent/tombstoned subject
/// (see the `database` tests). This unit test locks the lower-layer divergence
/// the `WriteOverlay::set_property` doc promises so that gate has a
/// characterization to sit above.
#[test]
fn overlay_records_orphan_property_unvalidated() {
    let base = small_base();
    let records = BaseRecords::from_view(base.get()).expect("base records");
    let name = base.get().catalog().property_key_id("name").expect("name");
    let subject = PropertySubject::Element(ElementId::new(1));
    let orphan = PropertyValue::Text("ghost".to_owned());

    // Overlay: tombstone element 1, THEN set a property on it. The tombstone
    // clears element 1's property delta and masks its base properties; the
    // later set re-records a visible value for the now-deleted subject.
    let mut write = WriteOverlay::new(base_next_ids(&base), base.get().catalog().clone());
    write.tombstone_element(&records, ElementId::new(1));
    write.set_property(subject, name, orphan.clone());
    let overlay = write.freeze();
    let view = MergedState::new(&records, &overlay);

    // The subject is gone, but the property is visible through every read path:
    // the overlay is permissive by design (referential integrity is the write
    // transaction's job).
    assert!(
        view.element(ElementId::new(1)).is_none(),
        "subject tombstoned"
    );
    assert_eq!(
        view.property(subject, name).map(Cow::into_owned),
        Some(orphan.clone()),
        "orphan property is visible through point read"
    );
    assert!(
        view.properties()
            .any(|(found_subject, found_key, value)| found_subject == subject
                && found_key == name
                && value.as_ref() == &orphan),
        "orphan property is visible through the property iterator"
    );
    assert!(
        view.property_equal(name, &orphan).contains(&subject),
        "orphan property is visible through property_equal"
    );
}

/// `Snapshot` wires base + overlay into a pinnable unit and `view()` merges
/// correctly; `(generation, lsn)` identity is preserved.
#[test]
fn snapshot_view_merges() {
    let base = Arc::new(small_base());
    let next = base_next_ids(&base);
    let catalog = base.get().catalog().clone();
    let overlay = Arc::new(Overlay::empty(next, catalog));
    let snapshot = Snapshot::new(
        CheckpointGeneration::new(7),
        CommitSeq::new(42),
        Arc::clone(&base),
        Arc::clone(&overlay),
    )
    .expect("snapshot");

    assert_eq!(snapshot.generation(), CheckpointGeneration::new(7));
    assert_eq!(snapshot.lsn(), CommitSeq::new(42));
    let view = snapshot.view();
    assert_eq!(view.element_count(), 3);
    assert_eq!(view.relation_count(), 2);
}

/// `with_applied` builds a fresh overlay from a parent plus a writer delta
/// WITHOUT mutating the parent: the parent's visible state is unchanged after
/// the child is built, the child reflects the delta, and the watermark advances
/// monotonically.
#[test]
fn with_applied_leaves_parent_frozen() {
    let base = small_base();
    let records = BaseRecords::from_view(base.get()).expect("base records");
    let next = base_next_ids(&base);
    let catalog = base.get().catalog().clone();

    // Parent overlay tombstones element 1.
    let mut parent_write = WriteOverlay::new(next, catalog);
    parent_write.tombstone_element(&records, ElementId::new(1));
    let parent = Arc::new(parent_write.freeze());

    // A reader pins the parent.
    let parent_pin = Arc::clone(&parent);
    let parent_view_before = MergedState::new(&records, &parent_pin);
    let parent_count_before = parent_view_before.element_count();

    // Child delta creates a new element and tombstones element 2.
    let mut child_write = WriteOverlay::new(parent.next_ids(), parent.catalog().clone());
    let fresh = child_write.create_element().expect("fresh");
    child_write.tombstone_element(&records, ElementId::new(2));
    let child = parent.with_applied(&child_write);

    // The parent is untouched: same visible count, element 2 still visible, the
    // fresh id absent in the parent.
    let parent_view_after = MergedState::new(&records, &parent_pin);
    assert_eq!(parent_view_after.element_count(), parent_count_before);
    assert!(parent_view_after.element(ElementId::new(2)).is_some());
    assert!(parent_view_after.element(fresh).is_none());

    // The child reflects the delta on top of the parent.
    let child_view = MergedState::new(&records, &child);
    assert!(
        child_view.element(ElementId::new(1)).is_none(),
        "parent tombstone carries"
    );
    assert!(
        child_view.element(ElementId::new(2)).is_none(),
        "child tombstone"
    );
    assert!(child_view.element(fresh).is_some(), "child create");

    // Watermark advanced monotonically.
    assert!(child.next_ids().element > parent.next_ids().element);
}

/// An oracle model of the merged visible state: a HashMap-backed apply of the
/// overlay delta onto the base records.
struct Oracle {
    /// Visible elements by id.
    elements: HashMap<ElementId, ElementRecord>,
    /// Visible relations by id.
    relations: HashMap<RelationId, RelationRecord>,
    /// Visible incidences by id.
    incidences: HashMap<IncidenceId, IncidenceRecord>,
    /// Visible property values by `(subject, key)`.
    properties: HashMap<(PropertySubject, PropertyKeyId), PropertyValue>,
}

/// A single random overlay operation the proptest applies to BOTH the
/// `WriteOverlay` under test and the oracle.
#[derive(Clone, Debug)]
enum Op {
    /// Create a new element.
    CreateElement,
    /// Create a new relation.
    CreateRelation,
    /// Tombstone the base element with id `1 + (n % base_elements)` (or absent).
    TombstoneElement(u64),
    /// Set the `name` property on the base element with id `1 + (n % 3)`.
    SetName(u64, String),
    /// Remove the `rank` property on the base element with id `1 + (n % 3)`.
    RemoveRank(u64),
}

/// Applies `op` to the `WriteOverlay` under test.
fn apply_to_write(
    write: &mut WriteOverlay,
    records: &BaseRecords,
    name: PropertyKeyId,
    rank: PropertyKeyId,
    op: &Op,
) {
    match op {
        Op::CreateElement => {
            write.create_element().expect("create element");
        }
        Op::CreateRelation => {
            write.create_relation().expect("create relation");
        }
        Op::TombstoneElement(raw) => {
            write.tombstone_element(records, ElementId::new(1 + (raw % 4)));
        }
        Op::SetName(raw, text) => {
            write.set_property(
                PropertySubject::Element(ElementId::new(1 + (raw % 3))),
                name,
                PropertyValue::Text(text.clone()),
            );
        }
        Op::RemoveRank(raw) => {
            let _ = records;
            write.remove_property(
                PropertySubject::Element(ElementId::new(1 + (raw % 3))),
                rank,
            );
        }
    }
}

/// Applies `op` to the oracle, mirroring the overlay semantics: a tombstoned
/// element drops its properties, a create allocates the next id from a cursor.
#[expect(
    clippy::too_many_arguments,
    reason = "the oracle apply threads every dimension the overlay tracks (record maps, the two property keys, and the id cursors) so the proptest can mirror exactly one overlay mutation per op"
)]
fn apply_to_oracle(
    oracle: &mut Oracle,
    name: PropertyKeyId,
    rank: PropertyKeyId,
    next_element: &mut u64,
    next_relation: &mut u64,
    op: &Op,
) {
    match op {
        Op::CreateElement => {
            let id = ElementId::new(*next_element);
            *next_element += 1;
            oracle.elements.insert(
                id,
                ElementRecord {
                    id,
                    labels: BTreeSet::new(),
                },
            );
        }
        Op::CreateRelation => {
            let id = RelationId::new(*next_relation);
            *next_relation += 1;
            oracle.relations.insert(
                id,
                RelationRecord {
                    id,
                    relation_type: None,
                    labels: BTreeSet::new(),
                },
            );
        }
        Op::TombstoneElement(raw) => {
            let id = ElementId::new(1 + (raw % 4));
            oracle.elements.remove(&id);
            oracle
                .properties
                .retain(|(subject, _key), _value| *subject != PropertySubject::Element(id));
        }
        Op::SetName(raw, text) => {
            let id = ElementId::new(1 + (raw % 3));
            // The overlay set does not resurrect a tombstoned element's record,
            // but the property value is still recorded for present subjects. The
            // proptest only sets names on ids 1..=3, which are base elements; an
            // earlier tombstone of the same id removes the element AND its
            // properties, and a later set re-adds only the property. Mirror that.
            oracle.properties.insert(
                (PropertySubject::Element(id), name),
                PropertyValue::Text(text.clone()),
            );
        }
        Op::RemoveRank(raw) => {
            let id = ElementId::new(1 + (raw % 3));
            oracle
                .properties
                .remove(&(PropertySubject::Element(id), rank));
        }
    }
}

/// Builds the oracle's initial visible state from the base records.
fn oracle_from_base(records: &BaseRecords, base_elements: &[ElementRecord]) -> Oracle {
    let mut elements = HashMap::new();
    for record in base_elements {
        elements.insert(record.id, record.clone());
    }
    let mut relations = HashMap::new();
    let mut incidences = HashMap::new();
    let mut properties = HashMap::new();
    // Read the base through an empty overlay to enumerate exactly the base
    // visible set (the merge of an empty overlay is the base).
    let empty = Overlay::empty(
        NextIds {
            element: ElementId::new(1),
            relation: RelationId::new(1),
            incidence: IncidenceId::new(1),
            role: RoleId::new(1),
            label: crate::LabelId::new(1),
            relation_type: crate::RelationTypeId::new(1),
            property_key: PropertyKeyId::new(1),
            projection: crate::ProjectionId::new(1),
            index: crate::IndexId::new(1),
        },
        crate::Catalog::empty(),
    );
    let view = MergedState::new(records, &empty);
    for relation in view.relations() {
        relations.insert(relation.id, relation.into_owned());
    }
    for incidence in view.incidences() {
        incidences.insert(incidence.id, incidence.into_owned());
    }
    for (subject, key, value) in view.properties() {
        properties.insert((subject, key), value.into_owned());
    }
    Oracle {
        elements,
        relations,
        incidences,
        properties,
    }
}

/// Proptest case budget: the full sweep on a native run, a small sweep under
/// miri (each case freezes and re-attaches a base over owned bytes, which is the
/// path miri certifies but is far slower interpreted, so a handful of cases keep
/// `cargo miri test overlay` tractable while still exercising the merge over
/// owned bytes).
#[cfg(miri)]
const MERGE_CASES: u32 = 4;
/// Full proptest case budget on a native run.
#[cfg(not(miri))]
const MERGE_CASES: u32 = 256;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(MERGE_CASES))]

    /// Differential test: a random write delta applied to a `WriteOverlay`
    /// merged over a frozen base reads EXACTLY equal to the same delta applied
    /// to a `HashMap` oracle of the base — same visible
    /// element/relation/incidence sets, same property values, tombstones
    /// removed, overlay overrides win, across point reads AND full iterations.
    #[test]
    fn merge_matches_oracle(
        ops in proptest::collection::vec(
            prop_oneof![
                Just(Op::CreateElement),
                Just(Op::CreateRelation),
                any::<u64>().prop_map(Op::TombstoneElement),
                (any::<u64>(), "[a-z]{1,5}").prop_map(|(raw, text)| Op::SetName(raw, text)),
                any::<u64>().prop_map(Op::RemoveRank),
            ],
            0..24,
        )
    ) {
        let base = small_base();
        let records = BaseRecords::from_view(base.get()).expect("base records");
        let name = base.get().catalog().property_key_id("name").expect("name");
        let rank = base.get().catalog().property_key_id("rank").expect("rank");

        // Seed both sides.
        let base_elements: Vec<ElementRecord> = {
            let empty = Overlay::empty(base_next_ids(&base), base.get().catalog().clone());
            MergedState::new(&records, &empty)
                .elements()
                .map(Cow::into_owned)
                .collect()
        };
        let mut oracle = oracle_from_base(&records, &base_elements);
        let mut write = WriteOverlay::new(base_next_ids(&base), base.get().catalog().clone());
        let mut next_element = base_next_ids(&base).element.get();
        let mut next_relation = base_next_ids(&base).relation.get();

        for op in &ops {
            apply_to_write(&mut write, &records, name, rank, op);
            apply_to_oracle(&mut oracle, name, rank, &mut next_element, &mut next_relation, op);
        }

        let overlay = write.freeze();
        let view = MergedState::new(&records, &overlay);

        // Point reads: every id the oracle considers visible reads back equal,
        // and every tombstoned base id reads back absent.
        for (id, record) in &oracle.elements {
            let read = view.element(*id).map(Cow::into_owned);
            prop_assert_eq!(read.as_ref(), Some(record), "element {} mismatch", id.get());
        }
        for raw in 1u64..=4 {
            let id = ElementId::new(raw);
            prop_assert_eq!(
                view.element(id).is_some(),
                oracle.elements.contains_key(&id),
                "element {} visibility mismatch",
                raw
            );
        }

        // Full iterations: the merged visible set equals the oracle set exactly.
        let merged_elements: BTreeMap<ElementId, ElementRecord> = view
            .elements()
            .map(|record| {
                let record = record.into_owned();
                (record.id, record)
            })
            .collect();
        let oracle_elements: BTreeMap<ElementId, ElementRecord> =
            oracle.elements.iter().map(|(id, record)| (*id, record.clone())).collect();
        prop_assert_eq!(merged_elements, oracle_elements, "element set mismatch");

        let merged_relations: BTreeMap<RelationId, RelationRecord> = view
            .relations()
            .map(|record| {
                let record = record.into_owned();
                (record.id, record)
            })
            .collect();
        let oracle_relations: BTreeMap<RelationId, RelationRecord> =
            oracle.relations.iter().map(|(id, record)| (*id, record.clone())).collect();
        prop_assert_eq!(merged_relations, oracle_relations, "relation set mismatch");

        let merged_incidences: BTreeMap<IncidenceId, IncidenceRecord> = view
            .incidences()
            .map(|record| {
                let record = record.into_owned();
                (record.id, record)
            })
            .collect();
        let oracle_incidences: BTreeMap<IncidenceId, IncidenceRecord> =
            oracle.incidences.iter().map(|(id, record)| (*id, *record)).collect();
        prop_assert_eq!(merged_incidences, oracle_incidences, "incidence set mismatch");

        let merged_properties: BTreeMap<(PropertySubject, PropertyKeyId), PropertyValue> = view
            .properties()
            .map(|(subject, key, value)| ((subject, key), value.into_owned()))
            .collect();
        let oracle_properties: BTreeMap<(PropertySubject, PropertyKeyId), PropertyValue> = oracle
            .properties
            .iter()
            .map(|(pair, value)| (*pair, value.clone()))
            .collect();
        prop_assert_eq!(merged_properties, oracle_properties, "property set mismatch");

        // Counts agree.
        prop_assert_eq!(view.element_count(), oracle.elements.len());
        prop_assert_eq!(view.relation_count(), oracle.relations.len());
        prop_assert_eq!(view.incidence_count(), oracle.incidences.len());
    }
}
