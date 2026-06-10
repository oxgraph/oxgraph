//! Owned read-side views: whole-record reads with typed property access.
//!
//! [`Reader::element`](crate::Reader::element) and
//! [`Reader::relation`](crate::Reader::relation) return these owned views in one
//! call — id, labels, and every property together — replacing per-key `Cow`
//! reads. Property access is typed through [`Key<T>`](crate::Key): a read of the
//! wrong Rust type is a compile error.
//!
//! # Performance
//!
//! Building a view is `O(label count + property count)` for the subject; typed
//! property access is `O(log property count)`.

use std::collections::BTreeMap;

use crate::{
    ElementId, LabelId, PropertyKeyId, PropertyValue, RelationId, RelationTypeId,
    error::DbError,
    typed::{Key, Readable, ValueType},
};

/// A typed, owned property bag for one subject.
///
/// # Performance
///
/// `get`/`require` are `O(log property count)`; `value` is `O(log property
/// count)`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Properties {
    /// Visible property values keyed by property key id, in key order.
    values: BTreeMap<PropertyKeyId, PropertyValue>,
}

impl Properties {
    /// Builds a property bag from `(key, value)` pairs.
    ///
    /// # Performance
    ///
    /// This function is `O(n log n)` in the pair count.
    pub(crate) fn from_pairs(
        pairs: impl IntoIterator<Item = (PropertyKeyId, PropertyValue)>,
    ) -> Self {
        Self {
            values: pairs.into_iter().collect(),
        }
    }

    /// Reads a typed property, or `None` when absent or type-mismatched.
    ///
    /// # Performance
    ///
    /// This method is `O(log property count)` (plus `O(value length)` for text).
    #[must_use]
    pub fn get<T: ValueType, V: Readable<T>>(&self, key: Key<T>) -> Option<V> {
        self.values.get(&key.id()).and_then(V::read)
    }

    /// Reads a required typed property.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::MissingProperty`] when the property is absent.
    ///
    /// # Performance
    ///
    /// This method is `O(log property count)`.
    pub fn require<T: ValueType, V: Readable<T>>(&self, key: Key<T>) -> Result<V, DbError> {
        self.get(key).ok_or_else(|| {
            DbError::Query(crate::error::QueryError::MissingProperty { key: key.id() })
        })
    }

    /// Returns the raw value for an untyped key.
    ///
    /// # Performance
    ///
    /// This method is `O(log property count)`.
    #[must_use]
    pub fn value(&self, key: PropertyKeyId) -> Option<&PropertyValue> {
        self.values.get(&key)
    }

    /// Returns the number of visible properties.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether there are no visible properties.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Iterates `(key, value)` pairs in ascending key order.
    ///
    /// # Performance
    ///
    /// Iteration is `O(property count)`.
    pub fn iter(&self) -> impl Iterator<Item = (PropertyKeyId, &PropertyValue)> {
        self.values.iter().map(|(key, value)| (*key, value))
    }
}

/// An owned element view: id, labels, and all properties in one read.
///
/// # Performance
///
/// Cloning is `O(label count + property count)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Element {
    /// Canonical element id.
    pub id: ElementId,
    /// Labels assigned to this element, ascending.
    pub labels: Vec<LabelId>,
    /// The element's property bag.
    properties: Properties,
}

impl Element {
    /// Assembles an owned element view.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn new(id: ElementId, labels: Vec<LabelId>, properties: Properties) -> Self {
        Self {
            id,
            labels,
            properties,
        }
    }

    /// Returns whether this element carries `label`.
    ///
    /// # Performance
    ///
    /// This method is `O(log label count)`.
    #[must_use]
    pub fn has(&self, label: LabelId) -> bool {
        self.labels.binary_search(&label).is_ok()
    }

    /// Returns this element's property bag.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn properties(&self) -> &Properties {
        &self.properties
    }
}

/// An owned relation view: id, type, labels, and all properties in one read.
///
/// # Performance
///
/// Cloning is `O(label count + property count)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Relation {
    /// Canonical relation id.
    pub id: RelationId,
    /// The relation's type, if set.
    pub relation_type: Option<RelationTypeId>,
    /// Labels assigned to this relation, ascending.
    pub labels: Vec<LabelId>,
    /// The relation's property bag.
    properties: Properties,
}

impl Relation {
    /// Assembles an owned relation view.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    pub(crate) const fn new(
        id: RelationId,
        relation_type: Option<RelationTypeId>,
        labels: Vec<LabelId>,
        properties: Properties,
    ) -> Self {
        Self {
            id,
            relation_type,
            labels,
            properties,
        }
    }

    /// Returns whether this relation carries `label`.
    ///
    /// # Performance
    ///
    /// This method is `O(log label count)`.
    #[must_use]
    pub fn has(&self, label: LabelId) -> bool {
        self.labels.binary_search(&label).is_ok()
    }

    /// Returns this relation's property bag.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    #[must_use]
    pub const fn properties(&self) -> &Properties {
        &self.properties
    }
}
