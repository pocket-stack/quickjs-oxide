//! Primitive constructors finish coercion before observing a supplied new.target prototype.
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::PrimitiveKind,
    heap::ContextId,
    object::PropertyKey,
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum PrimitiveConstructorStep {
    Complete(Completion),
    Primitive { resume: PrimitiveConstructorResume },
    String { resume: PrimitiveConstructorResume },
    Read { resume: PrimitiveConstructorResume },
}
enum Phase {
    Value,
    Prototype,
}
pub(crate) struct PrimitiveConstructorResume(Box<PrimitiveConstructorResumeState>);
impl std::ops::Deref for PrimitiveConstructorResume {
    type Target = PrimitiveConstructorResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for PrimitiveConstructorResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<PrimitiveConstructorResume>() <= 8);
pub(crate) struct PrimitiveConstructorResumeState {
    pending_effect: PrimitiveConstructorStepPending,
    realm: ContextId,
    kind: PrimitiveKind,
    new_target: Value,
    value: Value,
    phase: Phase,
}
impl PrimitiveConstructorStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "primitive constructor argv was not padded",
            ))?;
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "primitive constructor requires constructor-or-function invocation",
            ));
        };
        if matches!(kind, PrimitiveKind::Symbol | PrimitiveKind::BigInt)
            && !matches!(new_target, Value::Undefined)
        {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_not_constructor_error(realm, new_target)?,
            )));
        }
        let resume = PrimitiveConstructorResume(Box::new(PrimitiveConstructorResumeState {
            pending_effect: PrimitiveConstructorStepPending::default(),
            realm,
            kind,
            new_target: new_target.clone(),
            value: Value::Undefined,
            phase: Phase::Value,
        }));
        match kind {
            PrimitiveKind::Boolean => {
                resume.converted(runtime, Value::Bool(runtime.value_to_boolean(&argument)?))
            }
            PrimitiveKind::Number if arguments.actual_arg_count == 0 => {
                resume.converted(runtime, Value::Int(0))
            }
            PrimitiveKind::String if arguments.actual_arg_count == 0 => {
                resume.converted(runtime, Value::String(JsString::from_static("")))
            }
            PrimitiveKind::Symbol if matches!(argument, Value::Undefined) => Ok(Self::Complete(
                Completion::Return(Value::Symbol(runtime.new_symbol(None)?)),
            )),
            PrimitiveKind::String
                if matches!(new_target, Value::Undefined)
                    && matches!(argument, Value::Symbol(_)) =>
            {
                let Value::Symbol(symbol) = argument else {
                    unreachable!()
                };
                resume.converted(
                    runtime,
                    Value::String(runtime.symbol_descriptive_string(&symbol)?),
                )
            }
            PrimitiveKind::String | PrimitiveKind::Symbol => {
                if !matches!(argument, Value::Object(_)) {
                    let result = runtime.native_to_js_string(realm, &argument)?;
                    resume.string(runtime, result)
                } else {
                    Ok({
                        let __pending_field_value = argument;
                        let __pending_field_resume = resume;
                        Self::request_string(__pending_field_value, __pending_field_resume)
                    })
                }
            }
            PrimitiveKind::Number | PrimitiveKind::BigInt => {
                if !matches!(argument, Value::Object(_)) {
                    resume.primitive(runtime, Completion::Return(argument))
                } else {
                    Ok({
                        let __pending_field_value = argument;
                        let __pending_field_resume = resume;
                        Self::request_primitive(__pending_field_value, __pending_field_resume)
                    })
                }
            }
        }
    }
}
impl PrimitiveConstructorResume {
    pub(crate) fn primitive(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<PrimitiveConstructorStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Value) {
            return Err(RuntimeError::Invariant(
                "primitive constructor coercion phase mismatch",
            ));
        }
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(PrimitiveConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        let value = match self.0.kind {
            PrimitiveKind::Number => {
                match runtime.number_constructor_from_primitive(self.0.realm, &value)? {
                    NativeConversion::Value(value) => Value::number(value),
                    NativeConversion::Throw(value) => {
                        return Ok(PrimitiveConstructorStep::Complete(Completion::Throw(value)));
                    }
                }
            }
            PrimitiveKind::BigInt => {
                match runtime.bigint_constructor_from_primitive(self.0.realm, &value)? {
                    NativeConversion::Value(value) => Value::BigInt(value),
                    NativeConversion::Throw(value) => {
                        return Ok(PrimitiveConstructorStep::Complete(Completion::Throw(value)));
                    }
                }
            }
            _ => {
                return Err(RuntimeError::Invariant(
                    "primitive constructor coercion kind mismatch",
                ));
            }
        };
        self.converted(runtime, value)
    }
    pub(crate) fn string(
        self,
        runtime: &Runtime,
        result: NativeConversion<JsString>,
    ) -> Result<PrimitiveConstructorStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Value) {
            return Err(RuntimeError::Invariant(
                "primitive constructor string phase mismatch",
            ));
        }
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(PrimitiveConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        if self.0.kind == PrimitiveKind::Symbol {
            return Ok(PrimitiveConstructorStep::Complete(Completion::Return(
                Value::Symbol(runtime.new_symbol(Some(value))?),
            )));
        }
        self.converted(runtime, Value::String(value))
    }
    fn converted(
        mut self,
        runtime: &Runtime,
        value: Value,
    ) -> Result<PrimitiveConstructorStep, RuntimeError> {
        if matches!(self.0.new_target, Value::Undefined) {
            return Ok(PrimitiveConstructorStep::Complete(Completion::Return(
                value,
            )));
        }
        self.0.value = value;
        self.0.phase = Phase::Prototype;
        Ok({
            let __pending_field_receiver = self.0.new_target.clone();
            let __pending_field_key = runtime.intern_property_key("prototype")?;
            let __pending_field_resume = self;
            PrimitiveConstructorStep::request_read(
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
    ) -> Result<PrimitiveConstructorStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Prototype) {
            return Err(RuntimeError::Invariant(
                "primitive constructor prototype phase mismatch",
            ));
        }
        let prototype = match result {
            Completion::Return(Value::Object(object)) => object,
            Completion::Throw(value) => {
                return Ok(PrimitiveConstructorStep::Complete(Completion::Throw(value)));
            }
            Completion::Return(_) => {
                let realm = match runtime
                    .function_realm_from_value(self.0.realm, &self.0.new_target)?
                {
                    NativeConversion::Value(realm) => realm,
                    NativeConversion::Throw(value) => {
                        return Ok(PrimitiveConstructorStep::Complete(Completion::Throw(value)));
                    }
                };
                runtime.primitive_prototype_for_realm(realm, self.0.kind)?
            }
        };
        Ok(PrimitiveConstructorStep::Complete(Completion::Return(
            Value::Object(runtime.new_primitive_object(&prototype, self.0.kind, self.0.value)?),
        )))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: PrimitiveConstructorStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            PrimitiveConstructorStep::Complete(result) => return Ok(result),
            PrimitiveConstructorStep::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                resume.primitive(
                    runtime,
                    runtime.to_primitive(
                        realm,
                        value,
                        crate::engine::vm::ToPrimitiveHint::Number,
                    )?,
                )?
            }
            PrimitiveConstructorStep::String { mut resume } => {
                let value = resume.take_string_value();
                resume.string(runtime, runtime.native_to_js_string(realm, &value)?)?
            }
            PrimitiveConstructorStep::Read { mut resume } => {
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

#[cfg(test)]
mod local_completion_tests {
    use super::*;

    #[test]
    fn primitive_constructor_finishes_without_a_conversion_request() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let invocation = NativeInvocation::Construct {
            new_target: Value::Undefined,
        };
        let arguments = NativeArguments {
            actual_arg_count: 1,
            readable: vec![Value::Int(42)],
        };
        let result = PrimitiveConstructorStep::start(
            &runtime,
            context.realm,
            PrimitiveKind::String,
            &invocation,
            &arguments,
        )
        .unwrap();
        assert!(
            matches!(result, PrimitiveConstructorStep::Complete(Completion::Return(Value::String(value))) if value == JsString::from_static("42"))
        );
    }

    #[test]
    fn local_constructor_conversion_keeps_symbol_and_new_target_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let trace='', symbol=Symbol('x'), caught=false;
            const value={toString(){trace+='v';return 'x';}};
            const target=new Proxy(function(){},{get(t,k,r){if(k==='prototype')trace+='p';return Reflect.get(t,k,r);}});
            let object=Reflect.construct(String,[value],target);
            try{new String(symbol)}catch(e){caught=e instanceof TypeError;}
            return String(42)==='42'&&String() === '' && String(undefined)==='undefined'
                && String(symbol)==='Symbol(x)'&&caught&&trace==='vp'
                && String.prototype.valueOf.call(object)==='x'&&Number(42n)===42&&BigInt('42')===42n;
        })()"#).unwrap(),Value::Bool(true));
    }
}

#[derive(Default)]
struct PrimitiveConstructorStepPending {
    primitive_value: Option<Value>,
    string_value: Option<Value>,
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
}
impl PrimitiveConstructorStep {
    pub(crate) fn request_primitive(value: Value, mut resume: PrimitiveConstructorResume) -> Self {
        resume.0.pending_effect.primitive_value = Some(value);
        Self::Primitive { resume }
    }
    pub(crate) fn request_string(value: Value, mut resume: PrimitiveConstructorResume) -> Self {
        resume.0.pending_effect.string_value = Some(value);
        Self::String { resume }
    }
    pub(crate) fn request_read(
        receiver: Value,
        key: PropertyKey,
        mut resume: PrimitiveConstructorResume,
    ) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
}
impl PrimitiveConstructorResume {
    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .pending_effect
            .primitive_value
            .take()
            .expect("PrimitiveConstructorStep Primitive value")
    }
    pub(crate) fn take_string_value(&mut self) -> Value {
        self.0
            .pending_effect
            .string_value
            .take()
            .expect("PrimitiveConstructorStep String value")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("PrimitiveConstructorStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("PrimitiveConstructorStep Read key")
    }
}
const _: () = assert!(std::mem::size_of::<PrimitiveConstructorStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<PrimitiveConstructorStep>() <= 64);
