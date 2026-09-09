use crate::engine::builtins::native::NativeCProto;
use crate::engine::object::CompleteOrdinaryPropertyDescriptor;

#[cfg(feature = "test262-host")]
use super::*;

#[cfg(feature = "test262-host")]
#[test]
fn code_point_range_context_api_preserves_quickjs_native_contract() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let helper = context.new_code_point_range_function().unwrap();

    assert_eq!(
        NativeFunctionId::StringCodePointRange.descriptor().cproto,
        NativeCProto::Generic
    );
    assert!(!runtime.is_constructor(helper.as_object()).unwrap());
    assert_eq!(runtime.callable_realm(&helper).unwrap(), context.realm);
    assert_eq!(
        runtime.get_prototype_of(helper.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );

    let name = runtime.intern_property_key("name").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    assert!(matches!(
        runtime.get_own_property(helper.as_object(), &name).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: false,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("codePointRange")
    ));
    assert!(matches!(
        runtime
            .get_own_property(helper.as_object(), &length)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(2),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    ));
    {
        let state = runtime.0.state.borrow();
        let ObjectPayload::NativeFunction { data, .. } = &state
            .heap
            .object(helper.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("codePointRange was not a native function");
        };
        assert_eq!(data.target, NativeFunctionId::StringCodePointRange);
        assert_eq!(data.realm, Some(context.realm));
        assert_eq!(data.min_readable_args, 2);
    }

    assert_eq!(
        context.call(&helper, Value::Undefined, &[]).unwrap(),
        Value::String(JsString::from_static(""))
    );
    assert_eq!(
        context
            .call(
                &helper,
                Value::Undefined,
                &[Value::Float(65.9), Value::Float(68.9)],
            )
            .unwrap(),
        Value::String(JsString::from_static("ABC"))
    );
    assert_eq!(
        context
            .call(
                &helper,
                Value::Undefined,
                &[Value::Float(4_294_967_361.0), Value::Int(68)],
            )
            .unwrap(),
        Value::String(JsString::from_static("ABC"))
    );
    assert_eq!(
        context
            .call(&helper, Value::Undefined, &[Value::Int(-1), Value::Int(68)],)
            .unwrap(),
        Value::String(JsString::from_static(""))
    );
    assert_eq!(
        context
            .call(
                &helper,
                Value::Undefined,
                &[Value::Int(0xd7ff), Value::Int(0xd801)],
            )
            .unwrap(),
        Value::String(JsString::try_from_utf16([0xd7ff, 0xd800]).unwrap())
    );
    assert_eq!(
        context
            .call(
                &helper,
                Value::Undefined,
                &[Value::Int(0x10_ffff), Value::Int(-1)],
            )
            .unwrap(),
        Value::String(JsString::try_from_utf16([0xdbff, 0xdfff]).unwrap())
    );

    let Value::String(full_range) = context
        .call(&helper, Value::Undefined, &[Value::Int(0), Value::Int(-1)])
        .unwrap()
    else {
        panic!("full codePointRange did not return a String");
    };
    assert_eq!(full_range.len(), 0x21_0000);
    assert_eq!(full_range.code_unit_at(0), Some(0));
    assert_eq!(full_range.code_unit_at(0x1_0000), Some(0xd800));
    assert_eq!(full_range.code_unit_at(0x1_0001), Some(0xdc00));
    assert_eq!(full_range.code_unit_at(0x20_fffe), Some(0xdbff));
    assert_eq!(full_range.code_unit_at(0x20_ffff), Some(0xdfff));
}
