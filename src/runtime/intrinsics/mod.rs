use super::*;

mod array;
mod array_buffer;
mod atomics;
pub(in crate::runtime) use array_buffer::typed_array::CanonicalNumericIndex;
pub(super) mod date;
mod error;
mod eval;
mod iterator;
mod json;
mod map;
mod math;
mod object;
pub(super) mod promise;
mod proxy;
mod reflect;
mod regexp;
mod replacement;
mod set;
mod shared_array_buffer;
mod string;
mod weak_collection;
mod weak_ref;

/// Pinned QuickJS `JS_ToInt64Free` for an already numeric value.
///
/// Rust's float-to-integer cast saturates outside the signed range. QuickJS
/// instead preserves the low 64 bits while the binary exponent remains close
/// enough to the mantissa, and maps still larger magnitudes to zero.
fn quickjs_to_int64_free(number: f64) -> i64 {
    const EXPONENT_BIAS: u64 = 1023;
    const MANTISSA_BITS: u64 = 52;
    const MANTISSA_MASK: u64 = (1_u64 << MANTISSA_BITS) - 1;

    let bits = number.to_bits();
    let exponent = (bits >> MANTISSA_BITS) & 0x7ff;
    if exponent <= EXPONENT_BIAS + 62 {
        return number as i64;
    }
    if exponent <= EXPONENT_BIAS + 62 + 53 {
        let significand = (bits & MANTISSA_MASK) | (1_u64 << MANTISSA_BITS);
        let shift = u32::try_from(exponent - EXPONENT_BIAS - MANTISSA_BITS)
            .expect("QuickJS ToInt64 exponent shift fits u32");
        let signed = (significand << shift) as i64;
        return if bits >> 63 == 0 {
            signed
        } else {
            signed.wrapping_neg()
        };
    }
    0
}

impl Runtime {
    /// Perform ordinary throwing Set for builtin algorithms which publish
    /// values through `[[Set]]` rather than CreateDataProperty.
    pub(in crate::runtime) fn set_property_or_throw(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        match self.internal_set(realm, object, key, value, Value::Object(object.clone()))? {
            NativeConversion::Value(InternalSetResult::Accepted) => Ok(None),
            NativeConversion::Throw(value) => Ok(Some(value)),
            NativeConversion::Value(result) => {
                let error = match result {
                    InternalSetResult::RejectedProxyTrap => {
                        Error::new(ErrorKind::Type, "proxy: cannot set property")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::ReadOnly) => {
                        self.native_atom_error(ErrorKind::Type, "'", key, "' is read-only")?
                    }
                    InternalSetResult::Rejected(PropertySetRejection::ArrayLengthReadOnly) => {
                        let length = self.intern_property_key("length")?;
                        self.native_atom_error(ErrorKind::Type, "'", &length, "' is read-only")?
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NotConfigurable) => {
                        Error::new(ErrorKind::Type, "not configurable")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NoSetter) => {
                        Error::new(ErrorKind::Type, "no setter for property")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NotExtensible) => {
                        Error::new(ErrorKind::Type, "object is not extensible")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NotObject) => {
                        Error::new(ErrorKind::Type, "not an object")
                    }
                    InternalSetResult::Accepted => unreachable!("accepted Set returned above"),
                };
                Ok(Some(self.new_native_error_from_error(
                    realm,
                    NativeErrorKind::Type,
                    &error,
                )?))
            }
        }
    }
}
