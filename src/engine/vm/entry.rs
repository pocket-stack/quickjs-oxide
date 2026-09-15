//! Existing Context entry validation followed by an owned root request.
use super::call::ConstructNewTarget;
use super::{Completion, RootOperation, execute_root};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::heap::ContextId;
use crate::engine::object::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, ObjectRef, OrdinaryPropertyDescriptor,
    PropertyKey,
};
use crate::engine::value::{Value, conversion::NativeConversion};

pub(super) type DescriptorReply = NativeConversion<Option<CompleteOrdinaryPropertyDescriptor>>;
pub(crate) fn call(
    runtime: &Runtime,
    realm: ContextId,
    callable: &CallableRef,
    receiver: Value,
    arguments: &[Value],
) -> Result<Completion, RuntimeError> {
    runtime.0.state.borrow().heap.context(realm)?;
    runtime.validate_value_domain(&receiver, "call this value")?;
    for argument in arguments {
        runtime.validate_value_domain(argument, "call argument")?;
    }
    if !callable.belongs_to(runtime) {
        return Err(RuntimeError::WrongRuntime("callable"));
    }
    execute_root(
        runtime.clone(),
        realm,
        RootOperation::Call {
            callable: callable.clone(),
            receiver,
            arguments: arguments.to_vec(),
        },
    )
    .map_err(RuntimeError::Engine)
}
pub(crate) fn construct(
    runtime: &Runtime,
    realm: ContextId,
    constructor: &CallableRef,
    new_target: &CallableRef,
    arguments: &[Value],
) -> Result<Completion, RuntimeError> {
    let (constructor, new_target) =
        match runtime.prepare_constructor_pair(realm, constructor, new_target)? {
            NativeConversion::Value(pair) => pair,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
    let normalized = match runtime.normalize_constructor(
        realm,
        constructor,
        ConstructNewTarget::Validated(new_target),
        arguments.to_vec(),
    )? {
        NativeConversion::Value(normalized) => normalized,
        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
    };
    execute_root(runtime.clone(), realm, RootOperation::Construct(normalized))
        .map_err(RuntimeError::Engine)
}
pub(crate) fn get(
    runtime: &Runtime,
    realm: ContextId,
    object: &ObjectRef,
    key: &PropertyKey,
    receiver: Value,
) -> Result<Completion, RuntimeError> {
    runtime.validate_object_and_key(object, key)?;
    runtime.validate_value_domain(&receiver, "property receiver")?;
    execute_root(
        runtime.clone(),
        realm,
        RootOperation::Get {
            object: object.clone(),
            key: key.clone(),
            receiver,
        },
    )
    .map_err(RuntimeError::Engine)
}
pub(crate) fn own(
    runtime: &Runtime,
    realm: ContextId,
    object: &ObjectRef,
    key: &PropertyKey,
) -> Result<DescriptorReply, RuntimeError> {
    runtime.is_proxy_object(object)?;
    runtime.validate_object_and_key(object, key)?;
    super::driver::execute_root_descriptor(
        runtime.clone(),
        realm,
        RootOperation::Own {
            object: object.clone(),
            key: key.clone(),
        },
    )
    .map_err(RuntimeError::Engine)
}
pub(crate) fn define(
    runtime: &Runtime,
    realm: ContextId,
    object: &ObjectRef,
    key: &PropertyKey,
    descriptor: &OrdinaryPropertyDescriptor,
) -> Result<NativeConversion<bool>, RuntimeError> {
    runtime.is_proxy_object(object)?;
    runtime.validate_object_and_key(object, key)?;
    runtime.validate_descriptor_domains(descriptor)?;
    boolean(
        execute_root(
            runtime.clone(),
            realm,
            RootOperation::Define {
                object: object.clone(),
                key: key.clone(),
                descriptor: descriptor.clone(),
            },
        )
        .map_err(RuntimeError::Engine)?,
    )
}
pub(crate) fn set(
    runtime: &Runtime,
    realm: ContextId,
    object: &ObjectRef,
    key: &PropertyKey,
    value: Value,
    receiver: Value,
) -> Result<NativeConversion<bool>, RuntimeError> {
    runtime.validate_object_and_key(object, key)?;
    runtime.validate_value_domain(&value, "property value")?;
    runtime.validate_value_domain(&receiver, "property receiver")?;
    boolean(
        execute_root(
            runtime.clone(),
            realm,
            RootOperation::Set {
                object: object.clone(),
                key: key.clone(),
                value,
                receiver,
            },
        )
        .map_err(RuntimeError::Engine)?,
    )
}
fn boolean(completion: Completion) -> Result<NativeConversion<bool>, RuntimeError> {
    match completion {
        Completion::Return(Value::Bool(value)) => Ok(NativeConversion::Value(value)),
        Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        _ => Err(RuntimeError::Invariant(
            "property entry did not return a boolean",
        )),
    }
}

#[cfg(all(test, feature = "profiling", feature = "test262-host"))]
mod tests {
    use crate::engine::api::profiling::CostProfile;
    use crate::engine::api::{Runtime, Value};
    use crate::engine::object::{DescriptorField, OrdinaryPropertyDescriptor};

    #[test]
    fn context_property_roots_keep_typed_descriptors_across_proxy_callbacks_and_gc() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.install_test262_host().unwrap();
        let Value::Object(proxy) = context
            .eval(
                r#"new Proxy({}, {
            getOwnPropertyDescriptor(t,k){$262.gc();return Reflect.getOwnPropertyDescriptor(t,k)},
            defineProperty(t,k,d){$262.gc();return Reflect.defineProperty(t,k,d)},
            get(t,k,r){$262.gc();return Reflect.get(t,k,r)},
            set(t,k,v,r){$262.gc();return Reflect.set(t,k,v,r)}
        })"#,
            )
            .unwrap()
        else {
            panic!("expected proxy")
        };
        let value = context.new_object().unwrap();
        let key = runtime.intern_property_key("x").unwrap();
        let descriptor = OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::Object(value.clone())),
            writable: DescriptorField::Present(true),
            enumerable: DescriptorField::Present(true),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };
        let profile = CostProfile::start();
        assert!(
            context
                .define_own_property(&proxy, &key, &descriptor)
                .unwrap()
        );
        assert_eq!(
            context.get_property(&proxy, &key).unwrap(),
            Value::Object(value.clone())
        );
        let own = context.get_own_property(&proxy, &key).unwrap().unwrap();
        assert!(
            matches!(own, crate::engine::object::CompleteOrdinaryPropertyDescriptor::Data { value: Value::Object(actual), writable: true, enumerable: true, configurable: true } if actual == value)
        );
        assert!(context.set_property(&proxy, &key, Value::Int(42)).unwrap());
        assert_eq!(context.get_property(&proxy, &key).unwrap(), Value::Int(42));
        let snapshot = profile.snapshot();
        assert_eq!(snapshot.legacy_dispatches, 0);
        assert_eq!(snapshot.owned_bridge_exits, 0);
        assert_eq!(snapshot.owned_sync_call_bridges, 0);
    }

    #[test]
    fn context_native_call_and_proxy_construct_share_owned_callback_execution() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.install_test262_host().unwrap();
        let Value::Object(map) = context.eval("Array.prototype.map").unwrap() else {
            panic!("expected map")
        };
        let map = runtime.as_callable(&map).unwrap().unwrap();
        let callback = context.eval("x=>{$262.gc();return x+1}").unwrap();
        let input = context.new_array_from_values(vec![Value::Int(2)]).unwrap();
        let Value::Object(constructor) = context.eval("new Proxy(function C(x){this.x=x},{construct(t,a,n){$262.gc();return Reflect.construct(t,a,n)}})").unwrap() else { panic!("expected constructor") };
        let constructor = runtime.as_callable(&constructor).unwrap().unwrap();
        let key = runtime.intern_property_key("x").unwrap();
        let index = runtime.intern_property_key("0").unwrap();
        let profile = CostProfile::start();
        let Value::Object(result) = context
            .call(&map, Value::Object(input), &[callback])
            .unwrap()
        else {
            panic!("expected mapped array")
        };
        assert_eq!(
            context.get_property(&result, &index).unwrap(),
            Value::Int(3)
        );
        let Value::Object(result) = context.construct(&constructor, &[Value::Int(9)]).unwrap()
        else {
            panic!("expected instance")
        };
        assert_eq!(context.get_property(&result, &key).unwrap(), Value::Int(9));
        let snapshot = profile.snapshot();
        assert_eq!(snapshot.legacy_dispatches, 0);
        assert_eq!(snapshot.owned_bridge_exits, 0);
        assert_eq!(snapshot.owned_sync_call_bridges, 0);
    }
    #[test]
    fn template_value_constants_keep_identity_and_roots_through_owned_entry_and_gc() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.install_test262_host().unwrap();
        let site = {
            let Value::Object(site) = context.eval("(function(){function tag(t){$262.gc();return t}return function(){return tag`alive${42}raw`}})()").unwrap() else {
                panic!("expected template site")
            };
            runtime.as_callable(&site).unwrap().unwrap()
        };
        let profile = CostProfile::start();
        let first = context.call(&site, Value::Undefined, &[]).unwrap();
        runtime.run_gc().unwrap();
        assert_eq!(context.call(&site, Value::Undefined, &[]).unwrap(), first);
        drop(site);
        runtime.run_gc().unwrap();
        let Value::Object(first) = first else {
            panic!("expected template object")
        };
        let raw = context
            .get_property(&first, &runtime.intern_property_key("raw").unwrap())
            .unwrap();
        let Value::Object(raw) = raw else {
            panic!("expected raw template")
        };
        let zero = runtime.intern_property_key("0").unwrap();
        assert_eq!(
            context.get_property(&first, &zero).unwrap(),
            Value::String(crate::engine::value::JsString::from_static("alive"))
        );
        assert_eq!(
            context.get_property(&raw, &zero).unwrap(),
            Value::String(crate::engine::value::JsString::from_static("alive"))
        );
        let snapshot = profile.snapshot();
        assert_eq!(snapshot.legacy_dispatches, 0);
        assert_eq!(snapshot.owned_bridge_exits, 0);
        assert_eq!(snapshot.owned_sync_call_bridges, 0);
    }
}
