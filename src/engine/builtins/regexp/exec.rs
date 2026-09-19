//! Builtin and abstract RegExp execution.

use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::builtins::native::RegExpNativeKind;
use crate::engine::heap::ContextId;

use crate::engine::object::{CompleteOrdinaryPropertyDescriptor, ObjectRef, PropertyKey};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::{Completion, ToPrimitiveHint};

use crate::engine::vm::call::{DirectCallTarget, NativeArguments, NativeInvocation};
use crate::regexp::{
    ExecError, RegExpFlags, execute_latin1_with_interrupt, execute_with_interrupt,
};

impl Runtime {
    pub(crate) fn call_regexp_exec_native(
        &self,
        realm: ContextId,
        kind: RegExpNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            RegExpExecStep::start(self, realm, kind, &invocation, arguments)?,
        )
    }
    pub(crate) fn regexp_exec_abstract(
        &self,
        realm: ContextId,
        regexp: Value,
        input: Value,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            RegExpExecStep::abstract_exec(self, realm, regexp, input)?,
        )
    }
    fn finish_builtin_regexp_exec(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        input: JsString,
        last_index: u64,
    ) -> Result<Completion, RuntimeError> {
        let this_value = &Value::Object(object.clone());
        // QuickJS keeps the branded RegExp identity across both coercions, but
        // reads `re->bytecode` only afterwards. Either conversion may call the
        // legacy `compile()` method, so snapshot the current program and flags
        // only after those observable calls have completed.
        let current = self
            .genuine_regexp(this_value)?
            .ok_or(RuntimeError::Invariant(
                "branded RegExp lost its compiled payload during exec coercion",
            ))?;
        let program = current.program;
        let flags = program.flags();
        let updates_last_index =
            flags.contains(RegExpFlags::GLOBAL) || flags.contains(RegExpFlags::STICKY);
        let start = if updates_last_index { last_index } else { 0 };
        let flat = input.linearize();
        let matched = if start > flat.len() as u64 {
            None
        } else {
            let start = usize::try_from(start).expect("RegExp start bounded by String length");
            let execution = if let Some(units) = flat.flat_latin1() {
                execute_latin1_with_interrupt(program.as_ref(), units, start, || false)
            } else {
                execute_with_interrupt(
                    program.as_ref(),
                    flat.flat_utf16().expect("linearized input"),
                    start,
                    || false,
                )
            };
            match execution {
                Ok(value) => value,
                Err(ExecError::OutOfMemory) => {
                    return Ok(Completion::Throw(self.new_native_error_jsvalue(
                        realm,
                        NativeErrorKind::Internal,
                        "out of memory in regexp execution",
                    )?));
                }
                Err(ExecError::Interrupted) => {
                    return Ok(Completion::Throw(self.new_native_error_jsvalue(
                        realm,
                        NativeErrorKind::Internal,
                        "interrupted",
                    )?));
                }
                Err(ExecError::InvalidProgram(_)) => {
                    return Err(RuntimeError::Invariant(
                        "compiled RegExp program failed executor validation",
                    ));
                }
                Err(ExecError::StartOutOfBounds { .. }) => {
                    return Err(RuntimeError::Invariant(
                        "bounded RegExp start was rejected by executor",
                    ));
                }
            }
        };

        let Some(matched) = matched else {
            if updates_last_index
                && let Some(exception) = self.set_regexp_last_index(realm, object, 0)?
            {
                return Ok(Completion::Throw(exception));
            }
            return Ok(Completion::Return(Value::Null));
        };

        let complete = matched.capture(0).ok_or(RuntimeError::Invariant(
            "successful RegExp execution omitted capture zero",
        ))?;
        if updates_last_index {
            let end = i32::try_from(complete.end).map_err(|_| {
                RuntimeError::Invariant("RegExp match end exceeded signed String range")
            })?;
            // This write happens before any result/indices allocation.
            if let Some(exception) = self.set_regexp_last_index(realm, object, end)? {
                return Ok(Completion::Throw(exception));
            }
        }

        self.build_regexp_result(realm, input, program, matched)
            .map(Completion::Return)
    }

    fn regexp_last_index_value(&self, object: &ObjectRef) -> Result<Value, RuntimeError> {
        let key = self.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::LastIndex)?;
        let descriptor = self
            .get_own_property(object, &key)?
            .ok_or(RuntimeError::Invariant(
                "genuine RegExp object had no lastIndex property",
            ))?;
        let CompleteOrdinaryPropertyDescriptor::Data { value, .. } = descriptor else {
            return Err(RuntimeError::Invariant(
                "RegExp lastIndex became an accessor",
            ));
        };
        Ok(value)
    }

    pub(crate) fn set_regexp_last_index(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        value: i32,
    ) -> Result<Option<Value>, RuntimeError> {
        let key = self.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::LastIndex)?;
        self.set_property_or_throw(realm, object, &key, Value::Int(value))
    }
}

/// Shared RegExpExec owns the selected exec method and branded fallback state.
pub(crate) enum RegExpExecStep {
    Complete(Completion),
    Read { resume: RegExpExecResume },
    Primitive { resume: RegExpExecResume },
    Call { resume: RegExpExecResume },
}
pub(crate) struct RegExpExecResume(Box<RegExpExecResumeState>);
impl std::ops::Deref for RegExpExecResume {
    type Target = RegExpExecResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for RegExpExecResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<RegExpExecResume>() <= 8);
pub(crate) struct RegExpExecResumeState {
    step_pending: RegExpExecStepPending,
    realm: ContextId,
    regexp: Value,
    input: Value,
    test: bool,
    phase: ExecPhase,
}
enum ExecPhase {
    Method,
    Called,
    Input,
    LastIndex(JsString),
}
impl RegExpExecStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: RegExpNativeKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp exec/test did not receive a generic invocation",
            ));
        };
        let input = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(
                "RegExp exec/test input argv was not padded",
            ))?
            .clone();
        match kind {
            RegExpNativeKind::Exec => RegExpExecResume(Box::new(RegExpExecResumeState {
                step_pending: RegExpExecStepPending::default(),
                realm,
                regexp: this_value.clone(),
                input,
                test: false,
                phase: ExecPhase::Input,
            }))
            .builtin(runtime),
            RegExpNativeKind::Test => {
                Self::abstract_start(runtime, realm, this_value.clone(), input, true)
            }
            _ => Err(RuntimeError::Invariant(
                "non-exec RegExp selector reached exec dispatch",
            )),
        }
    }
    pub(crate) fn abstract_exec(
        runtime: &Runtime,
        realm: ContextId,
        regexp: Value,
        input: Value,
    ) -> Result<Self, RuntimeError> {
        Self::abstract_start(runtime, realm, regexp, input, false)
    }
    fn abstract_start(
        runtime: &Runtime,
        realm: ContextId,
        regexp: Value,
        input: Value,
        test: bool,
    ) -> Result<Self, RuntimeError> {
        let key = runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Exec)?;
        if matches!(regexp, Value::Null | Value::Undefined) {
            let base = if matches!(regexp, Value::Null) {
                "null"
            } else {
                "undefined"
            };
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(
                    realm,
                    NativeErrorKind::Type,
                    &format!("cannot read property 'exec' of {base}"),
                )?,
            )));
        }
        Ok(Self::make_read(
            regexp.clone(),
            key,
            RegExpExecResume(Box::new(RegExpExecResumeState {
                step_pending: RegExpExecStepPending::default(),
                realm,
                regexp,
                input,
                test,
                phase: ExecPhase::Method,
            })),
        ))
    }
}
impl RegExpExecResume {
    fn complete(&mut self, result: Completion) -> RegExpExecStep {
        RegExpExecStep::Complete(match result {
            Completion::Return(value) if self.0.test => {
                Completion::Return(Value::Bool(!matches!(value, Value::Null)))
            }
            result => result,
        })
    }
    fn builtin(mut self, runtime: &Runtime) -> Result<RegExpExecStep, RuntimeError> {
        if !matches!(&self.0.regexp, Value::Object(_))
            || runtime.genuine_regexp(&self.0.regexp)?.is_none()
        {
            return Ok(self.complete(Completion::Throw(runtime.new_native_error_jsvalue(
                self.0.realm,
                NativeErrorKind::Type,
                "RegExp object expected",
            )?)));
        }
        let input = self.0.input.clone();
        let resume = {
            let updated_0 = ExecPhase::Input;
            self.0.phase = updated_0;
            self
        };
        if matches!(input, Value::Object(_)) {
            Ok(RegExpExecStep::make_primitive(
                input,
                ToPrimitiveHint::String,
                resume,
            ))
        } else {
            resume.resume(runtime, Completion::Return(input))
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<RegExpExecStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(self.complete(Completion::Throw(value))),
        };
        match self.0.phase {
            ExecPhase::Method => {
                let callable = match &value {
                    Value::Object(object) => runtime.as_callable(object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    return self.builtin(runtime);
                };
                let mut arguments = Vec::new();
                if arguments.try_reserve_exact(1).is_err() {
                    return Ok(self.complete(Completion::Throw(runtime.new_native_error_jsvalue(
                        self.0.realm,
                        NativeErrorKind::Internal,
                        "out of memory",
                    )?)));
                }
                arguments.push(self.0.input.clone());
                Ok(RegExpExecStep::make_call(
                    DirectCallTarget::Callable(callable),
                    self.0.regexp.clone(),
                    arguments,
                    {
                        let updated_0 = ExecPhase::Called;
                        self.0.phase = updated_0;
                        self
                    },
                ))
            }
            ExecPhase::Called => {
                if matches!(value, Value::Object(_) | Value::Null) {
                    Ok(self.complete(Completion::Return(value)))
                } else {
                    Ok(self.complete(Completion::Throw(runtime.new_native_error_jsvalue(
                        self.0.realm,
                        NativeErrorKind::Type,
                        "RegExp exec method must return an object or null",
                    )?)))
                }
            }
            ExecPhase::Input => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "RegExp input conversion returned an object",
                    ));
                }
                let input = match runtime.native_to_js_string(self.0.realm, &value)? {
                    NativeConversion::Value(input) => input,
                    NativeConversion::Throw(value) => {
                        return Ok(self.complete(Completion::Throw(value)));
                    }
                };
                let Value::Object(object) = &self.0.regexp else {
                    return Err(RuntimeError::Invariant(
                        "RegExp input conversion lost its branded receiver",
                    ));
                };
                let value = runtime.regexp_last_index_value(object)?;
                let resume = {
                    let updated_0 = ExecPhase::LastIndex(input);
                    self.0.phase = updated_0;
                    self
                };
                if matches!(value, Value::Object(_)) {
                    Ok(RegExpExecStep::make_primitive(
                        value,
                        ToPrimitiveHint::Number,
                        resume,
                    ))
                } else {
                    // No callback: the branded receiver and input stay owned by
                    // this domain; the generic conversion/Query is never built.
                    resume.resume(runtime, Completion::Return(value))
                }
            }
            ExecPhase::LastIndex(input) => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "RegExp lastIndex conversion returned an object",
                    ));
                }
                let last_index = match runtime.native_to_length(self.0.realm, &value)? {
                    NativeConversion::Value(index) => index,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpExecStep::Complete(Completion::Throw(value)));
                    }
                };
                let Value::Object(object) = &self.0.regexp else {
                    return Err(RuntimeError::Invariant(
                        "RegExp lastIndex conversion lost its branded receiver",
                    ));
                };
                let result =
                    runtime.finish_builtin_regexp_exec(self.0.realm, object, input, last_index)?;
                Ok({
                    let updated_0 = ExecPhase::Called;
                    self.0.phase = updated_0;
                    self
                }
                .complete(result))
            }
        }
    }
}
fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: RegExpExecStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            RegExpExecStep::Complete(result) => return Ok(result),
            RegExpExecStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
            RegExpExecStep::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                let hint = resume.take_primitive_hint();
                {
                    let result = if matches!(value, Value::Object(_)) {
                        runtime.to_primitive(realm, value, hint)?
                    } else {
                        Completion::Return(value)
                    };
                    resume.resume(runtime, result)?
                }
            }
            RegExpExecStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                {
                    let DirectCallTarget::Callable(callable) = target else {
                        return Err(RuntimeError::Invariant(
                            "RegExp exec requested an invalid call target",
                        ));
                    };
                    resume.resume(
                        runtime,
                        runtime.call_internal(realm, &callable, receiver, &arguments)?,
                    )?
                }
            }
        };
    }
}

#[cfg(test)]
mod local_exec_tests {
    use super::*;

    #[test]
    fn primitive_regexp_exec_completes_inside_its_domain() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let invocation = NativeInvocation::Call {
            this_value: context.eval("/a/g").unwrap(),
        };
        let arguments = NativeArguments {
            actual_arg_count: 1,
            readable: vec![Value::String(JsString::from_static("a"))],
        };
        assert!(matches!(
            RegExpExecStep::start(
                &runtime,
                context.realm,
                RegExpNativeKind::Exec,
                &invocation,
                &arguments
            )
            .unwrap(),
            RegExpExecStep::Complete(Completion::Return(Value::Object(_)))
        ));
    }

    #[test]
    fn regexp_local_conversion_preserves_reentry_and_live_program() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let trace='', re=/a/g;
            const input={toString(){trace+='i';re.lastIndex={valueOf(){trace+='l';return 0}};return 'a'}};
            if(re.exec(input)[0]!=='a'||trace!=='il'||re.lastIndex!==1)return false;
            const marker={};re.lastIndex={valueOf(){throw marker}};
            try{re.exec('a');return false}catch(e){if(e!==marker)return false}
            const frozen=/a/g;Object.defineProperty(frozen,'lastIndex',{writable:false});
            try{frozen.exec('a');return false}catch(e){if(!(e instanceof TypeError))return false}
            return /é/.exec('é')[0]==='é' && /a/.test('a');
        })()"#).unwrap(),Value::Bool(true));
    }
}

#[derive(Default)]
pub(crate) struct RegExpExecStepPending {
    receiver: Option<Value>,
    key: Option<PropertyKey>,
    value: Option<Value>,
    hint: Option<ToPrimitiveHint>,
    target: Option<DirectCallTarget>,
    arguments: Option<Vec<Value>>,
}
impl RegExpExecStep {
    pub(crate) fn make_read(
        receiver: Value,
        key: PropertyKey,
        mut resume: RegExpExecResume,
    ) -> Self {
        resume.0.step_pending.receiver = Some(receiver);
        resume.0.step_pending.key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn make_primitive(
        value: Value,
        hint: ToPrimitiveHint,
        mut resume: RegExpExecResume,
    ) -> Self {
        resume.0.step_pending.value = Some(value);
        resume.0.step_pending.hint = Some(hint);
        Self::Primitive { resume }
    }
    pub(crate) fn make_call(
        target: DirectCallTarget,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: RegExpExecResume,
    ) -> Self {
        resume.0.step_pending.target = Some(target);
        resume.0.step_pending.receiver = Some(receiver);
        resume.0.step_pending.arguments = Some(arguments);
        Self::Call { resume }
    }
}
impl RegExpExecResume {
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .step_pending
            .receiver
            .take()
            .expect("RegExpExecStep::Read lost receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("RegExpExecStep::Read lost key")
    }

    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpExecStep::Primitive lost value")
    }
    pub(crate) fn take_primitive_hint(&mut self) -> ToPrimitiveHint {
        self.0
            .step_pending
            .hint
            .take()
            .expect("RegExpExecStep::Primitive lost hint")
    }

    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .step_pending
            .target
            .take()
            .expect("RegExpExecStep::Call lost target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .step_pending
            .receiver
            .take()
            .expect("RegExpExecStep::Call lost receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .step_pending
            .arguments
            .take()
            .expect("RegExpExecStep::Call lost arguments")
    }
}

const _: () = assert!(std::mem::size_of::<RegExpExecStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<RegExpExecStep>() <= 64);
