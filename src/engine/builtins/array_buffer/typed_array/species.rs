//! TypedArray species construction shared by copying prototype methods.
//!
//! QuickJS uses `JS_SpeciesConstructor` with an undefined default sentinel,
//! then either constructs the authored species or allocates the source element
//! class directly in the builtin's defining realm.

use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::TypedArrayElementKind,
    heap::ContextId,
    object::{ObjectRef, PropertyKey, WellKnownSymbol},
    value::{Value, conversion::NativeConversion},
    vm::{Completion, call::ConstructorRef},
};

impl Runtime {
    pub(crate) fn typed_array_species_create(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        source_element: TypedArrayElementKind,
        length: u64,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        finish_species(
            self,
            realm,
            TypedSpeciesStep::start(self, realm, source.clone(), source_element, length)?,
        )
    }
    pub(crate) fn typed_array_species_create_subarray(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        source_element: TypedArrayElementKind,
        buffer: &ObjectRef,
        byte_offset: u64,
        length: Option<u64>,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        finish_species(
            self,
            realm,
            TypedSpeciesStep::start_view(
                self,
                realm,
                source.clone(),
                source_element,
                buffer.clone(),
                byte_offset,
                length,
            )?,
        )
    }

    pub(crate) fn typed_array_create_from_constructor(
        &self,
        realm: ContextId,
        constructor: Value,
        length: u64,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        self.typed_array_create_from_constructor_arguments(
            realm,
            constructor,
            &[Value::number(length as f64)],
            Some(length),
        )
    }

    /// Construct the result of the static `from` / `of` builtins.
    ///
    /// QuickJS reaches these call sites through `JS_CallConstructor`: a
    /// primitive receiver fails its callable check as "not a function".
    /// Species construction deliberately bypasses this seam because a
    /// primitive `@@species` value instead reports "not a constructor".
    pub(crate) fn typed_array_create_from_static_constructor(
        &self,
        realm: ContextId,
        constructor: Value,
        length: u64,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        if !matches!(&constructor, Value::Object(_)) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        }
        self.typed_array_create_from_constructor(realm, constructor, length)
    }

    /// Construct and validate a species result with the exact authored
    /// argument vector. QuickJS only enforces a minimum result length for its
    /// one-argument create form; `subarray` deliberately uses two or three
    /// arguments and therefore accepts any live TypedArray result.
    fn typed_array_create_from_constructor_arguments(
        &self,
        realm: ContextId,
        constructor: Value,
        arguments: &[Value],
        minimum_length: Option<u64>,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let mut owned = Vec::new();
        if owned.try_reserve_exact(arguments.len()).is_err() {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "out of memory",
            )?));
        }
        owned.extend_from_slice(arguments);
        finish_species(
            self,
            realm,
            TypedSpeciesStep::create(self, realm, constructor, owned, minimum_length)?,
        )
    }
    fn validate_typed_array_construction(
        &self,
        realm: ContextId,
        result: Completion,
        minimum_length: Option<u64>,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let target = match result {
            Completion::Return(Value::Object(value)) => value,
            Completion::Return(_) => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a TypedArray",
                )?));
            }
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(_) = self.typed_array_snapshot_if_branded(&target)? else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a TypedArray",
            )?));
        };
        let target_length = match self.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if minimum_length.is_some_and(|length| u64::from(target_length) < length) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "TypedArray length is too small",
            )?));
        }
        Ok(NativeConversion::Value(target))
    }

    /// Finish the ArrayBuffer overload after every observable prototype and
    /// numeric conversion. `subarray` reuses this exact non-observable tail
    /// for its default-species path, bypassing mutable public constructors.
    pub(crate) fn new_typed_array_view_from_coerced(
        &self,
        realm: ContextId,
        prototype: &ObjectRef,
        element: TypedArrayElementKind,
        buffer: &ObjectRef,
        byte_offset: u64,
        requested_length: Option<u64>,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let width = u64::from(element.byte_length());
        // Alignment precedes the detached-buffer check in pinned QuickJS.
        if byte_offset % width != 0 {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid offset",
            )?));
        }
        let backing = self.snapshot_buffer_access(buffer.object_id())?.state;
        if backing.detached {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        let (byte_offset, fixed_byte_length) = if let Some(length) = requested_length {
            let bytes = length.checked_mul(width).ok_or(RuntimeError::Invariant(
                "ToIndex TypedArray byte length overflowed u64",
            ))?;
            let end = byte_offset
                .checked_add(bytes)
                .ok_or(RuntimeError::Invariant(
                    "ToIndex TypedArray end offset overflowed u64",
                ))?;
            if end > u64::from(backing.byte_length) {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "invalid length",
                )?));
            }
            (
                u32::try_from(byte_offset)
                    .map_err(|_| RuntimeError::Invariant("validated byteOffset overflowed u32"))?,
                Some(u32::try_from(bytes).map_err(|_| {
                    RuntimeError::Invariant("validated TypedArray byte length overflowed u32")
                })?),
            )
        } else {
            if byte_offset > u64::from(backing.byte_length) {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "invalid offset",
                )?));
            }
            let byte_offset = u32::try_from(byte_offset)
                .map_err(|_| RuntimeError::Invariant("validated byteOffset overflowed u32"))?;
            let available = backing.byte_length - byte_offset;
            let fixed_byte_length = if backing.max_byte_length.is_some() {
                None
            } else {
                if u64::from(available) % width != 0 {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Range,
                        "invalid length",
                    )?));
                }
                Some(available)
            };
            (byte_offset, fixed_byte_length)
        };
        let target = self.new_typed_array_object(
            prototype,
            buffer,
            byte_offset,
            fixed_byte_length,
            element,
        )?;
        Ok(NativeConversion::Value(target))
    }
}

pub(crate) enum TypedSpeciesStep {
    Complete(NativeConversion<ObjectRef>),
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: TypedSpeciesResume,
    },
    Construct {
        constructor: ConstructorRef,
        arguments: Vec<Value>,
        resume: TypedSpeciesResume,
    },
}
pub(crate) struct TypedSpeciesResume(Box<TypedSpeciesResumeState>);
impl std::ops::Deref for TypedSpeciesResume {
    type Target = TypedSpeciesResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TypedSpeciesResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<TypedSpeciesResume>() <= 8);
pub(crate) struct TypedSpeciesResumeState {
    realm: ContextId,
    phase: SpeciesPhase,
}
struct SpeciesInput {
    source: ObjectRef,
    element: TypedArrayElementKind,
    mode: SpeciesMode,
}
enum SpeciesMode {
    Length(u64),
    View {
        buffer: ObjectRef,
        byte_offset: u64,
        length: Option<u64>,
    },
}
enum SpeciesPhase {
    Constructor(SpeciesInput),
    Species(SpeciesInput),
    Constructed(Option<u64>),
}
impl TypedSpeciesStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        source: ObjectRef,
        element: TypedArrayElementKind,
        length: u64,
    ) -> Result<Self, RuntimeError> {
        Self::lookup(
            runtime,
            realm,
            SpeciesInput {
                source,
                element,
                mode: SpeciesMode::Length(length),
            },
        )
    }
    pub(crate) fn start_view(
        runtime: &Runtime,
        realm: ContextId,
        source: ObjectRef,
        element: TypedArrayElementKind,
        buffer: ObjectRef,
        byte_offset: u64,
        length: Option<u64>,
    ) -> Result<Self, RuntimeError> {
        Self::lookup(
            runtime,
            realm,
            SpeciesInput {
                source,
                element,
                mode: SpeciesMode::View {
                    buffer,
                    byte_offset,
                    length,
                },
            },
        )
    }
    fn lookup(
        runtime: &Runtime,
        realm: ContextId,
        input: SpeciesInput,
    ) -> Result<Self, RuntimeError> {
        Ok(Self::Read {
            object: input.source.clone(),
            key: runtime.intern_property_key("constructor")?,
            resume: TypedSpeciesResume(Box::new(TypedSpeciesResumeState {
                realm,
                phase: SpeciesPhase::Constructor(input),
            })),
        })
    }
    pub(crate) fn create(
        runtime: &Runtime,
        realm: ContextId,
        constructor: Value,
        arguments: Vec<Value>,
        minimum_length: Option<u64>,
    ) -> Result<Self, RuntimeError> {
        let Value::Object(object) = constructor else {
            return Ok(Self::Complete(NativeConversion::Throw(
                runtime.new_native_error(realm, NativeErrorKind::Type, "not a constructor")?,
            )));
        };
        if !runtime.is_constructor(&object)? {
            return Ok(Self::Complete(NativeConversion::Throw(
                runtime.new_not_constructor_error(realm, &Value::Object(object))?,
            )));
        }
        let constructor = match runtime.constructor_from_value(realm, Value::Object(object))? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(Self::Complete(NativeConversion::Throw(value)));
            }
        };
        Ok(Self::Construct {
            constructor,
            arguments,
            resume: TypedSpeciesResume(Box::new(TypedSpeciesResumeState {
                realm,
                phase: SpeciesPhase::Constructed(minimum_length),
            })),
        })
    }
}
impl TypedSpeciesResume {
    fn selected(
        runtime: &Runtime,
        realm: ContextId,
        input: SpeciesInput,
        species: Value,
    ) -> Result<TypedSpeciesStep, RuntimeError> {
        if matches!(species, Value::Undefined | Value::Null) {
            let prototype = runtime.typed_array_default_prototype(realm, input.element)?;
            return Ok(TypedSpeciesStep::Complete(match input.mode {
                SpeciesMode::Length(length) => {
                    runtime.new_typed_array_for_length(realm, &prototype, input.element, length)?
                }
                SpeciesMode::View {
                    buffer,
                    byte_offset,
                    length,
                } => runtime.new_typed_array_view_from_coerced(
                    realm,
                    &prototype,
                    input.element,
                    &buffer,
                    byte_offset,
                    length,
                )?,
            }));
        }
        let mut arguments = Vec::new();
        let count = match &input.mode {
            SpeciesMode::Length(_) => 1,
            SpeciesMode::View {
                length: Some(_), ..
            } => 3,
            SpeciesMode::View { .. } => 2,
        };
        if arguments.try_reserve_exact(count).is_err() {
            return Ok(TypedSpeciesStep::Complete(NativeConversion::Throw(
                runtime.new_native_error(realm, NativeErrorKind::Internal, "out of memory")?,
            )));
        }
        let minimum = match input.mode {
            SpeciesMode::Length(length) => {
                arguments.push(Value::number(length as f64));
                Some(length)
            }
            SpeciesMode::View {
                buffer,
                byte_offset,
                length,
            } => {
                arguments.push(Value::Object(buffer));
                arguments.push(Value::number(byte_offset as f64));
                if let Some(length) = length {
                    arguments.push(Value::number(length as f64));
                }
                None
            }
        };
        TypedSpeciesStep::create(runtime, realm, species, arguments, minimum)
    }
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<TypedSpeciesStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(TypedSpeciesStep::Complete(NativeConversion::Throw(value)));
            }
        };
        match self.0.phase {
            SpeciesPhase::Constructor(input) => {
                if matches!(value, Value::Undefined) {
                    return Self::selected(runtime, self.0.realm, input, Value::Undefined);
                }
                let Value::Object(object) = value else {
                    return Ok(TypedSpeciesStep::Complete(NativeConversion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "not an object",
                        )?,
                    )));
                };
                Ok(TypedSpeciesStep::Read {
                    object,
                    key: PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Species)),
                    resume: Self(Box::new(TypedSpeciesResumeState {
                        realm: self.0.realm,
                        phase: SpeciesPhase::Species(input),
                    })),
                })
            }
            SpeciesPhase::Species(input) => Self::selected(runtime, self.0.realm, input, value),
            SpeciesPhase::Constructed(minimum) => Ok(TypedSpeciesStep::Complete(
                runtime.validate_typed_array_construction(
                    self.0.realm,
                    Completion::Return(value),
                    minimum,
                )?,
            )),
        }
    }
}
fn finish_species(
    runtime: &Runtime,
    realm: ContextId,
    mut step: TypedSpeciesStep,
) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
    loop {
        step = match step {
            TypedSpeciesStep::Complete(result) => return Ok(result),
            TypedSpeciesStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
            TypedSpeciesStep::Construct {
                constructor,
                arguments,
                resume,
            } => resume.resume(
                runtime,
                runtime.construct_constructor_internal(
                    realm,
                    &constructor,
                    &constructor,
                    &arguments,
                )?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TypedSpeciesStep>() <= 64);
