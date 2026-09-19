//! Schedule typed domain requests and their JavaScript child frames.
//! Domain owners retain algorithms; this driver owns reply routing and roots.
use super::{
    Completion,
    call::{BytecodeCallRequest, CallableExecution, DirectCallTarget},
    driver::{CallStep, push_frame},
    exception::runtime_error_to_vm_error,
    execution::RunningExecution,
    frame::{FrameId, OperationTarget, ReturnOwner, ReturnTarget, ReturnValue},
};
use crate::engine::api::{Error, runtime::Runtime};
use crate::engine::code::function::metadata::FunctionKind;
use crate::engine::object::{
    CompleteOrdinaryPropertyDescriptor, ObjectRef, OrdinaryRead, PropertyKey, ProxyGetResume,
    ProxyGetStep, ProxyOwnResume, ProxyOwnStep,
};
use crate::engine::object::{
    OrdinaryPropertyDescriptor, PreparedHas, ProxyBooleanKind, ProxyBooleanResume, ProxyBooleanStep,
};
use crate::engine::value::conversion::descriptor::{DescriptorResume, DescriptorStep};
use crate::engine::value::{Value, conversion::NativeConversion};

use crate::engine::object::{ProxyPrototypeKind, ProxyPrototypeStep};

mod construct;
mod dispatch_conversion;
mod dispatch_execution;
mod dispatch_iteration;
mod dispatch_read;
mod dispatch_write;

mod native;
#[cfg(feature = "profiling")]
mod profiling;
mod request;
mod storage;
use native::start_into as native_scope;
use request::{Resume, Step};
pub(super) use storage::QueryStorage;

pub(super) struct PendingProxyGet {
    identity: u64,
    // The innermost domain guard must leave before its native activation.
    resume: Resume,
    query: Query,
}

impl PendingProxyGet {
    pub(super) fn is_direct_property_read(&self, operation: Option<OperationTarget>) -> bool {
        operation == Some(OperationTarget::PropertyGet(self.identity))
            && matches!(self.query.finish, Some(Finish::PropertyRead(_)))
    }

    #[cfg(test)]
    pub(super) fn with_parent_depth_for_test(
        realm: crate::engine::heap::ContextId,
        depth: usize,
    ) -> Box<Self> {
        Box::new(Self {
            identity: 0,
            resume: Resume::Identity,
            query: Query {
                #[cfg(feature = "profiling")]
                had_callback: false,
                realm,
                parents: Parents((0..depth).map(|_| Resume::Identity).collect()),
                natives: Vec::new(),
                saved_native_depth: 0,
                spare_parents: Vec::new(),
                finish: None,
            },
        })
    }

    pub(super) fn continuation_depth(&self) -> usize {
        self.query.continuation_depth()
    }
}

#[derive(Default)]
struct Parents(Vec<Resume>);
impl Parents {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn try_reserve(&mut self, additional: usize) -> Result<(), std::collections::TryReserveError> {
        let previous = self.0.capacity();
        storage::reserve(&mut self.0, additional, "query.parents")?;
        #[cfg(feature = "profiling")]
        if self.0.capacity() != previous {
            crate::engine::api::profiling::record_owned_execution_event(
                "query_parent_capacity_growth",
            );
        }
        #[cfg(not(feature = "profiling"))]
        let _ = previous;
        Ok(())
    }
    fn push(&mut self, resume: Resume) {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("parent_push");
        self.0.push(resume);
    }
    fn pop(&mut self) -> Option<Resume> {
        let result = self.0.pop();
        #[cfg(feature = "profiling")]
        if result.is_some() {
            crate::engine::api::profiling::record_owned_execution_event("parent_pop");
        }
        result
    }
}

struct Query {
    #[cfg(feature = "profiling")]
    had_callback: bool,
    realm: crate::engine::heap::ContextId,
    parents: Parents,
    natives: Vec<NativeScope>,
    saved_native_depth: u128,
    spare_parents: Vec<Parents>,
    finish: Option<Finish>,
}
struct NativeScope {
    call: super::call::PreparedNativeCall,
    parents: Parents,
    resume: Resume,
    parent_realm: crate::engine::heap::ContextId,
}
impl Query {
    fn continuation_depth(&self) -> usize {
        usize::try_from(self.saved_native_depth + self.parents.len() as u128).unwrap_or(usize::MAX)
    }

    fn finish_native(
        &mut self,
        runtime: &Runtime,
        slots: &mut super::stack::SlotStore,
        result: Result<Completion, Error>,
    ) -> Result<Step, Error> {
        self.finish_native_outcome(
            runtime,
            slots,
            result.map(super::call::NativeInvokeOutcome::Completion),
        )
    }
    fn finish_native_outcome(
        &mut self,
        runtime: &Runtime,
        slots: &mut super::stack::SlotStore,
        result: Result<super::call::NativeInvokeOutcome, Error>,
    ) -> Result<Step, Error> {
        let scope = self
            .natives
            .pop()
            .ok_or_else(|| Error::internal("native result has no scope"))?;
        self.saved_native_depth -= 1 + scope.parents.len() as u128;
        while self.parents.pop().is_some() {}
        let empty = std::mem::replace(&mut self.parents, scope.parents);
        // Reservation happens before installing the native scope.
        self.spare_parents.push(empty);
        self.realm = scope.parent_realm;
        native::finish(runtime, slots, scope.call, scope.resume, result)
    }
}
impl Drop for Query {
    fn drop(&mut self) {
        // Current domain states belong to the innermost native activation.
        // Each saved resume/parent stack belongs to its caller, outside that
        // activation; release them before proceeding to the next outer scope.
        while self.parents.pop().is_some() {}
        while let Some(mut scope) = self.natives.pop() {
            drop(scope.call);
            drop(scope.resume);
            while scope.parents.pop().is_some() {}
        }
    }
}

enum Next {
    Continue,
    Invoke,
    Done(Progress),
    Call {
        entry: Box<super::frame::FrameEntry>,
        pc: usize,
        resume: Resume,
    },
}

enum Finish {
    Root,
    ForIn(usize),
    Class(Box<super::construct_driver::PendingClass>),
    Numeric(usize),
    VmCall(ReturnValue),
    Discard(usize),
    Iterator(FrameId),
    IteratorNext(FrameId),
    Write {
        key: PropertyKey,
        strict: bool,
        depth: usize,
    },
    PropertyRead(usize),
    Call {
        depth: usize,
        tail: bool,
    },
    Conversion(super::conversion_driver::ConversionWait),
}

fn finish_numeric(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    value: JsValue,
    previous: Option<JsValue>,
    _depth: usize,
) -> Result<CallStep, Error> {
    super::frame_operations::commit_numeric_output(
        runtime, execution, frame, value, previous, _depth,
    )?;
    Ok(CallStep::Entered)
}

fn finish_instruction(
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    completion: Completion,
    push: bool,
    depth: usize,
) -> Result<Progress, Error> {
    finish_instruction_call(execution, owner, completion, push, depth).map(Progress::Call)
}

// Shared instruction completion keeps the broad conversion transport outside
// callers that already know they are completing an ordinary property opcode.
fn finish_instruction_call(
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    completion: Completion,
    push: bool,
    _depth: usize,
) -> Result<CallStep, Error> {
    match completion {
        Completion::Return(value) => {
            let parent = execution.frames.current_mut(owner.frame()?)?;
            if push {
                execution.slots.push(&mut parent.window, value)?;
            }
            parent.resume_pc = parent
                .fault_pc
                .checked_add(1)
                .ok_or_else(|| Error::internal("property resume PC overflow"))?;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_instruction(_depth);
            Ok(CallStep::Entered)
        }
        completion => Ok(CallStep::Complete(completion)),
    }
}

pub(super) enum Progress {
    Call(CallStep),
    Conversion(super::conversion_driver::ConversionTask),
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    object: ObjectRef,
    key: PropertyKey,
    receiver: Value,
    depth: usize,
) -> Result<CallStep, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("property operation identity exhausted"))?;
    parent.property_generation = identity;
    let realm = parent.executable.realm;
    let result = (|| {
        let step = ProxyGetStep::start_buffered(
            runtime,
            realm,
            object,
            key,
            receiver,
            execution.slots.take_argument_buffer(3)?,
        )
        .map_err(runtime_error_to_vm_error)?;
        advance(
            runtime,
            execution,
            frame,
            identity,
            Vec::new(),
            step.into(),
            Finish::PropertyRead(depth),
        )
    })();
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("property read returned a conversion")),
    }
}

/// A super lookup retains its frozen base independently of the getter receiver.
#[allow(clippy::too_many_arguments)]
pub(super) fn start_owned_read(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    object: ObjectRef,
    key: PropertyKey,
    receiver: Value,
    depth: usize,
) -> Result<CallStep, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("property operation identity exhausted"))?;
    parent.property_generation = identity;
    let realm = parent.executable.realm;
    let step = Step::Read {
        object: Some(object.clone()),
        key: Some(key),
        receiver: Some(receiver),
        resume: Some(Resume::ReadOwner(object)),
    };
    let result = advance(
        runtime,
        execution,
        frame,
        identity,
        Vec::new(),
        step,
        Finish::PropertyRead(depth),
    );
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("super read returned a conversion")),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_boolean(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    object: ObjectRef,
    kind: ProxyBooleanKind,
    strict_delete: bool,
    depth: usize,
) -> Result<CallStep, Error> {
    let realm = execution.frames.current_mut(frame)?.executable.realm;
    if let ProxyBooleanKind::Delete(key) = &kind
        && !runtime
            .is_proxy_object(&object)
            .map_err(runtime_error_to_vm_error)?
    {
        let result = runtime
            .delete_property(&object, key)
            .and_then(|deleted| {
                runtime.finish_property_delete(NativeConversion::Value(deleted), strict_delete)
            })
            .map_err(runtime_error_to_vm_error);
        // These input owners are released before the instruction publishes its
        // result, exactly as in the completed BooleanResult adapter.
        drop(kind);
        drop(object);
        return match result {
            Ok(completion) => {
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_execution_event(
                    "delete_completed_without_query",
                );
                finish_instruction_call(
                    execution,
                    ReturnOwner::Frame(frame),
                    completion,
                    true,
                    depth,
                )
            }
            Err(error) => super::property_driver::throw_error(runtime, realm, error),
        };
    }
    let parent = execution.frames.current_mut(frame)?;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("property operation identity exhausted"))?;
    parent.property_generation = identity;
    let realm = parent.executable.realm;
    let resume = Resume::BooleanResult {
        payload: Box::new(request::BooleanResultPayload {
            _object: object.clone(),
            _key: match &kind {
                ProxyBooleanKind::Has(key) | ProxyBooleanKind::Delete(key) => Some(key.clone()),
                _ => None,
            },
            strict_delete,
        }),
    };
    let step = match kind {
        ProxyBooleanKind::Has(key) => Step::Has {
            object: Some(object),
            key: Some(key),
            resume: Some(resume),
        },
        ProxyBooleanKind::Delete(key) => Step::Delete {
            object: Some(object),
            key: Some(key),
            resume: Some(resume),
        },
        ProxyBooleanKind::Extensible => Step::Extensible {
            object: Some(object),
            resume: Some(resume),
        },
        ProxyBooleanKind::PreventExtensions => Step::PreventExtensions {
            object: Some(object),
            resume: Some(resume),
        },
    };
    let result = advance(
        runtime,
        execution,
        frame,
        identity,
        Vec::new(),
        step,
        Finish::PropertyRead(depth),
    );
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("boolean query returned a conversion")),
    }
}

/// The query entry is also used to validate the protocol before native entry migration.
#[cfg(test)]
pub(super) fn start_prototype(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    object: ObjectRef,
    kind: ProxyPrototypeKind,
) -> Result<CallStep, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("property operation identity exhausted"))?;
    parent.property_generation = identity;
    let realm = parent.executable.realm;
    let result = (|| {
        let mut parents = Vec::new();
        storage::reserve(&mut parents, 1, "query.parents")
            .map_err(|_| Error::internal("property continuation allocation failed"))?;
        parents.push(Resume::ReadOwner(object.clone()));
        let step = ProxyPrototypeStep::start(runtime, realm, object, kind)
            .map_err(runtime_error_to_vm_error)?;
        advance(
            runtime,
            execution,
            frame,
            identity,
            parents,
            step.into(),
            Finish::PropertyRead(0),
        )
    })();
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("prototype query returned a conversion")),
    }
}

pub(super) fn start_conversion(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    object: ObjectRef,
    key: PropertyKey,
    wait: super::conversion_driver::ConversionWait,
) -> Result<Progress, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("property operation identity exhausted"))?;
    parent.property_generation = identity;
    let realm = parent.executable.realm;
    let result = (|| {
        let receiver = Value::Object(object.clone());
        let step = ProxyGetStep::start_buffered(
            runtime,
            realm,
            object,
            key,
            receiver,
            execution.slots.take_argument_buffer(3)?,
        )
        .map_err(runtime_error_to_vm_error)?;
        advance(
            runtime,
            execution,
            frame,
            identity,
            Vec::new(),
            step.into(),
            Finish::Conversion(wait),
        )
    })();
    finish_error(runtime, realm, result)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    proxy: ObjectRef,
    receiver: crate::engine::value::JsValue,
    arguments: Vec<crate::engine::value::JsValue>,
    tail: bool,
    depth: usize,
) -> Result<CallStep, Error> {
    let receiver = runtime
        .root_and_release_jsvalue(receiver)
        .map_err(runtime_error_to_vm_error)?;
    let arguments = arguments
        .into_iter()
        .map(|argument| runtime.root_and_release_jsvalue(argument))
        .collect::<Result<Vec<_>, _>>()
        .map_err(runtime_error_to_vm_error)?;
    match start_proxy_call(
        runtime,
        execution,
        frame,
        proxy,
        receiver,
        arguments,
        Finish::Call { depth, tail },
    )? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("Proxy call returned a conversion")),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_callback_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    callable: crate::engine::object::CallableRef,
    receiver: crate::engine::value::JsValue,
    arguments: Vec<crate::engine::value::JsValue>,
    tail: bool,
    depth: usize,
) -> Result<CallStep, Error> {
    let receiver = runtime
        .root_and_release_jsvalue(receiver)
        .map_err(runtime_error_to_vm_error)?;
    let arguments = arguments
        .into_iter()
        .map(|argument| runtime.root_and_release_jsvalue(argument))
        .collect::<Result<Vec<_>, _>>()
        .map_err(runtime_error_to_vm_error)?;
    match start_owned_callback(
        runtime,
        execution,
        frame,
        callable,
        receiver,
        arguments,
        Finish::Call { depth, tail },
    )? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("native call returned a conversion")),
    }
}

/// Enter a native target already normalized and classified by `enter_call`.
/// Generic callback entry still performs normalization for its other callers.
#[allow(clippy::too_many_arguments)]
pub(super) fn start_classified_native_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    callable: crate::engine::object::CallableRef,
    target: crate::engine::builtins::native::NativeFunctionId,
    defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    receiver: Value,
    arguments: Vec<Value>,
    tail: bool,
    depth: usize,
) -> Result<CallStep, Error> {
    start_native_with_classification(
        runtime,
        execution,
        frame,
        callable,
        target,
        defining_realm,
        min_readable_args,
        receiver,
        arguments,
        tail,
        depth,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_native_with_classification(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    callable: crate::engine::object::CallableRef,
    target: crate::engine::builtins::native::NativeFunctionId,
    defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    receiver: Value,
    arguments: Vec<Value>,
    tail: bool,
    depth: usize,
    selected: Option<super::frames::NativeClassification>,
    operation: Option<crate::engine::builtins::continuation::NativeOperation>,
) -> Result<CallStep, Error> {
    let kind = match operation {
        Some(operation) => Some(operation),
        None => super::frames::native_operation(runtime, &callable)
            .map_err(runtime_error_to_vm_error)?,
    }
    .ok_or_else(|| Error::internal("classified native lost owned operation"))?;
    if let Some(synchronous) = kind.synchronous(&arguments) {
        let realm = execution.frames.current_mut(frame)?.executable.realm;
        let result = (|| {
            {
                let _operation = runtime.operation();
            }
            let completion = if !execution.frames.can_push_with_continuations(0)
                || runtime.host_stack_would_overflow()
            {
                overflow(runtime, realm)?
            } else {
                native::begin_synchronous(
                    runtime,
                    &mut execution.slots,
                    realm,
                    callable,
                    target,
                    defining_realm,
                    min_readable_args,
                    receiver,
                    arguments,
                    synchronous,
                    selected,
                )?
            };
            let result = finish_call_instruction_call(
                execution,
                ReturnOwner::Frame(frame),
                completion,
                depth,
                tail,
            );
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "native_call_completed_without_query",
            );
            result
        })();
        return result.or_else(|error| super::property_driver::throw_error(runtime, realm, error));
    }
    start_waitable_native_call(
        runtime,
        execution,
        frame,
        callable,
        target,
        defining_realm,
        min_readable_args,
        receiver,
        arguments,
        tail,
        depth,
        selected,
        kind,
    )
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(super) fn start_waitable_native_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    callable: crate::engine::object::CallableRef,
    target: crate::engine::builtins::native::NativeFunctionId,
    defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    receiver: Value,
    arguments: Vec<Value>,
    tail: bool,
    depth: usize,
    selected: Option<super::frames::NativeClassification>,
    kind: crate::engine::builtins::continuation::NativeOperation,
) -> Result<CallStep, Error> {
    let realm = execution.frames.current_mut(frame)?.executable.realm;
    let owner = ReturnOwner::Frame(frame);
    let result = (|| {
        {
            let _operation = runtime.operation();
        }
        if !execution.frames.can_push_with_continuations(0) || runtime.host_stack_would_overflow() {
            return finish_call_instruction_call(
                execution,
                owner,
                overflow(runtime, realm)?,
                depth,
                tail,
            );
        }
        match native::begin_local(
            runtime,
            &mut execution.slots,
            &mut execution.query_storage,
            realm,
            callable,
            target,
            defining_realm,
            min_readable_args,
            receiver,
            arguments,
            kind,
            selected,
            execution.frames.can_push_with_continuations(1),
        )? {
            native::LocalNativeResult::Complete(completion) => {
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_execution_event(
                    "native_call_completed_without_query",
                );
                finish_call_instruction_call(execution, owner, completion, depth, tail)
            }
            native::LocalNativeResult::Waiting(mut records) => {
                // Take individual live fields, never pop/move the wide record.
                let call = records[0].call.take().expect("waiting activation");
                let mut parent = records[0].parents.pop();
                let mut query = execution.query_storage.acquire(
                    realm,
                    Vec::new(),
                    Finish::Call { depth, tail },
                );
                let identity = (|| {
                    let frame_state = execution.frames.current_mut(frame)?;
                    let identity = frame_state
                        .property_generation
                        .checked_add(1)
                        .ok_or_else(|| Error::internal("property operation identity exhausted"))?;
                    storage::reserve(
                        &mut query.natives,
                        1 + usize::from(parent.is_some()),
                        "query.native_scopes",
                    )
                    .map_err(|_| Error::internal("native continuation allocation failed"))?;
                    storage::reserve(
                        &mut query.spare_parents,
                        1 + usize::from(parent.is_some()),
                        "query.spare_parents",
                    )
                    .map_err(|_| Error::internal("native parent storage allocation failed"))?;
                    frame_state.property_generation = identity;
                    Ok(identity)
                })();
                let identity = match identity {
                    Ok(identity) => identity,
                    Err(error) => {
                        let mut result =
                            native::finish_result(runtime, &mut execution.slots, call, Err(error));
                        // Release the abandoned inner state while its outer
                        // activation still owns the protocol call. The reply
                        // resume is likewise consumed before the outer finish.
                        records[0].step =
                            Step::Complete(Some(Completion::Return(
                                crate::engine::value::JsValue::Undefined,
                            )));
                        if let Some(mut parent) = parent.take() {
                            let outer = parent.call.take().expect("outer replace activation");
                            drop(parent);
                            result =
                                native::finish_result(runtime, &mut execution.slots, outer, result);
                        }
                        let result = result.and_then(native::identity_completion);
                        execution.query_storage.recycle_native_wait(records);
                        query.recycle(&mut execution.query_storage);
                        return result.and_then(|completion| {
                            finish_call_instruction_call(execution, owner, completion, depth, tail)
                        });
                    }
                };
                let resume = if let Some(mut parent) = parent.take() {
                    native::install_waiting(
                        &mut query,
                        parent.call.take().expect("outer replace activation"),
                        Resume::Identity,
                    );
                    Resume::StringReplace(parent.resume)
                } else {
                    Resume::Identity
                };
                native::install_waiting(&mut query, call, resume);
                let step = std::mem::replace(
                    &mut records[0].step,
                    Step::Complete(Some(Completion::Return(
                        crate::engine::value::JsValue::Undefined,
                    ))),
                );
                execution.query_storage.recycle_native_wait(records);
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_execution_event(
                    "native_call_direct_wait",
                );
                drive_native_call(runtime, execution, owner, identity, query, Ok(step))
            }
        }
    })();
    result.or_else(|error| super::property_driver::throw_error(runtime, realm, error))
}

fn finish_call_instruction(
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    completion: Completion,
    depth: usize,
    tail: bool,
) -> Result<Progress, Error> {
    finish_call_instruction_call(execution, owner, completion, depth, tail).map(Progress::Call)
}

fn finish_call_instruction_call(
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    completion: Completion,
    depth: usize,
    tail: bool,
) -> Result<CallStep, Error> {
    if tail {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_instruction(depth);
        return Ok(CallStep::Complete(completion));
    }
    finish_instruction_call(execution, owner, completion, true, depth)
}

#[inline(never)]
fn drive_native_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    identity: u64,
    query: Query,
    result: Result<Step, Error>,
) -> Result<CallStep, Error> {
    match drive(runtime, execution, owner, identity, query, result)? {
        Progress::Call(step) => Ok(step),
        // Internal invariant errors pass unchanged through throw_error.
        Progress::Conversion(_) => Err(Error::internal("native call returned a conversion")),
    }
}

pub(super) fn start_apply(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    kind: crate::engine::code::bytecode::ApplyKind,
) -> Result<CallStep, Error> {
    let realm = execution.frames.current_mut(frame)?.executable.realm;
    let result = (|| {
        let parent = execution.frames.current_mut(frame)?;
        // The operands stay in their slots; the spread machine borrows rooted
        // copies while `start_instruction` consumes the slot owners.
        let step = crate::engine::builtins::InvokeStep::start_spread(
            runtime,
            realm,
            kind,
            runtime
                .root_value(execution.slots.peek(&parent.window, 2)?)
                .map_err(runtime_error_to_vm_error)?,
            runtime
                .root_value(execution.slots.peek(&parent.window, 1)?)
                .map_err(runtime_error_to_vm_error)?,
            runtime
                .root_value(execution.slots.peek(&parent.window, 0)?)
                .map_err(runtime_error_to_vm_error)?,
        )
        .map_err(runtime_error_to_vm_error)?;
        start_instruction(runtime, execution, frame, step.into(), 3)
    })();
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("Apply returned a conversion")),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_construct(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    target: super::call::ConstructorRef,
    new_target: crate::engine::value::JsValue,
    arguments: Vec<crate::engine::value::JsValue>,
    operand_count: usize,
) -> Result<CallStep, Error> {
    let realm = execution.frames.current_mut(frame)?.executable.realm;
    // The construct machine consumes public roots; the operand owners are
    // rooted at this entry boundary.
    let arguments = arguments
        .into_iter()
        .map(|argument| runtime.root_and_release_jsvalue(argument))
        .collect::<Result<Vec<_>, _>>()
        .map_err(runtime_error_to_vm_error)?;
    let result = start_instruction(
        runtime,
        execution,
        frame,
        Step::Construct {
            target: Some(target),
            new_target: Some(super::call::ConstructNewTarget::Raw(new_target)),
            arguments: Some(arguments),
            resume: Some(Resume::Identity),
        },
        operand_count,
    );
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("Construct returned a conversion")),
    }
}

fn start_instruction(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    step: Step,
    operand_count: usize,
) -> Result<Progress, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("instruction operation identity exhausted"))?;
    parent.property_generation = identity;
    let depth = execution.slots.depth(&parent.window);
    // The request owns every source value before any window owner is released.
    for _ in 0..operand_count {
        execution.slots.pop(&mut parent.window)?;
    }
    advance(
        runtime,
        execution,
        frame,
        identity,
        Vec::new(),
        step,
        Finish::Call { depth, tail: false },
    )
}

pub(super) fn start_native_conversion_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    callable: crate::engine::object::CallableRef,
    receiver: Value,
    arguments: Vec<Value>,
    wait: super::conversion_driver::ConversionWait,
) -> Result<Progress, Error> {
    start_owned_callback(
        runtime,
        execution,
        frame,
        callable,
        receiver,
        arguments,
        Finish::Conversion(wait),
    )
}

fn start_owned_callback(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    callable: crate::engine::object::CallableRef,
    receiver: Value,
    arguments: Vec<Value>,
    finish: Finish,
) -> Result<Progress, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("property operation identity exhausted"))?;
    parent.property_generation = identity;
    let realm = parent.executable.realm;
    let result = advance(
        runtime,
        execution,
        frame,
        identity,
        Vec::new(),
        Step::Call {
            target: Some(DirectCallTarget::Callable(callable)),
            receiver: Some(receiver),
            arguments: Some(arguments),
            resume: Some(Resume::Identity),
        },
        finish,
    );
    finish_error(runtime, realm, result)
}

pub(super) fn start_conversion_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    proxy: ObjectRef,
    receiver: Value,
    arguments: Vec<Value>,
    wait: super::conversion_driver::ConversionWait,
) -> Result<Progress, Error> {
    start_proxy_call(
        runtime,
        execution,
        frame,
        proxy,
        receiver,
        arguments,
        Finish::Conversion(wait),
    )
}

fn start_proxy_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    proxy: ObjectRef,
    receiver: Value,
    arguments: Vec<Value>,
    finish: Finish,
) -> Result<Progress, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("property operation identity exhausted"))?;
    parent.property_generation = identity;
    let realm = parent.executable.realm;
    let result = (|| {
        let step =
            crate::engine::object::ProxyCallStep::start(runtime, realm, proxy, receiver, arguments)
                .map_err(runtime_error_to_vm_error)?;
        advance(
            runtime,
            execution,
            frame,
            identity,
            Vec::new(),
            step.into(),
            finish,
        )
    })();
    finish_error(runtime, realm, result)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_write(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    object: ObjectRef,
    key: PropertyKey,
    value: Value,
    receiver: Value,
    strict: bool,
    depth: usize,
) -> Result<CallStep, Error> {
    start_write_progress(
        runtime, execution, frame, object, key, value, receiver, strict, depth,
    )
    .map(super::property_driver::PropertyProgress::into_call_step)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_write_progress(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    object: ObjectRef,
    key: PropertyKey,
    value: Value,
    receiver: Value,
    strict: bool,
    depth: usize,
) -> Result<super::property_driver::PropertyProgress, Error> {
    start_write_adapted(
        runtime,
        execution,
        frame,
        Some(object),
        key,
        value,
        receiver,
        strict,
        depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_receiver_write_progress(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    key: PropertyKey,
    value: Value,
    receiver: Value,
    strict: bool,
    depth: usize,
) -> Result<super::property_driver::PropertyProgress, Error> {
    start_write_adapted(
        runtime, execution, frame, None, key, value, receiver, strict, depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn start_write_adapted(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    object: Option<ObjectRef>,
    key: PropertyKey,
    value: Value,
    receiver: Value,
    strict: bool,
    depth: usize,
) -> Result<super::property_driver::PropertyProgress, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let realm = parent.executable.realm;
    let result = (|| {
        let mut waiting_result = None;
        let waiting = |step| {
            waiting_result = Some(if waiting_result.is_some() {
                Err(Error::internal("Set start repeated its waiting step"))
            } else {
                // The selector borrows the finalization key. Only a pending
                // continuation needs a separate finalization owner.
                advance_write_pending(runtime, execution, frame, step, key.clone(), strict, depth)
            });
        };
        let action = match object {
            Some(object) => crate::engine::object::SetStep::start_into(
                runtime,
                Some(realm),
                object,
                key.clone(),
                value,
                receiver,
                waiting,
            ),
            None => crate::engine::object::SetStep::start_receiver_into(
                runtime, realm, &key, value, receiver, waiting,
            ),
        }
        .map_err(runtime_error_to_vm_error)?;
        match action {
            Some(action) => {
                finish_write_action(runtime, execution, frame, action, key, strict, depth)
            }
            None => waiting_result
                .ok_or_else(|| Error::internal("Set start omitted its waiting step"))?,
        }
    })();
    match result {
        Ok(progress) => Ok(progress),
        Err(error) => finish_write_error(runtime, realm, error),
    }
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn advance_write_pending(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    step: crate::engine::object::SetStep,
    key: PropertyKey,
    strict: bool,
    depth: usize,
) -> Result<super::property_driver::PropertyProgress, Error> {
    // Set's initial operation guard has already left before entering this sink.
    match step
        .advance_without_callback(runtime)
        .map_err(runtime_error_to_vm_error)?
    {
        crate::engine::object::SetStep::Complete(action) => {
            finish_write_action(runtime, execution, frame, action, key, strict, depth)
        }
        step => schedule_write(runtime, execution, frame, step.into(), key, strict, depth),
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_write_action(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    action: crate::engine::object::operations::PropertySetAction,
    key: PropertyKey,
    strict: bool,
    depth: usize,
) -> Result<super::property_driver::PropertyProgress, Error> {
    if matches!(
        action,
        crate::engine::object::operations::PropertySetAction::Call { .. }
    ) {
        return schedule_write_action(runtime, execution, frame, action, key, strict, depth);
    }
    let completion = runtime
        .finish_property_set(
            request::set_result(action).map_err(runtime_error_to_vm_error)?,
            &key,
            strict,
        )
        .map_err(runtime_error_to_vm_error)?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event("write_completed_without_query");
    use super::property_driver::PropertyProgress;
    match finish_instruction_call(
        execution,
        ReturnOwner::Frame(frame),
        completion,
        false,
        depth,
    )? {
        CallStep::Entered => Ok(PropertyProgress::Completed),
        step => Ok(PropertyProgress::Deferred(step)),
    }
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn schedule_write_action(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    action: crate::engine::object::operations::PropertySetAction,
    key: PropertyKey,
    strict: bool,
    depth: usize,
) -> Result<super::property_driver::PropertyProgress, Error> {
    schedule_write(
        runtime,
        execution,
        frame,
        Step::SetComplete(Some(action)),
        key,
        strict,
        depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn schedule_write(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    step: Step,
    key: PropertyKey,
    strict: bool,
    depth: usize,
) -> Result<super::property_driver::PropertyProgress, Error> {
    // Only a waiting Set has a reply identity. Synchronous storage completion
    // never reads or mutates this cold counter.
    let parent = execution.frames.current_mut(frame)?;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("property operation identity exhausted"))?;
    parent.property_generation = identity;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event("set_wait_handoff");
    write_call_progress(advance(
        runtime,
        execution,
        frame,
        identity,
        Vec::new(),
        step,
        Finish::Write { key, strict, depth },
    )?)
}

fn write_call_progress(
    progress: Progress,
) -> Result<super::property_driver::PropertyProgress, Error> {
    match progress {
        Progress::Call(step) => Ok(super::property_driver::PropertyProgress::Deferred(step)),
        Progress::Conversion(_) => Err(Error::internal("Set returned a conversion operation")),
    }
}

// Error materialization runs only after all initial write input owners have
// left. Successful completion never crosses this cold normalization boundary.
#[inline(never)]
fn finish_write_error(
    runtime: &Runtime,
    realm: crate::engine::heap::ContextId,
    error: Error,
) -> Result<super::property_driver::PropertyProgress, Error> {
    super::property_driver::throw_error(runtime, realm, error)
        .map(super::property_driver::PropertyProgress::Deferred)
}

pub(super) fn reply(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    target: ReturnTarget,
    completion: Completion,
) -> Result<Progress, Error> {
    reply_outcome(
        runtime,
        execution,
        target,
        super::suspend::VmRunOutcome::Complete(completion),
    )
}

pub(super) fn reply_suspended(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    target: ReturnTarget,
    outcome: super::suspend::VmRunOutcome,
) -> Result<Progress, Error> {
    reply_outcome(runtime, execution, target, outcome)
}

fn take_pending(
    execution: &mut RunningExecution,
    owner: ReturnOwner,
) -> Result<Box<PendingProxyGet>, Error> {
    match owner {
        ReturnOwner::Frame(frame) => execution.frames.take_pending(frame),
        ReturnOwner::Root => execution
            .root_query
            .take()
            .ok_or_else(|| Error::internal("request reply has no pending operation")),
    }
}
fn put_pending(
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    pending: Box<PendingProxyGet>,
) -> Result<(), Error> {
    match owner {
        ReturnOwner::Frame(frame) => execution.frames.put_pending(frame, pending),
        ReturnOwner::Root => {
            if execution.root_query.is_some() {
                return Err(Error::internal("request overwrote a pending reply"));
            }
            execution.root_query = Some(pending);
            Ok(())
        }
    }
}

fn reply_outcome(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    target: ReturnTarget,
    outcome: super::suspend::VmRunOutcome,
) -> Result<Progress, Error> {
    let pending = take_pending(execution, target.owner)?;
    if target.operation != Some(OperationTarget::PropertyGet(pending.identity)) {
        // The rejected query used to own its iterator box. Drop its native
        // scopes first, then release the corresponding resident frame owner.
        let iterator = match pending.query.finish.as_ref() {
            Some(Finish::Iterator(id) | Finish::IteratorNext(id)) => Some(*id),
            _ => None,
        };
        drop(pending);
        if let Some(id) = iterator {
            if let Ok(frame) = execution.frames.current_mut(id) {
                if let Some(rare) = frame.cold.rare.get_mut() {
                    rare.iterator_wait = None;
                }
            }
        }
        return Err(Error::internal(
            "request reply belongs to another operation",
        ));
    }
    let (identity, query, resume) = execution.query_storage.release_pending(pending);
    let realm = match target.owner {
        ReturnOwner::Root => query.realm,
        ReturnOwner::Frame(id) => execution.frames.current_mut(id)?.executable.realm,
    };
    let step = match outcome {
        super::suspend::VmRunOutcome::Complete(completion) => resume.resume(runtime, completion),
        outcome => resume.suspended(runtime, outcome),
    }
    .map_err(runtime_error_to_vm_error);
    let result = drive(runtime, execution, target.owner, identity, query, step);
    finish_error(runtime, realm, result)
}

pub(super) fn start_root(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    realm: crate::engine::heap::ContextId,
    operation: super::driver::RootOperation,
) -> Result<Progress, Error> {
    let step: Step = match operation {
        super::driver::RootOperation::Call {
            callable,
            receiver,
            arguments,
        } => Step::Call {
            target: Some(DirectCallTarget::Callable(callable)),
            receiver: Some(receiver),
            arguments: Some(arguments),
            resume: Some(Resume::Identity),
        },
        super::driver::RootOperation::Construct(normalized) => construct::prepared(
            runtime,
            ReturnOwner::Root,
            1,
            realm,
            normalized,
            Resume::Identity,
        )?,
        super::driver::RootOperation::Get {
            object,
            key,
            receiver,
        } => Step::Read {
            object: Some(object),
            key: Some(key),
            receiver: Some(receiver),
            resume: Some(Resume::Identity),
        },
        super::driver::RootOperation::Own { object, key } => Step::Descriptor {
            object: Some(object),
            key: Some(key),
            resume: Some(Resume::RootDescriptor),
        },
        super::driver::RootOperation::Define {
            object,
            key,
            descriptor,
        } => Step::Define {
            object: Some(object),
            key: Some(key),
            descriptor: Some(descriptor),
            resume: Some(Resume::RootDefine),
        },
        super::driver::RootOperation::Set {
            object,
            key,
            value,
            receiver,
        } => Step::Set {
            object: Some(object),
            key: Some(key),
            value: Some(value),
            receiver: Some(receiver),
            resume: Some(Resume::RootSet),
        },

        super::driver::RootOperation::ModuleCallback(step) => step.into(),
        super::driver::RootOperation::ModuleEvaluation(step) => step.into(),
        super::driver::RootOperation::ModuleLink(step) => step.into(),
        super::driver::RootOperation::FromSync(step) => step.into(),
        super::driver::RootOperation::AsyncGenerator(step) => step.into(),
        super::driver::RootOperation::Promise(step) => step.into(),
        super::driver::RootOperation::Async(step) => step.into(),
    };
    let query = execution
        .query_storage
        .acquire(realm, Vec::new(), Finish::Root);
    let result = drive(runtime, execution, ReturnOwner::Root, 1, query, Ok(step));
    finish_error(runtime, realm, result)
}

fn finish_error(
    runtime: &Runtime,
    realm: crate::engine::heap::ContextId,
    result: Result<Progress, Error>,
) -> Result<Progress, Error> {
    match result {
        Ok(step) => Ok(step),
        Err(error) => {
            super::property_driver::throw_error(runtime, realm, error).map(Progress::Call)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn advance(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    identity: u64,
    parents: Vec<Resume>,
    step: Step,
    finish: Finish,
) -> Result<Progress, Error> {
    let realm = execution.frames.current_mut(frame)?.executable.realm;
    let query = execution.query_storage.acquire(realm, parents, finish);
    drive(
        runtime,
        execution,
        ReturnOwner::Frame(frame),
        identity,
        query,
        Ok(step),
    )
}

fn drive(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    identity: u64,
    query: Query,
    step: Result<Step, Error>,
) -> Result<Progress, Error> {
    let result = drive_inner(runtime, execution, owner, identity, query, step);
    if result.is_err() {
        if let Ok(id) = owner.frame() {
            if let Ok(frame) = execution.frames.current_mut(id) {
                if let Some(rare) = frame.cold.rare.get_mut() {
                    rare.iterator_wait = None;
                }
            }
        }
    }
    result
}
fn drive_inner(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    identity: u64,
    mut query: Query,
    step: Result<Step, Error>,
) -> Result<Progress, Error> {
    let mut step = step.map_err(Some);
    #[cfg(feature = "profiling")]
    {
        use crate::engine::api::profiling::record_owned_execution_layout as layout;
        layout::<Step>("Step");
        layout::<Resume>("Resume");
        layout::<Next>("Next");
        layout::<super::conversion_driver::ConversionTask>("ConversionTask");
        layout::<super::frame::FrameCold>("FrameCold");
    }
    loop {
        let result = match &mut step {
            Ok(step) => advance_inner(runtime, execution, owner, identity, &mut query, step),
            Err(error) => Err(error.take().expect("pending dispatch error")),
        };
        match result {
            Ok(Next::Done(result)) => {
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_execution_event(
                    if query.had_callback {
                        "query_completed_after_callback"
                    } else {
                        "query_completed_without_callback"
                    },
                );
                query.recycle(&mut execution.query_storage);
                return Ok(result);
            }
            Ok(Next::Call { entry, pc, resume }) => {
                #[cfg(feature = "profiling")]
                let had_callback = std::mem::replace(&mut query.had_callback, true);
                let pending = execution.query_storage.pending(identity, query, resume);
                put_pending(execution, owner, pending)?;
                match push_frame(execution, *entry) {
                    Ok(id) => {
                        #[cfg(feature = "profiling")]
                        crate::engine::api::profiling::record_owned_execution_event(
                            "query_bytecode_callback",
                        );
                        let child = execution.frames.current_mut(id)?;
                        child.resume_pc = pc;
                        child.fault_pc = pc.saturating_sub(1);
                        return Ok(Progress::Call(CallStep::Entered));
                    }
                    Err(error) => {
                        let pending = take_pending(execution, owner)?;
                        let (_, restored, resume) =
                            execution.query_storage.release_pending(pending);
                        drop(resume);
                        query = restored;
                        #[cfg(feature = "profiling")]
                        {
                            query.had_callback = had_callback;
                        }
                        step = Err(Some(error));
                    }
                }
            }
            Ok(Next::Continue | Next::Invoke) => {
                return Err(Error::internal(
                    "query dispatch escaped without a terminal step",
                ));
            }
            Err(error) if !query.natives.is_empty() => {
                // The native frame and all argv roots are still owned here.
                step = query
                    .finish_native(runtime, &mut execution.slots, Err(error))
                    .map_err(Some);
            }
            Err(error) => {
                query.recycle(&mut execution.query_storage);
                return Err(error);
            }
        }
    }
}

fn advance_inner(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    identity: u64,
    query: &mut Query,
    step: &mut Step,
) -> Result<Next, Error> {
    loop {
        // Keep domain dispatch frames bounded on the existing 256 KiB host stack.
        // Each helper returns before another request category is dispatched.
        // A plain function pointer keeps the borrowed dispatch ABI explicit without a closure capture or heap transport.
        #[allow(clippy::type_complexity)]
        let dispatch: fn(
            &Runtime,
            &mut RunningExecution,
            ReturnOwner,
            u64,
            &mut Query,
            &mut Step,
        ) -> Result<Next, Error> = match &*step {
            Step::RootDescriptor(..)
            | Step::Complete { .. }
            | Step::ForInComplete { .. }
            | Step::NumericComplete { .. }
            | Step::NativeRawComplete { .. } => dispatch_execution::finish,
            Step::ResumeFrame { .. } | Step::ConstructorReady { .. } | Step::Native { .. } => {
                dispatch_execution::activation
            }
            Step::ModuleCallbackOperation { .. }
            | Step::ModuleBodyOperation { .. }
            | Step::ModuleLink { .. }
            | Step::PromiseOperation { .. }
            | Step::IntrinsicPromiseResolve { .. }
            | Step::Construct { .. }
            | Step::ConstructProxy { .. }
            | Step::IndirectEval { .. }
            | Step::NumericHtmlDda { .. } => dispatch_execution::prepare,
            Step::RegExpSpecies { .. }
            | Step::RegExpSpeciesComplete { .. }
            | Step::Aggregate { .. }
            | Step::ArraySpecies { .. }
            | Step::ArrayPush { .. }
            | Step::IteratorNext { .. }
            | Step::IteratorNextComplete { .. }
            | Step::IteratorCall { .. }
            | Step::IteratorClose { .. }
            | Step::ObjectTag { .. }
            | Step::RegExpExec { .. }
            | Step::IteratorCloseWithResume { .. }
            | Step::OrdinaryInstance { .. }
            | Step::ParseIterator { .. }
            | Step::ArrayCopy { .. } => dispatch_iteration::advance,
            Step::String { .. }
            | Step::OrdinaryPrimitive { .. }
            | Step::Arguments { .. }
            | Step::ArgumentsComplete { .. }
            | Step::Primitive { .. }
            | Step::Number { .. }
            | Step::NumberComplete { .. }
            | Step::LengthComplete { .. }
            | Step::Element { .. }
            | Step::ElementComplete { .. }
            | Step::TypedComplete { .. } => dispatch_conversion::primitive,
            Step::ConstructorSource { .. }
            | Step::ConstructorSourceComplete { .. }
            | Step::TypedSpeciesView { .. }
            | Step::TypedIteratorMethod { .. }
            | Step::TypedIteratorMethodComplete { .. }
            | Step::TypedCollect { .. }
            | Step::TypedCollectComplete { .. }
            | Step::TypedCreate { .. }
            | Step::TypedSpecies { .. }
            | Step::TypedSpeciesComplete { .. } => dispatch_conversion::constructor,
            Step::SnapshotEnumerable { .. }
            | Step::OwnFlag { .. }
            | Step::Keys { .. }
            | Step::KeysComplete { .. }
            | Step::ReadValue { .. } => dispatch_write::keys,
            Step::SetContinue { .. }
            | Step::SetLength { .. }
            | Step::SetSpecial { .. }
            | Step::SetComplete { .. }
            | Step::PreparedSet { .. }
            | Step::Set { .. }
            | Step::SetProxy { .. } => dispatch_write::set,
            Step::Defined { .. } | Step::Define { .. } | Step::DefineOrdinary { .. } => {
                dispatch_write::define
            }
            Step::OwnComplete { .. }
            | Step::BooleanComplete { .. }
            | Step::GetPrototype { .. }
            | Step::SetPrototype { .. } => dispatch_read::prototype,
            Step::Delete { .. } | Step::PreventExtensions { .. } | Step::Extensible { .. } => {
                dispatch_read::attributes
            }
            Step::Convert { .. }
            | Step::Converted { .. }
            | Step::Has { .. }
            | Step::PreparedHas { .. }
            | Step::Read { .. }
            | Step::PreparedRead { .. }
            | Step::Call { .. }
            | Step::Descriptor { .. } => dispatch_read::get,
        };
        #[cfg(feature = "profiling")]
        profiling::record_dispatch(step);
        let next = dispatch(runtime, execution, owner, identity, query, step)?;
        let (target, receiver, arguments, resume) = match next {
            Next::Continue => {
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_execution_event("query_continue");
                continue;
            }
            Next::Invoke => {
                let Step::Call {
                    target,
                    receiver,
                    arguments,
                    resume,
                } = step
                else {
                    return Err(Error::internal("invoke marker lost resident call request"));
                };
                (
                    target.take().expect("selected call target"),
                    receiver.take().expect("selected call receiver"),
                    arguments.take().expect("selected call arguments"),
                    resume.take().expect("selected call continuation"),
                )
            }
            next => return Ok(next),
        };
        match invoke(
            runtime, execution, owner, identity, query, target, receiver, arguments, resume, step,
        )? {
            Next::Continue => {}
            next => return Ok(next),
        }
    }
}

#[inline(never)]
// Transfer the selected callable and its reply ownership directly; a bundled request would add a second transport.
#[allow(clippy::too_many_arguments)]
fn invoke(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    identity: u64,
    query: &mut Query,
    target: DirectCallTarget,
    receiver: Value,
    arguments: Vec<Value>,
    resume: Resume,
    next_step: &mut Step,
) -> Result<Next, Error> {
    let realm = query.realm;
    let step;
    let callable = match target {
        DirectCallTarget::Callable(callable) => callable,
        DirectCallTarget::NonCallableProxy(proxy) => {
            if !execution
                .frames
                .can_push_with_continuations(query.continuation_depth())
            {
                step = resume
                    .resume(runtime, overflow(runtime, realm)?)
                    .map_err(runtime_error_to_vm_error)?;
                *next_step = step;
                return Ok(Next::Continue);
            }
            query
                .parents
                .try_reserve(1)
                .map_err(|_| Error::internal("property continuation allocation failed"))?;
            query.parents.push(resume);
            step = crate::engine::object::ProxyCallStep::start(
                runtime, realm, proxy, receiver, arguments,
            )
            .map_err(runtime_error_to_vm_error)?
            .into();
            *next_step = step;
            return Ok(Next::Continue);
        }
    };
    if let Some(call) =
        super::call::ordinary::OrdinaryCall::select_callback(runtime, callable.as_object())
            .map_err(runtime_error_to_vm_error)?
    {
        if !execution
            .frames
            .can_push_with_continuations(query.continuation_depth())
            || runtime.bytecode_call_would_overflow()
        {
            call.executable()
                .ensure_root(runtime)
                .map_err(runtime_error_to_vm_error)?;
            let completion = runtime
                .bytecode_stack_overflow_completion(
                    realm,
                    call.executable().root().expect("rooted overflow frame"),
                )
                .map_err(runtime_error_to_vm_error)?;
            *next_step = resume
                .resume(runtime, completion)
                .map_err(runtime_error_to_vm_error)?;
            return Ok(Next::Continue);
        }
        let entry = call.prepare_callback(
            &mut execution.call_storage,
            receiver,
            arguments,
            realm,
            ReturnTarget {
                owner,
                value_use: ReturnValue::Push,
                tail: false,
                operation: Some(OperationTarget::PropertyGet(identity)),
            },
        )?;
        return Ok(Next::Call {
            entry: Box::new(entry),
            pc: 0,
            resume,
        });
    }
    let super::call::NormalizedCallback {
        callable,
        receiver,
        arguments,
        classification,
    } = match super::call::normalize_callback(runtime, realm, callable, receiver, arguments)? {
        NativeConversion::Value(call) => call,
        NativeConversion::Throw(value) => {
            // The normalization boundary threw a public root; transfer it into
            // the internal completion without a retain/release pair.
            let value = runtime
                .into_jsvalue(value)
                .map_err(runtime_error_to_vm_error)?;
            step = resume
                .resume(runtime, Completion::Throw(value))
                .map_err(runtime_error_to_vm_error)?;
            *next_step = step;
            return Ok(Next::Continue);
        }
    };
    if matches!(classification, CallableExecution::Proxy) {
        if !execution
            .frames
            .can_push_with_continuations(query.continuation_depth())
        {
            step = resume
                .resume(runtime, overflow(runtime, realm)?)
                .map_err(runtime_error_to_vm_error)?;
            *next_step = step;
            return Ok(Next::Continue);
        }
        query
            .parents
            .try_reserve(1)
            .map_err(|_| Error::internal("property continuation allocation failed"))?;
        query.parents.push(resume);
        // The proxy-call machine consumes public roots; the normalized
        // internal owners are rooted at this sub-driver boundary.
        let receiver = runtime
            .root_and_release_jsvalue(receiver)
            .map_err(runtime_error_to_vm_error)?;
        let arguments = arguments
            .into_iter()
            .map(|argument| runtime.root_and_release_jsvalue(argument))
            .collect::<Result<Vec<_>, _>>()
            .map_err(runtime_error_to_vm_error)?;
        step = crate::engine::object::ProxyCallStep::start(
            runtime,
            realm,
            callable.as_object().clone(),
            receiver,
            arguments,
        )
        .map_err(runtime_error_to_vm_error)?
        .into();
        *next_step = step;
        return Ok(Next::Continue);
    }
    if let CallableExecution::Native {
        target,
        realm: defining_realm,
        min_readable_args,
    } = classification
        && super::frames::native_operation(runtime, &callable)
            .map_err(runtime_error_to_vm_error)?
            .is_some()
    {
        native_scope(
            runtime,
            execution,
            query,
            callable,
            target,
            defining_realm,
            min_readable_args,
            super::call::NativeInvokeMode::Ordinary,
            super::call::NativeInvocation::Call {
                this_value: runtime
                    .root_and_release_jsvalue(receiver)
                    .map_err(runtime_error_to_vm_error)?,
            },
            arguments
                .into_iter()
                .map(|argument| runtime.root_and_release_jsvalue(argument))
                .collect::<Result<Vec<_>, _>>()
                .map_err(runtime_error_to_vm_error)?,
            resume,
            next_step,
        )?;
        return Ok(Next::Continue);
    }
    if let CallableExecution::Bytecode {
        bytecode,
        closure_slots,
    } = classification
    {
        let metadata = runtime
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(bytecode.bytecode_id())
            .map_err(|error| Error::internal(error.to_string()))?
            .metadata;
        let kind = metadata.function_kind;
        let module_link = metadata.is_module && matches!(receiver, crate::engine::value::JsValue::Bool(true));
        {
            if !execution
                .frames
                .can_push_with_continuations(query.continuation_depth())
                || runtime.bytecode_call_would_overflow()
            {
                let completion = runtime
                    .bytecode_stack_overflow_completion(realm, &bytecode)
                    .map_err(runtime_error_to_vm_error)?;
                step = resume
                    .resume(runtime, completion)
                    .map_err(runtime_error_to_vm_error)?;
                *next_step = step;
                return Ok(Next::Continue);
            }
            let resume = if matches!(kind, FunctionKind::Normal | FunctionKind::Async) {
                resume
            } else {
                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("generator creation continuation allocation failed")
                })?;
                query.parents.push(resume);
                Resume::GeneratorCreate(super::suspend::creation::GeneratorCreation {
                    realm,
                    callable: callable.clone(),
                    asynchronous: kind == FunctionKind::AsyncGenerator,
                })
            };
            let request = BytecodeCallRequest {
                callable,
                receiver,
                arguments,
                bytecode,
                closure_slots,
                new_target: crate::engine::value::JsValue::Undefined,
                caller_realm: realm,
                return_to: ReturnTarget {
                    owner,
                    value_use: ReturnValue::Push,
                    tail: false,
                    operation: Some(OperationTarget::PropertyGet(identity)),
                },
            };
            let entry = request.prepare(runtime, &mut execution.call_storage)?;
            let resume = if kind == FunctionKind::Async && !module_link {
                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("async body continuation allocation failed"))?;
                query.parents.push(resume);
                Resume::Async(
                    super::async_function::AsyncResume::start(runtime, realm)
                        .map_err(runtime_error_to_vm_error)?,
                )
            } else {
                resume
            };
            return Ok(Next::Call {
                entry: Box::new(entry),
                pc: 0,
                resume,
            });
        }
    }
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event("native_leaf_completion");
    // Normalization has consumed Bound and Proxy targets; bytecode always
    // installs an explicit child above. Only a classified native leaf can
    // reach this synchronous ABI, never a generic JS-call dispatcher.
    let CallableExecution::Native {
        target,
        realm: defining_realm,
        min_readable_args,
    } = classification
    else {
        return Err(Error::internal(
            "native leaf continuation lost its classification",
        ));
    };
    runtime
        .0
        .state
        .borrow()
        .heap
        .context(realm)
        .map_err(|error| Error::internal(error.to_string()))?;
    // Internal values carry no runtime branding; the slot authentication
    // above already proved every operand owner.
    let completion = if runtime.native_call_would_overflow(target) {
        overflow(runtime, realm)?
    } else {
        let execution_realm = if target.uses_calling_realm() {
            realm
        } else {
            defining_realm
        };
        // The native ABI consumes public roots; the normalized internal owners
        // are rooted at this leaf boundary and released after the call.
        let rooted_receiver = runtime
            .root_value(&receiver)
            .map_err(runtime_error_to_vm_error)?;
        let rooted_arguments = arguments
            .iter()
            .map(|argument| runtime.root_value(argument))
            .collect::<Result<Vec<_>, _>>()
            .map_err(runtime_error_to_vm_error)?;
        let completion = runtime
            .call_native_function(
                &callable,
                execution_realm,
                target,
                min_readable_args,
                rooted_receiver,
                &rooted_arguments,
            )
            .map_err(runtime_error_to_vm_error);
        runtime
            .release_jsvalue(receiver)
            .map_err(runtime_error_to_vm_error)?;
        for argument in arguments {
            runtime
                .release_jsvalue(argument)
                .map_err(runtime_error_to_vm_error)?;
        }
        completion?
    };
    step = resume
        .resume(runtime, completion)
        .map_err(runtime_error_to_vm_error)?;
    *next_step = step;
    Ok(Next::Continue)
}

fn overflow(runtime: &Runtime, realm: crate::engine::heap::ContextId) -> Result<Completion, Error> {
    Ok(Completion::Throw(
        runtime
            .new_native_error_jsvalue(
                realm,
                crate::engine::api::error::NativeErrorKind::Internal,
                "stack overflow",
            )
            .map_err(runtime_error_to_vm_error)?,
    ))
}

#[cfg(test)]
mod native_scope_tests {
    use super::*;
    use crate::engine::api::{Context, ErrorKind};

    #[cfg(feature = "profiling")]
    #[test]
    fn narrow_native_completion_keeps_pc_tail_waiting_and_cleanup_order() {
        use crate::engine::api::profiling::CostProfile;
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        let value = context.eval(r#"(function(){
            var total=0, gets=0, values=0, traps=0, errors=0, map=new Map();
            function tail(x){return Math.min(8,x)}
            for(var i=0;i<8;i++){
                total+=Math.min(3,5);total++;
                if(map.set(i,i)!==map)return 0;
                var arg={get valueOf(){gets++;return function(){values++;map.set('nested',41);return 2}}};
                total+=tail(arg);
                var proxy=new Proxy({},{get(t,k){if(k==='valueOf'){traps++;return function(){return 4}}}});
                total+=Math.max(proxy,1);
                try{Math.min(Symbol());return 0}catch(e){if(e instanceof TypeError)errors++}
                try{Math.max({valueOf(){throw 17}});return 0}catch(e){if(e===17)errors++}
            }
            return total===80&&gets===8&&values===8&&traps===8&&errors===16&&map.get('nested')===41&&tail(2)===2?42:0;
        })()"#).unwrap();
        assert_eq!(value, Value::Int(42));
        let costs = profile.snapshot();
        assert!(
            costs
                .owned_execution_events
                .get("native_call_completed_without_query")
                .copied()
                .unwrap_or(0)
                > 20,
            "{costs:?}"
        );
        assert!(
            costs
                .owned_execution_events
                .get("native_call_direct_wait")
                .copied()
                .unwrap_or(0)
                > 0,
            "{costs:?}"
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn narrow_native_completion_keeps_foreign_error_realm_on_cold_and_warm_calls() {
        let runtime = Runtime::new();
        let mut caller = runtime.new_context();
        let mut defining = runtime.new_context();
        let minimum = defining.eval("Math.min").unwrap();
        let prototype = defining.eval("TypeError.prototype").unwrap();
        let global = caller.global_object().unwrap();
        for (name, value) in [
            ("foreignMinimum", minimum),
            ("foreignErrorPrototype", prototype),
        ] {
            caller
                .set_property(&global, &runtime.intern_property_key(name).unwrap(), value)
                .unwrap();
        }
        drop(defining);
        assert_eq!(caller.eval("(function(){var n=0;function tail(x){return foreignMinimum(x)}for(var i=0;i<8;i++){if(tail(42)!==42)return 0;try{tail(Symbol());return 0}catch(e){if(Object.getPrototypeOf(e)!==foreignErrorPrototype)return 0;n++}}return n===8?42:0})()").unwrap(),Value::Int(42));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn narrow_native_completion_keeps_budget_before_coercion_and_activation() {
        use crate::engine::vm::{
            call::BytecodeCallRequest,
            execution::ExecutionLimits,
            frame::{ReturnTarget, ReturnValue},
        };
        for cached in [false, true] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let parent_callable = runtime
                .callable_from_value(context.eval("(function(){return 1+2})").unwrap())
                .unwrap();
            let callable = runtime
                .callable_from_value(context.eval("Math.min").unwrap())
                .unwrap();
            let argument = context
                .eval("globalThis.nativeBudgetCalls=0;({valueOf(){nativeBudgetCalls++;return 42}})")
                .unwrap();
            let CallableExecution::Native {
                target,
                realm,
                min_readable_args,
            } = runtime.bytecode_for_callable(&callable).unwrap()
            else {
                panic!("native")
            };
            let CallableExecution::Bytecode {
                bytecode,
                closure_slots,
            } = runtime.bytecode_for_callable(&parent_callable).unwrap()
            else {
                panic!("bytecode")
            };
            let mut execution = RunningExecution::new(
                &runtime,
                ExecutionLimits {
                    frames: 1,
                    slots: 32,
                },
            )
            .unwrap();
            let entry = BytecodeCallRequest {
                callable: parent_callable,
                receiver: Value::Undefined,
                new_target: Value::Undefined,
                arguments: Vec::new(),
                bytecode,
                closure_slots,
                caller_realm: context.realm,
                return_to: ReturnTarget {
                    value_use: ReturnValue::Push,
                    owner: ReturnOwner::Root,
                    tail: false,
                    operation: None,
                },
            }
            .prepare(&runtime, &mut execution.call_storage)
            .unwrap();
            let frame = super::super::driver::push_frame(&mut execution, entry).unwrap();
            if cached {
                let query = execution.query_storage.acquire(
                    context.realm,
                    Vec::new(),
                    Finish::Call {
                        depth: 0,
                        tail: false,
                    },
                );
                query.recycle(&mut execution.query_storage);
            }
            let result = start_classified_native_call(
                &runtime,
                &mut execution,
                frame,
                callable,
                target,
                realm,
                min_readable_args,
                Value::Undefined,
                vec![argument],
                false,
                0,
            )
            .unwrap();
            assert!(matches!(result, CallStep::Complete(Completion::Throw(_))));
            let parent = execution.frames.current_mut(frame).unwrap();
            assert_eq!(parent.property_generation, 0);
            assert_eq!(parent.resume_pc, 0);
            assert_eq!(execution.slots.depth(&parent.window), 0);
            assert_eq!(runtime.0.state.borrow().active_frames.len(), 1);
            drop(execution);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
            assert_eq!(context.eval("nativeBudgetCalls").unwrap(), Value::Int(0));
        }
    }

    #[test]
    fn native_local_completion_needs_neither_query_identity_nor_warm_cache() {
        use crate::engine::vm::{
            call::BytecodeCallRequest,
            execution::ExecutionLimits,
            frame::{ReturnTarget, ReturnValue},
        };
        for (name, receiver, mut arguments) in [
            ("Math.min", "undefined", vec![Value::Int(3), Value::Int(2)]),
            ("String", "undefined", vec![Value::Int(42)]),
            (
                "String.prototype.replace",
                "'a'",
                vec![
                    Value::Undefined,
                    Value::String(crate::engine::value::JsString::from_static("b")),
                ],
            ),
            ("Array.prototype.push", "[]", vec![Value::Int(42)]),
            ("Array.prototype.pop", "[42]", Vec::new()),
            (
                "RegExp.prototype.exec",
                "/a/g",
                vec![Value::String(crate::engine::value::JsString::from_static(
                    "a",
                ))],
            ),
            (
                "RegExp.prototype[Symbol.replace]",
                "/a/g",
                vec![
                    Value::String(crate::engine::value::JsString::from_static("a")),
                    Value::String(crate::engine::value::JsString::from_static("b")),
                ],
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let receiver = context.eval(receiver).unwrap();
            if name == "RegExp.prototype[Symbol.replace]" || name == "String.prototype.replace" {
                // The unchanged standard matcher predicate requires a Data
                // native exec. Materialize that lazy property only: do not run
                // replace or warm this execution's Query cache.
                context.eval("RegExp.prototype.exec").unwrap();
            }
            if name == "String.prototype.replace" {
                arguments[0] = context.eval("/a/g").unwrap();
            }
            let callable = runtime
                .callable_from_value(context.eval(name).unwrap())
                .unwrap();
            let CallableExecution::Native {
                target,
                realm,
                min_readable_args,
            } = runtime.bytecode_for_callable(&callable).unwrap()
            else {
                panic!("native")
            };
            let parent = runtime
                .callable_from_value(context.eval("(function(){return 1+2})").unwrap())
                .unwrap();
            let CallableExecution::Bytecode {
                bytecode,
                closure_slots,
            } = runtime.bytecode_for_callable(&parent).unwrap()
            else {
                panic!("bytecode")
            };
            let mut execution = RunningExecution::new(
                &runtime,
                ExecutionLimits {
                    frames: 4,
                    slots: 32,
                },
            )
            .unwrap();
            let entry = BytecodeCallRequest {
                callable: parent,
                receiver: Value::Undefined,
                new_target: Value::Undefined,
                arguments: Vec::new(),
                bytecode,
                closure_slots,
                caller_realm: context.realm,
                return_to: ReturnTarget {
                    value_use: ReturnValue::Push,
                    owner: ReturnOwner::Root,
                    tail: false,
                    operation: None,
                },
            }
            .prepare(&runtime, &mut execution.call_storage)
            .unwrap();
            let frame = super::super::driver::push_frame(&mut execution, entry).unwrap();
            execution
                .frames
                .current_mut(frame)
                .unwrap()
                .property_generation = u64::MAX;
            assert!(!execution.query_storage.has_cached_entry());
            let result = start_classified_native_call(
                &runtime,
                &mut execution,
                frame,
                callable,
                target,
                realm,
                min_readable_args,
                receiver,
                arguments,
                true,
                0,
            )
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
            assert!(
                matches!(result, CallStep::Complete(Completion::Return(_))),
                "{name}"
            );
            assert_eq!(
                execution
                    .frames
                    .current_mut(frame)
                    .unwrap()
                    .property_generation,
                u64::MAX,
                "{name}"
            );
            assert!(!execution.query_storage.has_cached_entry(), "{name}");
            assert_eq!(runtime.0.state.borrow().active_frames.len(), 1);
            drop(execution);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    fn prepare(
        runtime: &Runtime,
        context: &mut Context,
        name: &str,
    ) -> super::super::call::PreparedNativeCall {
        let callable = runtime
            .callable_from_value(context.eval(name).unwrap())
            .unwrap();
        let CallableExecution::Native {
            target,
            realm,
            min_readable_args,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("expected native")
        };
        runtime
            .prepare_native_invocation(
                &callable,
                realm,
                target,
                min_readable_args,
                super::super::call::NativeInvocation::Call {
                    this_value: Value::Undefined,
                },
                &[],
                super::super::call::NativeInvokeMode::Ordinary,
            )
            .unwrap()
    }

    #[test]
    fn nested_native_scope_errors_keep_frames_until_capture_and_restore_parent_realm() {
        let runtime = Runtime::new();
        let mut slots = super::super::stack::SlotStore::new(0);
        let mut caller = runtime.new_context();
        let mut outer = runtime.new_context();
        let mut inner = runtime.new_context();
        let prototype = inner.eval("TypeError.prototype").unwrap();
        let first = prepare(&runtime, &mut outer, "Object.getPrototypeOf");
        let second = prepare(&runtime, &mut inner, "Reflect.setPrototypeOf");
        let mut query = Query {
            #[cfg(feature = "profiling")]
            had_callback: false,
            realm: inner.realm,
            parents: Parents(vec![Resume::Identity]),
            saved_native_depth: 4,
            natives: vec![
                NativeScope {
                    call: first,
                    parents: Parents(vec![Resume::Identity]),
                    resume: Resume::Identity,
                    parent_realm: caller.realm,
                },
                NativeScope {
                    call: second,
                    parents: Parents(vec![Resume::Identity]),
                    resume: Resume::Identity,
                    parent_realm: outer.realm,
                },
            ],
            spare_parents: Vec::with_capacity(2),
            finish: Some(Finish::PropertyRead(0)),
        };
        assert_eq!(query.continuation_depth(), 5);
        let step = query
            .finish_native(
                &runtime,
                &mut slots,
                Err(Error::new(ErrorKind::Type, "nested native failure")),
            )
            .unwrap();
        assert_eq!(query.realm, outer.realm);
        assert_eq!(query.continuation_depth(), 3);
        assert_eq!(runtime.0.state.borrow().active_frames.len(), 1);
        let Step::Complete(Some(Completion::Throw(Value::Object(error)))) = step else {
            panic!("expected captured error")
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap().map(Value::Object),
            Some(prototype)
        );
        let stack = caller
            .get_property(&error, &runtime.intern_property_key("stack").unwrap())
            .unwrap();
        let Value::String(stack) = stack else {
            panic!("expected stack")
        };
        let stack = stack.to_string();
        assert!(stack.contains("getPrototypeOf (native)"), "{stack}");
        assert!(stack.contains("setPrototypeOf (native)"), "{stack}");
        let step = query
            .finish_native(
                &runtime,
                &mut slots,
                Ok(Completion::Throw(Value::Object(error.clone()))),
            )
            .unwrap();
        assert!(
            matches!(step, Step::Complete(Some(Completion::Throw(Value::Object(value)))) if value == error)
        );
        assert_eq!(query.realm, caller.realm);
        assert_eq!(query.continuation_depth(), 1);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}

pub(super) fn start_iterator_read(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    pending: super::iterator_driver::PendingIterator,
    receiver: crate::engine::value::JsValue,
    key: PropertyKey,
) -> Result<CallStep, Error> {
    let receiver = runtime
        .root_and_release_jsvalue(receiver)
        .map_err(runtime_error_to_vm_error)?;
    start_iterator_query(
        runtime,
        execution,
        pending,
        Step::ReadValue {
            receiver: Some(receiver),
            key: Some(key),
            resume: Some(Resume::Identity),
        },
        false,
    )
}
pub(super) fn start_iterator_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    pending: super::iterator_driver::PendingIterator,
    callable: crate::engine::object::CallableRef,
    receiver: crate::engine::value::JsValue,
) -> Result<CallStep, Error> {
    let receiver = runtime
        .root_and_release_jsvalue(receiver)
        .map_err(runtime_error_to_vm_error)?;
    start_iterator_query(
        runtime,
        execution,
        pending,
        Step::Call {
            target: Some(DirectCallTarget::Callable(callable)),
            receiver: Some(receiver),
            arguments: Some(Vec::new()),
            resume: Some(Resume::Identity),
        },
        false,
    )
}
pub(super) fn start_iterator_invoke(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    pending: super::iterator_driver::PendingIterator,
    target: DirectCallTarget,
    receiver: crate::engine::value::JsValue,
    arguments: Vec<crate::engine::value::JsValue>,
) -> Result<CallStep, Error> {
    let receiver = runtime
        .root_and_release_jsvalue(receiver)
        .map_err(runtime_error_to_vm_error)?;
    let arguments = arguments
        .into_iter()
        .map(|argument| runtime.root_and_release_jsvalue(argument))
        .collect::<Result<Vec<_>, _>>()
        .map_err(runtime_error_to_vm_error)?;
    start_iterator_query(
        runtime,
        execution,
        pending,
        Step::Call {
            target: Some(target),
            receiver: Some(receiver),
            arguments: Some(arguments),
            resume: Some(Resume::Identity),
        },
        false,
    )
}
pub(super) fn start_iterator_next(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    pending: super::iterator_driver::PendingIterator,
    iterator: crate::engine::value::JsValue,
    method: crate::engine::object::CallableRef,
) -> Result<CallStep, Error> {
    let crate::engine::value::JsValue::Object(iterator) = iterator else {
        return Err(Error::internal("iterator record lost object receiver"));
    };
    let step = crate::engine::builtins::IteratorNextStep::start_callable(
        runtime,
        pending.realm(),
        crate::engine::object::ObjectRef::from_borrowed_handle(runtime.clone(), iterator)
            .map_err(super::exception::heap_error_to_vm_error)?,
        method,
    )
    .map_err(runtime_error_to_vm_error)?;
    // A read-only discriminator selects the shared Array-next domain, not a
    // workload or an iterator source representation. Full native validation
    // still happens at its original entry boundary.
    let direct = pending.is_next_iteration()
        && execution.query_storage.has_cached_entry()
        && match &step {
            crate::engine::builtins::IteratorNextStep::Call { callable, .. } => {
                let state = runtime.0.state.borrow();
                matches!(state.heap.object(callable.as_object().object_id()).map(|object| &object.payload),
                    Ok(crate::engine::heap::ObjectPayload::NativeFunction { data, .. })
                        if data.target == crate::engine::builtins::native::NativeFunctionId::ArrayIteratorNext)
            }
            _ => false,
        };
    if direct {
        let realm = pending.realm();
        let result = start_array_next_direct(runtime, execution, pending, step);
        return match finish_error(runtime, realm, result)? {
            Progress::Call(step) => Ok(step),
            Progress::Conversion(_) => {
                Err(Error::internal("iterator returned unrelated conversion"))
            }
        };
    }
    start_iterator_query(runtime, execution, pending, step.into(), true)
}

/// Complete a known Array-next before allocating a generic iterator operation.
/// The original iterator record remains live until value/done is committed.
// The no-wait iterator path uses the existing caller facts and operands without allocating a pending request.
#[allow(clippy::too_many_arguments)]
pub(super) fn start_array_next_without_pending(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    record_base: usize,
    callable: crate::engine::object::CallableRef,
    defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    iterator: crate::engine::value::JsValue,
) -> Result<CallStep, Error> {
    use crate::engine::builtins::{IteratorNextResume, ObjectIteratorStep};
    let realm = execution.frames.current_mut(frame)?.executable.realm;
    let result = (|| -> Result<Progress, Error> {
        let result = if !execution.frames.can_push_with_continuations(0)
            || runtime.host_stack_would_overflow()
        {
            ObjectIteratorStep::Throw(match overflow(runtime, realm)? {
                Completion::Throw(value) => runtime
                    .root_and_release_jsvalue(value)
                    .map_err(runtime_error_to_vm_error)?,
                _ => return Err(Error::internal("iterator overflow did not throw")),
            })
        } else {
            let mut waiting =
                Step::Complete(Some(Completion::Return(crate::engine::value::JsValue::Undefined)));
            let mut waiting_call = None;
            let result = native::compact_array_next_into(
                runtime,
                &mut execution.slots,
                realm,
                callable,
                defining_realm,
                min_readable_args,
                runtime
                    .root_and_release_jsvalue(iterator)
                    .map_err(runtime_error_to_vm_error)?,
                &mut waiting,
                &mut waiting_call,
            )?;
            let resume = IteratorNextResume::for_raw(realm);
            let Some(result) = result else {
                let identity = iterator_query_identity(execution, frame)?;
                let pending = super::iterator_driver::next_wait(execution, frame, record_base)?;
                let mut query = execution
                    .query_storage
                    .acquire(realm, Vec::new(), Finish::Root);
                storage::reserve(&mut query.natives, 1, "query.native_scopes")
                    .map_err(|_| Error::internal("native continuation allocation failed"))?;
                storage::reserve(&mut query.spare_parents, 1, "query.spare_parents")
                    .map_err(|_| Error::internal("native parent storage allocation failed"))?;
                let waiting_call = waiting_call
                    .ok_or_else(|| Error::internal("Array-next wait lost activation"))?;
                query.finish = Some(install_iterator_finish(execution, pending, true)?);
                native::install_waiting(&mut query, waiting_call, Resume::IteratorNext(resume));
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_execution_event(
                    "iterator_native_direct_wait",
                );
                return drive(
                    runtime,
                    execution,
                    ReturnOwner::Frame(frame),
                    identity,
                    query,
                    Ok(waiting),
                );
            };
            resume
                .raw_completion(result)
                .map_err(runtime_error_to_vm_error)?
                .map_err(|_| Error::internal("Array-next returned an ordinary result object"))?
        };
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event(
            "iterator_native_completed_without_query",
        );
        let (value, done, abrupt) = match result {
            ObjectIteratorStep::Yield(value) => (
                runtime
                    .into_jsvalue(value)
                    .map_err(runtime_error_to_vm_error)?,
                false,
                None,
            ),
            ObjectIteratorStep::Done => (crate::engine::value::JsValue::Undefined, true, None),
            ObjectIteratorStep::Throw(value) => (
                crate::engine::value::JsValue::Undefined,
                false,
                Some(runtime.into_jsvalue(value).map_err(runtime_error_to_vm_error)?),
            ),
        };
        super::iterator_driver::finish_next(
            runtime, execution, frame, record_base, value, done, abrupt,
        )
        .map(Progress::Call)
    })();
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("Array-next returned unrelated conversion")),
    }
}

#[inline(never)]
fn start_array_next_direct(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    mut pending: super::iterator_driver::PendingIterator,
    step: crate::engine::builtins::IteratorNextStep,
) -> Result<Progress, Error> {
    use crate::engine::builtins::{IteratorNextStep, ObjectIteratorStep};
    let IteratorNextStep::Call {
        callable,
        iterator,
        resume,
    } = step
    else {
        return Err(Error::internal("direct next lost its callable phase"));
    };
    let frame = pending.frame();
    let realm = pending.realm();
    let identity = iterator_query_identity(execution, frame)?;
    let Some((target, defining_realm, min_readable_args)) = runtime
        .direct_native_callable_metadata(&callable)
        .map_err(runtime_error_to_vm_error)?
    else {
        return Err(Error::internal("direct Array-next lost native metadata"));
    };
    let completion = if !execution.frames.can_push_with_continuations(0)
        || runtime.host_stack_would_overflow()
    {
        // Budget failures are parsed by the same iterator continuation and
        // disable the owned iterator region through PendingIterator::finish.
        match overflow(runtime, realm)? {
            Completion::Throw(value) => ObjectIteratorStep::Throw(
                runtime
                    .root_and_release_jsvalue(value)
                    .map_err(runtime_error_to_vm_error)?,
            ),
            _ => return Err(Error::internal("iterator overflow did not throw")),
        }
    } else {
        if !execution.query_storage.reserve_cached_native_entry()? {
            return Err(Error::internal("direct native entry lost reserved storage"));
        }
        let mut waiting =
            Step::Complete(Some(Completion::Return(crate::engine::value::JsValue::Undefined)));
        let mut waiting_call = None;
        let result = native::begin_into(
            runtime,
            &mut execution.slots,
            realm,
            callable,
            target,
            defining_realm,
            min_readable_args,
            super::call::NativeInvokeMode::IteratorNextRaw,
            super::call::NativeInvocation::Call {
                this_value: Value::Object(iterator),
            },
            Vec::new(),
            crate::engine::builtins::continuation::NativeOperation::ArrayNext,
            &mut waiting,
            &mut waiting_call,
        )?;
        let Some(result) = result else {
            // The first native step may already have advanced the iterator.
            // Install exactly that activation and selected wait, never restart.
            let finish = install_iterator_finish(execution, pending, true)?;
            let mut query = execution.query_storage.acquire(realm, Vec::new(), finish);
            native::install_waiting(
                &mut query,
                waiting_call.expect("native wait has an activation"),
                Resume::IteratorNext(resume),
            );
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "iterator_native_direct_wait",
            );
            return drive(
                runtime,
                execution,
                ReturnOwner::Frame(frame),
                identity,
                query,
                Ok(waiting),
            );
        };
        match resume
            .raw_completion(result)
            .map_err(runtime_error_to_vm_error)?
        {
            Ok(result) => result,
            Err(_) => {
                return Err(Error::internal(
                    "Array-next returned an ordinary result object",
                ));
            }
        }
    };
    let action = pending.next_query(runtime, completion)?;
    if !matches!(action, super::iterator_driver::IteratorAction::Finish) {
        return Err(Error::internal(
            "direct ForOfNext did not finish its instruction",
        ));
    }
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event(
        "iterator_native_completed_without_query",
    );
    super::iterator_driver::finish(runtime, execution, pending).map(Progress::Call)
}

fn iterator_query_identity(execution: &mut RunningExecution, frame: FrameId) -> Result<u64, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("iterator query identity exhausted"))?;
    parent.property_generation = identity;
    Ok(identity)
}
/// Query completion carries only identity. The frame's reusable cold record
/// owns iterator state across both local steps and actual callback suspension.
fn install_iterator_finish(
    execution: &mut RunningExecution,
    pending: super::iterator_driver::PendingIterator,
    next: bool,
) -> Result<Finish, Error> {
    let id = pending.frame();
    let frame = execution.frames.current_mut(id)?;
    if frame.cold.iterator_wait.is_some() {
        return Err(Error::internal("iterator resident record already occupied"));
    }
    frame.cold.iterator_wait = Some(pending);
    Ok(if next {
        Finish::IteratorNext(id)
    } else {
        Finish::Iterator(id)
    })
}
fn take_iterator_finish(
    execution: &mut RunningExecution,
    id: FrameId,
) -> Result<super::iterator_driver::PendingIterator, Error> {
    execution
        .frames
        .current_mut(id)?
        .cold
        .iterator_wait
        .take()
        .ok_or_else(|| Error::internal("iterator resident record missing"))
}

fn start_iterator_query(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    pending: super::iterator_driver::PendingIterator,
    step: Step,
    next: bool,
) -> Result<CallStep, Error> {
    let frame = pending.frame();
    let realm = pending.realm();
    let identity = iterator_query_identity(execution, frame)?;
    let finish = if next {
        install_iterator_finish(execution, pending, true)?
    } else {
        install_iterator_finish(execution, pending, false)?
    };
    let result = advance(
        runtime,
        execution,
        frame,
        identity,
        Vec::new(),
        step,
        finish,
    );
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("iterator returned unrelated conversion")),
    }
}
pub(super) fn start_instance(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    candidate: Value,
    target: ObjectRef,
    depth: usize,
) -> Result<CallStep, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let realm = parent.executable.realm;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("instance query identity exhausted"))?;
    parent.property_generation = identity;
    let result = (|| {
        let step = crate::engine::builtins::InstanceStep::start(runtime, realm, candidate, target)
            .map_err(runtime_error_to_vm_error)?;
        advance(
            runtime,
            execution,
            frame,
            identity,
            Vec::new(),
            step.into(),
            Finish::Call { depth, tail: false },
        )
    })();
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("instanceof returned conversion")),
    }
}

pub(super) fn start_object_copy(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    target_depth: usize,
    source_depth: usize,
    excluded_depth: Option<usize>,
) -> Result<CallStep, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let realm = parent.executable.realm;
    // The copy machine borrows rooted copies; the slot owners stay live until
    // the pops below consume them.
    let target = runtime
        .root_value(execution.slots.peek(&parent.window, target_depth)?)
        .map_err(runtime_error_to_vm_error)?;
    let Value::Object(target) = target else {
        return Err(Error::internal(
            "CopyDataProperties target is not an object",
        ));
    };
    let source = runtime
        .root_value(execution.slots.peek(&parent.window, source_depth)?)
        .map_err(runtime_error_to_vm_error)?;
    let excluded = if let Some(depth) = excluded_depth {
        let excluded = runtime
            .root_value(execution.slots.peek(&parent.window, depth)?)
            .map_err(runtime_error_to_vm_error)?;
        let Value::Object(object) = excluded else {
            return Err(Error::internal(
                "CopyDataProperties exclusion is not an object",
            ));
        };
        Some(object)
    } else {
        None
    };
    let step = crate::engine::builtins::ObjectCopyStep::start(runtime, target, source, excluded)
        .map_err(runtime_error_to_vm_error)?;
    let depth = execution.slots.depth(&parent.window);
    // Computing the next identity is pure. Only a selected wait publishes it.
    let identity = parent.property_generation.checked_add(1);
    let mut rejected_source = None;
    if excluded_depth.is_none() {
        let source = execution.slots.pop(&mut parent.window)?;
        if identity.is_none() {
            // At exhaustion retain this owner only long enough to restore the
            // old failure input if a real wait is selected.
            rejected_source = Some(source);
        } else {
            // The normal source owner releases before any copy effects, as before.
            runtime
                .release_jsvalue(source)
                .map_err(runtime_error_to_vm_error)?;
        }
    }
    let result = (|| {
        let step = step
            .advance_without_callback(runtime)
            .map_err(runtime_error_to_vm_error)?;
        if let crate::engine::builtins::ObjectCopyStep::Complete(completion) = step {
            if let Some(source) = rejected_source.take() {
                runtime
                    .release_jsvalue(source)
                    .map_err(runtime_error_to_vm_error)?;
            }
            return finish_instruction_call(
                execution,
                ReturnOwner::Frame(frame),
                completion,
                false,
                depth,
            )
            .map(Progress::Call);
        }
        let Some(identity) = identity else {
            if let Some(source) = rejected_source.take() {
                let parent = execution.frames.current_mut(frame)?;
                execution.slots.push(&mut parent.window, source)?;
            }
            // Earlier local definitions remain on the fresh target; undoing
            // them or pre-reading all values would change copy semantics.
            // The selected getter/Proxy request has never been executed.
            return Err(Error::internal("copy query identity exhausted"));
        };
        execution.frames.current_mut(frame)?.property_generation = identity;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("copy_wait_handoff");
        advance(
            runtime,
            execution,
            frame,
            identity,
            Vec::new(),
            step.into(),
            Finish::Discard(depth),
        )
    })();
    drop(rejected_source);
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("object copy returned conversion")),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_vm_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    callable: crate::engine::object::CallableRef,
    receiver: Value,
    arguments: Vec<Value>,
    value_use: ReturnValue,
) -> Result<CallStep, Error> {
    match start_owned_callback(
        runtime,
        execution,
        frame,
        callable,
        receiver,
        arguments,
        Finish::VmCall(value_use),
    )? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("VM callback returned a conversion")),
    }
}

pub(super) fn start_environment(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    step: super::environment_bindings::operation::EnvironmentStep,
    value_use: ReturnValue,
    depth: usize,
) -> Result<CallStep, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let realm = parent.executable.realm;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("environment query identity exhausted"))?;
    parent.property_generation = identity;
    let finish = match value_use {
        ReturnValue::Push => Finish::PropertyRead(depth),
        ReturnValue::Discard => Finish::Discard(depth),
    };
    let result = advance(
        runtime,
        execution,
        frame,
        identity,
        Vec::new(),
        step.into(),
        finish,
    );
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("environment returned a conversion")),
    }
}

enum IteratorProgress {
    Step(Step),
    Done(CallStep),
}
fn continue_iterator(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    query: &mut Query,
    pending: super::iterator_driver::PendingIterator,
    action: super::iterator_driver::IteratorAction,
) -> Result<IteratorProgress, Error> {
    use super::iterator_driver::IteratorAction;
    let (step, next) = match action {
        IteratorAction::Finish => {
            return super::iterator_driver::finish(runtime, execution, pending)
                .map(IteratorProgress::Done);
        }
        IteratorAction::Read(receiver, key) => (
            Step::ReadValue {
                receiver: Some(
                    runtime
                        .root_and_release_jsvalue(receiver)
                        .map_err(runtime_error_to_vm_error)?,
                ),
                key: Some(key),
                resume: Some(Resume::Identity),
            },
            false,
        ),
        IteratorAction::Call(callable, receiver) => (
            Step::Call {
                target: Some(DirectCallTarget::Callable(callable)),
                receiver: Some(
                    runtime
                        .root_and_release_jsvalue(receiver)
                        .map_err(runtime_error_to_vm_error)?,
                ),
                arguments: Some(Vec::new()),
                resume: Some(Resume::Identity),
            },
            false,
        ),
        IteratorAction::Invoke(target, receiver, arguments) => (
            Step::Call {
                target: Some(target),
                receiver: Some(
                    runtime
                        .root_and_release_jsvalue(receiver)
                        .map_err(runtime_error_to_vm_error)?,
                ),
                arguments: Some(
                    arguments
                        .into_iter()
                        .map(|argument| runtime.root_and_release_jsvalue(argument))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(runtime_error_to_vm_error)?,
                ),
                resume: Some(Resume::Identity),
            },
            false,
        ),
        IteratorAction::Next(callable, receiver) => {
            let JsValue::Object(iterator) = receiver else {
                return Err(Error::internal("iterator record lost object receiver"));
            };
            (
                crate::engine::builtins::IteratorNextStep::start_callable(
                    runtime,
                    pending.realm(),
                    ObjectRef::from_borrowed_handle(runtime.clone(), iterator)
                        .map_err(super::exception::heap_error_to_vm_error)?,
                    callable,
                )
                .map_err(runtime_error_to_vm_error)?
                .into(),
                true,
            )
        }
    };
    query.finish = Some(if next {
        install_iterator_finish(execution, pending, true)?
    } else {
        install_iterator_finish(execution, pending, false)?
    });
    Ok(IteratorProgress::Step(step))
}

pub(super) fn start_numeric(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    step: super::numeric::operation::NumericStep,
    depth: usize,
) -> Result<super::frame_operations::NumericProgress, Error> {
    use super::frame_operations::NumericProgress;
    let parent = execution.frames.current_mut(frame)?;
    let realm = parent.executable.realm;
    let step = match step {
        super::numeric::operation::NumericStep::Complete { value, previous } => {
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "numeric_completed_without_query",
            );
            return match finish_numeric(runtime, execution, frame, value, previous, depth)? {
                CallStep::Entered => Ok(NumericProgress::Completed),
                _ => Err(Error::internal(
                    "immediate numeric completion changed its frame protocol",
                )),
            };
        }
        super::numeric::operation::NumericStep::Throw(value) => {
            return Ok(NumericProgress::Deferred(CallStep::Complete(
                Completion::Throw(value),
            )));
        }
        step => step,
    };
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("numeric query identity exhausted"))?;
    parent.property_generation = identity;
    let result = advance(
        runtime,
        execution,
        frame,
        identity,
        Vec::new(),
        step.into(),
        Finish::Numeric(depth),
    );
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(NumericProgress::Deferred(step)),
        Progress::Conversion(_) => Err(Error::internal(
            "numeric operation returned conversion task",
        )),
    }
}

pub(super) fn start_class_parent(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    pending: Box<super::construct_driver::PendingClass>,
    parent: ObjectRef,
    realm: crate::engine::heap::ContextId,
    frame: FrameId,
) -> Result<CallStep, Error> {
    let step = Step::ReadValue {
        receiver: Some(Value::Object(parent)),
        key: Some(
            runtime
                .intern_property_key("prototype")
                .map_err(|error| Error::internal(error.to_string()))?,
        ),
        resume: Some(Resume::Identity),
    };
    start_instruction_query(
        runtime,
        execution,
        frame,
        realm,
        step,
        Finish::Class(pending),
    )
}
pub(super) fn start_public_field(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    object: ObjectRef,
    key: PropertyKey,
    value: crate::engine::value::JsValue,
    depth: usize,
) -> Result<CallStep, Error> {
    let realm = execution.frames.current_mut(frame)?.executable.realm;
    let step = Step::Define {
        object: Some(object),
        key: Some(key),
        descriptor: Some(Runtime::public_class_field_descriptor(
            runtime
                .root_and_release_jsvalue(value)
                .map_err(runtime_error_to_vm_error)?,
        )),
        resume: Some(Resume::PublicField),
    };
    start_instruction_query(
        runtime,
        execution,
        frame,
        realm,
        step,
        Finish::Discard(depth),
    )
}
fn start_instruction_query(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    realm: crate::engine::heap::ContextId,
    step: Step,
    finish: Finish,
) -> Result<CallStep, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let identity = parent
        .property_generation
        .checked_add(1)
        .ok_or_else(|| Error::internal("instruction query identity exhausted"))?;
    parent.property_generation = identity;
    let result = advance(
        runtime,
        execution,
        frame,
        identity,
        Vec::new(),
        step,
        finish,
    );
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("instruction returned a conversion task")),
    }
}

pub(super) fn start_for_in_query(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    step: super::for_in::operation::ForInStep,
    depth: usize,
) -> Result<CallStep, Error> {
    use super::for_in::operation::ForInStep;
    let parent = execution.frames.current_mut(frame)?;
    let realm = parent.executable.realm;
    // Empty Query acquisition has no budget check. Local non-Proxy steps do
    // not push parents; the first waiting Proxy keeps its original admission
    // check and effect order in advance/dispatch, with continuation depth zero.
    let result = (|| match step
        .advance_without_callback(runtime)
        .map_err(runtime_error_to_vm_error)?
    {
        ForInStep::Complete { value, done } => {
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "for_in_completed_without_query",
            );
            // The for-in machine is a public-root consumer; its completion
            // owner transfers into the internal value without a counting pair.
            let value = runtime
                .into_jsvalue(value)
                .map_err(runtime_error_to_vm_error)?;
            finish_for_in(execution, frame, value, done, depth)
        }
        ForInStep::Throw(value) => {
            let value = runtime
                .into_jsvalue(value)
                .map_err(runtime_error_to_vm_error)?;
            Ok(CallStep::Complete(Completion::Throw(value)))
        }
        step => {
            let parent = execution.frames.current_mut(frame)?;
            let identity = parent
                .property_generation
                .checked_add(1)
                .ok_or_else(|| Error::internal("instruction query identity exhausted"))?;
            parent.property_generation = identity;
            start_for_in_pending(runtime, execution, frame, identity, step, depth)
        }
    })();
    match result {
        Ok(step) => Ok(step),
        Err(error) => super::property_driver::throw_error(runtime, realm, error),
    }
}

// Keep the wide query transport and its selected Proxy step off the local path.
#[inline(never)]
fn start_for_in_pending(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    identity: u64,
    step: super::for_in::operation::ForInStep,
    depth: usize,
) -> Result<CallStep, Error> {
    match advance(
        runtime,
        execution,
        frame,
        identity,
        Vec::new(),
        step.into(),
        Finish::ForIn(depth),
    )? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal("instruction returned a conversion task")),
    }
}

fn finish_for_in(
    execution: &mut RunningExecution,
    frame: FrameId,
    value: crate::engine::value::JsValue,
    done: Option<bool>,
    _depth: usize,
) -> Result<CallStep, Error> {
    let parent = execution.frames.current_mut(frame)?;
    execution.slots.push(&mut parent.window, value)?;
    if let Some(done) = done {
        execution
            .slots
            .push(&mut parent.window, crate::engine::value::JsValue::Bool(done))?;
    }
    parent.resume_pc = parent
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("for-in resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(_depth);
    Ok(CallStep::Entered)
}

pub(super) fn start_literal_definition(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    step: crate::engine::object::object_literal::element::LiteralDefinitionStep,
    depth: usize,
) -> Result<CallStep, Error> {
    let realm = execution.frames.current_mut(frame)?.executable.realm;
    start_instruction_query(
        runtime,
        execution,
        frame,
        realm,
        step.into(),
        Finish::Discard(depth),
    )
}

pub(super) fn start_import(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
) -> Result<CallStep, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let realm = parent.executable.realm;
    parent
        .executable
        .ensure_root(runtime)
        .map_err(runtime_error_to_vm_error)?;
    // The import machine borrows rooted copies; `start_instruction` consumes
    // the two slot owners.
    let options = runtime
        .root_value(execution.slots.peek(&parent.window, 0)?)
        .map_err(runtime_error_to_vm_error)?;
    let specifier = runtime
        .root_value(execution.slots.peek(&parent.window, 1)?)
        .map_err(runtime_error_to_vm_error)?;
    let result = crate::engine::modules::import::ImportStep::start(
        runtime,
        realm,
        parent.executable.root(),
        specifier,
        options,
    )
    .map_err(runtime_error_to_vm_error)
    .and_then(|step| start_instruction(runtime, execution, frame, step.into(), 2));
    match finish_error(runtime, realm, result)? {
        Progress::Call(step) => Ok(step),
        Progress::Conversion(_) => Err(Error::internal(
            "dynamic import returned an untyped conversion",
        )),
    }
}

#[cfg(test)]
mod write_completion_tests {
    use super::*;

    #[test]
    fn write_completion_runs_following_opcode_once_across_local_waiting_and_throw() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"
            (() => {
                let trace = '', calls = 0;
                const target = {x: 0}, marker = {};
                const setter = {set x(v) { calls++; target.x = v; }};
                const proxy = new Proxy({}, {set() { calls++; throw marker; }});
                const frozen = Object.freeze({x: 1});
                for (let i = 0; i < 3; i++) {
                    target.x = i; trace += 'a';
                    setter.x = i + 1; trace += 'b';
                    try { proxy.x = i; trace += '?'; }
                    catch (e) { trace += e === marker ? 'c' : '?'; }
                    try { (function() { 'use strict'; frozen.x = i; })(); trace += '?'; }
                    catch (e) { trace += e instanceof TypeError ? 'd' : '?'; }
                }
                return trace === 'abcdabcdabcd' && calls === 6 && target.x === 3;
            })()
        "#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}

#[cfg(test)]
mod iterator_resident_layout_tests {
    #[test]
    fn iterator_finish_keeps_the_shared_query_small() {
        // A resident iterator must not inflate every property's/native call's
        // completion enum with its operation-specific state.
        assert!(
            size_of::<super::Finish>()
                < size_of::<super::super::iterator_driver::PendingIteratorState>()
        );
        assert!(size_of::<super::super::iterator_driver::PendingIterator>() <= 8);
        println!(
            "Finish={} Query={} PendingIterator={} FrameRare={}",
            size_of::<super::Finish>(),
            size_of::<super::Query>(),
            size_of::<super::super::iterator_driver::PendingIterator>(),
            size_of::<super::super::frame::FrameRare>()
        );
    }
}
