# Implementation status

Last audited: 2026-07-15. The completion definition remains
[`parity.md`](parity.md); this file records progress and must not be used to
claim full parity.

## Implemented on the final architecture path

- QuickJS 2026-06-04 release metadata, archive checksum, bytecode version,
  Unicode version, and Test262 commit are pinned in `compat/upstream.toml`.
- The process-isolated Rust Test262 runner now saves a complete conservative
  outcome vector for all 102,037 sloppy/strict variants. A checksum-pinned
  capability profile, audited negative-test canaries, and source/metadata host
  requirements keep unsupported grammar, features, modes, and `$262` hooks from
  becoming false passes. Bounded workers preserve canonical byte-for-byte TSV
  and JSONL ordering. The current vector has 21,740 passes: a 26.02% lower bound
  after the 18,475 pinned QuickJS target exclusions, or 72.34% among the 30,052
  variants with a non-unsupported observed outcome. The fixed smoke remains 189
  passes and four explicit parser-frontier results. See `docs/test262.md` for
  the denominators and why none of these figures is a parity claim.
- The lexer models parser-selected division/RegExp/template lexical goals,
  source spans and ASI trivia, contextual keywords, numeric/String/BigInt/
  template/RegExp tokens, UTF-16 escapes, comments, and punctuator longest
  matching. Identifier classification ports QuickJS's checksum-pinned Unicode
  17 compressed `ID_Start`/`ID_Continue` tables, including direct and valid
  escaped BMP/astral spellings, ECMAScript `$`/`_` and ZWNJ/ZWJ additions,
  non-normalization, private names, and UTF-16 buffer accounting. Every scalar
  is checked against the official release and execution tests cross the real
  compiler, resolver, atom and VM path. The compiler consumes tokens on parser
  demand through fallible advances; true lexical failures propagate only when
  reached, unrecognized ASCII is retained as a raw token, and directive probes
  seek back before strict-context rescanning. This matches the pinned
  malformed-escape commitment and tested reserved/parser/lexer error priority,
  including line and column. Module/generator/async contextual-word behavior
  remains with those unimplemented grammar surfaces.
- Runtime-local atoms preserve exact UTF-16 spellings, cover immediate integer
  atoms, string/global-symbol interning, unique/private/well-known symbols, and
  explicit retain/release. Safe handles carry a runtime domain and slot
  generation while raw table slots use QuickJS-style free-list reuse.
- Primitive values preserve compact integer vs float values, exact `-0`/NaN
  equality variants, Latin-1/UTF-16 strings including lone surrogates, and
  arbitrary-precision BigInts with QuickJS's short/heap normalization and
  2026-06-04 `asIntN`/`asUintN` behavior.
- The compiler builds a nested `FunctionIr` tree with unresolved identifier
  operations over typed, function-local `ScopeId` and `BindingId` arenas.
  Scope zero owns arguments and function-scoped storage, while every script or
  ordinary function has a distinct authored-body scope. Non-empty blocks,
  `if`, classic `for`, and `switch` add typed parser/IR scopes at the pinned
  QuickJS boundaries; the populated body/block/for/switch lifetimes are
  described below. Unresolved identifier reads and every lvalue rewrite
  retain their original use-site scope, and each child function records its
  parent's definition-site scope. Resolution walks children in source-order
  DFS postorder, searches each ancestor from that frozen definition scope, and
  deduplicates closure relays by storage identity rather than source name.
  `var` bindings retain root storage plus their first declaration scope, sloppy
  duplicate parameters remain distinct slots with the last slot winning, and
  the private named-function binding remains a lazy root local.

  Source lexical population now covers simple-name `let` and `const` lists in
  the direct Program global lexical environment plus four local authored
  environments: an ordinary function body (including a normal `%Function%`
  constructor body), every non-empty nested brace block, the one CaseBlock scope
  shared by every clause of a `switch`, and the initializer scope of a classic
  `for (;;)` loop. Block, switch, and classic-for locals also work in scripts.
  `let` without an initializer performs explicit `undefined`
  initialization, `const` requires an initializer, and anonymous function
  initializers retain contextual NamedEvaluation. Registration occurs before
  each initializer is parsed; duplicate names are rejected within one lexical
  scope, all switch clauses participate in the same duplicate check, and
  shadowing an outer lexical, parameter, or private named-function binding is
  allowed where QuickJS allows it. A `var` in the loop body or another
  descendant still conflicts with the head lexical, while a function-scoped
  binding outside the loop may be shadowed by it. Body-scope parameter
  conflicts and the pinned release's asymmetric first-declaration-scope `var`
  conflict behavior are preserved for earlier and later `var` declarations.
  The declaration probe retains QuickJS's contextual sloppy-`let` and
  LineTerminator rule: statement-list positions and classic heads recognize
  the declaration form, while a single-statement position keeps a
  line-terminated ambiguous `let` as an identifier expression and rejects an
  unambiguous lexical declaration with the pinned diagnostic.

  Scope lifetime is represented in IR by typed `EnterScope(ScopeId)` and
  `LeaveScope(ScopeId)` operations rather than by a body-only slot list. After
  declaration and closure resolution, lowering expands entry to
  `SetLocalUninitialized` for the scope's lexical locals in QuickJS's
  newest-first order. Exit expands to `CloseLocal` only for locals captured by
  a child; uncaptured exits emit no bytecode. A normal block exit therefore
  detaches captured cells, while executing the same block again enters fresh
  TDZ cells. Explicit `break` and `continue` emit the equivalent of QuickJS
  `close_scopes` for every lexical scope crossed by the edge, interleaved with
  the existing switch-selector stack cleanup; the matched control's own scope
  stays live until its common tail performs the normal exit. This covers local
  and labeled jumps across nested blocks, switches, and loops without changing
  parser scope state on the unreachable linear path. Closure relays are also
  used to derive which defining locals require `CloseLocal`, including
  transitive capture. The late read-only fault-PC projection now runs on the
  fully lowered instruction stream, so expanded entry/exit instructions take
  part in the same pinned QuickJS dead-code, label-threading, and source-marker
  rules.

  A classic-for head has one authored `EnterScope`, before its initializer; it
  is not re-entered or reset to TDZ on every iteration. For a captured head
  binding, QuickJS closes the initializer cell before the first test, closes
  the current cell on normal body fallthrough before the update, and closes
  the final cell at the shared loop-exit tail. `CloseLocal` detaches the current
  VarRef while leaving its value in the frame slot, so a later capture creates
  the next cell without reinitializing the binding. `break` reaches the final
  tail close. In QuickJS 2026-06-04, however, a `continue` targeting that same
  classic loop jumps directly to its update or test and skips the normal-body
  close for the head scope; descendant block/switch cells are still closed.
  Consequently a captured head binding can be shared across the continued and
  following iteration. The implementation and oracle deliberately preserve
  this pinned `/* XXX: check continue case */` behavior rather than silently
  substituting the specification's expected fresh cell.

  Direct Program `let`/`const` does not use a script-frame local. The compiler
  records typed `GlobalDeclaration` descriptors in declaration source order,
  before its child-first resolver can install `ParentGlobal` capture relays.
  Script execution mirrors QuickJS `js_closure2`: it first checks every global
  declaration without mutating the realm, then creates all accepted bindings in
  the null-prototype global lexical object, and only then runs authored
  bytecode. `PutVarInit` initializes the resulting TDZ cell; let/const remain
  absent from `globalThis`, survive later `Context::eval` calls, cannot be
  deleted by a direct identifier, and retain writable/enumerable/configurable
  flags of `W1 E1 C1` and `W0 E1 C1` respectively. A configurable global-object
  property may coexist under the same name, while an existing global lexical or
  non-configurable own global property rejects the whole declaration batch
  before any binding is created. Compilation and publication alone do not
  instantiate declarations; the compatibility check occurs when the script
  closure is executed. As in `JS_EvalFunction`, declaration checks, binding
  creation, and any resulting SyntaxError use the initiating Context even when
  the bytecode was compiled in another realm; the authored body subsequently
  executes with the realm stored on the bytecode.

  The pinned failed-initializer behavior is preserved deliberately. A created
  but uninitialized Program lexical still blocks redeclaration and direct delete
  remains false. The declaring script and its typed `ParentGlobal` captures see
  the named TDZ, but a later ordinary global descriptor reports the name as not
  defined and direct `typeof` yields `undefined`; QuickJS `OP_get_var` consults
  closure-descriptor lexical metadata for this read while writes consult VarRef
  metadata. Strip-debug therefore retains names on `GlobalDeclaration` and
  `ParentGlobal` descriptors because they are semantic atoms, not debug-only
  lexical names.

  Simple-name Script `var` now uses the same production global-declaration
  path in Program bodies, blocks, `if`/`switch` statements, and classic
  `for (;;)` heads. It never consumes a script-frame local. Matching QuickJS,
  the compiler keeps two related structures: one canonical global binding with
  the first declaration's scope for parser conflict lookup, and one ordered
  declaration record for every syntactic `var`, including duplicates. Each
  record publishes a non-lexical mutable `GlobalDeclaration`; a child capture
  relays the first same-name descriptor through `ParentGlobal`. This preserves
  both the 65,534-descriptor limit and the pinned first-declaration-scope quirk,
  such as allowing `var x; { var x; let x }` while rejecting two first-seen
  declarations followed by `let x` in that same block. Initializers remain at
  their authored positions and use `PutVar`; no-initializer declarations emit
  no authored write.

  Declaration instantiation preflights the complete mixed var/lexical batch
  before creating anything. A new Program var creates an own global data
  property with `W1 E1 C0` and value `undefined`, including vars in unreachable
  statements. Repeated and later no-initializer vars do not reset a value.
  Existing own data/accessor/AutoInit properties are accepted without changing
  their attributes or kind; accessors use the hidden unresolved-VarRef table so
  authored reads and writes fall back through the ordinary global object. A
  deleted configurable property keeps that shared cell hidden so an older
  closure reconnects when a later var recreates the property. An inherited
  property does not count as the declaration's own binding. Missing names on a
  non-extensible global object throw `cannot define variable` TypeError before
  a same-name lexical redeclaration check, exactly in the pinned order.

  As with Program lexicals, compile/publication alone does not instantiate a
  var. `Context::execute` performs declaration checks and binding creation in
  the initiating Context, while authored bytecode uses its defining realm.
  Fresh or existing data VarRefs therefore stay attached to the initiating
  realm, but an existing accessor's hidden uninitialized cell makes initializer
  fallback read/write the defining realm's global object. Preflight errors use
  the initiating realm; initializer errors use the bytecode realm. Duplicate
  ordinary global declarations are verifier-valid and share one runtime cell;
  duplicate lexical declarations remain rejected when the first same-name
  descriptor is lexical. An earlier Annex normal descriptor instead masks
  later repeated lexical records, whose descriptors reuse the first lexical
  runtime cell.

  Direct Program ordinary named function declarations now use their distinct
  QuickJS `JS_VAR_GLOBAL_FUNCTION_DECL` path. Every syntax node keeps an ordered
  `GlobalFunction` descriptor and child constant. Before authored bytecode, the
  compiler emits `FClosure` plus a declaration-time raw `PutVarInit` in source
  order, resolving every write to the first same-name global descriptor;
  repeated functions therefore share one binding and the last hoist wins. A
  declaration child has an intrinsic function name but no private
  named-expression local, so recursive name resolution goes through its
  authored global environment. Ordinary `var` and function declarations may
  repeat in either order, and a later authored var initializer can overwrite
  the hoist.

  The pinned parser asymmetry is preserved rather than normalized: a function
  followed by a same-name Program lexical is a syntax error, while a preceding
  `let`/`const` followed by one or more functions is accepted. In the accepted
  order, the lexical and function descriptors create separate lexical/global
  roots, but every hoist raw-writes the first lexical cell before authored
  initialization. This bypasses TDZ and const checks exactly like QuickJS;
  authored lexical initialization may then replace the value, while the
  separate `globalThis` function property remains `undefined`.

  Function declaration preflight is also distinct from `var`: a missing name
  on a non-extensible global, a fixed accessor, or a non-configurable data
  property lacking writable/enumerable attributes throws caller-realm
  `cannot define variable` TypeError before the lexical-conflict SyntaxError
  check. Accepted configurable data, accessor, and AutoInit properties are
  normalized without invoking accessors to a writable, enumerable,
  non-configurable VarRef property; an accepted fixed writable/enumerable data
  property retains its cell identity. Compile-in-A/execute-in-B instantiates
  the property and reports preflight errors in B, while the hoisted function
  object and authored-body errors retain A's function realm. Direct Program
  declarations are covered by a pinned differential matrix; async and
  generator declarations remain explicit boundaries, while the ordinary Annex
  B statement forms are described below.

  Direct ordinary FunctionBody declarations now use QuickJS's separate local
  hoist path. Each named child is parsed immediately but emits no closure at its
  authored position. Its constant replaces the previous hoist attached to the
  canonical ordinary binding: the last same-name declaration therefore wins,
  an existing parameter (including the last duplicate parameter slot) is
  reused, and a new name receives a function-scoped local. On body entry the
  compiler emits one `FClosure + PutArg` per hoisted argument in slot order,
  followed by one `FClosure + PutLocal` per hoisted root local in slot order,
  before body lexical TDZ initialization. An authored `var` initializer still
  runs later and may replace the function, while an initializer-free `var`
  preserves it.

  These declaration children carry their intrinsic `.name` but no private
  named-expression binding; recursion and mutation resolve through the shared,
  mutable parent argument/local cell. They can capture a later body lexical.
  Because QuickJS connects that captured cell before entering the body scope,
  the runtime accepts the first already-uninitialized captured TDZ entry as a
  no-op while still rejecting an initialized captured lifetime that skipped
  `CloseLocal`. Function/lexical same-name conflicts are symmetric in ordinary
  bodies, unlike the pinned Program lexical-first quirk. The normal
  `%Function%` constructor body follows this path too. Ordinary functions with
  the current simple-identifier parameter grammar now select their implicit
  `arguments` binding lazily during resolution. Direct `delete arguments`
  remains `false` without allocation; an explicit parameter or body lexical of
  that name suppresses the implicit object. Otherwise an entry prologue creates
  it before direct body-function hoists, so `var arguments` shares the object
  local and a same-name body function overwrites that initialized local in the
  pinned order. The implicit binding also precedes a sloppy named-expression
  private self name.

  Sloppy simple parameters use mapped Arguments VarRef cells and strict
  functions use an unmapped snapshot. `length` and indexed properties use the
  authored actual argc rather than padded formal slots; duplicate formals,
  extra and missing actuals, escaped mappings and nested calls follow QuickJS.
  The object has `%Object.prototype%`, the `Arguments` brand, cached original
  `Array.prototype.values` iterator, mapped data or strict poison `callee`, and
  exact descriptors/key order. Existing-index Set stays fast; explicit define,
  delete, accessor conversion and `writable: false` reproduce QuickJS's
  fast/slow and mapping-detach transitions, including representation-sensitive
  `for-in`. Default/rest/destructuring parameter lists and their forced-unmapped
  arguments semantics, direct/indirect eval environments, arrows, and
  async/generator functions remain separate slices.

  Ordinary declarations in brace blocks and a switch CaseBlock use QuickJS's
  distinct scoped-function path. The binding is registered immediately after
  its name, before parsing parameters or the child body, so redefinition error
  priority matches upstream. Every syntax node owns a mutable lexical local and
  an entry closure; sloppy same-scope duplicates keep separate slots and child
  constants, with the last declaration visible by name, while strict duplicates
  are rejected. A switch uses one shared declaration scope entered after the
  discriminant and before every case test.

  QuickJS also evaluates a second `FClosure` at every declaration's authored
  position. In sloppy code the first eligible declaration duplicates that
  second object into a function-root normal var (or a normal global var), then
  drops the remaining value. Consequently the block lexical and Annex B outer
  function have different identity; with duplicates, the block name uses the
  last child while the outer name uses the first. A prior effective enclosing
  lexical, a simple same-name parameter, or the `arguments` name suppresses the
  outer update. Existing vars are reused. A newly synthesized root local records the
  root as its declaration scope, preserving QuickJS's later-block and later
  body-lexical quirks. Program Annex writes resolve dynamically rather than
  through the declaration slot, so an Annex var registered before a later
  same-name Program lexical hits that lexical's TDZ at runtime exactly as in
  the pinned release.

  The ordinary Annex B statement forms now follow QuickJS's declaration-mask
  model rather than treating every single-statement position alike. Sloppy
  `if` consequents and alternates allow an ordinary FunctionDeclaration; strict
  arms reject it. The `if` enters one shared lexical scope before evaluating
  its condition, so the condition sees the last same-name entry closure from
  either arm. Only the first same-scope declaration is Annex-eligible: choosing
  a duplicate `else` therefore evaluates its authored closure but leaves the
  outer var undefined (or preserves its earlier value). Each loop re-entry
  creates fresh lexical cells. Direct function bodies of `while`, `do`, and
  classic `for` remain QuickJS syntax errors, while a nested `if` reopens its
  own sloppy Annex B permission.

  A sloppy label reached from ProgramBody, FunctionBody, a block, a switch, or
  another eligible label may forward ordinary-function permission; strict
  labels and labels directly under an `if` arm may not. Labels add break
  control but no lexical scope, so chained labels and neighboring declarations
  share the current environment. FunctionBody labelled functions use a body
  lexical entry closure and may shadow a same-name parameter while suppressing
  the Annex root write. The implemented global Script/`Context::eval`
  ProgramBody path is QuickJS's special exception: a labelled function
  allocates no lexical slot and evaluates one closure at its source position,
  then performs both global writes. Direct and indirect JavaScript eval
  environments remain a separate frontier and instead create an eval-local
  lexical entry. The global-path duplicate write is observable
  through an existing accessor setter. QuickJS spells the second operation
  `OP_put_var_init`; the Rust VM lowers it to a declaration-bound `PutVar` because
  a raw VarRef initialization in this runtime would bypass that accessor. This
  is an internal representation choice preserving the two observable setter
  calls. Repeated Program labels therefore each overwrite the global. A
  same-name declaration first authored directly in ProgramBody causes the
  pinned redefinition error, while a later lexical is accepted by the parser
  and makes an earlier label write hit its TDZ. QuickJS always consults the
  first same-name global record: if that record is an earlier Annex normal var,
  it masks the later Program lexical for subsequent Annex eligibility and label
  conflict checks. A later block/if/label function may consequently overwrite
  an initialized `let` or throw on an initialized `const`. The same lookup also
  permits repeated Program lexical and `var` records; every initializer runs,
  while the first lexical record determines the binding's constness for later
  ordinary writes. These ordered mixed descriptor sequences are retained by
  both publication trust boundaries, and duplicate lexical descriptors reuse
  the first lexical VarRef during global instantiation. Authored identifier
  resolution also keeps the first normal descriptor: before the later lexical
  initializer runs, reads fall back to the replacement global-object property
  and observe `undefined`, while writes still consult the transformed VarRef's
  TDZ and const metadata.

  Scope entry resets ordinary lexical lifetimes before allocating scoped
  function closures, then initializes function locals in QuickJS's newest-first
  order. This ordering preserves fresh captured cells on loop re-entry without
  weakening the runtime's missing-`CloseLocal` invariant. It differs from
  QuickJS's interleaved TDZ/function allocation order only under an injected
  allocation failure; values, identity, closure cells, errors, stacks, and
  realm behavior are pinned by differential tests. Normal exits, `break`, and
  `continue` close captured block cells. A caught throw deliberately leaves
  intervening captured block cells open, preserving the pinned QuickJS reuse
  quirk when control resumes in the same frame.

  `return` and an uncaught `throw` do not emit lexical leave operations;
  whole-frame teardown detaches their captured locals, as in QuickJS. A caught
  `throw` resumes at the nearest same-frame handler without synthesizing those
  leave operations, including the pinned observable captured-cell result
  `2|2|false` on repeated block entry. A `return` crossing `finally` has the
  same skipped-close behavior if the finally body replaces it with a same-frame
  `break`, `continue`, return, or throw; the reuse hook is limited to exception
  dispatch and verified `NipCatch` return unwinds rather than ordinary gosubs.

  The `oracle_try_catch_finally` target locks the implemented synchronous
  exception-region boundary. It compares the same source
  against the Rust engine and the checksum-pinned QuickJS process for primitive,
  object, native, getter, callee, and constructor throws; nearest handlers,
  rethrows, catch binding scopes and closures; and normal plus abrupt `finally`
  completion. Its control-flow matrix includes return override, labelled and
  unlabelled break/continue across nested finally clauses, retained switch
  state, Script completion, the pinned caught-throw captured-cell result
  `2|2|false`, and captured-cell reuse across nested abrupt-finally overrides.
  Separate checks lock compile-A/execute-in-B realm behavior, exact
  Full/StripSource/StripDebug stacks and parser diagnostics. Catch binding
  destructuring remains an explicit compiler frontier rather than being
  treated as a simple identifier binding.

  The upstream anchors for this slice are `quickjs.c` 21775-21785
  (`BlockEnv`), 28225-28361 (break/continue/return through finally),
  29270-29423 (try/catch/finally parsing), 18948-18981 and 19052-19065
  (`catch`/`gosub`/`ret`/`nip_catch` execution), and 20545-20570 (same-frame
  exception-handler search), plus `quickjs-opcode.h` 181-184. The Rust VM keeps
  catch markers private, models `gosub` return PCs as verified typed stack
  slots, and resumes thrown values or materialized native errors in the same
  frame. Internal and I/O engine invariants remain uncatchable.

  The pinned source anchors for this slice are `quickjs.c` 10817-10855
  (`JS_CheckDefineGlobalVar`), 17151-17285 (global declaration/reference VarRef
  creation), 17307-17359 (two-pass declaration instantiation), 17571
  (`close_lexical_var`), 18487-18555 (`GetVar`/`PutVarInit`
  descriptor-versus-cell checks), 23933
  (`find_var_in_child_scope`), 23989/24035/24047
  (`push_scope`/`pop_scope`/`close_scopes`), 24156-24173/24202-24307
  (ordered global-var records and scoped declaration conflicts), 24186 and 26096
  (`define_var`/`js_define_var`), 28225 (`emit_break`), 28378
  (`js_parse_block`), 28398-28456 (var initializers), 28784-28831 (label mask
  propagation), 28901-28932 (the shared `if` scope and sloppy Annex B mask),
  29004-29147 (classic
  `for`, including initializer and
  normal-body closes plus the `continue` quirk), 29172-29268
  (SwitchStatement and its shared CaseBlock scope), 29460-29494
  (statement-list function declarations),
  31917-31940 (direct function source elements), 32577-32614 (closure append
  and first-name lookup), 33132-33161 (`OP_scope_put_var_init` resolution to
  global `OP_put_var_init`), 33888-33977
  (hoisted definitions and raw global-function writes), 34281/34315
  (`OP_enter_scope`/`OP_leave_scope` expansion), and 35837-35902
  (`add_global_variables`), plus 36383-36942 (function declaration parsing,
  scoped lexical creation, Annex source writes, and argument/local hoist
  attachment)
  in QuickJS 2026-06-04. Direct/indirect eval declaration environments,
  async/generator declarations, for-in destructuring, `for-await`, for-of
  destructuring, single-statement lexical declarations, and catch/class scopes
  remain explicit boundaries rather than falling back to local or ordinary
  global storage.

  The immutable function format and VM provide the lexical-frame substrate for
  this compiler slice. Published bytecode owns
  QuickJS `JSVarDef`-shaped argument/local definitions with optional atom names,
  lexical/const flags and binding kind; closure descriptors carry the same
  semantics across every relay and distinguish unresolved `Global` access from
  declaration-instantiating `GlobalDeclaration`. Frame locals and heap-owned VarRefs preserve an
  explicit uninitialized TDZ sentinel. `SetLocalUninitialized`,
  `GetLocalCheck`, `PutLocalCheck`/`SetLocalCheck`,
  `GetVarRefCheck`/`PutVarRefCheck`, and `CloseLocal` cover lexical entry,
  access, mutation and captured-cell detachment, with metadata/opcode
  compatibility checked again at publication. `InitializeLocal` is a typed Rust-bytecode
  distinction for QuickJS's ordinary lexical-initializer `put_loc` after
  `OP_scope_put_var_init` resolution. It is not a spelling of QuickJS
  `put_loc_check_init`, whose initialize-once check is specific to derived
  `this`. Accordingly, repeated `InitializeLocal` execution retains upstream's
  plain overwrite behavior, including the next captured `for-in`/`for-of`
  iteration after `CloseLocal`. Detached bytecode and runtime-backed frames
  enforce TDZ; as a typed trust-boundary hardening, runtime-backed frames seed
  every published lexical vardef with the sentinel before executing bytecode,
  while `SetLocalUninitialized` still represents source scope entry and
  re-entry. Source lexical reads lower to checked local/VarRef reads, mutable
  writes to checked writes, initialization to ordinary-overwrite
  `InitializeLocal`, and immutable writes to the atom-bearing
  `ThrowReadOnly`. A value-preserving captured mutable write expands to
  `Dup; PutVarRefCheck`; transitive `ParentClosure` relays retain the lexical
  name and flags required by publication preflight. Full and strip-source modes
  retain lexical vardef/relay names for TDZ diagnostics. Strip-debug removes
  those debug names on every relay, including classic-for captures, while
  retaining the `ThrowReadOnly` atom, so TDZ becomes generic but const
  assignment remains named. Full and
  strip-source error stacks also project QuickJS's two late resolver passes
  when a const write becomes terminal: dead-code marker inheritance, mutable
  label references, Goto threading, constant tests, physical-label barriers,
  and conditional/Goto inversion feed the observable fault PC. Published variable
  definitions additionally enforce constness, while runtime VarRefs enforce
  close-before-reentry and preserve detached lexical lifetimes across the
  implemented block re-entry paths.

  After resolution the compiler lowers to stack bytecode. In addition to the
  primitive expression grammar,
  the current source path supports anonymous and named ordinary function
  expressions, simple parameters, `return`/fallthrough, function-local `var`,
  Script-wide simple-name `var`, direct Program simple-name `let`/`const`, and
  the
  body/block/switch/classic-for-head lexical slice above,
  recursive block statements, `if`/`else` (including nearest-`if` binding),
  `while`/`do-while`, classic `for (;;)` loops, `switch` control flow and
  labeled statements with named and unnamed `break`/`continue`,
  relational `in`/`instanceof`,
  simple/arithmetic/exponentiation/shift/bitwise/logical identifier assignment,
  prefix/postfix identifier and member updates, direct calls,
  transitive parameter/local and private function-name capture through
  `ParentClosure` relays, and QuickJS-style contextual `SetName` for direct
  anonymous initializers and assignments. Named expressions use a
  per-invocation private self binding; sloppy writes are ignored and strict
  writes raise the QuickJS-compatible read-only TypeError. Script source
  elements and function/block/single-statement bodies now enter through one
  QuickJS-shaped statement parser. Each function owns a typed break-control
  subset of QuickJS `BlockEnv`, distinguishing regular labeled statements,
  loops and switches so unnamed jumps skip regular labels while named jumps
  search outward;
  nested functions cannot target an enclosing function's controls. Root
  scripts reserve the unspellable `eval_ret_idx` local at
  slot zero: expression statements store completion, empty blocks preserve it,
  and `if` resets it before its condition. `while` resets once before its header;
  `do-while` targets its reset on every entered iteration, sends `continue` to
  the condition and lets `break` skip it. Conditions never become completion
  values, and the `do-while` trailing semicolon is unconditionally optional as
  in QuickJS. Classic `for` uses a non-committing clone-Lexer port of
  `js_parse_skip_parens_token` to select a head with top-level semicolons,
  explicitly propagates QuickJS AllowIn/NoIn grammar state, and shares the
  function-local `var` declaration path. Its simple-name lexical declarations
  use the same NoIn initializer grammar, conflict registration, TDZ,
  NamedEvaluation, closure, read-only, and StripDebug paths in scripts,
  ordinary functions, and normal `%Function%` constructor bodies.
  Initializer/test/update values are
  discarded; `continue` selects update, test or body according to the missing
  clauses. With both test and update, the relocatable update IR fragment moves
  after the body like QuickJS's optimize pass, retaining Nop source slots and
  rebasing only internal jumps so inherited debug markers remain exact. A
  directly attached label becomes the loop's break/continue name; every other
  label creates a regular break-only control, active duplicates fail before
  consuming the second label, and the pinned release's outer-wrapper behavior
  for multiple labels is retained. Labels and jumps emit no synthetic source
  marker. Switch follows QuickJS's retained-discriminant CFG: case expressions
  are tested in source order with `StrictEq`, including cases after a middle
  `default`; matched and fallthrough paths share bodies, while the final failed
  test enters the recorded default or the common tail. The discriminant stays
  on the operand stack through every body. A local `break` reaches the shared
  tail Drop, whereas a jump to an outer label or loop emits the typed
  `BlockEnv.drop_count` cleanup for each crossed switch before its Goto and
  restores the parser's fallthrough stack shape for later source. Reachable
  `return` and `throw` consume their completion value and abandon remaining
  frame values instead of requiring a synthetic switch cleanup, matching the
  pinned verifier/VM contract. Differentials lock default search versus body
  order, strict identity without coercion, fallthrough/completion, nested and
  cross-control cleanup, ASI, function-local `var`, arbitrary thrown values,
  exact diagnostics and source stacks. Stack-limit probes separately lock the
  65,534-slot discriminant, retained-body and retained-plus-Dup case-test
  boundaries, as well as unreachable source after `break`. The CaseBlock now
  owns one shared lexical scope: its entry precedes case-expression dispatch,
  all simple-name declarations are therefore in TDZ across every clause,
  duplicate declarations conflict across cases, and normal or abrupt exits
  close captured cells while preserving selector cleanup. This is the current
  simple-name switch lexical slice, not complete SwitchStatement parity.
  Synchronous `for-of` now follows QuickJS `js_parse_for_in_of` for simple
  `var`/`let`/`const`, identifier, fixed-member and computed-member targets.
  The assignment fragment is emitted before the iterable expression and
  skipped on first entry; the head lexical environment is therefore already
  in TDZ while evaluating the iterable. Captured lexical head cells close at
  the pinned per-iteration boundary. Local and labelled continue retain the
  active iterator, while edges crossing an iterator control close it in
  inner-to-outer order and interleave correctly with switch cleanup and
  try/finally subroutines. `for-await-of` and for-of destructuring remain
  explicit frontiers. The classic head continues to port QuickJS's
  sloppy `is_let(..., DECL_MASK_OTHER)` ambiguity; the shared statement parser
  applies the corresponding list-versus-single-statement mask.

  Typed `ForOfStart`, `ForOfNext`, `IteratorClose`, and
  `IteratorClosePreserve` bytecode model QuickJS's three-slot iterator record,
  with the catch-offset marker kept private and unforgeable. Ordered Catch and
  Iterator unwind regions are checked at bytecode verification and again by
  the VM. Exhaustion and next/done/value faults disable the record before
  propagating, so they do not call `return`; body, assignment, break, return,
  and outer jumps do. Pending throws retain their original value across close
  getter/call failures and skip the close-result Object check, while a close
  fault replaces a normal break or return. Direct native
  `NativeCProto::IteratorNext` methods use QuickJS's raw value/done ABI through
  the same active frame and defining realm; ordinary JavaScript calls still
  receive a realm-correct `{ value, done }` object, and bound/bytecode methods
  retain generic result-object parsing.

  The generic runtime protocol performs observable `@@iterator`, cached
  `next`, `done`, `value`, and `return` operations in pinned order. Realm-rooted
  `%IteratorPrototype%` and `%StringIteratorPrototype%` provide iterator
  identity plus the pinned `@@toStringTag` accessor/data descriptors without
  exposing the still-pending global `Iterator` or Iterator Helpers. String
  iteration advances by Unicode code point while preserving lone UTF-16
  surrogates and releases its source at exhaustion. `oracle_for_of` locks the
  value, accessor, close-precedence, nested-control, cross-realm, diagnostics,
  stack and strip-mode matrix against QuickJS 2026-06-04.

  The pinned anchors are `quickjs.c` 16512-16720 (iterator protocol and
  IteratorClose), 18985-19049 (for-of and close opcodes), 20545-20570
  (exception-time iterator closing), 28225-28335 (abrupt-control cleanup),
  28546-28769 (`js_parse_for_in_of`), 44182-44510 (Iterator prototype), and
  46508-46680 (String Iterator), plus `quickjs-opcode.h` 201-210.

  Synchronous `for-in` now uses the same upstream parser path for simple
  `var`/`let`/`const`, identifier, fixed-member and computed-member heads. Its
  right operand is a full comma Expression, including QuickJS's sloppy-only
  legacy `var` initializer. Typed `ForInStart` and `ForInNext` bytecode preserve
  the hidden enumeration object with stack effects 1-to-1 and 1-to-3; local
  continue retains it, while break and crossed control edges drop it without
  IteratorClose. Nullish inputs enumerate nothing, other primitives box in the
  executing realm, and only string keys can be yielded.

  The hidden heap object snapshots each ordinary prototype level only when it
  is reached. Enumerability is captured with that snapshot, own-property
  presence is checked live before yield, non-enumerable or deleted nearer names
  still enter the visited set, and prototype links are read live between
  levels. QuickJS's representation-sensitive fast-Array path is tracked
  explicitly: dense count-only iteration converts to a current own-key visited
  set before prototype traversal, while descriptor or sparse-index conversion
  remains irreversibly slow. Differential regressions lock both mutation modes,
  ordinary shadowing, ordering, lexical cells, labels and finally cleanup.
  Destructuring heads remain explicit parser frontiers. The VM host outcomes
  already preserve arbitrary JavaScript throws; Proxy enumeration and its
  duplicate prototype pre-scan/trap order still require Proxy internal methods.
  Anchors are `quickjs.c` 16282-16509 and 28546-28769, plus
  `quickjs-opcode.h` 201-204.

- Array literals follow QuickJS's three-phase lowering rather than a generic
  builder rewrite: up to 32 leading dense elements use `ArrayFrom`, later
  fixed elements use indexed defines, and the first elision or spread switches
  to a dynamic index carried on the VM stack. Empty arrays, holes, trailing
  commas, nested literals, prefixes beyond 32 elements, and iterable spread
  therefore preserve the pinned stack shapes and source sites. Spread uses
  the ordinary iterator protocol and the `js_append_enumerate` close rule: a
  `next` or element-definition failure closes with a pending exception, and a
  close failure cannot replace the original throw. Typed bytecode operands and
  the verifier reject malformed counts, constant indices, and stack joins.
  The pinned anchors are `quickjs.c` 16840-16925 (`js_append_enumerate`),
  19685-19710 (Array opcodes), and 25669-25795
  (`js_parse_array_literal`), plus the corresponding opcode definitions in
  `quickjs-opcode.h`.

- Object literals now follow the data-property portion of QuickJS
  `js_parse_object_literal`. A realm-correct ordinary Object stays below fixed
  identifier/keyword/String/numeric/BigInt properties, shorthand properties,
  and computed properties on the typed VM stack. Computed keys perform the
  observable `ToPropertyKey` before the RHS and preserve anonymous-function
  naming, while fixed names reuse `DefineField` and computed names reuse the
  generic `DefineArrayEl` plus key drop. Static `__proto__` changes
  `[[Prototype]]` only for Object/null candidates, primitives are ignored,
  duplicate ProtoSetters are genuine early errors, and shorthand or computed
  `__proto__` remains an ordinary data property. Object spread snapshots the
  enumerable own String/Symbol keys of the currently reachable ordinary
  source objects, performs live Get in key order, and defines C/W/E data
  properties instead of invoking inherited setters; matching the pinned
  release, primitive sources including String are ignored. The specialized
  typed `CopyDataProperties` operation is deliberately object-literal-only and
  has no destructuring exclude list. Methods, accessors, generator/async
  methods, home-object wiring, and Proxy/exotic-source spread remain explicit
  frontiers. The pinned anchors are `quickjs.c` 24485-24621 and 24850-24965
  plus the matching object/define/name/proto/copy opcodes in
  `quickjs-opcode.h`; `oracle_object_literals` locks descriptors, key and
  evaluation order, names, ProtoSetter behavior, spread, errors, and defining
  realms against QuickJS 2026-06-04.

- Untagged template literals follow QuickJS `js_parse_template` rather than a
  generic string-interpolation rewrite. A no-substitution template pushes only
  its cooked String. An interpolated template keeps the cooked head as a
  primitive receiver, performs one observable `concat` lookup before every
  substitution, parses each substitution as a full Expression, skips empty
  later cooked segments, and performs one `CallMethod` after all expressions
  have completed. Raw and cooked UTF-16, malformed-escape commitment,
  continuation anchoring, nested template/Div goal transitions, getter/call/
  coercion ordering, last-substitution source-marker inheritance, and the
  deferred, reachability-aware 65,534-slot bytecode stack limit are pinned to
  the release. The
  synthetic concat operations emit no new marker, matching upstream; exact
  expression-statement entry seeding prevents them from inheriting a prior
  statement's marker and preserves the expression start inside composites.
  Tagged templates remain explicit and unsupported pending frozen cooked/raw
  template objects and per-site identity caching.
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
  standard non-index surface is intentionally limited to the first twelve
  UTF-16/search methods, the `substring`/`substr`/`slice` subrange trio,
  `repeat`, the `padEnd`/`padStart` pair, the five-property trim group, the
  conversion pair, the four Unicode case-conversion methods,
  `Symbol.iterator` and the thirteen-property Annex-B CreateHTML family until
  later table slices land.
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
  stack depth. Compiler lowering first mirrors QuickJS `resolve_labels` for its
  exact direct Boolean/Null/Undefined/Int32 constant-condition set, replacing
  the adjacent push/branch slots with `Nop`/`Goto`; String, Float and BigInt
  conditions deliberately remain dynamic branches. Maximum stack is derived
  from the resulting control-flow walk rather than the parser's linear emission
  order, so folded dead arms and oversized calls after a terminal return remain
  valid dead bytecode while the same reachable path raises the QuickJS
  `InternalError`. Closed non-terminating control-flow graphs are valid, while a
  reachable fallthrough beyond the bytecode end is still rejected. Detached
  bytecode declares its local-frame width rather than
  inferring it from opcodes; live and dead local operands are bounded by that
  declaration and QuickJS's 65,534-slot limit. Runtime publication additionally
  checks constant kinds, frame
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
  path and `Math.pow` share Rust `f64::powf` plus QuickJS's
  `abs(base) == 1`/non-finite-exponent NaN correction, with pinned-oracle
  matrices locking the observed libc-`pow` results. Its BigInt path preserves
  negative-exponent errors, `0`/`1`/`-1`
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
  `in` and `instanceof` occupy the same relational level through dedicated
  `(2 -> 1)` bytecode. `in` validates its RHS Object before converting the LHS
  with the String `ToPropertyKey` hint, then tests ordinary own and prototype
  presence without materializing autoinit properties or invoking accessors.
  Its runtime entry returns a Completion so the later Proxy/exotic `has` path
  can preserve trap throws without changing the opcode. `instanceof` performs
  the full `JS_IsInstanceOf` sequence: RHS Object validation, observable
  `@@hasInstance` lookup, callable method invocation and ToBoolean, followed by
  the callable OrdinaryHasInstance fallback only for a nullish method. The
  existing standard native method supplies its defining-realm frame, while
  bound functions delegate through the complete path without recursing on the
  Rust host stack. Pinned differentials lock precedence, classic-for NoIn,
  evaluation and key-conversion order, custom/inherited/accessor methods,
  arbitrary throws, exact errors and source sites; host tests additionally
  lock deep bound chains and cross-realm error ownership. Proxy/exotic
  `[[HasProperty]]` and
  `[[GetPrototypeOf]]` remain wider object-model gaps.
- The runtime owns a generational Object/Shape arena. Public Object, Symbol and
  property-key roots implement Dup/Free through explicit reference counts;
  heap edges remain raw handles, zero-count teardown is iterative, and
  QuickJS-style trial deletion removes object/property/prototype cycles.
- Ordinary objects use immutable shared Shapes containing prototype plus
  ordered key/flag metadata and parallel per-object property slots. The current
  internal methods cover complete descriptor validation/storage, data get/set
  with explicit receiver, delete, own-key order, extensibility, prototype cycle
  checks, exact lone-surrogate keys, and runtime-domain rejection.
- Genuine Array objects have a dedicated heap class and a mandatory slot-zero
  `length` property. Indexed defines grow length; `ArraySetLength` performs the
  pinned conversion before the writable check, deletes descending indices,
  rolls back to the first non-configurable index, and still applies a requested
  writable-to-false transition. Realm roots own a genuine empty
  `%Array.prototype%`, `%ArrayIteratorPrototype%`, and `%Array%` constructor.
  Calls and construction implement the one-number length case, multi-element
  creation, observable `newTarget.prototype`, and cross-realm fallback. The
  constructor exposes the complete pinned static table `isArray`, `from`,
  `of`, and `@@species`; `from` covers iterable and array-like routes, mapper
  ordering, constructor receivers, CreateDataProperty, iterator closing, and
  final length Set. The currently implemented prototype subset contains `at`,
  `with`, `concat`, `every`, `some`, `forEach`, `map`, `filter`, `reduce`,
  `reduceRight`, `fill`, `find`, `findIndex`, `findLast`, `findLastIndex`,
  `indexOf`, `lastIndexOf`, `includes`, `join`, `toString`, `toLocaleString`,
  `pop`, `push`, `shift`, `unshift`, `reverse`, `toReversed`, `sort`,
  `toSorted`, `slice`, `splice`, `toSpliced`, `copyWithin`, `flatMap`, `flat`,
  generic `values`, `keys`, `entries`, and the `@@iterator` alias in their
  pinned filtered order.
  `at` uses
  saturating Int64 index conversion and HasProperty-before-Get; the three
  searches snapshot ToLength, skip `fromIndex` conversion for zero length,
  preserve omitted-versus-explicit-undefined behavior, and use QuickJS's
  negative-offset Int64 clamp. `includes` performs ordinary Get and
  SameValueZero so a hole can match `undefined`; index searches use
  HasProperty and strict equality so holes are skipped while inherited values
  remain visible. All four are generic over ordinary and primitive receivers,
  preserve getter/coercion order, and allocate native errors in the method's
  defining realm. `with` reuses those index rules but allocates a defining-realm
  base Array without constructor/species lookup. It enforces QuickJS's signed
  31-bit dense allocation limit before indexed reads, skips the replaced
  source getter, copies the others in ascending HasProperty/Get order, and
  turns holes into own `undefined` elements. `concat` uses ArraySpeciesCreate
  before examining spreadability, processes the boxed receiver followed by
  actual arguments, and observes `@@isConcatSpreadable` before the Array
  fallback. Spread values snapshot ToLength and copy present Has/Get values
  while preserving holes; single values remain unboxed. Numeric writes use
  CreateDataProperty, whereas the final result length uses an ordinary
  throwing Set. Custom result properties under holes therefore survive, and
  partial writes, inherited length setters, the MAX_SAFE limit, and QuickJS's
  exact `Array loo long` diagnostic remain observable. `fill` is a generic
  in-place mutation: it snapshots ToLength, converts explicit non-undefined `start`
  before `end` even for an empty range, and applies ascending ordinary throwing
  Set operations. Holes become own values, inherited setters remain observable,
  a failing write preserves earlier mutations, and boxing/native errors use the
  method's defining realm while user throws are preserved. `every`, `some`,
  and `forEach` implement the non-allocating modes of QuickJS's shared callback
  kernel: they validate the callback after ToLength, skip holes through
  HasProperty/Get while observing inherited values and mutation, pass the
  boxed receiver plus index/value and exact `thisArg`, and preserve the pinned
  short-circuit or exhaustive completion mode. Their allocating `map` and
  `filter` modes use the complete `ArraySpeciesCreate` path for genuine Arrays,
  including constructor/species observation, custom result objects, and the
  cross-realm default-Array exception. `map` preallocates the snapshotted length
  and preserves holes; `filter` starts at zero, applies ToBoolean only to the
  callback result, and compactly defines the original values. Both use
  CreateDataProperty without a final length Set on custom results. `reduce`
  and `reduceRight` share QuickJS's directional accumulator kernel. They
  distinguish an omitted initial value from an explicitly supplied `undefined`, scan holes with
  HasProperty to select the first accumulator, throw the exact `empty array`
  TypeError when none exists, and pass accumulator/value/index/boxed receiver
  to each callback with undefined `this`. The four `find*`
  methods share QuickJS's callback kernel: they validate the predicate after
  ToLength even for empty receivers, Get and visit every snapshotted index
  including holes, pass value/index/the original unboxed receiver, traverse in
  the selected direction, and preserve callback/Get abrupt completions and
  defining-realm native errors. `join` snapshots ToLength before converting
  its separator, then performs a direct Uint32-indexed Get per slot so holes,
  inherited values, mutation, and the pinned post-2^32 wrap remain observable.
  Nullish elements contribute empty fields. `toLocaleString` shares that
  kernel but ignores all supplied arguments, invokes each non-nullish
  element's locale method with zero arguments, and ToStrings its return value.
  Array `toString` dynamically reads `join`, returns a callable join's result
  without conversion, and otherwise uses the intrinsic Object-toString
  fallback. QuickJS's recursive-array behavior is retained as a catchable
  `InternalError: stack overflow`; a deterministic call-entry ceiling protects
  the Rust host stack. Its current 64-stringification-frame limit can reject
  deeper acyclic nesting earlier than pinned QuickJS and observes fewer
  recursive side effects; an iterative native-call trampoline remains required
  for exact stack-threshold parity. The 30-bit StringBuffer failure order is
  covered with a reduced-limit unit probe, including Gets and locale invocation
  that occur after separator append failure while later result ToString is
  skipped.
  `pop`/`shift` and `push`/`unshift` use the two shared QuickJS magic-selected
  mutation kernels. They snapshot ToLength, retain full Int64 property keys,
  and perform ordinary throwing Set/Delete operations. `shift` copies forward
  while `unshift` copies backward, using HasProperty before Get so inherited
  values and holes are preserved exactly; `pop` and `shift` save their result
  before later mutations can fail. All four perform the final length Set even
  for an empty removal or zero supplied arguments. Insertion uses the actual
  argument count, rejects a result above MAX_SAFE before indexed writes with
  QuickJS's exact `Array loo long` TypeError, and otherwise preserves every
  completed prefix mutation on a later failure. Genuine Arrays also retain the
  Uint32 length boundary: a push at length 2^32-1 first creates the ordinary
  `"4294967295"` property, then the final length Set throws RangeError without
  rolling that property back.
  `reverse` snapshots ToLength, then examines each lower/upper pair through
  full Int64 HasProperty/Get operations before it begins that pair's mutation.
  Its four presence combinations use QuickJS's exact Set/Set, Set/Delete,
  Delete/Set, or no-op order, so sparse holes and inherited values move without
  densification and every successful prefix survives a later failure. It never
  writes length and returns the original boxed receiver. `toReversed` instead
  preallocates a complete dense result buffer, reads source indices in descending
  order, leaves an own `undefined` slot for every hole, and returns a
  defining-realm base Array. It does not observe `constructor` or `@@species`.
  The pinned implementation deliberately uses
  HasProperty followed by a conditional Get rather than the specification's
  unconditional Get; the eventual Proxy path must preserve that visible `has`
  trap. It also inherits `js_allocate_fast_array`'s signed 31-bit length ceiling,
  throwing defining-realm `RangeError: invalid array length` before any indexed
  read above that boundary. As with `with`, Rust reserves and initializes the
  equivalent dense value buffer before source access, but allocates the actual
  Array object after those reads; exact allocator-failure ordering still needs
  a bulk dense-array allocator.
  `sort` validates a supplied comparator before ToObject, snapshots ToLength,
  and collects present values in ascending HasProperty/Get order. Holes are
  omitted and explicit `undefined` values are counted separately. Its fallible
  iterative sorter is a direct port of pinned `rqsort`, including the exact
  median-of-three/insertion/heapsort comparison choreography. Default ordering
  lazily caches each slot's ToString result and compares raw UTF-16 code units;
  a custom comparator receives undefined `this`, skips bit-identical raw
  values, ToNumbers its result, and uses original positions to stabilize ties.
  Source String literals now retain QuickJS's runtime-wide atom identity across
  functions, eval publications, contexts, and property-key round trips; the
  canonical decimal tagged-integer spellings `"0"` through `"2147483647"`
  deliberately remain independent per constant-pool occurrence, and released
  atoms keep only weak canonical identities while a derived String value is
  still live.
  Writeback first places non-undefined values, then always Sets each undefined,
  then Deletes the hole suffix. Matching a pinned QuickJS optimization, a
  non-undefined slot whose original position already equals its destination
  skips Set entirely; accessor effects and comparator mutations can therefore
  survive, while later Set/Delete failures retain every completed prefix.
  `toSorted` validates the same way, preallocates the signed-31-bit dense result,
  copies source indices in ascending conditional HasProperty/Get order so holes
  become own undefined, then runs the identical sort kernel without consulting
  constructor or species. Its defining-realm base Array is now created before
  comparator/ToString effects, though—as for `with` and `toReversed`—the actual
  object still follows source reads rather than QuickJS's earlier allocation;
  recoverable OOM ordering remains a bulk-allocator gap. Recursive comparator
  and ToString calls use a deterministic 16-sort-frame safety ceiling and throw
  catchable `InternalError: stack overflow`; pinned QuickJS permits more frames,
  so an iterative native-call trampoline is still required for exact threshold
  and side-effect parity. The pinned conditional-Get Proxy behavior remains
  attached to the wider pending Proxy/object-model slice.
  `slice` and `splice` retain QuickJS's shared magic-selected kernel. Both
  snapshot ToLength, apply saturating Int64 relative-index clamps, distinguish
  omitted arguments from explicit `undefined` where `argc` requires it, and
  complete `ArraySpeciesCreate` before copying present values in ascending
  HasProperty/Get/CreateDataProperty order. Holes remain holes, inherited
  values become own C/W/E data properties, species may return the source
  itself, and even an empty result receives the final ordinary throwing length
  Set. `splice` finishes that entire removed-result phase before source
  mutation. It then moves a shrinking tail forward or a growing tail backward,
  Deletes an old tail in descending order, Sets inserted items in ascending
  order, and always Sets the final source length. Every completed result write,
  move, Delete, insertion and genuine-Array length growth remains visible when
  a later operation fails. The full MAX_SAFE and ordinary `"4294967295"`
  property boundaries are retained, including QuickJS's exact
  `TypeError: Array loo long` spelling and the later genuine-Array RangeError.
  `toSpliced` uses the adjacent non-species path: it checks MAX_SAFE, reserves a
  signed-31-bit dense defining-realm result before indexed reads, queries only
  the retained prefix and suffix in ascending conditional Has/Get order, and
  turns every retained hole into an own `undefined`. Constructor/species,
  deleted indices and a replaceable global Array are not observed. Its
  MAX_SAFE overflow is a TypeError while the dense INT32 ceiling is a
  RangeError, both with `invalid array length`. A deterministic four-frame
  slice-family safety ceiling makes recursive getters catchable as
  `InternalError: stack overflow`; pinned QuickJS permits a deeper
  platform-stack-dependent chain, so exact threshold and side-effect parity
  still require the general native-call trampoline. As with the other dense
  change-by-copy methods, Rust reserves the complete value buffer before
  source access but creates indexed Array storage afterward; exact recoverable
  allocator ordering and bulk-storage complexity remain pending. Proxy trap
  behavior remains attached to the wider pending Proxy/object-model slice.
  `copyWithin`
  snapshots and clamps all three bounds in QuickJS order, selects a backward
  traversal only for overlapping ranges, and performs source HasProperty/Get
  followed by a throwing target Set, or a throwing Delete for a source hole.
  Inherited source values, deletion failures, and partial mutation remain
  observable without allocating a result Array.
  `flatMap` and `flat` share an iterative port of `JS_FlattenIntoArray`.
  Both snapshot ToLength before mapper validation or saturating Int32 depth
  conversion, then complete ArraySpeciesCreate with zero before any indexed
  source access. Their depth-first frames use HasProperty followed by
  conditional Get, compact holes at every visited level, include inherited
  values, snapshot each nested Array length on entry, and flatten only genuine
  Arrays rather than consulting `@@isConcatSpreadable`. `flatMap` invokes its
  mapper only for present outer elements with value/index/boxed-source and the
  exact `thisArg`; returned Arrays flatten once without remapping. Custom
  species results receive throwing CreateDataProperty writes with no final
  length Set, so aliases, rejected definitions and every completed prefix stay
  observable. The MAX_SAFE failure is the exact `TypeError: Array too long`.
  Explicit DFS storage avoids Rust host recursion, while a deterministic 3833
  frame ceiling keeps cyclic or extremely deep flattening catchable as
  `InternalError: stack overflow`; the pinned C-stack threshold remains
  platform dependent, so the deepest failing case can expose a different
  completed target prefix. Mapper/getter code that
  recursively re-enters `flatMap` or `flat` also uses a separate 8-active-call
  ceiling because native-to-bytecode calls still consume the Rust host stack;
  pinned QuickJS permits hundreds on the current oracle, so the general
  iterative call trampoline remains necessary for exact threshold and
  side-effect parity.
  Array Iterators re-read Uint32 length on every `next`, observe holes and
  mutation through ordinary Get, allocate entry-pair Arrays in the defining
  realm, use the raw native-next ABI in for-of, and eagerly release their source
  on exhaustion. The pinned Array prototype algorithm table is now complete.
  `Array.prototype[Symbol.unscopables]` follows QuickJS's lazy object-table
  publication: the outer property is a non-writable, non-enumerable,
  configurable data property whose auto-init slot retains its defining realm
  until materialization or removal. Each realm receives a distinct
  null-prototype object with the exact pinned 16-key order (`at`, `copyWithin`,
  `entries`, `fill`, `find`, `findIndex`, `findLast`, `findLastIndex`, `flat`,
  `flatMap`, `includes`, `keys`, `toReversed`, `toSorted`, `toSpliced`,
  `values`); every value is `true` and every inner property is writable,
  enumerable and configurable. QuickJS 2026-06-04 does not include `with` in
  this table.
  Source-level `with` statement parsing and object-environment lookup remain a
  separate pending language/environment slice.
  The pinned runtime anchors are `quickjs.c`
  212, 5628-5671, 9433-9524, 10369-10592, 13210-13663, 41472-42226,
  42228-43118, 43122-43335, 43344-43454, 44519-44583, and 56220-56390.
- Every realm now publishes `%Object%` as a constructor-or-function native
  linked to `%Object.prototype%`. Call and construction preserve existing
  objects, box every primitive family in the defining realm, allocate ordinary
  objects for nullish values, and honor custom `newTarget.prototype` with the
  new-target realm fallback. The pinned static table is now complete and is
  exactly
  `create`, `getPrototypeOf`, `setPrototypeOf`, `defineProperty`,
  `defineProperties`, `getOwnPropertyNames`, `getOwnPropertySymbols`,
  `groupBy`, `keys`, `values`, `entries`, `isExtensible`,
  `preventExtensions`, `getOwnPropertyDescriptor`,
  `getOwnPropertyDescriptors`, `is`, `assign`, `seal`, `freeze`, `isSealed`,
  `isFrozen`, `fromEntries`, `hasOwn`.
  Prototype mutation keeps
  same-value success plus exact immutable, non-extensible and cycle failures.
  Descriptor conversion follows QuickJS's inherited field probes and
  `enumerable`, `configurable`, `value`, `writable`, `get`, `set` order,
  including its `invalid getter`/`invalid setter` exception-overwrite quirk.
  The pinned non-spec `defineProperties` path snapshots enumerable own keys but
  converts and defines each descriptor immediately; its flag filtering does
  not materialize lazy AutoInit properties. Own-name/symbol results are genuine
  defining-realm Arrays and cover ordinary, Array and String-exotic ordering.
  `groupBy` validates its callback before touching the iterable, caches `next`
  once, and passes each value plus its monotonic safe-integer index with the
  defining-realm global object as callback `this`. Callback and property-key
  conversion failures close the iterator while preserving the original throw;
  iterator-step and internal Array-push failures deliberately do not close it.
  The result has a null prototype, supports string and Symbol keys, and defines
  each group as a writable, enumerable, configurable property containing a
  defining-realm Array. Appends reuse QuickJS's ordinary push Set/final-length
  path, so an inherited Array index setter or rejection remains observable. A
  deterministic nine-active-call guard keeps recursive callbacks catchable as
  `InternalError: stack overflow`; pinned QuickJS permits a deeper
  platform-stack-dependent chain, so exact threshold parity still requires the
  general native-call trampoline.
  `keys`, `values` and `entries` share the pinned `js_object_keys` kernel: they
  box through `ToObject`, snapshot all own string keys once, then re-read each
  current descriptor and skip a key which disappeared or became
  non-enumerable. Only `values` and `entries` perform a subsequent Get, so an
  earlier getter can delete, hide or redefine a later snapshotted key while a
  newly added key remains absent. Numeric/string ordering, Symbol exclusion,
  Array and String-exotic keys, and compact defining-realm result Arrays match
  QuickJS; `entries` pairs are defining-realm Arrays as well. A conservative
  nine-active-call family guard, selected from the heaviest measured getter and
  helper reentry path on the default 2 MiB libtest thread, converts deeper
  `values`/`entries` recursion into a catchable `InternalError: stack overflow`.
  Pinned QuickJS permits a much deeper platform-dependent chain, so exact
  threshold parity and byte-accurate interleaved-frame accounting still require
  the native-call trampoline. Proxy trap order and invariants remain part of the
  explicit global Proxy boundary because the runtime does not yet publish
  Proxy objects.
  `isExtensible` and `preventExtensions` preserve QuickJS's deliberate
  non-boxing branch: every primitive, including nullish, Symbol and BigInt,
  reports non-extensible, while `preventExtensions` returns that exact
  primitive unchanged. Ordinary objects use their existing extensibility bit;
  prevention is irreversible and idempotent, returns the original object, and
  leaves existing property descriptors untouched. Proxy trap forwarding and
  invariants, plus the resizable TypedArray rejection branch, remain explicit
  boundaries until those object kinds exist; the ordinary API is not presented
  as their completion-aware internal method.
  Descriptor reads preserve `ToObject` before property-key conversion, never
  call a stored getter, and publish fresh defining-realm ordinary objects. Data
  fields are created in `value`, `writable`, `enumerable`, `configurable` order;
  accessor fields use `get`, `set`, `enumerable`, `configurable`, with every
  field writable, enumerable and configurable. The plural operation snapshots
  all own string and Symbol keys once, then re-reads each current descriptor,
  skips a deleted key, ignores additions, and does not dynamically invoke a
  monkey-patched singular method. A nine-active-call family guard plus a shared
  weighted native re-entry budget converts both direct and interleaved
  property-key coercion into catchable `InternalError: stack overflow` before
  Rust exhausts the host stack. The weights preserve the previously measured
  deeper join, sort, slice and flatten ceilings, but pinned QuickJS still
  permits platform-dependent chains, so exact byte-threshold parity requires
  the native-call trampoline. Proxy descriptor traps/invariants, integer-indexed
  TypedArray details and module-namespace exotic descriptors remain
  explicit object-model boundaries. Future Proxy work must preserve two pinned
  deviations: incomplete identity checks for some frozen descriptors, and the
  nested-Proxy undefined-trap path which bypasses target `[[IsExtensible]]`.
  `Object.is` directly applies SameValue without coercion: all NaN payloads
  compare equal, positive and negative zero remain distinct, primitive values
  compare by value, and objects and Symbols compare by identity.
  `Object.assign` boxes its target in the defining realm, skips nullish sources,
  and handles every other source from left to right. Each supported source
  snapshots its currently enumerable own string and Symbol keys before any
  Get, then performs Get followed by throwing Set for every retained key. This
  preserves inherited setters, getter/setter ordering, partial mutation on an
  abrupt completion, String indices, and QuickJS's pinned ordinary-object
  deviation: deletion or enumerable-bit changes after the snapshot do not
  remove a retained key, while newly enumerable or newly added keys stay
  absent. Shape-time filtering leaves non-enumerable AutoInit slots lazy.
  Direct getter/setter recursion has a nine-call family guard and interleaved
  recursion is covered by the shared weighted budget. Proxy descriptor recheck
  and invariant quirks, stale TypedArray index snapshots, and module namespace
  sources remain explicit object-model boundaries.
  `seal` and `freeze` preserve every primitive without boxing. For objects they
  first prevent extensions and then snapshot every own string and Symbol key.
  `seal` clears configurability while preserving data writability;
  `freeze` additionally clears writability only for a currently writable data
  descriptor. Both preserve values, enumerability and accessor identity, never
  execute stored accessors, materialize compatible AutoInit slots in key order,
  and return the exact input object. `isSealed` and `isFrozen` return true for
  primitives and preserve QuickJS's observable non-spec order: snapshot keys,
  read and short-circuit on current descriptors, and query extensibility only
  after every descriptor passes. Ordinary, Array and String-wrapper descriptor
  transitions are covered, including mapped and unmapped Arguments objects;
  Proxy trap order/partial failures, non-empty TypedArray rejection, and module
  namespace behavior remain explicit object-model boundaries until those
  exotic kinds exist.
  `fromEntries` allocates a fresh ordinary result in the builtin's defining
  realm before reading its input, obtains and caches a synchronous iterator's
  `next`, and requires every yielded entry itself to be an object. It reads
  entry properties `0` then `1`, converts the key only after both Gets, and
  defines an own writable, enumerable, configurable data property, so duplicate
  keys overwrite in place while Symbol and `__proto__` keys remain direct data
  keys. Once an iterator object exists, every later abrupt completion—including
  `next` lookup/call, iterator-result `done`/`value`, entry Gets and key
  conversion—performs QuickJS's pending-exception `IteratorClose`; return
  getter/call failures cannot replace the original throw. Normal exhaustion
  does not close. A four-active-call guard plus the shared weighted budget keeps
  direct and interleaved getter/key-coercion recursion catchable. Proxy entry
  traps, Map/Set iterators, generators/finally, TypedArrays and module namespace
  entries remain explicit boundaries until those object kinds exist.
  `hasOwn` converts and boxes its target in the defining realm before converting
  its property key, deliberately reversing the legacy prototype method's
  observable conversion order. It probes only the resulting object's own
  descriptor, so inherited properties are absent, stored accessors are not
  called, String UTF-16 indices and `length` are present, Symbols retain
  identity, and lazy AutoInit slots remain unmaterialized. A measured
  nine-active-call family guard turns recursive `@@toPrimitive` reentry into a
  catchable `InternalError` before the Rust host stack is exhausted; exact
  QuickJS platform-stack depth still awaits the general native-call trampoline.
  Proxy `getOwnPropertyDescriptor` traps and invariants, integer-indexed
  TypedArrays, and module namespaces remain the corresponding explicit
  object-model boundaries.
  Anchors: `quickjs.c` 8905-8950, 10680-10702, 15840-15927, 16639-16675,
  16923-16996, 39796-40716, 40748-40927,
  50728-50831, 50992-51107, 52115-52230, and 56291-56313.
- Shape caches are weak and unlink by finalized generational Shape ID. Shape
  and Symbol atom ownership is paired through heap cleanup, including failure
  paths and runtime teardown.
- Each Context now owns explicit realm roots for `%Object.prototype%`, a
  callable `%Function.prototype%`, the global object and the null-prototype
  global lexical-binding object (`global_var_obj` in QuickJS). Default object
  allocation uses its realm prototype, and `%Object.prototype%` carries
  QuickJS's immutable-prototype bit.
- The realm root set reserves five typed primitive `class_proto` slots. Number,
  Boolean, Symbol and BigInt retain their complete intrinsic slices. `%String%`
  now publishes its complete constructor own table, while its prototype remains
  an explicitly incomplete stack built on the strictly named `String exotic
  core/substrate`. Its realm
  slot roots a genuinely branded wrapper around the empty UTF-16 string whose
  initial own `length` has `W0 E0 C1`. Sloppy ordinary-function boxing creates a
  fresh String-payload wrapper with `W0 E0 C0` own `length`. In-range UTF-16
  code-unit indices are virtual `W0 E1 C0` properties integrated with
  get-own-property, define-own-property, has-own-property, delete-property and
  own-property-keys; ownKeys merges them with stored numeric, string and symbol
  keys in QuickJS order. The UTF-16 prefix then installs `at`, `charCodeAt`,
  `charAt`, `concat`, `codePointAt`, `isWellFormed`, `toWellFormed`, `indexOf`,
  `lastIndexOf`, `includes`, `endsWith` and `startsWith`, then skips the pending
  `match`/`matchAll`/`search`/`split` entries before publishing `substring`,
  `substr`, `slice` and `repeat`, skips the pending `replace`/`replaceAll`, and
  publishes `padEnd` then `padStart`, followed by `trim`, `trimEnd`,
  `trimRight`, `trimStart` and `trimLeft`, in pinned table-relative order ahead
  of the conversion core's exact `toString`/`valueOf` brand methods. It then
  publishes `toLowerCase`, `toUpperCase`, `toLocaleLowerCase` and
  `toLocaleUpperCase`, followed by `Symbol.iterator`, and appends the thirteen
  Annex-B CreateHTML methods before the `constructor` back-reference.
  These generic methods preserve
  `JS_ToStringCheckObject`, `JS_ToInt32Sat`, raw UTF-16 code units and lone
  surrogates; concat converts actual arguments sequentially and enforces
  QuickJS's `(1 << 30) - 1` length cap. The index-search pair converts receiver,
  search value and only a present position in that order, scans exact code
  units, and retains QuickJS's distinct `indexOf` clamping and `lastIndexOf`
  NaN/default-position behavior. The regexp-aware `includes`/`endsWith`/
  `startsWith` family additionally performs `IsRegExp` through `Symbol.match`
  before search-value conversion, preserves every abrupt-completion boundary,
  clamps position with `JS_ToInt32Clamp`, and scans UTF-16 code units. Until a
  RegExp object class lands, the internal-brand fallback remains an exhaustive
  false branch with pinned oracle-only vectors. The three distinct generic
  subrange callables have `length=2`, convert receiver, start and a non-undefined
  end in that order, use QuickJS's saturated Int32 clamps rather than generic
  `ToIntegerOrInfinity`, and copy exact UTF-16 ranges with full-range handle
  reuse plus wide-to-Latin-1 compression. The generic `repeat` callable has
  `length=1`, converts its receiver before a saturated Int64 count, distinguishes
  `invalid repeat count` from `invalid string length`, preserves raw UTF-16 and
  source width in one exact flat buffer, and turns repeat-buffer allocation
  failure into `InternalError:out of memory`. The generic-magic `padEnd` and
  `padStart` callables have `length=1`, convert receiver and saturated Int32
  target before observing an optional filler, return early for an already-long
  source or empty filler, and only then enforce the 30-bit result cap. Their
  narrow-first fallible buffer repeats and truncates raw UTF-16 code units,
  chooses the final width from copied content, and maps both initial and
  widening reservation failure to defining-realm `InternalError:out of
  memory`. The generic-magic trim callables all have `length=0`; `trim`,
  `trimEnd` and `trimStart` retain QuickJS's magic masks 3, 2 and 1. The
  writable, configurable `trimRight` and `trimLeft` properties initially copy
  exactly the `trimEnd` and `trimStart` function objects, including their
  canonical function names, while later alias/canonical property mutation is
  independent in either direction. Only the receiver is converted, with a
  String hint; every argument is ignored. The raw UTF-16 scans recognize the
  exact 25 `lre_is_space` code units from U+0009..U+000D, U+0020, U+00A0,
  U+1680, U+2000..U+200A, U+2028, U+2029, U+202F, U+205F, U+3000 and U+FEFF;
  U+0085, U+180E and U+200B remain non-space boundaries. A full-range result
  reuses the converted String, an all-space result uses the narrow empty
  String, and a partial result preserves exact code units while compressing a
  wide source when the retained range fits Latin-1. Partial-result reservation
  failure is catchable as defining-realm `InternalError:out of memory`, while
  the full-range and all-space paths do not enter that checked partial-result
  reservation path or consume its scoped failure hook. Allocations surrounding
  those paths remain within the general allocator gap described below.
  The four case-conversion properties are distinct AutoInit GenericMagic
  callables with `length=0`, even though each locale-named method selects the
  same lower/upper kernel as its ordinary counterpart. Only the receiver is
  converted, using `JS_ToStringCheckObject`; every locale and extra argument is
  ignored without property access or coercion. Conversion ports QuickJS's
  checksum-pinned Unicode 17 `case_conv_table1`, `case_conv_table2` and
  `case_conv_ext` arrays together with its compressed `Cased` and
  `Case_Ignorable` properties. Forward/backward UTF-16 code-point traversal
  preserves unmatched surrogates, applies astral and multi-code-point
  mappings, and implements the context-sensitive Greek final-sigma rule by
  skipping Case_Ignorable code points on both sides. The fallible narrow-first
  output builder widens only when mapped content requires UTF-16, checks the
  30-bit String limit across expansions, and reports defining-realm
  `InternalError:string too long` or `InternalError:out of memory`; its scoped
  reservation failure is catchable and one-shot. Separate method identities,
  deletion/replacement, cross-realm calls, GC, raw surrogate/rope boundaries,
  shared String recursion and recovery are differential- and white-box-tested.
  The pinned anchors are `quickjs.c` 46215-46304 and 46656-46659 plus
  `libunicode.c` 51-190 and 347-376.
  The Annex-B CreateHTML slice publishes distinct AutoInit GenericMagic
  callables for `anchor`, `big`, `blink`, `bold`, `fixed`, `fontcolor`,
  `fontsize`, `italics`, `link`, `small`, `strike`, `sub` and `sup`. The four
  attribute variants have `length=1`; the other nine have `length=0`. Their
  exact `a/name`, `font/color`, `font/size`, `a/href` and no-attribute tag
  mappings port QuickJS's selector table. Receiver conversion precedes buffer
  creation; an attribute variant then applies `JS_ToStringCheckObject` to only
  argv[0], rejecting a directly missing, undefined or null value, while every
  extra argument and every argument to a no-attribute variant is ignored.
  Attribute output replaces only raw U+0022 with `&quot;`; ampersands, angle
  brackets, NUL, astral pairs and lone surrogates remain raw, as does the
  complete source String. The narrow-first String buffer preserves that code-
  unit stream and QuickJS's latched-error order: an earlier checked length or
  reservation failure does not skip observable attribute conversion, and a
  later user throw still wins. The final checked failures are defining-realm
  `InternalError:string too long` and `InternalError:out of memory`; the scoped
  reservation hook is one-shot and normal calls recover. Cross-realm calls,
  saved-callable realm retention, recursion through the shared runtime stack
  guard, deletion, replacement and GC are covered. The pinned anchors are
  `quickjs.c` 4002-4338 (`StringBuffer` error latching), 46546-46615
  (`js_string_CreateHTML`) and 46661-46674 (the thirteen prototype entries).
  Together with `length`, the conversion pair, `Symbol.iterator` and the
  `constructor` back-reference, the implemented String prototype now covers
  45/53 own keys. This forty-five-key list is only the QuickJS-relative order
  filtered to implemented keys, not a claim of full prototype parity. The
  callable/constructible
  global `%String%` owns `length`, `name`, lazy `fromCharCode`, `fromCodePoint`
  and `raw`, then the prototype relationship in the pinned order and
  descriptors. Calls retain the Symbol descriptive-string exception,
  construction creates a branded wrapper, and the statics preserve QuickJS's
  UTF-16, code-point and template-raw conversion/error order. Primitive
  non-index reads and writes now traverse the bytecode realm's String prototype
  with the raw receiver, and String receivers use the implemented
  Object-prototype boxing/tag/value routes in the native method's defining
  realm. The common String value kernel uses compact Latin-1/UTF-16 leaves plus
  QuickJS-shaped ropes: 512/8192 flat thresholds, short head/tail merging,
  depth-60/Fibonacci rebalance, O(1) length, cross-leaf UTF-16 access,
  content-based equality/hash and cached linearization. VM `+`, native
  `concat`, and the implemented internal concatenation sites all use its
  checked 30-bit-length path; atom/property-key publication stores a linearized
  key. Public valid-UTF-8 and exact-UTF-16 constructors are fallible, reject
  `(1 << 30)` code units before unbounded reserve, and ignore hostile upper
  iterator hints. The shared latched UTF-16 builder is used by backtrace and
  Annex-B escape output; lexer String/template/identifier buffers, dynamic
  Function source assembly, and URI output all apply the same checked
  arithmetic. Lexer overflow stops immediately as an `InternalError` with
  message `string too long`, whereas URI validation continues and a later
  `URIError` overrides an earlier output overflow, matching the pinned native
  loops. `try_from_bytes` additionally matches `JS_NewStringLen`'s explicit
  byte length, embedded NUL, WTF-8 surrogate acceptance, non-BMP pair output,
  legacy UTF-8 lead shapes and idiosyncratic invalid-run skip. The
  `try_to_wtf8_bytes`/`try_to_cesu8_bytes` pair emits the payload bytes of
  `JS_ToCStringLen2` without its synthetic trailing NUL; the normal mode joins
  valid surrogate pairs even across rope leaves, while CESU-8 encodes each
  code unit independently. Output reservation is fallible and does not apply
  the JavaScript String length cap to expanded byte buffers. Native Error
  materialization passes its current messages through the pinned `char[256]`
  buffer: at most 255 raw bytes survive, an embedded formatted NUL terminates
  `JS_NewString`, and a split UTF-8 tail is decoded with the same replacement
  rules. The migrated not-constructor `%s` route streams the exact WTF-8
  function name, stops that argument at NUL, then continues its literal
  suffix. A private byte-message sidecar now crosses compiler and VM `Error`
  transport without re-encoding through the public UTF-8 diagnostic cache.
  The current atom-named Type, Reference and Syntax diagnostics additionally
  reproduce `JS_AtomGetStr(..., char[64], 64)`. For table-backed text atoms,
  only narrow all-ASCII spellings use the unbounded atom-pointer fast path; all
  other text spellings use the scratch path, encode each UTF-16 code unit
  independently, and stop before starting a unit once 58 bytes have already
  been written. Argument NUL still stops `%s`
  while the literal suffix continues, and the result then enters the shared
  255-byte outer buffer. The migrated callers cover ordinary/global read-only
  writes, fixed-name nullish reads, nullish writes, missing bindings, TDZ and
  VarRef descriptor reads, VM `ThrowReadOnly`, and reserved-identifier
  validation.
  The remaining eight String-prototype own keys (`match`, `matchAll`, `search`,
  `split`, `replace`, `replaceAll`, `normalize` and `localeCompare`),
  Context-level observable
  `ToString`, borrowed C-pointer/refcount ownership, native atom
  diagnostics attached to not-yet-implemented private-field/module/
  global-var/function-declaration surfaces, exact byte-sidecar migration for the remaining
  numeric-parser and lexer diagnostic builders, and general recoverable
  allocator failure handling stay unpublished.
  `%Number.prototype%` is a Number-class wrapper
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
- Every realm publishes the complete pinned `%Math%` intrinsic as a writable,
  non-enumerable, configurable global AutoInit property. Materialization
  preserves the upstream 37-method order, eight frozen constants and
  configurable `@@toStringTag = "Math"`; the methods themselves remain lazy
  native properties. UnaryF64 and BinaryF64 cproto adapters perform
  defining-realm `ToNumber` conversions in argument order, while custom
  kernels preserve signed zero, NaN and integer coercion behavior for the
  remaining selectors. `Math.random` advances a realm-local xorshift64-star
  stream, and `Math.sumPrecise` ports the pinned signed wrapping-limb
  accumulator, Number-only iterator contract and `IteratorClose`/no-close
  split. Rust tests and a dedicated QuickJS differential lock the complete
  graph, descriptors, key order, call-only behavior, algorithms, cross-realm
  conversions and iterator failures.
- Every realm publishes the complete pinned `%Reflect%` intrinsic as a
  writable, non-enumerable, configurable global AutoInit property. The
  non-constructable namespace has the exact 13-method table, names, lengths,
  Generic/GenericMagic cproto split, key order and configurable
  `@@toStringTag = "Reflect"`. Its `apply` and `construct` reuse the shared
  QuickJS-sized array-like argument-list kernel while preserving the pinned
  validation and observable conversion order. The remaining methods delegate
  to the ordinary property/descriptor/prototype/extensibility kernels with the
  exact target checks, receiver behavior, boolean failure results and ordered
  string/symbol key arrays. Dedicated Rust and QuickJS differential tests lock
  mutation/deletion of the lazy global, cross-realm result/error ownership,
  callback recursion recovery, detached-method lifetime, final realm GC and
  the complete graph and semantic vector.
- The non-observable `%Date%` foundation now follows the pinned QuickJS
  2026-06-04 implementation at `quickjs.c` 47223-47279 and 54786-55939. The
  heap has a genuine edge-free Date payload with mutable binary64 milliseconds,
  exact invalid-Date NaN branding, exhaustive class/payload validation and a
  dedicated realm-root slot and GC edge for QuickJS's ordinary, unbranded
  `%Date.prototype%` (its value methods therefore reject that prototype).
  Pure modules port the proleptic Gregorian calendar and TimeClip/MakeDate
  evaluation order, the ISO-first and legacy 127-code-unit parser, and all
  eight UTC/local/fixed-locale formatter modes including extended years, GMT
  offsets and Invalid Date behavior. A
  runtime-owned injectable host boundary supplies `SystemTime` and
  JavaScript-sign timezone offsets through `tz-rs`; extracting
  `ordinary_to_primitive` from the runtime facade also prepares Date's distinct
  default/string-hint path without duplicating Object conversion semantics.
  This foundation deliberately does **not** publish the global `Date`
  constructor or its static/prototype native table yet, so it exposes no new
  JavaScript behavior and must not move the Test262 baseline. One host parity
  limitation remains explicit: on Windows with no `TZ` environment variable,
  the current `tz-rs` local-zone lookup fails into the UTC fallback. A real
  cross-platform local-time backend is required before Date or full QuickJS
  feature parity can be claimed.
- The global object has QuickJS's dedicated payload and hidden
  `uninitialized_vars` object. Global data properties and the lexical-binding
  object can store `PropertySlot::VarRef` cells; define, descriptor lookup,
  assignment, accessor conversion and delete preserve shared-cell identity.
  Deleting or converting a global property moves a still-referenced cell back
  to the hidden object, resets it to Uninitialized, and allows a later data
  definition to reconnect the same closures. These VarRef, hidden-object,
  Shape and atom edges participate in reference counting and trial-deletion GC.
  Script-wide simple-name `var` and direct Program simple-name `let`/`const`
  now drive this substrate through production declaration instantiation rather
  than test-only helpers. The
  global lexical object stores the persistent binding while a same-name
  configurable `globalThis` property keeps a separate value; existing global
  VarRef cells are split exactly as in QuickJS so older closures reconnect to
  the lexical cell without changing the property value. Runtime preflight reads
  raw own shape flags and invokes no getter or autoinitializer. All declaration
  checks precede all creation, including source-order conflict priority and
  no-partial-binding behavior, and a non-extensible global object still accepts
  lexical declarations because they do not create properties on it.
- Every realm installs `Infinity`, `NaN` and `undefined` as non-writable,
  non-enumerable, non-configurable global data properties, matching the pinned
  QuickJS 2026-06-04 descriptors and direct-delete results. The implemented
  global string-key surface preserves upstream relative own-key order as the
  Error family, `Array`, `Object`, `Function`, `parseInt`, `parseFloat`, `isNaN`,
  `isFinite`, the six URI/escape functions, the three constants, `Number`,
  `Boolean`, `String`, `Math`, `Reflect`, `Symbol`, `globalThis`, then `BigInt`. This is
  not a claim that the wider global builtin table is complete.
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
  bootstrap preserves the exact relative order of all implemented entries:
  Math, Reflect, Symbol, `globalThis`, then BigInt. The remaining intervening
  generator intrinsics must be inserted before `globalThis` when they land.
- Unresolved identifiers no longer use a string-key global opcode. Resolution
  installs one root `Global` closure descriptor and `ParentGlobal` relays on
  every nested function path; declared Program lexicals and vars instead start
  at source-ordered root `GlobalDeclaration` records. Publication interns each
  exact name
  and root script instantiation binds the root cell in the initiating Context.
  `GetVar` reads initialized cells directly. For an uninitialized cell,
  a descriptor marked lexical raises the named TDZ ReferenceError; an ordinary
  descriptor performs one observable global-object `[[Get]]`. `GetVarUndef`
  suppresses only that ordinary missing-property case. This descriptor-based
  distinction is why a later eval sees a failed Program lexical as not defined
  even though its VarRef metadata still blocks writes, deletion and
  redeclaration.
  Sloppy direct-identifier `delete` uses the corresponding late scope result
  without first performing `GetValue`: argument, local, closure, private
  function-name, implicit `arguments` and lexical paths return `false`, while
  global/unresolved paths perform `HasProperty` followed by `DeleteProperty`
  on the executing bytecode realm's global object (and return `true` when no property is
  present). Parentheses retain the direct Reference, while comma/composed
  values do not. Strict direct IdentifierReferences are rejected as early
  errors at the pinned QuickJS source position.
  A function created by script execution retains the root cells selected at
  that script's instantiation. Ordinary VM fallback property operations and
  Reference/Type errors use the executing bytecode realm; declaration-preflight
  errors are the caller-Context exception described above.
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
  Getter/GetterMagic, and UnaryF64/BinaryF64 adapters share active native
  frame bookkeeping, restore it across return/throw/engine-error paths, and
  keep the mutable object constructor bit independent from cproto and own
  `length`. Native defining-realm edges participate in trial-deletion GC.
- Typed autoinit also covers native methods, constant intrinsic strings and the
  complete global Math object. The current Object/Function/Error function-list
  prefixes and Math method table expose keys and descriptors before allocating
  their values; ownKeys/has-own/delete remain shape-only. Get/gOPD and a
  compatible define materialize once in the stored realm; define first checks
  the lazy flags, so impossible changes to a non-configurable slot are rejected
  without allocation while configurable builtins can be replaced by data or
  accessor descriptors. Initializer failure commits an ordinary `undefined`
  slot while releasing that realm edge.
- `%Object.prototype%` installs the complete pinned table in order:
  `toString`, `toLocaleString`, `valueOf`, `hasOwnProperty`, `isPrototypeOf`,
  `propertyIsEnumerable`, the `__proto__` getter/setter, the four Annex-B
  `__define*__`/`__lookup*__` helpers, then `constructor`. Property-key and
  receiver conversion ordering, inherited accessor lookup, prototype walking,
  primitive short-circuits, exact nullish diagnostics and lazy method metadata
  are differential-tested. Completion-aware
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
  null/undefined tags. `toLocaleString` preserves QuickJS's exact nullish
  property-read diagnostics.
- Sloppy ordinary bytecode functions normalize primitive `this` lazily and
  cache the normalized value in the frame. Number, Boolean, Symbol, BigInt and
  the String exotic substrate therefore allocate at most one genuine wrapper
  per invocation; repeated `this` reads preserve identity, escaped wrappers
  retain the callee realm's matching prototype, and strict functions continue
  to observe the raw primitive. The same cached path is used when a sloppy
  inherited Number/String/Boolean/Symbol/BigInt getter or setter receives a
  primitive receiver. String lookup exposes the implemented 45-key prototype
  surface described above together with user-defined prototype properties;
  the remaining eight standard entries are absent.
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
  inheritance, CR/CRLF and Unicode line/column behavior. A thirteen-input native
  atom-Error differential drives a real strict read-only property assignment
  and locks the narrow-ASCII fast path, byte-57/58 scratch boundary, UTF-16
  surrogate-pair split, `%s` NUL handling, literal suffix and outer 255-byte
  truncation against the pinned oracle. A Function-prototype differential locks
  the implemented own-key prefix, poison-accessor identity and frozen thrower,
  `call` forwarding/throws, lazy define behavior, and `@@hasInstance` ordering,
  descriptors, short circuits and prototype errors.
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
  The compiler-only lexical-scope regression group separately locks emitted
  lexical vardefs, newest-first scope entry, local/captured TDZ, transitive
  closure cells, normal and abrupt `CloseLocal`, fresh block re-entry, mutable
  and const write priority, contextual sloppy `let`, exact early conflicts and
  locations, named initialization, normal `%Function%` bodies, nested script
  locals, shared switch conflicts, classic-for single-entry/cell lifetimes and
  pinned continue behavior, Program-global declaration descriptors, and
  explicit nonclassic-for/destructuring
  boundaries, and strip-debug name removal without losing read-only atoms.
  The companion `oracle_function_body_lexicals` target compares ordinary and
  normal-`Function` body/block/switch values, nested script block/switch locals,
  direct and transitive closure cells, repeated-entry and break/continue scope
  exits, cross-case TDZ/conflicts, TDZ/read-only CLI stacks, and the intentionally
  unsupported destructuring boundary with the pinned release.
  The `oracle_for_lexicals` target separately locks classic-head initialization
  and NoIn parsing, ordinary and normal-`Function` values, script-local and
  cross-eval captures, initializer/body/update cell identity, the pinned
  shared-head-cell continue quirk, labeled jumps through a nested switch,
  conflicts, exact full/StripDebug TDZ and read-only stacks, and explicit
  destructuring boundaries.
  The `oracle_program_lexicals` target locks direct Program values, declaration
  source order, repeated eval persistence, globalThis separation and VarRef
  splitting, preflight atomicity, failed-initializer behavior, exact
  full/StripDebug stacks and parser errors, plus the still-explicit
  destructuring boundary.
  The `oracle_program_vars` target locks duplicate declaration records,
  no-initializer and unreachable-statement instantiation, classic-for shared
  cells, NamedEvaluation, cross-eval persistence and hidden-cell reconnection,
  exact global property attributes, data/accessor/AutoInit/inherited and
  non-extensible paths, mixed-declaration preflight atomicity, full/StripDebug
  stacks, parser conflicts, and explicit destructuring/nonclassic-loop
  boundaries.
  The `oracle_program_functions` target locks direct hoisting, duplicate and
  var/function source ordering, the pinned lexical-first asymmetry, global
  property normalization and rejection, two-pass atomicity, cross-eval cell
  identity, compile/execute realm splitting, and exact full/StripDebug parser
  and runtime stacks.
  The `oracle_function_body_declarations` target locks direct local/argument
  hoisting, duplicate and `var` ordering (including the `arguments` name),
  captured later lexicals and failed initializers, normal `%Function%` bodies,
  exact full/StripDebug stacks and parser errors, explicit unsupported
  declaration boundaries, and the pinned cross-realm regression.
  The `oracle_arguments` target locks 33 pinned QuickJS value observations over
  lazy binding selection, actual argc, mapped/unmapped aliases, duplicate and
  shadowing rules, body hoists, escaped cells, descriptor and integrity
  transitions, cached realm intrinsics, callee poisoning, construction,
  call/apply/bind forwarding, and fast/slow `for-in`. Rust-only tests separately
  pin realm-local iterator/poison identities, heap VarRef edges and fast-state
  transitions. The Annex B block-function probe deliberately retains the
  pinned QuickJS behavior even though one Test262 staging test expects the
  outer implicit object to be overwritten.
  The `oracle_block_functions` target locks block/switch entry visibility,
  separate lexical/Annex closure identity, sloppy duplicate first-versus-last
  behavior, strict and source-ordered conflicts, parameter/`arguments` Annex
  suppression, mutation before the authored declaration, captured-cell loop
  re-entry, failed initializers, Program Annex/global-lexical ordering, normal
  `%Function%` bodies, exact full/StripDebug stacks, realm splitting, and the
  remaining explicit async/generator boundaries.
  The `oracle_annex_b_statements` target separately locks declaration-mask
  propagation, shared `if` scope entry, first-Annex/last-lexical duplicate
  behavior, skipped and repeated control-flow paths, labelled scope identity,
  ProgramBody's no-lexical double global write (including accessor effects),
  parameter suppression, Program ordering and TDZ state, normal `%Function%`
  bodies, compile/execute realm splitting, exact parser diagnostics and
  full/StripDebug runtime stacks, plus the explicit `with` boundary and its
  nested-`if` future behavior.
  The `oracle_try_catch_finally` target locks the implemented synchronous
  exception-region boundary: catch dispatch and scopes, complete
  abrupt-finally control flow, Script completion, the pinned caught-throw cell
  quirk, realm splitting, three debug modes, exact diagnostics, and the still
  explicit catch-destructuring frontier.
  The `oracle_for_of` target locks simple binding/reference heads, the generic
  iterator protocol and accessor order, Unicode String iteration, natural and
  abrupt close behavior, completion precedence, nested labels/switch/finally,
  raw native-next dispatch, realm splitting, exact diagnostics, and all three
  debug modes. Generic Array iteration is now covered by `oracle_array`;
  `for-await-of`, for-of destructuring, and Iterator Helpers remain separate
  milestones.
  The `oracle_for_in` target locks ordinary and representation-sensitive fast
  Array enumeration, per-level snapshots, live presence/prototype changes,
  shadowing, primitive boxing, simple assignment/declaration heads, lexical
  cells, labels/finally cleanup, and exact initializer diagnostics.
- `Runtime` and `Context` are distinct; `qjs -e` and file execution use the
  Rust compiler/VM path and never delegate to an external engine.

## Not implemented yet

The complete pinned Test262 vector is now recorded conservatively. Remaining
parser frontiers with generic syntax diagnostics cannot contribute negative
test passes until they gain typed `Unsupported` provenance or are individually
audited as genuine early errors. Native `$262` host hooks, module
parse/link/evaluate, Promises/jobs and async completion, the ES5.1 suite, and a
separate QuickJS-runner-quirk profile remain future milestones. Unsupported and
host-missing outcomes are failures, not additional feature skips.

The language slice is intentionally narrow. Async/generator declarations,
for-in destructuring, `for-await`, for-of destructuring, other general
assignment targets, module resolution, object method/accessor definitions and
their home-object semantics, non-simple parameter lists, direct/indirect eval
declaration environments, arrow/async/generator functions, `with`, and callable
Proxy classes are not yet implemented. Unsupported declaration contexts are
rejected instead of being
faked as Program functions or ordinary vars. Source `let`/`const` is currently
limited to simple identifier lists in direct Program code, authored
ordinary-function bodies, non-empty nested brace blocks, shared switch scopes,
classic `for (;;)` heads, and synchronous simple-binding `for-in`/`for-of`
heads. These forms also work in scripts, and ordinary bodies including classic
heads are available through the normal `%Function%` constructor.
Single-statement lexical declarations and destructuring loop heads,
destructuring (including catch binding patterns), and class lexical
environments remain later compiler slices. Direct
Program lexicals now use the production global VarRef path with two-phase
instantiation; simple-name Program vars and direct ordinary function
declarations use ordered, kind-specific global declaration records. One
internal resource-failure
hardening gap is tracked here: the Rust path currently allocates the callable
after creating accepted global bindings, whereas QuickJS reserves the
callable object first. Ordinary JavaScript cannot trigger the intervening heap
failure today; matching the allocation order safely requires a provisional
two-phase bytecode-function reservation plus failure-injection coverage, rather
than attempting to roll back migrated VarRefs after the fact.

Derived/class/super construction, dynamic Generator/Async/AsyncGenerator
Function constructors, `AggregateError`, other native builtin constructor
families, and Proxy construct dispatch remain. Typed
target/cproto, data-bearing Error selector, realm, arity padding, production
BoundFunction allocation and frame foundations exist. Generic setter and raw
iterator-next cproto adapters are active; specialized F64 adapters and the
wider builtin table remain.

One host-only Reflect parity edge remains explicit. QuickJS's C API can set a
constructor bit on an otherwise non-callable ordinary object, after which
`Reflect.construct` accepts it as `newTarget`; the Rust embedding helper still
requires a callable payload as well as the bit. Ordinary JavaScript cannot
manufacture that state, so it does not affect the current Test262 or language
surface, but complete embedding-API parity must eventually reproduce it.

Explicit `throw`, nested propagation, VM-generated native errors, eager Error
backtraces, synchronous catch/finally regions, and synchronous iterator cleanup
share the implemented completion path. Async/generator/Promise frame integration,
recoverable OOM and backtrace-allocation fallback, interrupt/termination, and
the remaining abrupt-completion surfaces are still open. The `JS_STRIP_DEBUG` /
`JS_STRIP_SOURCE` debug/source-stripping decision is implemented as a
runtime-wide three-state policy sampled by subsequent compilation: strip-source
retains filename/PC metadata but removes authored source, while strip-debug
removes the represented function source/location payload. The `qjs`
`--strip-source` and `-s` options select the same states in upstream order,
including combined short options and their effect on `toString`, function debug
accessors and Error backtraces. Strip-debug compilation also removes ordinary
lexical vardef and captured-relay names while retaining atoms needed by
read-only execution; bytecode debug serialization remains pending. The normal
`%Function%` graph is present, but dynamic formal parameters remain limited to
simple identifiers and bodies to the current statement, expression, and simple
body/block/switch/classic-for and for-in/of-head lexical-declaration grammar;
default/rest/destructuring parameters and their non-simple Arguments semantics,
generator/async kinds, and Proxy new-target realms remain pending.
Compiler input is still UTF-8,
so dynamic source containing an unpaired UTF-16 surrogate throws an explicit
implementation-gap `InternalError` instead of being silently rewritten. The
parser now requests tokens through fallible advances, and directive probes
seek back before strict-context rescanning, so current-token grammar errors no
longer lose to untouched later lexical failures. Contextual word reparsing for
module/generator/async grammar remains with those unimplemented surfaces. The
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
separate String exotic, UTF-16-prefix and conversion cores cover branded
empty-prototype and sloppy-this wrappers, UTF-16 virtual own properties, the
first twelve generic code-unit/search methods, the three generic subrange
methods, `repeat`, `padEnd`/`padStart`, the five-property trim group,
`toString`/`valueOf`, the four Unicode case-conversion methods,
`Symbol.iterator`, the thirteen-property Annex-B CreateHTML family, non-index
prototype lookup and the implemented
Object-prototype routes. The global `%String%` constructor, its three statics
and the prototype relationship complete that constructor's own table. Their
shared value kernel does publish the pinned flat/rope concat thresholds,
bounded Fibonacci
rebalance, cross-leaf code-unit semantics, content identity, atom
linearization, checked VM/native concat errors, valid-UTF-8/exact-UTF-16
dynamic constructors, checked lexer/URI/Function-source builders, their
distinct overflow ordering, arbitrary-byte `JS_NewStringLen` decoding, and
owned WTF-8/CESU-8 payload export. Repeat adds its pinned flat,
width-preserving, exact-reservation kernel and catchable result-buffer OOM.
The pad pair adds QuickJS's narrow-first buffer, content-driven widening,
UTF-16 filler truncation and catchable result-buffer reservations.
The trim group adds the exact 25-code-unit whitespace set, raw UTF-16
one-sided scans, canonical alias identity with independent properties, and a
catchable partial-result reservation.
The case-conversion group adds the pinned Unicode 17 compressed mapping,
extension, `Cased` and `Case_Ignorable` tables; astral and multi-code-point
mappings; context-sensitive Greek final sigma; raw surrogate preservation; and
a narrow-first fallible result buffer. The locale-named pair deliberately
ignores every argument and remains distinct in identity from the ordinary pair.
The CreateHTML family adds the pinned selector/tag table, receiver-before-
attribute conversion, quote-only attribute escaping, raw UTF-16 output and a
narrow-first latched-error builder with catchable length and reservation
failures.
Native Errors additionally share the
255-byte visible payload of QuickJS's fixed formatter; sidecar-bearing messages
retain exact raw bytes across compiler/VM Error transport. They also implement
the not-constructor dynamic name plus the current `JS_AtomGetStr`-backed
read-only/nullish/binding/TDZ/reserved-identifier diagnostics. It does not
publish the remaining eight prototype own keys, Context/C pointer embedding
semantics, atom diagnostics belonging
to unimplemented language/builtin surfaces, exact byte-sidecar construction
for every parser/lexer diagnostic, or general recoverable allocator failures
outside the repeat/pad/trim/case/CreateHTML result-buffer reservations. Rope
linearization and final `Box`/`Rc` allocation, including those surrounding the
checked trim, case and CreateHTML buffers, remain part of that general
allocator gap. Pad, case and CreateHTML widening use a second fallible exact
UTF-16 buffer and then release the narrow buffer, rather than preserving
QuickJS allocator/
realloc identity and peak-memory behavior.
Prefix/postfix update expressions
(including QuickJS's valid `++x ** 2` form) are implemented for the current
identifier and ordinary fixed/computed member References. Sloppy
direct-identifier delete is implemented
for the current static scope tree and defining-realm global object. Dynamic
object-environment lookup/deletion introduced by `with` or direct `eval`, the
remaining eight entries of String's 53-key prototype surface, the RegExp object
class needed by `IsRegExp`'s internal-brand fallback, Proxy/exotic internal
methods, and the full
`function_accessors.js` fixture are still pending. AggregateError and
uncatchable termination state are also pending. Array
destructuring consumers, `with` object-environment
semantics, other
iterator classes and helpers,
RegExp, the remaining RegExp-/Unicode-backed String methods, object-literal
methods/accessors and exotic-source spread, and the rest of the builtin table
build on those layers.

The remaining parity surface also includes the full grammar/opcode set, the
Unicode 17 normalization/script/property tables beyond the implemented
identifier, case-conversion, `Cased` and `Case_Ignorable` data, RegExp bytecode
engine, modules, jobs/Promises/async,
generators, TypedArrays/Atomics, WeakRef/finalization, bytecode version 5 and
BJSON interoperability, `std`/`os`, workers, REPL/qjsc, and the complete Rust
and C embedding APIs.

Code organization is also not final. Runtime white-box tests live in
`runtime/tests.rs`, while the Array constructor, prototype, iterator, species,
and sorting implementation now lives in `runtime/intrinsics/array.rs`.
The Object constructor, implemented statics and implemented prototype handler
surface now live with `groupBy` in `runtime/intrinsics/object.rs`; the String
constructor/static table, implemented prototype-table initialization,
index-search pair, regexp-aware includes family, subrange trio, `repeat`, the
pad pair, trim group, Unicode case-conversion group and Annex-B CreateHTML
family live in `runtime/intrinsics/string.rs`, while the remaining String
initialization and handlers still await migration there. The complete Math
object table, selectors, numerical kernels, random and precise-sum handlers live
in `runtime/intrinsics/math.rs`; the complete Reflect table and handlers live in
`runtime/intrinsics/reflect.rs`. The unpublished Date foundation is isolated in
`runtime/intrinsics/date/`: `calendar.rs`, `parse.rs`, `format.rs`, and
`host.rs` own the pure calendar, parser, formatter, and injectable host seams,
while the branded payload and realm-root edge remain in the heap. The complete
VM-to-runtime trait adapter,
per-frame argument/local/capture storage, iterator protocol bridge and
bytecode-host error conversion now live in `runtime/vm_host.rs`; host layout is
private to that module, including bytecode frame initialization. The hidden
for-in enumeration algorithm and prototype-level snapshots live in
`runtime/for_in.rs`. Arguments construction, cached realm intrinsic roots,
mapped VarRef transitions and representation state live in the 621-line
`runtime/arguments.rs`, so this feature adds only module wiring and exhaustive
class matches to the parent. Ordinary,
String-exotic and Array property lookup/definition, AutoInit materialization,
deletion, own-key, prototype and extensibility operations now live in
`runtime/properties.rs`; their action records remain the parent module's
internal ABI for VM, Context and intrinsic consumers. Native cproto adaptation,
raw iterator-next selection and the exhaustive `NativeFunctionId` match now
live in `runtime/native_dispatch.rs`; builtin additions no longer extend the
main runtime file merely to wire a selector. Bytecode draft validation and
iterative flattening now live in `runtime/bytecode_publish.rs`. The test, Array,
Object, VM-host, property, native-dispatch and bytecode-publication
no-semantic-change splits reduced `runtime.rs` from roughly thirty-two thousand
lines to 9,937 lines. Realm-aware property completion wrappers and storage
helpers, bytecode publication linking and call dispatch, runtime/root lifecycle,
and the remaining intrinsic families still share the file; `compiler.rs`
similarly combines several compiler phases.
Dedicated structural milestones must keep splitting those seams under the same
differential and Rust-only gates, and future feature work must not resume
extending either monolith indefinitely.

## Reproduce current evidence

```sh
cargo test --locked --workspace --all-targets

QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_boolean_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_symbol_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_exotic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_conversion_core -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_utf16_prefix -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_index_search -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_includes -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_subrange -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_repeat -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_pad -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_trim -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_create_html -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_case -- --nocapture
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
  cargo test --test oracle_math_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_reflect_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_function_body_lexicals -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_function_body_declarations -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_arguments -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_block_functions -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_annex_b_statements -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_try_catch_finally -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_for_of -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_for_in -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_for_lexicals -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_program_lexicals -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_program_vars -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_program_functions -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_group_by -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_enumeration -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_extensibility -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_descriptors -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_is -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_assign -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_integrity -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_from_entries -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_has_own -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_search -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_with -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_concat -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_stringification -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_mutators -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_reverse -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_sort -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_slice_splice -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_fill -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_copy_within -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_find -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_iteration -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_map_filter -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_reduce -- --nocapture

./scripts/test-parity-slice.sh
./scripts/test-test262-smoke.sh
./scripts/test-test262-provenance.sh
./scripts/test-test262-reflect.sh
./scripts/test-test262-full.sh
```

The direct commands above run the dedicated Boolean, Symbol,
String constructor/static table, String-exotic substrate, String UTF-16 prefix,
String index-search, regexp-aware includes and String subranges, String-conversion core,
Unicode String case conversion, String-rope/byte/native-Error kernels, Unicode
identifier core, global
BaseObjects, complete Number-, BigInt-, Math- and Reflect-intrinsic
differentials, and the
Program-var/function, Program/body/block/switch/classic-for lexical-scope,
ordinary mapped/unmapped Arguments object,
single/labelled Annex B, synchronous try/catch/finally, synchronous
for-in/for-of, Array core/literal/iterator/search/callback/mutation/change-by-copy,
Object
literal, and Object constructor/static-prefix/prototype slices. The atom-Error
target contains thirteen
pinned-oracle inputs in addition to its Rust-side expectation test. The Unicode
identifier target checks every scalar, real compiler/runtime cases, and the
parser-driven identifier diagnostic matrix; the Unicode case target checks the
full conversion/property fingerprint plus final-sigma, raw UTF-16, locale and
runtime graph behavior. The gate also verifies the complete pinned Test262
metadata fingerprint, the fixed 193-variant smoke vectors, the negative-test
provenance canaries, and the hashed 102,037-variant classified vector. A
separate statement-control-flow target locks block/`if`/loop completion,
nearest-loop jumps, per-function isolation, ASI/directive boundaries and exact
diagnostics; the switch target locks case/default search, fallthrough,
completion and cross-control cleanup; the template target locks raw/cooked
UTF-16, continuation goals, concat lowering/order, diagnostics,
tagged-template boundaries, and folded control-flow reachability at the
65,534-slot stack limit. The full gate discovers every `tests/oracle_*.rs`
integration target, reuses an executable `QJS_ORACLE` or checksum-verifies and
builds the pinned test-only oracle, obtains and checksum-verifies the matching
Unicode table source, then runs both generated-table drift checks, formatting,
unit/integration/oracle tests, Clippy, and the Rust-only product gate. The oracle
is never part of the product dependency graph or runtime.
