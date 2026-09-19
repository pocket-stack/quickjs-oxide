//! Array endpoint mutations retain their copy cursor across observable property operations.

use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::{ArrayPopKind, ArrayPushKind},
    heap::ContextId,
    object::{ObjectRef, PropertyKey, operations::InternalSetResult},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum MutationKind {
    Push(ArrayPushKind),
    Pop(ArrayPopKind),
}
impl MutationKind {
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        match target {
            NativeFunctionId::ArrayPrototypePush(kind) => Some(Self::Push(kind)),
            NativeFunctionId::ArrayPrototypePop(kind) => Some(Self::Pop(kind)),
            _ => None,
        }
    }
}
pub(crate) enum MutationStep {
    Complete(Completion),

    PreparedRead { resume: MutationResume },

    PreparedSet { resume: MutationResume },
    Read { resume: MutationResume },
    Number { resume: MutationResume },
    Copy { resume: MutationResume },
    Set { resume: MutationResume },
    Delete { resume: MutationResume },
}
enum Phase {
    Length,
    Number,
    Result,
    Copy,
    Write,
    DeleteLast,
    LengthWrite,
}
pub(crate) struct MutationResume(Box<MutationResumeState>);
impl std::ops::Deref for MutationResume {
    type Target = MutationResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for MutationResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<MutationResume>() <= 8);
pub(crate) struct MutationResumeState {
    pending_effect: MutationStepPending,
    scheduler_set_key: Option<PropertyKey>,
    realm: ContextId,
    kind: MutationKind,
    object: ObjectRef,
    arguments: Vec<Value>,
    // Push never uses the Pop result slot. A single immediate argument lives
    // there until completion, avoiding a Vec allocation without moving roots.
    inline_argument: bool,
    phase: Phase,
    length: u64,
    new_length: u64,
    cursor: u64,
    result: Value,
}
// This enum carries only the selected effect. The source, arguments and result
// remain in one MutationResume until a real callback requires owned transport.
enum MutationAction {
    Complete(Completion),
    Read(PropertyKey),
    Number(Value),
    Copy {
        to: u64,
        from: u64,
        count: u64,
        backwards: bool,
    },
    Set {
        key: PropertyKey,
        value: Value,
    },
    Delete(PropertyKey),
}

impl MutationStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: MutationKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array mutation requires generic invocation",
            ));
        };
        let values = &arguments.readable[..arguments.actual_arg_count];
        let inline = match (kind, values) {
            (
                MutationKind::Push(_),
                [
                    value @ (Value::Undefined
                    | Value::Null
                    | Value::Bool(_)
                    | Value::Int(_)
                    | Value::Float(_)),
                ],
            ) => Some(value.clone()),
            _ => None,
        };
        let values = if inline.is_some() {
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "array_mutation_inline_argument",
            );
            Vec::new()
        } else {
            values.to_vec()
        };
        Self::start_arguments(runtime, realm, kind, this_value.clone(), values, inline)
    }
    pub(crate) fn start_values(
        runtime: &Runtime,
        realm: ContextId,
        kind: MutationKind,
        receiver: Value,
        arguments: Vec<Value>,
    ) -> Result<Self, RuntimeError> {
        Self::start_arguments(runtime, realm, kind, receiver, arguments, None)
    }
    fn start_arguments(
        runtime: &Runtime,
        realm: ContextId,
        kind: MutationKind,
        receiver: Value,
        arguments: Vec<Value>,
        inline: Option<Value>,
    ) -> Result<Self, RuntimeError> {
        let object = match runtime.native_to_object(realm, receiver)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };

        {
            let completed = match kind {
                MutationKind::Push(ArrayPushKind::Push) => {
                    let value = inline
                        .as_ref()
                        .or_else(|| (arguments.len() == 1).then(|| &arguments[0]));
                    match value {
                        Some(value) => runtime.try_dense_push(&object, value)?,
                        None => None,
                    }
                }
                MutationKind::Pop(ArrayPopKind::Pop) => runtime.try_dense_pop(&object)?,
                _ => None,
            };
            if let Some(value) = completed {
                return Ok(Self::Complete(Completion::Return(value)));
            }
        }
        let action = MutationAction::Read(
            runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)?,
        );
        MutationResume(Box::new(MutationResumeState {
            pending_effect: MutationStepPending::default(),
            scheduler_set_key: None,
            realm,
            kind,
            object,
            arguments,
            inline_argument: inline.is_some(),
            phase: Phase::Length,
            length: 0,
            new_length: 0,
            cursor: 0,
            result: inline.unwrap_or(Value::Undefined),
        }))
        .drive(runtime, action)
    }
}
impl MutationResume {
    pub(crate) fn with_scheduler_set_key(mut self, key: PropertyKey) -> Self {
        self.0.scheduler_set_key = Some(key);
        self
    }
    pub(crate) fn take_scheduler_set_key(&mut self) -> PropertyKey {
        self.0.scheduler_set_key.take().expect("waiting Set key")
    }

    fn argument_count(&self) -> usize {
        if self.0.inline_argument {
            1
        } else {
            self.0.arguments.len()
        }
    }
    fn argument(&self, index: usize) -> Option<&Value> {
        if self.0.inline_argument {
            (index == 0).then_some(&self.0.result)
        } else {
            self.0.arguments.get(index)
        }
    }
    fn resume_once(
        &mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<MutationAction, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(MutationAction::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Length => {
                self.0.phase = Phase::Number;
                Ok(MutationAction::Number(value))
            }
            Phase::Result => {
                self.0.result = value;
                self.copy_next(runtime)
            }
            Phase::Copy => self.copied(runtime),
            _ => Err(RuntimeError::Invariant(
                "Array mutation received unexpected value reply",
            )),
        }
    }
    fn number_once(
        &mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<MutationAction, RuntimeError> {
        if !matches!(self.0.phase, Phase::Number) {
            return Err(RuntimeError::Invariant(
                "Array mutation number phase mismatch",
            ));
        }
        self.0.length = match result {
            NativeConversion::Value(number) => Runtime::length_from_number(number),
            NativeConversion::Throw(value) => {
                return Ok(MutationAction::Complete(Completion::Throw(value)));
            }
        };
        match self.0.kind {
            MutationKind::Push(_) => {
                self.0.new_length = self.0.length.saturating_add(self.argument_count() as u64);
                if self.0.new_length > (1_u64 << 53) - 1 {
                    return Ok(MutationAction::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "Array loo long",
                        )?,
                    )));
                }
                self.copy_next(runtime)
            }
            MutationKind::Pop(kind) => {
                self.0.new_length = self.0.length.saturating_sub(1);
                if self.0.length == 0 {
                    return self.write_length(runtime);
                }
                let index = if kind == ArrayPopKind::Shift {
                    0
                } else {
                    self.0.new_length
                };
                self.0.phase = Phase::Result;
                Ok(MutationAction::Read(runtime.property_key_for_index(index)?))
            }
        }
    }
    fn copy_next(&mut self, runtime: &Runtime) -> Result<MutationAction, RuntimeError> {
        let (to, from, count, backwards) = match self.0.kind {
            MutationKind::Push(ArrayPushKind::Unshift) if self.argument_count() != 0 => {
                (self.argument_count() as u64, 0, self.0.length, true)
            }
            MutationKind::Pop(ArrayPopKind::Shift) => (0, 1, self.0.new_length, false),
            _ => return self.copied(runtime),
        };
        self.0.phase = Phase::Copy;
        Ok(MutationAction::Copy {
            to,
            from,
            count,
            backwards,
        })
    }
    fn copied(&mut self, runtime: &Runtime) -> Result<MutationAction, RuntimeError> {
        self.0.cursor = 0;
        match self.0.kind {
            MutationKind::Push(_) => self.write_next(runtime),
            MutationKind::Pop(_) => {
                self.0.phase = Phase::DeleteLast;
                Ok(MutationAction::Delete(
                    runtime.property_key_for_index(self.0.new_length)?,
                ))
            }
        }
    }
    fn write_next(&mut self, runtime: &Runtime) -> Result<MutationAction, RuntimeError> {
        if let Some(value) = self.argument(self.0.cursor as usize).cloned() {
            let from = match self.0.kind {
                MutationKind::Push(ArrayPushKind::Unshift) if self.argument_count() != 0 => 0,
                _ => self.0.length,
            };
            self.0.phase = Phase::Write;
            return Ok(MutationAction::Set {
                key: runtime.property_key_for_index(from + self.0.cursor)?,
                value,
            });
        }
        let redundant = matches!(self.0.kind, MutationKind::Push(ArrayPushKind::Push))
            && self.0.new_length <= u64::from(u32::MAX)
            && matches!(runtime.array_length_state_if_genuine(&self.0.object)?, Some((length, true)) if u64::from(length) == self.0.new_length);
        if redundant {
            self.complete()
        } else {
            self.write_length(runtime)
        }
    }
    fn write_length(&mut self, runtime: &Runtime) -> Result<MutationAction, RuntimeError> {
        self.0.phase = Phase::LengthWrite;
        Ok(MutationAction::Set {
            key: runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)?,
            value: Value::number(self.0.new_length as f64),
        })
    }
    fn complete(&mut self) -> Result<MutationAction, RuntimeError> {
        Ok(MutationAction::Complete(Completion::Return(
            match self.0.kind {
                MutationKind::Push(_) => Value::number(self.0.new_length as f64),
                MutationKind::Pop(_) => std::mem::replace(&mut self.0.result, Value::Undefined),
            },
        )))
    }
    fn boolean_once(
        &mut self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<MutationAction, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(MutationAction::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::DeleteLast => {
                if !value {
                    return Ok(MutationAction::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "could not delete property",
                        )?,
                    )));
                }
                self.write_length(runtime)
            }
            _ => Err(RuntimeError::Invariant(
                "Array mutation boolean phase mismatch",
            )),
        }
    }
    fn set_once(
        &mut self,
        runtime: &Runtime,
        key: PropertyKey,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<MutationAction, RuntimeError> {
        if let Some(value) = runtime.finish_set_property_or_throw(self.0.realm, &key, result)? {
            return Ok(MutationAction::Complete(Completion::Throw(value)));
        }
        match self.0.phase {
            Phase::Write => {
                self.0.cursor += 1;
                self.write_next(runtime)
            }
            Phase::LengthWrite => self.complete(),
            _ => Err(RuntimeError::Invariant("Array mutation set phase mismatch")),
        }
    }
}
impl MutationResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<MutationStep, RuntimeError> {
        let action = self.resume_once(runtime, reply)?;
        self.drive(runtime, action)
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<f64>,
    ) -> Result<MutationStep, RuntimeError> {
        let action = self.number_once(runtime, reply)?;
        self.drive(runtime, action)
    }
    pub(crate) fn boolean(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<bool>,
    ) -> Result<MutationStep, RuntimeError> {
        let action = self.boolean_once(runtime, reply)?;
        self.drive(runtime, action)
    }
    pub(crate) fn set(
        mut self,
        runtime: &Runtime,
        key: PropertyKey,
        reply: NativeConversion<InternalSetResult>,
    ) -> Result<MutationStep, RuntimeError> {
        let action = self.set_once(runtime, key, reply)?;
        self.drive(runtime, action)
    }
    fn drive(
        mut self,
        runtime: &Runtime,
        mut action: MutationAction,
    ) -> Result<MutationStep, RuntimeError> {
        loop {
            {
                use crate::engine::object::{OrdinaryRead, SetStep};
                use crate::engine::value::conversion::number::NumberStep;
                action = match action {
                    MutationAction::Read(key) => {
                        let receiver = Value::Object(self.0.object.clone());
                        match runtime.prepare_ordinary_read_borrowed(
                            &self.0.object,
                            &key,
                            &receiver,
                        )? {
                            OrdinaryRead::Complete(value) => self.resume_once(
                                runtime,
                                Completion::Return(value.unwrap_or(Value::Undefined)),
                            )?,
                            read => {
                                return Ok(MutationStep::request_prepared_read(read, key, self));
                            }
                        }
                    }
                    MutationAction::Number(value) if !matches!(value, Value::Object(_)) => {
                        let NumberStep::Complete(reply) =
                            NumberStep::start(runtime, self.0.realm, value)?
                        else {
                            return Err(RuntimeError::Invariant(
                                "primitive mutation number suspended",
                            ));
                        };
                        self.number_once(runtime, reply)?
                    }
                    MutationAction::Set { key, value } => {
                        let mut pending = None;
                        let selected = SetStep::start_receiver_into(
                            runtime,
                            self.0.realm,
                            &key,
                            value,
                            Value::Object(self.0.object.clone()),
                            |step| pending = Some(step),
                        )?;
                        let step = match selected {
                            Some(action) => SetStep::Complete(action),
                            None => pending
                                .ok_or(RuntimeError::Invariant(
                                    "mutation Set lost selected effect",
                                ))?
                                .advance_without_callback(runtime)?,
                        };
                        match step {
                            SetStep::Complete(action)
                                if !matches!(
                                    action,
                                    crate::engine::object::operations::PropertySetAction::Call { .. }
                                ) =>
                            {
                                self.set_once(runtime, key, local_set_result(action)?)?
                            }
                            step => {
                                return Ok(MutationStep::request_prepared_set(
                                    Box::new(step),
                                    key,
                                    self,
                                ));
                            }
                        }
                    }
                    MutationAction::Delete(key) if !runtime.is_proxy_object(&self.0.object)? => {
                        let reply =
                            runtime.internal_delete_property(self.0.realm, &self.0.object, &key)?;
                        self.boolean_once(runtime, reply)?
                    }
                    action => return Ok(self.wait(action)),
                };
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_execution_event(
                    "array_mutation_local_stage",
                );
            }
        }
    }
    fn wait(self, action: MutationAction) -> MutationStep {
        match action {
            MutationAction::Complete(result) => MutationStep::Complete(result),
            MutationAction::Read(key) => {
                MutationStep::request_read(self.0.object.clone(), key, self)
            }
            MutationAction::Number(value) => MutationStep::request_number(value, self),
            MutationAction::Copy {
                to,
                from,
                count,
                backwards,
            } => {
                MutationStep::request_copy(self.0.object.clone(), to, from, count, backwards, self)
            }
            MutationAction::Set { key, value } => {
                MutationStep::request_set(self.0.object.clone(), key, value, self)
            }
            MutationAction::Delete(key) => {
                MutationStep::request_delete(self.0.object.clone(), key, self)
            }
        }
    }
}

fn local_set_result(
    action: crate::engine::object::operations::PropertySetAction,
) -> Result<NativeConversion<InternalSetResult>, RuntimeError> {
    use crate::engine::object::operations::PropertySetAction;
    Ok(match action {
        PropertySetAction::Complete => NativeConversion::Value(InternalSetResult::Accepted),
        PropertySetAction::Rejected(reason) => {
            NativeConversion::Value(InternalSetResult::Rejected(reason))
        }
        PropertySetAction::RejectedProxyTrap => {
            NativeConversion::Value(InternalSetResult::RejectedProxyTrap)
        }
        PropertySetAction::Throw(value) => NativeConversion::Throw(value),
        PropertySetAction::Call { .. } => {
            return Err(RuntimeError::Invariant(
                "mutation completed before setter returned",
            ));
        }
    })
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: MutationStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            MutationStep::Complete(result) => return Ok(result),

            MutationStep::PreparedRead { mut resume } => {
                let read = resume.take_prepared_read_read();
                let key = resume.take_prepared_read_key();
                {
                    let reply = match runtime.finish_prepared_read(realm, &key, read)? {
                        NativeConversion::Value(value) => {
                            Completion::Return(value.unwrap_or(Value::Undefined))
                        }
                        NativeConversion::Throw(value) => Completion::Throw(value),
                    };
                    resume.resume(runtime, reply)?
                }
            }

            MutationStep::PreparedSet { mut resume } => {
                let step = resume.take_prepared_set_step();
                let key = resume.take_prepared_set_key();
                {
                    use crate::engine::object::{SetStep, operations::PropertySetAction};
                    let mut step = *step;
                    let result = loop {
                        match step {
                            SetStep::Complete(PropertySetAction::Call { payload }) => {
                                let crate::engine::object::operations::PropertySetterCall {
                                    setter,
                                    receiver,
                                    argument,
                                } = *payload;

                                break match runtime.call_internal(
                                    realm,
                                    &setter,
                                    receiver,
                                    &[argument],
                                )? {
                                    Completion::Return(_) => {
                                        NativeConversion::Value(InternalSetResult::Accepted)
                                    }
                                    Completion::Throw(value) => NativeConversion::Throw(value),
                                };
                            }
                            SetStep::Complete(action) => break local_set_result(action)?,
                            request => step = request.finish_sync(runtime)?,
                        }
                    };
                    resume.set(runtime, key, result)?
                }
            }
            MutationStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            MutationStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            MutationStep::Copy { mut resume } => {
                let object = resume.take_copy_object();
                let to = resume.take_copy_to();
                let from = resume.take_copy_from();
                let count = resume.take_copy_count();
                let backwards = resume.take_copy_backwards();
                resume.resume(
                    runtime,
                    super::copy::finish(
                        runtime,
                        realm,
                        super::copy::CopyStep::start(
                            runtime, realm, object, to, from, count, backwards,
                        )?,
                    )?,
                )?
            }
            MutationStep::Set { mut resume } => {
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
            MutationStep::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                resume.boolean(
                    runtime,
                    runtime.internal_delete_property(realm, &object, &key)?,
                )?
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_mutation_keeps_selected_setter_and_proxy_once() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let trace='', stored;
            const proto={set 0(value){trace+='s';stored=value;}};
            const target=Object.create(proto);target.length=0;
            Object.defineProperty(target,'length',{get(){trace+='g';return 0;},set(value){trace+='l'+value;},configurable:true});
            if(Array.prototype.push.call(target,7)!==1 || stored!==7 || trace!=='gsl1')return false;
            trace='';const data={length:0};
            const proxy=new Proxy(data,{get(o,k,r){if(k==='length')trace+='g';return Reflect.get(o,k,r);},set(o,k,v,r){trace+='s'+k;return Reflect.set(o,k,v,r);}});
            if(Array.prototype.push.call(proxy,8)!==1 || trace!=='gs0slength' || data[0]!==8)return false;
            trace=''; const pop=Object.create({get 1(){trace+='r';return 9;}});
            Object.defineProperty(pop,'length',{get(){trace+='g';return 2;},set(v){trace+='l'+v;}});
            return Array.prototype.pop.call(pop)===9 && trace==='grl1';
        })()"#).unwrap(), Value::Bool(true));
    }

    #[test]
    fn local_mutation_keeps_partial_effects_on_rejected_length_or_delete() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            const a=[1,2];Object.defineProperty(a,'1',{configurable:false});
            let rejected=false;try{a.pop();}catch(e){rejected=e instanceof TypeError;}
            if(!rejected || a.length!==2 || a[1]!==2)return false;
            const target={length:0};Object.defineProperty(target,'length',{writable:false});
            rejected=false;try{Array.prototype.push.call(target,3);}catch(e){rejected=e instanceof TypeError;}
            if(!rejected || target[0]!==3 || target.length!==0)return false;
            const b=[1];Object.defineProperty(b,'length',{writable:false});
            rejected=false;try{b.push(4);}catch(e){rejected=e instanceof TypeError;}
            return rejected && b.length===1 && !(1 in b);
        })()"#).unwrap(), Value::Bool(true));
    }

    #[test]
    fn push_inline_argument_uses_actual_count_and_retains_immediate_payload() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        #[cfg(feature = "profiling")]
        let profile = crate::engine::api::profiling::CostProfile::start();
        for value in [
            Value::Undefined,
            Value::Null,
            Value::Bool(true),
            Value::Int(42),
            Value::Float(-0.0),
        ] {
            let array = runtime.new_array(context.realm).unwrap();
            let invocation = NativeInvocation::Call {
                this_value: Value::Object(array.clone()),
            };
            let arguments = NativeArguments {
                actual_arg_count: 1,
                readable: vec![value.clone(), Value::Undefined],
            };
            let step = MutationStep::start(
                &runtime,
                context.realm,
                MutationKind::Push(ArrayPushKind::Push),
                &invocation,
                &arguments,
            )
            .unwrap();
            let result = finish(&runtime, context.realm, step).unwrap();
            assert!(matches!(result, Completion::Return(Value::Int(1))));
            let key = runtime.property_key_for_index(0).unwrap();
            let Completion::Return(actual) = runtime
                .get_property_in_realm(context.realm, &array, &key)
                .unwrap()
            else {
                panic!("element read threw")
            };
            assert!(actual.same_quickjs_representation(&value));
        }
        let array = runtime.new_array(context.realm).unwrap();
        let invocation = NativeInvocation::Call {
            this_value: Value::Object(array),
        };
        let arguments = NativeArguments {
            actual_arg_count: 0,
            readable: vec![Value::Undefined],
        };
        let step = MutationStep::start(
            &runtime,
            context.realm,
            MutationKind::Push(ArrayPushKind::Push),
            &invocation,
            &arguments,
        )
        .unwrap();
        assert!(matches!(
            finish(&runtime, context.realm, step).unwrap(),
            Completion::Return(Value::Int(0))
        ));
        #[cfg(feature = "profiling")]
        assert_eq!(
            profile
                .snapshot()
                .owned_execution_events
                .get("array_mutation_inline_argument")
                .copied()
                .unwrap_or(0),
            5
        );
    }

    #[test]
    fn push_inline_argument_preserves_observable_mutation_steps() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let value = context.eval(r#"(function () {
            var trace = '', stored;
            var target = {
                get length() { trace += 'g'; return { valueOf: function () { trace += 'n'; return 0; } }; },
                set length(v) { trace += 'l' + v; },
                set 0(v) { trace += 's'; stored = v; }
            };
            if (Array.prototype.push.call(target, -0) !== 1 ||
                trace !== 'gnsl1' || 1 / stored !== -Infinity) return false;
            var a = [2, 3];
            if (a.unshift(1) !== 3 || a.join(',') !== '1,2,3') return false;
            var object = {}, b = [];
            if (b.push(object) !== 1 || b[0] !== object) return false;
            if (b.push(4, 5) !== 3 || b[1] !== 4 || b[2] !== 5) return false;
            var frozen = Object.freeze([]), threw = false;
            try { frozen.push(1); } catch (e) { threw = e instanceof TypeError; }
            return threw && frozen.length === 0;
        })()"#).unwrap();
        assert!(matches!(value, Value::Bool(true)));
    }
}

#[derive(Default)]
struct MutationStepPending {
    prepared_read_read: Option<crate::engine::object::OrdinaryRead>,
    prepared_read_key: Option<PropertyKey>,
    prepared_set_step: Option<Box<crate::engine::object::SetStep>>,
    prepared_set_key: Option<PropertyKey>,
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    number_value: Option<Value>,
    copy_object: Option<ObjectRef>,
    copy_to: Option<u64>,
    copy_from: Option<u64>,
    copy_count: Option<u64>,
    copy_backwards: Option<bool>,
    set_object: Option<ObjectRef>,
    set_key: Option<PropertyKey>,
    set_value: Option<Value>,
    delete_object: Option<ObjectRef>,
    delete_key: Option<PropertyKey>,
}
impl MutationStep {
    pub(crate) fn request_prepared_read(
        read: crate::engine::object::OrdinaryRead,
        key: PropertyKey,
        mut resume: MutationResume,
    ) -> Self {
        resume.0.pending_effect.prepared_read_read = Some(read);
        resume.0.pending_effect.prepared_read_key = Some(key);
        Self::PreparedRead { resume }
    }
    pub(crate) fn request_prepared_set(
        step: Box<crate::engine::object::SetStep>,
        key: PropertyKey,
        mut resume: MutationResume,
    ) -> Self {
        resume.0.pending_effect.prepared_set_step = Some(step);
        resume.0.pending_effect.prepared_set_key = Some(key);
        Self::PreparedSet { resume }
    }
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: MutationResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: MutationResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_copy(
        object: ObjectRef,
        to: u64,
        from: u64,
        count: u64,
        backwards: bool,
        mut resume: MutationResume,
    ) -> Self {
        resume.0.pending_effect.copy_object = Some(object);
        resume.0.pending_effect.copy_to = Some(to);
        resume.0.pending_effect.copy_from = Some(from);
        resume.0.pending_effect.copy_count = Some(count);
        resume.0.pending_effect.copy_backwards = Some(backwards);
        Self::Copy { resume }
    }
    pub(crate) fn request_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: MutationResume,
    ) -> Self {
        resume.0.pending_effect.set_object = Some(object);
        resume.0.pending_effect.set_key = Some(key);
        resume.0.pending_effect.set_value = Some(value);
        Self::Set { resume }
    }
    pub(crate) fn request_delete(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: MutationResume,
    ) -> Self {
        resume.0.pending_effect.delete_object = Some(object);
        resume.0.pending_effect.delete_key = Some(key);
        Self::Delete { resume }
    }
}
impl MutationResume {
    pub(crate) fn take_prepared_read_read(&mut self) -> crate::engine::object::OrdinaryRead {
        self.0
            .pending_effect
            .prepared_read_read
            .take()
            .expect("MutationStep PreparedRead read")
    }
    pub(crate) fn take_prepared_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .prepared_read_key
            .take()
            .expect("MutationStep PreparedRead key")
    }
    pub(crate) fn take_prepared_set_step(&mut self) -> Box<crate::engine::object::SetStep> {
        self.0
            .pending_effect
            .prepared_set_step
            .take()
            .expect("MutationStep PreparedSet step")
    }
    pub(crate) fn take_prepared_set_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .prepared_set_key
            .take()
            .expect("MutationStep PreparedSet key")
    }
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("MutationStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("MutationStep Read key")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("MutationStep Number value")
    }
    pub(crate) fn take_copy_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .copy_object
            .take()
            .expect("MutationStep Copy object")
    }
    pub(crate) fn take_copy_to(&mut self) -> u64 {
        self.0
            .pending_effect
            .copy_to
            .take()
            .expect("MutationStep Copy to")
    }
    pub(crate) fn take_copy_from(&mut self) -> u64 {
        self.0
            .pending_effect
            .copy_from
            .take()
            .expect("MutationStep Copy from")
    }
    pub(crate) fn take_copy_count(&mut self) -> u64 {
        self.0
            .pending_effect
            .copy_count
            .take()
            .expect("MutationStep Copy count")
    }
    pub(crate) fn take_copy_backwards(&mut self) -> bool {
        self.0
            .pending_effect
            .copy_backwards
            .take()
            .expect("MutationStep Copy backwards")
    }
    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .set_object
            .take()
            .expect("MutationStep Set object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .set_key
            .take()
            .expect("MutationStep Set key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .pending_effect
            .set_value
            .take()
            .expect("MutationStep Set value")
    }
    pub(crate) fn take_delete_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .delete_object
            .take()
            .expect("MutationStep Delete object")
    }
    pub(crate) fn take_delete_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .delete_key
            .take()
            .expect("MutationStep Delete key")
    }
}
const _: () = assert!(std::mem::size_of::<MutationStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<MutationStep>() <= 64);
