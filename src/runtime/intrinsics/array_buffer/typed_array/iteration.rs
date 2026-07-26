//! Callback-based `%TypedArray%.prototype` iteration algorithms.
//!
//! Pinned QuickJS validates and snapshots the branded view before checking the
//! callback. The callback methods then visit that original range without a
//! `HasProperty` check while keeping each element read live.

use super::*;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod transform_tests;

impl Runtime {
    pub(super) fn call_typed_array_iteration(
        &self,
        realm: ContextId,
        kind: ArrayIterationKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        if !matches!(
            kind,
            ArrayIterationKind::Every
                | ArrayIterationKind::Some
                | ArrayIterationKind::ForEach
                | ArrayIterationKind::Map
                | ArrayIterationKind::Filter
        ) {
            return Err(RuntimeError::Invariant(
                "unpublished TypedArray iteration native reached dispatch",
            ));
        }
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype iteration received a constructor invocation",
            ));
        };
        let target = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = match self.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => u64::from(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let callback = self.callable_from_value(
            arguments
                .readable
                .first()
                .ok_or(RuntimeError::Invariant(
                    "TypedArray iteration callback argv was not padded",
                ))?
                .clone(),
        )?;
        let this_arg = if arguments.actual_arg_count > 1 {
            arguments
                .readable
                .get(1)
                .ok_or(RuntimeError::Invariant(
                    "TypedArray iteration thisArg was missing",
                ))?
                .clone()
        } else {
            Value::Undefined
        };
        let receiver = Value::Object(target.clone());
        let source_element = self.typed_array_snapshot(&target)?.element;
        let mapped = if kind == ArrayIterationKind::Map {
            Some(
                match self.typed_array_species_create(realm, &target, source_element, length)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                },
            )
        } else {
            None
        };
        let selected = if kind == ArrayIterationKind::Filter {
            Some(self.new_array(realm)?)
        } else {
            None
        };
        let mut selected_length = 0_u64;

        for index in 0..length {
            let value = self
                .typed_array_read_index(&target, index)?
                .unwrap_or(Value::Undefined);
            let callback_result = match self.call_internal(
                realm,
                &callback,
                this_arg.clone(),
                &[value.clone(), Value::number(index as f64), receiver.clone()],
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            match kind {
                ArrayIterationKind::Every if !callback_result.to_boolean() => {
                    return Ok(Completion::Return(Value::Bool(false)));
                }
                ArrayIterationKind::Some if callback_result.to_boolean() => {
                    return Ok(Completion::Return(Value::Bool(true)));
                }
                ArrayIterationKind::Every
                | ArrayIterationKind::Some
                | ArrayIterationKind::ForEach => {}
                ArrayIterationKind::Map => {
                    let mapped = mapped.as_ref().ok_or(RuntimeError::Invariant(
                        "TypedArray map result was not allocated",
                    ))?;
                    match self.typed_array_set_index(realm, mapped, index, &callback_result)? {
                        NativeConversion::Value(()) => {}
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                }
                ArrayIterationKind::Filter if callback_result.to_boolean() => {
                    let selected = selected.as_ref().ok_or(RuntimeError::Invariant(
                        "TypedArray filter temporary Array was not allocated",
                    ))?;
                    let key = self.intern_property_key(&selected_length.to_string())?;
                    if !self.define_own_property(
                        selected,
                        &key,
                        &OrdinaryPropertyDescriptor {
                            value: DescriptorField::Present(value),
                            writable: DescriptorField::Present(true),
                            enumerable: DescriptorField::Present(true),
                            configurable: DescriptorField::Present(true),
                            ..OrdinaryPropertyDescriptor::new()
                        },
                    )? {
                        return Err(RuntimeError::Invariant(
                            "TypedArray filter temporary Array rejected a dense element",
                        ));
                    }
                    selected_length =
                        selected_length
                            .checked_add(1)
                            .ok_or(RuntimeError::Invariant(
                                "TypedArray filter selected length overflowed u64",
                            ))?;
                }
                ArrayIterationKind::Filter => {}
            }
        }

        let result = match kind {
            ArrayIterationKind::Every => Value::Bool(true),
            ArrayIterationKind::Some => Value::Bool(false),
            ArrayIterationKind::ForEach => Value::Undefined,
            ArrayIterationKind::Map => Value::Object(
                mapped.ok_or(RuntimeError::Invariant("TypedArray map result disappeared"))?,
            ),
            ArrayIterationKind::Filter => {
                return self.typed_array_filter_result(
                    realm,
                    &target,
                    source_element,
                    selected.ok_or(RuntimeError::Invariant(
                        "TypedArray filter temporary Array disappeared",
                    ))?,
                    selected_length,
                );
            }
        };
        Ok(Completion::Return(result))
    }
}
