use super::*;

#[test]
fn pending_promise_reaction_does_not_retain_the_species_result_object() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(result) = context
        .eval(
            r#"
var pendingSource = new Promise(function () {});
function CustomCapability(executor) {
    executor(function () {}, function () {});
    return { marker: 42 };
}
pendingSource.constructor = { [Symbol.species]: CustomCapability };
pendingSource.then();
"#,
        )
        .unwrap()
    else {
        panic!("custom Promise capability did not return its result object");
    };
    let result_id = result.object_id();
    drop(result);

    runtime.run_gc().unwrap();
    assert!(
        runtime.0.state.borrow().heap.object(result_id).is_err(),
        "a pending reaction retained the custom species result object"
    );
    assert_eq!(
        context.eval("typeof pendingSource.then").unwrap(),
        Value::String(JsString::from_static("function"))
    );
}

#[test]
fn weak_ref_and_finalization_registry_globals_match_intrinsic_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"
(function () {
    function flags(descriptor) {
        return Number(descriptor.writable) +
            "" + Number(descriptor.enumerable) +
            Number(descriptor.configurable);
    }
    var weakGlobal = Object.getOwnPropertyDescriptor(globalThis, "WeakRef");
    var weakPrototype = Object.getOwnPropertyDescriptor(WeakRef, "prototype");
    var weakConstructor = Object.getOwnPropertyDescriptor(
        WeakRef.prototype,
        "constructor"
    );
    var deref = Object.getOwnPropertyDescriptor(WeakRef.prototype, "deref");
    var weakTag = Object.getOwnPropertyDescriptor(
        WeakRef.prototype,
        Symbol.toStringTag
    );
    var registryGlobal = Object.getOwnPropertyDescriptor(
        globalThis,
        "FinalizationRegistry"
    );
    var registryPrototype = Object.getOwnPropertyDescriptor(
        FinalizationRegistry,
        "prototype"
    );
    var registryConstructor = Object.getOwnPropertyDescriptor(
        FinalizationRegistry.prototype,
        "constructor"
    );
    var register = Object.getOwnPropertyDescriptor(
        FinalizationRegistry.prototype,
        "register"
    );
    var unregister = Object.getOwnPropertyDescriptor(
        FinalizationRegistry.prototype,
        "unregister"
    );
    var registryTag = Object.getOwnPropertyDescriptor(
        FinalizationRegistry.prototype,
        Symbol.toStringTag
    );
    return [
        typeof WeakRef,
        flags(weakGlobal),
        WeakRef.name,
        WeakRef.length,
        flags(weakPrototype),
        flags(weakConstructor),
        deref.value.name,
        deref.value.length,
        flags(deref),
        Object.prototype.toString.call(WeakRef.prototype),
        flags(weakTag),
        typeof FinalizationRegistry,
        flags(registryGlobal),
        FinalizationRegistry.name,
        FinalizationRegistry.length,
        flags(registryPrototype),
        flags(registryConstructor),
        register.value.name,
        register.value.length,
        flags(register),
        unregister.value.name,
        unregister.value.length,
        flags(unregister),
        Object.prototype.toString.call(FinalizationRegistry.prototype),
        flags(registryTag)
    ].join("|");
})()
"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "function|101|WeakRef|1|000|101|deref|0|101|[object WeakRef]|001|\
             function|101|FinalizationRegistry|1|000|101|register|2|101|\
             unregister|1|101|[object FinalizationRegistry]|001"
        ))
    );
}

#[test]
fn weak_ref_and_finalization_registry_reject_invalid_calls_brands_and_registered_symbols() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"
(function () {
    function throwsTypeError(callback) {
        try {
            callback();
            return false;
        } catch (error) {
            return error instanceof TypeError;
        }
    }
    var registry = new FinalizationRegistry(function () {});
    var registered = Symbol.for("weak-target-probe");
    return [
        throwsTypeError(function () { WeakRef({}); }),
        throwsTypeError(function () { FinalizationRegistry(function () {}); }),
        throwsTypeError(function () { new WeakRef(1); }),
        throwsTypeError(function () { new FinalizationRegistry({}); }),
        throwsTypeError(function () { WeakRef.prototype.deref.call({}); }),
        throwsTypeError(function () {
            FinalizationRegistry.prototype.register.call({}, {}, 1);
        }),
        throwsTypeError(function () {
            FinalizationRegistry.prototype.unregister.call({}, {});
        }),
        throwsTypeError(function () { new WeakRef(registered); }),
        throwsTypeError(function () { registry.register(registered, 1); }),
        throwsTypeError(function () { registry.register({}, 1, registered); }),
        throwsTypeError(function () { registry.unregister(registered); })
    ].join("|");
})()
"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "true|true|true|true|true|true|true|true|true|true|true"
        ))
    );
}

#[test]
fn weak_ref_dereferences_object_local_and_well_known_symbol_targets() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"
var weakObjectTarget = { marker: 42 };
var weakLocalTarget = Symbol("local-target");
var weakObjectReference = new WeakRef(weakObjectTarget);
var weakLocalReference = new WeakRef(weakLocalTarget);
var weakWellKnownReference = new WeakRef(Symbol.iterator);
[
    weakObjectReference.deref() === weakObjectTarget,
    weakObjectReference.deref().marker === 42,
    weakLocalReference.deref() === weakLocalTarget,
    weakWellKnownReference.deref() === Symbol.iterator
].join("|");
"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("true|true|true|true"))
    );

    context
        .eval("weakObjectTarget = null; weakLocalTarget = null;")
        .unwrap();
    runtime.run_gc().unwrap();
    assert_eq!(
        context
            .eval(
                r#"[
    weakObjectReference.deref() === undefined,
    weakLocalReference.deref() === undefined,
    weakWellKnownReference.deref() === Symbol.iterator
].join("|")"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("true|true|true"))
    );
}

#[test]
fn weak_ref_temporaries_are_not_kept_alive_until_the_end_of_the_job() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"[
    new WeakRef({}).deref() === undefined,
    new WeakRef(Symbol("temporary")).deref() === undefined
].join("|")"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("true|true"))
    );
}

#[test]
fn finalization_registry_register_unregister_and_same_value_are_observable_from_js() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"
var weakRegistrationLog = "";
var weakRegistry = new FinalizationRegistry(function (held) {
    weakRegistrationLog += held;
});
var weakObjectTarget = {};
var weakSecondObjectTarget = {};
var weakObjectToken = {};
var weakSymbolTarget = Symbol("target");
var weakSymbolToken = Symbol("token");
var weakSameObject = {};
var weakSameSymbol = Symbol("same");
var weakRegistrationResult = [
    weakRegistry.register(weakObjectTarget, "object", weakObjectToken) === undefined,
    weakRegistry.register(weakSecondObjectTarget, "second", weakObjectToken) === undefined,
    weakRegistry.register(weakSymbolTarget, "symbol", weakSymbolToken) === undefined,
    weakRegistry.unregister(weakObjectToken),
    weakRegistry.unregister(weakObjectToken) === false,
    weakRegistry.unregister(weakSymbolToken),
    weakRegistry.unregister(weakSymbolToken) === false,
    (function () {
        try {
            weakRegistry.register(weakSameObject, weakSameObject);
            return false;
        } catch (error) {
            return error instanceof TypeError;
        }
    })(),
    (function () {
        try {
            weakRegistry.register(weakSameSymbol, weakSameSymbol);
            return false;
        } catch (error) {
            return error instanceof TypeError;
        }
    })()
].join("|");
weakObjectTarget = null;
weakSecondObjectTarget = null;
weakObjectToken = null;
weakSymbolTarget = null;
weakSymbolToken = null;
weakSameObject = null;
weakSameSymbol = null;
weakRegistrationResult;
"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "true|true|true|true|true|true|true|true|true"
        ))
    );

    runtime.run_gc().unwrap();
    assert!(!runtime.is_job_pending());
    assert_eq!(
        context.eval("weakRegistrationLog").unwrap(),
        Value::String(JsString::from_static(""))
    );
}

#[test]
fn finalization_jobs_share_the_promise_fifo() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    context
        .eval(
            r#"
var weakFifoLog = "";
var weakFifoThis = null;
var weakFifoRegistry = new FinalizationRegistry(function (held) {
    "use strict";
    weakFifoThis = this;
    weakFifoLog += "finalizer-" + held + ",";
});
var weakFifoTarget = {};
weakFifoRegistry.register(weakFifoTarget, "held");
Promise.resolve().then(function () { weakFifoLog += "promise-before,"; });
weakFifoTarget = null;
"#,
        )
        .unwrap();
    runtime.run_gc().unwrap();
    context
        .eval(
            r#"Promise.resolve().then(function () {
    weakFifoLog += "promise-after,";
});"#,
        )
        .unwrap();

    assert_eq!(
        context.eval("weakFifoLog").unwrap(),
        Value::String(JsString::from_static(""))
    );
    assert_eq!(
        runtime.execute_pending_job().unwrap().context(),
        Some(context.realm)
    );
    assert_eq!(
        context.eval("weakFifoLog").unwrap(),
        Value::String(JsString::from_static("promise-before,"))
    );
    assert_eq!(
        runtime.execute_pending_job().unwrap().context(),
        Some(context.realm)
    );
    assert_eq!(
        context.eval("weakFifoLog").unwrap(),
        Value::String(JsString::from_static("promise-before,finalizer-held,"))
    );
    assert_eq!(
        context.eval("weakFifoThis === undefined").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        runtime.execute_pending_job().unwrap().context(),
        Some(context.realm)
    );
    assert_eq!(
        context.eval("weakFifoLog").unwrap(),
        Value::String(JsString::from_static(
            "promise-before,finalizer-held,promise-after,"
        ))
    );
    assert!(!runtime.execute_pending_job().unwrap().executed());
}

#[test]
fn finalization_target_death_observes_mixed_weak_object_construction_order() {
    let one_pass_runtime = Runtime::new();
    let mut one_pass = one_pass_runtime.new_context();
    one_pass
        .eval(
            r#"
var onePassLog = "";
var onePassMap = new WeakMap();
var onePassRegistry = new FinalizationRegistry(function (held) {
    onePassLog += held;
});
var onePassKey = {};
var onePassTarget = {};
onePassMap.set(onePassKey, onePassTarget);
onePassRegistry.register(onePassTarget, "one-pass");
onePassKey = null;
onePassTarget = null;
"#,
        )
        .unwrap();

    one_pass_runtime.run_gc().unwrap();
    assert!(one_pass_runtime.is_job_pending());
    assert_eq!(
        one_pass_runtime.execute_pending_job().unwrap().context(),
        Some(one_pass.realm)
    );
    assert_eq!(
        one_pass.eval("onePassLog").unwrap(),
        Value::String(JsString::from_static("one-pass"))
    );
    assert!(!one_pass_runtime.is_job_pending());

    let two_pass_runtime = Runtime::new();
    let mut two_pass = two_pass_runtime.new_context();
    two_pass
        .eval(
            r#"
var twoPassLog = "";
var twoPassRegistry = new FinalizationRegistry(function (held) {
    twoPassLog += held;
});
var twoPassMap = new WeakMap();
var twoPassKey = {};
var twoPassTarget = {};
twoPassMap.set(twoPassKey, twoPassTarget);
twoPassRegistry.register(twoPassTarget, "two-pass");
twoPassKey = null;
twoPassTarget = null;
"#,
        )
        .unwrap();

    two_pass_runtime.run_gc().unwrap();
    assert!(!two_pass_runtime.is_job_pending());
    assert_eq!(
        two_pass.eval("twoPassLog").unwrap(),
        Value::String(JsString::from_static(""))
    );
    two_pass_runtime.run_gc().unwrap();
    assert!(two_pass_runtime.is_job_pending());
    assert_eq!(
        two_pass_runtime.execute_pending_job().unwrap().context(),
        Some(two_pass.realm)
    );
    assert_eq!(
        two_pass.eval("twoPassLog").unwrap(),
        Value::String(JsString::from_static("two-pass"))
    );
    assert!(!two_pass_runtime.is_job_pending());
}

#[test]
fn throwing_finalization_callback_does_not_discard_the_next_job() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"
var weakThrowLog = "";
var weakThrowRegistry = new FinalizationRegistry(function (held) {
    weakThrowLog += held + ",";
    if (held === "first")
        throw "cleanup failed";
});
var weakThrowFirst = {};
var weakThrowSecond = {};
weakThrowRegistry.register(weakThrowFirst, "first");
weakThrowRegistry.register(weakThrowSecond, "second");
weakThrowFirst = null;
weakThrowSecond = null;
"#,
        )
        .unwrap();
    runtime.run_gc().unwrap();

    let error = runtime.execute_pending_job().unwrap_err();
    assert_eq!(error.context(), Some(context.realm));
    assert_eq!(error.error(), &RuntimeError::Exception);
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::String(JsString::from_static("cleanup failed")))
    );
    assert!(runtime.is_job_pending());
    assert_eq!(
        runtime.execute_pending_job().unwrap().context(),
        Some(context.realm)
    );
    assert_eq!(
        context.eval("weakThrowLog").unwrap(),
        Value::String(JsString::from_static("first,second,"))
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn unregister_cannot_cancel_an_already_queued_finalization_job() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"
var queuedFinalizationLog = "";
var queuedFinalizationRegistry = new FinalizationRegistry(function (held) {
    queuedFinalizationLog += held;
});
var queuedFinalizationToken = {};
var queuedFinalizationTarget = {};
queuedFinalizationRegistry.register(
    queuedFinalizationTarget,
    "already-queued",
    queuedFinalizationToken
);
queuedFinalizationTarget = null;
"#,
        )
        .unwrap();

    runtime.run_gc().unwrap();
    assert!(runtime.is_job_pending());
    assert_eq!(
        context
            .eval("queuedFinalizationRegistry.unregister(queuedFinalizationToken)")
            .unwrap(),
        Value::Bool(false)
    );
    assert!(runtime.is_job_pending());
    assert_eq!(
        runtime.execute_pending_job().unwrap().context(),
        Some(context.realm)
    );
    assert_eq!(
        context.eval("queuedFinalizationLog").unwrap(),
        Value::String(JsString::from_static("already-queued"))
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn weak_intrinsic_constructor_fallback_uses_the_new_target_realm() {
    let runtime = Runtime::new();
    let mut constructor_context = runtime.new_context();
    let mut target_context = runtime.new_context();
    let weak_ref = global_callable(&runtime, &mut constructor_context, "WeakRef");
    let target_weak_ref = global_callable(&runtime, &mut target_context, "WeakRef");
    let finalization_registry =
        global_callable(&runtime, &mut constructor_context, "FinalizationRegistry");
    let target_finalization_registry =
        global_callable(&runtime, &mut target_context, "FinalizationRegistry");
    let Value::Object(constructor_weak_ref_prototype) =
        own_data_value(&runtime, weak_ref.as_object(), "prototype")
    else {
        panic!("constructor-realm WeakRef prototype was not an object");
    };
    let Value::Object(target_weak_ref_prototype) =
        own_data_value(&runtime, target_weak_ref.as_object(), "prototype")
    else {
        panic!("target-realm WeakRef prototype was not an object");
    };
    let Value::Object(constructor_registry_prototype) =
        own_data_value(&runtime, finalization_registry.as_object(), "prototype")
    else {
        panic!("constructor-realm FinalizationRegistry prototype was not an object");
    };
    let Value::Object(target_registry_prototype) = own_data_value(
        &runtime,
        target_finalization_registry.as_object(),
        "prototype",
    ) else {
        panic!("target-realm FinalizationRegistry prototype was not an object");
    };
    assert_ne!(constructor_weak_ref_prototype, target_weak_ref_prototype);
    assert_ne!(constructor_registry_prototype, target_registry_prototype);
    let Value::Object(new_target_object) = target_context
        .eval("(function ForeignWeakNewTarget(){})")
        .unwrap()
    else {
        panic!("foreign new.target source did not return a function");
    };
    let new_target = runtime
        .as_callable(&new_target_object)
        .unwrap()
        .expect("foreign new.target was not callable");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    assert!(
        target_context
            .define_own_property(
                &new_target_object,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Null),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    let weak_target = constructor_context.new_object().unwrap();
    let Value::Object(weak_instance) = constructor_context
        .construct_with_new_target(&weak_ref, &new_target, &[Value::Object(weak_target)])
        .unwrap()
    else {
        panic!("cross-realm WeakRef construction did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&weak_instance).unwrap(),
        Some(target_weak_ref_prototype)
    );

    let callback = eval_callable(
        &runtime,
        &mut constructor_context,
        "(function cleanupCallback(){})",
    );
    let Value::Object(registry_instance) = constructor_context
        .construct_with_new_target(
            &finalization_registry,
            &new_target,
            &[Value::Object(callback.as_object().clone())],
        )
        .unwrap()
    else {
        panic!("cross-realm FinalizationRegistry construction did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&registry_instance).unwrap(),
        Some(target_registry_prototype)
    );
}
