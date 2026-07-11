use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{
    AccessorValue, CallableRef, Context, DescriptorField, JsString, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value, WellKnownSymbol,
};

const ORACLE_HELPERS: &str = r#"
function show(value) {
    if (value === undefined) return "undefined";
    if (typeof value === "number" || typeof value === "boolean") return String(value);
    if (typeof value === "bigint") return String(value) + "n";
    if (typeof value === "string") return "string:" + value;
    return "unexpected";
}
function observe(thunk) {
    try {
        return show(thunk());
    } catch (error) {
        if (typeof error === "string") return "throw-string:" + error;
        return "throw:" + error.name + "|" + error.message;
    }
}
function bit(value) { return value ? 1 : 0; }
"#;

const NUMERIC_PROBE: &str = r#"
let numericHint = "none";
const numericObject = {
    [Symbol.toPrimitive](hint) { numericHint = hint; return 6; }
};
const leftNumericSymbol = Symbol("left");
const rightNumericSymbol = Symbol("right");
console.log("unary=" + observe(() => +numericObject) + "," + observe(() => -numericObject));
console.log("binary=" + observe(() => numericObject - 2) + "," +
            observe(() => numericObject * 3) + "," +
            observe(() => numericObject / 2) + "," +
            observe(() => numericObject % 4));
let rightValueCalls = 0;
const rightValue = {
    [Symbol.toPrimitive]() { rightValueCalls++; return 1; }
};
console.log("symbol-left-right-value=" +
            observe(() => leftNumericSymbol - rightValue) + "," +
            observe(() => leftNumericSymbol * rightValue) + "," +
            observe(() => leftNumericSymbol / rightValue) + "," +
            observe(() => leftNumericSymbol % rightValue) +
            "|calls:" + rightValueCalls);
let rightThrowCalls = 0;
const rightThrow = {
    [Symbol.toPrimitive]() { rightThrowCalls++; throw "right"; }
};
console.log("symbol-left-right-throw=" +
            observe(() => leftNumericSymbol - rightThrow) + "," +
            observe(() => leftNumericSymbol * rightThrow) + "," +
            observe(() => leftNumericSymbol / rightThrow) + "," +
            observe(() => leftNumericSymbol % rightThrow) +
            "|calls:" + rightThrowCalls);
console.log("bigint-symbol-add=" + observe(() => 1n + rightNumericSymbol) + "," +
            observe(() => leftNumericSymbol + 1n));
console.log("bigint-symbol-sub=" + observe(() => 1n - rightNumericSymbol) + "," +
            observe(() => leftNumericSymbol - 1n));
console.log("relational=" + observe(() => numericObject < 7) + "," +
            observe(() => numericObject <= 6) + "," +
            observe(() => numericObject > 5) + "," +
            observe(() => numericObject >= 6));
console.log("numeric-hint=" + numericHint);
console.log("bigint-string-fraction=" + observe(() => 1n < "1.5") + "," +
            observe(() => "1.5" > 1n));
console.log("bigint-string-large=" +
            observe(() => 9007199254740992n < "9007199254740993") + "," +
            observe(() => "9007199254740993" > 9007199254740992n));
console.log("bigint-invalid-right=" + observe(() => 1n < "invalid") + "," +
            observe(() => 1n <= "invalid") + "," +
            observe(() => 1n > "invalid") + "," +
            observe(() => 1n >= "invalid"));
console.log("bigint-invalid-left=" + observe(() => "invalid" < 1n) + "," +
            observe(() => "invalid" <= 1n) + "," +
            observe(() => "invalid" > 1n) + "," +
            observe(() => "invalid" >= 1n));
"#;

const ADD_PROBE: &str = r#"
let addHint = "none";
const stringObject = {
    [Symbol.toPrimitive](hint) { addHint = hint; return "x"; }
};
console.log("add-string=" + observe(() => stringObject + 2) + "|hint:" + addHint);
const bigintObject = { [Symbol.toPrimitive]() { return 7n; } };
console.log("add-mixed=" + observe(() => bigintObject + 1));
console.log("function-plus=" + observe(() => +(function () {})));
"#;

const BITWISE_COERCION_PROBE: &str = r#"
let bitwiseHints = "";
let bitwiseHintCalls = 0;
const bitwiseNumber = {
    [Symbol.toPrimitive](hint) {
        bitwiseHints = bitwiseHints + hint + ",";
        bitwiseHintCalls = bitwiseHintCalls + 1;
        return 6;
    }
};
console.log("bitwise-number=" + observe(() => ~bitwiseNumber) + "," +
            observe(() => bitwiseNumber & 3) + "," +
            observe(() => bitwiseNumber ^ 3) + "," +
            observe(() => bitwiseNumber | 3) +
            "|hints:" + bitwiseHints + "|calls:" + bitwiseHintCalls);

let bitwiseOrder = 0;
const bitwiseLeft = {
    [Symbol.toPrimitive]() { bitwiseOrder = bitwiseOrder * 10 + 1; return 5; }
};
const bitwiseRight = {
    [Symbol.toPrimitive]() { bitwiseOrder = bitwiseOrder * 10 + 2; return 3; }
};
let orderedAnd = observe(() => bitwiseLeft & bitwiseRight) + "@" + bitwiseOrder;
bitwiseOrder = 0;
let orderedXor = observe(() => bitwiseLeft ^ bitwiseRight) + "@" + bitwiseOrder;
bitwiseOrder = 0;
let orderedOr = observe(() => bitwiseLeft | bitwiseRight) + "@" + bitwiseOrder;
console.log("bitwise-order=" + orderedAnd + "," + orderedXor + "," + orderedOr);

let mixedOrder = 0;
const mixedBigIntLeft = {
    [Symbol.toPrimitive]() { mixedOrder = mixedOrder * 10 + 1; return 1n; }
};
const mixedNumberRight = {
    [Symbol.toPrimitive]() { mixedOrder = mixedOrder * 10 + 2; return 1; }
};
const mixedNumberLeft = {
    [Symbol.toPrimitive]() { mixedOrder = mixedOrder * 10 + 1; return 1; }
};
const mixedBigIntRight = {
    [Symbol.toPrimitive]() { mixedOrder = mixedOrder * 10 + 2; return 1n; }
};
let mixedBigIntNumberAnd = observe(() => mixedBigIntLeft & mixedNumberRight) + "@" + mixedOrder;
mixedOrder = 0;
let mixedBigIntNumberXor = observe(() => mixedBigIntLeft ^ mixedNumberRight) + "@" + mixedOrder;
mixedOrder = 0;
let mixedBigIntNumberOr = observe(() => mixedBigIntLeft | mixedNumberRight) + "@" + mixedOrder;
mixedOrder = 0;
let mixedNumberBigIntAnd = observe(() => mixedNumberLeft & mixedBigIntRight) + "@" + mixedOrder;
mixedOrder = 0;
let mixedNumberBigIntXor = observe(() => mixedNumberLeft ^ mixedBigIntRight) + "@" + mixedOrder;
mixedOrder = 0;
let mixedNumberBigIntOr = observe(() => mixedNumberLeft | mixedBigIntRight) + "@" + mixedOrder;
console.log("bitwise-mixed=" +
            mixedBigIntNumberAnd + "," + mixedBigIntNumberXor + "," + mixedBigIntNumberOr + "," +
            mixedNumberBigIntAnd + "," + mixedNumberBigIntXor + "," + mixedNumberBigIntOr);

const leftBitwiseSymbol = Symbol("left-bitwise");
let symbolRightCalls = 0;
const symbolRight = {
    [Symbol.toPrimitive]() { symbolRightCalls = symbolRightCalls + 1; return 1; }
};
console.log("bitwise-symbol-left=" +
            observe(() => leftBitwiseSymbol & symbolRight) + "," +
            observe(() => leftBitwiseSymbol ^ symbolRight) + "," +
            observe(() => leftBitwiseSymbol | symbolRight) +
            "|right-calls:" + symbolRightCalls);

const bitwiseSentinel = {};
let leftThrowCalls = 0;
let leftThrowRightCalls = 0;
const leftThrow = {
    [Symbol.toPrimitive]() { leftThrowCalls = leftThrowCalls + 1; throw bitwiseSentinel; }
};
const afterLeftThrow = {
    [Symbol.toPrimitive]() { leftThrowRightCalls = leftThrowRightCalls + 1; return 1; }
};
function bitwiseThrownSame(thunk) {
    try { thunk(); return 0; }
    catch (error) { return error === bitwiseSentinel ? 1 : 0; }
}
console.log("bitwise-left-throw=" +
            bitwiseThrownSame(() => ~leftThrow) + "," +
            bitwiseThrownSame(() => leftThrow & afterLeftThrow) + "," +
            bitwiseThrownSame(() => leftThrow ^ afterLeftThrow) + "," +
            bitwiseThrownSame(() => leftThrow | afterLeftThrow) +
            "|left-calls:" + leftThrowCalls + "|right-calls:" + leftThrowRightCalls);

let rightThrowLeftCalls = 0;
let rightThrowCalls = 0;
const beforeRightThrow = {
    [Symbol.toPrimitive]() { rightThrowLeftCalls = rightThrowLeftCalls + 1; return 1; }
};
const rightThrowBitwise = {
    [Symbol.toPrimitive]() { rightThrowCalls = rightThrowCalls + 1; throw bitwiseSentinel; }
};
console.log("bitwise-right-throw=" +
            bitwiseThrownSame(() => beforeRightThrow & rightThrowBitwise) + "," +
            bitwiseThrownSame(() => beforeRightThrow ^ rightThrowBitwise) + "," +
            bitwiseThrownSame(() => beforeRightThrow | rightThrowBitwise) +
            "|left-calls:" + rightThrowLeftCalls + "|right-calls:" + rightThrowCalls);
"#;

const SHIFT_COERCION_PROBE: &str = r#"
let shiftHints = "";
let shiftHintCalls = 0;
const shiftNumber = {
    [Symbol.toPrimitive](hint) {
        shiftHints = shiftHints + hint + ",";
        shiftHintCalls = shiftHintCalls + 1;
        return -1;
    }
};
console.log("shift-number=" + observe(() => shiftNumber << 1) + "," +
            observe(() => shiftNumber >> 1) + "," +
            observe(() => shiftNumber >>> 0) +
            "|hints:" + shiftHints + "|calls:" + shiftHintCalls);
console.log("shift-unsigned-result=" + observe(() => -1 >>> 0) + "," +
            observe(() => -2147483648 >>> 0) + "," +
            observe(() => -1 >>> 1));

let shiftOrder = "";
const shiftOrderedLeft = {
    [Symbol.toPrimitive]() { shiftOrder = shiftOrder + "l"; return 16; }
};
const shiftOrderedRight = {
    [Symbol.toPrimitive]() { shiftOrder = shiftOrder + "r"; return 1; }
};
function shiftEvalLeft() { shiftOrder = shiftOrder + "L"; return shiftOrderedLeft; }
function shiftEvalRight() { shiftOrder = shiftOrder + "R"; return shiftOrderedRight; }
let orderedShl = observe(() => shiftEvalLeft() << shiftEvalRight()) + "@" + shiftOrder;
shiftOrder = "";
let orderedSar = observe(() => shiftEvalLeft() >> shiftEvalRight()) + "@" + shiftOrder;
shiftOrder = "";
let orderedShr = observe(() => shiftEvalLeft() >>> shiftEvalRight()) + "@" + shiftOrder;
console.log("shift-order=" + orderedShl + "," + orderedSar + "," + orderedShr);

const shiftSentinel = {};
function shiftThrownSame(thunk) {
    try { thunk(); return 0; }
    catch (error) { return error === shiftSentinel ? 1 : 0; }
}
let shiftLeftThrowCalls = 0;
let shiftAfterLeftThrowCalls = 0;
const shiftLeftThrow = {
    [Symbol.toPrimitive]() {
        shiftLeftThrowCalls = shiftLeftThrowCalls + 1;
        throw shiftSentinel;
    }
};
const shiftAfterLeftThrow = {
    [Symbol.toPrimitive]() {
        shiftAfterLeftThrowCalls = shiftAfterLeftThrowCalls + 1;
        return 1;
    }
};
console.log("shift-left-throw=" +
            shiftThrownSame(() => shiftLeftThrow << shiftAfterLeftThrow) + "," +
            shiftThrownSame(() => shiftLeftThrow >> shiftAfterLeftThrow) + "," +
            shiftThrownSame(() => shiftLeftThrow >>> shiftAfterLeftThrow) +
            "|left-calls:" + shiftLeftThrowCalls +
            "|right-calls:" + shiftAfterLeftThrowCalls);

let shiftBeforeRightThrowCalls = 0;
let shiftRightThrowCalls = 0;
const shiftBeforeRightThrow = {
    [Symbol.toPrimitive]() {
        shiftBeforeRightThrowCalls = shiftBeforeRightThrowCalls + 1;
        return 1;
    }
};
const shiftRightThrow = {
    [Symbol.toPrimitive]() {
        shiftRightThrowCalls = shiftRightThrowCalls + 1;
        throw shiftSentinel;
    }
};
console.log("shift-right-throw=" +
            shiftThrownSame(() => shiftBeforeRightThrow << shiftRightThrow) + "," +
            shiftThrownSame(() => shiftBeforeRightThrow >> shiftRightThrow) + "," +
            shiftThrownSame(() => shiftBeforeRightThrow >>> shiftRightThrow) +
            "|left-calls:" + shiftBeforeRightThrowCalls +
            "|right-calls:" + shiftRightThrowCalls);

let shiftMixedOrder = 0;
const shiftBigIntLeft = {
    [Symbol.toPrimitive]() { shiftMixedOrder = shiftMixedOrder * 10 + 1; return 8n; }
};
const shiftNumberRight = {
    [Symbol.toPrimitive]() { shiftMixedOrder = shiftMixedOrder * 10 + 2; return 1; }
};
const shiftNumberLeft = {
    [Symbol.toPrimitive]() { shiftMixedOrder = shiftMixedOrder * 10 + 1; return 8; }
};
const shiftBigIntRight = {
    [Symbol.toPrimitive]() { shiftMixedOrder = shiftMixedOrder * 10 + 2; return 1n; }
};
let mixedBigIntNumberShl = observe(() => shiftBigIntLeft << shiftNumberRight) + "@" + shiftMixedOrder;
shiftMixedOrder = 0;
let mixedBigIntNumberSar = observe(() => shiftBigIntLeft >> shiftNumberRight) + "@" + shiftMixedOrder;
shiftMixedOrder = 0;
let mixedNumberBigIntShl = observe(() => shiftNumberLeft << shiftBigIntRight) + "@" + shiftMixedOrder;
shiftMixedOrder = 0;
let mixedNumberBigIntSar = observe(() => shiftNumberLeft >> shiftBigIntRight) + "@" + shiftMixedOrder;
console.log("shift-mixed=" + mixedBigIntNumberShl + "," + mixedBigIntNumberSar + "," +
            mixedNumberBigIntShl + "," + mixedNumberBigIntSar);

shiftMixedOrder = 0;
let unsignedBigIntNumber = observe(() => shiftBigIntLeft >>> shiftNumberRight) + "@" + shiftMixedOrder;
shiftMixedOrder = 0;
let unsignedNumberBigInt = observe(() => shiftNumberLeft >>> shiftBigIntRight) + "@" + shiftMixedOrder;
shiftMixedOrder = 0;
let unsignedBigIntBigInt = observe(() => shiftBigIntLeft >>> shiftBigIntRight) + "@" + shiftMixedOrder;
console.log("shift-unsigned-bigint=" + unsignedBigIntNumber + "," +
            unsignedNumberBigInt + "," + unsignedBigIntBigInt);

const shiftSymbol = Symbol("shift");
let shiftSymbolRightCalls = 0;
const shiftSymbolRight = {
    [Symbol.toPrimitive]() { shiftSymbolRightCalls = shiftSymbolRightCalls + 1; return 1n; }
};
console.log("shift-symbol-left=" + observe(() => shiftSymbol << shiftSymbolRight) + "," +
            observe(() => shiftSymbol >> shiftSymbolRight) + "," +
            observe(() => shiftSymbol >>> shiftSymbolRight) +
            "|right-calls:" + shiftSymbolRightCalls);
let shiftBeforeSymbolCalls = 0;
const shiftBeforeSymbol = {
    [Symbol.toPrimitive]() { shiftBeforeSymbolCalls = shiftBeforeSymbolCalls + 1; return 1n; }
};
console.log("shift-symbol-right=" + observe(() => shiftBeforeSymbol << shiftSymbol) + "," +
            observe(() => shiftBeforeSymbol >> shiftSymbol) + "," +
            observe(() => shiftBeforeSymbol >>> shiftSymbol) +
            "|left-calls:" + shiftBeforeSymbolCalls);
"#;

const EXPONENTIATION_COERCION_PROBE: &str = r#"
let powerHints = "";
let powerHintCalls = 0;
const powerNumber = {
    [Symbol.toPrimitive](hint) {
        powerHints = powerHints + hint + ",";
        powerHintCalls = powerHintCalls + 1;
        return 2;
    }
};
console.log("power-number=" + observe(() => powerNumber ** 3) +
            "|hints:" + powerHints + "|calls:" + powerHintCalls);

let powerOrder = "";
const powerOrderedLeft = {
    [Symbol.toPrimitive]() { powerOrder = powerOrder + "l"; return 2; }
};
const powerOrderedRight = {
    [Symbol.toPrimitive]() { powerOrder = powerOrder + "r"; return 3; }
};
function powerEvalLeft() { powerOrder = powerOrder + "L"; return powerOrderedLeft; }
function powerEvalRight() { powerOrder = powerOrder + "R"; return powerOrderedRight; }
console.log("power-order=" + observe(() => powerEvalLeft() ** powerEvalRight()) +
            "@" + powerOrder);

powerOrder = "";
const powerA = {
    [Symbol.toPrimitive]() { powerOrder = powerOrder + "a"; return 2; }
};
const powerB = {
    [Symbol.toPrimitive]() { powerOrder = powerOrder + "b"; return 3; }
};
const powerC = {
    [Symbol.toPrimitive]() { powerOrder = powerOrder + "c"; return 2; }
};
function powerEvalA() { powerOrder = powerOrder + "A"; return powerA; }
function powerEvalB() { powerOrder = powerOrder + "B"; return powerB; }
function powerEvalC() { powerOrder = powerOrder + "C"; return powerC; }
console.log("power-right-associative=" +
            observe(() => powerEvalA() ** powerEvalB() ** powerEvalC()) +
            "@" + powerOrder);

const powerSentinel = {};
function powerThrownSame(thunk) {
    try { thunk(); return 0; }
    catch (error) { return error === powerSentinel ? 1 : 0; }
}
let powerLeftThrowCalls = 0;
let powerAfterLeftThrowCalls = 0;
const powerLeftThrow = {
    [Symbol.toPrimitive]() {
        powerLeftThrowCalls = powerLeftThrowCalls + 1;
        throw powerSentinel;
    }
};
const powerAfterLeftThrow = {
    [Symbol.toPrimitive]() {
        powerAfterLeftThrowCalls = powerAfterLeftThrowCalls + 1;
        return 3;
    }
};
console.log("power-left-throw=" +
            powerThrownSame(() => powerLeftThrow ** powerAfterLeftThrow) +
            "|left-calls:" + powerLeftThrowCalls +
            "|right-calls:" + powerAfterLeftThrowCalls);

let powerBeforeRightThrowCalls = 0;
let powerRightThrowCalls = 0;
const powerBeforeRightThrow = {
    [Symbol.toPrimitive]() {
        powerBeforeRightThrowCalls = powerBeforeRightThrowCalls + 1;
        return 2;
    }
};
const powerRightThrow = {
    [Symbol.toPrimitive]() {
        powerRightThrowCalls = powerRightThrowCalls + 1;
        throw powerSentinel;
    }
};
console.log("power-right-throw=" +
            powerThrownSame(() => powerBeforeRightThrow ** powerRightThrow) +
            "|left-calls:" + powerBeforeRightThrowCalls +
            "|right-calls:" + powerRightThrowCalls);

let powerMixedOrder = 0;
const powerBigIntLeft = {
    [Symbol.toPrimitive]() { powerMixedOrder = powerMixedOrder * 10 + 1; return 2n; }
};
const powerNumberRight = {
    [Symbol.toPrimitive]() { powerMixedOrder = powerMixedOrder * 10 + 2; return 3; }
};
const powerNumberLeft = {
    [Symbol.toPrimitive]() { powerMixedOrder = powerMixedOrder * 10 + 1; return 2; }
};
const powerBigIntRight = {
    [Symbol.toPrimitive]() { powerMixedOrder = powerMixedOrder * 10 + 2; return 3n; }
};
let powerBigIntNumber = observe(() => powerBigIntLeft ** powerNumberRight) +
                        "@" + powerMixedOrder;
powerMixedOrder = 0;
let powerNumberBigInt = observe(() => powerNumberLeft ** powerBigIntRight) +
                        "@" + powerMixedOrder;
console.log("power-mixed=" + powerBigIntNumber + "," + powerNumberBigInt);

powerMixedOrder = 0;
const powerMixedThrowRight = {
    [Symbol.toPrimitive]() {
        powerMixedOrder = powerMixedOrder * 10 + 2;
        throw powerSentinel;
    }
};
let powerMixedThrow = powerThrownSame(() => powerBigIntLeft ** powerMixedThrowRight) +
                      "@" + powerMixedOrder;
powerMixedOrder = 0;
const powerMixedSymbolRight = {
    [Symbol.toPrimitive]() {
        powerMixedOrder = powerMixedOrder * 10 + 2;
        return Symbol("mixed-power");
    }
};
let powerMixedSymbol = observe(() => powerBigIntLeft ** powerMixedSymbolRight) +
                       "@" + powerMixedOrder;
console.log("power-mixed-priority=" + powerMixedThrow + "," + powerMixedSymbol);

const powerSymbol = Symbol("power");
let powerAfterSymbolCalls = 0;
const powerAfterSymbol = {
    [Symbol.toPrimitive]() {
        powerAfterSymbolCalls = powerAfterSymbolCalls + 1;
        throw powerSentinel;
    }
};
console.log("power-symbol-before-throw=" +
            observe(() => powerSymbol ** powerAfterSymbol) +
            "|right-calls:" + powerAfterSymbolCalls);
let powerBeforeSymbolCalls = 0;
const powerBeforeSymbol = {
    [Symbol.toPrimitive]() {
        powerBeforeSymbolCalls = powerBeforeSymbolCalls + 1;
        throw powerSentinel;
    }
};
console.log("power-throw-before-symbol=" +
            powerThrownSame(() => powerBeforeSymbol ** powerSymbol) +
            "|left-calls:" + powerBeforeSymbolCalls);
let powerConvertedBeforeSymbolCalls = 0;
const powerConvertedBeforeSymbol = {
    [Symbol.toPrimitive]() {
        powerConvertedBeforeSymbolCalls = powerConvertedBeforeSymbolCalls + 1;
        return 2;
    }
};
console.log("power-symbol-right=" +
            observe(() => powerConvertedBeforeSymbol ** powerSymbol) +
            "|left-calls:" + powerConvertedBeforeSymbolCalls);

console.log("power-number-special=" +
            observe(() => (0 / 0) ** 0) + "," +
            observe(() => (0 / 0) ** 1) + "," +
            observe(() => 1 ** (1 / 0)) + "," +
            observe(() => (-1) ** (-1 / 0)) + "," +
            observe(() => (-2) ** 0.5) + "," +
            observe(() => 1 / ((-0) ** 3)) + "," +
            observe(() => 1 / ((-0) ** 2)) + "," +
            observe(() => (-0) ** -3) + "," +
            observe(() => (-0) ** -2) + "," +
            observe(() => (-1 / 0) ** 3) + "," +
            observe(() => 1 / ((-1 / 0) ** -3)) + "," +
            observe(() => 2 ** 1024) + "," +
            observe(() => 2 ** -1074) + "," +
            observe(() => 2 ** -1075));
"#;

const EQUALITY_PROBE: &str = r#"
const symbolValue = Symbol("s");
function box(value) {
    return { [Symbol.toPrimitive]() { return value; } };
}
function equality(name, object, primitive) {
    console.log("eq-" + name + "=" +
        bit(object == primitive) + "," + bit(primitive == object) + "," +
        bit(object != primitive) + "," + bit(primitive != object));
}
equality("number", box(7), 7);
equality("string", box("x"), "x");
equality("bigint", box(7n), 7n);
equality("symbol", box(symbolValue), symbolValue);
equality("boolean", box(1), true);
"#;

const ORDER_AND_ERROR_PROBE: &str = r#"
let order = 0;
const left = { [Symbol.toPrimitive]() { order = order * 10 + 1; return 1; } };
const right = { [Symbol.toPrimitive]() { order = order * 10 + 2; return 2; } };
console.log("order-add=" + observe(() => left + right) + "|order:" + order);
order = 0;
console.log("order-relational=" + observe(() => left < right) + "|order:" + order);

order = 0;
const ordinary = {
    valueOf() { order = order * 10 + 1; return ordinary; },
    toString() { order = order * 10 + 2; return "5"; },
};
console.log("ordinary-number=" + observe(() => +ordinary) + "|order:" + order);
order = 0;
console.log("ordinary-add=" + observe(() => ordinary + 1) + "|order:" + order);

const sentinel = {};
const getterThrow = {};
Object.defineProperty(getterThrow, Symbol.toPrimitive, {
    get() { throw sentinel; },
    configurable: true,
});
let getterSame = false;
try { +getterThrow; } catch (error) { getterSame = error === sentinel; }
console.log("getter-throw=" + (getterSame ? "same" : "changed"));

const methodThrow = { [Symbol.toPrimitive]() { throw sentinel; } };
let methodSame = false;
try { +methodThrow; } catch (error) { methodSame = error === sentinel; }
console.log("method-throw=" + (methodSame ? "same" : "changed"));

const objectReturn = { [Symbol.toPrimitive]() { return {}; } };
console.log("object-return=" + observe(() => +objectReturn));
const noncallable = { [Symbol.toPrimitive]: 1 };
console.log("noncallable=" + observe(() => +noncallable));

const stackObject = {
    [Symbol.toPrimitive]: function convert() { throw new Error("coerce"); }
};
function frameNames(stack) {
    return stack.trim().split("\n").slice(0, 3).map(line => {
        const match = /^\s*at ([^(]+?)(?: \(|$)/.exec(line);
        return match ? match[1].trim() : "?";
    }).join(",");
}
try {
    (function outer() { return +stackObject; })();
} catch (error) {
    console.log("stack=" + frameNames(error.stack));
}
"#;

#[test]
fn vm_object_coercion_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP VM object-coercion differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let rust = [
        rust_numeric_observations(),
        rust_add_observations(),
        rust_bitwise_coercion_observations(),
        rust_shift_coercion_observations(),
        rust_exponentiation_coercion_observations(),
        rust_equality_observations(),
        rust_order_and_error_observations(),
    ]
    .concat();
    let oracle = [
        ("numeric object coercion", NUMERIC_PROBE),
        ("addition object coercion", ADD_PROBE),
        ("bitwise object coercion", BITWISE_COERCION_PROBE),
        ("shift object coercion", SHIFT_COERCION_PROBE),
        (
            "exponentiation object coercion",
            EXPONENTIATION_COERCION_PROBE,
        ),
        ("abstract equality object coercion", EQUALITY_PROBE),
        ("coercion order and errors", ORDER_AND_ERROR_PROBE),
    ]
    .into_iter()
    .flat_map(|(description, probe)| oracle_observations(&oracle, probe, description))
    .collect::<Vec<_>>();

    assert_eq!(rust, oracle);
}

struct Harness {
    runtime: Runtime,
    context: Context,
    to_primitive: PropertyKey,
}

impl Harness {
    fn new() -> Self {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let to_primitive =
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
        Self {
            runtime,
            context,
            to_primitive,
        }
    }

    fn function(&mut self, source: &str) -> CallableRef {
        function(&self.runtime, &mut self.context, source)
    }

    fn object_with_exotic(&mut self, method: Value) -> ObjectRef {
        let object = self.context.new_object().unwrap();
        define_data(
            &self.runtime,
            &mut self.context,
            &object,
            &self.to_primitive,
            method,
        );
        object
    }

    fn bind(&mut self, name: &str, value: Value) {
        define_global(&self.runtime, &mut self.context, name, value);
    }

    fn observe(&mut self, source: &str) -> String {
        observe_eval(&self.runtime, &mut self.context, source)
    }
}

fn rust_numeric_observations() -> Vec<String> {
    let mut harness = Harness::new();
    harness.bind(
        "numericHint",
        Value::String(JsString::try_from_utf8("none").unwrap()),
    );
    let method = harness.function("(function(hint){ numericHint = hint; return 6; })");
    let object = harness.object_with_exotic(Value::Object(method.as_object().clone()));
    harness.bind("numericObject", Value::Object(object));
    let left_symbol = harness
        .runtime
        .new_symbol(Some(JsString::try_from_utf8("left").unwrap()))
        .unwrap();
    let right_symbol = harness
        .runtime
        .new_symbol(Some(JsString::try_from_utf8("right").unwrap()))
        .unwrap();
    harness.bind("leftNumericSymbol", Value::Symbol(left_symbol));
    harness.bind("rightNumericSymbol", Value::Symbol(right_symbol));

    let unary_plus = harness.observe("+numericObject");
    let unary_neg = harness.observe("-numericObject");
    let sub = harness.observe("numericObject - 2");
    let mul = harness.observe("numericObject * 3");
    let div = harness.observe("numericObject / 2");
    let rem = harness.observe("numericObject % 4");

    harness.bind("rightValueCalls", Value::Int(0));
    let right_value_method =
        harness.function("(function(){ rightValueCalls = rightValueCalls + 1; return 1; })");
    let right_value =
        harness.object_with_exotic(Value::Object(right_value_method.as_object().clone()));
    harness.bind("rightValue", Value::Object(right_value));
    let symbol_left_right_value = ["-", "*", "/", "%"]
        .map(|operator| harness.observe(&format!("leftNumericSymbol {operator} rightValue")))
        .join(",");
    let right_value_calls =
        integer_global(&harness.runtime, &mut harness.context, "rightValueCalls");

    harness.bind("rightThrowCalls", Value::Int(0));
    let right_throw_method =
        harness.function("(function(){ rightThrowCalls = rightThrowCalls + 1; throw \"right\"; })");
    let right_throw =
        harness.object_with_exotic(Value::Object(right_throw_method.as_object().clone()));
    harness.bind("rightThrow", Value::Object(right_throw));
    let symbol_left_right_throw = ["-", "*", "/", "%"]
        .map(|operator| harness.observe(&format!("leftNumericSymbol {operator} rightThrow")))
        .join(",");
    let right_throw_calls =
        integer_global(&harness.runtime, &mut harness.context, "rightThrowCalls");

    let bigint_symbol_add = harness.observe("1n + rightNumericSymbol");
    let symbol_bigint_add = harness.observe("leftNumericSymbol + 1n");
    let bigint_symbol_sub = harness.observe("1n - rightNumericSymbol");
    let symbol_bigint_sub = harness.observe("leftNumericSymbol - 1n");
    let lt = harness.observe("numericObject < 7");
    let lte = harness.observe("numericObject <= 6");
    let gt = harness.observe("numericObject > 5");
    let gte = harness.observe("numericObject >= 6");
    let Value::String(hint) = global_value(&harness.runtime, &mut harness.context, "numericHint")
    else {
        panic!("numeric hint was not a string");
    };
    let bigint_fraction = harness.observe("1n < \"1.5\"");
    let fraction_bigint = harness.observe("\"1.5\" > 1n");
    let bigint_large = harness.observe("9007199254740992n < \"9007199254740993\"");
    let large_bigint = harness.observe("\"9007199254740993\" > 9007199254740992n");
    let invalid_right = ["<", "<=", ">", ">="]
        .map(|operator| harness.observe(&format!("1n {operator} \"invalid\"")))
        .join(",");
    let invalid_left = ["<", "<=", ">", ">="]
        .map(|operator| harness.observe(&format!("\"invalid\" {operator} 1n")))
        .join(",");

    vec![
        format!("unary={unary_plus},{unary_neg}"),
        format!("binary={sub},{mul},{div},{rem}"),
        format!("symbol-left-right-value={symbol_left_right_value}|calls:{right_value_calls}"),
        format!("symbol-left-right-throw={symbol_left_right_throw}|calls:{right_throw_calls}"),
        format!("bigint-symbol-add={bigint_symbol_add},{symbol_bigint_add}"),
        format!("bigint-symbol-sub={bigint_symbol_sub},{symbol_bigint_sub}"),
        format!("relational={lt},{lte},{gt},{gte}"),
        format!("numeric-hint={}", hint.to_utf8_lossy()),
        format!("bigint-string-fraction={bigint_fraction},{fraction_bigint}"),
        format!("bigint-string-large={bigint_large},{large_bigint}"),
        format!("bigint-invalid-right={invalid_right}"),
        format!("bigint-invalid-left={invalid_left}"),
    ]
}

fn rust_add_observations() -> Vec<String> {
    let mut harness = Harness::new();
    harness.bind(
        "addHint",
        Value::String(JsString::try_from_utf8("none").unwrap()),
    );
    let string_method = harness.function("(function(hint){ addHint = hint; return \"x\"; })");
    let string_object =
        harness.object_with_exotic(Value::Object(string_method.as_object().clone()));
    harness.bind("stringObject", Value::Object(string_object));
    let add_string = harness.observe("stringObject + 2");
    let Value::String(hint) = global_value(&harness.runtime, &mut harness.context, "addHint")
    else {
        panic!("addition hint was not a string");
    };

    let bigint_method = harness.function("(function(){ return 7n; })");
    let bigint_object =
        harness.object_with_exotic(Value::Object(bigint_method.as_object().clone()));
    harness.bind("bigintObject", Value::Object(bigint_object));
    let mixed = harness.observe("bigintObject + 1");
    let function_plus = harness.observe("+(function(){})");

    vec![
        format!("add-string={add_string}|hint:{}", hint.to_utf8_lossy()),
        format!("add-mixed={mixed}"),
        format!("function-plus={function_plus}"),
    ]
}

fn rust_bitwise_coercion_observations() -> Vec<String> {
    let mut harness = Harness::new();

    harness.bind(
        "bitwiseHints",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    harness.bind("bitwiseHintCalls", Value::Int(0));
    let bitwise_number_method = harness.function(
        "(function(hint){ bitwiseHints = bitwiseHints + hint + \",\"; \
         bitwiseHintCalls = bitwiseHintCalls + 1; return 6; })",
    );
    let bitwise_number =
        harness.object_with_exotic(Value::Object(bitwise_number_method.as_object().clone()));
    harness.bind("bitwiseNumber", Value::Object(bitwise_number));
    let unary_not = harness.observe("~bitwiseNumber");
    let bitwise_and = harness.observe("bitwiseNumber & 3");
    let bitwise_xor = harness.observe("bitwiseNumber ^ 3");
    let bitwise_or = harness.observe("bitwiseNumber | 3");
    let Value::String(bitwise_hints) =
        global_value(&harness.runtime, &mut harness.context, "bitwiseHints")
    else {
        panic!("bitwise hints marker was not a string");
    };
    let bitwise_hint_calls =
        integer_global(&harness.runtime, &mut harness.context, "bitwiseHintCalls");

    harness.bind("bitwiseOrder", Value::Int(0));
    let bitwise_left_method =
        harness.function("(function(){ bitwiseOrder = bitwiseOrder * 10 + 1; return 5; })");
    let bitwise_right_method =
        harness.function("(function(){ bitwiseOrder = bitwiseOrder * 10 + 2; return 3; })");
    let bitwise_left =
        harness.object_with_exotic(Value::Object(bitwise_left_method.as_object().clone()));
    let bitwise_right =
        harness.object_with_exotic(Value::Object(bitwise_right_method.as_object().clone()));
    harness.bind("bitwiseLeft", Value::Object(bitwise_left));
    harness.bind("bitwiseRight", Value::Object(bitwise_right));
    let mut ordered = Vec::new();
    for operator in ["&", "^", "|"] {
        set_global(
            &harness.runtime,
            &mut harness.context,
            "bitwiseOrder",
            Value::Int(0),
        );
        let value = harness.observe(&format!("bitwiseLeft {operator} bitwiseRight"));
        let order = integer_global(&harness.runtime, &mut harness.context, "bitwiseOrder");
        ordered.push(format!("{value}@{order}"));
    }

    harness.bind("mixedOrder", Value::Int(0));
    let mixed_bigint_left_method =
        harness.function("(function(){ mixedOrder = mixedOrder * 10 + 1; return 1n; })");
    let mixed_number_right_method =
        harness.function("(function(){ mixedOrder = mixedOrder * 10 + 2; return 1; })");
    let mixed_number_left_method =
        harness.function("(function(){ mixedOrder = mixedOrder * 10 + 1; return 1; })");
    let mixed_bigint_right_method =
        harness.function("(function(){ mixedOrder = mixedOrder * 10 + 2; return 1n; })");
    let mixed_bigint_left =
        harness.object_with_exotic(Value::Object(mixed_bigint_left_method.as_object().clone()));
    let mixed_number_right =
        harness.object_with_exotic(Value::Object(mixed_number_right_method.as_object().clone()));
    let mixed_number_left =
        harness.object_with_exotic(Value::Object(mixed_number_left_method.as_object().clone()));
    let mixed_bigint_right =
        harness.object_with_exotic(Value::Object(mixed_bigint_right_method.as_object().clone()));
    harness.bind("mixedBigIntLeft", Value::Object(mixed_bigint_left));
    harness.bind("mixedNumberRight", Value::Object(mixed_number_right));
    harness.bind("mixedNumberLeft", Value::Object(mixed_number_left));
    harness.bind("mixedBigIntRight", Value::Object(mixed_bigint_right));
    let mut mixed = Vec::new();
    for (left, right) in [
        ("mixedBigIntLeft", "mixedNumberRight"),
        ("mixedNumberLeft", "mixedBigIntRight"),
    ] {
        for operator in ["&", "^", "|"] {
            set_global(
                &harness.runtime,
                &mut harness.context,
                "mixedOrder",
                Value::Int(0),
            );
            let value = harness.observe(&format!("{left} {operator} {right}"));
            let order = integer_global(&harness.runtime, &mut harness.context, "mixedOrder");
            mixed.push(format!("{value}@{order}"));
        }
    }

    let left_bitwise_symbol = harness
        .runtime
        .new_symbol(Some(JsString::try_from_utf8("left-bitwise").unwrap()))
        .unwrap();
    harness.bind("leftBitwiseSymbol", Value::Symbol(left_bitwise_symbol));
    harness.bind("symbolRightCalls", Value::Int(0));
    let symbol_right_method =
        harness.function("(function(){ symbolRightCalls = symbolRightCalls + 1; return 1; })");
    let symbol_right =
        harness.object_with_exotic(Value::Object(symbol_right_method.as_object().clone()));
    harness.bind("symbolRight", Value::Object(symbol_right));
    let symbol_left = ["&", "^", "|"]
        .map(|operator| harness.observe(&format!("leftBitwiseSymbol {operator} symbolRight")))
        .join(",");
    let symbol_right_calls =
        integer_global(&harness.runtime, &mut harness.context, "symbolRightCalls");

    let bitwise_sentinel = harness.context.new_object().unwrap();
    harness.bind("bitwiseSentinel", Value::Object(bitwise_sentinel.clone()));
    harness.bind("leftThrowCalls", Value::Int(0));
    harness.bind("leftThrowRightCalls", Value::Int(0));
    let left_throw_method = harness
        .function("(function(){ leftThrowCalls = leftThrowCalls + 1; throw bitwiseSentinel; })");
    let after_left_throw_method = harness
        .function("(function(){ leftThrowRightCalls = leftThrowRightCalls + 1; return 1; })");
    let left_throw =
        harness.object_with_exotic(Value::Object(left_throw_method.as_object().clone()));
    let after_left_throw =
        harness.object_with_exotic(Value::Object(after_left_throw_method.as_object().clone()));
    harness.bind("leftThrow", Value::Object(left_throw));
    harness.bind("afterLeftThrow", Value::Object(after_left_throw));
    let mut left_throw_same = vec![eval_thrown_identity(
        &harness.runtime,
        &mut harness.context,
        "~leftThrow",
        &bitwise_sentinel,
    )];
    left_throw_same.extend(["&", "^", "|"].map(|operator| {
        eval_thrown_identity(
            &harness.runtime,
            &mut harness.context,
            &format!("leftThrow {operator} afterLeftThrow"),
            &bitwise_sentinel,
        )
    }));
    let left_throw_same = left_throw_same
        .into_iter()
        .map(|same| if same { "1" } else { "0" })
        .collect::<Vec<_>>()
        .join(",");
    let left_throw_calls = integer_global(&harness.runtime, &mut harness.context, "leftThrowCalls");
    let left_throw_right_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "leftThrowRightCalls",
    );

    harness.bind("rightThrowLeftCalls", Value::Int(0));
    harness.bind("rightThrowCalls", Value::Int(0));
    let before_right_throw_method = harness
        .function("(function(){ rightThrowLeftCalls = rightThrowLeftCalls + 1; return 1; })");
    let right_throw_method = harness
        .function("(function(){ rightThrowCalls = rightThrowCalls + 1; throw bitwiseSentinel; })");
    let before_right_throw =
        harness.object_with_exotic(Value::Object(before_right_throw_method.as_object().clone()));
    let right_throw =
        harness.object_with_exotic(Value::Object(right_throw_method.as_object().clone()));
    harness.bind("beforeRightThrow", Value::Object(before_right_throw));
    harness.bind("rightThrowBitwise", Value::Object(right_throw));
    let right_throw_same = ["&", "^", "|"]
        .map(|operator| {
            eval_thrown_identity(
                &harness.runtime,
                &mut harness.context,
                &format!("beforeRightThrow {operator} rightThrowBitwise"),
                &bitwise_sentinel,
            )
        })
        .map(|same| if same { "1" } else { "0" })
        .join(",");
    let right_throw_left_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "rightThrowLeftCalls",
    );
    let right_throw_calls =
        integer_global(&harness.runtime, &mut harness.context, "rightThrowCalls");

    vec![
        format!(
            "bitwise-number={unary_not},{bitwise_and},{bitwise_xor},{bitwise_or}|hints:{}|calls:{bitwise_hint_calls}",
            bitwise_hints.to_utf8_lossy()
        ),
        format!("bitwise-order={}", ordered.join(",")),
        format!("bitwise-mixed={}", mixed.join(",")),
        format!("bitwise-symbol-left={symbol_left}|right-calls:{symbol_right_calls}"),
        format!(
            "bitwise-left-throw={left_throw_same}|left-calls:{left_throw_calls}|right-calls:{left_throw_right_calls}"
        ),
        format!(
            "bitwise-right-throw={right_throw_same}|left-calls:{right_throw_left_calls}|right-calls:{right_throw_calls}"
        ),
    ]
}

fn rust_shift_coercion_observations() -> Vec<String> {
    let mut harness = Harness::new();

    harness.bind(
        "shiftHints",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    harness.bind("shiftHintCalls", Value::Int(0));
    let shift_number_method = harness.function(
        "(function(hint){ shiftHints = shiftHints + hint + \",\"; \
         shiftHintCalls = shiftHintCalls + 1; return -1; })",
    );
    let shift_number =
        harness.object_with_exotic(Value::Object(shift_number_method.as_object().clone()));
    harness.bind("shiftNumber", Value::Object(shift_number));
    let shift_shl = harness.observe("shiftNumber << 1");
    let shift_sar = harness.observe("shiftNumber >> 1");
    let shift_shr = harness.observe("shiftNumber >>> 0");
    let shift_hints = string_global(&harness.runtime, &mut harness.context, "shiftHints");
    let shift_hint_calls = integer_global(&harness.runtime, &mut harness.context, "shiftHintCalls");
    let unsigned_results = ["-1 >>> 0", "-2147483648 >>> 0", "-1 >>> 1"]
        .map(|source| harness.observe(source))
        .join(",");

    harness.bind(
        "shiftOrder",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let ordered_left_method =
        harness.function("(function(){ shiftOrder = shiftOrder + \"l\"; return 16; })");
    let ordered_right_method =
        harness.function("(function(){ shiftOrder = shiftOrder + \"r\"; return 1; })");
    let ordered_left =
        harness.object_with_exotic(Value::Object(ordered_left_method.as_object().clone()));
    let ordered_right =
        harness.object_with_exotic(Value::Object(ordered_right_method.as_object().clone()));
    harness.bind("shiftOrderedLeft", Value::Object(ordered_left));
    harness.bind("shiftOrderedRight", Value::Object(ordered_right));
    let eval_left = harness
        .function("(function(){ shiftOrder = shiftOrder + \"L\"; return shiftOrderedLeft; })");
    let eval_right = harness
        .function("(function(){ shiftOrder = shiftOrder + \"R\"; return shiftOrderedRight; })");
    harness.bind(
        "shiftEvalLeft",
        Value::Object(eval_left.as_object().clone()),
    );
    harness.bind(
        "shiftEvalRight",
        Value::Object(eval_right.as_object().clone()),
    );
    let mut ordered = Vec::new();
    for operator in ["<<", ">>", ">>>"] {
        set_global(
            &harness.runtime,
            &mut harness.context,
            "shiftOrder",
            Value::String(JsString::try_from_utf8("").unwrap()),
        );
        let value = harness.observe(&format!("shiftEvalLeft() {operator} shiftEvalRight()"));
        let order = string_global(&harness.runtime, &mut harness.context, "shiftOrder");
        ordered.push(format!("{value}@{}", order.to_utf8_lossy()));
    }

    let sentinel = harness.context.new_object().unwrap();
    harness.bind("shiftSentinel", Value::Object(sentinel.clone()));
    harness.bind("shiftLeftThrowCalls", Value::Int(0));
    harness.bind("shiftAfterLeftThrowCalls", Value::Int(0));
    let left_throw_method = harness.function(
        "(function(){ shiftLeftThrowCalls = shiftLeftThrowCalls + 1; throw shiftSentinel; })",
    );
    let after_left_throw_method = harness.function(
        "(function(){ shiftAfterLeftThrowCalls = shiftAfterLeftThrowCalls + 1; return 1; })",
    );
    let left_throw =
        harness.object_with_exotic(Value::Object(left_throw_method.as_object().clone()));
    let after_left_throw =
        harness.object_with_exotic(Value::Object(after_left_throw_method.as_object().clone()));
    harness.bind("shiftLeftThrow", Value::Object(left_throw));
    harness.bind("shiftAfterLeftThrow", Value::Object(after_left_throw));
    let left_throw_same = ["<<", ">>", ">>>"]
        .map(|operator| {
            eval_thrown_identity(
                &harness.runtime,
                &mut harness.context,
                &format!("shiftLeftThrow {operator} shiftAfterLeftThrow"),
                &sentinel,
            )
        })
        .map(|same| if same { "1" } else { "0" })
        .join(",");
    let left_throw_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "shiftLeftThrowCalls",
    );
    let after_left_throw_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "shiftAfterLeftThrowCalls",
    );

    harness.bind("shiftBeforeRightThrowCalls", Value::Int(0));
    harness.bind("shiftRightThrowCalls", Value::Int(0));
    let before_right_throw_method = harness.function(
        "(function(){ shiftBeforeRightThrowCalls = shiftBeforeRightThrowCalls + 1; return 1; })",
    );
    let right_throw_method = harness.function(
        "(function(){ shiftRightThrowCalls = shiftRightThrowCalls + 1; throw shiftSentinel; })",
    );
    let before_right_throw =
        harness.object_with_exotic(Value::Object(before_right_throw_method.as_object().clone()));
    let right_throw =
        harness.object_with_exotic(Value::Object(right_throw_method.as_object().clone()));
    harness.bind("shiftBeforeRightThrow", Value::Object(before_right_throw));
    harness.bind("shiftRightThrow", Value::Object(right_throw));
    let right_throw_same = ["<<", ">>", ">>>"]
        .map(|operator| {
            eval_thrown_identity(
                &harness.runtime,
                &mut harness.context,
                &format!("shiftBeforeRightThrow {operator} shiftRightThrow"),
                &sentinel,
            )
        })
        .map(|same| if same { "1" } else { "0" })
        .join(",");
    let before_right_throw_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "shiftBeforeRightThrowCalls",
    );
    let right_throw_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "shiftRightThrowCalls",
    );

    harness.bind("shiftMixedOrder", Value::Int(0));
    let bigint_left_method =
        harness.function("(function(){ shiftMixedOrder = shiftMixedOrder * 10 + 1; return 8n; })");
    let number_right_method =
        harness.function("(function(){ shiftMixedOrder = shiftMixedOrder * 10 + 2; return 1; })");
    let number_left_method =
        harness.function("(function(){ shiftMixedOrder = shiftMixedOrder * 10 + 1; return 8; })");
    let bigint_right_method =
        harness.function("(function(){ shiftMixedOrder = shiftMixedOrder * 10 + 2; return 1n; })");
    let bigint_left =
        harness.object_with_exotic(Value::Object(bigint_left_method.as_object().clone()));
    let number_right =
        harness.object_with_exotic(Value::Object(number_right_method.as_object().clone()));
    let number_left =
        harness.object_with_exotic(Value::Object(number_left_method.as_object().clone()));
    let bigint_right =
        harness.object_with_exotic(Value::Object(bigint_right_method.as_object().clone()));
    harness.bind("shiftBigIntLeft", Value::Object(bigint_left));
    harness.bind("shiftNumberRight", Value::Object(number_right));
    harness.bind("shiftNumberLeft", Value::Object(number_left));
    harness.bind("shiftBigIntRight", Value::Object(bigint_right));
    let mut mixed = Vec::new();
    for (left, right) in [
        ("shiftBigIntLeft", "shiftNumberRight"),
        ("shiftNumberLeft", "shiftBigIntRight"),
    ] {
        for operator in ["<<", ">>"] {
            set_global(
                &harness.runtime,
                &mut harness.context,
                "shiftMixedOrder",
                Value::Int(0),
            );
            let value = harness.observe(&format!("{left} {operator} {right}"));
            let order = integer_global(&harness.runtime, &mut harness.context, "shiftMixedOrder");
            mixed.push(format!("{value}@{order}"));
        }
    }
    let mut unsigned_bigint = Vec::new();
    for (left, right) in [
        ("shiftBigIntLeft", "shiftNumberRight"),
        ("shiftNumberLeft", "shiftBigIntRight"),
        ("shiftBigIntLeft", "shiftBigIntRight"),
    ] {
        set_global(
            &harness.runtime,
            &mut harness.context,
            "shiftMixedOrder",
            Value::Int(0),
        );
        let value = harness.observe(&format!("{left} >>> {right}"));
        let order = integer_global(&harness.runtime, &mut harness.context, "shiftMixedOrder");
        unsigned_bigint.push(format!("{value}@{order}"));
    }

    let symbol = harness
        .runtime
        .new_symbol(Some(JsString::try_from_utf8("shift").unwrap()))
        .unwrap();
    harness.bind("shiftSymbol", Value::Symbol(symbol));
    harness.bind("shiftSymbolRightCalls", Value::Int(0));
    let symbol_right_method = harness
        .function("(function(){ shiftSymbolRightCalls = shiftSymbolRightCalls + 1; return 1n; })");
    let symbol_right =
        harness.object_with_exotic(Value::Object(symbol_right_method.as_object().clone()));
    harness.bind("shiftSymbolRight", Value::Object(symbol_right));
    let symbol_left = ["<<", ">>", ">>>"]
        .map(|operator| harness.observe(&format!("shiftSymbol {operator} shiftSymbolRight")))
        .join(",");
    let symbol_right_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "shiftSymbolRightCalls",
    );
    harness.bind("shiftBeforeSymbolCalls", Value::Int(0));
    let before_symbol_method = harness.function(
        "(function(){ shiftBeforeSymbolCalls = shiftBeforeSymbolCalls + 1; return 1n; })",
    );
    let before_symbol =
        harness.object_with_exotic(Value::Object(before_symbol_method.as_object().clone()));
    harness.bind("shiftBeforeSymbol", Value::Object(before_symbol));
    let symbol_right = ["<<", ">>", ">>>"]
        .map(|operator| harness.observe(&format!("shiftBeforeSymbol {operator} shiftSymbol")))
        .join(",");
    let before_symbol_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "shiftBeforeSymbolCalls",
    );

    vec![
        format!(
            "shift-number={shift_shl},{shift_sar},{shift_shr}|hints:{}|calls:{shift_hint_calls}",
            shift_hints.to_utf8_lossy()
        ),
        format!("shift-unsigned-result={unsigned_results}"),
        format!("shift-order={}", ordered.join(",")),
        format!(
            "shift-left-throw={left_throw_same}|left-calls:{left_throw_calls}|right-calls:{after_left_throw_calls}"
        ),
        format!(
            "shift-right-throw={right_throw_same}|left-calls:{before_right_throw_calls}|right-calls:{right_throw_calls}"
        ),
        format!("shift-mixed={}", mixed.join(",")),
        format!("shift-unsigned-bigint={}", unsigned_bigint.join(",")),
        format!("shift-symbol-left={symbol_left}|right-calls:{symbol_right_calls}"),
        format!("shift-symbol-right={symbol_right}|left-calls:{before_symbol_calls}"),
    ]
}

fn rust_exponentiation_coercion_observations() -> Vec<String> {
    let mut harness = Harness::new();

    harness.bind(
        "powerHints",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    harness.bind("powerHintCalls", Value::Int(0));
    let power_number_method = harness.function(
        "(function(hint){ powerHints = powerHints + hint + \",\"; \
         powerHintCalls = powerHintCalls + 1; return 2; })",
    );
    let power_number =
        harness.object_with_exotic(Value::Object(power_number_method.as_object().clone()));
    harness.bind("powerNumber", Value::Object(power_number));
    let power_number = harness.observe("powerNumber ** 3");
    let power_hints = string_global(&harness.runtime, &mut harness.context, "powerHints");
    let power_hint_calls = integer_global(&harness.runtime, &mut harness.context, "powerHintCalls");

    harness.bind(
        "powerOrder",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let ordered_left_method =
        harness.function("(function(){ powerOrder = powerOrder + \"l\"; return 2; })");
    let ordered_right_method =
        harness.function("(function(){ powerOrder = powerOrder + \"r\"; return 3; })");
    let ordered_left =
        harness.object_with_exotic(Value::Object(ordered_left_method.as_object().clone()));
    let ordered_right =
        harness.object_with_exotic(Value::Object(ordered_right_method.as_object().clone()));
    harness.bind("powerOrderedLeft", Value::Object(ordered_left));
    harness.bind("powerOrderedRight", Value::Object(ordered_right));
    let eval_left = harness
        .function("(function(){ powerOrder = powerOrder + \"L\"; return powerOrderedLeft; })");
    let eval_right = harness
        .function("(function(){ powerOrder = powerOrder + \"R\"; return powerOrderedRight; })");
    harness.bind(
        "powerEvalLeft",
        Value::Object(eval_left.as_object().clone()),
    );
    harness.bind(
        "powerEvalRight",
        Value::Object(eval_right.as_object().clone()),
    );
    let ordered = harness.observe("powerEvalLeft() ** powerEvalRight()");
    let ordered_log = string_global(&harness.runtime, &mut harness.context, "powerOrder");

    set_global(
        &harness.runtime,
        &mut harness.context,
        "powerOrder",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let power_a_method =
        harness.function("(function(){ powerOrder = powerOrder + \"a\"; return 2; })");
    let power_b_method =
        harness.function("(function(){ powerOrder = powerOrder + \"b\"; return 3; })");
    let power_c_method =
        harness.function("(function(){ powerOrder = powerOrder + \"c\"; return 2; })");
    let power_a = harness.object_with_exotic(Value::Object(power_a_method.as_object().clone()));
    let power_b = harness.object_with_exotic(Value::Object(power_b_method.as_object().clone()));
    let power_c = harness.object_with_exotic(Value::Object(power_c_method.as_object().clone()));
    harness.bind("powerA", Value::Object(power_a));
    harness.bind("powerB", Value::Object(power_b));
    harness.bind("powerC", Value::Object(power_c));
    let eval_a =
        harness.function("(function(){ powerOrder = powerOrder + \"A\"; return powerA; })");
    let eval_b =
        harness.function("(function(){ powerOrder = powerOrder + \"B\"; return powerB; })");
    let eval_c =
        harness.function("(function(){ powerOrder = powerOrder + \"C\"; return powerC; })");
    harness.bind("powerEvalA", Value::Object(eval_a.as_object().clone()));
    harness.bind("powerEvalB", Value::Object(eval_b.as_object().clone()));
    harness.bind("powerEvalC", Value::Object(eval_c.as_object().clone()));
    let right_associative = harness.observe("powerEvalA() ** powerEvalB() ** powerEvalC()");
    let right_associative_log = string_global(&harness.runtime, &mut harness.context, "powerOrder");

    let sentinel = harness.context.new_object().unwrap();
    harness.bind("powerSentinel", Value::Object(sentinel.clone()));
    harness.bind("powerLeftThrowCalls", Value::Int(0));
    harness.bind("powerAfterLeftThrowCalls", Value::Int(0));
    let left_throw_method = harness.function(
        "(function(){ powerLeftThrowCalls = powerLeftThrowCalls + 1; throw powerSentinel; })",
    );
    let after_left_throw_method = harness.function(
        "(function(){ powerAfterLeftThrowCalls = powerAfterLeftThrowCalls + 1; return 3; })",
    );
    let left_throw =
        harness.object_with_exotic(Value::Object(left_throw_method.as_object().clone()));
    let after_left_throw =
        harness.object_with_exotic(Value::Object(after_left_throw_method.as_object().clone()));
    harness.bind("powerLeftThrow", Value::Object(left_throw));
    harness.bind("powerAfterLeftThrow", Value::Object(after_left_throw));
    let left_throw_same = eval_thrown_identity(
        &harness.runtime,
        &mut harness.context,
        "powerLeftThrow ** powerAfterLeftThrow",
        &sentinel,
    );
    let left_throw_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "powerLeftThrowCalls",
    );
    let after_left_throw_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "powerAfterLeftThrowCalls",
    );

    harness.bind("powerBeforeRightThrowCalls", Value::Int(0));
    harness.bind("powerRightThrowCalls", Value::Int(0));
    let before_right_throw_method = harness.function(
        "(function(){ powerBeforeRightThrowCalls = powerBeforeRightThrowCalls + 1; return 2; })",
    );
    let right_throw_method = harness.function(
        "(function(){ powerRightThrowCalls = powerRightThrowCalls + 1; throw powerSentinel; })",
    );
    let before_right_throw =
        harness.object_with_exotic(Value::Object(before_right_throw_method.as_object().clone()));
    let right_throw =
        harness.object_with_exotic(Value::Object(right_throw_method.as_object().clone()));
    harness.bind("powerBeforeRightThrow", Value::Object(before_right_throw));
    harness.bind("powerRightThrow", Value::Object(right_throw));
    let right_throw_same = eval_thrown_identity(
        &harness.runtime,
        &mut harness.context,
        "powerBeforeRightThrow ** powerRightThrow",
        &sentinel,
    );
    let before_right_throw_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "powerBeforeRightThrowCalls",
    );
    let right_throw_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "powerRightThrowCalls",
    );

    harness.bind("powerMixedOrder", Value::Int(0));
    let bigint_left_method =
        harness.function("(function(){ powerMixedOrder = powerMixedOrder * 10 + 1; return 2n; })");
    let number_right_method =
        harness.function("(function(){ powerMixedOrder = powerMixedOrder * 10 + 2; return 3; })");
    let number_left_method =
        harness.function("(function(){ powerMixedOrder = powerMixedOrder * 10 + 1; return 2; })");
    let bigint_right_method =
        harness.function("(function(){ powerMixedOrder = powerMixedOrder * 10 + 2; return 3n; })");
    let bigint_left =
        harness.object_with_exotic(Value::Object(bigint_left_method.as_object().clone()));
    let number_right =
        harness.object_with_exotic(Value::Object(number_right_method.as_object().clone()));
    let number_left =
        harness.object_with_exotic(Value::Object(number_left_method.as_object().clone()));
    let bigint_right =
        harness.object_with_exotic(Value::Object(bigint_right_method.as_object().clone()));
    harness.bind("powerBigIntLeft", Value::Object(bigint_left));
    harness.bind("powerNumberRight", Value::Object(number_right));
    harness.bind("powerNumberLeft", Value::Object(number_left));
    harness.bind("powerBigIntRight", Value::Object(bigint_right));
    let mut mixed = Vec::new();
    for (left, right) in [
        ("powerBigIntLeft", "powerNumberRight"),
        ("powerNumberLeft", "powerBigIntRight"),
    ] {
        set_global(
            &harness.runtime,
            &mut harness.context,
            "powerMixedOrder",
            Value::Int(0),
        );
        let value = harness.observe(&format!("{left} ** {right}"));
        let order = integer_global(&harness.runtime, &mut harness.context, "powerMixedOrder");
        mixed.push(format!("{value}@{order}"));
    }

    set_global(
        &harness.runtime,
        &mut harness.context,
        "powerMixedOrder",
        Value::Int(0),
    );
    let mixed_throw_right_method = harness.function(
        "(function(){ powerMixedOrder = powerMixedOrder * 10 + 2; throw powerSentinel; })",
    );
    let mixed_throw_right =
        harness.object_with_exotic(Value::Object(mixed_throw_right_method.as_object().clone()));
    harness.bind("powerMixedThrowRight", Value::Object(mixed_throw_right));
    let mixed_throw_same = eval_thrown_identity(
        &harness.runtime,
        &mut harness.context,
        "powerBigIntLeft ** powerMixedThrowRight",
        &sentinel,
    );
    let mixed_throw_order =
        integer_global(&harness.runtime, &mut harness.context, "powerMixedOrder");
    set_global(
        &harness.runtime,
        &mut harness.context,
        "powerMixedOrder",
        Value::Int(0),
    );
    let mixed_symbol = harness
        .runtime
        .new_symbol(Some(JsString::try_from_utf8("mixed-power").unwrap()))
        .unwrap();
    harness.bind("powerMixedSymbol", Value::Symbol(mixed_symbol));
    let mixed_symbol_right_method = harness.function(
        "(function(){ powerMixedOrder = powerMixedOrder * 10 + 2; return powerMixedSymbol; })",
    );
    let mixed_symbol_right =
        harness.object_with_exotic(Value::Object(mixed_symbol_right_method.as_object().clone()));
    harness.bind("powerMixedSymbolRight", Value::Object(mixed_symbol_right));
    let mixed_symbol_result = harness.observe("powerBigIntLeft ** powerMixedSymbolRight");
    let mixed_symbol_order =
        integer_global(&harness.runtime, &mut harness.context, "powerMixedOrder");

    let symbol = harness
        .runtime
        .new_symbol(Some(JsString::try_from_utf8("power").unwrap()))
        .unwrap();
    harness.bind("powerSymbol", Value::Symbol(symbol));
    harness.bind("powerAfterSymbolCalls", Value::Int(0));
    let after_symbol_method = harness.function(
        "(function(){ powerAfterSymbolCalls = powerAfterSymbolCalls + 1; throw powerSentinel; })",
    );
    let after_symbol =
        harness.object_with_exotic(Value::Object(after_symbol_method.as_object().clone()));
    harness.bind("powerAfterSymbol", Value::Object(after_symbol));
    let symbol_before_throw = harness.observe("powerSymbol ** powerAfterSymbol");
    let after_symbol_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "powerAfterSymbolCalls",
    );

    harness.bind("powerBeforeSymbolCalls", Value::Int(0));
    let before_symbol_method = harness.function(
        "(function(){ powerBeforeSymbolCalls = powerBeforeSymbolCalls + 1; throw powerSentinel; })",
    );
    let before_symbol =
        harness.object_with_exotic(Value::Object(before_symbol_method.as_object().clone()));
    harness.bind("powerBeforeSymbol", Value::Object(before_symbol));
    let throw_before_symbol = eval_thrown_identity(
        &harness.runtime,
        &mut harness.context,
        "powerBeforeSymbol ** powerSymbol",
        &sentinel,
    );
    let before_symbol_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "powerBeforeSymbolCalls",
    );

    harness.bind("powerConvertedBeforeSymbolCalls", Value::Int(0));
    let converted_before_symbol_method = harness.function(
        "(function(){ powerConvertedBeforeSymbolCalls = \
         powerConvertedBeforeSymbolCalls + 1; return 2; })",
    );
    let converted_before_symbol = harness.object_with_exotic(Value::Object(
        converted_before_symbol_method.as_object().clone(),
    ));
    harness.bind(
        "powerConvertedBeforeSymbol",
        Value::Object(converted_before_symbol),
    );
    let symbol_right = harness.observe("powerConvertedBeforeSymbol ** powerSymbol");
    let converted_before_symbol_calls = integer_global(
        &harness.runtime,
        &mut harness.context,
        "powerConvertedBeforeSymbolCalls",
    );

    let number_special = [
        "(0 / 0) ** 0",
        "(0 / 0) ** 1",
        "1 ** (1 / 0)",
        "(-1) ** (-1 / 0)",
        "(-2) ** 0.5",
        "1 / ((-0) ** 3)",
        "1 / ((-0) ** 2)",
        "(-0) ** -3",
        "(-0) ** -2",
        "(-1 / 0) ** 3",
        "1 / ((-1 / 0) ** -3)",
        "2 ** 1024",
        "2 ** -1074",
        "2 ** -1075",
    ]
    .map(|source| harness.observe(source))
    .join(",");

    vec![
        format!(
            "power-number={power_number}|hints:{}|calls:{power_hint_calls}",
            power_hints.to_utf8_lossy()
        ),
        format!("power-order={ordered}@{}", ordered_log.to_utf8_lossy()),
        format!(
            "power-right-associative={right_associative}@{}",
            right_associative_log.to_utf8_lossy()
        ),
        format!(
            "power-left-throw={}|left-calls:{left_throw_calls}|right-calls:{after_left_throw_calls}",
            if left_throw_same { 1 } else { 0 }
        ),
        format!(
            "power-right-throw={}|left-calls:{before_right_throw_calls}|right-calls:{right_throw_calls}",
            if right_throw_same { 1 } else { 0 }
        ),
        format!("power-mixed={}", mixed.join(",")),
        format!(
            "power-mixed-priority={}@{mixed_throw_order},{mixed_symbol_result}@{mixed_symbol_order}",
            if mixed_throw_same { 1 } else { 0 }
        ),
        format!("power-symbol-before-throw={symbol_before_throw}|right-calls:{after_symbol_calls}"),
        format!(
            "power-throw-before-symbol={}|left-calls:{before_symbol_calls}",
            if throw_before_symbol { 1 } else { 0 }
        ),
        format!("power-symbol-right={symbol_right}|left-calls:{converted_before_symbol_calls}"),
        format!("power-number-special={number_special}"),
    ]
}

fn rust_equality_observations() -> Vec<String> {
    let mut harness = Harness::new();
    let symbol = harness
        .runtime
        .new_symbol(Some(JsString::try_from_utf8("s").unwrap()))
        .unwrap();
    harness.bind("symbolValue", Value::Symbol(symbol));

    let cases = [
        ("number", "7", "(function(){ return 7; })"),
        ("string", "\"x\"", "(function(){ return \"x\"; })"),
        ("bigint", "7n", "(function(){ return 7n; })"),
        (
            "symbol",
            "symbolValue",
            "(function(){ return symbolValue; })",
        ),
        ("boolean", "true", "(function(){ return 1; })"),
    ];

    cases
        .into_iter()
        .map(|(name, primitive, method_source)| {
            let method = harness.function(method_source);
            let object = harness.object_with_exotic(Value::Object(method.as_object().clone()));
            let object_name = format!("equalityObject{name}");
            harness.bind(&object_name, Value::Object(object));
            let eq_left = bool_bit(&harness.observe(&format!("{object_name} == {primitive}")));
            let eq_right = bool_bit(&harness.observe(&format!("{primitive} == {object_name}")));
            let neq_left = bool_bit(&harness.observe(&format!("{object_name} != {primitive}")));
            let neq_right = bool_bit(&harness.observe(&format!("{primitive} != {object_name}")));
            format!("eq-{name}={eq_left},{eq_right},{neq_left},{neq_right}")
        })
        .collect()
}

fn rust_order_and_error_observations() -> Vec<String> {
    let mut harness = Harness::new();
    harness.bind("order", Value::Int(0));
    let left_method = harness.function("(function(){ order = order * 10 + 1; return 1; })");
    let right_method = harness.function("(function(){ order = order * 10 + 2; return 2; })");
    let left = harness.object_with_exotic(Value::Object(left_method.as_object().clone()));
    let right = harness.object_with_exotic(Value::Object(right_method.as_object().clone()));
    harness.bind("left", Value::Object(left));
    harness.bind("right", Value::Object(right));
    let add = harness.observe("left + right");
    let add_order = integer_global(&harness.runtime, &mut harness.context, "order");
    set_global(
        &harness.runtime,
        &mut harness.context,
        "order",
        Value::Int(0),
    );
    let relational = harness.observe("left < right");
    let relational_order = integer_global(&harness.runtime, &mut harness.context, "order");

    set_global(
        &harness.runtime,
        &mut harness.context,
        "order",
        Value::Int(0),
    );
    let ordinary = harness.context.new_object().unwrap();
    harness.bind("ordinary", Value::Object(ordinary.clone()));
    let value_of = harness.function("(function(){ order = order * 10 + 1; return ordinary; })");
    let to_string = harness.function("(function(){ order = order * 10 + 2; return \"5\"; })");
    let value_of_key = harness.runtime.intern_property_key("valueOf").unwrap();
    let to_string_key = harness.runtime.intern_property_key("toString").unwrap();
    define_data(
        &harness.runtime,
        &mut harness.context,
        &ordinary,
        &value_of_key,
        Value::Object(value_of.as_object().clone()),
    );
    define_data(
        &harness.runtime,
        &mut harness.context,
        &ordinary,
        &to_string_key,
        Value::Object(to_string.as_object().clone()),
    );
    let ordinary_number = harness.observe("+ordinary");
    let ordinary_number_order = integer_global(&harness.runtime, &mut harness.context, "order");
    set_global(
        &harness.runtime,
        &mut harness.context,
        "order",
        Value::Int(0),
    );
    let ordinary_add = harness.observe("ordinary + 1");
    let ordinary_add_order = integer_global(&harness.runtime, &mut harness.context, "order");

    let sentinel = harness.context.new_object().unwrap();
    harness.bind("sentinel", Value::Object(sentinel.clone()));
    let getter = harness.function("(function(){ throw sentinel; })");
    let getter_throw = harness.context.new_object().unwrap();
    assert!(
        harness
            .context
            .define_own_property(
                &getter_throw,
                &harness.to_primitive,
                &getter_descriptor(getter),
            )
            .unwrap()
    );
    harness.bind("getterThrow", Value::Object(getter_throw));
    let getter_same = eval_thrown_identity(
        &harness.runtime,
        &mut harness.context,
        "+getterThrow",
        &sentinel,
    );

    let method = harness.function("(function(){ throw sentinel; })");
    let method_throw = harness.object_with_exotic(Value::Object(method.as_object().clone()));
    harness.bind("methodThrow", Value::Object(method_throw));
    let method_same = eval_thrown_identity(
        &harness.runtime,
        &mut harness.context,
        "+methodThrow",
        &sentinel,
    );

    let returned_object = harness.context.new_object().unwrap();
    harness.bind("returnedObject", Value::Object(returned_object));
    let object_method = harness.function("(function(){ return returnedObject; })");
    let object_return =
        harness.object_with_exotic(Value::Object(object_method.as_object().clone()));
    harness.bind("objectReturn", Value::Object(object_return));
    let object_return = harness.observe("+objectReturn");

    let noncallable = harness.object_with_exotic(Value::Int(1));
    harness.bind("noncallable", Value::Object(noncallable));
    let noncallable = harness.observe("+noncallable");

    let convert = harness.function("(function convert(){ throw new Error(\"coerce\"); })");
    let stack_object = harness.object_with_exotic(Value::Object(convert.as_object().clone()));
    harness.bind("stackObject", Value::Object(stack_object));
    let stack = error_stack_frame_names(
        &harness.runtime,
        &mut harness.context,
        "(function outer(){ return +stackObject; })()",
    );

    vec![
        format!("order-add={add}|order:{add_order}"),
        format!("order-relational={relational}|order:{relational_order}"),
        format!("ordinary-number={ordinary_number}|order:{ordinary_number_order}"),
        format!("ordinary-add={ordinary_add}|order:{ordinary_add_order}"),
        format!(
            "getter-throw={}",
            if getter_same { "same" } else { "changed" }
        ),
        format!(
            "method-throw={}",
            if method_same { "same" } else { "changed" }
        ),
        format!("object-return={object_return}"),
        format!("noncallable={noncallable}"),
        format!("stack={stack}"),
    ]
}

fn function(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("function probe did not return an object: {source}");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("function probe was not callable: {source}"))
}

fn define_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    define_data(runtime, context, &global, &key, value);
}

fn set_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    assert!(context.set_property(&global, &key, value).unwrap());
}

fn global_value(runtime: &Runtime, context: &mut Context, name: &str) -> Value {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    context.get_property(&global, &key).unwrap()
}

fn integer_global(runtime: &Runtime, context: &mut Context, name: &str) -> i32 {
    let Value::Int(value) = global_value(runtime, context, name) else {
        panic!("global marker {name} was not an integer");
    };
    value
}

fn string_global(runtime: &Runtime, context: &mut Context, name: &str) -> JsString {
    let Value::String(value) = global_value(runtime, context, name) else {
        panic!("global marker {name} was not a string");
    };
    value
}

fn define_data(
    _runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    key: &PropertyKey,
    value: Value,
) {
    assert!(
        context
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn getter_descriptor(getter: CallableRef) -> OrdinaryPropertyDescriptor {
    OrdinaryPropertyDescriptor {
        get: DescriptorField::Present(AccessorValue::Callable(getter)),
        set: DescriptorField::Present(AccessorValue::Undefined),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    }
}

fn observe_eval(runtime: &Runtime, context: &mut Context, source: &str) -> String {
    match context.eval(source) {
        Ok(value) => show_value(value),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap()
                .expect("exception completion had no value");
            show_exception(runtime, context, exception)
        }
        Err(error) => panic!("eval probe failed with an engine error for {source:?}: {error}"),
    }
}

fn show_value(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => number_to_string(value),
        Value::BigInt(value) => format!("{value}n"),
        Value::String(value) => format!("string:{}", value.to_utf8_lossy()),
        value => panic!("unexpected probe return value: {value:?}"),
    }
}

fn show_exception(runtime: &Runtime, context: &mut Context, exception: Value) -> String {
    match exception {
        Value::String(value) => format!("throw-string:{}", value.to_utf8_lossy()),
        Value::Object(error) if runtime.is_error_object(&error).unwrap() => {
            let name = runtime.intern_property_key("name").unwrap();
            let message = runtime.intern_property_key("message").unwrap();
            let Value::String(name) = context.get_property(&error, &name).unwrap() else {
                panic!("error name was not a string");
            };
            let Value::String(message) = context.get_property(&error, &message).unwrap() else {
                panic!("error message was not a string");
            };
            format!("throw:{}|{}", name.to_utf8_lossy(), message.to_utf8_lossy())
        }
        value => panic!("unexpected thrown probe value: {value:?}"),
    }
}

fn bool_bit(value: &str) -> u8 {
    match value {
        "true" => 1,
        "false" => 0,
        value => panic!("expected boolean observation, got {value:?}"),
    }
}

fn eval_thrown_identity(
    _runtime: &Runtime,
    context: &mut Context,
    source: &str,
    expected: &ObjectRef,
) -> bool {
    assert_eq!(context.eval(source), Err(RuntimeError::Exception));
    matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(object)) if object == *expected
    )
}

fn error_stack_frame_names(runtime: &Runtime, context: &mut Context, source: &str) -> String {
    assert_eq!(context.eval(source), Err(RuntimeError::Exception));
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        panic!("stack coercion did not throw an object");
    };
    assert!(runtime.is_error_object(&error).unwrap());
    let stack_key = runtime.intern_property_key("stack").unwrap();
    let Value::String(stack) = context.get_property(&error, &stack_key).unwrap() else {
        panic!("coercion Error.stack was not a string");
    };
    stack
        .to_utf8_lossy()
        .lines()
        .take(3)
        .map(|line| {
            let line = line.trim().strip_prefix("at ").unwrap_or(line.trim());
            line.split_once(" (")
                .map_or(line, |(name, _)| name)
                .trim()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn oracle_observations(oracle: &OsStr, probe: &str, description: &str) -> Vec<String> {
    let source = format!("{ORACLE_HELPERS}\n{probe}");
    let output = Command::new(oracle)
        .args(["-e", &source])
        .output()
        .unwrap_or_else(|error| panic!("run QuickJS {description} oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS {description} oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS {description} emitted non-UTF-8 output: {error}"))
        .lines()
        .map(str::to_owned)
        .collect()
}
