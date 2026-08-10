use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CompleteOrdinaryPropertyDescriptor, DescriptorField, OrdinaryPropertyDescriptor, Runtime,
    RuntimeError, Value,
};

const ORACLE_PROBE: &str = r#"
var f = (0, function(a, b) {});
function bits(d) {
    return Number(d.writable) + "," + Number(d.enumerable) + "," + Number(d.configurable);
}
var nameDesc = Object.getOwnPropertyDescriptor(f, "name");
var lengthDesc = Object.getOwnPropertyDescriptor(f, "length");
print("name=" + nameDesc.value + "|" + bits(nameDesc));
print("length=" + lengthDesc.value + "|" + bits(lengthDesc));
print("lazy-before-get=" + Object.prototype.hasOwnProperty.call(f, "prototype") + "|" + Reflect.ownKeys(f).join(","));
var prototypeDesc = Object.getOwnPropertyDescriptor(f, "prototype");
print("prototype=" + bits(prototypeDesc));
print("keys=" + Reflect.ownKeys(f).join(","));
print("function-prototype-prefix=" + Reflect.ownKeys(Function.prototype).slice(0, 2).join(","));
print("inferred-direct=" + (function(){ var inferred = function(){}; return inferred; })().name);
print("inferred-parenthesized=" + (function(){ var inferred = (((function(){}))); return inferred; })().name);
print("inferred-escaped=" + (function(){ var \u0066 = function(){}; return f; })().name);
print("not-inferred-comma=" + (function(){ var inferred = (0, function(){}); return inferred; })().name);
print("not-inferred-conditional=" + (function(){ var inferred = true ? function(){} : function(){}; return inferred; })().name);
print("not-inferred-logical=" + (function(){ var inferred = 0 || function(){}; return inferred; })().name);
print("named-recursion=" + (function factorial(n){ return n <= 1 ? 1 : n * factorial(n - 1); })(5));
var namedIdentity = function self(){ return self; };
print("named-self-identity=" + (namedIdentity() === namedIdentity));
var namedDescendant = function self(){ return function(){ return self; }; };
print("named-descendant-capture=" + (namedDescendant()() === namedDescendant));
print("named-parameter-shadow=" + (function self(self){ return self; })(7));
print("named-var-shadow=" + (function self(){ var self = 8; return self; })());
var namedSloppy = function self(){ var before = self; var assigned = (self = 9); return (self === before) + "|" + assigned; };
print("named-sloppy-self-assignment=" + namedSloppy());
try { (function self(){ "use strict"; self = 9; })(); } catch (e) { print("named-strict-self-assignment=" + e.name + "|" + e.message); }
var namedMeta = function explicit(a, b){};
var namedNameDesc = Object.getOwnPropertyDescriptor(namedMeta, "name");
var namedLengthDesc = Object.getOwnPropertyDescriptor(namedMeta, "length");
print("named-name=" + namedNameDesc.value + "|" + bits(namedNameDesc));
print("named-length=" + namedLengthDesc.value + "|" + bits(namedLengthDesc));
print("named-before-prototype=" + Object.prototype.hasOwnProperty.call(namedMeta, "prototype") + "|" + Reflect.ownKeys(namedMeta).join(","));
var namedPrototypeDesc = Object.getOwnPropertyDescriptor(namedMeta, "prototype");
print("named-prototype=" + bits(namedPrototypeDesc) + "|" + (namedPrototypeDesc.value.constructor === namedMeta));
print("named-keys=" + Reflect.ownKeys(namedMeta).join(","));
print("constructor=" + (prototypeDesc.value.constructor === f));
print("prototype-chain=" + (Object.getPrototypeOf(prototypeDesc.value) === Object.prototype));
print("lazy-stable=" + (f.prototype === prototypeDesc.value) + "|" + (f.prototype.constructor === f) + "|" + (Object.getPrototypeOf(f.prototype) === Object.prototype));
var compatible = function(){};
Object.defineProperty(compatible, "prototype", {});
print("lazy-compatible-define=" + (compatible.prototype.constructor === compatible));
var incompatible = function(){}, rejected = false;
try { Object.defineProperty(incompatible, "prototype", { configurable: true }); } catch (e) { rejected = e instanceof TypeError; }
print("lazy-incompatible-define=" + rejected);
var explicitF = function(){ return new.target }, explicitG = function(){};
print("explicit-new-target=" + (Reflect.construct(explicitF, [], explicitG) === explicitG));
Object.defineProperty(explicitG, "prototype", { value: null });
print("new-target-prototype-fallback=" + (Object.getPrototypeOf(Reflect.construct(function(){}, [], explicitG)) === Object.prototype));
var baseInstance = new f();
print("construct-prototype=" + (Object.getPrototypeOf(baseInstance) === f.prototype));
print("construct-result-primitive-fallback=" + (function(){ var F=function(){return 1}; return typeof new F() === "object"; })());
print("construct-object-override=" + (function(){ var marker=function(){}; var F=function(){return marker}; return new F() === marker; })());
print("construct-new-target=" + (function(){ var F=function(){return new.target}; return new F() === F; })());
print("call-new-target=" + (function(){ var F=function(){return new.target}; return typeof F() === "undefined"; })());
var fp = Function.prototype;
var fpLengthDesc = Object.getOwnPropertyDescriptor(fp, "length");
var fpNameDesc = Object.getOwnPropertyDescriptor(fp, "name");
print("function-prototype=" + String(fp()) + "|" + bits(fpLengthDesc) + "|" + bits(fpNameDesc) + "|" + (Object.getOwnPropertyDescriptor(fp, "prototype") === undefined));
Object.defineProperty(fp, "length", { value: 99 });
print("native-length-independent=" + fp.length + "|" + (fp() === undefined));
try { Reflect.construct(fp, []); } catch (e) { print("function-prototype-construct-error=" + e.name + "|" + e.message); }
"#;

#[test]
fn ordinary_function_object_kernel_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP function-object oracle differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(rust_observations(), oracle_observations(&oracle));
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(function) = context.eval("(0, function(a, b) {})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let constructor = runtime.intern_property_key("constructor").unwrap();
    let message = runtime.intern_property_key("message").unwrap();

    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::String(name_value),
        writable: name_writable,
        enumerable: name_enumerable,
        configurable: name_configurable,
    } = runtime.get_own_property(&function, &name).unwrap().unwrap()
    else {
        panic!("unexpected name descriptor");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Int(length_value),
        writable: length_writable,
        enumerable: length_enumerable,
        configurable: length_configurable,
    } = runtime
        .get_own_property(&function, &length)
        .unwrap()
        .unwrap()
    else {
        panic!("unexpected length descriptor");
    };
    let lazy_has_prototype = runtime.has_own_property(&function, &prototype_key).unwrap();
    let function_keys = runtime
        .own_property_keys(&function)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect::<Vec<_>>()
        .join(",");
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(prototype),
        writable: prototype_writable,
        enumerable: prototype_enumerable,
        configurable: prototype_configurable,
    } = runtime
        .get_own_property(&function, &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("unexpected prototype descriptor");
    };
    let constructor_matches =
        context.get_property(&prototype, &constructor).unwrap() == Value::Object(function.clone());
    let prototype_chain_matches = runtime.get_prototype_of(&prototype).unwrap().unwrap()
        == context.object_prototype().unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(second_prototype),
        ..
    } = runtime
        .get_own_property(&function, &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("second prototype read did not produce an object");
    };
    let lazy_identity_stable = second_prototype == prototype;
    let function_prototype = context.function_prototype().unwrap();
    let function_prototype_prefix = runtime
        .own_property_keys(&function_prototype)
        .unwrap()
        .into_iter()
        .take(2)
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect::<Vec<_>>()
        .join(",");
    let inferred_direct = function_name(
        &runtime,
        &mut context,
        "(function(){ var inferred = function(){}; return inferred; })()",
    );
    let inferred_parenthesized = function_name(
        &runtime,
        &mut context,
        "(function(){ var inferred = (((function(){}))); return inferred; })()",
    );
    let inferred_escaped = function_name(
        &runtime,
        &mut context,
        "(function(){ var \\u0066 = function(){}; return f; })()",
    );
    let not_inferred_comma = function_name(
        &runtime,
        &mut context,
        "(function(){ var inferred = (0, function(){}); return inferred; })()",
    );
    let not_inferred_conditional = function_name(
        &runtime,
        &mut context,
        "(function(){ var inferred = true ? function(){} : function(){}; return inferred; })()",
    );
    let not_inferred_logical = function_name(
        &runtime,
        &mut context,
        "(function(){ var inferred = 0 || function(){}; return inferred; })()",
    );
    let Value::Int(named_recursion) = context
        .eval("(function factorial(n){ return n <= 1 ? 1 : n * factorial(n - 1); })(5)")
        .unwrap()
    else {
        panic!("named recursion probe did not produce an integer");
    };
    let named_self_identity = eval_boolean(
        &mut context,
        "(function(){ var namedIdentity = function self(){ return self; }; return namedIdentity() === namedIdentity; })()",
    );
    let named_descendant_capture = eval_boolean(
        &mut context,
        "(function(){ var namedDescendant = function self(){ return function(){ return self; }; }; return namedDescendant()() === namedDescendant; })()",
    );
    let Value::Int(named_parameter_shadow) = context
        .eval("(function self(self){ return self; })(7)")
        .unwrap()
    else {
        panic!("named parameter-shadow probe did not produce an integer");
    };
    let Value::Int(named_var_shadow) = context
        .eval("(function self(){ var self = 8; return self; })()")
        .unwrap()
    else {
        panic!("named var-shadow probe did not produce an integer");
    };
    let Value::String(named_sloppy_self_assignment) = context
        .eval(
            "(function self(){ var before = self; var assigned = (self = 9); return (self === before) + '|' + assigned; })()",
        )
        .unwrap()
    else {
        panic!("sloppy named self-assignment probe did not produce a string");
    };
    assert!(matches!(
        context.eval("(function self(){ 'use strict'; self = 9; })()"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(named_strict_exception) = context.take_exception().unwrap().unwrap() else {
        panic!("strict named self-assignment error was not an object");
    };
    let Value::String(named_strict_error_name) = context
        .get_property(&named_strict_exception, &name)
        .unwrap()
    else {
        panic!("strict named self-assignment error name was not a string");
    };
    let Value::String(named_strict_error_message) = context
        .get_property(&named_strict_exception, &message)
        .unwrap()
    else {
        panic!("strict named self-assignment error message was not a string");
    };

    let Value::Object(named_meta) = context.eval("(0, function explicit(a, b){})").unwrap() else {
        panic!("named metadata probe did not produce an object");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::String(named_name_value),
        writable: named_name_writable,
        enumerable: named_name_enumerable,
        configurable: named_name_configurable,
    } = runtime
        .get_own_property(&named_meta, &name)
        .unwrap()
        .unwrap()
    else {
        panic!("unexpected named function name descriptor");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Int(named_length_value),
        writable: named_length_writable,
        enumerable: named_length_enumerable,
        configurable: named_length_configurable,
    } = runtime
        .get_own_property(&named_meta, &length)
        .unwrap()
        .unwrap()
    else {
        panic!("unexpected named function length descriptor");
    };
    let named_has_prototype = runtime
        .has_own_property(&named_meta, &prototype_key)
        .unwrap();
    let named_keys_before = runtime
        .own_property_keys(&named_meta)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect::<Vec<_>>()
        .join(",");
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(named_prototype),
        writable: named_prototype_writable,
        enumerable: named_prototype_enumerable,
        configurable: named_prototype_configurable,
    } = runtime
        .get_own_property(&named_meta, &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("unexpected named function prototype descriptor");
    };
    let named_prototype_constructor = context
        .get_property(&named_prototype, &constructor)
        .unwrap()
        == Value::Object(named_meta.clone());
    let named_keys = runtime
        .own_property_keys(&named_meta)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect::<Vec<_>>()
        .join(",");

    let Value::Object(compatible) = context.eval("(0, function(){})").unwrap() else {
        panic!("compatible lazy function probe did not produce an object");
    };
    assert!(
        runtime
            .define_own_property(
                &compatible,
                &prototype_key,
                &OrdinaryPropertyDescriptor::new(),
            )
            .unwrap()
    );
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(compatible_prototype),
        ..
    } = runtime
        .get_own_property(&compatible, &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("compatible define did not materialize the prototype");
    };
    let lazy_compatible_define = context
        .get_property(&compatible_prototype, &constructor)
        .unwrap()
        == Value::Object(compatible.clone());

    let Value::Object(incompatible) = context.eval("(0, function(){})").unwrap() else {
        panic!("incompatible lazy function probe did not produce an object");
    };
    let lazy_incompatible_define = !runtime
        .define_own_property(
            &incompatible,
            &prototype_key,
            &OrdinaryPropertyDescriptor {
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
        .unwrap();

    let callable = runtime.as_callable(&function).unwrap().unwrap();
    let Value::Object(explicit_f_object) = context
        .eval("(0, function(){ return new.target; })")
        .unwrap()
    else {
        panic!("explicit-new-target constructor did not produce an object");
    };
    let explicit_f = runtime.as_callable(&explicit_f_object).unwrap().unwrap();
    let Value::Object(explicit_g_object) = context.eval("(0, function(){})").unwrap() else {
        panic!("explicit new.target did not produce an object");
    };
    let explicit_g = runtime.as_callable(&explicit_g_object).unwrap().unwrap();
    let explicit_new_target = context
        .construct_with_new_target(&explicit_f, &explicit_g, &[])
        .unwrap()
        == Value::Object(explicit_g_object.clone());
    assert!(
        runtime
            .define_own_property(
                &explicit_g_object,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Null),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(fallback_instance) = context
        .construct_with_new_target(&callable, &explicit_g, &[])
        .unwrap()
    else {
        panic!("primitive newTarget.prototype fallback did not produce an object");
    };
    let new_target_prototype_fallback = runtime
        .get_prototype_of(&fallback_instance)
        .unwrap()
        .unwrap()
        == context.object_prototype().unwrap();

    let Value::Object(base_instance) = context.construct(&callable, &[]).unwrap() else {
        panic!("ordinary constructor did not produce an object");
    };
    let construct_prototype_matches =
        runtime.get_prototype_of(&base_instance).unwrap() == Some(prototype.clone());
    let construct_result_primitive_fallback = eval_boolean(
        &mut context,
        "(function(){ var F=function(){return 1}; return typeof new F() === 'object'; })()",
    );
    let construct_object_override = eval_boolean(
        &mut context,
        "(function(){ var marker=function(){}; var F=function(){return marker}; return new F() === marker; })()",
    );
    let construct_new_target = eval_boolean(
        &mut context,
        "(function(){ var F=function(){return new.target}; return new F() === F; })()",
    );
    let call_new_target = eval_boolean(
        &mut context,
        "(function(){ var F=function(){return new.target}; return typeof F() === 'undefined'; })()",
    );

    let CompleteOrdinaryPropertyDescriptor::Data {
        writable: fp_length_writable,
        enumerable: fp_length_enumerable,
        configurable: fp_length_configurable,
        ..
    } = runtime
        .get_own_property(&function_prototype, &length)
        .unwrap()
        .unwrap()
    else {
        panic!("Function.prototype length was not a data property");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        writable: fp_name_writable,
        enumerable: fp_name_enumerable,
        configurable: fp_name_configurable,
        ..
    } = runtime
        .get_own_property(&function_prototype, &name)
        .unwrap()
        .unwrap()
    else {
        panic!("Function.prototype name was not a data property");
    };
    let fp_has_no_prototype = runtime
        .get_own_property(&function_prototype, &prototype_key)
        .unwrap()
        .is_none();
    let function_prototype_callable = runtime.as_callable(&function_prototype).unwrap().unwrap();
    let fp_call_is_undefined = context
        .call(&function_prototype_callable, Value::Undefined, &[])
        .unwrap()
        == Value::Undefined;
    assert!(
        runtime
            .define_own_property(
                &function_prototype,
                &length,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(99)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let native_length_independent = context.get_property(&function_prototype, &length).unwrap()
        == Value::Int(99)
        && context
            .call(&function_prototype_callable, Value::Undefined, &[])
            .unwrap()
            == Value::Undefined;
    assert!(matches!(
        context.construct(&function_prototype_callable, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(construct_exception) = context.take_exception().unwrap().unwrap() else {
        panic!("Function.prototype construct error was not an object");
    };
    let Value::String(construct_error_name) =
        context.get_property(&construct_exception, &name).unwrap()
    else {
        panic!("Function.prototype construct error name was not a string");
    };
    let Value::String(construct_error_message) = context
        .get_property(&construct_exception, &message)
        .unwrap()
    else {
        panic!("Function.prototype construct error message was not a string");
    };

    vec![
        format!(
            "name={}|{}",
            name_value.to_utf8_lossy(),
            bits(name_writable, name_enumerable, name_configurable)
        ),
        format!(
            "length={length_value}|{}",
            bits(length_writable, length_enumerable, length_configurable)
        ),
        format!("lazy-before-get={lazy_has_prototype}|{function_keys}"),
        format!(
            "prototype={}",
            bits(
                prototype_writable,
                prototype_enumerable,
                prototype_configurable
            )
        ),
        format!("keys={function_keys}"),
        format!("function-prototype-prefix={function_prototype_prefix}"),
        format!("inferred-direct={inferred_direct}"),
        format!("inferred-parenthesized={inferred_parenthesized}"),
        format!("inferred-escaped={inferred_escaped}"),
        format!("not-inferred-comma={not_inferred_comma}"),
        format!("not-inferred-conditional={not_inferred_conditional}"),
        format!("not-inferred-logical={not_inferred_logical}"),
        format!("named-recursion={named_recursion}"),
        format!("named-self-identity={named_self_identity}"),
        format!("named-descendant-capture={named_descendant_capture}"),
        format!("named-parameter-shadow={named_parameter_shadow}"),
        format!("named-var-shadow={named_var_shadow}"),
        format!(
            "named-sloppy-self-assignment={}",
            named_sloppy_self_assignment.to_utf8_lossy()
        ),
        format!(
            "named-strict-self-assignment={}|{}",
            named_strict_error_name.to_utf8_lossy(),
            named_strict_error_message.to_utf8_lossy()
        ),
        format!(
            "named-name={}|{}",
            named_name_value.to_utf8_lossy(),
            bits(
                named_name_writable,
                named_name_enumerable,
                named_name_configurable
            )
        ),
        format!(
            "named-length={named_length_value}|{}",
            bits(
                named_length_writable,
                named_length_enumerable,
                named_length_configurable
            )
        ),
        format!("named-before-prototype={named_has_prototype}|{named_keys_before}"),
        format!(
            "named-prototype={}|{named_prototype_constructor}",
            bits(
                named_prototype_writable,
                named_prototype_enumerable,
                named_prototype_configurable
            )
        ),
        format!("named-keys={named_keys}"),
        format!("constructor={constructor_matches}"),
        format!("prototype-chain={prototype_chain_matches}"),
        format!(
            "lazy-stable={lazy_identity_stable}|{constructor_matches}|{prototype_chain_matches}"
        ),
        format!("lazy-compatible-define={lazy_compatible_define}"),
        format!("lazy-incompatible-define={lazy_incompatible_define}"),
        format!("explicit-new-target={explicit_new_target}"),
        format!("new-target-prototype-fallback={new_target_prototype_fallback}"),
        format!("construct-prototype={construct_prototype_matches}"),
        format!("construct-result-primitive-fallback={construct_result_primitive_fallback}"),
        format!("construct-object-override={construct_object_override}"),
        format!("construct-new-target={construct_new_target}"),
        format!("call-new-target={call_new_target}"),
        format!(
            "function-prototype={}|{}|{}|{}",
            if fp_call_is_undefined {
                "undefined"
            } else {
                "not-undefined"
            },
            bits(
                fp_length_writable,
                fp_length_enumerable,
                fp_length_configurable
            ),
            bits(fp_name_writable, fp_name_enumerable, fp_name_configurable),
            fp_has_no_prototype
        ),
        format!("native-length-independent=99|{native_length_independent}"),
        format!(
            "function-prototype-construct-error={}|{}",
            construct_error_name.to_utf8_lossy(),
            construct_error_message.to_utf8_lossy()
        ),
    ]
}

fn eval_boolean(context: &mut quickjs_oxide::Context, source: &str) -> bool {
    let Value::Bool(value) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("boolean function probe failed for {source:?}: {error}"))
    else {
        panic!("boolean function probe did not produce a boolean for {source:?}");
    };
    value
}

fn function_name(runtime: &Runtime, context: &mut quickjs_oxide::Context, source: &str) -> String {
    let Value::Object(function) = context.eval(source).unwrap() else {
        panic!("function-name probe did not produce an object");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::String(name),
        ..
    } = runtime.get_own_property(&function, &name).unwrap().unwrap()
    else {
        panic!("function-name probe did not produce a string data property");
    };
    name.to_utf8_lossy()
}

fn bits(writable: bool, enumerable: bool, configurable: bool) -> String {
    format!(
        "{},{},{}",
        u8::from(writable),
        u8::from(enumerable),
        u8::from(configurable)
    )
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS function-object oracle");
    assert!(
        output.status.success(),
        "QuickJS function-object oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS function-object oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
