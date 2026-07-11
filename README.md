# quickjs-oxide

`quickjs-oxide` is a from-scratch, memory-safe Rust rewrite of QuickJS. The
compatibility target is the upstream **QuickJS 2026-06-04** release and its
ES2025 behavior, not merely a JavaScript-like language.

The repository is still an incomplete rewrite. The current vertical slices
execute primitive expressions, named ordinary functions and closures, and
defining-realm global bindings through the real lexer, late scope resolver,
bytecode and VM. Block statements, `if`/`else`, `while`/`do-while`, classic
`for (;;)` and labeled statements with both named and unnamed
`break`/`continue` now share the QuickJS statement-parser spine;
scripts carry its hidden eval-completion local so empty blocks preserve a prior
value, `if` and `while` reset at their upstream points, and `do-while` resets on
every entered iteration. Break controls are isolated per function, distinguish
regular labels from loops, search outward for named targets, and preserve the
pinned release's direct-loop-label and multiple-label behavior. Closed
infinite-loop bytecode is verified without weakening reachable-fallthrough
checks. Classic `for` mirrors QuickJS's top-level-semicolon probe,
ExpressionNoIn propagation, update-block relocation and exact continue target.
`for-in`/`for-of`/`for-await`, switch and abrupt-cleanup constructs remain
explicit grammar slices.
Untagged template literals now follow QuickJS `js_parse_template`: the cooked
head is retained as the primitive receiver, `concat` is looked up exactly once
before any substitution, every substitution parses as a full Expression, and
one receiver-preserving call receives the raw values plus only non-empty later
cooked segments. Parser-selected template continuation goals preserve nested
templates, division, raw/cooked escape behavior, diagnostic priority, stack
limits, and the rule that synthetic concat operations do not overwrite an
inherited source marker. Expression statements seed that marker before parsing,
matching getter-fault sites after prior statements and inside composite
expressions. Tagged templates remain a separate explicit gap until the
Array-backed frozen template-object cache exists.
They also implement runtime-owned
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
complete Number, Boolean, Symbol, and BigInt slots enabled. The String slot
remains an explicitly incomplete stack built on the `String exotic
core/substrate`: it
roots a branded empty-string prototype object whose initial own `length` is
non-writable, non-enumerable, and configurable. Sloppy ordinary-function
boxing creates genuine UTF-16-backed String payload wrappers with a
non-writable, non-enumerable, non-configurable own `length`. Their virtual
code-unit index properties flow through the five get-own/define-own/has-own/
delete/own-key entry points, including numeric/string/symbol own-key ordering.
The adjacent String UTF-16 prefix installs QuickJS's first seven prototype
methods in exact table order: `at`, `charCodeAt`, `charAt`, `concat`,
`codePointAt`, `isWellFormed`, and `toWellFormed`. They preserve raw code-unit
indexing and lone surrogates, `JS_ToInt32Sat` index conversion,
`JS_ToStringCheckObject` receiver ordering, defining-realm errors, sequential
concat coercion and the `(1 << 30) - 1` code-unit limit. The additional String
conversion core installs the independently complete `toString` and `valueOf`
brand methods. The resulting ten-key list is documented only as an
implemented-key filter rather than the full 53-key table. String primitive
non-index lookup now walks the current bytecode realm's prototype;
primitive assignment preserves raw-receiver inherited setters and QuickJS's
`not an object` versus read-only split. String receivers also participate in
the three implemented Object-prototype routes, including defining-realm
boxing, observable `@@toStringTag`, cross-realm brand checks, and collection.
The shared `JsString` kernel now mirrors QuickJS's compact Latin-1/UTF-16
leaves and bounded rope representation: the 512/8192 flat-concatenation
thresholds, short-fringe merging, depth-60 limit, 44 Fibonacci rebalance
buckets, content equality/hash, cross-leaf UTF-16 access, and cached
linearization are implemented without putting ropes in the object GC graph.
VM `+`, `String.prototype.concat`, and the implemented native concatenation
sites share the checked 30-bit path; property-key/atom publication
linearizes by UTF-16 content. Public valid-UTF-8 and exact-UTF-16 dynamic
construction now enforce the same limit through fallible APIs; hostile
iterator hints cannot trigger an enormous eager reserve. A shared latched
UTF-16 builder covers backtraces and Annex-B escaping, while lexer String,
template and identifier buffers, dynamic Function source assembly, and URI
expansion use checked length paths with their distinct QuickJS error ordering.
Identifier scanning consumes the checksum-pinned QuickJS Unicode 17 compressed
`ID_Start`/`ID_Continue` tables plus the ECMAScript ASCII and join-control
rules. Direct and valid escaped BMP/astral spellings share the same binding,
retain their source spans, avoid normalization, and count the 30-bit buffer cap
in UTF-16 code units. Exhaustive scalar classification and compiler/VM
differentials are pinned to the official release. Compiler token consumption is
now parser-driven: fallible advances propagate only reached lexical errors,
unrecognized ASCII remains a raw token, and directive-prologue probes seek back
and rescan under the selected strict context. This preserves QuickJS's
transactional malformed-identifier-escape commitment and the tested reserved/
parser/lexer diagnostic priority, including exact line/column positions.
Contextual word
classification for the still-unimplemented module, generator and async grammar
will extend the same token path. The byte boundary now reproduces
`JS_NewStringLen`: embedded NUL,
WTF-8 surrogates, non-BMP pairs, legacy five/six-byte lead handling, and the
release's unusual invalid-byte replacement/skip rule are preserved. Fallible
owned-byte exporters implement `JS_ToCStringLen2` payload semantics in both
WTF-8 and CESU-8 modes, including cross-rope surrogate pairs and interior NUL.
Native Error construction now shares QuickJS's `char[256]` byte boundary:
the 255-byte payload may split UTF-8 before `JS_NewString` decoding, and the
implemented not-constructor `%s` path preserves WTF-8, C-string NUL and suffix
ordering. Sidecar-bearing native messages preserve exact raw bytes across
compiler/VM `Error` transport without round-tripping through Rust UTF-8. The
implemented atom-named Type, Reference and Syntax diagnostics also reproduce
`JS_AtomGetStr`'s `char[64]` formatting. For table-backed text atoms, only
narrow all-ASCII spellings bypass the scratch buffer; every other text spelling
is encoded one UTF-16 code unit at a time and stops before starting a unit at
byte 58, while `%s` NUL and literal-suffix ordering remain intact before the
outer 255-byte cap. This covers the current read-only, fixed-name nullish reads,
nullish writes, missing-binding, TDZ, VM read-only and reserved-identifier paths.
Context-level observable `ToString`, borrowed C-pointer/refcount ABI, native-error
callers belonging to unimplemented Array/private-field/module/global-
declaration surfaces, exact byte-sidecar migration for the remaining numeric-
parser and lexer diagnostics, and general recoverable allocator failures remain
unfinished invariants.
`%Number.prototype%` is the pinned boxed-`+0` Number-class object. The global
`%Number%` publishes the exact ordered 17/7 constructor/prototype surface:
call/construct and BigInt conversion, parser aliases captured by identity,
non-coercing predicates, frozen constants, radix 2–36 `toString`,
fixed/exponential/precision formats, `toLocaleString`, `valueOf`,
custom/cross-realm `newTarget.prototype`, and defining-realm fallback.
`%Boolean%` retains its exact boxed-`false` graph and call/construct behavior.
`%Symbol%` is call-only and rejects `new` before argument conversion; it
preserves absent versus empty UTF-16 descriptions, a runtime-wide `for`/
`keyFor` registry, and 13 runtime-unique well-known identities. Its ordinary
prototype publishes `toString`, `valueOf`, the `description` getter,
`@@toPrimitive`, and `@@toStringTag`; boxed Symbol payloads retain their atoms
through Object-prototype routes, cross-realm calls, and collection.
`%BigInt%` provides the pinned call-only constructor conversion, rejects `new`
before converting its argument, and preserves the release's signed-limb and
preallocation quirks in `asUintN`/`asIntN`. Its ordinary prototype publishes
`toString`, `valueOf`, `constructor`, and `@@toStringTag`; boxed BigInt payloads
and Object-prototype routes preserve the same brand, tag-deletion, cross-realm,
and collection behavior as upstream. Number, Boolean, Symbol, and BigInt
primitive member lookup and assignment traverse the current realm's matching
prototype without eager boxing and preserve the raw receiver for strict
inherited getters and setters; sloppy ordinary functions instead create one
cached wrapper per invocation. Their boxed instances participate in
`Object.prototype.toString`, `toLocaleString`, and `valueOf`, including
observable `@@toStringTag`, cross-realm boxing, and the runtime's
reference-counting/cycle graph. The global `%String%` constructor and the
remaining prototype methods outside the filtered ten-key UTF-16/conversion
surface remain explicit gaps, as do the global `Object` constructor and the
dependent Iterator, Array, RegExp, and Unicode layers.

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
reports `[object global]` while preserving symbol-category own-key ordering.
The later `globalThis` binding is a writable, non-enumerable, configurable own
data property that self-references its realm's global object. The implemented
relative constructor tail is `Number`, `Boolean`, `Symbol`, `globalThis`, then
`BigInt`.
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
`eval`, Proxy/exotic delete dispatch, the global String constructor, the
remaining 43 String-prototype own keys, the borrowed C-string embedding ABI,
and general recoverable string-allocation failures remain unfinished slices.
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
cargo test --test checked_string_construction
cargo run --bin qjs -- -e '(function(a) { return a + 1; })(41)'
```

`qjs -e` intentionally follows upstream and does not print the expression's
completion value. To run a curated set of the current differential suites
against a separately built official release:

```sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_primitives -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_boolean_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_symbol_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_exotic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_conversion_core -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_utf16_prefix -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_rope -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_byte_codec -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_native_error_format -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_native_error_atom_format -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_unicode_identifiers -- --nocapture
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
  cargo test --test oracle_global_this -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_bigint_intrinsic -- --nocapture
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
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_statement_control_flow -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_template_literals -- --nocapture
```

The 35 commands above are direct entry points for a curated evidence set. The
full gate below currently discovers all 43 `tests/oracle_*.rs` integration
targets through Cargo's `--all-targets` run, including suites not repeated in
this list.

Or run the complete current gate—including checksum-verified oracle setup,
regenerated-Unicode-table drift detection, formatting, tests, Clippy, and the
Rust-only dependency audit—with one command:

```sh
./scripts/test-parity-slice.sh
```
