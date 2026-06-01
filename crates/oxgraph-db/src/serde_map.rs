//! Shared serde adapters for `BTreeMap` collections keyed by non-string IDs.
//!
//! `serde_json` object keys must be strings, so the catalog and state modules
//! persist their ID-keyed maps as ordered `(key, value)` entry arrays. This
//! module hoists the single shared adapter pair so both modules reference one
//! copy via `#[serde(with = "crate::serde_map")]`. The nested property map in
//! [`crate::state`] reuses the same entry-array shape through the [`nested`]
//! submodule.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Serializes a map as an ordered entry array.
///
/// # Performance
///
/// This function is `O(n)` for `n` entries plus one borrowing allocation.
pub(crate) fn serialize<S, K, V>(map: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    K: Serialize,
    V: Serialize,
{
    Serialize::serialize(&map.iter().collect::<Vec<_>>(), serializer)
}

/// Deserializes a map from an ordered entry array.
///
/// # Performance
///
/// This function is `O(n log n)` for `n` entries inserted into the map.
pub(crate) fn deserialize<'de, D, K, V>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    K: Ord + DeserializeOwned,
    V: DeserializeOwned,
{
    <Vec<(K, V)> as Deserialize>::deserialize(deserializer)
        .map(|entries| entries.into_iter().collect())
}

/// Shared serde adapter for maps whose values are themselves ID-keyed maps.
///
/// The outer and inner maps both persist as ordered entry arrays, so the nested
/// shape reuses the flat [`serialize`] / [`deserialize`] contract one level
/// deeper.
pub(crate) mod nested {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Serialize, de::DeserializeOwned};

    /// A map whose values are themselves ID-keyed maps.
    type NestedMap<K, IK, IV> = BTreeMap<K, BTreeMap<IK, IV>>;

    /// Serializes a nested map as ordered entry arrays at both levels.
    ///
    /// # Performance
    ///
    /// This function is `O(n)` for `n` total entries plus borrowing
    /// allocations per nested map.
    pub(crate) fn serialize<S, K, IK, IV>(
        map: &NestedMap<K, IK, IV>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
        K: Serialize,
        IK: Serialize,
        IV: Serialize,
    {
        let entries = map
            .iter()
            .map(|(key, values)| (key, values.iter().collect::<Vec<_>>()))
            .collect::<Vec<_>>();
        Serialize::serialize(&entries, serializer)
    }

    /// Deserializes a nested map from ordered entry arrays at both levels.
    ///
    /// # Performance
    ///
    /// This function is `O(n log n)` for `n` total entries.
    pub(crate) fn deserialize<'de, D, K, IK, IV>(
        deserializer: D,
    ) -> Result<NestedMap<K, IK, IV>, D::Error>
    where
        D: serde::Deserializer<'de>,
        K: Ord + DeserializeOwned,
        IK: Ord + DeserializeOwned,
        IV: DeserializeOwned,
    {
        <Vec<(K, Vec<(IK, IV)>)> as Deserialize>::deserialize(deserializer).map(|entries| {
            entries
                .into_iter()
                .map(|(key, values)| (key, values.into_iter().collect()))
                .collect()
        })
    }
}
