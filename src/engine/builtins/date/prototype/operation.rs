//! Date prototype operations retain the specified pre-coercion fields or re-read setYear state.
use super::{date_argument, date_input_fields};
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::{
        date::calendar::{DateFields, get_date_fields, set_date_fields, time_clip},
        native::{DateNativeKind, DateSetFieldKind},
    },
    heap::ContextId,
    object::{CallableRef, ObjectRef, PropertyKey},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum DatePrototypeStep {
    Complete(Completion),
    Number {
        value: Value,
        resume: DatePrototypeResume,
    },
    Primitive {
        value: Value,
        hint: ToPrimitiveHint,
        resume: DatePrototypeResume,
    },
    OrdinaryPrimitive {
        object: ObjectRef,
        hint: ToPrimitiveHint,
    },
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: DatePrototypeResume,
    },
    Call {
        callable: CallableRef,
        receiver: Value,
    },
}
enum Phase {
    Time,
    Field {
        field: DateSetFieldKind,
        fields: DateFields,
        had_fields: bool,
        all_finite: bool,
    },
    Year,
    JsonPrimitive,
    JsonMethod,
}
pub(crate) struct DatePrototypeResume(Box<DatePrototypeResumeState>);
impl std::ops::Deref for DatePrototypeResume {
    type Target = DatePrototypeResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for DatePrototypeResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<DatePrototypeResume>() <= 8);
pub(crate) struct DatePrototypeResumeState {
    realm: ContextId,
    object: ObjectRef,
    phase: Phase,
    arguments: std::vec::IntoIter<Value>,
    converted: usize,
    actual: usize,
}
impl DatePrototypeStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: DateNativeKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Date prototype requires generic invocation",
            ));
        };
        if kind == DateNativeKind::ToPrimitive {
            let Value::Object(object) = this_value else {
                return Ok(Self::Complete(Completion::Throw(
                    runtime.new_native_error(realm, NativeErrorKind::Type, "not an object")?,
                )));
            };
            let hint = match date_argument(arguments, 0)? {
                Value::String(value)
                    if value == &JsString::from_static("number")
                        || value == &JsString::from_static("integer") =>
                {
                    ToPrimitiveHint::Number
                }
                Value::String(value)
                    if value == &JsString::from_static("string")
                        || value == &JsString::from_static("default") =>
                {
                    ToPrimitiveHint::String
                }
                _ => {
                    return Ok(Self::Complete(Completion::Throw(
                        runtime.new_native_error(realm, NativeErrorKind::Type, "invalid hint")?,
                    )));
                }
            };
            return Ok(Self::OrdinaryPrimitive {
                object: object.clone(),
                hint,
            });
        }
        if kind == DateNativeKind::ToJson {
            let object = match runtime.native_to_object(realm, this_value.clone())? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => {
                    return Ok(Self::Complete(Completion::Throw(value)));
                }
            };
            return Ok(Self::Primitive {
                value: Value::Object(object.clone()),
                hint: ToPrimitiveHint::Number,
                resume: DatePrototypeResume(Box::new(DatePrototypeResumeState {
                    realm,
                    object,
                    phase: Phase::JsonPrimitive,
                    arguments: Vec::new().into_iter(),
                    converted: 0,
                    actual: 0,
                })),
            });
        }
        let (object, value) = match runtime.date_this_time_value(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let (phase, count) = match kind {
            DateNativeKind::SetTime => (Phase::Time, 1),
            DateNativeKind::SetYear => (Phase::Year, 1),
            DateNativeKind::SetField(field) => {
                let first = usize::from(field.first_field());
                let end = usize::from(field.end_field());
                let fields =
                    get_date_fields(value, field.uses_local_time(), first == 0, |instant| {
                        runtime.date_timezone_offset_minutes(instant)
                    });
                let had_fields = fields.is_some();
                (
                    Phase::Field {
                        field,
                        fields: fields.unwrap_or([0.0; 9]),
                        had_fields,
                        all_finite: had_fields,
                    },
                    arguments.actual_arg_count.min(end.saturating_sub(first)),
                )
            }
            _ => {
                return Err(RuntimeError::Invariant(
                    "pure Date method reached callback operation",
                ));
            }
        };
        DatePrototypeResume(Box::new(DatePrototypeResumeState {
            realm,
            object,
            phase,
            arguments: arguments
                .readable
                .get(..count)
                .ok_or(RuntimeError::Invariant("Date setter argv was not padded"))?
                .to_vec()
                .into_iter(),
            converted: 0,
            actual: arguments.actual_arg_count,
        }))
        .next(runtime)
    }
}
impl DatePrototypeResume {
    fn next(mut self, runtime: &Runtime) -> Result<DatePrototypeStep, RuntimeError> {
        if let Some(value) = self.0.arguments.next() {
            return Ok(DatePrototypeStep::Number {
                value,
                resume: self,
            });
        }
        let Phase::Field {
            field,
            fields,
            had_fields,
            all_finite,
        } = self.0.phase
        else {
            return Err(RuntimeError::Invariant(
                "Date setter numeric result missing",
            ));
        };
        if !had_fields {
            return Ok(DatePrototypeStep::Complete(Completion::Return(
                Value::number(f64::NAN),
            )));
        }
        let value = if all_finite && self.0.actual > 0 {
            set_date_fields(
                &date_input_fields(&fields),
                field.uses_local_time(),
                |instant| runtime.date_timezone_offset_minutes(instant),
            )
        } else {
            f64::NAN
        };
        Ok(DatePrototypeStep::Complete(
            runtime.set_date_this_time_value(&self.0.object, value)?,
        ))
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<DatePrototypeStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(DatePrototypeStep::Complete(Completion::Throw(value)));
            }
        };
        match &mut self.0.phase {
            Phase::Time => Ok(DatePrototypeStep::Complete(
                runtime.set_date_this_time_value(&self.0.object, time_clip(value))?,
            )),
            Phase::Year => Ok(DatePrototypeStep::Complete(
                runtime.finish_date_set_year(&self.0.object, value)?,
            )),
            Phase::Field {
                field,
                fields,
                all_finite,
                ..
            } => {
                if !value.is_finite() {
                    *all_finite = false;
                }
                fields[usize::from(field.first_field()) + self.0.converted] = value.trunc();
                self.0.converted += 1;
                self.next(runtime)
            }
            _ => Err(RuntimeError::Invariant("Date setter number phase mismatch")),
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<DatePrototypeStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(DatePrototypeStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::JsonPrimitive => {
                if value.as_number().is_some_and(|value| !value.is_finite()) {
                    return Ok(DatePrototypeStep::Complete(Completion::Return(Value::Null)));
                }
                self.0.phase = Phase::JsonMethod;
                Ok(DatePrototypeStep::Read {
                    object: self.0.object.clone(),
                    key: runtime.intern_property_key("toISOString")?,
                    resume: self,
                })
            }
            Phase::JsonMethod => {
                let callable = match value {
                    Value::Object(object) => runtime.as_callable(&object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    return Ok(DatePrototypeStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "object needs toISOString method",
                        )?,
                    )));
                };
                Ok(DatePrototypeStep::Call {
                    callable,
                    receiver: Value::Object(self.0.object),
                })
            }
            _ => Err(RuntimeError::Invariant(
                "Date prototype value phase mismatch",
            )),
        }
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: DatePrototypeStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            DatePrototypeStep::Complete(result) => return Ok(result),
            DatePrototypeStep::Number { value, resume } => {
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            DatePrototypeStep::Primitive {
                value,
                hint,
                resume,
            } => resume.resume(runtime, runtime.to_primitive(realm, value, hint)?)?,
            DatePrototypeStep::OrdinaryPrimitive { object, hint } => {
                return runtime.ordinary_to_primitive(realm, &object, hint);
            }
            DatePrototypeStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
            DatePrototypeStep::Call { callable, receiver } => {
                return runtime.call_internal(realm, &callable, receiver, &[]);
            }
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<DatePrototypeStep>() <= 64);
