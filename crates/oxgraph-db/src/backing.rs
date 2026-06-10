//! Zero-copy base-snapshot backing and the borrowing base-attach layer.
//!
//! A frozen base file (the immutable per-generation OXGTOPO container produced by
//! [`crate::freeze::freeze_view`]) is read by BORROWING its fixed record arrays
//! directly out of a byte buffer, never decoding them into owned collections. The
//! buffer is an owned [`Vec<u8>`] read fully from the file; it exposes `&[u8]`
//! through the [`Backing`] [`Deref`], so the borrow code below is identical on
//! every target and miri exercises it directly.
//!
//! On unix (outside miri) the base is memory-mapped read-only through the
//! [`oxgraph_mmap`] shim crate, which owns the single audited `unsafe` mmap call
//! so this crate keeps `unsafe_code = "forbid"`. Under miri, on non-unix
//! targets, or when the caller forces it, the base is read fully into an owned
//! `Vec<u8>` instead. Both variants expose `&[u8]` through the [`Backing`]
//! [`Deref`], so the borrow code below is identical over either, and miri
//! exercises the owned arm.
//!
//! Borrowing is achieved exclusively through [`yoke::Yoke`]: a [`BaseCart`] owns
//! the backing bytes, and [`Base::open`] attaches a [`BaseView`] of borrowed
//! slices to that cart inside [`Yoke::try_attach_to_cart`], mirroring
//! `oxgraph-postgres` (engine.rs / builder.rs / topology.rs). The foreign
//! [`oxgraph_snapshot::Snapshot`] is opened only inside the attach closure to
//! extract the typed slices and is never stored — it is not [`yoke::Yokeable`].
//! No raw-pointer reinterpretation is used, so `unsafe_code = forbid` holds.
//!
//! Integrity verification is FUSED into the bind pass. [`Base::open`] first
//! verifies the container's `table_crc32c` (via
//! [`oxgraph_snapshot::Snapshot::open_checked`]); then every section the
//! attach binds has its payload CRC-32C verified exactly once, inside the
//! bind funnels ([`freeze::typed_records`], [`freeze::raw_blob`], and
//! [`posting_slices`] below) as the section is borrowed. There is no separate
//! whole-base CRC scan: one pass over the bound payload bytes covers the
//! store, and a truncated or corrupted base surfaces as a clean
//! [`StorageError`] naming the failing section at open.
//!
//! # Performance
//!
//! `perf: unspecified`; this module defines the backing primitive and the
//! open-time bind pass (`O(base bytes)` total — each bound section is
//! checksum-folded once as it is borrowed). The [`BaseView`] accessors are
//! `O(log n)` binary searches over the canonically sorted record arrays.

use std::{fs::File, io::Read, ops::Deref, path::Path};

use oxgraph_snapshot::Snapshot;
use yoke::Yoke;
use zerocopy::{
    FromBytes,
    byteorder::{LE, U64},
};

use crate::{Catalog, StorageError, crc, freeze, index::BorrowedBaseIndex, wire};

/// Immutable backing bytes for one base file: a read-only memory map or a fully
/// owned vector. Both expose `&[u8]` through [`Deref`], so the borrow path in
/// [`Base::open`] is byte-for-byte identical over either variant.
///
/// On unix outside miri the base is mapped through the [`oxgraph_mmap`] shim (the
/// one audited `unsafe` island), so this crate stays `unsafe_code = forbid`.
/// Under miri, on non-unix targets, or when the caller forces it, the base is
/// read into an owned vector, so miri always exercises the [`Backing::Owned`]
/// arm.
///
/// # Performance
///
/// [`Deref`] is `O(1)`. Constructing [`Backing::Mmap`] is an `O(1)` syscall
/// (pages fault in lazily); constructing [`Backing::Owned`] is `O(base bytes)`.
pub(crate) enum Backing {
    /// Read-only memory map of the base file (unix, outside miri, default).
    #[cfg(all(unix, not(miri)))]
    Mmap(oxgraph_mmap::Mmap),
    /// Fully owned base bytes (miri, non-unix, or caller-forced).
    Owned(Vec<u8>),
}

impl Backing {
    /// Wraps already-owned base `bytes` as [`Backing::Owned`].
    ///
    /// # Performance
    ///
    /// This function is `O(1)`; it moves the vector in.
    const fn owned(bytes: Vec<u8>) -> Self {
        Self::Owned(bytes)
    }
}

impl Deref for Backing {
    type Target = [u8];

    /// Borrows the backing bytes as a slice, regardless of variant.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn deref(&self) -> &[u8] {
        match self {
            #[cfg(all(unix, not(miri)))]
            Self::Mmap(map) => &map[..],
            Self::Owned(bytes) => bytes.as_slice(),
        }
    }
}

/// Opens the base file at `path` into a [`Backing`].
///
/// On unix outside miri (and unless `force_owned`), the file is mapped read-only
/// through the [`oxgraph_mmap`] shim; otherwise it is read fully into an owned
/// vector. `force_owned` lets a caller opt out of mmap (e.g. for a test or a
/// filesystem where mapping is undesirable).
///
/// # Errors
///
/// Returns [`StorageError::NotFound`] when the file is absent and [`StorageError::Io`] for
/// any other open, map, or read failure.
///
/// # Performance
///
/// `O(1)` for the mmap path (one syscall, lazy faults); `O(base bytes)` for the
/// owned read.
pub(crate) fn open_backing(path: &Path, force_owned: bool) -> Result<Backing, StorageError> {
    let file = File::open(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => StorageError::NotFound,
        _kind => StorageError::io("open base file", error),
    })?;
    map_or_read(file, force_owned)
}

/// Maps the base read-only via the [`oxgraph_mmap`] shim, falling back to an
/// owned read when `force_owned` is set.
///
/// # Errors
///
/// Returns [`StorageError::Io`] when the map or read fails.
///
/// # Performance
///
/// `O(1)` for the mmap path; `O(base bytes)` when `force_owned`.
#[cfg(all(unix, not(miri)))]
fn map_or_read(file: File, force_owned: bool) -> Result<Backing, StorageError> {
    if force_owned {
        return read_owned(file);
    }
    let map = oxgraph_mmap::map_read_only(&file)
        .map_err(|error| StorageError::io("mmap base file", error))?;
    Ok(Backing::Mmap(map))
}

/// Reads the base into an owned [`Backing::Owned`]; the only path on miri and
/// non-unix targets, where mmap is unavailable.
///
/// # Errors
///
/// Returns [`StorageError::Io`] when the read fails.
///
/// # Performance
///
/// This function is `O(base bytes)`.
#[cfg(not(all(unix, not(miri))))]
fn map_or_read(file: File, force_owned: bool) -> Result<Backing, StorageError> {
    // No mmap on this target; the backing is always owned, so `force_owned` is
    // already satisfied.
    let _ = force_owned;
    read_owned(file)
}

/// Reads the whole base file into an owned [`Backing::Owned`].
///
/// # Errors
///
/// Returns [`StorageError::Io`] when the read fails.
///
/// # Performance
///
/// This function is `O(base bytes)`.
fn read_owned(mut file: File) -> Result<Backing, StorageError> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| StorageError::io("read base file", error))?;
    Ok(Backing::owned(bytes))
}

/// Owned, fixed-size header value extracted from the base's
/// [`wire::DbHeaderRecord`].
///
/// This is a small `Copy` value the [`BaseView`] owns (it is not borrowed from
/// the backing). It mirrors the durable stamps and the nine id allocators; per
/// the reconciled design those values are a checkpoint snapshot of the folded
/// state, not the live frontier.
///
/// # Performance
///
/// Copying is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DbHeader {
    /// OXGDB format version recorded in the base header.
    pub(crate) format_version: u32,
    /// Checkpoint-time committed transaction sequence.
    pub(crate) commit_seq: u64,
    /// Checkpoint-time writer transaction id.
    pub(crate) transaction_id: u64,
    /// Checkpoint/root generation stamp folded into this base.
    pub(crate) checkpoint_generation: u64,
    /// Next element id candidate snapshot.
    pub(crate) next_element: u64,
    /// Next relation id candidate snapshot.
    pub(crate) next_relation: u64,
    /// Next incidence id candidate snapshot.
    pub(crate) next_incidence: u64,
    /// Next role id candidate snapshot.
    pub(crate) next_role: u64,
    /// Next label id candidate snapshot.
    pub(crate) next_label: u64,
    /// Next relation-type id candidate snapshot.
    pub(crate) next_relation_type: u64,
    /// Next property-key id candidate snapshot.
    pub(crate) next_property_key: u64,
    /// Next projection id candidate snapshot.
    pub(crate) next_projection: u64,
    /// Next index id candidate snapshot.
    pub(crate) next_index: u64,
}

impl DbHeader {
    /// Builds an owned header from a borrowed [`wire::DbHeaderRecord`].
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    const fn from_record(record: &wire::DbHeaderRecord) -> Self {
        Self {
            format_version: record.format_version.get(),
            commit_seq: record.commit_seq.get(),
            transaction_id: record.transaction_id.get(),
            checkpoint_generation: record.checkpoint_generation.get(),
            next_element: record.next_element.get(),
            next_relation: record.next_relation.get(),
            next_incidence: record.next_incidence.get(),
            next_role: record.next_role.get(),
            next_label: record.next_label.get(),
            next_relation_type: record.next_relation_type.get(),
            next_property_key: record.next_property_key.get(),
            next_projection: record.next_projection.get(),
            next_index: record.next_index.get(),
        }
    }
}

/// The Yoke cart owning the backing bytes a [`BaseView`] borrows from.
///
/// # Performance
///
/// Construction is `O(1)`; it moves the backing in.
pub(crate) struct BaseCart {
    /// Immutable base bytes the view's slices borrow from.
    pub(crate) bytes: Backing,
}

/// Borrowed zero-copy view over one base generation.
///
/// Every bulk array is a borrowed `zerocopy` slice into the [`BaseCart`] backing;
/// only the [`Catalog`] and the [`DbHeader`] are owned, because they are small
/// and need name maps the wire form does not carry. [`oxgraph_snapshot::Snapshot`]
/// is not stored here — it is opened only inside the attach closure to extract
/// these slices (see [`Base::open`]). The record arrays are canonically sorted
/// ascending by id, so the accessors below binary-search them.
///
/// # Performance
///
/// Holding the view is `O(1)`; accessors are `O(log n)` (binary search) or
/// `O(1)` (slice/iterator borrows).
//
// `prove_covariant` makes the covariance contract explicit and self-documenting,
// matching the in-repo `oxgraph-postgres` precedent (`EngineState`,
// `GraphTopology`). Every lifetime-bearing field here is a shared `&'a [T]`, so
// covariance holds; the attribute turns a future variance regression into an
// obvious compile error at this site rather than a confusing derive error.
#[derive(yoke::Yokeable)]
#[yoke(prove_covariant)]
pub(crate) struct BaseView<'a> {
    /// Element records sorted ascending by element id.
    elements: &'a [wire::ElementWire],
    /// Relation records sorted ascending by relation id.
    relations: &'a [wire::RelationWire],
    /// Incidence records sorted ascending by incidence id.
    incidences: &'a [wire::IncidenceWire],
    /// Flat element-label id run sliced by each [`wire::ElementWire`].
    element_labels: &'a [U64<LE>],
    /// Flat relation-label id run sliced by each [`wire::RelationWire`].
    relation_labels: &'a [U64<LE>],
    /// Typed property records sorted by `(subject_kind, subject_id, key)`.
    properties: &'a [wire::PropertyWire],
    /// Concatenated property text values sliced by each [`wire::PropertyWire`].
    property_text: &'a [u8],
    /// Derived index postings borrowed zero-copy out of the base's persisted
    /// `SECTION_INDEX_*` sections, so open never rebuilds them from records.
    index: BorrowedBaseIndex<'a>,
    /// Owned catalog rebuilt from the catalog record sections.
    catalog: Catalog,
    /// Owned header value extracted from the base header section.
    header: DbHeader,
}

impl<'a> BaseView<'a> {
    /// Returns the borrowed label id run for an element record.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to borrow; the returned slice has `label_len`
    /// entries.
    #[must_use]
    pub(crate) fn element_label_run(&self, record: &wire::ElementWire) -> Option<&'a [U64<LE>]> {
        label_slice(
            self.element_labels,
            record.label_off.get(),
            record.label_len.get(),
        )
    }

    /// Returns the borrowed label id run for a relation record.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to borrow; the returned slice has `label_len`
    /// entries.
    #[must_use]
    pub(crate) fn relation_label_run(&self, record: &wire::RelationWire) -> Option<&'a [U64<LE>]> {
        label_slice(
            self.relation_labels,
            record.label_off.get(),
            record.label_len.get(),
        )
    }

    /// Returns the borrowed UTF-8 text value for a text property record.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to borrow; validating UTF-8 is `O(text_len)`.
    #[must_use]
    pub(crate) fn property_text(&self, record: &wire::PropertyWire) -> Option<&'a [u8]> {
        let start = record.text_off.get() as usize;
        let end = start.checked_add(record.text_len.get() as usize)?;
        self.property_text.get(start..end)
    }

    /// Iterates every borrowed element record in canonical id order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`; a full walk is `O(n)`.
    pub(crate) fn elements(&self) -> impl Iterator<Item = &'a wire::ElementWire> {
        self.elements.iter()
    }

    /// Iterates every borrowed relation record in canonical id order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`; a full walk is `O(n)`.
    pub(crate) fn relations(&self) -> impl Iterator<Item = &'a wire::RelationWire> {
        self.relations.iter()
    }

    /// Iterates every borrowed incidence record in canonical id order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`; a full walk is `O(n)`.
    pub(crate) fn incidences(&self) -> impl Iterator<Item = &'a wire::IncidenceWire> {
        self.incidences.iter()
    }

    /// Iterates every borrowed property record in canonical order.
    ///
    /// # Performance
    ///
    /// Creating the iterator is `O(1)`; a full walk is `O(p)`.
    pub(crate) fn properties(&self) -> impl Iterator<Item = &'a wire::PropertyWire> {
        self.properties.iter()
    }

    /// Returns the derived index postings borrowed zero-copy from this base's
    /// persisted sections. The returned [`BorrowedBaseIndex`] borrows from the
    /// same backing as this view (lifetime `'a`), so it lives as long as the
    /// attached [`Base`].
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(crate) const fn index(&self) -> BorrowedBaseIndex<'a> {
        self.index
    }

    /// Borrows the owned catalog.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(crate) const fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Returns the owned header value.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(crate) const fn header(&self) -> &DbHeader {
        &self.header
    }
}

/// Slices a `(offset, len)` label run, returning `None` when out of bounds.
///
/// # Performance
///
/// This function is `O(1)`.
fn label_slice(run: &[U64<LE>], offset: u32, len: u32) -> Option<&[U64<LE>]> {
    let start = offset as usize;
    let end = start.checked_add(len as usize)?;
    run.get(start..end)
}

/// One attached base generation: a [`BaseView`] of borrowed slices over the
/// backing bytes owned by a [`BaseCart`].
///
/// The lifetime is erased to `'static` by [`Yoke`]; the view's borrows stay
/// valid because the cart it borrows from is co-owned and never moves out. The
/// type is [`Send`] and [`Sync`] (asserted in tests): the backing is `Send +
/// Sync`, the borrowed slices are over `Sync` `zerocopy` data, and the owned
/// [`Catalog`]/[`DbHeader`] are `Send + Sync`.
///
/// # Performance
///
/// Construction is `O(base bytes)` (CRC verify + slice extraction); thereafter
/// [`Self::get`] is `O(1)`.
pub(crate) struct Base {
    /// The view borrowed from its co-owned backing cart.
    yoke: Yoke<BaseView<'static>, Box<BaseCart>>,
}

impl Base {
    /// Opens a base file, verifying integrity at bind: the container's table
    /// checksum first, then every bound section's payload CRC as it is
    /// borrowed.
    ///
    /// The [`BaseView`] is attached inside [`Yoke::try_attach_to_cart`]:
    /// [`oxgraph_snapshot::Snapshot`] is opened (checked) inside the closure
    /// to extract the typed slices and build the owned catalog/header, and is
    /// dropped there. Each section's payload CRC is verified exactly once by
    /// the bind funnels; a mismatch is rejected as
    /// [`StorageError::InvalidStore`] naming the failing section kind.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::NotFound`] when the file is absent, [`StorageError::Io`] on
    /// an IO failure, and [`StorageError::InvalidStore`] when the base is malformed,
    /// the table checksum fails, or a bound section's payload CRC does not match.
    ///
    /// # Performance
    ///
    /// This function is `O(base bytes)`: one fused pass that checksums each
    /// bound section as it is borrowed.
    pub(crate) fn open(path: &Path, force_owned: bool) -> Result<Self, StorageError> {
        let backing = open_backing(path, force_owned)?;
        Self::attach(backing)
    }

    /// Attaches a base view directly over already-owned `bytes`, bypassing file
    /// IO. This exercises the exact verify-at-bind + Yoke-attach +
    /// slice-extraction path of [`Self::open`] without touching the filesystem,
    /// so miri (whose isolation blocks `File::open`) can certify the borrow
    /// code over the owned backing.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidStore`] when the bytes are malformed, the
    /// table checksum fails, or a bound section's payload CRC does not match.
    ///
    /// # Performance
    ///
    /// This function is `O(base bytes)`.
    #[cfg(test)]
    pub(crate) fn open_owned_bytes(bytes: Vec<u8>) -> Result<Self, StorageError> {
        Self::attach(Backing::owned(bytes))
    }

    /// Attaches a borrowing view over `backing`, verifying integrity at bind
    /// (table checksum via [`Snapshot::open_checked`], then each bound
    /// section's payload CRC inside the bind funnels).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidStore`] when checksum verification or
    /// section extraction fails.
    ///
    /// # Performance
    ///
    /// This function is `O(base bytes)`.
    pub(crate) fn attach(backing: Backing) -> Result<Self, StorageError> {
        let cart = Box::new(BaseCart { bytes: backing });
        let yoke = Yoke::try_attach_to_cart(cart, |cart: &BaseCart| attach_view(&cart.bytes))?;
        Ok(Self { yoke })
    }

    /// Borrows the attached base view.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub(crate) fn get(&self) -> &BaseView<'_> {
        self.yoke.get()
    }
}

/// Opens the snapshot over `bytes` (checked: the v2 `table_crc32c` is
/// verified) and extracts the borrowed [`BaseView`], verifying each bound
/// section's payload CRC exactly once via the bind funnels.
///
/// Called only inside [`Yoke::try_attach_to_cart`]; the [`oxgraph_snapshot::Snapshot`]
/// it opens is dropped at the end of this function and is never stored. This is
/// the ONLY place sections are bound, so the funnel verification here is the
/// single integrity pass per open — post-open reads use the borrowed slices
/// the view holds and never re-verify.
///
/// # Errors
///
/// Returns [`StorageError::InvalidStore`] when the bytes are malformed, the table
/// checksum fails, a section's payload CRC does not match (the message names
/// the failing kind), a section is not borrowable as its typed slice, the
/// header is missing, the format version is unsupported, or the property array
/// is not sorted by `(subject_kind, subject_id, key)` (the canonical order
/// [`crate::wire::encode_subject`] produces and [`crate::overlay::BaseRecords`]
/// materializes in).
///
/// # Performance
///
/// This function is `O(base bytes)`: each bound section is checksum-folded
/// once as it is borrowed, the catalog decode is `O(catalog + name bytes)`,
/// and the property-sort check is one `O(properties)` scan.
fn attach_view(bytes: &[u8]) -> Result<BaseView<'_>, StorageError> {
    let snapshot = Snapshot::open_checked(bytes, crc::checksum_append)
        .map_err(|error| StorageError::invalid_store(error.to_string()))?;

    let headers =
        freeze::typed_records::<wire::DbHeaderRecord>(&snapshot, wire::SECTION_DB_HEADER)?;
    let header_record = headers
        .first()
        .ok_or_else(|| StorageError::invalid_store("base is missing the header section"))?;
    if header_record.format_version.get() != wire::OXGDB_FORMAT_VERSION {
        return Err(StorageError::UnsupportedFormat {
            found: header_record.format_version.get(),
            expected: wire::OXGDB_FORMAT_VERSION,
        });
    }
    let header = DbHeader::from_record(header_record);

    let string_table = freeze::raw_blob(&snapshot, wire::SECTION_STRING_TABLE)?;
    let defs = freeze::typed_records::<U64<LE>>(&snapshot, wire::SECTION_CATALOG_DEFS)?;
    let catalog = freeze::decode_catalog(&snapshot, string_table, defs)
        .map_err(|error| StorageError::invalid_store(error.to_string()))?;

    let properties =
        freeze::typed_records::<wire::PropertyWire>(&snapshot, wire::SECTION_PROPERTY_RECORDS)?;
    verify_properties_sorted(properties)?;

    let index = attach_index(&snapshot)?;

    Ok(BaseView {
        elements: freeze::typed_records::<wire::ElementWire>(
            &snapshot,
            wire::SECTION_ELEMENT_RECORDS,
        )?,
        relations: freeze::typed_records::<wire::RelationWire>(
            &snapshot,
            wire::SECTION_RELATION_RECORDS,
        )?,
        incidences: freeze::typed_records::<wire::IncidenceWire>(
            &snapshot,
            wire::SECTION_INCIDENCE_RECORDS,
        )?,
        element_labels: freeze::typed_records::<U64<LE>>(&snapshot, wire::SECTION_ELEMENT_LABELS)?,
        relation_labels: freeze::typed_records::<U64<LE>>(
            &snapshot,
            wire::SECTION_RELATION_LABELS,
        )?,
        properties,
        property_text: freeze::raw_blob(&snapshot, wire::SECTION_PROPERTY_TEXT)?,
        index,
        catalog,
        header,
    })
}

/// Borrows the derived [`BorrowedBaseIndex`] out of the base's five persisted
/// posting sections, splitting each framed section into its directory and value
/// pool and reinterpreting them as typed slices.
///
/// # Errors
///
/// Returns [`StorageError::InvalidStore`] when a posting section's payload CRC does
/// not match, its frame is malformed, a directory or pool cannot be
/// reinterpreted as its typed slice, or a directory entry slices outside its
/// pool.
///
/// # Performance
///
/// This function is `O(posting payload bytes + directory entries)`: each
/// posting section is checksum-folded once at bind, then bounds-validated; the
/// borrows themselves are `O(1)`.
fn attach_index<'a>(snapshot: &Snapshot<'a>) -> Result<BorrowedBaseIndex<'a>, StorageError> {
    let (label_dir, label_pool) = posting_slices(snapshot, wire::SECTION_INDEX_LABEL_POSTINGS)?;
    let (relation_type_dir, relation_type_pool) =
        posting_slices(snapshot, wire::SECTION_INDEX_RELATION_TYPE_POSTINGS)?;
    let (element_incidence_dir, element_incidence_pool) =
        posting_slices(snapshot, wire::SECTION_INDEX_ELEMENT_INCIDENCES)?;
    let (relation_incidence_dir, relation_incidence_pool) =
        posting_slices(snapshot, wire::SECTION_INDEX_RELATION_INCIDENCES)?;
    let (equality_dir, equality_pool) = posting_slices(snapshot, wire::SECTION_INDEX_EQUALITY)?;
    let equality_text = freeze::raw_blob(snapshot, wire::SECTION_INDEX_EQUALITY_TEXT)?;
    BorrowedBaseIndex::from_sections(
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
    )
    .map_err(|error| StorageError::invalid_store(error.to_string()))
}

/// Splits one framed posting section into its directory `[T]` slice and its value
/// pool `[U64<LE>]` slice, or two empty slices when the section is absent (the
/// store omits empty posting maps).
///
/// This is a bind funnel like [`freeze::typed_records`]: it verifies the
/// section's payload CRC-32C before splitting, and it is called only from
/// [`attach_index`] — once per posting section per open.
///
/// # Errors
///
/// Returns [`StorageError::InvalidStore`] when the payload checksum does not match
/// the section entry (the message names the failing kind), the frame prefix is
/// malformed, or a region cannot be reinterpreted as its typed slice.
///
/// # Performance
///
/// This function is `O(payload bytes)`: one checksum fold, then an `O(1)`
/// split.
#[expect(
    clippy::type_complexity,
    reason = "the directory `[T]` and value-pool `[U64<LE>]` slices are returned together as one framed section's two regions"
)]
fn posting_slices<'a, T>(
    snapshot: &Snapshot<'a>,
    kind: u32,
) -> Result<(&'a [T], &'a [U64<LE>]), StorageError>
where
    T: FromBytes + zerocopy::Immutable + zerocopy::KnownLayout,
{
    let Some(section) = snapshot.section(kind) else {
        return Ok((&[], &[]));
    };
    section
        .verify(crc::checksum_append)
        .map_err(|error| StorageError::invalid_store(error.to_string()))?;
    let (dir_bytes, pool_bytes) = freeze::split_posting_section(section.bytes())?;
    let dir = <[T]>::ref_from_bytes(dir_bytes)
        .map_err(|_error| StorageError::invalid_store("posting directory is not a whole array"))?;
    let pool = <[U64<LE>]>::ref_from_bytes(pool_bytes)
        .map_err(|_error| StorageError::invalid_store("posting value pool is not a whole array"))?;
    Ok((dir, pool))
}

/// Returns the canonical sort key for a property record: `(subject_kind,
/// subject_id, key)`. This is the order [`crate::wire::encode_subject`]
/// contractually produces and that [`verify_properties_sorted`] enforces at open.
///
/// # Performance
///
/// This function is `O(1)`.
const fn property_sort_key(record: &wire::PropertyWire) -> (u32, u64, u64) {
    (
        record.subject_kind.get(),
        record.subject_id.get(),
        record.key.get(),
    )
}

/// Verifies that `properties` is sorted ascending by `(subject_kind,
/// subject_id, key)`.
///
/// [`crate::overlay::BaseRecords`] materializes the base properties in this
/// triple order, and the write order is implicitly tied to the
/// [`crate::PropertySubject`] variant order matching the
/// [`crate::wire::encode_subject`] kind tags (see those docs); this open-time
/// scan turns any future desynchronization into a loud [`StorageError`] at open
/// rather than a silent missed record on the read path.
///
/// # Errors
///
/// Returns [`StorageError::InvalidStore`] when two adjacent records are out of order.
///
/// # Performance
///
/// This function is `O(properties)`: one linear adjacency scan.
fn verify_properties_sorted(properties: &[wire::PropertyWire]) -> Result<(), StorageError> {
    let ordered = properties
        .windows(2)
        .all(|pair| property_sort_key(&pair[0]) <= property_sort_key(&pair[1]));
    if ordered {
        Ok(())
    } else {
        Err(StorageError::invalid_store(
            "base property records are not sorted by (subject_kind, subject_id, key)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::test_support::small_base;

    /// `Base` must be `Send + Sync` so a `Reader` pinning it can cross
    /// threads in the MVCC layer.
    const fn assert_send_sync<T: Send + Sync>() {}
    const _: () = assert_send_sync::<Base>();

    /// Asserts the borrowed view of `base` matches the canonical small fixture:
    /// three elements all labelled "Person", two "calls" relations, two
    /// incidences, and one text property.
    fn assert_small_reads(base: &Base) {
        let view = base.get();
        assert_eq!(view.header().format_version, wire::OXGDB_FORMAT_VERSION);
        assert_eq!(view.elements().count(), 3);
        assert_eq!(view.relations().count(), 2);
        assert_eq!(view.incidences().count(), 2);

        // Elements 1 and 2 carry the "Person" label run; element 3 has none.
        let person = view.catalog().label_id("Person").expect("Person label");
        let labelled = view
            .elements()
            .filter(|record| {
                view.element_label_run(record)
                    .is_some_and(|labels| labels.iter().any(|word| word.get() == person.get()))
            })
            .count();
        assert_eq!(labelled, 2, "two elements carry the Person label");

        // The text property on element 1 borrows "Alice".
        let alice = view
            .properties()
            .find(|record| view.property_text(record).is_some())
            .expect("a text property");
        assert_eq!(view.property_text(alice).expect("text"), b"Alice");

        // The catalog round-tripped (owned) and the property array is sorted.
        assert!(view.catalog().property_key_id("name").is_some());
    }

    /// The owned backing borrows the canonical reads from an in-memory buffer.
    /// This drives the verify-at-bind Yoke attach + every surviving
    /// [`BaseView`] accessor with NO filesystem IO, so it is the path miri
    /// exercises (miri isolation blocks `File::open`).
    #[test]
    fn owned_backing_borrows_canonical_reads() {
        assert_small_reads(&small_base());
    }

    /// `verify_properties_sorted` accepts an ascending array and rejects any
    /// out-of-order adjacency, in each of the three sort dimensions. This is the
    /// open-time guard that converts a `PropertySubject`/`encode_subject` desync
    /// from a silent missed read into a loud `StorageError`.
    #[test]
    fn property_sort_check_accepts_sorted_rejects_unsorted() {
        use zerocopy::byteorder::{U32, U64};

        let make = |kind: u32, id: u64, key: u64| wire::PropertyWire {
            subject_kind: U32::new(kind),
            value_tag: U32::new(0),
            subject_id: U64::new(id),
            key: U64::new(key),
            scalar: U64::new(0),
            text_off: U32::new(0),
            text_len: U32::new(0),
        };

        // Sorted by (kind, id, key) — including a higher-kind subject with a
        // lower id, which must still sort after the lower-kind subjects.
        let sorted = [make(0, 1, 1), make(0, 1, 2), make(0, 2, 1), make(2, 1, 1)];
        assert!(verify_properties_sorted(&sorted).is_ok());
        assert!(verify_properties_sorted(&[]).is_ok());
        assert!(verify_properties_sorted(&[make(1, 5, 9)]).is_ok());

        // Out of order on key, on id, and on kind respectively.
        assert!(verify_properties_sorted(&[make(0, 1, 2), make(0, 1, 1)]).is_err());
        assert!(verify_properties_sorted(&[make(0, 2, 1), make(0, 1, 1)]).is_err());
        assert!(verify_properties_sorted(&[make(2, 1, 1), make(0, 1, 1)]).is_err());
    }

    /// A base whose header records an unsupported OXGDB format version is
    /// rejected at open with [`StorageError::UnsupportedFormat`] — the "no legacy
    /// reader, no rebuild fallback" contract for the persisted-index format bump.
    /// The format byte is patched and the header section's entry CRC (plus the
    /// table CRC) re-stamped, so the failure is the version check, not a
    /// checksum mismatch.
    #[test]
    fn unsupported_format_version_is_rejected() {
        use zerocopy::IntoBytes;

        let mut bytes = small_base_bytes();
        // Locate the header section payload via the snapshot.
        let header_offset = {
            let snapshot = Snapshot::open(&bytes).expect("reopen frozen base");
            let header = snapshot
                .section(wire::SECTION_DB_HEADER)
                .expect("header section");
            header.bytes().as_ptr().addr() - bytes.as_ptr().addr()
        };
        // `format_version` is the first `U32<LE>` field of `DbHeaderRecord`; bump
        // it past the supported version.
        let bogus = wire::OXGDB_FORMAT_VERSION + 1;
        let version_field = size_of::<zerocopy::byteorder::U32<LE>>();
        bytes[header_offset..header_offset + version_field]
            .copy_from_slice(zerocopy::byteorder::U32::<LE>::new(bogus).as_bytes());
        // Re-stamp the patched header section's entry CRC (and the table CRC)
        // so the version check (not the checksum) is what rejects the base.
        oxgraph_snapshot::patch_section_crc(
            &mut bytes,
            wire::SECTION_DB_HEADER,
            crate::crc::checksum_append,
        )
        .expect("re-stamp patched header section");

        let result = Base::open_owned_bytes(bytes).map(|_base| ());
        assert!(
            matches!(
                result,
                Err(StorageError::UnsupportedFormat { found, expected })
                    if found == bogus && expected == wire::OXGDB_FORMAT_VERSION
            ),
            "unsupported format must be rejected loudly, got {result:?}",
        );
    }

    /// Flipping ANY payload byte of ANY section makes the attach fail with a
    /// checksum error naming the failing kind — the bind funnels verify every
    /// bound section's CRC at open, so no section payload byte is uncovered.
    /// Driven in memory so it runs under miri.
    #[test]
    fn corrupt_section_payload_fails_attach_naming_kind() {
        let pristine = small_base_bytes();
        // Collect every section's (kind, payload offset, payload length); the
        // snapshot borrow must end before the bytes are mutated.
        let sections: Vec<(u32, usize, usize)> = {
            let snapshot = Snapshot::open(&pristine).expect("reopen frozen base");
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
                assert_checksum_rejects(bytes, kind);
            }
        }
    }

    /// Asserts that attaching `bytes` fails with a payload-checksum error
    /// naming section `kind`.
    fn assert_checksum_rejects(bytes: Vec<u8>, kind: u32) {
        let result = Base::open_owned_bytes(bytes).map(|_base| ());
        let Err(StorageError::InvalidStore { message }) = result else {
            panic!("corrupt section {kind:#06X} must fail attach, got {result:?}");
        };
        assert!(
            message.contains(&format!("section {kind} payload checksum mismatch")),
            "checksum error must name section {kind:#06X}, got: {message}",
        );
    }

    /// Flipping a byte inside the section TABLE fails the attach via the
    /// header's `table_crc32c` ([`Snapshot::open_checked`]) before any section
    /// is bound. The flipped byte is an entry's `version` word, which the
    /// structural open does not interpret, so the table checksum is the check
    /// that rejects it.
    #[test]
    fn corrupt_table_byte_fails_attach() {
        let mut bytes = small_base_bytes();
        // First entry's `version` field: offset (8) + length (8) + kind (4) = 20
        // bytes into the first table entry.
        bytes[oxgraph_snapshot::HEADER_SIZE + 20] ^= 0xFF;
        let result = Base::open_owned_bytes(bytes).map(|_base| ());
        let Err(StorageError::InvalidStore { message }) = result else {
            panic!("corrupt section table must fail attach, got {result:?}");
        };
        assert!(
            message.contains("table checksum mismatch"),
            "table corruption must fail the table checksum, got: {message}",
        );
    }

    /// Freezes the small fixture's bytes for byte-level corruption / file tests,
    /// preserving the base's catalog and watermark so the re-frozen bytes carry
    /// the same canonical state `small_base()` built.
    fn small_base_bytes() -> Vec<u8> {
        use crate::{
            freeze::{FreezeStamps, freeze_view},
            overlay::{BaseRecords, MergedState, Overlay},
            state::NextIds,
        };
        // Re-freeze the small base view (small_base() already proves it attaches),
        // seeding the empty overlay with the base's own catalog + watermark so
        // nothing is lost.
        let base = small_base();
        let records = BaseRecords::from_view(base.get()).expect("base records");
        let header = *base.get().header();
        let overlay = Overlay::empty(NextIds::from_header(&header), base.get().catalog().clone());
        let view = MergedState::new(&records, &overlay);
        freeze_view(
            &view,
            FreezeStamps {
                commit_seq: 1,
                transaction_id: 1,
                generation: 1,
            },
        )
        .expect("freeze small base bytes")
    }

    /// File-backed tests that touch the real filesystem; gated off miri, whose
    /// isolation blocks file IO.
    #[cfg(not(miri))]
    mod file_backed {
        use std::{
            path::PathBuf,
            sync::atomic::{AtomicU64, Ordering},
        };

        use super::*;

        /// Per-process path counter for unique temporary base files.
        static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

        /// Returns a unique temporary base-file path and writes `bytes` to it.
        fn write_temp_base(name: &str, bytes: &[u8]) -> PathBuf {
            let id = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "oxgraph-db-backing-{name}-{}-{id}.oxgdb",
                std::process::id()
            ));
            std::fs::write(&path, bytes).expect("write temp base");
            path
        }

        /// `Base::open` borrows canonical reads from a real base file, and the
        /// mmap default (`force_owned = false`) and the forced-owned read
        /// (`force_owned = true`) yield identical reads.
        #[test]
        fn file_open_borrows_canonical_reads() {
            let bytes = small_base_bytes();
            let path = write_temp_base("open", &bytes);

            let default_open = Base::open(&path, false).expect("open base");
            let forced_owned = Base::open(&path, true).expect("open base force_owned");

            assert_small_reads(&default_open);
            assert_small_reads(&forced_owned);

            // mmap (default) and owned (forced) must agree on element counts and
            // every borrowed label run.
            let lhs = default_open.get();
            let rhs = forced_owned.get();
            assert_eq!(lhs.elements().count(), rhs.elements().count());
            for (lhs_record, rhs_record) in lhs.elements().zip(rhs.elements()) {
                assert_eq!(lhs_record.id.get(), rhs_record.id.get());
                assert_eq!(
                    lhs.element_label_run(lhs_record),
                    rhs.element_label_run(rhs_record),
                );
            }

            let _ = std::fs::remove_file(&path);
        }

        /// A missing base file opens to `NotFound`.
        #[test]
        fn missing_base_is_not_found() {
            let path = std::env::temp_dir().join(format!(
                "oxgraph-db-backing-missing-{}-{}.oxgdb",
                std::process::id(),
                NEXT_PATH.fetch_add(1, Ordering::Relaxed),
            ));
            let _ = std::fs::remove_file(&path);
            let result = Base::open(&path, true).map(|_base| ());
            assert!(
                matches!(result, Err(StorageError::NotFound)),
                "got {result:?}"
            );
        }
    }
}
