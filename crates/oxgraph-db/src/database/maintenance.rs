//! Checkpointing: the auto-checkpoint policy, validation, and the
//! crash-safe fold of base+overlay into a fresh base generation.

use std::path::Path;

use super::{
    Db,
    open::{base_file, create_empty_log, delta_file, file_len, write_superblock},
};
use crate::{
    DbError,
    backing::Base,
    freeze::{self, FreezeStamps},
    lock::WriterLock,
    storage, wal,
};

/// Auto-checkpoint policy: decides when a dirty commit should fold the
/// delta-log into a fresh base generation, bounding the log tail that recovery
/// must replay.
///
/// The default is size-ratio: trigger when the delta-log grows past `factor`
/// times the live base size (`factor` configurable). [`CheckpointPolicy::Manual`]
/// disables auto-triggering entirely (folded only by an explicit
/// [`Db::compact`]).
///
/// # Performance
///
/// Copying this value is `O(1)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointPolicy {
    /// Never auto-checkpoint; the caller folds explicitly via [`Db::compact`].
    Manual,
    /// Auto-checkpoint after a dirty commit once the delta-log exceeds `factor`
    /// times the live base size (a small floor guards a tiny/empty base so the
    /// gen-0 store does not checkpoint on its first commit).
    SizeRatio {
        /// Log-to-base size factor `K`; the log may grow to `K × base` bytes
        /// before the next dirty commit folds it.
        factor: u32,
    },
}

impl CheckpointPolicy {
    /// The default auto-checkpoint factor `K`: fold when the delta-log exceeds
    /// four times the live base size.
    pub const DEFAULT_FACTOR: u32 = 4;

    /// The base-size floor (bytes) below which the size-ratio policy never fires,
    /// so a freshly created (near-empty) base is not checkpointed on its first
    /// commits before it carries meaningful data.
    const MIN_BASE_BYTES: u64 = 4 * 1024;

    /// Returns whether a delta-log of `log_bytes` over a base of `base_bytes`
    /// should trigger an auto-checkpoint under this policy.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    const fn should_checkpoint(self, log_bytes: u64, base_bytes: u64) -> bool {
        match self {
            Self::Manual => false,
            Self::SizeRatio { factor } => {
                let floor = if base_bytes < Self::MIN_BASE_BYTES {
                    Self::MIN_BASE_BYTES
                } else {
                    base_bytes
                };
                log_bytes > floor.saturating_mul(factor as u64)
            }
        }
    }
}

impl Default for CheckpointPolicy {
    /// The default policy: size-ratio with [`CheckpointPolicy::DEFAULT_FACTOR`].
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn default() -> Self {
        Self::SizeRatio {
            factor: Self::DEFAULT_FACTOR,
        }
    }
}

impl Db {
    /// Returns the configured auto-checkpoint policy.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn checkpoint_policy(&self) -> CheckpointPolicy {
        self.checkpoint_policy
    }

    /// Sets the auto-checkpoint policy consulted after each dirty commit.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub const fn set_checkpoint_policy(&mut self, policy: CheckpointPolicy) {
        self.checkpoint_policy = policy;
    }

    /// Validates the current handle by re-reading the superblock and verifying
    /// the live base's content CRC.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the superblock or base fails validation.
    ///
    /// # Performance
    ///
    /// This method is `O(base bytes)`.
    pub fn validate(&self) -> Result<(), DbError> {
        wal::read_superblock(&self.root)?;
        Base::open(&self.root.join(base_file(self.base_generation)), false)
            .map(|_base| ())
            .map_err(DbError::from)
    }

    /// Validates an OXGDB database at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the store fails to open and recover.
    ///
    /// # Performance
    ///
    /// This function is `O(base bytes + log bytes)`.
    pub fn validate_path(path: impl AsRef<Path>) -> Result<(), DbError> {
        Self::open(path).map(|_database| ())
    }

    /// Folds the current base+overlay into a new base generation, rotating the
    /// delta-log and republishing the superblock (a manual checkpoint).
    ///
    /// This is the checkpoint primitive, exposed here so the existing `compact`
    /// API keeps its "rewrite the store compactly" contract. Auto-triggering is
    /// configured separately via [`Db::set_checkpoint_policy`].
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when encoding, writing, or publishing the new
    /// generation fails.
    ///
    /// # Performance
    ///
    /// This method is `O(visible state bytes)`.
    pub fn compact(&mut self) -> Result<(), DbError> {
        self.checkpoint()
    }

    /// Folds the current base+overlay into base-`{g+1}`, creates an empty
    /// delta-`{g+1}`.log, republishes the superblock naming `g+1` (the
    /// linearization point), then unlinks the old base and log.
    ///
    /// The order is crash-safe: the new base is fully durable BEFORE the
    /// superblock names it (so a crash before the superblock leaves the OLD
    /// superblock authoritative and the orphan new base is ignored), and the old
    /// base/log are unlinked only AFTER the superblock names the new generation
    /// (so a crash before the unlink leaves the NEW superblock authoritative and
    /// the orphan old files are ignored). The
    /// [`crate::wire::SuperblockRecord`] rename is the single linearization point.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when encoding, writing, or publishing fails.
    ///
    /// # Performance
    ///
    /// This method is `O(visible state bytes)`.
    pub(crate) fn checkpoint(&mut self) -> Result<(), DbError> {
        self.checkpoint_inner(
            #[cfg(test)]
            CheckpointStop::Complete,
        )
    }

    /// Crash-safe checkpoint body. Under `#[cfg(test)]` it accepts a
    /// [`CheckpointStop`] that simulates a crash by returning early right after a
    /// chosen fsync point, leaving the on-disk files exactly as a real crash
    /// there would, so the crash-matrix test can reopen and assert recovery.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when encoding, writing, or publishing fails.
    ///
    /// # Performance
    ///
    /// This method is `O(visible state bytes)`.
    pub(super) fn checkpoint_inner(
        &mut self,
        #[cfg(test)] stop: CheckpointStop,
    ) -> Result<(), DbError> {
        let _lock = WriterLock::acquire(&self.root)?;
        let next_generation = self
            .base_generation
            .checked_add(1)
            .ok_or_else(|| DbError::invalid_store("checkpoint generation overflow"))?;
        let view = self.current.view();
        let commit_seq = self.current.lsn().get();
        let base_bytes = freeze::freeze_view(
            &view,
            FreezeStamps {
                commit_seq,
                transaction_id: self.last_transaction_id.get(),
                generation: next_generation,
            },
        )?;
        // (1) write base-{g+1} (temp + fsync + rename + dir-fsync).
        storage::atomic_write(
            &self.root,
            &self
                .root
                .join(format!("{}.tmp", base_file(next_generation))),
            &self.root.join(base_file(next_generation)),
            &base_bytes,
        )?;
        // (2) create empty delta-{g+1}.log (fsync + dir-fsync).
        create_empty_log(&self.root, next_generation)?;
        // Crash point A: new base + new log durable, superblock NOT yet
        // published. The OLD superblock still names `g`, so recovery uses the old
        // generation; the new base/log are orphans.
        #[cfg(test)]
        if matches!(stop, CheckpointStop::BeforeSuperblock) {
            return Ok(());
        }
        // (3) publish the superblock naming g+1 — the linearization point.
        write_superblock(
            &self.root,
            next_generation,
            commit_seq,
            commit_seq,
            self.last_transaction_id.get(),
        )?;
        // Crash point B: superblock now names g+1, old base/log NOT yet unlinked.
        // Recovery uses the new generation; the old base/log are orphans.
        #[cfg(test)]
        if matches!(stop, CheckpointStop::BeforeRotate) {
            return Ok(());
        }
        // Re-open over the new generation, then (4) unlink the old base + log.
        let reopened = Self::open(&self.root)?;
        let old_generation = self.base_generation;
        let policy = self.checkpoint_policy;
        self.current = reopened.current;
        self.base_generation = reopened.base_generation;
        self.last_transaction_id = reopened.last_transaction_id;
        // The reopen reset the policy to the default; restore the caller's.
        self.checkpoint_policy = policy;
        let _ = std::fs::remove_file(self.root.join(base_file(old_generation)));
        let _ = std::fs::remove_file(self.root.join(delta_file(old_generation)));
        let _ = storage::sync_directory(&self.root);
        Ok(())
    }

    /// Auto-checkpoints when the configured [`CheckpointPolicy`] says the
    /// delta-log has grown too large relative to the base. Called after a dirty
    /// commit publishes its frame. A failed fold is surfaced so the caller can
    /// observe it; the committed data is already durable in the log regardless.
    ///
    /// # Errors
    ///
    /// Returns [`DbError`] when the triggered fold fails.
    ///
    /// # Performance
    ///
    /// This method is `O(1)` to decide; `O(visible state bytes)` when it folds.
    pub(super) fn maybe_auto_checkpoint(&mut self) -> Result<(), DbError> {
        let log_bytes = file_len(&self.root.join(delta_file(self.base_generation)));
        let base_bytes = file_len(&self.root.join(base_file(self.base_generation)));
        if self
            .checkpoint_policy
            .should_checkpoint(log_bytes, base_bytes)
        {
            self.checkpoint()?;
        }
        Ok(())
    }
}

/// Test-only crash-injection point for [`Db::checkpoint_inner`]: stops the
/// fold right after a chosen fsync so the crash-matrix test can reopen and assert
/// the recovered state at each crash window.
///
/// The crash-matrix test that constructs the non-`Complete` variants is
/// `#[cfg(not(miri))]` (it reopens a real store across simulated crashes, which
/// miri's isolation cannot model), so under miri only `Complete` is constructed
/// and the other variants are expectedly unused.
///
/// # Performance
///
/// `perf: unspecified`; a test-only control tag.
#[cfg(test)]
#[cfg_attr(
    miri,
    expect(
        dead_code,
        reason = "the crash-injection variants are constructed only by the #[cfg(not(miri))] crash-matrix test"
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CheckpointStop {
    /// Run the whole checkpoint (the production path).
    Complete,
    /// Stop after the new base + new log are durable, before the superblock is
    /// published (the old superblock stays authoritative).
    BeforeSuperblock,
    /// Stop after the superblock names the new generation, before the old
    /// base/log are unlinked (the new superblock is authoritative).
    BeforeRotate,
}
