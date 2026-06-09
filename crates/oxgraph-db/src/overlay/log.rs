//! The writer's replayable mutation log and the WAL definition-body codec.
//!
//! [`MutationLog`] captures every mutation a writer applies, in order, as
//! fixed [`MutationOp`] records plus an interned blob for variable-length
//! names/text; the commit path serializes it verbatim into the WAL frame. The
//! codec functions below encode and decode the projection/index definition
//! bodies those ops reference.

use zerocopy::byteorder::{LE, U64};

use crate::{
    DbError, LabelId, ProjectionId, PropertyKeyId, RelationTypeId, RoleId,
    catalog::{IndexDefinition, ProjectionDefinition},
    state::NextIds,
    wire::{self, MUTATION_OP_PAYLOAD_WORDS, MutationOp},
};

/// An ordered, replayable record of every mutation a writer applied, captured
/// AS each mutation happens so the WAL frame replays into a byte-identical
/// overlay. Each entry is a fixed [`MutationOp`]; variable-length names/text are
/// interned into [`Self::blob`] and referenced by `(offset, len)` payload words.
///
/// The writer records ops in two places — into its [`WriteOverlay`] maps (for
/// in-memory reads and the published overlay) and into this log (for the WAL) —
/// and the commit path serializes this log verbatim. Recovery decodes the same
/// ops and re-applies them through the same [`WriteOverlay`] mutators, so the
/// replayed overlay equals the committed one.
///
/// # Performance
///
/// Each push is `O(1)` amortized; interning a name is `O(name.len())`.
#[derive(Clone, Debug, Default)]
pub(crate) struct MutationLog {
    /// Mutation ops in application order.
    pub(super) ops: Vec<MutationOp>,
    /// Interned UTF-8 names/text referenced by `(offset, len)` payload words.
    pub(super) blob: Vec<u8>,
}

impl MutationLog {
    /// Returns whether any op has been recorded (a non-dirty writer logs none).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(crate) const fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Interns a name/text value into the blob, returning its `(offset, len)` as
    /// `u64` payload words. Both fit `u64` for any in-memory buffer, so this is
    /// infallible; the overall frame `len` is `u32`-checked at append time.
    ///
    /// # Performance
    ///
    /// This method is `O(value.len())`.
    pub(super) fn intern(&mut self, value: &[u8]) -> (u64, u64) {
        let offset = self.blob.len() as u64;
        self.blob.extend_from_slice(value);
        let len = value.len() as u64;
        (offset, len)
    }

    /// Interns a `u64` definition-body word run into the blob as little-endian
    /// bytes, returning the byte `(offset, len)` of the run.
    ///
    /// # Performance
    ///
    /// This method is `O(words.len())`.
    pub(super) fn intern_words(&mut self, words: &[u64]) -> (u64, u64) {
        let offset = self.blob.len() as u64;
        for word in words {
            self.blob.extend_from_slice(&word.to_le_bytes());
        }
        let len = size_of_val(words) as u64;
        (offset, len)
    }

    /// Records one op with the given kind, packed flags, and leading payload
    /// words (remaining words zero).
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(super) fn push(&mut self, op_kind: u32, flags: u32, words: &[u64]) {
        let mut payload = [U64::<LE>::new(0); MUTATION_OP_PAYLOAD_WORDS];
        for (slot, value) in payload.iter_mut().zip(words) {
            *slot = U64::new(*value);
        }
        self.ops.push(MutationOp {
            op_kind: op_kind.into(),
            flags: flags.into(),
            payload,
        });
    }

    /// Appends the nine-value next-id watermark as the final op of a dirty
    /// frame, so recovery restores allocators without recomputing them.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    pub(super) fn push_watermark(&mut self, next: NextIds) {
        self.push(
            wire::OP_NEXT_ID_WATERMARK,
            0,
            &[
                next.element.get(),
                next.relation.get(),
                next.incidence.get(),
                next.role.get(),
                next.label.get(),
                next.relation_type.get(),
                next.property_key.get(),
                next.projection.get(),
                next.index.get(),
            ],
        );
    }
}

/// Reads a `(offset, len)` UTF-8 slice from a replay frame blob.
///
/// # Errors
///
/// Returns [`DbError::LogCorrupt`] when the slice is out of bounds or not UTF-8.
///
/// # Performance
///
/// This function is `O(len)`.
pub(super) fn blob_str(blob: &[u8], offset: u64, len: u64, lsn: u64) -> Result<String, DbError> {
    let start = usize::try_from(offset).map_err(|_overflow| DbError::LogCorrupt {
        lsn,
        reason: "blob offset overflow",
    })?;
    let length = usize::try_from(len).map_err(|_overflow| DbError::LogCorrupt {
        lsn,
        reason: "blob length overflow",
    })?;
    let end = start.checked_add(length).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "blob slice overflow",
    })?;
    let bytes = blob.get(start..end).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "blob slice out of bounds",
    })?;
    core::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_error| DbError::LogCorrupt {
            lsn,
            reason: "blob slice is not UTF-8",
        })
}

/// Definition-body discriminant for a binary graph projection (stamped in the
/// `OP_CATALOG_REGISTER_PROJECTION` op `flags`).
const DEF_PROJECTION_GRAPH: u32 = 0;
/// Definition-body discriminant for a hypergraph projection.
const DEF_PROJECTION_HYPER: u32 = 1;
/// Definition-body discriminant for a label membership index.
const DEF_INDEX_LABEL: u32 = 0;
/// Definition-body discriminant for a relation-type membership index.
const DEF_INDEX_RELATION_TYPE: u32 = 1;
/// Definition-body discriminant for a single-key equality index.
const DEF_INDEX_PROPERTY_EQUALITY: u32 = 2;
/// Definition-body discriminant for a single-key range index.
const DEF_INDEX_PROPERTY_RANGE: u32 = 3;
/// Definition-body discriminant for a composite equality index.
const DEF_INDEX_COMPOSITE_EQUALITY: u32 = 4;
/// Definition-body discriminant for a projection-materialization index.
const DEF_INDEX_PROJECTION: u32 = 5;

/// Pushes a length-prefixed id set into a definition-body word run.
///
/// # Performance
///
/// This function is `O(set size)`.
fn push_id_set(words: &mut Vec<u64>, ids: impl ExactSizeIterator<Item = u64>) {
    words.push(ids.len() as u64);
    words.extend(ids);
}

/// Encodes a projection definition body into a `u64` word run, returning the
/// `(discriminant, words)` the WAL op records.
///
/// # Performance
///
/// This function is `O(definition size)`.
pub(super) fn encode_projection_words(definition: &ProjectionDefinition) -> (u32, Vec<u64>) {
    let mut words = Vec::new();
    match definition {
        ProjectionDefinition::Graph(graph) => {
            words.push(graph.source_role.get());
            words.push(graph.target_role.get());
            push_id_set(&mut words, graph.relation_types.iter().map(|id| id.get()));
            (DEF_PROJECTION_GRAPH, words)
        }
        ProjectionDefinition::Hypergraph(hyper) => {
            push_id_set(&mut words, hyper.source_roles.iter().map(|id| id.get()));
            push_id_set(&mut words, hyper.target_roles.iter().map(|id| id.get()));
            push_id_set(&mut words, hyper.relation_types.iter().map(|id| id.get()));
            (DEF_PROJECTION_HYPER, words)
        }
    }
}

/// Encodes an index definition body into a `u64` word run, returning the
/// `(discriminant, words)` the WAL op records.
///
/// # Performance
///
/// This function is `O(definition size)`.
pub(super) fn encode_index_words(definition: &IndexDefinition) -> (u32, Vec<u64>) {
    match definition {
        IndexDefinition::Label { label } => (DEF_INDEX_LABEL, vec![label.get()]),
        IndexDefinition::RelationType { relation_type } => {
            (DEF_INDEX_RELATION_TYPE, vec![relation_type.get()])
        }
        IndexDefinition::PropertyEquality { key } => (DEF_INDEX_PROPERTY_EQUALITY, vec![key.get()]),
        IndexDefinition::PropertyRange { key } => (DEF_INDEX_PROPERTY_RANGE, vec![key.get()]),
        IndexDefinition::CompositeEquality { keys } => (
            DEF_INDEX_COMPOSITE_EQUALITY,
            keys.iter().map(|key| key.get()).collect(),
        ),
        IndexDefinition::Projection { projection } => {
            (DEF_INDEX_PROJECTION, vec![projection.get()])
        }
    }
}

/// Decodes the `(offset, len)` byte run of a definition body into its `u64`
/// words.
///
/// # Errors
///
/// Returns [`DbError::LogCorrupt`] when the run is out of bounds or not a whole
/// number of `u64` words.
///
/// # Performance
///
/// This function is `O(len)`.
pub(super) fn decode_def_words(
    blob: &[u8],
    offset: u64,
    len: u64,
    lsn: u64,
) -> Result<Vec<u64>, DbError> {
    let start = usize::try_from(offset).map_err(|_overflow| DbError::LogCorrupt {
        lsn,
        reason: "def offset overflow",
    })?;
    let length = usize::try_from(len).map_err(|_overflow| DbError::LogCorrupt {
        lsn,
        reason: "def length overflow",
    })?;
    let end = start.checked_add(length).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "def slice overflow",
    })?;
    let bytes = blob.get(start..end).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "def slice out of bounds",
    })?;
    if !bytes.len().is_multiple_of(size_of::<u64>()) {
        return Err(DbError::LogCorrupt {
            lsn,
            reason: "def slice is not whole u64 words",
        });
    }
    Ok(bytes
        .chunks_exact(size_of::<u64>())
        .map(|chunk| {
            let mut word = [0u8; 8];
            word.copy_from_slice(chunk);
            u64::from_le_bytes(word)
        })
        .collect())
}

/// Reads a length-prefixed id set from a definition-body word run at `cursor`,
/// advancing it past the set.
///
/// # Errors
///
/// Returns [`DbError::LogCorrupt`] when the length or slice is out of bounds.
///
/// # Performance
///
/// This function is `O(set size)`.
fn read_id_set(words: &[u64], cursor: &mut usize, lsn: u64) -> Result<Vec<u64>, DbError> {
    let count = usize::try_from(*words.get(*cursor).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "def missing id-set length",
    })?)
    .map_err(|_overflow| DbError::LogCorrupt {
        lsn,
        reason: "def id-set length overflow",
    })?;
    *cursor += 1;
    let end = cursor.checked_add(count).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "def id-set overflow",
    })?;
    let slice = words.get(*cursor..end).ok_or(DbError::LogCorrupt {
        lsn,
        reason: "def id-set out of bounds",
    })?;
    let ids = slice.to_vec();
    *cursor = end;
    Ok(ids)
}

/// Decodes a projection definition from a replay op's `flags` discriminant,
/// name, and body words.
///
/// # Errors
///
/// Returns [`DbError::LogCorrupt`] when the discriminant is unknown or the body
/// is malformed.
///
/// # Performance
///
/// This function is `O(definition size)`.
pub(super) fn decode_projection_def(
    discriminant: u32,
    name: String,
    words: &[u64],
    lsn: u64,
) -> Result<ProjectionDefinition, DbError> {
    match discriminant {
        DEF_PROJECTION_GRAPH => {
            let source_role = *words.first().ok_or(DbError::LogCorrupt {
                lsn,
                reason: "graph def missing source role",
            })?;
            let target_role = *words.get(1).ok_or(DbError::LogCorrupt {
                lsn,
                reason: "graph def missing target role",
            })?;
            let mut cursor = 2;
            let relation_types = read_id_set(words, &mut cursor, lsn)?
                .into_iter()
                .map(RelationTypeId::new)
                .collect();
            Ok(ProjectionDefinition::Graph(
                crate::catalog::GraphProjectionDefinition {
                    name,
                    relation_types,
                    source_role: RoleId::new(source_role),
                    target_role: RoleId::new(target_role),
                },
            ))
        }
        DEF_PROJECTION_HYPER => {
            let mut cursor = 0;
            let source_roles = read_id_set(words, &mut cursor, lsn)?
                .into_iter()
                .map(RoleId::new)
                .collect();
            let target_roles = read_id_set(words, &mut cursor, lsn)?
                .into_iter()
                .map(RoleId::new)
                .collect();
            let relation_types = read_id_set(words, &mut cursor, lsn)?
                .into_iter()
                .map(RelationTypeId::new)
                .collect();
            Ok(ProjectionDefinition::Hypergraph(
                crate::catalog::HypergraphProjectionDefinition {
                    name,
                    relation_types,
                    source_roles,
                    target_roles,
                },
            ))
        }
        _other => Err(DbError::LogCorrupt {
            lsn,
            reason: "unknown projection definition kind",
        }),
    }
}

/// Decodes an index definition from a replay op's `flags` discriminant and body
/// words.
///
/// # Errors
///
/// Returns [`DbError::LogCorrupt`] when the discriminant is unknown or the body
/// is malformed.
///
/// # Performance
///
/// This function is `O(definition size)`.
pub(super) fn decode_index_def(
    discriminant: u32,
    words: &[u64],
    lsn: u64,
) -> Result<IndexDefinition, DbError> {
    let first = || {
        words.first().copied().ok_or(DbError::LogCorrupt {
            lsn,
            reason: "index def missing id",
        })
    };
    match discriminant {
        DEF_INDEX_LABEL => Ok(IndexDefinition::Label {
            label: LabelId::new(first()?),
        }),
        DEF_INDEX_RELATION_TYPE => Ok(IndexDefinition::RelationType {
            relation_type: RelationTypeId::new(first()?),
        }),
        DEF_INDEX_PROPERTY_EQUALITY => Ok(IndexDefinition::PropertyEquality {
            key: PropertyKeyId::new(first()?),
        }),
        DEF_INDEX_PROPERTY_RANGE => Ok(IndexDefinition::PropertyRange {
            key: PropertyKeyId::new(first()?),
        }),
        DEF_INDEX_COMPOSITE_EQUALITY => Ok(IndexDefinition::CompositeEquality {
            keys: words.iter().map(|word| PropertyKeyId::new(*word)).collect(),
        }),
        DEF_INDEX_PROJECTION => Ok(IndexDefinition::Projection {
            projection: ProjectionId::new(first()?),
        }),
        _other => Err(DbError::LogCorrupt {
            lsn,
            reason: "unknown index definition kind",
        }),
    }
}
