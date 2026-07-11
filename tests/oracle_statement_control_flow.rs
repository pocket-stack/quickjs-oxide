use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const ORACLE_NORMALIZER: &str = r#"
var __qjo_type = typeof __qjo_value;
if (__qjo_type === "number") {
    if (__qjo_value !== __qjo_value) {
        print("number|NaN");
    } else if (__qjo_value === 0 && 1 / __qjo_value === -Infinity) {
        print("number|-0");
    } else if (__qjo_value === Infinity) {
        print("number|Infinity");
    } else if (__qjo_value === -Infinity) {
        print("number|-Infinity");
    } else {
        print("number|" + String(__qjo_value));
    }
} else if (__qjo_type === "string") {
    var __qjo_units = "";
    for (var __qjo_index = 0; __qjo_index < __qjo_value.length; __qjo_index++) {
        var __qjo_hex = __qjo_value.charCodeAt(__qjo_index).toString(16);
        if (__qjo_index !== 0) __qjo_units += ",";
        __qjo_units += ("0000" + __qjo_hex).slice(-4);
    }
    print("string|" + __qjo_value.length + "|" + __qjo_units);
} else if (__qjo_value === null) {
    print("object|null");
} else {
    print(__qjo_type + "|" + String(__qjo_value));
}
"#;

const VALUE_CASES: &[(&str, &str)] = &[
    ("empty script", ""),
    ("empty statement", ";"),
    ("empty statement preserves completion", "1; ;"),
    ("empty block preserves completion", "1; {}"),
    ("empty nested block preserves completion", "1; {{;}}"),
    ("block expression updates completion", "1; { 2; }"),
    ("nested block keeps its last expression", "{ 1; { 2; {} } }"),
    ("deep empty blocks preserve completion", "7; {{{}}}"),
    ("false if resets completion", "1; if (false) 2;"),
    ("taken empty if resets completion", "1; if (true) {}"),
    ("taken block updates completion", "1; if (true) { 2; }"),
    ("false branch selects else", "if (false) 1; else 2;"),
    ("true branch skips else", "if (true) 1; else 2;"),
    (
        "dangling else binds to nearest if",
        "if (true) if (false) 1; else 2;",
    ),
    (
        "outer else follows completed nested if",
        "if (false) if (true) 1; else 2; else 3;",
    ),
    ("taken empty statement", "if (true) ; else 2;"),
    ("empty else statement", "if (false) 1; else ;"),
    ("comma condition", "if ((0, 1)) 8; else 9;"),
    (
        "nested if resets a previous branch completion",
        "if (true) { 1; if (false) 2; }",
    ),
    (
        "function if return false",
        "(function(x){ if (x) return 1; else return 2; })(0)",
    ),
    (
        "function if return true",
        "(function(x){ if (x) return 1; else return 2; })(1)",
    ),
    (
        "nested branch return",
        "(function(){ if (true) { if (true) { return 42; } return 1; } return 0; })()",
    ),
    (
        "dead branch var is function scoped",
        "(function(){ if (false) { var x = 1; } return typeof x; })()",
    ),
    (
        "block var remains function scoped",
        "(function(){ { var x = 4; } return x; })()",
    ),
    (
        "selected var initializer",
        "(function(flag){ if (flag) { var x = 3; } return typeof x + '|' + x; })(true)",
    ),
    (
        "dead throw is not executed",
        "(function(){ if (false) throw 1; return 42; })()",
    ),
    (
        "return restricted production in a branch",
        "(function(){ if (true) return\n42; return 7; })()",
    ),
    (
        "function expression statements stay discarded",
        "(function(flag){ if (flag) 1; else 2; })()",
    ),
    (
        "condition and selected branch order",
        "(function(){ var log=''; var c=function(){log=log+'c';return true;}; var y=function(){log=log+'y';}; var n=function(){log=log+'n';}; if(c()) y(); else n(); return log; })()",
    ),
    (
        "condition and false branch order",
        "(function(){ var log=''; var c=function(){log=log+'c';return false;}; var y=function(){log=log+'y';}; var n=function(){log=log+'n';}; if(c()) y(); else n(); return log; })()",
    ),
    (
        "unselected branch has no effects",
        "(function(){ var x=0; if (true) x=1; else missing(); return x; })()",
    ),
    (
        "object condition does not coerce",
        "(function(){ var log=''; var o=function(){}; o.valueOf=function(){log=log+'v';return 0;}; if(o) log=log+'t'; return log; })()",
    ),
    (
        "nested condition side effect order",
        "(function(){ var log=''; if((log=log+'a',true)) if((log=log+'b',false)) log=log+'c'; else log=log+'d'; return log; })()",
    ),
    (
        "block string is not a directive",
        "{ 'use strict'; '\\1'; }",
    ),
    (
        "block before string prevents directive prologue",
        "{}; 'use strict'; '\\1';",
    ),
    (
        "function block string is not a directive",
        "(function(){ { 'use strict'; } return '\\1'.charCodeAt(0); })()",
    ),
    (
        "block strict spelling leaves legacy octal sloppy",
        "{ 'use strict'; 010; }",
    ),
    (
        "if strict spelling leaves legacy octal sloppy",
        "if (true) 'use strict'; 010;",
    ),
    ("LF participates in if ASI", "if (false)\n1;\nelse\n2;"),
    ("CR participates in if ASI", "if (false)\r1;\relse\r2;"),
    (
        "CRLF participates in if ASI",
        "if (false)\r\n1;\r\nelse\r\n2;",
    ),
    (
        "line separator participates in if ASI",
        "if (false)\u{2028}1;\u{2028}else\u{2028}2;",
    ),
    (
        "paragraph separator participates in if ASI",
        "if (false)\u{2029}1;\u{2029}else\u{2029}2;",
    ),
    ("false while resets completion", "1; while (false) 2;"),
    ("empty false while resets completion", "1; while (false);"),
    (
        "break preserves body completion",
        "while (true) { 3; break; }",
    ),
    (
        "nested while resets an earlier body completion",
        "while(true){ 2; while(false) 3; break; }",
    ),
    (
        "nested do preserves its final body completion",
        "while(true){ do 2; while(false); break; }",
    ),
    (
        "while condition does not become the completion",
        "1; while((5,false));",
    ),
    (
        "do condition does not replace its body completion",
        "do 2; while((3,false))",
    ),
    (
        "while iteration and continue order",
        "(function(){ var i=0; var sum=0; while(i<5){ i++; if(i===3) continue; sum+=i; } return sum; })()",
    ),
    (
        "while continue preserves the current body completion",
        "Function.loopIndex=0; while(Function.loopIndex++<2){ 9; continue; }",
    ),
    (
        "nested loop jumps select the nearest loop",
        "(function(){ var i=0; var j=0; while(i<3){ i++; while(true){ j++; break; } continue; j=99; } return i+'|'+j; })()",
    ),
    ("do body executes once", "do 2; while(false)"),
    (
        "empty do resets an earlier completion",
        "1; do; while(false)",
    ),
    (
        "do break leaves its reset completion",
        "1; do break; while(false)",
    ),
    (
        "do continue preserves its body completion",
        "do { 9; continue; } while(false)",
    ),
    (
        "do break skips its condition",
        "(function(){ var x=0; do { break; } while(x++); return x; })()",
    ),
    (
        "do continue reaches its condition",
        "(function(){ var x=0; do { continue; } while(x++<1); return x; })()",
    ),
    (
        "do resets completion for every iteration",
        "Function.loopIndex=0; do { if(Function.loopIndex++===0) 7; } while(Function.loopIndex<2)",
    ),
    (
        "do trailing semicolon is optional before same-line source",
        "do {} while(false) 9",
    ),
    ("do body uses line-terminator ASI", "do 1\nwhile(false)"),
    (
        "break line terminator starts unreachable source",
        "while(true){ break\nmissingAfterBreak; }",
    ),
    (
        "break CR starts unreachable source",
        "while(true){ break\rmissingAfterBreak; }",
    ),
    (
        "break CRLF starts unreachable source",
        "while(true){ break\r\nmissingAfterBreak; }",
    ),
    (
        "break line separator starts unreachable source",
        "while(true){ break\u{2028}missingAfterBreak; }",
    ),
    (
        "break paragraph separator starts unreachable source",
        "while(true){ break\u{2029}missingAfterBreak; }",
    ),
    (
        "continue block-comment line terminator starts unreachable source",
        "do { continue/*\n*/missingAfterContinue; } while(false)",
    ),
    (
        "continue CRLF comment starts unreachable source",
        "do { continue/*\r\n*/missingAfterContinue; } while(false)",
    ),
    (
        "Function constructor parses while jumps",
        "Function('var i=0; while(i<3)i++; return i')()",
    ),
    (
        "Function constructor parses do return",
        "Function('do { return 4; } while(false)')()",
    ),
    ("false for resets completion", "1; for(;false;) 2;"),
    (
        "for break preserves body completion",
        "for(;;){ 3; break; }",
    ),
    (
        "for continue runs update",
        "(function(){ var sum=0; for(var i=0;i<5;i++){ if(i===2) continue; sum+=i; } return sum; })()",
    ),
    (
        "for var declarator list remains function scoped",
        "(function(){ for(var i=0,j=1;i<2;i++,j+=2); return i+'|'+j; })()",
    ),
    (
        "for var initializer keeps named evaluation",
        "(function(){ for(var named=function(){};false;); return named.name; })()",
    ),
    (
        "for break skips update",
        "(function(){ var i=0; for(;;i++){ break; } return i; })()",
    ),
    (
        "for without test reaches update after its body",
        "(function(){ var i=0; for(;;i++){ if(i===3) break; } return i; })()",
    ),
    (
        "for without update returns to test",
        "(function(){ var i=0; for(;i<3;){ i++; } return i; })()",
    ),
    (
        "for comma clauses preserve order",
        "(function(){ var i=0; var x=0; for(i=0,x=1;i<3;i++,x+=2){} return i+'|'+x; })()",
    ),
    (
        "relocated for update keeps internal conditional targets",
        "(function(){ var i=0; var x=0; for(;i<3;i++,x+=i===1?10:1){} return x; })()",
    ),
    (
        "for continue preserves the current body completion",
        "Function.loopIndex=0; for(;Function.loopIndex++<2;){ 9; continue; }",
    ),
    (
        "for init test and update do not become completion",
        "for(Function.loopIndex=0;Function.loopIndex<1;Function.loopIndex++) 5",
    ),
    (
        "nested for jumps select the nearest loop",
        "(function(){ var i=0; var j=0; for(;i<3;i++){ for(;;){ j++; break; } continue; j=99; } return i+'|'+j; })()",
    ),
    (
        "Function constructor parses classic for",
        "Function('var sum=0; for(var i=0;i<4;i++)sum+=i; return sum')()",
    ),
    (
        "for header line terminators do not insert semicolons",
        "Function.loopIndex=0; for(Function.loopIndex=0\n;Function.loopIndex<2\n;Function.loopIndex++){} Function.loopIndex",
    ),
    (
        "contextual of remains an identifier in classic for",
        "(function(){ var of=9; for(of=0;of<2;of++); return of; })()",
    ),
    (
        "sloppy let assignment remains an expression in classic for",
        "(function(){ var let=9; for(let=0;let<2;let++); return let; })()",
    ),
    (
        "escaped let member remains an expression in classic for",
        "(function(){ var let=Function; for(l\\u0065t.loop=0;l\\u0065t.loop<2;l\\u0065t.loop++); return let.loop; })()",
    ),
    (
        "global sets in for clauses preserve remapped targets",
        "for(loopGlobal=0;loopGlobal<3;loopGlobal++); loopGlobal",
    ),
    (
        "for clauses execute in init test body update order",
        "(function(){ var log=''; var i=0; for(log+='i';(log+='t',i<2);(log+='u',i++)){ log+='b'; } return log; })()",
    ),
    (
        "return from for body skips update",
        "Function.returnUpdate=0; (function(){ for(;;Function.returnUpdate=1) return 7; })(); Function.returnUpdate",
    ),
    (
        "for update does not become completion",
        "1; Function.loopIndex=0; for(;Function.loopIndex++<1;9);",
    ),
    (
        "nested for resets an earlier body completion",
        "for(;;){ 2; for(;false;) 3; break; }",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "missing condition left parenthesis wins over later lex error",
        "if true 1; \"unterminated",
    ),
    (
        "missing condition right parenthesis wins over later lex error",
        "if (true 0; \"unterminated",
    ),
    ("missing consequent", "if (true)"),
    ("else cannot be a consequent", "if (true) else 1;"),
    ("orphan else", "else 1;"),
    ("unterminated then block", "if (true) {"),
    ("unterminated nonempty block", "if (true) { 1;"),
    ("stray right brace", "if (true) {} }"),
    ("stray right brace after root expression", "1 }"),
    ("stray right brace after if expression", "if (true) 1 }"),
    (
        "stray right brace after nested if expression",
        "if (false) if (true) 1 }",
    ),
    ("missing else branch", "if (true) {} else"),
    (
        "return remains an early error in a dead branch",
        "if (false) return 1;",
    ),
    (
        "escaped reserved binding remains an early error in a dead branch",
        "(function(){ if (false) { var \\u0069f=1; } })()",
    ),
    (
        "raw malformed escape wins before later lex error",
        "if (true) \\u{}; \"unterminated",
    ),
    (
        "dead return wins before a later lex error",
        "if (false) { return 1; \"unterminated",
    ),
    (
        "dead branch still scans its reached lexical error",
        "if (false) { \"unterminated",
    ),
    (
        "strict program directive reaches a block legacy escape",
        "\"use strict\"; { \"\\1\"; }",
    ),
    (
        "strict function directive reaches a block legacy escape",
        "(function(){ \"use strict\"; { return \"\\1\"; } })()",
    ),
    (
        "LF throw restriction reports the following token",
        "if (true) throw\n1;",
    ),
    (
        "CR throw restriction uses QuickJS debug coordinates",
        "if (true) throw\r1;",
    ),
    (
        "line separator throw restriction uses QuickJS debug coordinates",
        "if (true) throw\u{2028}1;",
    ),
    ("break outside a loop", "break"),
    ("continue outside a loop", "continue"),
    (
        "outer loop is not visible inside a nested function",
        "while(false) (function(){ break; })",
    ),
    (
        "outer do loop is not visible inside a nested function",
        "do (function(){ continue; }); while(false)",
    ),
    ("do requires while", "do {}"),
    (
        "do preserves QuickJS non-ASCII expect diagnostic",
        "do{} false",
    ),
    ("while requires a left parenthesis", "while true;"),
    ("while requires a right parenthesis", "while (true;"),
    ("while requires a body", "while (true)"),
    ("do expression body still requires ASI", "do 1 while(false)"),
    (
        "do condition requires a left parenthesis",
        "do{}while false",
    ),
    (
        "unimplemented break label is not silently discarded",
        "while(true){ break missing; }",
    ),
    (
        "unimplemented continue label is not silently discarded",
        "while(true){ continue missing; }",
    ),
    (
        "return remains an early error in a dead loop body",
        "while(false) return 1;",
    ),
    (
        "break scans the following lexical error first",
        "break \"unterminated",
    ),
    (
        "in-loop break scans the following lexical error first",
        "while(false){ break \"unterminated",
    ),
    (
        "outside break LF uses restricted-production ASI",
        "break\nmissingLabel",
    ),
    (
        "outside break CR keeps QuickJS debug coordinates",
        "break\rmissingLabel",
    ),
    ("outside break CRLF advances at LF", "break\r\nmissingLabel"),
    (
        "outside break line separator keeps QuickJS debug coordinates",
        "break\u{2028}missingLabel",
    ),
    (
        "outside break paragraph separator keeps QuickJS debug coordinates",
        "break\u{2029}missingLabel",
    ),
    (
        "outside continue comment terminator uses restricted-production ASI",
        "continue/*\r\n*/missingLabel",
    ),
    ("for requires a left parenthesis", "for true;"),
    ("for initializer requires its semicolon", "for(1 2;;);"),
    ("for test requires its semicolon", "for(;true 2;);"),
    ("for requires a right parenthesis", "for(;;"),
    ("for requires a body", "for(;;)"),
    (
        "outer for is not visible inside a nested function",
        "for(;false;) (function(){ break; })",
    ),
    (
        "outer for continue is not visible inside a nested function",
        "for(;false;) (function(){ continue; })",
    ),
    (
        "return remains an early error in a dead for body",
        "for(;false;) return 1;",
    ),
    (
        "for body continue label is not silently discarded",
        "for(;;){ continue missing; }",
    ),
    (
        "top-level in with later semicolons stays classic for",
        "for(Function.item in Function;;);",
    ),
    (
        "top-level of with later semicolons stays classic for",
        "for(Function.item of Function;;);",
    ),
    (
        "for pre-scan does not let a later lexical error win",
        "for(true token; ; \"unterminated",
    ),
];

#[test]
fn statement_control_flow_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP statement-control-flow differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let value = context
            .eval(source)
            .unwrap_or_else(|error| panic!("Rust rejected {description:?} ({source:?}): {error}"));
        assert_eq!(
            normalize_rust_value(&value),
            oracle_value_observation(&oracle, source, description),
            "value mismatch for {description:?} ({source:?})"
        );
    }
}

#[test]
fn statement_control_flow_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP statement diagnostic differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in ERROR_CASES {
        assert_eq!(
            rust_error_observation(source),
            oracle_error_observation(&oracle, source),
            "diagnostic mismatch for {description:?} ({source:?})"
        );
    }
}

fn oracle_value_observation(oracle: &OsStr, source: &str, description: &str) -> String {
    let script = format!("var __qjo_value = std.evalScript(scriptArgs[0]);\n{ORACLE_NORMALIZER}");
    let output = Command::new(oracle)
        .args(["--std", "-e", &script, source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description:?}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS rejected {description:?} ({source:?}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS value output was not UTF-8")
        .trim_end()
        .to_owned()
}

fn rust_error_observation(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval(source), Err(RuntimeError::Exception));
    take_rust_error(&runtime, &mut context)
}

fn take_rust_error(runtime: &Runtime, context: &mut Context) -> String {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Rust parser did not materialize an Error object");
    };
    let read = |context: &mut Context, name: &str| {
        let key = runtime.intern_property_key(name).unwrap();
        context.get_property(&error, &key).unwrap()
    };
    let Value::String(name) = read(context, "name") else {
        panic!("Rust Error.name was not a string");
    };
    let Value::String(message) = read(context, "message") else {
        panic!("Rust Error.message was not a string");
    };
    let Value::Int(line) = read(context, "lineNumber") else {
        panic!("Rust Error.lineNumber was not an integer");
    };
    let Value::Int(column) = read(context, "columnNumber") else {
        panic!("Rust Error.columnNumber was not an integer");
    };
    format!(
        "{}|{}|{line}:{column}",
        name.to_utf8_lossy(),
        message.to_utf8_lossy()
    )
}

fn oracle_error_observation(oracle: &OsStr, source: &str) -> String {
    let output = Command::new(oracle)
        .args(["--std", "-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {source:?}: {error}"));
    assert!(!output.status.success(), "QuickJS accepted {source:?}");
    let stderr = String::from_utf8(output.stderr).expect("QuickJS error output was not UTF-8");
    let mut lines = stderr.lines();
    let first = lines
        .find(|line| line.starts_with("SyntaxError: "))
        .unwrap_or_else(|| panic!("QuickJS emitted no SyntaxError for {source:?}: {stderr}"));
    let location = lines
        .find_map(|line| line.trim().strip_prefix("at <cmdline>:"))
        .unwrap_or_else(|| panic!("QuickJS emitted no location for {source:?}: {stderr}"));
    format!(
        "SyntaxError|{}|{location}",
        first.strip_prefix("SyntaxError: ").unwrap()
    )
}

fn normalize_rust_value(value: &Value) -> String {
    match value {
        Value::Undefined => "undefined|undefined".to_owned(),
        Value::Null => "object|null".to_owned(),
        Value::Bool(value) => format!("boolean|{value}"),
        Value::Int(value) => normalize_number(f64::from(*value)),
        Value::Float(value) => normalize_number(*value),
        Value::BigInt(value) => format!("bigint|{value}"),
        Value::String(value) => {
            let units = value
                .utf16_units()
                .map(|unit| format!("{unit:04x}"))
                .collect::<Vec<_>>()
                .join(",");
            format!("string|{}|{units}", value.len())
        }
        Value::Symbol(_) => "symbol|<identity>".to_owned(),
        Value::Object(_) => "object|<identity>".to_owned(),
    }
}

#[allow(clippy::float_cmp)]
fn normalize_number(value: f64) -> String {
    if value.is_nan() {
        "number|NaN".to_owned()
    } else if value == 0.0 && value.is_sign_negative() {
        "number|-0".to_owned()
    } else if value == f64::INFINITY {
        "number|Infinity".to_owned()
    } else if value == f64::NEG_INFINITY {
        "number|-Infinity".to_owned()
    } else {
        format!("number|{}", number_to_string(value))
    }
}
