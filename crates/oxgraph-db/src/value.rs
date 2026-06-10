//! Typed property and query values.

use std::{fmt, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::error::DbError;

/// Supported scalar property types.
///
/// # Performance
///
/// Copying and comparing a type tag are `O(1)`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PropertyType {
    /// Boolean property.
    Boolean,
    /// Signed 64-bit integer property.
    Integer,
    /// UTF-8 text property.
    Text,
}

/// One typed property value.
///
/// Text is held as a shared `Arc<str>`, so a value cloned across overlay
/// generations (the writer seeds from the parent's delta maps) shares one
/// allocation instead of copying the bytes. Ordering, equality, hashing, and
/// the serde representation are unchanged from the owned-`String` form: they
/// all delegate to the underlying `str`.
///
/// # Performance
///
/// Cloning is `O(1)` (text clones an `Arc`); comparing and hashing are
/// `O(value length)` for text and `O(1)` otherwise.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PropertyValue {
    /// Boolean value.
    Boolean(bool),
    /// Signed integer value.
    Integer(i64),
    /// UTF-8 text value, shared by reference count.
    Text(Arc<str>),
}

impl PropertyValue {
    /// Returns this value's type tag.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn value_type(&self) -> PropertyType {
        match self {
            Self::Boolean(_value) => PropertyType::Boolean,
            Self::Integer(_value) => PropertyType::Integer,
            Self::Text(_value) => PropertyType::Text,
        }
    }

    /// Returns the text payload, or `None` for a non-text value.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value.as_ref()),
            Self::Boolean(_) | Self::Integer(_) => None,
        }
    }

    /// Returns the integer payload, or `None` for a non-integer value.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn as_int(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            Self::Boolean(_) | Self::Text(_) => None,
        }
    }

    /// Returns the integer payload narrowed to `usize`, or `None` when the value
    /// is not an integer or falls outside `usize` range.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub fn as_count(&self) -> Option<usize> {
        match self {
            Self::Integer(value) => usize::try_from(*value).ok(),
            Self::Boolean(_) | Self::Text(_) => None,
        }
    }

    /// Returns the boolean payload, or `None` for a non-boolean value.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            Self::Integer(_) | Self::Text(_) => None,
        }
    }
}

impl From<bool> for PropertyValue {
    /// Wraps a boolean value.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from(value: bool) -> Self {
        Self::Boolean(value)
    }
}

impl From<i64> for PropertyValue {
    /// Wraps a signed 64-bit integer value.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<&str> for PropertyValue {
    /// Copies a string slice into a shared text value.
    ///
    /// # Performance
    ///
    /// This function is `O(value length)`.
    fn from(value: &str) -> Self {
        Self::Text(Arc::from(value))
    }
}

impl From<String> for PropertyValue {
    /// Copies a string into a shared text value (the `Arc<str>` allocation
    /// carries an inline reference count, so the string's buffer cannot be
    /// reused).
    ///
    /// # Performance
    ///
    /// This function is `O(value length)`.
    fn from(value: String) -> Self {
        Self::Text(Arc::from(value))
    }
}

impl From<Arc<str>> for PropertyValue {
    /// Wraps an already-shared string as a text value.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn from(value: Arc<str>) -> Self {
        Self::Text(value)
    }
}

impl TryFrom<u64> for PropertyValue {
    type Error = DbError;

    /// Narrows an unsigned 64-bit value into a signed [`PropertyValue::Integer`].
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query(crate::error::QueryError::ValueOutOfRange)`] when the value exceeds
    /// `i64::MAX`.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        i64::try_from(value)
            .map(Self::Integer)
            .map_err(|_overflow| DbError::Query(crate::error::QueryError::ValueOutOfRange))
    }
}

impl TryFrom<usize> for PropertyValue {
    type Error = DbError;

    /// Narrows a pointer-width unsigned value into a signed
    /// [`PropertyValue::Integer`].
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query(crate::error::QueryError::ValueOutOfRange)`] when the value exceeds
    /// `i64::MAX`.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    fn try_from(value: usize) -> Result<Self, Self::Error> {
        i64::try_from(value)
            .map(Self::Integer)
            .map_err(|_overflow| DbError::Query(crate::error::QueryError::ValueOutOfRange))
    }
}

impl fmt::Display for PropertyValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean(value) => write!(formatter, "{value}"),
            Self::Integer(value) => write!(formatter, "{value}"),
            Self::Text(value) => formatter.write_str(value),
        }
    }
}

/// Property value parser used by CLI, HTTP, `OxQL`.
///
/// # Errors
///
/// Returns the original token when it cannot be parsed as a supported value.
///
/// # Performance
///
/// This function is `O(token.len())`.
pub(crate) fn parse_value_token(token: &str) -> Result<PropertyValue, String> {
    let trimmed = token.trim();
    if trimmed == "true" {
        return Ok(PropertyValue::Boolean(true));
    }
    if trimmed == "false" {
        return Ok(PropertyValue::Boolean(false));
    }
    if let Ok(value) = trimmed.parse::<i64>() {
        return Ok(PropertyValue::Integer(value));
    }
    parse_quoted(trimmed).map(PropertyValue::from)
}

/// Parses one single- or double-quoted token.
fn parse_quoted(token: &str) -> Result<String, String> {
    let single = token
        .strip_prefix('\'')
        .and_then(|text| text.strip_suffix('\''));
    let double = token
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'));
    single
        .or(double)
        .map_or_else(|| Err(token.to_owned()), |value| Ok(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn from_scalars_roundtrip_through_accessors() {
        assert_eq!(PropertyValue::from(true).as_bool(), Some(true));
        assert_eq!(PropertyValue::from(7_i64).as_int(), Some(7));
        assert_eq!(PropertyValue::from("hi").as_text(), Some("hi"));
        assert_eq!(
            PropertyValue::from(String::from("hi")).as_text(),
            Some("hi")
        );
    }

    #[test]
    fn accessors_reject_mismatched_types() {
        let text = PropertyValue::from("x");
        assert_eq!(text.as_int(), None);
        assert_eq!(text.as_bool(), None);
        assert_eq!(text.as_count(), None);
    }

    proptest! {
        #[test]
        fn integer_roundtrips(value in any::<i64>()) {
            prop_assert_eq!(PropertyValue::from(value).as_int(), Some(value));
        }

        #[test]
        fn boolean_roundtrips(value in any::<bool>()) {
            prop_assert_eq!(PropertyValue::from(value).as_bool(), Some(value));
        }

        #[test]
        fn text_roundtrips(value in ".*") {
            let parsed = PropertyValue::from(value.as_str());
            prop_assert_eq!(parsed.as_text(), Some(value.as_str()));
        }

        #[test]
        fn try_from_u64_matches_checked_narrowing(value in any::<u64>()) {
            match i64::try_from(value) {
                Ok(expected) => prop_assert_eq!(
                    PropertyValue::try_from(value).ok().and_then(|parsed| parsed.as_int()),
                    Some(expected)
                ),
                Err(_overflow) => prop_assert!(PropertyValue::try_from(value).is_err()),
            }
        }

        #[test]
        fn as_count_matches_checked_conversion(value in any::<i64>()) {
            prop_assert_eq!(PropertyValue::Integer(value).as_count(), usize::try_from(value).ok());
        }
    }
}
