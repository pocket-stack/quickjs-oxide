use super::*;
use crate::engine::api::Runtime;

#[test]
fn primitive_preparation_bypasses_host_and_preserves_representation() {
    let runtime = Runtime::new();
    let function = DetachedBytecode::<Value> {
        code: vec![],
        constants: vec![],
        local_count: 0,
        max_stack: 0,
    };
    let mut host = DetachedHost::new(&function);
    let values = [
        Value::Undefined,
        Value::Null,
        Value::Bool(true),
        Value::Int(i32::MIN),
        Value::Float(42.0),
        Value::Float(-0.0),
        Value::Float(f64::from_bits(0x7ff8_0000_0000_0042)),
        Value::Float(f64::INFINITY),
        Value::BigInt(JsBigInt::from(42_i32)),
        Value::String(JsString::try_from_utf16(vec![0xd800]).unwrap()),
        Value::Symbol(runtime.new_symbol(None).unwrap()),
    ];
    for hint in [
        ToPrimitiveHint::Default,
        ToPrimitiveHint::Number,
        ToPrimitiveHint::String,
    ] {
        for input in &values {
            let Completion::Return(output) = to_primitive(&mut host, input.clone(), hint).unwrap()
            else {
                panic!("primitive conversion threw");
            };
            match (input, output) {
                (Value::Float(expected), Value::Float(actual)) => {
                    assert_eq!(actual.to_bits(), expected.to_bits());
                }
                (expected, actual) => assert_eq!(&actual, expected),
            }
        }
    }
    assert!(host.to_primitive_inputs.is_empty());

    let object = Value::Object(runtime.new_object(None).unwrap());
    for hint in [ToPrimitiveHint::Default, ToPrimitiveHint::Number] {
        assert!(to_primitive(&mut host, object.clone(), hint).is_err());
        assert_eq!(
            host.to_primitive_inputs.last(),
            Some(&(object.clone(), hint))
        );
    }
    assert_eq!(host.to_primitive_inputs.len(), 2);
}

#[test]
fn numeric_instructions_use_primitive_preparation_without_host_calls() {
    for (instruction, operands) in [
        (Instruction::Neg, vec![Value::Int(2)]),
        (Instruction::Plus, vec![Value::Float(2.0)]),
        (Instruction::Inc, vec![Value::Int(i32::MAX)]),
        (Instruction::Dec, vec![Value::Float(-0.0)]),
        (Instruction::PostInc, vec![Value::Int(2)]),
        (Instruction::PostDec, vec![Value::Int(2)]),
        (Instruction::BitNot, vec![Value::Int(2)]),
        (Instruction::Add, vec![Value::Int(2), Value::Float(3.0)]),
        (Instruction::Sub, vec![Value::Int(2), Value::Int(3)]),
        (Instruction::Shr, vec![Value::Int(2), Value::Int(1)]),
        (Instruction::Lt, vec![Value::Int(2), Value::Int(3)]),
    ] {
        let mut code: Vec<_> = (0..operands.len())
            .map(|index| Instruction::PushConst(u32::try_from(index).unwrap()))
            .collect();
        code.extend([instruction, Instruction::Return]);
        let function = DetachedBytecode {
            code,
            constants: operands,
            local_count: 0,
            max_stack: 2,
        };
        let mut host = DetachedHost::new(&function);
        assert!(matches!(
            VmActivation::new(2).run(&function.code, &mut host).unwrap(),
            VmExit::Complete(Completion::Return(_))
        ));
        assert!(host.to_primitive_inputs.is_empty());
    }
}

#[test]
fn numeric_object_conversion_preserves_hints_order_and_abrupt_completion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval(r#"
        (() => {
            function check(expression, hint, expected) {
                let events = [];
                const left = { [Symbol.toPrimitive](h) { events.push('L:' + h); return 6; } };
                const right = { [Symbol.toPrimitive](h) { events.push('R:' + h); return 2; } };
                if (expression(left, right) !== expected || events.join(',') !== 'L:' + hint + ',R:' + hint)
                    throw new Error('conversion order or hint');
                events = [];
                const marker = {};
                const throws = { [Symbol.toPrimitive](h) { events.push('L:' + h); throw marker; } };
                try { expression(throws, right); throw new Error('missing throw'); }
                catch (e) { if (e !== marker) throw e; }
                if (events.join(',') !== 'L:' + hint) throw new Error('right converted after left throw');
            }
            check((a, b) => a + b, 'default', 8);
            check((a, b) => a - b, 'number', 4);
            check((a, b) => a >>> b, 'number', 1);
            check((a, b) => a < b, 'number', false);
            check((a, b) => a > b, 'number', true);
            check((a, b) => a <= b, 'number', false);
            check((a, b) => a >= b, 'number', true);
            let hints = [];
            const object = { [Symbol.toPrimitive](h) { hints.push(h); return '6'; } };
            if (+object !== 6 || -object !== -6 || ~object !== -7) return false;
            let value = object;
            if (value++ !== 6 || value !== 7) return false;
            value = object;
            if (--value !== 5 || hints.join(',') !== 'number,number,number,number,number') return false;
            let ordinary = [];
            if (({ valueOf() { ordinary.push('valueOf'); return {}; },
                   toString() { ordinary.push('toString'); return '4'; } }) - 1 !== 3) return false;
            return ordinary.join(',') === 'valueOf,toString';
        })()
    "#).unwrap(), Value::Bool(true));
}

#[test]
fn numeric_preparation_keeps_string_bigint_and_number_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval(r#"
        (() => {
            if ('1' + 2 !== '12' || '10' < '2' !== true || 1n + 2n !== 3n) return false;
            if ('6' - true !== 5 || null * 3 !== 0 || !Number.isNaN(undefined - 1)) return false;
            if (2 + 0.5 !== 2.5 || !Object.is(-0 * 1, -0) || !Object.is(-0 + -0, -0)) return false;
            if (NaN < 1 || NaN >= 1 || Infinity + 1 !== Infinity) return false;
            let value = '6';
            if (value++ !== 6 || value !== 7) return false;
            value = 2147483647;
            if (++value !== 2147483648) return false;
            for (const operation of [() => 1n + 2, () => +1n, () => Symbol() - 1, () => 1n >>> 1n]) {
                try { operation(); return false; }
                catch (e) { if (!(e instanceof TypeError)) throw e; }
            }
            return true;
        })()
    "#).unwrap(), Value::Bool(true));
}
