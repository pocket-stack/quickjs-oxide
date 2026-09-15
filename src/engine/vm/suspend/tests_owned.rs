//! S06 coverage exercises complete callback chains, not only opcode execution.
use crate::engine::{
    api::{profiling::CostProfile, runtime::Runtime},
    value::Value,
};

fn assert_owned(source: &str, assertion: &str) {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let profile = CostProfile::start();
    context.eval(source).unwrap();
    let mut jobs = 0;
    while runtime.is_job_pending() {
        runtime.run_gc().unwrap();
        runtime.execute_pending_job().unwrap();
        jobs += 1;
        assert!(jobs < 1000, "unexpected job loop");
    }
    runtime.run_gc().unwrap();
    assert_eq!(context.eval(assertion).unwrap(), Value::Bool(true));
    let cost = profile.snapshot();
    assert_eq!(cost.legacy_dispatches, 0, "{cost:?}");
    assert_eq!(cost.owned_bridge_exits, 0, "{cost:?}");
    assert_eq!(cost.owned_sync_call_bridges, 0, "{cost:?}");
}

#[test]
fn promise_aggregate_finally_and_convenience_callbacks_remain_owned() {
    assert_owned(
        r#"
        var results=[], closed=0, calls=0;
        class P extends Promise {
            constructor(executor) { super((a,b)=>{calls++;executor(a,b)}); }
            static get [Symbol.species]() { return this; }
            static resolve(v) { return super.resolve(v); }
        }
        P.all([1,{then(r){r(2)}}]).finally(()=>3).then(v=>results.push('all:'+v));
        P.allSettled([1,P.reject(2)]).then(v=>results.push('settled:'+v[1].reason));
        P.any([P.reject(1),2]).then(v=>results.push('any:'+v));
        P.race([3,4]).then(v=>results.push('race:'+v));
        P.try((a,b)=>a+b,2,3).then(v=>results.push('try:'+v));
        var cap=P.withResolvers();cap.resolve(6);cap.promise.then(v=>results.push('cap:'+v));
        class Bad extends P { static resolve(){throw 7} }
        Bad.all({[Symbol.iterator](){return {next(){return {value:1}},return(){closed++;throw 8}}}}).catch(v=>results.push('close:'+v));
    "#,
        "results.sort().join('|')==='all:1,2|any:2|cap:6|close:7|race:3|settled:2|try:5' && closed===1 && calls>0",
    );
}

#[test]
fn async_generator_queue_private_capture_finally_and_sync_iteration_remain_owned() {
    assert_owned(
        r#"
        var results=[], cleaned=0, total=0;
        class Box { #n=40; async *values(a) { a++; try {yield this.#n+await Promise.resolve(a)} finally {await 0;cleaned++;yield 2} } }
        var iterator=new Box().values(0);
        iterator.next().then(r=>results.push(r.value+':'+r.done));
        iterator.return(9).then(r=>results.push(r.value+':'+r.done));
        iterator.next().then(r=>results.push(r.value+':'+r.done));
        (async()=>{for await(var n of [1,Promise.resolve(2)])total+=n})();
    "#,
        "results.join('|')==='41:false|2:false|9:true' && cleaned===1 && total===3",
    );
}

#[test]
fn resumable_property_and_conversion_callbacks_remain_owned() {
    assert_owned(
        r#"
        var result=0, threw=false;
        var o={};Object.defineProperty(o,'x',{get:async function(){return 42}});
        o.x.then(v=>result=v);
        var p={};Object.defineProperty(p,'g',{get:function*(){yield 3}});
        var g=p.g.next().value;
        try { +{[Symbol.toPrimitive]:async function(){return 1}} } catch(e) {threw=e instanceof TypeError}
    "#,
        "result===42 && g===3 && threw",
    );
}

#[test]
fn abandoned_async_generator_resume_completes_detached_activation_and_releases_runtime() {
    use crate::engine::{
        builtins::native::{GeneratorResumeKind, NativeFunctionId},
        heap::AsyncGeneratorState,
        vm::{
            async_generator::AsyncGeneratorStep,
            call::{NativeArguments, NativeInvocation},
        },
    };
    let runtime = Runtime::new();
    let weak = std::rc::Rc::downgrade(&runtime.0);
    let mut context = runtime.new_context();
    let Value::Object(generator) = context
        .eval("(async function*(x){yield x})({alive:42})")
        .unwrap()
    else {
        panic!("expected generator");
    };
    let step = AsyncGeneratorStep::start(
        &runtime,
        context.realm,
        NativeFunctionId::AsyncGeneratorPrototypeResume(GeneratorResumeKind::Next),
        &NativeInvocation::Call {
            this_value: Value::Object(generator.clone()),
        },
        &NativeArguments {
            readable: vec![Value::Undefined],
            actual_arg_count: 0,
        },
    )
    .unwrap();
    assert!(matches!(step, AsyncGeneratorStep::Run { .. }));
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .async_generator_snapshot(generator.object_id())
            .unwrap()
            .state,
        AsyncGeneratorState::Executing
    );
    runtime.run_gc().unwrap();
    drop(step);
    let snapshot = runtime
        .0
        .state
        .borrow()
        .heap
        .async_generator_snapshot(generator.object_id())
        .unwrap();
    assert_eq!(snapshot.state, AsyncGeneratorState::Completed);
    assert!(snapshot.activation.is_none());
    assert!(runtime.0.state.borrow().active_frames.is_empty());
    drop(generator);
    drop(context);
    runtime.run_gc().unwrap();
    drop(runtime);
    assert!(weak.upgrade().is_none());
}

#[test]
fn original_argument_roots_survive_parameter_replacement_and_repeated_suspension() {
    use crate::engine::heap::{GeneratorFrameBinding, RawValue};
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(generator) = context.eval("var saved=(function*(x){'use strict';x=7;yield 1;x=8;yield 2;return 3})({original:42});saved.next();saved").unwrap() else {panic!("expected generator")};
    let snapshot = || {
        runtime
            .0
            .state
            .borrow()
            .heap
            .generator_snapshot(generator.object_id())
            .unwrap()
            .1
            .unwrap()
    };
    let first = snapshot();
    assert_eq!(first.actual_argument_count, 1);
    let RawValue::Object(original) = first.original_arguments[0] else {
        panic!("missing original argument")
    };
    assert_eq!(
        first.arguments[0],
        GeneratorFrameBinding::Direct(RawValue::Int(7))
    );
    runtime.run_gc().unwrap();
    assert!(runtime.0.state.borrow().heap.object(original).is_ok());
    context.eval("saved.next()").unwrap();
    let second = snapshot();
    assert_eq!(second.original_arguments, first.original_arguments);
    assert_eq!(
        second.arguments[0],
        GeneratorFrameBinding::Direct(RawValue::Int(8))
    );
    context.eval("saved.next();saved=null").unwrap();
    runtime.run_gc().unwrap();
    assert!(runtime.0.state.borrow().heap.object(original).is_err());
}

#[test]
fn suspension_survives_a_remaining_module_opcode_handoff() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(function) = context
        .eval_with_filename(
            r#"
        var importFailures=0;
        async function f(){try{await import('./absent.js')}catch(e){importFailures++}}
        f();
        (async function(){await 0;try{await import('./absent.js')}catch(e){importFailures++}})();
        f
    "#,
            "suspension-entry.js",
        )
        .unwrap()
    else {
        panic!("expected async function");
    };
    let callable = runtime.as_callable(&function).unwrap().unwrap();
    context.call(&callable, Value::Undefined, &[]).unwrap();
    while runtime.is_job_pending() {
        runtime.run_gc().unwrap();
        runtime.execute_pending_job().unwrap();
    }
    assert_eq!(context.eval("importFailures").unwrap(), Value::Int(3));
}
