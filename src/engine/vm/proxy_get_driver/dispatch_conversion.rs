//! Bounded native-stack dispatch for conversion requests.
use super::{
    Error, Next, Query, Resume, ReturnOwner, RunningExecution, Runtime, Step, Value,
    runtime_error_to_vm_error,
};

#[inline(never)]
pub(super) fn primitive(
    runtime: &Runtime,
    _execution: &mut RunningExecution,
    _owner: ReturnOwner,
    _identity: u64,
    query: &mut Query,
    pending: &mut Step,
) -> Result<Next, Error> {
    let step = pending;
    loop {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event(
            "dispatch_conversion.primitive.visit",
        );
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("conversion_transition");
        let realm = query.realm;
        match &mut *step {
            Step::String { value, resume } => {
                let value = value.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                *step = Step::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                    resume: Some(Resume::StringValue {
                        realm,
                        resume: Box::new(resume),
                    }),
                };
                continue;
            }
            Step::OrdinaryPrimitive { object, hint } => {
                let object = object.take().expect("selected Step field");
                let hint = hint.take().expect("selected Step field");

                *step = crate::engine::value::conversion::primitive::PrimitiveResume::ordinary(
                    runtime, realm, object, hint,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::Arguments { value, resume } => {
                let value = value.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("argument continuation allocation failed"))?;
                query.parents.push(resume);
                *step = crate::engine::builtins::ArgumentsStep::start(runtime, realm, value)
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                continue;
            }
            Step::ArgumentsComplete(result) => {
                let result = result.take().expect("selected Step field");

                let parent = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("argument list lost its continuation"))?;
                *step = parent
                    .arguments(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            Step::Primitive {
                value,
                hint,
                resume,
            } => {
                let value = value.take().expect("selected Step field");
                let hint = hint.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                let next = crate::engine::value::conversion::primitive::PrimitiveResume::start(
                    runtime, realm, value, hint,
                );
                *step = match next {
                    crate::engine::value::conversion::primitive::PrimitiveStep::Complete(
                        result,
                    ) => resume
                        .resume(runtime, result)
                        .map_err(runtime_error_to_vm_error)?,
                    next => {
                        query.parents.try_reserve(1).map_err(|_| {
                            Error::internal("primitive continuation allocation failed")
                        })?;
                        query.parents.push(resume);
                        next.into()
                    }
                };
                continue;
            }
            Step::Number { value, resume } => {
                let value = value.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                // Only a request which can suspend needs a parent owner. Complete
                // results (including JS throws) use the same typed reply consumer.
                let next = crate::engine::value::conversion::number::NumberStep::start(
                    runtime, realm, value,
                )
                .map_err(runtime_error_to_vm_error)?;
                *step = match next {
                    crate::engine::value::conversion::number::NumberStep::Complete(result) => {
                        #[cfg(feature = "profiling")]
                        crate::engine::api::profiling::record_owned_execution_event(
                            "conversion_immediate",
                        );
                        resume
                            .number(runtime, result)
                            .map_err(runtime_error_to_vm_error)?
                            .into()
                    }
                    next => {
                        query.parents.try_reserve(1).map_err(|_| {
                            Error::internal("property continuation allocation failed")
                        })?;
                        query.parents.push(resume);
                        next.into()
                    }
                };
                continue;
            }
            Step::NumberComplete(result) => {
                let result = result.take().expect("selected Step field");

                let Some(resume) = query.parents.pop() else {
                    return Err(Error::internal(
                        "ToNumber result has no matching continuation",
                    ));
                };
                *step = resume
                    .number(runtime, result)
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                continue;
            }
            Step::LengthComplete(result) => {
                let result = result.take().expect("selected Step field");

                let resume = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("Array length result has no parent"))?;
                *step = resume
                    .length(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            Step::Element {
                element,
                value,
                resume,
            } => {
                let element = element.take().expect("selected Step field");
                let value = value.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                // Only a request which can suspend needs a parent owner. Complete
                // results (including JS throws) use the same typed reply consumer.
                let next =
                    crate::engine::builtins::ElementStep::start(runtime, realm, element, value)
                        .map_err(runtime_error_to_vm_error)?;
                *step = match next {
                    crate::engine::builtins::ElementStep::Complete(result) => resume
                        .element(runtime, result)
                        .map_err(runtime_error_to_vm_error)?
                        .into(),
                    next => {
                        query.parents.try_reserve(1).map_err(|_| {
                            Error::internal("property continuation allocation failed")
                        })?;
                        query.parents.push(resume);
                        next.into()
                    }
                };
                continue;
            }
            Step::ElementComplete(result) => {
                let result = result.take().expect("selected Step field");

                let Some(resume) = query.parents.pop() else {
                    return Err(Error::internal(
                        "element result has no matching continuation",
                    ));
                };
                *step = resume
                    .element(runtime, result)
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                continue;
            }
            Step::TypedComplete(result) => {
                let result = result.take().expect("selected Step field");

                let resume = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("TypedArray result has no parent"))?;
                *step = resume
                    .typed(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            _ => {
                return Ok(Next::Continue);
            }
        }
    }
}

#[inline(never)]
pub(super) fn constructor(
    runtime: &Runtime,
    _execution: &mut RunningExecution,
    _owner: ReturnOwner,
    _identity: u64,
    query: &mut Query,
    pending: &mut Step,
) -> Result<Next, Error> {
    let step = pending;
    loop {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event(
            "dispatch_conversion.constructor.visit",
        );
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("conversion_transition");
        let realm = query.realm;
        match &mut *step {
            Step::ConstructorSource { new_target, resume } => {
                let new_target = new_target.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("constructor source continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step = super::super::call::prototype::ProtoSourceStep::start(
                    runtime, realm, new_target,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::ConstructorSourceComplete(result) => {
                let result = result.take().expect("selected Step field");

                let parent = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("constructor source has no parent"))?;
                *step = parent
                    .constructor_source(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            Step::TypedSpeciesView {
                source,
                element,
                buffer,
                offset,
                length,
                resume,
            } => {
                let source = source.take().expect("selected Step field");
                let element = element.take().expect("selected Step field");
                let buffer = buffer.take().expect("selected Step field");
                let offset = offset.take().expect("selected Step field");
                let length = length.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("typed species view continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step = crate::engine::builtins::TypedSpeciesStep::start_view(
                    runtime, realm, source, element, buffer, offset, length,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::TypedIteratorMethod { source, resume } => {
                let source = source.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("typed iterator method continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step =
                    crate::engine::builtins::TypedIteratorMethodStep::start(runtime, realm, source)
                        .map_err(runtime_error_to_vm_error)?
                        .into();
                continue;
            }
            Step::TypedIteratorMethodComplete(result) => {
                let result = result.take().expect("selected Step field");

                let resume = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("typed iterator method lost parent"))?;
                *step = resume
                    .typed_iterator_method(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            Step::TypedCollect {
                source,
                method,
                element,
                resume,
            } => {
                let source = source.take().expect("selected Step field");
                let method = method.take().expect("selected Step field");
                let element = element.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("typed collection continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step = crate::engine::builtins::TypedCollectStep::start(
                    realm, source, method, element,
                )
                .into();
                continue;
            }
            Step::TypedCollectComplete(result) => {
                let result = result.take().expect("selected Step field");

                let resume = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("typed collection lost parent"))?;
                *step = resume
                    .typed_collected(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            Step::TypedCreate {
                constructor,
                length,
                resume,
            } => {
                let constructor = constructor.take().expect("selected Step field");
                let length = length.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("typed creation continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step = crate::engine::builtins::TypedSpeciesStep::create(
                    runtime,
                    realm,
                    constructor,
                    vec![Value::number(length as f64)],
                    Some(length),
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::TypedSpecies {
                source,
                element,
                length,
                resume,
            } => {
                let source = source.take().expect("selected Step field");
                let element = element.take().expect("selected Step field");
                let length = length.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("TypedArray species continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step = crate::engine::builtins::TypedSpeciesStep::start(
                    runtime, realm, source, element, length,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::TypedSpeciesComplete(result) => {
                let result = result.take().expect("selected Step field");

                let parent = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("TypedArray species has no parent"))?;
                *step = parent
                    .typed_species(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            _ => {
                return Ok(Next::Continue);
            }
        }
    }
}
