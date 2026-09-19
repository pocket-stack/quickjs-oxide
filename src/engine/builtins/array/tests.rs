use crate::engine::atom::AtomIdx;
use crate::engine::api::Context;
use crate::engine::heap::RawValue;

use super::*;

#[test]
fn reduced_flatten_target_limit_preserves_prefix_and_exact_error() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(&mut context, "[1,2,3]");
    let target = eval_object(&mut context, "Object()");

    let result = runtime
        .flatten_into_array_with_limits(
            context.realm,
            &target,
            source,
            3,
            0,
            None,
            &Value::Undefined,
            2,
            16,
        )
        .unwrap();
    let NativeConversion::Throw(Value::Object(error)) = result else {
        panic!("reduced flatten limit did not return an Error object");
    };
    assert_eq!(
        string_property(&runtime, &mut context, &error, "name"),
        "TypeError"
    );
    assert_eq!(
        string_property(&runtime, &mut context, &error, "message"),
        "Array too long",
    );
    assert_eq!(int_property(&runtime, &mut context, &target, "0"), 1);
    assert_eq!(int_property(&runtime, &mut context, &target, "1"), 2);
    assert!(
        runtime
            .get_own_property(
                &target,
                &runtime
                    .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Literal3)
                    .unwrap()
            )
            .unwrap()
            .is_none(),
        "the failing element was defined past the reduced target limit",
    );
    assert!(
        runtime
            .get_own_property(
                &target,
                &runtime
                    .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)
                    .unwrap()
            )
            .unwrap()
            .is_none(),
        "flatten performed a final length Set on an ordinary species target",
    );
}

#[test]
fn reduced_flatten_frame_limit_is_catchable_without_rust_recursion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(
        &mut context,
        "(function(){var source=[];source[0]=source;return source})()",
    );
    let target = eval_object(&mut context, "Object()");

    let result = runtime
        .flatten_into_array_with_limits(
            context.realm,
            &target,
            source,
            1,
            i32::MAX,
            None,
            &Value::Undefined,
            (1_u64 << 53) - 1,
            3,
        )
        .unwrap();
    let NativeConversion::Throw(Value::Object(error)) = result else {
        panic!("reduced flatten frame limit did not return an Error object");
    };
    assert_eq!(
        string_property(&runtime, &mut context, &error, "name"),
        "InternalError",
    );
    assert_eq!(
        string_property(&runtime, &mut context, &error, "message"),
        "stack overflow",
    );
}

#[test]
fn array_unscopables_autoinit_retains_then_releases_its_realm_edge() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let array_prototype = context.array_prototype().unwrap();
    let key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Unscopables));

    let (slot_index, count_before) = {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(array_prototype.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(AtomIdx::from_raw(key.atom().raw())).unwrap()).unwrap();
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(
                AutoInitProperty::ArrayUnscopables { realm }
            )) if *realm == context.realm
        ));
        (
            slot_index,
            state.heap.context_strong_count(context.realm).unwrap(),
        )
    };

    let Value::Object(unscopables) = context.get_property(&array_prototype, &key).unwrap() else {
        panic!("Array.prototype[Symbol.unscopables] was not an object");
    };
    let state = runtime.0.state.borrow();
    assert_eq!(
        state.heap.context_strong_count(context.realm).unwrap(),
        count_before - 1,
        "materialization did not release the autoinit's defining-realm edge",
    );
    let object = state.heap.object(array_prototype.object_id()).unwrap();
    assert!(matches!(
        object.slots.get(slot_index),
        Some(PropertySlot::Data(RawValue::Object(id))) if *id == unscopables.object_id()
    ));
    drop(state);
    assert_eq!(runtime.get_prototype_of(&unscopables).unwrap(), None);
}

#[test]
fn array_unscopables_metadata_and_delete_preserve_lazy_state() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let array_prototype = context.array_prototype().unwrap();
    let key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Unscopables));

    let count_before = {
        let state = runtime.0.state.borrow();
        state.heap.context_strong_count(context.realm).unwrap()
    };
    assert!(
        runtime
            .own_property_keys(&array_prototype)
            .unwrap()
            .contains(&key)
    );
    assert!(runtime.has_property(&array_prototype, &key).unwrap());
    {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(array_prototype.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(AtomIdx::from_raw(key.atom().raw())).unwrap()).unwrap();
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(
                AutoInitProperty::ArrayUnscopables { realm }
            )) if *realm == context.realm
        ));
        assert_eq!(
            state.heap.context_strong_count(context.realm).unwrap(),
            count_before,
            "ownKeys or HasProperty materialized the autoinit slot",
        );
    }

    assert!(runtime.delete_property(&array_prototype, &key).unwrap());
    let state = runtime.0.state.borrow();
    assert_eq!(
        state.heap.context_strong_count(context.realm).unwrap(),
        count_before - 1,
        "deleting the lazy property did not release its defining-realm edge",
    );
    let object = state.heap.object(array_prototype.object_id()).unwrap();
    let shape = state.heap.shape(object.shape).unwrap();
    assert!(shape.find(AtomIdx::from_raw(key.atom().raw())).is_none());
}

fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    object
}

fn int_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> i32 {
    let Value::Int(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an Int property");
    };
    value
}

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not a String property");
    };
    value.to_utf8_lossy()
}

#[test]
fn owned_array_domains_preserve_mutation_and_callback_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for source in [
        "(()=>{let a=[1,,3];let r=a.map((v,i)=>{if(i===0)a[1]=2;return v*2});return r.join(',')==='2,4,6'})()",
        "(()=>{let a=[1,2,3];let r=a.filter((v,i)=>{if(i===0)delete a[1];return true});return r.join(',')==='1,3'})()",
        "(()=>{let seen='';let a=[1,,3];let r=a.reduceRight((x,v,i)=>{seen+=i;return x+v},0);return r===4&&seen==='20'})()",
        "(()=>{let a=[,2];let seen='';let r=a.find((v,i)=>{seen+=i;return i===0});return r===undefined&&seen==='0'})()",
        "(()=>{let a=[1,,3];a.unshift(9);return a.length===4&&a[0]===9&&a[1]===1&&!(2 in a)&&a[3]===3})()",
        "(()=>{let a=[1,,3];let r=a.shift();return r===1&&a.length===2&&!(0 in a)&&a[1]===3})()",
        "(()=>{let a=[1,,3,4];a.copyWithin(1,0,3);return a.length===4&&a[1]===1&&!(2 in a)&&a[3]===3})()",
        "(()=>{let a=[1,,3,4];a.reverse();return a[0]===4&&a[1]===3&&!(2 in a)&&a[3]===1})()",
        "(()=>{let a=[1,,3,4];let d=a.splice(1,2,8,9,10);return d.length===2&&!(0 in d)&&d[1]===3&&a.join(',')==='1,8,9,10,4'})()",
        "(()=>{let a=[1,,3,4];let r=a.toSpliced(1,1,9);return r.join(',')==='1,9,3,4'&&a.length===4&&!(1 in a)})()",
        "(()=>{let a=[1,,3];let r=a.with(-1,8);return r.length===3&&r[2]===8&&(1 in r)&&r[1]===undefined})()",
        "(()=>{let a=[1,[2,,[3]],4];return a.flat(2).join(',')==='1,2,3,4'&&[1,2].flatMap(x=>[x,x]).join(',')==='1,1,2,2'})()",
        "(()=>{let trace='';let a={length:2,0:{toString(){trace+='a';return 'A'}},1:{toString(){trace+='b';return 'B'}}};let sep={toString(){trace+='s';return ':'}};return Array.prototype.join.call(a,sep)==='A:B'&&trace==='sab'})()",
        "(()=>{let a=[3,1,2];let r=a.toSorted((x,y)=>x-y);return r.join(',')==='1,2,3'&&a.join(',')==='3,1,2'})()",
        "(()=>{let a=[1,2];let s={ [Symbol.isConcatSpreadable]:true,length:2,0:3,1:4 };return a.concat(s).join(',')==='1,2,3,4'})()",
        "(()=>{let log='';let a={length:2,get 0(){log+='0';return 1},get 1(){log+='1';return 2}};let r=Array.prototype.toReversed.call(a);return log==='10'&&r.join(',')==='2,1'})()",
    ] {
        assert!(
            matches!(context.eval(source).unwrap(), Value::Bool(true)),
            "{source}"
        );
    }
}

#[test]
fn rqsort_cursor_orders_partition_insertion_and_large_inputs() {
    for length in [0, 1, 2, 6, 7, 32, 257, 4096] {
        let mut values = (0..length)
            .map(|index| ((index * 37 + 19) % 71, index))
            .collect::<Vec<_>>();
        quickjs_rqsort_by(&mut values, |values, left, right| {
            Ok::<_, ()>(values[left].cmp(&values[right]))
        })
        .unwrap();
        assert!(
            values.windows(2).all(|pair| pair[0] <= pair[1]),
            "length {length}"
        );
    }
}
