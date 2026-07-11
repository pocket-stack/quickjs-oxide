use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, RuntimeError, Value};

#[test]
fn implemented_error_backtraces_match_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Error stack oracle differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = [
        (
            "nested VM fault and tail-call sites",
            "(function outer(){ return (function inner(){ return 1n + 1; })(); })()",
        ),
        (
            "eager Error constructor stack",
            "throw (function outer(){ return (function inner(){ return new Error(\"boom\"); })(); })()",
        ),
        ("parse location", "1 +"),
        (
            "strict missing assignment inherits the LHS site",
            "\"use strict\"; missing = 1",
        ),
        (
            "assignment inherits an RHS identifier marker",
            "\"use strict\"; missing = Error",
        ),
        (
            "assignment inherits an RHS operator marker",
            "\"use strict\"; missing = 1 + 2",
        ),
        (
            "assignment inherits a private self-binding marker",
            "(function f(){ \"use strict\"; missing = f; })()",
        ),
        ("fixed member null fault", "null.x"),
        ("computed member undefined fault", "(void 0)['x']"),
        (
            "non-tail method call site",
            "(function(){ var value = Function.name(); return value; })()",
        ),
        (
            "tail method call site",
            "(function(){ return Function.name(); })()",
        ),
        (
            "template concat coercion inherits the last substitution site",
            "Function.x=function(){}; Function.x.toString=function(){throw new Error(\"x\")};\n`a${Function.x}b`",
        ),
        ("fixed member assignment fault", "null.x = 1"),
        ("computed member assignment fault", "null['x'] = 1"),
        ("fixed member delete fault", "delete null.x"),
        ("computed member delete fault", "delete (void 0)['x']"),
        (
            "strict fixed member assignment rejection",
            "\"use strict\";\nFunction.prototype = 1",
        ),
        (
            "strict fixed member delete rejection",
            "\"use strict\";\ndelete Function.prototype",
        ),
        (
            "native member setter fault",
            "Function.prototype.caller = 1",
        ),
        (
            "strict fixed compound assignment rejection",
            "\"use strict\";\nFunction.prototype += 1",
        ),
        (
            "strict computed compound assignment rejection",
            "\"use strict\";\nFunction['prototype'] += 1",
        ),
        (
            "binary bitwise mixed type uses operator site",
            "(function(){ return 1n & 1; })()",
        ),
        (
            "binary shift mixed type uses operator site",
            "(function(){ return 1n << 1; })()",
        ),
        (
            "unsigned shift BigInt rejection uses operator site",
            "(function(){ return 1n >>> 0n; })()",
        ),
        (
            "oversized BigInt shift uses operator site",
            "(function(){ return 1n << 1048576n; })()",
        ),
        (
            "binary exponent mixed type uses operator site",
            "(function(){ return 1n ** 1; })()",
        ),
        (
            "negative BigInt exponent uses operator site",
            "(function(){ return 2n ** -1n; })()",
        ),
        (
            "oversized BigInt exponent uses operator site",
            "(function(){ return 2n ** 1048575n; })()",
        ),
        (
            "strict fixed bitwise assignment rejection uses operator site",
            "\"use strict\";\nFunction.prototype &= 1",
        ),
        (
            "strict computed bitwise assignment rejection uses operator site",
            "\"use strict\";\nFunction['prototype'] ^= 1",
        ),
        (
            "strict fixed shift assignment rejection uses operator site",
            "\"use strict\";\nFunction.prototype <<= 1",
        ),
        (
            "strict computed shift assignment rejection uses operator site",
            "\"use strict\";\nFunction['prototype'] >>>= 1",
        ),
        (
            "strict fixed exponent assignment rejection uses operator site",
            "\"use strict\";\nFunction.prototype **= 1",
        ),
        (
            "strict computed exponent assignment rejection uses operator site",
            "\"use strict\";\nFunction['prototype'] **= 1",
        ),
        (
            "bitwise compound getter fault keeps member site",
            "Function.prototype.caller |= 1",
        ),
        (
            "shift compound getter fault keeps member site",
            "Function.prototype.caller >>= 1",
        ),
        (
            "exponent compound getter fault keeps member site",
            "Function.prototype.caller **= 1",
        ),
        (
            "bitwise compound mixed type uses operator site",
            "Function.__qjo_bit_error = 1n; Function.__qjo_bit_error &= 1",
        ),
        ("compound nullish pre-key fault", "null[true] += 1"),
        (
            "compound getter fault keeps member site",
            "Function.prototype.caller += 1",
        ),
        (
            "strict fixed logical assignment rejection",
            "\"use strict\";\nFunction.prototype &&= 1",
        ),
        (
            "strict computed logical assignment rejection",
            "\"use strict\";\nFunction['prototype'] &&= 1",
        ),
        (
            "logical assignment put inherits RHS call site",
            "\"use strict\";\nFunction.prototype &&= (function(){ return 1; })()",
        ),
        ("logical nullish pre-key fault", "null[true] ||= 1"),
        (
            "logical getter fault keeps member site",
            "Function.prototype.caller &&= 1",
        ),
        (
            "nullish RHS getter keeps member site",
            "null ?? Function.prototype.caller",
        ),
        (
            "nullish RHS call throw uses call site",
            "null ?? (function(){ throw new Error('boom'); })()",
        ),
        (
            "nullish RHS identifier keeps identifier site",
            "void 0 ?? missingNullishRhs",
        ),
        ("logical before nullish parse location", "1 || 2 ?? 3"),
        ("nullish before logical parse location", "1 ?? 2 || 3"),
        (
            "identifier compound missing read keeps identifier site",
            "missingIdentifierCompound += 1",
        ),
        (
            "identifier bitwise compound missing read keeps identifier site",
            "missingIdentifierBitwise &= 1",
        ),
        (
            "identifier shift compound missing read keeps identifier site",
            "missingIdentifierShift >>>= 1",
        ),
        (
            "identifier exponent compound missing read keeps identifier site",
            "missingIdentifierPower **= 1",
        ),
        (
            "strict private identifier arithmetic write uses operator site",
            "(function named(){ 'use strict'; named += 1; })()",
        ),
        (
            "strict private identifier bitwise write uses operator site",
            "(function named(){ 'use strict'; named &= 1; })()",
        ),
        (
            "strict private identifier shift write uses operator site",
            "(function named(){ 'use strict'; named <<= 1; })()",
        ),
        (
            "strict private identifier exponent write uses operator site",
            "(function named(){ 'use strict'; named **= 1; })()",
        ),
        (
            "strict private identifier logical write uses identifier site",
            "(function named(){ 'use strict'; named &&= 1; })()",
        ),
        (
            "identifier logical write inherits RHS call site",
            "(function named(){ 'use strict'; named &&= (function(){ return 1; })(); })()",
        ),
        (
            "strict eval compound early error uses RHS site",
            "(function(){ 'use strict'; eval += 1; })",
        ),
        (
            "strict parenthesized arguments compound early error uses RHS site",
            "(function(){ 'use strict'; (arguments) ??= 1; })",
        ),
        (
            "strict eval exponent compound early error uses RHS site",
            "(function(){ 'use strict'; eval **= 1; })",
        ),
        (
            "postfix update missing read keeps identifier site",
            "missingIdentifierUpdate++",
        ),
        (
            "prefix update missing read keeps identifier site",
            "++missingIdentifierUpdate",
        ),
        (
            "strict fixed postfix update rejection uses operator site",
            "\"use strict\";\nFunction.prototype++",
        ),
        (
            "strict computed prefix update rejection uses operator site",
            "\"use strict\";\n++Function['prototype']",
        ),
        (
            "postfix update getter fault keeps member site",
            "Function.prototype.caller++",
        ),
        ("postfix update nullish pre-key fault", "null[true]++"),
        (
            "strict private postfix update write",
            "(function named(){ 'use strict'; named++; })()",
        ),
        (
            "strict private prefix update write",
            "(function named(){ 'use strict'; ++named; })()",
        ),
        (
            "return marker supersedes strict private postfix update",
            "(function named(){ 'use strict'; return named++; })()",
        ),
        (
            "return marker supersedes strict private prefix update",
            "(function named(){ 'use strict'; return ++named; })()",
        ),
        (
            "postfix BigInt update allocation error uses operator site",
            "(function(){ var value = 1n << 1048575n; return value++; })()",
        ),
        (
            "CR remains a debug column",
            "(function f(){\rreturn 1n + 1;\r})()",
        ),
        (
            "CRLF advances only at LF",
            "(function f(){\r\nreturn 1n + 1;\r\n})()",
        ),
        (
            "line separator remains a debug column",
            "(function f(){\u{2028}return 1n + 1;\u{2028}})()",
        ),
        (
            "UTF-8 columns count lead bytes",
            "(function f(){ \"é🙂中\"; return 1n + 1; })()",
        ),
        (
            "while condition fault keeps its expression site",
            "while(\nnull.x) {}",
        ),
        (
            "while body fault keeps its expression site",
            "while(true){\nnull.x;}",
        ),
        (
            "do body fault keeps its expression site",
            "do {\nnull.x;} while(false)",
        ),
        (
            "do continue condition fault keeps its expression site",
            "do { continue; } while(\nnull.x)",
        ),
        (
            "for initializer fault keeps its expression site",
            "for(\nnull.x;;);",
        ),
        (
            "for test fault keeps its expression site",
            "for(;\nnull.x;);",
        ),
        (
            "for update fault keeps its expression site",
            "for(;;\nnull.x);",
        ),
        (
            "for body fault keeps its expression site",
            "for(;true;){\nnull.x;}",
        ),
        (
            "regular labeled body fault keeps its expression site",
            "outer: {\nnull.x;}",
        ),
        (
            "labeled while continue returns to the condition fault site",
            "Function.i=0; outer: while(Function.i++===0 ? true :\nnull.x){ continue outer; }",
        ),
        (
            "labeled do continue reaches its condition fault site",
            "outer: do { continue outer; } while(\nnull.x)",
        ),
        (
            "labeled for continue reaches the relocated update fault site",
            "Function.i=0; outer: for(;Function.i++<1;\nnull.x){ continue outer; }",
        ),
    ];

    for (description, source) in cases {
        assert_eq!(
            rust_stack(source),
            oracle_stack(&oracle, source, description),
            "QuickJS Error stack drifted for {description}: {source:?}",
        );
    }

    assert_eq!(
        rust_stack_with_symbol("~__qjo_bitwise_symbol"),
        oracle_stack(
            &oracle,
            "~Symbol()",
            "unary bitwise coercion uses operator site"
        ),
        "QuickJS unary bitwise Error stack drifted",
    );
}

fn rust_stack(source: &str) -> String {
    rust_stack_with_optional_symbol(source, false)
}

fn rust_stack_with_symbol(source: &str) -> String {
    rust_stack_with_optional_symbol(source, true)
}

fn rust_stack_with_optional_symbol(source: &str, bind_symbol: bool) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    if bind_symbol {
        let global = context.global_object().expect("Rust global object");
        let key = runtime
            .intern_property_key("__qjo_bitwise_symbol")
            .expect("Rust symbol binding key");
        let symbol = runtime.new_symbol(None).expect("Rust symbol value");
        assert!(
            context
                .set_property(&global, &key, Value::Symbol(symbol))
                .expect("bind Rust symbol")
        );
    }
    assert_eq!(
        context.eval_with_filename(source, "<cmdline>"),
        Err(RuntimeError::Exception),
        "Rust source unexpectedly completed: {source:?}",
    );
    let Value::Object(error) = context
        .take_exception()
        .expect("take Rust exception")
        .expect("Rust exception is present")
    else {
        panic!("Rust exception was not an object for {source:?}");
    };
    assert!(
        runtime
            .is_error_object(&error)
            .expect("classify Rust Error")
    );
    let stack = runtime.intern_property_key("stack").expect("stack key");
    let Value::String(stack) = context
        .get_property(&error, &stack)
        .expect("read Rust Error.stack")
    else {
        panic!("Rust Error.stack was not a string for {source:?}");
    };
    stack.to_utf8_lossy()
}

fn oracle_stack(oracle: &OsStr, source: &str, description: &str) -> String {
    let output = Command::new(oracle)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        !output.status.success(),
        "QuickJS unexpectedly completed {description}: {source:?}",
    );
    let stderr = String::from_utf8(output.stderr)
        .unwrap_or_else(|error| panic!("QuickJS emitted non-UTF-8 stderr: {error}"));
    let start = stderr
        .find("    at ")
        .unwrap_or_else(|| panic!("QuickJS emitted no Error stack for {description}: {stderr:?}"));
    stderr[start..].to_owned()
}
