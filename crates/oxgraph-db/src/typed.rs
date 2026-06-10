//! Compile-time typed handles over the catalog's plain ids.
//!
//! A [`Key<T>`] wraps a [`PropertyKeyId`] and carries its value type `T` in the
//! type system, so assigning a value of the wrong type through the typed write
//! surface is a compile error rather than a runtime
//! [`DbError::PropertyTypeMismatch`](crate::DbError). The markers [`Text`],
//! [`Int`], and [`Bool`] correspond to the three [`PropertyType`] variants.
//!
//! # Performance
//!
//! Every handle is a `Copy` newtype; constructing, copying, and unwrapping are
//! `O(1)`. [`Assignable::into_value`] is `O(1)` except for text, which is
//! `O(value length)`.

use core::marker::PhantomData;

use crate::{IndexId, PropertyKeyId, PropertyType, PropertyValue, error::DbError};

/// Sealing module so [`ValueType`] cannot be implemented downstream.
mod sealed {
    /// Sealed supertrait; only this crate's markers implement it.
    pub trait Sealed {}
}

/// A scalar value type usable as a typed-handle marker.
///
/// Implemented only by [`Text`], [`Int`], and [`Bool`]; sealed against
/// downstream implementations.
///
/// # Performance
///
/// `perf: unspecified` — a compile-time marker trait with no runtime cost.
pub trait ValueType: sealed::Sealed + Copy {
    /// The catalog property type this marker corresponds to.
    const TYPE: PropertyType;
}

/// Text value-type marker (corresponds to [`PropertyType::Text`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Text;

/// Integer value-type marker (corresponds to [`PropertyType::Integer`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Int;

/// Boolean value-type marker (corresponds to [`PropertyType::Boolean`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Bool;

impl sealed::Sealed for Text {}
impl sealed::Sealed for Int {}
impl sealed::Sealed for Bool {}

impl ValueType for Text {
    const TYPE: PropertyType = PropertyType::Text;
}
impl ValueType for Int {
    const TYPE: PropertyType = PropertyType::Integer;
}
impl ValueType for Bool {
    const TYPE: PropertyType = PropertyType::Boolean;
}

/// A property key that carries its value type `T` in the type system.
///
/// # Performance
///
/// Copying and unwrapping are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Key<T: ValueType> {
    /// The underlying catalog property key id.
    id: PropertyKeyId,
    /// Phantom value-type tag carrying `T`.
    _ty: PhantomData<T>,
}

impl<T: ValueType> Key<T> {
    /// Wraps a plain property key id as a typed key. The caller asserts the key
    /// was declared with type `T::TYPE`.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn from_id(id: PropertyKeyId) -> Self {
        Self {
            id,
            _ty: PhantomData,
        }
    }

    /// Returns the underlying plain property key id.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn id(self) -> PropertyKeyId {
        self.id
    }
}

/// An equality index whose indexed key has value type `T`.
///
/// # Performance
///
/// Copying and unwrapping are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EqualityIndex<T: ValueType> {
    /// The underlying catalog index id.
    id: IndexId,
    /// Phantom value-type tag carrying `T`.
    _ty: PhantomData<T>,
}

impl<T: ValueType> EqualityIndex<T> {
    /// Wraps a plain index id as a typed equality index. The caller asserts the
    /// index covers a key of type `T::TYPE`.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn from_id(id: IndexId) -> Self {
        Self {
            id,
            _ty: PhantomData,
        }
    }

    /// Returns the underlying plain index id.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn id(self) -> IndexId {
        self.id
    }
}

/// A range index whose indexed key has value type `T`.
///
/// # Performance
///
/// Copying and unwrapping are `O(1)`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RangeIndex<T: ValueType> {
    /// The underlying catalog index id.
    id: IndexId,
    /// Phantom value-type tag carrying `T`.
    _ty: PhantomData<T>,
}

impl<T: ValueType> RangeIndex<T> {
    /// Wraps a plain index id as a typed range index. The caller asserts the
    /// index covers a key of type `T::TYPE`.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn from_id(id: IndexId) -> Self {
        Self {
            id,
            _ty: PhantomData,
        }
    }

    /// Returns the underlying plain index id.
    ///
    /// # Performance
    ///
    /// This function is `O(1)`.
    #[must_use]
    pub const fn id(self) -> IndexId {
        self.id
    }
}

/// A Rust value that may be assigned to a property of value type `T`.
///
/// This is the compile-time gate behind the typed write surface: a value
/// implements `Assignable<T>` only for the `T` it can legally inhabit, so a
/// type-mismatched assignment fails to compile.
///
/// # Performance
///
/// [`Assignable::into_value`] is `O(1)` except for text (`O(value length)`).
pub trait Assignable<T: ValueType> {
    /// Converts into a [`PropertyValue`], checking range where narrowing.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::Query(crate::error::QueryError::ValueOutOfRange)`] when an unsigned value
    /// exceeds `i64::MAX`.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` except for text (`O(value length)`).
    fn into_value(self) -> Result<PropertyValue, DbError>;
}

impl Assignable<Text> for &str {
    fn into_value(self) -> Result<PropertyValue, DbError> {
        Ok(PropertyValue::Text(self.to_owned()))
    }
}

impl Assignable<Text> for String {
    fn into_value(self) -> Result<PropertyValue, DbError> {
        Ok(PropertyValue::Text(self))
    }
}

impl Assignable<Int> for i64 {
    fn into_value(self) -> Result<PropertyValue, DbError> {
        Ok(PropertyValue::Integer(self))
    }
}

impl Assignable<Int> for u64 {
    fn into_value(self) -> Result<PropertyValue, DbError> {
        PropertyValue::try_from(self)
    }
}

impl Assignable<Int> for usize {
    fn into_value(self) -> Result<PropertyValue, DbError> {
        PropertyValue::try_from(self)
    }
}

impl Assignable<Bool> for bool {
    fn into_value(self) -> Result<PropertyValue, DbError> {
        Ok(PropertyValue::Boolean(self))
    }
}

/// A Rust value that can be read back from a property of value type `T`.
///
/// # Performance
///
/// [`Readable::read`] is `O(1)` except for text (`O(value length)` to copy).
pub trait Readable<T: ValueType>: Sized {
    /// Reads `Self` from a [`PropertyValue`], or `None` on a type mismatch or
    /// out-of-range narrowing.
    ///
    /// # Performance
    ///
    /// This function is `O(1)` except for text (`O(value length)`).
    fn read(value: &PropertyValue) -> Option<Self>;
}

impl Readable<Text> for String {
    fn read(value: &PropertyValue) -> Option<Self> {
        value.as_text().map(str::to_owned)
    }
}

impl Readable<Int> for i64 {
    fn read(value: &PropertyValue) -> Option<Self> {
        value.as_int()
    }
}

impl Readable<Int> for usize {
    fn read(value: &PropertyValue) -> Option<Self> {
        value.as_count()
    }
}

impl Readable<Bool> for bool {
    fn read(value: &PropertyValue) -> Option<Self> {
        value.as_bool()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_map_to_property_types() {
        assert_eq!(<Text as ValueType>::TYPE, PropertyType::Text);
        assert_eq!(<Int as ValueType>::TYPE, PropertyType::Integer);
        assert_eq!(<Bool as ValueType>::TYPE, PropertyType::Boolean);
    }

    #[test]
    fn typed_keys_roundtrip_their_plain_ids() {
        let raw = PropertyKeyId::new(7);
        let key = Key::<Text>::from_id(raw);
        assert_eq!(key.id(), raw);
    }

    #[test]
    fn assignable_checks_range_for_unsigned() {
        assert_eq!(
            Assignable::<Int>::into_value(5_u64).ok(),
            Some(PropertyValue::Integer(5))
        );
        assert!(Assignable::<Int>::into_value(u64::MAX).is_err());
        assert_eq!(
            Assignable::<Text>::into_value("hi").ok(),
            Some(PropertyValue::Text("hi".to_owned()))
        );
    }

    #[test]
    fn readable_projects_matching_values() {
        assert_eq!(
            <i64 as Readable<Int>>::read(&PropertyValue::Integer(3)),
            Some(3)
        );
        assert_eq!(
            <String as Readable<Text>>::read(&PropertyValue::Integer(3)),
            None
        );
    }
}
