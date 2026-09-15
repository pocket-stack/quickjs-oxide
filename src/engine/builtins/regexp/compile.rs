//! Legacy `%RegExp.prototype.compile%` mutation.

use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::heap::{ContextId, RegExpObjectData};
use crate::engine::object::ObjectRef;
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::call::{NativeArguments, NativeInvocation};
use crate::engine::vm::{Completion, ToPrimitiveHint};
use crate::regexp::CompiledRegExp;
use std::rc::Rc;

impl Runtime {
    /// Pinned QuickJS `js_regexp_compile`.
    ///
    /// This deliberately uses the concrete RegExp brand rather than
    /// `IsRegExp`: neither `@@match` nor public source/flag properties are
    /// observed. Compilation is transactional, but the internal replacement
    /// precedes the final throwing `lastIndex` Set and is not rolled back when
    /// that Set fails.
    pub(crate) fn call_regexp_compile(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let mut step = RegExpCompileStep::start(self, realm, &invocation, arguments)?;
        loop {
            step = match step {
                RegExpCompileStep::Complete(result) => return Ok(result),
                RegExpCompileStep::Primitive { value, resume } => {
                    let result = if matches!(value, Value::Object(_)) {
                        self.to_primitive(realm, value, ToPrimitiveHint::String)?
                    } else {
                        Completion::Return(value)
                    };
                    resume.resume(self, result)?
                }
            };
        }
    }
    fn finish_regexp_compile(
        &self,
        realm: ContextId,
        regexp: &ObjectRef,
        pattern: JsString,
        program: Rc<CompiledRegExp>,
    ) -> Result<Completion, RuntimeError> {
        let previous = self.0.state.borrow_mut().heap.replace_regexp_data(
            regexp.object_id(),
            RegExpObjectData::Compiled { pattern, program },
        )?;
        if !matches!(previous, RegExpObjectData::Compiled { .. }) {
            return Err(RuntimeError::Invariant(
                "observable RegExp object was not initialized",
            ));
        }

        let last_index = self.intern_property_key("lastIndex")?;
        if let Some(value) =
            self.set_property_or_throw(realm, regexp, &last_index, Value::Int(0))?
        {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(regexp.clone())))
    }
}

pub(crate) enum RegExpCompileStep {
    Complete(Completion),
    Primitive {
        value: Value,
        resume: RegExpCompileResume,
    },
}
pub(crate) struct RegExpCompileResume(Box<RegExpCompileResumeState>);
impl std::ops::Deref for RegExpCompileResume {
    type Target = RegExpCompileResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for RegExpCompileResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<RegExpCompileResume>() <= 8);
pub(crate) struct RegExpCompileResumeState {
    realm: ContextId,
    regexp: ObjectRef,
    flags: Value,
    phase: CompilePhase,
}
enum CompilePhase {
    Pattern,
    Flags(JsString),
}
impl RegExpCompileStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp.prototype.compile did not receive a generic invocation",
            ));
        };
        let Some(_) = runtime.genuine_regexp(this_value)? else {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error(realm, NativeErrorKind::Type, "RegExp object expected")?,
            )));
        };
        let Value::Object(regexp) = this_value else {
            return Err(RuntimeError::Invariant(
                "genuine RegExp snapshot accepted a primitive receiver",
            ));
        };
        let pattern = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "RegExp compile pattern argv was not padded",
        ))?;
        let flags = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "RegExp compile flags argv was not padded",
        ))?;
        if let Some(genuine) = runtime.genuine_regexp(pattern)? {
            if !matches!(flags, Value::Undefined) {
                return Ok(Self::Complete(Completion::Throw(
                    runtime.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "flags must be undefined",
                    )?,
                )));
            }
            return Ok(Self::Complete(runtime.finish_regexp_compile(
                realm,
                regexp,
                genuine.pattern,
                genuine.program,
            )?));
        }
        let resume = RegExpCompileResume(Box::new(RegExpCompileResumeState {
            realm,
            regexp: regexp.clone(),
            flags: flags.clone(),
            phase: CompilePhase::Pattern,
        }));
        if matches!(pattern, Value::Undefined) {
            resume.pattern(runtime, JsString::from_static(""))
        } else {
            Ok(Self::Primitive {
                value: pattern.clone(),
                resume,
            })
        }
    }
}
impl RegExpCompileResume {
    fn pattern(
        mut self,
        runtime: &Runtime,
        pattern: JsString,
    ) -> Result<RegExpCompileStep, RuntimeError> {
        if matches!(self.0.flags, Value::Undefined) {
            let program = Runtime::compile_regexp_program(&pattern, &JsString::from_static(""))?;
            Ok(RegExpCompileStep::Complete(runtime.finish_regexp_compile(
                self.0.realm,
                &self.0.regexp,
                pattern,
                program,
            )?))
        } else {
            Ok(RegExpCompileStep::Primitive {
                value: self.0.flags.clone(),
                resume: {
                    let updated_0 = CompilePhase::Flags(pattern);
                    self.0.phase = updated_0;
                    self
                },
            })
        }
    }
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<RegExpCompileStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(RegExpCompileStep::Complete(Completion::Throw(value)));
            }
        };
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "RegExp compile conversion returned an object",
            ));
        }
        let value = match runtime.native_to_js_string(self.0.realm, &value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(RegExpCompileStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            CompilePhase::Pattern => self.pattern(runtime, value),
            CompilePhase::Flags(pattern) => {
                let program = Runtime::compile_regexp_program(&pattern, &value)?;
                Ok(RegExpCompileStep::Complete(runtime.finish_regexp_compile(
                    self.0.realm,
                    &self.0.regexp,
                    pattern,
                    program,
                )?))
            }
        }
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<RegExpCompileStep>() <= 64);
