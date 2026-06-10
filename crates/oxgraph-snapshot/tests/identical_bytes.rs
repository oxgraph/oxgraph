//! Property test: the no-`alloc` planner ([`SnapshotPlan::write_into`]) and the
//! `alloc`-gated write-through encoder ([`SnapshotWriter::finish`]) emit
//! byte-for-byte identical snapshots for the same logical sections (exact table
//! reservation), and the result re-opens.
//!
//! This is the cross-encoder equivalence law: a caller-supplied buffer filled
//! by the plan and an owned buffer streamed by the writer cannot diverge.

use std::collections::BTreeSet;

use oxgraph_layout_util::crc32c_append;
use oxgraph_snapshot::{
    PendingSection, PlanError, Snapshot, SnapshotError, SnapshotPlan, SnapshotWriter,
};
use proptest::{prelude::*, test_runner::TestCaseError};

/// Converts writer/planner results into proptest failures.
fn prop_plan<T>(result: Result<T, PlanError>) -> Result<T, TestCaseError> {
    result.map_err(|error| TestCaseError::fail(error.to_string()))
}

/// Converts snapshot open results into proptest failures.
fn prop_open<T>(result: Result<T, SnapshotError>) -> Result<T, TestCaseError> {
    result.map_err(|error| TestCaseError::fail(error.to_string()))
}

/// One generated section: a unique kind, a version, an alignment log2 within
/// the format cap, and a bounded payload.
#[derive(Clone, Debug)]
struct GenSection {
    /// Section kind tag.
    kind: u32,
    /// Section version.
    version: u32,
    /// Payload alignment as `log2`, within the format cap.
    alignment_log2: u8,
    /// Section payload bytes.
    payload: Vec<u8>,
}

prop_compose! {
    fn gen_section()(
        kind in 0u32..64,
        version in 0u32..4,
        alignment_log2 in 0u8..=4,
        payload in proptest::collection::vec(any::<u8>(), 0..32),
    ) -> GenSection {
        GenSection { kind, version, alignment_log2, payload }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn writer_and_plan_emit_identical_bytes(
        sections in proptest::collection::vec(gen_section(), 0..8),
    ) {
        // Deduplicate kinds and sort ascending: both encoders mandate the format's
        // strictly-ascending kind order, and we want the success path here.
        let mut seen = BTreeSet::new();
        let mut unique: Vec<GenSection> = sections
            .into_iter()
            .filter(|section| seen.insert(section.kind))
            .collect();
        unique.sort_by_key(|section| section.kind);

        // Build via the write-through encoder with an exact reservation.
        let mut writer = prop_plan(SnapshotWriter::new(unique.len(), crc32c_append))?;
        for section in &unique {
            prop_plan(writer.section_bytes(
                section.kind,
                section.version,
                section.alignment_log2,
                &section.payload,
            ))?;
        }
        let from_writer = prop_plan(writer.finish())?;

        // Build via the no-alloc plan into a heap buffer.
        let pending: Vec<PendingSection<'_>> = unique
            .iter()
            .map(|section| PendingSection {
                kind: section.kind,
                version: section.version,
                alignment_log2: section.alignment_log2,
                payload: section.payload.as_slice(),
            })
            .collect();
        let plan = prop_plan(SnapshotPlan::new(&pending))?;
        let needed = prop_plan(plan.encoded_len())?;
        let mut from_plan = vec![0u8; needed];
        let written = prop_plan(plan.write_into(&mut from_plan, crc32c_append))?;
        prop_assert_eq!(written, needed);

        // The two encoders must agree byte for byte.
        prop_assert_eq!(&from_writer, &from_plan);

        // And the result re-opens and exposes the same sections.
        let snapshot = prop_open(Snapshot::open(&from_writer))?;
        prop_assert_eq!(snapshot.section_count(), unique.len());
        for section in &unique {
            let Some(view) = snapshot.section(section.kind) else {
                return Err(TestCaseError::fail("section missing after open"));
            };
            prop_assert_eq!(view.bytes(), section.payload.as_slice());
            prop_assert_eq!(view.version(), section.version);
        }
    }
}
