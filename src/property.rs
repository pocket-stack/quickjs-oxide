//! Ordinary ECMAScript property descriptors.
//!
//! QuickJS represents descriptor field presence with `JS_PROP_HAS_*` bits.
//! This module keeps that distinction in the type instead: an outer `Option`
//! says whether a field was supplied, while the inner `Option` on accessors
//! distinguishes an absent field from a present `undefined` getter or setter.

use std::error::Error;
use std::fmt;

/// A possibly incomplete ECMAScript Property Descriptor record.
///
/// `value: None` means that `[[Value]]` is absent. By contrast,
/// `get: Some(None)` means that `[[Get]]` is present and is `undefined`;
/// `get: None` means that `[[Get]]` is absent altogether. The same distinction
/// applies to `set`.
///
/// The value stored in `get` or `set`, when present, must already have been
/// checked by the caller to be callable. `ToPropertyDescriptor` performs that
/// check before `ValidateAndApplyPropertyDescriptor` in ECMAScript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyDescriptor<V> {
    pub value: Option<V>,
    pub writable: Option<bool>,
    pub get: Option<Option<V>>,
    pub set: Option<Option<V>>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}

impl<V> PropertyDescriptor<V> {
    /// Construct an empty, generic descriptor.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            value: None,
            writable: None,
            get: None,
            set: None,
            enumerable: None,
            configurable: None,
        }
    }

    /// Return whether none of the descriptor's fields are present.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.value.is_none()
            && self.writable.is_none()
            && self.get.is_none()
            && self.set.is_none()
            && self.enumerable.is_none()
            && self.configurable.is_none()
    }

    /// Return whether this record is a data descriptor.
    #[must_use]
    pub const fn is_data_descriptor(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }

    /// Return whether this record is an accessor descriptor.
    #[must_use]
    pub const fn is_accessor_descriptor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }

    /// Return whether this record is a generic descriptor.
    #[must_use]
    pub const fn is_generic_descriptor(&self) -> bool {
        !self.is_data_descriptor() && !self.is_accessor_descriptor()
    }
}

impl<V> Default for PropertyDescriptor<V> {
    fn default() -> Self {
        Self::new()
    }
}

/// A complete descriptor suitable for storage on an ordinary object.
///
/// A missing accessor is represented by `None`, corresponding to the
/// ECMAScript value `undefined`. All boolean attributes are explicit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletePropertyDescriptor<V> {
    Data {
        value: V,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    },
    Accessor {
        get: Option<V>,
        set: Option<V>,
        enumerable: bool,
        configurable: bool,
    },
}

impl<V> CompletePropertyDescriptor<V> {
    /// Return the descriptor's `[[Enumerable]]` attribute.
    #[must_use]
    pub const fn enumerable(&self) -> bool {
        match self {
            Self::Data { enumerable, .. } | Self::Accessor { enumerable, .. } => *enumerable,
        }
    }

    /// Return the descriptor's `[[Configurable]]` attribute.
    #[must_use]
    pub const fn configurable(&self) -> bool {
        match self {
            Self::Data { configurable, .. } | Self::Accessor { configurable, .. } => *configurable,
        }
    }

    /// Return whether this is a data descriptor.
    #[must_use]
    pub const fn is_data_descriptor(&self) -> bool {
        matches!(self, Self::Data { .. })
    }

    /// Return whether this is an accessor descriptor.
    #[must_use]
    pub const fn is_accessor_descriptor(&self) -> bool {
        matches!(self, Self::Accessor { .. })
    }
}

/// Why an ordinary property definition was rejected.
///
/// ECMAScript exposes these rejections as `false` or a `TypeError`, depending
/// on the operation that requested the definition. Keeping the reason here lets
/// that caller choose the appropriate language-level behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyDefinitionError {
    /// A record cannot contain both data and accessor fields.
    InvalidDescriptor,
    /// A missing own property cannot be added to a non-extensible object.
    NotExtensible,
    /// `configurable: true` was requested for a non-configurable property.
    ConfigurableOnNonConfigurable,
    /// `enumerable` was changed on a non-configurable property.
    EnumerableOnNonConfigurable,
    /// Data/accessor kind conversion was requested for a non-configurable property.
    KindOnNonConfigurable,
    /// `writable: true` was requested for a non-configurable, non-writable property.
    WritableOnNonWritable,
    /// A non-SameValue value was requested for a non-configurable, non-writable property.
    ValueOnNonWritable,
    /// A non-SameValue getter was requested for a non-configurable property.
    GetterOnNonConfigurable,
    /// A non-SameValue setter was requested for a non-configurable property.
    SetterOnNonConfigurable,
}

impl fmt::Display for PropertyDefinitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidDescriptor => {
                "a property descriptor cannot be both a data and an accessor descriptor"
            }
            Self::NotExtensible => "object is not extensible",
            Self::ConfigurableOnNonConfigurable
            | Self::EnumerableOnNonConfigurable
            | Self::KindOnNonConfigurable
            | Self::GetterOnNonConfigurable
            | Self::SetterOnNonConfigurable => "property is not configurable",
            Self::WritableOnNonWritable | Self::ValueOnNonWritable => "property is not writable",
        };
        formatter.write_str(message)
    }
}

impl Error for PropertyDefinitionError {}

/// Validate and apply an ordinary ECMAScript Property Descriptor.
///
/// This is the object-independent core of
/// `ValidateAndApplyPropertyDescriptor`. It mirrors QuickJS's
/// `check_define_prop_flags` and ordinary `JS_DefineProperty` update path, but
/// returns the complete descriptor to store rather than mutating an object or
/// shape directly.
///
/// `undefined` supplies the default `[[Value]]` for a newly-created data
/// property or for accessor-to-data conversion. `same_value` must implement
/// ECMAScript `SameValue`; it is used for the changes that a frozen property is
/// allowed to make. The callback is not used to compare two absent accessors,
/// which are both the value `undefined` by construction.
///
/// # Errors
///
/// Returns [`PropertyDefinitionError::InvalidDescriptor`] for a mixed data and
/// accessor record. All other errors are ordinary property-definition
/// rejections that the caller may expose either as `false` or as a `TypeError`.
pub fn validate_and_apply_property_descriptor<V, SameValue>(
    extensible: bool,
    descriptor: &PropertyDescriptor<V>,
    current: Option<&CompletePropertyDescriptor<V>>,
    undefined: &V,
    same_value: SameValue,
) -> Result<CompletePropertyDescriptor<V>, PropertyDefinitionError>
where
    V: Clone,
    SameValue: Fn(&V, &V) -> bool,
{
    let descriptor_is_data = descriptor.is_data_descriptor();
    let descriptor_is_accessor = descriptor.is_accessor_descriptor();

    if descriptor_is_data && descriptor_is_accessor {
        return Err(PropertyDefinitionError::InvalidDescriptor);
    }

    let Some(current) = current else {
        if !extensible {
            return Err(PropertyDefinitionError::NotExtensible);
        }

        let enumerable = descriptor.enumerable.unwrap_or(false);
        let configurable = descriptor.configurable.unwrap_or(false);
        return if descriptor_is_accessor {
            Ok(CompletePropertyDescriptor::Accessor {
                get: descriptor.get.clone().unwrap_or(None),
                set: descriptor.set.clone().unwrap_or(None),
                enumerable,
                configurable,
            })
        } else {
            Ok(CompletePropertyDescriptor::Data {
                value: descriptor
                    .value
                    .clone()
                    .unwrap_or_else(|| undefined.clone()),
                writable: descriptor.writable.unwrap_or(false),
                enumerable,
                configurable,
            })
        };
    };

    if descriptor.is_empty() {
        return Ok(current.clone());
    }

    let current_configurable = current.configurable();
    if !current_configurable {
        if descriptor.configurable == Some(true) {
            return Err(PropertyDefinitionError::ConfigurableOnNonConfigurable);
        }
        if descriptor
            .enumerable
            .is_some_and(|enumerable| enumerable != current.enumerable())
        {
            return Err(PropertyDefinitionError::EnumerableOnNonConfigurable);
        }
    }

    let current_is_data = current.is_data_descriptor();
    let changes_kind =
        (descriptor_is_data && !current_is_data) || (descriptor_is_accessor && current_is_data);
    if changes_kind && !current_configurable {
        return Err(PropertyDefinitionError::KindOnNonConfigurable);
    }

    match (current, descriptor_is_data, descriptor_is_accessor) {
        (
            CompletePropertyDescriptor::Data {
                value, writable, ..
            },
            true,
            false,
        ) if !current_configurable && !writable => {
            if descriptor.writable == Some(true) {
                return Err(PropertyDefinitionError::WritableOnNonWritable);
            }
            if descriptor
                .value
                .as_ref()
                .is_some_and(|new_value| !same_value(new_value, value))
            {
                return Err(PropertyDefinitionError::ValueOnNonWritable);
            }
        }
        (CompletePropertyDescriptor::Accessor { get, set, .. }, false, true)
            if !current_configurable =>
        {
            if descriptor
                .get
                .as_ref()
                .is_some_and(|new_get| !same_optional_value(new_get, get, &same_value))
            {
                return Err(PropertyDefinitionError::GetterOnNonConfigurable);
            }
            if descriptor
                .set
                .as_ref()
                .is_some_and(|new_set| !same_optional_value(new_set, set, &same_value))
            {
                return Err(PropertyDefinitionError::SetterOnNonConfigurable);
            }
        }
        _ => {}
    }

    let mut result = if descriptor_is_accessor && current_is_data {
        CompletePropertyDescriptor::Accessor {
            get: None,
            set: None,
            enumerable: current.enumerable(),
            configurable: current_configurable,
        }
    } else if descriptor_is_data && !current_is_data {
        CompletePropertyDescriptor::Data {
            value: undefined.clone(),
            writable: false,
            enumerable: current.enumerable(),
            configurable: current_configurable,
        }
    } else {
        current.clone()
    };

    match (&mut result, descriptor_is_data, descriptor_is_accessor) {
        (
            CompletePropertyDescriptor::Data {
                value, writable, ..
            },
            true,
            false,
        ) => {
            if let Some(new_value) = &descriptor.value {
                *value = new_value.clone();
            }
            if let Some(new_writable) = descriptor.writable {
                *writable = new_writable;
            }
        }
        (CompletePropertyDescriptor::Accessor { get, set, .. }, false, true) => {
            if let Some(new_get) = &descriptor.get {
                *get = new_get.clone();
            }
            if let Some(new_set) = &descriptor.set {
                *set = new_set.clone();
            }
        }
        _ => {}
    }

    match &mut result {
        CompletePropertyDescriptor::Data {
            enumerable,
            configurable,
            ..
        }
        | CompletePropertyDescriptor::Accessor {
            enumerable,
            configurable,
            ..
        } => {
            if let Some(new_enumerable) = descriptor.enumerable {
                *enumerable = new_enumerable;
            }
            if let Some(new_configurable) = descriptor.configurable {
                *configurable = new_configurable;
            }
        }
    }

    Ok(result)
}

fn same_optional_value<V, SameValue>(
    left: &Option<V>,
    right: &Option<V>,
    same_value: &SameValue,
) -> bool
where
    SameValue: Fn(&V, &V) -> bool,
{
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => same_value(left, right),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CompletePropertyDescriptor, PropertyDefinitionError, PropertyDescriptor,
        validate_and_apply_property_descriptor,
    };

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum TestValue {
        Undefined,
        Number(i32),
        Function(u8),
    }

    fn apply(
        extensible: bool,
        descriptor: &PropertyDescriptor<TestValue>,
        current: Option<&CompletePropertyDescriptor<TestValue>>,
    ) -> Result<CompletePropertyDescriptor<TestValue>, PropertyDefinitionError> {
        validate_and_apply_property_descriptor(
            extensible,
            descriptor,
            current,
            &TestValue::Undefined,
            PartialEq::eq,
        )
    }

    fn data(
        value: TestValue,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    ) -> CompletePropertyDescriptor<TestValue> {
        CompletePropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        }
    }

    fn accessor(
        get: Option<TestValue>,
        set: Option<TestValue>,
        enumerable: bool,
        configurable: bool,
    ) -> CompletePropertyDescriptor<TestValue> {
        CompletePropertyDescriptor::Accessor {
            get,
            set,
            enumerable,
            configurable,
        }
    }

    #[test]
    fn descriptor_classification_follows_present_fields() {
        let generic = PropertyDescriptor::<TestValue> {
            enumerable: Some(true),
            ..PropertyDescriptor::new()
        };
        assert!(generic.is_generic_descriptor());

        let data = PropertyDescriptor::<TestValue> {
            writable: Some(false),
            ..PropertyDescriptor::new()
        };
        assert!(data.is_data_descriptor());
        assert!(!data.is_accessor_descriptor());

        let accessor = PropertyDescriptor::<TestValue> {
            get: Some(None),
            ..PropertyDescriptor::new()
        };
        assert!(accessor.is_accessor_descriptor());
        assert!(!accessor.is_data_descriptor());
    }

    #[test]
    fn mixed_data_and_accessor_descriptor_is_invalid() {
        let descriptor = PropertyDescriptor {
            value: Some(TestValue::Number(1)),
            get: Some(None),
            ..PropertyDescriptor::new()
        };

        assert_eq!(
            apply(true, &descriptor, None),
            Err(PropertyDefinitionError::InvalidDescriptor)
        );
    }

    #[test]
    fn new_generic_and_data_properties_receive_ecmascript_defaults() {
        assert_eq!(
            apply(true, &PropertyDescriptor::new(), None),
            Ok(data(TestValue::Undefined, false, false, false))
        );

        let descriptor = PropertyDescriptor {
            value: Some(TestValue::Number(7)),
            enumerable: Some(true),
            ..PropertyDescriptor::new()
        };
        assert_eq!(
            apply(true, &descriptor, None),
            Ok(data(TestValue::Number(7), false, true, false))
        );
    }

    #[test]
    fn new_accessor_property_defaults_missing_accessors_and_attributes() {
        let descriptor = PropertyDescriptor {
            get: Some(Some(TestValue::Function(1))),
            configurable: Some(true),
            ..PropertyDescriptor::new()
        };
        assert_eq!(
            apply(true, &descriptor, None),
            Ok(accessor(Some(TestValue::Function(1)), None, false, true))
        );
    }

    #[test]
    fn non_extensible_only_rejects_creation() {
        let descriptor = PropertyDescriptor {
            value: Some(TestValue::Number(2)),
            ..PropertyDescriptor::new()
        };
        assert_eq!(
            apply(false, &descriptor, None),
            Err(PropertyDefinitionError::NotExtensible)
        );

        let current = data(TestValue::Number(1), true, false, true);
        assert_eq!(
            apply(false, &descriptor, Some(&current)),
            Ok(data(TestValue::Number(2), true, false, true))
        );
    }

    #[test]
    fn empty_descriptor_is_a_no_op_for_every_existing_kind() {
        let descriptors = [
            data(TestValue::Number(1), false, true, false),
            accessor(Some(TestValue::Function(1)), None, false, false),
        ];

        for current in descriptors {
            assert_eq!(
                apply(false, &PropertyDescriptor::new(), Some(&current)),
                Ok(current)
            );
        }
    }

    #[test]
    fn non_configurable_common_attribute_matrix() {
        let current = data(TestValue::Number(1), true, false, false);
        let cases = [
            (
                PropertyDescriptor {
                    configurable: Some(true),
                    ..PropertyDescriptor::new()
                },
                Err(PropertyDefinitionError::ConfigurableOnNonConfigurable),
            ),
            (
                PropertyDescriptor {
                    enumerable: Some(true),
                    ..PropertyDescriptor::new()
                },
                Err(PropertyDefinitionError::EnumerableOnNonConfigurable),
            ),
            (
                PropertyDescriptor {
                    configurable: Some(false),
                    enumerable: Some(false),
                    ..PropertyDescriptor::new()
                },
                Ok(current.clone()),
            ),
        ];

        for (descriptor, expected) in cases {
            assert_eq!(apply(true, &descriptor, Some(&current)), expected);
        }
    }

    #[test]
    fn non_configurable_properties_cannot_change_kind() {
        let data_property = data(TestValue::Number(1), true, false, false);
        let accessor_descriptor = PropertyDescriptor {
            get: Some(None),
            ..PropertyDescriptor::new()
        };
        assert_eq!(
            apply(true, &accessor_descriptor, Some(&data_property)),
            Err(PropertyDefinitionError::KindOnNonConfigurable)
        );

        let accessor_property = accessor(None, None, false, false);
        let data_descriptor = PropertyDescriptor {
            writable: Some(false),
            ..PropertyDescriptor::new()
        };
        assert_eq!(
            apply(true, &data_descriptor, Some(&accessor_property)),
            Err(PropertyDefinitionError::KindOnNonConfigurable)
        );
    }

    #[test]
    fn configurable_kind_conversion_uses_defaults_and_preserves_attributes() {
        let data_property = data(TestValue::Number(1), true, true, true);
        let accessor_descriptor = PropertyDescriptor {
            set: Some(Some(TestValue::Function(2))),
            ..PropertyDescriptor::new()
        };
        assert_eq!(
            apply(true, &accessor_descriptor, Some(&data_property)),
            Ok(accessor(None, Some(TestValue::Function(2)), true, true))
        );

        let accessor_property = accessor(
            Some(TestValue::Function(1)),
            Some(TestValue::Function(2)),
            true,
            true,
        );
        let data_descriptor = PropertyDescriptor {
            value: Some(TestValue::Number(9)),
            ..PropertyDescriptor::new()
        };
        assert_eq!(
            apply(true, &data_descriptor, Some(&accessor_property)),
            Ok(data(TestValue::Number(9), false, true, true))
        );
    }

    #[test]
    fn frozen_data_property_allows_only_same_value_and_false_writable() {
        let current = data(TestValue::Number(1), false, false, false);
        let cases = [
            (
                PropertyDescriptor {
                    writable: Some(true),
                    ..PropertyDescriptor::new()
                },
                Err(PropertyDefinitionError::WritableOnNonWritable),
            ),
            (
                PropertyDescriptor {
                    value: Some(TestValue::Number(2)),
                    ..PropertyDescriptor::new()
                },
                Err(PropertyDefinitionError::ValueOnNonWritable),
            ),
            (
                PropertyDescriptor {
                    value: Some(TestValue::Number(1)),
                    writable: Some(false),
                    ..PropertyDescriptor::new()
                },
                Ok(current.clone()),
            ),
        ];

        for (descriptor, expected) in cases {
            assert_eq!(apply(true, &descriptor, Some(&current)), expected);
        }
    }

    #[test]
    fn writable_property_can_be_changed_and_frozen_atomically() {
        let current = data(TestValue::Number(1), true, false, false);
        let descriptor = PropertyDescriptor {
            value: Some(TestValue::Number(2)),
            writable: Some(false),
            ..PropertyDescriptor::new()
        };

        assert_eq!(
            apply(true, &descriptor, Some(&current)),
            Ok(data(TestValue::Number(2), false, false, false))
        );
    }

    #[test]
    fn frozen_accessor_requires_same_getter_and_setter() {
        let current = accessor(
            Some(TestValue::Function(1)),
            Some(TestValue::Function(2)),
            false,
            false,
        );

        let same = PropertyDescriptor {
            get: Some(Some(TestValue::Function(1))),
            set: Some(Some(TestValue::Function(2))),
            ..PropertyDescriptor::new()
        };
        assert_eq!(apply(true, &same, Some(&current)), Ok(current.clone()));

        let different_getter = PropertyDescriptor {
            get: Some(Some(TestValue::Function(3))),
            ..PropertyDescriptor::new()
        };
        assert_eq!(
            apply(true, &different_getter, Some(&current)),
            Err(PropertyDefinitionError::GetterOnNonConfigurable)
        );

        let different_setter = PropertyDescriptor {
            set: Some(Some(TestValue::Function(3))),
            ..PropertyDescriptor::new()
        };
        assert_eq!(
            apply(true, &different_setter, Some(&current)),
            Err(PropertyDefinitionError::SetterOnNonConfigurable)
        );
    }

    #[test]
    fn absent_accessor_is_distinct_from_present_undefined() {
        let current = accessor(Some(TestValue::Function(1)), None, false, false);

        let absent = PropertyDescriptor {
            enumerable: Some(false),
            ..PropertyDescriptor::new()
        };
        assert_eq!(apply(true, &absent, Some(&current)), Ok(current.clone()));

        let present_undefined = PropertyDescriptor {
            get: Some(None),
            ..PropertyDescriptor::new()
        };
        assert_eq!(
            apply(true, &present_undefined, Some(&current)),
            Err(PropertyDefinitionError::GetterOnNonConfigurable)
        );

        let undefined_current = accessor(None, None, false, false);
        assert_eq!(
            apply(true, &present_undefined, Some(&undefined_current)),
            Ok(undefined_current)
        );
    }

    #[test]
    fn configurable_property_accepts_accessor_replacement_and_attributes() {
        let current = accessor(
            Some(TestValue::Function(1)),
            Some(TestValue::Function(2)),
            false,
            true,
        );
        let descriptor = PropertyDescriptor {
            get: Some(None),
            set: Some(Some(TestValue::Function(3))),
            enumerable: Some(true),
            configurable: Some(false),
            ..PropertyDescriptor::new()
        };

        assert_eq!(
            apply(true, &descriptor, Some(&current)),
            Ok(accessor(None, Some(TestValue::Function(3)), true, false))
        );
    }

    #[test]
    fn generic_descriptor_updates_attributes_without_changing_kind() {
        let descriptor = PropertyDescriptor {
            enumerable: Some(true),
            ..PropertyDescriptor::new()
        };
        let properties = [
            (
                data(TestValue::Number(1), true, false, true),
                data(TestValue::Number(1), true, true, true),
            ),
            (
                accessor(Some(TestValue::Function(1)), None, false, true),
                accessor(Some(TestValue::Function(1)), None, true, true),
            ),
        ];

        for (current, expected) in properties {
            assert_eq!(apply(true, &descriptor, Some(&current)), Ok(expected));
        }
    }

    #[test]
    fn caller_supplied_same_value_controls_frozen_value_comparison() {
        let current = data(TestValue::Number(1), false, false, false);
        let descriptor = PropertyDescriptor {
            value: Some(TestValue::Number(2)),
            ..PropertyDescriptor::new()
        };

        let result = validate_and_apply_property_descriptor(
            true,
            &descriptor,
            Some(&current),
            &TestValue::Undefined,
            |left, right| matches!((left, right), (TestValue::Number(_), TestValue::Number(_))),
        );
        assert_eq!(result, Ok(data(TestValue::Number(2), false, false, false)));
    }
}
