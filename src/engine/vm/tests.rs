use crate::engine::api::error::{Error, ErrorKind};

use crate::engine::code::bytecode::{
    ArgumentsKind, DefineMethodKind, DetachedBytecode, DynamicEnvironmentSource,
    EvalVariableSource, Instruction, IteratorCallKind, WithObjectSource,
};
use crate::engine::value::{JsString, Value};

use super::{
    Completion, DefineClassOutcome, DetachedDynamicEnvironmentOperation,
    DetachedEvalVariableOperation, DetachedHost, DirectEvalInvocation, Vm, VmActivation, VmExit,
    VmHost, VmResume, VmSuspendKind, VmSuspension, number_to_int32, number_to_uint32,
};

#[test]
fn executes_arithmetic_stack_bytecode() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(6),
            Instruction::PushI32(7),
            Instruction::Mul,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };

    assert_eq!(Vm::new().execute(&function).unwrap(), Value::Int(42));
}

#[test]
fn dup3_clones_the_three_values_in_order_without_a_temporary_buffer() {
    for (drop_count, expected) in [(0, 3), (1, 2), (2, 1)] {
        let mut code = vec![
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::Dup3,
        ];
        code.extend(std::iter::repeat_n(Instruction::Drop, drop_count));
        code.push(Instruction::Return);
        let function = DetachedBytecode::<Value> {
            code,
            constants: vec![],
            local_count: 0,
            max_stack: 6,
        };

        assert_eq!(Vm::new().execute(&function).unwrap(), Value::Int(expected));
    }
}

#[test]
fn unary_arithmetic_preserves_quickjs_numeric_tags_and_float_bits() {
    fn execute(value: Value, instruction: Instruction) -> Value {
        Vm::new()
            .execute(&DetachedBytecode::<Value> {
                code: vec![Instruction::PushConst(0), instruction, Instruction::Return],
                constants: vec![value],
                local_count: 0,
                max_stack: 1,
            })
            .unwrap()
    }

    fn execute_post(value: Value, instruction: Instruction, selector: Instruction) -> Value {
        Vm::new()
            .execute(&DetachedBytecode::<Value> {
                code: vec![
                    Instruction::PushConst(0),
                    instruction,
                    selector,
                    Instruction::Return,
                ],
                constants: vec![value],
                local_count: 0,
                max_stack: 2,
            })
            .unwrap()
    }

    for (instruction, expected) in [
        (Instruction::Neg, (-42.0_f64).to_bits()),
        (Instruction::Plus, 42.0_f64.to_bits()),
        (Instruction::Inc, 43.0_f64.to_bits()),
        (Instruction::Dec, 41.0_f64.to_bits()),
    ] {
        let Value::Float(actual) = execute(Value::Float(42.0), instruction) else {
            panic!("integral Float64 unary result lost its Float tag");
        };
        assert_eq!(actual.to_bits(), expected);
    }

    let nan_bits = 0x7ff8_0000_0000_0042;
    let Value::Float(negated_nan) =
        execute(Value::Float(f64::from_bits(nan_bits)), Instruction::Neg)
    else {
        panic!("negated NaN lost its Float tag");
    };
    assert_eq!(negated_nan.to_bits(), nan_bits ^ (1_u64 << 63));

    for (value, instruction, expected) in [
        (0, Instruction::Neg, (-0.0_f64).to_bits()),
        (i32::MIN, Instruction::Neg, 2_147_483_648.0_f64.to_bits()),
        (i32::MAX, Instruction::Inc, 2_147_483_648.0_f64.to_bits()),
        (i32::MIN, Instruction::Dec, (-2_147_483_649.0_f64).to_bits()),
    ] {
        let Value::Float(actual) = execute(Value::Int(value), instruction) else {
            panic!("Int32 unary boundary did not promote to Float64");
        };
        assert_eq!(actual.to_bits(), expected);
    }

    for (instruction, old_bits, new_bits) in [
        (Instruction::PostInc, 42.0_f64.to_bits(), 43.0_f64.to_bits()),
        (Instruction::PostDec, 42.0_f64.to_bits(), 41.0_f64.to_bits()),
    ] {
        let Value::Float(old) =
            execute_post(Value::Float(42.0), instruction.clone(), Instruction::Drop)
        else {
            panic!("postfix Float64 old value lost its Float tag");
        };
        let Value::Float(new) = execute_post(Value::Float(42.0), instruction, Instruction::Nip)
        else {
            panic!("postfix Float64 replacement lost its Float tag");
        };
        assert_eq!(old.to_bits(), old_bits);
        assert_eq!(new.to_bits(), new_bits);
    }

    assert_eq!(
        execute_post(
            Value::Int(i32::MAX),
            Instruction::PostInc,
            Instruction::Drop
        ),
        Value::Int(i32::MAX)
    );
    let Value::Float(promoted) =
        execute_post(Value::Int(i32::MAX), Instruction::PostInc, Instruction::Nip)
    else {
        panic!("postfix Int32 boundary replacement did not promote to Float64");
    };
    assert_eq!(promoted.to_bits(), 2_147_483_648.0_f64.to_bits());
}

#[test]
fn generator_initial_yield_resumes_without_an_input_operand() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::InitialYield,
            Instruction::PushI32(42),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    let mut host = DetachedHost::new(&function);

    let VmExit::Suspend(suspension) = VmActivation::new(1).run(&function.code, &mut host).unwrap()
    else {
        panic!("initial_yield did not suspend");
    };
    assert_eq!(suspension.kind(), VmSuspendKind::Initial);

    assert_eq!(
        suspension
            .resume_initial(&function.code, &mut host)
            .unwrap(),
        VmExit::Complete(Completion::Return(Value::Int(42)))
    );
}

#[test]
fn generator_yield_snapshot_and_next_resume_preserve_quickjs_stack_abi() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(7),
            Instruction::Yield,
            Instruction::PushI32(0),
            Instruction::StrictEq,
            Instruction::IfFalse(6),
            Instruction::Return,
            Instruction::PushI32(-1),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 3,
    };
    let mut host = DetachedHost::new(&function);
    let VmExit::Suspend(mut suspension) =
        VmActivation::new(3).run(&function.code, &mut host).unwrap()
    else {
        panic!("yield did not suspend");
    };
    assert_eq!(suspension.kind(), VmSuspendKind::Yield);
    assert_eq!(suspension.take_yielded().unwrap(), Value::Int(7));

    let (kind, parts) = suspension.into_parts().unwrap();
    assert_eq!(kind, VmSuspendKind::Yield);
    assert_eq!(parts.pc, 2);
    assert_eq!(parts.stack, vec![Value::Undefined]);
    assert!(parts.regions.is_empty());
    let suspension = VmSuspension::from_parts(kind, parts).unwrap();

    assert_eq!(
        suspension
            .resume(&function.code, &mut host, VmResume::Next(Value::Int(42)))
            .unwrap(),
        VmExit::Complete(Completion::Return(Value::Int(42)))
    );
}

#[test]
fn generator_yield_return_resume_pushes_magic_one() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(7),
            Instruction::Yield,
            Instruction::PushI32(1),
            Instruction::StrictEq,
            Instruction::IfFalse(6),
            Instruction::Return,
            Instruction::PushI32(-1),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 3,
    };
    let mut host = DetachedHost::new(&function);
    let VmExit::Suspend(mut suspension) =
        VmActivation::new(3).run(&function.code, &mut host).unwrap()
    else {
        panic!("yield did not suspend");
    };
    assert_eq!(suspension.take_yielded().unwrap(), Value::Int(7));

    assert_eq!(
        suspension
            .resume(&function.code, &mut host, VmResume::Return(Value::Int(42)),)
            .unwrap(),
        VmExit::Complete(Completion::Return(Value::Int(42)))
    );
}

#[test]
fn generator_plain_yield_throw_enters_existing_unwind_path() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(4),
            Instruction::PushI32(7),
            Instruction::Yield,
            Instruction::Return,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    let mut host = DetachedHost::new(&function);
    let VmExit::Suspend(mut suspension) =
        VmActivation::new(2).run(&function.code, &mut host).unwrap()
    else {
        panic!("yield did not suspend");
    };
    assert_eq!(suspension.take_yielded().unwrap(), Value::Int(7));

    assert_eq!(
        suspension
            .resume(&function.code, &mut host, VmResume::Throw(Value::Int(55)),)
            .unwrap(),
        VmExit::Complete(Completion::Return(Value::Int(55)))
    );
}

#[test]
fn await_fulfilment_restores_one_expression_value_without_generator_magic() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(7),
            Instruction::Await,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    let mut host = DetachedHost::new(&function);
    let VmExit::Suspend(mut suspension) =
        VmActivation::new(1).run(&function.code, &mut host).unwrap()
    else {
        panic!("await did not suspend");
    };
    assert_eq!(suspension.kind(), VmSuspendKind::Await);
    assert_eq!(suspension.take_awaited().unwrap(), Value::Int(7));

    let (kind, parts) = suspension.into_parts().unwrap();
    assert_eq!(kind, VmSuspendKind::Await);
    assert_eq!(parts.pc, 2);
    assert_eq!(parts.stack, vec![Value::Undefined]);
    let suspension = VmSuspension::from_parts(kind, parts).unwrap();

    assert_eq!(
        suspension
            .resume_await_fulfill(&function.code, &mut host, Value::Int(42))
            .unwrap(),
        VmExit::Complete(Completion::Return(Value::Int(42)))
    );
}

#[test]
fn await_rejection_enters_existing_unwind_path() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(4),
            Instruction::PushI32(7),
            Instruction::Await,
            Instruction::Return,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    let mut host = DetachedHost::new(&function);
    let VmExit::Suspend(mut suspension) =
        VmActivation::new(2).run(&function.code, &mut host).unwrap()
    else {
        panic!("await did not suspend");
    };
    assert_eq!(suspension.take_awaited().unwrap(), Value::Int(7));

    assert_eq!(
        suspension
            .resume_await_reject(&function.code, &mut host, Value::Int(55))
            .unwrap(),
        VmExit::Complete(Completion::Return(Value::Int(55)))
    );
}

#[test]
fn generator_yield_return_runs_compiled_finally_unwind_path() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(7),
            Instruction::PushI32(7),
            Instruction::Yield,
            Instruction::IfFalse(7),
            Instruction::NipCatch,
            Instruction::Gosub(8),
            Instruction::Return,
            Instruction::Return,
            Instruction::PushI32(77),
            Instruction::Throw,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 3,
    };
    let mut host = DetachedHost::new(&function);
    let VmExit::Suspend(mut suspension) =
        VmActivation::new(3).run(&function.code, &mut host).unwrap()
    else {
        panic!("yield did not suspend");
    };
    assert_eq!(suspension.take_yielded().unwrap(), Value::Int(7));

    assert_eq!(
        suspension
            .resume(&function.code, &mut host, VmResume::Return(Value::Int(42)),)
            .unwrap(),
        VmExit::Complete(Completion::Throw(Value::Int(77)))
    );
    assert_eq!(host.captured_local_reuse_preparations, 1);
}

#[test]
fn generator_yield_star_throw_resume_injects_magic_two() {
    for (instruction, kind) in [
        (Instruction::YieldStar, VmSuspendKind::YieldStar),
        (Instruction::AsyncYieldStar, VmSuspendKind::AsyncYieldStar),
    ] {
        let function = DetachedBytecode::<Value> {
            code: vec![
                Instruction::PushI32(7),
                instruction,
                Instruction::PushI32(2),
                Instruction::StrictEq,
                Instruction::IfFalse(6),
                Instruction::Return,
                Instruction::PushI32(-1),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 3,
        };
        let mut host = DetachedHost::new(&function);
        let VmExit::Suspend(mut suspension) =
            VmActivation::new(3).run(&function.code, &mut host).unwrap()
        else {
            panic!("yield_star did not suspend");
        };
        assert_eq!(suspension.kind(), kind);
        assert_eq!(suspension.take_yielded().unwrap(), Value::Int(7));

        assert_eq!(
            suspension
                .resume(&function.code, &mut host, VmResume::Throw(Value::Int(55)),)
                .unwrap(),
            VmExit::Complete(Completion::Return(Value::Int(55)))
        );
    }
}

#[test]
fn yield_star_iterator_start_and_next_keep_an_ordinary_four_slot_record() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(10),
            Instruction::IteratorStart,
            Instruction::PushI32(42),
            Instruction::IteratorNext,
            Instruction::YieldStar,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 4,
    };
    let mut host = DetachedHost::new(&function);
    host.iterator_start_record = Some((Value::Int(1), Value::Int(2)));
    host.call_results
        .push_back(Ok(Completion::Return(Value::Int(99))));

    let VmExit::Suspend(mut suspension) =
        VmActivation::new(4).run(&function.code, &mut host).unwrap()
    else {
        panic!("yield_star iterator result did not suspend");
    };
    assert_eq!(suspension.kind(), VmSuspendKind::YieldStar);
    assert_eq!(suspension.take_yielded().unwrap(), Value::Int(99));
    let (_, parts) = suspension.into_parts().unwrap();
    assert_eq!(
        parts.stack,
        vec![
            Value::Int(1),
            Value::Int(2),
            Value::Undefined,
            Value::Undefined,
        ]
    );
    assert!(parts.regions.is_empty());
    assert_eq!(
        host.call_inputs,
        [(Value::Int(2), Value::Int(1), vec![Value::Int(42)])]
    );
}

#[test]
fn yield_star_iterator_call_uses_typed_method_and_argument_modes() {
    for (kind, method_name, arguments, result) in [
        (
            IteratorCallKind::ReturnWithValue,
            "return",
            vec![Value::Int(42)],
            90,
        ),
        (
            IteratorCallKind::ThrowWithValue,
            "throw",
            vec![Value::Int(42)],
            91,
        ),
        (IteratorCallKind::ReturnWithoutValue, "return", vec![], 92),
    ] {
        let function = DetachedBytecode::<Value> {
            code: vec![
                Instruction::PushI32(1),
                Instruction::PushI32(2),
                Instruction::Undefined,
                Instruction::PushI32(42),
                Instruction::IteratorCall(kind),
                Instruction::IfTrue(7),
                Instruction::Return,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 5,
        };
        let mut host = DetachedHost::new(&function);
        host.get_property_results
            .push_back(Completion::Return(Value::Int(7)));
        host.call_results
            .push_back(Ok(Completion::Return(Value::Int(result))));

        assert_eq!(
            VmActivation::new(5)
                .execute(&function.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(result))
        );
        assert_eq!(
            host.get_property_inputs,
            [(
                Value::Int(1),
                Value::String(JsString::from_static(method_name)),
            )]
        );
        assert_eq!(
            host.call_inputs,
            [(Value::Int(7), Value::Int(1), arguments)]
        );
    }

    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::Undefined,
            Instruction::PushI32(42),
            Instruction::IteratorCall(IteratorCallKind::ThrowWithValue),
            Instruction::IfTrue(7),
            Instruction::Return,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 5,
    };
    let mut host = DetachedHost::new(&function);
    host.get_property_results
        .push_back(Completion::Return(Value::Undefined));
    assert_eq!(
        VmActivation::new(5)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert!(host.call_inputs.is_empty());
}

#[test]
fn yield_star_iterator_protocol_errors_match_quickjs() {
    for (instruction, message) in [
        (
            Instruction::IteratorCheckObject,
            "iterator must return an object",
        ),
        (
            Instruction::ThrowIteratorMissingThrow,
            "iterator does not have a throw method",
        ),
    ] {
        let function = DetachedBytecode::<Value> {
            code: vec![Instruction::PushI32(1), instruction, Instruction::Return],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        let mut host = DetachedHost::new(&function);
        let error = VmActivation::new(1)
            .execute(&function.code, &mut host)
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Type);
        assert_eq!(error.message(), message);
    }
}

#[test]
fn class_definition_opcodes_preserve_quickjs_stack_order() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Undefined,
            Instruction::PushI32(7),
            Instruction::DefineClass {
                name: 0,
                has_heritage: false,
            },
            Instruction::Nip,
            Instruction::Return,
        ],
        constants: vec![Value::String(JsString::from_static("C"))],
        local_count: 0,
        max_stack: 2,
    };
    function.verify().unwrap();
    let mut host = DetachedHost::new(&function);
    host.define_class_results
        .push_back(DefineClassOutcome::Defined {
            constructor: Value::Int(11),
            prototype: Value::Int(42),
        });
    assert_eq!(
        VmActivation::new(2)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert_eq!(
        host.define_class_inputs,
        [(Value::Undefined, Value::Int(7), 0, false)]
    );

    let thrown = Value::String(JsString::from_static("class throw"));
    let mut host = DetachedHost::new(&function);
    host.define_class_results
        .push_back(DefineClassOutcome::Throw(thrown.clone()));
    assert_eq!(
        VmActivation::new(2)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Throw(thrown)
    );
}

#[test]
fn check_ctor_rejects_calls_and_accepts_construction_frames() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::CheckCtor,
            Instruction::PushI32(42),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    let mut host = DetachedHost::new(&function);
    let error = VmActivation::new(1)
        .execute(&function.code, &mut host)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Type);
    assert_eq!(
        error.message(),
        "class constructors must be invoked with 'new'"
    );

    let mut frame = VmActivation::new(1);
    frame.new_target = Value::Int(1);
    let mut host = DetachedHost::new(&function);
    assert_eq!(
        frame.execute(&function.code, &mut host).unwrap(),
        Completion::Return(Value::Int(42))
    );
}

#[test]
fn swap_exchanges_only_the_top_two_values() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(20),
            Instruction::PushI32(22),
            Instruction::Swap,
            Instruction::Sub,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    assert_eq!(Vm::new().execute(&function).unwrap(), Value::Int(2));
}

#[test]
fn tail_invocations_complete_the_frame_with_exact_call_operands() {
    let plain = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(10),
            Instruction::PushI32(11),
            Instruction::PushI32(12),
            Instruction::TailCall(2),
            Instruction::PushI32(-1),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 3,
    };
    plain.verify().unwrap();
    let mut host = DetachedHost::new(&plain);
    host.call_results
        .push_back(Ok(Completion::Return(Value::Int(42))));
    assert_eq!(
        VmActivation::new(3)
            .execute(&plain.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert_eq!(
        host.call_inputs,
        [(
            Value::Int(10),
            Value::Undefined,
            vec![Value::Int(11), Value::Int(12)]
        )]
    );

    let method = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(20),
            Instruction::PushI32(21),
            Instruction::PushI32(22),
            Instruction::PushI32(23),
            Instruction::TailCallMethod(2),
            Instruction::PushI32(-1),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 4,
    };
    method.verify().unwrap();
    let mut host = DetachedHost::new(&method);
    host.call_results
        .push_back(Ok(Completion::Return(Value::Int(43))));
    assert_eq!(
        VmActivation::new(4)
            .execute(&method.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(43))
    );
    assert_eq!(
        host.call_inputs,
        [(
            Value::Int(21),
            Value::Int(20),
            vec![Value::Int(22), Value::Int(23)]
        )]
    );
}

#[test]
fn tail_invocation_throws_use_the_activation_backtrace_and_catch_path() {
    let caught = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(4),
            Instruction::PushI32(7),
            Instruction::TailCall(0),
            Instruction::Drop,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    caught.verify().unwrap();
    let mut host = DetachedHost::new(&caught);
    host.call_results
        .push_back(Ok(Completion::Throw(Value::Int(77))));
    assert_eq!(
        VmActivation::new(2)
            .execute(&caught.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(77))
    );
    assert_eq!(host.backtrace_values, [Value::Int(77)]);
    assert_eq!(host.captured_local_reuse_preparations, 1);

    let uncaught = DetachedBytecode::<Value> {
        code: vec![Instruction::PushI32(8), Instruction::TailCall(0)],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    uncaught.verify().unwrap();
    let mut host = DetachedHost::new(&uncaught);
    host.call_results
        .push_back(Ok(Completion::Throw(Value::Int(88))));
    assert_eq!(
        VmActivation::new(1)
            .execute(&uncaught.code, &mut host)
            .unwrap(),
        Completion::Throw(Value::Int(88))
    );
    assert_eq!(host.backtrace_values, [Value::Int(88)]);
}

#[test]
fn dynamic_import_passes_raw_specifier_and_options_in_source_order() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(20),
            Instruction::PushI32(22),
            Instruction::Import,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    function.verify().unwrap();

    let mut host = DetachedHost::new(&function);
    host.dynamic_import_results
        .push_back(Ok(Completion::Return(Value::Int(42))));
    assert_eq!(
        VmActivation::new(2)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert_eq!(
        host.dynamic_import_inputs,
        [(Value::Int(20), Value::Int(22))]
    );

    let mut thrown = DetachedHost::new(&function);
    thrown
        .dynamic_import_results
        .push_back(Ok(Completion::Throw(Value::Int(77))));
    assert_eq!(
        VmActivation::new(2)
            .execute(&function.code, &mut thrown)
            .unwrap(),
        Completion::Throw(Value::Int(77))
    );
    assert_eq!(
        thrown.dynamic_import_inputs,
        [(Value::Int(20), Value::Int(22))]
    );

    let mut failed = DetachedHost::new(&function);
    failed
        .dynamic_import_results
        .push_back(Err(Error::internal("dynamic import host failure")));
    let error = VmActivation::new(2)
        .execute(&function.code, &mut failed)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert_eq!(error.message(), "dynamic import host failure");
    assert_eq!(
        failed.dynamic_import_inputs,
        [(Value::Int(20), Value::Int(22))]
    );
}

#[test]
fn eval_opcode_gates_original_identity_and_preserves_fallback_arguments() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(7),
            Instruction::PushI32(11),
            Instruction::PushI32(12),
            Instruction::Eval {
                argument_count: 2,
                environment: 17,
            },
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 3,
    };
    function.verify().unwrap();

    let mut original = DetachedHost::new(&function);
    original.eval_identity_results.push_back(Ok(true));
    original
        .direct_eval_results
        .push_back(Ok(Completion::Return(Value::Int(42))));
    assert_eq!(
        VmActivation::new(3)
            .execute(&function.code, &mut original)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert_eq!(
        original.direct_eval_inputs,
        [DirectEvalInvocation {
            input: Value::Int(11),
            environment: 17,
            this_value: Value::Undefined,
            new_target: Value::Undefined,
            caller_strict: true,
        }]
    );
    assert_eq!(original.eval_identity_inputs, [Value::Int(7)]);
    assert!(original.call_inputs.is_empty());

    let mut replacement = DetachedHost::new(&function);
    replacement.eval_identity_results.push_back(Ok(false));
    replacement
        .call_results
        .push_back(Ok(Completion::Return(Value::Int(43))));
    assert_eq!(
        VmActivation::new(3)
            .execute(&function.code, &mut replacement)
            .unwrap(),
        Completion::Return(Value::Int(43))
    );
    assert!(replacement.direct_eval_inputs.is_empty());
    assert_eq!(replacement.eval_identity_inputs, [Value::Int(7)]);
    assert_eq!(
        replacement.call_inputs,
        [(
            Value::Int(7),
            Value::Undefined,
            vec![Value::Int(11), Value::Int(12)]
        )]
    );

    let no_arguments = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Undefined,
            Instruction::Eval {
                argument_count: 0,
                environment: 29,
            },
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    let mut original = DetachedHost::new(&no_arguments);
    original.eval_identity_results.push_back(Ok(true));
    original
        .direct_eval_results
        .push_back(Ok(Completion::Return(Value::Undefined)));
    assert_eq!(
        VmActivation::new(1)
            .execute(&no_arguments.code, &mut original)
            .unwrap(),
        Completion::Return(Value::Undefined)
    );
    assert_eq!(
        original.direct_eval_inputs,
        [DirectEvalInvocation {
            input: Value::Undefined,
            environment: 29,
            this_value: Value::Undefined,
            new_target: Value::Undefined,
            caller_strict: true,
        }]
    );
    assert_eq!(original.eval_identity_inputs, [Value::Undefined]);
}

#[test]
fn string_direct_eval_forwards_environment_and_lazily_normalizes_this() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(7),
            Instruction::PushConst(0),
            Instruction::Eval {
                argument_count: 1,
                environment: 23,
            },
            Instruction::Return,
        ],
        constants: vec![Value::String(JsString::from_static("40 + 2"))],
        local_count: 0,
        max_stack: 2,
    };
    function.verify().unwrap();

    let mut host = DetachedHost::new(&function);
    host.eval_identity_results.push_back(Ok(true));
    host.box_primitive_results.push_back(Ok(Value::Int(99)));
    host.direct_eval_results
        .push_back(Ok(Completion::Return(Value::Int(42))));
    let mut frame = VmActivation::new(2);
    frame.this_value = Value::Int(8);
    frame.new_target = Value::Int(9);
    frame.strict = false;
    assert_eq!(
        frame.execute(&function.code, &mut host).unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert_eq!(host.box_primitive_inputs, [Value::Int(8)]);
    assert_eq!(
        host.direct_eval_inputs,
        [DirectEvalInvocation {
            input: Value::String(JsString::from_static("40 + 2")),
            environment: 23,
            this_value: Value::Int(99),
            new_target: Value::Int(9),
            caller_strict: false,
        }]
    );

    let non_string = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(7),
            Instruction::PushI32(42),
            Instruction::Eval {
                argument_count: 1,
                environment: u16::MAX,
            },
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    non_string.verify().unwrap();
    let mut host = DetachedHost::new(&non_string);
    host.eval_identity_results.push_back(Ok(true));
    host.direct_eval_results
        .push_back(Ok(Completion::Return(Value::Int(42))));
    let mut frame = VmActivation::new(2);
    frame.this_value = Value::Int(8);
    frame.strict = false;
    assert_eq!(
        frame.execute(&non_string.code, &mut host).unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert!(host.box_primitive_inputs.is_empty());
    assert_eq!(host.direct_eval_inputs[0].this_value, Value::Int(8));
    assert_eq!(host.direct_eval_inputs[0].environment, u16::MAX);
}

#[test]
fn arguments_opcode_forwards_kind_and_host_completion() {
    for kind in [ArgumentsKind::Mapped, ArgumentsKind::Unmapped] {
        let function = DetachedBytecode::<Value> {
            code: vec![Instruction::Arguments(kind), Instruction::Return],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        function.verify().unwrap();
        let mut host = DetachedHost::new(&function);
        host.arguments_results
            .push_back((kind, Completion::Return(Value::Int(42))));
        assert_eq!(
            VmActivation::new(1)
                .execute(&function.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(42))
        );
    }

    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Arguments(ArgumentsKind::Unmapped),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    let thrown = Value::String(JsString::from_static("arguments throw"));
    let mut host = DetachedHost::new(&function);
    host.arguments_results
        .push_back((ArgumentsKind::Unmapped, Completion::Throw(thrown.clone())));
    assert_eq!(
        VmActivation::new(1)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Throw(thrown)
    );
}

#[test]
fn rest_opcode_forwards_start_and_host_completion() {
    let function = DetachedBytecode::<Value> {
        code: vec![Instruction::Rest(2), Instruction::Return],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    function.verify().unwrap();

    let mut host = DetachedHost::new(&function);
    host.rest_results
        .push_back((2, Completion::Return(Value::Int(42))));
    assert_eq!(
        VmActivation::new(1)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );

    let thrown = Value::String(JsString::from_static("rest throw"));
    let mut host = DetachedHost::new(&function);
    host.rest_results
        .push_back((2, Completion::Throw(thrown.clone())));
    assert_eq!(
        VmActivation::new(1)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Throw(thrown)
    );
}

#[test]
fn eval_variable_object_opcodes_preserve_stack_and_host_operands() {
    let source = EvalVariableSource::Local(0);
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::VariableEnvironment,
            Instruction::PutLocal(0),
            Instruction::HasEvalVariable { source, name: 0 },
            Instruction::Drop,
            Instruction::GetEvalVariable { source, name: 0 },
            Instruction::Drop,
            Instruction::PushI32(7),
            Instruction::PutEvalVariable { source, name: 0 },
            Instruction::DeleteEvalVariable { source, name: 0 },
            Instruction::Drop,
            Instruction::PushI32(11),
            Instruction::DefineEvalVariable { source, name: 0 },
            Instruction::PushI32(42),
            Instruction::Return,
        ],
        constants: vec![Value::String(JsString::from_static("added"))],
        local_count: 1,
        max_stack: 1,
    };
    function.verify().unwrap();

    let environment = Value::String(JsString::from_static("variable environment"));
    let mut host = DetachedHost::new(&function);
    host.variable_environment_results
        .push_back(Completion::Return(environment.clone()));
    for value in [
        Value::Bool(true),
        Value::Int(3),
        Value::Undefined,
        Value::Bool(true),
        Value::Undefined,
    ] {
        host.eval_variable_results
            .push_back(Completion::Return(value));
    }
    assert_eq!(
        VmActivation::new(1)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert_eq!(host.get_local(0).unwrap(), environment);
    assert_eq!(
        host.eval_variable_operations,
        [
            DetachedEvalVariableOperation::Has(source, 0),
            DetachedEvalVariableOperation::Get(source, 0),
            DetachedEvalVariableOperation::Put(source, 0, Value::Int(7)),
            DetachedEvalVariableOperation::Delete(source, 0),
            DetachedEvalVariableOperation::Define(source, 0, Value::Int(11)),
        ]
    );
}

#[test]
fn to_object_boxes_primitives_and_rejects_nullish_values() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(7),
            Instruction::ToObject,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    function.verify().unwrap();
    let mut host = DetachedHost::new(&function);
    host.box_primitive_results.push_back(Ok(Value::Int(42)));
    assert_eq!(
        VmActivation::new(1)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert_eq!(host.box_primitive_inputs, [Value::Int(7)]);

    for nullish in [Instruction::Null, Instruction::Undefined] {
        let function = DetachedBytecode::<Value> {
            code: vec![nullish, Instruction::ToObject, Instruction::Return],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        function.verify().unwrap();
        let mut host = DetachedHost::new(&function);
        let error = VmActivation::new(1)
            .execute(&function.code, &mut host)
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Type);
        assert_eq!(error.message(), "cannot convert to object");
        assert!(host.box_primitive_inputs.is_empty());
    }
}

#[test]
fn dynamic_environment_opcodes_forward_sources_strictness_and_stack_values() {
    let source = DynamicEnvironmentSource::With(WithObjectSource::Local(0));
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::HasDynamicBinding { source, name: 0 },
            Instruction::Drop,
            Instruction::GetDynamicBinding { source, name: 0 },
            Instruction::Drop,
            Instruction::PushI32(7),
            Instruction::PutDynamicBinding { source, name: 0 },
            Instruction::DeleteDynamicBinding { source, name: 0 },
            Instruction::Drop,
            Instruction::DynamicEnvironmentObject(source),
            Instruction::GetRefValue(0),
            Instruction::PutRefValue(0),
            Instruction::DynamicEnvironmentObject(source),
            Instruction::GetRefValueUndef(0),
            Instruction::PutRefValue(0),
            Instruction::GlobalReference(7),
            Instruction::Drop,
            Instruction::PushI32(42),
            Instruction::Return,
        ],
        constants: vec![Value::String(JsString::from_static("binding"))],
        local_count: 1,
        max_stack: 2,
    };
    function.verify().unwrap();

    let first_environment = Value::String(JsString::from_static("first environment"));
    let second_environment = Value::String(JsString::from_static("second environment"));
    let mut host = DetachedHost::new(&function);
    for completion in [
        Completion::Return(Value::Bool(true)),
        Completion::Return(Value::Int(3)),
        Completion::Return(Value::Undefined),
        Completion::Return(Value::Bool(true)),
        Completion::Return(first_environment.clone()),
        Completion::Return(Value::Int(11)),
        Completion::Return(Value::Undefined),
        Completion::Return(second_environment.clone()),
        Completion::Return(Value::Undefined),
        Completion::Return(Value::Undefined),
        Completion::Return(Value::String(JsString::from_static("global reference"))),
    ] {
        host.dynamic_environment_results.push_back(completion);
    }
    assert_eq!(
        VmActivation::new(2)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert_eq!(
        host.dynamic_environment_operations,
        [
            DetachedDynamicEnvironmentOperation::Has(source, 0),
            DetachedDynamicEnvironmentOperation::Get(source, 0, true),
            DetachedDynamicEnvironmentOperation::Put(source, 0, Value::Int(7), true),
            DetachedDynamicEnvironmentOperation::Delete(source, 0),
            DetachedDynamicEnvironmentOperation::Object(source),
            DetachedDynamicEnvironmentOperation::GetRef(first_environment.clone(), 0, true,),
            DetachedDynamicEnvironmentOperation::PutRef(first_environment, 0, Value::Int(11), true,),
            DetachedDynamicEnvironmentOperation::Object(source),
            DetachedDynamicEnvironmentOperation::GetRef(second_environment.clone(), 0, false,),
            DetachedDynamicEnvironmentOperation::PutRef(
                second_environment,
                0,
                Value::Undefined,
                true,
            ),
            DetachedDynamicEnvironmentOperation::GlobalReference(7),
        ]
    );

    let throwing_reference = DetachedBytecode::<Value> {
        code: vec![Instruction::GlobalReference(3), Instruction::Return],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    throwing_reference.verify().unwrap();
    let thrown = Value::String(JsString::from_static("global reference throw"));
    let mut host = DetachedHost::new(&throwing_reference);
    host.dynamic_environment_results
        .push_back(Completion::Throw(thrown.clone()));
    assert_eq!(
        VmActivation::new(1)
            .execute(&throwing_reference.code, &mut host)
            .unwrap(),
        Completion::Throw(thrown)
    );
    assert_eq!(
        host.dynamic_environment_operations,
        [DetachedDynamicEnvironmentOperation::GlobalReference(3)]
    );
}

#[test]
fn detached_vm_catches_values_and_manages_private_handlers() {
    let thrown = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(3),
            Instruction::PushI32(7),
            Instruction::Throw,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    assert_eq!(Vm::new().execute(&thrown).unwrap(), Value::Int(7));

    let normal = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(4),
            Instruction::DropCatch,
            Instruction::PushI32(3),
            Instruction::Return,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    assert_eq!(Vm::new().execute(&normal).unwrap(), Value::Int(3));

    let nip = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(10),
            Instruction::Catch(6),
            Instruction::PushI32(20),
            Instruction::PushI32(30),
            Instruction::NipCatch,
            Instruction::Return,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 4,
    };
    assert_eq!(Vm::new().execute(&nip).unwrap(), Value::Int(30));

    let nested = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(7),
            Instruction::Catch(5),
            Instruction::PushI32(11),
            Instruction::Throw,
            Instruction::Nop,
            Instruction::Throw,
            Instruction::Nop,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 3,
    };
    assert_eq!(Vm::new().execute(&nested).unwrap(), Value::Int(11));
}

#[test]
fn iterator_unwind_preserves_exception_and_completion_precedence() {
    let pending_throw = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(6),
            Instruction::PushI32(1),
            Instruction::ForOfStart,
            Instruction::PushI32(41),
            Instruction::Throw,
            Instruction::Nop,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 5,
    };
    pending_throw.verify().unwrap();
    let mut host = DetachedHost::new(&pending_throw);
    host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
    // A close throw must not replace the already-pending value 41.
    host.iterator_close_results.push_back(Some(Value::Int(99)));
    assert_eq!(
        VmActivation::new(5)
            .execute(&pending_throw.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(41))
    );
    assert_eq!(host.iterator_close_pending, vec![true]);

    let normal_close_throw = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(7),
            Instruction::PushI32(1),
            Instruction::ForOfStart,
            Instruction::IteratorClose,
            Instruction::DropCatch,
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 4,
    };
    normal_close_throw.verify().unwrap();
    let mut host = DetachedHost::new(&normal_close_throw);
    host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
    host.iterator_close_results.push_back(Some(Value::Int(77)));
    assert_eq!(
        VmActivation::new(4)
            .execute(&normal_close_throw.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(77))
    );
    assert_eq!(host.iterator_close_pending, vec![false]);

    let preserve_close_throw = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(1),
            Instruction::ForOfStart,
            Instruction::PushI32(42),
            Instruction::IteratorClosePreserve,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 4,
    };
    preserve_close_throw.verify().unwrap();
    let mut host = DetachedHost::new(&preserve_close_throw);
    host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
    host.iterator_close_results.push_back(Some(Value::Int(88)));
    assert_eq!(
        VmActivation::new(4)
            .execute(&preserve_close_throw.code, &mut host)
            .unwrap(),
        Completion::Throw(Value::Int(88))
    );
    assert_eq!(host.iterator_close_pending, vec![false]);

    let drop_without_close = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(1),
            Instruction::ForOfStart,
            Instruction::PushI32(42),
            Instruction::IteratorDropPreserve,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 4,
    };
    drop_without_close.verify().unwrap();
    let mut host = DetachedHost::new(&drop_without_close);
    host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
    assert_eq!(
        VmActivation::new(4)
            .execute(&drop_without_close.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert!(host.iterator_close_pending.is_empty());
}

#[test]
fn for_of_next_disables_done_and_throwing_iterators() {
    let done = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(1),
            Instruction::ForOfStart,
            Instruction::ForOfNext(0),
            Instruction::Drop,
            Instruction::Drop,
            Instruction::IteratorClose,
            Instruction::PushI32(3),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 5,
    };
    done.verify().unwrap();
    let mut host = DetachedHost::new(&done);
    host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
    host.iterator_next_results
        .push_back(Ok((Value::Undefined, true)));
    assert_eq!(
        VmActivation::new(5).execute(&done.code, &mut host).unwrap(),
        Completion::Return(Value::Int(3))
    );
    assert!(host.iterator_close_pending.is_empty());

    let next_throw = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(10),
            Instruction::PushI32(1),
            Instruction::ForOfStart,
            Instruction::ForOfNext(0),
            Instruction::Drop,
            Instruction::Drop,
            Instruction::IteratorClose,
            Instruction::DropCatch,
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 6,
    };
    next_throw.verify().unwrap();
    let mut host = DetachedHost::new(&next_throw);
    host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
    host.iterator_next_results.push_back(Err(Value::Int(55)));
    assert_eq!(
        VmActivation::new(6)
            .execute(&next_throw.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(55))
    );
    assert!(host.iterator_close_pending.is_empty());
}

#[test]
fn array_literal_opcodes_preserve_operands_and_element_order() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::ArrayFrom(2),
            Instruction::PushI32(3),
            Instruction::DefineField(0),
            Instruction::PushI32(4),
            Instruction::PushI32(5),
            Instruction::DefineArrayEl,
            Instruction::Drop,
            Instruction::Return,
        ],
        constants: vec![Value::String(JsString::from_static("2"))],
        local_count: 0,
        max_stack: 3,
    };
    function.verify().unwrap();
    let mut host = DetachedHost::new(&function);
    host.array_from_results
        .push_back(Completion::Return(Value::String(JsString::from_static(
            "array",
        ))));
    host.define_field_results
        .push_back(Completion::Return(Value::Undefined));
    host.define_array_element_results
        .push_back(Completion::Return(Value::Undefined));
    assert_eq!(
        VmActivation::new(3)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("array")))
    );
    assert_eq!(host.array_from_inputs, [vec![Value::Int(1), Value::Int(2)]]);
    assert_eq!(
        host.defined_fields,
        [(
            Value::String(JsString::from_static("array")),
            0,
            Value::Int(3)
        )]
    );
    assert_eq!(
        host.defined_array_elements,
        [(
            Value::String(JsString::from_static("array")),
            Value::Int(4),
            Value::Int(5)
        )]
    );

    let dup1 = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::Dup1,
            Instruction::Add,
            Instruction::Add,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 3,
    };
    assert_eq!(Vm::new().execute(&dup1).unwrap(), Value::Int(4));
}

#[test]
fn object_literal_opcodes_preserve_target_and_operand_order() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Object,
            Instruction::Undefined,
            Instruction::DefineMethod {
                key: 0,
                kind: DefineMethodKind::Method,
                enumerable: true,
            },
            Instruction::PushI32(8),
            Instruction::Undefined,
            Instruction::DefineMethodComputed {
                kind: DefineMethodKind::Getter,
                enumerable: false,
            },
            Instruction::Null,
            Instruction::SetProto,
            Instruction::PushI32(7),
            Instruction::CopyDataProperties,
            Instruction::Return,
        ],
        constants: vec![Value::String(JsString::from_static("method"))],
        local_count: 0,
        max_stack: 3,
    };
    function.verify().unwrap();
    let object = Value::String(JsString::from_static("object"));
    let mut host = DetachedHost::new(&function);
    host.object_results
        .push_back(Completion::Return(object.clone()));
    host.define_method_results
        .push_back(Completion::Return(Value::Undefined));
    host.define_method_computed_results
        .push_back(Completion::Return(Value::Undefined));
    host.set_object_prototype_results
        .push_back(Completion::Return(Value::Undefined));
    host.copy_data_properties_results
        .push_back(Completion::Return(Value::Undefined));

    assert_eq!(
        VmActivation::new(3)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Return(object.clone())
    );
    assert_eq!(
        host.defined_methods,
        [(
            object.clone(),
            0,
            Value::Undefined,
            DefineMethodKind::Method,
            true
        )]
    );
    assert_eq!(
        host.defined_computed_methods,
        [(
            object.clone(),
            Value::Int(8),
            Value::Undefined,
            DefineMethodKind::Getter,
            false
        )]
    );
    assert_eq!(
        host.set_object_prototype_inputs,
        [(object.clone(), Value::Null)]
    );
    assert_eq!(host.copy_data_properties_inputs, [(object, Value::Int(7))]);
}

#[test]
fn object_rest_copy_reads_depth_operands_after_to_object_and_preserves_the_stack() {
    let excluded = Value::String(JsString::from_static("excluded"));
    let primitive_source = Value::String(JsString::from_static("ab"));
    let boxed_source = Value::String(JsString::from_static("boxed source"));
    let reference = Value::String(JsString::from_static("prepared reference"));
    let target = Value::String(JsString::from_static("target"));
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushConst(0),
            Instruction::PushConst(1),
            Instruction::ToObject,
            Instruction::PushConst(2),
            Instruction::PushConst(3),
            Instruction::CopyDataPropertiesExcluded {
                target_depth: 0,
                source_depth: 2,
                excluded_depth: 3,
            },
            Instruction::Drop,
            Instruction::Drop,
            Instruction::Drop,
            Instruction::Return,
        ],
        constants: vec![
            excluded.clone(),
            primitive_source.clone(),
            reference,
            target.clone(),
        ],
        local_count: 0,
        max_stack: 4,
    };
    function.verify().unwrap();
    let mut host = DetachedHost::new(&function);
    host.box_primitive_results
        .push_back(Ok(boxed_source.clone()));
    host.copy_data_properties_excluded_results
        .push_back(Completion::Return(Value::Undefined));

    assert_eq!(
        VmActivation::new(4)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Return(excluded.clone())
    );
    assert_eq!(host.box_primitive_inputs, [primitive_source]);
    assert_eq!(
        host.copy_data_properties_excluded_inputs,
        [(target, boxed_source, excluded)]
    );
}

#[test]
fn object_literal_opcodes_forward_host_throws() {
    let thrown = Value::String(JsString::from_static("literal throw"));

    let object = DetachedBytecode::<Value> {
        code: vec![Instruction::Object, Instruction::Return],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    object.verify().unwrap();
    let mut host = DetachedHost::new(&object);
    host.object_results
        .push_back(Completion::Throw(thrown.clone()));
    assert_eq!(
        VmActivation::new(1)
            .execute(&object.code, &mut host)
            .unwrap(),
        Completion::Throw(thrown.clone())
    );

    let proto = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(1),
            Instruction::Null,
            Instruction::SetProto,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    proto.verify().unwrap();
    let mut host = DetachedHost::new(&proto);
    host.set_object_prototype_results
        .push_back(Completion::Throw(thrown.clone()));
    assert_eq!(
        VmActivation::new(2)
            .execute(&proto.code, &mut host)
            .unwrap(),
        Completion::Throw(thrown.clone())
    );
    assert_eq!(
        host.set_object_prototype_inputs,
        [(Value::Int(1), Value::Null)]
    );

    let spread = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::CopyDataProperties,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    spread.verify().unwrap();
    let mut host = DetachedHost::new(&spread);
    host.copy_data_properties_results
        .push_back(Completion::Throw(thrown.clone()));
    assert_eq!(
        VmActivation::new(2)
            .execute(&spread.code, &mut host)
            .unwrap(),
        Completion::Throw(thrown.clone())
    );
    assert_eq!(
        host.copy_data_properties_inputs,
        [(Value::Int(1), Value::Int(2))]
    );

    let rest = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::PushI32(4),
            Instruction::CopyDataPropertiesExcluded {
                target_depth: 0,
                source_depth: 2,
                excluded_depth: 3,
            },
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 4,
    };
    rest.verify().unwrap();
    let mut host = DetachedHost::new(&rest);
    host.copy_data_properties_excluded_results
        .push_back(Completion::Throw(thrown.clone()));
    assert_eq!(
        VmActivation::new(4).execute(&rest.code, &mut host).unwrap(),
        Completion::Throw(thrown)
    );
    assert_eq!(
        host.copy_data_properties_excluded_inputs,
        [(Value::Int(4), Value::Int(2), Value::Int(1))]
    );
}

#[test]
fn append_uses_iterator_protocol_and_preserves_pending_throw_on_close() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushConst(0),
            Instruction::PushI32(0),
            Instruction::PushConst(1),
            Instruction::Append,
            Instruction::Drop,
            Instruction::Return,
        ],
        constants: vec![
            Value::String(JsString::from_static("array")),
            Value::String(JsString::from_static("iterable")),
        ],
        local_count: 0,
        max_stack: 3,
    };
    function.verify().unwrap();

    let mut success = DetachedHost::new(&function);
    success.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
    success
        .iterator_next_results
        .push_back(Ok((Value::Int(7), false)));
    success
        .iterator_next_results
        .push_back(Ok((Value::Int(8), false)));
    success
        .iterator_next_results
        .push_back(Ok((Value::Undefined, true)));
    success
        .define_array_element_results
        .push_back(Completion::Return(Value::Undefined));
    success
        .define_array_element_results
        .push_back(Completion::Return(Value::Undefined));
    assert_eq!(
        VmActivation::new(3)
            .execute(&function.code, &mut success)
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("array")))
    );
    assert_eq!(
        success.defined_array_elements,
        [
            (
                Value::String(JsString::from_static("array")),
                Value::Int(0),
                Value::Int(7)
            ),
            (
                Value::String(JsString::from_static("array")),
                Value::Int(1),
                Value::Int(8)
            )
        ]
    );
    assert!(success.iterator_close_pending.is_empty());

    let mut next_throw = DetachedHost::new(&function);
    next_throw.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
    next_throw
        .iterator_next_results
        .push_back(Err(Value::Int(55)));
    next_throw
        .iterator_close_results
        .push_back(Some(Value::Int(99)));
    assert_eq!(
        VmActivation::new(3)
            .execute(&function.code, &mut next_throw)
            .unwrap(),
        Completion::Throw(Value::Int(55))
    );
    assert_eq!(next_throw.iterator_close_pending, [true]);

    let mut define_throw = DetachedHost::new(&function);
    define_throw.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
    define_throw
        .iterator_next_results
        .push_back(Ok((Value::Int(7), false)));
    define_throw
        .define_array_element_results
        .push_back(Completion::Throw(Value::Int(66)));
    define_throw.iterator_close_results.push_back(None);
    assert_eq!(
        VmActivation::new(3)
            .execute(&function.code, &mut define_throw)
            .unwrap(),
        Completion::Throw(Value::Int(66))
    );
    assert_eq!(define_throw.iterator_close_pending, [true]);

    let invalid_index = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushConst(0),
            Instruction::PushConst(1),
            Instruction::PushConst(2),
            Instruction::Append,
            Instruction::Drop,
            Instruction::Return,
        ],
        constants: vec![
            Value::String(JsString::from_static("array")),
            Value::Float(0.0),
            Value::String(JsString::from_static("iterable")),
        ],
        local_count: 0,
        max_stack: 3,
    };
    invalid_index.verify().unwrap();
    let mut invalid = DetachedHost::new(&invalid_index);
    invalid.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
    assert_eq!(
        VmActivation::new(3)
            .execute(&invalid_index.code, &mut invalid)
            .unwrap_err()
            .message(),
        "invalid index for append"
    );
    assert_eq!(
        invalid.iterator_start_record,
        Some((Value::Int(10), Value::Int(11)))
    );
}

#[test]
fn detached_vm_rejects_array_allocation_without_runtime_intrinsics() {
    let function = DetachedBytecode::<Value> {
        code: vec![Instruction::ArrayFrom(0), Instruction::Return],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    assert_eq!(
        Vm::new().execute(&function).unwrap_err().message(),
        "detached VM cannot create runtime-owned Array objects"
    );
}

#[test]
fn iterator_region_above_gosub_address_closes_without_consuming_it() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(9),
            Instruction::Gosub(5),
            Instruction::Return,
            Instruction::Nop,
            Instruction::Nop,
            Instruction::PushI32(1),
            Instruction::ForOfStart,
            Instruction::IteratorClose,
            Instruction::Ret,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 5,
    };
    function.verify().unwrap();
    let mut host = DetachedHost::new(&function);
    host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
    host.iterator_close_results.push_back(None);
    assert_eq!(
        VmActivation::new(5)
            .execute(&function.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(9))
    );
    assert_eq!(host.iterator_close_pending, vec![false]);
}

#[test]
fn detached_vm_executes_typed_gosub_return_and_cleanup() {
    let returning = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(9),
            Instruction::Gosub(4),
            Instruction::Return,
            Instruction::Nop,
            Instruction::Ret,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    assert_eq!(Vm::new().execute(&returning).unwrap(), Value::Int(9));

    let abrupt = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(9),
            Instruction::Gosub(4),
            Instruction::Return,
            Instruction::Nop,
            Instruction::DropGosub,
            Instruction::Drop,
            Instruction::PushI32(4),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    assert_eq!(Vm::new().execute(&abrupt).unwrap(), Value::Int(4));

    let caught_inside_gosub = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(9),
            Instruction::Gosub(4),
            Instruction::Return,
            Instruction::Nop,
            Instruction::Catch(8),
            Instruction::PushI32(7),
            Instruction::Throw,
            Instruction::Nop,
            Instruction::Drop,
            Instruction::Ret,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 4,
    };
    assert_eq!(
        Vm::new().execute(&caught_inside_gosub).unwrap(),
        Value::Int(9)
    );
}

#[test]
fn captured_local_reuse_hook_is_limited_to_abrupt_resume_boundaries() {
    let return_unwind = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(4),
            Instruction::PushI32(9),
            Instruction::NipCatch,
            Instruction::Return,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    let mut host = DetachedHost::new(&return_unwind);
    assert_eq!(
        VmActivation::new(2)
            .execute(&return_unwind.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(9))
    );
    assert_eq!(host.captured_local_reuse_preparations, 1);

    let caught_throw = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(4),
            Instruction::PushI32(7),
            Instruction::Throw,
            Instruction::Nop,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    let mut host = DetachedHost::new(&caught_throw);
    assert_eq!(
        VmActivation::new(2)
            .execute(&caught_throw.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(7))
    );
    assert_eq!(host.captured_local_reuse_preparations, 1);

    let ordinary_gosub = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(5),
            Instruction::Gosub(4),
            Instruction::Return,
            Instruction::Nop,
            Instruction::Ret,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    let mut host = DetachedHost::new(&ordinary_gosub);
    assert_eq!(
        VmActivation::new(2)
            .execute(&ordinary_gosub.code, &mut host)
            .unwrap(),
        Completion::Return(Value::Int(5))
    );
    assert_eq!(host.captured_local_reuse_preparations, 0);

    let malformed_nip = DetachedBytecode::<Value> {
        code: vec![Instruction::PushI32(1), Instruction::NipCatch],
        constants: vec![],
        local_count: 0,
        max_stack: 1,
    };
    let mut host = DetachedHost::new(&malformed_nip);
    let error = VmActivation::new(1)
        .execute(&malformed_nip.code, &mut host)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert_eq!(host.captured_local_reuse_preparations, 0);
}

#[test]
fn runtime_ret_validation_is_an_uncatchable_engine_invariant() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::Catch(4),
            Instruction::Undefined,
            Instruction::Ret,
            Instruction::Nop,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };
    let mut host = DetachedHost::new(&function);
    let error = VmActivation::new(2)
        .execute(&function.code, &mut host)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert_eq!(error.message(), "invalid ret value");
}

#[test]
fn detached_vm_uses_the_declared_undefined_local_frame() {
    let initial = DetachedBytecode::<Value> {
        code: vec![Instruction::GetLocal(0), Instruction::Return],
        constants: vec![],
        local_count: 1,
        max_stack: 1,
    };
    assert_eq!(Vm::new().execute(&initial).unwrap(), Value::Undefined);

    let written = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(42),
            Instruction::PutLocal(0),
            Instruction::GetLocal(0),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 1,
        max_stack: 1,
    };
    assert_eq!(Vm::new().execute(&written).unwrap(), Value::Int(42));
}

#[test]
fn detached_vm_enforces_lexical_local_tdz_and_initialization() {
    let tdz = DetachedBytecode::<Value> {
        code: vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::GetLocalCheck(0),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 1,
        max_stack: 1,
    };
    let error = Vm::new().execute(&tdz).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Reference);
    assert_eq!(error.message(), "lexical variable is not initialized");

    let initialized = DetachedBytecode::<Value> {
        code: vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::PushI32(40),
            Instruction::InitializeLocal(0),
            Instruction::GetLocalCheck(0),
            Instruction::PushI32(2),
            Instruction::Add,
            Instruction::SetLocalCheck(0),
            Instruction::CloseLocal(0),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 1,
        max_stack: 2,
    };
    assert_eq!(Vm::new().execute(&initialized).unwrap(), Value::Int(42));

    let consuming_write = DetachedBytecode::<Value> {
        code: vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::Undefined,
            Instruction::InitializeLocal(0),
            Instruction::PushI32(42),
            Instruction::PutLocalCheck(0),
            Instruction::GetLocalCheck(0),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 1,
        max_stack: 1,
    };
    assert_eq!(Vm::new().execute(&consuming_write).unwrap(), Value::Int(42));
}

#[test]
fn detached_vm_enforces_derived_this_one_shot_and_return_shape() {
    let duplicate_super = DetachedBytecode::<Value> {
        code: vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::PushI32(1),
            Instruction::InitializeDerivedLocal(0),
            Instruction::PushI32(2),
            Instruction::InitializeDerivedLocal(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 1,
        max_stack: 1,
    };
    let error = Vm::new().execute(&duplicate_super).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Reference);
    assert_eq!(error.message(), "'this' can be initialized only once");

    let primitive_return = DetachedBytecode::<Value> {
        code: vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::PushI32(1),
            Instruction::ReturnDerived(0),
        ],
        constants: vec![],
        local_count: 1,
        max_stack: 1,
    };
    let error = Vm::new().execute(&primitive_return).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Type);
    assert_eq!(
        error.message(),
        "derived class constructor must return an object or undefined"
    );

    let missing_super = DetachedBytecode::<Value> {
        code: vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::Undefined,
            Instruction::ReturnDerived(0),
        ],
        ..primitive_return
    };
    let error = Vm::new().execute(&missing_super).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Reference);
    assert_eq!(error.message(), "lexical variable is not initialized");
}

#[test]
fn detached_vm_rejects_checked_writes_in_the_tdz_and_allows_plain_reinitialization() {
    for (write, preserves_value) in [
        (Instruction::PutLocalCheck(0), false),
        (Instruction::SetLocalCheck(0), true),
    ] {
        let mut code = vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::PushI32(1),
            write,
        ];
        if !preserves_value {
            code.push(Instruction::Undefined);
        }
        code.push(Instruction::Return);
        let function = DetachedBytecode::<Value> {
            code,
            constants: vec![],
            local_count: 1,
            max_stack: 1,
        };
        let error = Vm::new().execute(&function).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Reference);
        assert_eq!(error.message(), "lexical variable is not initialized");
    }

    let twice = DetachedBytecode::<Value> {
        code: vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::PushI32(1),
            Instruction::InitializeLocal(0),
            Instruction::PushI32(2),
            Instruction::InitializeLocal(0),
            Instruction::GetLocalCheck(0),
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 1,
        max_stack: 1,
    };
    assert_eq!(Vm::new().execute(&twice).unwrap(), Value::Int(2));
}

#[test]
fn detached_membership_rejects_primitive_right_operands_before_host_dispatch() {
    for (operator, message) in [
        (Instruction::In, "invalid 'in' operand"),
        (
            Instruction::InstanceOf,
            "invalid 'instanceof' right operand",
        ),
    ] {
        let function = DetachedBytecode::<Value> {
            code: vec![
                Instruction::PushI32(1),
                Instruction::PushI32(2),
                operator,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        let error = Vm::new().execute(&function).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Type);
        assert_eq!(error.message(), message);
    }
}

#[test]
fn executes_power_stack_bytecode_and_quickjs_number_edges() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::PushI32(2),
            Instruction::Pow,
            Instruction::Pow,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 3,
    };

    assert_eq!(Vm::new().execute(&function).unwrap(), Value::Int(512));
    assert!(crate::engine::value::number::pow(1.0, f64::INFINITY).is_nan());
    assert!(crate::engine::value::number::pow(-1.0, f64::NEG_INFINITY).is_nan());
    assert!(crate::engine::value::number::pow(-2.0, 0.5).is_nan());
    assert_eq!(crate::engine::value::number::pow(f64::NAN, 0.0), 1.0);
    assert_eq!(crate::engine::value::number::pow(2.0, -2.0), 0.25);
    assert_eq!(
        crate::engine::value::number::pow(-0.0, 3.0).to_bits(),
        (-0.0f64).to_bits()
    );
    assert_eq!(
        crate::engine::value::number::pow(-0.0, -3.0),
        f64::NEG_INFINITY
    );
}

#[test]
fn executes_string_addition() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushConst(0),
            Instruction::PushConst(1),
            Instruction::Add,
            Instruction::Return,
        ],
        constants: vec![
            Value::String(JsString::from_static("quick")),
            Value::String(JsString::from_static("js")),
        ],
        local_count: 0,
        max_stack: 2,
    };

    assert_eq!(
        Vm::new().execute(&function).unwrap(),
        Value::String(JsString::from_static("quickjs"))
    );
}

#[test]
fn string_addition_builds_ropes_and_reports_the_quickjs_length_error() {
    let chunk = JsString::try_from_utf8(&"x".repeat(8193)).unwrap();
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushConst(0),
            Instruction::PushConst(1),
            Instruction::Add,
            Instruction::Return,
        ],
        constants: vec![Value::String(chunk.clone()), Value::String(chunk.clone())],
        local_count: 0,
        max_stack: 2,
    };
    let Value::String(rope) = Vm::new().execute(&function).unwrap() else {
        panic!("large String addition did not return a String");
    };
    assert_eq!(rope.len(), 16_386);
    assert!(!rope.is_flat());

    let mut near_limit = chunk;
    for _ in 0..16 {
        near_limit = near_limit.try_concat(&near_limit).unwrap();
    }
    let overflow = DetachedBytecode::<Value> {
        code: function.code,
        constants: vec![Value::String(near_limit.clone()), Value::String(near_limit)],
        local_count: 0,
        max_stack: 2,
    };
    let error = Vm::new().execute(&overflow).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "string too long");
}

#[test]
fn executes_bitwise_stack_bytecode() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(0b1010),
            Instruction::PushI32(0b1100),
            Instruction::BitAnd,
            Instruction::BitNot,
            Instruction::PushI32(0b0011),
            Instruction::BitXor,
            Instruction::PushI32(0b0100),
            Instruction::BitOr,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };

    assert_eq!(Vm::new().execute(&function).unwrap(), Value::Int(-12));
}

#[test]
fn to_int32_uses_ecmascript_modulo_semantics() {
    for (value, expected) in [
        (0.0, 0),
        (-0.0, 0),
        (f64::NAN, 0),
        (f64::INFINITY, 0),
        (f64::NEG_INFINITY, 0),
        (1.9, 1),
        (-1.9, -1),
        (2_147_483_647.0, i32::MAX),
        (2_147_483_648.0, i32::MIN),
        (4_294_967_295.0, -1),
        (4_294_967_296.0, 0),
        (4_294_967_297.0, 1),
        (-2_147_483_649.0, i32::MAX),
        (1.0e300, 0),
    ] {
        assert_eq!(number_to_int32(value), expected, "input {value:?}");
    }

    for (value, expected) in [
        (-1.0, u32::MAX),
        (2_147_483_648.0, 2_147_483_648),
        (4_294_967_295.0, u32::MAX),
        (4_294_967_296.0, 0),
        (-4_294_967_295.0, 1),
    ] {
        assert_eq!(number_to_uint32(value), expected, "input {value:?}");
    }
}

#[test]
fn executes_shift_stack_bytecode() {
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushI32(-8),
            Instruction::PushI32(1),
            Instruction::Sar,
            Instruction::PushI32(1),
            Instruction::Shr,
            Instruction::PushI32(2),
            Instruction::Shl,
            Instruction::Return,
        ],
        constants: vec![],
        local_count: 0,
        max_stack: 2,
    };

    assert_eq!(Vm::new().execute(&function).unwrap(), Value::Int(-8));
}
