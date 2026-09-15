//! Mechanical adapters for iterator domain requests.
use super::{Completion, DirectCallTarget, Resume, Step, Value};

impl From<crate::engine::builtins::IteratorCloseStep> for Step {
    fn from(step: crate::engine::builtins::IteratorCloseStep) -> Self {
        use crate::engine::builtins::IteratorCloseStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read {
                object,
                key,
                resume,
            } => Self::Read {
                receiver: Some(Value::Object(object.clone())),
                object: Some(object),
                key: Some(key),
                resume: Some(Resume::IteratorClose(resume)),
            },
            T::Call {
                callable,
                iterator,
                resume,
            } => Self::Call {
                target: Some(DirectCallTarget::Callable(callable)),
                receiver: Some(Value::Object(iterator)),
                arguments: Some(Vec::new()),
                resume: Some(Resume::IteratorClose(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::IteratorNextStep> for Step {
    fn from(step: crate::engine::builtins::IteratorNextStep) -> Self {
        use crate::engine::builtins::IteratorNextStep as T;
        match step {
            T::Complete(result) => Self::IteratorNextComplete(Some(result)),
            T::Read {
                object,
                key,
                resume,
            } => Self::Read {
                receiver: Some(Value::Object(object.clone())),
                object: Some(object),
                key: Some(key),
                resume: Some(Resume::IteratorNext(resume)),
            },
            T::Call {
                callable,
                iterator,
                resume,
            } => Self::IteratorCall {
                callable: Some(callable),
                iterator: Some(iterator),
                resume: Some(resume),
            },
        }
    }
}

impl From<crate::engine::builtins::IteratorConsumeStep> for Step {
    fn from(step: crate::engine::builtins::IteratorConsumeStep) -> Self {
        use crate::engine::builtins::IteratorConsumeStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::IteratorConsume(resume)),
                }
            }
            T::Next { mut resume } => {
                let iterator = resume.take_next_iterator();
                let method = resume.take_next_method();
                Self::IteratorNext {
                    iterator: Some(iterator),
                    method: Some(method),
                    resume: Some(Resume::IteratorConsume(resume)),
                }
            }
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(Value::Undefined),
                    arguments: Some(arguments),
                    resume: Some(Resume::IteratorConsume(resume)),
                }
            }
            T::Close {
                iterator,
                completion,
            } => Self::IteratorClose {
                iterator: Some(iterator),
                completion: Some(completion),
            },
        }
    }
}

impl From<crate::engine::builtins::IteratorHelperStep> for Step {
    fn from(step: crate::engine::builtins::IteratorHelperStep) -> Self {
        use crate::engine::builtins::IteratorHelperStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::IteratorHelper(resume)),
                }
            }
            T::Next { mut resume } => {
                let iterator = resume.take_next_iterator();
                let method = resume.take_next_method();
                Self::IteratorNext {
                    iterator: Some(iterator),
                    method: Some(method),
                    resume: Some(Resume::IteratorHelper(resume)),
                }
            }
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::IteratorHelper(resume)),
                }
            }
            T::Close { mut resume } => {
                let iterator = resume.take_close_iterator();
                let completion = resume.take_close_completion();
                Self::IteratorCloseWithResume {
                    iterator: Some(iterator),
                    completion: Some(completion),
                    resume: Some(Resume::IteratorHelper(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::IteratorCreateStep> for Step {
    fn from(step: crate::engine::builtins::IteratorCreateStep) -> Self {
        use crate::engine::builtins::IteratorCreateStep as T;
        match step {
            T::CloseInvalidCount { iterator, resume } => Self::IteratorCloseWithResume {
                iterator: Some(iterator),
                completion: Some(Completion::Throw(Value::Undefined)),
                resume: Some(Resume::IteratorInvalidCount(resume)),
            },

            T::Complete(result) => Self::Complete(Some(result)),
            T::Read {
                object,
                key,
                resume,
            } => Self::Read {
                receiver: Some(Value::Object(object.clone())),
                object: Some(object),
                key: Some(key),
                resume: Some(Resume::IteratorCreate(resume)),
            },
            T::Number { value, resume } => Self::Number {
                value: Some(value),
                resume: Some(Resume::IteratorCreate(resume)),
            },
            T::Close {
                iterator,
                completion,
            } => Self::IteratorClose {
                iterator: Some(iterator),
                completion: Some(completion),
            },
        }
    }
}

impl From<crate::engine::builtins::IteratorFromStep> for Step {
    fn from(step: crate::engine::builtins::IteratorFromStep) -> Self {
        use crate::engine::builtins::IteratorFromStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::IteratorFrom(resume)),
                }
            }
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(Vec::new()),
                    resume: Some(Resume::IteratorFrom(resume)),
                }
            }
            T::Instance { mut resume } => {
                let constructor = resume.take_instance_constructor();
                let value = resume.take_instance_value();
                Self::OrdinaryInstance {
                    constructor: Some(constructor),
                    value: Some(value),
                    resume: Some(Resume::IteratorFrom(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::IteratorWrapStep> for Step {
    fn from(step: crate::engine::builtins::IteratorWrapStep) -> Self {
        use crate::engine::builtins::IteratorWrapStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::IteratorWrap(resume)),
                }
            }
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(Vec::new()),
                    resume: Some(Resume::IteratorWrap(resume)),
                }
            }
            T::Next { mut resume } => {
                let iterator = resume.take_next_iterator();
                let method = resume.take_next_method();
                Self::IteratorNext {
                    iterator: Some(iterator),
                    method: Some(method),
                    resume: Some(Resume::IteratorWrap(resume)),
                }
            }
            T::Parse { mut resume } => {
                let result = resume.take_parse_result();
                Self::ParseIterator {
                    result: Some(result),
                    resume: Some(Resume::IteratorWrap(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::IteratorConcatStep> for Step {
    fn from(step: crate::engine::builtins::IteratorConcatStep) -> Self {
        use crate::engine::builtins::IteratorConcatStep as T;
        match step {
            T::Complete(result) => Self::NativeRawComplete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::IteratorConcat(resume)),
                }
            }
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(Vec::new()),
                    resume: Some(Resume::IteratorConcat(resume)),
                }
            }
            T::Next { mut resume } => {
                let iterator = resume.take_next_iterator();
                let method = resume.take_next_method();
                Self::IteratorNext {
                    iterator: Some(iterator),
                    method: Some(method),
                    resume: Some(Resume::IteratorConcat(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::IteratorConstructorStep> for Step {
    fn from(step: crate::engine::builtins::IteratorConstructorStep) -> Self {
        use crate::engine::builtins::IteratorConstructorStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Prototype { new_target, resume } => Self::ConstructorSource {
                new_target: Some(new_target),
                resume: Some(Resume::IteratorConstructor(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::IteratorTagStep> for Step {
    fn from(step: crate::engine::builtins::IteratorTagStep) -> Self {
        use crate::engine::builtins::IteratorTagStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Own { mut resume } => {
                let object = resume.take_own_object();
                let key = resume.take_own_key();
                Self::OwnFlag {
                    object: Some(object),
                    key: Some(key),
                    enumerable: Some(false),
                    resume: Some(Resume::IteratorTag(resume)),
                }
            }
            T::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                Self::Define {
                    object: Some(object),
                    key: Some(key),
                    descriptor: Some(descriptor),
                    resume: Some(Resume::IteratorTag(resume)),
                }
            }
            T::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                Self::Set {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    value: Some(value),
                    resume: Some(Resume::IteratorTag(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::MapCallbackStep> for Step {
    fn from(step: crate::engine::builtins::MapCallbackStep) -> Self {
        use crate::engine::builtins::MapCallbackStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::MapCallback(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::SetEachStep> for Step {
    fn from(step: crate::engine::builtins::SetEachStep) -> Self {
        use crate::engine::builtins::SetEachStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::SetEach(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::WeakComputedStep> for Step {
    fn from(step: crate::engine::builtins::WeakComputedStep) -> Self {
        use crate::engine::builtins::WeakComputedStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Call {
                callable,
                arguments,
                resume,
            } => Self::Call {
                target: Some(DirectCallTarget::Callable(callable)),
                receiver: Some(Value::Undefined),
                arguments: Some(arguments),
                resume: Some(Resume::WeakComputed(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::SetOperationStep> for Step {
    fn from(step: crate::engine::builtins::SetOperationStep) -> Self {
        use crate::engine::builtins::SetOperationStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::SetOperation(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::SetOperation(resume)),
                }
            }
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::SetOperation(resume)),
                }
            }
            T::Parse { mut resume } => {
                let result = resume.take_parse_result();
                Self::ParseIterator {
                    result: Some(result),
                    resume: Some(Resume::SetOperation(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::CollectionStep> for Step {
    fn from(step: crate::engine::builtins::CollectionStep) -> Self {
        use crate::engine::builtins::CollectionStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Prototype { mut resume } => {
                let new_target = resume.take_prototype_new_target();
                Self::ConstructorSource {
                    new_target: Some(new_target),
                    resume: Some(Resume::Collection(resume)),
                }
            }
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::Collection(resume)),
                }
            }
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Collection(resume)),
                }
            }
            T::Next { mut resume } => {
                let iterator = resume.take_next_iterator();
                let method = resume.take_next_method();
                Self::IteratorNext {
                    iterator: Some(iterator),
                    method: Some(method),
                    resume: Some(Resume::Collection(resume)),
                }
            }
            T::Close { mut resume } => {
                let iterator = resume.take_close_iterator();
                let completion = resume.take_close_completion();
                Self::IteratorCloseWithResume {
                    iterator: Some(iterator),
                    completion: Some(completion),
                    resume: Some(Resume::Collection(resume)),
                }
            }
        }
    }
}
