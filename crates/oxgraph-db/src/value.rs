//! Typed property and query values.

use std::fmt;

use serde::{Deserialize, Serialize};

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
/// # Performance
///
/// Copying is `O(value length)` for text and `O(1)` otherwise.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PropertyValue {
    /// Boolean value.
    Boolean(bool),
    /// Signed integer value.
    Integer(i64),
    /// UTF-8 text value.
    Text(String),
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

/// Property value parser used by CLI, HTTP, `OxQL`, and Cypher tests.
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
    parse_quoted(trimmed).map(PropertyValue::Text)
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
