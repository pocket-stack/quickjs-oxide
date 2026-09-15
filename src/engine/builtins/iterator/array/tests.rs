use crate::engine::api::profiling::CostProfile;
use crate::engine::api::{Runtime, Value};

#[test]
fn ordinary_array_next_finishes_locally_for_values_keys_and_entries() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let profile = CostProfile::start();
    assert_eq!(
        context
            .eval(
                r#"(()=>{
        let sum=0;for(let value of [1,2,3])sum+=value;
        let keys=[4,5].keys();let first=keys.next();let second=keys.next();let done=keys.next();
        let entries=[7].entries();let entry=entries.next();let finished=entries.next();
        return sum===6 && first.value===0 && second.value===1 && done.done
            && !entry.done && entry.value[0]===0 && entry.value[1]===7 && finished.done;
    })()"#
            )
            .unwrap(),
        Value::Bool(true)
    );
    let costs = profile.snapshot();
    assert!(
        costs
            .owned_execution_events
            .get("array_next_completed_without_waiting_state")
            .copied()
            .unwrap_or(0)
            >= 9
    );
    assert_eq!(costs.owned_bridge_exits, 0);
    assert_eq!(costs.owned_sync_call_bridges, 0);
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn length_conversion_and_prepared_getters_are_not_replayed() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval(r#"(()=>{
        let log=[];let value=1;
        let source={get length(){log.push('length');return {valueOf(){log.push('number');value=9;return 1;}};},get 0(){log.push('index');return value;}};
        let iterator=Array.prototype.values.call(source);let result=iterator.next();
        let done=iterator.next();
        return result.value===9 && !result.done && done.done
            && log.join(',')==='length,number,index,length,number';
    })()"#).unwrap(),Value::Bool(true));
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn proxy_source_and_hole_prototype_getters_run_once() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval(r#"(()=>{
        let log=[];
        let source=new Proxy([3,4],{get(target,key,receiver){log.push(String(key));return Reflect.get(target,key,receiver);}});
        let iterator=Array.prototype.values.call(source);
        let first=iterator.next();let second=iterator.next();let done=iterator.next();
        let hole=[,];Object.setPrototypeOf(hole,{get 0(){log.push('hole');return 7;}});
        let inherited=Array.prototype.values.call(hole).next();
        return first.value===3 && second.value===4 && done.done && inherited.value===7
            && log.join(',')==='length,0,length,1,length,hole';
    })()"#).unwrap(),Value::Bool(true));
}

#[test]
fn index_getter_throw_advances_index_but_length_throw_does_not() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval(
                r#"(()=>{
        let array=[1,5];Object.defineProperty(array,'0',{get(){throw 42;}});
        let iterator=array.values();let indexThrow=false;
        try{iterator.next();}catch(error){indexThrow=error===42;}
        let afterIndex=iterator.next();
        let reads=0;let source={get length(){if(reads++===0)throw 77;return 1;},0:9};
        let other=Array.prototype.values.call(source);let lengthThrow=false;
        try{other.next();}catch(error){lengthThrow=error===77;}
        let afterLength=other.next();
        return indexThrow && afterIndex.value===5 && lengthThrow && afterLength.value===9;
    })()"#
            )
            .unwrap(),
        Value::Bool(true)
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn typed_array_next_revalidates_bounds_and_detachment_before_reading() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval(
                r#"(()=>{
        let buffer=new ArrayBuffer(4,{maxByteLength:8});let array=new Uint8Array(buffer,0,4);
        array[0]=7;let iterator=array.values();let first=iterator.next();buffer.resize(0);
        let rejected=false;try{iterator.next();}catch(error){rejected=error instanceof TypeError;}
        buffer.resize(4);array[1]=9;let resumed=iterator.next();
        return first.value===7 && rejected && resumed.value===9;
    })()"#
            )
            .unwrap(),
        Value::Bool(true)
    );
    let buffer=context.eval("globalThis.sourceForDetach=new Uint8Array([1]);globalThis.iteratorForDetach=sourceForDetach.values();sourceForDetach.buffer").unwrap();
    context.detach_array_buffer(&buffer).unwrap();
    assert_eq!(context.eval("(()=>{try{iteratorForDetach.next();return false;}catch(e){return e instanceof TypeError;}})()").unwrap(),Value::Bool(true));
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn dense_immediate_next_preserves_frozen_holes_proxy_and_loop_mutations() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let profile = CostProfile::start();
    assert_eq!(context.eval(r#"(() => {
        let sum = 0;
        const a = [1,2,3];
        for (const value of a) {
            sum += value;
            if (value === 1) { a[1] = 7; a.push(4); }
            if (value === 7) a.length = 3;
        }
        const frozen = Object.freeze([4,5]);
        let frozenSum = 0; for (const value of frozen) frozenSum += value;
        let calls = 0;
        const hole = [,2], it = hole.values();
        Object.setPrototypeOf(hole, {get 0() { calls++; return it.next().value; }});
        const first = it.next(), done = it.next();
        let trace = '';
        const proxy = new Proxy([8], {get(t,k,r) { trace += String(k) + ','; return Reflect.get(t,k,r); }});
        const pit = Array.prototype.values.call(proxy);
        const pfirst = pit.next(), pdone = pit.next();
        const entries = [9].entries().next().value;
        return sum === 11 && frozenSum === 9 && calls === 1 && first.value === 2 && done.done
            && pfirst.value === 8 && pdone.done && trace === 'length,0,length,'
            && entries[0] === 0 && entries[1] === 9;
    })()"#).unwrap(), Value::Bool(true));
    let cost = profile.snapshot();
    assert!(
        cost.owned_execution_events
            .get("array_next_dense_immediate_leaf")
            .copied()
            .unwrap_or(0)
            >= 3
    );
    assert!(
        cost.owned_execution_events
            .get("native_activation_prepared")
            .copied()
            .unwrap_or(0)
            >= 3
    );
    assert_eq!(cost.owned_bridge_exits, 0);
    assert_eq!(cost.owned_sync_call_bridges, 0);
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}
