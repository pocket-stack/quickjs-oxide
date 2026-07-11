# quickjs-oxide

`quickjs-oxide` is a from-scratch, memory-safe Rust rewrite of QuickJS. The
compatibility target is the upstream **QuickJS 2026-06-04** release and its
ES2025 behavior, not merely a JavaScript-like language.

The repository is still an incomplete rewrite. The current vertical slices
execute primitive expressions, named ordinary functions and closures, and
defining-realm global bindings through the real lexer, late scope resolver,
bytecode and VM. They also implement runtime-owned
Atom/Object/Shape/Context/FunctionBytecode/VarRef nodes, ordinary properties
and accessors, native Error objects, per-function filename/PC/source debug
metadata, QuickJS-style eager `Error.stack` for the current synchronous
bytecode/native frame chain, and the first ordered `%Function.prototype%`
slice (`caller`, `arguments`, `call`, `apply`, `bind`, `toString`, and
ordinary/bound `@@hasInstance`) plus the eager `fileName`, `lineNumber`, and
`columnNumber` debug accessors. Bound functions have a dedicated traced heap
payload and forward calls, construction, `new.target`, realm lookup, and
instance checks along the QuickJS paths; function source text and native
fallback templates are also observable through `Function.prototype.toString`.
The normal `%Function%` constructor is rooted in each realm with its exact
constructor/prototype/global descriptors. For the currently supported ordinary
function grammar it follows QuickJS's exact dynamic-source wrapper, indirect
global compilation, actual-argc conversion order, call/construct split,
cross-realm behavior, and `newTarget.prototype` fallback.

The primitive-class substrate now reserves typed realm-local `class_proto`
slots for Number, String, Boolean, Symbol, and BigInt, with semantically
complete Number and Boolean slots enabled. `%Number.prototype%` is the pinned
boxed-`+0` Number-class object. The global `%Number%` publishes the exact
ordered 17/7 constructor/prototype surface: call/construct and BigInt
conversion, parser aliases captured by identity, non-coercing predicates,
frozen constants, radix 2–36 `toString`, fixed/exponential/precision formats,
`toLocaleString`, `valueOf`, custom/cross-realm `newTarget.prototype`, and
defining-realm fallback. `%Boolean%` retains its exact boxed-`false` graph and
call/construct behavior. Number and Boolean primitive member lookup traverses
the current realm's matching prototype without eager boxing and preserves the
raw receiver for strict inherited getters and setters; sloppy ordinary
functions instead create one cached wrapper per invocation. Both wrapper
classes participate in `Object.prototype.toString`, `toLocaleString`, and
`valueOf`, including observable `@@toStringTag`, cross-realm boxing, and the
runtime's reference-counting/cycle graph. String, Symbol, and BigInt wrapper
graphs remain explicit gaps, and the global `Object` constructor is not yet
published.

Source-level fixed/computed member reads, receiver-preserving method calls, and
member constructor heads now lower through QuickJS-shaped property bytecode;
the pinned differential locks chaining, `ToPropertyKey` order, `this`, String
index/length reads, and member error locations. Simple member assignment and
property delete use the corresponding QuickJS lvalue/stack shapes, including
the intentionally different computed-key conversion order, setter receiver,
strict rejection and delete-without-getter behavior. Arithmetic,
exponentiation, shift and bitwise member assignment follow
`GetField2`/`GetArrayEl3`: `+=`, `-=`, `*=`, `/=`, `%=`, `**=`, `<<=`, `>>=`,
`>>>=`, `&=`, `^=` and `|=` reuse the old value, while `&&=`, `||=` and `??=`
branch through QuickJS's `Nip` cleanup shape.
Both paths convert a computed object key exactly once before the getter. Unary
`~`, binary `&`, `^`, `|`, and binary `<<`, `>>`, `>>>` use QuickJS's exact
precedence levels and ordered `ToNumeric`. Number shifts preserve the signed
`ToInt32`/unsigned `ToUint32` split and masked counts; BigInt shifts preserve
negative-count reversal, arithmetic right-shift saturation, allocation
failures, and the pinned release's one-sign-limb allocation extension.
Right-associative `**` follows QuickJS's unary-level parser, including unary-RHS
acceptance and the unparenthesized-unary-LHS early error. Its Number path uses
Rust `f64::powf` plus QuickJS's `±1`/non-finite-exponent correction, with a
pinned differential locking the observed libc-`pow` results. Its BigInt path
preserves negative-exponent errors, constant shortcuts, and exact limb
preallocation boundaries.
The crate-wide Number-to-string path now uses a safe-Rust rewrite of pinned
QuickJS `dtoa.c`: exact BigUint rational arithmetic backs the published
`%Number.prototype%` decimal FREE, radix 2–36, fixed, exponential, and
precision methods. FREE uses ties-to-even shortest-roundtrip selection, while
explicit digit modes use ties-away-from-zero. Deterministic bit-pattern and
intrinsic differentials lock those strings against QuickJS, and BigInt has a
separately differenced ties-to-even/overflow conversion used by
`Number(BigInt)`. The global `parseInt` and `parseFloat` functions now use a
matching UTF-16 parser substrate with QuickJS's prefix scans, radix conversion
order, signed zero, bounded mantissa tables, and even its observable 38-digit
decimal truncation;
their native objects are captured by identity as `Number.parseInt` and
`Number.parseFloat`. The global `isNaN` and `isFinite` functions are separate
coercing natives: they apply ordered `ToNumber`, preserve object conversion
and defining-realm errors, and remain observably distinct from the
non-coercing `Number.isNaN`/`Number.isFinite` statics.
The same pinned global-function prefix includes `decodeURI`,
`decodeURIComponent`, `encodeURI`, `encodeURIComponent`, `escape`, and
`unescape`. Their safe-Rust codec operates directly on ECMAScript UTF-16 code
units: URI decoding preserves reserved escape spelling, validates
percent-encoded UTF-8 and throws the exact URIError variants; URI encoding
enforces surrogate pairs, while the Annex-B pair retains its permissive
`%XX`/`%uXXXX` code-unit behavior.
Each realm also installs QuickJS's configurable, read-only global-object own
property keyed by the runtime's well-known `Symbol.toStringTag`, with value
`"global"`. The host-visible `%Object.prototype.toString%` path therefore
reports `[object global]` while preserving symbol-category own-key ordering;
this slice does not yet expose the `globalThis` binding or `Symbol` constructor.
Binary `??` uses QuickJS's shared short-circuit join for a chain, preserves the
selected operand without coercion, and enforces the unparenthesized
`??`/`&&`/`||` mixing restriction. The same arithmetic, exponentiation, shift,
bitwise and logical assignment operators accept direct or parenthesized
identifier References and resolve late to argument, local, closure, global, or
private function-name paths. Prefix/postfix `++` and `--` use QuickJS-shaped
`Inc`/`Dec`/`PostInc`/`PostDec` bytecode and the same retained References;
postfix keeps the already-`ToNumeric` old value, while member writes use
`Perm3`/`Perm4` to preserve it. Their restricted LineTerminator production,
`++x ** 2` power interaction, strict lvalue errors, source markers, Number
edges, BigInt short/heap behavior, and the pinned slow-decrement quirk are
covered by differential tests. Sloppy direct-identifier `delete` now follows
the pinned QuickJS scope rewrite without first reading the binding: argument,
local, closure, private function-name and implicit-`arguments` paths return
`false`, while global/unresolved paths perform defining-realm `HasProperty`
followed by `DeleteProperty`; strict direct references are early errors. Each
realm also installs the frozen `Infinity`, `NaN` and `undefined` global data
properties. Dynamic object-environment resolution through `with`/direct
`eval`, Proxy/exotic delete dispatch, and the remaining String/Symbol/BigInt
primitive prototype graphs remain unfinished slices.
Runtime-wide full/strip-source/strip-debug modes follow QuickJS's immutable
bytecode publication boundary, and the `qjs` CLI exposes `--strip-source` and
`-s` with upstream last-option-wins behavior.
The implemented VM operators use completion-aware QuickJS-style Number/default
`ToPrimitive` and ordered `ToNumeric` paths, including exact BigInt/String
relational comparison and preservation of user-thrown coercion values.
Explicit reference counting and cycle removal own those metadata, frame and
intrinsic roots. Passing these tests proves only those slices;
it does not imply QuickJS feature parity. Recoverable OOM behavior, the complete
dynamic Function-family grammar, bytecode debug serialization, and most of the
language and builtin library remain unfinished. See
[`docs/parity.md`](docs/parity.md) for the full completion contract and
[`docs/status.md`](docs/status.md) for the current audited boundary.

## Design constraints

- Product crates are Rust-only: no QuickJS C source, generated bindings, FFI,
  or native QuickJS linkage.
- The compiler targets a stack bytecode VM, following QuickJS's execution
  model rather than interpreting an AST.
- Runtime and context/realm lifetimes remain distinct.
- Atoms, shapes, property descriptors, deterministic reference counting with
  cycle removal, jobs, modules, RegExp, Unicode, BigInt, and the standard
  library are parity requirements rather than optional extensions.
- A separately installed upstream `qjs` may be used only as a differential-test
  oracle.

## Development

```sh
cargo test --workspace --all-targets
cargo run --bin qjs -- -e '(function(a) { return a + 1; })(41)'
```

`qjs -e` intentionally follows upstream and does not print the expression's
completion value. To run the current differential suites against a separately
built official release:

```sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_primitives -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_boolean_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_number_parse_kernel -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_global_number_parsers -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_global_numeric_predicates -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_global_uri_codecs -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_global_to_string_tag -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_number_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_number_constructor_conversion -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_power_numbers -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_power_bigints -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_update_numeric_matrix -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_update_expressions -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_update_function_constructor -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_identifier_delete -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_error_stacks -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_vm_object_coercion -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_function_bind_to_string -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_function_debug_accessors -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_function_constructor -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_member_reads -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_member_writes -- --nocapture
```

Or run the complete current gate—including checksum-verified oracle setup,
formatting, tests, Clippy, and the Rust-only dependency audit—with one command:

```sh
./scripts/test-parity-slice.sh
```
