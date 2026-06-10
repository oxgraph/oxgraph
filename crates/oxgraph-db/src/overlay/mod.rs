//! In-memory delta overlay layered over the borrowed base.
//!
//! This is the state model of the MVCC engine: an owned, change-sized
//! delta ([`Overlay`]) layered over the immutable, borrowed base
//! ([`crate::backing::Base`]), unified for reads by a k-way merge
//! ([`MergedState`]) so a lookup sees the overlay first and falls through to
//! the base. Three layers compose:
//!
//! * [`WriteOverlay`] — the MUTABLE delta a single in-flight write transaction accumulates. It
//!   records creates, tombstones, property sets/removes, and catalog registrations into per-family
//!   maps WITHOUT touching the base, and carries the nine monotonic id allocators (the watermark).
//!   It is private to the writer and never shared.
//! * [`Overlay`] — the FROZEN, published delta: the same data, immutable. [`WriteOverlay::freeze`]
//!   turns the writer's accumulated delta into one. A published `Arc<Overlay>` is immutable
//!   FOREVER; a commit seeds a fresh writer from the parent overlay
//!   ([`WriteOverlay::from_overlay`]), applies the new mutations on top, freezes it, and publishes
//!   a brand-new `Arc<Overlay>` — it NEVER mutates a published overlay in place.
//! * [`MergedState`] — the borrowed read view that merges an `Overlay` over a base snapshot. Point
//!   reads return [`Cow`]: a base-only id borrows from the base (zero clone, the fast path); an
//!   overlay-supplied or overlay-overridden id is owned by the overlay; a tombstoned id is absent.
//!
//! Canonical ids are NEVER reused: the overlay only adds records or tombstones
//! them (masking a base record), never renumbers, and the watermark is monotonic
//! across every commit (each fresh writer is seeded from the parent's watermark).
//!
//! # Wiring
//!
//! This overlay/merge model IS the live read/write path. `query.rs`,
//! `projection.rs`, and `database.rs` all read state through [`StateView`] (a
//! [`Snapshot`]'s [`MergedState`], or a writer's [`WriteMergedState`]), and
//! writes accumulate into a [`WriteOverlay`] that a commit freezes into a
//! published [`Overlay`]. The clone-and-apply primitive that proves the merge
//! laws (`Overlay::with_applied`, gated to `cfg(test)`/`cfg(kani)`) mirrors that
//! commit semantics for the proofs; the live path freezes the seeded writer
//! directly.
//!
//! # Performance
//!
//! `perf: unspecified`; this module defines the overlay/merge primitives. Each
//! item below carries its own contract: overlay point reads and mutators are
//! `O(log change)`; building the next overlay (clone the parent, apply the delta)
//! is `O(parent change + delta change)`; merge iterators are `O(base + overlay
//! change)`.

use std::collections::BTreeMap;

use crate::{
    ElementId, IncidenceId, RelationId,
    state::{ElementRecord, IncidenceRecord, RelationRecord},
};

mod frozen;
mod log;
mod merge;
mod write;

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

pub(crate) use frozen::{BaseRecords, Overlay, Snapshot};
#[cfg(any(test, kani))]
pub(crate) use merge::OverlayLayer;
pub(crate) use merge::{MergedState, StateView, WriteMergedState};
pub(crate) use write::WriteOverlay;

/// One owned, masked entry in an overlay delta: `Some(record)` adds or overrides
/// the record for that id; `None` is a tombstone that hides the base record (and
/// any earlier overlay record) for that id.
///
/// # Performance
///
/// Copying the variant tag is `O(1)`; cloning a present record is `O(record
/// size)`.
type Delta<R> = BTreeMap<<R as Keyed>::Id, Option<R>>;

/// A record keyed by a canonical id, so the per-family delta maps can name their
/// key type generically.
///
/// # Performance
///
/// [`Self::record_id`] is `O(1)`.
pub(crate) trait Keyed {
    /// Canonical id type that keys this record's delta map.
    type Id: Copy + Ord;

    /// Returns this record's canonical id.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn record_id(&self) -> Self::Id;
}

impl Keyed for ElementRecord {
    type Id = ElementId;

    fn record_id(&self) -> Self::Id {
        self.id
    }
}

impl Keyed for RelationRecord {
    type Id = RelationId;

    fn record_id(&self) -> Self::Id {
        self.id
    }
}

impl Keyed for IncidenceRecord {
    type Id = IncidenceId;

    fn record_id(&self) -> Self::Id {
        self.id
    }
}
