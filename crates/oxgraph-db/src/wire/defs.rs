//! The single projection/index definition-body codec.
//!
//! A catalog definition body is a run of `u64` words with a `u32` kind
//! discriminant. The SAME logical layout is persisted in two places — interned
//! into a WAL frame blob by the writer's mutation log, and packed into the
//! base's `SECTION_CATALOG_DEFS` shared run by freeze — so this module is the
//! single source of truth for the discriminants and the word sequences. Each
//! consumer owns only its container concerns (blob slicing and `LogCorrupt`
//! mapping in the log; `DefWire` record slicing and `InvalidStore` mapping in
//! freeze).

use crate::{
    LabelId, ProjectionId, PropertyKeyId, RelationTypeId, RoleId,
    catalog::{
        GraphProjectionDefinition, HypergraphProjectionDefinition, IndexDefinition,
        ProjectionDefinition,
    },
};

/// Definition-body discriminant for a binary graph projection.
pub(crate) const DEF_PROJECTION_GRAPH: u32 = 0;
/// Definition-body discriminant for a hypergraph projection.
pub(crate) const DEF_PROJECTION_HYPER: u32 = 1;
/// Definition-body discriminant for a label membership index.
pub(crate) const DEF_INDEX_LABEL: u32 = 0;
/// Definition-body discriminant for a relation-type membership index.
pub(crate) const DEF_INDEX_RELATION_TYPE: u32 = 1;
/// Definition-body discriminant for a single-key equality index.
pub(crate) const DEF_INDEX_PROPERTY_EQUALITY: u32 = 2;
/// Definition-body discriminant for a single-key range index.
pub(crate) const DEF_INDEX_PROPERTY_RANGE: u32 = 3;
/// Definition-body discriminant for a composite equality index.
pub(crate) const DEF_INDEX_COMPOSITE_EQUALITY: u32 = 4;
/// Definition-body discriminant for a projection-materialization index.
pub(crate) const DEF_INDEX_PROJECTION: u32 = 5;

/// A definition-body decode failure: the discriminant is unknown or the word
/// run is malformed. Consumers wrap the reason into their own error surface
/// ([`crate::DbError::LogCorrupt`] on the WAL path,
/// [`crate::DbError::InvalidStore`] on the base path).
///
/// # Performance
///
/// Copying this value is `O(1)`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DefDecodeError {
    /// Static description of the malformation.
    pub(crate) reason: &'static str,
}

/// Builds a decode error from its reason.
///
/// # Performance
///
/// This function is `O(1)`.
const fn malformed(reason: &'static str) -> DefDecodeError {
    DefDecodeError { reason }
}

/// Pushes a length-prefixed id set into a definition-body word run.
///
/// # Performance
///
/// This function is `O(set size)`.
fn push_id_set(words: &mut Vec<u64>, ids: impl ExactSizeIterator<Item = u64>) {
    words.push(ids.len() as u64);
    words.extend(ids);
}

/// Reads a length-prefixed id set from a definition-body word run at `cursor`,
/// advancing it past the set.
///
/// # Errors
///
/// Returns [`DefDecodeError`] when the length or slice is out of bounds.
///
/// # Performance
///
/// This function is `O(set size)`.
fn read_id_set(words: &[u64], cursor: &mut usize) -> Result<Vec<u64>, DefDecodeError> {
    let count = usize::try_from(
        *words
            .get(*cursor)
            .ok_or(malformed("def missing id-set length"))?,
    )
    .map_err(|_overflow| malformed("def id-set length overflow"))?;
    *cursor += 1;
    let end = cursor
        .checked_add(count)
        .ok_or(malformed("def id-set overflow"))?;
    let slice = words
        .get(*cursor..end)
        .ok_or(malformed("def id-set out of bounds"))?;
    let ids = slice.to_vec();
    *cursor = end;
    Ok(ids)
}

/// Encodes a projection definition body into a `u64` word run, returning the
/// `(discriminant, words)` both persistence paths record.
///
/// # Performance
///
/// This function is `O(definition size)`.
pub(crate) fn encode_projection_body(definition: &ProjectionDefinition) -> (u32, Vec<u64>) {
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
/// `(discriminant, words)` both persistence paths record.
///
/// # Performance
///
/// This function is `O(definition size)`.
pub(crate) fn encode_index_body(definition: &IndexDefinition) -> (u32, Vec<u64>) {
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

/// Decodes a projection definition from its discriminant, name, and body words.
///
/// # Errors
///
/// Returns [`DefDecodeError`] when the discriminant is unknown or the body is
/// malformed.
///
/// # Performance
///
/// This function is `O(definition size)`.
pub(crate) fn decode_projection_body(
    discriminant: u32,
    name: String,
    words: &[u64],
) -> Result<ProjectionDefinition, DefDecodeError> {
    match discriminant {
        DEF_PROJECTION_GRAPH => {
            let source_role = *words
                .first()
                .ok_or(malformed("graph def missing source role"))?;
            let target_role = *words
                .get(1)
                .ok_or(malformed("graph def missing target role"))?;
            let mut cursor = 2;
            let relation_types = read_id_set(words, &mut cursor)?
                .into_iter()
                .map(RelationTypeId::new)
                .collect();
            Ok(ProjectionDefinition::Graph(GraphProjectionDefinition {
                name,
                relation_types,
                source_role: RoleId::new(source_role),
                target_role: RoleId::new(target_role),
            }))
        }
        DEF_PROJECTION_HYPER => {
            let mut cursor = 0;
            let source_roles = read_id_set(words, &mut cursor)?
                .into_iter()
                .map(RoleId::new)
                .collect();
            let target_roles = read_id_set(words, &mut cursor)?
                .into_iter()
                .map(RoleId::new)
                .collect();
            let relation_types = read_id_set(words, &mut cursor)?
                .into_iter()
                .map(RelationTypeId::new)
                .collect();
            Ok(ProjectionDefinition::Hypergraph(
                HypergraphProjectionDefinition {
                    name,
                    relation_types,
                    source_roles,
                    target_roles,
                },
            ))
        }
        _other => Err(malformed("unknown projection definition kind")),
    }
}

/// Decodes an index definition from its discriminant and body words.
///
/// # Errors
///
/// Returns [`DefDecodeError`] when the discriminant is unknown or the body is
/// malformed.
///
/// # Performance
///
/// This function is `O(definition size)`.
pub(crate) fn decode_index_body(
    discriminant: u32,
    words: &[u64],
) -> Result<IndexDefinition, DefDecodeError> {
    let first = || {
        words
            .first()
            .copied()
            .ok_or(malformed("index def missing id"))
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
        _other => Err(malformed("unknown index definition kind")),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// Strategy producing arbitrary projection definitions.
    fn projection_strategy() -> impl Strategy<Value = ProjectionDefinition> {
        let ids = proptest::collection::btree_set(0u64..1_000, 0..8);
        let graph = (any::<u64>(), any::<u64>(), ids.clone()).prop_map(
            |(source, target, relation_types)| {
                ProjectionDefinition::Graph(GraphProjectionDefinition {
                    name: "p".to_owned(),
                    relation_types: relation_types
                        .into_iter()
                        .map(RelationTypeId::new)
                        .collect(),
                    source_role: RoleId::new(source),
                    target_role: RoleId::new(target),
                })
            },
        );
        let hyper = (ids.clone(), ids.clone(), ids).prop_map(|(sources, targets, types)| {
            ProjectionDefinition::Hypergraph(HypergraphProjectionDefinition {
                name: "p".to_owned(),
                relation_types: types.into_iter().map(RelationTypeId::new).collect(),
                source_roles: sources.into_iter().map(RoleId::new).collect(),
                target_roles: targets.into_iter().map(RoleId::new).collect(),
            })
        });
        prop_oneof![graph, hyper]
    }

    /// Strategy producing arbitrary index definitions.
    fn index_strategy() -> impl Strategy<Value = IndexDefinition> {
        prop_oneof![
            any::<u64>().prop_map(|id| IndexDefinition::Label {
                label: LabelId::new(id)
            }),
            any::<u64>().prop_map(|id| IndexDefinition::RelationType {
                relation_type: RelationTypeId::new(id)
            }),
            any::<u64>().prop_map(|id| IndexDefinition::PropertyEquality {
                key: PropertyKeyId::new(id)
            }),
            any::<u64>().prop_map(|id| IndexDefinition::PropertyRange {
                key: PropertyKeyId::new(id)
            }),
            proptest::collection::vec(any::<u64>(), 1..6).prop_map(|keys| {
                IndexDefinition::CompositeEquality {
                    keys: keys.into_iter().map(PropertyKeyId::new).collect(),
                }
            }),
            any::<u64>().prop_map(|id| IndexDefinition::Projection {
                projection: ProjectionId::new(id)
            }),
        ]
    }

    proptest! {
        /// Encode→decode is the identity for every projection definition.
        #[test]
        fn projection_body_roundtrips(definition in projection_strategy()) {
            let (kind, words) = encode_projection_body(&definition);
            let decoded = decode_projection_body(kind, "p".to_owned(), &words)
                .expect("well-formed body decodes");
            prop_assert_eq!(decoded, definition);
        }

        /// Encode→decode is the identity for every index definition.
        #[test]
        fn index_body_roundtrips(definition in index_strategy()) {
            let (kind, words) = encode_index_body(&definition);
            let decoded = decode_index_body(kind, &words).expect("well-formed body decodes");
            prop_assert_eq!(decoded, definition);
        }
    }
}
