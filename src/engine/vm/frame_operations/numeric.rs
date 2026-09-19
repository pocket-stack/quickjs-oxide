//! One numeric operand transfer shared by same-frame and outer-driver paths.
use crate::engine::api::{Error, runtime::Runtime};
use crate::engine::vm::{
    driver::CallStep,
    execution::RunningExecution,
    frame::FrameId,
    numeric::operation::{NumericKind, NumericStep},
};

pub(in crate::engine::vm) enum NumericProgress {
    Completed,
    Deferred(CallStep),
}
impl NumericProgress {
    pub(in crate::engine::vm) fn into_call_step(self) -> CallStep {
        match self {
            Self::Completed => CallStep::Entered,
            Self::Deferred(step) => step,
        }
    }
}

/// Commit in the original previous/value order. Pending owners remain outside
/// RunSlots even if authentication or a later push fails.
pub(in crate::engine::vm) fn commit_output(
    execution: &mut RunningExecution,
    id: FrameId,
    value: crate::engine::value::JsValue,
    previous: Option<crate::engine::value::JsValue>,
    _depth: usize,
) -> Result<(), Error> {
    let frame = execution.frames.current_mut(id)?;
    let mut value = Some(value);
    let mut previous = previous;
    {
        let mut slots = execution.slots.run_window(&mut frame.window)?;
        if previous.is_some() {
            slots.push_pending(&mut previous)?;
        }
        slots.push_pending(&mut value)?;
    }
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("numeric resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(_depth);
    Ok(())
}

pub(in crate::engine::vm) fn try_complete_primitive(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    kind: NumericKind,
) -> Result<Option<NumericProgress>, Error> {
    use crate::engine::value::JsValue;
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let depth = execution.slots.depth(&frame.window);
    let mut transaction = execution.slots.frame_transaction(&mut frame.window)?;
    let (left, right) = {
        let mut slots = transaction.slots();
        // A malformed stack declines untouched: the canonical outer entry must
        // still pop RHS before reporting a missing LHS.
        for offset in 0..if kind.unary() { 1 } else { 2 } {
            if matches!(slots.peek(offset), Err(_) | Ok(JsValue::Object(_))) {
                return Ok(None);
            }
        }
        let right = slots.pop().expect("validated primitive operand");
        if kind.unary() {
            (right, None)
        } else {
            (
                slots.pop().expect("validated primitive left operand"),
                Some(right),
            )
        }
    };
    if !kind.primitive_arithmetic() {
        return match NumericStep::start(runtime, kind, left, right) {
            Ok(step) => crate::engine::vm::proxy_get_driver::start_numeric(
                runtime, execution, id, step, depth,
            )
            .map(Some),
            Err(error) => crate::engine::vm::property_driver::throw_error(runtime, realm, error)
                .map(|step| Some(NumericProgress::Deferred(step))),
        };
    }
    let output = match crate::engine::vm::numeric::operation::primitive_output(
        runtime, kind, left, right,
    ) {
        Ok(output) => output,
        Err(error) => {
            return crate::engine::vm::property_driver::throw_error(runtime, realm, error)
                .map(|step| Some(NumericProgress::Deferred(step)));
        }
    };
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event("numeric_completed_without_query");
    let mut value = Some(output.value);
    let mut previous = output.previous;
    {
        let mut slots = transaction.slots();
        if previous.is_some() {
            slots.push_pending(&mut previous)?;
        }
        slots.push_pending(&mut value)?;
    }
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("numeric resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(Some(NumericProgress::Completed))
}

pub(in crate::engine::vm) fn complete(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    kind: NumericKind,
) -> Result<NumericProgress, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let depth = execution.slots.depth(&frame.window);
    // Preserve the existing unary/right-before-left pop order and error path.
    let (left, right) = if kind.unary() {
        (execution.slots.pop(&mut frame.window)?, None)
    } else {
        let right = execution.slots.pop(&mut frame.window)?;
        (execution.slots.pop(&mut frame.window)?, Some(right))
    };
    let result = match NumericStep::start(runtime, kind, left, right) {
        Ok(step) => {
            crate::engine::vm::proxy_get_driver::start_numeric(runtime, execution, id, step, depth)?
        }
        Err(error) => NumericProgress::Deferred(crate::engine::vm::property_driver::throw_error(
            runtime, realm, error,
        )?),
    };
    match result {
        NumericProgress::Deferred(CallStep::Bridge) => {
            Err(Error::internal("numeric operation attempted replay"))
        }
        result => Ok(result),
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::api::{Runtime, Value};

    use super::*;
    use crate::engine::vm::{
        call::CallableExecution,
        conversion_driver::{PrimitiveCompletion, complete_primitives},
        execution::ExecutionLimits,
        frame::{ColdFrame, FrameCold, FrameEntry},
        stack::FrameStorage,
    };

    fn fixture(
        runtime: &Runtime,
        context: &mut crate::engine::api::Context,
    ) -> (RunningExecution, FrameId) {
        let Value::Object(function) = context.eval("(function(a,b){return a*b})").unwrap() else {
            panic!("function");
        };
        let callable = runtime.as_callable(&function).unwrap().unwrap();
        let CallableExecution::Bytecode {
            bytecode,
            closure_slots,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("bytecode");
        };
        let prepared = runtime
            .prepare_bytecode_frame(&callable, Value::Undefined, Value::Undefined, &[], bytecode)
            .unwrap();
        let locals = prepared.locals.len();
        let entry = FrameEntry {
            initialize_bindings: false,
            executable: prepared.executable,
            property_generation: 0,
            iterator_generation: 0,
            caller_realm: context.realm,
            active_frame: prepared.active_frame.token(),
            cold: ColdFrame::new(FrameCold {
                rare: std::cell::OnceCell::new(),
                return_to: None,
                entry_guard: Some(prepared.active_frame),
                function: function.into(),
                closure_slots,
                reusable_captured_locals: vec![false; locals],
                input: prepared.input.into(),
            }),
            storage: FrameStorage {
                original_arguments: vec![],
                parameters: prepared.arguments,
                locals: prepared.locals,
                operands: vec![],
            },
        };
        let mut execution = RunningExecution::new(runtime, ExecutionLimits::default()).unwrap();
        let id = crate::engine::vm::driver::push_frame(&mut execution, entry).unwrap();
        (execution, id)
    }

    fn push(execution: &mut RunningExecution, id: FrameId, value: Value) {
        let frame = execution.frames.current_mut(id).unwrap();
        execution.slots.push(&mut frame.window, value).unwrap();
    }

    #[test]
    fn primitive_transaction_preserves_partial_numeric_input_and_output_errors() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (mut execution, id) = fixture(&runtime, &mut context);
        push(&mut execution, id, Value::Int(7));
        assert!(
            try_complete_primitive(&runtime, &mut execution, id, NumericKind::Mul)
                .unwrap()
                .is_none()
        );
        let frame = execution.frames.current_mut(id).unwrap();
        assert_eq!(execution.slots.depth(&frame.window), 1);
        assert!(complete(&runtime, &mut execution, id, NumericKind::Mul).is_err());
        let frame = execution.frames.current_mut(id).unwrap();
        assert_eq!(
            execution.slots.depth(&frame.window),
            0,
            "outer RHS consumption must survive missing LHS"
        );
        // Leave exactly one free slot: previous commits even when value cannot.
        loop {
            let frame = execution.frames.current_mut(id).unwrap();
            if execution
                .slots
                .push(&mut frame.window, Value::Int(0))
                .is_err()
            {
                break;
            }
        }
        let frame = execution.frames.current_mut(id).unwrap();
        execution.slots.pop(&mut frame.window).unwrap();
        let fault = frame.fault_pc;
        let resume = frame.resume_pc;
        assert!(
            commit_output(&mut execution, id, Value::Int(99), Some(Value::Int(41)), 0).is_err()
        );
        let frame = execution.frames.current_mut(id).unwrap();
        assert_eq!(
            execution.slots.peek(&frame.window, 0).unwrap(),
            &Value::Int(41)
        );
        assert_eq!((frame.fault_pc, frame.resume_pc), (fault, resume));
    }

    #[test]
    fn resident_numeric_preflight_preserves_canonical_missing_left_consumption() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (mut execution, id) = fixture(&runtime, &mut context);
        push(
            &mut execution,
            id,
            Value::String(crate::engine::value::JsString::from_static("7")),
        );
        {
            let frame = execution.frames.current_mut(id).unwrap();
            let mut transaction = execution
                .slots
                .frame_transaction(&mut frame.window)
                .unwrap();
            let slots = transaction.slots();
            assert!(!crate::engine::vm::run::test_supported_numeric(
                &slots,
                NumericKind::Mul
            ));
            assert!(slots.peek(0).is_ok(), "preflight must not consume RHS");
            assert!(slots.peek(1).is_err());
        }
        // Rejection takes the unchanged canonical path, which consumes RHS
        // before the missing LHS is diagnosed. No resident helper is entered.
        assert!(complete(&runtime, &mut execution, id, NumericKind::Mul).is_err());
        let frame = execution.frames.current_mut(id).unwrap();
        assert_eq!(execution.slots.depth(&frame.window), 0);
        assert!(execution.pending.is_none());
    }

    #[test]
    fn resident_numeric_postfix_output_failure_keeps_previous_and_pc() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (mut execution, id) = fixture(&runtime, &mut context);
        let text = Value::String(crate::engine::value::JsString::from_static("41"));
        loop {
            let frame = execution.frames.current_mut(id).unwrap();
            if execution
                .slots
                .push(&mut frame.window, Value::Int(0))
                .is_err()
            {
                break;
            }
        }
        let frame = execution.frames.current_mut(id).unwrap();
        execution.slots.pop(&mut frame.window).unwrap();
        execution.slots.push(&mut frame.window, text).unwrap();
        let depth = execution.slots.depth(&frame.window);
        let before = (frame.fault_pc, frame.resume_pc);
        let realm = frame.executable.realm;
        let active_frame = frame.active_frame;
        let fault_pc = frame.fault_pc;
        runtime
            .update_active_bytecode_pc(active_frame, crate::engine::vm::BytecodePc::new(fault_pc))
            .unwrap();
        {
            let mut transaction = execution
                .slots
                .frame_transaction(&mut frame.window)
                .unwrap();
            {
                let slots = transaction.slots();
                assert!(crate::engine::vm::run::test_supported_numeric(
                    &slots,
                    NumericKind::PostInc
                ));
            }
            let result = crate::engine::vm::run::test_complete_numeric(
                &runtime,
                realm,
                &mut transaction,
                NumericKind::PostInc,
                &mut execution.pending,
                active_frame,
                fault_pc,
            );
            assert!(
                result.is_err(),
                "second postfix output has no remaining capacity"
            );
            let slots = transaction.slots();
            assert_eq!(
                slots.peek(0).unwrap(),
                &Value::Int(41),
                "previous commits before value fails"
            );
        }
        assert_eq!(execution.slots.depth(&frame.window), depth);
        assert_eq!((frame.fault_pc, frame.resume_pc), before);
        assert!(
            execution.pending.is_none(),
            "capacity failure remains an engine error"
        );
    }

    #[test]
    fn primitive_transaction_identity_domain_and_wait_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (mut execution, id) = fixture(&runtime, &mut context);
        execution
            .frames
            .current_mut(id)
            .unwrap()
            .property_generation = u64::MAX;
        push(&mut execution, id, Value::Int(6));
        push(&mut execution, id, Value::Int(7));
        assert!(matches!(
            try_complete_primitive(&runtime, &mut execution, id, NumericKind::Mul).unwrap(),
            Some(NumericProgress::Completed)
        ));
        let frame = execution.frames.current_mut(id).unwrap();
        assert_eq!(frame.property_generation, u64::MAX);
        assert_eq!(
            execution.slots.pop(&mut frame.window).unwrap(),
            Value::Int(42)
        );
        assert!(matches!(
            crate::engine::vm::proxy_get_driver::start_numeric(
                &runtime,
                &mut execution,
                id,
                NumericStep::Throw(Value::Int(17)),
                0
            )
            .unwrap(),
            NumericProgress::Deferred(CallStep::Complete(crate::engine::vm::Completion::Throw(
                Value::Int(17)
            )))
        ));
        let object = runtime.new_object(None).unwrap();
        let step =
            NumericStep::start(NumericKind::Plus, Value::Object(object.clone()), None).unwrap();
        assert!(
            crate::engine::vm::proxy_get_driver::start_numeric(
                &runtime,
                &mut execution,
                id,
                step,
                0
            )
            .is_err()
        );
        push(&mut execution, id, Value::Object(object));
        let mut identity = u64::MAX;
        assert!(complete_primitives(&runtime, &mut execution, id, false, &mut identity).is_err());
        let frame = execution.frames.current_mut(id).unwrap();
        assert_eq!(execution.slots.depth(&frame.window), 1);
        identity = 10;
        assert!(matches!(
            complete_primitives(&runtime, &mut execution, id, false, &mut identity).unwrap(),
            PrimitiveCompletion::Declined
        ));
        assert_eq!(identity, 11);
        let frame = execution.frames.current_mut(id).unwrap();
        execution.slots.pop(&mut frame.window).unwrap();
        let foreign = Runtime::new();
        push(
            &mut execution,
            id,
            Value::Object(foreign.new_object(None).unwrap()),
        );
        identity = u64::MAX;
        let error = complete_primitives(&runtime, &mut execution, id, false, &mut identity)
            .err()
            .expect("foreign conversion operand must fail before identity issue");
        assert!(error.to_string().contains("conversion operand"));
        assert_eq!(identity, u64::MAX);
        let frame = execution.frames.current_mut(id).unwrap();
        assert_eq!(execution.slots.depth(&frame.window), 1);
    }

    #[test]
    fn primitive_transaction_parsing_and_string_store_keep_shared_semantics() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let cases=['','  \t\n',' 1.25e2 ','0x10','0b101','0o17','Infinity','-Infinity','-0','bad','\ud800'];
            for(let s of cases) {
                let direct=s-0, callback=({valueOf(){return s}})-0;
                if(!Object.is(direct,callback)||!Object.is(+s,+({valueOf(){return s}})))return false;
            }
            let s='', alias='', b=1n;
            for(let i=0;i<40;i++){alias=s;s+='\ud800x';s='y'+s;b=b*3n;b+=1n;b+=2n;}
            if(s.length!==120||alias.length!==117||typeof b!=='bigint')return false;
            let marker={}, trace='', local='old';
            try{local+= { [Symbol.toPrimitive](hint){trace+=hint;throw marker} }}
            catch(e){if(e!==marker)return false;trace+='caught'}finally{trace+='finally'}
            let capture=()=>local;
            local+='new';
            return capture()==='oldnew' && trace==='defaultcaughtfinally';
        })()"#).unwrap(), Value::Bool(true));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn copy_transaction_exhausted_wait_restores_source_without_replaying_selected_getter() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let source = context
            .eval("globalThis.copyCalls=0;({a:1,get b(){copyCalls++;return 2}})")
            .unwrap();
        let target = runtime.new_object(None).unwrap();
        let (mut execution, id) = fixture(&runtime, &mut context);
        execution
            .frames
            .current_mut(id)
            .unwrap()
            .property_generation = u64::MAX;
        push(&mut execution, id, Value::Object(target.clone()));
        push(&mut execution, id, source.clone());
        let result = crate::engine::vm::proxy_get_driver::start_object_copy(
            &runtime,
            &mut execution,
            id,
            1,
            0,
            None,
        );
        assert!(
            matches!(result, Err(ref error) if error.message()=="copy query identity exhausted")
        );
        let frame = execution.frames.current_mut(id).unwrap();
        assert_eq!(execution.slots.depth(&frame.window), 2);
        assert_eq!(execution.slots.peek(&frame.window, 0).unwrap(), &source);
        assert_eq!(frame.property_generation, u64::MAX);
        // Classification may already have completed ordinary fresh-target
        // definitions. No selected getter was called or replayed to discover it.
        assert!(
            runtime
                .get_own_property(&target, &runtime.intern_property_key("a").unwrap())
                .unwrap()
                .is_some()
        );
        assert!(
            runtime
                .get_own_property(&target, &runtime.intern_property_key("b").unwrap())
                .unwrap()
                .is_none()
        );
        drop(execution);
        assert_eq!(context.eval("copyCalls").unwrap(), Value::Int(0));
    }

    #[test]
    fn copy_transaction_sync_exhaustion_and_failure_keep_input_commit_boundary() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for fail in [false, true] {
            let source = context.eval("({a:1})").unwrap();
            let target = context
                .eval(if fail { "Object.freeze({})" } else { "({})" })
                .unwrap();
            let (mut execution, id) = fixture(&runtime, &mut context);
            execution
                .frames
                .current_mut(id)
                .unwrap()
                .property_generation = u64::MAX;
            push(&mut execution, id, target);
            push(&mut execution, id, source);
            let result = crate::engine::vm::proxy_get_driver::start_object_copy(
                &runtime,
                &mut execution,
                id,
                1,
                0,
                None,
            );
            if fail {
                assert!(
                    result.is_err()
                        || matches!(
                            result,
                            Ok(CallStep::Complete(crate::engine::vm::Completion::Throw(_)))
                        )
                );
            } else {
                assert!(matches!(result, Ok(CallStep::Entered)));
            }
            let frame = execution.frames.current_mut(id).unwrap();
            assert_eq!(
                execution.slots.depth(&frame.window),
                1,
                "source consumed before the first ordinary copy effect"
            );
            assert_eq!(frame.property_generation, u64::MAX);
        }
    }

    #[test]
    fn primitive_numeric_completion_keeps_bigint_operators_and_two_update_results() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        #[cfg(feature = "profiling")]
        let profile = crate::engine::api::profiling::CostProfile::start();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let x=5n, old=x++, next=++x, removed=x--, last=--x;
            let a=6n, b=3n;
            let values=[a*b,a/b,a%b,a**b,a<<b,a>>b,a&b,a|b,a^b,~a,-a];
            let errors='';
            try{a/0n}catch(e){errors+=e instanceof RangeError?'z':'!'}
            try{a*2}catch(e){errors+=e instanceof TypeError?'m':'!'}
            try{a>>>b}catch(e){errors+=e instanceof TypeError?'u':'!'}
            try{Symbol()*2}catch(e){errors+=e instanceof TypeError?'s':'!'}
            finally{errors+='f'}
            return values.join(',')==='18,2,0,216,48,0,2,7,5,-7,-6'
                && old===5n && next===7n && removed===7n && last===5n && x===5n
                && errors==='zmusf';
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        #[cfg(feature = "profiling")]
        assert!(
            profile
                .snapshot()
                .owned_execution_events
                .get("numeric_completed_in_same_frame")
                .copied()
                .unwrap_or(0)
                >= 10
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn object_numeric_fallback_keeps_conversion_order_throw_identity_and_finally() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let trace='', marker={}, left={valueOf(){trace+='l';return 6n}},
                right={valueOf(){trace+='r';return 3n}};
            if(left*right!==18n)throw 'product';
            try{({valueOf(){trace+='x';throw marker}})*right}catch(e){if(e===marker)trace+='t'}
            finally{trace+='f'}
            let target={valueOf(){trace+='p';return 8n}};
            let old=target++;
            const fixed={valueOf(){trace+='c';return 2n}};
            try{fixed++}catch(e){if(e instanceof TypeError)trace+='e'}
            return trace==='lrxtfpce' && old===8n && target===9n;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}
