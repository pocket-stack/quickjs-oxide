//! Schedule prepared property reads without replaying observable key conversion.
//! Storage selection remains in object; VM owns input transfer and child replies.
use super::{
    Completion,
    call::{BytecodeCallRequest, CallableExecution},
    call_bridge::Action,
    driver::{CallStep, push_frame},
    exception::runtime_error_to_vm_error,
    execution::RunningExecution,
    frame::{FrameId, ReturnTarget},
};
use crate::engine::{
    api::{Error, runtime::Runtime},
    code::function::metadata::FunctionKind,
    heap::ContextId,
    object::{OrdinaryRead, PropertyKey},
    value::{Value, conversion::NativeConversion},
};

#[derive(Clone, Copy)]
pub(super) enum ReadKey {
    Static(u32),
    Computed { keep_key: bool },
}

/// Only Completed proves that the instruction finished in this same frame.
/// Deferred preserves the existing callback/query protocol even if it happens
/// to return Entered after synchronous work.
pub(super) enum PropertyProgress {
    Completed,
    MethodCall(u16),
    Deferred(CallStep),
}

impl PropertyProgress {
    pub(super) fn into_call_step(self) -> CallStep {
        match self {
            Self::Completed | Self::MethodCall(_) => CallStep::Entered,
            Self::Deferred(step) => step,
        }
    }
}

/// Converted inputs stay owned after ToPrimitive's reply, even if lookup next
/// reaches a Proxy or a callable whose domain continuation is still pending.
pub(super) struct ConvertedRead {
    pub base: Value,
    pub key: Value,
    pub keep_receiver: bool,
    pub keep_key: bool,
}

pub(super) fn throw_error(
    runtime: &Runtime,
    realm: ContextId,
    error: Error,
) -> Result<CallStep, Error> {
    let Some(kind) =
        crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
    else {
        return Err(error);
    };
    Ok(CallStep::Complete(Completion::Throw(
        runtime
            .new_native_error_from_error(realm, kind, &error)
            .map_err(runtime_error_to_vm_error)?,
    )))
}

pub(super) fn read(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    key_kind: ReadKey,
    keep_receiver: bool,
) -> Result<CallStep, Error> {
    read_progress(runtime, execution, id, key_kind, keep_receiver)
        .map(PropertyProgress::into_call_step)
}

pub(super) fn read_progress(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    key_kind: ReadKey,
    keep_receiver: bool,
) -> Result<PropertyProgress, Error> {
    read_progress_selected(runtime, execution, id, key_kind, keep_receiver, &mut None)
}

pub(super) fn read_progress_selected(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    key_kind: ReadKey,
    keep_receiver: bool,
    native: &mut Option<crate::engine::object::LinkedNativeSelection>,
) -> Result<PropertyProgress, Error> {
    let frame = execution.frames.current_mut(id)?;
    let computed = matches!(key_kind, ReadKey::Computed { .. });
    let realm = frame.executable.realm;
    let mut selected_read = None;
    if let ReadKey::Static(index) = key_kind {
        use super::stack::LinkedReadCompletion;
        let body = &mut *frame.cold;
        let executable = &*body.executable;
        let depth = execution.slots.depth(&body.window);
        let mut preserved_receiver = None;
        let mut retained_key = None;
        let mut method_call = None;
        let candidate = keep_receiver
            .then(|| executable.fusion.method_call(frame.fault_pc))
            .flatten();
        let result = execution.slots.with_linked_own_read_selected(
            &mut body.window,
            runtime,
            &executable,
            index,
            candidate.map(|_| &mut *native),
            |slots, value| {
                // Lookup has finished and retained the result. Move the base
                // owner into the enclosing driver scope before publication.
                preserved_receiver = Some(slots.pop()?);
                // Base has moved outside the slot window. The result and
                // receiver need two slots; decline fusion before any literal
                // pushes if the verified capacity cannot hold the whole span.
                let count = candidate.filter(|count| {
                    slots.has_operand_capacity(count + 2)
                        && super::method_arguments::available(
                            slots,
                            &executable.code[frame.fault_pc + 1..frame.fault_pc + count + 1],
                        )
                });
                publish_read_result(
                    slots,
                    &mut frame.resume_pc,
                    frame.fault_pc,
                    &mut preserved_receiver,
                    &mut retained_key,
                    keep_receiver,
                    value,
                )?;
                if let Some(count) = count {
                    // GetField2 completed even if a later fallible argument
                    // retain fails at its own canonical PC.
                    record_read_completion(depth);
                    let start = frame.fault_pc;
                    for offset in 0..count {
                        // Copy retains never drain references or call JS. On a
                        // failed retain, publish this canonical argument PC once
                        // the RunSlots borrow has ended below.
                        frame.fault_pc = start + offset + 1;
                        frame.resume_pc = frame.fault_pc;
                        let literal = super::method_arguments::argument(
                            slots,
                            &executable.code[frame.fault_pc],
                        )?;
                        slots.push(literal)?;
                        #[cfg(feature = "profiling")]
                        crate::engine::api::profiling::record_owned_instruction(depth + offset + 1);
                    }
                    frame.resume_pc = start + count + 1;
                    #[cfg(feature = "profiling")]
                    crate::engine::api::profiling::record_owned_execution_event("method_call_span");
                    method_call = Some(count as u16);
                }
                Ok(())
            },
        );
        match result {
            Ok(LinkedReadCompletion::Completed) => {
                if method_call.is_none() {
                    record_read_completion(depth);
                }
                #[cfg(feature = "profiling")]
                if preserved_receiver.is_some() {
                    // One actual driver-scope owner drop, not a claim that
                    // this was the runtime's final root or that GC ran.
                    crate::engine::api::profiling::record_owned_execution_event(
                        "linked_read_base_owner_drop",
                    );
                }
                return Ok(method_call
                    .map(PropertyProgress::MethodCall)
                    .unwrap_or(PropertyProgress::Completed));
            }
            Ok(LinkedReadCompletion::Pending(read)) => selected_read = Some(read),
            Ok(LinkedReadCompletion::Declined) => {}
            Ok(LinkedReadCompletion::LookupError(error)) => {
                return throw_error(runtime, realm, error).map(PropertyProgress::Deferred);
            }
            Err(error) => {
                runtime
                    .update_active_bytecode_pc(
                        frame.active_frame,
                        super::BytecodePc::new(frame.fault_pc),
                    )
                    .map_err(runtime_error_to_vm_error)?;
                drop(retained_key);
                drop(preserved_receiver);
                return Err(error);
            }
        }
    }
    let base = execution.slots.peek(&frame.window, usize::from(computed))?;
    if computed && matches!(base, Value::Null | Value::Undefined) {
        let key = execution.slots.peek(&frame.window, 0)?;
        let message = if matches!(key_kind, ReadKey::Computed { keep_key: true })
            && !matches!(key, Value::Int(_) | Value::String(_) | Value::Symbol(_))
        {
            "value has no property"
        } else if matches!(base, Value::Null) {
            "cannot read property of null"
        } else {
            "cannot read property of undefined"
        };
        return throw_error(
            runtime,
            realm,
            Error::new(crate::engine::api::error::ErrorKind::Type, message),
        )
        .map(PropertyProgress::Deferred);
    }
    let (key, retained_key) = match key_kind {
        ReadKey::Static(index) => {
            let Some(atom) = frame
                .executable
                .property_key_atoms
                .as_ref()
                .and_then(|atoms| atoms.get(index as usize))
                .copied()
                .filter(|atom| !atom.is_null())
            else {
                return Err(Error::internal("property read has no linked key"));
            };
            let key = Some(
                PropertyKey::from_borrowed_atom(runtime.clone(), atom)
                    .map_err(|error| Error::internal(error.to_string()))?,
            );
            (key, None)
        }
        ReadKey::Computed { keep_key } => {
            let value = execution.slots.peek(&frame.window, 0)?;
            if matches!(value, Value::Object(_)) {
                return Err(Error::internal(
                    "object property key did not enter its conversion operation",
                ));
            }
            let key = match runtime
                .native_to_property_key(realm, value.clone())
                .map_err(runtime_error_to_vm_error)?
            {
                NativeConversion::Value(key) => key,
                NativeConversion::Throw(value) => {
                    return Ok(PropertyProgress::Deferred(CallStep::Complete(
                        Completion::Throw(value),
                    )));
                }
            };
            let retained = keep_key
                .then(|| match value {
                    Value::Int(_) | Value::String(_) | Value::Symbol(_) => Ok(value.clone()),
                    value => value.to_js_string().map(Value::String),
                })
                .transpose()?;
            (Some(key), retained)
        }
    };
    let depth = execution.slots.depth(&frame.window);
    // Lookup borrows the original rooted operand. Only a pending callback
    // needs a second receiver owner; completed reads move this slot directly.
    let read = match selected_read.map(Ok).unwrap_or_else(|| {
        runtime.prepare_value_property_read_borrowed(
            realm,
            base,
            key.as_ref()
                .ok_or(crate::engine::api::runtime_error::RuntimeError::Invariant(
                    "fallback read lost its key",
                ))?,
        )
    }) {
        Ok(read) => read,
        Err(error) => {
            return throw_error(runtime, realm, runtime_error_to_vm_error(error))
                .map(PropertyProgress::Deferred);
        }
    };
    match read {
        OrdinaryRead::Complete(value) => complete_read(
            execution,
            id,
            None,
            retained_key,
            keep_receiver,
            1 + usize::from(computed),
            value.unwrap_or(Value::Undefined),
            depth,
        )
        .map(|()| PropertyProgress::Completed),
        read => {
            let preserved_receiver = base.clone();
            read_pending(
                runtime,
                execution,
                id,
                preserved_receiver,
                key.ok_or_else(|| Error::internal("pending read lost its key"))?,
                read,
                retained_key,
                keep_receiver,
                1 + usize::from(computed),
                depth,
            )
            .map(PropertyProgress::Deferred)
        }
    }
}

pub(super) fn read_converted(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    input: Box<ConvertedRead>,
) -> Result<CallStep, Error> {
    let ConvertedRead {
        base,
        key,
        keep_receiver,
        keep_key,
    } = *input;
    if matches!(key, Value::Object(_)) {
        return Err(Error::internal(
            "ToPrimitive returned an object property key",
        ));
    }
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let depth = execution.slots.depth(&frame.window) + 2;
    // After an object-key conversion, GetArrayEl3 retains String/Symbol, even
    // if ToPrimitive returned an Int. Direct Int keys retain their original tag.
    let retained = if keep_key {
        Some(match &key {
            Value::Symbol(_) | Value::String(_) => key.clone(),
            value => Value::String(value.to_js_string()?),
        })
    } else {
        None
    };
    let key = match runtime
        .native_to_property_key(realm, key)
        .map_err(runtime_error_to_vm_error)?
    {
        NativeConversion::Value(key) => key,
        NativeConversion::Throw(value) => return Ok(CallStep::Complete(Completion::Throw(value))),
    };
    finish_read(
        runtime,
        execution,
        id,
        base,
        key,
        retained,
        keep_receiver,
        0,
        depth,
    )
    .map(PropertyProgress::into_call_step)
}

#[allow(clippy::too_many_arguments)]
fn finish_read(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    base: Value,
    key: PropertyKey,
    retained_key: Option<Value>,
    keep_receiver: bool,
    consume: usize,
    depth: usize,
) -> Result<PropertyProgress, Error> {
    let realm = execution.frames.current_mut(id)?.executable.realm;
    let read = match runtime.prepare_value_property_read_borrowed(realm, &base, &key) {
        Ok(read) => read,
        Err(error) => {
            return throw_error(runtime, realm, runtime_error_to_vm_error(error))
                .map(PropertyProgress::Deferred);
        }
    };
    read_prepared_progress(
        runtime,
        execution,
        id,
        base,
        key,
        read,
        retained_key,
        keep_receiver,
        consume,
        depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn read_prepared(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    preserved_receiver: Value,
    key: PropertyKey,
    read: OrdinaryRead,
    retained_key: Option<Value>,
    keep_receiver: bool,
    consume: usize,
    depth: usize,
) -> Result<CallStep, Error> {
    read_prepared_progress(
        runtime,
        execution,
        id,
        preserved_receiver,
        key,
        read,
        retained_key,
        keep_receiver,
        consume,
        depth,
    )
    .map(PropertyProgress::into_call_step)
}

#[allow(clippy::too_many_arguments)]
fn read_prepared_progress(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    preserved_receiver: Value,
    key: PropertyKey,
    read: OrdinaryRead,
    retained_key: Option<Value>,
    keep_receiver: bool,
    consume: usize,
    depth: usize,
) -> Result<PropertyProgress, Error> {
    match read {
        OrdinaryRead::Complete(value) => complete_read(
            execution,
            id,
            Some(preserved_receiver),
            retained_key,
            keep_receiver,
            consume,
            value.unwrap_or(Value::Undefined),
            depth,
        )
        .map(|()| PropertyProgress::Completed),
        read => read_pending(
            runtime,
            execution,
            id,
            preserved_receiver,
            key,
            read,
            retained_key,
            keep_receiver,
            consume,
            depth,
        )
        .map(PropertyProgress::Deferred),
    }
}

// Keep the no-callback path out of the callback dispatcher's large native frame.
#[allow(clippy::too_many_arguments)]
fn complete_read(
    execution: &mut RunningExecution,
    id: FrameId,
    mut preserved_receiver: Option<Value>,
    mut retained_key: Option<Value>,
    keep_receiver: bool,
    consume: usize,
    value: Value,
    depth: usize,
) -> Result<(), Error> {
    if consume > 2 || (preserved_receiver.is_none() && consume == 0) {
        return Err(Error::internal("property read consumes too many operands"));
    }
    let mut value = Some(value);
    let frame = execution.frames.current_mut(id)?;
    let mut transaction = execution.slots.frame_transaction(&mut frame.cold.window)?;
    let discarded = {
        let mut slots = transaction.slots();
        // Moving the base preserves its owner until after result publication.
        // The remaining removed key may be released inside this window only
        // when its tag proves that it cannot free storage or drain deferred GC.
        let immediate_key = preserved_receiver.is_none()
            && (consume == 1
                || matches!(
                    slots.peek(0)?,
                    Value::Undefined
                        | Value::Null
                        | Value::Bool(_)
                        | Value::Int(_)
                        | Value::Float(_)
                ));
        let mut discarded = [None, None];
        for destination in discarded.iter_mut().take(consume) {
            *destination = Some(slots.pop()?);
        }
        if preserved_receiver.is_none() {
            preserved_receiver = discarded[consume - 1].take();
        }
        if immediate_key {
            drop(discarded);
            publish_read_result(
                &mut slots,
                &mut frame.resume_pc,
                frame.fault_pc,
                &mut preserved_receiver,
                &mut retained_key,
                keep_receiver,
                &mut value,
            )?;
            record_read_completion(depth);
            return Ok(());
        }
        discarded
    };
    // Preserve original pop/release order outside RunSlots for owning keys
    // and externally prepared reads. The base and normalized key stay rooted.
    drop(discarded);
    let mut slots = transaction.slots();
    publish_read_result(
        &mut slots,
        &mut frame.resume_pc,
        frame.fault_pc,
        &mut preserved_receiver,
        &mut retained_key,
        keep_receiver,
        &mut value,
    )?;
    record_read_completion(depth);
    Ok(())
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn publish_read_result(
    slots: &mut super::stack::RunSlots<'_>,
    resume_pc: &mut usize,
    fault_pc: usize,
    preserved_receiver: &mut Option<Value>,
    retained_key: &mut Option<Value>,
    keep_receiver: bool,
    value: &mut Option<Value>,
) -> Result<(), Error> {
    if keep_receiver {
        slots.push_pending(preserved_receiver)?;
    }
    if retained_key.is_some() {
        slots.push_pending(retained_key)?;
    }
    *resume_pc = fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("property resume PC overflow"))?;
    slots.push_pending(value)?;
    Ok(())
}

#[inline]
fn record_read_completion(_depth: usize) {
    #[cfg(feature = "profiling")]
    {
        crate::engine::api::profiling::record_owned_instruction(_depth);
        crate::engine::api::profiling::record_owned_execution_event(
            "property_read_completed_directly",
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn read_pending(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    preserved_receiver: Value,
    key: PropertyKey,
    read: OrdinaryRead,
    retained_key: Option<Value>,
    keep_receiver: bool,
    consume: usize,
    depth: usize,
) -> Result<CallStep, Error> {
    let realm = execution.frames.current_mut(id)?.executable.realm;
    let mut request = None;
    let mut deferred = None;
    let mut proxy = None;
    let mut proxy_callback = None;
    let mut native_callback = None;
    let value = match read {
        OrdinaryRead::Complete(value) => Some(value.unwrap_or(Value::Undefined)),
        OrdinaryRead::Call { getter, receiver } => {
            let super::call::NormalizedCallback {
                callable,
                receiver,
                arguments,
                classification,
            } = match super::call::normalize_callback(runtime, realm, getter, receiver, Vec::new())?
            {
                NativeConversion::Value(call) => call,
                NativeConversion::Throw(value) => {
                    return Ok(CallStep::Complete(Completion::Throw(value)));
                }
            };
            let normal = match &classification {
                CallableExecution::Bytecode { bytecode, .. } => {
                    let state = runtime.0.state.borrow();
                    state
                        .heap
                        .function_bytecode(bytecode.bytecode_id())
                        .map_err(|error| Error::internal(error.to_string()))?
                        .metadata
                        .function_kind
                        == FunctionKind::Normal
                }
                _ => false,
            };
            let is_resumable =
                matches!(classification, CallableExecution::Bytecode { .. }) && !normal;
            let is_proxy = matches!(classification, CallableExecution::Proxy);
            let is_owned_native = matches!(&classification, CallableExecution::Native { .. }
                if super::frames::native_operation(runtime, &callable).map_err(runtime_error_to_vm_error)?.is_some());
            if let CallableExecution::Bytecode {
                bytecode,
                closure_slots,
            } = classification
                && normal
            {
                if !execution.frames.can_push() || runtime.bytecode_call_would_overflow() {
                    return runtime
                        .bytecode_stack_overflow_completion(realm, &bytecode)
                        .map(CallStep::Complete)
                        .map_err(runtime_error_to_vm_error);
                }
                request = Some(BytecodeCallRequest {
                    callable,
                    receiver,
                    arguments,
                    new_target: Value::Undefined,
                    bytecode,
                    closure_slots,
                    caller_realm: realm,
                    return_to: ReturnTarget {
                        value_use: super::frame::ReturnValue::Push,
                        owner: crate::engine::vm::frame::ReturnOwner::Frame(id),
                        tail: false,
                        operation: None,
                    },
                });
            } else if is_proxy {
                proxy_callback = Some((callable, receiver, arguments));
            } else if is_owned_native || is_resumable {
                native_callback = Some((callable, receiver, arguments));
            } else {
                deferred = Some(Action::Call {
                    callable,
                    receiver,
                    arguments,
                });
            }
            None
        }
        OrdinaryRead::Special {
            object, receiver, ..
        } => {
            // The Proxy protocol owns the remaining lookup stages. Earlier
            // key conversion stays consumed when its callbacks suspend.
            proxy = Some((object, key, receiver));
            None
        }
    };
    let frame = execution.frames.current_mut(id)?;
    for _ in 0..consume {
        execution.slots.pop(&mut frame.cold.window)?;
    }
    if keep_receiver {
        execution
            .slots
            .push(&mut frame.cold.window, preserved_receiver)?;
    }
    if let Some(key) = retained_key {
        execution.slots.push(&mut frame.cold.window, key)?;
    }
    if let Some((object, key, receiver)) = proxy {
        return super::proxy_get_driver::start(
            runtime, execution, id, object, key, receiver, depth,
        );
    }
    if let Some((callable, receiver, arguments)) = proxy_callback {
        return super::proxy_get_driver::start_call(
            runtime,
            execution,
            id,
            callable.as_object().clone(),
            receiver,
            arguments,
            false,
            depth,
        );
    }
    if let Some((callable, receiver, arguments)) = native_callback {
        return super::proxy_get_driver::start_callback_call(
            runtime, execution, id, callable, receiver, arguments, false, depth,
        );
    }
    if let Some(action) = deferred {
        return super::call_bridge::prepare_property(execution, id, realm, action, depth);
    }
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("property resume PC overflow"))?;
    if let Some(request) = request {
        let entry = request.prepare(runtime, &mut execution.call_storage)?;
        push_frame(execution, entry)?;
    } else {
        execution.slots.push(
            &mut frame.cold.window,
            value.ok_or_else(|| Error::internal("property result missing"))?,
        )?;
    }
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(CallStep::Entered)
}

#[cfg(test)]
mod read_completion_tests {
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn linked_owning_read_transaction_preserves_method_receiver_and_selected_errors() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let log='', marker={}, old;
            let o={tag:42,method(){return this.tag},get x(){log+='g';return marker}};
            old=o.x;
            if(old!==marker||o.method()!==42)return false;
            Object.defineProperty(o,'x',{get(){log+='t';throw marker}});
            try{o.x;return false}catch(e){if(e!==marker)return false}finally{log+='f'}
            let p=new Proxy({x:marker},{get(t,k,r){log+='p';return Reflect.get(t,k,r)}});
            if(p.x!==marker)return false;
            let inherited=Object.create({get x(){log+='h';return this.tag}});inherited.tag=42;
            if(inherited.x!==42)return false;
            return log==='gtfph' && ({x:{tag:42}}).x.tag===42;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn completed_reads_keep_last_receiver_and_result_owners() {
        let runtime = Runtime::new();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let result = context
            .eval(
                r#"(()=>{
            for(let i=0;i<40;i++) {
                let self=(()=>{let x={n:i};x.self=x;return x})().self;
                let item=[{n:i}][0];
                if(self.self!==self || self.n!==i || item.n!==i)throw 'lost owner';
                if(({n:i,method(){return this.n}})['method']()!==i)throw 'lost receiver';
            }
            return ({child:{tag:42}}).child;
        })()"#,
            )
            .unwrap();
        runtime.run_gc().unwrap();
        let Value::Object(object) = result else {
            panic!("expected surviving result");
        };
        assert_eq!(
            context
                .get_property(&object, &runtime.intern_property_key("tag").unwrap())
                .unwrap(),
            Value::Int(42)
        );
        drop(object);
        drop(context);
        runtime.run_gc().unwrap();
        drop(runtime);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn completed_and_pending_reads_keep_keys_receivers_and_terminal_typed_indices() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let trace='', key={toString(){trace+='k';return 'x'}};
            try { null[key] } catch(e) { if(e instanceof TypeError)trace+='n'; }
            let o={get x(){trace+='g';return {tag:7}}};
            if(o[key].tag!==7)throw 'getter result';
            let a=[];
            Object.setPrototypeOf(a,{get 0(){trace+=this===a?'h':'!';return 8}});
            if(a[0]!==8)throw 'hole';
            let symbol=Symbol(), b={[symbol]:3,1:4,true:5};
            b[symbol]++;b[1n]++;b[true]++;
            if(b[symbol]!==4 || b[1]!==5 || b[true]!==6)throw 'retained keys';
            let buffer=new ArrayBuffer(4,{maxByteLength:8}), t=new Uint8Array(buffer);
            t[0]=23;
            Object.setPrototypeOf(t,{get 0(){trace+='bad';return 99}});
            if(t[0]!==23 || t['-0']!==undefined)throw 'typed initial';
            buffer.resize(0);
            if(t[0]!==undefined || t[NaN]!==undefined)throw 'typed terminal';
            return trace==='nkgh';
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}
