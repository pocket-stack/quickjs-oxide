//! Mechanical adapters for array domain requests.
use super::{DirectCallTarget, Resume, Step, Value};

impl From<crate::engine::builtins::ArrayMutationStep> for Step {
    fn from(step: crate::engine::builtins::ArrayMutationStep) -> Self {
        use crate::engine::builtins::ArrayMutationStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::PreparedRead { mut resume } => {
                let read = resume.take_prepared_read_read();
                let key = resume.take_prepared_read_key();
                Self::PreparedRead {
                    read: Some(read),
                    key: Some(key),
                    resume: Some(Resume::ArrayMutation(resume)),
                }
            }
            T::PreparedSet { mut resume } => {
                let step = resume.take_prepared_set_step();
                let key = resume.take_prepared_set_key();
                Self::PreparedSet {
                    step: Some(step),
                    resume: Some(Resume::ArrayMutationSet {
                        resume: resume.with_scheduler_set_key(key),
                    }),
                }
            }
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayMutation(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::ArrayMutation(resume)),
                }
            }
            T::Copy { mut resume } => {
                let object = resume.take_copy_object();
                let to = resume.take_copy_to();
                let from = resume.take_copy_from();
                let count = resume.take_copy_count();
                let backwards = resume.take_copy_backwards();
                Self::ArrayCopy {
                    object: Some(object),
                    to: Some(to),
                    from: Some(from),
                    count: Some(count),
                    backwards: Some(backwards),
                    resume: Some(Resume::ArrayMutation(resume)),
                }
            }
            T::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                Self::Set {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    value: Some(value),
                    key: Some(key.clone()),
                    resume: Some(Resume::ArrayMutationSet {
                        resume: resume.with_scheduler_set_key(key),
                    }),
                }
            }
            T::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                Self::Delete {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayMutation(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ArrayCallbackStep> for Step {
    fn from(step: crate::engine::builtins::ArrayCallbackStep) -> Self {
        use crate::engine::builtins::ArrayCallbackStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayCallback(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::ArrayCallback(resume)),
                }
            }
            T::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayCallback(resume)),
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
                    resume: Some(Resume::ArrayCallback(resume)),
                }
            }
            T::Species { mut resume } => {
                let source = resume.take_species_source();
                let length = resume.take_species_length();
                Self::ArraySpecies {
                    source: Some(source),
                    length: Some(length),
                    resume: Some(Resume::ArrayCallback(resume)),
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
                    resume: Some(Resume::ArrayCallback(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ArraySpeciesStep> for Step {
    fn from(step: crate::engine::builtins::ArraySpeciesStep) -> Self {
        use crate::engine::builtins::ArraySpeciesStep as T;
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
                resume: Some(Resume::ArraySpecies(resume)),
            },
            T::Construct { target, arguments } => Self::Construct {
                new_target: Some(crate::engine::vm::call::ConstructNewTarget::Validated(
                    target.clone(),
                )),
                target: Some(target),
                arguments: Some(arguments),
                resume: Some(Resume::Identity),
            },
        }
    }
}

impl From<crate::engine::builtins::ArrayNextStep> for Step {
    fn from(step: crate::engine::builtins::ArrayNextStep) -> Self {
        use crate::engine::builtins::ArrayNextStep as T;
        match step {
            T::Complete(result) => Self::NativeRawComplete(Some(result)),
            T::PreparedRead { mut resume } => {
                let read = resume.take_prepared();
                let key = resume.take_key();
                Self::PreparedRead {
                    read: Some(read),
                    key: Some(key),
                    resume: Some(Resume::ArrayNext(resume)),
                }
            }
            T::Read { mut resume } => {
                let (object, key) = resume.take_read();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayNext(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::ArrayNext(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ArraySortStep> for Step {
    fn from(step: crate::engine::builtins::ArraySortStep) -> Self {
        use crate::engine::builtins::ArraySortStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArraySort(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::ArraySort(resume)),
                }
            }
            T::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArraySort(resume)),
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
                    resume: Some(Resume::ArraySortSet {
                        resume: resume.with_scheduler_set_key(key),
                    }),
                }
            }
            T::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                Self::Delete {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArraySort(resume)),
                }
            }
            T::String { mut resume } => {
                let value = resume.take_string_value();
                Self::String {
                    value: Some(value),
                    resume: Some(Resume::ArraySort(resume)),
                }
            }
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(Value::Undefined),
                    arguments: Some(arguments),
                    resume: Some(Resume::ArraySort(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ArrayIndexedStep> for Step {
    fn from(step: crate::engine::builtins::ArrayIndexedStep) -> Self {
        use crate::engine::builtins::ArrayIndexedStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayIndexed(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::ArrayIndexed(resume)),
                }
            }
            T::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayIndexed(resume)),
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
                    resume: Some(Resume::ArrayIndexedSet {
                        resume: resume.with_scheduler_set_key(key),
                    }),
                }
            }
            T::Copy { mut resume } => {
                let object = resume.take_copy_object();
                let to = resume.take_copy_to();
                let from = resume.take_copy_from();
                let count = resume.take_copy_count();
                let backwards = resume.take_copy_backwards();
                Self::ArrayCopy {
                    object: Some(object),
                    to: Some(to),
                    from: Some(from),
                    count: Some(count),
                    backwards: Some(backwards),
                    resume: Some(Resume::ArrayIndexed(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ArrayReverseStep> for Step {
    fn from(step: crate::engine::builtins::ArrayReverseStep) -> Self {
        use crate::engine::builtins::ArrayReverseStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayReverse(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::ArrayReverse(resume)),
                }
            }
            T::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayReverse(resume)),
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
                    resume: Some(Resume::ArrayReverseSet {
                        resume: resume.with_scheduler_set_key(key),
                    }),
                }
            }
            T::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                Self::Delete {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayReverse(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ArrayStringStep> for Step {
    fn from(step: crate::engine::builtins::ArrayStringStep) -> Self {
        use crate::engine::builtins::ArrayStringStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::ArrayString(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::ArrayString(resume)),
                }
            }
            T::String { mut resume } => {
                let value = resume.take_string_value();
                Self::String {
                    value: Some(value),
                    resume: Some(Resume::ArrayString(resume)),
                }
            }
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(Vec::new()),
                    resume: Some(Resume::ArrayString(resume)),
                }
            }
            T::ObjectTag { receiver } => Self::ObjectTag {
                receiver: Some(receiver),
            },
        }
    }
}

impl From<crate::engine::builtins::ArrayBuildStep> for Step {
    fn from(step: crate::engine::builtins::ArrayBuildStep) -> Self {
        use crate::engine::builtins::ArrayBuildStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::ArrayBuild(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::ArrayBuild(resume)),
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
                    resume: Some(Resume::ArrayBuild(resume)),
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
                    resume: Some(Resume::ArrayBuildSet {
                        resume: resume.with_scheduler_set_key(key),
                    }),
                }
            }
            T::Close {
                iterator,
                completion,
            } => Self::IteratorClose {
                iterator: Some(iterator),
                completion: Some(completion),
            },
            T::Construct { mut resume } => {
                let target = resume.take_construct_target();
                let arguments = resume.take_construct_arguments();
                Self::Construct {
                    new_target: Some(crate::engine::vm::call::ConstructNewTarget::Validated(
                        target.clone(),
                    )),
                    target: Some(target),
                    arguments: Some(arguments),
                    resume: Some(Resume::ArrayBuild(resume)),
                }
            }
            T::Parse { mut resume } => {
                let result = resume.take_parse_result();
                Self::ParseIterator {
                    result: Some(result),
                    resume: Some(Resume::ArrayBuild(resume)),
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
                    resume: Some(Resume::ArrayBuild(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ArrayCopyStep> for Step {
    fn from(step: crate::engine::builtins::ArrayCopyStep) -> Self {
        use crate::engine::builtins::ArrayCopyStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayCopy(resume)),
                }
            }
            T::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayCopy(resume)),
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
                    resume: Some(Resume::ArrayCopySet {
                        resume: resume.with_scheduler_set_key(key),
                    }),
                }
            }
            T::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                Self::Delete {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayCopy(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ArrayConcatStep> for Step {
    fn from(step: crate::engine::builtins::ArrayConcatStep) -> Self {
        use crate::engine::builtins::ArrayConcatStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayConcat(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::ArrayConcat(resume)),
                }
            }
            T::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayConcat(resume)),
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
                    resume: Some(Resume::ArrayConcatSet {
                        resume: resume.with_scheduler_set_key(key),
                    }),
                }
            }
            T::Species { mut resume } => {
                let source = resume.take_species_source();
                Self::ArraySpecies {
                    source: Some(source),
                    length: Some(0),
                    resume: Some(Resume::ArrayConcat(resume)),
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
                    resume: Some(Resume::ArrayConcat(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ArrayFlattenStep> for Step {
    fn from(step: crate::engine::builtins::ArrayFlattenStep) -> Self {
        use crate::engine::builtins::ArrayFlattenStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayFlatten(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::ArrayFlatten(resume)),
                }
            }
            T::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArrayFlatten(resume)),
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
                    resume: Some(Resume::ArrayFlatten(resume)),
                }
            }
            T::Species { mut resume } => {
                let source = resume.take_species_source();
                Self::ArraySpecies {
                    source: Some(source),
                    length: Some(0),
                    resume: Some(Resume::ArrayFlatten(resume)),
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
                    resume: Some(Resume::ArrayFlatten(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ArrayConstructorStep> for Step {
    fn from(step: crate::engine::builtins::ArrayConstructorStep) -> Self {
        use crate::engine::builtins::ArrayConstructorStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::ArrayConstructor(resume)),
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
                    resume: Some(Resume::ArrayConstructorSet {
                        resume: resume.with_scheduler_set_key(key),
                    }),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ArraySliceStep> for Step {
    fn from(step: crate::engine::builtins::ArraySliceStep) -> Self {
        use crate::engine::builtins::ArraySliceStep as T;
        match step {
            T::Return(value) => Self::Complete(Some(crate::engine::vm::Completion::Return(value))),
            T::Throw(value) => Self::Complete(Some(crate::engine::vm::Completion::Throw(value))),
            T::PreparedRead { mut resume } => {
                let (read, key) = resume.take_preparedread();
                Self::PreparedRead {
                    read: Some(read),
                    key: Some(key),
                    resume: Some(Resume::ArraySlice(resume)),
                }
            }
            T::PreparedHas { mut resume } => {
                let (probe, key) = resume.take_preparedhas();
                Self::PreparedHas {
                    probe: Some(probe),
                    key: Some(key),
                    resume: Some(Resume::ArraySlice(resume)),
                }
            }
            T::Read { mut resume } => {
                let (object, key) = resume.take_read();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArraySlice(resume)),
                }
            }
            T::Number { mut resume } => {
                let (value,) = resume.take_number();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::ArraySlice(resume)),
                }
            }
            T::Has { mut resume } => {
                let (object, key) = resume.take_has();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArraySlice(resume)),
                }
            }
            T::Set { mut resume } => {
                let (object, key, value) = resume.take_set();
                Self::Set {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key.clone()),
                    value: Some(value),
                    resume: Some(Resume::ArraySliceSet {
                        resume: resume.with_scheduler_set_key(key),
                    }),
                }
            }
            T::Delete { mut resume } => {
                let (object, key) = resume.take_delete();
                Self::Delete {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ArraySlice(resume)),
                }
            }
            T::Species { mut resume } => {
                let (source, length) = resume.take_species();
                Self::ArraySpecies {
                    source: Some(source),
                    length: Some(length),
                    resume: Some(Resume::ArraySlice(resume)),
                }
            }
            T::Define { mut resume } => {
                let (object, key, descriptor) = resume.take_define();
                Self::Define {
                    object: Some(object),
                    key: Some(key),
                    descriptor: Some(descriptor),
                    resume: Some(Resume::ArraySlice(resume)),
                }
            }
            T::Copy { mut resume } => {
                let (object, to, from, count, backwards) = resume.take_copy();
                Self::ArrayCopy {
                    object: Some(object),
                    to: Some(to),
                    from: Some(from),
                    count: Some(count),
                    backwards: Some(backwards),
                    resume: Some(Resume::ArraySlice(resume)),
                }
            }
        }
    }
}
