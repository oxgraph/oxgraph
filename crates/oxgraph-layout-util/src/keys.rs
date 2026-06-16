//! Injective composite-key encoding.
//!
//! [`encode_composite_key`] joins text fields into one string such that distinct
//! field tuples can never collide. It is the building block for a stable identity
//! key over an equality index, where a naive separator-join (`a:b:c`) is unsound:
//! the fields themselves may contain the separator, so `["a", "b:c"]` and
//! `["a:b", "c"]` would collide.

use alloc::string::{String, ToString};

/// Encodes `fields` into one string that is injective in the field tuple:
/// distinct inputs always produce distinct outputs and equal inputs produce
/// equal outputs, whatever the fields contain.
///
/// Each field is length-prefixed (`{byte_len}:{field}`). The byte count, read up
/// to the first `:`, fixes each field's boundary unambiguously even when a field
/// contains `:` or leading digits, so the concatenation is a prefix-free code.
/// The result is an opaque identity token, not intended to be parsed back.
///
/// # Performance
///
/// This function is `O(total field bytes)` with a single output allocation.
#[must_use]
pub fn encode_composite_key(fields: &[&str]) -> String {
    let capacity = fields
        .iter()
        .map(|field| field.len().saturating_add(8))
        .sum();
    let mut key = String::with_capacity(capacity);
    for field in fields {
        key.push_str(&field.len().to_string());
        key.push(':');
        key.push_str(field);
    }
    key
}

#[cfg(test)]
mod tests {
    use super::encode_composite_key;

    #[test]
    fn distinct_field_boundaries_do_not_collide() {
        // A naive `:`-join would render both as "a:b:c"; length-prefixing keeps
        // distinct tuples distinct.
        assert_ne!(
            encode_composite_key(&["a", "b:c"]),
            encode_composite_key(&["a:b", "c"]),
        );
    }

    #[test]
    fn equal_inputs_encode_equally_and_arity_matters() {
        assert_eq!(
            encode_composite_key(&["x", "y"]),
            encode_composite_key(&["x", "y"]),
        );
        // Field count is part of the identity.
        assert_ne!(
            encode_composite_key(&["xy"]),
            encode_composite_key(&["x", "y"]),
        );
    }

    #[test]
    fn empty_digit_and_separator_fields_stay_injective() {
        assert_ne!(encode_composite_key(&[]), encode_composite_key(&[""]));
        assert_ne!(encode_composite_key(&[""]), encode_composite_key(&["", ""]));
        // A leading digit in a field cannot be mistaken for a length prefix.
        assert_ne!(
            encode_composite_key(&["5abc"]),
            encode_composite_key(&["5", "abc"]),
        );
        // Fields containing the ':' separator stay distinct across boundaries.
        assert_ne!(
            encode_composite_key(&["1:1", "a"]),
            encode_composite_key(&["1", "1:a"]),
        );
    }
}
