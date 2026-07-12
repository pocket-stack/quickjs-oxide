use super::*;

#[test]
fn reduced_group_by_element_limit_checks_before_next_and_preserves_throw() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let iterable = eval_object(
        &mut context,
        r#"(function(){
            globalThis.groupByNextCount=0;
            globalThis.groupByCallbackCount=0;
            globalThis.groupByReturnCount=0;
            var iterator=Object();
            iterator.next=function(){
                groupByNextCount++;
                var result=Object();
                result.done=false;
                result.value=groupByNextCount;
                return result;
            };
            iterator.return=function(){
                groupByReturnCount++;
                throw "close replacement";
            };
            var iterable=Object();
            iterable[Symbol.iterator]=function(){return iterator};
            return iterable;
        })()"#,
    );
    let callback = eval_object(
        &mut context,
        r#"(function(value,index){
            groupByCallbackCount++;
            return "group";
        })"#,
    );
    let arguments = NativeArguments {
        actual_arg_count: 2,
        readable: vec![Value::Object(iterable), Value::Object(callback)],
    };

    let completion = runtime
        .call_object_group_by_with_element_limit(
            context.realm,
            NativeInvocation::Call {
                this_value: Value::Undefined,
            },
            &arguments,
            2,
        )
        .unwrap();
    let Completion::Throw(Value::Object(error)) = completion else {
        panic!("reduced Object.groupBy limit did not throw an Error object");
    };
    assert_eq!(
        string_property(&runtime, &mut context, &error, "name"),
        "TypeError",
    );
    assert_eq!(
        string_property(&runtime, &mut context, &error, "message"),
        "too many elements",
    );
    assert_eq!(eval_int(&mut context, "groupByNextCount"), 2);
    assert_eq!(eval_int(&mut context, "groupByCallbackCount"), 2);
    assert_eq!(eval_int(&mut context, "groupByReturnCount"), 1);
}

#[test]
fn recursive_group_by_callback_ceiling_is_catchable() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let value = context
        .eval(
            r#"(function(){
                function recurse(depth){
                    return Object.groupBy([depth],function(){
                        if(depth!==0)recurse(depth-1);
                        return "group";
                    });
                }
                recurse(8);
                try{recurse(9);return "missing"}
                catch(error){return "ok|"+error.name+":"+error.message}
            })()"#,
        )
        .unwrap();
    assert_eq!(
        value,
        Value::String(JsString::from_static("ok|InternalError:stack overflow",)),
    );
}

fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    object
}

fn eval_int(context: &mut Context, source: &str) -> i32 {
    let Value::Int(value) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an Int");
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
