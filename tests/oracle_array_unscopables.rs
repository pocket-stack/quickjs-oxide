use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, PropertyKey, Runtime, Value,
    WellKnownSymbol,
};

// This target pins QuickJS 2026-06-04 `js_array_unscopables_funcs` and the
// Symbol.unscopables-specific null-prototype `JS_DEF_OBJECT` autoinit path.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "repeated reads preserve identity and inner mutations",
        r#"(function(){
            var first=Array.prototype[Symbol.unscopables];
            var second=Array.prototype[Symbol.unscopables];
            first.at=false;first.extra=7;
            return (first===second)+"|"+second.at+"|"+second.extra+"|"+
                (Object.getPrototypeOf(first)===null);
        })()"#,
    ),
    (
        "sloppy assignment cannot replace the outer read-only property",
        r#"(function(){
            var prototype=Array.prototype,original=prototype[Symbol.unscopables];
            prototype[Symbol.unscopables]=Object();
            return prototype[Symbol.unscopables]===original;
        })()"#,
    ),
    (
        "the configurable outer property can be replaced with new attributes",
        r#"(function(){
            var prototype=Array.prototype,replacement=Object(),descriptor=Object();
            descriptor.value=replacement;descriptor.writable=true;descriptor.enumerable=true;
            descriptor.configurable=true;
            Object.defineProperty(prototype,Symbol.unscopables,descriptor);
            prototype[Symbol.unscopables]=9;
            return prototype[Symbol.unscopables]+"|"+(replacement!==prototype[Symbol.unscopables]);
        })()"#,
    ),
    (
        "the outer autoinit property can be deleted before its first Get",
        r#"(function(){
            var prototype=Array.prototype;
            var deleted=delete prototype[Symbol.unscopables];
            return deleted+"|"+(prototype[Symbol.unscopables]===undefined)+"|"+
                (Symbol.unscopables in prototype);
        })()"#,
    ),
    (
        "every inner property is writable enumerable and configurable in behavior",
        r#"(function(){
            var value=Array.prototype[Symbol.unscopables];
            value.at=false;var deleted=delete value.flat;value.added=true;
            return value.at+"|"+deleted+"|"+("flat" in value)+"|"+value.added+"|"+
                Object.prototype.propertyIsEnumerable.call(value,"at")+"|"+
                Object.prototype.propertyIsEnumerable.call(value,"added");
        })()"#,
    ),
    (
        "the pinned table excludes with and includes every completed Array method",
        r#"(function(){
            var value=Array.prototype[Symbol.unscopables];
            return ("with" in value)+"|"+value.at+value.copyWithin+value.entries+value.fill+
                value.find+value.findIndex+value.findLast+value.findLastIndex+value.flat+
                value.flatMap+value.includes+value.keys+value.toReversed+value.toSorted+
                value.toSpliced+value.values;
        })()"#,
    ),
];

const GRAPH_ORACLE: &str = r#"
var prototype=Array.prototype,own=Reflect.ownKeys(prototype),symbols=[];
for(var i=0;i<own.length;i++) {
  if(own[i]===Symbol.iterator)symbols[symbols.length]='iterator';
  if(own[i]===Symbol.unscopables)symbols[symbols.length]='unscopables';
}
function bits(descriptor) {
  return 'D'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable);
}
var outer=Object.getOwnPropertyDescriptor(prototype,Symbol.unscopables),value=outer.value;
var keys=Reflect.ownKeys(value),entries=[];
for(var i=0;i<keys.length;i++) {
  var descriptor=Object.getOwnPropertyDescriptor(value,keys[i]);
  entries[entries.length]=keys[i]+':'+descriptor.value+':'+bits(descriptor);
}
print('symbols='+symbols.join(',')+':own='+own.length);
print('outer='+bits(outer)+':null-proto='+(Object.getPrototypeOf(value)===null)+
      ':array='+Array.isArray(value));
print('keys='+keys.join(','));
print('entries='+entries.join('|'));
"#;

#[test]
fn array_unscopables_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array unscopables oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in VALUE_CASES {
        let observation = observe_oracle(&oracle, source, description);
        assert!(
            observation.starts_with("return|"),
            "unscopables oracle vector did not return for {description}: {observation:?}",
        );
    }
    assert_eq!(oracle_graph_observations(&oracle).len(), 4);
}

#[test]
fn array_unscopables_values_identity_and_mutation_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array unscopables values: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle(&oracle, source, description),
            "Array unscopables behavior drifted for {description}",
        );
    }
}

#[test]
fn array_unscopables_graph_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array unscopables graph: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array unscopables graph or descriptor table drifted",
    );
}

#[test]
fn array_unscopables_are_distinct_null_prototype_objects_per_realm() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Unscopables));
    let first_prototype = first.array_prototype().unwrap();
    let second_prototype = second.array_prototype().unwrap();
    let Value::Object(first_value) = first.get_property(&first_prototype, &key).unwrap() else {
        panic!("first realm unscopables value was not an object");
    };
    let Value::Object(second_value) = second.get_property(&second_prototype, &key).unwrap() else {
        panic!("second realm unscopables value was not an object");
    };
    assert_ne!(first_value, second_value);
    assert_eq!(runtime.get_prototype_of(&first_value).unwrap(), None);
    assert_eq!(runtime.get_prototype_of(&second_value).unwrap(), None);
    assert!(bool_property(&runtime, &mut first, &first_value, "at"));
    assert!(bool_property(
        &runtime,
        &mut second,
        &second_value,
        "flatMap",
    ));
}

fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    let value = context
        .eval(source)
        .unwrap_or_else(|error| panic!("Rust rejected {description} ({source:?}): {error}"));
    format!(
        "return|{}|{}",
        value_type(runtime, &value),
        primitive_value_text(value),
    )
}

fn observe_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
    let wrapper = r#"
try {
  var value = std.evalScript(scriptArgs[0]);
  print('return|' + typeof value + '|' + String(value));
} catch (error) {
  if (error !== null && typeof error === 'object')
    print('throw|object|' + error.name + '|' + error.message);
  else
    print('throw|' + typeof error + '|' + String(error));
}
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS observer failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"))
        .trim_end()
        .to_owned()
}

fn rust_graph_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let prototype = context.array_prototype().unwrap();
    let iterator = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
    let unscopables = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Unscopables));
    let own_keys = runtime.own_property_keys(&prototype).unwrap();
    let own_key_count = own_keys.len();
    let symbols = own_keys
        .into_iter()
        .filter_map(|key| {
            if key == iterator {
                Some("iterator")
            } else if key == unscopables {
                Some("unscopables")
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let descriptor = runtime
        .get_own_property(&prototype, &unscopables)
        .unwrap()
        .expect("missing Array.prototype[Symbol.unscopables]");
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(value),
        writable,
        enumerable,
        configurable,
    } = descriptor
    else {
        panic!("Array.prototype[Symbol.unscopables] was not an object data property");
    };
    let keys = runtime
        .own_property_keys(&value)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect::<Vec<_>>();
    let entries = keys
        .iter()
        .map(|name| {
            let key = runtime.intern_property_key(name).unwrap();
            let descriptor = runtime
                .get_own_property(&value, &key)
                .unwrap()
                .unwrap_or_else(|| panic!("missing unscopables.{name}"));
            let CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Bool(entry),
                writable,
                enumerable,
                configurable,
            } = descriptor
            else {
                panic!("unscopables.{name} was not a boolean data property");
            };
            format!(
                "{name}:{entry}:D{}{}{}",
                Number(writable),
                Number(enumerable),
                Number(configurable),
            )
        })
        .collect::<Vec<_>>();
    vec![
        format!("symbols={}:own={own_key_count}", symbols.join(",")),
        format!(
            "outer=D{}{}{}:null-proto={}:array={}",
            Number(writable),
            Number(enumerable),
            Number(configurable),
            runtime.get_prototype_of(&value).unwrap().is_none(),
            runtime.is_array_object(&value).unwrap(),
        ),
        format!("keys={}", keys.join(",")),
        format!("entries={}", entries.join("|")),
    ]
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS unscopables graph: {error}"));
    assert!(
        output.status.success(),
        "QuickJS unscopables graph failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS unscopables graph output was not UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn bool_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> bool {
    let Value::Bool(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not a boolean property");
    };
    value
}

fn value_type(runtime: &Runtime, value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Null => "object",
        Value::Bool(_) => "boolean",
        Value::Int(_) | Value::Float(_) => "number",
        Value::BigInt(_) => "bigint",
        Value::String(_) => "string",
        Value::Object(object) => {
            if runtime.as_callable(object).unwrap().is_some() {
                "function"
            } else {
                "object"
            }
        }
        Value::Symbol(_) => "symbol",
    }
}

fn primitive_value_text(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => quickjs_oxide::value::number_to_string(value),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "<object>".to_owned(),
        Value::Symbol(_) => "<symbol>".to_owned(),
    }
}

struct Number(bool);

impl std::fmt::Display for Number {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0 { "1" } else { "0" })
    }
}
