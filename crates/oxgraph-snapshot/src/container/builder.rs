//! Alloc-gated owning snapshot builder.
//!
//! [`SnapshotBuilder`] is a thin convenience over [`SnapshotPlan`]: it owns
//! payload byte buffers and computes the final encoded `Vec<u8>` for the
//! caller. The byte layout it produces is identical to what
//! [`SnapshotPlan`] would emit for an equivalent set of borrowed sections.

use alloc::{vec, vec::Vec};

use zerocopy::IntoBytes;

use super::{
    MAX_ALIGNMENT_LOG2, MAX_SECTION_COUNT,
    error::PlanError,
    plan::{PendingSection, SnapshotPlan},
};

/// One owned section pending in a [`SnapshotBuilder`].
#[derive(Clone, Debug)]
struct OwnedSection {
    /// Section kind.
    kind: u32,
    /// Section version.
    version: u32,
    /// Declared payload alignment as `log2`.
    alignment_log2: u8,
    /// Owned payload bytes.
    payload: Vec<u8>,
}

/// Owning snapshot builder that produces a `Vec<u8>` on finish.
///
/// The builder rejects duplicate kinds, alignment overflows, and section
/// count overflows at `add_section*` time, so the only failure that can
/// reach [`SnapshotBuilder::finish`] is [`PlanError::PayloadOverflow`] —
/// when the cumulative encoded length would overflow `u64` or `usize`.
///
/// # Performance
///
/// `add_section*` methods are `O(s)` for the in-progress section count.
/// [`finish`](Self::finish) is `O(s + total payload bytes)`.
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct SnapshotBuilder {
    /// Owned, in-order sections.
    sections: Vec<OwnedSection>,
}

impl SnapshotBuilder {
    /// Constructs an empty builder.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a section with the given metadata and owned payload.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError`] when `alignment_log2` is too large, when the
    /// builder already holds the maximum permitted section count, or when
    /// `kind` collides with an earlier section.
    ///
    /// # Performance
    ///
    /// This method is `O(s)` for the in-progress section count.
    pub fn add_section(
        &mut self,
        kind: u32,
        version: u32,
        alignment_log2: u8,
        payload: Vec<u8>,
    ) -> Result<&mut Self, PlanError> {
        if alignment_log2 > MAX_ALIGNMENT_LOG2 {
            return Err(PlanError::AlignmentTooLarge { alignment_log2 });
        }
        if self.sections.len() >= MAX_SECTION_COUNT as usize {
            return Err(PlanError::TooManySections {
                count: self.sections.len() + 1,
            });
        }
        for prior in &self.sections {
            if prior.kind == kind {
                return Err(PlanError::DuplicateKind { kind });
            }
        }
        self.sections.push(OwnedSection {
            kind,
            version,
            alignment_log2,
            payload,
        });
        Ok(self)
    }

    /// Appends a section whose alignment is derived from `T`.
    ///
    /// The payload is copied via [`zerocopy::IntoBytes`].
    ///
    /// # Errors
    ///
    /// Returns [`PlanError`] for the same reasons as
    /// [`add_section`](Self::add_section), plus
    /// [`PlanError::AlignmentTooLarge`] when `align_of::<T>()` exceeds
    /// the v1 cap.
    ///
    /// # Performance
    ///
    /// This method is `O(s + payload.len() * size_of::<T>())`.
    pub fn add_section_typed<T>(
        &mut self,
        kind: u32,
        version: u32,
        payload: &[T],
    ) -> Result<&mut Self, PlanError>
    where
        T: zerocopy::IntoBytes + zerocopy::Immutable,
    {
        let alignment = core::mem::align_of::<T>();
        let alignment_log2 = match u8::try_from(alignment.trailing_zeros()) {
            Ok(value) => value,
            Err(_error) => {
                return Err(PlanError::AlignmentTooLarge {
                    alignment_log2: u8::MAX,
                });
            }
        };
        if alignment_log2 > MAX_ALIGNMENT_LOG2 {
            return Err(PlanError::AlignmentTooLarge { alignment_log2 });
        }
        let bytes = payload.as_bytes().to_vec();
        self.add_section(kind, version, alignment_log2, bytes)
    }

    /// Returns the number of pending sections.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Encodes the pending sections into an owned snapshot byte vector.
    ///
    /// The builder enforces per-insert invariants
    /// ([`PlanError::AlignmentTooLarge`], [`PlanError::TooManySections`],
    /// [`PlanError::DuplicateKind`]) on `add_section*`, so this method
    /// only fails when the cumulative payload arithmetic overflows
    /// (`u64`/`usize`), surfaced as [`PlanError::PayloadOverflow`].
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::PayloadOverflow`] when the total encoded length
    /// would overflow `u64` or `usize`. All other [`PlanError`] variants
    /// are caught at `add_section*` time and cannot reach this method.
    ///
    /// # Performance
    ///
    /// This method is `O(s + total payload bytes)`.
    pub fn finish(self) -> Result<Vec<u8>, PlanError> {
        let pending: Vec<PendingSection<'_>> = self
            .sections
            .iter()
            .map(|section| PendingSection {
                kind: section.kind,
                version: section.version,
                alignment_log2: section.alignment_log2,
                payload: section.payload.as_slice(),
            })
            .collect();

        let plan = SnapshotPlan::new(&pending)?;
        let needed = plan.encoded_len()?;
        let mut out = vec![0u8; needed];
        plan.write_into(&mut out)?;
        Ok(out)
    }
}
