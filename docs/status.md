# Implementation status

Last audited: 2026-07-20. The completion definition remains
[`parity.md`](parity.md); this file records progress and must not be used to
claim full parity.

## Implemented on the final architecture path

- QuickJS 2026-06-04 release metadata, archive checksum, bytecode version,
  Unicode version, and Test262 commit are pinned in `compat/upstream.toml`.
- The process-isolated Rust Test262 runner now saves a complete conservative
  outcome vector for all 102,037 sloppy/strict variants. A checksum-pinned
  capability profile now admits 69 reviewed feature tags and 423 exact audited
  negative-test paths. Those fail-closed canaries and the source/metadata host
  requirements keep unsupported grammar,
  features, modes, and `$262` hooks from becoming false passes. Bounded workers
  preserve canonical byte-for-byte TSV and JSONL ordering. The current vector
  has 34,878 passes and 38,421 runnable variants: 34.18% raw, a 41.74% lower
  bound after the 18,475 pinned QuickJS target exclusions, or 94.10% among the
  37,066 variants with a non-unsupported observed outcome. The fixed smoke now
  has 191 passes and two explicit parser-frontier results. See
  `docs/test262.md` for the denominators and why none of these figures is a
  parity claim. The first
  observable RegExp intrinsic slice added 669 full-vector passes and moved ten
  advanced-pattern variants from generic runtime failure to typed unsupported
  results. The subsequent R1b literal slice adds another 840 passes. Its exact
  full-vector join has 1,193 transitions and no previous-pass regression; the
  independent 96-variant focused vector remains the faster literal gate. The
  R1c search protocol adds 118 passes and admits 64 more jobs. Its exact
  102,037-key join records 66 `fail-runtime -> pass`, 52
  `unsupported-feature -> pass` and 12 `unsupported-feature ->
  unsupported-parser` transitions with zero previous-pass regression; the
  independent 132-variant search vector preserves the object-literal parser and
  adjacent-feature frontiers rather than widening this milestone.
  R1d adds the generic String/RegExp match protocol pair and 212 full-vector
  passes while admitting 144 more jobs. Its exact join records 86
  `fail-runtime -> pass`, 126 `unsupported-feature -> pass`, 16
  `unsupported-feature -> unsupported-parser`, and two
  `unsupported-feature -> fail-runtime` transitions, again with zero
  previous-pass regression. R1e publishes the RegExp split protocol without
  widening the capability profile or admitted-job count. Its exact join records
  only 90 `fail-runtime -> pass` transitions across all 102,037 keys, with zero
  previous-pass regression, missing, extra, or duplicate rows. R1f adds the
  pinned legacy RegExp `compile` mutation and one Unicode decimal-escape syntax
  refinement. Its exact join records 44 `fail-runtime -> pass` and two
  `unsupported-runtime -> pass` transitions, again with zero previous-pass
  regression or key drift. R1g ports scoped `(?ims-ims:...)` RegExp modifiers
  from the pinned compiler. Its complete 460-variant feature join records 448
  `unsupported-feature -> pass` and 12 `unsupported-feature ->
  unsupported-parser` transitions, with no other outcome movement. R1h ports
  String `replace`/`replaceAll` and the generic RegExp `@@replace` path,
  recording 110 `fail-runtime -> pass`, 170 `unsupported-feature -> pass`,
  four newly exposed parser failures, and 38 newly exposed typed parser
  frontiers. The exact 102,037-key join has zero previous-pass regressions.
  R1i adds QuickJS's raw standard-RegExp predicate and direct `@@replace`
  matcher. It is a semantic-path milestone rather than a coverage expansion:
  both the focused 376-variant replacement report and the complete
  102,037-variant report remain byte-identical to R1h.
  R1j ports `String.prototype.matchAll`,
  `RegExp.prototype[Symbol.matchAll]`, and the branded RegExp String Iterator.
  Its complete join adds 66 passes and admits 114 more jobs without regressing
  any previous pass. R1k adds numeric backreferences and the inseparable
  non-Unicode Annex B decimal/octal fallback. Its complete join adds 68 passes
  and admits four audited parse-negative variants, again with no previous-pass
  regression. R1l adds forward positive/negative lookahead and its Annex B
  quantifiable form. Its complete join converts 52 already-admitted variants
  to pass without moving any other category or regressing a previous pass.
  R1m adds Unicode property escapes and admits their fail-closed Test262
  surface. Its complete join adds 298 passes and 1,170 runnable variants:
  288 move from `unsupported-feature` to pass, ten from
  `unsupported-parser` to pass, and 882 generated property-table variants move
  from `unsupported-feature` to the existing harness-parser frontier. No
  previous pass regresses.
  R1n removes that generated-data frontier with the pinned QuickJS
  `codePointRange` host helper, identifier-only array BindingPatterns in
  synchronous for-in/of declarations, and binary RegExp range lookup. The
  complete join adds 916 passes without changing the 34,457 admitted jobs or
  regressing a previous pass.
  R1o ports positive and negative variable-length lookbehind through the same
  QuickJS-shaped assertion stack. It adds 50 passes and admitted jobs with no
  previous-pass regression or outcome drift outside the frozen 27-path set.
  R1p ports ordinary named captures, named forward/backward references,
  null-prototype `groups`/`indices.groups`, and `$<name>` replacement. It adds
  162 full-vector passes and 184 admitted jobs with no previous-pass
  regression; four linked `\k` canaries outside the 101-path manifest also
  resolve as expected.
  R1q audits and declares duplicate named captures without changing the
  already-compatible engine. It adds 26 passes and 32 admitted jobs; all 38
  complete-row changes stay inside the frozen 19-path set. At that landing,
  six arrow variants reached the existing parser frontier and six
  match-indices variants remained independently gated. R1r likewise needs no
  production engine change: a pinned QuickJS source-and-probe audit confirms
  that the existing `d` flag, `hasIndices`, UTF-16 range, unmatched capture,
  named `indices.groups`, construction, and descriptor behavior already have
  target parity. Declaring `regexp-match-indices` adds 38 passes and 50
  admitted jobs. All 50 outcome changes and ten detail-only changes stay
  inside its frozen 31-path set, for 60 complete-row changes and no
  previous-pass regression. R1s audits and declares `regexp-dotall`, again
  without a production engine change. It adds 18 passes and 26 admitted jobs;
  all 26 outcome changes and six detail-only changes stay inside the frozen
  17-path set, for 32 complete-row changes and no previous-pass regression.
  R1t audits and declares `u180e`, again without changing production code.
  It adds 40 passes and 50 admitted jobs; ten newly admitted variants expose
  the existing global-`eval` frontier. All 50 row and outcome changes stay
  inside the frozen 25-path set, with no previous-pass regression.
  R1u installs the realm-local `%eval%` intrinsic shell at the same
  `js_global_funcs` position and with the same cached-original identity model
  as pinned QuickJS. Metadata, descriptors, non-constructability, global
  mutation, cross-realm calling, and every non-String argument now have target
  behavior; primitive String source execution remains a typed, uncatchable
  `Unsupported` frontier until the compiler and VM have direct/indirect eval
  environments. The complete positive slice adds 55 passes across 31 paths.
  The full join also moves 1,448 missing-eval runtime failures to the typed
  frontier and corrects 41 old false passes whose assertions had accidentally
  accepted or swallowed the missing-global `ReferenceError`. Net pass growth
  is therefore 14, not 55; this is an explicit false-positive correction, not
  a regression in previously implemented JavaScript semantics.
  R1v adds QuickJS-shaped syntactic direct-eval lowering without opening
  String source execution. The parser retains the call-site `ScopeId` in
  `IrOp::EvalCall`, then publishes the current shell as `Instruction::Eval`;
  this avoids putting an uninterpretable parser scope number in public
  bytecode while preserving the information needed for the later linked eval
  environment table. The VM performs QuickJS's current-realm original-object
  identity gate: a match consumes only the first already-evaluated argument
  and bypasses a native `%eval%` frame, while a mismatch calls the replacement
  with `this = undefined` and the complete argument list. Parenthesized and
  locally bound identifier references are candidates; comma, alias, property,
  `.call`/`.apply`, and conditional/assignment results remain ordinary calls;
  construction remains on the non-eval `Construct` path. Pinned QuickJS probes
  freeze that call-form matrix, including the still-deferred spread and
  optional-call boundaries. Both the focused
  55-variant report and the complete 102,037-variant Test262 reports are
  byte-identical to R1u, as required for this semantic-path milestone.
  R1w links each direct-eval instruction to an immutable caller-environment
  descriptor modeled on QuickJS's live scope chain. The compiler walks from
  the call scope through every lexical parent and function-definition scope,
  records current-frame Local/Argument sources and named ancestor Closure
  relays, forces `arguments` and private function-name bindings, deduplicates
  equal call-site descriptors, and marks eval-visible locals captured so their
  existing `CloseLocal` lifecycle is used. Publication owns every retained
  name atom and rejects unreferenced tables, malformed function segments,
  source-kind crossings, global relay disguises, and name/flag/source
  mismatches against the parent function tree. The VM validates the complete
  descriptor before turning String-call sources into live VarRef roots;
  non-String eval remains identity-returning and does not inspect scopes or
  normalize `this`. Primitive String execution deliberately remains the same
  typed `Unsupported` frontier, so both Test262 vectors are byte-identical to
  R1v.
  R1x opens the first primitive-String execution slice on a dedicated synthetic
  Eval root rather than reusing the Script root. Direct roots import the exact
  ordered caller descriptor as authenticated `EvalEnvironment` closure slots;
  indirect roots have no caller slots and use the original `%eval%` callable's
  defining realm and global `this`. Eval-local `let`/`const`, expression and
  statement completion, caller-cell reads/writes, returned closures, strict
  inheritance, catchable parse errors, and nested indirect eval now execute.
  The compiler, heap and publication boundary independently enforce root kind,
  strictness, binding count/order/names/flags, root-only external slots and
  child relay topology. Compilation and publication happen before caller
  Local/Argument cells become VarRefs, matching QuickJS's error ordering; the
  caller bytecode and materialized roots remain owned through instantiation and
  execution. Full, StripSource and StripDebug modes retain the semantic names
  needed by returned external closures.

  The eval gate expands from 31 paths / 55 variants to 74 paths / 138 variants,
  all passing. Its manifest, TSV and JSONL SHA-256 values are
  `99aa8af497946369babf6f639f5ccfb4c8da5bffb7587f75825ead076556c314`,
  `2b3f87db4ae4333cee6ff896c3d0ead2e061fd98000b0673a6fa32ff4acd7ad4`
  and
  `29e965a24abdd74d70ea0970a8c2afd6ce20f5b52153239f1b15bb7ec651b34e`.
  Existing frozen manifests move with the same implementation: RegExp core
  rises from 438 to 448 passes, RegExp match from 184 to 186, generic String
  split from 236 to 240, and U+180E from 40 to 50.
  The exact full-vector join keeps all 102,037 keys, adds 575 passes, and has
  zero previous-pass regressions. Thirteen formerly typed frontiers become
  visible runtime failures: ten stop at existing arrow/async/generator or
  non-simple-parameter grammar boundaries, while three are pinned QuickJS's
  already-recorded SpiderMonkey staging differences. The complete vector is
  at the R1x landing 28,216 passes with 34,849 runnable jobs; TSV/JSONL
  SHA-256 values are
  `c62f104a2a3801c9b3eca38362fa5075f1fc21564395c58f45dfb23153ef1530`
  and
  `526c00942821ff5f153e08d3056627bbe35e7e12e4cde3702a55c220351bbd09`.

  R1y ports QuickJS-shaped eval declaration environments. Every sloppy
  direct-eval-capable activation owns a hidden null-prototype `<var>` object;
  eval bytecode reaches it only through authenticated Local/Closure metadata
  and typed has/get/put/delete/define operations. Source-ordered `var` and
  ordinary FunctionDeclaration records preserve repeated-eval overwrite,
  function/var order, deletion fallback, catch-parameter reuse, caller lexical
  conflicts, and implicit `arguments` precedence.
  Strict eval keeps declarations local; indirect and global direct eval use
  configurable global declarations; sloppy function eval resolves the nearest
  current or ancestor variable object. The same path covers Annex B block,
  single-statement and labelled declarations, including QuickJS's distinct
  lexical and outer-write closures.

  The independent declaration gate freezes 497 paths / 519 variants and all
  pass. Its manifest, TSV and JSONL SHA-256 values are
  `ecc3cb3b50f8b59cae548fa9c1017dfd1d71878644bf204146d4002015c2bd70`,
  `1b9cfacfe80671d5e2579865b7efb1478b5d7c1da70b240b71a1cccc3cf1c80a`
  and
  `0a0e7db1f1c80431302b14b66148f34efa998f38811e965f126c2d548ab6dd6d`.
  The exact R1x/R1y full join retains all 102,037 unique keys and every prior
  pass. It records 752 `unsupported-runtime -> pass`, 16
  `fail-runtime -> pass`, and 16 `unsupported-runtime -> fail-runtime`
  transitions: 15 are checksum-pinned Test262 failures also observed in the
  target QuickJS release, and one reaches the existing generator/async grammar
  frontier. Net growth is 768 passes. The final vector has 28,984 passes and
  34,849 runnable jobs, no engine/runner fault, and TSV/JSONL SHA-256 values
  `cca9eadc35c3c5f9acdf24b00cb9d65b0a2ca20a65860e137185f4f7fa48c4e4`
  and
  `348e25af619fcf81ef534b82f57571889c1d2ab7f06cad3d5233e7d49fae240f`.

  R1z recursively relays QuickJS-shaped direct-eval caller environments. A
  synthetic eval root now retains the exact imported scope-kind sequence,
  including empty catch/block/function scopes, and an authenticated global,
  strict-local, or external `<var>` declaration target. Nested eval bytecode
  relays every imported descriptor through intervening closures; publication
  traces each closure slot back to its exact caller-binding ordinal and rejects
  wrong-scope, wrong-cell, or wrong-variable-target drafts. Pinned QuickJS
  probes cover three nested levels, catch reuse across direct versus ordinary
  function boundaries, strict non-leakage, lexical conflicts, and escaped
  closures after the caller frame detaches.

  The independent R1z gate freezes the complete former frontier: 25 paths / 30
  variants. Twenty-nine pass; the remaining SpiderMonkey staging variant now
  stops at the independent `with` statement parser frontier. The complete
  102,037-key join records 29 `unsupported-runtime -> pass` transitions and one
  detail-only refinement, all inside that manifest, with no missing, extra,
  duplicate, or previous-pass row. At the R1z landing, the full vector had
  29,013 passes and 34,849 runnable jobs. R1z-era focused TSV/JSONL SHA-256
  values are
  `3a6dd32c7f3d0154b36946c6894f9cdba79a12d7086bf5602a210360b90f5248`
  and
  `23f4e2115b5a1ed322eac39faa51517912825562e71965a73261b3f4ad86a1fb`;
  full-vector values are
  `2ba53703827155be4ce36f11a52b48c3ac1bb4efc8f61da9cc31b6b1ca8e125a`
  and
  `c9369e14acb1469b20aea4caab2c0a880cb7f040a72718d629f38e1301582650`.

  R2a closes the named-function-expression/eval declaration precedence gap by
  preserving QuickJS's two distinct lookup orders. Already-authored ordinary
  code resolves its private FunctionName before its own hidden `<var>` object;
  a synthetic eval root keeps the ordered external chain with `<var>` before
  that private name, so same-named `var`/FunctionDeclaration values remain
  visible inside eval and to later eval calls without replacing the caller's
  recursive self binding. The same compiler path reproduces QuickJS's pinned
  `add_eval_variables` quirk and creation order: eval closure tables are seeded
  when the compiler enters each function, before source-ordered children and
  the parent's own name resolution. Physical parent sources are source-keyed;
  the first request fixes flags/kind while a missing semantic name may still be
  retained later. An ordinary eval descendant can therefore establish a
  mutable Normal view in a plain parent, while a later plain leaf restores
  FunctionName metadata on its own descriptor; Eval-root-origin bindings keep
  their imported flags. Publication propagates the underlying ordinary
  FunctionName provenance and accepts an erased Normal view only when it is
  consumed by an authenticated, referenced eval environment or is an erased
  ParentClosure ancestor of such a slot. Shared VarRefs then admit only the
  corresponding FunctionName-cell/Normal-view pair. A 25-row
  Rust/pinned-QuickJS differential freezes direct and recursive declarations,
  caller/source strictness, delete/write fallback, both source-order outcomes,
  deep ordinary/eval relays, and FunctionName/erased-Normal Eval-root controls.

  The pinned Test262 snapshot has no exact test for a named function
  expression whose direct or nested eval declares the private self name: that
  declaration-shape cohort is 0 paths / 0 variants, so R2a deliberately adds no
  empty manifest and claims no coverage increase. The complete 102,037-variant gate
  remains byte-identical at 29,013 passes and 34,849 runnable jobs, including
  the same TSV/JSONL hashes above. `runtime.rs` remains 9,730 lines; the
  descriptor compatibility and publication checks stay in the existing
  split runtime modules.

  R2b ports sloppy `with` through QuickJS-shaped scope and Reference
  machinery. Each authored statement owns an authenticated hidden `<with>`
  Object binding; resolver order interleaves lexical scopes, with objects and
  eval variable objects, while `Symbol.unscopables`, repeated `HasProperty`,
  delete, call receivers, for-in/of writes and captured lifetimes retain their
  distinct paths. Typed VM operations keep environment sources out of ordinary
  JavaScript values. `GlobalReference` mirrors `OP_make_var_ref`: it snapshots
  a global property or unresolved sentinel before the RHS and consults the
  current realm's live lexical VarRef object for TDZ/readonly checks, including
  lexicals declared after a function's bytecode was published. Direct eval
  imports dynamic lookup but deliberately retains QuickJS's later assignment
  resolution and undefined call receiver.

  A 26-case single-script differential plus two cross-script sequences match
  QuickJS 2026-06-04. The frozen 203-path / 205-variant `with` cohort moves
  from zero to 198 passes with no remaining `with` parser/runtime frontier.
  Five rows expose the existing arrow parser gap, one direct-eval row exposes
  the same gap at runtime, and one mixed staging row first reaches generator
  syntax. The exact full join changes only those 205 rows, has no previous-pass
  regression, and raises the complete vector to 29,211 passes; full TSV/JSONL
  hashes are
  `8eba52564839d3a11a92ac28c883494cfc51d1f49785b07e7d3ac62ec867965c`
  and
  `54122f8b86f8cdbea6f3de6aa9532f770b72df1f6bf28bdc7cd62ec665b32ca1`.
  `runtime.rs` is 9,732 lines; the new dynamic-environment implementation is
  in `runtime/vm_host/dynamic_environment.rs`.

  R2c ports synchronous ArrowFunction parsing and lexical-environment behavior
  from QuickJS. Simple identifier parameter lists, expression/block bodies,
  strictness, source/name/length metadata and non-constructability now share
  the ordinary bytecode-function path without publishing a prototype.
  Hidden `this` and `new.target` pseudo bindings relay through Arrow and direct
  eval frames to their nearest owning frame; `arguments` remains ordinary name
  resolution, including `with` and eval variable environments. Thirty-four
  pinned QuickJS cases freeze lookahead, reserved-word diagnostics, metadata,
  nested closures, `with`, direct eval, `typeof this`, and construction.

  The 40-path focused gate expands to 66 variants and passes 66/66. Declaring
  `arrow-function` admits 575 more full-suite jobs, while Arrow syntax also
  unblocks untagged consumers. The exact 102,037-key join adds 1,043 passes:
  474 `fail-parse -> pass`, five `fail-runtime -> pass`, 30 `harness-error ->
  pass`, and 534 `unsupported-feature -> pass`. Every one of the previous
  29,211 passes remains a pass. The complete vector reaches 30,254 passes and
  35,424 runnable jobs; full TSV/JSONL SHA-256 values are
  `c28acb10ae63e46e8aad1372f679c3be3b283322c2f690e0296bf0a77e243345`
  and
  `e82fbff1bdd49b300ea561d7ad21b9c3d62ed4d640f7080c3375bc9044bf32f9`.

  R2e begins with a capability-profile truth-up rather than an engine semantic
  change. A path-by-path Rust and pinned-QuickJS audit found 22 already
  implemented Test262 feature tags that the fail-closed profile still hid, and
  95 already-correct negative tests whose exact phase/type provenance had not
  yet been admitted. The profile now contains 53 feature tags and 403 exact
  negative paths with SHA-256
  `e2043efeaa2d8b4420d0c82550f7ba42d53588897ec14ac87f6f03c4358a8218`.
  The runner contract independently fixes those sets in Rust and validates
  every negative path against the pinned suite metadata. All 28 non-full
  Test262 gates retain their prior keys, runnable counts, pass counts and
  outcome summaries; their 30 checked-in report artifacts change only for the
  R2e profile metadata and the resulting report hashes. This inventory
  milestone changes no lexer, compiler, VM or intrinsic implementation. The
  complete 102,037-key join admits 1,342 more jobs and reaches 31,459 passes:
  1,205 rows move from `unsupported-feature` to pass and 137 move to an
  existing typed parser frontier. Another 507 rows change only their remaining
  unsupported-feature detail. All 1,849 changed rows carry one of the 22 newly
  reviewed tags, and there are zero previous-pass regressions, missing, extra,
  or duplicate keys. The 36,766 runnable jobs have TSV/JSONL SHA-256
  `7e05dd58a0387d8639d09b3896917ad38fd8fd8fdecef85a3f0bcd26f730a22a`
  and
  `c9faabfd53bd125b3f7e4f3f6cbce884e0ce3172de320a1056398de60aa73ab6`.

  R2f ports synchronous, simple-parameter ObjectLiteral concise methods through
  a QuickJS-shaped define-method path. Fixed identifier/keyword/String/numeric
  keys and computed String/numeric/Symbol keys, contextual `get`/`set`/`async`
  identifiers before `(`, inferred names, source/name/length metadata, C/W/E
  property descriptors,
  dynamic `this`, owned `arguments`/`new.target`/direct-eval environments,
  strictness inheritance, trailing commas, duplicate-parameter early errors,
  non-constructability, missing `prototype`, and ordinary `__proto__()` data
  properties are pinned against QuickJS 2026-06-04. Accessors, async/generator
  methods, non-simple parameters, and home-object/`super` semantics remain typed
  frontiers.

  The frozen ObjectLiteral-method gate contains 74 paths and 144 variants; all
  144 are admitted and pass. Its manifest/key-set SHA-256 values are
  `e9f877f938d52a5f5ccbe13af35822b0cb94a9486bb0857156f254a4b532ae75`
  and
  `ebba13cb8173521639bc12b78f2d5acb498893984f8e42e744a57f6c82f08b9a`;
  focused TSV/JSONL SHA-256 values are
  `41a1812b56f74b21967c155f33f93261c767aed6338562535faaded4227e7c4c`
  and
  `5dbf57993c5c4c1dd47f31769e20bbde16c31bc41d486edd8f1999c19d91e16b`.
  Ten independently audited parse-negative paths move the capability profile to
  53 feature tags and 413 exact negative paths, with SHA-256
  `1a5258a57285ff43149d8377692b5f1a3939ed19c790cbee81abab6912d21e51`.
  Existing frozen focused gates also expose the shared grammar improvement:
  Date reaches 1,478 passes (+62), String split 248 (+6), RegExp match 192
  (+2), compile 58 (+2), replace 326 (+18), matchAll 108 (+26), named groups
  172 (+4), and match indices 48 (+4). Reflect keeps 365 passes while four
  parser frontiers advance to runtime assertions; dotAll keeps 26 passes. These
  manifests overlap and are not a full-suite pass delta.

  The exact R2e/R2f full-vector join retains all 102,037 unique keys with no
  missing, extra, or duplicate keys and no previous-pass regression. It adds
  492 passes: 472
  previously typed `unsupported-parser` variants now pass, and the 20 variants
  from the ten newly audited parse-negative paths move from
  `unsupported-negative-provenance` to pass. Of the other exposed parser
  consumers, 38 now report an ordinary parse failure, 89 reach runtime
  assertions, and six reach a narrower typed runtime frontier. No other
  outcome moves. The join has 625 outcome changes and 631 detail-only changes.
  Runnable jobs rise by 20 to 36,786 and the complete vector reaches 31,951
  passes. Full TSV/JSONL SHA-256 values are
  `b63cd00601ea67854cd837a023d1ee14d0b7bdcd02b5e337c0f3eb14f4aa9a67`
  and
  `4196b714970aae9710d76d07e169c1f96ce80afe65cf37d4677ec2da20e3fe2d`.

  R2g ports synchronous ObjectLiteral getters and simple-parameter setters
  through the same QuickJS-shaped define-method path. Fixed and computed
  String, numeric, keyword, and Symbol keys; one-time `ToPropertyKey`;
  getter/setter half merging and replacement; data/accessor conversion;
  inferred names and descriptors; dynamic `this`, `arguments`, `new.target`,
  and direct eval; inherited and body strictness; non-constructability; source
  spans; and ordinary accessor-named `__proto__` properties are pinned against
  QuickJS 2026-06-04. Accessor arity and strict reserved-word diagnostics keep
  the oracle's error priority. Non-simple setter parameters, HomeObject/`super`,
  and async/generator methods remain typed independent frontiers.

  The frozen ObjectLiteral-accessor gate contains 70 paths and 128 variants;
  all 128 are admitted and pass. Its manifest/key-set SHA-256 values are
  `02e2810fd012d7f2191cfd2a14d0ae54425c82717c9b8aacd5460e65f9d72175`
  and
  `2b70d0e1d0054705fe4da193374a67ad664c5f5027d17fb21e1873bb3f8fc1e3`;
  focused TSV/JSONL SHA-256 values are
  `fec46a88e750f33f59085a09386a0f05bd563a5c11ed1310bbd19f8de18cb70a`
  and
  `51f232d679e7045da9634cc0d417cf74815d0f9a1af6064eb1385e6aafa260bd`.
  Nine independently audited parse-negative paths move the capability profile
  to 53 feature tags and 422 exact negative paths, with SHA-256
  `73da0ef92820d81935e2f784a2f0e9ce565ccd10c302d8905c4bd4353c3a81ef`.

  All 23 existing script-focused gates remain green. Nine gain 76 overlapping
  passes, while the separately frozen Reflect and Date vectors add four and
  eight; Date also exposes two existing missing-JSON runtime failures. The
  exact R2f/R2g full-vector join retains all 102,037 unique keys with no
  missing, extra, or duplicate keys and no previous-pass regressions. It adds
  447 passes across
  267 paths: 436 accessor consumers, two strict reserved-word consumers, and
  nine newly audited negative variants. Ten former parser frontiers now report
  ordinary parse failures and 42 reach downstream runtime failures instead of
  remaining hidden. Runnable jobs rise from 36,786 to 36,795 and the complete
  vector reaches 32,398 passes. Full TSV/JSONL SHA-256 values are
  `8510e4117dd3854cd3c428548e36e0bba13a31abd66a875decf5f774850302d3`
  and
  `71cba68a097d685638b4f77f5e77676ea161e4212410724937ab9804d3c43cb8`.

  R2h adds QuickJS-shaped HomeObject state and direct SuperProperty Reference
  semantics to synchronous ObjectLiteral methods, getters, and setters. The
  HomeObject is installed after inferred naming and before property definition;
  the base follows its current prototype while ordinary reads/writes and the
  final method call use the current receiver. Matching the pinned
  implementation, a getter reached by `super.x()` first receives the frozen
  super base before
  its returned function is called with the method receiver. Fixed/computed
  reads, calls, assignments, logical
  assignments, updates, for-in/of targets, key-coercion/error ordering,
  strict-versus-sloppy rejected writes, and deletion errors are pinned against
  QuickJS 2026-06-04. The R2i and R2j follow-ups below resolve Arrow and
  direct-eval inheritance; parameter initializers, classes/derived construction,
  and async/generator methods remain separate frontiers.

  The frozen ObjectLiteral-super gate contains 26 paths and 48 variants; all 48
  are admitted and pass. Its manifest/key-set SHA-256 values are
  `75a8d27edff0f6add47f2538a1d44b07509353c1352e759427d4ef93dffd0210`
  and
  `e25ea45b40345ed6e368d2010f3a48b46364f822845094546a658526b530d41a`;
  focused TSV/JSONL SHA-256 values are
  `f9d39c6ecbbd768899ad6d9a0962a87271c35a3af8fef16f7a375d82139bb28d`
  and
  `501107f4cb1dd6f8db6a5e7a43b127a244abce810626fde34c2342e89fe1309e`.
  Declaring `super` and one audited negative path moves the profile to 54
  feature tags and 423 exact negative paths, with SHA-256
  `85cec5c2713df52c631ed38b96621e253baf9e1fafc06eceeea19e9eba64c6f9`.

  All existing focused gates remain green after regeneration; the smoke vector
  also advances two intended early errors to pass. The exact R2g/R2h join keeps
  all 102,037 unique keys and every previous pass. It adds 82 passes, exposes
  18 honest downstream frontiers/failures, and records nine detail-only changes.
  Runnable jobs rise from 36,795 to 36,825 and the complete vector reaches
  32,480 passes. Full TSV/JSONL SHA-256 values are
  `44f6f555cc8f72a6d0ff5ed392468a315b44d8c2cd289f7b72a65adde8c58a78`
  and
  `4d220f27199ee71757e368eb863a535264cc9914a85efaa90d69d54813dd575c`.

  R2i extends those ObjectLiteral SuperProperty References through synchronous
  ArrowFunctions. The arrow owns neither `this` nor HomeObject: the compiler
  lazily materializes both pseudo bindings in the enclosing method or accessor
  and relays them through ordinary closure slots, including nested and escaped
  arrows. The HomeObject's live prototype, lexical receiver, computed writes
  and updates, strictness, getter-call receiver split, and delete/grammar
  boundaries are pinned by an 11-case QuickJS differential.

  The focused ObjectLiteral-arrow-super gate freezes four paths and eight
  sloppy/strict variants; all eight are admitted and pass. Its manifest/key-set
  SHA-256 values are
  `d29f77c5920b21a92f61b0022eb186b5ba24e100f6ffa52b4d952347c9aaad90`
  and
  `4ac13c25ee6b84ee9019b53f5119fb2d7dc3154eb9785eda8800f725bbf32eba`;
  focused TSV/JSONL SHA-256 values are
  `afa0f32205ef75af6aae165a3b2e74023d4408cef423333cad63454f9c402872`
  and
  `0c35ca795fc6b8329bcc6a3af0bbe7878d9819e22bf8b590f2634c79fbba4cbc`.
  The capability profile remains unchanged at 54 feature tags and 423 exact
  negative paths.

  The exact R2h/R2i full-vector join retains all 102,037 unique keys with no
  missing, extra, duplicate, or detail-only rows and no previous-pass
  regressions. Exactly four rows move from `unsupported-parser` to pass: the
  sloppy/strict variants of
  `prop-dot-obj-val-from-arrow.js` and `prop-expr-obj-val-from-arrow.js`.
  Runnable jobs remain 36,825 and the complete vector reaches 32,484 passes.
  Full TSV/JSONL SHA-256 values are
  `dcc079d5c819b066703046136bfe2bdb17a6f02723796c6a8020680db0bb3acb`
  and
  `c82f264111cd4d0526f2f607ead97aab0e2776b49410b58d25425b8491df2664`.

  R2j extends that lexical SuperProperty capability through direct eval without
  treating stored HomeObject state as parser authority. Matching QuickJS, the
  compiler carries independent `super_call_allowed` and `super_allowed` bits:
  synchronous ObjectLiteral methods/accessors publish `(false, true)`, ordinary
  functions, scripts, and indirect eval publish `(false, false)`, and Arrow and
  direct-eval compilation inherit the exact pair. Bytecode publication and VM
  invocation authenticate that exact pair. The HomeObject pseudo local remains
  storage and closure transport only. This admits direct and nested eval in
  methods/accessors, plus authored and eval-created Arrow relays, while ordinary
  function, global, and indirect-eval boundaries cut the capability off.
  `super()` remains disabled pending classes and derived constructors.

  The resident oracle freezes 16 cases with an always-on Rust expectation test
  plus pinned-QuickJS expectation and direct differential checks when
  `QJS_ORACLE` is present. The focused ObjectLiteral-eval-super gate freezes 12
  paths and 24 sloppy/strict variants; all 24 are admitted and pass. Its
  manifest/key-set SHA-256 values are
  `8643870c3932da98f7ba60cb4e7d4499b02783853f4154f096122796bd998b0f`
  and
  `6f193e1ebf25a09717fe1c9bbd032d3f1b9cc38eb602870e551f50d5e82277fa`;
  focused TSV/JSONL SHA-256 values are
  `5fa67acef400c5525df9eace328219a30539a1661776ebc964e9ac6c4d38a470`
  and
  `5274231bdedc8c3d99f159626cdeef92fe4cf1fe6a9427d70b6f81f9928fbf0a`.
  The capability profile remains unchanged at 54 feature tags and 423 exact
  negative paths.

  The exact R2i/R2j full-vector join retains all 102,037 unique keys with no
  missing, extra, or duplicate keys. Exactly six rows move from `fail-runtime`
  to pass, with no previous-pass regression, detail-only change, or row-metadata
  drift. Runnable jobs remain 36,825; the complete vector reaches 32,490 passes
  and `fail-runtime` falls to 2,425. Full TSV/JSONL SHA-256 values are
  `8a1633a0d527bc77926124f3a6e1fa5ef340e6e79626a22ed171f37dafb8c6e0`
  and
  `b904278dd9c8cc5d3cf54babd037723ec7e52d015a636fe0d19ef5a4b0f36cfb`.

  R2k ports QuickJS tagged-template semantics without adding a global cache.
  The parser records cooked/optional-undefined and raw UTF-16 segments as a
  structural constant; runtime publication materializes the two frozen
  realm-local Arrays once, and the bytecode constant edge preserves per-site
  identity across closures, StripDebug mode, and cycle collection. Tagged
  calls reuse ordinary Reference promotion for dot, computed, `with`, and
  `super` receivers, while tagged `eval` remains indirect. Constructor
  precedence, chained tags, invalid escapes, descriptor shape, evaluation and
  abrupt order, dynamic eval/Function site separation, newline continuation,
  and direct-eval HomeObject relay are pinned by 16 QuickJS differential
  vectors. A separate Rust lifecycle test locks site identity across StripDebug
  publication and cycle collection.

  The focused gate freezes 48 paths and 89 variants. It executes 85: 83 pass
  and two stop at the pre-existing PrivateName literal runtime frontier. Two
  `create-realm` variants remain host-unsupported and two TCO variants remain
  excluded by the pinned configuration. Its manifest/key-set/non-pass hashes
  are
  `d3a7e597a049e9a78830ee089a90db27c6b6b0b8b2d049cd76b30f5515e6d23a`,
  `91852cd5c970debac2ef05af2715198736757b1276a34e6a73722df86bd80356`,
  and
  `981d8dba14c5cad2481e890d2dfc0925fd5ef03139aca7109d52891166a2c4aa`;
  focused TSV/JSONL hashes are
  `62322ceafcf309aedb8ee6a0b155fef9f24a67356a5408a496647a6f93ed353d`
  and
  `c91514b3d5b4500ec88d491e19719b139422bd7910876993fbb6a36a9cb70230`.

  Declaring `template` moves the profile to 55 reviewed feature tags with
  SHA-256
  `d146a337c9bab8b171aaddfe31d404073a9d3cbb65fd7ac7d6ab46fdefe69ef7`.
  The exact R2j/R2k join retains all 102,037 unique keys and records 79
  `unsupported-parser -> pass`, two `unsupported-runtime -> pass`, two
  `unsupported-feature -> pass`, and two
  `unsupported-parser -> unsupported-runtime` transitions. There are no
  missing, extra, duplicate, or detail-only rows and no previous-pass
  regressions. Runnable jobs reach 36,827 and the
  complete vector reaches 32,573 passes. Full TSV/JSONL SHA-256 values are
  `96dfb48f8887e525ff2813e4f8ac9ab7cf191f9e0fedd0d8724ee52943ce60e9`
  and
  `799be95a11b86d2b1efdfa694cd88971a600c64992fd07b03d61d913377f2e23`.

  R2l ports the pinned strict JSON parser and post-order reviver walk. It uses
  JSON's own UTF-16 grammar rather than the JavaScript lexer, allocates
  realm-correct Arrays and ordinary objects directly, preserves QuickJS's
  duplicate-key parse-record selection, and supplies the third reviver
  context argument with an exact primitive `source` slice only while the
  parsed value still matches. The focused gate freezes 84 paths and 168
  variants: 166 pass, while the sloppy/strict forms of the 2,097,153-element
  dense-array stress test retain a visible timeout frontier. There are no
  unsupported or skipped rows. Its manifest/key-set/non-pass hashes are
  `16b919d34d9eebcc60a92e038e0a6fd565e9306c1ba17cffc6f62ce0f05f23c4`,
  `36e19d071bb8ad9e4982ae85a5f32a3205925b6bf68fe335cfd1cbdfb429cff9`,
  and
  `2436785b58ef14db6e47d65537af5a9edf58e33bec81837eaf2f3b36f1eee4d0`;
  landing TSV/JSONL hashes under the R2k profile were
  `31d01dbc119767d5eb9e2be69c9054f97ca78a3b4ca5e5ae60faf9ed1f29b8e9`
  and
  `7ed6c23a8b94dfb2854f9be793c4aba388d64a432e0a931d6d8d81dbb7c38dbf`;
  after R2m's profile-metadata migration, the R2m-era gate hashes were
  `22377dfabe093c798ec712be77ab06ca600e11725666945e523b68410d6927cb`
  and
  `2fa563ffd36405eee7433e0aada0abe1a1474e64b31228949f5a0dc04af2da04`.

  R2m completes the JSON intrinsic family on the pinned QuickJS path.
  `JSON.stringify` preserves replacer/space/root-holder order, `toJSON` and
  replacer calls, wrapper coercion, key and length snapshots, path-only cycle
  detection, UTF-16 quoting, BigInt errors, and pretty-print gaps. Traversal
  uses an explicit task stack, so it has no Rust-recursion cutoff below the
  pinned engine; differential cases lock both 257 and 4,096 nested Arrays. Its
  focused 80-path/160-variant gate passes 160/160. `JSON.rawJSON` validates
  through the same strict parser and constructs a null-prototype,
  non-extensible object with a runtime-wide unforgeable heap brand. Stringify
  splices the exact
  source lexeme; `JSON.isRawJSON` checks that brand without invoking user code.
  The 22-path/44-variant raw gate records 36 passes, four unrelated rest/spread
  parse failures, two unrelated arrow-destructuring parser frontiers, and two
  pinned staging exclusions. Stringify manifest/key-set/non-pass hashes are
  `001d8337407a2689dc181120160bc6d45d6b03765ec5ca0c2c7f3421f9705f11`,
  `ab8b0bdfa3895693115c79579f936d2559806dbc95f2588537267a73d6039892`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`;
  its R2m-landing TSV/JSONL hashes are
  `38ebfa11ff63d080072eb93845711ff4f90bd6753a70fa793edc0c128f89bd82`
  and
  `1ff4e957792cf2f1702f21df30bd7656d5448a71f5cf9fcc6f37c9cd48fa445b`.
  Raw JSON manifest/key-set/non-pass hashes are
  `8e4d1fa6f59eae77cf1a35668ea02002de4d4f4cae146bb9ea6bde1c849b1df4`,
  `c5be0b3a9dd6c106d9e1c19cd15726b7a6756ac5ee464d4279fd835d520ddee7`,
  and
  `2c8fb7640ded74e86d6e5b8990dcaf8650ec0eccbc855cb2dcbef808e8caae8a`;
  its R2m-landing TSV/JSONL hashes are
  `bb3792c4b565855a533a56db306f9fb465b6f899ca739db3a0ceb92979a0cf34`
  and
  `4d76fd54f0d4878a816f452170f1b7436fec0c86a0c601d925f86aca1ae16264`.

  Declaring `json-parse-with-source` and `well-formed-json-stringify` moves the
  capability profile to 57 reviewed feature tags with SHA-256
  `0c6b9ef80d683bd69a97f87bbee10e7029432deb25d23695a96c251e9dfc9f66`.
  Every profile-aware older focused baseline is re-emitted because its report
  header pins this hash; those changes are metadata-only, with outcomes and
  key sets unchanged, while the sections above retain landing-history hashes.
  The exact R2k/R2m full join retains all 102,037 unique keys with no missing,
  extra, duplicate, or previous-pass-regression rows. Of 518 outcome changes,
  472 move from `fail-runtime` to pass, 38 from `unsupported-feature` to pass,
  two from `unsupported-feature` to `unsupported-parser`, four from
  `unsupported-feature` to `fail-parse`, and two dense-array rows from
  `fail-runtime` to timeout; nine additional rows change detail only. Runnable
  jobs reach 36,871 and the complete vector reaches 33,083 passes, a net gain
  of 510. Full TSV/JSONL SHA-256 values are
  `63d5a44dd8d057e220882d02abebb1b221fdb1a419ce1fc691e1ed084d2b0a3e`
  and
  `0b8eedcae7d427a6bf7fbbcefb412d9f2691c0bdf00c4bc2229bbfd1a8212fb2`.

  R2n ports the pinned strong `Map` family through realm-local constructor,
  prototype, and iterator graphs. Heap-backed ordered records use
  `SameValueZero`, normalize negative zero, retain deletion tombstones, and
  preserve live mutation semantics for iterators and `forEach`. Construction
  follows QuickJS's cached-adder and `IteratorClose` ordering; the complete
  surface includes `set`, `get`, `has`, `delete`, `clear`, `size`, `forEach`,
  `keys`, `values`, `entries`, `getOrInsert`, `getOrInsertComputed`, species,
  tags, and `Map.groupBy`.

  The dependency-audited focused gate freezes 186 paths / 370 variants and all
  370 pass. `Symbol.iterator` and `upsert` are admitted only by its runner-bound
  scoped profile, whose SHA-256 is
  `16ab6bfe18540aae398c847905f492491e81500045b45a6bfb21f447fd537ea2`;
  this is not a global claim for Set, WeakMap, or other consumers. Focused
  manifest/key-set/non-pass hashes are
  `50387c488c3ade2aafbbe2cd4cecc387bc0c97a76808831d74b634407b990cd1`,
  `2704f0c3407fa65dec9297df89f3643eba808f72347b530c71f091be15b14d81`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`;
  TSV/JSONL hashes are
  `10e2e4ca4f285eaaf345c1231b7707951e72882e1d603dc144cdde50eb8ed645`
  and
  `e8645afd72aec2e917fbc11ae4c9502bbb4473897414cc9882027d79082cda69`.

  Declaring only `Map` and `array-grouping` globally moves the capability
  profile to 59 reviewed feature tags and 423 audited negative paths, with
  SHA-256
  `0f4617ff1678710c97620aa1257c4868b2a4daf0f4f917f9d7393566ee549c45`.
  The exact R2m/R2n full join retains all 102,037 unique keys and records 234
  `fail-runtime -> pass`, 80 `unsupported-feature -> pass`, eight
  `unsupported-feature -> fail-runtime`, and four
  `unsupported-feature -> unsupported-parser` transitions. The eight runtime
  failures expose four WeakMap receiver-brand paths in both modes; WeakMap
  remains unimplemented. The four parser frontiers are the two subclass-Map
  class paths in both modes. Eighteen more rows change detail only. There is no
  previous-pass regression or outcome drift outside the reviewed admission
  set: the focused Map manifest plus rows gated by the newly global `Map` or
  `array-grouping` tags. Runnable variants reach 36,963 and passes reach 33,397,
  a net gain of 314.
  Full TSV/JSONL SHA-256 values are
  `5a0502380cb281bb089fe229cb1ec806228dd70e75987f852476984cb4d30271`
  and
  `2370d923625dc76d0a89c8314ed16875a402bccde665b6e45e30948e7526a2f8`.

  R2o ports the pinned observable strong `Set` family through realm-local
  constructor, prototype, and independent Set-iterator graphs. Its heap-backed
  ordered records use `SameValueZero`, normalize negative zero, and preserve
  live mutation for iterators and `forEach`. Construction follows QuickJS's
  cached-adder and `IteratorClose` order. The surface includes `add`, `has`,
  `delete`, `clear`, `size`, `forEach`, the exact keys/values alias, `entries`,
  species and tags, `Set.groupBy`, and all seven set-composition methods. Those
  methods follow QuickJS's set-like protocol, branch-specific iteration and
  close behavior, and defining-realm result allocation without consulting a
  subclass species or overridden `add`.

  The dependency-audited focused gate freezes 322 paths / 642 variants and all
  642 pass. The global profile already admits `Set` and `set-methods`; its
  runner-bound scoped profile adds only the exact well-known-Symbol dependencies
  needed by that frozen surface and has SHA-256
  `6869e9d28fff1d5bd4e5b698dcdf6ee677b9134a91781ad7abe226200d669455`.
  Class, generator/object-generator, rest-parameter, lexical-destructuring,
  WeakSet, and `$262.createRealm` dependencies remain separate frontiers.
  Focused manifest/key-set/non-pass hashes are
  `44c6b6b599e7fe48324aaa693fa684649469c35209bc5c1edb34f0eebe2085b9`,
  `5b4959128a9fb34b72b83950fd329f8a98bbbb2b08f256d5ff8bc3f7bc73a0ac`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`;
  TSV/JSONL hashes are
  `b45345b024a33560f2244b69bcdd181e2c5f07add1a04d9fe474169117cb222b`
  and
  `de7d718b67a1bae7d8031345ce55ba7f32aa8a5d6bcefd745ac2c4401ae65e3f`.

  Declaring only `Set` and `set-methods` globally moves the capability profile
  to 61 reviewed feature tags and 423 audited negative paths, with SHA-256
  `086b4964eebc8dd8960b33aaa333b0adaeefb1447cbf63f893042ab269a5a17b`.
  The exact R2n/R2o full join retains all 102,037 unique keys and records 342
  `fail-runtime -> pass`, 302 `unsupported-feature -> pass`, 82
  `unsupported-feature -> unsupported-parser`, 50
  `unsupported-feature -> fail-parse`, and 14
  `unsupported-feature -> fail-runtime` transitions. The focused manifest
  accounts for 602 of the 644 new full-vector passes; 42 linked Map-brand,
  for-of, and staging variants pass outside it. Its other 40 scoped variants
  remain fail-closed under the global profile because their Symbol dependencies
  are deliberately admitted only by the scoped gate. The 14 newly exposed
  runtime failures are WeakMap/WeakSet receiver-brand cases; the parser and
  parse failures expose the already tracked class, generator/object-method, and
  parameter-syntax frontiers. There is no previous-pass regression or outcome
  drift outside the focused manifest and rows selected by the newly global
  tags. Runnable variants reach 37,411 and passes reach 34,041, a net gain of
  644. Full TSV/JSONL SHA-256 values are
  `14f8412069dc7ba2a648c2facead1cbcd79ccf2cc5116832602f50decd5f95ab`
  and
  `c29229ceeee55db836e701d8a2984ef0ba9eb9396d6deca8a5166026b58bb71b`.

  R2p audits the already-implemented well-known Symbol graph and globally
  admits `Symbol.asyncIterator`, `Symbol.hasInstance`, `Symbol.iterator`,
  `Symbol.prototype.description`, `Symbol.species`, `Symbol.toPrimitive`,
  `Symbol.toStringTag`, and `Symbol.unscopables`. Existing focused QuickJS
  differentials pin their intrinsic graph, descriptors, coercion, iteration,
  species, instance checks, tags, and unscopables behavior; no production
  runtime change is needed for this admission milestone.

  Its dependency-audited Test262 gate freezes 517 paths / 1,010 variants under
  an exact 30-feature scoped profile with SHA-256
  `ff674aafc4b1b61b0c40042f831b44c600b1f741e06b8c8c35863b876919aa7b`.
  All 806 Symbol-ready variants pass. The other 204 expose only independent
  class, rest/spread, Promise, buffer/TypedArray, Proxy, and weak-collection
  frontiers: 60 parse failures, 98 runtime failures, 18 harness failures, and
  28 typed parser frontiers. Normalized-manifest/manifest-file/key-set/non-pass
  hashes are
  `eaf2a48408b6b1f5673389335cda73cb66bed062636a669c655460d9fef99a4b`,
  `6147636f7950b899f7c0eea25078e2f4c9c4c7fda2977181dd7c9671aa0bcde2`,
  `e87d58ad7a8be3e60b5545129a70a1abd70ee350654092a4aa066d17dc69e450`,
  and
  `4783b1a8bb909a6e4706138265c477cfa3979bb6821f09f590e4c8c66a0dd5d2`;
  TSV/JSONL hashes are
  `ed0363676e7efdfc6bb24ee396739cf67d49a4ce685c3bd37d98569a60a96267`
  and
  `75c40ff9adf28f0b9120c23af44268b4660189ff815e3f4c2ba0b74786ede048`.

  The global profile reaches 69 reviewed tags and 423 audited negative paths,
  with SHA-256
  `a1a347d2d74c946a50f1e26fca6c1756c0e9948f087de3aed2339b3a4c7d6677`.
  The exact R2o/R2p join retains all 102,037 keys. Its 1,010 outcome changes
  exactly equal the focused key set: 806 become passes, while 204 expose the
  independent frontiers above. Another 1,954 rows change detail only. Every
  changed row carries a newly admitted tag, with zero previous-pass regression,
  missing/extra key, or unrelated outcome movement. Runnable variants reach
  38,421 and passes reach 34,847. Full TSV/JSONL SHA-256 values are
  `a56285e53591df1d2026da4d6334d42e374a107cbcc7744e87f1d8b4c49d865d`
  and
  `0f1b3899b73d990575b8ee1f4cb11e308847c5fd3fb728b13b3e3e583e08f15e`.

  Binding/destructuring is the next high-yield semantic line. WeakMap and
  WeakSet remain later work because they first require genuine weak heap edges.

  R2q takes the first binding slice across the existing declaration
  architecture. Flat ArrayBindingPatterns now work for `var`, `let`, and
  `const` in Program code, ordinary-function bodies, nested blocks, shared
  switch scopes, classic `for` heads, and synchronous `for-in`/`for-of` heads.
  Identifier leaves, empty patterns, elisions, trailing commas, undefined-only
  defaults with NamedEvaluation, and terminal rest bindings share one lowering
  owner. Direct declarations use QuickJS-shaped right-hand-side control-flow
  inversion and the existing iterator/unwind bytecode. `var` also prepares its
  dynamic Reference before `IteratorStep`, fixing observable `with` cases whose
  iterator mutates the object environment before the write.

  The dependency-audited R2q Test262 gate freezes 90 paths / 180 variants and
  passes all 180. Its exact two-feature scoped profile has SHA-256
  `8232e2c11e908f7cbf5a9e0f34fbd5223a9551b49ae64647f2a72b2314bcaf84`.
  Normalized-manifest/manifest-file/TSV/JSONL hashes are
  `257af4e4f08f01ed33c0d88a7c64b44dd29adee6bbc64d87cb0213402e72c048`,
  `db17670a1f7715a325a07087b766f6e64cf2bb24cec727278db05db3f79ee679`,
  `f0a66030c0a650874b003639775cb87149a4fcd221a1cfd80f603ab8d86f0dde`,
  and
  `ca54eb7e1763501e130fff72dd67ec90469ab8fbc580e12809b6e6cda88e2f35`.
  `destructuring-binding` remains scoped rather than globally admitted, but
  untagged Test262 and staging paths still exercise the new compiler surface.
  The exact 102,037-key R2p/R2q join records 23
  `unsupported-parser -> pass`, eight `fail-parse -> pass`, two
  `unsupported-parser -> fail-parse`, and four
  `fail-parse -> unsupported-parser` transitions, with zero previous-pass
  regression. The two new parse failures are both modes of one unsupported
  destructuring-assignment staging path; the four typed parser outcomes are
  nested patterns. Two other rows retain `fail-parse` but change to the same
  assignment diagnostic, so 39 data rows change bytes in total. Passes rise by
  31 to 34,878 while runnable variants remain
  38,421. The full summary now has 552 parse failures and 1,204 typed parser
  frontiers; every other R2p category is unchanged. Full TSV/JSONL hashes are
  `bc9e6f71acbad459fabfcd2838c691cf318a781dea3dc2239161eced7c065c2f`
  and
  `b0b99d49bec652fa0b686a8d9af4296a5b156db6fec849c56168fb1dc41e6b7e`.

  Parameter initializers and other non-simple parameters, classes/derived
  construction, and async/generator method/accessor forms stay typed frontiers.
  One entry-prologue composition debt also remains. R2i fixes the pseudo-local
  group itself to QuickJS's HomeObject, `new.target`, then `this` order, but the
  compiler still installs
  that group and the var-object/arguments/function-hoist group with separate
  prepend passes. The later prepend therefore leaves the declaration group
  before the pseudo locals, opposite QuickJS's complete entry prefix. Closures
  capture the intended cells and the mixed-entry oracle observes no semantic
  divergence; a single composer must eventually publish all entry groups once
  in upstream order.

  Generator/async and destructuring eval declarations, classes/derived
  constructors, and ill-formed UTF-16 source stay explicit frontiers.
  QuickJS also allocates the callable and VarRef
  array before capturing caller cells, while this Rust slice materializes the
  roots first and then allocates the callable; only successful-compilation
  OOM/GC priority is affected, but exact allocation-order parity remains a
  later generic closure-instantiation task.
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
- The first runtime-independent RegExp kernel follows pinned
  `libregexp.c`/`libregexp-opcode.h` rather than a host regex library.
  `src/regexp/` owns exact QuickJS flag bits, a UTF-16 pattern parser, typed IR,
  a compiler, and a non-recursive executor with explicit backtrack/capture/
  register undo stacks and a 10,000-step interrupt poll. The audited core covers
  literals, dot/anchors, alternation, capturing and noncapturing groups,
  greedy/lazy and bounded quantifiers, classes/ranges/inversion,
  `\dDsSwW`, word boundaries, basic escapes, leftmost/sticky search, raw
  UTF-16 and `u`-mode surrogate handling, and checksum-pinned Unicode 17
  RegExp case folding. Numeric backreferences preserve forward, self,
  unmatched, empty, scoped-ignoreCase, Unicode code-point and capture
  backtracking semantics; out-of-range non-Unicode decimal escapes follow
  QuickJS's Annex B octal/identity widths in and outside character classes.
  Forward lookahead uses typed positive/negative control frames on the same
  explicit stack: positive success commits captures while discarding internal
  alternatives, negative completion always rolls them back, and outer
  backtracking can still undo a committed positive capture. Non-Unicode
  quantified assertions retain QuickJS's Annex B zero-advance behavior.
  Lookbehind reuses those assertion frames while code generation reverses each
  alternative's terms, emits QuickJS-shaped `Prev` instructions around
  ordinary consuming atoms, swaps capture boundaries, and selects a bounded
  backward backreference. Variable-length, nested forward/backward assertions,
  greedy/lazy captures, anchors, word boundaries, and Unicode surrogate
  movement are covered without a recursive sub-executor.
  Unicode `u` patterns resolve exact-case General_Category, Script,
  Script_Extensions, and binary property aliases from checksum-pinned Unicode
  17 Rust tables generated through the pinned QuickJS implementation.
  `\P` preserves QuickJS's `u+i` inversion-before-folding order, non-Unicode
  `\p`/`\P` remain identity escapes, and property sets preserve QuickJS's
  class-range error priority. Full-domain case folding visits only the 1,585
  Unicode code points affected by the pinned case table rather than expanding
  all 1,114,112 code points.
  Ordinary named captures normalize raw and escaped Unicode 17 identifier
  names into runtime-independent metadata aligned to captures 1..N. Named
  references reuse the existing multi-capture forward/backward instructions;
  QuickJS's Annex B `\k` fallback, fixed name buffer, wrapping global
  alternative scope, and forward-scan cursor quirk are preserved. Match
  `groups` and `indices.groups` are null-prototype objects with exact
  duplicate-name order/value behavior, and named replacement uses the generic
  `$<name>` substitution route.
  Nullable finite repetitions carry QuickJS's
  zero-advance rollback rule; ignore-case class complements are folded before
  inversion; sequential quantifiers reuse temporary registers.

  The heap also has a genuine edge-free RegExp brand with explicit
  uninitialized/compiled states and reference-counted source/program leaves.
  Realm data atomically roots the ordinary `%RegExp.prototype%`, constructor,
  and canonical one-slot `lastIndex` shape. Typed native selectors publish the
  constructor/call identity and branded-copy paths, all flag/source/flags
  accessors, generic `toString`, builtin and abstract `exec`, `test`, legacy
  `compile`, species,
  `lastIndex` coercion/update/reset, captures, result metadata, and `d` indices.
  Allocation, coercion, error and result realms follow the pinned QuickJS
  order; matcher execution stays behind the interrupt-aware R0 boundary. The
  executor polls that boundary, while the runtime currently supplies a
  noninterrupting closure until the host interrupt hook is published.

  RegExp literals now follow QuickJS's compile-once/instantiate-many boundary:
  the compiler validates and compiles the pattern into a typed bytecode
  constant, and `Instruction::RegExp` creates a fresh object for every
  evaluation without observing the global constructor. The object uses the
  bytecode execution realm's canonical RegExp shape and prototype, with a new
  zero-valued `lastIndex`; invalid and unsupported patterns therefore retain
  compile-time diagnostics rather than becoming catchable constructor-time
  failures. At the R1b landing, a frozen 48-path/96-variant focused vector
  recorded 88 passes, two runtime failures and six typed parser frontiers: two
  lookaround and four
  backreference variants. All 88 passes were RegExp-literal parser frontiers
  under R1a. The two runtime variants stopped at an earlier
  `String.prototype.match` call; R1d makes both pass, R1k resolves four
  backreference variants, and R1l resolves the final two lookahead variants.
  The current focused literal vector therefore passes all 96 variants.

  Forty-four original matcher cases and 35 targeted observable intrinsic vectors match
  pinned QuickJS, including cross-realm construction/results/errors. The frozen
  225-path/450-variant Test262 RegExp-core vector now has 448 passes. R1x
  executes the five eval consumers; only two typed legacy-control frontiers
  remain.
  `RegExp.escape` and the remaining advanced
  literal grammar remain intentionally unpublished rather than stubbed. The
  R1a complete join recorded only 669
  `fail-runtime -> pass` and
  ten `fail-runtime -> unsupported-runtime` transitions. The R1b join matches
  all 102,037 keys and moves 840 `unsupported-parser -> pass`, 226
  `unsupported-parser -> fail-runtime`, 24 `unsupported-parser -> fail-parse`,
  and 103 `unsupported-harness-parser -> harness-error`, again with no
  previous-pass regression.

  R1c publishes the generic `RegExp.prototype[Symbol.search]` and
  `String.prototype.search` pair in pinned table order. String search rejects a
  nullish receiver before pattern access, performs object-only `Symbol.search`
  delegation with the original unconverted receiver and raw return value,
  bypasses boxed prototypes for primitive patterns, and otherwise constructs
  through the defining realm's retained canonical RegExp constructor before a
  dynamic search-method call. RegExp search requires an object receiver,
  converts the input before reading `lastIndex`, uses SameValue when resetting
  and restoring that property, invokes abstract RegExpExec, and returns `-1` or
  the result object's raw `index` while preserving every abrupt-completion
  boundary. Eight Rust tests—six comparison groups over nine QuickJS
  differential vectors, one oracle self-check and one cross-realm runtime
  test—lock metadata, order, delegation, constructor/global bypass, signed-zero
  and NaN restoration, abrupt paths, abstract exec and cross-realm behavior
  against `quickjs.c` 45609-45657, 46623-46640, 48817-48873 and 49007-49027.

  One observable parity gap remains in the shared native recursion guard: the
  fifth nested mixed String/RegExp match/search/split frame throws
  `InternalError`
  after four active protocol frames, while pinned QuickJS continues. This is a
  host-stack safety frontier rather than a protocol-algorithm rule, but still
  requires a trampoline or exact QuickJS stack budgeting before feature parity
  can be claimed.

  Its frozen 66-path/132-variant Test262 search vector now admits and passes
  128 variants; four retain adjacent feature requirements. R2g resolves the
  final 12 accessor consumers. At R1c the focused manifest gained
  110 passes from R1b and eight additional variants passed outside it, moving
  the full vector from 24,699 to 24,817. The exact join matches all 102,037 keys
  with no
  previous-pass regression: 66 `fail-runtime -> pass`, 52
  `unsupported-feature -> pass` and 12 `unsupported-feature ->
  unsupported-parser`.

  R1d publishes `String.prototype.match` and
  `RegExp.prototype[Symbol.match]` in the pinned table order. String match uses
  the same isolated generic-protocol helper as search: it rejects a nullish
  receiver before pattern access, delegates only object patterns through the
  ordinary `Symbol.match` Get with the original unconverted receiver and raw
  return value, bypasses boxed prototypes for primitives, and otherwise uses
  the defining realm's retained canonical RegExp constructor followed by a
  dynamic match-method call. RegExp match converts the input before reading and
  converting `flags`; non-global matching returns abstract RegExpExec's raw
  object or null, while global matching detects `g` plus `u`/`v`, resets
  `lastIndex`, repeatedly obtains and stringifies result slot zero into a
  defining-realm Array, and advances empty matches by the pinned UTF-16 rule.
  Abrupt completions retain their exact mutation and realm boundaries.

  The 155-line algorithm lives in
  `runtime/intrinsics/regexp/match_protocol.rs`; String match/search sharing
  remains in `runtime/intrinsics/string/regexp.rs`, and only eight exhaustive
  facade lines reached `runtime.rs`. Eleven Rust oracle, differential,
  cross-realm and recursion-guard tests pass; every differential vector
  matches QuickJS 2026-06-04 while the explicit guard test preserves the
  depth frontier above. The frozen
  104-path/208-variant match vector now admits and passes 206 variants; two remain behind
  `regexp-v-flag`. R1x executes the legacy eval consumer. Its current TSV
  and JSONL SHA-256 values are
  `5aa6b8b6c61a48acf72417d583f3439b8fbfc5dde9020b8c8341e31759a790a6`
  and
  `5f3e63c0d709819e47a57e4bfbb3929a565b615d74a6a95966b3dc19c90948e2`.
  Admitting `Symbol.match` brings the conservative profile to 18 tags with
  SHA-256
  `cc10293aa847f5a449ac2b039709dff98d264b672dddc8828b8e17d8b7e12d9a`.
  The exact full join matches all 102,037 keys with no missing, extra or
  duplicate rows and no previous-pass regression: 86 `fail-runtime -> pass`,
  126 `unsupported-feature -> pass`, 16 `unsupported-feature ->
  unsupported-parser`, and two `unsupported-feature -> fail-runtime`. Those
  last two variants are one Annex-B path that at R1d reached the
  then-unimplemented `RegExp.prototype[Symbol.split]`. The transitions move the
  complete vector to 25,029 passes and 32,497 admitted jobs; the full
  TSV/JSONL SHA-256 values are
  `a695d6299b44e4298b553c28c12983b6b12fc9d8522f1216e18e16a6bad28012`
  and
  `fb305cd709b2af1bf28de5fc82b440f836a0567ff8ed3e36af967723e3beb64b`.
  The literal-focused vector independently moves from 88 to 90 passes.

  R1e publishes `RegExp.prototype[Symbol.split]` in pinned table order and
  activates the already-audited generic `String.prototype.split` delegation
  for RegExp separators. The protocol ports QuickJS's SpeciesConstructor,
  flags-to-sticky construction, `u`/`v` UTF-16 advance, abstract RegExpExec,
  capture insertion, limit checks, mutation, abrupt-completion and
  defining-realm boundaries. The reusable species helper remains in
  `runtime/intrinsics/regexp/constructor.rs`; the 237-line loop lives in
  `runtime/intrinsics/regexp/split.rs`, and only four exhaustive facade lines
  reach `runtime.rs`.

  Eight Rust tests over 19 QuickJS differential vectors pass. The frozen direct
  46-path/92-variant RegExp split vector now admits and passes 50 variants; 40
  core variants remain conservatively gated by the undeclared `Symbol.species`
  profile tag and two require the create-realm host hook; R2g resolves the four
  former accessor parser frontiers. Species construction itself is
  locked by the QuickJS differential suite. Its current TSV and JSONL SHA-256
  values are
  `377746133482618291d3948d5a2da8a30f2cd7c6a7ca9cf3fce3589f426b8be5`
  and
  `853e1dcd3353307b0c6e2b71f4acfa3df3014f9c1dd516caad6d3f62a3f51629`.
  The independent 127-path/254-variant String split gate now admits and passes
  248 variants; four retain adjacent feature requirements and two require the
  IsHTMLDDA host hook. R1p resolves the two Annex B `\k` separator variants,
  R1x executes the two eval consumers, R2c resolves the Arrow consumers, and
  R2f resolves the six concise-method consumers. Its
  current TSV/JSONL SHA-256 values are
  `4b13051099f6c20379c67e3177e9cf46829f569ace3fe6f0eb7c48655fdc0f54`
  and
  `565c51f190bd1f44de4754bb76c155aa88c72b84a229d54bbb388f3213d83683`.

  The exact R1d-to-R1e full join has only 90 `fail-runtime -> pass`
  transitions, moving the complete vector to 25,119 passes while leaving
  32,497 admitted jobs unchanged. Five- and eight-worker reports are
  byte-identical; the full TSV/JSONL SHA-256 values are
  `5673ac15896bab5b1665bf8930db517447012c3d63d69bfbb1da9b8e7f9574c1`
  and
  `fe98f9fdb5f4c21c25cd045d8b1824fe34e3481e26c8661376d7afe78596fa64`.
  Two `staging/sm/RegExp/split.js` variants remain `fail-runtime`, but now
  proceed to the independent missing-JSON-global frontier; this is a
  detail-only change rather than an outcome transition. The conservative
  profile remains at 18 tags with SHA-256
  `cc10293aa847f5a449ac2b039709dff98d264b672dddc8828b8e17d8b7e12d9a`.

  R1f publishes `RegExp.prototype.compile` between `exec` and `test`, with
  pinned name/length/descriptors and the concrete RegExp brand used by
  QuickJS. Genuine RegExp patterns clone their internal source/program without
  observing `@@match`, `source`, or flag properties; ordinary patterns convert
  pattern before flags and compile transactionally. Successful compilation
  replaces the payload before the throwing `lastIndex = 0` Set, so a readonly
  `lastIndex` reports TypeError while retaining the new matcher. Same-object,
  derived and cross-realm branded copies, defining-realm native errors,
  user-error provenance, failure atomicity and catchable conversion recursion
  are locked by six Rust tests over 16 pinned QuickJS differential vectors. The
  implementation lives in a 96-line sibling module. The shared native-stack
  policy is now isolated in `runtime/native_stack.rs`, leaving `runtime.rs` at
  9,787 lines while keeping compile's measured recursion ceiling explicit.

  At R1f the frozen 35-path/70-variant compile vector recorded 44 passes;
  its only runtime failures were the sloppy/strict variants of one staging
  replace path at the then-missing `@@replace` protocol. Later slices bring the
  current vector to 60 passes, four runtime failures, four configured
  legacy-feature skips, and two create-realm host
  frontiers. Its current TSV/JSONL SHA-256 values are
  `42e98acb28de0b33a359fb169e0171738e91ecde5cbba7fde4ec8461447c6073`
  and
  `b9ee3a249eb3f0945727cea6c8a3319f69d584a0f00b6709ff09144719cbbdb3`.
  A QuickJS-shaped lexical capture-count prepass also distinguishes known
  out-of-range Unicode decimal escapes from in-range references, moving the two
  `unicode_restricted_octal_escape.js` variants to pass while preserving typed
  Unsupported results until the reference executor landed. R1k completes that
  path, so the R1k RegExp-core gate moved from 430 at R1a to 434, with six
  typed frontier outcomes. Later RegExp slices and R1x lead to the 448-pass current
  vector summarized above.

  The exact R1e-to-R1f full join matches all 102,037 keys with no missing,
  extra, or duplicate rows and no previous-pass regression. Its only changes
  are 44 `fail-runtime -> pass` and two `unsupported-runtime -> pass`, moving
  the complete vector to 25,165 passes and reducing runtime failures to 3,803
  and typed runtime frontiers to eight. Five- and eight-worker reports are
  byte-identical; the full TSV/JSONL SHA-256 values are
  `57caefa97b579fafeb6b56ba45da7daf9cbe5e168849e4ab0459b87452d4745e`
  and
  `613a396d850698fff9472991e547946eac6bc9bc4f3b95cf90ce57d85953dee0`.
  At that milestone the next RegExp priorities were split between
  matchAll/replace protocol work and advanced pattern grammar; none of this is
  a parity completion claim.

  R1g ports QuickJS's scoped RegExp modifier grammar
  `(?ims-ims:...)` into the runtime-independent compiler. Duplicate modifiers
  are rejected within each list before empty/overlapping sets and a missing
  colon, matching the pinned error priority. Each modifier group snapshots the
  effective `i`, `m`, and `s` state, applies it to literals, character-class
  canonicalization, word boundaries, anchors, and dot instructions, then
  restores the enclosing state. The group remains noncapturing and
  quantifiable, and the RegExp object's global flags are unchanged. Eighteen
  QuickJS differential vectors cover grammar, nesting, Unicode case folding,
  constructor/literal equivalence, captures, quantification, and global exec
  state; all four oracle test groups and all 675 library tests pass. The change
  stays in `src/regexp/compiler.rs`; `runtime.rs` remains 9,787 lines.

  The complete focused feature vector freezes 230 paths and 460 variants. At
  R1g it admitted all 460, recorded 452 passes, and left eight Unicode
  property-escape parser frontiers; R1m resolves those final eight, so the
  current gate passes all 460. Its current TSV/JSONL SHA-256 values are
  `e592663e667fc508e7f0f1af348924b9a9aab8035468188ff39e852833f1a817`
  and
  `9879b6b3166b91409666e10b384ddeed9fce6e9c5a3fa87294a09066ee075e9d`.
  Publishing the feature also audits exactly 83 modifier-owned literal
  parse-negative paths, moving the capability profile to 19 feature tags and
  101 negative paths with SHA-256
  `0d26aedd5b5d7fa00b6c2551a93c7d776f22e2934b790615d6dc58c454156d5f`.

  The exact R1f-to-R1g full join matches all 102,037 keys with no missing,
  extra, duplicate, outside-feature, or previous-pass regression. Its only
  changes are 448 `unsupported-feature -> pass` and 12
  `unsupported-feature -> unsupported-parser`, moving the complete vector to
  25,613 passes and 32,957 admitted jobs. Five- and eight-worker reports are
  byte-identical; the full TSV/JSONL SHA-256 values are
  `5ece50a681fcb4fe97779002b179174930d2cdbdb4bd2120e0679678bd96b161`
  and
  `83539d1bcea789f87853cdc6d9862dd2741d61a5b6696e8513e551318c9e5df8`.
  Earlier focused reports change only in their profile-hash metadata; replacing
  the new header hash with the R1f value reconstructs every old report hash
  exactly, so their outcome rows and milestone provenance remain unchanged.

  R1h ports QuickJS's shared replacement kernel instead of implementing the
  three public entry points independently. `ReplacementStringBuffer` retains
  narrow strings until widening is required, uses fallible growth, and latches
  the first allocation failure while later observable getters and callbacks
  continue in the pinned order. A shared `GetSubstitution` implementation
  handles `$&`, ``$` ``, `$'`, numbered captures, named captures and raw UTF-16.
  String `replace`/`replaceAll` preserve object-only `Symbol.replace`
  delegation, conversion order, empty search advancement, callback arguments,
  and the global-RegExp requirement for `replaceAll`. The generic RegExp
  `@@replace` path collects every abstract
  `exec` result before reading captures or invoking callbacks, preserves
  backward-position observation, enforces QuickJS's 65,534-argument ceiling,
  and keeps `lastIndex`, Unicode advancement, named groups and abrupt
  completion order aligned with the pinned source.

  R1i ports QuickJS's standard-RegExp predicate without performing ordinary
  property reads: it requires a genuine RegExp, a numeric raw own `lastIndex`,
  exact native `exec`, `flags`, `global`, and `unicode` targets, and stops raw
  prototype traversal at Array, Arguments, or String exotic objects. AutoInit
  remains observable: a cold `exec` slot forces the first call through the
  generic path, while its materialization can make a later call—or the same
  call after replacement conversion—eligible. Native target identity is
  compared independently of realm, matching QuickJS's C-function-plus-magic
  check, and deliberately does not inspect the other flag getters.

  Eligible non-functional replacements drive the compiled matcher directly,
  without abstract `exec`, result arrays, groups, or indices allocation.
  Capture ranges feed the shared substitution parser directly, while global,
  sticky, empty-match Unicode advancement, executor errors, the second direct
  StringBuffer, and `lastIndex` writes follow the pinned order. Six String and
  all nine RegExp differential groups now pass against QuickJS 2026-06-04,
  including predicate fallback, exotic prototypes, unchecked getters,
  cross-realm native targets, captures, global/sticky state, and Unicode empty
  matches.

  Recursive custom `exec` initially exposed a native-stack mismatch on the
  fixed 2 MiB oracle thread. Splitting replacement processing and VM
  call/numeric dispatch reduced the debug
  `CallFrame::execute_inner<RuntimeVmHost>` frame from about 75.9 KiB to
  57.0 KiB. `recurse(8)`, catchable infinite-recursion
  `InternalError: stack overflow`, logical `Function.prototype.call` frames,
  and post-overflow recovery now match the pinned oracle without enlarging the
  test stack or weakening the depth requirement. The call trampoline advances
  one window through its owned argv instead of copying every suffix, so a
  20-frame logical call chain also matches QuickJS without the former
  non-protective 16-frame family ceiling.

  The frozen replace manifest covers 191 paths and 376 variants. At R1h it
  admitted 332 and recorded 286 passes. R1i's direct standard-RegExp path
  preserved that outcome vector; at R1p it admitted 348 and recorded 300
  passes. The current R2h vector admits 354 and records 350 passes: four fail to
  parse, 16 retain independently undeclared features, two require create-realm,
  and four require IsHTMLDDA. The current
  focused TSV/JSONL SHA-256 values are
  `5521571759251d3b2a70a343a0e1397b80fbd4ad989ddca05c19680466c982c0`
  and
  `2f425f6a24aa21bf42a3ada7d6f7fc456cfb3fcc99cdf750b043f28b62db9c12`.
  Publishing `String.prototype.replaceAll` and `Symbol.replace` moves the
  capability profile to 21 feature tags with SHA-256
  `921df0ef452f4d1286162093ebdf81a74d0805eb7c04601c86abd6ec7347ed7f`.

  The exact R1g-to-R1h full join matches all 102,037 keys with no missing,
  extra, duplicate, or previous-pass regression. Its transitions are 110
  `fail-runtime -> pass`, 170 `unsupported-feature -> pass`, four
  `unsupported-feature -> fail-parse`, and 38
  `unsupported-feature -> unsupported-parser`. The complete vector moves to
  25,893 passes and 33,169 admitted jobs. The full TSV/JSONL SHA-256 values are
  `2895a8d2ddbe5857e83b573827e46b4a60a97d89b5882727c85ff75d2ff9d368`
  and
  `64fed7fd3bb722d470bbd420e42995e138aed5d6f3588b7d2657973cb3968419`.

  R1i changes the route taken by already-passing branded RegExp replacements,
  so it intentionally does not widen the capability profile or frozen
  manifests. Re-running both gates produces the same 286/376 focused result,
  the same 25,893/102,037 complete result, and the exact same four report
  hashes above. The exact R1h-to-R1i join therefore has zero outcome
  transitions, missing keys, extra keys, duplicates, or previous-pass
  regressions.

  R1j adds a distinct `RegExpStringIterator` heap class with its own
  `%RegExpStringIteratorPrototype%`, raw IteratorNext ABI, and matcher GC edge.
  Completion flips only the iterator's `done` bit; the matcher and input remain
  retained until finalization, matching QuickJS. `RegExp @@matchAll` preserves
  input conversion, species lookup, flags conversion, construction,
  `lastIndex` cloning, cached global/full-Unicode modes, abstract `exec`, empty
  match advancement, and exception retry state. String `matchAll` preserves
  the observable `Get(@@matchAll)`, `IsRegExp`, flags-validation, delegation
  order, while its fallback uses the defining realm's retained RegExp
  constructor with the literal `g` flag.

  Twelve differential tests across 26 QuickJS vectors cover metadata,
  construction order, custom exec, done/error behavior, Unicode empty matches,
  fallback, global validation, and cross-realm ownership. The frozen 68-path
  Test262 gate expands to 136 variants: 112 are admitted, 64 pass, and the
  remaining 72 stay at explicit unrelated-feature, parser, or harness
  frontiers. The focused TSV/JSONL SHA-256 values are
  `03def26414f02bf5056ebb1421a28d28178c29946b07fc8d0e085fdbb9bfe72b`
  and
  `b020aa4bd8cd878a8b96aa66b1736eee991df4fc87b6adda3510101a0a911fd8`.
  The complete vector moves to 25,959 passes and 33,283 admitted jobs. Its
  TSV/JSONL SHA-256 values are
  `5f0e4601ce6b0212dacdd5c98fc1ba4cb2c8c217e3f0eb6c91411ad6e3f243fa`
  and
  `a829007d38ffe4bd84b7420200b0fef505671808e1a003326c2fccb6383edcd6`.
  The exact R1i-to-R1j join has 66 `unsupported-feature -> pass`, 20
  `unsupported-feature -> unsupported-harness-parser`, 28
  `unsupported-feature -> unsupported-parser` transitions, with zero
  previous-pass regressions.

  R1k adds a QuickJS-shaped variable-length `BackReference` instruction whose
  boxed capture list is already compatible with future duplicate named
  captures. The parser caches a lexical total-capture prepass, consumes the
  complete decimal number, accepts forward references, and otherwise replays
  the source through Annex B octal/identity rules. The non-recursive executor
  compares through bounded UTF-16/code-point cursors, commits position only
  after a complete match, applies scoped `i` at the reference site, and treats
  forward, self, unmatched, and empty captures as zero-length success.

  Two pinned QuickJS differential groups cover successful matches, syntax
  errors, capture reset/backtracking, scoped and Unicode case folding, surrogate
  boundaries, complete-number priority, and Annex B widths. The static
  49-path/98-variant Test262 gate admitted 92 variants before named groups.
  R1l resolves its four
  linked lookahead variants and R1o resolves fourteen linked lookbehind
  variants. R1p admits the final six: two Annex B cases pass and four
  match/reference cases reach the existing lexical-destructuring parser
  frontier. The current gate therefore has 94 passes and four typed parser
  frontiers. Its TSV/JSONL SHA-256 values are
  `06ac527e434ebe7b7b7ed0e50193a716ebedc9d4b0fa028c5b1e3f87a0458268`
  and
  `02b8672453b17e260ea13e28f74bb9aea04caaccd3f274232e525f2e5fb6bb33`.

  The complete vector moves to 26,027 passes and 33,287 admitted jobs. Its
  exact R1j-to-R1k outcome delta is 62 `unsupported-parser -> pass`, two
  `unsupported-runtime -> pass`, and four
  `unsupported-negative-provenance -> pass`, with no other category movement
  and no previous-pass regression. The full TSV/JSONL SHA-256 values are
  `0bdf4955b2a9060279d0ad4232f653adb2018e9864654148f068caf22c0aabd6`
  and
  `7fcfbcd8157fa1d21d52af7df7e3b2226db7be08bfe42254994a28d56a5b9857`.
  Auditing the two Unicode decimal-escape negative paths moves the profile to
  103 exact negatives with SHA-256
  `6f27d9fcfa5a13423796ad48fe8ccbf8d5edcd49118ad7f0f64cc5a936090645`.

  R1l follows QuickJS's paired `lookahead`/`lookahead_match` opcode shape with
  typed Split/positive/negative control frames rather than recursive
  sub-execution. Positive completion compacts capture/register undo entries
  into the surviving outer transaction, preserving assertion atomicity while
  still allowing an outer alternative to roll those writes back. Negative
  completion never leaks body state. Thirty-one execution vectors and eight
  grammar vectors match pinned QuickJS, including nested assertions, scoped
  modifiers, astral input, capture/backreference interaction, and every
  Annex B/Unicode quantifier boundary.

  The static 26-path/52-variant lookahead gate now passes all 52 variants. Its
  TSV/JSONL SHA-256 values are
  `87bd4bf3ef361c063779f46c04d332349ec0c376d120cb854523c860cc32280e`
  and
  `ba716c99a6a95dc3a9bb1847bee65447de845aebc6e28c4ac69ce891c5bba024`.
  The complete vector moves to 26,079 passes while admitted jobs remain
  33,287. The exact R1k-to-R1l delta is 50
  `unsupported-parser -> pass` and two `unsupported-runtime -> pass`, with no
  other category movement or previous-pass regression. Full TSV/JSONL
  SHA-256 values are
  `9a60ea477bb8d383b316b9418683865031b43b3609400d7bcacb448cb535a85b`
  and
  `b69f3de1d2e61d3cb7667e6de1ffe2f5a811569df83b1cf34929008aaf8e393a`.

  R1m materializes Unicode 17 property sets as generated Rust half-open
  ranges: 38 General_Category values, 176 Script values, 176
  Script_Extensions values, and the 55 binary properties accepted by pinned
  QuickJS. Thirty-seven execution vectors and 28 grammar/error vectors match
  the oracle, including exact aliases, lone surrogates, astral input, scoped
  modifiers, the upstream empty-`=` quirk, and class-range error priority.
  Product builds do not link C; the checksum-pinned C helper is test-only and
  the parity gate regenerates and compares the Rust tables.

  The static 148-path/296-variant Unicode-property gate passes all 296
  variants. Its TSV/JSONL SHA-256 values are
  `66a129065346b23b454c6275b15301508bc8a4afaf6dacd8a473d6a948b7c392`
  and
  `87b704d71d7d8e33403abd81445cfd302c136fc2de30308c7f7caf9ceed9d869`.
  The complete vector reaches 26,377 passes and 34,457 admitted jobs. The
  exact R1l-to-R1m delta is 288 `unsupported-feature -> pass`, 882
  `unsupported-feature -> unsupported-harness-parser`, and ten
  `unsupported-parser -> pass`, with no other category movement or previous
  pass regression. Full TSV/JSONL SHA-256 values are
  `275fd8b3f6b1e5f078b6aad58bfc33797abaf6637179f47cc52228bc8f52feda`
  and
  `c2e14d42cfbb933946d9ce738d27c371e15fa3b9865131c2a6160cfe70b480f9`.

  R1n adds the QuickJS-exported `js_string_codePointRange` helper as a
  realm-bound, non-constructible native which the Test262 worker publishes
  under `$262`; it does not publish the remaining host hooks. The compiler
  reuses nested `ForOfStart`/`ForOfNext`/`IteratorClose` regions for
  identifier-only `const`/`let`/`var` array declaration patterns in
  synchronous for-in/of. Holes, empty and trailing patterns, early exhaustion,
  fresh lexical cells, and inner/outer abrupt-close precedence match pinned
  QuickJS. Assignment, object, default, rest, and nested patterns remain
  explicit typed frontiers. Normalized RegExp ranges now use binary membership
  lookup, so full-domain generated property tests do not multiply input length
  by the number of property intervals.

  The cumulative 589-path/1,178-variant Unicode-property gate passes every
  variant. Its TSV/JSONL SHA-256 values are
  `33e3da0a2ff60501fd68a838e80dbfced58551a27ceb5a96d51cb230b07e9488`
  and
  `3c75c5e8bbb3551554475e2eb8e1e8af053633456da5ee704f05589a2d508e6d`.
  The exact 102,037-key full join records 896
  `unsupported-harness-parser -> pass`, six
  `unsupported-harness-parser -> unsupported-parser`, 20
  `unsupported-parser -> pass`, six `unsupported-parser -> fail-runtime`, and
  two `unsupported-parser -> fail-parse` transitions. All 935 changed complete
  rows are inside the pre-audited 475-path set, with no previous-pass
  regression or outside-set drift. The vector reaches 27,293 passes while
  admitted jobs remain 34,457. Full TSV/JSONL SHA-256 values are
  `6035ae86888c4db9e99b73be65e706bf7b90ee83c108082a3e7931f2000edc61`
  and
  `fb37235d0d651a2d424cb4f63c16b6662813183f25fd2126e970bacb3506c50d`.

  R1o follows pinned QuickJS's backwards-direction compiler rather than adding
  a second matcher. Each lookbehind alternative retains source priority while
  its terms execute in reverse; consuming atoms use `Prev, op, Prev`, captures
  swap their saved boundaries, and participating numeric backreferences
  compare right-to-left without crossing their capture start. The existing
  non-recursive positive/negative assertion controls preserve atomicity,
  capture retention, rollback, and interruption behavior.

  Forty-two execution vectors and ten grammar vectors match pinned QuickJS.
  At the R1o landing, the frozen 27-path/54-variant gate passed the 50 variants
  owned solely by lookbehind and left four co-tagged named-group variants
  gated. R1p resolves those four, so the current gate passes all 54. Its
  current TSV/JSONL SHA-256 values are
  `590b466885fe087bc30cb02e1adc1b1076af0322e229a998af8cda3a680131dd`
  and
  `5aca0c7d11afea0d6c1facd893663ad2000f7a95860703112c641dd8a8fa914c`.
  The exact R1n/R1o full join matches all 102,037 keys: 34
  `unsupported-feature -> pass` and 16
  `unsupported-negative-provenance -> pass`, with 50 outcome changes, 54
  complete-row changes, no previous-pass regression, and no drift outside the
  frozen set. The vector reaches 27,343 passes and 34,507 admitted jobs. Full
  TSV/JSONL SHA-256 values are
  `50fe24e393c2532e2c25fc2113e6bbb48c163678a6bc8a0991f8c6ad0d8273c1`
  and
  `c997357b861109bfd17c46ad0c8059004f2b797cf9254394b90892dca078810b`.

  R1p stores normalized group names beside the pure Rust compiled program,
  excluding capture zero and without retaining realm/heap handles. Named
  references lower to the existing candidate-list backreference IR in both
  directions. A dedicated result builder publishes null-prototype `groups` and
  `indices.groups`; duplicate names retain their first property position while
  the last participating capture supplies the value. The direct replacement
  predicate follows QuickJS by declining named programs before mutation, so
  the generic path supplies functional-replacer groups and `$<name>`.

  Fifty-nine differential vectors plus a defining-realm test cover name
  grammar and diagnostics, escaped Unicode/surrogate pairs, Annex B fallback,
  forward references, QuickJS's 8-bit alternative-scope wrap and forward-scan
  cursor quirk, lookbehind references, result descriptors/order, indices,
  replacement, construction, copy, and legacy compile. At the R1p landing,
  the frozen 101-path/202-variant gate admits 184 variants and passes 158; its
  six parse failures and 20 typed parser frontiers expose pre-existing arrow,
  class, object-method, and destructuring gaps, while 18 variants retain
  honest adjacent gates. R1p focused TSV/JSONL hashes are
  `505845ba54ec78ae1a636f91f7285e447444d3ffca8b66a03592591573a15d26`
  and
  `5daec58cf49af34cdf2ad8e70d5a945513e6490180ab4c74e9e996f39d4fa234`.

  The exact R1o/R1p join matches all 102,037 keys. It records 158
  `unsupported-feature -> pass`, six `unsupported-feature -> fail-parse`, 20
  `unsupported-feature -> unsupported-parser`, two
  `unsupported-parser -> pass`, and two `unsupported-runtime -> pass`
  transitions. There are 188 outcome and 204 complete-row changes, no
  previous-pass regression, and only four linked `\k` canaries outside the
  focused manifest. The vector reaches 27,505 passes and 34,691 admitted jobs.
  Full TSV/JSONL hashes are
  `ff31a5f63b2b9e27f5650dd99c301cbff9c863314cce48e592f97b6ca1df2704`
  and
  `e1766ea22ab3e33ef610310a6d83ce101eb66dcfa598d581ebaed257295e9402`.
  The engine changes stay in `src/regexp/` and
  `runtime/intrinsics/regexp/result.rs`; `runtime.rs` remains 9,677 lines.

  R1q's source audit confirms that R1p already mirrors pinned QuickJS's global
  wrapping 8-bit duplicate-name scope, including its nested-alternative leak,
  multi-capture backreference selection, capture reset, result ordering, and
  defined-value replacement behavior. No production engine change is needed.
  The frozen 19-path/38-variant duplicate-name gate admits 32 variants and
  passes 26 at the R1q landing. Six variants in three callback-heavy tests
  reach the existing arrow parser frontier; the six co-tagged match-indices
  variants remain gated in that historical report and are admitted by R1r.
  Focused TSV/JSONL hashes are
  `bd55aacd10c14cf1f0f7a38e11a610ad3763bce8c4f326c9a6ae3ad548a8ef30`
  and
  `1b9dc971d9c965910b7e0bd88573e80553d17b74651c0ef4762dd34d998cc666`.

  The exact R1p/R1q join matches all 102,037 keys. It records 26
  `unsupported-feature -> pass` and six
  `unsupported-feature -> fail-parse` transitions. All 32 outcome changes and
  38 complete-row changes are inside the frozen manifest, with no
  previous-pass regression. The vector reaches 27,531 passes and 34,723
  admitted jobs. Full TSV/JSONL hashes are
  `16759de6e768905a3feae8dc96889936668838f42b64217bd70776cb6e56db96`
  and
  `36b947828eda57d0216d84e623b6af51143d26586860db3639cc3875765fc7e0`.
  The profile now contains 27 reviewed features and 307 audited negative
  paths, with SHA-256
  `8b78e178e2c433f5c9f40b101482a74cb3c5dc61967aa9ab9ee523479e132aa8`.
  `runtime.rs` remains 9,677 lines.

  R1r audits and declares `regexp-match-indices` after pinned QuickJS source
  review and focused probes confirm that the existing production engine
  already matches the target's `d` flag and canonical flag order,
  `hasIndices`, UTF-16 match ranges, unmatched-capture `undefined` values,
  null-prototype named `indices.groups`, duplicate-name selection,
  construction/legacy-compile behavior, and observable descriptors. No
  production engine change is needed. Seven dedicated differential tests lock
  result/pair descriptors, low-surrogate `lastIndex`, protocol propagation,
  replacement non-observation, and nested defining realms against the pinned
  oracle.

  At the R1r landing, the frozen 31-path/62-variant gate admits 50 variants and
  passes 38. Two variants expose the existing arrow-function parse frontier,
  four stop in the existing `deepEqual.js` harness frontier, and six reach the
  typed object-setter parser frontier. Ten variants remain behind the
  independently gated `regexp-dotall` feature in that historical report and
  are admitted by R1s, while two retain the missing `$262.createRealm` host
  requirement. Focused TSV/JSONL hashes are
  `b626f453c4a22402c9bf35f0b6a95ad3cf54cb2095ff21c023a150ec6904a230`
  and
  `edc7cb06eb9d18596202ae4d6f9faa4e56c1e2c4a6a81b51a54a26b0b34cd31f`.

  The exact R1q/R1r join matches all 102,037 keys. It records 38
  `unsupported-feature -> pass`, two `unsupported-feature -> fail-parse`, four
  `unsupported-feature -> harness-error`, and six `unsupported-feature ->
  unsupported-parser` transitions. All 50 outcome changes and ten detail-only
  changes stay inside the focused manifest, for 60 complete-row changes and no
  previous-pass regression. The vector reaches 27,569 passes and 34,773
  admitted jobs. Full TSV/JSONL hashes are
  `e09478accaf05c27e39555c5a4c1889617c97ce5c1454ddf945c7f675ea3d2ef`
  and
  `95ea74491558035ac02af4f60c3a2d202120798fc2ab08c41c7050a6031e950b`.
  The profile now contains 28 reviewed features and 307 audited negative
  paths, with SHA-256
  `b39bee15a2aaa88e00c8f7ca6cb0736313456d43a77e176a8c5cf7844e9ea718`.
  `runtime.rs` remains 9,677 lines.

  R1s audits and declares `regexp-dotall` after pinned QuickJS source review
  and focused probes confirm that the existing Rust path already matches the
  target. The `s` flag uses QuickJS's bit, selects the all-character
  instruction instead of ordinary dot, and shares the executor's exact UTF-16
  and Unicode width. Scoped modifiers restore their enclosing state, while
  literals, construction, legacy `compile`, accessors, canonical flags,
  protocols, species-created matchers, and defining-realm brand checks retain
  dotAll semantics. No production engine change is needed. Six dedicated
  differential tests lock the oracle vectors, matching and UTF-16 state,
  public/construction surface, nested scoped modifiers, matchAll/split species
  flags, and cross-realm getter brands and error realms.

  At R1s the frozen 17-path/34-variant gate admitted 26 variants and passed 18,
  with Arrow, accessor, `u180e`, `regexp-v-flag`, and create-realm frontiers
  explicit. Later slices resolve Arrow and `u180e`; R2g resolves the final four
  accessor consumers. The current gate admits and passes 30 variants, while
  two remain behind `regexp-v-flag` and two retain the missing
  `$262.createRealm` host requirement. Its exact summary is
  `pass=30 unsupported-feature=2 unsupported-host-create-realm=2`. Focused
  TSV/JSONL hashes are
  `3d5bda20dece92150f0398cb6f2d70a4114ff46fea69c7326ef056e439c7e246`
  and
  `a584c2db7b136338cb5ea9ca5116572f17ce2121740b5670889ab035e979bd23`.

  The exact R1r/R1s join matches all 102,037 keys. It records 18
  `unsupported-feature -> pass`, four `unsupported-feature -> fail-parse`, and
  four `unsupported-feature -> unsupported-parser` transitions. All 26
  outcome changes and six detail-only changes are inside the frozen manifest,
  for 32 complete-row changes and no previous-pass regression. The vector
  reaches 27,587 passes and 34,799 admitted jobs. Full TSV/JSONL hashes are
  `44f7ee3d6de6c97962c4b372da2f492882b8834d76663b334dd46265fae9e69f`
  and
  `fa263cbcd0483000f0645f017d486e4a4403d5227b97ce3bf5e812bf8a6857ce`.
  The profile now contains 29 reviewed features and 307 audited negative
  paths, with SHA-256
  `84fe6615092829a107e66beb49ac54b00a1910616424494f47e5f75c8ccc7880`.
  The admission and differential locks add no production code; `runtime.rs`
  remains 9,677 lines.

  R1t audits U+180E against pinned QuickJS at the lexer, numeric-conversion,
  trimming, Final Sigma, and RegExp layers. Both engines treat it as ordinary
  format content rather than ECMAScript whitespace: raw token separation is a
  SyntaxError, comments and literals preserve it, Number rejects it,
  prefix-number parsers stop at it, trim does not cross it, lowercase skips it
  as Case_Ignorable, and `\s` excludes it while dot and `\S` match it. Seven
  dedicated differential tests lock those boundaries. No production engine
  change is needed; global `eval` and JSON remain independent subsystem
  frontiers rather than U+180E exceptions.

  The complete 25-path/50-variant focused gate is fully admitted and passes 40.
  Its ten runtime failures are the five `*-eval.js` paths in sloppy and strict
  mode: four pairs report the missing global `eval` ReferenceError, while the
  whitespace pair correctly records Test262's resulting assertion error. The
  single parse-negative path is separately provenance-audited and passes as a
  real lexer-originated SyntaxError. Focused TSV/JSONL hashes are
  `3e42dd0c0e7272d51f02a03f95c1d907218b9f3ee5e29a20c0c6760565fbaf0c`
  and
  `4d6e6d514c9a4e6108f828b57b53507e24564df2d0a670a31132a878dbbc8d5c`.

  The exact R1s/R1t join matches all 102,037 keys. It records 40
  `unsupported-feature -> pass` and ten `unsupported-feature -> fail-runtime`
  transitions. All 50 outcome and complete-row changes stay inside the frozen
  manifest, with no detail-only changes or previous-pass regression. The
  vector reaches 27,627 passes and 34,849 admitted jobs. Full TSV/JSONL hashes
  are
  `7ea006b596e26f56712c9618f74cd8a5af9aada88702d08f855e6bc8eb313424`
  and
  `6d1d42c46ff6ff145dd72890c90abf6047d11910545599186e5f285028a21fc4`.
  The profile now contains 30 reviewed features and 308 audited negative
  paths, with SHA-256
  `3c5dee6fa18c428a45556488873ab216dd99e9f8859875ce2e4d1475d307aca6`.
  The milestone adds no production code; `runtime.rs` remains 9,677 lines.

  R1u adds the global `%eval%` callable without pretending that String source
  execution is complete. Pinned QuickJS source and differential probes lock
  `name`, `length`, property flags, lack of `prototype`, non-constructability,
  no-argument `undefined`, non-String identity without coercion, held aliases
  after global deletion/replacement, and cross-realm calls. Each realm also
  retains its original callable independently of the writable/configurable
  global property, matching QuickJS's `JSContext.eval_obj`; that root is the
  identity gate required by the future direct-eval opcode. Primitive Strings
  return the engine-level `Unsupported` error
  `eval source execution is not implemented yet`, which JavaScript
  `try`/`catch` cannot misclassify as a language exception.

  The frozen positive gate contains all 31 paths and 55 variants that move to
  pass because of this shell, and passes 55/55. Its manifest SHA-256 is
  `ae398ca6148d5babf468e7ba1cdcf956f454d35cdb6f612a3c4444d2b3c97cea`;
  focused TSV/JSONL hashes are
  `9d364c24169423efa49ecfa384c86280f94011b430fa787f72a8214fe867a6f6`
  and
  `63d5717d85f57c19705196aee0333c18cc270242b37e431622a035a8c34cf2fd`.
  This is the complete positive transition surface, not a claim that String
  eval or direct eval is implemented.

  The exact R1t/R1u join matches all 102,037 keys with no additions, removals,
  detail-only changes, or duplicate rows. It records 55
  `fail-runtime -> pass`, 1,448 `fail-runtime -> unsupported-runtime`, and 41
  `pass -> unsupported-runtime` transitions. The latter are fully audited
  missing-eval false positives: 31 variants had mistaken the outer
  “`eval` is not defined” `ReferenceError` for an expected source-thrown
  `ReferenceError`, and ten had swallowed that same error with a broad catch
  before asserting untouched state. The vector therefore reaches 27,641
  passes and keeps 34,849 admitted jobs. Full TSV/JSONL hashes are
  `59736a4a4f63122a458a33374d2afd873a706aeb7ff271b52f9fa4aa2aa71fbe`
  and
  `c4849aecc54afcc7c73bb182cd240bc9cf35634bc74bc4d5558d6951898af2f2`.
  The capability profile remains byte-identical at
  `3c5dee6fa18c428a45556488873ab216dd99e9f8859875ce2e4d1475d307aca6`.
  Eval code lives in `runtime/intrinsics/eval.rs`; bootstrap wiring adds only
  two lines to `runtime.rs`, now 9,679 lines.

  R1v then establishes the direct-eval bytecode and realm-identity path while
  deliberately keeping the same String-source frontier. Compiler tests prove
  that `eval(x)`, `(eval)(x)`, nested parentheses, escaped spelling, and a
  local binding named `eval` publish `Eval`, while composed values, aliases,
  properties, `.call`, conditionals, assignments, and `new` do not. VM tests
  lock the `(argc + 1) -> 1` stack contract, first-argument-only original path,
  complete-argument replacement fallback, and undefined receiver. Runtime
  tests prove the cached identity is realm-local and survives deletion or
  replacement of the global property. The parser IR retains the exact
  call-site scope for the future immutable eval-environment descriptor; a raw
  `ScopeId` is intentionally not exposed in verified bytecode.

  This is a zero-scoreboard-movement architecture milestone. The eval-focused
  TSV/JSONL remain byte-identical at
  `9d364c24169423efa49ecfa384c86280f94011b430fa787f72a8214fe867a6f6`
  and
  `63d5717d85f57c19705196aee0333c18cc270242b37e431622a035a8c34cf2fd`;
  the full TSV/JSONL remain byte-identical at
  `59736a4a4f63122a458a33374d2afd873a706aeb7ff271b52f9fa4aa2aa71fbe`
  and
  `c4849aecc54afcc7c73bb182cd240bc9cf35634bc74bc4d5558d6951898af2f2`.
  There are zero outcome, complete-row, detail-only, key, or pass-count
  changes. `runtime.rs` remains 9,679 lines; all new eval behavior stays in
  the compiler, typed VM boundary, and `runtime/intrinsics/eval.rs`.

  R1w replaces the retained parser scope with a published immutable
  `EvalEnvironment` table. Each descriptor is ordered inner-to-outer and
  segmented by function roots: the current function contributes only exact
  Local/Argument definitions, while every ancestor contributes named Closure
  relays through its definition scope, ending at the script Program body.
  Repeated eval sites in the same scope share one descriptor. Ordinary relays
  allocated by earlier identifier resolution are upgraded in place to retain
  their semantic name, including under StripDebug, without changing the
  closure slot or VarRef identity. Eval-visible locals join the existing
  capture analysis and block-lifetime `CloseLocal` path.

  The publication boundary now checks that descriptor count matches exact
  function-tree depth, every segment has the QuickJS-shaped Body/Root
  topology, current versus ancestor source kinds cannot cross, all indices and
  flags match authoritative definitions, and named ParentClosure relays trace
  back to a same-name local or argument rather than a disguised global. Each
  eval-name atom owns one bytecode metadata reference with exact multiplicity.
  The VM performs a two-phase validation before capturing any frame cell, then
  materializes Local/Argument cells and clones existing Closure VarRefs only
  for primitive String input. Non-String input stays fully lazy and returns
  the original value; String input still ends at the exact typed Unsupported
  boundary after the environment has been materialized.

  Pinned QuickJS environment probes cover sloppy/strict direct and indirect
  eval, lexical/var declarations and conflicts, `this`, `arguments`, and
  `new.target`. The focused report remains 55/55 with TSV/JSONL SHA-256
  `9d364c24169423efa49ecfa384c86280f94011b430fa787f72a8214fe867a6f6`
  and
  `63d5717d85f57c19705196aee0333c18cc270242b37e431622a035a8c34cf2fd`;
  the complete report remains 27,641/102,037 with 34,849 runnable jobs and
  hashes
  `59736a4a4f63122a458a33374d2afd873a706aeb7ff271b52f9fa4aa2aa71fbe`
  and
  `c4849aecc54afcc7c73bb182cd240bc9cf35634bc74bc4d5558d6951898af2f2`.
  `runtime.rs` is 9,692 lines, only 13 above R1v; publication logic lives in
  `runtime/bytecode_publish.rs`, and frame integration lives in
  `runtime/vm_host.rs`.

  Opening String execution requires three explicit follow-ups from the pinned
  QuickJS audit: a persistent sloppy dynamic variable environment for newly
  introduced `var` bindings, an explicit defining realm at the eval runtime
  boundary, and an EvalRoot publication mode (or equivalent synthetic parent)
  for compiled eval bytecode. Exact per-block descriptor provenance and an
  owned bytecode root are also required before environments may escape the
  current synchronous call. R1w does not claim any of those later semantics.

  Advanced grammar still fails closed: Unicode set/string properties, all
  `v`-mode execution, and unported Annex-B control escapes return typed
  unsupported errors. Pattern group nesting is temporarily capped at 256 with
  a catchable `stack overflow`
  compile error so adversarial input cannot overflow the Rust stack; a later
  iterative parser/compiler must replace that conservative resource frontier
  before the runtime surface is exposed as complete.
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

  Source lexical population now covers simple-name and flat-array `let` and
  `const` lists in the direct Program global lexical environment plus four
  local authored environments: an ordinary function body (including a normal
  `%Function%` constructor body), every non-empty nested brace block, the one
  CaseBlock scope shared by every clause of a `switch`, and the initializer
  scope of a classic `for (;;)` loop. Block, switch, and classic-for locals also
  work in scripts.
  A simple-name `let` without an initializer performs explicit `undefined`
  initialization, while `const` and flat array patterns require an initializer.
  Array patterns accept identifier leaves, empty/elided/trailing elements,
  undefined-only defaults, and terminal rest; anonymous function initializers
  retain contextual NamedEvaluation. Registration occurs before each
  initializer is parsed; duplicate names are rejected within one lexical scope,
  all switch clauses participate in the same duplicate check, and
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

  Script `var` names from simple declarations and flat array patterns use the
  same production global-declaration path in Program bodies, blocks,
  `if`/`switch` statements, and classic `for (;;)` heads. They never consume a
  script-frame local. Matching QuickJS,
  the compiler keeps two related structures: one canonical global binding with
  the first declaration's scope for parser conflict lookup, and one ordered
  declaration record for every bound identifier, including duplicates. Each
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
  then performs both global writes. Direct and indirect eval roots now use the
  same ProgramBody exception through their dedicated declaration environments;
  strict direct eval instead keeps its declarations local. The global-path
  duplicate write is observable
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
  in QuickJS 2026-06-04. Async/generator eval declarations, `for-await`,
  destructuring assignment, object/nested binding patterns, destructuring in
  parameters or catch bindings, single-statement lexical declarations, and
  class scopes remain explicit boundaries rather than falling back to local or
  ordinary global storage.

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
  Script-wide simple-name or flat-array `var`, direct Program simple-name or
  flat-array `let`/`const`, and the body/block/switch/classic-for-head lexical
  slice above,
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
  function-local `var` declaration path. Its simple-name and flat-array lexical
  declarations use the same NoIn initializer grammar, conflict registration,
  TDZ, NamedEvaluation, closure, read-only, and StripDebug paths in scripts,
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
  every declared name is therefore in TDZ across every clause,
  duplicate declarations conflict across cases, and normal or abrupt exits
  close captured cells while preserving selector cleanup. This declaration
  slice covers simple names and flat array patterns, not complete
  SwitchStatement parity.
  Synchronous `for-of` follows QuickJS `js_parse_for_in_of` for
  `var`/`let`/`const`, identifier, fixed-member and computed-member targets.
  The assignment fragment is emitted before the iterable expression and
  skipped on first entry; the head lexical environment is therefore already
  in TDZ while evaluating the iterable. Captured lexical head cells close at
  the pinned per-iteration boundary. Local and labelled continue retain the
  active iterator, while edges crossing an iterator control close it in
  inner-to-outer order and interleave correctly with switch cleanup and
  try/finally subroutines. Flat array declaration patterns reuse a nested
  iterator record with QuickJS close semantics and accept identifier leaves,
  empty/elided/trailing elements, undefined-only defaults, and terminal rest.
  Destructuring assignment, object/nested patterns, patterns in parameters or
  catch bindings, and `for-await-of` remain explicit frontiers. The classic
  head continues to port QuickJS's
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
  has no destructuring exclude list. Synchronous simple-parameter concise
  methods use dedicated fixed/computed define-method operations. Computed keys
  reuse the canonical property key without a second observable conversion;
  methods receive QuickJS-compatible inferred names and C/W/E data descriptors,
  own dynamic `this`/`arguments`/`new.target`/direct-eval environments, and stay
  callable but non-constructible without a `prototype`. Contextual `get`, `set`
  and `async` remain ordinary names before `(`, while `__proto__()` is an
  ordinary own data property. Synchronous getters and simple-parameter setters
  use the same fixed/computed define-method path, including one-time computed
  key conversion, descriptor-half pairing/replacement, data/accessor
  conversion, inferred names, and non-constructability. Async/generator
  methods and non-simple parameters remain explicit frontiers. Synchronous
  methods/accessors that directly reference `super` carry a retained
  HomeObject and use its live prototype with the current method receiver. The
  pinned getter-call exception first invokes an accessor with the frozen super
  base, then calls its result with the method receiver. Reads, calls, writes,
  updates, deletion errors, and loop assignment targets
  share dedicated verified bytecode/VM helpers. Synchronous arrows nested in a
  method or accessor inherit its lexical receiver and HomeObject through
  authenticated closure slots. Synchronous direct eval inherits the exact
  `super_call_allowed`/`super_allowed` capability pair, including nested eval and
  authored or eval-created Arrow relays; ordinary functions, global code, and
  indirect eval cut that capability off. Parameter initializers, classes/derived
  construction, async/generator methods, and Proxy/exotic-source spread remain
  explicit frontiers. The pinned anchors are
  `quickjs.c` 24485-24621 and
  24850-24965 plus the matching object/define/name/proto/copy opcodes in
  `quickjs-opcode.h`; `oracle_object_literals` locks the data-property/spread
  slice, `oracle_object_methods` locks the concise-method slice, and
  `oracle_object_accessors` locks the getter/setter slice against QuickJS
  2026-06-04; `oracle_object_super` locks the direct HomeObject/SuperProperty
  slice, `oracle_object_super_arrow` locks the lexical-arrow relay, and
  `oracle_object_super_eval` locks direct-eval inheritance and its cutoffs.

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
  UTF-16/search methods, generic `search` and `split`, the
  `substring`/`substr`/`slice` subrange trio, `repeat`, the
  `padEnd`/`padStart` pair, the five-property trim group, the conversion pair,
  the four Unicode case-conversion methods,
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
  deterministic eight-active-call guard keeps recursive callbacks catchable as
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
  direct and interleaved getter/key-coercion recursion catchable. Strong Map
  entry iteration and Set values which are themselves entry objects now run in
  the pinned differential. Proxy entry traps, generators/finally, TypedArrays
  and module namespace entries remain explicit boundaries until those object
  kinds exist.
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
  `lastIndexOf`, `includes`, `endsWith` and `startsWith`, publishes `match`,
  `matchAll`, `search` then `split`, then publishes
  `substring`, `substr`, `slice`, `repeat`, `replace` and `replaceAll`, and
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
  clamps position with `JS_ToInt32Clamp`, and scans UTF-16 code units. The heap
  now has a genuine RegExp payload and the internal-brand fallback recognizes
  only that class; the R1a realm graph and constructor now make that branded
  path observable. The generic `match` and `search` callables each have
  `length=1` and share an isolated protocol helper. Each performs object-only
  delegation through its matching well-known Symbol before receiver conversion,
  otherwise converts the receiver and uses the defining realm's canonical
  RegExp constructor plus the newly constructed object's dynamic protocol
  hook. Neither observes a replacement global `RegExp`, while
  retained-constructor and prototype mutations stay visible. The generic
  `split` callable has `length=2` and ports pinned QuickJS `js_string_split`.
  Nullish receivers are rejected before any separator access. Only object
  separators perform the ordinary `Symbol.split` Get; a present non-nullish
  method is called with the separator as `this`, the original unconverted
  receiver and limit, and exactly two arguments. Primitive separators never
  consult their boxed prototypes. The fallback path converts the receiver,
  allocates its result Array in the method's defining realm, converts a present
  limit with ToUint32, and converts the separator even when the resulting limit
  is zero. Undefined separators, empty sources and separators, repeated
  matches, tails, astral pairs and lone surrogates follow the pinned raw UTF-16
  code-unit loop; indexed results use CreateDataProperty and update Array
  length. Native errors use the defining realm while getter, custom-splitter
  and conversion throws retain their original identity and realm. The AutoInit
  graph, deletion/replacement, coercion and abrupt order, limit boundaries,
  cross-realm results/errors, recursion recovery, detached-callable lifetime
  and final GC are locked by nine passing
  oracle/differential/white-box integration tests plus five intrinsic unit
  tests. The pinned anchors are `quickjs.c` 45894-45980 and 46640.

  At the generic-split landing, the 127-path focused Test262 vector had 186
  passes out of 254 variants and deliberately exposed the still-unimplemented
  RegExp protocol. R1e wires `RegExp.prototype[Symbol.split]`; the same frozen
  vector reached 234 passes, four independent missing-global-`eval` runtime
  failures, eight adjacent feature outcomes, two IsHTMLDDA host outcomes and
  six typed parser frontiers. R1p resolves two Annex B `\k` variants, R1x
  executes the two eval consumers, R2c resolves the Arrow consumers, and R2f
  resolves the six concise-method consumers. The current vector admits and
  passes 248 variants with no parser frontier. Declaring
  `Symbol.split` in the conservative
  capability profile originally meant only that the well-known symbol and
  generic/custom delegation were audited; R1e completes the currently
  published RegExp side without changing that 18-tag profile. The three
  distinct generic
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
  46/53 own keys. This forty-six-key list is only the QuickJS-relative order
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
  The remaining two String-prototype own keys (`normalize` and `localeCompare`),
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
- The observable `%Date%` intrinsic now follows the pinned QuickJS
  2026-06-04 implementation at `quickjs.c` 47223-47279 and 54786-55939. The
  heap has a genuine edge-free Date payload with mutable binary64 milliseconds,
  exact invalid-Date NaN branding, exhaustive class/payload validation and a
  dedicated realm-root slot and GC edge for QuickJS's ordinary, unbranded
  `%Date.prototype%` (its value methods therefore reject that prototype).
  Pure modules port the proleptic Gregorian calendar and TimeClip/MakeDate
  evaluation order, the ISO-first and legacy 127-code-unit parser, and all
  eight UTC/local/fixed-locale formatter modes including extended years, GMT
  offsets and Invalid Date behavior. A runtime-owned injectable host boundary
  supplies `SystemTime` and
  JavaScript-sign timezone offsets through `tz-rs`; an unset `TZ` reloads the
  host local-zone configuration instead of freezing the first `/etc/localtime`
  snapshot. The exact constructor/static table, ordinary unbranded prototype,
  47-entry source table, `toGMTString` callable alias, getters, setters,
  generic `toJSON`, and forced-ordinary `@@toPrimitive` are now published with
  their pinned names, lengths, descriptor flags, key order, coercion order,
  TimeClip boundaries, new-target realm fallback, and error behavior. The Date
  implementation lives outside the runtime facade and enters native dispatch
  through one typed handler family.

  Forty-four Date unit tests, six grouped QuickJS differentials, one oracle
  vector self-check, and two cross-realm/GC integration tests pass. With
  generic split and RegExp R1a linked, the 799-path focused Test262 vector has
  1,290 passes out of 1,598 variants. At the Date landing, the exact
  complete-vector join
  moved 21,740 to 23,016 passes through 1,276 `fail-runtime -> pass`
  transitions with no previous-pass regression and no change outside the
  manifest. The Date-landing five- and eight-worker focused and full reports
  are byte-identical. One host parity limitation remains explicit: on Windows,
  both an unset `TZ` and an IANA-zone `TZ` currently fall back to UTC because
  `tz-rs` has no native local-zone/zoneinfo backend there. A real
  cross-platform local-time backend is still required before full QuickJS
  feature parity can be claimed.
- Every realm publishes the pinned `%JSON%` namespace in `isRawJSON`, `parse`,
  `rawJSON`, `stringify`, then `@@toStringTag` order. The implementation follows
  `quickjs.c` 49257-50181 with a dedicated strict UTF-16 parser and parse-record
  tree, post-order reviver walk and exact primitive source contexts, ordered
  stringify transform/traversal, well-formed UTF-16 quoting, and a runtime-wide
  Raw JSON heap brand. Raw objects have a null prototype, one frozen enumerable
  source slot, and no duplicate payload edge; brand checks invoke no user code,
  while stringify splices only the internally validated exact source. The
  parser, reviver, Raw JSON, and stringify owners remain separate modules under
  `runtime/intrinsics/json/`. Dedicated QuickJS differentials cover the graph,
  descriptors, coercion and callback order, mutation snapshots, duplicate
  keys, cycles, raw UTF-16, cross-realm ownership, and Raw JSON branding.
- Every realm publishes a genuine strong `%Map%` constructor, ordinary
  `%Map.prototype%`, and realm-local `%MapIteratorPrototype%`. The dedicated
  intrinsic module follows the pinned constructor/adder/iterator-close order,
  `SameValueZero` keys, negative-zero normalization, live mutation behavior,
  callback reentrancy, exact descriptors and aliases, get-or-insert methods,
  species, tags, and `Map.groupBy`. Heap records own object and Symbol edges;
  iterator exhaustion releases its source, and the realm roots only the class
  prototypes rather than keeping the public constructor artificially alive.
  The current stable-vector representation deliberately retains tombstones and
  uses linear lookup. That preserves the tested observable semantics but does
  not yet match QuickJS's hash lookup and reclaimable zombie records, so long
  delete histories remain an explicit resource-parity frontier.
- Every realm also publishes an independently branded strong `%Set%`
  constructor, ordinary `%Set.prototype%`, and realm-local
  `%SetIteratorPrototype%`. The dedicated intrinsic implements constructor
  closing, ordered `SameValueZero` membership, live iteration, callback
  mutation, exact aliases and descriptors, all seven set-composition methods,
  species, tags, and `Set.groupBy`. Set-like operands are observed in the
  pinned `size`/`has`/`keys` order, while Set-producing methods allocate a base
  Set in their defining realm without consulting subclass species or an
  overridden `add`. Heap records own object and Symbol edges, exhausted
  iterators release their source, and roots follow the same constructor-lifetime
  discipline as Map. The shared stable-vector kernel still retains tombstones
  permanently and uses linear lookup rather than QuickJS's hash lookup and
  reclaimable zombie records; observable semantics are locked, but long
  deletion histories remain a resource- and complexity-parity frontier.
- The global object has QuickJS's dedicated payload and hidden
  `uninitialized_vars` object. Global data properties and the lexical-binding
  object can store `PropertySlot::VarRef` cells; define, descriptor lookup,
  assignment, accessor conversion and delete preserve shared-cell identity.
  Deleting or converting a global property moves a still-referenced cell back
  to the hidden object, resets it to Uninitialized, and allows a later data
  definition to reconnect the same closures. These VarRef, hidden-object,
  Shape and atom edges participate in reference counting and trial-deletion GC.
  Script-wide simple-name or flat-array `var` and direct Program simple-name or
  flat-array `let`/`const` now drive this substrate through production
  declaration instantiation rather than test-only helpers. The
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
  `Boolean`, `String`, `Math`, `Reflect`, `Symbol`, `globalThis`, `BigInt`,
  `Date`, `RegExp`, `JSON`, `Map`, then `Set`. This is not a claim that the wider
  global builtin table is complete.
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
  primitive receiver. String lookup exposes the implemented 48-key prototype
  surface described above together with user-defined prototype properties;
  the remaining five standard entries are absent.
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
  pinned continue behavior, Program-global declaration descriptors, flat array
  binding declarations, explicit nonclassic-for and remaining destructuring
  boundaries, and strip-debug name removal without losing read-only atoms.
  The companion `oracle_function_body_lexicals` target compares ordinary and
  normal-`Function` body/block/switch values, nested script block/switch locals,
  direct and transitive closure cells, repeated-entry and break/continue scope
  exits, cross-case TDZ/conflicts, TDZ/read-only CLI stacks, flat array binding
  declarations, and the remaining object/nested-pattern boundary with the
  pinned release.
  The `oracle_for_lexicals` target separately locks classic-head initialization
  and NoIn parsing, ordinary and normal-`Function` values, script-local and
  cross-eval captures, initializer/body/update cell identity, the pinned
  shared-head-cell continue quirk, labeled jumps through a nested switch,
  conflicts, exact full/StripDebug TDZ and read-only stacks, flat array binding
  declarations, and explicit object/nested destructuring boundaries.
  The `oracle_program_lexicals` target locks direct Program values, declaration
  source order, repeated eval persistence, globalThis separation and VarRef
  splitting, preflight atomicity, failed-initializer behavior, exact
  full/StripDebug stacks and parser errors, flat array binding declarations,
  and the still-explicit object/nested-pattern boundary.
  The `oracle_program_vars` target locks duplicate declaration records,
  no-initializer and unreachable-statement instantiation, classic-for shared
  cells, NamedEvaluation, cross-eval persistence and hidden-cell reconnection,
  exact global property attributes, data/accessor/AutoInit/inherited and
  non-extensible paths, mixed-declaration preflight atomicity, full/StripDebug
  stacks, parser conflicts, flat array binding declarations, and explicit
  object/nested-pattern and nonclassic-loop boundaries.
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
  The `oracle_for_of` target locks simple binding/reference heads, flat array
  declaration patterns, the generic iterator protocol and accessor order,
  Unicode String iteration, natural and abrupt close behavior, completion
  precedence, nested labels/switch/finally, raw
  native-next dispatch, realm splitting, exact diagnostics, and all three
  debug modes. Generic Array iteration is now covered by `oracle_array`;
  `for-await-of`, destructuring assignment, object/nested patterns, other
  binding contexts, and Iterator Helpers remain separate milestones.
  The `oracle_for_in` target locks ordinary and representation-sensitive fast
  Array enumeration, per-level snapshots, live presence/prototype changes,
  shadowing, primitive boxing, simple assignment plus simple-name/flat-array
  declaration heads, lexical cells, labels/finally cleanup, and exact
  initializer diagnostics.
- `Runtime` and `Context` are distinct; `qjs -e` and file execution use the
  Rust compiler/VM path and never delegate to an external engine.

## Not implemented yet

The complete pinned Test262 vector is now recorded conservatively. Remaining
parser frontiers with generic syntax diagnostics cannot contribute negative
test passes until they gain typed `Unsupported` provenance or are individually
audited as genuine early errors. The remaining native `$262` host hooks, module
parse/link/evaluate, Promises/jobs and async completion, the ES5.1 suite, and a
separate QuickJS-runner-quirk profile remain future milestones. Unsupported and
host-missing outcomes are failures, not additional feature skips.

The former default-libtest-stack gate debt is closed. QuickJS checks its real
platform stack pointer at both native and bytecode call boundaries; the
`unsafe`-free runtime now captures a safe address marker at the outermost call
and shares QuickJS's one-MiB byte budget across native and bytecode entries.
The measured ARM64 debug hot-opcode frame (71,024 bytes) is isolated behind an
`inline(never)` helper, so suspended `Call`/`Eval`/`Construct` instructions no
longer retain it for every callee. Explicit 2 MiB tests cover 32 finite
bytecode calls, ordinary recursion, recursive constructors, mixed
`Object.hasOwn`/`@@toPrimitive` reentry, and runtime recovery; the pinned
Sputnik 32-IIFE case also passes both normal-runner variants. This remains a
resource-parity approximation: the marker does not query the OS stack bound,
the conservative native-family budgets remain, and syntactically nested
parser/compiler work still uses host recursion. A complete execution
trampoline plus explicit compiler work storage is required to recover
upstream's substantially deeper platform-dependent limits throughout.

The language slice remains incomplete. Async/generator declarations,
`for-await`, destructuring assignment, object/nested binding patterns,
destructuring parameters and catch bindings, other general assignment targets,
module resolution,
async/generator methods, parameter-initializer inheritance of HomeObject/`super`,
classes and derived constructors, non-simple parameter lists including
ObjectLiteral setters, async and non-simple Arrow forms, and callable Proxy
classes are not yet
implemented.
Unsupported declaration contexts are rejected instead of being
faked as Program functions or ordinary vars. Source `let`/`const` supports
simple identifiers and flat array patterns in direct Program code, authored
ordinary-function bodies, non-empty nested brace blocks, shared switch scopes,
classic `for (;;)` heads, and synchronous `for-in`/`for-of` heads. Flat arrays
cover identifier leaves, empty/elided/trailing elements, undefined-only
defaults, and terminal rest. These forms also work in scripts, and ordinary
bodies including classic heads are available through the normal `%Function%`
constructor. Single-statement lexical declarations, object/nested loop-head
patterns, destructuring assignment, patterns in parameters or catch bindings,
and class lexical environments remain later compiler slices. Direct
Program lexicals now use the production global VarRef path with two-phase
instantiation; simple-name and flat-array Program vars and direct ordinary
function declarations use ordered, kind-specific global declaration records.
One internal resource-failure
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
first twelve generic code-unit/search methods, generic `match`, generic
`search`, generic `split`, the three
generic subrange methods, `repeat`, `padEnd`/`padStart`, the five-property trim
group,
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
Generic match/search adds object-only delegation through the selected
well-known Symbol, intrinsic RegExp fallback and dynamic invocation of the
constructed object's corresponding protocol method.
Generic split adds object-only `Symbol.split` delegation, raw receiver/limit
forwarding, ordered ordinary conversion and exact UTF-16 separator/tail output
in a defining-realm Array. R1e supplies the RegExp protocol side through a
defining-realm SpeciesConstructor, sticky clone, abstract RegExpExec and exact
capture/limit/UTF-16 advance loop.
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
publish the remaining three prototype own keys, Context/C pointer embedding
semantics, atom diagnostics belonging
to unimplemented language/builtin surfaces, exact byte-sidecar construction
for every parser/lexer diagnostic, or general recoverable allocator failures
outside the repeat/pad/trim/case/CreateHTML/replacement result-buffer
reservations. Rope
linearization and final `Box`/`Rc` allocation, including those surrounding the
checked trim, case, CreateHTML and replacement buffers, remain part of that
general allocator gap. Pad, case and CreateHTML widening use a second fallible
exact UTF-16 buffer and then release the narrow buffer, rather than preserving
QuickJS allocator/
realloc identity and peak-memory behavior.
Prefix/postfix update expressions
(including QuickJS's valid `++x ** 2` form) are implemented for the current
identifier and ordinary fixed/computed member References. Sloppy
direct-identifier delete is implemented
for the current static scope tree and defining-realm global object. Dynamic
object-environment lookup/deletion introduced by `with` or direct `eval`, the
remaining two entries of String's 53-key prototype surface, `RegExp.escape`,
advanced RegExp grammar,
Proxy/exotic internal methods, and the full
`function_accessors.js` fixture are still pending. AggregateError and
uncatchable termination state are also pending. Destructuring assignment,
object/nested binding patterns, parameter and catch patterns, other iterator
classes and helpers, the remaining RegExp grammar/static surface and
Unicode-backed String methods, non-simple ObjectLiteral setter parameters,
async/generator methods, parameter-initializer `super` inheritance,
classes/derived constructors, exotic-source spread, and the rest of
the builtin table build on those layers.

The remaining parity surface also includes the full grammar/opcode set, the
Unicode 17 normalization/script/property tables beyond the implemented
identifier, case-conversion, `Cased` and `Case_Ignorable` data, the advanced
RegExp grammar, modules, jobs/Promises/async,
generators, TypedArrays/Atomics, WeakRef/finalization, bytecode version 5 and
BJSON interoperability, `std`/`os`, workers, REPL/qjsc, and the complete Rust
and C embedding APIs.

Code organization is also not final. Runtime white-box tests live in
`runtime/tests.rs`, while the Array constructor, prototype, iterator, species,
and sorting implementation now lives in `runtime/intrinsics/array.rs`.
The Object constructor, implemented statics and implemented prototype handler
surface now live with `groupBy` in `runtime/intrinsics/object.rs`; the String
constructor/static table, implemented prototype-table initialization,
index-search pair, regexp-aware includes family, generic split, subrange trio,
`repeat`, the pad pair, trim group, Unicode case-conversion group and Annex-B
CreateHTML family live in `runtime/intrinsics/string.rs`; generic
match/matchAll/search
protocol integration lives in `runtime/intrinsics/string/regexp.rs`, while the
remaining String initialization and handlers still await migration there. The
complete Math
object table, selectors, numerical kernels, random and precise-sum handlers live
in `runtime/intrinsics/math.rs`; the complete Reflect table and handlers live in
`runtime/intrinsics/reflect.rs`. The observable Date intrinsic is isolated in
`runtime/intrinsics/date/`: `calendar.rs`, `parse.rs`, `format.rs`, and
`host.rs` own the pure calendar, parser, formatter, and injectable host seams;
`constructor.rs` and `prototype.rs` own the observable native handlers, while
the branded payload, typed selectors, and realm-root edge remain in the heap.
The observable RegExp shell is likewise isolated in
`runtime/intrinsics/regexp/`: installation/dispatch, constructor/allocation,
accessors/source formatting, builtin/abstract execution and the match, search
and split protocols, plus the legacy compile mutation, live in separate
modules, while `src/regexp/` remains runtime independent. The R1d match loop is
a dedicated 155-line module; R1e adds a 237-line split module plus a reusable
SpeciesConstructor helper, and R1f adds a 96-line compile module rather than
returning any of those algorithms to the facade.
The complete VM-to-runtime trait adapter,
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
lines to 9,937 lines; subsequent wiring reached 9,944 before the RegExp brand
added only nine exhaustive-class arms. Observable RegExp bootstrap and dispatch
add seven facade lines; the host-stack guard note reached 9,963 lines, and R1b
literal wiring adds only ten more. R1c search dispatch adds nine facade lines,
leaving the parent at 9,982 lines; R1d match dispatch adds eight facade lines,
and R1e split dispatch adds only four more, leaving that milestone's parent at
9,994 lines. R1f then moves the complete 224-line native-stack policy to
`runtime/native_stack.rs`; the compile wiring and extraction leave the current
`runtime.rs` at 9,787 lines. R1h keeps the replacement algorithms in dedicated
String, RegExp and shared-substitution modules, then moves internal call and
bound-argument dispatch into `runtime/native_dispatch.rs`; the parent is now
9,650 lines. R1i adds its raw predicate, direct matcher, and range-aware
substitution support inside those same dedicated modules; `runtime.rs` remains
9,650 lines. R1j keeps the complete matchAll algorithms in
`runtime/intrinsics/regexp/match_all.rs` and String's existing RegExp protocol
module; only exhaustive class wiring reaches the parent, now 9,660 lines. The
subsequent R1k-R1o wiring leaves the parent at 9,677 lines. R1p moves result
construction into `runtime/intrinsics/regexp/result.rs`, so named captures add
zero lines to `runtime.rs`. R1u keeps eval bootstrap and dispatch semantics in
`runtime/intrinsics/eval.rs`; the parent receives only the two-line bootstrap
call and remains 9,679 lines. R1w's descriptor plumbing reached 9,692 lines;
R1x keeps String-eval compilation, realm selection and closure instantiation in
`runtime/intrinsics/eval.rs`, publication checks in
`runtime/bytecode_publish.rs`, and frame capture in `runtime/vm_host.rs`. The
parent is 9,701 lines rather than absorbing those algorithms. The feature
algorithms do not return to the parent monolith. R1y keeps declaration
compilation in `compiler.rs`, publication in `runtime/bytecode_publish.rs`, and
variable-object operations in `runtime/vm_host.rs`; `runtime.rs` grows only to
9,730 lines for the host dispatch boundary and redeclaration materialization.
R1z keeps recursive caller-profile linking in `compiler.rs`, provenance checks
in `runtime/bytecode_publish.rs`, and live descriptor validation in
`runtime/intrinsics/eval.rs` plus `runtime/vm_host.rs`; `runtime.rs` remains
9,730 lines. R2b's dispatch wiring leaves it at 9,732 lines, and R2c changes no
runtime facade code. R2d-1 moves the 7,961-line compiler white-box test module
to `compiler/tests.rs`; `compiler.rs` falls from 20,560 to 12,576 lines with
production compiler code byte-for-byte unchanged. R2d-2a then moves the
333-line Arrow parser and non-committing cover-grammar scanner to
`compiler/arrow.rs`; the moved method bodies are unchanged apart from module
visibility and `compiler.rs` falls again to 12,248 lines. R2d-2b isolates the
256-line `<this>`/`<new.target>` owner, eval-exposure and prologue resolver in
`compiler/pseudo_binding.rs`; the parent reaches 12,012 lines without changing
identifier-resolution events or entry-prefix ordering. R2d-2c moves the
178-line ordinary-function definition parser and its two transfer records to
`compiler/function.rs`; the parser bodies are unchanged and `compiler.rs` now
stands at 11,842 lines. R2d-2d then moves the unchanged 171-line object-literal
lowering method into the 182-line `compiler/object_literal.rs` module, reducing
the parent to 11,671 lines and giving the next method/accessor slice a bounded
compiler home. Further production phase splits remain required as those
semantics land.
At the R2d-2c landing, the complete 102,037-variant Test262 report remained
byte-for-byte identical to the R2c hashes above at 30,254 passes; the subsequent
R2e profile truth-up changes only selection and classified report metadata.
R2f keeps method parsing in `compiler/object_literal.rs` and
`compiler/function.rs`, and method publication in the new 100-line
`runtime/object_literal.rs`. `runtime.rs` grows only by the module declaration
to 9,733 lines; `compiler.rs` is 11,706 lines, so the feature does not resume
growth of either parent monolith.
R2g keeps accessor parsing and diagnostics in those same bounded compiler
modules and reuses `runtime/object_literal.rs` without changing the runtime
facade. `compiler/function.rs` is 315 lines,
`compiler/object_literal.rs` is 290, `compiler.rs` is 11,714 after the strict
reserved-word priority fix, and `runtime.rs` remains 9,733 lines.
R2h keeps runtime behavior in the new 165-line `runtime/home_object.rs` and
97-line `runtime/vm_host/super_property.rs`; `runtime.rs` grows only by the
module declaration to 9,734 lines. Generic Reference-expression lowering raises
`compiler.rs` to 11,874 lines while the bounded object/function parsers remain
290/315 lines. That compiler growth is tracked as structural debt for the next
expression-lowering split rather than a precedent for resuming monolith growth.
R2i leaves `runtime.rs`, `runtime/home_object.rs`, and
`runtime/vm_host/super_property.rs` at 9,734/165/97 lines and extends the bounded
`compiler/pseudo_binding.rs` owner to 288 lines for the authenticated
HomeObject pseudo local and closure relay. `compiler.rs` reaches 11,899 lines;
`compiler/arrow.rs`, `compiler/function.rs`, and
`compiler/object_literal.rs` remain 333/315/290 lines. The separate entry-prefix
prepend passes remain the composer-order debt described above; they should be
unified rather than moved back into the compiler facade.
R2j keeps `runtime.rs` flat at 9,734 lines. The current capability and
authentication owners are `compiler.rs` at 11,998 lines,
`compiler/pseudo_binding.rs` at 295, `runtime/bytecode_publish.rs` at 5,003,
`runtime/intrinsics/eval.rs` at 663, and `runtime/vm_host.rs` at 3,198; the
resident compiler expectation coverage puts `compiler/tests.rs` at 8,737.
Keeping the runtime facade unchanged is intentional, while the compiler facade
and its test file remain explicit monolith debt for a later phase-aligned split.
R2k moves all template-literal lowering into the new 191-line
`compiler/template.rs`, reducing `compiler.rs` from 11,998 to 11,956 lines even
after tagged calls are added. Realm-local template object publication lives in
the new 111-line `runtime/template_object.rs`; `runtime.rs` grows only 16 lines
from 9,734 to 9,750 for the constant-pool plumbing and module hook. This keeps
the feature on bounded owners instead of resuming either monolith's growth.
R2l/R2m keep JSON algorithms in `runtime/intrinsics/json/`: the strict parser,
reviver walk, Raw JSON brand, and iterative stringifier occupy 517/208/116/741
lines.
R2n keeps the strong-Map algorithms in the dedicated 1,141-line
`runtime/intrinsics/map.rs`; the 9,613-line heap owns the branded Map and
MapIterator payloads, ordered records, iterator state, roots, and atom
lifetimes. Initialization, dispatch, and exhaustive payload routing move
`runtime.rs` only from 9,762 to 9,791 lines; `compiler.rs` remains 11,956
lines. This is a bounded intrinsic-family addition rather than another
algorithm folded into the runtime or compiler monolith.
R2o likewise keeps the Set algorithms in the dedicated 1,536-line
`runtime/intrinsics/set.rs`. Initialization, dispatch, and exhaustive payload
routing move `runtime.rs` only from 9,791 to 9,817 lines, while `compiler.rs`
remains 11,956 lines. The shared heap owner reaches 10,419 lines with the
independently branded Set/SetIterator payloads, ordered records, roots, and atom
lifetimes; that heap monolith remains explicit split debt even though Set did
not fold its algorithms back into the runtime facade.
R2p adds no production code. R2q keeps the runtime facade at 9,822 lines and
moves the shared flat-array binding lowering into the new 418-line
`compiler/destructuring.rs`; `compiler.rs` is 11,838 lines. Ordinary
declarations and synchronous iteration heads now use that same bounded owner,
so the feature does not duplicate binding logic or resume runtime-monolith
growth.
The RegExp kernel itself is isolated in
`src/regexp/` as flags, typed opcodes, compiler and executor modules rather than
growing the runtime facade. Realm-aware property completion wrappers and storage
helpers, bytecode publication linking and call dispatch, runtime/root lifecycle,
and the remaining intrinsic families still share the file; `compiler.rs`
similarly combines several compiler phases.
Dedicated structural milestones must keep splitting those seams under the same
differential and Rust-only gates, and future feature work must not resume
extending either monolith indefinitely.

`README.md` remains the 45-line public entry point; milestone bookkeeping stays
in these dedicated status and Test262 documents.

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
  cargo test --test oracle_string_split -- --nocapture
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
  cargo test --test oracle_date_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_regexp_engine -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_regexp_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_search -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_match -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_regexp_split -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_regexp_compile -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_regexp_modifiers -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_replace -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_regexp_replace -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_regexp_match_all -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_string_match_all -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_regexp_backreferences -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_regexp_lookahead -- --nocapture
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
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_unicode_u180e -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_eval_intrinsic -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_with -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_arrow_functions -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_methods -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_accessors -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_super -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_super_arrow -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_super_eval -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_tagged_templates -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_json_parse -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_json_stringify -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_json_raw -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_map -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_set -- --nocapture

./scripts/test-parity-slice.sh
./scripts/test-test262-smoke.sh
./scripts/test-test262-provenance.sh
./scripts/test-test262-reflect.sh
./scripts/test-test262-date.sh
./scripts/test-test262-string-split.sh
./scripts/test-test262-regexp-core.sh
./scripts/run-test262-regexp-literals.sh
./scripts/run-test262-regexp-search.sh
./scripts/run-test262-regexp-match.sh
./scripts/run-test262-regexp-split.sh
./scripts/run-test262-regexp-compile.sh
./scripts/run-test262-regexp-modifiers.sh
./scripts/run-test262-replace.sh
./scripts/run-test262-regexp-match-all.sh
./scripts/run-test262-regexp-backreferences.sh
./scripts/run-test262-regexp-lookahead.sh
./scripts/run-test262-regexp-lookbehind.sh
./scripts/run-test262-regexp-unicode-properties.sh
./scripts/run-test262-regexp-named-groups.sh
./scripts/run-test262-regexp-duplicate-named-groups.sh
./scripts/run-test262-regexp-match-indices.sh
./scripts/run-test262-regexp-dotall.sh
./scripts/run-test262-unicode-u180e.sh
./scripts/run-test262-eval-intrinsic.sh
./scripts/run-test262-eval-declarations.sh
./scripts/run-test262-nested-direct-eval.sh
./scripts/run-test262-with.sh
./scripts/run-test262-arrow.sh
./scripts/run-test262-object-methods.sh
./scripts/run-test262-object-accessors.sh
./scripts/run-test262-object-super.sh
./scripts/run-test262-object-super-arrow.sh
./scripts/run-test262-object-super-eval.sh
./scripts/test-test262-tagged-template.sh
./scripts/test-test262-json-parse.sh
./scripts/test-test262-json-stringify.sh
./scripts/test-test262-json-raw.sh
./scripts/test-test262-map.sh
./scripts/test-test262-set.sh
./scripts/test-test262-symbol-protocols.sh
./scripts/test-test262-array-binding-flat.sh
./scripts/test-test262-full.sh
```

The direct commands above run the dedicated Boolean, Symbol,
String constructor/static table, String-exotic substrate, String UTF-16 prefix,
String index-search, regexp-aware includes, generic String match/search/split and
String subranges, String-conversion core,
Unicode String case conversion, String-rope/byte/native-Error kernels, Unicode
identifier core, global
BaseObjects, complete Number-, BigInt-, Math-, Reflect- and Date-intrinsic
differentials, the runtime-independent RegExp-kernel, observable
RegExp-intrinsic differentials, the search/match/split protocol differentials,
and the legacy compile mutation differentials, and the
Program-var/function, Program/body/block/switch/classic-for lexical-scope,
ordinary mapped/unmapped Arguments object,
single/labelled Annex B, synchronous try/catch/finally, synchronous
for-in/for-of, Array core/literal/iterator/search/callback/mutation/change-by-copy,
Object literal/concise-method/accessor/direct/arrow/direct-eval-super, and Object
constructor/static-prefix/prototype
slices. The atom-Error
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
and folded control-flow reachability at the 65,534-slot stack limit. The tagged
template target separately locks frozen cooked/raw site objects, receiver and
evaluation order, compilation-site identity, direct-eval/super composition,
and GC lifetime. The JSON targets lock strict UTF-16 parsing, reviver source
contexts, stringify traversal/coercion/quoting, and Raw JSON branding and
source splicing. The Map target locks its intrinsic graph, descriptors,
constructor closing order, ordered `SameValueZero` records, live iteration,
callback mutation, realm ownership, and GC/atom edges. The Set target separately
locks its independent brands, exact aliases, set-like protocol and all seven
composition methods under the same mutation, realm, and lifetime boundaries.
The full gate discovers every `tests/oracle_*.rs`
integration target, reuses an executable `QJS_ORACLE` or checksum-verifies and
builds the pinned test-only oracle, obtains and checksum-verifies the matching
Unicode table source, then runs both generated-table drift checks, formatting,
unit/integration/oracle tests, Clippy, and the Rust-only product gate. The oracle
is never part of the product dependency graph or runtime.
