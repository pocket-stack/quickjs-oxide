//! Concat preserves species selection and per-element spreadability/property order.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{
        DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
        operations::{InternalDefineResult, InternalSetResult},
    },
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum ConcatStep {
    Complete(Completion),
    Species { resume: ConcatResume },
    Read { resume: ConcatResume },
    Number { resume: ConcatResume },
    Has { resume: ConcatResume },
    Define { resume: ConcatResume },
    Set { resume: ConcatResume },
}
enum Phase {
    Species,
    Spread,
    Length,
    Number,
    Has,
    Read,
    Define(bool),
    Set,
}
pub(crate) struct ConcatResume(Box<ConcatResumeState>);
impl std::ops::Deref for ConcatResume {
    type Target = ConcatResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ConcatResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ConcatResume>() <= 8);
pub(crate) struct ConcatResumeState {
    pending_effect: ConcatStepPending,
    scheduler_set_key: Option<PropertyKey>,
    realm: ContextId,
    phase: Phase,
    result: Option<ObjectRef>,
    elements: std::vec::IntoIter<Value>,
    element: Value,
    next_index: u64,
    index: u64,
    length: u64,
}
impl ConcatStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array concat requires generic invocation",
            ));
        };
        let source = match runtime.native_to_object(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let mut elements = Vec::with_capacity(arguments.actual_arg_count + 1);
        elements.push(Value::Object(source.clone()));
        elements.extend(
            arguments.readable[..arguments.actual_arg_count]
                .iter()
                .cloned(),
        );
        Ok(Self::request_species(
            source,
            ConcatResume(Box::new(ConcatResumeState {
                pending_effect: ConcatStepPending::default(),
                scheduler_set_key: None,
                realm,
                phase: Phase::Species,
                result: None,
                elements: elements.into_iter(),
                element: Value::Undefined,
                next_index: 0,
                index: 0,
                length: 0,
            })),
        ))
    }
}
impl ConcatResume {
    pub(crate) fn with_scheduler_set_key(mut self, key: PropertyKey) -> Self {
        self.0.scheduler_set_key = Some(key);
        self
    }
    pub(crate) fn take_scheduler_set_key(&mut self) -> PropertyKey {
        self.0.scheduler_set_key.take().expect("waiting Set key")
    }

    fn object(&self) -> Result<ObjectRef, RuntimeError> {
        match &self.0.element {
            Value::Object(object) => Ok(object.clone()),
            _ => Err(RuntimeError::Invariant(
                "Array concat spread source missing",
            )),
        }
    }
    fn result(&self) -> Result<ObjectRef, RuntimeError> {
        self.0
            .result
            .clone()
            .ok_or(RuntimeError::Invariant("Array concat result missing"))
    }
    fn too_long(&self, runtime: &Runtime) -> Result<ConcatStep, RuntimeError> {
        Ok(ConcatStep::Complete(Completion::Throw(
            runtime.new_native_error_jsvalue(self.0.realm, NativeErrorKind::Type, "Array loo long")?,
        )))
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<ConcatStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(ConcatStep::Complete(Completion::Throw(value))),
        };
        match self.0.phase {
            Phase::Species => {
                let Value::Object(object) = value else {
                    return Err(RuntimeError::Invariant(
                        "ArraySpeciesCreate returned a primitive",
                    ));
                };
                self.0.result = Some(object);
                self.next(runtime)
            }
            Phase::Spread => {
                let spread = if matches!(value, Value::Undefined) {
                    match runtime.internal_is_array(self.0.realm, &self.0.element)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(ConcatStep::Complete(Completion::Throw(value)));
                        }
                    }
                } else {
                    runtime.value_to_boolean(&value)?
                };
                if spread {
                    self.0.phase = Phase::Length;
                    Ok(ConcatStep::request_read(
                        self.object()?,
                        runtime
                            .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)?,
                        self,
                    ))
                } else {
                    self.single(runtime)
                }
            }
            Phase::Length => {
                self.0.phase = Phase::Number;
                Ok(ConcatStep::request_number(value, self))
            }
            Phase::Read => self.define(runtime, value, true),
            _ => Err(RuntimeError::Invariant("Array concat value phase mismatch")),
        }
    }
    fn next(mut self, runtime: &Runtime) -> Result<ConcatStep, RuntimeError> {
        let Some(element) = self.0.elements.next() else {
            self.0.phase = Phase::Set;
            return Ok(ConcatStep::request_set(
                self.result()?,
                runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)?,
                Value::number(self.0.next_index as f64),
                self,
            ));
        };
        self.0.element = element;
        self.0.index = 0;
        if let Value::Object(object) = &self.0.element {
            let object = object.clone();
            self.0.phase = Phase::Spread;
            Ok(ConcatStep::request_read(
                object,
                PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::IsConcatSpreadable)),
                self,
            ))
        } else {
            self.single(runtime)
        }
    }
    fn single(self, runtime: &Runtime) -> Result<ConcatStep, RuntimeError> {
        if self.0.next_index >= (1_u64 << 53) - 1 {
            return self.too_long(runtime);
        }
        let value = self.0.element.clone();
        self.define(runtime, value, false)
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<ConcatStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Number) {
            return Err(RuntimeError::Invariant(
                "Array concat number phase mismatch",
            ));
        }
        self.0.length = match result {
            NativeConversion::Value(value) => Runtime::length_from_number(value),
            NativeConversion::Throw(value) => {
                return Ok(ConcatStep::Complete(Completion::Throw(value)));
            }
        };
        if self.0.next_index.saturating_add(self.0.length) > (1_u64 << 53) - 1 {
            return self.too_long(runtime);
        }
        self.indexed(runtime)
    }
    fn indexed(mut self, runtime: &Runtime) -> Result<ConcatStep, RuntimeError> {
        if self.0.index == self.0.length {
            return self.next(runtime);
        }
        self.0.phase = Phase::Has;
        Ok(ConcatStep::request_has(
            self.object()?,
            runtime.property_key_for_index(self.0.index)?,
            self,
        ))
    }
    pub(crate) fn boolean(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<ConcatStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Has) {
            return Err(RuntimeError::Invariant(
                "Array concat boolean phase mismatch",
            ));
        }
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ConcatStep::Complete(Completion::Throw(value)));
            }
        };
        if value {
            self.0.phase = Phase::Read;
            Ok(ConcatStep::request_read(
                self.object()?,
                runtime.property_key_for_index(self.0.index)?,
                self,
            ))
        } else {
            self.0.index += 1;
            self.0.next_index += 1;
            self.indexed(runtime)
        }
    }
    fn define(
        mut self,
        runtime: &Runtime,
        value: Value,
        indexed: bool,
    ) -> Result<ConcatStep, RuntimeError> {
        self.0.phase = Phase::Define(indexed);
        Ok(ConcatStep::request_define(
            self.result()?,
            runtime.property_key_for_index(self.0.next_index)?,
            OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
            self,
        ))
    }
    pub(crate) fn defined(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<InternalDefineResult>,
    ) -> Result<ConcatStep, RuntimeError> {
        let Phase::Define(indexed) = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "Array concat define phase mismatch",
            ));
        };
        if let Some(value) =
            runtime.finish_create_indexed_data_property(self.0.realm, self.0.next_index, result)?
        {
            return Ok(ConcatStep::Complete(Completion::Throw(value)));
        }
        self.0.next_index += 1;
        if indexed {
            self.0.index += 1;
            self.indexed(runtime)
        } else {
            self.next(runtime)
        }
    }
    pub(crate) fn set(
        self,
        runtime: &Runtime,
        key: PropertyKey,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<ConcatStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Set) {
            return Err(RuntimeError::Invariant("Array concat set phase mismatch"));
        }
        if let Some(value) = runtime.finish_set_property_or_throw(self.0.realm, &key, result)? {
            return Ok(ConcatStep::Complete(Completion::Throw(value)));
        }
        Ok(ConcatStep::Complete(Completion::Return(Value::Object(
            self.result()?,
        ))))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ConcatStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            ConcatStep::Complete(result) => return Ok(result),
            ConcatStep::Species { mut resume } => {
                let source = resume.take_species_source();
                resume.resume(
                    runtime,
                    super::species::finish(
                        runtime,
                        realm,
                        super::species::SpeciesStep::start(runtime, realm, &source, 0)?,
                    )?,
                )?
            }
            ConcatStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            ConcatStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            ConcatStep::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                resume.boolean(
                    runtime,
                    runtime.internal_has_property(realm, &object, &key)?,
                )?
            }
            ConcatStep::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                resume.defined(
                    runtime,
                    runtime.internal_define_own_property(realm, &object, &key, &descriptor)?,
                )?
            }
            ConcatStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                {
                    let result = runtime.internal_set(
                        realm,
                        &object,
                        &key,
                        value,
                        Value::Object(object.clone()),
                    )?;
                    resume.set(runtime, key, result)?
                }
            }
        };
    }
}

#[derive(Default)]
struct ConcatStepPending {
    species_source: Option<ObjectRef>,
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    number_value: Option<Value>,
    has_object: Option<ObjectRef>,
    has_key: Option<PropertyKey>,
    define_object: Option<ObjectRef>,
    define_key: Option<PropertyKey>,
    define_descriptor: Option<OrdinaryPropertyDescriptor>,
    set_object: Option<ObjectRef>,
    set_key: Option<PropertyKey>,
    set_value: Option<Value>,
}
impl ConcatStep {
    pub(crate) fn request_species(source: ObjectRef, mut resume: ConcatResume) -> Self {
        resume.0.pending_effect.species_source = Some(source);
        Self::Species { resume }
    }
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ConcatResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: ConcatResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_has(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ConcatResume,
    ) -> Self {
        resume.0.pending_effect.has_object = Some(object);
        resume.0.pending_effect.has_key = Some(key);
        Self::Has { resume }
    }
    pub(crate) fn request_define(
        object: ObjectRef,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
        mut resume: ConcatResume,
    ) -> Self {
        resume.0.pending_effect.define_object = Some(object);
        resume.0.pending_effect.define_key = Some(key);
        resume.0.pending_effect.define_descriptor = Some(descriptor);
        Self::Define { resume }
    }
    pub(crate) fn request_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: ConcatResume,
    ) -> Self {
        resume.0.pending_effect.set_object = Some(object);
        resume.0.pending_effect.set_key = Some(key);
        resume.0.pending_effect.set_value = Some(value);
        Self::Set { resume }
    }
}
impl ConcatResume {
    pub(crate) fn take_species_source(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .species_source
            .take()
            .expect("ConcatStep Species source")
    }
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ConcatStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ConcatStep Read key")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("ConcatStep Number value")
    }
    pub(crate) fn take_has_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .has_object
            .take()
            .expect("ConcatStep Has object")
    }
    pub(crate) fn take_has_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .has_key
            .take()
            .expect("ConcatStep Has key")
    }
    pub(crate) fn take_define_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .define_object
            .take()
            .expect("ConcatStep Define object")
    }
    pub(crate) fn take_define_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .define_key
            .take()
            .expect("ConcatStep Define key")
    }
    pub(crate) fn take_define_descriptor(&mut self) -> OrdinaryPropertyDescriptor {
        self.0
            .pending_effect
            .define_descriptor
            .take()
            .expect("ConcatStep Define descriptor")
    }
    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .set_object
            .take()
            .expect("ConcatStep Set object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .set_key
            .take()
            .expect("ConcatStep Set key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .pending_effect
            .set_value
            .take()
            .expect("ConcatStep Set value")
    }
}
const _: () = assert!(std::mem::size_of::<ConcatStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ConcatStep>() <= 64);
