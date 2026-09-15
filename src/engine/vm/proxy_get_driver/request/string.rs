//! Mechanical adapters for string domain requests.
use super::{Resume, Step, Value};

impl From<crate::engine::builtins::StringReplaceStep> for Step {
    fn from(step: crate::engine::builtins::StringReplaceStep) -> Self {
        use crate::engine::builtins::StringReplaceStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::PreparedRead { mut resume } => {
                let read = resume.take_preparedread_read();
                let key = resume.take_preparedread_key();
                Self::PreparedRead {
                    read: Some(read),
                    key: Some(key),
                    resume: Some(Resume::StringReplace(resume)),
                }
            }
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                    resume: Some(Resume::StringReplace(resume)),
                }
            }
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::StringReplace(resume)),
                }
            }
            T::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::StringReplace(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::RegExpExecStep> for Step {
    fn from(step: crate::engine::builtins::RegExpExecStep) -> Self {
        use crate::engine::builtins::RegExpExecStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::RegExpExec(resume)),
                }
            }
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                let hint = resume.take_primitive_hint();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(hint),
                    resume: Some(Resume::RegExpExec(resume)),
                }
            }
            T::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::RegExpExec(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::RegExpPresentationStep> for Step {
    fn from(step: crate::engine::builtins::RegExpPresentationStep) -> Self {
        use crate::engine::builtins::RegExpPresentationStep as T;
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
                resume: Some(Resume::RegExpPresentation(resume)),
            },
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                resume: Some(Resume::RegExpPresentation(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::RegExpReplaceStep> for Step {
    fn from(step: crate::engine::builtins::RegExpReplaceStep) -> Self {
        use crate::engine::builtins::RegExpReplaceStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::PreparedSet { mut resume } => {
                let step = resume.take_preparedset_step();
                Self::PreparedSet {
                    step: Some(step),
                    resume: Some(Resume::RegExpReplace(resume)),
                }
            }
            T::PreparedRead { mut resume } => {
                let read = resume.take_preparedread_read();
                let key = resume.take_preparedread_key();
                Self::PreparedRead {
                    read: Some(read),
                    key: Some(key),
                    resume: Some(Resume::RegExpReplace(resume)),
                }
            }
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::RegExpReplace(resume)),
                }
            }
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                let hint = resume.take_primitive_hint();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(hint),
                    resume: Some(Resume::RegExpReplace(resume)),
                }
            }
            T::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::RegExpReplace(resume)),
                }
            }
            T::Exec { mut resume } => {
                let regexp = resume.take_exec_regexp();
                let input = resume.take_exec_input();
                Self::RegExpExec {
                    regexp: Some(regexp),
                    input: Some(input),
                    resume: Some(Resume::RegExpReplace(resume)),
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
                    resume: Some(Resume::RegExpReplace(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::StringTextStep> for Step {
    fn from(step: crate::engine::builtins::StringTextStep) -> Self {
        use crate::engine::builtins::StringTextStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive {
                value,
                hint,
                resume,
            } => Self::Primitive {
                value: Some(value),
                hint: Some(hint),
                resume: Some(Resume::StringText(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::StringSearchStep> for Step {
    fn from(step: crate::engine::builtins::StringSearchStep) -> Self {
        use crate::engine::builtins::StringSearchStep as T;
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
                resume: Some(Resume::StringSearch(resume)),
            },
            T::Primitive {
                value,
                hint,
                resume,
            } => Self::Primitive {
                value: Some(value),
                hint: Some(hint),
                resume: Some(Resume::StringSearch(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::StringSplitStep> for Step {
    fn from(step: crate::engine::builtins::StringSplitStep) -> Self {
        use crate::engine::builtins::StringSplitStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::StringSplit(resume)),
                }
            }
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                let hint = resume.take_primitive_hint();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(hint),
                    resume: Some(Resume::StringSplit(resume)),
                }
            }
            T::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::StringSplit(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::RegExpConstructorStep> for Step {
    fn from(step: crate::engine::builtins::RegExpConstructorStep) -> Self {
        use crate::engine::builtins::RegExpConstructorStep as T;
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
                resume: Some(Resume::RegExpConstructor(resume)),
            },
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                resume: Some(Resume::RegExpConstructor(resume)),
            },
            T::Prototype { new_target, resume } => Self::ConstructorSource {
                new_target: Some(new_target),
                resume: Some(Resume::RegExpConstructor(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::RegExpCompileStep> for Step {
    fn from(step: crate::engine::builtins::RegExpCompileStep) -> Self {
        use crate::engine::builtins::RegExpCompileStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                resume: Some(Resume::RegExpCompile(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::StringProtocolStep> for Step {
    fn from(step: crate::engine::builtins::StringProtocolStep) -> Self {
        use crate::engine::builtins::StringProtocolStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::StringProtocol(resume)),
                }
            }
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                    resume: Some(Resume::StringProtocol(resume)),
                }
            }
            T::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::StringProtocol(resume)),
                }
            }
            T::Construct { mut resume } => {
                let constructor = resume.take_construct_constructor();
                let arguments = resume.take_construct_arguments();
                Self::Construct {
                    new_target: Some(crate::engine::vm::call::ConstructNewTarget::Validated(
                        constructor.clone(),
                    )),
                    target: Some(constructor),
                    arguments: Some(arguments),
                    resume: Some(Resume::StringProtocol(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::RegExpSearchStep> for Step {
    fn from(step: crate::engine::builtins::RegExpSearchStep) -> Self {
        use crate::engine::builtins::RegExpSearchStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::RegExpSearch(resume)),
                }
            }
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                    resume: Some(Resume::RegExpSearch(resume)),
                }
            }
            T::Exec { mut resume } => {
                let regexp = resume.take_exec_regexp();
                let input = resume.take_exec_input();
                Self::RegExpExec {
                    regexp: Some(regexp),
                    input: Some(input),
                    resume: Some(Resume::RegExpSearch(resume)),
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
                    resume: Some(Resume::RegExpSearch(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::RegExpMatchStep> for Step {
    fn from(step: crate::engine::builtins::RegExpMatchStep) -> Self {
        use crate::engine::builtins::RegExpMatchStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::RegExpMatch(resume)),
                }
            }
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                let hint = resume.take_primitive_hint();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(hint),
                    resume: Some(Resume::RegExpMatch(resume)),
                }
            }
            T::Exec { mut resume } => {
                let regexp = resume.take_exec_regexp();
                let input = resume.take_exec_input();
                Self::RegExpExec {
                    regexp: Some(regexp),
                    input: Some(input),
                    resume: Some(Resume::RegExpMatch(resume)),
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
                    resume: Some(Resume::RegExpMatch(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::RegExpMatchAllStep> for Step {
    fn from(step: crate::engine::builtins::RegExpMatchAllStep) -> Self {
        use crate::engine::builtins::RegExpMatchAllStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::RegExpMatchAll(resume)),
                }
            }
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                let hint = resume.take_primitive_hint();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(hint),
                    resume: Some(Resume::RegExpMatchAll(resume)),
                }
            }
            T::Species { mut resume } => {
                let regexp = resume.take_species_regexp();
                Self::RegExpSpecies {
                    regexp: Some(regexp),
                    resume: Some(Resume::RegExpMatchAll(resume)),
                }
            }
            T::Construct { mut resume } => {
                let constructor = resume.take_construct_constructor();
                let arguments = resume.take_construct_arguments();
                Self::Construct {
                    new_target: Some(crate::engine::vm::call::ConstructNewTarget::Validated(
                        constructor.clone(),
                    )),
                    target: Some(constructor),
                    arguments: Some(arguments),
                    resume: Some(Resume::RegExpMatchAll(resume)),
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
                    resume: Some(Resume::RegExpMatchAll(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::RegExpSplitStep> for Step {
    fn from(step: crate::engine::builtins::RegExpSplitStep) -> Self {
        use crate::engine::builtins::RegExpSplitStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::RegExpSplit(resume)),
                }
            }
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                let hint = resume.take_primitive_hint();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(hint),
                    resume: Some(Resume::RegExpSplit(resume)),
                }
            }
            T::Species { mut resume } => {
                let regexp = resume.take_species_regexp();
                Self::RegExpSpecies {
                    regexp: Some(regexp),
                    resume: Some(Resume::RegExpSplit(resume)),
                }
            }
            T::Construct { mut resume } => {
                let constructor = resume.take_construct_constructor();
                let arguments = resume.take_construct_arguments();
                Self::Construct {
                    new_target: Some(crate::engine::vm::call::ConstructNewTarget::Validated(
                        constructor.clone(),
                    )),
                    target: Some(constructor),
                    arguments: Some(arguments),
                    resume: Some(Resume::RegExpSplit(resume)),
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
                    resume: Some(Resume::RegExpSplit(resume)),
                }
            }
            T::Exec { mut resume } => {
                let regexp = resume.take_exec_regexp();
                let input = resume.take_exec_input();
                Self::RegExpExec {
                    regexp: Some(regexp),
                    input: Some(input),
                    resume: Some(Resume::RegExpSplit(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::RegExpSpeciesStep> for Step {
    fn from(step: crate::engine::builtins::RegExpSpeciesStep) -> Self {
        use crate::engine::builtins::RegExpSpeciesStep as T;
        match step {
            T::Complete(result) => Self::RegExpSpeciesComplete(Some(result)),
            T::Read {
                object,
                key,
                resume,
            } => Self::Read {
                receiver: Some(Value::Object(object.clone())),
                object: Some(object),
                key: Some(key),
                resume: Some(Resume::RegExpSpecies(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::RegExpIteratorStep> for Step {
    fn from(step: crate::engine::builtins::RegExpIteratorStep) -> Self {
        use crate::engine::builtins::RegExpIteratorStep as T;
        match step {
            T::Complete(result) => Self::NativeRawComplete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::RegExpIterator(resume)),
                }
            }
            T::String { mut resume } => {
                let value = resume.take_string_value();
                Self::String {
                    value: Some(value),
                    resume: Some(Resume::RegExpIterator(resume)),
                }
            }
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                    resume: Some(Resume::RegExpIterator(resume)),
                }
            }
            T::Exec { mut resume } => {
                let regexp = resume.take_exec_regexp();
                let input = resume.take_exec_input();
                Self::RegExpExec {
                    regexp: Some(regexp),
                    input: Some(input),
                    resume: Some(Resume::RegExpIterator(resume)),
                }
            }
            T::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                Self::Set {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key.clone()),
                    value: Some(value),
                    resume: Some(Resume::RegExpIteratorSet {
                        resume: resume.with_scheduler_set_key(key),
                    }),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::StringFactoryStep> for Step {
    fn from(step: crate::engine::builtins::StringFactoryStep) -> Self {
        use crate::engine::builtins::StringFactoryStep as T;
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
                resume: Some(Resume::StringFactory(resume)),
            },
            T::Number { value, resume } => Self::Number {
                value: Some(value),
                resume: Some(Resume::StringFactory(resume)),
            },
            T::String { value, resume } => Self::String {
                value: Some(value),
                resume: Some(Resume::StringFactory(resume)),
            },
        }
    }
}
