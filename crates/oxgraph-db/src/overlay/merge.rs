//! The merged read surface: [`StateView`], the overlay-over-base k-way merge.
//!
//! [`LayeredState`] layers an [`OverlayLayer`] (the frozen [`Overlay`] or a
//! writer's in-flight [`WriteOverlay`]) over [`BaseRecords`]; point reads
//! return [`Cow`] (base-only ids borrow, overlay ids are owned, tombstones
//! are absent) and iterators k-way merge the two ascending streams.

use std::{
    borrow::Cow,
    collections::{BTreeMap, btree_map},
};

use super::{
    Delta, Keyed,
    frozen::{BaseRecords, Overlay},
    write::WriteOverlay,
};
use crate::{
    Catalog, DbError, ElementId, IncidenceId, LabelId, PropertyKeyId, RelationId, RelationTypeId,
    catalog::PropertyFamily,
    index::OverlayIndex,
    state::{ElementRecord, IncidenceRecord, NextIds, PropertySubject, RelationRecord},
    value::PropertyValue,
};

/// The read surface a merged state exposes. Two tiers compose:
///
/// * the POINT/ITER/CATALOG/WATERMARK base surface — point reads, full iterators, counts, the
///   merged catalog, and the watermark; point reads return [`Cow`] so a base-only id borrows (zero
///   clone) while an overlay-supplied id is owned (these are the required methods below);
/// * the adjacency / membership / typed-lookup surface the live consumers call, provided as default
///   methods over the base surface (the block after [`Self::next_ids`]).
///
/// # Surface
///
/// The trait covers the full surface the live consumers
/// (`query.rs`/`projection.rs`/`database.rs`) call:
///
/// * adjacency accessors — [`Self::relation_incidences`] / [`Self::element_incidences`] (incidence
///   adjacency; both projection builders walk them, e.g. `projection.rs` `from_state` via
///   `relation_incidences`);
/// * membership lookups — [`Self::elements_with_label`] / [`Self::relations_with_type`] (`query.rs`
///   `scan_elements_with_label` / `scan_relations_with_type`);
/// * the typed lookup family — [`Self::typed_property_equal`],
///   [`Self::typed_property_equal_for_family`], [`Self::validate_lookup_value_for_family`],
///   [`Self::property_equal`], [`Self::property_range`], [`Self::typed_property_range`],
///   [`Self::typed_property_composite_equal`] (`query.rs` property scans, `database.rs` index
///   lookups).
///
/// # Index-backed lookups
///
/// The default methods below are merge-aware SCANS (overlay-over-base,
/// tombstones masked) — correct, and the differential ORACLE the index path is
/// tested against. The live [`LayeredState`] impl OVERRIDES the membership and
/// property lookups ([`Self::elements_with_label`],
/// [`Self::relations_with_type`], [`Self::property_equal`],
/// [`Self::property_range`], [`Self::typed_property_composite_equal`]) with
/// index-backed implementations that run in `O(log n + matches)` instead of
/// `O(n)`: they probe the per-generation [`crate::index::BaseIndex`] postings
/// (`Arc`-shared, built once per [`Snapshot`]) merged with the overlay's
/// incremental [`crate::index::OverlayIndex`] deltas. The typed wrappers
/// ([`Self::typed_property_equal`], [`Self::typed_property_range`], …) validate
/// against the catalog then delegate to those overridden lookups, so they are
/// index-backed too. The scan oracles survive under `#[cfg(test)]` on
/// [`LayeredState`] (the `*_scan` methods) for the differential test.
///
/// This trait is the live read surface: `query.rs`, `projection.rs`, and
/// `database.rs` all reach state through it.
///
/// # Performance
///
/// `perf: unspecified`; each method carries its own contract in its impl.
pub(crate) trait StateView {
    /// Returns the visible element for `id`, or `None` when absent or
    /// tombstoned.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn element(&self, id: ElementId) -> Option<Cow<'_, ElementRecord>>;

    /// Returns the visible relation for `id`, or `None` when absent/tombstoned.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn relation(&self, id: RelationId) -> Option<Cow<'_, RelationRecord>>;

    /// Returns the visible incidence for `id`, or `None` when absent/tombstoned.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn incidence(&self, id: IncidenceId) -> Option<Cow<'_, IncidenceRecord>>;

    /// Returns the visible property value for `(subject, key)`, or `None` when
    /// absent or removed.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn property(
        &self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<Cow<'_, PropertyValue>>;

    /// Returns whether an element is visible.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn contains_element(&self, id: ElementId) -> bool {
        self.element(id).is_some()
    }

    /// Returns whether a relation is visible.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn contains_relation(&self, id: RelationId) -> bool {
        self.relation(id).is_some()
    }

    /// Returns whether an incidence is visible.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    fn contains_incidence(&self, id: IncidenceId) -> bool {
        self.incidence(id).is_some()
    }

    /// Iterates every visible element in ascending id order (overlay wins,
    /// tombstones masked).
    ///
    /// # Performance
    ///
    /// A full walk is `O(base + overlay change)`.
    fn elements(&self) -> impl Iterator<Item = Cow<'_, ElementRecord>>;

    /// Iterates every visible relation in ascending id order.
    ///
    /// # Performance
    ///
    /// A full walk is `O(base + overlay change)`.
    fn relations(&self) -> impl Iterator<Item = Cow<'_, RelationRecord>>;

    /// Iterates every visible incidence in ascending id order.
    ///
    /// # Performance
    ///
    /// A full walk is `O(base + overlay change)`.
    fn incidences(&self) -> impl Iterator<Item = Cow<'_, IncidenceRecord>>;

    /// Iterates every visible `(subject, key, value)` property triple in
    /// ascending `(subject, key)` order.
    ///
    /// # Performance
    ///
    /// A full walk is `O(base properties + overlay property change)`.
    fn properties(
        &self,
    ) -> impl Iterator<Item = (PropertySubject, PropertyKeyId, Cow<'_, PropertyValue>)>;

    /// Returns the number of visible elements.
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay change)`.
    fn element_count(&self) -> usize {
        self.elements().count()
    }

    /// Returns the number of visible relations.
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay change)`.
    fn relation_count(&self) -> usize {
        self.relations().count()
    }

    /// Returns the number of visible incidences.
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay change)`.
    fn incidence_count(&self) -> usize {
        self.incidences().count()
    }

    /// Returns the merged catalog (overlay registrations folded over base).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn catalog(&self) -> &Catalog;

    /// Returns the nine monotonic id allocators (the watermark).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn next_ids(&self) -> NextIds;

    /// Returns every visible incidence attached to `element`, in ascending
    /// incidence-id order.
    ///
    /// Returns an owned [`Vec`] (not a borrowed iterator) because the merged set
    /// mixes records owned by the overlay with records borrowed from the base, so
    /// no single borrow can span both. [`IncidenceRecord`] is [`Copy`], so the
    /// owned vector is cheap.
    ///
    /// # Performance
    ///
    /// This method is `O(base incidences + overlay incidence change)`: one merge
    /// scan filtered by element.
    fn element_incidences(&self, element: ElementId) -> Vec<IncidenceRecord> {
        self.incidences()
            .filter(|record| record.element == element)
            .map(Cow::into_owned)
            .collect()
    }

    /// Returns every visible incidence belonging to `relation`, in ascending
    /// incidence-id order.
    ///
    /// See [`Self::element_incidences`] for why the result is an owned [`Vec`].
    ///
    /// # Performance
    ///
    /// This method is `O(base incidences + overlay incidence change)`: one merge
    /// scan filtered by relation.
    fn relation_incidences(&self, relation: RelationId) -> Vec<IncidenceRecord> {
        self.incidences()
            .filter(|record| record.relation == relation)
            .map(Cow::into_owned)
            .collect()
    }

    /// Returns the ids of every visible element carrying `label`, in ascending
    /// element-id order.
    ///
    /// This default is the merge-scan oracle; the live [`LayeredState`] impl
    /// overrides it with an index-backed lookup.
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay element change)`: one merge scan testing
    /// each visible element's label set.
    fn elements_with_label(&self, label: LabelId) -> Vec<ElementId> {
        self.elements()
            .filter(|record| record.labels.contains(&label))
            .map(|record| record.id)
            .collect()
    }

    /// Returns the ids of every visible relation of `relation_type`, in ascending
    /// relation-id order.
    ///
    /// This default is the merge-scan oracle; the live [`LayeredState`] impl
    /// overrides it with an index-backed lookup.
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay relation change)`: one merge scan testing
    /// each visible relation's type.
    fn relations_with_type(&self, relation_type: RelationTypeId) -> Vec<RelationId> {
        self.relations()
            .filter(|record| record.relation_type == Some(relation_type))
            .map(|record| record.id)
            .collect()
    }

    /// Returns the subjects whose property under `key` equals `value`, in
    /// ascending subject order.
    ///
    /// An unvalidated scan over the merged visible properties. This default is
    /// the merge-scan oracle; the live [`LayeredState`] impl overrides it with an
    /// index-backed lookup.
    ///
    /// # Performance
    ///
    /// This method is `O(base properties + overlay property change)`: one merge
    /// scan filtered by `(key, value)`.
    fn property_equal(&self, key: PropertyKeyId, value: &PropertyValue) -> Vec<PropertySubject> {
        self.properties()
            .filter(|(_subject, candidate_key, candidate_value)| {
                *candidate_key == key && candidate_value.as_ref() == value
            })
            .map(|(subject, _key, _value)| subject)
            .collect()
    }

    /// Returns the subjects whose typed property under `key` equals `value`, after
    /// validating `value` against the key schema.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownPropertyKey`] when `key` is absent from the
    /// merged catalog, or [`DbError::PropertyTypeMismatch`] when `value`'s type
    /// does not match the key schema.
    ///
    /// # Performance
    ///
    /// `O(base properties + overlay property change)` for the default scan; this
    /// method delegates to [`Self::property_equal`], so when the impl overrides
    /// that with an index — as [`LayeredState`] does (the live read path) — the
    /// realized cost is `O(log n + matches + overlay change)`.
    fn typed_property_equal(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Result<Vec<PropertySubject>, DbError> {
        self.validate_lookup_value(key, value)?;
        Ok(self.property_equal(key, value))
    }

    /// Returns the subjects in `family` whose typed property under `key` equals
    /// `value`, after validating both the family and the value type.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownPropertyKey`], [`DbError::WrongPropertyFamily`],
    /// or [`DbError::PropertyTypeMismatch`] when the merged-catalog check fails.
    ///
    /// # Performance
    ///
    /// `O(base properties + overlay property change)` for the default scan; this
    /// method delegates to [`Self::property_equal`], so when the impl overrides
    /// that with an index — as [`LayeredState`] does (the live read path) — the
    /// realized cost is `O(log n + matches + overlay change)`.
    fn typed_property_equal_for_family(
        &self,
        key: PropertyKeyId,
        family: PropertyFamily,
        value: &PropertyValue,
    ) -> Result<Vec<PropertySubject>, DbError> {
        self.validate_lookup_value_for_family(key, family, value)?;
        Ok(self.property_equal(key, value))
    }

    /// Returns the subjects whose ordered property under `key` falls within the
    /// inclusive range `[min, max]`, in ascending subject order.
    ///
    /// An unvalidated scan over the merged visible properties.
    ///
    /// # Performance
    ///
    /// `O(base properties + overlay property change)` for this default scan;
    /// [`LayeredState`] overrides it with an index-backed walk that is
    /// `O(log n + matching postings + matches + overlay change)`.
    fn property_range(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Vec<PropertySubject> {
        self.properties()
            .filter(|(_subject, candidate_key, value)| {
                *candidate_key == key && value.as_ref() >= min && value.as_ref() <= max
            })
            .map(|(subject, _key, _value)| subject)
            .collect()
    }

    /// Returns the subjects whose typed property under `key` falls within the
    /// inclusive range `[min, max]`, after validating both bounds.
    ///
    /// An inverted range (`min > max`) yields an empty result without scanning.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownPropertyKey`] or [`DbError::PropertyTypeMismatch`]
    /// when either bound fails the merged-catalog check.
    ///
    /// # Performance
    ///
    /// `O(base properties + overlay property change)` for the default scan; this
    /// method delegates to [`Self::property_range`], so when the impl overrides
    /// that with an index — as [`LayeredState`] does (the live read path) — the
    /// realized cost is `O(log n + matching postings + matches + overlay change)`.
    fn typed_property_range(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Result<Vec<PropertySubject>, DbError> {
        self.validate_lookup_value(key, min)?;
        self.validate_lookup_value(key, max)?;
        if min > max {
            return Ok(Vec::new());
        }
        Ok(self.property_range(key, min, max))
    }

    /// Returns the subjects matching an ordered typed property tuple: a subject is
    /// returned only when it carries every `keys[i]` equal to `values[i]`, in
    /// ascending subject order.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::unsupported`] on a `keys`/`values` arity mismatch, and
    /// [`DbError::UnknownPropertyKey`] or [`DbError::PropertyTypeMismatch`] when a
    /// `(key, value)` pair fails the merged-catalog check.
    ///
    /// # Performance
    ///
    /// This method is `O((base subjects + overlay subject change) × tuple
    /// arity)`: one grouped merge scan per subject.
    fn typed_property_composite_equal(
        &self,
        keys: &[PropertyKeyId],
        values: &[PropertyValue],
    ) -> Result<Vec<PropertySubject>, DbError> {
        if keys.len() != values.len() {
            return Err(DbError::unsupported(
                "composite equality tuple arity mismatch",
            ));
        }
        for (key, value) in keys.iter().copied().zip(values) {
            self.validate_lookup_value(key, value)?;
        }
        Ok(self
            .properties_by_subject()
            .into_iter()
            .filter_map(|(subject, subject_values)| {
                keys.iter()
                    .copied()
                    .zip(values)
                    .all(|(key, value)| subject_values.get(&key) == Some(value))
                    .then_some(subject)
            })
            .collect())
    }

    /// Validates a lookup `value`'s type against the merged catalog's schema for
    /// `key`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownPropertyKey`] when `key` is absent from the
    /// merged catalog, or [`DbError::PropertyTypeMismatch`] when `value`'s type
    /// does not match the key schema.
    ///
    /// # Performance
    ///
    /// This method is `O(log catalog keys)`.
    fn validate_lookup_value(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Result<(), DbError> {
        let definition = self
            .catalog()
            .property_key(key)
            .ok_or(DbError::UnknownPropertyKey { id: key })?;
        let actual = value.value_type();
        if definition.value_type != actual {
            return Err(DbError::PropertyTypeMismatch {
                expected: definition.value_type,
                actual,
            });
        }
        Ok(())
    }

    /// Validates a lookup `value`'s type and subject `family` against the merged
    /// catalog's schema for `key`.
    ///
    /// Delegates entirely to the merged catalog returned by [`Self::catalog`].
    ///
    /// # Errors
    ///
    /// Returns [`DbError::UnknownPropertyKey`], [`DbError::WrongPropertyFamily`],
    /// or [`DbError::PropertyTypeMismatch`].
    ///
    /// # Performance
    ///
    /// This method is `O(log catalog keys)`.
    fn validate_lookup_value_for_family(
        &self,
        key: PropertyKeyId,
        family: PropertyFamily,
        value: &PropertyValue,
    ) -> Result<(), DbError> {
        let definition = self
            .catalog()
            .property_key(key)
            .ok_or(DbError::UnknownPropertyKey { id: key })?;
        if definition.family != family {
            return Err(DbError::WrongPropertyFamily {
                expected: definition.family,
                actual: family,
            });
        }
        if definition.value_type != value.value_type() {
            return Err(DbError::PropertyTypeMismatch {
                expected: definition.value_type,
                actual: value.value_type(),
            });
        }
        Ok(())
    }

    /// Returns the merged visible properties grouped by subject, each subject's
    /// keys mapped to their visible value, ascending by `(subject, key)`.
    ///
    /// This materializes one entry per visible `(subject, key)` pair so a
    /// per-subject predicate (e.g. composite equality) can test every key of a
    /// subject together.
    ///
    /// # Performance
    ///
    /// This method is `O(base properties + overlay property change)`.
    fn properties_by_subject(
        &self,
    ) -> BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>> {
        let mut grouped: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>> =
            BTreeMap::new();
        for (subject, key, value) in self.properties() {
            grouped
                .entry(subject)
                .or_default()
                .insert(key, value.into_owned());
        }
        grouped
    }
}

/// The delta-map read surface a layered overlay exposes to the merge. Both the
/// frozen [`Overlay`] (published over a snapshot) and the in-flight
/// [`WriteOverlay`] (a single writer's private delta) implement it, so the merge
/// iterators and point reads are written once and serve both.
///
/// # Performance
///
/// Every accessor is `O(1)`.
pub(crate) trait OverlayLayer {
    /// Returns the element delta map (id -> add/override, or tombstone).
    fn elements(&self) -> &Delta<ElementRecord>;
    /// Returns the relation delta map.
    fn relations(&self) -> &Delta<RelationRecord>;
    /// Returns the incidence delta map.
    fn incidences(&self) -> &Delta<IncidenceRecord>;
    /// Returns the property delta map.
    fn properties(
        &self,
    ) -> &BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, Option<PropertyValue>>>;
    /// Returns the merged catalog (parent registrations plus this layer's).
    fn catalog(&self) -> &Catalog;
    /// Returns the nine monotonic id allocators (the watermark).
    fn next_ids(&self) -> NextIds;
    /// Returns this layer's incremental index deltas, so a merged lookup can be
    /// index-backed (`(base index ∪ added) \ removed`) rather than a full scan.
    fn index(&self) -> &OverlayIndex;
}

impl OverlayLayer for Overlay {
    fn elements(&self) -> &Delta<ElementRecord> {
        &self.elements
    }

    fn relations(&self) -> &Delta<RelationRecord> {
        &self.relations
    }

    fn incidences(&self) -> &Delta<IncidenceRecord> {
        &self.incidences
    }

    fn properties(
        &self,
    ) -> &BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, Option<PropertyValue>>> {
        &self.properties
    }

    fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    fn next_ids(&self) -> NextIds {
        self.next
    }

    fn index(&self) -> &OverlayIndex {
        &self.index
    }
}

impl OverlayLayer for WriteOverlay {
    fn elements(&self) -> &Delta<ElementRecord> {
        &self.elements
    }

    fn relations(&self) -> &Delta<RelationRecord> {
        &self.relations
    }

    fn incidences(&self) -> &Delta<IncidenceRecord> {
        &self.incidences
    }

    fn properties(
        &self,
    ) -> &BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, Option<PropertyValue>>> {
        &self.properties
    }

    fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    fn next_ids(&self) -> NextIds {
        self.next
    }

    fn index(&self) -> &OverlayIndex {
        &self.index
    }
}

/// A borrowed merged read view: an [`OverlayLayer`] layered over [`BaseRecords`].
///
/// Reads resolve overlay-first: a point read returns the overlay's record when
/// the overlay has an entry (a set value is [`Cow::Owned`]; a tombstone is
/// `None`), and otherwise borrows the base record ([`Cow::Borrowed`], the
/// zero-clone fast path) — `None` when the base lacks it too. Iterators k-way
/// merge the two ascending streams, yielding each id once with the overlay
/// winning and tombstones masked.
///
/// # Performance
///
/// Construction is `O(1)`; reads carry the contracts on [`StateView`].
#[derive(Clone, Copy)]
pub(crate) struct LayeredState<'a, L: OverlayLayer> {
    /// Owned base records the merge borrows from for base-only ids.
    pub(super) base: &'a BaseRecords,
    /// The overlay layer (frozen or in-flight) layered over the base.
    pub(super) overlay: &'a L,
}

impl<'a, L: OverlayLayer> LayeredState<'a, L> {
    /// Builds a merged view over an explicit `base`/`overlay` pair.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn new(base: &'a BaseRecords, overlay: &'a L) -> Self {
        Self { base, overlay }
    }

    /// Returns the visible element for `id`, borrowed from the DATA (lifetime
    /// `'a`) rather than from `&self`, so the returned [`Cow`] outlives this
    /// temporary view. The [`StateView`] impl delegates here.
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    pub(crate) fn element_ref(&self, id: ElementId) -> Option<Cow<'a, ElementRecord>> {
        match self.overlay.elements().get(&id) {
            Some(Some(record)) => Some(Cow::Owned(record.clone())),
            Some(None) => None,
            None => self.base.element(id).map(Cow::Borrowed),
        }
    }

    /// Returns the visible relation for `id`, borrowed from the data (see
    /// [`Self::element_ref`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    pub(crate) fn relation_ref(&self, id: RelationId) -> Option<Cow<'a, RelationRecord>> {
        match self.overlay.relations().get(&id) {
            Some(Some(record)) => Some(Cow::Owned(record.clone())),
            Some(None) => None,
            None => self.base.relation(id).map(Cow::Borrowed),
        }
    }

    /// Returns the visible incidence for `id`, borrowed from the data (see
    /// [`Self::element_ref`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    pub(crate) fn incidence_ref(&self, id: IncidenceId) -> Option<Cow<'a, IncidenceRecord>> {
        match self.overlay.incidences().get(&id) {
            Some(Some(record)) => Some(Cow::Owned(*record)),
            Some(None) => None,
            None => self.base.incidence(id).map(Cow::Borrowed),
        }
    }

    /// Returns the visible property value for `(subject, key)`, borrowed from the
    /// data (see [`Self::element_ref`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log change + log n)`.
    pub(crate) fn property_ref(
        &self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<Cow<'a, PropertyValue>> {
        match self
            .overlay
            .properties()
            .get(&subject)
            .and_then(|keys| keys.get(&key))
        {
            Some(Some(value)) => Some(Cow::Owned(value.clone())),
            Some(None) => None,
            None => self.base.property(subject, key).map(Cow::Borrowed),
        }
    }

    /// Returns every visible property of `subject` as owned `(key, value)` pairs
    /// in ascending key order, overlay-over-base (overlay values win; overlay
    /// tombstones hide the base value).
    ///
    /// # Performance
    ///
    /// This method is `O(subject base keys + subject overlay change)`.
    pub(crate) fn subject_properties(
        &self,
        subject: PropertySubject,
    ) -> Vec<(PropertyKeyId, PropertyValue)> {
        let mut merged: BTreeMap<PropertyKeyId, PropertyValue> = self
            .base
            .properties
            .get(&subject)
            .cloned()
            .unwrap_or_default();
        let overlay_keys = self.overlay.properties().get(&subject);
        for (key, value) in overlay_keys.into_iter().flatten() {
            if let Some(value) = value {
                merged.insert(*key, value.clone());
            } else {
                merged.remove(key);
            }
        }
        merged.into_iter().collect()
    }

    /// Returns every subject carrying property `key` (under any value) with its
    /// visible value, index-backed (base posting merged with overlay deltas).
    ///
    /// # Performance
    ///
    /// This method is `O(postings for key + overlay change)`.
    pub(crate) fn property_key_subjects(
        &self,
        key: PropertyKeyId,
    ) -> Vec<(PropertySubject, PropertyValue)> {
        self.overlay
            .index()
            .property_key_subjects(self.base.index(), key)
    }

    /// Returns the merged catalog borrowed from the data (lifetime `'a`), so the
    /// reference outlives this temporary view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) fn catalog_ref(&self) -> &'a Catalog {
        self.overlay.catalog()
    }
}

/// The published merged read view: a frozen [`Overlay`] over [`BaseRecords`].
///
/// # Performance
///
/// `perf: unspecified`; an alias.
pub(crate) type MergedState<'a> = LayeredState<'a, Overlay>;

/// The in-flight merged read view: a single writer's [`WriteOverlay`] over
/// [`BaseRecords`], used by the write transaction to read its own pending state
/// for referential-integrity validation.
///
/// # Performance
///
/// `perf: unspecified`; an alias.
pub(crate) type WriteMergedState<'a> = LayeredState<'a, WriteOverlay>;

impl<L: OverlayLayer> StateView for LayeredState<'_, L> {
    fn element(&self, id: ElementId) -> Option<Cow<'_, ElementRecord>> {
        self.element_ref(id)
    }

    fn relation(&self, id: RelationId) -> Option<Cow<'_, RelationRecord>> {
        self.relation_ref(id)
    }

    fn incidence(&self, id: IncidenceId) -> Option<Cow<'_, IncidenceRecord>> {
        self.incidence_ref(id)
    }

    fn property(
        &self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<Cow<'_, PropertyValue>> {
        self.property_ref(subject, key)
    }

    fn elements(&self) -> impl Iterator<Item = Cow<'_, ElementRecord>> {
        MergeIter::new(self.base.elements.values(), self.overlay.elements().iter())
    }

    fn relations(&self) -> impl Iterator<Item = Cow<'_, RelationRecord>> {
        MergeIter::new(
            self.base.relations.values(),
            self.overlay.relations().iter(),
        )
    }

    fn incidences(&self) -> impl Iterator<Item = Cow<'_, IncidenceRecord>> {
        MergeIter::new(
            self.base.incidences.values(),
            self.overlay.incidences().iter(),
        )
    }

    fn properties(
        &self,
    ) -> impl Iterator<Item = (PropertySubject, PropertyKeyId, Cow<'_, PropertyValue>)> {
        PropertyMergeIter::new(
            base_property_triples(self.base),
            overlay_property_triples(self.overlay.properties()),
        )
    }

    fn catalog(&self) -> &Catalog {
        self.overlay.catalog()
    }

    fn next_ids(&self) -> NextIds {
        self.overlay.next_ids()
    }

    /// Index-backed override of the membership lookup: merges the base label
    /// posting with the overlay's label deltas, instead of scanning every
    /// element. Equal to the scan oracle ([`Self::elements_with_label_scan`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log n + matches + overlay change)`.
    fn elements_with_label(&self, label: LabelId) -> Vec<ElementId> {
        self.overlay
            .index()
            .elements_with_label(self.base.index(), label)
    }

    /// Index-backed override of the relation-type membership lookup: merges the
    /// base relation-type posting with the overlay's deltas. Equal to the scan
    /// oracle ([`Self::relations_with_type_scan`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log n + matches + overlay change)`.
    fn relations_with_type(&self, relation_type: RelationTypeId) -> Vec<RelationId> {
        self.overlay
            .index()
            .relations_with_type(self.base.index(), relation_type)
    }

    /// Index-backed override of the element reverse-adjacency lookup: resolves the
    /// merged element→incidence posting to records, instead of scanning every
    /// incidence. Equal to the default merge-scan oracle.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + degree)`.
    fn element_incidences(&self, element: ElementId) -> Vec<IncidenceRecord> {
        self.overlay
            .index()
            .element_incidences(self.base.index(), element)
            .into_iter()
            .filter_map(|id| self.incidence_ref(id).map(Cow::into_owned))
            .collect()
    }

    /// Index-backed override of the relation reverse-adjacency lookup. Equal to
    /// the default merge-scan oracle.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + degree)`.
    fn relation_incidences(&self, relation: RelationId) -> Vec<IncidenceRecord> {
        self.overlay
            .index()
            .relation_incidences(self.base.index(), relation)
            .into_iter()
            .filter_map(|id| self.incidence_ref(id).map(Cow::into_owned))
            .collect()
    }

    /// Index-backed override of the equality lookup: probes the base equality
    /// posting for `(key, value)` and merges the overlay's deltas, instead of
    /// scanning every property. Equal to the scan oracle
    /// ([`Self::property_equal_scan`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log n + matches + overlay change)`.
    fn property_equal(&self, key: PropertyKeyId, value: &PropertyValue) -> Vec<PropertySubject> {
        self.overlay
            .index()
            .property_equal(self.base.index(), key, value)
    }

    /// Index-backed override of the range lookup: walks the contiguous ordered
    /// slice of the base equality map in `[min, max]` and merges the overlay's
    /// deltas, instead of scanning every property. Equal to the scan oracle
    /// ([`Self::property_range_scan`]).
    ///
    /// # Performance
    ///
    /// This method is `O(log n + matching postings + matches + overlay change)`.
    fn property_range(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Vec<PropertySubject> {
        self.overlay
            .index()
            .property_range(self.base.index(), key, min, max)
    }

    /// Index-backed override of the composite-equality lookup: intersects the
    /// per-key merged equality postings of the validated `(key, value)` tuple,
    /// instead of grouping every subject's properties. Equal to the scan oracle
    /// ([`Self::typed_property_composite_equal_scan`]).
    ///
    /// # Errors
    ///
    /// Returns [`DbError::unsupported`] on a `keys`/`values` arity mismatch, and
    /// [`DbError::UnknownPropertyKey`] or [`DbError::PropertyTypeMismatch`] when a
    /// `(key, value)` pair fails the merged-catalog check.
    ///
    /// # Performance
    ///
    /// This method is `O(tuple arity × (matches + overlay change))`.
    fn typed_property_composite_equal(
        &self,
        keys: &[PropertyKeyId],
        values: &[PropertyValue],
    ) -> Result<Vec<PropertySubject>, DbError> {
        if keys.len() != values.len() {
            return Err(DbError::unsupported(
                "composite equality tuple arity mismatch",
            ));
        }
        for (key, value) in keys.iter().copied().zip(values) {
            self.validate_lookup_value(key, value)?;
        }
        let pairs: Vec<(PropertyKeyId, PropertyValue)> =
            keys.iter().copied().zip(values.iter().cloned()).collect();
        Ok(self
            .overlay
            .index()
            .property_composite_equal(self.base.index(), &pairs))
    }
}

/// Merge-SCAN oracles: the scan lookup implementations, kept under
/// `#[cfg(test)]` so the index-backed [`StateView`] overrides can be
/// differential-tested against them. Each scans the full merged visible set and
/// filters; the index-backed methods MUST return the same result for every
/// input.
///
/// # Performance
///
/// `perf: unspecified`; test-only scan oracles, each `O(base + overlay change)`.
#[cfg(test)]
impl<L: OverlayLayer> LayeredState<'_, L> {
    /// Scan oracle for [`StateView::elements_with_label`].
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay element change)`.
    pub(crate) fn elements_with_label_scan(&self, label: LabelId) -> Vec<ElementId> {
        self.elements()
            .filter(|record| record.labels.contains(&label))
            .map(|record| record.id)
            .collect()
    }

    /// Scan oracle for [`StateView::relations_with_type`].
    ///
    /// # Performance
    ///
    /// This method is `O(base + overlay relation change)`.
    pub(crate) fn relations_with_type_scan(
        &self,
        relation_type: RelationTypeId,
    ) -> Vec<RelationId> {
        self.relations()
            .filter(|record| record.relation_type == Some(relation_type))
            .map(|record| record.id)
            .collect()
    }

    /// Scan oracle for [`StateView::element_incidences`].
    ///
    /// # Performance
    ///
    /// This method is `O(base incidences + overlay incidence change)`.
    pub(crate) fn element_incidences_scan(&self, element: ElementId) -> Vec<IncidenceRecord> {
        self.incidences()
            .filter(|record| record.element == element)
            .map(Cow::into_owned)
            .collect()
    }

    /// Scan oracle for [`StateView::relation_incidences`].
    ///
    /// # Performance
    ///
    /// This method is `O(base incidences + overlay incidence change)`.
    pub(crate) fn relation_incidences_scan(&self, relation: RelationId) -> Vec<IncidenceRecord> {
        self.incidences()
            .filter(|record| record.relation == relation)
            .map(Cow::into_owned)
            .collect()
    }

    /// Scan oracle for [`StateView::property_equal`].
    ///
    /// # Performance
    ///
    /// This method is `O(base properties + overlay property change)`.
    pub(crate) fn property_equal_scan(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Vec<PropertySubject> {
        self.properties()
            .filter(|(_subject, candidate_key, candidate_value)| {
                *candidate_key == key && candidate_value.as_ref() == value
            })
            .map(|(subject, _key, _value)| subject)
            .collect()
    }

    /// Scan oracle for [`StateView::property_range`].
    ///
    /// # Performance
    ///
    /// This method is `O(base properties + overlay property change)`.
    pub(crate) fn property_range_scan(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Vec<PropertySubject> {
        self.properties()
            .filter(|(_subject, candidate_key, value)| {
                *candidate_key == key && value.as_ref() >= min && value.as_ref() <= max
            })
            .map(|(subject, _key, _value)| subject)
            .collect()
    }

    /// Scan oracle for [`StateView::typed_property_composite_equal`] (the inner
    /// grouped-scan path, without revalidating the catalog).
    ///
    /// # Performance
    ///
    /// This method is `O((base subjects + overlay subject change) × arity)`.
    pub(crate) fn property_composite_equal_scan(
        &self,
        keys: &[PropertyKeyId],
        values: &[PropertyValue],
    ) -> Vec<PropertySubject> {
        self.properties_by_subject()
            .into_iter()
            .filter_map(|(subject, subject_values)| {
                keys.iter()
                    .copied()
                    .zip(values)
                    .all(|(key, value)| subject_values.get(&key) == Some(value))
                    .then_some(subject)
            })
            .collect()
    }
}

/// K-way merge of an ascending base record stream and an ascending overlay delta
/// stream over the same id type, yielding each visible record once: the overlay
/// wins on a shared id, a `None` overlay entry (tombstone) masks the base, and
/// an overlay-only id appears as owned.
///
/// # Performance
///
/// Each `next` is `O(1)` amortized; a full walk is `O(base + overlay change)`.
struct MergeIter<'a, R: Keyed + Clone> {
    /// Peekable ascending base record iterator.
    base: std::iter::Peekable<btree_map::Values<'a, R::Id, R>>,
    /// Peekable ascending overlay delta iterator.
    overlay: std::iter::Peekable<btree_map::Iter<'a, R::Id, Option<R>>>,
}

impl<'a, R: Keyed + Clone> MergeIter<'a, R> {
    /// Builds a merge iterator from the two ascending sources.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn new(
        base: btree_map::Values<'a, R::Id, R>,
        overlay: btree_map::Iter<'a, R::Id, Option<R>>,
    ) -> Self {
        Self {
            base: base.peekable(),
            overlay: overlay.peekable(),
        }
    }

    /// Advances the base cursor and emits the consumed base record borrowed.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn take_base(&mut self) -> Option<Cow<'a, R>> {
        self.base.next().map(Cow::Borrowed)
    }

    /// Advances the overlay cursor: emits a set value owned, or `None` when the
    /// consumed entry is a tombstone (the caller loops to the next entry). When
    /// `mask_base` is set, the matching base record is consumed first so the
    /// overlay overrides it.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn take_overlay(&mut self, mask_base: bool) -> Step<Cow<'a, R>> {
        if mask_base {
            let _masked = self.base.next();
        }
        let Some((_id, entry)) = self.overlay.next() else {
            return Step::Done;
        };
        entry.as_ref().map_or(Step::Again, |record| {
            Step::Yield(Cow::Owned(record.clone()))
        })
    }

    /// Advances the merge by one decision over the two ascending peeks: a
    /// base-only or strictly-lower base id yields the borrowed base record; an
    /// overlay-lower or tied id consumes the overlay (masking the base on a tie),
    /// yielding the set value or looping past a tombstone.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn step(&mut self) -> Step<Cow<'a, R>> {
        let base_id = self.base.peek().map(|record| record.record_id());
        let overlay_id = self.overlay.peek().map(|(id, _entry)| **id);
        match (base_id, overlay_id) {
            (None, None) => Step::Done,
            (Some(_base), None) => self.take_base().map_or(Step::Done, Step::Yield),
            (Some(base), Some(overlay)) if base < overlay => {
                self.take_base().map_or(Step::Done, Step::Yield)
            }
            (_other, Some(overlay)) => self.take_overlay(base_id == Some(overlay)),
        }
    }
}

impl<'a, R: Keyed + Clone> Iterator for MergeIter<'a, R> {
    type Item = Cow<'a, R>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.step() {
                Step::Done => return None,
                Step::Yield(item) => return Some(item),
                Step::Again => {}
            }
        }
    }
}

/// One outcome of a single k-way merge step: terminate, yield an item, or loop
/// past a masked entry (a tombstone) to the next decision.
///
/// # Performance
///
/// `perf: unspecified`; this is a control-flow tag.
enum Step<T> {
    /// Both streams are exhausted; the merge is finished.
    Done,
    /// Emit this visible item.
    Yield(T),
    /// A tombstone was consumed; retry the next decision.
    Again,
}

/// K-way merge of the ascending base property triples and the ascending overlay
/// property delta, yielding each visible `(subject, key, value)` once in
/// ascending `(subject, key)` order: the overlay wins on a shared `(subject,
/// key)`, a `None` overlay entry (removal) masks the base, and an overlay-only
/// set value appears owned.
///
/// Both sides are pre-flattened into ascending vectors so the merge is a single
/// linear walk over two slices (the `BTreeMap`s are already sorted by `(subject,
/// key)`, so flattening preserves order). The cursors index into them.
///
/// # Performance
///
/// Each `next` is `O(1)`; a full walk is `O(base + overlay change)`.
struct PropertyMergeIter<'a> {
    /// Ascending base `(subject, key, &value)` triples.
    base: Vec<(PropertySubject, PropertyKeyId, &'a PropertyValue)>,
    /// Ascending overlay `(subject, key, set/remove)` triples.
    overlay: Vec<(PropertySubject, PropertyKeyId, &'a Option<PropertyValue>)>,
    /// Cursor into `base`.
    base_index: usize,
    /// Cursor into `overlay`.
    overlay_index: usize,
}

impl<'a> PropertyMergeIter<'a> {
    /// Builds a property merge iterator over two pre-flattened ascending sides.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn new(
        base: Vec<(PropertySubject, PropertyKeyId, &'a PropertyValue)>,
        overlay: Vec<(PropertySubject, PropertyKeyId, &'a Option<PropertyValue>)>,
    ) -> Self {
        Self {
            base,
            overlay,
            base_index: 0,
            overlay_index: 0,
        }
    }
}

impl<'a> PropertyMergeIter<'a> {
    /// Advances the base cursor and emits the consumed `(subject, key, value)`
    /// triple borrowed.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn take_base(&mut self) -> Option<(PropertySubject, PropertyKeyId, Cow<'a, PropertyValue>)> {
        let (subject, key, value) = *self.base.get(self.base_index)?;
        self.base_index += 1;
        Some((subject, key, Cow::Borrowed(value)))
    }

    /// Advances the overlay cursor: emits a set value owned, or loops past a
    /// removal. When `mask_base` is set, the matching base triple is consumed
    /// first so the overlay overrides it.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn take_overlay(
        &mut self,
        mask_base: bool,
    ) -> Step<(PropertySubject, PropertyKeyId, Cow<'a, PropertyValue>)> {
        if mask_base {
            self.base_index += 1;
        }
        let Some(&(subject, key, entry)) = self.overlay.get(self.overlay_index) else {
            return Step::Done;
        };
        self.overlay_index += 1;
        entry.as_ref().map_or(Step::Again, |value| {
            Step::Yield((subject, key, Cow::Owned(value.clone())))
        })
    }

    /// Advances the property merge by one decision over the two ascending peeks.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn step(&mut self) -> Step<(PropertySubject, PropertyKeyId, Cow<'a, PropertyValue>)> {
        let base_pair = self
            .base
            .get(self.base_index)
            .map(|&(subject, key, _value)| (subject, key));
        let overlay_pair = self
            .overlay
            .get(self.overlay_index)
            .map(|&(subject, key, _entry)| (subject, key));
        match (base_pair, overlay_pair) {
            (None, None) => Step::Done,
            (Some(_base), None) => self.take_base().map_or(Step::Done, Step::Yield),
            (Some(base), Some(overlay)) if base < overlay => {
                self.take_base().map_or(Step::Done, Step::Yield)
            }
            (_other, Some(overlay)) => self.take_overlay(base_pair == Some(overlay)),
        }
    }
}

impl<'a> Iterator for PropertyMergeIter<'a> {
    type Item = (PropertySubject, PropertyKeyId, Cow<'a, PropertyValue>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.step() {
                Step::Done => return None,
                Step::Yield(item) => return Some(item),
                Step::Again => {}
            }
        }
    }
}

/// Flattens the base property map into an ascending `(subject, key, &value)`
/// vector. The nested `BTreeMap`s are already sorted, so the flattened order is
/// ascending by `(subject, key)`.
///
/// # Performance
///
/// This function is `O(base properties)`.
fn base_property_triples(
    base: &BaseRecords,
) -> Vec<(PropertySubject, PropertyKeyId, &PropertyValue)> {
    base.properties
        .iter()
        .flat_map(|(subject, keys)| keys.iter().map(move |(key, value)| (*subject, *key, value)))
        .collect()
}

/// Flattens the overlay property delta into an ascending `(subject, key,
/// set/remove)` vector.
///
/// # Performance
///
/// This function is `O(overlay property change)`.
fn overlay_property_triples(
    properties: &BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, Option<PropertyValue>>>,
) -> Vec<(PropertySubject, PropertyKeyId, &Option<PropertyValue>)> {
    properties
        .iter()
        .flat_map(|(subject, keys)| keys.iter().map(move |(key, value)| (*subject, *key, value)))
        .collect()
}
