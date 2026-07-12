use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{
    AccessorValue, Context, DescriptorField, JsString, OrdinaryPropertyDescriptor, Runtime,
    RuntimeError, Value,
};

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "if condition sees the entry closure before the authored Annex closure",
        "(function(){var entry;if((entry=single,true))function single(){return single};return typeof entry+'|'+(entry!==single)+'|'+(entry()===entry)+'|'+(single()===entry)})()",
    ),
    (
        "skipped if arm still declares an undefined Annex outer",
        "(function(){if(false)function skipped(){};return typeof skipped+'|'+delete skipped})()",
    ),
    (
        "same-name if else keeps the last lexical and first Annex child",
        "(function(){var seen;if((seen=duplicate,true))function duplicate(){return 1}else function duplicate(){return 2};return seen()+'|'+duplicate()+'|'+(seen!==duplicate)})()",
    ),
    (
        "executed duplicate else does not replace an existing Annex outer",
        "(function(){var duplicate=7,seen;if((seen=duplicate,false))function duplicate(){return 1}else function duplicate(){return 2};return seen()+'|'+duplicate})()",
    ),
    (
        "same-name parameter suppresses only the Annex outer write",
        "(function(parameter){var entry;if((entry=parameter,true))function parameter(){return 4};return typeof entry+'|'+parameter+'|'+entry()})(7)",
    ),
    (
        "prior lexical suppresses an if-arm Annex outer without hiding its entry",
        "(function(){var entry;let shadow=7;if((entry=shadow,true))function shadow(){return 8};return entry()+'|'+shadow})()",
    ),
    (
        "nested if re-enters fresh lexical closures on every loop iteration",
        "(function(){var i=0,first,second;while(i<2)if((i++,i===1?first=looped:second=looped,true))function looped(){return looped};return (first!==second)+'|'+(first()===first)+'|'+(second()===second)+'|'+(looped()===second)})()",
    ),
    (
        "continue closes an escaped if lexical before the next iteration",
        "(function(){var i=0,first,second;while(i<2)if((i++,i===1?first=fresh:second=fresh,false))function fresh(){return fresh}else continue;return (first!==second)+'|'+(first()===first)+'|'+(second()===second)+'|'+typeof fresh})()",
    ),
    (
        "label chain adds no lexical scope around its function",
        "(function(){var entry;{a:b:function labelled(){return labelled};entry=labelled}return (entry()===entry)+'|'+(labelled()===entry)+'|'+(entry!==labelled)})()",
    ),
    (
        "labelled duplicates share block first-Annex last-lexical behavior",
        "(function(){var entry;{a:function labelled(){return 1};b:function labelled(){return 2};entry=labelled}return entry()+'|'+labelled()+'|'+(entry===labelled)})()",
    ),
    (
        "Program labelled function has one global self identity",
        "programLabel:function programLabel(){return programLabel};programLabel()===programLabel",
    ),
    (
        "repeated Program labels overwrite the global at each source position",
        "a:function programDuplicate(){return 1};b:function programDuplicate(){return 2};programDuplicate()",
    ),
    (
        "Program label source write follows a later direct function hoist",
        "programOrderLabel:function programOrder(){return 1};function programOrder(){return 2};programOrder()",
    ),
    (
        "Program var initializer runs after a labelled source write",
        "programVarLabel:function programVarLabel(){return 1};var programVarLabel=function(){return 2};programVarLabel()",
    ),
    (
        "FunctionBody label lexical shadows a same-name parameter from entry",
        "(function(parameter){var before=typeof parameter;label:function parameter(){return 4};return before+'|'+parameter()})(7)",
    ),
    (
        "FunctionBody arguments label shadows the implicit binding without Annex",
        "(function(){var before=typeof arguments;label:function arguments(){return 6};return before+'|'+arguments()})()",
    ),
    (
        "Function constructor supports if-arm and labelled Annex B forms",
        "Function(\"if(true)function one(){return 10};tag:function two(){return 11};return one()+'|'+two()\")()",
    ),
    (
        "labels may wrap an if which reopens its own Annex B mask",
        "(function(){outer:inner:if(true)function nested(){return 12};return nested()})()",
    ),
    (
        "loop body if reopens Annex B even when the loop never executes",
        "(function(){while(false)if(true)function nestedLoop(){};return typeof nestedLoop+'|'+delete nestedLoop})()",
    ),
    (
        "skipped Program arm permits the later lexical initializer",
        "if(false)function laterFalse(){};let laterFalse=7;laterFalse",
    ),
    (
        "first Annex global record lets a later label replace an initialized lexical",
        "if(false){function firstAnnexRecord(){return 0}};let firstAnnexRecord=3;label:function firstAnnexRecord(){return 2};typeof firstAnnexRecord+'|'+firstAnnexRecord()",
    ),
    (
        "first Annex global record masks a later lexical for a block function",
        "if(false){function maskedBlockLexical(){return 0}};let maskedBlockLexical=3;{function maskedBlockLexical(){return 2}};typeof maskedBlockLexical+'|'+maskedBlockLexical()",
    ),
    (
        "first Annex global record masks a later lexical for an if-arm function",
        "if(false){function maskedIfLexical(){return 0}};let maskedIfLexical=3;if(true)function maskedIfLexical(){return 2};typeof maskedIfLexical+'|'+maskedIfLexical()",
    ),
    (
        "first Annex global record masks repeated Program lexicals",
        "if(false)function repeatedLexical(){};let repeatedLexical=1;let repeatedLexical=2;repeatedLexical",
    ),
    (
        "first Annex global record masks a later Program var",
        "if(false)function varAfterLexical(){};let varAfterLexical=1;var varAfterLexical=2;varAfterLexical",
    ),
    (
        "first lexical kind controls mutation after masked duplicate declarations",
        "if(false)function firstLetControls(){};let firstLetControls=1;const firstLetControls=2;firstLetControls=3;firstLetControls",
    ),
    (
        "first Annex descriptor controls direct reads before and after lexical initialization",
        "if(false)function maskedDirectRead(){};var before=maskedDirectRead+'|'+typeof maskedDirectRead;let maskedDirectRead=1;before+'|'+maskedDirectRead+'|'+typeof maskedDirectRead",
    ),
    (
        "first Annex descriptor relays through nested reads before and after lexical initialization",
        "if(false)function maskedNestedRead(){};var read=function(){return typeof maskedNestedRead+'|'+maskedNestedRead};var before=read();let maskedNestedRead=1;before+'|'+read()",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "executed Program if Annex write observes a later lexical TDZ",
        "if(true)function laterIfLexical(){};let laterIfLexical;",
    ),
    (
        "Program labelled Annex write observes a later lexical TDZ",
        "label:function laterLabelLexical(){};let laterLabelLexical;",
    ),
    (
        "first Annex global record exposes a later const to the next Annex write",
        "if(false){function maskedConstLexical(){return 0}};const maskedConstLexical=3;{function maskedConstLexical(){return 2}}",
    ),
    (
        "first masked const remains read-only after a duplicate let initializer",
        "if(false)function firstConstControls(){};const firstConstControls=1;let firstConstControls=2;firstConstControls=3",
    ),
    (
        "var initializer still observes a masked Program const",
        "if(false)function constVarInitializer(){};const constVarInitializer=1;var constVarInitializer=2",
    ),
    (
        "label before masked lexical and var still fails at the authored TDZ write",
        "label:function labelLexicalVar(){};let labelLexicalVar=1;var labelLexicalVar;",
    ),
    (
        "first Annex descriptor keeps a later lexical in TDZ for direct writes",
        "if(false)function maskedDirectWrite(){};maskedDirectWrite=2;let maskedDirectWrite=1",
    ),
    (
        "first Annex descriptor relay keeps a later lexical in TDZ for nested writes",
        "if(false)function maskedNestedWrite(){};var write=function(){maskedNestedWrite=2};write();let maskedNestedWrite=1",
    ),
    (
        "first Annex descriptor keeps an initialized const read-only for direct writes",
        "if(false)function maskedDirectConstWrite(){};const maskedDirectConstWrite=1;maskedDirectConstWrite=2",
    ),
    (
        "first Annex descriptor relay keeps an initialized const read-only",
        "if(false)function maskedNestedConstWrite(){};const maskedNestedConstWrite=1;(function(){maskedNestedConstWrite=2})()",
    ),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "if-arm Annex outer closure fault",
        "(function singleOuter(){\n  if(true) function singleInner(){\n    missingSingleAnnex;\n  }\n  return singleInner();\n})()",
    ),
    (
        "label-chain lexical closure fault",
        "(function labelOuter(){\n  first:second:function labelInner(){\n    missingLabelChain;\n  }\n  return labelInner();\n})()",
    ),
    (
        "Function constructor if-arm fault",
        "Function(\"if(true) function dynamicSingle(){\\n  missingDynamicSingle;\\n}\\nreturn dynamicSingle();\")()",
    ),
    (
        "duplicate if Annex-first closure fault",
        "(function duplicateOuter(){\n  if((Function.savedDuplicateIf=duplicateIf,true))\n    function duplicateIf(){ missingFirstIfTarget; }\n  else\n    function duplicateIf(){ missingSecondIfTarget; }\n  return duplicateIf();\n})()",
    ),
    (
        "duplicate if lexical-last closure fault",
        "(function duplicateOuter(){\n  if((Function.savedDuplicateIf=duplicateIf,true))\n    function duplicateIf(){ missingFirstIfTarget; }\n  else\n    function duplicateIf(){ missingSecondIfTarget; }\n  return Function.savedDuplicateIf();\n})()",
    ),
];

const SYNTAX_ERROR_CASES: &[(&str, &str)] = &[
    (
        "strict if-arm function",
        "(function(){'use strict';if(true)function strictIf(){}})",
    ),
    (
        "strict if context error precedes a malformed function header",
        "(function(){'use strict';if(true)function malformed(})",
    ),
    (
        "direct while-body function",
        "(function(){while(false)function whileBody(){}})",
    ),
    (
        "direct do-body function",
        "(function(){do function doBody(){} while(false)})",
    ),
    (
        "direct for-body function",
        "(function(){for(;false;)function forBody(){}})",
    ),
    (
        "if arm cannot forward function permission through a label",
        "(function(){if(true)label:function labelledIf(){}})",
    ),
    (
        "strict labelled function",
        "(function(){'use strict';label:function strictLabel(){}})",
    ),
    (
        "prior Program var blocks a labelled function",
        "var priorVar;label:function priorVar(){}",
    ),
    (
        "prior Program var conflict precedes a malformed labelled child",
        "var priorMalformed;label:function priorMalformed(}",
    ),
    (
        "prior Program function blocks a labelled function",
        "function priorFunction(){}label:function priorFunction(){}",
    ),
    (
        "prior Program lexical blocks a labelled function",
        "let priorLexical;label:function priorLexical(){}",
    ),
    (
        "prior body lexical blocks a labelled function",
        "(function(){let bodyLexical;label:function bodyLexical(){}})",
    ),
    (
        "prior body var blocks a labelled function",
        "(function(){var bodyVar;label:function bodyVar(){}})",
    ),
    (
        "labelled body function blocks a later lexical",
        "(function(){label:function laterBodyLexical(){};let laterBodyLexical})",
    ),
    (
        "duplicate label chain",
        "(function(){same:same:if(true)function duplicateLabel(){}})",
    ),
    (
        "declared function directive validates its if-arm name strictly",
        "(function(){if(true)function eval(){'use strict';}})",
    ),
    (
        "if-arm generator is not an Annex B ordinary function",
        "if(true)function* singleGenerator(){}",
    ),
    (
        "labelled generator is not an Annex B ordinary function",
        "label:function* labelledGenerator(){}",
    ),
    (
        "if-arm async declaration requires the other-declaration mask",
        "if(true)async function singleAsync(){}",
    ),
    (
        "labelled async declaration requires the other-declaration mask",
        "label:async function labelledAsync(){}",
    ),
];

const WITH_BOUNDARY_CASES: &[(&str, &str, bool)] = &[
    (
        "with directly containing a function remains a QuickJS syntax error",
        "(function(){with(Function)function directWith(){}})()",
        false,
    ),
    (
        "with containing an if would reopen Annex B once with is implemented",
        "Function.withNested=7;with(Function)if(true)function withNested(){return 13};withNested()+'|'+Function.withNested",
        true,
    ),
];

#[test]
fn annex_b_statement_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Annex B statement differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle_sequence(&oracle, &[source], description),
            "Annex B statement value drifted for {description}: {source:?}",
        );
    }
}

#[test]
fn annex_b_statement_errors_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Annex B statement error differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in ERROR_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle_sequence(&oracle, &[source], description),
            "Annex B statement error drifted for {description}: {source:?}",
        );
    }
}

#[test]
fn program_label_annex_global_state_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program label global-state differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let description = "Program labelled Annex precedes a later global lexical";
    let sources = [
        "label:function orderedLabelCollision(){};let orderedLabelCollision;",
        "typeof globalThis.orderedLabelCollision",
        "orderedLabelCollision",
    ];
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let rust = sources
        .iter()
        .map(|source| observe_rust_eval(&runtime, &mut context, source, description))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        rust,
        observe_oracle_sequence(&oracle, &sources, description)
    );
}

#[test]
fn program_label_invokes_existing_global_setter_twice() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program label accessor differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let description = "Program labelled function performs both QuickJS global writes";
    let sources = [
        "Function.labelSetterHits=0;Function.labelSetterValue=0;Object.defineProperty(globalThis,'__qjo_label_accessor',{configurable:true,get:function(){return Function.labelSetterValue},set:function(value){Function.labelSetterHits++;Function.labelSetterValue=value}});0",
        "label:function __qjo_label_accessor(){return 17};Function.labelSetterHits+'|'+typeof __qjo_label_accessor+'|'+__qjo_label_accessor()",
    ];
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval("Function.labelSetterHits=0;Function.labelSetterValue=0")
        .unwrap();
    let Value::Object(getter) = context
        .eval("(function(){return Function.labelSetterValue})")
        .unwrap()
    else {
        panic!("Program label accessor getter was not a function");
    };
    let Value::Object(setter) = context
        .eval("(function(value){Function.labelSetterHits++;Function.labelSetterValue=value})")
        .unwrap()
    else {
        panic!("Program label accessor setter was not a function");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    let setter = runtime.as_callable(&setter).unwrap().unwrap();
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("__qjo_label_accessor").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let rust = format!(
        "return|number|0\n{}",
        observe_rust_eval(&runtime, &mut context, sources[1], description)
    );
    assert_eq!(
        rust,
        observe_oracle_sequence(&oracle, &sources, description)
    );
}

#[test]
fn annex_b_statement_full_and_strip_debug_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Annex B statement stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in STACK_CASES {
        compare_cli(&oracle, &[], source, description);
        compare_cli(&oracle, &["-s"], source, description);
    }
}

#[test]
fn annex_b_statement_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Annex B statement parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in SYNTAX_ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn unsupported_with_annex_boundaries_remain_explicit() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP with/Annex B boundaries: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source, oracle_accepts) in WITH_BOUNDARY_CASES {
        let quickjs = observe_oracle_sequence(&oracle, &[source], description);
        assert_eq!(
            quickjs.starts_with("return|"),
            oracle_accepts,
            "pinned QuickJS changed its with/Annex B boundary: {quickjs}"
        );
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let rust = observe_rust_eval(&runtime, &mut context, source, description);
        assert!(
            rust.starts_with("throw|object|SyntaxError|with statements are not implemented yet"),
            "with boundary was not rejected explicitly: {rust}"
        );
    }
}

#[test]
fn program_label_cross_realm_regression() {
    // Pinned with the QuickJS C API compile-in-A/execute-in-B path. The Rust
    // API exposes the same operation directly, so this remains unconditional.
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    defining.eval("globalThis.realmTag='A'").unwrap();
    caller.eval("globalThis.realmTag='B'").unwrap();

    let bytecode = defining
        .compile(
            "label:function crossRealmLabel(){return realmTag+'|'+(this===globalThis)};\
             crossRealmLabel",
        )
        .unwrap();
    let Value::Object(function) = caller.execute(&bytecode).unwrap() else {
        panic!("cross-realm Program label did not return its function");
    };
    let Value::Object(prototype_a) = defining.eval("Function.prototype").unwrap() else {
        panic!("defining Function.prototype was not an object");
    };
    let Value::Object(prototype_b) = caller.eval("Function.prototype").unwrap() else {
        panic!("caller Function.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&function).unwrap(),
        Some(prototype_a)
    );
    assert_ne!(
        runtime.get_prototype_of(&function).unwrap(),
        Some(prototype_b)
    );
    let callable = runtime.as_callable(&function).unwrap().unwrap();
    assert_eq!(
        caller.call(&callable, Value::Undefined, &[]).unwrap(),
        Value::String(JsString::try_from_utf8("B|false").unwrap())
    );
}

fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_value_text(value)
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    error_string_property(runtime, context, &error, "name", description),
                    error_string_property(runtime, context, &error, "message", description),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    primitive_value_text(value)
                ),
            }
        }
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}

fn observe_oracle_sequence(oracle: &OsStr, sources: &[&str], description: &str) -> String {
    let wrapper = r#"
(function () {
for (var index = 0; index < scriptArgs.length; index++) {
  try {
    var value = std.evalScript(scriptArgs[index]);
    print('return|' + typeof value + '|' + String(value));
  } catch (error) {
    if (error !== null && typeof error === 'object')
      print('throw|object|' + error.name + '|' + error.message);
    else
      print('throw|' + typeof error + '|' + String(error));
  }
}
})();
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper])
        .args(sources)
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS sequence failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"));
    stdout.strip_suffix('\n').unwrap_or(&stdout).to_owned()
}

fn compare_cli(oracle: &OsStr, options: &[&str], source: &str, description: &str) {
    let rust = run_cli(
        env!("CARGO_BIN_EXE_qjs").as_ref(),
        options,
        source,
        description,
    );
    let quickjs = run_cli(oracle, options, source, description);
    assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
    assert_eq!(rust.stdout, quickjs.stdout, "{description}");
    assert_eq!(rust.stderr, quickjs.stderr, "{description}");
}

fn run_cli(program: &OsStr, options: &[&str], source: &str, description: &str) -> Output {
    Command::new(program)
        .args(options)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}

fn error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &quickjs_oxide::ObjectRef,
    name: &str,
    description: &str,
) -> String {
    let key = runtime
        .intern_property_key(name)
        .expect("Error property key");
    let Value::String(value) = context
        .get_property(error, &key)
        .unwrap_or_else(|failure| panic!("read Error.{name} for {description}: {failure}"))
    else {
        panic!("Error.{name} was not a string for {description}");
    };
    value.to_utf8_lossy()
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
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "<object>".to_owned(),
        Value::Symbol(_) => "<symbol>".to_owned(),
    }
}
