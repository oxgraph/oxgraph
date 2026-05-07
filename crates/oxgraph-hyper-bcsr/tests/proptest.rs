//! Property tests for borrowed bipartite-CSR validation and traversal.
//!
//! The harness generates valid bipartite-CSR layouts from per-hyperedge picks,
//! then exercises the open-and-traverse surface of [`BcsrHypergraph`]. Property
//! 4 additionally tampers one outgoing-bucket entry to confirm the
//! `Layout` / `Strict` distinction holds — `Strict` must reject the
//! cross-direction inconsistency, while `Layout` is required to accept it.

use std::collections::BTreeSet;

use oxgraph_hyper::{
    DirectedHyperedgeParticipants, ElementIncidenceCount, HyperedgeParticipantCount,
    HyperedgeParticipants, IncidentHyperedgeCount, IncidentHyperedges, RelationIncidenceCount,
};
use oxgraph_hyper_bcsr::{
    BcsrError, BcsrHyperedgeId, BcsrHypergraph, BcsrRoleSide, BcsrSections, BcsrValidation,
    BcsrVertexId,
};
use proptest::prelude::*;

/// Per-hyperedge picks tuple drawn from the strategy:
/// `(vertex_count, hyperedge_count, head_picks, tail_picks)`. Each pick is a
/// vector of vertex IDs that may contain duplicates and out-of-range values;
/// the builder dedups and clamps both.
type BcsrPicks = (u32, u32, Vec<Vec<u32>>, Vec<Vec<u32>>);

/// One side's flattened CSR triple: `(offsets, flat_values, per_bucket_degrees)`.
/// Used as the return shape of [`flatten_hyperedge_major`] and
/// [`flatten_vertex_major`].
type FlattenedSide = (Vec<u32>, Vec<u32>, Vec<u32>);

/// Owned bipartite-CSR buffers backing a borrowed [`BcsrSections`] for one
/// generated test case. The per-hyperedge head/tail and per-vertex
/// outgoing/incoming degrees are kept alongside the buffers so properties can
/// check counts without re-walking offsets.
#[derive(Debug)]
struct OwnedBcsr {
    /// `hyperedge_count + 1` head offsets.
    head_offsets: Vec<u32>,
    /// `P_head` flat head participant vertex IDs.
    head_participants: Vec<u32>,
    /// `hyperedge_count + 1` tail offsets.
    tail_offsets: Vec<u32>,
    /// `P_tail` flat tail participant vertex IDs.
    tail_participants: Vec<u32>,
    /// `vertex_count + 1` outgoing offsets.
    vertex_outgoing_offsets: Vec<u32>,
    /// `P_outgoing` flat outgoing hyperedge IDs.
    vertex_outgoing_hyperedges: Vec<u32>,
    /// `vertex_count + 1` incoming offsets.
    vertex_incoming_offsets: Vec<u32>,
    /// `P_incoming` flat incoming hyperedge IDs.
    vertex_incoming_hyperedges: Vec<u32>,
    /// Vertex count drawn from the strategy.
    vertex_count: u32,
    /// Hyperedge count drawn from the strategy.
    hyperedge_count: u32,
    /// Per-hyperedge head degrees, length `hyperedge_count`.
    head_degrees: Vec<u32>,
    /// Per-hyperedge tail degrees, length `hyperedge_count`.
    tail_degrees: Vec<u32>,
    /// Per-vertex outgoing degrees, length `vertex_count`.
    outgoing_degrees: Vec<u32>,
    /// Per-vertex incoming degrees, length `vertex_count`.
    incoming_degrees: Vec<u32>,
}

impl OwnedBcsr {
    /// Returns a borrowed [`BcsrSections`] view over the owned buffers.
    fn sections(&self) -> BcsrSections<'_, u32, u32, u32> {
        BcsrSections {
            head_offsets: &self.head_offsets,
            head_participants: &self.head_participants,
            tail_offsets: &self.tail_offsets,
            tail_participants: &self.tail_participants,
            vertex_outgoing_offsets: &self.vertex_outgoing_offsets,
            vertex_outgoing_hyperedges: &self.vertex_outgoing_hyperedges,
            vertex_incoming_offsets: &self.vertex_incoming_offsets,
            vertex_incoming_hyperedges: &self.vertex_incoming_hyperedges,
        }
    }
}

/// Strategy producing `(vertex_count, hyperedge_count, head_picks, tail_picks)`
/// where each pick is a vector of vertex IDs (possibly with duplicates and
/// out-of-range values, both filtered by the builder). Bounds are tight enough
/// that every count fits in `u32` on every supported target.
fn gen_bcsr_picks() -> impl Strategy<Value = BcsrPicks> {
    (0u32..16, 0u32..16).prop_flat_map(|(vertex_count, hyperedge_count)| {
        let h_count = usize::try_from(hyperedge_count).unwrap_or(0);
        let pick_count_max = if vertex_count == 0 { 0 } else { 4usize };
        let pick_value_max = vertex_count.saturating_sub(1);
        let head = proptest::collection::vec(
            proptest::collection::vec(0u32..=pick_value_max, 0..=pick_count_max),
            h_count,
        );
        let tail = proptest::collection::vec(
            proptest::collection::vec(0u32..=pick_value_max, 0..=pick_count_max),
            h_count,
        );
        (Just(vertex_count), Just(hyperedge_count), head, tail)
    })
}

/// Builds a valid bipartite-CSR layout from raw picks: dedups + sorts each
/// hyperedge's head/tail picks (clamped to `< vertex_count`) and inverts into
/// per-vertex outgoing/incoming buckets.
fn build_owned_bcsr(
    vertex_count: u32,
    hyperedge_count: u32,
    head_picks: &[Vec<u32>],
    tail_picks: &[Vec<u32>],
) -> Result<OwnedBcsr, TestCaseError> {
    let head_sets = dedup_picks(head_picks, vertex_count);
    let tail_sets = dedup_picks(tail_picks, vertex_count);

    let (head_offsets, head_participants, head_degrees) = flatten_hyperedge_major(&head_sets)?;
    let (tail_offsets, tail_participants, tail_degrees) = flatten_hyperedge_major(&tail_sets)?;

    let v_count = try_into_usize(vertex_count)?;
    let outgoing_per_vertex = invert_into_vertex_major(&head_sets, v_count)?;
    let incoming_per_vertex = invert_into_vertex_major(&tail_sets, v_count)?;

    let (vertex_outgoing_offsets, vertex_outgoing_hyperedges, outgoing_degrees) =
        flatten_vertex_major(&outgoing_per_vertex)?;
    let (vertex_incoming_offsets, vertex_incoming_hyperedges, incoming_degrees) =
        flatten_vertex_major(&incoming_per_vertex)?;

    Ok(OwnedBcsr {
        head_offsets,
        head_participants,
        tail_offsets,
        tail_participants,
        vertex_outgoing_offsets,
        vertex_outgoing_hyperedges,
        vertex_incoming_offsets,
        vertex_incoming_hyperedges,
        vertex_count,
        hyperedge_count,
        head_degrees,
        tail_degrees,
        outgoing_degrees,
        incoming_degrees,
    })
}

/// Drops out-of-range and duplicate vertex IDs from each hyperedge's picks.
fn dedup_picks(picks: &[Vec<u32>], vertex_count: u32) -> Vec<BTreeSet<u32>> {
    picks
        .iter()
        .map(|inner| {
            inner
                .iter()
                .copied()
                .filter(|v| *v < vertex_count)
                .collect()
        })
        .collect()
}

/// Builds (`offsets`, `flat_participants`, `degrees`) for one hyperedge-major
/// side from per-hyperedge sorted vertex sets.
fn flatten_hyperedge_major(sets: &[BTreeSet<u32>]) -> Result<FlattenedSide, TestCaseError> {
    let mut offsets = Vec::with_capacity(sets.len() + 1);
    let mut flat = Vec::new();
    let mut degrees = Vec::with_capacity(sets.len());
    offsets.push(0u32);
    for set in sets {
        degrees.push(try_into_u32(set.len())?);
        flat.extend(set.iter().copied());
        offsets.push(try_into_u32(flat.len())?);
    }
    Ok((offsets, flat, degrees))
}

/// Inverts a per-hyperedge incidence list into per-vertex hyperedge sets.
fn invert_into_vertex_major(
    sets: &[BTreeSet<u32>],
    vertex_count: usize,
) -> Result<Vec<BTreeSet<u32>>, TestCaseError> {
    let mut per_vertex: Vec<BTreeSet<u32>> = (0..vertex_count).map(|_| BTreeSet::new()).collect();
    for (h_idx, set) in sets.iter().enumerate() {
        let h_id = try_into_u32(h_idx)?;
        for vertex in set {
            let v_idx = try_into_usize(*vertex)?;
            per_vertex[v_idx].insert(h_id);
        }
    }
    Ok(per_vertex)
}

/// Builds (`offsets`, `flat_hyperedges`, `degrees`) for one vertex-major side.
fn flatten_vertex_major(per_vertex: &[BTreeSet<u32>]) -> Result<FlattenedSide, TestCaseError> {
    let mut offsets = Vec::with_capacity(per_vertex.len() + 1);
    let mut flat = Vec::new();
    let mut degrees = Vec::with_capacity(per_vertex.len());
    offsets.push(0u32);
    for set in per_vertex {
        degrees.push(try_into_u32(set.len())?);
        flat.extend(set.iter().copied());
        offsets.push(try_into_u32(flat.len())?);
    }
    Ok((offsets, flat, degrees))
}

/// Converts a `usize` into a `u32`, mapping the failure to a `TestCaseError`.
fn try_into_u32(value: usize) -> Result<u32, TestCaseError> {
    u32::try_from(value)
        .map_err(|err| TestCaseError::fail(format!("usize {value} does not fit u32: {err:?}")))
}

/// Converts a `u32` into a `usize`, mapping the failure to a `TestCaseError`.
fn try_into_usize(value: u32) -> Result<usize, TestCaseError> {
    usize::try_from(value)
        .map_err(|err| TestCaseError::fail(format!("u32 {value} does not fit usize: {err:?}")))
}

/// Walks an `ExactSizeIterator` end-to-end and confirms `len()` decrements by
/// exactly one per `next()` call until it reaches 0.
fn check_exact_size_walk<I>(mut iter: I) -> Result<(), TestCaseError>
where
    I: ExactSizeIterator,
{
    let mut remaining = iter.len();
    while iter.next().is_some() {
        remaining = remaining.checked_sub(1).ok_or_else(|| {
            TestCaseError::fail("ExactSizeIterator yielded more elements than initial len()")
        })?;
        prop_assert_eq!(iter.len(), remaining);
    }
    if iter.len() != 0 {
        return Err(TestCaseError::fail(format!(
            "iterator len() == {} after exhaustion",
            iter.len()
        )));
    }
    Ok(())
}

/// Locates a single-entry vertex-outgoing bucket and returns a tampered
/// `vertex_outgoing_hyperedges` array where that entry has been swapped to
/// `(original + 1) % hyperedge_count`. Returns `None` if no such bucket exists
/// or if `hyperedge_count < 2`.
///
/// Single-entry buckets keep the strictly-ascending invariant trivially
/// preserved after the swap, so `Layout` validation still accepts the result;
/// `Strict` validation is required to detect that the hyperedge-major head
/// index no longer agrees with the swapped vertex-major outgoing index.
fn tamper_single_outgoing_bucket(owned: &OwnedBcsr) -> Option<Vec<u32>> {
    if owned.hyperedge_count < 2 {
        return None;
    }
    for v_idx in 0..owned.outgoing_degrees.len() {
        let start = usize::try_from(owned.vertex_outgoing_offsets[v_idx]).ok()?;
        let end = usize::try_from(owned.vertex_outgoing_offsets[v_idx + 1]).ok()?;
        if end.checked_sub(start)? != 1 {
            continue;
        }
        let original = owned.vertex_outgoing_hyperedges[start];
        let swapped = (original + 1) % owned.hyperedge_count;
        if swapped == original {
            continue;
        }
        let mut tampered = owned.vertex_outgoing_hyperedges.clone();
        tampered[start] = swapped;
        return Some(tampered);
    }
    None
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Generated bipartite-CSR layouts validate at both Layout and Strict.
    #[test]
    fn generated_bcsr_validates_at_layout_and_strict(
        (vertex_count, hyperedge_count, head_picks, tail_picks) in gen_bcsr_picks()
    ) {
        let owned = build_owned_bcsr(vertex_count, hyperedge_count, &head_picks, &tail_picks)?;
        let sections = owned.sections();
        prop_assert!(BcsrHypergraph::open(sections).is_ok());
        prop_assert!(BcsrHypergraph::open_with(sections, BcsrValidation::Strict).is_ok());
    }

    /// Per-hyperedge and per-vertex traversal counts agree with the explicit
    /// degree counts the generator records, and the cross-direction sum
    /// invariant `Σ head_degree == Σ outgoing_degree` holds (and symmetrically
    /// for tail/incoming).
    #[test]
    fn generated_bcsr_counts_match_traversal(
        (vertex_count, hyperedge_count, head_picks, tail_picks) in gen_bcsr_picks()
    ) {
        let owned = build_owned_bcsr(vertex_count, hyperedge_count, &head_picks, &tail_picks)?;
        let view = match BcsrHypergraph::open_with(owned.sections(), BcsrValidation::Strict) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!(
                "Strict open rejected valid input: {error:?}"
            ))),
        };

        prop_assert_eq!(view.vertex_count(), try_into_usize(owned.vertex_count)?);
        prop_assert_eq!(view.hyperedge_count(), try_into_usize(owned.hyperedge_count)?);

        for (h_idx, head_deg) in owned.head_degrees.iter().enumerate() {
            let tail_deg = owned.tail_degrees[h_idx];
            let expected = try_into_usize(*head_deg + tail_deg)?;
            let h_id = BcsrHyperedgeId(try_into_u32(h_idx)?);
            prop_assert_eq!(view.hyperedge_participants(h_id).count(), expected);
            prop_assert_eq!(view.hyperedge_participant_count(h_id), expected);
            prop_assert_eq!(view.relation_incidence_count(h_id), expected);
        }

        for (v_idx, out_deg) in owned.outgoing_degrees.iter().enumerate() {
            let in_deg = owned.incoming_degrees[v_idx];
            let expected = try_into_usize(*out_deg + in_deg)?;
            let v_id = BcsrVertexId(try_into_u32(v_idx)?);
            prop_assert_eq!(view.incident_hyperedges(v_id).count(), expected);
            prop_assert_eq!(view.incident_hyperedge_count(v_id), expected);
            prop_assert_eq!(view.element_incidence_count(v_id), expected);
        }

        let sum_head: u32 = owned.head_degrees.iter().sum();
        let sum_outgoing: u32 = owned.outgoing_degrees.iter().sum();
        prop_assert_eq!(sum_head, sum_outgoing);
        let sum_tail: u32 = owned.tail_degrees.iter().sum();
        let sum_incoming: u32 = owned.incoming_degrees.iter().sum();
        prop_assert_eq!(sum_tail, sum_incoming);
    }

    /// `BcsrVertexSlice` (the iterator returned by `source_participants` and
    /// `target_participants`) preserves `ExactSizeIterator::len()` as it
    /// advances.
    #[test]
    fn generated_bcsr_iterators_preserve_exact_size(
        (vertex_count, hyperedge_count, head_picks, tail_picks) in gen_bcsr_picks()
    ) {
        let owned = build_owned_bcsr(vertex_count, hyperedge_count, &head_picks, &tail_picks)?;
        let view = match BcsrHypergraph::open(owned.sections()) {
            Ok(value) => value,
            Err(error) => return Err(TestCaseError::fail(format!(
                "Layout open rejected valid input: {error:?}"
            ))),
        };

        for h_idx in 0..owned.hyperedge_count {
            let h_id = BcsrHyperedgeId(h_idx);
            check_exact_size_walk(view.source_participants(h_id))?;
            check_exact_size_walk(view.target_participants(h_id))?;
        }
    }

    /// Layout passes but Strict rejects when one outgoing-bucket entry is
    /// swapped to a different in-range hyperedge — the canonical
    /// `BcsrError::CrossDirectionMismatch` path applied to generated inputs
    /// rather than the single hand-built fixture in `tests/bcsr.rs`.
    #[test]
    fn tampered_outgoing_bucket_is_layout_pass_strict_fail(
        (vertex_count, hyperedge_count, head_picks, tail_picks) in gen_bcsr_picks()
    ) {
        let owned = build_owned_bcsr(vertex_count, hyperedge_count, &head_picks, &tail_picks)?;
        let Some(tampered) = tamper_single_outgoing_bucket(&owned) else {
            return Ok(());
        };

        let tampered_sections = BcsrSections {
            head_offsets: &owned.head_offsets,
            head_participants: &owned.head_participants,
            tail_offsets: &owned.tail_offsets,
            tail_participants: &owned.tail_participants,
            vertex_outgoing_offsets: &owned.vertex_outgoing_offsets,
            vertex_outgoing_hyperedges: &tampered,
            vertex_incoming_offsets: &owned.vertex_incoming_offsets,
            vertex_incoming_hyperedges: &owned.vertex_incoming_hyperedges,
        };

        prop_assert!(
            BcsrHypergraph::open(tampered_sections).is_ok(),
            "Layout should accept a single-entry-bucket swap"
        );

        match BcsrHypergraph::open_with(tampered_sections, BcsrValidation::Strict) {
            Err(BcsrError::CrossDirectionMismatch { side: BcsrRoleSide::Outgoing, .. }) => {}
            other => return Err(TestCaseError::fail(format!(
                "expected CrossDirectionMismatch{{side:Outgoing,..}} after tampering; got {other:?}"
            ))),
        }
    }
}
