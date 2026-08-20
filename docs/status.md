# Implementation status

quickjs-oxide is an unsafe-free Rust rewrite targeting semantic Feature Parity
with QuickJS 2026-06-04. It is runnable on the command line and as the real
Rust/WASM engine in the GitHub Pages playground, but it is not yet at Feature
Parity.

## Current baseline

<!-- current-test262-metrics:start -->
The authoritative R3fj Test262 vector has:

- 79,982 full-corpus passes out of 102,037 variants (78.385%)
- 80,032 eligible variants out of 102,037 (78.434%)
- 79,982 passes out of 80,032 runnable variants (99.938%, secondary quality
  metric)
- 50 classified failures and no timeouts among eligible variants
<!-- current-test262-metrics:end -->

The pass count includes three exact `(path, variant)` results where Rust passes
a test listed in pinned QuickJS's known-error file. These narrow target
deviations are registered in [`deviations.md`](deviations.md); they do not imply
that the pinned engine passes those tests.

The exact profile, inputs, summary, line counts, and report hashes live in
[`dev-support/test262/current.conf`](../dev-support/test262/current.conf).

## Implemented architecture

- Rust compiler, verified bytecode, runtime, jobs, modules, and embedding API
- byte-exact Script and ECMAScript Module embedding APIs and loader payloads,
  with QuickJS-compatible malformed UTF-8, WTF-8/CESU-8, source retention, and
  diagnostic locations
- byte-exact strict and extended JSON module-loader payloads plus CLI file
  ingestion, preserving malformed-byte semantics and diagnostic locations
- binary data, typed arrays, shared memory, and Atomics slices
- collections, weak references, finalization, Promises, and iterator slices
- Unicode 17 case, identifier, normalization, and property data
- physical dense Array storage shared by literals, builtin results, and JSON
- public and private instance/static data fields, private methods and
  accessors across ordinary, generator, async, and async-generator forms,
  static blocks, and private brand checks with `#name in object`, including
  QuickJS-matched early errors
- static-module graphs, live namespaces, default exports, top-level await with
  async dependency/SCC scheduling, static import attributes with loader
  validation, strict and QuickJS-extended JSON synthetic modules, and
  canonical host-populated `import.meta` objects
- Script-goal dynamic `import()` with FIFO load/finish jobs, live host-loader
  callback sampling, import attributes, cached cycle-root evaluation Promises,
  namespace reuse, Promise assimilation, and exact propagation of arbitrary
  JavaScript values thrown by normalize, attribute-check, and load callbacks
- initiating-Context access for module-host callbacks, same-Context compiled
  module results, synchronous nested loader compilation with QuickJS-matched
  callback depth and evaluation order, and catchable native-stack exhaustion
- QuickJS-ordered parse-time module publication: the construction identity is
  cache-visible before the first token, each request prefix is visible before
  its attribute callback, successful completion preserves the same identity,
  and failed/referenced identities remain deterministic without unsafe raw
  pointer reuse
- native command-line execution, including byte-preserving file-module goal
  detection and filesystem dependencies, top-level-await settlement, and
  `import.meta` `url`/`main`; qjs-compatible side-effect-free structured
  `print`/`console.log` output with byte-exact WTF-8 String transport; plus a
  Rust/WASM browser playground
- a narrow trusted-bytecode Rust API for the pinned BC5 branch-free scalar
  Script cohort: `undefined`, `null`, booleans, the complete direct Int32
  family, signed-i32 BigInts, and single String values, plus exact Float64,
  arbitrary-precision signed BigInt, and String values behind an
  index-zero/one-entry constant-pool pair. Any admitted push may carry a finite
  chain of pinned scalar unary operations (`neg`, unary `plus`, `dec`, `inc`,
  bitwise `not`, logical `not`, and `typeof`). It completes the compatible
  whole-image read, translates an inert DTO to typed Rust instructions and
  primitive constants, and enters the ordinary verifier and transactional
  publication path before execution. Constant-pool Strings retain primitive-
  constant identity, ordinary String atoms use the runtime's canonical atom
  identity, empty direct/atom Strings share that canonical empty identity, and
  tagged-integer atoms produce a fresh decimal String on every execution.
  Private and Symbol atoms remain understood but unadmitted
- a second narrow trusted-bytecode API for one ordinary synchronous function
  selected from a compile-only root constant pool. It admits primitive
  constants, arguments and locals, direct and expanded stack operations, unary,
  postfix, binary, and HTMLDDA-aware predicates, plus bounded conditional/loop
  control flow, direct `return_undef`, and exactly the five plain-call physical
  rows through a distinct verified publication role. Raw 34 `call` carries its
  `NPop` argument count, while raw 236-239 `call0`-`call3` carry their implicit
  `NPopX` counts; all publish as `Call(argc)` with an undefined receiver.
  Stage 3A additionally admits raw 33 `call_constructor`, raw 36 `call_method`,
  and raw 38 `array_from`, preserving each `NPop` count through a distinct
  typed instruction. Stage 3B admits raw 39 `apply` through a typed
  `Call`/`Construct` kind: only canonical `U16` magic 0 and 1 cross the archive
  boundary, while 2 and 65,535 are rejected before publication. Stage 3C
  admits raw 35 `tail_call` and raw 37 `tail_call_method` as distinct terminal
  `NPop` instructions with their argument counts preserved end to end. Stage
  3D admits operand-free raw 48 `throw` as an explicit terminal completion,
  preserving the thrown value through the typed archive/publication chain into
  the engine's existing `Instruction::Throw` path. Stage 3E admits only raw 49
  `throw_error` subtype 0, retaining its String atom spelling and provenance
  through an owned diagnostic-name DTO before publishing the engine's existing
  `Instruction::ThrowReadOnly` path. Subtypes 1 through 255 remain unadmitted.
  Stage 3F additionally admits operand-free raw 177 `nop` through exact unit
  `Nop` DTOs into the engine's existing `Instruction::Nop`; raw 47
  `return_async` remains blocked.

The public API and Test262 runner now report the same engine diagnostics.
Detached public bytecode/VM execution has been retired, the Test262 runner
loads its profile and exact admissions from hash-authenticated data instead of
compiling milestone identity or admission tables, and its `$262` realm/agent
host is isolated behind a non-default feature.

## Remaining parity work

Major open frontiers include remaining module-host lifetime and
allocation-failure edge matrices and the unsupported/failed leaves recorded by
the current Test262 vector.
The private BC_VERSION 5 foundation now has bounded wire primitives and the
pinned BigInt payload codec, including QuickJS's asymmetric 16,385-limb writer
edge. A heap-independent WireGraph slice now validates and canonically rewrites
primitives, ordinary objects, arrays, ArrayBuffers, TypedArrays, primitive
wrapper ObjectValues, Dates, shared identity, and cycles with explicit decode
and emitted-traversal budgets. Its data-object semantics include header atom
interning, tagged decimal keys, first-slot/last-value duplicate properties,
compatible null-atom consumption, depth-first output atom rebuilding,
fixed-versus-resizable ArrayBuffer state, and per-buffer plus aggregate
current-backing-store byte limits. All 12 pinned TypedArray kinds preserve
view/backing identity, byte offsets, element counts, alignment, and
current-byte-length bounds. The writer also preserves QuickJS's observable
RAB-shrink asymmetry: it can emit a zero-length out-of-bounds view that the
reader itself rejects. ObjectValue preserves Boolean, Int32/Float64 bit-level
Number, narrow/wide String, and canonical BigInt wrapper payloads. Its reader
also matches QuickJS's asymmetric `JS_ToObject` path: object children reuse the
existing NodeId and append another reference-table entry, including pending
Ordinary/Array ancestors, while the canonical writer rebuilds identity without
the redundant tag. Date keeps its own identity and the exact Int32-versus-
Float64 payload representation. Reader-only Float64 values retain `-0`,
infinities, subnormals, and every NaN payload bit without applying the normal
JavaScript constructor's `TimeClip`; non-number children are rejected only
after their complete subtree has been read, matching QuickJS's allocation and
diagnostic order. A later heap materializer must retain that numeric wire
representation and install it without reusing the runtime's `TimeClip` Date
constructor path. TemplateObject now preserves the dense cooked-element
sequence, its mandatory raw wire child, preorder identity, and cycles through
either child. Elements consume the same per-container and aggregate budgets as
Array elements; the fixed raw slot does not. Pinned QuickJS consumes an
undefined raw child but omits the own `.raw` property, and prevents extensions
on every decoded template Array. Its indexed properties are enumerable but
non-writable and non-configurable; a defined `.raw` has all three flags clear.
Unlike a language-level tagged-template object, its Array `length` remains
writable. The later heap materializer must reproduce those descriptor
decisions and must not reuse `template_object::seal_template_array`, which
intentionally makes the language-level template length non-writable.
QuickJS's writer selects this tag only for a non-extensible Array under the
bytecode flag, so the eventual public writer must retain that flag-dependent
selection even though the pure graph layer can canonically re-emit an
explicitly represented tag. BC5 atom handling now has an explicit
caller-selected data or bytecode namespace, a release-pinned 242-entry
predefined-atom catalog, and separate checked codecs for metadata ULEB atoms
and raw `u32` opcode operands.
A separate inseparable transport input now admits pinned QuickJS
SharedArrayBuffer records without persisting or interpreting their native
pointer tokens. It binds wire bytes to the writer's per-record occurrence
table, rejects missing, extra, reordered, or mismatched entries, and
alpha-renames equal native tokens to archive-local backing IDs. Distinct SAB
wrapper nodes can share one pointer-free backing descriptor, while
ObjectReference preserves wrapper identity without consuming another side
entry. Record occurrences, unique backings, per-backing capacity, and aggregate
unique capacity are independently bounded. TypedArray may reference an
archived SAB and uses its wrapper's current byte length for layout checks.
Growable records remain archiveable even though pinned QuickJS's native reader
later rejects an externally managed resizable buffer. The ordinary decoder
keeps SAB disabled, and ordinary graph/bytecode-image writers return an
explicit archived-backing-context error rather than emitting a stale or
fabricated pointer token. No production constructor for native occurrence
tokens exists in this archive-only milestone; a later dedicated host bridge
must own that authority before runtime materialization is admitted.
The data graph uses that shared namespace without auto-detection or local
`first_atom` arithmetic, while an authenticated differential gate checks every
catalog ID, kind, spelling, and ordering against the pinned `quickjs-atom.h`.
The final function-code ABI now has a separately authenticated 244-entry
opcode catalog, including 66 short opcodes whose first 19 wire IDs overlap
QuickJS's 19 temporary compiler descriptors. A bounded, heap-independent
scanner safely rejects the reserved `invalid` opcode, unknown opcodes,
truncated instructions, offset overflow, and independent
byte/instruction/relocation budget excesses.
Both authenticated catalogs share one fail-closed C/Rust source inspector;
production manifest attributes and imports are exact allowlists, while
conditional compilation, external crates, macros, and all exclamation tokens
are rejected before either frozen digest is accepted.
It records structural instruction spans, resolves all 21 fixed-width atom
operands into typed bytecode-namespace identities, preserves QuickJS's
end-of-payload invalid-atom diagnostic position, and can canonically re-encode
in the same namespace. The scanned `ImageCode` remains deliberately
non-executable: it does not validate stack or control-flow semantics, create
runtime atoms, or bypass the existing verified-bytecode publication path.
A private `native_plan` archive stage derives a typed, non-executable
instruction/PC plan from an authenticated function. Its table
covers all 29 pinned operand formats and every release-pinned descriptor
outcome; it authenticates instruction sidecar offsets, opcode bytes and widths,
the exact ordered atom-relocation bijection, each relative-label operand base,
and in-range instruction-boundary targets. Raw `ImageAtom`/`PinnedAtomId`
identities and native code bytes never enter its DTOs: atoms become sealed
semantic class, index, spelling, and input-table-provenance projections. The
stage imports no engine `Instruction`, heap/VM type, or runtime
`JsString`/`Value`. A private, raw-indexed `function_translate` registry is now
the plan's sole production consumer. It checks all 244 pinned descriptor
formats and projects only sanitized semantic DTOs to those admission bridges.
The scalar policy remains 30 opcodes; the stage-3G ordinary policy is 130,
and their union is 131 (113 blocked, one scalar-only, 101 ordinary-only, and 29
shared registry rows). The reviewed stage-one 57-row atom-free set, stage-two
five-row plain-call set, and stage-3A raw 33, 36, and 38 set are unchanged;
stage 3B adds exactly raw 39 `apply`, and stage 3C adds exactly raw 35
`tail_call` and raw 37 `tail_call_method`; stage 3D adds exactly raw 48
`throw`; stage 3E adds exactly raw 49 `throw_error` with its `AtomU8` subtype
fixed to zero; stage 3F adds exactly raw 177 `nop` with its `None` operand;
stage 3G adds exactly raw 11 `object` with its `None` operand.
Raw 47 `return_async` remains blocked. The now-empty `Invocation` and
`Exception` blockers are removed; the
blocked frontier retains 15 typed categories, each with at least one row. In
the registry's typed-category order, its exact count vector is
`1, 7, 2, 1, 3, 7, 16, 15, 25, 4, 9, 11, 5, 4, 3`.
The scalar and ordinary paths no longer own duplicate opcode-name lowering
tables. The plan and translation remain runtime-independent archive stages;
only the separate publication bridge creates executable instructions. This
stage changes no public API, source syntax, Test262 profile, or Feature Parity
claim.
A bounded `FunctionRecordPrefix` layer now reads and canonically writes the
fixed FunctionBytecode body after tag 12: flags, frame metadata, locals,
closures, scanned code, and optional debug bytes. It stops immediately before
the first of `pending_constant_pool_count` recursively encoded values and never
admits the record to execution. A complete, bounded `BytecodeImage` reader now owns
the remaining traversal. It reads the bytecode header once, normalizes numeric,
predefined-string, narrow/wide, and duplicate slots into one semantic atom
namespace, and immediately relocates every function metadata and opcode atom.
It preserves private and symbol identities and QuickJS's strict-reject versus
compatible-omit disposition for null property keys. Strict mode rejects
aliased narrow fields and reserved flag bits; compatible mode preserves
QuickJS's `u32`-to-`u16` truncation, while signed negative-size and
decrement-overflow spellings remain hard safety rejections.

The whole-image reader uses one heterogeneous frame stack, one `DataMachine`,
one `ObjectArena`, and preorder function and module tables across the root,
every constant pool, every request-attributes value, and every module function
object. FunctionBytecode and Module records never consume object-reference IDs.
Their frames retain linear, source-bound data completions until a consuming
whole-image finalizer unwraps every nested value by move. Function and Module
IDs carry the same non-wrapping machine-source token, are checked against
reserved slots before publication, and cannot index a different image.
Aggregate limits independently bound function and module counts, mixed
traversal depth, constant-pool entries, locals, closures, code bytes,
instruction spans, atom relocations, debug bytes, and all four module metadata
tables in addition to the per-record, wire, and graph limits. Each function
prefix and each staged module table receives the intersection of its
per-record cap and the remaining whole-image budget before table allocation or
payload copying/scanning. The completed `BytecodeImage` is deliberately
non-executable: it has no heap materializer, verifier bypass, or evaluation
entry point.
The matching canonical writer consumes that immutable image through a
source-bound authentication plan before exposing any bytes. Decode and encode
share the same whole-image totals, remaining-budget intersection, and
per-function-versus-aggregate error attribution. The plan rebuilds dynamic
atoms in QuickJS first-use order, including the request-name/attributes
continuation, regenerates opcode atom operands from typed relocations, assigns
object-reference IDs in one preorder spanning every nested value, and never
assigns those IDs to FunctionBytecode or Module records. Module writes preserve
unknown non-zero export types, normalize boolean bytes, and retain arbitrary
request attributes and function-object values without imposing linker-only
relationships on the archival codec.
Ordinary-object writes match `JS_WriteObjectTag` by omitting enumerable symbol
and private-name properties before their values can affect atoms, traversal,
references, or resource accounting. A complete encoded-size proof precedes
the final bounded little-endian emission; failed authentication never returns
a partial buffer. The canonical writer and general whole-image model remain
internal archival facilities rather than a general public bytecode surface.
The one public read path is deliberately narrower: after the entire image has
decoded in QuickJS-compatible mode, it accepts only a stripped Script root
with one completion local. Non-atom scalar forms still require zero input atom
slots and no native atom relocations. A `push_atom_value` String instead
requires exactly one authenticated relocation and at most one input atom slot;
when the slot exists, its raw operand must prove that it is the relocation's
source. The function constant pool is empty for direct and atom-value pushes,
or is the exact index-zero/one-entry Float64, BigInt, or String pair described
below. Its release-pinned direct scalar push is `undefined`, `null`, either
boolean, a signed-i32 BigInt, the empty String, or the complete Int32 family,
optionally followed by the authenticated unary chain and then
`set_loc0; return`. The Int32 path accepts
QuickJS-reader-compatible wider i8/i16/i32 spellings as well as
compiler-canonical short forms.

Single-String admission preserves QuickJS's distinct identity paths rather
than flattening them into one representation. A constant-pool String is a
primitive function constant: repeated executions of the same published
function reuse it, while separately loaded functions keep independent
constants. Ordinary predefined or dynamic String atoms canonicalize in the
runtime, including `push_empty_string` and atom-backed empty Strings. A tagged
integer atom bypasses that table and creates a fresh canonical-decimal String
on every execution. Null, Private, and Symbol atom operands remain unadmitted
and cannot reach publication. It does not establish Feature Parity and leaves
the authoritative Test262 vector and the metrics above unchanged.
The decoder and writer planner are physically split into shared-driver,
Function, and Module files while retaining one frame/task stack and one set of
atom, reference, preorder, and budget state. All binary-object submodules are
private. A self-testing architecture gate rejects VM/compiler and executable
bytecode dependencies, runtime consumers, crate-surface exports, and widened
module visibility before fast CI or the parity slice can proceed. Its sole
runtime-facing facade exposes an inert scalar draft to one sibling bridge; a
self-testing consumer gate rejects any second reader, raw image leak, heap/VM
dependency, verifier bypass, direct bytecode root construction, or alternate
publication path.
The same gate rejects shared-memory runtime types, `unsafe`, raw pointers,
`NonNull`, and native raw-ownership bridges anywhere in the archival codec.
An authenticated SharedArrayBuffer C oracle pins writer side-table order,
refs-on/off duplication and release counts, fresh-runtime aliasing, redacted
wire shapes, and the growable writer/native-reader asymmetry. Rust transport
tests consume those three exact shapes with non-zero typed tokens and prove
that changing the native token leaves the completed archive identical.
An authenticated public-C-API oracle pins stripped `42;` as a 25-byte
BC5 vector, reads it in a fresh QuickJS runtime, evaluates it to 42, and gates
both the Rust codec and trusted scalar execution path against the exact bytes.
The same table-driven oracle pins compiler-canonical `push_minus1`,
`push_0..7`, `push_i8`, `push_i16`, and `push_i32` transition boundaries plus
valid non-canonical i8/i16/i32 reader spellings. It also pins exact BC5 and
fresh-runtime type/value receipts for `undefined`, `null`, `push_false`,
`push_true`, `push_bigint_i32`, and `push_empty_string`; Rust admits the full
signed Int32 and direct signed-i32 BigInt ranges on those direct zero-slot
forms.
The expanded table also pins canonical and reader-compatible single-String
forms: narrow Latin-1 and wide UTF-16 constant-pool payloads, ordinary pinned
and dynamic atoms, compatible atom-slot rewrites, and tagged-integer atoms.
It covers embedded NULs, astral pairs, lone surrogates, and the Private/Symbol
boundary without treating those Symbol-valued operands as Strings. A
non-address C-oracle matrix separately pins the cpool, ordinary-atom, empty,
and tagged-integer representation-identity relationships used by Rust.
Float64 admission remains a separate exact pair: canonical `push_const8 0` or
reader-compatible `push_const 0`, exactly one `BC_TAG_FLOAT64` pool entry, and
no other constant-pool shape. The oracle pins compiler wires for `0.5`, the
first value above signed Int32, the minimum subnormal, the maximum finite value,
and positive infinity; compatible wires additionally preserve positive and
negative zero, integral Float64 values, both infinities, and quiet/signaling NaN
payloads on the pinned 64-bit QuickJS build. Rust carries the authenticated
`u64` bits into `Value::Float` instead of applying numeric canonicalization;
32-bit QuickJS NaN-boxing representation parity remains outside this milestone.
BigInt constant admission uses the same atomic opcode/index/pool pair with
exactly one `BC_TAG_BIG_INT` entry. Pinned QuickJS emits this form for literals
outside signed Int32; Rust retains the reader-normalized, signed little-endian
payload in the inert draft, then uses the ordinary BigInt codec and constant
publication path. Zero, negative compatible payloads, the short/heap boundary,
arbitrary-precision values within the trusted-input cap, and compatible
redundant sign extension are covered. Every admitted scalar push may be
followed by any finite chain drawn from the pinned one-byte, stack-one-to-one
unary table: `neg` (`0x8a`), unary `plus` (`0x8b`), `dec` (`0x8c`), `inc`
(`0x8d`), bitwise `not` (`0x93`), logical `not` (`0x94`), and `typeof`
(`0x95`). The inert draft keeps the starting value and ordered operation slice
separate. Admission authenticates every instruction sidecar's opcode and
offset against its owned raw byte, plus the final `set_loc0; return`; the
pinned descriptor gate separately locks each unary operation's one-to-one
stack effect. Publication emits the corresponding typed instructions in source
order. No unary result is computed while decoding or lowering, so BigInt unary
`plus` is accepted and raises its `TypeError` only when executed, before any
later operation in the chain.

The authenticated function-bytecode C oracle exercises 42 synthesized unary
vectors: 40 compatible inputs and two explicitly outside Symbol-atom inputs.
Every vector survives read/write identity and then executes in a fresh pinned
QuickJS runtime. The matrix covers all seven operations, Number and BigInt
boundary tags, exact Float64 bits, String `ToNumeric`, scalar truthiness,
mixed/double chains, the execution-time BigInt unary-plus diagnostic, and
non-address `typeof` String identity within and across runtimes.

Numeric execution preserves QuickJS representation details: native Float64
inputs retain their Float64 tag and payload behavior, Int32 overflow boundaries
promote to Float64, and BigInt arithmetic stays in the BigInt domain. `typeof`
returns one of eight construction-preloaded, runtime-canonical atom-backed type
Strings, shared within one runtime but not across runtimes. Heap-BigInt
decrement also retains this pinned QuickJS release's unsigned opcode-enum
arithmetic quirk instead of silently
substituting spec-correct subtraction. Recoverable allocator-failure parity
remains part of the later `num-bigint` hardening gate.

Postfix update operations, reference/local mutation, `delete`, `void`, `await`,
and Object, Private, or Symbol inputs remain outside this scalar-only cohort.
That was the last scalar-specific admission milestone. The first broader step
now admits one ordinary synchronous FunctionBytecode leaf selected by constant-
pool index from an authenticated compile-only root. The child must retain the
reviewed normal/simple-parameter/prototype metadata, null function/local names,
no debug info or closures, no modules or object identities, no input atom table
or variable references, and only primitive constants plus the exact canonical
empty atom String. Its typed cohort covers all admitted atom-free primitive
pushes; 13 direct stack operations and six exact multi-instruction expansions;
the seven unary,
two postfix, and 20 binary operations; five tag/`typeof` predicates;
`if_false`, `if_true`, `goto`, `return`, and zero-stack `return_undef`. Direct
signed-i32 BigInt and empty-String pushes append stable synthetic constants
after the original pool. Native branches are reindexed through cumulative
expanded output positions. Stage two admits exactly raw 34 `call` (`NPop`) and
raw 236-239 `call0`-`call3` (`NPopX`) as plain calls. Their operand is the
argument count itself, never count plus one: the callee and those arguments are
consumed, the receiver is `undefined`, source argument/callback order is
preserved, and one return value is produced. Stage 3A admits three non-tail
invocation operations without widening that plain-call contract. Raw 33
`call_constructor` preserves distinct constructor and `newTarget` values plus
the ordered arguments; raw 36 `call_method` preserves the original receiver for
strict methods; raw 38 `array_from` preserves element order and creates a fresh
Array on every execution. Their `u16` operand is passed through unchanged at
each translation and publication boundary. Stage 3B adds raw 39 `apply`, whose
three stack inputs produce one result. Its raw `U16` magic becomes a typed
`Call` for 0 or `Construct` for 1 through the translation, ordinary-draft, and
publisher boundaries; every other value, including 2 and 65,535, is
`Unadmitted` before heap, atom, pending-exception, or bytecode publication.
Pinned QuickJS checks function callability first. A null or undefined argument
list then takes an ordinary zero-argument call for either magic, using the raw
second operand as `this` and leaving `new.target` undefined. A non-null list is
built before constructor capability is checked, and construct mode forwards
the raw second operand as `newTarget`. Stage 3C admits the remaining invocation
rows: raw 35 `tail_call` consumes `function, args...`, calls with an undefined
receiver, and raw 37 `tail_call_method` consumes `receiver, function, args...`
without changing that receiver. Both preserve the `NPop` argument count through
`Recipe`, `FunctionOp`, `OrdinaryLeafOp`, and `Instruction`; the verifier models
them as `argc + 1 -> 0` and `argc + 2 -> 0` terminal operations. They are not a
proper-tail-call or trampoline claim. The VM uses the ordinary recursive host
call path and immediately makes its returned completion the current frame's
completion. A normal return therefore exits the frame without executing a
following instruction, while a throw still attaches the current activation's
backtrace and follows its catch and iterator-unwind regions before escaping.
Stage 3D admits explicit raw 48 `throw` without widening that invocation
cohort. Its `None` operand is preserved as the exact unit variants
`Recipe::Throw`, `FunctionOp::Throw`, and `OrdinaryLeafOp::Throw` before the
publisher emits `Instruction::Throw`. The verifier consumes one value,
produces none, and enqueues no fallthrough, so unreachable instructions after
the throw do not weaken its terminal completion. The existing VM pops that
same value into `Completion::Throw`; `execute` then routes it through `raise`,
which attaches an Error backtrace before transferring to a catch or closing an
active iterator. Iterator-close failure cannot replace the original pending
throw. An uncaught completion reaches the existing Context pending-exception
slot with primitive or object identity intact. This milestone changes no VM
instruction, unwind algorithm, or public API; it admits one pinned archive
opcode into the already implemented exception path.

Stage 3E admits raw 49 only as the typed chain
`Recipe::ThrowReadOnly` to `FunctionOp::ThrowReadOnly(AtomOperand)` to
`OrdinaryLeafOp::ThrowReadOnly(DetachedAtomName)` and finally
`Instruction::ThrowReadOnly` referencing one synthesized verified String
constant. The archive may declare zero or one input atom slot: a declared slot
must be used by this diagnostic, and the projected atom must be a String;
index, null, Private, Symbol, unused, and multi-slot forms remain unadmitted.
The subtype byte must be zero; every value from 1 through 255 is rejected before
publication. QuickJS raw 49 consumes no stack value and is terminal, so the
verifier models `ThrowReadOnly` as 0-to-0 and enqueues no fallthrough. The VM
does not pop: its existing read-only-error hook resolves the verified String,
creates the defining-realm `TypeError`, and enters the same materialize,
backtrace, catch, iterator-unwind, and pending-exception chain already used by
other native errors. This milestone adds no VM instruction, unwind algorithm,
public API, source syntax, Test262 admission, or Feature Parity claim.

Stage 3F admits raw 177 only as the exact one-to-one typed chain
`Recipe::Nop` to `FunctionOp::Nop` to `OrdinaryLeafOp::Nop` and finally the
existing `Instruction::Nop`. Its `None` operand is never aliased to another
recipe, erased, expanded, or given a synthetic constant. The source-to-output
instruction map retains one output index for the Nop, so a branch landing on
raw 177 still targets that typed instruction. The engine stack model remains
0-to-0 and the verifier follows the ordinary successor edge: a Nop followed by
raw 41 `return_undef` verifies, while a reachable raw177-only fallthrough is
rejected as `bytecode ended without return`. The VM's existing empty Nop arm
does not pop, push, call the host, complete the frame, or alter pending state.
Stage 3F changes neither production bytecode nor VM implementation; it also
adds no public surface, source syntax, Test262 admission, or Feature Parity claim.

Stage 3G admits raw 11 only as the exact one-to-one typed chain
`Recipe::Object` to `FunctionOp::Object` to `OrdinaryLeafOp::Object` and
finally the existing `Instruction::Object`. Its `None` operand and
ordinary-only audience are never aliased, widened, erased, expanded, or
remapped. The source-to-output map retains one typed instruction index, and
publication neither drops the operation nor consumes a synthetic constant
index. `Instruction::Object` remains a 0-to-1 stack operation; the verifier
follows its ordinary successor, requires declared maximum stack one, and
rejects reachable raw11-only fallthrough. The existing VM Object arm delegates
once to the executing bytecode's realm, pushes the returned fresh ordinary
Object, and propagates a host throw without substituting a value. That host
allocates through the current defining realm, so cross-realm callers still
receive distinct, empty, extensible Objects rooted at the defining realm's
`Object.prototype` with no pending exception. Raw 47 remains blocked as
`Completion`. Stage 3G changes neither production bytecode nor VM
implementation. Stage 3G exposes no new source syntax, public API, Test262
admission, or Feature Parity claim.

The pinned QuickJS C oracle compiles a two-argument loop/branch function with
`GLOBAL | COMPILE_ONLY` and `JS_STRIP_DEBUG` into an exact 119-byte root/child
vector. It pins byte-identical read/write, child offset 25 and flags `0x0243`,
two arguments, two locals, two Float64 constants, 46 code bytes and 38
instructions, plus all five native branch targets. Fresh-runtime calls return
42 for `(3, 3)` and 0 for `(3, 4)` with exact integer tags. The oracle also
locks constructor/prototype identity and sloppy-versus-strict `caller` and
`arguments` behavior. Public `JS_WriteObject` rejects the evaluated child
closure with the exact `TypeError: unsupported object class`. Rust consumes the
same vector through `Context::read_trusted_ordinary_function`, maps every target
to an IR instruction index, and exercises both branches through the dedicated
verified publication path. Additional pinned cases cover the newly admitted
rows, all six stack expansions, expanded-branch reindexing, exact Float64 bits
and signed-i32 BigInts, canonical empty String, HTMLDDA predicate distinctions,
and zero-stack `return_undef`. The pinned upstream C oracle also discovers and
round-trips the exact five plain-call rows, executes argument counts 0 through
4, and records 36 undefined-receiver observations. The Rust public anonymous,
atom-free fixture independently locks `Call(0)` through `Call(4)`, strict
`this === undefined`, argument and callback order, and the returned value. It
also proves recoverable non-callable `TypeError`, clean pending-exception retry,
typed-verifier underflow and declared-`max_stack` rejection without heap, atom,
or pending-state publication, and a branch target landing on `Call` across an
existing multi-instruction expansion. Stage-3A public Rust vectors additionally
lock constructor/`newTarget` separation, allocated-result prototype identity
from `newTarget.prototype`, and argument order; exact strict-method receiver
identity; ordered zero- and three-element fresh Arrays; recoverable invocation
exceptions; verifier rollback; and a branch target landing on `CallMethod`
after an existing multi-instruction expansion. Stage-3B Rust vectors add exact
typed magic 0/1 admission and prepublication rejection of 2/65,535; dense call
and construct lists; both nullish-list shortcuts; poisoned-list and constructor
error order; raw primitive, ordinary-object, callable, bound, native, base,
derived, and Proxy `newTarget` propagation; calling-realm and native-family
prototype fallback; construct-only Proxy/species behavior without callable
narrowing; recoverable pending state; three-pop/one-push verification; and a
reindexed branch landing directly on `Apply`. Stage-3C Rust evidence pins the
compiler-natural 57-byte raw-35 BC5 function (FNV-1a-64
`8ff9d2c10c7e2228`) and the compiler-authenticated, property-free 62-byte
manual raw-37 function (FNV-1a-64 `e87d54c0a2a140ca`). Their published code is
exactly `GetArg(0), GetArg(1), GetArg(2), TailCall(2)` and `GetArg(0),
GetArg(1), GetArg(2), GetArg(3), TailCallMethod(2)`. Execution proves the
undefined plain-call receiver, exact strict-method receiver, argument order,
immediate frame completion, recoverable non-callable errors, verifier underflow
and declared maximum rejection with heap/atom rollback, and the existing
activation's backtrace, catch, and unwind behavior.
Stage-3D Rust evidence uses the compiler-natural strict 45-byte BC5 function
(FNV-1a-64 `73cf217e06c5fee2`) whose child begins at offset 43, has flags
`0x0243`, one argument, no variables or constants, maximum stack one, and exact
code `cf30` (`GetArg(0), Throw`). It locks byte-exact metadata and publication,
integer, ordinary-object, and Error identity, pending-exception clearing and
retry, backtrace attachment before a caller catch, iterator close before that
catch without exception replacement, direct terminal behavior, a reindexed
branch landing on `Throw`, verifier underflow/declared-maximum rejection, and
transactional heap/atom rollback.
Stage-3E Rust evidence distinguishes the compiler-natural 58-byte strict
const-assignment origin from the admitted 47-byte property-free derivation.
The natural wire (FNV-1a-64 `026914eda60a481f`, SHA-256
`a07b3f39a5e3929af4899a07686e91324e4ee9c54b729f518813eaa4a1875199`)
retains one lexical local and raw 94 and is therefore rejected by the ordinary
cohort. The mechanically derived wire (FNV-1a-64 `b4c1126c283093af`, SHA-256
`d05cabd4c18598b024f66eab8fd723c412fc5a469325b26fca5042507dea3ee8`)
retains exactly one used input atom slot, no locals or constants, maximum stack
zero, and child code `31f300000000` containing only raw 49/subtype 0. Tests lock
byte identity, String-only zero/one-slot provenance, rejection of every
nonzero subtype and non-String/unused/multi-slot atom form, publication through
one synthetic String constant, empty-stack verification, Unicode and lone-
surrogate name preservation, defining-realm `TypeError` identity, an own
backtrace before catch, direct pending publication and clearing, caller-catch
terminal behavior, and transactional retry after every rejected form.
Stage-3F Rust evidence uses the exact 41-byte property-free strict function
wire (FNV-1a-64 `1c522736e3cbef92`, SHA-256
`26c2e58ec14861dc797a7c3a3701f258ba392b649a15554256b61d7634fccdd0`).
It has no atoms, locals, variables, closures, or constants, maximum stack zero,
and child code `b129` containing raw 177 followed by raw 41. Publication keeps
both typed instruction indices and the defining Function prototype; repeated
calls from a second realm return `undefined` without pending exceptions. A
mechanically truncated 40-byte raw177-only form is rejected for reachable
fallthrough with exact heap/atom rollback before the valid wire retries, and a
separate branch fixture proves `Goto(1)` still lands on `Instruction::Nop`.
Stage-3G Rust evidence uses the compiler-natural exact 41-byte strict
object-return wire (FNV-1a-64 `3c41af3fef8b3a1e`, SHA-256
`a58ccbed5658ba6a9de99e909d5ba0b4af59ad47fccf0f5cccdff072d6494db9`).
It has no atoms, arguments, variables, locals, variable references, closures,
or constants; flags are `0x0243`, `js_mode` is strict, maximum stack is one,
the child code starts at offset 39, and exact code `0b28` is raw 11 followed by
terminal raw 40. Publication preserves `[Instruction::Object,
Instruction::Return]`, empty constants, and the defining Function prototype.
Four defining- and caller-realm calls return pairwise-distinct empty,
extensible Objects, all rooted at the defining realm's `Object.prototype`,
and neither context retains a pending exception. A 40-byte raw11-only
fallthrough and a maximum-stack-zero declaration are both rejected as
`Unsupported` with exact heap/atom rollback before the valid wire retries.
A separate branch fixture publishes `[Goto(1), Object, Return]` and proves the
typed raw11 index, fresh identity, defining prototype, and clean pending state.

The pinned stage-3A C oracle compiler-naturally emits the exact target raws 33,
36, and 38, with compiler union 17, 33, 36, 38, 40, 62, 155, 179, and
207-209, then locks whole-wire identity across read/write and fresh-runtime
execution. The natural constructor and Array cases are fully within the
admitted cohort. The natural method case also needs blocked property opcode raw
62, so public `CallMethod` evidence instead uses an authenticated property-free
manual child. Manual execution proves a constructor distinct from `newTarget`,
ordered arguments, and result prototype from `newTarget.prototype`; exact
strict-method receiver identity, argument count/order, and result; and distinct
empty and `[1, 2, 3]` Arrays with stable element order. At that milestone, raw
35, 37, and 39 were the explicit deferred frontier; Stage 3B moves only raw 39.

The pinned Stage-3B C oracle adds compiler-natural magic-0 and magic-1 `apply`
wires and four compiler-authenticated 58-byte property-free manual wires for
magic 0, 1, 2, and 65,535. Fresh-runtime read/write and execution lock nullish
ordinary calls, dense call/construct behavior, raw `newTarget`, callability and
list-building order, and construct-only Proxy trap order. Stage 3C extends that
oracle with the compiler-natural 57-byte raw-35 wire and 64-byte raw-37 wire.
The natural method wire also contains blocked raw 62 `get_field2`, so the
public raw-37 evidence uses a separately compiler-authenticated 62-byte,
property-free manual wire. Fresh-runtime read/write and execution prove both
terminal layouts have no trailing return, exact receiver and argument order,
non-callable error order, thrown callee/getter identity through catch, retained
callee-to-tail backtraces, and pinned QuickJS `InternalError: stack overflow`
under recursive raw 35 and 37 calls rather than PTC. Stage 3D
adds the exact compiler-natural strict 45-byte raw-48 wire, FNV-1a-64
`73cf217e06c5fee2`, SHA-256
`b7998b9678635e7e0a4eb2e465b683d168395adc7f156f733c25521907e3c8a8`,
and child metadata/code `flags:0243`, one argument, stack one, no constants,
offset 43, and `cf30`. Fresh-runtime read/write identity and execution prove
C-API pending-exception clearing, integer/object/Error identity, Error
backtrace attachment before catch, terminal no-return behavior, and iterator
close ordering that preserves the original exception. Raw 49 `throw_error`
was explicitly deferred at that milestone. Stage 3E adds the exact
compiler-natural 58-byte raw-49 origin and exact mechanically derived
47-byte property-free wire described above. Fresh-runtime read/write and
execution lock subtype-0 `TypeError`, Unicode spelling, defining-realm identity,
own backtrace, pending-exception clearing, caller-catch no-return behavior, and
the executable subtype-1 `SyntaxError` and subtype-255 `InternalError` oracle
contrast while Rust admits only subtype 0. Stage 3F authenticates the
compiler-natural strict empty-function baseline as the exact 40-byte raw41-only
wire (FNV-1a-64 `bb77ba50387051a2`, SHA-256
`a50422c2b092ab4162505321642241e7d24c43c5617e4b4ef0d076cde44b6f92`).
The oracle mechanically changes child `code_len` from one to two and inserts
raw 177 before the natural raw 41, producing the exact 41-byte wire (FNV-1a-64
`1c522736e3cbef92`, SHA-256
`26c2e58ec14861dc797a7c3a3701f258ba392b649a15554256b61d7634fccdd0`).
Parsed metadata pins zero atoms, constants, locals, arguments, and stack.
Fresh-runtime read/write is byte-identical; repeated defining- and caller-realm
calls return `undefined` with no pending exception. Raw 177 is explicitly never
claimed compiler-natural, and the malformed raw177-only wire remains a Rust
verifier negative that C never reads or executes. Stage 3G additionally
compiler-naturally emits the exact 41-byte strict
object-return wire (FNV-1a-64 `3c41af3fef8b3a1e`, SHA-256
`a58ccbed5658ba6a9de99e909d5ba0b4af59ad47fccf0f5cccdff072d6494db9`)
under `GLOBAL | COMPILE_ONLY` with `JS_STRIP_DEBUG`. Its property-free metadata
pins stack one, code offset 39, and code `0b28`; fresh-runtime read/write is
byte-identical. Repeated defining- and caller-realm calls produce four distinct
empty extensible Objects whose prototype is the defining realm's
`Object.prototype`, never the caller's, with no pending exception. Strict C11
produced a byte-identical 1,474-line transcript; all 19 authenticated oracles
pass both direct validation and the full oracle gate. The source SHA-256 is
`dad41b3667a1a8301e67feaeaf1c0732fc1da3f4e9f29e60d093e1b6ccfb6846`, the
transcript SHA-256 is
`81a76b22c3bf897655296f37a93babe44fe167d58f48bc0d745ce8b22ea1af2b`, and the
oracle-manifest SHA-256 is
`ec87164ad79e8866c36953a9006beb97c579e797893fc23a06cc3432edeadc5c`.

The latest full R3fj execution, exact-source GitHub Actions run `32387004558`,
job `96483833519`, authenticates Stage 3G source
`c4321142ff3ba28376fe1d28d8e65b915f44356c` with engine fingerprint
`604438cd3a131ee7799ffcea40cc53963737d6196f14e038030517d33554e4e5`.
The run reached only the expected stale-receipt checksum failure, while its
always-upload artifact `9414084897` (SHA-256
`af613111e1d67c48abb8b4013b5b4ce17462e82cd654bf231f9115697e20d6e8`)
records the 102,050-line TSV as
`063cf74bb6082fc1a98a63dd839559068e34ae30fdae1dd48d5dfacb655cfb7c`
and the 102,039-line JSONL as
`ac69fbb72ce5d6df5de6218b49244ce339c76fc9bef9bcba2312e57ea8b18004`.
Each full receipt contains the Stage 3G fingerprint exactly once and no Stage
3F fingerprint. Fingerprint-only normalization replaces that single occurrence
with the authenticated Stage 3F fingerprint
`cd14be7aca9f4cfbbb7f9f38b3b5ab6e020710b70adea602e372ce1fb07e6d52`
and makes both files byte-for-byte identical to Stage 3F run `32367564952`,
artifact `9406416671` (SHA-256
`e817c07c41e72f181816b44b5ff610b18a29499e4bba92ad9b6801772167f198`):
all 102,037 classified outcomes, 79,982 full passes, and 80,032 eligible
variants are unchanged. The refreshed 6,844-pass focused TSV and JSONL are
byte-identical on exact-source replay at hashes
`277c5a5b083e31bca0364733666be9271d2e33bf8dc36357814df8a91e1154b5`
and `bbbb14874de7152884b8431bc5da70b69c81b5f112b039b4693b64161ee7fb52`.
This promoted receipt is source-current for Stage 3G and covers the raw-11
Object admission and its Rust/C evidence without changing the Test262 profile
or any focused or full metric reported above. It remains the exact Stage-3G
lifecycle boundary, retains the Stage-3F raw-177 coverage, and makes no new
conformance claim.

The same oracle pins compatible 32-bit `scope_next` wrapping, exact
`SyntaxError` diagnostics for wrong-version, truncated, malformed-ULEB, and
invalid-atom inputs, `InternalError` for an oversized string declaration and
the allocation failure from a signed high-bit bytecode length, and the
`TypeError`/`RangeError` branches for malformed ArrayBuffer and
TypedArray layouts. Rust maps these authenticated reader branches without
collapsing them into one error class; compatible high-bit metadata that
QuickJS accepts but this cohort cannot model, including Module indices,
remains non-JavaScript `Unsupported` with no pending exception. A second
authenticated 110-byte
vector pins a root-to-outer-to-inner constant-pool chain, the captured closure
descriptor, and fresh-runtime evaluation to 42. A third authenticated 75-byte
reference vector proves that neither the outer nor nested FunctionBytecode
record consumes an object-reference ID: a cpool TemplateObject is ID 1, its
raw object is ID 2, and the enclosing root later refers back to ID 1. A fresh
runtime also observes the cpool result and root property as the same object.
An authenticated 33-byte ancestor-reference vector adds the inverse topology:
an enclosing Ordinary object is reference ID 0, its FunctionBytecode property
has no reference ID, and that function's constant pool resolves
ObjectReference(0); fresh QuickJS execution proves `root.f()` is the identical
root object. A pinned-QuickJS C-oracle 50-byte whole-image vector combines the
stripped return-42 FunctionBytecode with a Uint8Array and two aliases of its
SharedArrayBuffer.
The function consumes no object-reference ID, the view and backing receive IDs
1 and 2, and both later aliases resolve `ObjectReference(2)`. Its sole native
token is authenticated against the writer side table and zeroed before the
wire is printed; a fresh runtime evaluates the function to 42 and preserves
the view bytes and all backing aliases. The Rust transport-aware whole-image
decoder now consumes that exact topology with a nonzero test token, proves
that token alpha-renaming leaves the pointer-free semantic snapshot unchanged,
and retains the function code, view/backing identity, reference numbering, and
one backing descriptor in a single `ArchivedBytecodeImage`. Additional
whole-image vectors prove that two complete SAB records with the same token
share one archive backing while distinct tokens retain two ordered backings.
Rust does not execute the transport archive's embedded function; its return-42
receipt remains the pinned QuickJS C-oracle result. Separate admitted scalar
Script and ordinary-leaf images are translated and executed by Rust through
their distinct verified publication roles. Authenticated negative vectors also
pin QuickJS's
three diagnostic classes when FunctionBytecode appears as the
child of ObjectValue, Date, or
TypedArray; a truncated-record probe proves that all three parents first decode
the complete function child before applying their typed rejection.
Writer-specific public-C-API vectors additionally pin nested keep-source,
strip-source, and strip-debug shapes. For pinned 2026-06-04,
`JS_WRITE_OBJ_BSWAP` does not change any of those output bytes, and both flag
forms load in a fresh runtime and evaluate to 42. A separate oracle constructs
enumerable string, symbol, and private properties whose two non-string values
share a circular object; bytecode writing without the reference flag still
succeeds, emits only `keep: 42`, and fresh-runtime inspection observes no
symbol or private properties. Rust tests reproduce that exact 13-byte
canonical output and verify that the skipped values are never traversed.
A Module-specific public-C-API oracle now pins a 109-byte stripped BC5 Module
through fresh-runtime read, byte-exact reserialization, resolve, evaluation,
and a global `42` receipt. Its `JS_WRITE_OBJ_BSWAP` bytes are identical. A
second 283-byte vector records the complete request/attribute, local and
indirect export, star export, default/named/namespace import, top-level-await,
and FunctionBytecode-body topology. The Rust whole-image reader and canonical
writer now preserve both vectors byte exactly without pretending that the
archival image can link or execute them yet.
The data decoder separates preorder identity registration from value
completion: every parent/root attachment now uses one completed-subtree
delivery path owned by the decode state. Its reference state is an independent
generic `ObjectArena`; the whole-image decoder carries one instance through
constant pools and module children without registering FunctionBytecode or
Module records themselves. The data-value and container state machine
is now independently generic over value and property-key carriers as
`DataMachine`/`DataFrame`. The data-only facade still owns its own header
interning, key timing, frame stack, root delivery, and unconditional cursor
finalization, and still rejects FunctionBytecode and Module without consuming
their payloads. The separate whole-image driver carries authenticated function
and module identities through ordinary properties, Arrays, and TemplateObjects
while reusing the same budgets and arena; TypedArray, ObjectValue, and Date
expose typed failures when such an identity is invalid in their child position.
The arena represents incomplete identities with kind-checked pending/ready
slots. Source-bound linear node reservations, opaque data frames, and linear
completed values prevent stale or
cross-machine commits; machine identities never wrap, and raw node values
cannot be rebranded as caller-produced opaque values. The value adapter is
sealed inside the graph reader, so sibling modules cannot substitute a
classifier which hides raw node identities. Atomic reference reservations keep
alias publication indivisible, while independently bounded
reference entries can alias pending or ready identities without consuming
another node.
Malicious TypedArray placeholder paths are rejected deterministically instead
of reproducing pinned QuickJS's native crashes.
The ordinary data-only graph facade still rejects SharedArrayBuffer,
FunctionBytecode, and Module before their payloads; only its inseparable
transport-aware counterpart admits SAB into `ArchivedWireGraph`. The ordinary
`BytecodeImage` reader still rejects SharedArrayBuffer, while its separate
transport-aware counterpart atomically binds the completed image to the
authenticated occurrence table as `ArchivedBytecodeImage`. Neither transport
archive exposes a bare graph/image or descriptor-table split, and neither
transport reader is a public binary-object API. The scalar API cannot consume
or expose either archive. The canonical image writer continues to reject every
reachable archived SAB because there is no live backing capability, ownership
callback bridge, or occurrence-side-table output; this milestone is decode,
not encode or round-trip support. A general heap materializer, broader
native-code translation, public QuickJS-compatible read/write flags, and a
public authenticated whole-image host bridge remain future milestones.
In addition,
`num-bigint` lacks fallible construction, so heap materialization, decoder OOM
mapping, and allocator fault-injection remain hardening gates before untrusted
input admission.
Failed acyclic source graphs retry like QuickJS. Parse-time resolution success and
one-shot failure latches match the pinned callback order; an incomplete graph
is non-executable and dynamic import rejects deterministically. Rust safely
uses an `Aborted` identity where pinned QuickJS can retain a dangling pointer
whose subsequent use enters native undefined behavior. Native crash and
allocator-aliasing probes are deliberately not automated. Reclaiming the
resulting vacant module-cache slots remains architecture work. A Feature Parity
claim additionally requires the acceptance contract in
[`parity.md`](parity.md), including QuickJS differential evidence and
non-Test262 behavior.

The R3en architecture-hygiene pass is complete. Cargo integration-test targets
fell from 186 to 5 (one shared oracle harness). Repeated runtime-completion,
value, property, CLI, and QuickJS transport helper families now share support
code; a token-level gate rejects the retired and shared-provider fingerprints
without conflating domain-specific prelude or spelling helpers. Path-sensitive
non-oracle tests remain separate, while `$262` oracle modules are feature-gated
inside the shared harness. Negative
diagnostics use a source-authenticated data contract. The ModuleImportBinding,
public-class-field, public-static-initialization, private-data-field, and
private-callable cohorts gate the exact QuickJS error message and line/column
policy as well as phase and type. Logical-assignment, optional-chain-assignment,
generator-yield collateral cases, and both direct and parenthesized
assignment-target cohorts discovered during admission are exact-contracted too.
R3el adds the QuickJS-matched single-statement function, lexical, and class
declaration diagnostics, plus strict-code `with` statement diagnostics.

The active tree now retains only 23 referenced `tests/test262-*` artifacts; 313
superseded manifests and ledgers are authenticated in the R3eh history release.
Fast CI rejects any new unreferenced Test262 bookkeeping file.

## Verification

```sh
cargo test --locked --workspace --all-targets
cargo test --locked --features test262-host --lib --bins
./scripts/test-quickjs-c-oracles.sh --check
./scripts/test-test262.sh --check
./scripts/test-test262.sh --focused
TEST262_WORKERS=2 ./scripts/test-test262.sh --full
./scripts/test-web-playground.sh
```

Historical milestone gates, profiles, result vectors, baselines, and the former
long-form ledgers are preserved in the release archive indexed under
[`dev-support/test262/archive`](../dev-support/test262/archive/index.tsv).
