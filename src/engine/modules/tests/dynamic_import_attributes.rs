use super::*;

#[test]
fn dynamic_import_attributes_snapshot_descriptors_before_any_value_get() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, controls) =
        AttributeModuleLoader::new([("attributes.js", "export const ok = true;")]);
    let _registration = runtime.set_module_loader(loader);
    let promise = eval_dynamic_import(
        &mut context,
        r#"
globalThis.__dynamicAttributeLog = [];
var attributeSymbol = Symbol("ignored");
var attributeTarget = {};
Object.defineProperty(attributeTarget, "a", {
value: "A", enumerable: true, configurable: true
});
Object.defineProperty(attributeTarget, "b", {
value: "B", enumerable: true, configurable: true
});
Object.defineProperty(attributeTarget, attributeSymbol, {
value: "ignored", enumerable: true, configurable: true
});
var attributeProxy = new Proxy(attributeTarget, {
ownKeys: function (target) {
    __dynamicAttributeLog.push("ownKeys");
    return Reflect.ownKeys(target);
},
getOwnPropertyDescriptor: function (target, key) {
    __dynamicAttributeLog.push("descriptor:" + key);
    return Object.getOwnPropertyDescriptor(target, key);
},
get: function (target, key) {
    __dynamicAttributeLog.push("get:" + key);
    if (key === "a") {
        Object.defineProperty(target, "b", {
            value: "B", enumerable: false, configurable: true
        });
    }
    return target[key];
}
});
import("attributes.js", { with: attributeProxy })
"#,
        "entry.js",
    );
    assert_script_true(
        &mut context,
        "__dynamicAttributeLog.join(',') === 'ownKeys,descriptor:a,descriptor:b,get:a,get:b'",
    );
    assert_eq!(
        controls.checks.borrow().as_slice(),
        [vec![
            ("a".to_owned(), "A".to_owned()),
            ("b".to_owned(), "B".to_owned()),
        ]]
    );
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        controls.loads.borrow().as_slice(),
        [RecordedAttributeLoad {
            name: "attributes.js".to_owned(),
            attributes: Some(vec![
                ("a".to_owned(), "A".to_owned()),
                ("b".to_owned(), "B".to_owned()),
            ]),
        }]
    );
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
}

#[test]
fn dynamic_import_empty_attributes_still_reach_checker_and_loader() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, controls) =
        AttributeModuleLoader::new([("empty-attributes.js", "export const ok = true;")]);
    let _registration = runtime.set_module_loader(loader);
    let promise = eval_dynamic_import(
        &mut context,
        "import('empty-attributes.js', { with: {} })",
        "entry.js",
    );

    assert_eq!(controls.checks.borrow().as_slice(), [Vec::new()]);
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        controls.loads.borrow().as_slice(),
        [RecordedAttributeLoad {
            name: "empty-attributes.js".to_owned(),
            attributes: Some(Vec::new()),
        }]
    );
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
}

#[test]
fn dynamic_import_rejects_non_string_attribute_values_before_enqueue() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, controls) =
        AttributeModuleLoader::new([("bad-attributes.js", "export const ok = true;")]);
    let _registration = runtime.set_module_loader(loader);
    let promise = eval_dynamic_import(
        &mut context,
        "import('bad-attributes.js', { with: { type: 42 } })",
        "entry.js",
    );

    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Rejected
    );
    assert!(controls.checks.borrow().is_empty());
    assert!(controls.loads.borrow().is_empty());
    assert!(!runtime.is_job_pending());
}
