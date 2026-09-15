//! Published execution must retain dynamic binding transitions after static
//! access-mode checks move to publication. These tests use the real compiler,
//! publisher and runtime host, not synthetic instruction fixtures.
use crate::engine::api::runtime::Runtime;
use crate::engine::value::Value;

#[test]
fn invalid_binding_modes_are_rejected_before_creating_a_runtime_frame() {
    use crate::engine::code::bytecode::Instruction;
    use crate::engine::code::function::metadata::FunctionMetadata;
    use crate::engine::code::function::{UnlinkedFunction, UnlinkedVariableDefinition};
    use crate::engine::value::JsString;

    let cases = [
        (
            Instruction::GetLocal(0),
            UnlinkedVariableDefinition::lexical(Some(JsString::from_static("n")), false),
            "unchecked local opcode referenced a lexical definition",
        ),
        (
            Instruction::GetLocalCheck(0),
            UnlinkedVariableDefinition::ordinary(None),
            "checked lexical-local opcode referenced an ordinary definition",
        ),
    ];
    for (instruction, definition, expected) in cases {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let before = runtime.heap_counts().function_bytecode_nodes;
        let function = UnlinkedFunction::fixture(
            vec![instruction, Instruction::Return],
            vec![],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_fixture_definitions(vec![], vec![definition]);
        let error = runtime
            .publish_unlinked_function(context.realm, function)
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, before);
    }
}

#[test]
fn published_bindings_keep_capture_eval_and_argument_aliases_live() {
    for (source, expected) in [
        (
            "(function(){var n=2; n+=3; var f=()=>n; n+=7; return f();})()",
            12,
        ),
        (
            "(function(){var n=2; eval('var f=()=>n'); n=7; return f();})()",
            7,
        ),
        (
            "(function(a){arguments[0]=7; return a+arguments.length;})(1,2,3)",
            10,
        ),
        (
            "(function(a){'use strict'; arguments[0]=7; return a+arguments.length;})(1,2,3)",
            4,
        ),
    ] {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert!(
            matches!(context.eval(source).unwrap(), Value::Int(n) if n == expected),
            "{source}"
        );
    }
}

#[test]
fn published_lexical_reads_preserve_tdz_then_observe_initialization() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let value = context
        .eval(
            "(function(){
        var f=()=>n; var caught=0;
        try { f(); } catch(e) { if (!(e instanceof ReferenceError)) throw e; caught=1; }
        let n=41; return f()+caught;
    })()",
        )
        .unwrap();
    assert!(matches!(value, Value::Int(42)));
}

#[test]
fn published_resume_keeps_captured_cells_live_through_finally() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let value = context
        .eval(
            "(function(){
        function* g(){let n=2; let f=()=>n; yield f; try {yield ++n;} finally {n=9;}}
        var it=g(); var f=it.next().value;
        if(f()!==2 || it.next().value!==3) throw Error('resume');
        it.return(0); return f()+1;
    })()",
        )
        .unwrap();
    assert!(matches!(value, Value::Int(10)));
}

#[test]
fn published_lexical_writes_preserve_tdz_const_and_iteration_lifetimes() {
    for source in [
        "(function(){let x=1; x=4; return x===4;})()",
        "(function(){let x=1; const set=v=>x=v; set(4); return x===4;})()",
        "(function(){const set=v=>x=v; let caught=false; try{set(4)}catch(e){caught=e instanceof ReferenceError} let x=1; set(5); return caught&&x===5;})()",
        "(function(){let caught=false; try{x=4}catch(e){caught=e instanceof ReferenceError} let x=1; return caught&&x===1;})()",
        "(function(){const x=1; try{x=4}catch(e){return e instanceof TypeError&&x===1} return false;})()",
        "(function(){const x=1; const set=v=>x=v; try{set(4)}catch(e){return e instanceof TypeError&&x===1} return false;})()",
        "(function(){let fs=[]; for(let i=0;i<3;i++){let x=i; fs.push(()=>++x)} return fs[0]()===1&&fs[1]()===2&&fs[2]()===3&&fs[0]()===2;})()",
        "(function(){let x=1; eval('x=4'); return x===4;})()",
    ] {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert!(
            matches!(context.eval(source).unwrap(), Value::Bool(true)),
            "{source}"
        );
    }
}

#[test]
fn published_eval_reuses_topology_but_observes_live_scope_and_super() {
    for source in [
        "(function(a){let x=2; const f=()=>eval('x+=a'); f(); return x===5;})(3)",
        "(function(){let x=1; let o={x:4}; with(o){eval('x+=2')} return x===1&&o.x===6;})()",
        "(function(){let fs=[]; for(let i=0;i<3;i++){let x=i; fs.push(eval('()=>++x'))} return fs[0]()===1&&fs[1]()===2&&fs[2]()===3;})()",
        "(function(){class A{m(){return 2}} class B extends A{m(){return eval('super.m()')+1}} return new B().m()===3;})()",
        "(function(){class A{constructor(){this.n=3}} class B extends A{constructor(){eval('super()')}} return new B().n===3;})()",
        "(function(){let x=1; eval(\"eval('x=4')\"); return x===4;})()",
    ] {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert!(
            matches!(context.eval(source).unwrap(), Value::Bool(true)),
            "{source}"
        );
    }
}

#[test]
fn instruction_fetch_rejects_invalid_pc_before_advancing_it() {
    use super::{DetachedHost, VmActivation};
    use crate::engine::code::bytecode::{DetachedBytecode, Instruction};
    let function = DetachedBytecode::<Value> {
        code: vec![Instruction::Nop],
        constants: vec![],
        local_count: 0,
        max_stack: 0,
    };
    for pc in [0, 1, usize::MAX] {
        let mut host = DetachedHost::new(&function);
        let mut activation = VmActivation::new(0);
        activation.pc = pc;
        let error = activation
            .execute_inner(&function.code, &mut host)
            .err()
            .expect("invalid PC must fail at instruction fetch");
        assert_eq!(error.message(), "bytecode ended without return");
        assert_eq!(activation.pc, if pc == 0 { 1 } else { pc });
    }
}

#[test]
fn repeated_closure_creation_reuses_cells_without_erasing_their_metadata() {
    for source in [
        "(function(){let x=1; let a=()=>x; let b=()=>++x; return b()===2&&a()===2;})()",
        "(function(){let fs=[]; for(let i=0;i<3;i++){let x=i; fs.push(()=>x,()=>++x)} return fs[1]()===1&&fs[0]()===1&&fs[3]()===2&&fs[2]()===2;})()",
        "(function named(){let a=()=>named; let b=eval('()=>named'); named=1; return a()===b()&&typeof a()==='function';})()",
        "(function(){class A{#x=3; read(){return [()=>this.#x,()=>this.#x]}} let fs=new A().read(); return fs[0]()===3&&fs[1]()===3;})()",
    ] {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert!(
            matches!(context.eval(source).unwrap(), Value::Bool(true)),
            "{source}"
        );
    }
}

#[test]
fn static_branch_targets_remain_checked_at_untrusted_boundaries() {
    use super::{DetachedHost, VmActivation};
    use crate::engine::code::bytecode::{DetachedBytecode, Instruction};
    use crate::engine::code::function::UnlinkedFunction;
    use crate::engine::code::function::metadata::FunctionMetadata;

    // Even an unreachable malformed operand is rejected by publication.
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let draft = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Goto(u32::MAX),
        ],
        vec![],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    let error = runtime
        .publish_unlinked_function(context.realm, draft)
        .unwrap_err();
    assert!(
        error.to_string().contains("jump target is out of bounds"),
        "{error}"
    );

    for branch in [
        Instruction::Goto(2),
        Instruction::IfTrue(2),
        Instruction::IfFalse(2),
    ] {
        let function = DetachedBytecode::<Value> {
            code: vec![branch.clone()],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        let mut host = DetachedHost::new(&function);
        let mut activation = VmActivation::new(1);
        activation
            .stack
            .push(Value::Bool(!matches!(branch, Instruction::IfFalse(_))));
        let error = activation
            .execute_inner(&function.code, &mut host)
            .err()
            .unwrap();
        assert_eq!(error.message(), "jump target is out of bounds");
    }
}

#[test]
fn synthetic_runtime_host_checks_static_targets() {
    use super::VmHost;
    use super::host_bridge::RuntimeVmHost;
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let host = RuntimeVmHost::empty_for_test(runtime.clone(), context.realm);
    assert_eq!(host.static_branch_target(0, 1).unwrap(), 0);
    for (target, length) in [(0, 0), (1, 1), (u32::MAX, 1)] {
        assert_eq!(
            host.static_branch_target(target, length)
                .unwrap_err()
                .message(),
            "jump target is out of bounds"
        );
    }
}

#[test]
fn published_static_branches_preserve_resume_finally_and_loop_targets() {
    for source in [
        "(function(){let n=0; do {++n} while(n<3); while(n>1){--n} return n===1;})()",
        "(function(){let n=0; for(let i=0;i<5;i++){try{if(i%2)continue; n+=i;}finally{++n;}}return n===11;})()",
        "(function(){function* g(){let n=0;try{while(n<3){yield n++;}}finally{n=9;}return n;}let it=g();return it.next().value===0&&it.next().value===1&&it.return(7).value===7;})()",
    ] {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert!(
            matches!(context.eval(source).unwrap(), Value::Bool(true)),
            "{source}"
        );
    }
}

#[test]
fn paired_stack_reads_preserve_order_and_root_cleanup_on_every_length() {
    use super::VmActivation;
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut activation = VmActivation::new(3);
    assert_eq!(
        activation.pop_pair().unwrap_err().message(),
        "bytecode stack underflow"
    );
    let single = context.new_object().unwrap();
    let single_id = single.object_id();
    activation.stack.push(Value::Object(single));
    assert_eq!(
        activation.pop_pair().unwrap_err().message(),
        "bytecode stack underflow"
    );
    assert!(activation.stack.is_empty());
    assert!(runtime.0.state.borrow().heap.object(single_id).is_err());

    let left = context.new_object().unwrap();
    let right = context.new_object().unwrap();
    let left_id = left.object_id();
    let right_id = right.object_id();
    activation
        .stack
        .extend([Value::Int(7), Value::Object(left), Value::Object(right)]);
    let (left, right) = activation.pop_pair().unwrap();
    assert_eq!(activation.stack, [Value::Int(7)]);
    assert!(matches!(&left, Value::Object(root) if root.object_id() == left_id));
    assert!(matches!(&right, Value::Object(root) if root.object_id() == right_id));
    assert!(runtime.0.state.borrow().heap.object(left_id).is_ok());
    assert!(runtime.0.state.borrow().heap.object(right_id).is_ok());
    drop((left, right));
    assert!(runtime.0.state.borrow().heap.object(left_id).is_err());
    assert!(runtime.0.state.borrow().heap.object(right_id).is_err());
}

#[test]
fn depth_stack_reads_select_from_the_tail_and_retain_roots() {
    use super::VmActivation;
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = context.new_object().unwrap();
    let id = object.object_id();
    let mut activation = VmActivation::new(2);
    activation
        .stack
        .extend([Value::Object(object), Value::Int(9)]);
    assert_eq!(activation.clone_at_depth(0).unwrap(), Value::Int(9));
    let saved = activation.clone_at_depth(1).unwrap();
    for depth in [2, u8::MAX] {
        assert_eq!(
            activation.clone_at_depth(depth).unwrap_err().message(),
            "bytecode stack depth operand is out of bounds"
        );
        assert_eq!(activation.stack.len(), 2);
    }
    drop(activation);
    assert!(runtime.0.state.borrow().heap.object(id).is_ok());
    drop(saved);
    assert!(runtime.0.state.borrow().heap.object(id).is_err());
}

#[test]
fn depth_stack_reads_cover_the_entire_bytecode_operand_range() {
    use super::VmActivation;
    for length in [0, 1, 255, 256, 257] {
        let mut activation = VmActivation::new(length);
        activation
            .stack
            .extend((0..length).map(|index| Value::Int(index as i32)));
        for depth in 0..=u8::MAX {
            let result = activation.clone_at_depth(depth);
            if usize::from(depth) < length {
                assert_eq!(
                    result.unwrap(),
                    Value::Int((length - usize::from(depth) - 1) as i32)
                );
            } else {
                assert_eq!(
                    result.unwrap_err().message(),
                    "bytecode stack depth operand is out of bounds"
                );
            }
            assert_eq!(activation.stack.len(), length);
        }
    }
}
