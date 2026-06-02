//! Kani proofs for small database algebra contracts.

use core::hash::{Hash, Hasher};

use crate::{
    CheckpointGeneration, CommitSeq, ElementId, IncidenceId, IndexId, LabelId, ProjectionElementId,
    ProjectionId, ProjectionIncidenceId, ProjectionRelationId, PropertyFamily, PropertyKeyId,
    PropertySubject, PropertyType, RelationId, RelationTypeId, RoleId, TransactionId, wire,
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
