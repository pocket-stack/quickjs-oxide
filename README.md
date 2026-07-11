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
slots for Number, String, Boolean, Symbol, and BigInt, while enabling only the
semantically complete Boolean slot. `%Boolean.prototype%` is the pinned
boxed-`false` Boolean-class object, and the global `%Boolean%` implements the
exact call/construct split, descriptors, `toString`, `valueOf`, wrapper
coercion, custom and cross-realm `newTarget.prototype`, and defining-realm
fallback. Boolean primitive member lookup traverses that realm's prototype
without eagerly boxing and preserves the raw receiver for strict inherited
getters and setters; sloppy ordinary functions instead create one cached
wrapper per invocation. Boolean wrappers also participate in
`Object.prototype.toString`, `toLocaleString`, and `valueOf`, including
observable `@@toStringTag`, cross-realm boxing, and the runtime's reference
counting/cycle graph. Number, String, Symbol, and BigInt constructors,
prototype roots, wrappers, and inherited lookup remain explicit gaps, and the
global `Object` constructor is not yet published; this is Boolean parity, not
five-class primitive-graph parity.

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
QuickJS `dtoa.c`: exact BigUint rational arithmetic implements decimal FREE
formatting today and has tested-but-not-yet-published radix 2–36, fixed,
exponential, and precision entry points ready for the complete `%Number%`
intrinsic. FREE uses ties-to-even shortest-roundtrip selection, while explicit
digit modes use ties-away-from-zero. A deterministic bit-pattern differential
locks those strings against QuickJS, and BigInt has a separately differenced
ties-to-even/overflow conversion for the future `Number(BigInt)` path. The
Number realm prototype remains absent until its parser aliases and full 17/7
constructor/prototype surface can be published atomically.
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
`eval`, Proxy/exotic delete dispatch, and the remaining Number/String/Symbol/
BigInt primitive prototype graphs remain unfinished slices.
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
