//! Observation boundaries after a cheap ordinary prefix and after resumption.
use crate::engine::api::{Runtime, Value, profiling::CostProfile};

fn clean(runtime: &Runtime) {
    assert!(runtime.0.state.borrow().active_frames.is_empty());
    assert_eq!(runtime.0.active_frame_depth.get(), 0);
}

#[test]
fn partial_flush_refreshes_resumed_ancestor_pc_without_duplicate_records() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let profile = CostProfile::start();
    let value = context.eval_with_filename(
        "function outer(){\n var a=middle();\n return last(a);\n}\nfunction middle(){return new Error('first').stack}\nfunction last(a){return [a,new Error('second').stack]}\nvar stacks=outer();\nstacks[0].includes('at outer (lazy-mid.js:2:') && stacks[1].includes('at outer (lazy-mid.js:3:') && stacks[1].split('at outer (').length===2 && !stacks[1].includes('at middle (')",
        "lazy-mid.js",
    ).unwrap();
    assert_eq!(value, Value::Bool(true));
    assert!(
        profile
            .snapshot()
            .owned_execution_events
            .get("lazy_frame_materialized")
            .copied()
            .unwrap_or(0)
            >= 3
    );
    clean(&runtime);
}

#[test]
fn proxy_callback_observes_ordinary_ancestors_once_and_restores_after_throw() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let profile = CostProfile::start();
    let value = context.eval_with_filename(
        "function outer(){return inner()}\nfunction inner(){return proxy.x}\nfunction reenter(){return new Error('trap').stack}\nvar saved; var proxy=new Proxy({}, {get(){saved=reenter();throw 17}});\nvar caught=false;try{outer()}catch(e){caught=e===17}\ncaught && saved.includes('at outer (lazy-proxy.js:1:') && saved.includes('at inner (lazy-proxy.js:2:') && saved.includes('at reenter (') && saved.split('at outer (').length===2",
        "lazy-proxy.js",
    ).unwrap();
    assert_eq!(value, Value::Bool(true));
    assert!(
        profile
            .snapshot()
            .owned_execution_events
            .get("lazy_frame_materialized")
            .copied()
            .unwrap_or(0)
            >= 2
    );
    clean(&runtime);
    assert_eq!(context.eval("6*7").unwrap(), Value::Int(42));
}

#[test]
fn suspension_resume_reports_new_fault_pc_and_retires_detached_registration() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval_with_filename(
        "function* gen(){\n yield 1;\n return fail();\n}\nfunction fail(){return Symbol()-1}\nvar iterator=gen(); iterator.next().value",
        "lazy-resume.js",
    ).unwrap();
    clean(&runtime);
    let profile = CostProfile::start();
    assert_eq!(context.eval("var valid=false; try{iterator.next()}catch(e){valid=e instanceof TypeError && e.stack.includes('at gen (lazy-resume.js:3:') && e.stack.includes('at fail (lazy-resume.js:5:')} valid").unwrap(),Value::Bool(true));
    assert!(
        profile
            .snapshot()
            .owned_execution_events
            .get("lazy_frame_materialized")
            .copied()
            .unwrap_or(0)
            >= 1
    );
    clean(&runtime);
    assert_eq!(
        context.eval("iterator.next().done").unwrap(),
        Value::Bool(true)
    );
    clean(&runtime);
}

struct ClearTracker(Runtime);
impl Drop for ClearTracker {
    fn drop(&mut self) {
        self.0.clear_host_promise_rejection_tracker();
    }
}

#[test]
fn host_reentry_and_rust_panic_retire_materialized_lazy_frames() {
    use std::{cell::RefCell, rc::Rc};
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval_with_filename("function hostOuter(){return hostInner()}\nfunction hostInner(){return Promise.reject(23)}", "lazy-host.js").unwrap();
    let host_context = RefCell::new(context.clone());
    let observed = Rc::new(std::cell::Cell::new(false));
    let seen = observed.clone();
    let clear = ClearTracker(runtime.clone());
    runtime.set_host_promise_rejection_tracker(move |event| {
        if event.is_handled() { return; }
        let mut context=host_context.borrow_mut();
        let snapshot=context.eval("var hostStack=new Error('host').stack; hostStack.includes('at hostOuter (lazy-host.js:1:') && hostStack.includes('at hostInner (lazy-host.js:2:')").unwrap();
        assert_eq!(snapshot,Value::Bool(true));
        seen.set(true);
        panic!("lazy host unwind probe");
    });
    let profile = CostProfile::start();
    let unwind =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| context.eval("hostOuter()")));
    drop(clear);
    assert_eq!(
        unwind.unwrap_err().downcast_ref::<&str>().copied(),
        Some("lazy host unwind probe")
    );
    assert!(observed.get());
    assert!(
        profile
            .snapshot()
            .owned_execution_events
            .get("lazy_frame_materialized")
            .copied()
            .unwrap_or(0)
            >= 2
    );
    clean(&runtime);
    assert_eq!(context.eval("6*7").unwrap(), Value::Int(42));
}
