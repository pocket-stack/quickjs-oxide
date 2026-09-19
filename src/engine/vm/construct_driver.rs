//! Bytecode constructor bodies run as explicit child frames. Prototype lookup
//! getter replies resume the pending constructor; exotic/native prototype reads
//! use the same owned property query as other Get operations.
use crate::engine::api::{error::Error, runtime::Runtime};
use crate::engine::code::function::metadata::FunctionKind;
use crate::engine::value::{JsValue, Value};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::vm::Completion;
use crate::engine::vm::call::{BytecodeCallRequest, CallableExecution};
use crate::engine::vm::driver::{CallStep, push_frame};
use crate::engine::vm::exception::runtime_error_to_vm_error;
use crate::engine::vm::execution::RunningExecution;
use crate::engine::vm::frame::{FrameId, ReturnTarget};

pub(super) fn enter(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    count: u16,
    _identity: u64,
) -> Result<CallStep, Error> {
    let count = usize::from(count);
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    // The constructor classifier consumes a public root; the slot owner keeps
    // its own edge until `start_construct` consumes the operands.
    let target = runtime
        .root_value(execution.slots.peek(&frame.window, count + 1)?)
        .map_err(runtime_error_to_vm_error)?;
    let constructor = match runtime.constructor_from_value(realm, target) {
        Ok(NativeConversion::Value(target)) => target,
        Ok(NativeConversion::Throw(value)) => {
            let value = runtime
                .into_jsvalue(value)
                .map_err(runtime_error_to_vm_error)?;
            return Ok(CallStep::Complete(Completion::Throw(value)));
        }
        Err(error) => {
            return super::driver::rejected_call(runtime, realm, runtime_error_to_vm_error(error));
        }
    };
    let new_target = runtime
        .dup_jsvalue(execution.slots.peek(&frame.window, count)?)
        .map_err(runtime_error_to_vm_error)?;
    let mut arguments = Vec::new();
    arguments
        .try_reserve_exact(count)
        .map_err(|_| Error::internal("construct arguments allocation failed"))?;
    for offset in (0..count).rev() {
        arguments.push(
            runtime
                .dup_jsvalue(execution.slots.peek(&frame.window, offset)?)
                .map_err(runtime_error_to_vm_error)?,
        );
    }
    super::proxy_get_driver::start_construct(
        runtime,
        execution,
        id,
        constructor,
        new_target,
        arguments,
        count + 2,
    )
}

pub(super) fn enter_default_derived(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    _identity: u64,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    if matches!(frame.cold.input.new_target, JsValue::Undefined) {
        return super::driver::rejected_call(
            runtime,
            realm,
            Error::new(
                crate::engine::api::error::ErrorKind::Type,
                "class constructors must be invoked with 'new'",
            ),
        );
    }
    // Preserve the old entry's live prototype lookup, argument snapshot, then
    // constructor validation order, including null and non-constructor errors.
    let target = runtime
        .get_prototype_of(&frame.cold.function)
        .map_err(runtime_error_to_vm_error)?
        .map_or(Value::Null, Value::Object);
    let arguments = execution
        .slots
        .snapshot_actual_arguments(&frame.window, runtime)?;
    let new_target = runtime
        .dup_jsvalue(&frame.cold.input.new_target)
        .map_err(runtime_error_to_vm_error)?;
    let constructor = match runtime.constructor_from_value(realm, target) {
        Ok(NativeConversion::Value(constructor)) => constructor,
        Ok(NativeConversion::Throw(value)) => {
            let value = runtime
                .into_jsvalue(value)
                .map_err(runtime_error_to_vm_error)?;
            return Ok(CallStep::Complete(Completion::Throw(value)));
        }
        Err(error) => {
            return super::driver::rejected_call(runtime, realm, runtime_error_to_vm_error(error));
        }
    };
    super::proxy_get_driver::start_construct(
        runtime,
        execution,
        id,
        constructor,
        new_target,
        arguments,
        0,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InitializerKind {
    Install,
    Instance,
    Static,
    Block,
}

/// No replay after begin installs brands or commits static initialization. Publication authenticates
/// class initializers as ordinary bytecode functions with zero parameters.
#[inline(never)]
pub(super) fn initializer(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    mode: InitializerKind,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    let result = (|| -> Result<CallStep, Error> {
        let frame = execution.frames.current_mut(id)?;
        let (initializer, receiver) = match mode {
            InitializerKind::Install => {
                runtime
                    .install_class_instance_initializer(
                        realm,
                        runtime
                            .root_value(execution.slots.peek(&frame.window, 2)?)
                            .map_err(runtime_error_to_vm_error)?,
                        runtime
                            .root_value(execution.slots.peek(&frame.window, 1)?)
                            .map_err(runtime_error_to_vm_error)?,
                        runtime
                            .root_value(execution.slots.peek(&frame.window, 0)?)
                            .map_err(runtime_error_to_vm_error)?,
                    )
                    .map_err(runtime_error_to_vm_error)?;
                (None, JsValue::Undefined)
            }
            InitializerKind::Instance => {
                let receiver = runtime
                    .root_value(execution.slots.peek(&frame.window, 1)?)
                    .map_err(runtime_error_to_vm_error)?;
                let callable = runtime
                    .begin_class_instance_initializer(
                        realm,
                        runtime
                            .root_value(execution.slots.peek(&frame.window, 0)?)
                            .map_err(runtime_error_to_vm_error)?,
                        &receiver,
                    )
                    .map_err(runtime_error_to_vm_error)?;
                let receiver = runtime
                    .into_jsvalue(receiver)
                    .map_err(runtime_error_to_vm_error)?;
                (callable, receiver)
            }
            InitializerKind::Static => {
                let (callable, receiver) = runtime
                    .begin_class_static_initializer(
                        realm,
                        runtime
                            .root_value(execution.slots.peek(&frame.window, 1)?)
                            .map_err(runtime_error_to_vm_error)?,
                        runtime
                            .root_value(execution.slots.peek(&frame.window, 0)?)
                            .map_err(runtime_error_to_vm_error)?,
                    )
                    .map_err(runtime_error_to_vm_error)?;
                let receiver = runtime
                    .into_jsvalue(receiver)
                    .map_err(runtime_error_to_vm_error)?;
                (Some(callable), receiver)
            }
            InitializerKind::Block => {
                let receiver = runtime
                    .root_value(&frame.cold.input.this_value)
                    .map_err(runtime_error_to_vm_error)?;
                let callable = runtime
                    .begin_class_static_block(
                        realm,
                        &frame.cold.function,
                        &receiver,
                        runtime
                            .root_value(execution.slots.peek(&frame.window, 0)?)
                            .map_err(runtime_error_to_vm_error)?,
                    )
                    .map_err(runtime_error_to_vm_error)?;
                let receiver = runtime
                    .into_jsvalue(receiver)
                    .map_err(runtime_error_to_vm_error)?;
                (Some(callable), receiver)
            }
        };
        let request = if let Some(callable) = initializer {
            let CallableExecution::Bytecode {
                bytecode,
                closure_slots,
            } = runtime
                .bytecode_for_callable(&callable)
                .map_err(runtime_error_to_vm_error)?
            else {
                return Err(Error::internal(
                    "authenticated class initializer is not bytecode",
                ));
            };
            let kind = runtime
                .0
                .state
                .borrow()
                .heap
                .function_bytecode(bytecode.bytecode_id())
                .map_err(|error| Error::internal(error.to_string()))?
                .metadata
                .function_kind;
            if kind != FunctionKind::Normal {
                return Err(Error::internal(
                    "authenticated class initializer is not ordinary bytecode",
                ));
            }
            Some(BytecodeCallRequest {
                callable,
                receiver,
                new_target: JsValue::Undefined,
                arguments: Vec::new(),
                bytecode,
                closure_slots,
                caller_realm: realm,
                return_to: ReturnTarget {
                    owner: crate::engine::vm::frame::ReturnOwner::Frame(id),
                    tail: false,
                    operation: None,
                    value_use: super::frame::ReturnValue::Discard,
                },
            })
        } else {
            None
        };
        if let Some(request) = &request {
            if !execution.frames.can_push() || runtime.bytecode_call_would_overflow() {
                return runtime
                    .bytecode_stack_overflow_completion(realm, &request.bytecode)
                    .map(CallStep::Complete)
                    .map_err(runtime_error_to_vm_error);
            }
        }
        let frame = execution.frames.current_mut(id)?;
        execution.slots.pop(&mut frame.window)?;
        frame.resume_pc = frame
            .fault_pc
            .checked_add(1)
            .ok_or_else(|| Error::internal("initializer resume PC overflow"))?;
        if let Some(request) = request {
            let entry = request.prepare(runtime, &mut execution.call_storage)?;
            push_frame(execution, entry)?;
        }
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_instruction(depth);
        Ok(CallStep::Entered)
    })();
    match result {
        Err(error) => {
            let Some(kind) =
                crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
            else {
                return Err(error);
            };
            Ok(CallStep::Complete(Completion::Throw(
                runtime
                    .new_native_error_from_error_jsvalue(realm, kind, &error)
                    .map_err(runtime_error_to_vm_error)?,
            )))
        }
        result => result,
    }
}

/// Object heritage owns its prototype lookup across every callback kind.
/// Other class publication steps are NoJs.
#[inline(never)]
pub(super) fn define_class(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    name: u32,
    has_heritage: bool,
    _identity: u64,
) -> Result<CallStep, Error> {
    use crate::engine::heap::{BytecodeConstant, RawValue};
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let parent = execution.slots.peek(&frame.window, 1)?;
    let Some(BytecodeConstant::Value(RawValue::String(name_id))) = frame.executable.constant(name)
    else {
        return Err(Error::internal("class name is not a published string"));
    };
    // The bytecode node owns the constant-pool edge for this string, so the
    // trusted read clones the payload Rc without retaining the arena node.
    let name = runtime.0.state.borrow().heap.string_fast(*name_id).clone();
    if has_heritage
        && let JsValue::Object(parent) = parent
    {
        let pending = PendingClass {
            frame: id,
            realm,
            parent: crate::engine::object::ObjectRef::from_borrowed_handle(runtime.clone(), *parent)
                .map_err(super::exception::heap_error_to_vm_error)?,
            constructor: runtime
                .dup_jsvalue(execution.slots.peek(&frame.window, 0)?)
                .map_err(runtime_error_to_vm_error)?,
            name: name.clone(),
        };
        return enter_class_parent(runtime, execution, pending);
    }
    let result = runtime.define_class_pair(
        realm,
        runtime
            .root_value(parent)
            .map_err(runtime_error_to_vm_error)?,
        runtime
            .root_value(execution.slots.peek(&frame.window, 0)?)
            .map_err(runtime_error_to_vm_error)?,
        &name,
        has_heritage,
    );
    finish_class_result(runtime, execution, id, result)
}

fn finish_class_result(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    result: Result<
        crate::engine::vm::DefineClassOutcome,
        crate::engine::api::runtime_error::RuntimeError,
    >,
) -> Result<CallStep, Error> {
    use crate::engine::vm::DefineClassOutcome;
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    // No replay once the fresh constructor/prototype pair is published.
    for _ in 0..2 {
        let discarded = execution.slots.pop(&mut frame.window)?;
        runtime
            .release_jsvalue(discarded)
            .map_err(runtime_error_to_vm_error)?;
    }
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("class definition resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    match result {
        Ok(DefineClassOutcome::Defined {
            constructor,
            prototype,
        }) => {
            execution.slots.push(&mut frame.window, constructor)?;
            execution.slots.push(&mut frame.window, prototype)?;
            Ok(CallStep::Entered)
        }
        Ok(DefineClassOutcome::Throw(value)) => Ok(CallStep::Complete(Completion::Throw(value))),
        Err(error) => {
            let error = runtime_error_to_vm_error(error);
            let Some(kind) =
                crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
            else {
                return Err(error);
            };
            Ok(CallStep::Complete(Completion::Throw(
                runtime
                    .new_native_error_from_error_jsvalue(realm, kind, &error)
                    .map_err(runtime_error_to_vm_error)?,
            )))
        }
    }
}

pub(super) struct PendingClass {
    pub(super) frame: FrameId,
    pub(super) realm: crate::engine::heap::ContextId,
    pub(super) parent: crate::engine::object::ObjectRef,
    constructor: JsValue,
    name: crate::engine::value::JsString,
}

#[inline(never)]
fn enter_class_parent(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    pending: PendingClass,
) -> Result<CallStep, Error> {
    if let Err(error) = runtime.validate_class_parent(&pending.parent) {
        return finish_class_result(runtime, execution, pending.frame, Err(error));
    }
    let parent = pending.parent.clone();
    let realm = pending.realm;
    let frame = pending.frame;
    super::proxy_get_driver::start_class_parent(
        runtime,
        execution,
        Box::new(pending),
        parent,
        realm,
        frame,
    )
}

pub(super) fn finish_class_reply(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    pending: PendingClass,
    completion: Completion,
) -> Result<CallStep, Error> {
    let result = match completion {
        Completion::Throw(value) => Ok(crate::engine::vm::DefineClassOutcome::Throw(value)),
        Completion::Return(prototype) => runtime.finish_derived_class_pair(
            pending.realm,
            runtime
                .root_and_release_jsvalue(pending.constructor)
                .map_err(runtime_error_to_vm_error)?,
            &pending.name,
            pending.parent,
            runtime
                .root_and_release_jsvalue(prototype)
                .map_err(runtime_error_to_vm_error)?,
        ),
    };
    finish_class_result(runtime, execution, pending.frame, result)
}

/// Define a field or method on an ordinary object or bytecode constructor.
/// Classification precedes any property, name, or HomeObject mutation.
#[inline(never)]
pub(super) fn define_property(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    key: Option<u32>,
    method: Option<(crate::engine::code::bytecode::DefineMethodKind, bool)>,
) -> Result<CallStep, Error> {
    use crate::engine::object::PropertyKey;
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let JsValue::Object(object) = execution
        .slots
        .peek(&frame.window, 1 + usize::from(key.is_none()))?
    else {
        if method.is_some() {
            return Err(Error::internal(if key.is_some() {
                "object-literal method target was not an Object"
            } else {
                "computed object-literal method target was not an Object"
            }));
        }
        return super::driver::rejected_call(
            runtime,
            realm,
            Error::new(crate::engine::api::error::ErrorKind::Type, "not an object"),
        );
    };
    let value = execution.slots.peek(&frame.window, 0)?;
    let computed = key.is_none();
    let key = match key {
        Some(index) => {
            let atom = frame
                .executable
                .property_key_atoms
                .as_ref()
                .and_then(|atoms| atoms.get(index as usize))
                .copied()
                .filter(|atom| !atom.is_null())
                .ok_or_else(|| Error::internal("definition has no linked property key"))?;
            PropertyKey::from_borrowed_atom(runtime.clone(), atom)
                .map_err(|e| Error::internal(e.to_string()))?
        }
        None => super::property_keys::canonical(runtime, execution.slots.peek(&frame.window, 1)?)?,
    };
    let depth = execution.slots.depth(&frame.window);
    if method.is_none() {
        let object = crate::engine::object::ObjectRef::from_borrowed_handle(runtime.clone(), *object)
            .map_err(super::exception::heap_error_to_vm_error)?;
        let value = execution.slots.pop(&mut frame.window)?;
        if computed {
            let discarded = execution.slots.pop(&mut frame.window)?;
            runtime
                .release_jsvalue(discarded)
                .map_err(runtime_error_to_vm_error)?;
        }
        return super::proxy_get_driver::start_public_field(
            runtime, execution, id, object, key, value, depth,
        );
    }
    let (kind, enumerable) = method.expect("method checked above");
    let object = crate::engine::object::ObjectRef::from_borrowed_handle(runtime.clone(), *object)
        .map_err(super::exception::heap_error_to_vm_error)?;
    let descriptor = match runtime.prepare_object_literal_method(
        &object,
        &key,
        runtime.root_value(value).map_err(runtime_error_to_vm_error)?,
        kind,
        enumerable,
    ) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                return super::driver::rejected_call(
                    runtime,
                    realm,
                    runtime_error_to_vm_error(error),
                );
            }
        };
    let discarded = execution.slots.pop(&mut frame.window)?;
    runtime
        .release_jsvalue(discarded)
        .map_err(runtime_error_to_vm_error)?;
    if computed {
        let discarded = execution.slots.pop(&mut frame.window)?;
        runtime
            .release_jsvalue(discarded)
            .map_err(runtime_error_to_vm_error)?;
    }
    super::proxy_get_driver::start_literal_definition(
        runtime,
        execution,
        id,
        crate::engine::object::object_literal::element::LiteralDefinitionStep::define(
            object, key, descriptor,
        ),
        depth,
    )
}

#[cfg(all(test, feature = "profiling"))]
mod owned_definition_tests {
    use crate::engine::{
        api::{profiling::CostProfile, runtime::Runtime},
        value::Value,
        vm::Completion,
    };

    #[test]
    fn rejected_calls_and_default_super_stay_owned() {
        for source in [
            "(function(){try{(42)()}catch(e){return e instanceof TypeError&&e.message==='not a function'?42:0}})",
            "(function(){try{({f:{}}).f()}catch(e){return e instanceof TypeError&&e.message==='not a function'?42:0}})",
            "(function(){try{return (null)()}catch(e){return e instanceof TypeError?42:0}})",
            "(function(){class D extends Object{};Object.setPrototypeOf(D,null);try{new D}catch(e){return e instanceof TypeError&&e.message==='not a function'?42:0}})",
            "(function(){class D extends Object{};Object.setPrototypeOf(D,{});try{new D}catch(e){return e instanceof TypeError?42:0}})",
            "(function(){class D extends Object{};Object.setPrototypeOf(D,()=>1);try{new D}catch(e){return e instanceof TypeError?42:0}})",
            "(function(){class D extends Object{};try{D()}catch(e){return e instanceof TypeError&&e.message===\"class constructors must be invoked with 'new'\"?42:0}})",
            "(function(){var n=0,p=new Proxy({}, {get apply(){n++;return undefined}});try{p()}catch(e){return e instanceof TypeError&&n===1?42:0}})",
            "(function(){var marker={},p=new Proxy({}, {get apply(){throw marker}});try{p()}catch(e){return e===marker?42:0}})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callable = runtime
                .callable_from_value(context.eval(source).unwrap())
                .unwrap();
            let profile = CostProfile::start();
            let result = runtime
                .call_internal(context.realm, &callable, Value::Undefined, &[])
                .unwrap();
            let _costs = profile.snapshot();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{source}: {result:?}"
            );
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn class_heritage_and_exotic_public_fields_keep_callbacks_in_owned_frames() {
        for source in [
            "(function(){var calls=0;var parent=new Proxy(function(){},{get(t,k,r){if(k==='prototype'){calls++;return Object.prototype}return Reflect.get(t,k,r)}});return function(){class C extends parent{}return calls===1?42:0}})()",
            "(function(){var calls=0,target={},proxy=new Proxy(target,{defineProperty(t,k,d){calls++;if(k!=='x'||d.value!==42||!d.writable||!d.enumerable||!d.configurable)throw 99;return Reflect.defineProperty(t,k,d)}});class Base{constructor(){return proxy}}return function(){class C extends Base{x=42}new C;return calls===1&&target.x===42?42:0}})()",
            "(function(){var calls=0,target=new Uint8Array(1);class Base{constructor(){return target}}return function(){class C extends Base{0={valueOf(){calls++;return 42}}}new C;return calls===1&&target[0]===42?42:0}})()",
            "(function(){var marker={},calls=0,proxy=new Proxy({}, {defineProperty(){calls++;throw marker}});class Base{constructor(){return proxy}}return function(){try{class C extends Base{x=42}new C}catch(e){return e===marker&&calls===1?42:0}return 0}})()",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callable = runtime
                .callable_from_value(context.eval(source).unwrap())
                .unwrap();
            let profile = CostProfile::start();
            let result = runtime
                .call_internal(context.realm, &callable, Value::Undefined, &[])
                .unwrap();
            let _costs = profile.snapshot();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{source}: {result:?}"
            );
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }
}
