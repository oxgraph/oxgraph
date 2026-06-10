//! Tests for the overlay state model: the Cow merge, k-way iteration, the
//! differential proptest against a `HashMap` oracle, and the published-overlay
//! immutability/clone-and-apply contract.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use proptest::prelude::*;

use super::{
    BaseRecords, MergedState, Overlay, OverlayLayer, Snapshot, StateView, WriteOverlay,
    test_support::small_base,
};
use crate::{
    ElementId, IncidenceId, LabelId, PropertyKeyId, RelationId, RelationTypeId, RoleId,
    backing::Base,
    freeze::{FreezeStamps, freeze_view},
    id::{CheckpointGeneration, CommitSeq},
    state::{ElementRecord, IncidenceRecord, LabelSet, NextIds, PropertySubject, RelationRecord},
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
        &records,
        PropertySubject::Element(ElementId::new(1)),
        name,
        PropertyValue::from("Alicia"),
    );
    // Remove element 1's rank.
    write.remove_property(&records, PropertySubject::Element(ElementId::new(1)), rank);
    let overlay = write.freeze();
    let view = MergedState::new(&records, &overlay);

    // Base-only property (element 2's name): borrowed.
    let bob = view
        .property(PropertySubject::Element(ElementId::new(2)), name)
        .expect("bob name");
    assert!(matches!(bob, Cow::Borrowed(_)), "base property must borrow");
    assert_eq!(bob.into_owned(), PropertyValue::from("Bob"));

    // Overlay-set property: owned.
    let alicia = view
        .property(PropertySubject::Element(ElementId::new(1)), name)
        .expect("alicia name");
    assert!(matches!(alicia, Cow::Owned(_)), "overlay property is owned");
    assert_eq!(alicia.into_owned(), PropertyValue::from("Alicia"));

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
/// Referential integrity is enforced one layer up, at the `Writer`
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
    let orphan = PropertyValue::from("ghost");

    // Overlay: tombstone element 1, THEN set a property on it. The tombstone
    // clears element 1's property delta and masks its base properties; the
    // later set re-records a visible value for the now-deleted subject.
    let mut write = WriteOverlay::new(base_next_ids(&base), base.get().catalog().clone());
    write.tombstone_element(&records, ElementId::new(1));
    write.set_property(&records, subject, name, orphan.clone());
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
    let base_records = Arc::new(BaseRecords::from_view(base.get()).expect("base records"));
    let snapshot = Snapshot::with_shared_base_records(
        CheckpointGeneration::new(7),
        CommitSeq::new(42),
        Arc::clone(&base),
        Arc::clone(&overlay),
        base_records,
    );

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
                records,
                PropertySubject::Element(ElementId::new(1 + (raw % 3))),
                name,
                PropertyValue::from(text.clone()),
            );
        }
        Op::RemoveRank(raw) => {
            write.remove_property(
                records,
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
                    labels: LabelSet::default(),
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
                    labels: LabelSet::default(),
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
                PropertyValue::from(text.clone()),
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

/// One random index-affecting op applied to a `WriteOverlay`: label adds,
/// relation-type sets, integer-`rank` sets/removes, and element / relation /
/// incidence tombstones. The op alphabet deliberately spans every index
/// dimension (label membership, relation-type membership, equality, range) AND
/// every tombstone withdrawal path (element, typed/property-bearing relation,
/// property-bearing incidence) so the differential test below exercises each
/// posting path — including `OverlayIndex::on_tombstone_relation` (relation-type
/// + property withdrawal) and `on_tombstone_incidence` (property withdrawal).
#[derive(Clone, Debug)]
enum IndexOp {
    /// Add the `Robot` label to base element `1 + (n % 4)`.
    LabelElement(u64),
    /// Set base relation `1 + (n % 2)`'s type to `calls`.
    TypeRelation(u64),
    /// Set the integer `rank` of base element `1 + (n % 3)` to `value`.
    SetRank(u64, i64),
    /// Remove the `rank` of base element `1 + (n % 3)`.
    RemoveRank(u64),
    /// Tombstone base element `1 + (n % 4)`.
    Tombstone(u64),
    /// Tombstone base relation `1 + (n % 2)` (r1 is typed `calls` and carries a
    /// `weight` property; r2 is untyped) — exercises `on_tombstone_relation`,
    /// which withdraws the relation-type and property postings.
    TombstoneRelation(u64),
    /// Tombstone base incidence `1 + (n % 2)` (inc1 carries a `slot` property) —
    /// exercises `on_tombstone_incidence`, which withdraws the property postings.
    TombstoneIncidence(u64),
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(MERGE_CASES))]

    /// Differential test for the index-backed lookups: a random delta applied
    /// to a `WriteOverlay` makes every index-backed `StateView` lookup
    /// (`elements_with_label`, `relations_with_type`, `property_equal`,
    /// `property_range`, `typed_property_composite_equal`) return EXACTLY what the
    /// surviving merge-scan oracle (`*_scan`) returns — across label/type
    /// membership, equality, range (including inverted and partial ranges), and
    /// composite equality — over a state mixing base records, overlay overrides,
    /// and element / relation / incidence tombstones (so the relation-type and
    /// relation/incidence property withdrawal paths are checked, not just the
    /// element half).
    #[test]
    fn index_lookups_match_scan_oracle(
        ops in proptest::collection::vec(
            prop_oneof![
                any::<u64>().prop_map(IndexOp::LabelElement),
                any::<u64>().prop_map(IndexOp::TypeRelation),
                (any::<u64>(), -8i64..8).prop_map(|(raw, value)| IndexOp::SetRank(raw, value)),
                any::<u64>().prop_map(IndexOp::RemoveRank),
                any::<u64>().prop_map(IndexOp::Tombstone),
                any::<u64>().prop_map(IndexOp::TombstoneRelation),
                any::<u64>().prop_map(IndexOp::TombstoneIncidence),
            ],
            0..24,
        )
    ) {
        let base = small_base();
        let records = BaseRecords::from_view(base.get()).expect("base records");
        let catalog = base.get().catalog();
        let robot = catalog.label_id("Robot").expect("robot label");
        let person = catalog.label_id("Person").expect("person label");
        let calls = catalog.relation_type_id("calls").expect("calls type");
        let name = catalog.property_key_id("name").expect("name key");
        let rank = catalog.property_key_id("rank").expect("rank key");
        let weight = catalog.property_key_id("weight").expect("weight key");
        let slot = catalog.property_key_id("slot").expect("slot key");

        let mut write = WriteOverlay::new(base_next_ids(&base), catalog.clone());
        for op in &ops {
            match op {
                IndexOp::LabelElement(raw) => {
                    write.add_element_label(&records, ElementId::new(1 + (raw % 4)), robot);
                }
                IndexOp::TypeRelation(raw) => {
                    write.set_relation_type(&records, RelationId::new(1 + (raw % 2)), calls);
                }
                IndexOp::SetRank(raw, value) => {
                    write.set_property(
                        &records,
                        PropertySubject::Element(ElementId::new(1 + (raw % 3))),
                        rank,
                        PropertyValue::Integer(*value),
                    );
                }
                IndexOp::RemoveRank(raw) => {
                    write.remove_property(
                        &records,
                        PropertySubject::Element(ElementId::new(1 + (raw % 3))),
                        rank,
                    );
                }
                IndexOp::Tombstone(raw) => {
                    write.tombstone_element(&records, ElementId::new(1 + (raw % 4)));
                }
                IndexOp::TombstoneRelation(raw) => {
                    write.tombstone_relation(&records, RelationId::new(1 + (raw % 2)));
                }
                IndexOp::TombstoneIncidence(raw) => {
                    write.tombstone_incidence(&records, IncidenceId::new(1 + (raw % 2)));
                }
            }
        }
        let overlay = write.freeze();
        let view = MergedState::new(&records, &overlay);

        // Label + relation-type membership: index == scan. The relation-type
        // assertion now exercises the `on_tombstone_relation` withdrawal path:
        // tombstoning typed r1 must drop it from the `calls` posting.
        for label in [person, robot] {
            prop_assert_eq!(
                view.elements_with_label(label),
                view.elements_with_label_scan(label),
                "label {} membership index != scan",
                label.get()
            );
        }
        prop_assert_eq!(
            view.relations_with_type(calls),
            view.relations_with_type_scan(calls),
            "relation-type membership index != scan"
        );

        // Reverse adjacency: the element- and relation-incidence index must equal
        // the scan oracle over every visible subject, exercising the
        // on_create_incidence / on_tombstone_incidence posting maintenance driven
        // by the random op sequence above.
        let element_ids: Vec<_> = view.elements().map(|record| record.id).collect();
        for element in element_ids {
            let mut indexed = view.element_incidences(element);
            let mut scanned = view.element_incidences_scan(element);
            indexed.sort_by_key(|record| record.id);
            scanned.sort_by_key(|record| record.id);
            prop_assert_eq!(
                indexed,
                scanned,
                "element {} adjacency index != scan",
                element.get()
            );
        }
        let relation_ids: Vec<_> = view.relations().map(|record| record.id).collect();
        for relation in relation_ids {
            let mut indexed = view.relation_incidences(relation);
            let mut scanned = view.relation_incidences_scan(relation);
            indexed.sort_by_key(|record| record.id);
            scanned.sort_by_key(|record| record.id);
            prop_assert_eq!(
                indexed,
                scanned,
                "relation {} adjacency index != scan",
                relation.get()
            );
        }

        // Relation- and incidence-family equality: the `on_tombstone_relation` /
        // `on_tombstone_incidence` property-posting withdrawals must match the
        // scan oracle. `weight=3` is r1's relation property; `slot=1` is inc1's
        // incidence property — both vanish from the index when tombstoned.
        for value in -1i64..=4 {
            let probe = PropertyValue::Integer(value);
            prop_assert_eq!(
                view.property_equal(weight, &probe),
                view.property_equal_scan(weight, &probe),
                "relation-family equality index != scan for weight={}",
                value
            );
            prop_assert_eq!(
                view.property_equal(slot, &probe),
                view.property_equal_scan(slot, &probe),
                "incidence-family equality index != scan for slot={}",
                value
            );
        }

        // Equality over every value the rank could hold (and a value never set).
        for value in -9i64..=9 {
            let probe = PropertyValue::Integer(value);
            prop_assert_eq!(
                view.property_equal(rank, &probe),
                view.property_equal_scan(rank, &probe),
                "equality index != scan for rank={}",
                value
            );
        }
        // A name (text) equality probe exercises the text-keyed posting too.
        let bob = PropertyValue::from("Bob");
        prop_assert_eq!(
            view.property_equal(name, &bob),
            view.property_equal_scan(name, &bob),
            "text equality index != scan"
        );

        // Range over several windows (full, partial, single, inverted).
        for (lo, hi) in [(-9i64, 9i64), (-3, 3), (0, 0), (2, 1)] {
            let min = PropertyValue::Integer(lo);
            let max = PropertyValue::Integer(hi);
            let mut indexed = view.property_range(rank, &min, &max);
            let mut scanned = view.property_range_scan(rank, &min, &max);
            indexed.sort();
            scanned.sort();
            prop_assert_eq!(indexed, scanned, "range index != scan for [{}, {}]", lo, hi);
        }

        // Composite equality over (rank, name) tuples (validated path == scan).
        for (rank_value, person_name) in [(-5i64, "Alice"), (0, "Bob"), (7, "Carol")] {
            let keys = [rank, name];
            let values = [
                PropertyValue::Integer(rank_value),
                PropertyValue::from(person_name),
            ];
            let mut indexed = view
                .typed_property_composite_equal(&keys, &values)
                .expect("composite lookup");
            let mut scanned = view.property_composite_equal_scan(&keys, &values);
            indexed.sort();
            scanned.sort();
            prop_assert_eq!(
                indexed,
                scanned,
                "composite index != scan for (rank={}, name={})",
                rank_value,
                person_name
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(MERGE_CASES))]

    /// Differential test for the PERSISTED (borrowed) base index vs the
    /// `from_records` owned oracle: a random store (built by applying random ops
    /// to `small_base`, then folding the merged state into fresh frozen base
    /// bytes) is opened TWO ways — `BaseRecords::open` borrows the persisted
    /// `SECTION_INDEX_*` postings zero-copy, while `BaseRecords::from_view`
    /// rebuilds the owned index from the decoded records. Every index accessor
    /// (`elements_with_label`, `relations_with_type`, `property_equal` over
    /// present AND absent keys, `property_range` over full/partial/single/inverted
    /// sub-ranges, `element_incidences`, `relation_incidences`,
    /// `property_key_subjects`) MUST return byte-identical ascending sequences over
    /// the two arms. Driven through an EMPTY overlay so each lookup reduces to the
    /// pure base posting.
    #[test]
    fn borrowed_index_matches_owned_oracle(
        ops in proptest::collection::vec(
            prop_oneof![
                any::<u64>().prop_map(IndexOp::LabelElement),
                any::<u64>().prop_map(IndexOp::TypeRelation),
                (any::<u64>(), -8i64..8).prop_map(|(raw, value)| IndexOp::SetRank(raw, value)),
                any::<u64>().prop_map(IndexOp::RemoveRank),
                any::<u64>().prop_map(IndexOp::Tombstone),
                any::<u64>().prop_map(IndexOp::TombstoneRelation),
                any::<u64>().prop_map(IndexOp::TombstoneIncidence),
            ],
            0..24,
        )
    ) {
        // Build a random store on top of `small_base` and freeze its merged state
        // into fresh base bytes (so the persisted index sections reflect it).
        let seed = small_base();
        let seed_records = BaseRecords::from_view(seed.get()).expect("seed records");
        let catalog = seed.get().catalog();
        let robot = catalog.label_id("Robot").expect("robot label");
        let person = catalog.label_id("Person").expect("person label");
        let calls = catalog.relation_type_id("calls").expect("calls type");
        let name = catalog.property_key_id("name").expect("name key");
        let rank = catalog.property_key_id("rank").expect("rank key");
        let weight = catalog.property_key_id("weight").expect("weight key");
        let slot = catalog.property_key_id("slot").expect("slot key");

        let mut write = WriteOverlay::new(base_next_ids(&seed), catalog.clone());
        for op in &ops {
            match op {
                IndexOp::LabelElement(raw) => {
                    write.add_element_label(&seed_records, ElementId::new(1 + (raw % 4)), robot);
                }
                IndexOp::TypeRelation(raw) => {
                    write.set_relation_type(&seed_records, RelationId::new(1 + (raw % 2)), calls);
                }
                IndexOp::SetRank(raw, value) => {
                    write.set_property(
                        &seed_records,
                        PropertySubject::Element(ElementId::new(1 + (raw % 3))),
                        rank,
                        PropertyValue::Integer(*value),
                    );
                }
                IndexOp::RemoveRank(raw) => {
                    write.remove_property(
                        &seed_records,
                        PropertySubject::Element(ElementId::new(1 + (raw % 3))),
                        rank,
                    );
                }
                IndexOp::Tombstone(raw) => {
                    write.tombstone_element(&seed_records, ElementId::new(1 + (raw % 4)));
                }
                IndexOp::TombstoneRelation(raw) => {
                    write.tombstone_relation(&seed_records, RelationId::new(1 + (raw % 2)));
                }
                IndexOp::TombstoneIncidence(raw) => {
                    write.tombstone_incidence(&seed_records, IncidenceId::new(1 + (raw % 2)));
                }
            }
        }
        let folded = write.freeze();
        let bytes = freeze_view(
            &MergedState::new(&seed_records, &folded),
            FreezeStamps { commit_seq: 1, transaction_id: 1, generation: 1 },
        )
        .expect("freeze folded state");

        // Open the SAME bytes twice: borrowed (persisted index) and owned (rebuilt
        // via from_records). `open` already debug-asserts they agree as a whole;
        // here we assert every accessor per-key explicitly.
        let base = Arc::new(Base::open_owned_bytes(bytes).expect("open folded base"));
        let borrowed = BaseRecords::open(&base).expect("borrowed records");
        let owned = BaseRecords::from_view(base.get()).expect("owned records");

        let empty = Overlay::empty(base_next_ids(&base), base.get().catalog().clone());
        let borrowed_view = MergedState::new(&borrowed, &empty);
        let owned_view = MergedState::new(&owned, &empty);

        // Label + relation-type membership over present and absent keys.
        for label in [person, robot, LabelId::new(9999)] {
            prop_assert_eq!(
                borrowed_view.elements_with_label(label),
                owned_view.elements_with_label(label),
                "label {} membership borrowed != owned",
                label.get()
            );
        }
        for ty in [calls, RelationTypeId::new(9999)] {
            prop_assert_eq!(
                borrowed_view.relations_with_type(ty),
                owned_view.relations_with_type(ty),
                "relation-type {} membership borrowed != owned",
                ty.get()
            );
        }

        // Reverse adjacency over every base element and relation, plus absentees.
        for raw in 1u64..=5 {
            let element = ElementId::new(raw);
            let mut b = borrowed_view.element_incidences(element);
            let mut o = owned_view.element_incidences(element);
            b.sort_by_key(|record| record.id);
            o.sort_by_key(|record| record.id);
            prop_assert_eq!(b, o, "element {} adjacency borrowed != owned", raw);
        }
        for raw in 1u64..=3 {
            let relation = RelationId::new(raw);
            let mut b = borrowed_view.relation_incidences(relation);
            let mut o = owned_view.relation_incidences(relation);
            b.sort_by_key(|record| record.id);
            o.sort_by_key(|record| record.id);
            prop_assert_eq!(b, o, "relation {} adjacency borrowed != owned", raw);
        }

        // Equality over present and absent integer values and a text value, for
        // every property key family (element rank, relation weight, incidence slot,
        // element name).
        for key in [rank, weight, slot] {
            for value in -9i64..=9 {
                let probe = PropertyValue::Integer(value);
                prop_assert_eq!(
                    borrowed_view.property_equal(key, &probe),
                    owned_view.property_equal(key, &probe),
                    "equality borrowed != owned for key {} value {}",
                    key.get(),
                    value
                );
            }
        }
        for text in ["Alice", "Bob", "Carol", "Zzz-absent"] {
            let probe = PropertyValue::from(text);
            prop_assert_eq!(
                borrowed_view.property_equal(name, &probe),
                owned_view.property_equal(name, &probe),
                "text equality borrowed != owned for name {:?}",
                text
            );
        }

        // Range over full/partial/single/inverted windows.
        for (lo, hi) in [(-9i64, 9i64), (-3, 3), (0, 0), (2, 1), (5, 9)] {
            let min = PropertyValue::Integer(lo);
            let max = PropertyValue::Integer(hi);
            prop_assert_eq!(
                borrowed_view.property_range(rank, &min, &max),
                owned_view.property_range(rank, &min, &max),
                "range borrowed != owned for [{}, {}]",
                lo,
                hi
            );
        }

        // `property_key_subjects` over every present key and an absent one.
        for key in [rank, name, weight, slot, PropertyKeyId::new(9999)] {
            prop_assert_eq!(
                borrowed_view.property_key_subjects(key),
                owned_view.property_key_subjects(key),
                "property_key_subjects borrowed != owned for key {}",
                key.get()
            );
        }

        // The whole-index snapshots must be byte-identical too.
        prop_assert!(
            crate::index::indexes_agree(borrowed.index(), owned.index()),
            "borrowed and owned index snapshots disagree"
        );
    }
}
