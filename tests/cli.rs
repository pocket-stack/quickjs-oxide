use crate::runtime_oracle::run_cli;
#[path = "support/runtime_oracle.rs"]
mod runtime_oracle;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_MODULE_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct QjsHostStdoutCase {
    description: &'static str,
    options: &'static [&'static str],
    source: &'static str,
    expected_status: i32,
    expected_stdout: &'static [u8],
    expected_stderr: &'static [u8],
}

const QJS_ROPE_SPECIAL_LOOKUP_STDOUT: &[u8] = concat!(
    "Error { name: \"",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "\"... 200 more characters }\n",
    "[Function (anonymous)]\n",
)
.as_bytes();

const QJS_ROPE_LEAF_BOUNDARY_SOURCE: &str = r#"var left = "a".repeat(500) + "\ud83d";
var right = JSON.parse("\"\\udc00" + "b".repeat(599) + "\"");
var rope = left + right;
print({ s: rope });
print(rope);
print({ s: rope });
var codecLeft = "a".repeat(500) + "\ud83d";
var codecRight = JSON.parse("\"\\udc00" + "b".repeat(599) + "\"");
var codecRope = codecLeft + codecRight;
try { Uint8Array.fromHex(codecRope); } catch (error) {}
print({ s: codecRope });"#;

fn qjs_rope_leaf_boundary_stdout() -> Vec<u8> {
    let mut output = b"{ s: \"".to_vec();
    output.extend_from_slice("a".repeat(500).as_bytes());
    output.extend_from_slice(br"\ud83d\udc00");
    output.extend_from_slice("b".repeat(498).as_bytes());
    output.extend_from_slice(b"\"... 101 more characters }\n");
    output.extend_from_slice("a".repeat(500).as_bytes());
    output.extend_from_slice("\u{1f400}".as_bytes());
    output.extend_from_slice("b".repeat(599).as_bytes());
    output.push(b'\n');
    output.extend_from_slice(b"{ s: \"");
    output.extend_from_slice("a".repeat(500).as_bytes());
    output.extend_from_slice("\u{1f400}".as_bytes());
    output.extend_from_slice("b".repeat(498).as_bytes());
    output.extend_from_slice(b"\"... 101 more characters }\n");
    output.extend_from_slice(b"{ s: \"");
    output.extend_from_slice("a".repeat(500).as_bytes());
    output.extend_from_slice("\u{1f400}".as_bytes());
    output.extend_from_slice("b".repeat(498).as_bytes());
    output.extend_from_slice(b"\"... 101 more characters }\n");
    output
}

const QJS_HOST_STDOUT_CASES: &[QjsHostStdoutCase] = &[
    QjsHostStdoutCase {
        description: "JS_PrintValue value shapes",
        options: &[],
        source: r#"print("primitives", undefined, null, true, false, 0, -0, 1.5, NaN,
      Infinity, -Infinity, 1n, 123456789012345678901234567890n,
      Symbol("sym"));
var key = Symbol("key");
var object = { alpha: 1, "not key": "x" };
Object.defineProperty(object, "hidden", { value: 2, enumerable: false });
object[key] = 3;
print(object);
print([1, 2], [1, , 3], new Uint8Array([3, 4]));
print(new Map([[1, "x"]]), new Set([2, 3]), /a+/gi, new Date(0));
function named() {}
print(named);
var error = new TypeError("boom");
delete error.stack;
print(error);
var root = { name: "root" };
root.self = root;
root.child = { parent: root, deep: { x: 1 } };
print(root);"#,
        expected_status: 0,
        expected_stdout: br#"primitives undefined null true false 0 -0 1.5 NaN Infinity -Infinity 1n 123456789012345678901234567890n Symbol(sym)
{ alpha: 1, "not key": "x", key: 3 }
[ 1, 2 ] [ 0: 1, 2: 3 ] Uint8Array(2) [ 3, 4 ]
Map(1) { 1 => "x" } Set(2) { 2, 3 } /a+/gi 1970-01-01T00:00:00.000Z
[Function named]
TypeError: boom
{ name: "root", self: [circular 0], child: { parent: [circular 0], deep: [Object] } }
"#,
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "JS_PrintValue avoids JavaScript side effects",
        options: &[],
        source: r#"var hits = [];
var object = {
  alpha: 1,
  get accessor() { hits.push("getter"); throw "getter"; },
  [Symbol.toPrimitive]() { hits.push("primitive"); throw "primitive"; },
  toString() { hits.push("toString"); throw "toString"; }
};
print(object);
var proxy = new Proxy({ x: 1 }, {
  ownKeys() { hits.push("ownKeys"); throw "ownKeys"; },
  getOwnPropertyDescriptor() { hits.push("gopd"); throw "gopd"; },
  get() { hits.push("get"); throw "get"; }
});
print(proxy);
var fn = function() {};
Object.defineProperty(fn, "name", {
  get() { hits.push("fn-name"); throw "fn-name"; }
});
print(fn);
var error = new Error("boom");
Object.defineProperty(error, "name", {
  get() { hits.push("error-name"); throw "error-name"; }
});
Object.defineProperty(error, "message", {
  get() { hits.push("error-message"); throw "error-message"; }
});
Object.defineProperty(error, "stack", {
  get() { hits.push("error-stack"); throw "error-stack"; }
});
print(error);
print("hits=" + hits.join(","));"#,
        expected_status: 0,
        expected_stdout: br#"{ alpha: 1, accessor: [Getter], "Symbol.toPrimitive": [Function [Symbol.toPrimitive]], toString: [Function toString] }
Object {  }
[Function (anonymous)]
Error
hits=
"#,
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "qjs print and console.log host descriptors",
        options: &[],
        source: r#"function bits(descriptor) {
  return [descriptor.writable, descriptor.enumerable, descriptor.configurable]
    .map(Number).join("");
}
var chosen = Reflect.ownKeys(globalThis).filter(function(key) {
  return key === "console" || key === "print";
}).join(",");
print("global-order", chosen);
print("global",
      bits(Object.getOwnPropertyDescriptor(globalThis, "print")),
      bits(Object.getOwnPropertyDescriptor(globalThis, "console")));
print("print", typeof print, print.name, print.length,
      Reflect.ownKeys(print).join(","),
      bits(Object.getOwnPropertyDescriptor(print, "name")),
      bits(Object.getOwnPropertyDescriptor(print, "length")));
print("console", typeof console, Reflect.ownKeys(console).join(","),
      bits(Object.getOwnPropertyDescriptor(console, "log")),
      Object.getPrototypeOf(console) === Object.prototype);
print("log", typeof console.log, console.log.name, console.log.length,
      Reflect.ownKeys(console.log).join(","),
      bits(Object.getOwnPropertyDescriptor(console.log, "name")),
      bits(Object.getOwnPropertyDescriptor(console.log, "length")),
      console.log === print);
var printResult = print("print-call", 42);
var logResult = console.log("log-call", 42);
print("returns", printResult, logResult);
var printConstruct = "missing";
var logConstruct = "missing";
try { new print(); } catch (error) {
  printConstruct = error.name + ":" + error.message;
}
try { new console.log(); } catch (error) {
  logConstruct = error.name + ":" + error.message;
}
print("construct", printConstruct, logConstruct);"#,
        expected_status: 0,
        expected_stdout: br#"global-order console,print
global 111 111
print function print 1 length,name 001 001
console object log 111 true
log function log 1 length,name 001 001 false
print-call 42
log-call 42
returns undefined undefined
construct TypeError:print is not a constructor TypeError:log is not a constructor
"#,
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "Map and Set ordinary delete removes records",
        options: &[],
        source: r#"var map = new Map([[1, "one"], [2, "two"], [3, "three"]]);
map.delete(2);
print("map", map);
var set = new Set([1, 2, 3]);
set.delete(2);
print("set", set);"#,
        expected_status: 0,
        expected_stdout: br#"map Map(2) { 1 => "one", 3 => "three" }
set Set(2) { 1, 3 }
"#,
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "Map and Set iterators retain the current deleted record",
        options: &[],
        source: r#"var map = new Map([[1, "one"], [2, "two"]]);
var mapIterator = map.entries();
mapIterator.next();
map.delete(1);
print("map-zombie", map);
mapIterator.next();
print("map-clean", map);
var set = new Set([1, 2]);
var setIterator = set.values();
setIterator.next();
set.delete(1);
print("set-zombie", set);
setIterator.next();
print("set-clean", set);"#,
        expected_status: 0,
        expected_stdout: br#"map-zombie Map(1) { , 2 => "two" }
map-clean Map(1) { 2 => "two" }
set-zombie Set(1) { , 2 }
set-clean Set(1) { 2 }
"#,
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "Map and Set forEach retain the current deleted record",
        options: &[],
        source: r#"var map = new Map([[1, "one"], [2, "two"]]);
map.forEach(function(value, key) {
  if (key === 1) {
    map.delete(key);
    print("map-callback", map);
  }
});
print("map-after", map);
var set = new Set([1, 2]);
set.forEach(function(value) {
  if (value === 1) {
    set.delete(value);
    print("set-callback", set);
  }
});
print("set-after", set);"#,
        expected_status: 0,
        expected_stdout: br#"map-callback Map(1) { , 2 => "two" }
map-after Map(1) { 2 => "two" }
set-callback Set(1) { , 2 }
set-after Set(1) { 2 }
"#,
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "Set methods retain the current record across user callbacks",
        options: &[],
        source: r#"var set = new Set([1, 2]);
var other = {
  size: 2,
  has: function(value) {
    set.delete(value);
    print(set);
    return false;
  },
  keys: function() { throw "unused"; }
};
print("result", set.isSubsetOf(other));"#,
        expected_status: 0,
        expected_stdout: br#"Set(1) { , 2 }
result false
"#,
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "fast and slow Arguments numeric properties",
        options: &[],
        source: r#"(function(a, b) {
  print("fast", arguments);
})(10, 20);
(function(a, b) {
  delete arguments[0];
  print("delete", arguments);
})(10, 20);
(function(a, b) {
  Object.defineProperty(arguments, "0", {
    value: 99,
    enumerable: true,
    writable: false,
    configurable: true
  });
  print("define", arguments);
})(10, 20);"#,
        expected_status: 0,
        expected_stdout: br#"fast Arguments {  }
delete Arguments { 1: 20 }
define Arguments { 0: 99, 1: 20 }
"#,
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "Iterator subclasses retain the QuickJS class tag",
        options: &[],
        source: r#"class CustomIterator extends Iterator {}
var iterator = new CustomIterator();
iterator.answer = 42;
print(iterator, iterator instanceof CustomIterator);"#,
        expected_status: 0,
        expected_stdout: b"Iterator { answer: 42 } true\n",
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "default collection limits, retained zombies, SAB, and depth classes",
        options: &[],
        source: r#"var map = new Map();
for (var i = 0; i < 102; i++) map.set(i, 0);
for (var i = 0; i < 101; i++) map[i] = 0;
var mapIterator = map.keys();
mapIterator.next();
map.delete(0);
print("zombie-before", map);

var set = new Set();
for (var i = 0; i < 102; i++) set.add(i);
for (var i = 0; i < 101; i++) set[i] = 0;
var setIterator = set.values();
for (var i = 0; i < 101; i++) setIterator.next();
set.delete(100);
print("zombie-after", set);

var sab = new SharedArrayBuffer(3);
var sabView = new Uint8Array(sab);
sabView.set([1, 2, 255]);
print("sab", sabView);

var promise = Promise.withResolvers();
print("depth", { level1: {
  typed: new Uint8Array([1]),
  generatorFunction: function* () {},
  asyncFunction: async function () {},
  asyncGeneratorFunction: async function* () {},
  generatorObject: (function* () {})(),
  asyncGeneratorObject: (async function* () {})(),
  resolve: promise.resolve,
  reject: promise.reject
} });"#,
        expected_status: 0,
        expected_stdout: br#"zombie-before Map(101) { , 1 => 0, 2 => 0, 3 => 0, 4 => 0, 5 => 0, 6 => 0, 7 => 0, 8 => 0, 9 => 0, 10 => 0, 11 => 0, 12 => 0, 13 => 0, 14 => 0, 15 => 0, 16 => 0, 17 => 0, 18 => 0, 19 => 0, 20 => 0, 21 => 0, 22 => 0, 23 => 0, 24 => 0, 25 => 0, 26 => 0, 27 => 0, 28 => 0, 29 => 0, 30 => 0, 31 => 0, 32 => 0, 33 => 0, 34 => 0, 35 => 0, 36 => 0, 37 => 0, 38 => 0, 39 => 0, 40 => 0, 41 => 0, 42 => 0, 43 => 0, 44 => 0, 45 => 0, 46 => 0, 47 => 0, 48 => 0, 49 => 0, 50 => 0, 51 => 0, 52 => 0, 53 => 0, 54 => 0, 55 => 0, 56 => 0, 57 => 0, 58 => 0, 59 => 0, 60 => 0, 61 => 0, 62 => 0, 63 => 0, 64 => 0, 65 => 0, 66 => 0, 67 => 0, 68 => 0, 69 => 0, 70 => 0, 71 => 0, 72 => 0, 73 => 0, 74 => 0, 75 => 0, 76 => 0, 77 => 0, 78 => 0, 79 => 0, 80 => 0, 81 => 0, 82 => 0, 83 => 0, 84 => 0, 85 => 0, 86 => 0, 87 => 0, 88 => 0, 89 => 0, 90 => 0, 91 => 0, 92 => 0, 93 => 0, 94 => 0, 95 => 0, 96 => 0, 97 => 0, 98 => 0, 99 => 0, 100 => 0, ... 1 more item, 0: 0, 1: 0, 2: 0, 3: 0, 4: 0, 5: 0, 6: 0, 7: 0, 8: 0, 9: 0, 10: 0, 11: 0, 12: 0, 13: 0, 14: 0, 15: 0, 16: 0, 17: 0, 18: 0, 19: 0, 20: 0, 21: 0, 22: 0, 23: 0, 24: 0, 25: 0, 26: 0, 27: 0, 28: 0, 29: 0, 30: 0, 31: 0, 32: 0, 33: 0, 34: 0, 35: 0, 36: 0, 37: 0, 38: 0, 39: 0, 40: 0, 41: 0, 42: 0, 43: 0, 44: 0, 45: 0, 46: 0, 47: 0, 48: 0, 49: 0, 50: 0, 51: 0, 52: 0, 53: 0, 54: 0, 55: 0, 56: 0, 57: 0, 58: 0, 59: 0, 60: 0, 61: 0, 62: 0, 63: 0, 64: 0, 65: 0, 66: 0, 67: 0, 68: 0, 69: 0, 70: 0, 71: 0, 72: 0, 73: 0, 74: 0, 75: 0, 76: 0, 77: 0, 78: 0, 79: 0, 80: 0, 81: 0, 82: 0, 83: 0, 84: 0, 85: 0, 86: 0, 87: 0, 88: 0, 89: 0, 90: 0, 91: 0, 92: 0, 93: 0, 94: 0, 95: 0, 96: 0, 97: 0, 98: 0, 99: 0, ... 1 more item }
zombie-after Set(101) { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, ... 1 more item, 0: 0, 1: 0, 2: 0, 3: 0, 4: 0, 5: 0, 6: 0, 7: 0, 8: 0, 9: 0, 10: 0, 11: 0, 12: 0, 13: 0, 14: 0, 15: 0, 16: 0, 17: 0, 18: 0, 19: 0, 20: 0, 21: 0, 22: 0, 23: 0, 24: 0, 25: 0, 26: 0, 27: 0, 28: 0, 29: 0, 30: 0, 31: 0, 32: 0, 33: 0, 34: 0, 35: 0, 36: 0, 37: 0, 38: 0, 39: 0, 40: 0, 41: 0, 42: 0, 43: 0, 44: 0, 45: 0, 46: 0, 47: 0, 48: 0, 49: 0, 50: 0, 51: 0, 52: 0, 53: 0, 54: 0, 55: 0, 56: 0, 57: 0, 58: 0, 59: 0, 60: 0, 61: 0, 62: 0, 63: 0, 64: 0, 65: 0, 66: 0, 67: 0, 68: 0, 69: 0, 70: 0, 71: 0, 72: 0, 73: 0, 74: 0, 75: 0, 76: 0, 77: 0, 78: 0, 79: 0, 80: 0, 81: 0, 82: 0, 83: 0, 84: 0, 85: 0, 86: 0, 87: 0, 88: 0, 89: 0, 90: 0, 91: 0, 92: 0, 93: 0, 94: 0, 95: 0, 96: 0, 97: 0, 98: 0, 99: 0, ... 1 more item }
sab Uint8Array(3) [ 1, 2, 255 ]
depth { level1: { typed: [Uint8Array], generatorFunction: [GeneratorFunction], asyncFunction: [AsyncFunction], asyncGeneratorFunction: [AsyncGeneratorFunction], generatorObject: [Generator], asyncGeneratorObject: [AsyncGenerator], resolve: [PromiseResolveFunction], reject: [PromiseRejectFunction] } }
"#,
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "rope-valued Error and Function special fields",
        options: &[],
        source: r#"var rope = "a".repeat(600) + "b".repeat(600);
var error = new Error("ignored");
Object.defineProperties(error, {
  name: { value: rope, enumerable: true },
  message: { value: rope },
  stack: { value: rope }
});
print(error);
function named() {}
Object.defineProperty(named, "name", { value: rope });
print(named);"#,
        expected_status: 0,
        expected_stdout: QJS_ROPE_SPECIAL_LOOKUP_STDOUT,
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "uncaught ordinary object uses JS_PrintValue on stderr",
        options: &[],
        source: "throw { a: 1 };",
        expected_status: 1,
        expected_stdout: b"",
        expected_stderr: b"{ a: 1 }\n",
    },
    QjsHostStdoutCase {
        description: "uncaught function preserves non-UTF-8 name bytes on stderr",
        options: &[],
        source: r#"var thrown = function() {};
Object.defineProperty(thrown, "name", { value: "\ud800" });
throw thrown;"#,
        expected_status: 1,
        expected_stdout: b"",
        expected_stderr: b"[Function \xed\xa0\x80]\n",
    },
    QjsHostStdoutCase {
        description: "qjs host preserves raw UTF-16 stdout bytes",
        options: &[],
        source: r#"print("\ud800", "\udc00", "A\0B");
print({ x: "\ud800" }, Symbol("\ud800"));
console.log("\ud800");"#,
        expected_status: 0,
        expected_stdout: b"\xed\xa0\x80 \xed\xb0\x80 A\0B\n{ x: \"\\ud800\" } Symbol(\"\\ud800\")\n\xed\xa0\x80\n",
        expected_stderr: b"",
    },
    QjsHostStdoutCase {
        description: "console.log is installed for Module goal",
        options: &["-m"],
        source: "console.log({ answer: 42 });",
        expected_status: 0,
        expected_stdout: b"{ answer: 42 }\n",
        expected_stderr: b"",
    },
];

const QJS_PRINT_VALUE_COMPREHENSIVE_SOURCE: &str = r#"print("SCALAR",undefined,null,false,true,0,-0,1.5,NaN,Infinity,-Infinity,1e21,1e-7,5e-324);
print("BIG",0n,-1n,9223372036854775807n,9223372036854775808n,123456789012345678901234567890n,-123456789012345678901234567890n);
print("SYM",Symbol(),Symbol(""),Symbol("alpha"),Symbol("not ident"),Symbol.iterator);
print("RAW-BYTES","A\x00B","\ud800","\udc00","\ud800\udc00");
print("TOP-RAW","a\nb");
print("STR-ESC",{s:"a\t\r\n\b\f\\\"\x00\x1f\x7f\x9f\ud800\udc00"});
print("STR-LIMIT",{s:"x".repeat(1003)});
print("STR-CUT",{s:"x".repeat(999)+"\ud83d\ude00"});

var effects=[];
var bomb={};
bomb[Symbol.toPrimitive]=function primitive(){effects.push("toPrimitive");throw 1};
bomb.toString=function toString(){effects.push("toString");throw 2};
bomb.valueOf=function valueOf(){effects.push("valueOf");throw 3};
print("NO-COERCE",bomb);
print("NO-COERCE-EFFECTS",JSON.stringify(effects));

var obj={plain:1};
Object.defineProperty(obj,"getOnly",{enumerable:true,get:function(){effects.push("getOnly");return 2}});
Object.defineProperty(obj,"setOnly",{enumerable:true,set:function(v){effects.push("setOnly")}});
Object.defineProperty(obj,"both",{enumerable:true,get:function(){effects.push("both-get");return 3},set:function(v){effects.push("both-set")}});
Object.defineProperty(obj,"hidden",{enumerable:false,value:4});
obj["not-ident"]=5;
obj[Symbol("sym key")]=6;
obj.method=function method(){};
print("OBJ",obj);
print("OBJ-EFFECTS",JSON.stringify(effects));

effects=[];
var proxy=new Proxy({a:1},{
  ownKeys:function(t){effects.push("ownKeys");return Reflect.ownKeys(t)},
  getOwnPropertyDescriptor:function(t,k){effects.push("gopd");return Object.getOwnPropertyDescriptor(t,k)},
  get:function(t,k,r){effects.push("get");return Reflect.get(t,k,r)}
});
print("PROXY",proxy);
print("PROXY-EFFECTS",JSON.stringify(effects));

print("ARR-DENSE",[1,"x",undefined,-0,1n,Symbol("s")]);
print("ARR-HOLES",new Array(3));
print("ARR-SPARSE",[1,,3]);
var extra=[1,2]; extra.foo=3; print("ARR-EXTRA",extra);
effects=[];
var accessorArray=[];
Object.defineProperty(accessorArray,"0",{enumerable:true,get:function(){effects.push("array-get");return 1}});
accessorArray.length=1;
print("ARR-ACCESSOR",accessorArray);
print("ARR-ACCESSOR-EFFECTS",JSON.stringify(effects));

var ac=[]; ac[0]=ac; print("ARRAY-CYCLE",ac);
var oc={}; oc.self=oc; print("OBJECT-CYCLE",oc);
print("DEPTH",{a:{b:{c:1}}});
var shared={z:1}; print("SHARED",{a:shared,b:shared});

var items=[];
for(var i=0;i<103;i++)items.push(i);
print("ARRAY-ITEMS",items);
var props={};
for(var i=0;i<103;i++)props["p"+i]=i;
print("OBJECT-ITEMS",props);

function named(a,b){}
named.extra=1;
print("FUNCTIONS",named,named.bind(null),(0,function(){}),Array,function* gen(){},async function af(){},class Klass{});

effects=[];
function base(){}
Object.defineProperty(base,"name",{enumerable:true,get:function(){effects.push("fn-name");return "changed"}});
base.extra=1;
print("FN-ACCESSOR",base);
print("FN-EFFECTS",JSON.stringify(effects));

var err=new TypeError("boom");
err.stack="STACK-A\nSTACK-B\n";
print("ERROR",err);
effects=[];
var errAccessor=new Error("ignored");
Object.defineProperty(errAccessor,"message",{get:function(){effects.push("message");return "bad"}});
Object.defineProperty(errAccessor,"stack",{get:function(){effects.push("stack");return "bad"}});
print("ERROR-ACCESSOR",errAccessor);
print("ERROR-EFFECTS",JSON.stringify(effects));

print("REGEXP",new RegExp(""),new RegExp("a/b\n","gimsuyd"));
var reNamed=new RegExp("(?<x>a)");
var reV=new RegExp("[a-z]","v");
print("REGEXP-NAMED",reNamed,"["+reNamed.flags+"]");
print("REGEXP-V",reV,"["+reV.flags+"]");
print("DATE",new Date(0),new Date(NaN));

var map=new Map([[1,"one"],["k",{x:2}]]);
var mapCycle=new Map(); mapCycle.set("self",mapCycle);
print("MAP",map);
print("MAP-CYCLE",mapCycle);
var set=new Set([1,"x",{y:2}]);
var setCycle=new Set(); setCycle.add(setCycle);
print("SET",set);
print("SET-CYCLE",setCycle);

print("TA-U8",new Uint8Array([0,1,255]));
print("TA-I8",new Int8Array([-128,-1,127]));
print("TA-BI",new BigInt64Array([-1n,0n,9223372036854775807n]));
print("TA-BU",new BigUint64Array([0n,18446744073709551615n]));
print("TA-F16",new Float16Array([1.5,-0,NaN,Infinity,-Infinity]));
print("TA-F64",new Float64Array([1.5,-0,NaN,Infinity,-Infinity,5e-324]));
var ta=new Uint8Array([1]); ta.extra=2; ta.self=ta;
print("TA-PROPS",ta);

var ab=new ArrayBuffer(2);
var sab=new SharedArrayBuffer(2);
var weakTarget={};
print("GENERIC",ab,sab,new DataView(ab),Promise.resolve(1),new WeakMap(),new WeakSet(),new WeakRef(weakTarget),new FinalizationRegistry(function(){}),[1][Symbol.iterator](),(function*(){yield 1})());
console.log("CONSOLE",{a:1},-0,1n,Symbol("s"));"#;

struct ModuleFixture {
    root: PathBuf,
}

impl ModuleFixture {
    fn new() -> Self {
        let id = NEXT_MODULE_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "quickjs-oxide-cli-module-{}-{id}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale CLI module fixture");
        }
        fs::create_dir_all(&root).expect("create CLI module fixture");
        Self { root }
    }

    fn write(&self, relative: &str, source: &str) -> PathBuf {
        self.write_bytes(relative, source.as_bytes())
    }

    fn write_bytes(&self, relative: &str, source: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create CLI module fixture directory");
        }
        fs::write(&path, source).expect("write CLI module fixture");
        path
    }
}

impl Drop for ModuleFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn qjs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_qjs"))
}

fn cli_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    return path.replace('\\', "/");
    #[cfg(not(windows))]
    path.into_owned()
}

fn run_file(arguments: &[&str], path: &Path) -> std::process::Output {
    qjs()
        .args(arguments)
        .arg(cli_path(path))
        .output()
        .expect("run qjs file")
}

fn expected_file_url(path: &Path) -> String {
    let filename = cli_path(path);
    if filename.contains(':') {
        filename
    } else {
        format!("file://{}", path.canonicalize().unwrap().display())
    }
}

#[test]
fn eval_executes_the_rust_compiler_and_vm() {
    let output = qjs().args(["-e", "(6 + 1) * 6"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn print_result_exposes_the_completion_value_without_changing_eval_default() {
    let output = qjs()
        .args([
            "--print-result",
            "-e",
            "(function(a) { return a + 1; })(41)",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"42\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn qjs_host_stdout_matches_pinned_quickjs_golden() {
    for case in QJS_HOST_STDOUT_CASES {
        let output = run_cli(
            env!("CARGO_BIN_EXE_qjs").as_ref(),
            case.options,
            case.source,
            case.description,
        );
        assert_eq!(
            output.status.code(),
            Some(case.expected_status),
            "{}",
            case.description,
        );
        assert_eq!(output.stdout, case.expected_stdout, "{}", case.description);
        assert_eq!(output.stderr, case.expected_stderr, "{}", case.description);
    }
}

#[test]
fn qjs_print_value_preserves_rope_leaves_and_raw_print_linearizes() {
    let output = run_cli(
        env!("CARGO_BIN_EXE_qjs").as_ref(),
        &[],
        QJS_ROPE_LEAF_BOUNDARY_SOURCE,
        "JS_PrintValue rope leaves and raw String linearization",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, qjs_rope_leaf_boundary_stdout());
    assert!(output.stderr.is_empty());
}

#[test]
fn qjs_host_stdout_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP qjs host differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let quickjs_outputs = QJS_HOST_STDOUT_CASES
        .iter()
        .map(|case| {
            let output = run_cli(&oracle, case.options, case.source, case.description);
            assert_eq!(
                output.status.code(),
                Some(case.expected_status),
                "pinned QuickJS exit status drifted for {}: {}",
                case.description,
                String::from_utf8_lossy(&output.stderr),
            );
            assert_eq!(
                output.stdout, case.expected_stdout,
                "pinned QuickJS golden drifted for {}",
                case.description,
            );
            assert_eq!(
                output.stderr, case.expected_stderr,
                "pinned QuickJS stderr golden drifted for {}",
                case.description,
            );
            output
        })
        .collect::<Vec<_>>();

    for (case, quickjs) in QJS_HOST_STDOUT_CASES.iter().zip(quickjs_outputs) {
        let oxide = run_cli(
            env!("CARGO_BIN_EXE_qjs").as_ref(),
            case.options,
            case.source,
            case.description,
        );
        assert_eq!(
            oxide.status.code(),
            quickjs.status.code(),
            "{}",
            case.description,
        );
        assert_eq!(oxide.stdout, quickjs.stdout, "{}", case.description);
        assert_eq!(oxide.stderr, quickjs.stderr, "{}", case.description);
    }

    let description = "comprehensive JS_PrintValue probe";
    let quickjs = run_cli(
        &oracle,
        &[],
        QJS_PRINT_VALUE_COMPREHENSIVE_SOURCE,
        description,
    );
    assert!(
        quickjs.status.success(),
        "QuickJS failed for {description}: {}",
        String::from_utf8_lossy(&quickjs.stderr),
    );
    assert!(
        quickjs.stderr.is_empty(),
        "QuickJS emitted stderr for {description}: {}",
        String::from_utf8_lossy(&quickjs.stderr),
    );

    let oxide = run_cli(
        env!("CARGO_BIN_EXE_qjs").as_ref(),
        &[],
        QJS_PRINT_VALUE_COMPREHENSIVE_SOURCE,
        description,
    );
    assert_eq!(oxide.status.code(), quickjs.status.code(), "{description}");
    assert_eq!(oxide.stdout, quickjs.stdout, "{description}");
    assert_eq!(oxide.stderr, quickjs.stderr, "{description}");

    let description = "JS_PrintValue rope leaves and raw String linearization";
    let expected = qjs_rope_leaf_boundary_stdout();
    let quickjs = run_cli(&oracle, &[], QJS_ROPE_LEAF_BOUNDARY_SOURCE, description);
    assert_eq!(quickjs.status.code(), Some(0), "{description}");
    assert_eq!(quickjs.stdout, expected, "{description}");
    assert!(quickjs.stderr.is_empty(), "{description}");
    let oxide = run_cli(
        env!("CARGO_BIN_EXE_qjs").as_ref(),
        &[],
        QJS_ROPE_LEAF_BOUNDARY_SOURCE,
        description,
    );
    assert_eq!(oxide.status.code(), quickjs.status.code(), "{description}");
    assert_eq!(oxide.stdout, quickjs.stdout, "{description}");
    assert_eq!(oxide.stderr, quickjs.stderr, "{description}");
}

#[test]
fn qjs_keeps_quickjs_default_non_blocking_host_policy() {
    let output = qjs()
        .args([
            "-e",
            "Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 1, 0)",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "TypeError: cannot block in this thread\n    at wait (native)\n    at <eval> (<cmdline>:1:13)\n"
    );
}

#[test]
fn eval_executes_source_level_functions_and_formats_native_errors() {
    let function = qjs()
        .args(["-e", "(function(a, b) { return a + b; })(20, 22)"])
        .output()
        .unwrap();
    assert!(function.status.success());

    let error = qjs().args(["-e", "1n + 1"]).output().unwrap();
    assert_eq!(error.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(error.stderr).unwrap(),
        "TypeError: cannot convert bigint to number\n    at <eval> (<cmdline>:1:4)\n"
    );
}

#[test]
fn unparenthesized_power_unary_error_omits_a_source_frame_like_quickjs() {
    for source in ["-2 ** 2", "-value++ ** 2"] {
        let output = qjs().args(["-e", source]).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{source}");
        assert!(output.stdout.is_empty(), "{source}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "SyntaxError: unparenthesized unary expression can't appear on the left-hand side of '**'\n\n",
            "{source}"
        );
    }

    let dynamic = qjs()
        .args(["-e", "Function(\"return -2 ** 2\")"])
        .output()
        .unwrap();
    assert_eq!(dynamic.status.code(), Some(1));
    assert!(dynamic.stdout.is_empty());
    assert_eq!(
        String::from_utf8(dynamic.stderr).unwrap(),
        "SyntaxError: unparenthesized unary expression can't appear on the left-hand side of '**'\n    at Function (native)\n    at <eval> (<cmdline>:1:9)\n"
    );
}

#[test]
fn eval_executes_the_dynamic_function_constructor_path() {
    for source in [
        "throw Function(\"a\", \"return a + 1\")(41)",
        "throw new Function(\"return 42\")()",
    ] {
        let output = qjs().args(["-e", source]).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap(), "42\n");
    }
}

#[test]
fn exception_output_quotes_strings_and_marks_bigints() {
    for (source, expected) in [
        ("throw \"x\"", "\"x\"\n"),
        (
            "throw 123456789012345678901234567890n",
            "123456789012345678901234567890n\n",
        ),
        ("throw -0", "-0\n"),
    ] {
        let output = qjs().args(["-e", source]).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{source}");
        assert!(output.stdout.is_empty(), "{source}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn unsupported_source_fails_instead_of_falling_back_to_an_external_engine() {
    let output = qjs().args(["-e", "answer"]).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("'answer' is not defined"));
}

#[test]
fn expression_position_statement_keywords_expose_quickjs_syntax_errors() {
    for keyword in [
        "return",
        "instanceof",
        "do",
        "while",
        "break",
        "continue",
        "switch",
        "throw",
        "try",
        "with",
    ] {
        let source = format!("var x = {keyword};");
        let output = qjs().args(["-e", &source]).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{source:?}");
        assert!(output.stdout.is_empty(), "{source:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "SyntaxError: unexpected token in expression: '{keyword}'\n    at <cmdline>:1:9\n"
            ),
            "{source:?}",
        );
    }
}

#[test]
fn dynamic_import_reaches_the_async_host_rejection_path() {
    let output = qjs()
        .args([
            "-e",
            "import('fixture').catch(function(error) { print(error.name + ':' + error.message); });",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostic = String::from_utf8(output.stdout).unwrap();
    assert!(diagnostic.starts_with("ReferenceError:"), "{diagnostic:?}");
    assert!(
        diagnostic.contains("module filename 'fixture'"),
        "{diagnostic:?}"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn eval_dynamic_import_uses_the_process_file_loader() {
    let fixture = ModuleFixture::new();
    let dependency = fixture.write(
        "eval-dependency.mjs",
        "export const answer = 42; export const main = import.meta.main;\n",
    );
    let specifier = dependency.to_string_lossy();
    let source = format!(
        "import({specifier:?}).then(function(module) {{ print(module.answer, module.main); }});"
    );
    let output = qjs().args(["-e", &source]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"42 false\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn eval_module_matches_qjs_platform_cmdline_import_meta_initialization() {
    let output = qjs()
        .args(["-m", "-e", "print(import.meta.url, import.meta.main)"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    #[cfg(windows)]
    assert_eq!(output.stdout, b"file://<cmdline> true\n");
    #[cfg(not(windows))]
    assert_eq!(output.stdout, b"undefined undefined\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn explicit_module_modes_load_relative_files_and_wait_for_top_level_await() {
    let fixture = ModuleFixture::new();
    let dependency = fixture.write(
        "dependency.js",
        concat!(
            "await Promise.resolve();\n",
            "export const answer = 42;\n",
            "export const dependencyMain = import.meta.main;\n",
            "export const dependencyUrl = import.meta.url;\n",
        ),
    );
    let entry = fixture.write(
        "entry.js",
        concat!(
            "import { answer, dependencyMain, dependencyUrl } from './dependency.js';\n",
            "print(answer);\n",
            "print(import.meta.main);\n",
            "print(import.meta.url);\n",
            "print(dependencyMain);\n",
            "print(dependencyUrl);\n",
        ),
    );
    let expected = format!(
        "42\ntrue\n{}\nfalse\n{}\n",
        expected_file_url(&entry),
        expected_file_url(&dependency),
    );

    for arguments in [["-m"].as_slice(), ["--module"].as_slice()] {
        let output = run_file(arguments, &entry);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn file_loader_matches_quickjs_json_json5_classification_and_rejects_unknown_keys() {
    let fixture = ModuleFixture::new();
    fixture.write("by-extension.json", r#"{"answer":40}"#);
    fixture.write("by-attribute.data", r#"{"answer":2}"#);
    fixture.write("by-json5-attribute.data", "{answer:+3,}");
    fixture.write("script.json5", "export default 4;\n");
    fixture.write("strict-override.json5", r#"{"answer":5}"#);
    fixture.write("extended-override.json", "{answer:0b110,}");
    fixture.write("unknown-on-json.json", r#"{"answer":7}"#);
    fixture.write("unknown-on-data.data", "export default 8;\n");
    let entry = fixture.write(
        "json-entry.mjs",
        concat!(
            "import extension from './by-extension.json';\n",
            "import attribute from './by-attribute.data' with { type: 'json' };\n",
            "import json5 from './by-json5-attribute.data' with { type: 'json5' };\n",
            "import script from './script.json5';\n",
            "import strictOverride from './strict-override.json5' with { type: 'json' };\n",
            "import extendedOverride from './extended-override.json' with { type: 'json5' };\n",
            "import unknownJson from './unknown-on-json.json' with { type: 'other' };\n",
            "import unknownData from './unknown-on-data.data' with { type: 'other' };\n",
            "print([extension.answer, attribute.answer, json5.answer, script, ",
            "strictOverride.answer, extendedOverride.answer, unknownJson.answer, ",
            "unknownData].join(','));\n",
        ),
    );
    let json = run_file(&[], &entry);
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    assert_eq!(json.stdout, b"40,2,3,4,5,6,7,8\n");
    assert!(json.stderr.is_empty());

    let rejected = fixture.write(
        "bad-attribute.mjs",
        "import './by-extension.json' with { integrity: 'x' };\n",
    );
    let rejected = run_file(&[], &rejected);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(rejected.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("TypeError: import attribute 'integrity' is not supported"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
}

#[test]
fn file_goal_autodetects_mjs_and_leading_static_module_syntax() {
    let fixture = ModuleFixture::new();
    fixture.write("static-dependency.js", "print('static import');\n");
    let extension = fixture.write(
        "extension.mjs",
        "await Promise.resolve(); print('extension');\n",
    );
    let syntax = fixture.write(
        "syntax.js",
        "// leading trivia\nexport const answer = 42; print(answer);\n",
    );
    let hashbang = fixture.write(
        "hashbang.js",
        "#!/usr/bin/env qjs\nexport const answer = 42; print(answer);\n",
    );
    let static_import = fixture.write("static-import.js", "import './static-dependency.js';\n");
    let dotfile = fixture.write(".mjs", "await Promise.resolve(); print('dotfile');\n");

    for (path, expected) in [
        (&extension, b"extension\n".as_slice()),
        (&syntax, b"42\n"),
        (&hashbang, b"42\n"),
        (&static_import, b"static import\n"),
        (&dotfile, b"dotfile\n"),
    ] {
        let output = run_file(&[], path);
        assert!(
            output.status.success(),
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected, "{}", path.display());
        assert!(output.stderr.is_empty(), "{}", path.display());
    }
}

#[test]
fn file_goal_preserves_raw_bytes_during_detection_and_evaluation() {
    let fixture = ModuleFixture::new();
    let raw_script = fixture.write_bytes("raw-script.js", b"/*\x80*/print(42);\n");
    let raw_module = fixture.write_bytes(
        "raw-module.js",
        b"/*\xff*/export const answer = 42; print(answer);\n",
    );
    let raw_hashbang_module = fixture.write_bytes(
        "raw-hashbang.js",
        b"#!\x80\xff\nexport const answer = 42; print(answer);\n",
    );

    for path in [&raw_script, &raw_module, &raw_hashbang_module] {
        let output = run_file(&[], path);
        assert!(
            output.status.success(),
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"42\n", "{}", path.display());
        assert!(output.stderr.is_empty(), "{}", path.display());
    }

    let nul_comment = fixture.write_bytes(
        "nul-comment.js",
        b"/*\0*/export const answer = 42; print(answer);\n",
    );
    let automatic = run_file(&[], &nul_comment);
    assert_eq!(automatic.status.code(), Some(1));
    assert!(automatic.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&automatic.stderr).contains("export"),
        "{}",
        String::from_utf8_lossy(&automatic.stderr)
    );

    let forced_module = run_file(&["--module"], &nul_comment);
    assert!(
        forced_module.status.success(),
        "{}",
        String::from_utf8_lossy(&forced_module.stderr)
    );
    assert_eq!(forced_module.stdout, b"42\n");
    assert!(forced_module.stderr.is_empty());

    let forced_script = run_file(&["--script"], &raw_module);
    assert_eq!(forced_script.status.code(), Some(1));
    assert!(forced_script.stdout.is_empty());
    assert!(String::from_utf8_lossy(&forced_script.stderr).contains("export"));
}

#[test]
fn file_module_loader_preserves_raw_dependency_bytes() {
    let fixture = ModuleFixture::new();
    fixture.write_bytes("raw-dependency.js", b"/*\x80*/export const answer = 42;\n");
    let entry = fixture.write(
        "raw-dependency-entry.mjs",
        "import { answer } from './raw-dependency.js'; print(answer);\n",
    );

    let output = run_file(&[], &entry);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"42\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn file_module_loader_preserves_raw_json_dependency_bytes() {
    let fixture = ModuleFixture::new();
    fixture.write_bytes(
        "raw-value.json",
        b"{\"wtf\":\"\xed\xa0\x80\",\"cesu\":\"\xed\xa0\xbd\xed\xb8\x80\"}",
    );
    fixture.write_bytes(
        "raw-value.data",
        b"/*\x80*/{answer:42,marker:'\xed\xa0\x80',}",
    );
    let entry = fixture.write(
        "raw-json-entry.mjs",
        concat!(
            "import strict from './raw-value.json';\n",
            "import extended from './raw-value.data' with { type: 'json5' };\n",
            "const exact = strict.wtf.length === 1 && ",
            "strict.wtf.charCodeAt(0) === 0xd800 && ",
            "strict.cesu.length === 2 && ",
            "strict.cesu.codePointAt(0) === 0x1f600 && ",
            "extended.marker.length === 1 && ",
            "extended.marker.charCodeAt(0) === 0xd800;\n",
            "print(exact ? extended.answer : 0);\n",
        ),
    );

    let output = run_file(&[], &entry);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"42\n");
    assert!(output.stderr.is_empty());

    let malformed = fixture.write_bytes("malformed.json", b"{\"x\":\"\x80\"}");
    let malformed_entry = fixture.write(
        "malformed-entry.mjs",
        "import value from './malformed.json'; print(value);\n",
    );
    let malformed_output = run_file(&[], &malformed_entry);
    assert_eq!(malformed_output.status.code(), Some(1));
    assert!(malformed_output.stdout.is_empty());
    let diagnostic = String::from_utf8(malformed_output.stderr).unwrap();
    assert!(
        diagnostic.contains("SyntaxError: Bad UTF-8 sequence"),
        "{diagnostic}"
    );
    assert!(
        diagnostic.contains(&format!("{}:1:7", cli_path(&malformed))),
        "{diagnostic}"
    );
}

#[test]
fn script_override_wins_over_mjs_module_detection() {
    let fixture = ModuleFixture::new();
    let entry = fixture.write("forced-script.mjs", "export const answer = 42;\n");

    let output = run_file(&["--script"], &entry);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("export"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let module_last = run_file(&["--script", "-m"], &entry);
    assert!(
        module_last.status.success(),
        "{}",
        String::from_utf8_lossy(&module_last.stderr)
    );
    assert!(module_last.stdout.is_empty());
    assert!(module_last.stderr.is_empty());

    let script_last = run_file(&["-m", "--script"], &entry);
    assert_eq!(script_last.status.code(), Some(1));
    assert!(script_last.stdout.is_empty());
    assert!(String::from_utf8_lossy(&script_last.stderr).contains("export"));
}

#[test]
fn dynamic_import_stays_script_goal_and_uses_the_file_loader() {
    let fixture = ModuleFixture::new();
    fixture.write("dependency.mjs", "export const answer = 42;\n");
    let entry = fixture.write(
        "dynamic.js",
        concat!(
            "import('./dependency.mjs').then(function(module) { print(module.answer); });\n",
            "print(this === globalThis);\n",
        ),
    );

    let output = run_file(&[], &entry);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"true\n42\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn rejected_module_promise_is_reported_once() {
    let fixture = ModuleFixture::new();
    let entry = fixture.write("rejected.mjs", "await Promise.resolve(); throw 42;\n");

    let output = run_file(&[], &entry);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"42\n");
}

#[test]
fn missing_main_file_uses_the_qjs_path_diagnostic_shape() {
    let fixture = ModuleFixture::new();
    let missing = fixture.root.join("missing.js");
    let output = run_file(&[], &missing);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.starts_with(&format!("{}: ", cli_path(&missing))));
    assert!(!diagnostic.starts_with("qjs:"));
}

#[test]
fn tracked_file_module_demo_returns_42() {
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/module-42.mjs");
    let output = run_file(&[], &demo);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"42\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn tracked_file_module_demo_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP file-module differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/module-42.mjs");
    let filename = cli_path(&demo);
    let oxide = run_file(&[], &demo);
    let quickjs = Command::new(oracle)
        .arg(&filename)
        .output()
        .expect("run QuickJS file-module demo");

    assert_eq!(oxide.status.code(), quickjs.status.code());
    assert_eq!(oxide.stdout, quickjs.stdout);
    assert_eq!(oxide.stderr, quickjs.stderr);
}

#[test]
fn version_names_the_pinned_compatibility_target() {
    let output = qjs().arg("--version").output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("QuickJS 2026-06-04"));
}

#[test]
fn strip_flags_match_quickjs_debug_stack_behavior_and_last_option_wins() {
    let source = "1n + 1";
    let located = "TypeError: cannot convert bigint to number\n    at <eval> (<cmdline>:1:4)\n";
    let stripped = "TypeError: cannot convert bigint to number\n    at <eval>\n";
    for (arguments, expected) in [
        (vec!["--strip-source", "-e", source], located),
        (vec!["-s", "-e", source], stripped),
        (vec!["-s", "--strip-source", "-e", source], located),
        (vec!["--strip-source", "-s", "-e", source], stripped),
        (vec!["-e", source, "-s"], stripped),
        (vec!["-se", source], stripped),
        (vec!["-e1n + 1", "--strip-source"], located),
    ] {
        let output = qjs().args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
    }

    for arguments in [vec!["-sq"], vec!["-qs"], vec!["-q", "-s"]] {
        let output = qjs().args(arguments).output().unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn primitive_exception_dump_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP CLI dump differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for (description, source) in [
        ("quoted string", "throw \"x\""),
        ("escaped string", "throw \"line\\n\\t\\\\\\\"\\0\\x7f\""),
        ("Unicode string", "throw \"é🙂中\""),
        ("short BigInt", "throw 1n"),
        ("heap BigInt", "throw 123456789012345678901234567890n"),
        ("negative zero", "throw -0"),
        ("invalid prefix update operand", "++1"),
        (
            "postfix under unary power early error has no source frame",
            "-value++ ** 2",
        ),
        (
            "strict private postfix update return marker",
            "(function named(){ 'use strict'; return named++; })()",
        ),
    ] {
        let rust = run_cli(env!("CARGO_BIN_EXE_qjs").as_ref(), &[], source, description);
        let quickjs = run_cli(&oracle, &[], source, description);
        assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
        assert_eq!(rust.stdout, quickjs.stdout, "{description}");
        assert_eq!(rust.stderr, quickjs.stderr, "{description}");
    }
}
