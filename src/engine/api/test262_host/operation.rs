//! Test262 evalScript owns conversion and compilation before requesting execution.
use super::EVAL_SCRIPT_FILENAME;
use crate::engine::api::{
    compile::Compilation, error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError,
};
use crate::engine::heap::ContextId;
use crate::engine::object::CallableRef;
use crate::engine::value::Value;
use crate::engine::vm::{
    Completion,
    call::{NativeArguments, NativeInvocation},
};

pub(crate) enum EvalScriptStep {
    Complete(Completion),
    String {
        value: Value,
        resume: EvalScriptResume,
    },
    Call {
        callable: CallableRef,
        receiver: Value,
    },
}
pub(crate) struct EvalScriptResume {
    realm: ContextId,
}
impl EvalScriptStep {
    pub(crate) fn start(
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Test262 evalScript received a constructor invocation",
            ));
        };
        let source = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Test262 evalScript argument was not padded",
        ))?;
        Ok(Self::String {
            value: source.clone(),
            resume: EvalScriptResume { realm },
        })
    }
}
impl EvalScriptResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<EvalScriptStep, RuntimeError> {
        let source = match completion {
            Completion::Throw(value) => {
                return Ok(EvalScriptStep::Complete(Completion::Throw(value)));
            }
            Completion::Return(Value::String(source)) => source,
            _ => {
                return Err(RuntimeError::Invariant(
                    "evalScript conversion returned a non-string",
                ));
            }
        };
        let realm = self.realm;
        // The compiler currently accepts UTF-8 source rather than an exact
        // UTF-16 code-unit stream. Reject an unpaired surrogate explicitly;
        // lossy replacement would silently evaluate different JavaScript.
        let source_units = source.utf16_units().collect::<Vec<_>>();
        let source =
            match String::from_utf16(&source_units) {
                Ok(source) => source,
                Err(_) => {
                    return Ok(EvalScriptStep::Complete(Completion::Throw(runtime.new_native_error_jsvalue(
                    realm,
                    NativeErrorKind::Internal,
                    "evalScript source containing a lone UTF-16 surrogate is not implemented",
                )?)));
                }
            };

        let script = match runtime.compile_in_realm(realm, &source, EVAL_SCRIPT_FILENAME)? {
            Compilation::Published(script) => script,
            Compilation::Throw(value) => {
                return Ok(EvalScriptStep::Complete(Completion::Throw(value)));
            }
        };
        let callable = runtime.new_bytecode_closure(realm, &script)?;
        let global_object = runtime.global_object_for_realm(realm)?;
        Ok(EvalScriptStep::Call {
            callable,
            receiver: Value::Object(global_object),
        })
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<EvalScriptStep>() <= 64);
