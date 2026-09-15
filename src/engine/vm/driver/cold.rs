//! One exhaustive cold-boundary dispatch; no exit is scanned by a second dispatcher.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Disposition {
    Entered,
    Complete,
    Rethrow,
    Bridge,
    Suspend(super::super::VmSuspendKind),
}

pub(super) struct Context<'a> {
    runtime: &'a Runtime,
    execution: &'a mut RunningExecution,
    id: FrameId,
    forwarded: &'a mut Option<Completion>,
    conversion: &'a mut Option<super::super::conversion_driver::ConversionTask>,
    next_operation: &'a mut u64,
    conversion_prepared: bool,
}
impl Context<'_> {
    fn complete(&mut self, completion: Completion) -> Disposition {
        let disposition = if matches!(completion, Completion::Throw(_)) {
            Disposition::Rethrow
        } else {
            Disposition::Complete
        };
        *self.forwarded = Some(completion);
        disposition
    }
    fn step(&mut self, step: CallStep) -> Disposition {
        match step {
            CallStep::Entered => Disposition::Entered,
            CallStep::Complete(completion) => self.complete(completion),
            CallStep::Bridge => Disposition::Bridge,
        }
    }
}

#[inline(never)]
pub(super) fn dispatch(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    exit: RunExit,
    forwarded: &mut Option<Completion>,
    conversion: &mut Option<super::super::conversion_driver::ConversionTask>,
    next_operation: &mut u64,
    conversion_prepared: bool,
) -> Result<Disposition, Error> {
    let mut context = Context {
        runtime,
        execution,
        id,
        forwarded,
        conversion,
        next_operation,
        conversion_prepared,
    };
    Ok(match exit {
        RunExit::Pure(operation) => {
            let step = super::super::frame_operations::pure(
                context.runtime,
                context.execution,
                context.id,
                operation,
            )?;
            context.step(step)
        }
        RunExit::CopyData {
            target,
            source,
            excluded,
        } => {
            let step = super::super::frame_operations::copy_data(
                context.runtime,
                context.execution,
                context.id,
                usize::from(target),
                usize::from(source),
                excluded.map(usize::from),
            )?;
            context.step(step)
        }
        RunExit::HomeObject => {
            let step = super::super::frame_operations::home_object(
                context.runtime,
                context.execution,
                context.id,
            )?;
            context.step(step)
        }
        RunExit::GetSuper => {
            let step = super::super::frame_operations::get_super(
                context.runtime,
                context.execution,
                context.id,
            )?;
            context.step(step)
        }
        RunExit::BindingError {
            index,
            redeclaration,
        } => {
            let step = super::super::frame_operations::binding_error(
                context.runtime,
                context.execution,
                context.id,
                index,
                redeclaration,
            )?;
            context.step(step)
        }
        RunExit::PrivateInitialize { index, kind } => {
            let step = super::super::frame_operations::private_initialize(
                context.runtime,
                context.execution,
                context.id,
                index,
                kind,
            )?;
            context.step(step)
        }
        RunExit::PrivateAccess { source, access } => {
            let step = super::super::frame_operations::private_access(
                context.runtime,
                context.execution,
                context.id,
                source,
                access,
            )?;
            context.step(step)
        }
        RunExit::LogicalNot => {
            let step = super::super::frame_operations::logical_not(
                context.runtime,
                context.execution,
                context.id,
            )?;
            context.step(step)
        }
        RunExit::ForIn(next) => {
            let step = super::super::frame_operations::for_in(
                context.runtime,
                context.execution,
                context.id,
                next,
            )?;
            context.step(step)
        }
        RunExit::Numeric(kind) => {
            let step = super::super::frame_operations::numeric(
                context.runtime,
                context.execution,
                context.id,
                kind,
            )?;
            context.step(step)
        }
        RunExit::StrictEquality(negate) => {
            let step = super::super::frame_operations::strict_equality(
                context.runtime,
                context.execution,
                context.id,
                negate,
            )?;
            context.step(step)
        }
        exit @ (RunExit::Arguments(_) | RunExit::Rest(_)) => {
            let step = super::super::frame_operations::arguments(
                context.runtime,
                context.execution,
                context.id,
                exit,
            )?;
            context.step(step)
        }
        RunExit::SetName(index) => {
            let step = super::super::frame_operations::set_name(
                context.runtime,
                context.execution,
                context.id,
                index,
            )?;
            context.step(step)
        }
        RunExit::InstantiateClosure(index) => {
            let step = super::super::frame_operations::instantiate_closure(
                context.runtime,
                context.execution,
                context.id,
                index,
            )?;
            context.step(step)
        }
        RunExit::ResetCaptured(index) => {
            let step = super::super::frame_operations::reset_captured(
                context.runtime,
                context.execution,
                context.id,
                index,
            )?;
            context.step(step)
        }
        RunExit::CloseCaptured(index) => {
            let step = super::super::frame_operations::close_captured(
                context.runtime,
                context.execution,
                context.id,
                index,
            )?;
            context.step(step)
        }
        exit @ (RunExit::Catch(_) | RunExit::DropCatch | RunExit::NipCatch) => {
            let step = super::super::frame_operations::catch(
                context.runtime,
                context.execution,
                context.id,
                exit,
            )?;
            context.step(step)
        }
        RunExit::Throw => {
            let step = super::super::frame_operations::throw(
                context.runtime,
                context.execution,
                context.id,
            )?;
            context.step(step)
        }
        RunExit::Binding {
            source,
            index,
            write,
            checked,
            keep,
        } => {
            let step = super::super::frame_operations::binding(
                context.runtime,
                context.execution,
                context.id,
                source,
                index,
                write,
                checked,
                keep,
            )?;
            context.step(step)
        }
        RunExit::LexicalUninitialized(index) => {
            let step = super::super::frame_operations::lexical_uninitialized(
                context.runtime,
                context.execution,
                context.id,
                index,
            )?;
            context.step(step)
        }
        RunExit::InitializeDerived(index) => {
            let step = super::super::frame_operations::initialize_derived(
                context.runtime,
                context.execution,
                context.id,
                index,
            )?;
            context.step(step)
        }
        RunExit::ReturnDerived(index) => {
            let step = super::super::frame_operations::return_derived(
                context.runtime,
                context.execution,
                context.id,
                index,
            )?;
            context.step(step)
        }
        RunExit::NormalizeThis => {
            let step = super::super::frame_operations::normalize_this(
                context.runtime,
                context.execution,
                context.id,
            )?;
            context.step(step)
        }
        RunExit::Call {
            arguments,
            method,
            tail,
        } => call(&mut context, arguments, method, tail)?,
        RunExit::Environment(super::super::environment_driver::Operation::Has { source, name }) => {
            with_has(&mut context, source, name)?
        }
        RunExit::Environment(op) => environment(&mut context, op)?,
        RunExit::DefineProperty { key, method } => define_property(&mut context, key, method)?,
        RunExit::DefineClass { name, has_heritage } => {
            define_class(&mut context, name, has_heritage)?
        }
        RunExit::ClassInitializer(mode) => class_initializer(&mut context, mode)?,
        RunExit::Construct(count) => construct(&mut context, count)?,
        RunExit::Apply(kind) => apply(&mut context, kind)?,
        RunExit::InitDerivedConstructor => init_derived_constructor(&mut context)?,
        RunExit::ConvertAdd => convert(&mut context, true, false)?,
        RunExit::ApplyEval(environment) => apply_eval(&mut context, environment)?,
        RunExit::Eval {
            arguments,
            environment,
        } => eval(&mut context, arguments, environment)?,
        RunExit::Import => import(&mut context)?,
        RunExit::Predicate(kind) => predicate(&mut context, kind)?,
        RunExit::SuperProperty(kind) => super_property(&mut context, kind)?,
        RunExit::SetProperty(key) => set_property(&mut context, key)?,
        RunExit::GetField {
            index,
            keep_receiver,
        } => get_field(&mut context, index, keep_receiver)?,
        RunExit::GetElement {
            keep_receiver,
            keep_key,
        } => get_element(&mut context, keep_receiver, keep_key)?,
        RunExit::ConvertPlus => convert(&mut context, false, false)?,
        RunExit::ConvertPropertyKey => convert(&mut context, false, true)?,
        exit @ (RunExit::ReplaceBinding { .. } | RunExit::ReleaseOperand { .. }) => {
            direct(&mut context, exit)?
        }
        RunExit::Complete => {
            if matches!(context.forwarded, Some(Completion::Throw(_))) {
                Disposition::Rethrow
            } else {
                Disposition::Complete
            }
        }
        RunExit::Bridge => Disposition::Bridge,
        RunExit::Suspend(kind) => Disposition::Suspend(kind),
        RunExit::PrimitiveThrow | RunExit::AddLocal | RunExit::Materialize => {
            return Err(Error::internal("resident-only exit reached cold dispatch"));
        }
    })
}

#[inline(never)]
fn direct(context: &mut Context<'_>, exit: RunExit) -> Result<Disposition, Error> {
    if super::super::frame_operations::complete_owned_slot(context.execution, context.id, exit)? {
        Ok(Disposition::Entered)
    } else {
        Err(Error::internal("direct cold operation was not completed"))
    }
}

#[inline(never)]
fn call(
    context: &mut Context<'_>,
    arguments: u16,
    method: bool,
    tail: bool,
) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    match super::enter_call(runtime, execution, id, arguments, method, tail)? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => return Ok(Disposition::Bridge),
    }
}

#[inline(never)]
fn with_has(
    context: &mut Context<'_>,
    source: crate::engine::code::bytecode::DynamicEnvironmentSource,
    name: u32,
) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    match super::super::with_driver::start(runtime, execution, id, source, name)? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => return Ok(Disposition::Bridge),
    }
}

#[inline(never)]
fn environment(
    context: &mut Context<'_>,
    op: super::super::environment_driver::Operation,
) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    match super::super::environment_driver::step(runtime, execution, id, op)? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => return Ok(Disposition::Bridge),
    }
}

#[inline(never)]
fn define_property(
    context: &mut Context<'_>,
    key: Option<u32>,
    method: Option<(crate::engine::code::bytecode::DefineMethodKind, bool)>,
) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    match super::super::construct_driver::define_property(runtime, execution, id, key, method)? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => return Ok(Disposition::Bridge),
    }
}

#[inline(never)]
fn define_class(
    context: &mut Context<'_>,
    name: u32,
    has_heritage: bool,
) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    (*context.next_operation) = (*context.next_operation)
        .checked_add(1)
        .ok_or_else(|| Error::internal("operation identity exhausted"))?;
    match super::super::construct_driver::define_class(
        runtime,
        execution,
        id,
        name,
        has_heritage,
        *context.next_operation,
    )? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => return Ok(Disposition::Bridge),
    }
}

#[inline(never)]
fn class_initializer(
    context: &mut Context<'_>,
    mode: super::super::construct_driver::InitializerKind,
) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    match super::super::construct_driver::initializer(runtime, execution, id, mode)? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => {
            return Err(Error::internal("class initialization attempted replay"));
        }
    }
}

#[inline(never)]
fn convert(
    context: &mut Context<'_>,
    addition: bool,
    property_key: bool,
) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    let mut invalid = false;
    if !context.conversion_prepared {
        let frame = execution.frames.current_mut(id)?;
        for offset in (0..=usize::from(addition)).rev() {
            invalid |= runtime
                .validate_value_domain(
                    execution.slots.peek(&frame.window, offset)?,
                    "conversion operand",
                )
                .is_err();
        }
    }
    if invalid {
        return Ok(Disposition::Bridge);
    } else {
        if !context.conversion_prepared {
            (*context.next_operation) = (*context.next_operation)
                .checked_add(1)
                .ok_or_else(|| Error::internal("conversion identity exhausted"))?;
        }
        *context.conversion = Some(crate::engine::vm::conversion_driver::ConversionTask::start(
            runtime,
            execution,
            id,
            *context.next_operation,
            addition,
            property_key,
        )?);
        return Ok(Disposition::Entered);
    }
}

#[inline(never)]
fn apply_eval(context: &mut Context<'_>, environment: u16) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    match super::super::eval_driver::apply(runtime, execution, id, environment)? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => return Ok(Disposition::Bridge),
    }
}

#[inline(never)]
fn eval(context: &mut Context<'_>, arguments: u16, environment: u16) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    match super::super::eval_driver::step(runtime, execution, id, arguments, environment)? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => return Ok(Disposition::Bridge),
    }
}

#[inline(never)]
fn import(context: &mut Context<'_>) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    match super::super::proxy_get_driver::start_import(runtime, execution, id)? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => return Err(Error::internal("dynamic import attempted replay")),
    }
}

#[inline(never)]
fn predicate(
    context: &mut Context<'_>,
    kind: super::super::predicate_driver::Kind,
) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    match super::super::predicate_driver::start(runtime, execution, id, kind)? {
        super::super::predicate_driver::Progress::Convert(input) => {
            (*context.next_operation) = (*context.next_operation)
                .checked_add(1)
                .ok_or_else(|| Error::internal("predicate conversion identity exhausted"))?;
            *context.conversion = Some(
                super::super::conversion_driver::ConversionTask::start_predicate(
                    runtime,
                    execution,
                    id,
                    *context.next_operation,
                    input,
                )?,
            );
            return Ok(Disposition::Entered);
        }
        super::super::predicate_driver::Progress::Call(CallStep::Entered) => {
            return Ok(Disposition::Entered);
        }
        super::super::predicate_driver::Progress::Call(CallStep::Complete(completion)) => {
            return Ok(context.complete(completion));
        }
        super::super::predicate_driver::Progress::Call(CallStep::Bridge) => {
            return Err(Error::internal("predicate attempted replay"));
        }
    }
}

#[inline(never)]
fn super_property(
    context: &mut Context<'_>,
    kind: super::super::super_property_driver::Kind,
) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    match super::super::super_property_driver::start(runtime, execution, id, kind)? {
        super::super::super_property_driver::Progress::Convert(input) => {
            (*context.next_operation) = (*context.next_operation)
                .checked_add(1)
                .ok_or_else(|| Error::internal("super conversion identity exhausted"))?;
            *context.conversion = Some(
                super::super::conversion_driver::ConversionTask::start_super_property(
                    runtime,
                    execution,
                    id,
                    *context.next_operation,
                    input,
                )?,
            );
            return Ok(Disposition::Entered);
        }
        super::super::super_property_driver::Progress::Call(CallStep::Entered) => {
            return Ok(Disposition::Entered);
        }
        super::super::super_property_driver::Progress::Call(CallStep::Complete(completion)) => {
            return Ok(context.complete(completion));
        }
        super::super::super_property_driver::Progress::Call(CallStep::Bridge) => {
            return Err(Error::internal("super property attempted replay"));
        }
    }
}

#[inline(never)]
fn set_property(context: &mut Context<'_>, key: Option<u32>) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    let frame = execution.frames.current_mut(id)?;
    if key.is_none() && matches!(execution.slots.peek(&frame.window, 1)?, Value::Object(_)) {
        (*context.next_operation) = (*context.next_operation)
            .checked_add(1)
            .ok_or_else(|| Error::internal("write conversion identity exhausted"))?;
        *context.conversion = Some(
            super::super::conversion_driver::ConversionTask::start_property_write(
                runtime,
                execution,
                id,
                *context.next_operation,
            )?,
        );
        return Ok(Disposition::Entered);
    }
    match super::super::property_write_driver::write(runtime, execution, id, key)? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => return Ok(Disposition::Bridge),
    }
}

#[inline(never)]
fn get_field(
    context: &mut Context<'_>,
    index: u32,
    keep_receiver: bool,
) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    match super::super::property_driver::read(
        runtime,
        execution,
        id,
        super::super::property_driver::ReadKey::Static(index),
        keep_receiver,
    )? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => return Ok(Disposition::Bridge),
    }
}

#[inline(never)]
fn get_element(
    context: &mut Context<'_>,
    keep_receiver: bool,
    keep_key: bool,
) -> Result<Disposition, Error> {
    let runtime = context.runtime;
    let execution = &mut *context.execution;
    let id = context.id;

    let frame = execution.frames.current_mut(id)?;
    if !matches!(
        execution.slots.peek(&frame.window, 1)?,
        Value::Null | Value::Undefined
    ) && matches!(execution.slots.peek(&frame.window, 0)?, Value::Object(_))
    {
        (*context.next_operation) = (*context.next_operation)
            .checked_add(1)
            .ok_or_else(|| Error::internal("property conversion identity exhausted"))?;
        *context.conversion = Some(
            super::super::conversion_driver::ConversionTask::start_property_read(
                runtime,
                execution,
                id,
                *context.next_operation,
                keep_receiver,
                keep_key,
            )?,
        );
        return Ok(Disposition::Entered);
    }
    match super::super::property_driver::read(
        runtime,
        execution,
        id,
        super::super::property_driver::ReadKey::Computed { keep_key },
        keep_receiver,
    )? {
        CallStep::Entered => return Ok(Disposition::Entered),
        CallStep::Complete(completion) => {
            return Ok(context.complete(completion));
        }
        CallStep::Bridge => return Ok(Disposition::Bridge),
    }
}

#[inline(never)]
fn construct(context: &mut Context<'_>, count: u16) -> Result<Disposition, Error> {
    *context.next_operation = context
        .next_operation
        .checked_add(1)
        .ok_or_else(|| Error::internal("operation identity exhausted"))?;
    let step = super::super::construct_driver::enter(
        context.runtime,
        context.execution,
        context.id,
        count,
        *context.next_operation,
    )?;
    Ok(context.step(step))
}

#[inline(never)]
fn apply(
    context: &mut Context<'_>,
    kind: crate::engine::code::bytecode::ApplyKind,
) -> Result<Disposition, Error> {
    *context.next_operation = context
        .next_operation
        .checked_add(1)
        .ok_or_else(|| Error::internal("operation identity exhausted"))?;
    let step = super::super::apply_driver::step(
        context.runtime,
        context.execution,
        context.id,
        kind,
        *context.next_operation,
    )?;
    Ok(context.step(step))
}

#[inline(never)]
fn init_derived_constructor(context: &mut Context<'_>) -> Result<Disposition, Error> {
    *context.next_operation = context
        .next_operation
        .checked_add(1)
        .ok_or_else(|| Error::internal("operation identity exhausted"))?;
    let step = super::super::construct_driver::enter_default_derived(
        context.runtime,
        context.execution,
        context.id,
        *context.next_operation,
    )?;
    Ok(context.step(step))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cold_dispatch_carries_only_disposition_not_completion_owners() {
        assert!(size_of::<Disposition>() <= 8);
        assert!(size_of::<RunExit>() <= 32);
    }
}
