use super::*;

/// ECMAScript-visible lifecycle of a branded synchronous generator object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GeneratorState {
    SuspendedStart,
    SuspendedYield,
    SuspendedYieldStar,
    Executing,
    Completed,
}

/// Heap-native representation of one argument or local binding retained by a
/// dormant generator frame. Runtime-owning root wrappers must never enter this
/// structure: every GC identity is stored as a raw arena edge instead.
#[derive(Clone, Debug, PartialEq)]
pub enum GeneratorFrameBinding {
    Direct(RawValue),
    Private(Atom),
    PrivateCallable(ObjectId),
    Uninitialized,
    Captured(VarRefId),
}

/// Raw VM fields retained across a synchronous-generator suspension.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratorVmActivation {
    pub stack: Vec<RawValue>,
    pub regions: Vec<crate::engine::vm::VmUnwindRegion>,
    pub pc: usize,
    pub callee_realm: ContextId,
    pub current_function: ObjectId,
    pub this_value: RawValue,
    pub normalized_this: Option<RawValue>,
    pub new_target: RawValue,
    pub strict: bool,
    pub callee_global: ObjectId,
}

/// Complete dormant execution state owned by one generator object.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratorActivationData {
    pub bytecode: FunctionBytecodeId,
    pub vm: GeneratorVmActivation,
    pub actual_argument_count: usize,
    pub arguments: Vec<GeneratorFrameBinding>,
    pub locals: Vec<GeneratorFrameBinding>,
    pub reusable_captured_locals: Vec<bool>,
}

/// ECMAScript-visible lifecycle of a branded async-generator object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AsyncGeneratorState {
    SuspendedStart,
    SuspendedYield,
    SuspendedYieldStar,
    Executing,
    AwaitingReturn,
    Completed,
}

/// One queued `.next`, `.return`, or `.throw` request and its Promise
/// capability. Every identity is stored as a raw traced edge.
#[derive(Clone, Debug, PartialEq)]
pub struct AsyncGeneratorRequestData {
    pub completion: GeneratorResumeKind,
    pub result: RawValue,
    pub promise: ObjectId,
    pub resolve: ObjectId,
    pub reject: ObjectId,
}

/// Complete hidden state of one genuine AsyncGenerator.
#[derive(Clone, Debug, PartialEq)]
pub struct AsyncGeneratorData {
    pub state: AsyncGeneratorState,
    pub activation: Option<Box<GeneratorActivationData>>,
    pub queue: VecDeque<AsyncGeneratorRequestData>,
    /// Realm whose intrinsic Promise machinery owns the currently installed
    /// await/return reaction callbacks.
    pub resume_realm: Option<ContextId>,
}

/// Settlement branch selected by an internal async-function resume callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AsyncFunctionResumeKind {
    Fulfill,
    Reject,
}

/// Settlement branch selected by an internal async-generator reaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AsyncGeneratorResumeKind {
    AwaitFulfill,
    AwaitReject,
    ReturnFulfill,
    ReturnReject,
}

/// Heap-visible lifecycle of one async-function driver.
///
/// The active VM frame is rooted by the runtime while `Executing`. At an
/// `await`, ownership transfers into the state object and the phase becomes
/// `Awaiting`; `Completed` is absorbing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AsyncFunctionPhase {
    Executing,
    Awaiting,
    Completed,
}

/// Hidden driver state shared by the two callbacks installed for one `await`.
///
/// QuickJS keeps this as a separate GC object so the pending Promise reaction
/// owns the dormant activation without exposing it through authored
/// properties. `driver_realm` is the original caller realm which supplies the
/// returned Promise and await jobs; it may differ from the bytecode activation
/// realm. Every arena identity here is a raw, traced edge.
#[derive(Clone, Debug, PartialEq)]
pub struct AsyncFunctionStateData {
    pub driver_realm: ContextId,
    pub outer_resolve: ObjectId,
    pub outer_reject: ObjectId,
    pub activation: Option<Box<GeneratorActivationData>>,
    pub phase: AsyncFunctionPhase,
}

pub(in crate::engine::heap) fn validate_async_function_state(
    heap: &Heap,
    object: &ObjectData,
    data: &AsyncFunctionStateData,
) -> Result<(), HeapError> {
    if object.is_constructor
        || (data.phase == AsyncFunctionPhase::Awaiting) != data.activation.is_some()
    {
        return Err(HeapError::Invariant(
            "AsyncFunction state has inconsistent phase and activation",
        ));
    }
    heap.context(data.driver_realm)?;
    for resolving_function in [data.outer_resolve, data.outer_reject] {
        if !object_data_is_callable(heap.object(resolving_function)?) {
            return Err(HeapError::Invariant(
                "AsyncFunction state retains a non-callable resolving function",
            ));
        }
    }

    let Some(activation) = data.activation.as_deref() else {
        return Ok(());
    };
    let bytecode = heap.function_bytecode(activation.bytecode)?;
    let vm = &activation.vm;
    let function = heap.object(vm.current_function)?;
    if bytecode.metadata.function_kind != FunctionKind::Async
        || bytecode.metadata.constructor_kind != ConstructorKind::None
        || bytecode.metadata.has_prototype
        || bytecode.metadata.class_initializer_kind.is_some()
        || bytecode.realm != vm.callee_realm
        || vm.strict != bytecode.metadata.strict
        || heap.context(vm.callee_realm)?.global_object != vm.callee_global
        || !matches!(
            function.payload,
            ObjectPayload::BytecodeFunction {
                bytecode: owner,
                ..
            } if owner == activation.bytecode
        )
        || function.is_constructor
        || activation.arguments.len() < usize::from(bytecode.metadata.argument_count)
        || activation.actual_argument_count > activation.arguments.len()
        || activation.locals.len() != usize::from(bytecode.metadata.local_count)
        || activation.reusable_captured_locals.len() != activation.locals.len()
        || vm.stack.len() > usize::from(bytecode.metadata.max_stack)
        || vm.pc == 0
        || vm.pc > bytecode.code.len()
    {
        return Err(HeapError::Invariant(
            "AsyncFunction activation has invalid frame metadata",
        ));
    }
    if !matches!(bytecode.code.get(vm.pc - 1), Some(Instruction::Await)) {
        return Err(HeapError::Invariant(
            "AsyncFunction activation is not parked after its await opcode",
        ));
    }

    for value in vm
        .stack
        .iter()
        .chain(std::iter::once(&vm.this_value))
        .chain(vm.normalized_this.iter())
        .chain(std::iter::once(&vm.new_target))
    {
        if !is_map_storable_value(value) {
            return Err(HeapError::Invariant(
                "AsyncFunction activation contains an internal-only value",
            ));
        }
    }
    for binding in activation.arguments.iter().chain(activation.locals.iter()) {
        match binding {
            GeneratorFrameBinding::Direct(value) if !is_map_storable_value(value) => {
                return Err(HeapError::Invariant(
                    "AsyncFunction frame binding contains an internal-only value",
                ));
            }
            GeneratorFrameBinding::Private(atom) if atom.is_null() => {
                return Err(HeapError::Invariant(
                    "AsyncFunction private binding contains the null atom",
                ));
            }
            GeneratorFrameBinding::PrivateCallable(callable) => {
                if !object_data_is_callable(heap.object(*callable)?) {
                    return Err(HeapError::Invariant(
                        "AsyncFunction private callable binding is not callable",
                    ));
                }
            }
            GeneratorFrameBinding::Captured(var_ref) => {
                heap.var_ref(*var_ref)?;
            }
            GeneratorFrameBinding::Direct(_)
            | GeneratorFrameBinding::Private(_)
            | GeneratorFrameBinding::Uninitialized => {}
        }
    }
    for (region_index, region) in vm.regions.iter().enumerate() {
        match *region {
            crate::engine::vm::VmUnwindRegion::Catch {
                target,
                stack_depth,
            } if target >= bytecode.code.len() || stack_depth > vm.stack.len() => {
                return Err(HeapError::Invariant(
                    "AsyncFunction catch region is outside its saved frame",
                ));
            }
            crate::engine::vm::VmUnwindRegion::Iterator { record_base, .. }
                if record_base.saturating_add(1) >= vm.stack.len() =>
            {
                return Err(HeapError::Invariant(
                    "AsyncFunction iterator region is outside its saved frame",
                ));
            }
            crate::engine::vm::VmUnwindRegion::Iterator {
                record_base,
                enabled: false,
                asynchronous: false,
            } if !matches!(vm.stack.get(record_base), Some(RawValue::Undefined)) => {
                return Err(HeapError::Invariant(
                    "AsyncFunction activation contains an invalid completed iterator",
                ));
            }
            crate::engine::vm::VmUnwindRegion::Iterator {
                record_base,
                enabled: false,
                asynchronous,
                ..
            } if asynchronous
                && (region_index + 1 != vm.regions.len()
                    || record_base.checked_add(3) != Some(vm.stack.len())
                    || !matches!(vm.stack.last(), Some(RawValue::Undefined))
                    || !matches!(
                        bytecode.code.get(vm.pc),
                        Some(Instruction::IteratorGetValueDone)
                    )) =>
            {
                return Err(HeapError::Invariant(
                    "AsyncFunction activation contains an invalid pending iterator state",
                ));
            }
            crate::engine::vm::VmUnwindRegion::Catch { .. }
            | crate::engine::vm::VmUnwindRegion::Iterator { .. } => {}
        }
    }
    Ok(())
}

pub(in crate::engine::heap) fn validate_async_generator_state(
    heap: &Heap,
    object: &ObjectData,
    data: &AsyncGeneratorData,
) -> Result<(), HeapError> {
    let state_shape_is_valid = match data.state {
        AsyncGeneratorState::SuspendedStart
        | AsyncGeneratorState::SuspendedYield
        | AsyncGeneratorState::SuspendedYieldStar => {
            data.activation.is_some() && data.resume_realm.is_none()
        }
        AsyncGeneratorState::Executing => {
            (data.activation.is_none() && data.resume_realm.is_none())
                || (data.resume_realm.is_some()
                    && data.activation.as_deref().is_some_and(|activation| {
                        matches!(
                            heap.function_bytecode(activation.bytecode),
                            Ok(bytecode)
                                if matches!(
                                    bytecode.code.get(activation.vm.pc.saturating_sub(1)),
                                    Some(Instruction::Await)
                                )
                        )
                    }))
        }
        AsyncGeneratorState::AwaitingReturn => {
            data.activation.is_none()
                && data.resume_realm.is_some()
                && data
                    .queue
                    .front()
                    .is_some_and(|request| request.completion == GeneratorResumeKind::Return)
        }
        AsyncGeneratorState::Completed => data.activation.is_none() && data.resume_realm.is_none(),
    };
    if object.is_constructor
        || !state_shape_is_valid
        || matches!(
            data.state,
            AsyncGeneratorState::Executing | AsyncGeneratorState::AwaitingReturn
        ) && data.queue.is_empty()
    {
        return Err(HeapError::Invariant(
            "AsyncGenerator has inconsistent state, activation, or queue",
        ));
    }
    if let Some(realm) = data.resume_realm {
        heap.context(realm)?;
    }
    for request in &data.queue {
        if !is_promise_storable_value(&request.result)
            || !matches!(
                heap.object(request.promise)?.payload,
                ObjectPayload::Promise(_)
            )
        {
            return Err(HeapError::Invariant(
                "AsyncGenerator request retains an invalid value or Promise",
            ));
        }
        let ObjectPayload::NativeFunction {
            data: resolve_data,
            internal:
                Some(InternalCallableData::PromiseResolving {
                    promise: resolve_promise,
                    already_resolved: resolve_cell,
                    kind: PromiseResolvingKind::Resolve,
                }),
        } = &heap.object(request.resolve)?.payload
        else {
            return Err(HeapError::Invariant(
                "AsyncGenerator request retains an invalid resolve function",
            ));
        };
        let ObjectPayload::NativeFunction {
            data: reject_data,
            internal:
                Some(InternalCallableData::PromiseResolving {
                    promise: reject_promise,
                    already_resolved: reject_cell,
                    kind: PromiseResolvingKind::Reject,
                }),
        } = &heap.object(request.reject)?.payload
        else {
            return Err(HeapError::Invariant(
                "AsyncGenerator request retains an invalid reject function",
            ));
        };
        if resolve_data.target != NativeFunctionId::PromiseResolving(PromiseResolvingKind::Resolve)
            || reject_data.target
                != NativeFunctionId::PromiseResolving(PromiseResolvingKind::Reject)
            || *resolve_promise != request.promise
            || *reject_promise != request.promise
            || !Rc::ptr_eq(resolve_cell, reject_cell)
        {
            return Err(HeapError::Invariant(
                "AsyncGenerator request capability does not resolve its Promise",
            ));
        }
    }

    let Some(activation) = data.activation.as_deref() else {
        return Ok(());
    };
    let bytecode = heap.function_bytecode(activation.bytecode)?;
    let vm = &activation.vm;
    let function = heap.object(vm.current_function)?;
    if bytecode.metadata.function_kind != FunctionKind::AsyncGenerator
        || bytecode.metadata.constructor_kind != ConstructorKind::None
        || !bytecode.metadata.has_prototype
        || bytecode.metadata.class_initializer_kind.is_some()
        || bytecode.realm != vm.callee_realm
        || vm.strict != bytecode.metadata.strict
        || heap.context(vm.callee_realm)?.global_object != vm.callee_global
        || !matches!(
            function.payload,
            ObjectPayload::BytecodeFunction {
                bytecode: owner,
                ..
            } if owner == activation.bytecode
        )
        || function.is_constructor
        || activation.arguments.len() < usize::from(bytecode.metadata.argument_count)
        || activation.actual_argument_count > activation.arguments.len()
        || activation.locals.len() != usize::from(bytecode.metadata.local_count)
        || activation.reusable_captured_locals.len() != activation.locals.len()
        || vm.stack.len() > usize::from(bytecode.metadata.max_stack)
        || vm.pc == 0
        || vm.pc > bytecode.code.len()
    {
        return Err(HeapError::Invariant(
            "AsyncGenerator activation has invalid frame metadata",
        ));
    }
    let suspension_matches = matches!(
        (data.state, bytecode.code.get(vm.pc - 1)),
        (
            AsyncGeneratorState::SuspendedStart,
            Some(Instruction::InitialYield)
        ) | (
            AsyncGeneratorState::SuspendedYield,
            Some(Instruction::Yield)
        ) | (
            AsyncGeneratorState::SuspendedYieldStar,
            Some(Instruction::AsyncYieldStar)
        ) | (AsyncGeneratorState::Executing, Some(Instruction::Await))
    );
    if !suspension_matches {
        return Err(HeapError::Invariant(
            "AsyncGenerator activation is parked after the wrong opcode",
        ));
    }
    for value in vm
        .stack
        .iter()
        .chain(std::iter::once(&vm.this_value))
        .chain(vm.normalized_this.iter())
        .chain(std::iter::once(&vm.new_target))
    {
        if !is_map_storable_value(value) {
            return Err(HeapError::Invariant(
                "AsyncGenerator activation contains an internal-only value",
            ));
        }
    }
    for binding in activation.arguments.iter().chain(activation.locals.iter()) {
        match binding {
            GeneratorFrameBinding::Direct(value) if !is_map_storable_value(value) => {
                return Err(HeapError::Invariant(
                    "AsyncGenerator frame binding contains an internal-only value",
                ));
            }
            GeneratorFrameBinding::Private(atom) if atom.is_null() => {
                return Err(HeapError::Invariant(
                    "AsyncGenerator private binding contains the null atom",
                ));
            }
            GeneratorFrameBinding::PrivateCallable(callable) => {
                if !object_data_is_callable(heap.object(*callable)?) {
                    return Err(HeapError::Invariant(
                        "AsyncGenerator private callable binding is not callable",
                    ));
                }
            }
            GeneratorFrameBinding::Captured(var_ref) => {
                heap.var_ref(*var_ref)?;
            }
            GeneratorFrameBinding::Direct(_)
            | GeneratorFrameBinding::Private(_)
            | GeneratorFrameBinding::Uninitialized => {}
        }
    }
    for (region_index, region) in vm.regions.iter().enumerate() {
        match *region {
            crate::engine::vm::VmUnwindRegion::Catch {
                target,
                stack_depth,
            } if target >= bytecode.code.len() || stack_depth > vm.stack.len() => {
                return Err(HeapError::Invariant(
                    "AsyncGenerator catch region is outside its saved frame",
                ));
            }
            crate::engine::vm::VmUnwindRegion::Iterator { record_base, .. }
                if record_base.saturating_add(1) >= vm.stack.len() =>
            {
                return Err(HeapError::Invariant(
                    "AsyncGenerator iterator region is outside its saved frame",
                ));
            }
            crate::engine::vm::VmUnwindRegion::Iterator {
                record_base,
                enabled: false,
                asynchronous: false,
            } if !matches!(vm.stack.get(record_base), Some(RawValue::Undefined)) => {
                return Err(HeapError::Invariant(
                    "AsyncGenerator activation contains an invalid completed iterator",
                ));
            }
            crate::engine::vm::VmUnwindRegion::Iterator {
                record_base,
                enabled: false,
                asynchronous,
                ..
            } if asynchronous
                && (region_index + 1 != vm.regions.len()
                    || !matches!(data.state, AsyncGeneratorState::Executing)
                    || record_base.checked_add(3) != Some(vm.stack.len())
                    || !matches!(vm.stack.last(), Some(RawValue::Undefined))
                    || !matches!(
                        bytecode.code.get(vm.pc),
                        Some(Instruction::IteratorGetValueDone)
                    )) =>
            {
                return Err(HeapError::Invariant(
                    "AsyncGenerator activation contains an invalid pending iterator state",
                ));
            }
            crate::engine::vm::VmUnwindRegion::Catch { .. }
            | crate::engine::vm::VmUnwindRegion::Iterator { .. } => {}
        }
    }
    Ok(())
}

pub(in crate::engine::heap) fn validate_async_from_sync_iterator_data(
    heap: &Heap,
    data: &AsyncFromSyncIteratorData,
) -> Result<(), HeapError> {
    heap.object(data.sync_iterator)?;
    if !is_map_storable_value(&data.next) {
        return Err(HeapError::Invariant(
            "Async-from-Sync Iterator payload contains an internal value sentinel",
        ));
    }
    for edge in raw_value_edges(&data.next) {
        let RawId::Object(object) = edge else {
            unreachable!("RawValue only owns object edges")
        };
        heap.object(object)?;
    }
    Ok(())
}
