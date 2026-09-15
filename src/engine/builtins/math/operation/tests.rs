use crate::engine::api::profiling::CostProfile;
use crate::engine::api::{Runtime, Value};

#[test]
fn primitive_math_completes_without_owning_a_second_argument_vector() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let profile = CostProfile::start();
    assert_eq!(
        context
            .eval(
                r#"(()=>{
        return Math.min(3,2,1)===1 && Object.is(Math.min(0,-0),-0)
            && Object.is(Math.max(-0,0),0) && Math.min()===Infinity && Math.max()===-Infinity
            && Math.imul(2147483647,2)===-2 && Math.clz32()===32
            && Math.hypot()===0 && Math.hypot(-3)===3 && Math.hypot(3,4)===5
            && Number.isNaN(Math.abs()) && Number.isNaN(Math.pow(-1,Infinity));
    })()"#
            )
            .unwrap(),
        Value::Bool(true)
    );
    let costs = profile.snapshot();
    assert!(
        costs
            .owned_execution_events
            .get("math_completed_without_argument_storage")
            .copied()
            .unwrap_or(0)
            >= 12
    );
    assert_eq!(
        costs
            .owned_execution_events
            .get("math_remaining_arguments_owned")
            .copied()
            .unwrap_or(0),
        0
    );
    assert_eq!(costs.owned_bridge_exits, 0);
    assert_eq!(costs.owned_sync_call_bridges, 0);
}

#[test]
fn object_suffix_conversion_keeps_order_even_after_nan() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval(
                r#"(()=>{
        let log=[];
        let a={valueOf(){log.push('a');return 2;}};
        let b={valueOf(){log.push('b');return 1;}};
        let result=Math.min(3,a,4,b);
        let nan=Math.max(NaN,{valueOf(){log.push('nan');return 5;}});
        let thrown=false;
        try{Math.min(NaN,{valueOf(){log.push('throw');throw 42;}});}catch(e){thrown=e===42;}
        let unused={valueOf(){throw 'unused';}};
        let unary=Math.abs(-3,unused);
        return result===1 && Number.isNaN(nan) && thrown && unary===3
            && log.join(',')==='a,b,nan,throw' && Math.min(1,2)===1;
    })()"#
            )
            .unwrap(),
        Value::Bool(true)
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn primitive_conversion_errors_are_catchable_and_keep_native_activation() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval(
                r#"(()=>{
        let count=0;
        for(let value of [1n,Symbol('x')]) {
            try{Math.min(1,value);}catch(e){if(e instanceof TypeError)count++;}
        }
        try{isNaN(1n);}catch(e){if(e instanceof TypeError)count++;}
        try{isFinite(Symbol());}catch(e){if(e instanceof TypeError)count++;}
        try{decodeURIComponent('%');}catch(e){if(e instanceof URIError)count++;}
        return count===5 && Math.min(2,1)===1 && isFinite(3);
    })()"#
            )
            .unwrap(),
        Value::Bool(true)
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn global_primitive_stages_preserve_input_then_radix_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let profile = CostProfile::start();
    assert_eq!(
        context
            .eval(
                r#"(()=>{
        let log=[];
        let radix={valueOf(){log.push('radix');return 2;}};
        let first=parseInt('11',radix);
        let input={toString(){log.push('input');return '10';}};
        let second=parseInt(input,radix);
        try{parseInt(Symbol(),radix);}catch(e){log.push('symbol');}
        return first===3 && second===2 && log.join(',')==='radix,input,radix,symbol'
            && parseInt('20',10)===20 && parseFloat('1.5tail')===1.5
            && encodeURIComponent('a b')==='a%20b' && isFinite(null) && isNaN('bad');
    })()"#
            )
            .unwrap(),
        Value::Bool(true)
    );
    let costs = profile.snapshot();
    assert!(
        costs
            .owned_execution_events
            .get("global_completed_without_waiting_state")
            .copied()
            .unwrap_or(0)
            >= 5
    );
    assert_eq!(costs.owned_bridge_exits, 0);
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}
