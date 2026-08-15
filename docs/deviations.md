# Deviation ledger

This ledger is subordinate to [`parity.md`](parity.md). Approved target
deviations are limited to the exact observable behavior and test variants named
below; they do not authorize adjacent differences. An unsupported feature or an
unresolved mismatch still blocks the relevant parity claim and is not silently
accepted as a deviation.

## Approved target deviations

### TEST262-ANNEXB-EVAL-001

- Status: approved target deviation on 2026-08-09.
- Approved by: the architecture-hygiene milestone review, under the repository
  maintainer's standing delegation of Feature Parity implementation judgment.
- Surface: Annex B eval function declarations inside a `with` environment.
- Exact Test262 key:
  `test/staging/sm/lexical-environment/block-scoped-functions-annex-b-eval.js`
  in the `sloppy` variant (`noStrict`).
- Upstream anchor: the pinned release's `test262_errors.txt` records the
  location `block-scoped-functions-annex-b-eval.js:11`. Pinned QuickJS produces
  `outer-gouter-geval-gtruefalseq`; the assertion expects
  `outer-geval-gwith-gtruefalseq`.
- Rationale: retain the Rust engine's Test262-conforming Annex B.3.3.3 result
  instead of reproducing this pinned QuickJS known failure. The declaration
  updates the eval variable environment without replacing the `with` object's
  own `g` property.
- Compatibility impact: code depending on the pinned bug observes the outer
  binding change and the `with` property remain unchanged in Rust. The
  deviation is limited to the exact binding interaction represented by the
  Test262 key above.

Minimal probe:

```js
var log = "";
function f() {
  log += g();
  function g() { return "outer-g"; }
  var o = { g: function () { return "with-g"; } };
  with (o) {
    eval('{ function g() { return "eval-g"; } }');
  }
  log += g();
  log += o.g();
}
f();
print(log);
```

From the repository root, run the probe as one line with the pinned oracle and
the Rust CLI:

```sh
./target/oracle/quickjs-2026-06-04/qjs -e 'var log="";function f(){log+=g();function g(){return "outer-g"}var o={g:function(){return "with-g"}};with(o){eval("{ function g(){ return \"eval-g\"; } }")}log+=g();log+=o.g()}f();print(log)'
cargo run --quiet --locked --bin qjs -- -e 'var log="";function f(){log+=g();function g(){return "outer-g"}var o={g:function(){return "with-g"}};with(o){eval("{ function g(){ return \"eval-g\"; } }")}log+=g();log+=o.g()}f();print(log)'
```

Pinned QuickJS prints `outer-gouter-geval-g`; Rust prints
`outer-geval-gwith-g`.

### TEST262-ARROW-FOR-HEAD-001

- Status: approved target deviation on 2026-08-09.
- Approved by: the architecture-hygiene milestone review, under the repository
  maintainer's standing delegation of Feature Parity implementation judgment.
- Surface: early-error parsing of an arrow expression in a classic `for`
  statement head created through the `Function` constructor.
- Exact Test262 keys:
  - `test/staging/sm/statements/arrow-function-in-for-statement-head.js` in the
    `sloppy` variant;
  - the same path in the `strict` variant.
- Upstream anchor: the pinned release's `test262_errors.txt` records both
  variants at `arrow-function-in-for-statement-head.js:13` because no
  `SyntaxError` is thrown.
- Rationale: retain the Rust parser's Test262-conforming early `SyntaxError`
  instead of reproducing this pinned QuickJS known failure. The constructed
  function body is parsed independently, so the same target difference is
  observable from both outer Test262 variants.
- Compatibility impact: source relying on pinned QuickJS accepting
  `for (x => 0 in 1;;) break;` through `Function` is rejected by Rust. The
  deviation does not broaden which arrow or `for` forms are rejected beyond
  this invalid grammar family.

Minimal probe:

```js
try {
  Function("for (x => 0 in 1;;) break;");
  print("accepted");
} catch (error) {
  print(error.name);
}
```

From the repository root:

```sh
./target/oracle/quickjs-2026-06-04/qjs -e 'try{Function("for (x => 0 in 1;;) break;");print("accepted")}catch(e){print(e.name)}'
cargo run --quiet --locked --bin qjs -- -e 'try{Function("for (x => 0 in 1;;) break;");print("accepted")}catch(e){print(e.name)}'
```

Pinned QuickJS prints `accepted`; Rust prints `SyntaxError`.

## Resolved findings

### FORIN-FAST-ARRAY-001

- Status: resolved on 2026-07-15; no deviation approval requested.
- Surface: representation-sensitive Array mutation during `for-in`.
- Upstream anchor: `quickjs.c` 16282-16509.
- Compatibility impact while open: a deleted dense own index could incorrectly
  hide an inherited key, and a newly added own key could fail to hide one.

Minimal deletion probe:

```js
(function () {
  var p = [];
  p[1] = "proto";
  var a = [0, 1], out = "";
  Object.setPrototypeOf(a, p);
  for (var key in a) {
    out += key + ",";
    if (key === "0") delete a[1];
  }
  return out;
})()
```

Pinned QuickJS and the current Rust engine both return `0,1,`. A second
differential forces the same current key set through `Object.defineProperty` or
sparse growth first; both engines then retain the slow representation and
return `0,`.

The fix records QuickJS's irreversible fast/slow Array state in the heap
payload. A count-only fast iterator refreshes the source's current own names
when prototype enumeration becomes necessary; a slow iterator retains its
initial snapshot. Both paths are pinned in
`tests/oracle/control_flow/oracle_for_in.rs`.

## Open implementation frontiers

- Dynamic import retries failed acyclic source graphs as pinned QuickJS does.
  Parse-in-progress definitions and request prefixes are published in the same
  callback order as pinned QuickJS, including its one-shot resolution latch.
  Rust represents a swallowed resolution failure as an explicit incomplete
  state with no partial raw dependency vector; direct link/execute returns a
  typed error and dynamic import rejects with a deterministic `InternalError`.
  If a resolved module still refers to a construction that later fails, Rust
  keeps that append-only identity as an edge-free `Aborted` sentinel, excludes
  it from name lookup, and gives a same-name retry a new identity. Pinned
  QuickJS can instead retain a dangling `JSModuleDef *`; subsequent use enters
  native undefined behavior and may manifest as a native crash or allocator
  aliasing. Those unsafe operations are deliberately excluded from the
  automated oracle suite. Reproducing those lifetimes is not an approved
  parity target. Vacant Rust module-cache slots and orphaned sentinels are
  currently not compacted, so repeated failed publications remain a
  resource-hygiene frontier.
- Admitted ordinary async-generator function, object-method, and public
  class-method direct-yield/await paths match the pinned driver, including
  poisoned Promise constructors, iterator-result resolution reentry, and
  completed-state queue re-entry.
  Internal allocation/setup
  failure is not yet recovered identically: iterator-result allocation can
  fail after the VM has advanced, and failure to create or install a private
  continuation can leave the front request capability pending for a later
  retry. These host-error paths require fault injection and a transactional
  pending-settlement representation before they can be admitted; they are an
  unresolved hardening frontier, not an approved observable deviation.
- `Promise.all`, `Promise.allSettled`, and `Promise.any` match pinned QuickJS on
  ordinary JavaScript-observable paths, but internal allocation failure is not
  yet routed identically. Failure to allocate the output Array currently
  returns a host runtime error instead of rejecting the new capability;
  failure to allocate an element callback also omits QuickJS's
  close-then-reject path. Internal `AggregateError` allocation has the same
  runtime-error boundary. The checked `u32` element index has a theoretical
  multi-billion-element RangeError boundary instead of QuickJS's C `int`
  environment behavior. These are unresolved hardening frontiers, not
  approved deviations.
