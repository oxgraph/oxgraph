//! The frozen, published overlay and the owned base realization under it.
//!
//! [`Overlay`] is the immutable published delta (an `Arc<Overlay>` is
//! immutable forever); [`BaseRecords`] is the owned realization of one base
//! generation's records, with its derived index borrowed zero-copy from the
//! mapped base; [`Snapshot`] is the immutable `(generation, lsn, base,
//! overlay)` unit a read transaction pins.

use std::{collections::BTreeMap, sync::Arc};

use yoke::Yoke;

#[cfg(any(test, kani))]
use super::write::WriteOverlay;
use super::{Delta, SubjectDelta, merge::MergedState};
use crate::{
    Catalog, CheckpointGeneration, DbError, ElementId, IncidenceId, LabelId, PropertyKeyId,
    RelationId, RoleId,
    backing::{Base, BaseView},
    id::CommitSeq,
    index::{BaseIndex, BorrowedBaseIndex, OverlayIndex, OwnedBaseIndex},
    state::{ElementRecord, IncidenceRecord, LabelSet, NextIds, PropertySubject, RelationRecord},
    value::{PropertyType, PropertyValue},
};
#[cfg(all(kani, feature = "kani-heavy"))]
use crate::{IndexId, ProjectionId, RelationTypeId};

/// The FROZEN, published delta: an immutable [`Overlay`] shared via `Arc`.
///
/// A published `Arc<Overlay>` is immutable forever. To advance state, a commit
/// seeds a fresh writer from the parent ([`WriteOverlay::from_overlay`]), applies
/// the new mutations on top, freezes the result with [`WriteOverlay::freeze`],
/// and publishes a brand-new `Arc<Overlay>`; it never mutates a published
/// overlay in place. (The `cfg(test)`/`cfg(kani)` `with_applied` helper proves
/// that same clone-the-parent, apply-the-delta semantics for the merge laws.) A
/// reader that pinned an `Arc<Overlay>` therefore observes a fixed delta for the
/// lifetime of its pin.
///
/// # Performance
///
/// Cloning is `O(change)` map entries with `O(1)` per entry: the delta map
/// node structure is cloned, but record label sets, text property values,
/// per-subject property delta maps, and index posting sets are `Arc`-shared
/// copy-on-write, so no payload bytes are duplicated. The accessors below are
/// `O(log change)` or `O(change)` as documented.
#[derive(Clone, Debug)]
pub(crate) struct Overlay {
    /// Element delta: id -> add/override record, or tombstone.
    pub(super) elements: Delta<ElementRecord>,
    /// Relation delta: id -> add/override record, or tombstone.
    pub(super) relations: Delta<RelationRecord>,
    /// Incidence delta: id -> add/override record, or tombstone.
    pub(super) incidences: Delta<IncidenceRecord>,
    /// Property delta: subject -> key -> set value, or remove (tombstone).
    /// Each per-subject inner map is `Arc`-shared copy-on-write with the
    /// writer seeded from this overlay.
    pub(super) properties: BTreeMap<PropertySubject, SubjectDelta>,
    /// Catalog additions folded into this published overlay.
    pub(super) catalog: Catalog,
    /// The nine monotonic id allocators (the watermark) for this overlay.
    pub(super) next: NextIds,
    /// Frozen incremental index deltas this overlay carries, so a reader's
    /// merged lookup over `(base index, overlay index)` is index-backed.
    pub(super) index: OverlayIndex,
}

impl Overlay {
    /// Creates the empty overlay for a freshly opened base: no deltas, the
    /// base's catalog, and the base's watermark.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`; it moves the watermark and catalog in.
    pub(crate) const fn empty(next: NextIds, catalog: Catalog) -> Self {
        Self {
            elements: BTreeMap::new(),
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            catalog,
            next,
            index: OverlayIndex::new(),
        }
    }

    /// Returns the watermark (the nine monotonic allocators) for this overlay.
    ///
    /// The live commit path seeds a writer from this overlay (via
    /// [`WriteOverlay::from_overlay`]) rather than reading the watermark out
    /// here, so this accessor is exercised by the merge-law proofs and tests.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[cfg(any(test, kani))]
    pub(crate) const fn next_ids(&self) -> NextIds {
        self.next
    }

    /// Builds the NEXT published overlay by cloning this one (the parent) and
    /// applying `delta` on top, layer by layer: a `delta` entry (record,
    /// tombstone, property set/remove) overrides the same key from the parent;
    /// the watermark and catalog are taken from `delta`. The parent is left
    /// untouched — this NEVER mutates a published overlay.
    ///
    /// The live commit path instead seeds the writer from the parent overlay
    /// ([`WriteOverlay::from_overlay`]) and freezes the seeded delta directly, so
    /// this clone-and-apply primitive is exercised by the merge-law proofs and
    /// tests (it proves the same parent-untouched, watermark-monotone contract).
    ///
    /// # Performance
    ///
    /// This function is `O(parent change + delta change)`.
    #[cfg(any(test, kani))]
    pub(crate) fn with_applied(&self, delta: &WriteOverlay) -> Self {
        let mut next = self.clone();
        for (id, entry) in &delta.elements {
            next.elements.insert(*id, entry.clone());
        }
        for (id, entry) in &delta.relations {
            next.relations.insert(*id, entry.clone());
        }
        for (id, entry) in &delta.incidences {
            next.incidences.insert(*id, *entry);
        }
        for (subject, keys) in &delta.properties {
            let merged = Arc::make_mut(next.properties.entry(*subject).or_default());
            for (key, value) in keys.iter() {
                merged.insert(*key, value.clone());
            }
        }
        next.catalog = delta.catalog.clone();
        next.next = delta.next;
        // The delta carries the full composed index (seeded from this parent's
        // index, then mutated), matching `freeze`/`from_overlay` on the live
        // path, so the published overlay takes the delta's index verbatim.
        next.index = delta.index.clone();
        next
    }
}

/// How one base generation's derived index postings are held: either an OWNED
/// build (the empty gen-0 base and the `#[cfg(test)]` differential oracle) or a
/// [`Yoke`] BORROWING the persisted postings zero-copy out of the mapped
/// [`Base`] (the production OPEN path — no rebuild from records).
///
/// The yoke co-owns its [`Arc<Base>`] cart, so the borrowed [`BorrowedBaseIndex`]
/// lives exactly as long as this [`BaseRecords`] (which is itself `Arc`-shared),
/// and `forbid(unsafe)` holds (the borrow is achieved through `yoke`, never raw
/// pointers).
///
/// # Performance
///
/// [`Self::view`] is `O(1)`.
enum HeldIndex {
    /// An owned posting build (empty base / test oracle).
    Owned(OwnedBaseIndex),
    /// Postings borrowed zero-copy from the mapped base, yoked to its `Arc`.
    Borrowed(Yoke<BorrowedBaseIndex<'static>, Arc<Base>>),
}

impl std::fmt::Debug for HeldIndex {
    /// Formats the held index without dumping the (potentially large) postings:
    /// only the arm tag, since [`Yoke`] and the mapped base are not [`Debug`].
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Owned(_owned) => formatter.write_str("HeldIndex::Owned(..)"),
            Self::Borrowed(_yoke) => formatter.write_str("HeldIndex::Borrowed(..)"),
        }
    }
}

impl HeldIndex {
    /// Returns a borrowing [`BaseIndex`] view over the held postings.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn view(&self) -> BaseIndex<'_> {
        match self {
            Self::Owned(owned) => BaseIndex::Owned(owned),
            Self::Borrowed(yoke) => BaseIndex::Borrowed(*yoke.get()),
        }
    }
}

impl Clone for HeldIndex {
    /// Clones the held index. The borrowed arm clones the [`Yoke`] (an
    /// `Arc::clone` of the cart plus a copy of the borrowed slice bundle), so no
    /// posting bytes are duplicated.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` for the borrowed arm and `O(postings)` for the
    /// owned arm.
    fn clone(&self) -> Self {
        match self {
            Self::Owned(owned) => Self::Owned(owned.clone()),
            Self::Borrowed(yoke) => Self::Borrowed(yoke.clone()),
        }
    }
}

/// Owned realization of one base generation's records, materialized once from a
/// borrowed [`BaseView`] so the merge has a stable `&ElementRecord` to borrow
/// for the [`Cow::Borrowed`] fast path, paired with that generation's derived
/// index postings — BORROWED zero-copy from the mapped base at open (no rebuild).
///
/// The borrowed [`BaseView`] holds wire records (`ElementWire` with a label
/// offset/length), not owned [`ElementRecord`] values, so a base point read
/// cannot hand out `&ElementRecord` directly. This type owns the decoded records
/// once; [`MergedState`] then borrows from it for base-only ids (zero clone per
/// read). The index, by contrast, is NOT rebuilt: [`Self::open`] borrows the
/// persisted `SECTION_INDEX_*` postings yoked to the [`Arc<Base>`].
///
/// # Memory cost
///
/// The record realization carries a standing ~2x base memory cost per generation
/// ([`Self::open`] duplicates the base record bytes into owned `BTreeMap`s); the
/// INDEX adds none — it is borrowed from the same mapped bytes. It is built once
/// per checkpoint generation and `Arc`-shared across every commit that pins that
/// generation via [`Snapshot::with_shared_base_records`], so a commit re-decodes
/// neither the base nor its index and stays `O(change)`.
///
/// # Performance
///
/// [`Self::open`] is `O(base records + labels + property bytes)` for the record
/// decode plus `O(index directory entries)` for the borrow validation (NOT the
/// `O(base)` index rebuild the prior design paid); every accessor below is
/// `O(log n)` or `O(1)`.
#[derive(Clone, Debug)]
pub(crate) struct BaseRecords {
    /// Decoded elements keyed by id, ascending.
    pub(super) elements: BTreeMap<ElementId, ElementRecord>,
    /// Decoded relations keyed by id, ascending.
    pub(super) relations: BTreeMap<RelationId, RelationRecord>,
    /// Decoded incidences keyed by id, ascending.
    pub(super) incidences: BTreeMap<IncidenceId, IncidenceRecord>,
    /// Decoded properties keyed by subject then key.
    pub(super) properties: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>>,
    /// Derived membership/equality postings for this generation: borrowed
    /// zero-copy from the mapped base at open, or owned for the empty/test base.
    index: HeldIndex,
}

/// One base generation's decoded record maps (without the index), shared by the
/// production [`BaseRecords::open`] and the `#[cfg(test)]` `from_view` paths.
///
/// # Performance
///
/// `perf: unspecified`; an aggregate of the four decoded maps.
type DecodedRecords = (
    BTreeMap<ElementId, ElementRecord>,
    BTreeMap<RelationId, RelationRecord>,
    BTreeMap<IncidenceId, IncidenceRecord>,
    BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>>,
);

/// Decodes the four owned record maps from a borrowed base `view`.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when a record references a label run,
/// property text, or subject kind the wire layout cannot resolve.
///
/// # Performance
///
/// This function is `O(base records + labels + property bytes)`.
fn decode_records(view: &BaseView<'_>) -> Result<DecodedRecords, DbError> {
    let mut elements = BTreeMap::new();
    for record in view.elements() {
        let labels = decode_labels(view.element_label_run(record))?;
        let id = ElementId::new(record.id.get());
        elements.insert(id, ElementRecord { id, labels });
    }
    let mut relations = BTreeMap::new();
    for record in view.relations() {
        let labels = decode_labels(view.relation_label_run(record))?;
        let id = RelationId::new(record.id.get());
        relations.insert(
            id,
            RelationRecord {
                id,
                relation_type: crate::wire::decode_relation_type(record.relation_type.get()),
                labels,
            },
        );
    }
    let mut incidences = BTreeMap::new();
    for record in view.incidences() {
        let id = IncidenceId::new(record.id.get());
        incidences.insert(
            id,
            IncidenceRecord {
                id,
                relation: RelationId::new(record.relation.get()),
                element: ElementId::new(record.element.get()),
                role: RoleId::new(record.role.get()),
            },
        );
    }
    let properties = decode_base_properties(view)?;
    Ok((elements, relations, incidences, properties))
}

impl BaseRecords {
    /// Builds the records of an empty base (the gen-0 base `create` writes): no
    /// elements, relations, incidences, properties, or index postings.
    ///
    /// This constructor is infallible: it allocates only empty maps and never
    /// decodes wire bytes.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn empty() -> Self {
        Self {
            elements: BTreeMap::new(),
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            index: HeldIndex::Owned(OwnedBaseIndex::empty()),
        }
    }

    /// Materializes one base generation's owned records from `base`, decoding the
    /// wire record arrays once and BORROWING the derived index postings zero-copy
    /// from the same mapped base (yoked to its `Arc`) — the index is NOT rebuilt.
    ///
    /// This is the production OPEN path. A base that lacks the persisted index
    /// postings was already rejected by [`Base::open`] with
    /// [`DbError::UnsupportedFormat`], so there is no rebuild-from-records
    /// fallback here.
    ///
    /// In debug builds, the borrowed index is differentially checked against the
    /// `from_records` oracle (see `debug_assert_index_matches`); release builds
    /// skip the check, so open stays non-`O(base)` for the index.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::InvalidStore`] when a record or a posting section is
    /// malformed.
    ///
    /// # Performance
    ///
    /// This function is `O(base records + labels + property bytes)` for the record
    /// decode plus `O(index directory entries)` for the borrow validation.
    pub(crate) fn open(base: &Arc<Base>) -> Result<Self, DbError> {
        let (elements, relations, incidences, properties) = decode_records(base.get())?;
        // Yoke the borrowed index to a clone of the base `Arc`, so the borrowed
        // postings live as long as these records (which are themselves shared).
        let index = HeldIndex::Borrowed(Yoke::attach_to_cart(Arc::clone(base), |base: &Base| {
            base.get().index()
        }));
        let records = Self {
            elements,
            relations,
            incidences,
            properties,
            index,
        };
        #[cfg(debug_assertions)]
        records.debug_assert_index_matches();
        Ok(records)
    }

    /// Asserts that every borrowed index posting equals the owned `from_records`
    /// oracle rebuilt from this generation's records, across every accessor. This
    /// is the open-time differential safety net; it exists ONLY in debug builds
    /// (and is called only there), so a release open never pays the `O(base)`
    /// rebuild in production.
    ///
    /// # Performance
    ///
    /// This method is `O(base records + labels + properties)`.
    #[cfg(debug_assertions)]
    fn debug_assert_index_matches(&self) {
        let oracle = OwnedBaseIndex::from_records(
            &self.elements,
            &self.relations,
            &self.incidences,
            &self.properties,
        );
        debug_assert!(
            crate::index::indexes_agree(self.index.view(), BaseIndex::Owned(&oracle)),
            "borrowed base index disagrees with the from_records oracle",
        );
    }

    /// Returns a borrowing view over this generation's derived index postings
    /// (membership + equality), backed by either the borrowed mapped sections or
    /// the owned build.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) fn index(&self) -> BaseIndex<'_> {
        self.index.view()
    }

    /// Materializes one base generation's owned records AND rebuilds the OWNED
    /// derived index from those records (via `from_records`). This is the
    /// `#[cfg(test)]` differential-oracle path: the production OPEN path
    /// ([`Self::open`]) borrows the persisted index instead of rebuilding it, and
    /// the tests compare the two. Test code holds a borrowed [`BaseView`] without
    /// the [`Arc<Base>`] the yoke needs, so this owned form serves them.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::InvalidStore`] when a record references a label run,
    /// property text, or subject kind the wire layout cannot resolve.
    ///
    /// # Performance
    ///
    /// This function is `O(base records + labels + property bytes)`.
    #[cfg(test)]
    pub(crate) fn from_view(view: &BaseView<'_>) -> Result<Self, DbError> {
        let (elements, relations, incidences, properties) = decode_records(view)?;
        let index = HeldIndex::Owned(OwnedBaseIndex::from_records(
            &elements,
            &relations,
            &incidences,
            &properties,
        ));
        Ok(Self {
            elements,
            relations,
            incidences,
            properties,
            index,
        })
    }

    /// Returns the base element record for `id`, if present.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    pub(super) fn element(&self, id: ElementId) -> Option<&ElementRecord> {
        self.elements.get(&id)
    }

    /// Returns the base relation record for `id`, if present.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    pub(super) fn relation(&self, id: RelationId) -> Option<&RelationRecord> {
        self.relations.get(&id)
    }

    /// Returns the base incidence record for `id`, if present.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    pub(super) fn incidence(&self, id: IncidenceId) -> Option<&IncidenceRecord> {
        self.incidences.get(&id)
    }

    /// Returns the base property value for `(subject, key)`, if present.
    ///
    /// # Performance
    ///
    /// This method is `O(log n)`.
    pub(super) fn property(
        &self,
        subject: PropertySubject,
        key: PropertyKeyId,
    ) -> Option<&PropertyValue> {
        self.properties
            .get(&subject)
            .and_then(|keys| keys.get(&key))
    }
}

/// Decodes a borrowed label run into an owned label set.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when the run slice is out of bounds.
///
/// # Performance
///
/// This function is `O(labels)`.
fn decode_labels(
    run: Option<&[zerocopy::byteorder::U64<zerocopy::byteorder::LE>]>,
) -> Result<LabelSet, DbError> {
    let run = run.ok_or_else(|| DbError::invalid_store("base label run out of bounds"))?;
    Ok(run.iter().map(|word| LabelId::new(word.get())).collect())
}

/// Decodes the base property records into an owned subject/key map.
///
/// # Errors
///
/// Returns [`DbError::InvalidStore`] when a record's subject kind, value tag, or
/// text slice cannot be resolved.
///
/// # Performance
///
/// This function is `O(properties + text bytes)`.
fn decode_base_properties(
    view: &BaseView<'_>,
) -> Result<BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>>, DbError> {
    let mut properties: BTreeMap<PropertySubject, BTreeMap<PropertyKeyId, PropertyValue>> =
        BTreeMap::new();
    for record in view.properties() {
        let subject =
            crate::wire::decode_subject(record.subject_kind.get(), record.subject_id.get())
                .ok_or_else(|| DbError::invalid_store("base property subject kind out of range"))?;
        let value_type = crate::wire::property_type_from_tag(record.value_tag.get())
            .ok_or_else(|| DbError::invalid_store("base property value tag out of range"))?;
        let value = match value_type {
            PropertyType::Boolean => PropertyValue::Boolean(record.scalar.get() != 0),
            PropertyType::Integer => PropertyValue::Integer(record.scalar.get().cast_signed()),
            PropertyType::Text => {
                let bytes = view
                    .property_text(record)
                    .ok_or_else(|| DbError::invalid_store("base property text out of bounds"))?;
                let text = core::str::from_utf8(bytes)
                    .map_err(|_error| DbError::invalid_store("base property text is not UTF-8"))?;
                PropertyValue::Text(Arc::from(text))
            }
        };
        properties
            .entry(subject)
            .or_default()
            .insert(PropertyKeyId::new(record.key.get()), value);
    }
    Ok(properties)
}

/// The immutable unit a read transaction pins: one base generation plus the
/// frozen overlay published over it, tagged by `(generation, lsn)` identity.
///
/// `reader` clones the database's current `Arc<Snapshot>`, so a reader holds
/// the whole triple by `Arc` and observes a fixed state even across later
/// commits/checkpoints. It is [`Send`] + [`Sync`] (asserted below)
/// because [`Base`] is `Send + Sync`, [`Overlay`] holds owned `Send + Sync`
/// value types, and the identity fields are `Copy`.
///
/// # Performance
///
/// Cloning a [`Snapshot`] is `O(1)` (it clones two `Arc`s and copies the
/// identity fields).
#[derive(Clone)]
pub(crate) struct Snapshot {
    /// Checkpoint generation of the pinned base.
    generation: CheckpointGeneration,
    /// Committed transaction sequence (the live frontier) of this snapshot.
    lsn: CommitSeq,
    /// The pinned, immutable base generation.
    base: Arc<Base>,
    /// The frozen overlay published over `base`.
    overlay: Arc<Overlay>,
    /// Owned realization of the base records the merge borrows from.
    base_records: Arc<BaseRecords>,
}

impl Snapshot {
    /// Publishes a new snapshot over the SAME base generation as a parent,
    /// SHARING the parent's already-materialized [`BaseRecords`] (and its derived
    /// [`crate::index::BaseIndex`]) by `Arc` instead of re-decoding the base wire
    /// bytes and rebuilding the index.
    ///
    /// This is the commit publish path: a commit never folds, so its base bytes
    /// are byte-identical to the parent's within the generation, and the records +
    /// index derived from them are identical too. Sharing them makes a commit
    /// `O(change)` (frame encode + overlay freeze) rather than `O(base)` — the
    /// caller MUST pass `base` and `base_records` drawn from the same parent
    /// snapshot so the shared records match the pinned base.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`: it clones two `Arc`s and copies the identity
    /// fields, with no base decode or index rebuild.
    pub(crate) const fn with_shared_base_records(
        generation: CheckpointGeneration,
        lsn: CommitSeq,
        base: Arc<Base>,
        overlay: Arc<Overlay>,
        base_records: Arc<BaseRecords>,
    ) -> Self {
        Self {
            generation,
            lsn,
            base,
            overlay,
            base_records,
        }
    }

    /// Returns this snapshot's checkpoint generation.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn generation(&self) -> CheckpointGeneration {
        self.generation
    }

    /// Returns this snapshot's committed transaction sequence (the frontier).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn lsn(&self) -> CommitSeq {
        self.lsn
    }

    /// Returns the frozen overlay published over this snapshot's base.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn overlay(&self) -> &Arc<Overlay> {
        &self.overlay
    }

    /// Returns the pinned base generation.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn base(&self) -> &Arc<Base> {
        &self.base
    }

    /// Returns the owned base records this snapshot's merge borrows from. The
    /// commit path layers a fresh [`WriteOverlay`] over these for write reads.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn base_records(&self) -> &Arc<BaseRecords> {
        &self.base_records
    }

    /// Returns a borrowed merged read view over this snapshot: overlay-then-base
    /// with tombstones masked.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to construct; reads through it carry their own
    /// contracts.
    pub(crate) fn view(&self) -> MergedState<'_> {
        MergedState {
            base: &self.base_records,
            overlay: &self.overlay,
        }
    }
}

/// Asserts a type is `Send + Sync` at compile time.
///
/// # Performance
///
/// `perf: unspecified`; compile-time only.
const fn assert_send_sync<T: Send + Sync>() {}

/// `Snapshot` MUST be `Send + Sync`: a `Reader` pinning it crosses
/// threads, and the snapshot owns only `Arc`-shared `Send + Sync` data plus
/// `Copy` identity fields.
const _: () = assert_send_sync::<Snapshot>();
/// `Overlay` MUST be `Send + Sync`: it is shared via `Arc<Overlay>` across the
/// snapshots a reader pins.
const _: () = assert_send_sync::<Overlay>();

/// Bounded, base-free constructors the kani proofs use to build small overlay
/// inputs directly (a frozen [`Base`] needs the container/snapshot machinery
/// kani cannot evaluate, so the proofs build [`BaseRecords`] and [`Overlay`]
/// element layers in memory and merge them).
///
/// # Performance
///
/// `perf: unspecified`; these are bounded proof helpers compiled only under
/// `cfg(kani)`.
#[cfg(kani)]
impl BaseRecords {
    /// Builds base records holding exactly the elements with the given ids (no
    /// labels), and no relations/incidences/properties.
    ///
    /// # Performance
    ///
    /// This function is `O(ids)`.
    pub(crate) fn proof_elements(ids: &[ElementId]) -> Self {
        let mut elements = BTreeMap::new();
        for id in ids.iter().copied() {
            elements.insert(
                id,
                ElementRecord {
                    id,
                    labels: LabelSet::default(),
                },
            );
        }
        Self {
            elements,
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            index: HeldIndex::Owned(OwnedBaseIndex::empty()),
        }
    }
}

// Support for the CBMC-heavy overlay-merge proofs only (gated identically to
// them; see `kani-heavy` in Cargo.toml).
#[cfg(all(kani, feature = "kani-heavy"))]
impl Overlay {
    /// Builds an overlay whose element layer holds the given `(id, present)`
    /// entries: `present` records an add/override (with no labels), `!present`
    /// records a tombstone. The watermark and catalog are empty.
    ///
    /// # Performance
    ///
    /// This function is `O(entries)`.
    pub(crate) fn proof_element_entries(entries: &[(ElementId, bool)]) -> Self {
        let mut elements = BTreeMap::new();
        for (id, present) in entries.iter().copied() {
            let entry = present.then(|| ElementRecord {
                id,
                labels: LabelSet::default(),
            });
            elements.insert(id, entry);
        }
        Self {
            elements,
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            catalog: Catalog::empty(),
            next: proof_zero_next_ids(),
            index: OverlayIndex::new(),
        }
    }

    /// Returns whether this overlay tombstones `id` at the element layer.
    ///
    /// # Performance
    ///
    /// This function is `O(log change)`.
    pub(crate) fn proof_is_element_tombstoned(&self, id: ElementId) -> bool {
        matches!(self.elements.get(&id), Some(None))
    }

    /// Builds an empty overlay whose watermark's element allocator is `next`
    /// (every other allocator starts at one). Used by the watermark proof to set
    /// a parent's element allocator without driving it through allocations.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) fn proof_with_next_element(next: u64) -> Self {
        let mut watermark = proof_zero_next_ids();
        watermark.element = ElementId::new(next);
        Self {
            elements: BTreeMap::new(),
            relations: BTreeMap::new(),
            incidences: BTreeMap::new(),
            properties: BTreeMap::new(),
            catalog: Catalog::empty(),
            next: watermark,
            index: OverlayIndex::new(),
        }
    }
}

/// The all-ones watermark used to seed proof overlays (the actual values are
/// irrelevant to the element-merge proofs).
///
/// # Performance
///
/// This function is `O(1)`.
#[cfg(all(kani, feature = "kani-heavy"))]
fn proof_zero_next_ids() -> NextIds {
    NextIds {
        element: ElementId::new(1),
        relation: RelationId::new(1),
        incidence: IncidenceId::new(1),
        role: RoleId::new(1),
        label: LabelId::new(1),
        relation_type: RelationTypeId::new(1),
        property_key: PropertyKeyId::new(1),
        projection: ProjectionId::new(1),
        index: IndexId::new(1),
    }
}
