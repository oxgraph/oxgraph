//! Kani proofs for small database algebra contracts.

use core::hash::{Hash, Hasher};

use crate::{
    CheckpointGeneration, CommitSeq, ElementId, IncidenceId, IndexId, LabelId, ProjectionId,
    PropertyFamily, PropertyKeyId, PropertySubject, PropertyType, PropertyValue, RelationId,
    RelationTypeId, RoleId, TransactionId,
    projection::{ProjectionElementId, ProjectionIncidenceId, ProjectionRelationId},
    wire,
};

/// Deterministic proof hasher for ID newtype hash/equality checks.
#[derive(Default)]
struct ProofHasher {
    /// Accumulated hash state.
    state: u64,
}

impl Hasher for ProofHasher {
    /// Finishes the accumulated hash.
    fn finish(&self) -> u64 {
        self.state
    }

    /// Mixes bytes into the accumulated hash.
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.state = self.state.wrapping_mul(257).wrapping_add(u64::from(*byte));
        }
    }
}

/// Hashes one value with the deterministic proof hasher.
fn proof_hash<T: Hash>(value: T) -> u64 {
    let mut hasher = ProofHasher::default();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Declares algebra proofs shared by canonical ID newtypes.
macro_rules! prove_id_newtype {
    ($type:ty, $checked_next:ident, $ord_eq:ident, $hash_eq:ident) => {
        /// Proves checked advancement is monotonic when it succeeds.
        #[kani::proof]
        fn $checked_next() {
            let raw: u64 = kani::any();
            kani::assume(raw < u64::MAX);
            let current = <$type>::new(raw);
            let Some(next) = current.checked_next() else {
                assert!(false);
                return;
            };
            assert!(next > current);
            assert_eq!(next.get(), raw + 1);
        }

        /// Proves ordering equality agrees with value equality.
        #[kani::proof]
        fn $ord_eq() {
            let left_raw: u64 = kani::any();
            let right_raw: u64 = kani::any();
            let left = <$type>::new(left_raw);
            let right = <$type>::new(right_raw);
            assert_eq!(left == right, left.cmp(&right).is_eq());
        }

        /// Proves equal IDs hash identically.
        #[kani::proof]
        fn $hash_eq() {
            let raw: u64 = kani::any();
            let left = <$type>::new(raw);
            let right = <$type>::new(raw);
            assert_eq!(proof_hash(left), proof_hash(right));
        }
    };
}

/// Declares algebra proofs shared by projection-local IDs.
macro_rules! prove_projection_id {
    ($type:ty, $ord_eq:ident, $hash_eq:ident) => {
        /// Proves ordering equality agrees with value equality.
        #[kani::proof]
        fn $ord_eq() {
            let left_raw: u32 = kani::any();
            let right_raw: u32 = kani::any();
            let left = <$type>::new(left_raw);
            let right = <$type>::new(right_raw);
            assert_eq!(left == right, left.cmp(&right).is_eq());
        }

        /// Proves equal projection-local IDs hash identically.
        #[kani::proof]
        fn $hash_eq() {
            let raw: u32 = kani::any();
            let left = <$type>::new(raw);
            let right = <$type>::new(raw);
            assert_eq!(proof_hash(left), proof_hash(right));
        }
    };
}

prove_id_newtype!(
    ElementId,
    element_id_checked_next_is_monotonic,
    element_id_ordering_equality_matches_value_equality,
    element_id_hash_matches_equality
);
prove_id_newtype!(
    RelationId,
    relation_id_checked_next_is_monotonic,
    relation_id_ordering_equality_matches_value_equality,
    relation_id_hash_matches_equality
);
prove_id_newtype!(
    IncidenceId,
    incidence_id_checked_next_is_monotonic,
    incidence_id_ordering_equality_matches_value_equality,
    incidence_id_hash_matches_equality
);
prove_id_newtype!(
    RoleId,
    role_id_checked_next_is_monotonic,
    role_id_ordering_equality_matches_value_equality,
    role_id_hash_matches_equality
);
prove_id_newtype!(
    LabelId,
    label_id_checked_next_is_monotonic,
    label_id_ordering_equality_matches_value_equality,
    label_id_hash_matches_equality
);
prove_id_newtype!(
    RelationTypeId,
    relation_type_id_checked_next_is_monotonic,
    relation_type_id_ordering_equality_matches_value_equality,
    relation_type_id_hash_matches_equality
);
prove_id_newtype!(
    PropertyKeyId,
    property_key_id_checked_next_is_monotonic,
    property_key_id_ordering_equality_matches_value_equality,
    property_key_id_hash_matches_equality
);
prove_id_newtype!(
    ProjectionId,
    projection_id_checked_next_is_monotonic,
    projection_id_ordering_equality_matches_value_equality,
    projection_id_hash_matches_equality
);
prove_id_newtype!(
    IndexId,
    index_id_checked_next_is_monotonic,
    index_id_ordering_equality_matches_value_equality,
    index_id_hash_matches_equality
);
prove_id_newtype!(
    CommitSeq,
    commit_sequence_checked_next_is_monotonic,
    commit_sequence_ordering_equality_matches_value_equality,
    commit_sequence_hash_matches_equality
);
prove_id_newtype!(
    TransactionId,
    transaction_id_checked_next_is_monotonic,
    transaction_id_ordering_equality_matches_value_equality,
    transaction_id_hash_matches_equality
);
prove_id_newtype!(
    CheckpointGeneration,
    checkpoint_generation_checked_next_is_monotonic,
    checkpoint_generation_ordering_equality_matches_value_equality,
    checkpoint_generation_hash_matches_equality
);

prove_projection_id!(
    ProjectionElementId,
    projection_element_id_ordering_equality_matches_value_equality,
    projection_element_id_hash_matches_equality
);
prove_projection_id!(
    ProjectionRelationId,
    projection_relation_id_ordering_equality_matches_value_equality,
    projection_relation_id_hash_matches_equality
);
prove_projection_id!(
    ProjectionIncidenceId,
    projection_incidence_id_ordering_equality_matches_value_equality,
    projection_incidence_id_hash_matches_equality
);

/// Proves the optional-relation-type sentinel encoding round-trips for any
/// allocated id (ids start at 1, so the `0` sentinel never collides), and that
/// the absent case maps to and from the sentinel.
#[kani::proof]
fn wire_relation_type_roundtrip() {
    let raw: u64 = kani::any();
    kani::assume(raw >= 1);
    let id = RelationTypeId::new(raw);
    assert_eq!(
        wire::decode_relation_type(wire::encode_relation_type(Some(id))),
        Some(id)
    );
    assert_eq!(wire::encode_relation_type(None), wire::RELATION_TYPE_NONE);
    assert_eq!(wire::decode_relation_type(wire::RELATION_TYPE_NONE), None);
}

/// Proves the property-family tag mapping round-trips for every family and
/// rejects unknown tags.
#[kani::proof]
fn wire_property_family_tag_roundtrip() {
    for family in [
        PropertyFamily::Element,
        PropertyFamily::Relation,
        PropertyFamily::Incidence,
    ] {
        assert_eq!(
            wire::property_family_from_tag(wire::property_family_tag(family)),
            Some(family)
        );
    }
    let tag: u32 = kani::any();
    kani::assume(tag > 2);
    assert_eq!(wire::property_family_from_tag(tag), None);
}

/// Proves the property-value-type tag mapping round-trips for every type and
/// rejects unknown tags.
#[kani::proof]
fn wire_property_type_tag_roundtrip() {
    for value_type in [
        PropertyType::Boolean,
        PropertyType::Integer,
        PropertyType::Text,
    ] {
        assert_eq!(
            wire::property_type_from_tag(wire::property_type_tag(value_type)),
            Some(value_type)
        );
    }
    let tag: u32 = kani::any();
    kani::assume(tag > 2);
    assert_eq!(wire::property_type_from_tag(tag), None);
}

/// Proves the property-subject `(kind, id)` encoding round-trips for every
/// subject family and rejects unknown kinds.
#[kani::proof]
fn wire_subject_roundtrip() {
    let raw: u64 = kani::any();
    let subjects = [
        PropertySubject::Element(ElementId::new(raw)),
        PropertySubject::Relation(RelationId::new(raw)),
        PropertySubject::Incidence(IncidenceId::new(raw)),
    ];
    for subject in subjects {
        let (kind, id) = wire::encode_subject(subject);
        assert_eq!(wire::decode_subject(kind, id), Some(subject));
    }
    let kind: u32 = kani::any();
    kani::assume(kind > 2);
    assert_eq!(wire::decode_subject(kind, raw), None);
}

/// Proves the `flags` word packs and unpacks two 16-bit-range tags losslessly in
/// both positions, which the property and catalog ops rely on to recover their
/// subject kind / value tag / family / value type.
#[kani::proof]
fn wire_flags_pack_roundtrip() {
    let low: u32 = kani::any();
    let high: u32 = kani::any();
    kani::assume(low <= 0xFFFF);
    kani::assume(high <= 0xFFFF);
    assert_eq!(wire::unpack_flags(wire::pack_flags(low, high)), (low, high));
}

/// Builds one `MutationOp` from arbitrary fields.
fn any_op(op_kind: u32) -> wire::MutationOp {
    let flags: u32 = kani::any();
    let mut payload = [zerocopy::byteorder::U64::<zerocopy::byteorder::LE>::new(0);
        wire::MUTATION_OP_PAYLOAD_WORDS];
    let mut index = 0;
    while index < wire::MUTATION_OP_PAYLOAD_WORDS {
        let word: u64 = kani::any();
        payload[index] = zerocopy::byteorder::U64::new(word);
        index += 1;
    }
    wire::MutationOp {
        op_kind: op_kind.into(),
        flags: flags.into(),
        payload,
    }
}

/// Proves a `MutationOp` round-trips through zerocopy byte encoding for the
/// given op kind: writing it to bytes and reading it back yields an identical
/// op. Bounded over the fixed-size record, so the model checker terminates.
macro_rules! prove_op_roundtrip {
    ($name:ident, $op_kind:path) => {
        /// Proves the named op kind round-trips through byte encoding.
        ///
        /// The op's bytes are taken as the fixed-size [`IntoBytes`] slice (NOT a
        /// heap `Vec`): a `Vec` makes the slice length symbolic to the model
        /// checker, which then unwinds `memcmp` unboundedly. The slice length is
        /// `size_of::<MutationOp>()`, so the unwind bound below is constant.
        #[kani::proof]
        #[kani::unwind(72)]
        fn $name() {
            use zerocopy::{FromBytes, IntoBytes};
            let op = any_op($op_kind);
            let bytes = op.as_bytes();
            let Ok(decoded) = wire::MutationOp::read_from_bytes(bytes) else {
                assert!(false);
                return;
            };
            assert!(op == decoded);
            assert_eq!(decoded.op_kind.get(), $op_kind);
        }
    };
}

prove_op_roundtrip!(op_create_element_roundtrips, wire::OP_CREATE_ELEMENT);
prove_op_roundtrip!(op_tombstone_element_roundtrips, wire::OP_TOMBSTONE_ELEMENT);
prove_op_roundtrip!(op_create_relation_roundtrips, wire::OP_CREATE_RELATION);
prove_op_roundtrip!(
    op_tombstone_relation_roundtrips,
    wire::OP_TOMBSTONE_RELATION
);
prove_op_roundtrip!(op_create_incidence_roundtrips, wire::OP_CREATE_INCIDENCE);
prove_op_roundtrip!(
    op_tombstone_incidence_roundtrips,
    wire::OP_TOMBSTONE_INCIDENCE
);
prove_op_roundtrip!(op_set_relation_type_roundtrips, wire::OP_SET_RELATION_TYPE);
prove_op_roundtrip!(op_add_element_label_roundtrips, wire::OP_ADD_ELEMENT_LABEL);
prove_op_roundtrip!(
    op_add_relation_label_roundtrips,
    wire::OP_ADD_RELATION_LABEL
);
prove_op_roundtrip!(op_set_property_roundtrips, wire::OP_SET_PROPERTY);
prove_op_roundtrip!(op_remove_property_roundtrips, wire::OP_REMOVE_PROPERTY);
prove_op_roundtrip!(op_register_role_roundtrips, wire::OP_CATALOG_REGISTER_ROLE);
prove_op_roundtrip!(
    op_register_label_roundtrips,
    wire::OP_CATALOG_REGISTER_LABEL
);
prove_op_roundtrip!(
    op_register_relation_type_roundtrips,
    wire::OP_CATALOG_REGISTER_RELATION_TYPE
);
prove_op_roundtrip!(
    op_register_property_key_roundtrips,
    wire::OP_CATALOG_REGISTER_PROPERTY_KEY
);
prove_op_roundtrip!(
    op_register_projection_roundtrips,
    wire::OP_CATALOG_REGISTER_PROJECTION
);
prove_op_roundtrip!(
    op_register_index_roundtrips,
    wire::OP_CATALOG_REGISTER_INDEX
);
prove_op_roundtrip!(op_next_id_watermark_roundtrips, wire::OP_NEXT_ID_WATERMARK);

/// Proves the [`wire::LogRecordHeader`] framing contract that
/// [`crate::wal::encode_commit`] must satisfy: a header stamped with `len ==
/// header + op_count * op_size + blob_len` round-trips through zerocopy bytes
/// with every framing field intact, the appended op decodes verbatim, and the
/// blob byte survives. Bounded to one op and a one-byte blob.
// kani-skip: `encode_commit` computes the `crc32c` field via the external
// `crc32c` crate (a foreign function the model checker cannot evaluate), so this
// proof assembles the record framing directly — exactly as `encode_commit`
// lays it out, minus the CRC — and proves the len/op/blob mapping only. The CRC
// (check vector, single-bit flip) is covered by the crc unit tests, the real
// `encode_commit`/`replay` round-trip by the wal unit tests, and the unbounded
// multi-record replay walk by the wal proptest.
#[kani::proof]
#[kani::unwind(80)]
fn single_record_framing_consistent() {
    use zerocopy::{FromBytes, IntoBytes};
    let lsn: u64 = kani::any();
    let txn: u64 = kani::any();
    let generation: u64 = kani::any();
    let op = any_op(wire::OP_CREATE_ELEMENT);
    let byte: u8 = kani::any();

    let header_len = size_of::<wire::LogRecordHeader>();
    let op_len = size_of::<wire::MutationOp>();
    let total = header_len + op_len + 1;

    let header = wire::LogRecordHeader {
        base_generation: generation.into(),
        lsn: lsn.into(),
        txn_id: txn.into(),
        magic: wire::OXGLOGR.into(),
        len: (total as u32).into(),
        op_count: 1u32.into(),
        crc32c: 0u32.into(),
    };
    let mut record = Vec::with_capacity(total);
    record.extend_from_slice(header.as_bytes());
    record.extend_from_slice(op.as_bytes());
    record.push(byte);

    // len == header + one op + one blob byte.
    assert_eq!(record.len(), total);

    let Ok(parsed) = wire::LogRecordHeader::read_from_bytes(&record[..header_len]) else {
        assert!(false);
        return;
    };
    assert_eq!(parsed.len.get() as usize, record.len());
    assert_eq!(parsed.magic.get(), wire::OXGLOGR);
    assert_eq!(parsed.lsn.get(), lsn);
    assert_eq!(parsed.txn_id.get(), txn);
    assert_eq!(parsed.base_generation.get(), generation);
    assert_eq!(parsed.op_count.get(), 1);

    let Ok(decoded_op) =
        wire::MutationOp::read_from_bytes(&record[header_len..header_len + op_len])
    else {
        assert!(false);
        return;
    };
    assert!(decoded_op == op);
    assert_eq!(record[header_len + op_len], byte);
}

/// Proves the overlay merge is TOTAL and DUPLICATE-FREE over a bounded id
/// universe: for any base presence and overlay element entries over ids
/// `{1, 2, 3}`, the merged element iterator yields each id at most once, in
/// strictly ascending order, never yields a tombstoned id, and yields an id
/// exactly when it is visible (present in the overlay as a set value, or present
/// in the base with no overlay tombstone/override). Bounded to three ids so the
/// model checker terminates.
// CBMC-heavy (BTreeMap merge path blow-up); opt-in via `--features kani-heavy`.
// The merged-visible-set contract is verified fast by `merge_matches_oracle`.
#[cfg(feature = "kani-heavy")]
#[kani::proof]
#[kani::unwind(8)]
fn overlay_merge_is_total_and_duplicate_free() {
    use crate::overlay::{BaseRecords, MergedState, Overlay, StateView};

    // Arbitrary base presence for ids 1, 2, 3.
    let base_has: [bool; 3] = [kani::any(), kani::any(), kani::any()];
    // Arbitrary overlay opinion for ids 1, 2, 3: 0 = no opinion, 1 = set
    // (present), 2 = tombstone.
    let overlay_kind: [u8; 3] = [kani::any(), kani::any(), kani::any()];
    for kind in overlay_kind {
        kani::assume(kind <= 2);
    }

    let ids = [ElementId::new(1), ElementId::new(2), ElementId::new(3)];

    let base_ids: Vec<ElementId> = ids
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, id)| base_has[index].then_some(id))
        .collect();
    let base = BaseRecords::proof_elements(&base_ids);

    let overlay_entries: Vec<(ElementId, bool)> = ids
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, id)| match overlay_kind[index] {
            1 => Some((id, true)),
            2 => Some((id, false)),
            _no_opinion => None,
        })
        .collect();
    let overlay = Overlay::proof_element_entries(&overlay_entries);
    let view = MergedState::new(&base, &overlay);

    // Collect the merged element ids.
    let merged: Vec<ElementId> = view.elements().map(|record| record.id).collect();

    // Strictly ascending: each id appears at most once and in order.
    let mut index = 1;
    while index < merged.len() {
        assert!(merged[index - 1] < merged[index]);
        index += 1;
    }

    // Visibility equivalence: id visible iff (overlay set) or (base present and
    // overlay neither tombstones nor overrides it).
    let mut id_index = 0;
    while id_index < ids.len() {
        let id = ids[id_index];
        let overlay_set = overlay_kind[id_index] == 1;
        let overlay_tombstone = overlay_kind[id_index] == 2;
        let base_present = base_has[id_index];
        let expected_visible = overlay_set || (base_present && !overlay_tombstone);

        let in_merged = merged.iter().any(|candidate| *candidate == id);
        assert_eq!(in_merged, expected_visible);

        // A point read agrees with the iterator, resolves to the queried id when
        // present, and is absent for a tombstoned id.
        let point = view.element(id);
        assert_eq!(point.is_some(), expected_visible);
        assert_eq!(
            point.map(|record| record.id),
            expected_visible.then_some(id)
        );
        if overlay_tombstone {
            assert!(overlay.proof_is_element_tombstoned(id));
        }
        id_index += 1;
    }
}

/// Proves tombstone-masking idempotence at the element layer: tombstoning an
/// absent id makes it absent (a no-op against the visible set when the base
/// also lacks it), and tombstoning the same id twice yields the same single
/// tombstone and the same `None` read as tombstoning it once. Bounded to one id.
// CBMC-heavy (BTreeMap merge path blow-up); opt-in via `--features kani-heavy`.
// The tombstone-idempotence contract is exercised by `merge_matches_oracle`.
#[cfg(feature = "kani-heavy")]
#[kani::proof]
#[kani::unwind(6)]
fn overlay_tombstone_is_idempotent() {
    use crate::overlay::{BaseRecords, MergedState, Overlay, StateView, WriteOverlay};

    let raw: u64 = kani::any();
    kani::assume(raw >= 1);
    let id = ElementId::new(raw);
    let base_present: bool = kani::any();

    let base_ids: Vec<ElementId> = if base_present { vec![id] } else { Vec::new() };
    let base = BaseRecords::proof_elements(&base_ids);
    let next = Overlay::proof_element_entries(&[]).next_ids();

    // Single tombstone.
    let mut once = WriteOverlay::new(next, crate::Catalog::empty());
    once.tombstone_element(&base, id);
    let once = once.freeze();

    // Double tombstone.
    let mut twice = WriteOverlay::new(next, crate::Catalog::empty());
    twice.tombstone_element(&base, id);
    twice.tombstone_element(&base, id);
    let twice = twice.freeze();

    // Both hide the id and both record a tombstone.
    assert!(MergedState::new(&base, &once).element(id).is_none());
    assert!(MergedState::new(&base, &twice).element(id).is_none());
    assert!(once.proof_is_element_tombstoned(id));
    assert!(twice.proof_is_element_tombstoned(id));

    // Tombstoning an absent id is a no-op against the visible set: when the base
    // also lacks the id, the merged element set is empty either way.
    if !base_present {
        assert_eq!(MergedState::new(&base, &once).element_count(), 0);
    }
}

/// Proves the next-id watermark is strictly monotonic under
/// `Overlay::with_applied`: a child overlay built from a parent plus a writer
/// delta that allocates one element has a strictly greater `next_element`
/// watermark than the parent, and the rest of the watermark never regresses.
// CBMC-heavy (`with_applied` BTreeMap merge path blow-up); opt-in via
// `--features kani-heavy`. The watermark-monotonicity contract is exercised by
// `merge_matches_oracle` (which constructs child overlays via `with_applied`).
#[cfg(feature = "kani-heavy")]
#[kani::proof]
#[kani::unwind(4)]
fn overlay_watermark_monotonic_under_apply() {
    use crate::overlay::{Overlay, OverlayLayer, WriteOverlay};

    let raw: u64 = kani::any();
    // Leave headroom so the single allocation cannot overflow.
    kani::assume(raw >= 1 && raw < u64::MAX - 1);

    // Parent overlay whose watermark's element allocator is `raw`.
    let parent = Overlay::proof_with_next_element(raw);

    let mut child_write = WriteOverlay::new(parent.next_ids(), parent.catalog().clone());
    let created = child_write.create_element().expect("create");
    assert_eq!(created.get(), raw);
    let child = parent.with_applied(&child_write);

    // Strictly greater element watermark, exactly one beyond.
    assert!(child.next_ids().element > parent.next_ids().element);
    assert_eq!(child.next_ids().element.get(), raw + 1);

    // The other allocators never regress.
    assert_eq!(child.next_ids().relation, parent.next_ids().relation);
    assert_eq!(child.next_ids().role, parent.next_ids().role);
}

/// Proves `PropertyValue::try_from::<u64>` narrows exactly when the value fits
/// `i64`, agreeing with the checked standard conversion (and never panicking).
///
/// The assertion compares the narrowed `i64` scalar, NOT the `PropertyValue`:
/// `PropertyValue`'s derived `PartialEq` includes its `Text(String)` arm, which
/// the model checker unwinds as an unbounded `memcmp` over a symbolic-length
/// string — even though this conversion only ever yields `Integer`/`None`.
#[kani::proof]
fn property_value_try_from_u64_matches_checked() {
    let raw: u64 = kani::any();
    match PropertyValue::try_from(raw) {
        Ok(PropertyValue::Integer(got)) => assert_eq!(Some(got), i64::try_from(raw).ok()),
        Ok(_) => assert!(false, "u64 conversion must yield Integer or an error"),
        Err(_) => assert!(i64::try_from(raw).is_err()),
    }
}

/// Proves `PropertyValue::as_count` agrees with the checked `usize` conversion
/// for every `i64` (total: it never panics and only narrows when in range).
#[kani::proof]
fn property_value_as_count_matches_checked() {
    let raw: i64 = kani::any();
    assert_eq!(
        PropertyValue::Integer(raw).as_count(),
        usize::try_from(raw).ok()
    );
}
