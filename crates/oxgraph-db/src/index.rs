//! Real (index-backed) membership and property lookups for the merged state.
//!
//! The [`crate::overlay::StateView`] membership and property lookups are
//! index-backed, running in `O(log n + matches)`, not `O(n)`.
//!
//! Two pieces compose, mirroring the base/overlay split of the state model:
//!
//! * [`BaseIndex`] — derived postings built ONCE in RAM from the immutable
//!   [`crate::overlay::BaseRecords`] of one generation: a per-`(key, value)` equality map (which
//!   drives equality + range via the ordered [`crate::PropertyValue`] key, and composite via
//!   per-key posting intersection), plus label and relation-type membership maps. It is
//!   `Arc`-shared by every reader of a generation, so `begin_read` stays `O(1)`.
//! * [`OverlayIndex`] — incremental deltas the overlay maintains as the writer mutates:
//!   `added`/`removed` postings per posting key. A lookup then computes `(base ∪ added) \ removed`
//!   over only the affected postings, so a base-only id costs `O(log n + matches)` and an
//!   overlay-touched id costs `O(log overlay change)`.
//!
//! # Correctness
//!
//! The index-backed result MUST equal the merge-scan result for every lookup.
//! The merge-scan oracle survives under `#[cfg(test)]` in [`super::overlay`] and
//! is differential-tested against the index path (see the overlay tests).
//!
//! # Performance
//!
//! `perf: unspecified`; this module defines the index primitives. Each item
//! below carries its own contract: [`BaseIndex::from_records`] is `O(base
//! records + labels + properties)` (one build per generation); every overlay
//! delta mutator is `O(log overlay change)`; every merged lookup is `O(log n +
//! matches + overlay change)`.

use std::collections::{BTreeMap, BTreeSet, btree_map};

use crate::{
    ElementId, LabelId, PropertyKeyId, PropertySubject, RelationId, RelationTypeId,
    state::{ElementRecord, RelationRecord},
    value::PropertyValue,
};

/// Derived equality posting key: a `(property key, value)` pair whose posting is
/// the set of subjects carrying exactly that value under that key.
///
/// The ordered [`PropertyValue`] in the second position makes the equality map a
/// single ordered index that also answers RANGE lookups: a `[min, max]` range
/// over `key` is the contiguous map slice from `(key, min)` to `(key, max)`.
///
/// # Performance
///
/// Cloning is `O(value length)` for text and `O(1)` otherwise; comparing is the
/// same.
type EqualityKey = (PropertyKeyId, PropertyValue);

/// Derived postings built once from one immutable base generation.
///
/// Every map here is rebuilt in RAM at [`Self::from_records`] from the owned
/// [`crate::overlay::BaseRecords`]; nothing is persisted into the base file (the
/// base format carries reserved `SECTION_INDEX_*` sections that stay unused —
/// the index is rebuilt in RAM). One build per generation, `Arc`-shared
/// thereafter.
///
/// # Performance
///
/// [`Self::from_records`] is `O(base records + labels + properties)`; every
/// lookup accessor is `O(log n + matches)`.
#[derive(Clone, Debug)]
pub(crate) struct BaseIndex {
    /// Elements carrying each label, ascending by element id within each posting.
    label_members: BTreeMap<LabelId, BTreeSet<ElementId>>,
    /// Relations of each relation type, ascending by relation id.
    relation_type_members: BTreeMap<RelationTypeId, BTreeSet<RelationId>>,
    /// Subjects carrying each `(key, value)`, ascending by subject. Ordered by
    /// `(key, value)` so a range over one key is a contiguous map slice.
    property_equality: BTreeMap<EqualityKey, BTreeSet<PropertySubject>>,
}

impl BaseIndex {
    /// Builds the empty base index (the gen-0 base has no records).
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn empty() -> Self {
        Self {
            label_members: BTreeMap::new(),
            relation_type_members: BTreeMap::new(),
            property_equality: BTreeMap::new(),
        }
    }

    /// Builds the base index from one generation's owned records, indexing every
    /// element label, every relation type, and every property value.
    ///
    /// # Performance
    ///
    /// This function is `O(base records + labels + properties)`.
    pub(crate) fn from_records(
        elements: &BTreeMap<ElementId, ElementRecord>,
        relations: &BTreeMap<RelationId, RelationRecord>,
        properties: &BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>>,
    ) -> Self {
        let mut label_members: BTreeMap<LabelId, BTreeSet<ElementId>> = BTreeMap::new();
        for record in elements.values() {
            for label in &record.labels {
                label_members.entry(*label).or_default().insert(record.id);
            }
        }
        let mut relation_type_members: BTreeMap<RelationTypeId, BTreeSet<RelationId>> =
            BTreeMap::new();
        for record in relations.values() {
            if let Some(relation_type) = record.relation_type {
                relation_type_members
                    .entry(relation_type)
                    .or_default()
                    .insert(record.id);
            }
        }
        let mut property_equality: BTreeMap<EqualityKey, BTreeSet<PropertySubject>> =
            BTreeMap::new();
        for (subject, keys) in properties {
            for (key, value) in keys {
                property_equality
                    .entry((*key, value.clone()))
                    .or_default()
                    .insert(*subject);
            }
        }
        Self {
            label_members,
            relation_type_members,
            property_equality,
        }
    }

    /// Returns the base posting of elements carrying `label`, or an empty set.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn label_posting(&self, label: LabelId) -> Option<&BTreeSet<ElementId>> {
        self.label_members.get(&label)
    }

    /// Returns the base posting of relations of `relation_type`, or an empty set.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn relation_type_posting(
        &self,
        relation_type: RelationTypeId,
    ) -> Option<&BTreeSet<RelationId>> {
        self.relation_type_members.get(&relation_type)
    }

    /// Returns the base posting of subjects carrying `(key, value)`, or an empty
    /// set.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + value length)`.
    fn equality_posting(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Option<&BTreeSet<PropertySubject>> {
        // The map is keyed by the owned `(PropertyKeyId, PropertyValue)` pair, so
        // probing it needs one owned key (the value is cloned once).
        self.property_equality.get(&(key, value.clone()))
    }

    /// Iterates the base postings whose key is `key` and whose value falls in
    /// the inclusive range `[min, max]`, ascending by value, as `(pair,
    /// subjects)` so the range merge can mask each posting by overlay removals.
    ///
    /// The equality map is ordered by `(key, value)`, so the range is one
    /// contiguous map slice — no full scan.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + matching postings)`.
    fn range_postings(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> impl Iterator<Item = (&EqualityKey, &BTreeSet<PropertySubject>)> {
        self.property_equality
            .range((key, min.clone())..=(key, max.clone()))
    }
}

/// Incremental index deltas an overlay maintains alongside its record/property
/// deltas, expressed as `added`/`removed` postings against the base index.
///
/// The merge of base + overlay for any posting is `(base ∪ added) \ removed`.
/// Each mutator records the membership transition the corresponding record /
/// property mutation causes:
///
/// * adding a posting (e.g. the subject newly carries `(key, value)`) inserts into `added` and
///   removes from `removed`;
/// * removing a posting (override to a different value, remove, or tombstone) inserts into
///   `removed` and removes from `added`.
///
/// Because a subject can be in at most one state per posting key, `added` and
/// `removed` for the same posting key are disjoint after every mutator.
///
/// # Performance
///
/// Construction is `O(1)`; each mutator is `O(log overlay change + value
/// length)`.
#[derive(Clone, Debug, Default)]
pub(crate) struct OverlayIndex {
    /// Label postings the overlay adds, by label.
    label_added: BTreeMap<LabelId, BTreeSet<ElementId>>,
    /// Label postings the overlay removes (tombstoned element), by label.
    label_removed: BTreeMap<LabelId, BTreeSet<ElementId>>,
    /// Relation-type postings the overlay adds, by relation type.
    relation_type_added: BTreeMap<RelationTypeId, BTreeSet<RelationId>>,
    /// Relation-type postings the overlay removes, by relation type.
    relation_type_removed: BTreeMap<RelationTypeId, BTreeSet<RelationId>>,
    /// Equality postings the overlay adds, by `(key, value)`.
    equality_added: BTreeMap<EqualityKey, BTreeSet<PropertySubject>>,
    /// Equality postings the overlay removes, by `(key, value)`.
    equality_removed: BTreeMap<EqualityKey, BTreeSet<PropertySubject>>,
}

impl OverlayIndex {
    /// Builds an empty overlay index (no deltas over the base).
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn new() -> Self {
        Self {
            label_added: BTreeMap::new(),
            label_removed: BTreeMap::new(),
            relation_type_added: BTreeMap::new(),
            relation_type_removed: BTreeMap::new(),
            equality_added: BTreeMap::new(),
            equality_removed: BTreeMap::new(),
        }
    }

    /// Records that `subject` now carries `(key, value)`: adds the posting and
    /// drops any prior removal of the same posting.
    ///
    /// # Performance
    ///
    /// This method is `O(log overlay change + value length)`.
    fn add_equality(&mut self, subject: PropertySubject, key: PropertyKeyId, value: PropertyValue) {
        remove_posting(&mut self.equality_removed, (key, value.clone()), &subject);
        self.equality_added
            .entry((key, value))
            .or_default()
            .insert(subject);
    }

    /// Records that `subject` no longer carries `(key, value)`: removes the
    /// posting and drops any prior addition of the same posting.
    ///
    /// # Performance
    ///
    /// This method is `O(log overlay change + value length)`.
    fn remove_equality(
        &mut self,
        subject: PropertySubject,
        key: PropertyKeyId,
        value: PropertyValue,
    ) {
        remove_posting(&mut self.equality_added, (key, value.clone()), &subject);
        self.equality_removed
            .entry((key, value))
            .or_default()
            .insert(subject);
    }

    /// Records the membership transition of one property set: the subject leaves
    /// its previous value's posting (when it had one) and joins the new value's.
    ///
    /// `previous` is the value visible BEFORE this set (the merged base+overlay
    /// value), so the index stays consistent whether the set overrides a base
    /// value, an earlier overlay value, or adds a fresh one.
    ///
    /// # Performance
    ///
    /// This method is `O(log overlay change + value length)`.
    pub(crate) fn on_set_property(
        &mut self,
        subject: PropertySubject,
        key: PropertyKeyId,
        previous: Option<&PropertyValue>,
        new_value: &PropertyValue,
    ) {
        if let Some(previous) = previous
            && previous != new_value
        {
            self.remove_equality(subject, key, previous.clone());
        }
        self.add_equality(subject, key, new_value.clone());
    }

    /// Records the membership transition of one property removal: the subject
    /// leaves its previous value's posting (when it had a visible value).
    ///
    /// # Performance
    ///
    /// This method is `O(log overlay change + value length)`.
    pub(crate) fn on_remove_property(
        &mut self,
        subject: PropertySubject,
        key: PropertyKeyId,
        previous: Option<&PropertyValue>,
    ) {
        if let Some(previous) = previous {
            self.remove_equality(subject, key, previous.clone());
        }
    }

    /// Records that `element` now carries `label`.
    ///
    /// # Performance
    ///
    /// This method is `O(log overlay change)`.
    pub(crate) fn on_add_element_label(&mut self, element: ElementId, label: LabelId) {
        remove_posting(&mut self.label_removed, label, &element);
        self.label_added.entry(label).or_default().insert(element);
    }

    /// Records that `relation`'s type changed from `previous` to `new_type`:
    /// it leaves the previous type's posting (when it had one) and joins the new.
    ///
    /// # Performance
    ///
    /// This method is `O(log overlay change)`.
    pub(crate) fn on_set_relation_type(
        &mut self,
        relation: RelationId,
        previous: Option<RelationTypeId>,
        new_type: RelationTypeId,
    ) {
        if let Some(previous) = previous
            && previous != new_type
        {
            self.remove_relation_type(relation, previous);
        }
        remove_posting(&mut self.relation_type_removed, new_type, &relation);
        self.relation_type_added
            .entry(new_type)
            .or_default()
            .insert(relation);
    }

    /// Records that `relation` left `relation_type`'s posting.
    ///
    /// # Performance
    ///
    /// This method is `O(log overlay change)`.
    fn remove_relation_type(&mut self, relation: RelationId, relation_type: RelationTypeId) {
        remove_posting(&mut self.relation_type_added, relation_type, &relation);
        self.relation_type_removed
            .entry(relation_type)
            .or_default()
            .insert(relation);
    }

    /// Records the full membership withdrawal of a tombstoned element: it leaves
    /// every label posting it carried (`labels`) and every equality posting of
    /// every property it had (`properties`).
    ///
    /// # Performance
    ///
    /// This method is `O((labels + properties) × log overlay change)`.
    pub(crate) fn on_tombstone_element(
        &mut self,
        element: ElementId,
        labels: &BTreeSet<LabelId>,
        properties: &BTreeMap<PropertyKeyId, PropertyValue>,
    ) {
        for label in labels {
            remove_posting(&mut self.label_added, *label, &element);
            self.label_removed
                .entry(*label)
                .or_default()
                .insert(element);
        }
        self.withdraw_subject_properties(PropertySubject::Element(element), properties);
    }

    /// Records the full membership withdrawal of a tombstoned relation: it leaves
    /// its relation-type posting (when typed) and every equality posting of every
    /// property it had.
    ///
    /// # Performance
    ///
    /// This method is `O((typed + properties) × log overlay change)`.
    pub(crate) fn on_tombstone_relation(
        &mut self,
        relation: RelationId,
        relation_type: Option<RelationTypeId>,
        properties: &BTreeMap<PropertyKeyId, PropertyValue>,
    ) {
        if let Some(relation_type) = relation_type {
            self.remove_relation_type(relation, relation_type);
        }
        self.withdraw_subject_properties(PropertySubject::Relation(relation), properties);
    }

    /// Records the full membership withdrawal of a tombstoned incidence: it
    /// leaves every equality posting of every property it had.
    ///
    /// # Performance
    ///
    /// This method is `O(properties × log overlay change)`.
    pub(crate) fn on_tombstone_incidence(
        &mut self,
        subject: PropertySubject,
        properties: &BTreeMap<PropertyKeyId, PropertyValue>,
    ) {
        self.withdraw_subject_properties(subject, properties);
    }

    /// Removes every `(key, value)` posting `subject` carried, given its visible
    /// property map.
    ///
    /// # Performance
    ///
    /// This method is `O(properties × log overlay change)`.
    fn withdraw_subject_properties(
        &mut self,
        subject: PropertySubject,
        properties: &BTreeMap<PropertyKeyId, PropertyValue>,
    ) {
        for (key, value) in properties {
            self.remove_equality(subject, *key, value.clone());
        }
    }

    /// Merges the base label posting for `label` with this overlay's deltas into
    /// the visible, ascending element-id list.
    ///
    /// # Performance
    ///
    /// This method is `O(matches + overlay change)`.
    pub(crate) fn elements_with_label(&self, base: &BaseIndex, label: LabelId) -> Vec<ElementId> {
        merge_posting(
            base.label_posting(label),
            self.label_added.get(&label),
            self.label_removed.get(&label),
        )
    }

    /// Merges the base relation-type posting for `relation_type` with this
    /// overlay's deltas into the visible, ascending relation-id list.
    ///
    /// # Performance
    ///
    /// This method is `O(matches + overlay change)`.
    pub(crate) fn relations_with_type(
        &self,
        base: &BaseIndex,
        relation_type: RelationTypeId,
    ) -> Vec<RelationId> {
        merge_posting(
            base.relation_type_posting(relation_type),
            self.relation_type_added.get(&relation_type),
            self.relation_type_removed.get(&relation_type),
        )
    }

    /// Merges the base equality posting for `(key, value)` with this overlay's
    /// deltas into the visible, ascending subject list.
    ///
    /// # Performance
    ///
    /// This method is `O(matches + overlay change + value length)`.
    pub(crate) fn property_equal(
        &self,
        base: &BaseIndex,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Vec<PropertySubject> {
        let pair = (key, value.clone());
        merge_posting(
            base.equality_posting(key, value),
            self.equality_added.get(&pair),
            self.equality_removed.get(&pair),
        )
    }

    /// Merges the base + overlay equality postings whose value falls in the
    /// inclusive range `[min, max]` under `key`, into the visible, ascending
    /// subject list (each subject once).
    ///
    /// The base side is the contiguous ordered slice of the equality map; the
    /// overlay side is the matching contiguous slice of `equality_added`/
    /// `equality_removed`. A subject visible under any value in range appears
    /// once.
    ///
    /// # Performance
    ///
    /// This method is `O(matching postings + matches + overlay change)`.
    pub(crate) fn property_range(
        &self,
        base: &BaseIndex,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Vec<PropertySubject> {
        // An inverted range matches nothing; return early so the ordered-map
        // `range` calls below never see `start > end` (which would panic).
        if min > max {
            return Vec::new();
        }
        let mut visible: BTreeSet<PropertySubject> = BTreeSet::new();
        // Base postings in range, masked per posting by this overlay's removals.
        for (pair, subjects) in base.range_postings(key, min, max) {
            let removed = self.equality_removed.get(pair);
            let kept = subjects
                .iter()
                .filter(|subject| removed.is_none_or(|set| !set.contains(subject)))
                .copied();
            visible.extend(kept);
        }
        // Overlay additions in range (a value the base never carried, or a fresh
        // subject) join the visible set directly.
        for (_pair, subjects) in self
            .equality_added
            .range((key, min.clone())..=(key, max.clone()))
        {
            visible.extend(subjects.iter().copied());
        }
        visible.into_iter().collect()
    }

    /// Intersects the per-key equality postings of an ordered `(key, value)`
    /// tuple into the subjects carrying EVERY pair, ascending. This is the
    /// composite-equality index path: probe each key's merged posting and
    /// intersect, starting from the smallest posting.
    ///
    /// `pairs` must be non-empty (the caller validates arity and key existence);
    /// an empty `pairs` yields an empty result.
    ///
    /// # Performance
    ///
    /// This method is `O(tuple arity × (matches + overlay change))`: each per-key
    /// posting is built index-backed, then intersected.
    pub(crate) fn property_composite_equal(
        &self,
        base: &BaseIndex,
        pairs: &[(PropertyKeyId, PropertyValue)],
    ) -> Vec<PropertySubject> {
        let mut per_key: Vec<BTreeSet<PropertySubject>> = pairs
            .iter()
            .map(|(key, value)| {
                self.property_equal(base, *key, value)
                    .into_iter()
                    .collect::<BTreeSet<_>>()
            })
            .collect();
        if per_key.is_empty() {
            return Vec::new();
        }
        // Intersect starting from the smallest posting to bound the work.
        per_key.sort_by_key(BTreeSet::len);
        let mut iter = per_key.into_iter();
        let Some(mut accumulator) = iter.next() else {
            return Vec::new();
        };
        for posting in iter {
            if accumulator.is_empty() {
                break;
            }
            accumulator.retain(|subject| posting.contains(subject));
        }
        accumulator.into_iter().collect()
    }
}

/// Removes `member` from the posting at `key` in `map`, dropping the posting
/// entirely when it becomes empty so the maps never grow unbounded with empty
/// sets.
///
/// # Performance
///
/// This function is `O(log overlay change + key compare)`.
fn remove_posting<K: Ord, M: Ord>(map: &mut BTreeMap<K, BTreeSet<M>>, key: K, member: &M) {
    if let btree_map::Entry::Occupied(mut entry) = map.entry(key) {
        entry.get_mut().remove(member);
        if entry.get().is_empty() {
            entry.remove();
        }
    }
}

/// Merges one posting key's base set with the overlay `added`/`removed` deltas
/// into the visible, ascending, deduplicated member list:
/// `(base ∪ added) \ removed`.
///
/// # Performance
///
/// This function is `O(base posting + added + removed)`.
fn merge_posting<M: Copy + Ord>(
    base: Option<&BTreeSet<M>>,
    added: Option<&BTreeSet<M>>,
    removed: Option<&BTreeSet<M>>,
) -> Vec<M> {
    let mut visible: BTreeSet<M> = BTreeSet::new();
    if let Some(base) = base {
        visible.extend(base.iter().copied());
    }
    if let Some(added) = added {
        visible.extend(added.iter().copied());
    }
    if let Some(removed) = removed {
        for member in removed {
            visible.remove(member);
        }
    }
    visible.into_iter().collect()
}
