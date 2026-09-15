//! ArraySpeciesCreate owns constructor selection until its construction finishes.
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{ObjectRef, PropertyKey, WellKnownSymbol},
    value::{Value, conversion::NativeConversion},
    vm::{Completion, call::ConstructorRef},
};
pub(crate) enum SpeciesStep {
    Complete(Completion),
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: SpeciesResume,
    },
    Construct {
        target: ConstructorRef,
        arguments: Vec<Value>,
    },
}
enum Phase {
    Constructor,
    Species,
}
pub(crate) struct SpeciesResume {
    realm: ContextId,
    length: u64,
    phase: Phase,
}
impl SpeciesStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        source: &ObjectRef,
        length: u64,
    ) -> Result<Self, RuntimeError> {
        match runtime.internal_is_array(realm, &Value::Object(source.clone()))? {
            NativeConversion::Throw(value) => Ok(Self::Complete(Completion::Throw(value))),
            NativeConversion::Value(false) => allocate(runtime, realm, length),
            NativeConversion::Value(true) => Ok(Self::Read {
                object: source.clone(),
                key: runtime.intern_property_key("constructor")?,
                resume: SpeciesResume {
                    realm,
                    length,
                    phase: Phase::Constructor,
                },
            }),
        }
    }
}
fn allocate(runtime: &Runtime, realm: ContextId, length: u64) -> Result<SpeciesStep, RuntimeError> {
    // The fresh, unpublished Array and numeric length cannot call JavaScript.
    Ok(SpeciesStep::Complete(runtime.new_array_with_length(
        realm,
        Some(Value::number(length as f64)),
    )?))
}
impl SpeciesResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<SpeciesStep, RuntimeError> {
        let mut constructor = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(SpeciesStep::Complete(Completion::Throw(value))),
        };
        if matches!(self.phase, Phase::Constructor) {
            if let Value::Object(object) = &constructor
                && runtime.is_constructor(object)?
            {
                let constructor_realm =
                    match runtime.function_realm_from_value(self.realm, &constructor)? {
                        NativeConversion::Value(realm) => realm,
                        NativeConversion::Throw(value) => {
                            return Ok(SpeciesStep::Complete(Completion::Throw(value)));
                        }
                    };
                if constructor_realm != self.realm
                    && runtime
                        .0
                        .state
                        .borrow()
                        .heap
                        .context(constructor_realm)?
                        .array_constructor
                        .is_some_and(|default| default == object.object_id())
                {
                    constructor = Value::Undefined;
                }
            }
            if let Value::Object(object) = constructor {
                self.phase = Phase::Species;
                return Ok(SpeciesStep::Read {
                    object,
                    key: PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Species)),
                    resume: self,
                });
            }
        } else if matches!(constructor, Value::Null) {
            constructor = Value::Undefined;
        }
        if matches!(constructor, Value::Undefined) {
            return allocate(runtime, self.realm, self.length);
        }
        match runtime.constructor_from_value(self.realm, constructor)? {
            NativeConversion::Value(target) => Ok(SpeciesStep::Construct {
                target,
                arguments: vec![Value::number(self.length as f64)],
            }),
            NativeConversion::Throw(value) => Ok(SpeciesStep::Complete(Completion::Throw(value))),
        }
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: SpeciesStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            SpeciesStep::Complete(result) => return Ok(result),
            SpeciesStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
            SpeciesStep::Construct { target, arguments } => {
                return runtime.construct_constructor_internal(realm, &target, &target, &arguments);
            }
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<SpeciesStep>() <= 64);
