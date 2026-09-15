//! Date construction owns converted fields before resolving the result prototype.
use super::{DEFAULT_DATE_FIELDS, MAX_DATE_ARGUMENTS, parsed_date_value};
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    builtins::{
        date::{
            calendar::{DateInputFields, set_date_fields_checked, time_clip},
            parse::parse_date_string,
        },
        native::DateNativeKind,
    },
    heap::ContextId,
    object::{ObjectRef, PropertyKey},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum DateConstructorStep {
    Complete(Completion),
    Primitive { resume: DateConstructorResume },
    String { resume: DateConstructorResume },
    Number { resume: DateConstructorResume },
    Read { resume: DateConstructorResume },
}
enum Phase {
    Single,
    Fields,
    Prototype,
    Parse,
}
pub(crate) struct DateConstructorResume(Box<DateConstructorResumeState>);
impl std::ops::Deref for DateConstructorResume {
    type Target = DateConstructorResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for DateConstructorResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<DateConstructorResume>() <= 8);
pub(crate) struct DateConstructorResumeState {
    pending_effect: DateConstructorStepPending,
    realm: ContextId,
    kind: DateNativeKind,
    new_target: Value,
    arguments: std::vec::IntoIter<Value>,
    fields: DateInputFields,
    index: usize,
    value: f64,
    phase: Phase,
}
impl DateConstructorStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: DateNativeKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let new_target = match (kind, invocation) {
            (DateNativeKind::Constructor, NativeInvocation::Construct { new_target }) => {
                new_target.clone()
            }
            (
                DateNativeKind::Now | DateNativeKind::Parse | DateNativeKind::Utc,
                NativeInvocation::Call { .. },
            ) => Value::Undefined,
            _ => {
                return Err(RuntimeError::Invariant(
                    "Date constructor/static invocation mismatch",
                ));
            }
        };
        if kind == DateNativeKind::Now {
            return Ok(Self::Complete(runtime.call_date_now()?));
        }
        if kind == DateNativeKind::Constructor && matches!(new_target, Value::Undefined) {
            return Ok(Self::Complete(runtime.call_date_as_function()?));
        }
        let count = arguments.actual_arg_count.min(MAX_DATE_ARGUMENTS);
        let values = arguments
            .readable
            .get(..count)
            .ok_or(RuntimeError::Invariant("Date actual arguments unreadable"))?
            .to_vec();
        let mut resume = DateConstructorResume(Box::new(DateConstructorResumeState {
            pending_effect: DateConstructorStepPending::default(),
            realm,
            kind,
            new_target,
            arguments: values.into_iter(),
            fields: DEFAULT_DATE_FIELDS,
            index: 0,
            value: f64::NAN,
            phase: Phase::Fields,
        }));
        if kind == DateNativeKind::Parse {
            resume.phase = Phase::Parse;
            return Ok({
                let __pending_field_value = arguments
                    .readable
                    .first()
                    .cloned()
                    .unwrap_or(Value::Undefined);
                let __pending_field_resume = resume;
                Self::request_string(__pending_field_value, __pending_field_resume)
            });
        }
        if arguments.actual_arg_count == 0 {
            if kind == DateNativeKind::Utc {
                return Ok(Self::Complete(Completion::Return(Value::Float(f64::NAN))));
            }
            resume.value = runtime.date_now_millis() as f64;
            return resume.prototype(runtime);
        }
        if kind == DateNativeKind::Constructor && arguments.actual_arg_count == 1 {
            let value = resume
                .arguments
                .next()
                .ok_or(RuntimeError::Invariant("Date sole argument missing"))?;
            if let Some(value) = runtime.genuine_date_value(&value)? {
                resume.value = time_clip(value);
                return resume.prototype(runtime);
            }
            resume.phase = Phase::Single;
            return Ok({
                let __pending_field_value = value;
                let __pending_field_resume = resume;
                Self::request_primitive(__pending_field_value, __pending_field_resume)
            });
        }
        resume.fields(runtime)
    }
}
impl DateConstructorResume {
    pub(crate) fn primitive(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<DateConstructorStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Single) {
            return Err(RuntimeError::Invariant("Date primitive phase mismatch"));
        }
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(DateConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        self.0.value = if let Value::String(string) = value {
            parsed_date_value(parse_date_string(&string), |instant| {
                runtime.date_timezone_offset_minutes(instant)
            })
        } else {
            match runtime.number_from_primitive(self.0.realm, &value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => {
                    return Ok(DateConstructorStep::Complete(Completion::Throw(value)));
                }
            }
        };
        self.0.value = time_clip(self.0.value);
        self.prototype(runtime)
    }
    pub(crate) fn string(
        self,
        runtime: &Runtime,
        result: NativeConversion<JsString>,
    ) -> Result<DateConstructorStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Parse) {
            return Err(RuntimeError::Invariant("Date parse phase mismatch"));
        }
        let string = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(DateConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        Ok(DateConstructorStep::Complete(Completion::Return(
            Value::number(parsed_date_value(parse_date_string(&string), |instant| {
                runtime.date_timezone_offset_minutes(instant)
            })),
        )))
    }
    fn fields(mut self, runtime: &Runtime) -> Result<DateConstructorStep, RuntimeError> {
        if let Some(value) = self.0.arguments.next() {
            return Ok({
                let __pending_field_value = value;
                let __pending_field_resume = self;
                DateConstructorStep::request_number(__pending_field_value, __pending_field_resume)
            });
        }
        self.0.value = set_date_fields_checked(
            self.0.fields,
            self.0.kind == DateNativeKind::Constructor,
            |instant| runtime.date_timezone_offset_minutes(instant),
        );
        if self.0.kind == DateNativeKind::Utc {
            return Ok(DateConstructorStep::Complete(Completion::Return(
                Value::number(self.0.value),
            )));
        }
        self.prototype(runtime)
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<DateConstructorStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Fields) {
            return Err(RuntimeError::Invariant("Date numeric field phase mismatch"));
        }
        self.0.fields[self.0.index] = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(DateConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        self.0.index += 1;
        self.fields(runtime)
    }
    fn prototype(mut self, runtime: &Runtime) -> Result<DateConstructorStep, RuntimeError> {
        self.0.phase = Phase::Prototype;
        Ok({
            let __pending_field_receiver = self.0.new_target.clone();
            let __pending_field_key = runtime.intern_property_key("prototype")?;
            let __pending_field_resume = self;
            DateConstructorStep::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<DateConstructorStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Prototype) {
            return Err(RuntimeError::Invariant(
                "Date prototype lookup phase mismatch",
            ));
        }
        let prototype = match result {
            Completion::Return(Value::Object(object)) => object,
            Completion::Throw(value) => {
                return Ok(DateConstructorStep::Complete(Completion::Throw(value)));
            }
            Completion::Return(_) => {
                let realm =
                    match runtime.function_realm_from_value(self.0.realm, &self.0.new_target)? {
                        NativeConversion::Value(realm) => realm,
                        NativeConversion::Throw(value) => {
                            return Ok(DateConstructorStep::Complete(Completion::Throw(value)));
                        }
                    };
                let prototype = runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .context(realm)?
                    .date_prototype
                    .ok_or(RuntimeError::Invariant("realm has no Date prototype"))?;
                ObjectRef::from_borrowed_handle(runtime.clone(), prototype)?
            }
        };
        Ok(DateConstructorStep::Complete(Completion::Return(
            Value::Object(runtime.new_date_object(&prototype, self.0.value)?),
        )))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: DateConstructorStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            DateConstructorStep::Complete(result) => return Ok(result),
            DateConstructorStep::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                resume.primitive(
                    runtime,
                    runtime.to_primitive(
                        realm,
                        value,
                        crate::engine::vm::ToPrimitiveHint::Default,
                    )?,
                )?
            }
            DateConstructorStep::String { mut resume } => {
                let value = resume.take_string_value();
                resume.string(runtime, runtime.native_to_js_string(realm, &value)?)?
            }
            DateConstructorStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            DateConstructorStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct DateConstructorStepPending {
    primitive_value: Option<Value>,
    string_value: Option<Value>,
    number_value: Option<Value>,
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
}
impl DateConstructorStep {
    pub(crate) fn request_primitive(value: Value, mut resume: DateConstructorResume) -> Self {
        resume.0.pending_effect.primitive_value = Some(value);
        Self::Primitive { resume }
    }
    pub(crate) fn request_string(value: Value, mut resume: DateConstructorResume) -> Self {
        resume.0.pending_effect.string_value = Some(value);
        Self::String { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: DateConstructorResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_read(
        receiver: Value,
        key: PropertyKey,
        mut resume: DateConstructorResume,
    ) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
}
impl DateConstructorResume {
    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .pending_effect
            .primitive_value
            .take()
            .expect("DateConstructorStep Primitive value")
    }
    pub(crate) fn take_string_value(&mut self) -> Value {
        self.0
            .pending_effect
            .string_value
            .take()
            .expect("DateConstructorStep String value")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("DateConstructorStep Number value")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("DateConstructorStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("DateConstructorStep Read key")
    }
}
const _: () = assert!(std::mem::size_of::<DateConstructorStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<DateConstructorStep>() <= 64);
