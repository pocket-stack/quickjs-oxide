//! Dynamic Function source fragments and eval results survive every conversion and prototype lookup.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::DynamicFunctionKind,
    code::dynamic_source::DynamicSourceBuilder,
    heap::ContextId,
    object::{ObjectRef, PropertyKey},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum DynamicFunctionStep {
    Complete(Completion),
    String { resume: DynamicFunctionResume },
    Eval { resume: DynamicFunctionResume },
    Read { resume: DynamicFunctionResume },
}
enum Phase {
    Parameters,
    Body,
    Eval,
    Prototype,
}
pub(crate) struct DynamicFunctionResume(Box<DynamicFunctionResumeState>);
impl std::ops::Deref for DynamicFunctionResume {
    type Target = DynamicFunctionResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for DynamicFunctionResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<DynamicFunctionResume>() <= 8);
pub(crate) struct DynamicFunctionResumeState {
    pending_effect: DynamicFunctionStepPending,
    realm: ContextId,
    kind: DynamicFunctionKind,
    new_target: Value,
    arguments: Vec<Value>,
    index: usize,
    source: Option<DynamicSourceBuilder>,
    phase: Phase,
    value: Value,
}
impl DynamicFunctionStep {
    pub(crate) fn start(
        _runtime: &Runtime,
        realm: ContextId,
        kind: DynamicFunctionKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function constructor requires constructor-or-function invocation",
            ));
        };
        let mut source = DynamicSourceBuilder::new();
        source.push_str("(")?;
        if matches!(
            kind,
            DynamicFunctionKind::Async | DynamicFunctionKind::AsyncGenerator
        ) {
            source.push_str("async ")?;
        }
        source.push_str("function")?;
        if matches!(
            kind,
            DynamicFunctionKind::Generator | DynamicFunctionKind::AsyncGenerator
        ) {
            source.push_str("*")?;
        }
        source.push_str(" anonymous(")?;
        DynamicFunctionResume(Box::new(DynamicFunctionResumeState {
            pending_effect: DynamicFunctionStepPending::default(),
            realm,
            kind,
            new_target: new_target.clone(),
            arguments: arguments.readable[..arguments.actual_arg_count].to_vec(),
            index: 0,
            source: Some(source),
            phase: Phase::Parameters,
            value: Value::Undefined,
        }))
        .parameter()
    }
}
impl DynamicFunctionResume {
    fn source(&mut self) -> Result<&mut DynamicSourceBuilder, RuntimeError> {
        self.0
            .source
            .as_mut()
            .ok_or(RuntimeError::Invariant("Function source builder missing"))
    }
    fn parameter(mut self) -> Result<DynamicFunctionStep, RuntimeError> {
        if self.0.index < self.0.arguments.len().saturating_sub(1) {
            if self.0.index != 0 {
                self.source()?.push_str(",")?;
            }
            return Ok({
                let __pending_field_value = self.0.arguments[self.0.index].clone();
                let __pending_field_resume = self;
                DynamicFunctionStep::request_string(__pending_field_value, __pending_field_resume)
            });
        }
        self.source()?.push_str("\n) {\n")?;
        if let Some(value) = self.0.arguments.last().cloned() {
            self.0.phase = Phase::Body;
            return Ok({
                let __pending_field_value = value;
                let __pending_field_resume = self;
                DynamicFunctionStep::request_string(__pending_field_value, __pending_field_resume)
            });
        }
        self.eval()
    }
    pub(crate) fn string(
        mut self,
        result: NativeConversion<JsString>,
    ) -> Result<DynamicFunctionStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(DynamicFunctionStep::Complete(Completion::Throw(value)));
            }
        };
        self.source()?.push_js_string(&value)?;
        match self.0.phase {
            Phase::Parameters => {
                self.0.index += 1;
                self.parameter()
            }
            Phase::Body => self.eval(),
            _ => Err(RuntimeError::Invariant(
                "Function source string reply phase mismatch",
            )),
        }
    }
    fn eval(mut self) -> Result<DynamicFunctionStep, RuntimeError> {
        self.source()?.push_str("\n})")?;
        let source = self
            .0
            .source
            .take()
            .ok_or(RuntimeError::Invariant("Function source builder missing"))?
            .finish()?;
        self.0.phase = Phase::Eval;
        Ok({
            let __pending_field_source = source;
            let __pending_field_resume = self;
            DynamicFunctionStep::request_eval(__pending_field_source, __pending_field_resume)
        })
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<DynamicFunctionStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(DynamicFunctionStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Eval => {
                if matches!(self.0.new_target, Value::Undefined) {
                    return Ok(DynamicFunctionStep::Complete(Completion::Return(value)));
                }
                self.0.value = value;
                self.0.phase = Phase::Prototype;
                Ok({
                    let __pending_field_receiver = self.0.new_target.clone();
                    let __pending_field_key = runtime.intern_property_key("prototype")?;
                    let __pending_field_resume = self;
                    DynamicFunctionStep::request_read(
                        __pending_field_receiver,
                        __pending_field_key,
                        __pending_field_resume,
                    )
                })
            }
            Phase::Prototype => {
                let prototype = if let Value::Object(object) = value {
                    object
                } else {
                    let realm = match runtime
                        .function_realm_from_value(self.0.realm, &self.0.new_target)?
                    {
                        NativeConversion::Value(realm) => realm,
                        NativeConversion::Throw(value) => {
                            return Ok(DynamicFunctionStep::Complete(Completion::Throw(value)));
                        }
                    };
                    let prototype = {
                        let state = runtime.0.state.borrow();
                        let context = state.heap.context(realm)?;
                        match self.0.kind {
                            DynamicFunctionKind::Normal => context.function_prototype,
                            DynamicFunctionKind::Generator => context.generator.ok_or(RuntimeError::Invariant("dynamic GeneratorFunction realm has no Generator intrinsics"))?.function_prototype,
                            DynamicFunctionKind::Async => context.async_function.ok_or(RuntimeError::Invariant("dynamic AsyncFunction realm has no AsyncFunction intrinsics"))?.function_prototype,
                            DynamicFunctionKind::AsyncGenerator => context.async_generator.ok_or(RuntimeError::Invariant("dynamic AsyncGeneratorFunction realm has no AsyncGenerator intrinsics"))?.function_prototype,
                        }
                    };
                    ObjectRef::from_borrowed_handle(runtime.clone(), prototype)?
                };
                let Value::Object(function) = self.0.value else {
                    return Ok(DynamicFunctionStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "not an object",
                        )?,
                    )));
                };
                if !runtime.set_prototype_of(&function, Some(&prototype))? {
                    return Ok(DynamicFunctionStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "prototype is immutable",
                        )?,
                    )));
                }
                Ok(DynamicFunctionStep::Complete(Completion::Return(
                    Value::Object(function),
                )))
            }
            _ => Err(RuntimeError::Invariant(
                "Function constructor value reply phase mismatch",
            )),
        }
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: DynamicFunctionStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            DynamicFunctionStep::Complete(result) => return Ok(result),
            DynamicFunctionStep::String { mut resume } => {
                let value = resume.take_string_value();
                resume.string(runtime.native_to_dynamic_source_fragment(realm, &value)?)?
            }
            DynamicFunctionStep::Eval { mut resume } => {
                let source = resume.take_eval_source();
                resume.resume(
                    runtime,
                    runtime.execute_indirect_string_eval(realm, &source)?,
                )?
            }
            DynamicFunctionStep::Read { mut resume } => {
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
struct DynamicFunctionStepPending {
    string_value: Option<Value>,
    eval_source: Option<JsString>,
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
}
impl DynamicFunctionStep {
    pub(crate) fn request_string(value: Value, mut resume: DynamicFunctionResume) -> Self {
        resume.0.pending_effect.string_value = Some(value);
        Self::String { resume }
    }
    pub(crate) fn request_eval(source: JsString, mut resume: DynamicFunctionResume) -> Self {
        resume.0.pending_effect.eval_source = Some(source);
        Self::Eval { resume }
    }
    pub(crate) fn request_read(
        receiver: Value,
        key: PropertyKey,
        mut resume: DynamicFunctionResume,
    ) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
}
impl DynamicFunctionResume {
    pub(crate) fn take_string_value(&mut self) -> Value {
        self.0
            .pending_effect
            .string_value
            .take()
            .expect("DynamicFunctionStep String value")
    }
    pub(crate) fn take_eval_source(&mut self) -> JsString {
        self.0
            .pending_effect
            .eval_source
            .take()
            .expect("DynamicFunctionStep Eval source")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("DynamicFunctionStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("DynamicFunctionStep Read key")
    }
}
const _: () = assert!(std::mem::size_of::<DynamicFunctionStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<DynamicFunctionStep>() <= 64);
