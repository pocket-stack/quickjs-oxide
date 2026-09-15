//! Shared suspension ownership for generators, async functions and async generators.
//!
//! Frozen raw records borrow their source roots until heap publication completes.
//! Thaw reconstructs all roots before language owners detach dormant heap edges.
//! The language state machines and microtask policy stay with their own drivers.

use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::atom::{Atom, AtomKind};
use crate::engine::code::function::metadata::{
    ClosureVariableKind, FunctionKind, VariableDefinition,
};
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::code::runtime::PublishedFunctionData;
use crate::engine::heap::roots::VarRefRoot;
use crate::engine::heap::{
    ContextId, GeneratorActivationData, GeneratorFrameBinding, GeneratorVmActivation, RawValue,
};
use crate::engine::object::{ObjectRef, PrivateNameRef};
use crate::engine::value::Value;
use crate::engine::vm::bindings::{FrameBinding, is_private_callable_kind};
use crate::engine::vm::call::CallableExecution;
use crate::engine::vm::frames::ActiveFrameToken;
use crate::engine::vm::host_bridge::RuntimeVmHost;
use crate::engine::vm::{
    BytecodePc, Completion, VmActivationParts, VmResume, VmSuspendKind, VmSuspension,
};

#[cfg(not(feature = "stack-vm"))]
use super::{Vm, VmExit};

pub(super) mod creation;

#[cfg(feature = "stack-vm")]
mod owned;
#[cfg(feature = "stack-vm")]
pub(super) use owned::{OwnedSuspension, PreparedResume};

/// Start every language suspension family through the configured executor.
pub(super) fn start(
    host: RuntimeVmHost,
    input: super::CallInput,
    original_arguments: &[Value],
) -> Result<VmRunOutcome, RuntimeError> {
    #[cfg(feature = "stack-vm")]
    {
        let runtime = host.runtime.clone();
        super::host_bridge::owned::execute(host, input, original_arguments)
            .and_then(|exit| exit.finish_suspending(runtime))
            .map_err(RuntimeError::Engine)
    }
    #[cfg(not(feature = "stack-vm"))]
    {
        let mut host = host;
        let result = Vm::new()
            .start_published(input, &mut host)
            .map_err(RuntimeError::Engine)?;
        match result {
            VmExit::Complete(completion) => Ok(VmRunOutcome::Complete(completion)),
            VmExit::Suspend(suspension) => {
                let mut originals = Vec::new();
                originals
                    .try_reserve_exact(original_arguments.len())
                    .map_err(|_| {
                        RuntimeError::Invariant("original argument snapshot allocation failed")
                    })?;
                for value in original_arguments {
                    originals.push(match value {
                        Value::Object(value) => Value::Object(value.try_clone()?),
                        value => value.clone(),
                    });
                }
                finish_suspension(host, suspension, originals)
            }
        }
    }
}

pub(in crate::engine::vm) fn finish_suspension(
    host: RuntimeVmHost,
    mut suspension: VmSuspension,
    originals: Vec<Value>,
) -> Result<VmRunOutcome, RuntimeError> {
    let value = match suspension.kind() {
        VmSuspendKind::Initial => Value::Undefined,
        VmSuspendKind::Await => suspension.take_awaited().map_err(RuntimeError::Engine)?,
        _ => suspension.take_yielded().map_err(RuntimeError::Engine)?,
    };
    Ok(VmRunOutcome::Suspend {
        value,
        activation: Box::new(freeze(host, suspension, originals)?),
    })
}

fn encode_generator_frame_binding(
    runtime: &Runtime,
    binding: &FrameBinding,
) -> Result<GeneratorFrameBinding, RuntimeError> {
    Ok(match binding {
        FrameBinding::Direct(value) => {
            GeneratorFrameBinding::Direct(runtime.raw_property_value(value)?)
        }
        FrameBinding::Private(name) => {
            if !name.belongs_to(runtime) {
                return Err(RuntimeError::WrongRuntime("generator private binding"));
            }
            GeneratorFrameBinding::Private(name.atom())
        }
        FrameBinding::PrivateCallable(callable) => {
            if !callable.belongs_to(runtime) {
                return Err(RuntimeError::WrongRuntime(
                    "generator private callable binding",
                ));
            }
            GeneratorFrameBinding::PrivateCallable(callable.as_object().object_id())
        }
        FrameBinding::Uninitialized => GeneratorFrameBinding::Uninitialized,
        FrameBinding::Captured(root) => {
            if !root.belongs_to(runtime) {
                return Err(RuntimeError::WrongRuntime("generator captured binding"));
            }
            GeneratorFrameBinding::Captured(root.id())
        }
    })
}

fn validate_decoded_generator_binding(
    runtime: &Runtime,
    binding: &FrameBinding,
    definition: Option<&VariableDefinition>,
) -> Result<(), RuntimeError> {
    let Some(definition) = definition else {
        if matches!(binding, FrameBinding::Direct(_)) {
            return Ok(());
        }
        return Err(RuntimeError::Invariant(
            "extra generator argument has a non-direct binding",
        ));
    };
    match binding {
        FrameBinding::Direct(_) if definition.kind.is_private() => Err(RuntimeError::Invariant(
            "generator private definition decoded as an ordinary value",
        )),
        FrameBinding::Private(_)
            if definition.kind != ClosureVariableKind::PrivateField
                || !definition.is_lexical
                || !definition.is_const =>
        {
            Err(RuntimeError::Invariant(
                "generator private-name binding disagrees with its definition",
            ))
        }
        FrameBinding::PrivateCallable(_)
            if !is_private_callable_kind(definition.kind)
                || !definition.is_lexical
                || !definition.is_const =>
        {
            Err(RuntimeError::Invariant(
                "generator private-callable binding disagrees with its definition",
            ))
        }
        FrameBinding::Uninitialized if !definition.is_lexical => Err(RuntimeError::Invariant(
            "generator non-lexical binding decoded as uninitialized",
        )),
        FrameBinding::Captured(root) => {
            let state = runtime.0.state.borrow();
            let cell = state.heap.var_ref(root.id())?;
            if (cell.is_lexical, cell.is_const, cell.kind)
                != (definition.is_lexical, definition.is_const, definition.kind)
            {
                return Err(RuntimeError::Invariant(
                    "generator captured binding metadata disagrees with its definition",
                ));
            }
            Ok(())
        }
        FrameBinding::Direct(_)
        | FrameBinding::Private(_)
        | FrameBinding::PrivateCallable(_)
        | FrameBinding::Uninitialized => Ok(()),
    }
}

fn decode_generator_frame_binding(
    runtime: &Runtime,
    binding: &GeneratorFrameBinding,
    definition: Option<&VariableDefinition>,
) -> Result<FrameBinding, RuntimeError> {
    let binding = match binding {
        GeneratorFrameBinding::Direct(value) => {
            FrameBinding::Direct(runtime.root_raw_value(value)?)
        }
        GeneratorFrameBinding::Private(atom) => {
            if runtime.0.state.borrow().atoms.kind(*atom)? != AtomKind::Private {
                return Err(RuntimeError::Invariant(
                    "generator private binding contains a non-private atom",
                ));
            }
            FrameBinding::Private(PrivateNameRef::from_borrowed_atom(runtime.clone(), *atom)?)
        }
        GeneratorFrameBinding::PrivateCallable(object) => {
            let object = ObjectRef::from_borrowed_handle(runtime.clone(), *object)?;
            let callable = runtime
                .as_callable(&object)?
                .ok_or(RuntimeError::Invariant(
                    "generator private callable binding lost callability",
                ))?;
            FrameBinding::PrivateCallable(callable)
        }
        GeneratorFrameBinding::Uninitialized => FrameBinding::Uninitialized,
        GeneratorFrameBinding::Captured(var_ref) => {
            FrameBinding::Captured(VarRefRoot::from_borrowed_handle(runtime.clone(), *var_ref)?)
        }
    };
    validate_decoded_generator_binding(runtime, &binding, definition)?;
    Ok(binding)
}

/// Raw resumable activation plus every transient root from which it was
/// encoded. The wrapper must outlive heap publication: raw
/// object/VarRef/bytecode/context identities are non-owning until a generator
/// object or hidden async-function state retains them.
pub(crate) struct EncodedVmActivation {
    pub(crate) kind: VmSuspendKind,
    pub(crate) data: GeneratorActivationData,
    _host: RuntimeVmHost,
    _parts: VmActivationParts,
    _original_arguments: Vec<Value>,
}

impl EncodedVmActivation {
    pub(crate) fn atoms(&self) -> Vec<Atom> {
        let vm = &self.data.vm;
        vm.stack
            .iter()
            .chain(self.data.original_arguments.iter())
            .chain(std::iter::once(&vm.this_value))
            .chain(vm.normalized_this.iter())
            .chain(std::iter::once(&vm.new_target))
            .filter_map(generator_raw_value_atom)
            .chain(
                self.data
                    .arguments
                    .iter()
                    .chain(self.data.locals.iter())
                    .filter_map(|binding| match binding {
                        GeneratorFrameBinding::Direct(value) => generator_raw_value_atom(value),
                        GeneratorFrameBinding::Private(atom) => Some(*atom),
                        GeneratorFrameBinding::PrivateCallable(_)
                        | GeneratorFrameBinding::Uninitialized
                        | GeneratorFrameBinding::Captured(_) => None,
                    }),
            )
            .collect()
    }
}

fn generator_raw_value_atom(value: &RawValue) -> Option<Atom> {
    match value {
        RawValue::Symbol(atom) | RawValue::Private(atom) => Some(*atom),
        RawValue::Undefined
        | RawValue::Null
        | RawValue::Bool(_)
        | RawValue::Int(_)
        | RawValue::Float(_)
        | RawValue::BigInt(_)
        | RawValue::String(_)
        | RawValue::Object(_)
        | RawValue::Uninitialized
        | RawValue::Exception => None,
    }
}

/// Fully rooted execution state reconstructed before its dormant heap edges
/// are detached. `host.active_frame_token` remains a sentinel until the
/// short-lived bytecode active frame is pushed for the actual resume.
pub(crate) struct RootedVmActivation {
    suspension: VmSuspension,
    host: RuntimeVmHost,
    saved_pc: usize,
    original_arguments: Vec<Value>,
}

pub(crate) enum VmActivationResume {
    Initial,
    Generator(VmResume),
    AwaitFulfill(Value),
    AwaitReject(Value),
}

pub(crate) enum VmRunOutcome {
    Complete(Completion),
    Suspend {
        value: Value,
        activation: Box<EncodedVmActivation>,
    },
}

impl RootedVmActivation {
    fn validate_resume(
        &self,
        runtime: &Runtime,
        resume: &VmActivationResume,
    ) -> Result<(), RuntimeError> {
        // Authenticate the resume input before installing any active frame or
        // invoking an unwinder. The dormant owner remains with the language
        // state machine until thaw has produced this single-use rooted value.
        if self.host.runtime.domain_id() != runtime.domain_id() {
            return Err(RuntimeError::WrongRuntime("suspended execution"));
        }
        match resume {
            VmActivationResume::Initial => {}
            VmActivationResume::Generator(
                VmResume::Next(value) | VmResume::Return(value) | VmResume::Throw(value),
            )
            | VmActivationResume::AwaitFulfill(value)
            | VmActivationResume::AwaitReject(value) => {
                runtime.validate_value_domain(value, "suspension resume value")?;
            }
        }
        Ok(())
    }

    pub(crate) fn run(
        self,
        runtime: &Runtime,
        resume: VmActivationResume,
    ) -> Result<VmRunOutcome, RuntimeError> {
        #[cfg(feature = "stack-vm")]
        {
            let prepared = self.prepare_owned(runtime, resume)?;
            super::driver::resume(runtime.clone(), prepared.entry, prepared.pc)
                .and_then(|exit| exit.finish_suspending(runtime.clone()))
                .map_err(RuntimeError::Engine)
        }
        #[cfg(not(feature = "stack-vm"))]
        {
            self.run_legacy(runtime, resume)
        }
    }

    #[cfg(feature = "stack-vm")]
    pub(super) fn prepare_owned(
        self,
        runtime: &Runtime,
        resume: VmActivationResume,
    ) -> Result<PreparedResume, RuntimeError> {
        #[cfg(feature = "profiling")]
        let _profile_phase =
            crate::engine::api::profiling::PhaseTimer::start_vm("thaw.prepare_owned");
        self.validate_resume(runtime, &resume)?;
        let Self {
            mut host,
            suspension,
            original_arguments,
            saved_pc,
            ..
        } = self;
        let root = host
            .executable
            .root()
            .ok_or(RuntimeError::Invariant(
                "resumable host has no published executable root",
            ))?
            .clone();
        let function = host
            .current_function
            .as_ref()
            .ok_or(RuntimeError::Invariant(
                "resumable host has no current function root",
            ))?
            .clone();
        let guard = runtime.push_bytecode_active_frame(
            function,
            root,
            host.current_realm,
            host.executable.frame_layout().is_strict(),
        )?;
        host.active_frame_token = guard.token();
        runtime.update_active_bytecode_pc(
            guard.token(),
            BytecodePc::new(saved_pc.saturating_sub(1)),
        )?;
        let mut prepared = owned::prepare(host, suspension, original_arguments, resume)?;
        prepared.entry.cold.entry_guard = Some(guard);
        Ok(prepared)
    }

    #[cfg(not(feature = "stack-vm"))]
    fn run_legacy(
        self,
        runtime: &Runtime,
        resume: VmActivationResume,
    ) -> Result<VmRunOutcome, RuntimeError> {
        self.validate_resume(runtime, &resume)?;
        let Self {
            suspension,
            mut host,
            saved_pc,
            original_arguments,
        } = self;
        let bytecode = host
            .executable
            .root()
            .ok_or(RuntimeError::Invariant(
                "resumable host has no published executable root",
            ))?
            .clone();
        let code = host.executable.code.clone();
        let strict = host.executable.frame_layout().is_strict();
        let function = host
            .current_function
            .as_ref()
            .ok_or(RuntimeError::Invariant(
                "resumable host has no current function root",
            ))?
            .clone();
        let active_frame =
            runtime.push_bytecode_active_frame(function, bytecode, host.current_realm, strict)?;
        host.active_frame_token = active_frame.token();
        runtime.update_active_bytecode_pc(
            active_frame.token(),
            BytecodePc::new(saved_pc.saturating_sub(1)),
        )?;
        let result = match (suspension.kind(), resume) {
            (VmSuspendKind::Initial, VmActivationResume::Initial) => {
                Vm::new().resume_published_initial(suspension, &code, &mut host)
            }
            (
                VmSuspendKind::Yield | VmSuspendKind::YieldStar | VmSuspendKind::AsyncYieldStar,
                VmActivationResume::Generator(resume),
            ) => Vm::new().resume_published(suspension, &code, &mut host, resume),
            (VmSuspendKind::Await, VmActivationResume::AwaitFulfill(value)) => {
                suspension.resume_await_fulfill(&code, &mut host, value)
            }
            (VmSuspendKind::Await, VmActivationResume::AwaitReject(reason)) => {
                suspension.resume_await_reject(&code, &mut host, reason)
            }
            _ => {
                return Err(RuntimeError::Invariant(
                    "resume operation disagrees with the suspended VM state",
                ));
            }
        };
        active_frame.finish()?;
        {
            match result.map_err(RuntimeError::Engine)? {
                VmExit::Complete(completion) => Ok(VmRunOutcome::Complete(completion)),
                VmExit::Suspend(mut suspension) => {
                    let value = match suspension.kind() {
                        VmSuspendKind::Initial => {
                            return Err(RuntimeError::Invariant(
                                "resumed activation reached an initial suspension",
                            ));
                        }
                        VmSuspendKind::Yield
                        | VmSuspendKind::YieldStar
                        | VmSuspendKind::AsyncYieldStar => {
                            suspension.take_yielded().map_err(RuntimeError::Engine)?
                        }
                        VmSuspendKind::Await => {
                            suspension.take_awaited().map_err(RuntimeError::Engine)?
                        }
                    };
                    let activation = freeze(host, suspension, original_arguments)?;
                    Ok(VmRunOutcome::Suspend {
                        value,
                        activation: Box::new(activation),
                    })
                }
            }
        }
    }
}

pub(crate) fn freeze(
    host: RuntimeVmHost,
    suspension: VmSuspension,
    original_arguments: Vec<Value>,
) -> Result<EncodedVmActivation, RuntimeError> {
    #[cfg(feature = "profiling")]
    let _profile_phase = crate::engine::api::profiling::PhaseTimer::start_vm("freeze.encode");
    let (kind, parts) = suspension.into_parts().map_err(RuntimeError::Engine)?;
    let bytecode = host.executable.root().ok_or(RuntimeError::Invariant(
        "resumable host has no current bytecode root",
    ))?;
    let caller_realm = parts.caller_realm.ok_or(RuntimeError::Invariant(
        "resumable VM activation has no caller realm",
    ))?;
    let callee_realm = parts.callee_realm.ok_or(RuntimeError::Invariant(
        "resumable VM activation has no callee realm",
    ))?;
    let current_function = parts
        .current_function
        .as_ref()
        .ok_or(RuntimeError::Invariant(
            "resumable VM activation has no current function",
        ))?;
    let callee_global = parts.callee_global.as_ref().ok_or(RuntimeError::Invariant(
        "resumable VM activation has no callee global",
    ))?;
    if caller_realm != host.caller_realm
        || callee_realm != host.current_realm
        || host.current_function.as_ref() != Some(current_function)
        || host.arguments.len() < host.executable.argument_definitions.len()
        || host.locals.len() != host.executable.local_definitions.len()
        || host.reusable_captured_locals.len() != host.locals.len()
        || host.actual_argument_count > host.arguments.len()
        || host.actual_argument_count != original_arguments.len()
    {
        return Err(RuntimeError::Invariant(
            "resumable VM activation disagrees with its runtime host",
        ));
    }
    let arguments = host
        .arguments
        .iter()
        .map(|binding| encode_generator_frame_binding(&host.runtime, binding))
        .collect::<Result<Vec<_>, _>>()?;
    let locals = host
        .locals
        .iter()
        .map(|binding| encode_generator_frame_binding(&host.runtime, binding))
        .collect::<Result<Vec<_>, _>>()?;
    let vm = GeneratorVmActivation {
        stack: parts
            .stack
            .iter()
            .map(|value| host.runtime.raw_property_value(value))
            .collect::<Result<Vec<_>, _>>()?,
        regions: parts.regions.clone(),
        pc: parts.pc,
        callee_realm,
        current_function: current_function.object_id(),
        this_value: host.runtime.raw_property_value(&parts.this_value)?,
        normalized_this: parts
            .normalized_this
            .as_ref()
            .map(|value| host.runtime.raw_property_value(value))
            .transpose()?,
        new_target: host.runtime.raw_property_value(&parts.new_target)?,
        strict: parts.strict,
        callee_global: callee_global.object_id(),
    };
    Ok(EncodedVmActivation {
        kind,
        data: GeneratorActivationData {
            bytecode: bytecode.bytecode_id(),
            vm,
            actual_argument_count: host.actual_argument_count,
            original_arguments: original_arguments
                .iter()
                .map(|value| host.runtime.raw_property_value(value))
                .collect::<Result<Vec<_>, _>>()?,
            arguments,
            locals,
            reusable_captured_locals: host.reusable_captured_locals.clone(),
        },
        _original_arguments: original_arguments,
        _host: host,
        _parts: parts,
    })
}

pub(crate) fn thaw(
    runtime: Runtime,
    kind: VmSuspendKind,
    resume_caller_realm: ContextId,
    data: &GeneratorActivationData,
    expected_function_kind: FunctionKind,
) -> Result<RootedVmActivation, RuntimeError> {
    #[cfg(feature = "profiling")]
    let _profile_phase = crate::engine::api::profiling::PhaseTimer::start_vm("thaw.decode");
    runtime.0.state.borrow().heap.context(resume_caller_realm)?;
    let bytecode_probe = FunctionBytecodeRef::from_borrowed_handle(runtime.clone(), data.bytecode)?;
    let executable = runtime.snapshot_function_bytecode(&bytecode_probe)?;
    let PublishedFunctionData {
        argument_definitions,
        local_definitions,
        metadata,
        realm,
        ..
    } = &*executable;
    let metadata = *metadata;
    let realm = *realm;
    drop(bytecode_probe);
    if metadata.function_kind != expected_function_kind
        || realm != data.vm.callee_realm
        || metadata.strict != data.vm.strict
        || data.arguments.len() < executable.frame_layout().arguments().len()
        || data.locals.len() != executable.frame_layout().locals().len()
        || data.reusable_captured_locals.len() != data.locals.len()
        || data.actual_argument_count > data.arguments.len()
        || data.original_arguments.len() != data.actual_argument_count
    {
        return Err(RuntimeError::Invariant(
            "raw resumable activation disagrees with published bytecode",
        ));
    }
    let current_function =
        ObjectRef::from_borrowed_handle(runtime.clone(), data.vm.current_function)?;
    let callable = runtime
        .as_callable(&current_function)?
        .ok_or(RuntimeError::Invariant(
            "resumable activation current function is not callable",
        ))?;
    let closure_slots = match runtime.bytecode_for_callable(&callable)? {
        CallableExecution::Bytecode {
            bytecode,
            closure_slots,
        } if bytecode.bytecode_id() == data.bytecode => closure_slots,
        CallableExecution::Bytecode { .. }
        | CallableExecution::Native { .. }
        | CallableExecution::Bound { .. }
        | CallableExecution::Proxy => {
            return Err(RuntimeError::Invariant(
                "resumable activation current function changed bytecode identity",
            ));
        }
    };
    if closure_slots.len() != executable.frame_layout().closures().len() {
        return Err(RuntimeError::Invariant(
            "resumable closure slot count disagrees with bytecode metadata",
        ));
    }
    let original_arguments = data
        .original_arguments
        .iter()
        .map(|value| runtime.root_raw_value(value))
        .collect::<Result<Vec<_>, _>>()?;
    let arguments = data
        .arguments
        .iter()
        .enumerate()
        .map(|(index, binding)| {
            decode_generator_frame_binding(&runtime, binding, argument_definitions.get(index))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let locals = data
        .locals
        .iter()
        .zip(local_definitions.iter())
        .map(|(binding, definition)| {
            decode_generator_frame_binding(&runtime, binding, Some(definition))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let callee_global = ObjectRef::from_borrowed_handle(runtime.clone(), data.vm.callee_global)?;
    let parts = VmActivationParts {
        stack: data
            .vm
            .stack
            .iter()
            .map(|value| runtime.root_raw_value(value))
            .collect::<Result<Vec<_>, _>>()?,
        regions: data.vm.regions.clone(),
        pc: data.vm.pc,
        caller_realm: Some(resume_caller_realm),
        callee_realm: Some(data.vm.callee_realm),
        current_function: Some(current_function.clone()),
        this_value: runtime.root_raw_value(&data.vm.this_value)?,
        normalized_this: data
            .vm
            .normalized_this
            .as_ref()
            .map(|value| runtime.root_raw_value(value))
            .transpose()?,
        new_target: runtime.root_raw_value(&data.vm.new_target)?,
        strict: data.vm.strict,
        callee_global: Some(callee_global),
    };
    let suspension = VmSuspension::from_parts(kind, parts).map_err(RuntimeError::Engine)?;
    let host = RuntimeVmHost {
        runtime,
        active_frame_token: ActiveFrameToken(0),
        current_realm: data.vm.callee_realm,
        caller_realm: resume_caller_realm,
        executable,
        current_function: Some(current_function),
        actual_argument_count: data.actual_argument_count,
        closure_slots,
        arguments,
        locals,
        reusable_captured_locals: data.reusable_captured_locals.clone(),
    };
    Ok(RootedVmActivation {
        suspension,
        host,
        saved_pc: data.vm.pc,
        original_arguments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::api::Context;
    use crate::engine::heap::GeneratorState;

    fn dormant(context: &mut Context) -> (ObjectRef, GeneratorActivationData) {
        let Value::Object(generator) = context
            .eval("(function*(value) { let held = value; yield held; return held; })({marker:42})")
            .unwrap()
        else {
            panic!("expected a generator");
        };
        let (_, data) = context
            .runtime()
            .0
            .state
            .borrow()
            .heap
            .generator_snapshot(generator.object_id())
            .unwrap();
        (generator, data.unwrap())
    }

    #[test]
    fn failed_thaw_releases_partial_roots_without_detaching_heap_state() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (generator, data) = dormant(&mut context);
        let GeneratorFrameBinding::Direct(RawValue::Object(argument)) = data.arguments[0] else {
            panic!("expected an object argument");
        };
        let counts = || {
            let state = runtime.0.state.borrow();
            (
                state.heap.object_strong_count(argument).unwrap(),
                state
                    .heap
                    .object_strong_count(data.vm.current_function)
                    .unwrap(),
                state
                    .heap
                    .function_bytecode_strong_count(data.bytecode)
                    .unwrap(),
            )
        };
        let before = counts();
        // The first operand and all bindings acquire roots before decoding
        // rejects the internal-only second operand.
        let mut malformed = data.clone();
        malformed.vm.stack = vec![RawValue::Object(argument), RawValue::Exception];
        assert!(
            thaw(
                runtime.clone(),
                VmSuspendKind::Initial,
                context.realm,
                &malformed,
                FunctionKind::Generator
            )
            .is_err()
        );
        assert_eq!(counts(), before);
        runtime.run_gc().unwrap();
        let (state, after) = runtime
            .0
            .state
            .borrow()
            .heap
            .generator_snapshot(generator.object_id())
            .unwrap();
        assert_eq!(state, GeneratorState::SuspendedStart);
        assert_eq!(after.as_ref(), Some(&data));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn abandoned_thaw_does_not_register_or_keep_the_runtime_alive() {
        let runtime = Runtime::new();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let (generator, data) = dormant(&mut context);
        let rooted = thaw(
            runtime.clone(),
            VmSuspendKind::Initial,
            context.realm,
            &data,
            FunctionKind::Generator,
        )
        .unwrap();
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        drop(generator);
        drop(context);
        runtime.run_gc().unwrap();
        drop(runtime);
        assert!(weak.upgrade().is_some());
        drop(rooted);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn resume_rejects_foreign_runtime_and_values_before_registering_a_frame() {
        let runtime = Runtime::new();
        let other = Runtime::new();
        let mut context = runtime.new_context();
        let (_generator, data) = dormant(&mut context);
        for foreign_runtime in [true, false] {
            let rooted = thaw(
                runtime.clone(),
                VmSuspendKind::Initial,
                context.realm,
                &data,
                FunctionKind::Generator,
            )
            .unwrap();
            let result = if foreign_runtime {
                rooted.run(&other, VmActivationResume::Initial)
            } else {
                rooted.run(
                    &runtime,
                    VmActivationResume::Generator(VmResume::Next(Value::Object(
                        other.new_object(None).unwrap(),
                    ))),
                )
            };
            assert!(matches!(result, Err(RuntimeError::WrongRuntime(_))));
            assert!(runtime.0.state.borrow().active_frames.is_empty());
            assert!(other.0.state.borrow().active_frames.is_empty());
        }
    }
}

#[cfg(all(test, feature = "stack-vm", feature = "profiling"))]
mod tests_owned;
