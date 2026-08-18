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
  constants, arguments and locals, arithmetic and comparison, and bounded
  conditional/loop control flow through a distinct verified publication role

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
`JsString`/`Value`. A narrow `bytecode_image` facade supplies those typed DTOs
to the scalar and ordinary-leaf admission bridges. The scalar path no longer
owns a duplicate native byte, width, instruction-sidecar, or atom-relocation
decoder, and the detached single-atom projection path has been retired. The
plan remains a runtime-independent archive representation; only the separate
publication bridge creates executable instructions. This adds no source
syntax, Feature Parity claim, or Test262 metric change, and a general
FunctionBytecode execution bridge remains later work.
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
or variable references, and only primitive constants. Its typed cohort covers
integer and constant pushes, argument/local get-put-set operations, `add`,
`sub`, `div`, `gt`, `strict_eq`, `if_false`, `goto`, and `return`. Native
branch destinations are resolved to instruction indices before the owned draft
crosses the archive boundary. A dedicated ordinary-leaf verifier then
authenticates the detached metadata and CFG before transactional publication
creates a callable closure.

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
verified publication path.

The authenticated R3fj receipt remains pinned to the preceding native-plan
source and is source-stale for this ordinary-leaf feature tree. Its 79,982 full
passes, 80,032 eligible variants, and 102,037 total variants remain the current
published metrics, not a fresh certification of this source. The Test262
profile is unchanged and an unchanged classified vector is expected, but only
a fresh full run can certify it. Current primary evidence for this feature is
the pinned C bytecode differential, Rust execution tests, and boundary gates.
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
