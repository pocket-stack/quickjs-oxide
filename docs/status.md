# Implementation status

Last audited: 2026-07-11. The completion definition remains
[`parity.md`](parity.md); this file records progress and must not be used to
claim full parity.

## Implemented on the final architecture path

- QuickJS 2026-06-04 release metadata, archive checksum, bytecode version,
  Unicode version, and Test262 commit are pinned in `compat/upstream.toml`.
- The lexer models parser-selected division/RegExp/template lexical goals,
  source spans and ASI trivia, contextual keywords, numeric/String/BigInt/
  template/RegExp tokens, UTF-16 escapes, comments, and punctuator longest
  matching. Unicode identifiers still fail explicitly rather than being
  accepted incorrectly.
- Runtime-local atoms preserve exact UTF-16 spellings, cover immediate integer
  atoms, string/global-symbol interning, unique/private/well-known symbols, and
  explicit retain/release. Safe handles carry a runtime domain and slot
  generation while raw table slots use QuickJS-style free-list reuse.
- Primitive values preserve compact integer vs float values, exact `-0`/NaN
  equality variants, Latin-1/UTF-16 strings including lone surrogates, and
  arbitrary-precision BigInts with QuickJS's short/heap normalization and
  2026-06-04 `asIntN`/`asUintN` behavior.
- The compiler builds a nested `FunctionIr` tree with unresolved identifier
  operations, then performs child-first QuickJS-style scope resolution before
  lowering to stack bytecode. In addition to the primitive expression grammar,
  the current source path supports anonymous and named ordinary function
  expressions, simple parameters, `return`/fallthrough, function-local `var`,
  simple/arithmetic/exponentiation/shift/bitwise/logical identifier assignment,
  prefix/postfix identifier and member updates, direct calls,
  transitive parameter/local and private function-name capture through
  `ParentClosure` relays, and QuickJS-style contextual `SetName` for direct
  anonymous initializers and assignments. Named expressions use a
  per-invocation private self binding; sloppy writes are ignored and strict
  writes raise the QuickJS-compatible read-only TypeError.
- Source `MemberExpression` lowering follows QuickJS's typed
  `GetField`/`GetField2` and `GetArrayEl`/`GetArrayEl2` split. Fixed and
  computed reads can be chained across line terminators; a following call
  rewrites only a live member Reference to the receiver-preserving form and
  then uses `CallMethod`. Parentheses preserve that Reference, while comma,
  conditional and logical values invalidate it. Computed reads evaluate the
  key expression but reject a null/undefined base before observable
  `ToPropertyKey(String)` conversion; getters and key conversion preserve
  arbitrary thrown completions and the original receiver. String primitives
  implement exact UTF-16 indexed own properties and `length`. Number, String,
  Boolean, Symbol and BigInt primitives additionally traverse the current
  bytecode realm's implemented matching prototype, preserving the raw
  primitive receiver for strict inherited getters and method calls. String's
  standard non-index surface is intentionally limited to the conversion pair
  until later table slices land.
- Simple member assignment mirrors QuickJS's lvalue rewrite rather than
  evaluating the getter: fixed targets lower through `Insert2; PutField`, and
  computed targets through `Insert3; PutArrayEl`, preserving the RHS as the
  expression value. Computed assignment deliberately delays observable
  `ToPropertyKey` until after the RHS, including for null/undefined bases.
  Ordinary setters receive the original base, discard normal return values and
  preserve throws; strict versus sloppy rejection distinguishes read-only,
  missing-setter and non-extensible cases. Number, String, Boolean, Symbol and
  BigInt primitive writes first walk their matching realm prototype, invoke
  inherited setters with the raw receiver, and preserve QuickJS's
  read-only/no-setter/not-an-object distinction before the strict/sloppy
  boundary. Member assignment does not apply identifier NamedEvaluation.
  Property `delete` rewrites both fixed and
  computed References to the common `Delete(base,key)` opcode, never invokes a
  getter, converts computed keys before ToObject, and implements strict/sloppy
  configurable behavior plus String's virtual index/length properties.
  Arithmetic, exponentiation, shift and bitwise member compound assignment
  (`+=`, `-=`, `*=`, `/=`, `%=`, `**=`, `<<=`, `>>=`, `>>>=`, `&=`, `^=`,
  `|=`) rewrites fixed getters to `GetField2` and computed getters to
  `GetArrayEl3`, so the old value and lvalue operands survive while an object
  key is converted exactly once before the getter and RHS. The arithmetic,
  exponentiation, shift or bitwise operator carries the compound-token source
  marker; `Insert2`/`Insert3` plus the same put opcodes preserve the final value
  and strict setter semantics.
  Logical member assignment (`&&=`, `||=`, `??=`) uses the same retained
  Reference, then matches QuickJS's `Dup`, conditional branch and `Nip`
  cleanup. The short branch returns the original value without evaluating the
  RHS or setter; the write branch preserves the RHS value. Unlike arithmetic,
  exponentiation, shift and bitwise compound assignment, the logical operator
  emits no new source marker.
- Identifier assignment keeps an unresolved tail Reference through parentheses
  and resolves it only after the full scope tree is known. Arithmetic,
  exponentiation, shift and bitwise compound assignment select
  `Get`/operator/`Set` paths for arguments, locals, closures and globals;
  logical compound assignment uses QuickJS's depth-zero branch with no `Nip`.
  Private named-function bindings
  preserve sloppy ignored writes and strict read-only throws. Direct logical
  assignment performs NamedEvaluation, including QuickJS's parenthesized-lvalue
  exception, while arithmetic, exponentiation, shift and bitwise compound
  assignment do not. Comma, conditional, bitwise and logical values are
  rejected as assignment targets, and strict `eval`/`arguments` lvalues are
  early errors at the upstream source position.
- Prefix/postfix `++` and `--` follow QuickJS's unary parser and lvalue rewrite
  rather than lowering to ordinary addition. Prefix operands use the zero power
  mode so `++x ** 2` updates before the outer exponentiation; postfix is
  accepted only without an intervening LineTerminator, including CRLF,
  U+2028/U+2029 and line-bearing block comments. Identifier updates resolve
  late across argument, local, closure, global and private function-name
  bindings. Fixed members retain `base, old` through `GetField2`; computed
  members retain `base, canonical-key, old` through `GetArrayEl3`, converting
  an object key exactly once. Prefix writes use `Insert2`/`Insert3` and preserve
  the new value; postfix `PostInc`/`PostDec` first preserves the converted old
  Numeric and uses `Perm3`/`Perm4` before the put. Number, BigInt, getter/key/
  coercion/setter ordering, strict/sloppy rejection, nullish fast checks,
  missing bindings, Function-constructor parsing and source markers are pinned
  to the oracle. BigInt decrement deliberately preserves the release's slow-
  path unsigned-enum quirk: short non-minimum values subtract one, while
  `i64::MIN` and heap values add `4294967295n` exactly as upstream.
- Binary nullish coalescing flattens a chain through QuickJS's shared
  `Dup; IsUndefinedOrNull; IfFalse` exit, preserving the first non-nullish
  operand without coercion and skipping every later operand. It has no
  operator source marker, invalidates a member Reference before an outer call
  or assignment, suppresses anonymous-function name inference, and enforces
  QuickJS's unparenthesized mixing boundary between `??` and `&&`/`||`.
- Bytecode publication first validates structural operands in every instruction
  (including unreachable code), then verifies reachable control-flow joins and
  stack depth. Runtime publication additionally checks constant kinds, frame
  indexes, private function-name source/name/const relay metadata, forbidden
  direct self-binding writes, Global/ParentGlobal versus ordinary closure-opcode
  categories, closure-name atom ownership, and relay consistency before changing
  the heap.
- Compiler output is first represented as a runtime-independent function tree,
  preflight verified, flattened without recursion, and then published as
  immutable runtime GC nodes. Bytecode nodes own their realm, constant-pool
  values and child bytecode; a 50,000-deep publication/release test covers the
  iterative ownership path.
- Primitive coercion, mixed BigInt comparison/equality, BigInt arithmetic,
  exponentiation, bitwise and shift operations, and string concatenation are
  covered by a real upstream-oracle differential suite. The implemented VM
  unary, arithmetic, exponentiation, bitwise, shift and relational operators
  route object operands through completion-aware Number-hint `ToPrimitive`.
  Decimal Number-to-string now routes through the shared safe-Rust formatter
  substrate rather than an external dtoa crate. Its exact BigUint rational
  rewrite follows pinned `dtoa.c` FREE RNDN selection and backs the published
  `%Number.prototype%` radix 2–36, FRAC/FIXED RNDNA, forced exponent,
  precision and `ToInt32Sat` paths. A pinned differential reconstructs
  85 raw binary64 bit patterns and compares 4,250 radix/fixed/exponential/
  precision strings, including subnormals, signed zero and non-finite values.
  BigInt-to-binary64 is a distinct ties-to-even path with signed-infinity
  overflow and its own constructor-oriented differential; ordinary `ToNumber`
  still correctly rejects BigInt. Global `parseInt`/`parseFloat` are now real
  realm-bound native functions with pinned name/length/global descriptors.
  Their shared UTF-16 substrate implements `ToString`-after-call prefix scans,
  parseInt's modulo-2^32 radix, radix 2–36, Infinity, signed zero and the full
  pinned `ATOD_MAX_DIGITS` table; this deliberately preserves QuickJS's
  observable non-power-of-two digit truncation rather than silently substituting
  another engine's rounding. Kernel and source-execution differentials compare
  raw binary64 results and complete native error frames; runtime tests also lock
  cross-realm error ownership and abrupt input-before-radix conversion. The
  complete Number graph captures those global parser callables by identity.
  Global `isNaN`/`isFinite` are distinct coercing natives from the static
  Number predicates: both apply completion-aware `ToNumber`, ignore `this` and
  extra arguments, preserve arbitrary conversion throws, and materialize
  framework errors in their defining realm.
  The next six global function-list entries implement URI encode/decode and
  Annex-B escape/unescape through a safe-Rust UTF-16 kernel. URI decoding
  preserves reserved `%XX` spelling for `decodeURI`, validates QuickJS's
  percent-encoded UTF-8 state machine and exact URIError messages; encoding
  validates surrogate pairing and emits uppercase UTF-8 escapes. The legacy
  pair deliberately works on individual code units and leaves malformed
  escapes literal.
  Unary `~` and binary `&`, `^`, `|` match QuickJS's signed modulo-2^32
  `ToInt32` Number path and its infinite-width BigInt two's-complement path.
  Right-associative `**` is parsed
  at QuickJS's unary level above multiplication, accepts a unary RHS, and
  rejects an unparenthesized unary LHS with the pinned early error. Its Number
  path uses Rust `f64::powf` plus QuickJS's `abs(base) == 1`/non-finite-exponent
  NaN correction, with pinned-oracle matrices locking the observed libc-`pow`
  results. Its BigInt path preserves negative-exponent errors, `0`/`1`/`-1`
  shortcuts, the `INT32_MAX` exponent ceiling, power-of-two exact allocation,
  and generic high-to-low square-and-multiply preallocation behavior. Binary
  `<<`, `>>`, and `>>>` occupy the QuickJS shift precedence level between
  additive and relational expressions. Their Number path masks a `ToUint32`
  count to five bits and preserves arithmetic versus unsigned results; their
  BigInt path supports negative-count direction reversal and huge-right-shift
  saturation. It also
  reproduces the 16,384-limb allocation guard and the pinned `js_bigint_extend`
  one-sign-limb bypass, including later allocation failures for the resulting
  16,385-limb value. After both operand expressions are evaluated, binary
  numeric operations complete the left `ToNumeric` before converting the
  right; exponentiation, bitwise and shift mixed Number/BigInt operands
  preserve the pinned error after both conversions. Unsigned right shift
  converts both operands before rejecting any BigInt with its distinct pinned
  TypeError. Relational comparison preserves the two-sided primitive-conversion
  order and uses `StringToBigInt` rather than Number rounding for BigInt/String pairs.
  Addition and abstract equality use the distinct default hint, preserve
  arbitrary thrown values, and keep QuickJS's observable conversion order.
- The runtime owns a generational Object/Shape arena. Public Object, Symbol and
  property-key roots implement Dup/Free through explicit reference counts;
  heap edges remain raw handles, zero-count teardown is iterative, and
  QuickJS-style trial deletion removes object/property/prototype cycles.
- Ordinary objects use immutable shared Shapes containing prototype plus
  ordered key/flag metadata and parallel per-object property slots. The current
  internal methods cover complete descriptor validation/storage, data get/set
  with explicit receiver, delete, own-key order, extensibility, prototype cycle
  checks, exact lone-surrogate keys, and runtime-domain rejection.
- Shape caches are weak and unlink by finalized generational Shape ID. Shape
  and Symbol atom ownership is paired through heap cleanup, including failure
  paths and runtime teardown.
- Each Context now owns explicit realm roots for `%Object.prototype%`, a
  callable `%Function.prototype%`, the global object and the null-prototype
  global lexical-binding object (`global_var_obj` in QuickJS). Default object
  allocation uses its realm prototype, and `%Object.prototype%` carries
  QuickJS's immutable-prototype bit.
- The realm root set reserves five typed primitive `class_proto` slots. Number,
  Boolean, Symbol and BigInt retain their complete intrinsic slices; String is
  enabled only as the strictly named `String exotic core/substrate`. Its realm
  slot roots a genuinely branded wrapper around the empty UTF-16 string whose
  initial own `length` has `W0 E0 C1`. Sloppy ordinary-function boxing creates a
  fresh String-payload wrapper with `W0 E0 C0` own `length`. In-range UTF-16
  code-unit indices are virtual `W0 E1 C0` properties integrated with
  get-own-property, define-own-property, has-own-property, delete-property and
  own-property-keys; ownKeys merges them with stored numeric, string and symbol
  keys in QuickJS order. The conversion-core extension installs the exact
  `toString`/`valueOf` brand methods after `length`; this three-key list is only
  the QuickJS-relative order filtered to implemented keys, not a claim of full
  53-key ownKeys parity. Primitive non-index reads and writes now traverse the
  bytecode realm's String prototype with the raw receiver, and String receivers
  use the implemented Object-prototype boxing/tag/value routes in the native
  method's defining realm. Global `%String%` and every remaining prototype
  method stay unpublished. `%Number.prototype%` is a Number-class wrapper
  containing `+0` and owns the pinned ordered seven-key method surface. Its
  constructor owns the exact ordered 17-key surface: parser aliases captured
  by identity, non-coercing predicates, frozen constants and the final
  prototype relationship. Calls use `ToNumeric` and the distinct BigInt-to-f64
  conversion; construction performs conversion before observing
  `newTarget.prototype` and falling back to the newTarget function realm.
  `%Boolean.prototype%` remains the boxed-`false` three-key graph with its exact
  `ToBoolean` call/construct behavior. `%Symbol%` is the complete pinned
  intrinsic slice: ordinary calls create a fresh symbol from an optional UTF-16
  description while construction fails before argument conversion. `for` and
  `keyFor` share a runtime-wide, cross-realm registry; the 13 frozen well-known
  constructor properties expose runtime-unique identities that remain outside
  that registry. Its ordinary prototype owns `toString`, `valueOf`, a getter
  that distinguishes absent and empty `description`, `constructor`,
  `@@toPrimitive`, and `@@toStringTag`. Genuine wrappers own a retained symbol
  atom and brand-check independently of prototype identity; wrapper/Object
  routes, primitive get/set, defining-realm errors, cross-realm identities and
  teardown participate in reference counting and trial-deletion GC. `%BigInt%`
  is the complete pinned
  intrinsic slice: ordinary calls perform its distinct constructor conversion,
  construction fails before argument conversion, and `asUintN`/`asIntN`
  preserve `ToIndex`/`ToBigInt` order, signed-limb truncation, allocation guards
  and the extended-limb preallocation gap. Its ordinary prototype owns
  `toString`, `valueOf`, `constructor`, and `@@toStringTag`; methods accept
  primitive and genuine boxed BigInt payloads independent of prototype
  identity. Typed context, wrapper, constructor, lazy-native and prototype
  edges, including cross-realm calls and boxing, participate in reference
  counting and trial-deletion GC.
- The global object has QuickJS's dedicated payload and hidden
  `uninitialized_vars` object. Global data properties and the lexical-binding
  object can store `PropertySlot::VarRef` cells; define, descriptor lookup,
  assignment, accessor conversion and delete preserve shared-cell identity.
  Deleting or converting a global property moves a still-referenced cell back
  to the hidden object, resets it to Uninitialized, and allows a later data
  definition to reconnect the same closures. These VarRef, hidden-object,
  Shape and atom edges participate in reference counting and trial-deletion GC.
- Every realm installs `Infinity`, `NaN` and `undefined` as non-writable,
  non-enumerable, non-configurable global data properties, matching the pinned
  QuickJS 2026-06-04 descriptors and direct-delete results. The implemented
  global string-key surface preserves upstream relative own-key order as
  `parseInt`, `parseFloat`, `isNaN`, `isFinite`, the six URI/escape functions,
  the three constants, `Number`, `Boolean`, `Symbol`, `globalThis`, then
  `BigInt`. This is not a claim that the wider global builtin table is
  complete.
- Every global object owns QuickJS's `[Symbol.toStringTag] = "global"` metadata
  as a non-writable, non-enumerable, configurable data property. The runtime's
  well-known identity is also exposed as the frozen public
  `Symbol.toStringTag` property. Symbol-category own-key ordering keeps the
  global tag after every string key, and the existing
  `%Object.prototype.toString%` path observes its value, deletion, non-string
  replacement and redefinition through the host API.
- Every realm exposes `globalThis` as a writable, non-enumerable, configurable
  own property whose value is that realm's global object. It uses the same
  global `VarRef` substrate as unresolved identifiers, so assignment, deletion,
  accessor conversion, reconnection, defining-realm lookup and the self-cycle's
  trial-deletion GC behavior remain coherent. Upstream places this property
  after String/Math/Reflect, Symbol and the generator intrinsics; the current
  bootstrap preserves the implemented Symbol-before-`globalThis` and
  `globalThis`-before-BigInt order, and the binding must move later as the
  remaining intervening intrinsics land.
- Unresolved identifiers no longer use a string-key global opcode. Resolution
  installs one root `Global` closure descriptor and `ParentGlobal` relays on
  every nested function path; publication interns each exact name and function
  instantiation binds the root cell in the bytecode's defining realm.
  `GetVar` reads initialized cells directly, raises ReferenceError for a lexical
  TDZ, and performs one observable global-object `[[Get]]` for an uninitialized
  non-lexical cell. `GetVarUndef` suppresses only a genuinely missing binding,
  so direct parenthesized `typeof name` returns `undefined` while a lexical TDZ,
  getter throw, or comma/composed reference still throws.
  Sloppy direct-identifier `delete` uses the corresponding late scope result
  without first performing `GetValue`: argument, local, closure, private
  function-name, implicit `arguments` and lexical paths return `false`, while
  global/unresolved paths perform `HasProperty` followed by `DeleteProperty`
  on the defining realm's global object (and return `true` when no property is
  present). Parentheses retain the direct Reference, while comma/composed
  values do not. Strict direct IdentifierReferences are rejected as early
  errors at the pinned QuickJS source position.
  Consequently a function invoked from another Context observes its defining
  realm's global/lexical environment, and global Reference/Type errors use that
  defining realm's native-error prototypes.
- Simple identifier assignment supports the QuickJS `PutVar` and `PutVarInit`
  paths. Mutable lexical cells update directly; lexical TDZ and const writes
  raise the corresponding ReferenceError or read-only TypeError. Non-lexical
  writes perform `HasProperty` before the global-object `[[Set]]`, distinguish
  strict missing names from sloppy creation, preserve non-writable properties,
  report no-setter rejection, discard normal setter return values, and
  propagate setter throws. Assignment-expression value preservation lowers to
  QuickJS's `Dup; PutVar` sequence rather than a synthetic runtime opcode.
- Script evaluation follows QuickJS's execution boundary: raw bytecode is
  instantiated as a bytecode-function object in the caller Context, the call
  frame roots `this`, arguments, locals and the current function, and execution
  switches to the realm stored on the bytecode. Runtime-owned snapshots keep an
  explicit bytecode root beside raw constant-pool IDs.
- Bytecode and native calls share one unified active-frame chain. Each record
  carries the callable, defining realm, strict/frame flags and typed bytecode or
  native invocation state; bytecode dispatch updates the current PC before each
  instruction. Stack-local guards own the function/bytecode roots, validate
  realm and payload agreement, and restore nested frames across return, throw,
  engine error and deferred-drop paths. The same chain is now the authoritative
  input to the implemented synchronous Error-backtrace slice.
- Runtime-published bytecode owns per-function debug metadata: an independently
  retained filename atom, the function definition location, an ordered PC-to-
  line/column table, and an exact source byte range for ordinary function
  expressions. The root script keeps the QuickJS name `<eval>` and no function
  source copy. Source locations follow pinned QuickJS rules: only LF advances
  the debug line and UTF-8 lead bytes, rather than raw bytes, advance the
  column. PC lookup uses the last entry at or before the active instruction;
  equal-PC entries are valid and the last one wins. Publication rejects
  malformed ranges, positions and ordering before interning metadata, while
  filename atom multiplicity, rollback and GC teardown are explicitly tested.
- Source-site lowering for the currently implemented grammar preserves the
  observable markers exercised by calls, explicit and parenthesis-free
  construction, operators, return/tail-call folding and identifier assignment.
  `Context::compile_with_filename`, `eval_with_filename` and their option-based
  variants carry the selected filename through nested bytecode and parse-error
  metadata.
- `FClosure`, `Call` and `CallMethod` use QuickJS's stack layouts. Captured
  arguments and locals are promoted lazily into shared VarRef cells; the parent
  frame and every descendant closure observe the same cell, repeated closure
  creation in one invocation reuses it, and separate invocations are isolated.
- Ordinary function objects expose QuickJS-compatible anonymous, intrinsic or
  inferred `name`, simple parameter `length`, and the observable `length`, `name`,
  `prototype` own-key order. The non-configurable writable `prototype` key is
  installed immediately as typed autoinit storage, but its object and
  `constructor` back-reference are allocated only by Get, complete descriptor
  lookup, assignment, or a compatible define. Shape-only own-key/has-own,
  rejected delete and incompatible define paths do not materialize it. The
  initializer uses the closure-creation realm's `%Object.prototype%`; an
  unread function therefore has no eager function/prototype cycle.
- Source `new`, `new.target`, the verified `Construct` stack opcode, and the
  Rust `Context::construct`/explicit-new-target APIs implement the ordinary
  base-constructor path. Constructor heads accept fixed/computed member chains
  with postfix calls disabled, matching QuickJS's split between the call owned
  by `new` and a call after the completed construction. `newTarget.prototype`
  uses observable property Get,
  a non-object result falls back to the newTarget function realm's
  `%Object.prototype%`, an explicit object return overrides the precreated
  `this`, and a primitive return falls back to it. Ordinary `Call` supplies an
  undefined `new.target`.
- `%Function.prototype%` is a non-constructable native callable returning
  `undefined`, with `name=""`, `length=0`, no own `prototype`, and
  `%Object.prototype%` as its prototype. Its implemented QuickJS function-list
  prefix reaches `caller`, `arguments`, `call`, `apply`, `bind`, and `toString`:
  both legacy accessors share one frozen, non-extensible `%ThrowTypeError%`
  rooted by the realm; their getter
  preserves QuickJS's sloppy ordinary-function compatibility exception while
  strict reads and every write throw `invalid property access`.
  `Function.prototype.call` uses actual argc rather than padded native argv,
  forwards `this` and arguments through the target callable's defining realm,
  and preserves thrown completions. `Function.prototype.apply` checks the
  target before touching its array-like argument, implements the normal
  null/undefined shortcut, Number-hint object conversion and `ToLength`,
  enforces QuickJS's 65,534 argument cap, and performs ordered ordinary indexed
  Gets before forwarding. `Function.prototype.bind` validates callable before
  metadata access, follows QuickJS's own-`length` numeric-only calculation and
  observable `name` ordering, and installs `length`/`name` as W0E0C1 data
  properties. Its dedicated BoundFunction payload strongly owns the target,
  bound receiver and each argument, participates in trial-deletion GC, uses the
  bind realm's `%Function.prototype%`, snapshots constructability, prepends
  arguments, preserves the earliest bound receiver, and applies QuickJS's
  recursive `new.target` replacement without adding a bound frame.
  `Function.prototype.toString` returns the exact captured bytecode source when
  present without reading `name`; otherwise it performs the observable name
  conversion and emits QuickJS's normal/generator/async/async-generator native
  template. The eager getter-only `fileName`, `lineNumber`, and `columnNumber`
  accessors inspect only the receiver's bytecode class, return its filename and
  one-based definition position, silently return `undefined` for non-bytecode
  receivers, and preserve QuickJS's `0` position when debug exists without a
  PC table. They use distinct realm-bound native getter objects with the exact
  names, arities and descriptors. The non-writable, non-enumerable,
  non-configurable `@@hasInstance`
  method implements ordinary prototype traversal and delegates a bound target
  through the full instance-check path, including custom target
  `Symbol.hasInstance` and thrown completions.
- The normal `%Function%` intrinsic is a constructor-or-function native rooted
  explicitly by its realm, published as the global `Function`, and linked to
  `%Function.prototype%` with the exact final key order and descriptors. Its
  dynamic constructor follows QuickJS's typed function-kind handler: it
  performs completion-aware parameter/body `ToString` in actual-argc order,
  builds the exact `(function anonymous(...))` wrapper, compiles it as
  `<input>` indirect global code in the constructor's defining realm, and only
  then performs the observable `newTarget.prototype` Get and cross-realm
  fallback. Generated functions preserve the upstream name, length, 1:2 debug
  definition site, authored source, strict duplicate-parameter validation,
  constructability and strip-mode behavior for the grammar accepted today.
- Native payloads carry a typed target, cproto descriptor, defining realm and
  minimum readable argument count; actual argc remains distinct from
  undefined-padded argv. Generic, constructor-only, constructor-or-function,
  and Getter/GetterMagic adapters share active native
  frame bookkeeping, restore it across return/throw/engine-error paths, and
  keep the mutable object constructor bit independent from cproto and own
  `length`. Native defining-realm edges participate in trial-deletion GC.
- Typed autoinit also covers native methods and constant intrinsic strings.
  The current Object/Function/Error function-list prefixes expose keys and
  descriptors before allocating their values; ownKeys/has-own/delete remain
  shape-only. Get/gOPD and a compatible define materialize once in the stored
  realm; define first checks the lazy flags, so impossible changes to a
  non-configurable slot are rejected without allocation while configurable
  builtins can be replaced by data or accessor descriptors. Initializer
  failure commits an ordinary `undefined` slot while releasing that realm
  edge.
- `%Object.prototype%` currently installs the exact initial function-list
  prefix `toString`, `toLocaleString`, `valueOf`. Completion-aware
  `ToPrimitive` implements observable `@@toPrimitive` with the exact
  `"string"`, `"number"`, or `"default"` hint, then the hint-selected ordinary
  `toString`/`valueOf` Get/Call ordering. It preserves user-thrown values and
  creates framework TypeErrors in the conversion realm. Number, String,
  Boolean and BigInt wrappers feed ordinary default-hint coercion through their
  implemented `valueOf` and `toString`, while Symbol wrappers use the inherited
  `@@toPrimitive`. `Object.prototype.valueOf` boxes any of these primitives in
  the native method's defining realm, `toLocaleString` performs the inherited
  Get/Call with the original primitive receiver, and `toString` boxes in that
  realm before observing inherited `@@toStringTag` getters. Number, String and
  Boolean then use their matching class tags; Symbol and BigInt obtain their
  tags from ordinary prototypes and fall back to `[object Object]` when those
  tags are deleted or non-string. Separate calls allocate distinct wrappers.
  Core tags also include Object, Function and Error plus primitive
  null/undefined tags. The global `Object` constructor remains unimplemented;
  null/undefined `toLocaleString` diagnostics are also outside this prefix.
- Sloppy ordinary bytecode functions normalize primitive `this` lazily and
  cache the normalized value in the frame. Number, Boolean, Symbol, BigInt and
  the String exotic substrate therefore allocate at most one genuine wrapper
  per invocation; repeated `this` reads preserve identity, escaped wrappers
  retain the callee realm's matching prototype, and strict functions continue
  to observe the raw primitive. The same cached path is used when a sloppy
  inherited Number/String/Boolean/Symbol/BigInt getter or setter receives a
  primitive receiver. String lookup still exposes only the implemented
  conversion pair plus user-defined prototype properties; the remaining
  standard method table is absent.
- The Error intrinsic graph now includes `Error` plus the seven non-Aggregate
  native Error constructors, their constructor/prototype/global relationships,
  lazy function-list properties, call-versus-construct active-function rule,
  observable newTarget prototype lookup and cross-realm fallback. Primitive and
  ordinary-object message conversion, inherited/undefined `cause`,
  `Error.prototype.toString`, and class-tag-based `Error.isError` use the native
  completion path. `Error` instances still share one Error object class tag,
  while all Error prototype objects remain ordinary objects.
- Error-class objects now receive QuickJS-style eager own `stack` data on the
  implemented synchronous native/bytecode paths. Native Error construction
  captures after message/cause processing and skips only the Error-constructor
  frame; VM-generated native errors and explicitly thrown Error objects capture
  before frames unwind when no own `stack` already exists. Backtraces preserve
  bytecode filenames and PC locations across realms, include native frames,
  read function names without invoking user getters, and can be recaptured
  after an own `stack` property is deleted. Syntax errors additionally install
  own `fileName`, `lineNumber`, and `columnNumber` before `stack`, using the
  explicit parse location. `EvalOptions::backtrace_barrier` implements the
  current `JS_EVAL_FLAG_BACKTRACE_BARRIER` behavior by marking only the frame
  which existed before eval and restoring it across every exit path.
- Ordinary getter/setter actions retain the callable, original receiver and
  setter argument across property mutation, invoke through the caller Context,
  discard normal setter return values, and propagate thrown completions.
- JavaScript exception transport has a private completion channel and a
  runtime-owned pending raw-value root. Realms explicitly own `Error.prototype`
  plus all eight QuickJS native-error prototypes; VM Type/Range/Reference/Syntax
  faults materialize Error-class objects in the executing bytecode realm and
  become `Completion::Throw`. Explicit thrown object and Symbol identities
  transfer through `Context::take_exception` without leaking roots. Engine
  invariants and explicitly unsupported behavior remain non-catchable errors.
- Runtime-aware `typeof` reports `"function"` for bytecode callables rather
  than treating every object as `"object"`.
- The object core has a dedicated official-QuickJS differential test covering
  key-order boundaries, descriptor defaults/frozen SameValue, inherited and
  explicit-receiver writes, prototype constraints, lone surrogates and
  well-known-vs-registry Symbol identity. A separate Error differential locks
  constructor/prototype descriptors and chains, call/construct results,
  message/cause conversion, toString/isError, Object tags and Symbol failures.
  A separate pinned-oracle Error-stack differential covers nested VM faults,
  tail-call sites, eager Error construction, parse metadata, assignment marker
  inheritance, CR/CRLF and Unicode line/column behavior. A Function-prototype
  differential locks the implemented own-key prefix, poison-accessor identity
  and frozen thrower, `call` forwarding/throws, lazy define behavior, and
  `@@hasInstance` ordering, descriptors, short circuits and prototype errors.
  A separate `apply` differential covers conversion and Get ordering, every
  abrupt path, holes/inheritance/accessors and the real 65,534/65,535 boundary.
  A VM object-coercion differential covers Number/default hints, unary,
  arithmetic, exponentiation, bitwise and shift operators, BigInt/String
  relations, abstract equality, left-to-right conversion, mixed-numeric and
  Symbol error precedence, arbitrary throws and coercion stacks. A dedicated
  1,421-case Number exponentiation matrix compares special values, signed zero,
  overflow, subnormal underflow, rounding boundaries and deterministic finite
  pairs through the real parser/VM against one pinned QuickJS batch. A separate
  725-case BigInt power matrix covers short/heap bases, both signs, odd/even
  exponents, constant shortcuts and thousands-of-bits exact decimal results.
  A 324-case update-numeric matrix compares prefix results and both postfix
  old/new values across Number bit patterns, numeric strings, short/heap BigInt
  boundaries and wide values. Separate update-expression and dynamic-Function
  differentials lock observable Reference/coercion order, readonly failures,
  ASI, power grammar, exact diagnostics and stack metadata.
  The identifier-delete differential covers late local/argument/closure/private
  resolution, implicit `arguments`, missing/configurable/non-configurable global
  properties, accessors without getter invocation, inherited properties,
  Reference-preserving parentheses, composed-value side effects, precedence,
  dynamic `%Function%` compilation, and strict diagnostics/stacks.
  A normal-Function-constructor differential locks the intrinsic/global graph,
  descriptors and key order, exact dynamic source and debug metadata, call/new
  behavior, source-conversion/parse/prototype-Get ordering, custom/fallback new
  targets, sloppy/strict duplicate parameters, exact covered diagnostics, and
  all three source/debug strip modes.
- `Runtime` and `Context` are distinct; `qjs -e` and file execution use the
  Rust compiler/VM path and never delegate to an external engine.

## Not implemented yet

The function slice is intentionally narrow. Function declarations/hoisting,
block scopes, source `let`/`const` declarations and their declaration-
instantiation rules, destructuring, other general assignment targets, module
resolution, computed property-definition
naming, mapped `arguments`, arrow/async/generator functions and callable Proxy
classes are not yet implemented. Top-level declarations are rejected instead
of being faked as frame locals. The internal global lexical VarRef path already
enforces
TDZ, mutable and const behavior, but it is currently exercised through
test-only creation/initialization helpers rather than source `let`/`const`
syntax. Ordinary reads and calls of the valid implicit `arguments` binding are
likewise rejected where materializing it as an ordinary local would be
observably wrong. Direct `delete arguments` is the narrow exception: it resolves
to `false` without materializing or reading the arguments object, including
inside `function arguments(){...}`, where QuickJS resolves the implicit
arguments binding before the private function name.
Derived/class/super construction, dynamic Generator/Async/AsyncGenerator
Function constructors, `AggregateError`, other native builtin constructor
families, Proxy construct dispatch, and Reflect APIs remain. Typed
target/cproto, data-bearing Error selector, realm, arity padding, production
BoundFunction allocation and frame foundations exist, but specialized
setter/F64/iterator cproto adapters and the wider builtin table remain.

Explicit `throw`, nested propagation, VM-generated native errors and eager
Error backtraces share the completion path for the current synchronous slice,
but catch/finally handler tables, async/generator/Promise frame integration,
recoverable OOM and backtrace-allocation fallback, interrupt/termination and
full abrupt-completion semantics remain. The `JS_STRIP_DEBUG` /
`JS_STRIP_SOURCE` debug/source-stripping decision is implemented as a
runtime-wide three-state policy sampled by subsequent compilation: strip-source
retains filename/PC metadata but removes authored source, while strip-debug
removes the represented function source/location payload. The `qjs`
`--strip-source` and `-s` options select the same states in upstream order,
including combined short options and their effect on `toString`, function debug
accessors and Error backtraces. QuickJS's additional stripping of non-observable
vardef/closure-var debug names and bytecode debug serialization are still
pending. The normal `%Function%` graph is present, but dynamic formal parameters
remain limited to simple identifiers and bodies to the current statement and
expression grammar; implicit `arguments`, default/rest/destructuring
parameters, generator/async kinds, and Proxy new-target realms remain pending.
Compiler input is still UTF-8,
so dynamic source containing an unpaired UTF-16 surrogate throws an explicit
implementation-gap `InternalError` instead of being silently rewritten. The
current parser does not yet produce generator or async bytecode, although the
function-kind metadata and `toString` fallback distinguish all four QuickJS
kinds. Bound dispatch is iterative and therefore does not consume the Rust host
stack, but exact QuickJS runtime-stack accounting and its deep-bound-chain
overflow threshold are not yet reproduced. VM object coercion is wired through
the implemented unary, arithmetic, exponentiation, bitwise, shift, relational,
addition and abstract-equality operators and now reaches the implemented
callable classes through `Function.prototype.toString`. Proxy hooks, Date's
special default hint behavior, OOM/interrupt edges and operators outside the
current bytecode slice also remain pending.

Accessors are executable through the Rust Context property API, and
strict/sloppy global identifier assignment is implemented. Source property
reads and receiver-preserving method calls are implemented for object/function
bases, exact String index/length reads, and the complete Number, Boolean,
Symbol and BigInt primitive prototype slices; simple member assignment and
property delete cover ordinary objects and the current primitive surface. The
separate String exotic and conversion cores cover branded empty-prototype and
sloppy-this wrappers, UTF-16 virtual own properties, `toString`/`valueOf`,
non-index prototype lookup and the implemented Object-prototype routes, but do
not publish the global constructor or remaining standard methods.
Prefix/postfix update expressions
(including QuickJS's valid `++x ** 2` form) are implemented for the current
identifier and ordinary fixed/computed member References. Sloppy
direct-identifier delete is implemented
for the current static scope tree and defining-realm global object. Dynamic
object-environment lookup/deletion introduced by `with` or direct `eval`, the
global String constructor, the remaining entries of its 53-key prototype
method surface, Proxy/exotic internal methods, and the full
`function_accessors.js` fixture are still pending. The global `Object`
constructor, AggregateError iterable-to-Array, remaining Object prototype
methods and uncatchable termination state are also pending. Arrays, iterators,
RegExp, Unicode-backed String methods, remaining object-literal forms and the
rest of the builtin table build on those layers.

The remaining parity surface also includes the full grammar/opcode set,
Unicode 17 tables, RegExp bytecode engine, modules, jobs/Promises/async,
generators, TypedArrays/Atomics, WeakRef/finalization, bytecode version 5 and
BJSON interoperability, `std`/`os`, workers, REPL/qjsc, and the complete Rust
and C embedding APIs.

## Reproduce current evidence

```sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_boolean_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_symbol_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_exotic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_conversion_core -- --nocapture
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

./scripts/test-parity-slice.sh
```

The first thirteen commands run the dedicated Boolean, Symbol, String-exotic
substrate, String-conversion core, global BaseObjects, complete
Number-intrinsic and BigInt-intrinsic differentials. The full gate command
checksum-verifies and builds the official test-only oracle, runs formatting,
unit/integration/oracle tests, Clippy, and the Rust-only product gate. The
oracle is never part of the product dependency graph or runtime.
