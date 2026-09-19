//! Test262 agent role checks and conversions retain the selected host session.
use super::{
    AgentRole, Test262AgentSession, install_worker_callback, lock_unpoisoned,
    registered_session_and_role,
};
use crate::engine::api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::builtins::native::Test262AgentKind;
use crate::engine::heap::{ContextId, shared_memory::SharedBufferHandle};
use crate::engine::value::{JsString, Value, conversion::NativeConversion};
use crate::engine::vm::{
    Completion,
    call::{NativeArguments, NativeInvocation},
};

pub(crate) enum AgentStep {
    Complete(Completion),
    String { value: Value, resume: AgentResume },
    Number { value: Value, resume: AgentResume },
}
enum Phase {
    Start,
    Report,
    Broadcast(SharedBufferHandle),
    Sleep,
}
pub(crate) struct AgentResume(Box<AgentResumeState>);
impl std::ops::Deref for AgentResume {
    type Target = AgentResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for AgentResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<AgentResume>() <= 8);
pub(crate) struct AgentResumeState {
    realm: ContextId,
    session: Test262AgentSession,
    phase: Phase,
}
impl AgentStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: Test262AgentKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Test262 agent function received a constructor invocation",
            ));
        };
        let Some((session, role)) = registered_session_and_role(runtime, realm) else {
            return Err(RuntimeError::Invariant(
                "Test262 agent function has no registered session",
            ));
        };
        match kind {
            Test262AgentKind::Start | Test262AgentKind::Broadcast => {
                if role == AgentRole::Worker {
                    return runtime
                        .test262_agent_type_error(realm, "cannot be called inside an agent")
                        .map(Self::Complete);
                }
                if kind == Test262AgentKind::Start {
                    Ok(Self::String {
                        value: arguments.readable[0].clone(),
                        resume: AgentResume(Box::new(AgentResumeState {
                            realm,
                            session,
                            phase: Phase::Start,
                        })),
                    })
                } else {
                    // Brand/detached/shared checks precede observable numeric conversion.
                    let handle = match runtime
                        .test262_agent_export_broadcast_buffer(realm, &arguments.readable[0])?
                    {
                        NativeConversion::Value(handle) => handle,
                        NativeConversion::Throw(value) => {
                            return Ok(Self::Complete(Completion::Throw(value)));
                        }
                    };
                    Ok(Self::Number {
                        value: arguments.readable[1].clone(),
                        resume: AgentResume(Box::new(AgentResumeState {
                            realm,
                            session,
                            phase: Phase::Broadcast(handle),
                        })),
                    })
                }
            }
            Test262AgentKind::Report => Ok(Self::String {
                value: arguments.readable[0].clone(),
                resume: AgentResume(Box::new(AgentResumeState {
                    realm,
                    session,
                    phase: Phase::Report,
                })),
            }),
            Test262AgentKind::Sleep => Ok(Self::Number {
                value: arguments.readable[0].clone(),
                resume: AgentResume(Box::new(AgentResumeState {
                    realm,
                    session,
                    phase: Phase::Sleep,
                })),
            }),
            Test262AgentKind::GetReport => (|| -> Result<Completion, RuntimeError> {
                let report = lock_unpoisoned(&session.inner.reports).pop_front();
                Ok(Completion::Return(match report {
                    Some(report) => Value::String(JsString::try_from_utf8(&report)?),
                    None => Value::Null,
                }))
            })()
            .map(Self::Complete),
            Test262AgentKind::Leaving => (|| -> Result<Completion, RuntimeError> {
                if role == AgentRole::Main {
                    return runtime
                        .test262_agent_type_error(realm, "must be called inside an agent");
                }
                // Pinned QuickJS performs no state transition or signal here.
                Ok(Completion::Return(Value::Undefined))
            })()
            .map(Self::Complete),
            Test262AgentKind::ReceiveBroadcast => (|| -> Result<Completion, RuntimeError> {
                if role == AgentRole::Main {
                    return runtime
                        .test262_agent_type_error(realm, "must be called inside an agent");
                }
                let callback = match &arguments.readable[0] {
                    Value::Object(object) => runtime.as_callable(object)?,
                    _ => None,
                };
                let Some(callback) = callback else {
                    return runtime.test262_agent_type_error(realm, "expecting function");
                };
                install_worker_callback(runtime.domain_id(), callback);
                Ok(Completion::Return(Value::Undefined))
            })()
            .map(Self::Complete),
            Test262AgentKind::MonotonicNow => {
                let milliseconds = session.inner.clock_origin.elapsed().as_millis();
                #[allow(clippy::cast_precision_loss)]
                Ok(Completion::Return(Value::Float(milliseconds as f64)))
            }
            .map(Self::Complete),
        }
    }
}
impl AgentResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<AgentStep, RuntimeError> {
        let source = match completion {
            Completion::Throw(value) => return Ok(AgentStep::Complete(Completion::Throw(value))),
            Completion::Return(Value::String(value)) => value,
            _ => {
                return Err(RuntimeError::Invariant(
                    "agent string conversion returned a non-string",
                ));
            }
        };
        let state = *self.0;
        let realm = state.realm;
        let session = &state.session;
        let result = (|| -> Result<Completion, RuntimeError> {
            match state.phase {
                Phase::Start => {
                    let source = match String::from_utf16(&source.utf16_units().collect::<Vec<_>>())
                    {
                        Ok(source) => source,
                        Err(_) => {
                            return Ok(Completion::Throw(runtime.new_native_error_jsvalue(
                            realm,
                            NativeErrorKind::Internal,
                            "agent source containing a lone UTF-16 surrogate is not implemented",
                        )?));
                        }
                    };
                    if let Err(error) = session.start_worker(source) {
                        return Ok(Completion::Throw(runtime.new_native_error_jsvalue(
                            realm,
                            NativeErrorKind::Internal,
                            &error,
                        )?));
                    }
                    Ok(Completion::Return(Value::Undefined))
                }
                Phase::Report => {
                    let report = source;
                    lock_unpoisoned(&session.inner.reports).push_back(report.to_utf8_lossy());
                    Ok(Completion::Return(Value::Undefined))
                }
                _ => Err(RuntimeError::Invariant(
                    "agent received an unexpected string reply",
                )),
            }
        })()?;
        Ok(AgentStep::Complete(result))
    }
    pub(crate) fn number(
        self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<AgentStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(AgentStep::Complete(Completion::Throw(value)));
            }
        };
        let state = *self.0;
        let realm = state.realm;
        let session = &state.session;
        let result = (|| -> Result<Completion, RuntimeError> {
            match state.phase {
                Phase::Broadcast(handle) => {
                    let value = crate::engine::value::number::to_int32(value);
                    if let Err(error) = session.broadcast(handle, value) {
                        return Ok(Completion::Throw(runtime.new_native_error_jsvalue(
                            realm,
                            NativeErrorKind::Internal,
                            &error,
                        )?));
                    }
                    Ok(Completion::Return(Value::Undefined))
                }
                Phase::Sleep => {
                    let duration = Runtime::to_uint32_number(value);
                    #[cfg(not(target_family = "wasm"))]
                    std::thread::sleep(std::time::Duration::from_millis(u64::from(duration)));
                    #[cfg(target_family = "wasm")]
                    if duration != 0 {
                        return self.test262_agent_type_error(
                            realm,
                            "sleep is unavailable on wasm targets",
                        );
                    }
                    Ok(Completion::Return(Value::Undefined))
                }
                _ => Err(RuntimeError::Invariant(
                    "agent received an unexpected number reply",
                )),
            }
        })()?;
        Ok(AgentStep::Complete(result))
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<AgentStep>() <= 64);
