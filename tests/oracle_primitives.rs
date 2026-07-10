use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{Runtime, RuntimeError, Value};

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
        if (__qjo_index !== 0) {
            __qjo_units += ",";
        }
        __qjo_units += ("0000" + __qjo_hex).slice(-4);
    }
    print("string|" + __qjo_value.length + "|" + __qjo_units);
} else if (__qjo_value === null) {
    print(__qjo_type + "|null");
} else {
    print(__qjo_type + "|" + String(__qjo_value));
}
"#;

const CASES: &[(&str, &str)] = &[
    // Arithmetic, precedence, and primitive numeric coercion.
    ("arithmetic precedence", "1 + 2 * 3"),
    ("parenthesized arithmetic", "(1 + 2) * 3"),
    ("subtraction and division", "7 - 10 / 2"),
    ("signed remainder", "-7 % 4"),
    ("unary plus string coercion", "+' 42 '"),
    ("subtraction coercion", "'9' - true"),
    ("numeric literal precision", "9007199254740991 + 1"),
    ("invalid hexadecimal separator string", "+'0x1_'"),
    ("invalid hexadecimal sign string", "+'0x+1'"),
    ("non-ecmascript infinity spelling", "+'inf'"),
    ("fraction-only numeric string", "+'.125'"),
    ("trailing-dot exponent numeric string", "+'1.e2'"),
    ("signed exponent numeric string", "+'-1.5e+2'"),
    ("non-ascii whitespace after number", "0\u{00a0}+1"),
    // QuickJS bitwise operators apply ToNumeric, then signed ToInt32 for
    // Numbers or infinite-width two's-complement operations for BigInts.
    ("bitwise not zero", "~0"),
    ("bitwise not modulo 2^32", "~4294967296"),
    ("bitwise not nan", "~(0 / 0)"),
    ("bitwise not infinity", "~(1 / 0)"),
    ("bitwise not signed boundary", "~2147483648"),
    ("bitwise not unsigned boundary", "~4294967295"),
    ("bitwise fractional ToInt32", "-1.9 & 3.7"),
    ("bitwise positive wrap", "4294967297 & -1"),
    ("bitwise negative wrap", "-2147483649 | 0"),
    ("bitwise exact integer wrap", "9007199254740991 | 0"),
    ("bitwise huge finite wrap", "1e300 | 1"),
    ("bitwise removes negative zero", "-0 ^ 0"),
    ("bitwise primitive coercion", "'7' ^ true"),
    ("bitwise precedence", "1 | 2 ^ 3 & 4"),
    ("equality binds before bitwise", "1 | 2 === 3"),
    ("bitwise binds before logical", "0 || 1 | 2"),
    ("bitwise binds inside nullish rhs", "null ?? 1 | 2"),
    (
        "bitwise expression and coercion order",
        "(function(){ var log = ''; var left = function(){}; var right = function(){}; left.valueOf = function(){ log = log + 'l'; return 6; }; right.valueOf = function(){ log = log + 'r'; return 3; }; var evalLeft = function(){ log = log + 'L'; return left; }; var evalRight = function(){ log = log + 'R'; return right; }; var result = evalLeft() & evalRight(); return result + '|' + log; })()",
    ),
    // Number observations that String(number) alone cannot distinguish.
    ("positive infinity", "1 / 0"),
    ("negative infinity", "-1 / 0"),
    ("nan", "0 / 0"),
    ("negative zero literal", "-0"),
    ("negative zero arithmetic", "0 * -1"),
    // Addition and strings, including astral and unpaired UTF-16 units.
    ("string and number concatenation", "'answer: ' + 42"),
    ("string and null concatenation", "'value=' + null"),
    ("astral utf16 concatenation", r"'\uD83D' + '\uDE80'"),
    ("unpaired utf16 concatenation", r"'\uD800' + 'x'"),
    ("literal astral character", "'a🚀' + 'z'"),
    // Abstract and strict equality.
    ("abstract numeric string equality", "'42' == 42"),
    ("strict numeric string inequality", "'42' !== 42"),
    ("abstract null undefined equality", "null == void 0"),
    ("strict null undefined inequality", "null !== void 0"),
    ("abstract boolean equality", "false == 0"),
    ("nan is not equal to itself", "0 / 0 != 0 / 0"),
    ("number representations are strictly equal", "1 === 1.0"),
    ("trimmed radix string equality", r"'\uFEFF 0x10 ' == 16"),
    // Numeric and UTF-16 lexicographic relational comparison.
    ("numeric less than", "2 < 10"),
    ("relational numeric coercion", "'10' < 2"),
    ("lexicographic string comparison", "'10' < '2'"),
    ("utf16 string comparison", r"'\uD800' < '\uE000'"),
    ("nan relational comparison", "0 / 0 >= 0"),
    // Logical operators preserve the selected operand, including the sign of zero.
    ("short circuit and", "0 && -0"),
    ("short circuit or", "1 || -0"),
    ("logical and selected value", "'left' && 'right'"),
    ("logical or selected value", "'' || 'fallback'"),
    // Nullish coalescing uses exact null/undefined checks and preserves identity.
    ("nullish null selects rhs", "null ?? 42"),
    ("nullish undefined selects rhs", "void 0 ?? 'fallback'"),
    ("nullish false preserves lhs", "false ?? true"),
    ("nullish zero preserves lhs", "0 ?? 1"),
    ("nullish negative zero preserves lhs", "-0 ?? 1"),
    ("nullish empty string preserves lhs", "'' ?? 'fallback'"),
    ("nullish chain selects last", "null ?? void 0 ?? 'last'"),
    (
        "nullish short circuit skips rhs",
        "(function(){ var calls = 0; var value = 1 ?? (calls = 1); return value + calls; })()",
    ),
    (
        "nullish chain stops at zero",
        "(function(){ var calls = 0; var value = null ?? (calls = calls + 1, 0) ?? (calls = 99); return value + calls; })()",
    ),
    ("nullish arithmetic precedence", "null ?? 1 + 2 * 3"),
    ("nullish conditional precedence", "0 ?? 1 ? 2 : 3"),
    ("parenthesized logical before nullish", "(false || 4) ?? 5"),
    (
        "parenthesized logical after nullish",
        "null ?? (false || 6)",
    ),
    (
        "nullish anonymous function is not named",
        "(function(){ var inferred; inferred = null ?? function(){}; return inferred.name; })()",
    ),
    (
        "nullish result call drops member receiver",
        "(Function.__nullishMethod = function(){ return this === Function; }, (Function.__nullishMethod ?? Function)())",
    ),
    (
        "nullish rhs member call keeps receiver",
        "(Function.__nullishRhs = function(){ return this === Function; }, null ?? Function.__nullishRhs())",
    ),
    // Identifier compound assignment shares QuickJS's late scope get/set path.
    (
        "identifier arithmetic compound local matrix",
        "(function(){ var value = 20; value += 2; value -= 4; value *= 3; value /= 2; value %= 5; return value; })()",
    ),
    (
        "identifier arithmetic compound argument",
        "(function(value){ value += 2; return value; })(3)",
    ),
    (
        "identifier arithmetic compound closure",
        "(function(value){ return (function(){ value += 2; return value; })() + (function(){ value += 3; return value; })(); })(1)",
    ),
    (
        "identifier arithmetic compound global",
        "(__qjo_identifier_compound = 1, __qjo_identifier_compound += 2)",
    ),
    (
        "identifier arithmetic compound is right associative",
        "(function(){ var left = 1, right = 2; var result = left += right *= 3; return result + left + right; })()",
    ),
    (
        "identifier logical and taken",
        "(function(value){ value &&= 9; return value; })(2)",
    ),
    (
        "identifier logical and short",
        "(function(value){ value &&= 9; return value; })(0)",
    ),
    (
        "identifier logical or taken",
        "(function(value){ value ||= 9; return value; })(0)",
    ),
    (
        "identifier logical or short",
        "(function(value){ value ||= 9; return value; })(2)",
    ),
    (
        "identifier logical nullish taken",
        "(function(value){ value ??= 9; return value; })(null)",
    ),
    (
        "identifier logical nullish false short",
        "(function(value){ value ??= 9; return value; })(false)",
    ),
    (
        "identifier logical function object short",
        "typeof (Function ||= 1)",
    ),
    (
        "identifier logical direct name inference",
        "(function(){ var named; named ??= function(){}; return named.name; })()",
    ),
    (
        "identifier logical parenthesized lhs has no name inference",
        "(function(){ var named; (named) ??= function(){}; return named.name; })()",
    ),
    (
        "identifier logical parenthesized assignment still infers",
        "(function(){ var named; (named ??= function(){}); return named.name; })()",
    ),
    (
        "identifier simple parenthesized lhs has no name inference",
        "(function(){ var named; (named) = function(){}; return named.name; })()",
    ),
    (
        "identifier logical right associative inner name",
        "(function(){ var a = 0, b = 0; a ||= b ||= function(){}; return a.name + '|' + b.name; })()",
    ),
    (
        "identifier logical comma rhs has no name inference",
        "(function(){ var named; named ||= (0, function(){}); return named.name; })()",
    ),
    (
        "sloppy private name arithmetic ignores write",
        "(function named(){ var result = named += ''; return typeof result + '|' + typeof named; })()",
    ),
    (
        "sloppy private name logical names rhs and ignores write",
        "(function named(){ var before = named; var result = named &&= function(){}; return (named === before) + '|' + result.name; })()",
    ),
    (
        "strict private name logical short",
        "(function named(){ 'use strict'; var before = named; return (named ||= 1) === before; })()",
    ),
    (
        "captured sloppy private name ignores write",
        "(function named(){ return function(){ named += ''; return typeof named; }; })()()",
    ),
    (
        "identifier bitwise compound matrix",
        "(function(){ var value = 14; value &= 11; value ^= 3; value |= 4; return value; })()",
    ),
    (
        "identifier bitwise compound is right associative",
        "(function(){ var left = 1, right = 3; var result = left |= right &= 2; return result * 100 + left * 10 + right; })()",
    ),
    (
        "fixed member bitwise compound matrix",
        "(function(){ Function.__qjo_bits = 14; Function.__qjo_bits &= 11; Function.__qjo_bits ^= 3; return Function.__qjo_bits |= 4; })()",
    ),
    (
        "computed member bitwise compound converts key once",
        "(function(){ var log = ''; var key = function(){}; key.toString = function(){ log = log + 'k'; return '__qjo_computed_bits'; }; Function.__qjo_computed_bits = 14; var result = Function[key] &= 11; return result + '|' + Function.__qjo_computed_bits + '|' + log; })()",
    ),
    (
        "bitwise compound does not infer anonymous function names",
        "(function(){ var names = ''; Function.prototype.valueOf = function(){ names = names + this.name + '|'; return 1; }; var direct = 0, paren = 0; direct |= function(){}; (paren) |= function(){}; Function.__qjo_name = 0; Function.__qjo_name |= function(){}; delete Function.prototype.valueOf; return names; })()",
    ),
    // Conditional selection and associativity.
    ("truthy conditional", "'x' ? -0 : 1"),
    ("falsy conditional", "0 ? 1 : 'no'"),
    ("nested conditional", "false ? 0 : true ? 1 : 2"),
    // Every currently materialized JavaScript primitive typeof result.
    ("typeof undefined", "typeof void 0"),
    ("typeof null", "typeof null"),
    ("typeof boolean", "typeof false"),
    ("typeof number", "typeof (1 / 0)"),
    ("typeof string", "typeof 'oxide'"),
    ("typeof global intrinsic", "typeof Error"),
    ("inherited global object lookup", "typeof toString"),
    ("typeof missing global", "typeof __qjo_missing_global"),
    (
        "typeof parenthesized missing global",
        "typeof ((__qjo_missing_global))",
    ),
    // Source-level functions use late identifier resolution and runtime FClosure.
    (
        "anonymous function parameters and direct call",
        "(function(a, b) { return a + b; })(20, 22)",
    ),
    (
        "missing function argument is undefined",
        "(function(a) { return typeof a; })()",
    ),
    (
        "extra function argument preserves declared slot",
        "(function(a) { return a; })(42, 99)",
    ),
    (
        "sloppy escaped contextual parameter and reference",
        "(function(impl\\u0065ments) { return impl\\u0065ments; })(1)",
    ),
    ("ordinary function fallthrough", "(function() { 1; })()"),
    (
        "return restricted production ASI",
        "(function() { return\n42; })()",
    ),
    (
        "transitive captured parameter",
        "(function(a) { return function() { return function(b) { return a + b; }; }; })(20)()(22)",
    ),
    (
        "transitive captured function local",
        "(function() { var a = 20; return function() { return function(b) { return a + b; }; }; })()()(22)",
    ),
    (
        "var binding is hoisted before initialization",
        "(function() { return x; var x = 1; })()",
    ),
    (
        "named function private recursion",
        "(function fact(n) { return n ? n * fact(n - 1) : 1; })(5)",
    ),
    (
        "dynamic Function anonymous wrapper self binding",
        "(function(f) { return f() === f; })(function anonymous() { return anonymous; })",
    ),
    (
        "named function transitive private capture",
        "(function(f) { return f()()() === f; })(function named() { return function() { return function() { return named; }; }; })",
    ),
    (
        "sloppy named function assignment preserves private binding",
        "(function(f) { return f() === f; })(function named() { named = 1; return named; })",
    ),
    (
        "nested strict write inherits sloppy function-name binding behavior",
        "(function(f) { return f()() === f; })(function named() { return function() { 'use strict'; named = 1; return named; }; })",
    ),
    (
        "named function parameter shadows private binding",
        "(function named(named) { return named; })(42)",
    ),
    (
        "named function var shadows private binding",
        "(function named() { var named = 42; return named; })()",
    ),
    (
        "named function binding is not visible outside",
        "(function named() {}), typeof named",
    ),
    ("typeof function object", "typeof (function() {})"),
    // QuickJS short/heap BigInt normalization and primitive integration.
    ("short bigint addition", "1n + 2n"),
    (
        "heap bigint multiplication",
        "123456789012345678901234567890n * 98765432109876543210n",
    ),
    ("bigint division truncates toward zero", "-7n / 3n"),
    ("bigint remainder follows dividend", "-7n % 3n"),
    ("bigint unary negation", "-9223372036854775808n"),
    ("bigint bitwise not", "~0n"),
    ("bigint bitwise not negative one", "~-1n"),
    ("bigint negative xor", "-1n ^ 255n"),
    ("bigint negative and sign extension", "-5n & 3n"),
    ("bigint negative or sign extension", "-5n | 2n"),
    ("bigint negative xor sign extension", "-5n ^ 2n"),
    (
        "heap bigint bitwise and",
        "123456789012345678901234567890n & -1n",
    ),
    (
        "heap bigint bitwise compound matrix",
        "(function(){ var value = -1n; value &= 123456789012345678901234567890n; value ^= 15n; value |= 2n; return value; })()",
    ),
    ("bigint string concatenation", "1n + ' oxide'"),
    ("bigint number abstract equality", "1n == 1"),
    ("bigint boolean abstract equality", "0n == false"),
    ("bigint string abstract equality", "255n == '0xff'"),
    (
        "bigint number precision inequality",
        "9007199254740993n == 9007199254740993",
    ),
    ("bigint fractional comparison", "2n < 2.5"),
    ("negative bigint fractional comparison", "-2n > -2.5"),
    ("bigint string relational conversion", "10n < '11'"),
    ("bigint logical selection", "0n || 2n"),
    ("typeof bigint", "typeof 1n"),
];

const SHARED_ERRORS: &[(&str, &str)] = &[
    (
        "strict mode reaches earlier directive escapes",
        r#"'\1'; 'use strict'; 0"#,
    ),
    (
        "strict mode rejects legacy octal numbers",
        "'use strict'; 010",
    ),
    ("conditional consequent excludes comma", "true ? 1, 2 : 3"),
    ("logical or cannot precede nullish", "1 || 2 ?? 3"),
    ("logical and cannot precede nullish", "1 && 2 ?? 3"),
    ("nullish cannot precede logical or", "1 ?? 2 || 3"),
    ("nullish cannot precede logical and", "1 ?? 2 && 3"),
    ("adjacent numeric expressions need ASI", "1 2"),
    ("malformed hexadecimal literal", "0x"),
    ("legacy octal cannot have a fraction", "01.1"),
    ("legacy octal cannot have an exponent", "01e1"),
    (
        "legacy leading-zero decimal cannot use separators",
        "08.1_2",
    ),
    ("unary plus rejects BigInt", "+1n"),
    ("mixed numeric addition rejects BigInt", "1n + 1"),
    ("BigInt division by zero", "1n / 0n"),
    (
        "strict function rejects future reserved parameter",
        "(function(implements) { 'use strict'; return implements; })(1)",
    ),
    (
        "strict function rejects eval var binding",
        "(function() { 'use strict'; var eval = 1; return eval; })()",
    ),
    (
        "strict function rejects future reserved identifier reference",
        "(function() { 'use strict'; return impl\\u0065ments; })()",
    ),
    (
        "strict function rejects eval expression name",
        "(function eval() { 'use strict'; })",
    ),
    (
        "escaped reserved word is not an identifier",
        "(function(\\u0069f) { return \\u0069f; })(1)",
    ),
];

const QUICKJS_ACCEPTED_DIRECTIVE_EDGES: &[(&str, &str)] = &[
    (
        "continued use-strict string is not a directive",
        "'use strict' + ''; 010",
    ),
    (
        "QuickJS directive ASI excludes logical not",
        "'use strict'\n!0; 010",
    ),
    (
        "QuickJS directive ASI excludes void",
        "'use strict'\nvoid 0; 010",
    ),
];

const RUNTIME_ERROR_CASES: &[(&str, &str)] = &[
    ("logical before nullish syntax message", "1 || 2 ?? 3"),
    ("nullish before logical syntax message", "1 ?? 2 || 3"),
    (
        "identifier compound missing global",
        "__qjo_missing_compound += 1",
    ),
    (
        "identifier logical compound missing global",
        "__qjo_missing_logical ||= 1",
    ),
    (
        "strict private name arithmetic write",
        "(function named(){ 'use strict'; named += 1; })()",
    ),
    (
        "strict private name logical write",
        "(function named(){ 'use strict'; named &&= 1; })()",
    ),
    (
        "captured strict private name logical write",
        "(function named(){ 'use strict'; return function(){ named &&= 1; }; })()()",
    ),
    (
        "strict eval compound early error",
        "(function(){ 'use strict'; eval += 1; })",
    ),
    (
        "strict eval simple assignment early error",
        "(function(){ 'use strict'; eval = 1; })",
    ),
    (
        "strict arguments compound early error",
        "(function(){ 'use strict'; (arguments) ??= 1; })",
    ),
    (
        "strict arguments simple assignment early error",
        "(function(){ 'use strict'; (arguments) = 1; })",
    ),
    ("mixed BigInt arithmetic", "1n + 1"),
    ("mixed BigInt bitwise left", "1n & 1"),
    ("mixed BigInt bitwise right", "1 | 1n"),
    ("BigInt unary plus", "+1n"),
    (
        "strict private name bitwise write",
        "(function named(){ 'use strict'; named &= 1; })()",
    ),
    (
        "strict eval bitwise compound early error",
        "(function(){ 'use strict'; eval &= 1; })",
    ),
    (
        "strict arguments bitwise compound early error",
        "(function(){ 'use strict'; (arguments) ^= 1; })",
    ),
    ("call non-callable", "(1)()"),
    ("construct non-callable", "new 1"),
    ("throw line terminator", "throw\n9"),
    (
        "typeof comma expression still resolves its identifier",
        "typeof (0, __qjo_missing_global)",
    ),
    (
        "strict named function assignment is read only",
        "(function named() { 'use strict'; named = 1; })()",
    ),
    (
        "captured strict named function assignment is read only",
        "(function named() { 'use strict'; return function() { named = 1; }; })()()",
    ),
];

#[test]
fn primitive_expressions_match_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP primitive oracle differential: set QJS_ORACLE to an upstream qjs executable"
        );
        return;
    };

    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for &(description, source) in CASES {
        let rust_value = context.eval(source).unwrap_or_else(|error| {
            panic!("Rust evaluation failed for {description:?} ({source:?}): {error}")
        });
        let rust_observation = normalize_rust_value(&rust_value);
        let oracle_observation = run_oracle(&oracle, source, description);

        assert_eq!(
            rust_observation, oracle_observation,
            "primitive differential mismatch for {description:?} ({source:?})"
        );
    }
}

#[test]
fn implemented_errors_match_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP syntax-error oracle differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for &(description, source) in SHARED_ERRORS {
        assert!(
            context.eval(source).is_err(),
            "Rust unexpectedly accepted {description:?} ({source:?})"
        );
        let output = Command::new(&oracle)
            .args(["-e", source])
            .output()
            .unwrap_or_else(|error| panic!("could not run oracle for {description}: {error}"));
        assert!(
            !output.status.success(),
            "oracle unexpectedly accepted {description:?} ({source:?})"
        );
    }

    for &(description, source) in QUICKJS_ACCEPTED_DIRECTIVE_EDGES {
        context.eval(source).unwrap_or_else(|error| {
            panic!("Rust unexpectedly rejected {description:?} ({source:?}): {error}")
        });
        let output = Command::new(&oracle)
            .args(["-e", source])
            .output()
            .unwrap_or_else(|error| panic!("could not run oracle for {description}: {error}"));
        assert!(
            output.status.success(),
            "oracle unexpectedly rejected {description:?} ({source:?}): {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn runtime_error_kind_and_message_match_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP runtime-error oracle differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let name_key = runtime.intern_property_key("name").unwrap();
    let message_key = runtime.intern_property_key("message").unwrap();
    for &(description, source) in RUNTIME_ERROR_CASES {
        assert_eq!(context.eval(source), Err(RuntimeError::Exception));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("Rust did not materialize an Error object for {description:?}");
        };
        let Value::String(name) = context.get_property(&error, &name_key).unwrap() else {
            panic!("Rust Error name was not a string for {description:?}");
        };
        let Value::String(message) = context.get_property(&error, &message_key).unwrap() else {
            panic!("Rust Error message was not a string for {description:?}");
        };
        let rust_observation = format!("{}|{}", name.to_utf8_lossy(), message.to_utf8_lossy());
        let oracle_observation = run_oracle_error(&oracle, source, description);
        assert_eq!(
            rust_observation, oracle_observation,
            "runtime-error differential mismatch for {description:?} ({source:?})"
        );
    }
}

#[test]
fn compiler_call_capacity_matches_quickjs_error_classes() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP compiler-capacity oracle differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let arguments = std::iter::repeat_n("0", usize::from(u16::MAX))
        .collect::<Vec<_>>()
        .join(",");
    let source = format!("(function() {{}})({arguments})");

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.compile(&source), Err(RuntimeError::Exception));
    let rust_observation = take_rust_error_observation(&runtime, &mut context);
    assert_eq!(rust_observation, "InternalError|stack overflow");

    let oracle_observation = run_oracle_error(&oracle, &source, "65,535 call arguments");
    assert!(
        oracle_observation.starts_with("InternalError|stack overflow"),
        "QuickJS call-stack boundary changed: {oracle_observation:?}"
    );

    let mut too_many = source;
    let closing_parenthesis = too_many
        .rfind(')')
        .expect("generated call expression has a closing parenthesis");
    too_many.insert_str(closing_parenthesis, ",0");
    assert_eq!(context.compile(&too_many), Err(RuntimeError::Exception));
    assert_eq!(
        take_rust_error_observation(&runtime, &mut context),
        "SyntaxError|Too many call arguments"
    );
    assert_eq!(
        run_oracle_error(&oracle, &too_many, "65,536 call arguments"),
        "SyntaxError|Too many call arguments"
    );
}

fn run_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
    let script = format!("var __qjo_value = ({source});\n{}", ORACLE_NORMALIZER);
    let output = Command::new(oracle)
        .arg("-e")
        .arg(script)
        .output()
        .unwrap_or_else(|error| {
            panic!("could not execute QJS_ORACLE for {description:?} ({source:?}): {error}")
        });

    assert!(
        output.status.success(),
        "QJS_ORACLE failed for {description:?} ({source:?}) with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap_or_else(|error| {
        panic!("QJS_ORACLE emitted non-UTF-8 output for {description:?} ({source:?}): {error}")
    });
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        1,
        "QJS_ORACLE must emit exactly one observation for {description:?} ({source:?}); stdout was {stdout:?}"
    );
    lines[0].to_owned()
}

fn run_oracle_error(oracle: &OsStr, source: &str, description: &str) -> String {
    let quoted = source
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    let script = format!(
        "try {{ eval(\"{quoted}\"); print(\"<no error>\"); }} catch (error) {{ print(error.name + \"|\" + error.message); }}"
    );
    let output = Command::new(oracle)
        .arg("-e")
        .arg(script)
        .output()
        .unwrap_or_else(|error| {
            panic!("could not execute QJS_ORACLE for {description:?} ({source:?}): {error}")
        });
    assert!(
        output.status.success(),
        "QJS_ORACLE failed for {description:?} ({source:?}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QJS_ORACLE emitted non-UTF-8 error output")
        .trim_end()
        .to_owned()
}

fn take_rust_error_observation(runtime: &Runtime, context: &mut quickjs_oxide::Context) -> String {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Rust did not materialize a native Error object");
    };
    let name_key = runtime.intern_property_key("name").unwrap();
    let message_key = runtime.intern_property_key("message").unwrap();
    let Value::String(name) = context.get_property(&error, &name_key).unwrap() else {
        panic!("Rust Error name was not a string");
    };
    let Value::String(message) = context.get_property(&error, &message_key).unwrap() else {
        panic!("Rust Error message was not a string");
    };
    format!("{}|{}", name.to_utf8_lossy(), message.to_utf8_lossy())
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
