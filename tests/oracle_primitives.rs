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
    // Exponentiation lives above multiplicative expressions, associates to the
    // right, and retains QuickJS's libc-pow special cases.
    ("number exponentiation", "2 ** 10"),
    ("exponentiation is right associative", "2 ** 3 ** 2"),
    ("power binds before multiplication", "2 * 3 ** 2"),
    ("power result binds into multiplication", "2 ** 3 * 4"),
    ("power accepts unary rhs", "2 ** -2"),
    ("parenthesized negative power base", "(-2) ** 3"),
    ("parenthesized power under unary minus", "-(2 ** 2)"),
    ("parenthesized power inside unary rhs", "2 ** -(2 ** 3)"),
    ("nan to the zero power", "(0 / 0) ** 0"),
    ("undefined to the zero power", "(void 0) ** 0"),
    ("one to nan power is nan", "1 ** (0 / 0)"),
    ("one to infinite power is nan", "1 ** (1 / 0)"),
    (
        "negative one to negative infinite power is nan",
        "(-1) ** (-1 / 0)",
    ),
    ("negative zero odd positive power", "(-0) ** 3"),
    ("negative zero even positive power", "(-0) ** 2"),
    ("negative zero odd negative power", "(-0) ** -3"),
    ("negative zero even negative power", "(-0) ** -2"),
    ("negative base fractional power", "(-2) ** 0.5"),
    ("minimum positive power subnormal", "2 ** -1074"),
    ("positive power underflow", "2 ** -1075"),
    ("negative power underflow keeps sign", "(-2) ** -1075"),
    ("negative infinity odd power", "(-1 / 0) ** 3"),
    ("negative infinity negative odd power", "(-1 / 0) ** -3"),
    ("negative infinity fractional power", "(-1 / 0) ** 0.5"),
    ("power primitive conversion", "'2' ** true"),
    (
        "power expression and coercion order",
        "(function(){ var log = ''; var left = function(){}; var right = function(){}; left.valueOf = function(){ log = log + 'l'; return 2; }; right.valueOf = function(){ log = log + 'r'; return 3; }; var evalLeft = function(){ log = log + 'L'; return left; }; var evalRight = function(){ log = log + 'R'; return right; }; var result = evalLeft() ** evalRight(); return result + '|' + log; })()",
    ),
    (
        "right associative power evaluation and coercion order",
        "(function(){ var log = ''; var a = function(){}, b = function(){}, c = function(){}; a.valueOf = function(){ log = log + 'a'; return 2; }; b.valueOf = function(){ log = log + 'b'; return 3; }; c.valueOf = function(){ log = log + 'c'; return 2; }; var evalA = function(){ log = log + 'A'; return a; }; var evalB = function(){ log = log + 'B'; return b; }; var evalC = function(){ log = log + 'C'; return c; }; var result = evalA() ** evalB() ** evalC(); return result + '|' + log; })()",
    ),
    (
        "power object operands both convert to BigInt",
        "(function(){ var log = ''; var left = function(){}, right = function(){}; left.valueOf = function(){ log = log + 'l'; return 2n; }; right.valueOf = function(){ log = log + 'r'; return 10n; }; return (left ** right) + '|' + log; })()",
    ),
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
    // Shift operators share QuickJS's ordered ToNumeric path. Number counts
    // are masked to five bits, while >>> preserves an unsigned 32-bit result.
    ("shift left number", "1 << 3"),
    ("shift right signed number", "-8 >> 2"),
    ("shift right unsigned number", "-1 >>> 0"),
    ("shift count masks at 32", "1 << 33"),
    ("negative number shift count wraps", "1 << -1"),
    ("shift count masks uint32 max", "1 << 4294967295"),
    ("shift count wraps at uint32", "1 << 4294967296"),
    ("signed shift converts lhs", "4294967295 >> 0"),
    ("unsigned shift converts lhs", "4294967295 >>> 0"),
    ("nan shifts as zero", "(0 / 0) << 5"),
    ("nan shift count is zero", "7 >> (0 / 0)"),
    ("infinity shifts as zero", "(1 / 0) >>> 0"),
    ("infinity shift count is zero", "7 >>> (1 / 0)"),
    ("shift removes negative zero", "-0 << 0"),
    ("fractional shift conversion", "-9.9 >> 1.9"),
    ("string and boolean shift conversion", "'7' << true"),
    ("additive binds before shift", "1 + 2 << 3"),
    ("shift rhs includes additive", "16 >> 1 + 1"),
    ("shift binds before relational", "1 << 2 < 5"),
    ("shift binds before bitwise", "8 >> 1 & 3"),
    ("shift operators are left associative", "64 >> 2 >> 1"),
    ("shift binds inside nullish rhs", "1 ?? 2 << 3"),
    (
        "shift expression and coercion order",
        "(function(){ var log = ''; var left = function(){}; var right = function(){}; left.valueOf = function(){ log = log + 'l'; return 8; }; right.valueOf = function(){ log = log + 'r'; return 1; }; var evalLeft = function(){ log = log + 'L'; return left; }; var evalRight = function(){ log = log + 'R'; return right; }; var result = evalLeft() >>> evalRight(); return result + '|' + log; })()",
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
        "sloppy private name postfix update ignores write",
        "(function named(){ var before = named; named.valueOf = function(){ return 4; }; var old = named++; return old + '|' + (named === before) + '|' + typeof named; })()",
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
    (
        "identifier shift compound matrix",
        "(function(){ var value = 8; value <<= 2; value >>= 1; value >>>= 2; return value; })()",
    ),
    (
        "identifier shift compound is right associative",
        "(function(){ var left = 1, right = 3; var result = left <<= right >>= 1; return result * 100 + left * 10 + right; })()",
    ),
    (
        "bigint shift compound assignment",
        "(function(){ var value = 1n; value <<= 65n; value >>= 64n; return value; })()",
    ),
    (
        "fixed member shift compound matrix",
        "(function(){ Function.__qjo_shift = -8; Function.__qjo_shift >>= 1; return Function.__qjo_shift >>>= 1; })()",
    ),
    (
        "computed member shift compound converts key once",
        "(function(){ var log = ''; var key = function(){}; key.toString = function(){ log = log + 'k'; return '__qjo_computed_shift'; }; Function.__qjo_computed_shift = 3; var result = Function[key] <<= 2; return result + '|' + Function.__qjo_computed_shift + '|' + log; })()",
    ),
    (
        "shift compound does not infer anonymous function names",
        "(function(){ var names = ''; Function.prototype.valueOf = function(){ names = names + this.name + '|'; return 1; }; var direct = 1, paren = 1; direct <<= function(){}; (paren) >>= function(){}; Function.__qjo_shift_name = 1; Function.__qjo_shift_name >>>= function(){}; delete Function.prototype.valueOf; return names; })()",
    ),
    (
        "identifier exponent compound",
        "(function(value){ value **= 3; return value; })(2)",
    ),
    (
        "identifier exponent compound is right associative",
        "(function(){ var left = 2, right = 3; var result = left **= right **= 2; return result + left + right; })()",
    ),
    (
        "bigint exponent compound assignment",
        "(function(){ var value = 2n; value **= 100n; return value; })()",
    ),
    (
        "fixed member exponent compound",
        "(function(){ Function.__qjo_power = 2; var result = Function.__qjo_power **= 3; return result + Function.__qjo_power; })()",
    ),
    (
        "computed member exponent compound converts key once",
        "(function(){ var log = ''; var key = function(){}; key.toString = function(){ log = log + 'k'; return '__qjo_computed_power'; }; Function.__qjo_computed_power = 2; var result = Function[key] **= 3; return result + '|' + Function.__qjo_computed_power + '|' + log; })()",
    ),
    (
        "exponent compound does not infer anonymous function names",
        "(function(){ var names = ''; Function.prototype.valueOf = function(){ names = names + this.name + '|'; return 1; }; var direct = 1, paren = 1; direct **= function(){}; (paren) **= function(){}; Function.__qjo_power_name = 1; Function.__qjo_power_name **= function(){}; delete Function.prototype.valueOf; return names; })()",
    ),
    // Update expressions retain QuickJS's dedicated inc/dec and post-inc/dec
    // semantics: postfix returns the converted old Numeric while prefix
    // returns the replacement written through the original Reference.
    (
        "identifier update value and conversion matrix",
        "(function(){ var value = '01'; var a = value++; var b = ++value; var c = value--; var d = --value; return typeof a + '|' + a + '|' + b + '|' + c + '|' + d + '|' + value; })()",
    ),
    (
        "postfix object returns converted old numeric",
        "(function(){ var value = function(){}; value.valueOf = function(){ return 4; }; var original = value; var old = value++; return (old === original) + '|' + typeof old + '|' + old + '|' + value; })()",
    ),
    (
        "bigint update matrix",
        "(function(){ var value = 4n; var old = value--; var replacement = ++value; return old * 100n + replacement * 10n + value; })()",
    ),
    (
        "captured identifier updates share one cell",
        "(function(value){ var update = function(){ return value++; }; return update() * 100 + (++value) * 10 + value; })(2)",
    ),
    (
        "global identifier postfix update",
        "(__qjo_update_global = '01', __qjo_update_global++ * 10 + __qjo_update_global)",
    ),
    (
        "fixed member update preserves old and new values",
        "(function(){ Function.__qjo_update = '01'; var old = Function.__qjo_update++; return old + '|' + ++Function.__qjo_update; })()",
    ),
    (
        "computed member update converts key once",
        "(function(){ var log = ''; var key = function(){}; key.toString = function(){ log = log + 'k'; return '__qjo_computed_update'; }; Function.__qjo_computed_update = '01'; var old = Function[key]++; return typeof old + '|' + old + '|' + Function.__qjo_computed_update + '|' + log; })()",
    ),
    (
        "prefix and postfix updates bind before power",
        "(function(){ var value = 2; return (++value ** 2) * 100 + (value++ ** 2) * 10 + value; })()",
    ),
    (
        "postfix line terminator starts prefix statement",
        "(function(){ var x = 1, y = 2; var result = x\n++y; return result * 100 + x * 10 + y; })()",
    ),
    (
        "postfix CRLF starts prefix statement",
        "(function(){ var x = 1, y = 2; var result = x\r\n++y; return result * 100 + x * 10 + y; })()",
    ),
    (
        "postfix line-separator starts prefix statement",
        "(function(){ var x = 1, y = 2; var result = x\u{2028}++y; return result * 100 + x * 10 + y; })()",
    ),
    (
        "postfix paragraph-separator starts prefix statement",
        "(function(){ var x = 1, y = 2; var result = x\u{2029}++y; return result * 100 + x * 10 + y; })()",
    ),
    (
        "postfix block-comment line starts prefix statement",
        "(function(){ var x = 1, y = 2; var result = x/*\n*/++y; return result * 100 + x * 10 + y; })()",
    ),
    (
        "postfix survives comment without line terminator",
        "(function(){ var value = 1; var old = value/**/++; return old * 10 + value; })()",
    ),
    (
        "prefix accepts a line terminator",
        "(function(){ var value = 1; return ++\nvalue; })()",
    ),
    (
        "parenthesized references remain update targets",
        "(function(){ var value = 1; var old = (value)++; return old * 100 + (++(value)) * 10 + value; })()",
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
    ("bigint exponentiation", "2n ** 100n"),
    ("negative bigint exponentiation", "(-2n) ** 3n"),
    ("zero bigint to zero power", "0n ** 0n"),
    (
        "zero bigint huge exponent shortcut",
        "0n ** 999999999999999999999999999999999999999n",
    ),
    (
        "one bigint huge exponent shortcut",
        "1n ** 999999999999999999999999999999999999999n",
    ),
    (
        "negative one bigint huge odd exponent shortcut",
        "(-1n) ** 999999999999999999999999999999999999999n",
    ),
    (
        "quickjs power of two allocation success boundary",
        "typeof (2n ** 1048574n)",
    ),
    (
        "quickjs negative power allocation sign boundary",
        "typeof ((-2n) ** 1048575n)",
    ),
    (
        "quickjs four power allocation success boundary",
        "typeof (4n ** 524287n)",
    ),
    (
        "quickjs generic bigint square allocation boundary",
        "typeof (((1n << 524286n) + 1n) ** 2n)",
    ),
    (
        "quickjs nominal bigint power identity",
        "((1n << 1048574n) ** 1n) >> 1048574n",
    ),
    (
        "quickjs extended bigint zeroth power shortcut",
        "(1n << 1048575n) ** 0n",
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
    ("bigint left shift", "1n << 65n"),
    ("negative bigint arithmetic right shift", "-9n >> 2n"),
    ("bigint negative left count reverses", "8n << -1n"),
    ("bigint negative right count reverses", "8n >> -2n"),
    (
        "bigint huge right shift saturates positive",
        "123456789n >> 999999999999999999999999999999n",
    ),
    (
        "bigint huge right shift saturates negative",
        "-1n >> 999999999999999999999999999999n",
    ),
    (
        "zero bigint huge left shift stays zero",
        "0n << 999999999999999999999999999999n",
    ),
    (
        "zero bigint reverse huge right shift stays zero",
        "0n >> -999999999999999999999999999999n",
    ),
    (
        "quickjs bigint shift sign-limb extension boundary",
        "typeof (1n << 1048575n)",
    ),
    (
        "quickjs bigint shift boundary round trip",
        "(1n << 1048575n) >> 1048575n",
    ),
    (
        "quickjs bigint extended value right shifts one limb",
        "((1n << 1048575n) >> 64n) >> 1048511n",
    ),
    (
        "quickjs bigint extended value negative count shifts one limb",
        "((1n << 1048575n) << -64n) >> 1048511n",
    ),
    (
        "quickjs negative bigint shift boundary",
        "typeof (-1n << 1048575n)",
    ),
    (
        "quickjs bigint add sign-limb extension boundary",
        "typeof ((1n << 1048574n) + (1n << 1048574n))",
    ),
    (
        "quickjs bigint negation sign-limb extension boundary",
        "typeof -(-1n << 1048575n)",
    ),
    (
        "quickjs max allocation bigint additive identity",
        "((1n << 1048574n) + 0n) >> 1048574n",
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
    ("unparenthesized negative power lhs", "-2 ** 2"),
    ("unparenthesized positive power lhs", "+2 ** 2"),
    ("unparenthesized logical-not power lhs", "!2 ** 2"),
    ("unparenthesized bitwise-not power lhs", "~2 ** 2"),
    ("unparenthesized typeof power lhs", "typeof 2 ** 2"),
    ("unparenthesized void power lhs", "void 2 ** 2"),
    ("unparenthesized delete power lhs", "delete Function ** 2"),
    ("unparenthesized unary power rhs chain", "2 ** -2 ** 3"),
    (
        "unparenthesized unary containing postfix update power lhs",
        "(function(value){ return -value++ ** 2; })(2)",
    ),
    (
        "prefix update rejects a power expression operand",
        "(function(value){ return ++(value ** 2); })(2)",
    ),
    (
        "postfix update rejects a power expression operand",
        "(function(value){ return (value ** 2)++; })(2)",
    ),
    ("prefix update rejects a value operand", "++1"),
    ("postfix update rejects a value operand", "1++"),
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
    ("unparenthesized unary power syntax message", "-2 ** 2"),
    ("prefix update invalid operand", "++1"),
    ("postfix update invalid operand", "1++"),
    (
        "strict eval prefix update early error",
        "(function(){ 'use strict'; ++eval; })",
    ),
    (
        "strict arguments postfix update early error",
        "(function(){ 'use strict'; arguments++; })",
    ),
    (
        "missing global postfix update",
        "__qjo_missing_update_global++",
    ),
    (
        "strict private name postfix update",
        "(function named(){ 'use strict'; return named++; })()",
    ),
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
    ("mixed BigInt shift left", "1n << 1"),
    ("mixed BigInt shift right", "1 >> 1n"),
    ("unsigned shift rejects two BigInts", "1n >>> 0n"),
    ("unsigned shift rejects bigint lhs", "1n >>> 0"),
    ("unsigned shift rejects bigint rhs", "1 >>> 0n"),
    ("oversized bigint left shift", "1n << 1048576n"),
    ("mixed BigInt exponent lhs", "1n ** 1"),
    ("mixed BigInt exponent rhs", "1 ** 1n"),
    ("negative BigInt exponent", "2n ** -1n"),
    ("zero BigInt negative exponent", "0n ** -1n"),
    ("power-of-two BigInt allocation boundary", "2n ** 1048575n"),
    (
        "negative power-of-two BigInt allocation boundary",
        "(-2n) ** 1048576n",
    ),
    ("BigInt exponent mathematical size guard", "2n ** 1048577n"),
    ("BigInt exponent int32 guard", "3n ** 2147483648n"),
    ("four power BigInt allocation boundary", "4n ** 524288n"),
    (
        "extended BigInt power identity allocation guard",
        "(1n << 1048575n) ** 1n",
    ),
    (
        "negative exponent precedes extended base allocation guard",
        "(1n << 1048575n) ** -1n",
    ),
    (
        "nominal BigInt square preallocation guard",
        "(1n << 1048574n) ** 2n",
    ),
    (
        "postfix BigInt update allocation guard",
        "(function(){ var value = 1n << 1048575n; return value++; })()",
    ),
    (
        "prefix BigInt decrement allocation guard",
        "(function(){ var value = 1n << 1048575n; return --value; })()",
    ),
    (
        "generic BigInt square next allocation boundary",
        "((1n << 524287n) + 1n) ** 2n",
    ),
    (
        "nonzero bigint reverse huge right shift overflows",
        "1n >> -999999999999999999999999999999n",
    ),
    (
        "extended bigint zero left shift allocation guard",
        "(1n << 1048575n) << 0n",
    ),
    (
        "extended bigint zero right shift allocation guard",
        "(1n << 1048575n) >> 0n",
    ),
    (
        "extended bigint sub-limb right shift allocation guard",
        "(1n << 1048575n) >> 63n",
    ),
    (
        "extended bigint bitwise allocation guard",
        "(1n << 1048575n) & -1n",
    ),
    (
        "extended bigint arithmetic allocation guard",
        "(1n << 1048575n) + 0n",
    ),
    (
        "extended bigint string allocation guard",
        "'' + (1n << 1048575n)",
    ),
    (
        "max allocation bigint multiply preallocation guard",
        "(1n << 1048574n) * 0n",
    ),
    (
        "max allocation bigint divide preallocation guard",
        "(1n << 1048574n) / 1n",
    ),
    (
        "max allocation bigint remainder preallocation guard",
        "(1n << 1048574n) % 1n",
    ),
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
    (
        "strict private name shift write",
        "(function named(){ 'use strict'; named <<= 1; })()",
    ),
    (
        "strict eval shift compound early error",
        "(function(){ 'use strict'; eval >>= 1; })",
    ),
    (
        "strict arguments shift compound early error",
        "(function(){ 'use strict'; (arguments) >>>= 1; })",
    ),
    (
        "strict private name exponent write",
        "(function named(){ 'use strict'; named **= 1; })()",
    ),
    (
        "strict eval exponent compound early error",
        "(function(){ 'use strict'; eval **= 1; })",
    ),
    (
        "strict arguments exponent compound early error",
        "(function(){ 'use strict'; (arguments) **= 1; })",
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
