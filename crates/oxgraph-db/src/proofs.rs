//! Kani proofs for small database algebra contracts.

use core::hash::{Hash, Hasher};

use crate::{
    CheckpointGeneration, CommitSeq, ElementId, IncidenceId, IndexId, LabelId, ProjectionElementId,
    ProjectionId, ProjectionIncidenceId, ProjectionRelationId, PropertyKeyId, RelationId,
    RelationTypeId, RoleId, TransactionId,
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
