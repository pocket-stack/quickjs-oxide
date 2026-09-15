//! Mechanical adapters for native domain requests.
use super::{Completion, Resume, Step, Value};

impl From<crate::engine::builtins::continuation::NativeStep> for Step {
    fn from(step: crate::engine::builtins::continuation::NativeStep) -> Self {
        use crate::engine::builtins::continuation::NativeStep;
        match step {
            NativeStep::ModuleCallback(step) => step.into(),
            #[cfg(feature = "test262-host")]
            NativeStep::Test262Agent(step) => step.into(),
            #[cfg(feature = "test262-host")]
            NativeStep::EvalScript(step) => step.into(),
            NativeStep::Async(step) => step.into(),
            NativeStep::FromSync(step) => step.into(),
            NativeStep::AsyncGenerator(step) => step.into(),
            NativeStep::Promise(step) => step.into(),
            NativeStep::GeneratorResume(step) => step.into(),
            NativeStep::Atomics(step) => step.into(),
            NativeStep::TypedCreate(step) => step.into(),
            NativeStep::BufferSlice(step) => step.into(),
            NativeStep::TypedWith(step) => step.into(),
            NativeStep::Uint8Codec(step) => step.into(),
            NativeStep::TypedSearch(step) => step.into(),
            NativeStep::TypedString(step) => step.into(),
            NativeStep::TypedSlice(step) => step.into(),
            NativeStep::TypedMutation(step) => step.into(),
            NativeStep::StringFactory(step) => step.into(),
            NativeStep::WeakConstructor(step) => step.into(),
            NativeStep::RegExpMatchAll(step) => step.into(),
            NativeStep::RegExpSplit(step) => step.into(),
            NativeStep::RegExpIterator(step) => step.into(),
            NativeStep::ObjectConstructor(step) => step.into(),
            NativeStep::Bind(step) => step.into(),
            NativeStep::FunctionText(step) => step.into(),
            NativeStep::DynamicFunction(step) => step.into(),
            NativeStep::JsonParse(step) => step.into(),
            NativeStep::JsonStringify(step) => step.into(),
            NativeStep::BufferConstructor(step) => step.into(),
            NativeStep::DataViewConstructor(step) => step.into(),
            NativeStep::TypedSet(step) => step.into(),
            NativeStep::RegExpConstructor(step) => step.into(),
            NativeStep::RegExpSearch(step) => step.into(),
            NativeStep::RegExpMatch(step) => step.into(),
            NativeStep::RegExpCompile(step) => step.into(),
            NativeStep::StringProtocol(step) => step.into(),
            NativeStep::GlobalEval(input) => match input {
                Value::String(source) => Self::IndirectEval {
                    source: Some(source),
                    resume: Some(Resume::Identity),
                },
                input => Self::Complete(Some(Completion::Return(input))),
            },
            NativeStep::JsonRaw { value, resume } => Self::String {
                value: Some(value),
                resume: Some(Resume::JsonRaw(resume)),
            },

            NativeStep::TypedSort(step) => step.into(),
            NativeStep::Math(step) => step.into(),
            NativeStep::Sum(step) => step.into(),
            NativeStep::PrimitiveConstructor(step) => step.into(),
            NativeStep::Global(step) => step.into(),
            NativeStep::Numeric(step) => step.into(),
            NativeStep::ScalarText(step) => step.into(),
            NativeStep::DateConstructor(step) => step.into(),
            NativeStep::DatePrototype(step) => step.into(),
            NativeStep::Error(step) => step.into(),
            NativeStep::MapCallback(step) => step.into(),
            NativeStep::SetEach(step) => step.into(),
            NativeStep::SetOperation(step) => step.into(),
            NativeStep::Collection(step) => step.into(),
            NativeStep::WeakComputed(step) => step.into(),

            NativeStep::ArrayConstructor(step) => step.into(),
            NativeStep::ArraySlice(step) => step.into(),
            NativeStep::IteratorConstructor(step) => step.into(),
            NativeStep::IteratorTag(step) => step.into(),
            NativeStep::TypedTraversal(step) => step.into(),
            NativeStep::TypedIteration(step) => step.into(),
            NativeStep::ArrayConcat(step) => step.into(),
            NativeStep::ArrayFlatten(step) => step.into(),
            NativeStep::StringText(step) => step.into(),
            NativeStep::StringSearch(step) => step.into(),
            NativeStep::StringSplit(step) => step.into(),
            NativeStep::Instance(step) => step.into(),
            NativeStep::IteratorFrom(step) => step.into(),
            NativeStep::IteratorWrap(step) => step.into(),
            NativeStep::IteratorConcat(step) => step.into(),
            NativeStep::ArrayBuild(step) => step.into(),
            NativeStep::ArraySort(step) => step.into(),
            NativeStep::ArrayIndexed(step) => step.into(),
            NativeStep::ArrayReverse(step) => step.into(),
            NativeStep::ArrayString(step) => step.into(),
            NativeStep::RegExpExec(step) => step.into(),
            NativeStep::RegExpPresentation(step) => step.into(),
            NativeStep::RegExpReplace(step) => step.into(),
            NativeStep::IteratorConsume(step) => step.into(),
            NativeStep::IteratorHelper(step) => step.into(),
            NativeStep::IteratorCreate(step) => step.into(),
            NativeStep::ArrayNext(step) => step.into(),
            NativeStep::Raw(result) => Self::NativeRawComplete(Some(result)),
            NativeStep::ArrayMutation(step) => step.into(),
            NativeStep::ArrayCallback(step) => step.into(),
            NativeStep::ObjectIteration(step) => step.into(),
            NativeStep::StringReplace(step) => step.into(),
            NativeStep::DataView(step) => step.into(),
            NativeStep::BufferMutation(step) => step.into(),
            NativeStep::Invoke(step) => step.into(),
            NativeStep::Prototype(step) => step.into(),
            NativeStep::Property(step) => step.into(),
            NativeStep::String(step) => step.into(),
            NativeStep::Complete(result) => Self::Complete(Some(result)),
            NativeStep::Definitions(step) => step.into(),
            NativeStep::Predicate(step) => step.into(),
        }
    }
}

impl From<crate::engine::vm::generator::GeneratorStep> for Step {
    fn from(step: crate::engine::vm::generator::GeneratorStep) -> Self {
        match step {
            crate::engine::vm::generator::GeneratorStep::Complete(outcome) => {
                Self::NativeRawComplete(Some(outcome))
            }
            crate::engine::vm::generator::GeneratorStep::Run {
                activation,
                input,
                resume,
            } => Self::ResumeFrame {
                activation: Some(activation),
                input: Some(input),
                resume: Some(Resume::Generator(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::promise::operation::PromiseStep> for Step {
    fn from(step: crate::engine::builtins::promise::operation::PromiseStep) -> Self {
        use crate::engine::builtins::promise::operation::PromiseStep as P;
        match step {
            P::Nested { mut resume } => {
                let step = resume.take_nested_step();
                Self::PromiseOperation {
                    step: Some(step),
                    resume: Some(Resume::Promise(resume)),
                }
            }
            P::Next { mut resume } => {
                let iterator = resume.take_next_iterator();
                let method = resume.take_next_method();
                Self::IteratorNext {
                    iterator: Some(iterator),
                    method: Some(method),
                    resume: Some(Resume::Promise(resume)),
                }
            }
            P::Close { mut resume } => {
                let iterator = resume.take_close_iterator();
                let completion = resume.take_close_completion();
                Self::IteratorCloseWithResume {
                    iterator: Some(iterator),
                    completion: Some(completion),
                    resume: Some(Resume::Promise(resume)),
                }
            }
            P::Complete(completion) => Self::Complete(Some(completion)),
            P::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::Promise(resume)),
                }
            }
            P::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(super::DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Promise(resume)),
                }
            }
            P::Construct { mut resume } => {
                let target = resume.take_construct_target();
                let arguments = resume.take_construct_arguments();
                Self::Construct {
                    new_target: Some(crate::engine::vm::call::ConstructNewTarget::Validated(
                        target.clone(),
                    )),
                    target: Some(target),
                    arguments: Some(arguments),
                    resume: Some(Resume::Promise(resume)),
                }
            }
            P::Prototype { mut resume } => {
                let new_target = resume.take_prototype_new_target();
                Self::ConstructorSource {
                    new_target: Some(new_target),
                    resume: Some(Resume::Promise(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::vm::async_from_sync_iterator::FromSyncStep> for Step {
    fn from(step: crate::engine::vm::async_from_sync_iterator::FromSyncStep) -> Self {
        use crate::engine::vm::async_from_sync_iterator::FromSyncStep as S;
        match step {
            S::Complete(completion) => Self::Complete(Some(completion)),
            S::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::FromSync(resume)),
                }
            }
            S::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(super::DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::FromSync(resume)),
                }
            }
            S::Resolve { mut resume } => {
                let value = resume.take_resolve_value();
                let realm = resume.take_resolve_realm();
                Self::IntrinsicPromiseResolve {
                    value: Some(value),
                    realm: Some(realm),
                    resume: Some(Resume::FromSync(resume)),
                }
            }
            S::Close { mut resume } => {
                let iterator = resume.take_close_iterator();
                let completion = resume.take_close_completion();
                Self::IteratorCloseWithResume {
                    iterator: Some(iterator),
                    completion: Some(completion),
                    resume: Some(Resume::FromSync(resume)),
                }
            }
        }
    }
}

#[cfg(feature = "test262-host")]
impl From<crate::engine::api::test262_host::operation::EvalScriptStep> for Step {
    fn from(step: crate::engine::api::test262_host::operation::EvalScriptStep) -> Self {
        use crate::engine::api::test262_host::operation::EvalScriptStep;
        match step {
            EvalScriptStep::Complete(result) => Self::Complete(Some(result)),
            EvalScriptStep::String { value, resume } => Self::String {
                value: Some(value),
                resume: Some(Resume::EvalScript(resume)),
            },
            EvalScriptStep::Call { callable, receiver } => Self::Call {
                target: Some(crate::engine::vm::call::DirectCallTarget::Callable(
                    callable,
                )),
                receiver: Some(receiver),
                arguments: Some(Vec::new()),
                resume: Some(Resume::Identity),
            },
        }
    }
}

#[cfg(feature = "test262-host")]
impl From<crate::engine::api::test262_agent::operation::AgentStep> for Step {
    fn from(step: crate::engine::api::test262_agent::operation::AgentStep) -> Self {
        use crate::engine::api::test262_agent::operation::AgentStep;
        match step {
            AgentStep::Complete(result) => Self::Complete(Some(result)),
            AgentStep::String { value, resume } => Self::String {
                value: Some(value),
                resume: Some(Resume::Test262Agent(resume)),
            },
            AgentStep::Number { value, resume } => Self::Number {
                value: Some(value),
                resume: Some(Resume::Test262Agent(resume)),
            },
        }
    }
}
