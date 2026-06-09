//! Real (index-backed) membership and property lookups for the merged state.
//!
//! The [`crate::overlay::StateView`] membership and property lookups are
//! index-backed, running in `O(log n + matches)`, not `O(n)`.
//!
//! Two pieces compose, mirroring the base/overlay split of the state model:
//!
//! * [`BaseIndex`] — derived postings for one immutable base generation. They are built ONCE (via
//!   [`OwnedBaseIndex::from_records`]) at FREEZE time, serialized into the base file's
//!   `SECTION_INDEX_*` sections, and at OPEN they are BORROWED zero-copy out of the mapped base
//!   bytes ([`BorrowedBaseIndex`]) — no in-RAM rebuild on the open path. The enum lets every
//!   accessor run over either the owned (freeze-time / test-oracle) or the borrowed (open-time)
//!   form. It is `Arc`-shared by every reader of a generation, so `reader` stays `O(1)`.
//! * [`OverlayIndex`] — incremental deltas the overlay maintains as the writer mutates:
//!   `added`/`removed` postings per posting key. A lookup then computes `(base ∪ added) \ removed`
//!   over only the affected postings, so a base-only id costs `O(log n + matches)` and an
//!   overlay-touched id costs `O(log overlay change)`.
//!
//! # Correctness
//!
//! The index-backed result MUST equal the merge-scan result for every lookup,
//! and the BORROWED postings MUST equal the OWNED ones byte-for-byte. The
//! merge-scan oracle survives under `#[cfg(test)]` in [`super::overlay`] and is
//! differential-tested against the index path (see the overlay tests); the
//! borrowed-vs-owned equality is differential-tested in this module's proptest
//! and `debug_assert!`-checked at open.
//!
//! # Performance
//!
//! `perf: unspecified`; this module defines the index primitives. Each item
//! below carries its own contract: [`OwnedBaseIndex::from_records`] is `O(base
//! records + labels + properties)` (one build per generation, at FREEZE);
//! [`BorrowedBaseIndex::from_sections`] is `O(1)` (it only validates section
//! shapes and borrows the slices); every overlay delta mutator is `O(log overlay
//! change)`; every merged lookup is `O(log n + matches + overlay change)`.

use std::collections::{BTreeMap, BTreeSet, btree_map};

use zerocopy::byteorder::{LE, U64};

use crate::{
    DbError, ElementId, IncidenceId, LabelId, PropertyKeyId, PropertySubject, RelationId,
    RelationTypeId,
    overlay::StateView,
    state::{ElementRecord, IncidenceRecord, RelationRecord},
    value::{PropertyType, PropertyValue},
    wire,
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

/// A borrowed-or-owned id posting: either a reference to one generation's owned
/// `BTreeSet` (the freeze-time / test-oracle form) or a borrowed flat `[U64<LE>]`
/// member run sliced out of a persisted posting section (the open-time,
/// zero-copy form). Both expose the SAME ascending member sequence, so the merge
/// path is identical over either.
///
/// # Performance
///
/// [`Self::iter`] is `O(posting length)` (the merge path drains it into a
/// `BTreeSet`); the underlying run is ascending in both arms (the owned set keeps
/// it sorted, the borrowed run is written sorted).
enum Posting<'a, T> {
    /// A borrowed reference to one generation's owned posting set.
    Owned(&'a BTreeSet<T>),
    /// A borrowed flat member run out of a persisted posting section.
    Borrowed(&'a [U64<LE>]),
}

impl<T: Copy + Ord + FromU64> Posting<'_, T> {
    /// Iterates the posting's members in ascending order, decoding each borrowed
    /// `U64<LE>` word into `T`.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`; a full walk is `O(posting length)`.
    fn iter(&self) -> impl Iterator<Item = T> + '_ {
        let owned = match self {
            Self::Owned(set) => Some(set.iter().copied()),
            Self::Borrowed(_run) => None,
        };
        let borrowed = match self {
            Self::Owned(_set) => None,
            Self::Borrowed(run) => Some(run.iter().map(|word| T::from_u64(word.get()))),
        };
        owned
            .into_iter()
            .flatten()
            .chain(borrowed.into_iter().flatten())
    }
}

/// A canonical id (or other posting member) decodable from its raw `u64` value
/// pool word.
///
/// # Performance
///
/// [`Self::from_u64`] is `O(1)`.
trait FromU64 {
    /// Reconstructs the member from its raw `u64` value-pool word.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from_u64(value: u64) -> Self;
}

impl FromU64 for ElementId {
    fn from_u64(value: u64) -> Self {
        Self::new(value)
    }
}

impl FromU64 for RelationId {
    fn from_u64(value: u64) -> Self {
        Self::new(value)
    }
}

impl FromU64 for IncidenceId {
    fn from_u64(value: u64) -> Self {
        Self::new(value)
    }
}

impl FromU64 for LabelId {
    fn from_u64(value: u64) -> Self {
        Self::new(value)
    }
}

impl FromU64 for RelationTypeId {
    fn from_u64(value: u64) -> Self {
        Self::new(value)
    }
}

/// A lightweight borrowing VIEW over one immutable base generation's derived
/// postings: either a reference to an OWNED build (the gen-0 empty base and the
/// `#[cfg(test)]` oracle) or the BORROWED zero-copy postings of the mapped base's
/// `SECTION_INDEX_*` sections (the open path). Every accessor runs over both
/// arms, so the read path never branches on the representation. The view is
/// `Copy` (a reference or a bundle of slices), so it threads through the merge
/// path without cloning.
///
/// # Performance
///
/// Cloning/copying the view is `O(1)`. Every lookup accessor is `O(log n +
/// matches)`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum BaseIndex<'a> {
    /// A reference to an owned posting build (empty base / test oracle).
    Owned(&'a OwnedBaseIndex),
    /// Postings borrowed zero-copy out of the mapped base's posting sections.
    Borrowed(BorrowedBaseIndex<'a>),
}

/// Owned derived postings built once in RAM from one base generation's records.
/// This is the freeze-time build (serialized into the base sections) and the
/// `#[cfg(test)]` differential oracle; the production OPEN path borrows instead
/// (see [`BorrowedBaseIndex`]).
///
/// # Performance
///
/// [`Self::from_records`] is `O(base records + labels + properties)`; every
/// lookup accessor is `O(log n + matches)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OwnedBaseIndex {
    /// Elements carrying each label, ascending by element id within each posting.
    label_members: BTreeMap<LabelId, BTreeSet<ElementId>>,
    /// Relations of each relation type, ascending by relation id.
    relation_type_members: BTreeMap<RelationTypeId, BTreeSet<RelationId>>,
    /// Subjects carrying each `(key, value)`, ascending by subject. Ordered by
    /// `(key, value)` so a range over one key is a contiguous map slice.
    property_equality: BTreeMap<EqualityKey, BTreeSet<PropertySubject>>,
    /// Incidences attached to each element, ascending by incidence id (reverse
    /// adjacency: a subject's incidences resolve in `O(log n + degree)`).
    element_incidences: BTreeMap<ElementId, BTreeSet<IncidenceId>>,
    /// Incidences contained by each relation, ascending by incidence id.
    relation_incidences: BTreeMap<RelationId, BTreeSet<IncidenceId>>,
}

/// Derived postings borrowed zero-copy out of one mapped base generation's
/// `SECTION_INDEX_*` sections: a sorted fixed-size DIRECTORY (binary-searchable)
/// plus a flat `[U64<LE>]` VALUE POOL per posting map, with a side text pool for
/// the equality directory's text values. No allocation, no decode — the open path
/// borrows these slices straight from the map and the directory order matches the
/// owned `BTreeMap` order exactly.
///
/// # Performance
///
/// [`BorrowedBaseIndex::from_sections`] is `O(1)`; every lookup accessor is
/// `O(log n + matches)` (a binary search over a directory plus a value-pool
/// slice).
//
// `prove_covariant` makes the covariance contract explicit: every field is a
// shared `&'a [T]`, so the type is covariant in `'a` (required to be a `Yoke`
// payload). It mirrors the in-repo `backing::BaseView` precedent.
#[derive(Clone, Copy, Debug, yoke::Yokeable)]
#[yoke(prove_covariant)]
pub(crate) struct BorrowedBaseIndex<'a> {
    /// Label directory, ascending by label id, into `label_pool`.
    label_dir: &'a [wire::PostingDirEntry],
    /// Flat element-id value pool sliced by `label_dir`.
    label_pool: &'a [U64<LE>],
    /// Relation-type directory, ascending by relation-type id, into
    /// `relation_type_pool`.
    relation_type_dir: &'a [wire::PostingDirEntry],
    /// Flat relation-id value pool sliced by `relation_type_dir`.
    relation_type_pool: &'a [U64<LE>],
    /// Equality directory, in `(key_id, PropertyValue)` order, into
    /// `equality_pool` (subjects) and `equality_text` (text values).
    equality_dir: &'a [wire::EqualityDirEntry],
    /// Flat subject value pool (two `u64` words per subject) sliced by
    /// `equality_dir`.
    equality_pool: &'a [U64<LE>],
    /// Concatenated UTF-8 text values sliced by `equality_dir`.
    equality_text: &'a [u8],
    /// Element-incidence directory, ascending by element id, into
    /// `element_incidence_pool`.
    element_incidence_dir: &'a [wire::PostingDirEntry],
    /// Flat incidence-id value pool sliced by `element_incidence_dir`.
    element_incidence_pool: &'a [U64<LE>],
    /// Relation-incidence directory, ascending by relation id, into
    /// `relation_incidence_pool`.
    relation_incidence_dir: &'a [wire::PostingDirEntry],
    /// Flat incidence-id value pool sliced by `relation_incidence_pool`.
    relation_incidence_pool: &'a [U64<LE>],
}

impl OwnedBaseIndex {
    /// Builds the empty owned index (the gen-0 base has no records).
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn empty() -> Self {
        Self {
            label_members: BTreeMap::new(),
            relation_type_members: BTreeMap::new(),
            property_equality: BTreeMap::new(),
            element_incidences: BTreeMap::new(),
            relation_incidences: BTreeMap::new(),
        }
    }

    /// Builds the base index from one generation's owned records, indexing every
    /// element label, every relation type, and every property value.
    ///
    /// This is the FREEZE-time build (its output is serialized into the base
    /// sections) and the differential test oracle; the production OPEN path
    /// borrows the persisted sections instead of calling this.
    ///
    /// # Performance
    ///
    /// This function is `O(base records + labels + properties)`.
    pub(crate) fn from_records(
        elements: &BTreeMap<ElementId, ElementRecord>,
        relations: &BTreeMap<RelationId, RelationRecord>,
        incidences: &BTreeMap<IncidenceId, IncidenceRecord>,
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
        let mut element_incidences: BTreeMap<ElementId, BTreeSet<IncidenceId>> = BTreeMap::new();
        let mut relation_incidences: BTreeMap<RelationId, BTreeSet<IncidenceId>> = BTreeMap::new();
        for record in incidences.values() {
            element_incidences
                .entry(record.element)
                .or_default()
                .insert(record.id);
            relation_incidences
                .entry(record.relation)
                .or_default()
                .insert(record.id);
        }
        Self {
            label_members,
            relation_type_members,
            property_equality,
            element_incidences,
            relation_incidences,
        }
    }

    /// Builds the owned index by walking a merged [`StateView`] directly, so
    /// the freeze path derives the postings without first re-materializing the
    /// whole state into scratch record maps.
    ///
    /// Equal to [`Self::from_records`] over the same visible state (the
    /// record-map form remains the differential oracle for the open path).
    ///
    /// # Performance
    ///
    /// This function is `O(visible records + labels + properties)`.
    pub(crate) fn from_state(view: &impl StateView) -> Self {
        let mut label_members: BTreeMap<LabelId, BTreeSet<ElementId>> = BTreeMap::new();
        for record in view.elements() {
            for label in &record.labels {
                label_members.entry(*label).or_default().insert(record.id);
            }
        }
        let mut relation_type_members: BTreeMap<RelationTypeId, BTreeSet<RelationId>> =
            BTreeMap::new();
        for record in view.relations() {
            if let Some(relation_type) = record.relation_type {
                relation_type_members
                    .entry(relation_type)
                    .or_default()
                    .insert(record.id);
            }
        }
        let mut property_equality: BTreeMap<EqualityKey, BTreeSet<PropertySubject>> =
            BTreeMap::new();
        for (subject, key, value) in view.properties() {
            property_equality
                .entry((key, value.into_owned()))
                .or_default()
                .insert(subject);
        }
        let mut element_incidences: BTreeMap<ElementId, BTreeSet<IncidenceId>> = BTreeMap::new();
        let mut relation_incidences: BTreeMap<RelationId, BTreeSet<IncidenceId>> = BTreeMap::new();
        for record in view.incidences() {
            element_incidences
                .entry(record.element)
                .or_default()
                .insert(record.id);
            relation_incidences
                .entry(record.relation)
                .or_default()
                .insert(record.id);
        }
        Self {
            label_members,
            relation_type_members,
            property_equality,
            element_incidences,
            relation_incidences,
        }
    }

    /// Returns the borrowed label/relation-type/element/relation simple posting
    /// maps and the equality map, so [`crate::freeze`] can serialize each into its
    /// section. Returned in the canonical posting-map evaluation order.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` (it borrows the maps).
    #[expect(
        clippy::type_complexity,
        reason = "the five posting maps are returned together so freeze serializes them in one pass"
    )]
    pub(crate) const fn maps(
        &self,
    ) -> (
        &BTreeMap<LabelId, BTreeSet<ElementId>>,
        &BTreeMap<RelationTypeId, BTreeSet<RelationId>>,
        &BTreeMap<EqualityKey, BTreeSet<PropertySubject>>,
        &BTreeMap<ElementId, BTreeSet<IncidenceId>>,
        &BTreeMap<RelationId, BTreeSet<IncidenceId>>,
    ) {
        (
            &self.label_members,
            &self.relation_type_members,
            &self.property_equality,
            &self.element_incidences,
            &self.relation_incidences,
        )
    }
}

/// The canonical sort key for an equality directory entry: `(key_id,
/// PropertyValue)`, reconstructed from the entry so the binary search matches the
/// owned `BTreeMap<(PropertyKeyId, PropertyValue), …>` order EXACTLY.
///
/// # Performance
///
/// This function is `O(value length)` for text and `O(1)` otherwise.
fn equality_entry_key(entry: &wire::EqualityDirEntry, text: &[u8]) -> Result<EqualityKey, DbError> {
    let key = PropertyKeyId::new(entry.key_id.get());
    let value_type = wire::property_type_from_tag(entry.value_tag.get())
        .ok_or_else(|| DbError::invalid_store("equality index value tag out of range"))?;
    let value = match value_type {
        PropertyType::Boolean => PropertyValue::Boolean(entry.value_scalar.get() != 0),
        PropertyType::Integer => PropertyValue::Integer(entry.value_scalar.get().cast_signed()),
        PropertyType::Text => {
            let start = usize::try_from(entry.text_off.get()).map_err(|_overflow| {
                DbError::invalid_store("equality index text offset overflow")
            })?;
            let len = usize::try_from(entry.text_len.get()).map_err(|_overflow| {
                DbError::invalid_store("equality index text length overflow")
            })?;
            let end = start
                .checked_add(len)
                .ok_or_else(|| DbError::invalid_store("equality index text slice overflow"))?;
            let bytes = text
                .get(start..end)
                .ok_or_else(|| DbError::invalid_store("equality index text out of bounds"))?;
            let string = core::str::from_utf8(bytes)
                .map_err(|_error| DbError::invalid_store("equality index text is not UTF-8"))?;
            PropertyValue::Text(string.to_owned())
        }
    };
    Ok((key, value))
}

impl<'a> BorrowedBaseIndex<'a> {
    /// Borrows the index postings out of the mapped base's posting sections,
    /// validating that each directory's `(offset, len)` slices stay inside its
    /// value pool. No allocation, no decode.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::InvalidStore`] when a directory entry slices outside its
    /// value pool, an equality entry's value tag/text is malformed, or a subject
    /// value-pool word pair fails to decode.
    ///
    /// # Performance
    ///
    /// This function is `O(directory entries)` for the bounds validation; the
    /// borrows themselves are `O(1)`.
    #[expect(
        clippy::too_many_arguments,
        reason = "each of the five posting sections contributes its directory and pool slices"
    )]
    pub(crate) fn from_sections(
        label_dir: &'a [wire::PostingDirEntry],
        label_pool: &'a [U64<LE>],
        relation_type_dir: &'a [wire::PostingDirEntry],
        relation_type_pool: &'a [U64<LE>],
        equality_dir: &'a [wire::EqualityDirEntry],
        equality_pool: &'a [U64<LE>],
        equality_text: &'a [u8],
        element_incidence_dir: &'a [wire::PostingDirEntry],
        element_incidence_pool: &'a [U64<LE>],
        relation_incidence_dir: &'a [wire::PostingDirEntry],
        relation_incidence_pool: &'a [U64<LE>],
    ) -> Result<Self, DbError> {
        validate_simple_dir(label_dir, label_pool, "label")?;
        validate_simple_dir(relation_type_dir, relation_type_pool, "relation-type")?;
        validate_simple_dir(
            element_incidence_dir,
            element_incidence_pool,
            "element-incidence",
        )?;
        validate_simple_dir(
            relation_incidence_dir,
            relation_incidence_pool,
            "relation-incidence",
        )?;
        validate_equality_dir(equality_dir, equality_pool, equality_text)?;
        Ok(Self {
            label_dir,
            label_pool,
            relation_type_dir,
            relation_type_pool,
            equality_dir,
            equality_pool,
            equality_text,
            element_incidence_dir,
            element_incidence_pool,
            relation_incidence_dir,
            relation_incidence_pool,
        })
    }
}

/// Validates that every entry of a simple posting directory slices a member run
/// that stays inside `pool`.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when an entry's `(members_off, members_len)`
/// runs past the value pool.
///
/// # Performance
///
/// This function is `O(directory entries)`.
fn validate_simple_dir(
    dir: &[wire::PostingDirEntry],
    pool: &[U64<LE>],
    label: &'static str,
) -> Result<(), DbError> {
    for entry in dir {
        slice_pool(pool, entry.members_off.get(), entry.members_len.get())
            .ok_or_else(|| DbError::invalid_store(format!("{label} posting run out of bounds")))?;
    }
    Ok(())
}

/// Validates that every entry of the equality directory reconstructs a valid key
/// and slices a subject run (two words per subject) inside `pool`.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when an entry's value tag/text is malformed,
/// its subject run runs past the value pool, or a subject word pair fails to
/// decode.
///
/// # Performance
///
/// This function is `O(directory entries + subject words)`.
fn validate_equality_dir(
    dir: &[wire::EqualityDirEntry],
    pool: &[U64<LE>],
    text: &[u8],
) -> Result<(), DbError> {
    for entry in dir {
        let _key = equality_entry_key(entry, text)?;
        let run = slice_pool(pool, entry.members_off.get(), entry.members_len.get())
            .ok_or_else(|| DbError::invalid_store("equality posting run out of bounds"))?;
        if run.len() % 2 != 0 {
            return Err(DbError::invalid_store(
                "equality posting subject run is not a whole number of (kind, id) pairs",
            ));
        }
        for pair in run.chunks_exact(2) {
            let kind = u32::try_from(pair[0].get())
                .map_err(|_overflow| DbError::invalid_store("equality subject kind overflow"))?;
            wire::decode_subject(kind, pair[1].get())
                .ok_or_else(|| DbError::invalid_store("equality subject kind out of range"))?;
        }
    }
    Ok(())
}

/// Slices a `(offset, len)` member run out of a `[U64<LE>]` value pool, returning
/// `None` when it runs out of bounds.
///
/// # Performance
///
/// This function is `O(1)`.
fn slice_pool(pool: &[U64<LE>], offset: u64, len: u64) -> Option<&[U64<LE>]> {
    let start = usize::try_from(offset).ok()?;
    let length = usize::try_from(len).ok()?;
    let end = start.checked_add(length)?;
    pool.get(start..end)
}

/// Binary-searches a simple posting directory (ascending by `key`) for `key`,
/// returning its member run sliced out of `pool`.
///
/// # Performance
///
/// This function is `O(log n)`.
fn simple_lookup<'a>(
    dir: &[wire::PostingDirEntry],
    pool: &'a [U64<LE>],
    key: u64,
) -> Option<&'a [U64<LE>]> {
    let index = dir
        .binary_search_by(|entry| entry.key.get().cmp(&key))
        .ok()?;
    let entry = dir.get(index)?;
    slice_pool(pool, entry.members_off.get(), entry.members_len.get())
}

impl BaseIndex<'_> {
    /// Returns the base posting of elements carrying `label`, or `None`.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn label_posting(&self, label: LabelId) -> Option<Posting<'_, ElementId>> {
        match self {
            Self::Owned(owned) => owned.label_members.get(&label).map(Posting::Owned),
            Self::Borrowed(borrowed) => {
                simple_lookup(borrowed.label_dir, borrowed.label_pool, label.get())
                    .map(Posting::Borrowed)
            }
        }
    }

    /// Returns the base posting of relations of `relation_type`, or `None`.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn relation_type_posting(
        &self,
        relation_type: RelationTypeId,
    ) -> Option<Posting<'_, RelationId>> {
        match self {
            Self::Owned(owned) => owned
                .relation_type_members
                .get(&relation_type)
                .map(Posting::Owned),
            Self::Borrowed(borrowed) => simple_lookup(
                borrowed.relation_type_dir,
                borrowed.relation_type_pool,
                relation_type.get(),
            )
            .map(Posting::Borrowed),
        }
    }

    /// Returns the base posting of incidences attached to `element`, or `None`.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn element_incidence_posting(&self, element: ElementId) -> Option<Posting<'_, IncidenceId>> {
        match self {
            Self::Owned(owned) => owned.element_incidences.get(&element).map(Posting::Owned),
            Self::Borrowed(borrowed) => simple_lookup(
                borrowed.element_incidence_dir,
                borrowed.element_incidence_pool,
                element.get(),
            )
            .map(Posting::Borrowed),
        }
    }

    /// Returns the base posting of incidences contained by `relation`, or `None`.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn relation_incidence_posting(&self, relation: RelationId) -> Option<Posting<'_, IncidenceId>> {
        match self {
            Self::Owned(owned) => owned.relation_incidences.get(&relation).map(Posting::Owned),
            Self::Borrowed(borrowed) => simple_lookup(
                borrowed.relation_incidence_dir,
                borrowed.relation_incidence_pool,
                relation.get(),
            )
            .map(Posting::Borrowed),
        }
    }

    /// Returns the ascending subjects carrying `(key, value)`, or `None`.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + value length)`.
    fn equality_subjects(
        &self,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Option<Vec<PropertySubject>> {
        match self {
            Self::Owned(owned) => owned
                .property_equality
                .get(&(key, value.clone()))
                .map(|set| set.iter().copied().collect()),
            Self::Borrowed(borrowed) => {
                let probe = (key, value.clone());
                let index = borrowed
                    .equality_dir
                    .binary_search_by(|entry| {
                        equality_entry_key(entry, borrowed.equality_text)
                            .map_or(std::cmp::Ordering::Less, |candidate| candidate.cmp(&probe))
                    })
                    .ok()?;
                let entry = borrowed.equality_dir.get(index)?;
                Some(borrowed.decode_subjects(entry))
            }
        }
    }

    /// Iterates this key's equality postings whose value falls in the inclusive
    /// range `[min, max]`, ascending by value, as `(EqualityKey, subjects)`.
    /// `min > max` yields nothing.
    ///
    /// The postings are ordered by `(key, value)`, so the range is one contiguous
    /// slice (a map range over the owned arm, a directory sub-slice over the
    /// borrowed arm) — no full scan.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + matching postings + matched subjects)`.
    fn range_postings(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Vec<(EqualityKey, Vec<PropertySubject>)> {
        if min > max {
            return Vec::new();
        }
        match self {
            Self::Owned(owned) => owned
                .property_equality
                .range((key, min.clone())..=(key, max.clone()))
                .map(|(pair, subjects)| (pair.clone(), subjects.iter().copied().collect()))
                .collect(),
            Self::Borrowed(borrowed) => borrowed.range_postings(key, min, max),
        }
    }

    /// Iterates this key's equality postings, ascending by value, as
    /// `(PropertyValue, subjects)` — the whole key, from its first posting to its
    /// last. Used by [`OverlayIndex::property_key_subjects`].
    ///
    /// # Performance
    ///
    /// This method is `O(log n + postings for key + subjects)`.
    fn key_postings(&self, key: PropertyKeyId) -> Vec<(PropertyValue, Vec<PropertySubject>)> {
        // `Boolean(false)` is the global minimum `PropertyValue`, so the range
        // begins at this key's first posting.
        let start = (key, PropertyValue::Boolean(false));
        match self {
            Self::Owned(owned) => owned
                .property_equality
                .range(start..)
                .take_while(|((entry_key, _value), _subjects)| *entry_key == key)
                .map(|((_key, value), subjects)| {
                    (value.clone(), subjects.iter().copied().collect())
                })
                .collect(),
            Self::Borrowed(borrowed) => borrowed.key_postings(key),
        }
    }
}

impl BorrowedBaseIndex<'_> {
    /// Decodes one equality directory entry's subject run from the value pool into
    /// an ascending owned subject list.
    ///
    /// # Performance
    ///
    /// This method is `O(subjects)`.
    fn decode_subjects(&self, entry: &wire::EqualityDirEntry) -> Vec<PropertySubject> {
        let run = slice_pool(
            self.equality_pool,
            entry.members_off.get(),
            entry.members_len.get(),
        )
        .unwrap_or(&[]);
        run.chunks_exact(2)
            .filter_map(|pair| {
                let kind = u32::try_from(pair[0].get()).ok()?;
                wire::decode_subject(kind, pair[1].get())
            })
            .collect()
    }

    /// Finds the directory index of this key's first posting (the lower bound at
    /// `(key, Boolean(false))`), or `None` when the key has no postings.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    fn key_lower_bound(&self, key: PropertyKeyId) -> usize {
        // Partition point at the first entry whose key_id >= `key`; since
        // Boolean(false) is the global-minimum value, this is the key's first
        // posting.
        self.equality_dir
            .partition_point(|entry| entry.key_id.get() < key.get())
    }

    /// The borrowed-arm range scan: the contiguous directory slice of `key`'s
    /// postings whose value falls in `[min, max]`.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + matching postings + matched subjects)`.
    fn range_postings(
        &self,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Vec<(EqualityKey, Vec<PropertySubject>)> {
        let mut out = Vec::new();
        for entry in self
            .equality_dir
            .get(self.key_lower_bound(key)..)
            .unwrap_or(&[])
        {
            if entry.key_id.get() != key.get() {
                break;
            }
            let Ok((entry_key, value)) = equality_entry_key(entry, self.equality_text) else {
                continue;
            };
            if value < *min {
                continue;
            }
            if value > *max {
                break;
            }
            out.push(((entry_key, value), self.decode_subjects(entry)));
        }
        out
    }

    /// The borrowed-arm whole-key scan: every posting of `key`, ascending by
    /// value.
    ///
    /// # Performance
    ///
    /// This method is `O(log n + postings for key + subjects)`.
    fn key_postings(&self, key: PropertyKeyId) -> Vec<(PropertyValue, Vec<PropertySubject>)> {
        let mut out = Vec::new();
        for entry in self
            .equality_dir
            .get(self.key_lower_bound(key)..)
            .unwrap_or(&[])
        {
            if entry.key_id.get() != key.get() {
                break;
            }
            let Ok((_key, value)) = equality_entry_key(entry, self.equality_text) else {
                continue;
            };
            out.push((value, self.decode_subjects(entry)));
        }
        out
    }
}

/// The owned-snapshot materialization + differential comparison exist ONLY for
/// the open-time `debug_assert!` safety net and the differential proptest; they
/// walk every posting (`O(base)`) and are never on the release read path, so they
/// are compiled out of a release non-test build.
#[cfg(any(debug_assertions, test))]
impl BaseIndex<'_> {
    /// Materializes this index view into an owned snapshot, so the borrowed and
    /// owned arms can be compared with `==`. Used by the open-time differential
    /// `debug_assert!` and the differential proptest ONLY — it walks every
    /// posting, so it is `O(base)` and never on the release read path.
    ///
    /// # Performance
    ///
    /// This method is `O(base records + labels + properties)`.
    pub(crate) fn to_owned_snapshot(self) -> OwnedBaseIndex {
        match self {
            Self::Owned(owned) => owned.clone(),
            Self::Borrowed(borrowed) => borrowed.to_owned_snapshot(),
        }
    }
}

#[cfg(any(debug_assertions, test))]
impl BorrowedBaseIndex<'_> {
    /// Materializes the borrowed postings into an owned snapshot.
    ///
    /// # Performance
    ///
    /// This method is `O(base records + labels + properties)`.
    fn to_owned_snapshot(self) -> OwnedBaseIndex {
        let label_members = materialize_simple(self.label_dir, self.label_pool);
        let relation_type_members =
            materialize_simple(self.relation_type_dir, self.relation_type_pool);
        let element_incidences =
            materialize_simple(self.element_incidence_dir, self.element_incidence_pool);
        let relation_incidences =
            materialize_simple(self.relation_incidence_dir, self.relation_incidence_pool);
        let mut property_equality: BTreeMap<EqualityKey, BTreeSet<PropertySubject>> =
            BTreeMap::new();
        for entry in self.equality_dir {
            if let Ok(key) = equality_entry_key(entry, self.equality_text) {
                property_equality.insert(key, self.decode_subjects(entry).into_iter().collect());
            }
        }
        OwnedBaseIndex {
            label_members,
            relation_type_members,
            property_equality,
            element_incidences,
            relation_incidences,
        }
    }
}

/// Materializes a borrowed simple posting directory + value pool into an owned
/// `key -> ascending member set` map.
///
/// # Performance
///
/// This method is `O(postings + members)`.
#[cfg(any(debug_assertions, test))]
fn materialize_simple<K, M>(
    dir: &[wire::PostingDirEntry],
    pool: &[U64<LE>],
) -> BTreeMap<K, BTreeSet<M>>
where
    K: Ord + FromU64,
    M: Ord + Copy + FromU64,
{
    let mut map: BTreeMap<K, BTreeSet<M>> = BTreeMap::new();
    for entry in dir {
        let run = slice_pool(pool, entry.members_off.get(), entry.members_len.get()).unwrap_or(&[]);
        let members = run.iter().map(|word| M::from_u64(word.get())).collect();
        map.insert(K::from_u64(entry.key.get()), members);
    }
    map
}

/// Returns whether two index views hold byte-identical postings, by materializing
/// and comparing them. Used ONLY by the open-time `debug_assert!` differential
/// check and the differential proptest; it is `O(base)` and never on the release
/// read path.
///
/// # Performance
///
/// This function is `O(base records + labels + properties)`.
#[cfg(any(debug_assertions, test))]
pub(crate) fn indexes_agree(left: BaseIndex<'_>, right: BaseIndex<'_>) -> bool {
    left.to_owned_snapshot() == right.to_owned_snapshot()
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
    /// Element→incidence adjacency the overlay adds, by element.
    element_incidence_added: BTreeMap<ElementId, BTreeSet<IncidenceId>>,
    /// Element→incidence adjacency the overlay removes, by element.
    element_incidence_removed: BTreeMap<ElementId, BTreeSet<IncidenceId>>,
    /// Relation→incidence adjacency the overlay adds, by relation.
    relation_incidence_added: BTreeMap<RelationId, BTreeSet<IncidenceId>>,
    /// Relation→incidence adjacency the overlay removes, by relation.
    relation_incidence_removed: BTreeMap<RelationId, BTreeSet<IncidenceId>>,
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
            element_incidence_added: BTreeMap::new(),
            element_incidence_removed: BTreeMap::new(),
            relation_incidence_added: BTreeMap::new(),
            relation_incidence_removed: BTreeMap::new(),
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

    /// Records that `incidence` (in `relation`, on `element`) now exists: it joins
    /// both reverse-adjacency postings and drops any prior removal.
    ///
    /// # Performance
    ///
    /// This method is `O(log overlay change)`.
    pub(crate) fn on_create_incidence(
        &mut self,
        incidence: IncidenceId,
        relation: RelationId,
        element: ElementId,
    ) {
        remove_posting(&mut self.element_incidence_removed, element, &incidence);
        self.element_incidence_added
            .entry(element)
            .or_default()
            .insert(incidence);
        remove_posting(&mut self.relation_incidence_removed, relation, &incidence);
        self.relation_incidence_added
            .entry(relation)
            .or_default()
            .insert(incidence);
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
        incidence: IncidenceId,
        relation: RelationId,
        element: ElementId,
        properties: &BTreeMap<PropertyKeyId, PropertyValue>,
    ) {
        remove_posting(&mut self.element_incidence_added, element, &incidence);
        self.element_incidence_removed
            .entry(element)
            .or_default()
            .insert(incidence);
        remove_posting(&mut self.relation_incidence_added, relation, &incidence);
        self.relation_incidence_removed
            .entry(relation)
            .or_default()
            .insert(incidence);
        self.withdraw_subject_properties(PropertySubject::Incidence(incidence), properties);
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
    pub(crate) fn elements_with_label(
        &self,
        base: BaseIndex<'_>,
        label: LabelId,
    ) -> Vec<ElementId> {
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
        base: BaseIndex<'_>,
        relation_type: RelationTypeId,
    ) -> Vec<RelationId> {
        merge_posting(
            base.relation_type_posting(relation_type),
            self.relation_type_added.get(&relation_type),
            self.relation_type_removed.get(&relation_type),
        )
    }

    /// Merges base + overlay reverse-adjacency postings for `element` into the
    /// visible, ascending incidence-id list.
    ///
    /// # Performance
    ///
    /// This method is `O(degree + overlay change)`.
    pub(crate) fn element_incidences(
        &self,
        base: BaseIndex<'_>,
        element: ElementId,
    ) -> Vec<IncidenceId> {
        merge_posting(
            base.element_incidence_posting(element),
            self.element_incidence_added.get(&element),
            self.element_incidence_removed.get(&element),
        )
    }

    /// Merges base + overlay reverse-adjacency postings for `relation` into the
    /// visible, ascending incidence-id list.
    ///
    /// # Performance
    ///
    /// This method is `O(degree + overlay change)`.
    pub(crate) fn relation_incidences(
        &self,
        base: BaseIndex<'_>,
        relation: RelationId,
    ) -> Vec<IncidenceId> {
        merge_posting(
            base.relation_incidence_posting(relation),
            self.relation_incidence_added.get(&relation),
            self.relation_incidence_removed.get(&relation),
        )
    }

    /// Returns every subject currently carrying property `key` (under any value)
    /// with its visible value, merging base + overlay. A property is single-valued
    /// per key, so each subject appears once. Drives the reconcile prune
    /// (`retain`): the complement of the kept values is the stale set.
    ///
    /// # Performance
    ///
    /// This method is `O(postings for key + overlay change)`.
    pub(crate) fn property_key_subjects(
        &self,
        base: BaseIndex<'_>,
        key: PropertyKeyId,
    ) -> Vec<(PropertySubject, PropertyValue)> {
        let start = (key, PropertyValue::Boolean(false));
        let mut visible: BTreeMap<PropertySubject, PropertyValue> = BTreeMap::new();
        for (value, subjects) in base.key_postings(key) {
            let removed = self.equality_removed.get(&(key, value.clone()));
            let kept = subjects
                .into_iter()
                .filter(|subject| removed.is_none_or(|set| !set.contains(subject)))
                .map(|subject| (subject, value.clone()));
            visible.extend(kept);
        }
        for ((entry_key, value), subjects) in self.equality_added.range(start..) {
            if *entry_key != key {
                break;
            }
            visible.extend(subjects.iter().map(|subject| (*subject, value.clone())));
        }
        visible.into_iter().collect()
    }

    /// Merges the base equality posting for `(key, value)` with this overlay's
    /// deltas into the visible, ascending subject list.
    ///
    /// # Performance
    ///
    /// This method is `O(matches + overlay change + value length)`.
    pub(crate) fn property_equal(
        &self,
        base: BaseIndex<'_>,
        key: PropertyKeyId,
        value: &PropertyValue,
    ) -> Vec<PropertySubject> {
        let pair = (key, value.clone());
        let base_subjects = base.equality_subjects(key, value).unwrap_or_default();
        merge_posting_seq(
            base_subjects,
            self.equality_added.get(&pair),
            self.equality_removed.get(&pair),
        )
    }

    /// Merges the base + overlay equality postings whose value falls in the
    /// inclusive range `[min, max]` under `key`, into the visible, ascending
    /// subject list (each subject once).
    ///
    /// The base side is the contiguous ordered slice of the equality postings; the
    /// overlay side is the matching contiguous slice of `equality_added`/
    /// `equality_removed`. A subject visible under any value in range appears
    /// once.
    ///
    /// # Performance
    ///
    /// This method is `O(matching postings + matches + overlay change)`.
    pub(crate) fn property_range(
        &self,
        base: BaseIndex<'_>,
        key: PropertyKeyId,
        min: &PropertyValue,
        max: &PropertyValue,
    ) -> Vec<PropertySubject> {
        if min > max {
            return Vec::new();
        }
        let mut visible: BTreeSet<PropertySubject> = BTreeSet::new();
        // Base postings in range, masked per posting by this overlay's removals.
        for (pair, subjects) in base.range_postings(key, min, max) {
            let removed = self.equality_removed.get(&pair);
            let kept = subjects
                .into_iter()
                .filter(|subject| removed.is_none_or(|set| !set.contains(subject)));
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
        base: BaseIndex<'_>,
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

/// Merges one posting key's base posting (owned or borrowed) with the overlay
/// `added`/`removed` deltas into the visible, ascending, deduplicated member
/// list: `(base ∪ added) \ removed`.
///
/// # Performance
///
/// This function is `O(base posting + added + removed)`.
fn merge_posting<M: Copy + Ord + FromU64>(
    base: Option<Posting<'_, M>>,
    added: Option<&BTreeSet<M>>,
    removed: Option<&BTreeSet<M>>,
) -> Vec<M> {
    let mut visible: BTreeSet<M> = BTreeSet::new();
    if let Some(base) = base {
        visible.extend(base.iter());
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

/// Merges a pre-collected ascending base subject sequence with the overlay
/// `added`/`removed` deltas into the visible, ascending, deduplicated subject
/// list. The equality posting cannot expose a `Posting<'_, T>` (its borrowed
/// members are `(kind, id)` word pairs, not single ids), so it collects first.
///
/// # Performance
///
/// This function is `O(base subjects + added + removed)`.
fn merge_posting_seq(
    base: Vec<PropertySubject>,
    added: Option<&BTreeSet<PropertySubject>>,
    removed: Option<&BTreeSet<PropertySubject>>,
) -> Vec<PropertySubject> {
    let mut visible: BTreeSet<PropertySubject> = base.into_iter().collect();
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
