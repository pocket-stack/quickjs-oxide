use super::Runtime;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::metadata::FunctionMetadata;
use crate::engine::code::function::{UnlinkedConstant, UnlinkedFunction};
use crate::engine::value::{JsString, Value};

fn repeated_static_name_draft() -> UnlinkedFunction {
    UnlinkedFunction::fixture(
        vec![
            Instruction::Object,
            Instruction::GetField(0),
            Instruction::Drop,
            Instruction::Object,
            Instruction::GetField(0),
            Instruction::Drop,
            Instruction::PushConst(1),
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static(
                "static_key_lifetime_probe",
            )))
            .unwrap(),
            UnlinkedConstant::primitive(Value::String(JsString::from_static(
                "ordinary_constant_stays_a_string",
            )))
            .unwrap(),
        ],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
}

#[test]
fn static_property_keys_reuse_one_owned_atom_and_release_with_bytecode() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    let bytecode = runtime
        .publish_unlinked_function(context.realm, repeated_static_name_draft())
        .unwrap();
    let atom = {
        let state = runtime.0.state.borrow();
        let data = state
            .heap
            .function_bytecode(bytecode.bytecode_id())
            .unwrap();
        let keys = data.property_key_atoms.as_ref().unwrap();
        assert_eq!(
            keys.len(),
            1,
            "ordinary trailing constants need no map slot"
        );
        assert_eq!(data.auxiliary_atoms.as_ref(), keys.as_ref());
        assert_eq!(state.atoms.resolve(keys[0]).unwrap().ref_count, Some(1));
        keys[0]
    };
    assert_eq!(runtime.test_atom_count(), baseline_atoms + 1);
    #[cfg(feature = "profiling")]
    {
        let snapshot = runtime.memory_snapshot();
        let keys = snapshot
            .categories
            .iter()
            .find(|entry| entry.name == "bytecode_property_keys")
            .unwrap();
        assert_eq!(keys.count, Some(1));
        assert_eq!(
            keys.used_bytes,
            Some(size_of::<crate::engine::atom::Atom>())
        );
        assert_eq!(snapshot, runtime.memory_snapshot());
    }
    for _ in 0..3 {
        assert_eq!(
            context.execute(&bytecode).unwrap(),
            Value::String(JsString::from_static("ordinary_constant_stays_a_string"))
        );
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .atoms
                .resolve(atom)
                .unwrap()
                .ref_count,
            Some(1)
        );
    }
    drop(bytecode);
    runtime.run_gc().unwrap();
    assert!(runtime.0.state.borrow().atoms.resolve(atom).is_err());
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn static_property_keys_are_runtime_local_and_failed_publication_rolls_back() {
    let first = Runtime::new();
    let mut first_context = first.new_context();
    let second = Runtime::new();
    let second_context = second.new_context();
    let expired = first.new_context();
    let expired_realm = expired.realm;
    drop(expired);
    first.run_gc().unwrap();
    let baseline = first.test_atom_count();
    assert!(
        first
            .publish_unlinked_function(expired_realm, repeated_static_name_draft())
            .is_err()
    );
    assert_eq!(first.test_atom_count(), baseline);

    let a = first
        .publish_unlinked_function(first_context.realm, repeated_static_name_draft())
        .unwrap();
    let b = second
        .publish_unlinked_function(second_context.realm, repeated_static_name_draft())
        .unwrap();
    let a_atom = first
        .snapshot_function_bytecode(&a)
        .unwrap()
        .property_key_atoms
        .unwrap()[0];
    let b_atom = second
        .snapshot_function_bytecode(&b)
        .unwrap()
        .property_key_atoms
        .unwrap()[0];
    assert_ne!(a_atom, b_atom);
    assert!(first_context.execute(&b).is_err());
    let mut sibling = first.new_context();
    assert_eq!(
        sibling.execute(&a).unwrap(),
        Value::String(JsString::from_static("ordinary_constant_stays_a_string"))
    );
}

#[test]
fn static_property_keys_preserve_numeric_spellings_and_exact_utf16() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut names = ["0", "01", "2147483647", "2147483648", "属性"]
        .map(JsString::from_static)
        .to_vec();
    names.push(JsString::try_from_utf16(vec![0xd800]).unwrap());
    for name in names {
        let draft = UnlinkedFunction::fixture(
            vec![
                Instruction::Object,
                Instruction::PushI32(42),
                Instruction::DefineField(0),
                Instruction::GetField(0),
                Instruction::Return,
            ],
            vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
            FunctionMetadata {
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let bytecode = runtime
            .publish_unlinked_function(context.realm, draft)
            .unwrap();
        let atom = runtime
            .snapshot_function_bytecode(&bytecode)
            .unwrap()
            .property_key_atoms
            .unwrap()[0];
        assert_eq!(
            runtime.0.state.borrow().atoms.to_js_string(atom).unwrap(),
            name
        );
        assert_eq!(context.execute(&bytecode).unwrap(), Value::Int(42));
    }
    let plain = context.compile("42").unwrap();
    assert!(
        runtime
            .snapshot_function_bytecode(&plain)
            .unwrap()
            .property_key_atoms
            .is_none()
    );
}

#[test]
fn static_property_keys_keep_getters_proxies_and_resumed_frames_observable() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval(
                r#"
        (() => {
            let gets = 0;
            const parent = { get observed() { gets++; return this.value; } };
            const target = Object.create(parent);
            target.value = 2;
            let traps = 0;
            const proxy = new Proxy(target, {
                get(t, k, r) { traps++; return Reflect.get(t, k, r); }
            });
            function* values() {
                yield proxy.observed;
                yield proxy.observed;
            }
            const iterator = values();
            const first = iterator.next().value;
            target.value = 9;
            const second = iterator.next().value;
            Object.setPrototypeOf(target, { observed: 20 });
            const third = proxy.observed;
            return first === 2 && second === 9 && third === 20 && gets === 2 && traps === 5;
        })()
    "#
            )
            .unwrap(),
        Value::Bool(true)
    );
}
