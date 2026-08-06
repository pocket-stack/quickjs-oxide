# Implementation status

Last audited: 2026-08-06. The completion definition remains
[`parity.md`](parity.md); this file records progress and must not be used to
claim full parity.

## R3dn scoped Test262 agent broadcast cohort A

R3dn implements the pinned QuickJS `$262.agent.broadcast` /
`receiveBroadcast` control plane for fixed `SharedArrayBuffer` backing. A
broadcast snapshots the invocation-time worker cohort, publishes one shared
backing handle and an `int32` payload, and waits for every worker to acknowledge
delivery before any callback has to finish. Workers keep callbacks rooted only
in their own runtime thread. A synchronous callback replacement is discarded
at the end of that delivery, while a Promise job may install the receiver for
the next generation, matching QuickJS's callback/job ordering.

This is a scoped implementation milestone, not yet a global Test262
promotion. The source- and metadata-authenticated cohort activates 15 paths /
30 sloppy-and-strict variants. Oxide passes all 30, while the other 43 paths /
86 variants remain `unsupported-host-agent`; the exact transition is 30 pass
gains, 86 unchanged rows, and zero regressions. Twenty single-worker replays
produce 600/600 activation passes, and pinned QuickJS 2026-06-04 passes the
same 30 variants.

The admitted sources use only fixed shared buffers. Ordinary `ArrayBuffer`
and growable `SharedArrayBuffer` broadcasts remain explicitly fail-closed
rather than pretending that a copied backing store has shared identity.
Worker exceptions are also surfaced more strictly than QuickJS's diagnostic
printing. Those differences must be resolved or separately audited before the
remaining agent cohorts can be admitted. WebAssembly continues to expose no
agent threads, and `Atomics.waitAsync` remains outside the pinned QuickJS
target.

Reproduce the checksum-bound scoped receipt with:

```sh
./scripts/test-test262-agent-broadcast-a.sh --check
```

## R3dm Test262 agent Stage A

R3dm adds an opt-in, QuickJS-shaped Test262 `$262.agent` host without making
the engine or its ordinary embedding contexts thread-safe. Each `agent.start`
creates a fresh `Runtime` and `Context` on a dedicated native thread with a
2 MiB stack. Only owned source text and an `Arc`/`Mutex` host coordinator cross
the thread boundary; no runtime, realm, value, object, or heap root does.
Workers enable blocking waits, reports use a shared FIFO queue, and cleanup
joins workers in start order. The installed method order, descriptors, role
checks, `sleep`, `monotonicNow`, `report`, `getReport`, `leaving`, and
`createRealm` behavior follow pinned QuickJS 2026-06-04.

Stage A deliberately does not claim the shared-buffer broadcast protocol.
`broadcast` and `receiveBroadcast` are present with the QuickJS method shape
but fail closed; native agent threads are unavailable on WebAssembly. The
Test262 runner adds a separate exact-path allowlist and revalidates the pinned
source hash and metadata before enabling the host. Only
`test/built-ins/Atomics/wait/good-views.js` is admitted. The other 58 paths /
116 variants remain `unsupported-host-agent`, and `Atomics.waitAsync` remains
outside the pinned QuickJS parity target.

The authenticated agent universe contains 59 paths / 118 sloppy-and-strict
variants. The R3dl parent records all 118 as unsupported; the R3dm candidate
passes the two admitted variants and leaves the other 116 byte-identical. A
20-run stability gate records 40/40 activation passes, and pinned QuickJS
passes both activation variants. Native tests separately cover the host shape,
FIFO reports, role failures, fresh-runtime isolation, start-order joining, and
worker failure cleanup.

The complete 102,037-variant join reaches 66,478 passes / 66,530 runnable. It
changes exactly the two admitted rows, reduces `unsupported-host-agent` from
118 to 116, and leaves the other 102,035 rows unchanged with zero regression.
Two independent release-mode full runs are byte-identical. The canonical
TSV/JSONL SHA-256 values are
`a05aed38d47216ca485334ad50656cbe3ddf8d5c9922a6eaf28e0ee9ff0863dc`
and
`4f0ff98da92582ae37571d754e6608b20aa707c0c1e456232b527e778b87e9c0`.

Reproduce the scoped implementation proof and global admission with:

```sh
./scripts/test-test262-agent-stage-a.sh --check
./scripts/test-test262-agent-stage-a-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-agent-stage-a-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-agent-stage-a-global.sh --full
```

## R3dl global SharedArrayBuffer and Atomics admission

R3dl globally admits the already implemented `SharedArrayBuffer` and `Atomics`
surfaces. The live profile grows from 130 to 132 reviewed feature tags and
hashes to
`47cf8351f7844340bbbff3ba9bb781faf552f8f27d0dd6cca2e35dbf9ad48232`.
The authenticated admission universe contains 445 paths / 886 variants: 435 /
866 activate and pass, the six `Atomics.pause` paths / 12 variants remain
passes, and four cross-realm paths / eight variants remain fail-closed. Pinned
QuickJS passes all 886 variants. The exact Oxide join records 866 pass gains,
eight diagnostic-only changes, 12 unchanged rows, and zero regressions.

The complete 102,037-variant join reaches 66,476 passes / 66,528 runnable and
reduces `unsupported-feature` to 12,761; every other outcome count is
unchanged. Two independent release-mode full runs are byte-identical. The
canonical TSV/JSONL SHA-256 values are
`501b64ed5c8367f33408225d956a262619163adf52baadf28f02811d14f3eae9`
and
`610e16ba65a0239556842efec7a745ba2885c72dfb3b8447c2578b8767ef7d40`.
The next shared-memory parity frontier is the 59-path / 118-variant Test262
`$262.agent` host; pinned QuickJS passes that entire cohort. `Atomics.waitAsync`
remains outside the pinned QuickJS target.

Reproduce the checksum-bound profile, focused differential, and full join with:

```sh
./scripts/test-test262-shared-atomics-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-shared-atomics-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-shared-atomics-global.sh --full
```

## R3dk verifiable public playground

R3dk is a presentation and provenance milestone, not a Test262 admission. The
public GitHub Pages playground runs the repository's Rust engine compiled to
WebAssembly in a dedicated worker. Its WASM package exports the crate version,
pinned QuickJS target, exact deployed commit, and `canBlock=false` host policy;
the page displays those values, while the Pages workflow injects and links its
trusted `github.sha`. Local builds without an injected identity are deliberately
labelled `local`; caller-supplied local identities are provenance labels, not
cryptographic attestations.

All 15 curated examples carry a human description and expected result. The new
`Atomics.wait host policy` example returns 42 only for the exact non-blocking
TypeError boundary. The Node/WASM gate checks all examples against an
independent expectation map and rejects browser-native evaluation. A second
gate serves the final `target/pages` tree to Chromium, waits for the worker and
engine, exercises both the default and wait-policy examples, verifies displayed
provenance, and fails on console, page, worker, request, or HTTP errors.

The canonical Test262 vector therefore remains 65,610 passes / 65,662 runnable
/ 102,037 total variants. Reproduce the artifact and both execution gates with:

```sh
./scripts/test-web-playground.sh
npm ci
npx playwright install chromium
npm run test:browser
```

## R3dj bounded non-agent Atomics.wait implementation

R3dj implements synchronous `Atomics.wait` and real `Atomics.notify` waiter
coordination. Wait locations use the shared backing allocation plus absolute
byte offset, so distinct runtimes and Int32/BigInt64 views of the same address
interoperate. A process-wide coordinator linearizes compare-and-register with
FIFO notification, releases its lock while waiting, handles spurious wakeups,
and gives timeout and notification races a single winner. Shared backing IDs
cannot wrap and be reused. New runtimes retain QuickJS's default
`can_block=false`; the Test262 worker opts in except for `CanBlockIsFalse`.

The source audit finds 93 raw `Atomics.wait` paths: 33 bounded direct-call
paths, 57 agent paths, and three descriptor/length/name metadata paths. Every
selected path runs in sloppy and strict mode. Oxide and pinned QuickJS
2026-06-04 both pass all 66 variants. The exact manifest and scoped profile
hash to
`38f69242c52bfda864397a6413dedad9eb3a60ca2c07683f857791300948348d`
and
`ae525546ca879c3f159d491df0a95e3df8bef438f70908057ab10c90ca1545e4`.
Oxide's 77-line TSV and 68-line JSONL hash to
`b90662c8814a1e3db00338aadb84731d0721e349c9b2a76ddeb0b583cb0d667a`
and
`4845a5629ecdc6b26b3e9ea2724cee1215a07903dc9b5139f76406627ee5bf6d`.
Native tests additionally prove finite timeout cleanup, FIFO/count behavior,
spurious-wakeup handling, timeout/notify races, and an actual cross-runtime
BigInt64 wait woken through an Int32 view. The global 130-tag Test262 vector is
unchanged at 65,610 passes / 65,662 runnable / 102,037 total. Four rows move
from `unsupported-host-can-block-false` to `unsupported-feature` because the
host policy is now implemented while broad `Atomics` is not globally admitted.
The current full TSV/JSONL hashes are
`17370398c6a211d4657ad763a6e40f0cd198d72faa14b2995f7937ad52a0c6db`
and
`6e12d86318b2f1d7e5f684962a02585b1a91a4d7830d6e05ed38f80c766cc9a1`.

This is not agent parity: the 57 `$262.agent` paths and their host protocol are
still excluded, and the selected Test262 paths contain no notification wakeup
or infinite wait. Pinned QuickJS has no `Atomics.waitAsync`, so waitAsync is
outside this rewrite's parity target. Reproduce the authenticated boundary,
native waiter proof, Oxide vector, and QuickJS oracle with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-atomics-wait-nonagent-bounded.sh --check
```

## R3di non-blocking shared Atomics

R3di extends the existing QuickJS-shaped Atomics implementation to integer
TypedArrays backed by `SharedArrayBuffer`. `load`, `store`, `add`, `sub`,
`and`, `or`, `xor`, `exchange`, and `compareExchange` preserve the pinned
validation, coercion, narrowing, return-value, and grow/revalidation order.
After observable JavaScript work has completed, every shared load, store, and
read-modify-write operation takes a process-global sequential-consistency gate
and then the target backing lock. This conservatively reproduces the
cross-buffer order of QuickJS's C11 `seq_cst` atomics without `unsafe`. It also
serializes independent buffers, so replacing that gate with narrower safe
atomic primitives is an explicit performance debt, not a semantic gap. The
ordinary ArrayBuffer extension remains unchanged.

At R3di, `Atomics.notify` accepted shared Int32Array and BigInt64Array views and
completed validation/count coercion but returned zero because that milestone
had no waiter registry. R3dj above supersedes that boundary with synchronous
waiters and real notification. `Atomics.waitAsync` remains absent, matching an
API that pinned QuickJS does not provide.

The scoped Test262 selection contains 100 paths / 200 variants. It combines
the 78 paths / 156 variants carrying the `SharedArrayBuffer` feature tag with
a disjoint 22-path / 44-variant source-audited spillover whose metadata omits
that tag. Oxide and pinned QuickJS 2026-06-04 both pass all 200 variants. Two
Oxide runs are byte-identical: the 211-line focused TSV hashes to
`e265924c5773626f73f5396803a8b3e19e5650bad49efe04a390dfa77b86548a`,
and the 202-line JSONL hashes to
`9a012e1be03b8f752efc15a1250548388c1c0c41f6443e44ec8be4f98842fb34`.
The exact combined manifest and selection-only profile hash to
`a9072513df3c730b87a84218a88755c229a8090e0100bc44bbd7d2550ac72dc0`
and
`ec33455551c3601859241870624b5017551aa04c8edbf8c9e899d4ef9b5332cc`.
This milestone does not globally admit `SharedArrayBuffer` or broad `Atomics`,
so the canonical 130-tag vector remains unchanged.

The browser playground adds a `Shared Atomics` example which stores 40,
atomically adds 2, and loads 42 through the real WASM engine. Reproduce the
scoped inventory/oracle checks and browser smoke with:

```sh
./scripts/test-test262-shared-atomics-nonblocking.sh --check
./scripts/test-web-playground.sh
```

## R3dh SharedArrayBuffer core

R3dh adds a distinct `%SharedArrayBuffer%` implementation rather than treating
shared storage as an ArrayBuffer mode. Its constructor, `byteLength`,
`maxByteLength`, `growable`, `grow`, `slice`, and `Symbol.species` behavior
follow pinned QuickJS 2026-06-04, including QuickJS's wrapper-local grow length.
Growable buffers reserve their maximum zero-filled capacity up front. Slice
creates an independent fixed shared backing, while constructor and species
subclass paths preserve realm and prototype behavior.

DataView and all 12 TypedArray classes can create fixed or length-tracking
views over the shared backing. The shared byte store uses safe synchronized
access with no `unsafe`; runtime heap borrows are released before a backing
lock is taken. This keeps coercions, property access, callbacks, and species
construction outside byte transactions while retaining the existing
ArrayBuffer behavior.

The public `SharedBufferHandle` bridge lets a host export a genuine
SharedArrayBuffer from one `Context` and import it into another runtime. The
sendable handle contains no runtime `Value`, arena identity, or heap edge.
Imported wrappers share bytes, keep independent wrapper-local visible lengths,
and remain valid across runtime garbage collection.

The pinned Test262 inventory contains 463 paths / 922 variants carrying the
`SharedArrayBuffer` feature. R3dh authenticates a no-Atomics core of 221 paths /
438 variants; Oxide and pinned QuickJS both pass all 438. Oxide's focused TSV
and JSONL SHA-256 values are
`03f445aa2978b001a7737bbd482e9b36d35182471b71961fa273d916d24450d8`
and
`b4d7ff88f0f9480eb81b068cdcaedcf3667aca70ab82e830d5ef7e5aafc01ad1`.

This is deliberately a selection-only milestone. It does not add the
`SharedArrayBuffer` tag to the global capability profile, so the 130-tag
canonical vector remains 65,610 passes / 65,662 runnable. At R3dh the remaining
inventory was explicitly partitioned into 78 non-blocking Atomics paths / 156
variants, 20 synchronous-wait paths without agents / 40 variants, 58 agent
paths / 116 variants, and 86 `Atomics.waitAsync` paths / 172 variants. Shared
Atomics were still pending then; R3di closes the non-blocking slice and R3dj
adds the bounded synchronous waiter core, while the Test262 agent host and
agent-backed conformance remain future milestones.
Pinned QuickJS does not provide `Atomics.waitAsync`, so that API is not a
feature-parity milestone.

The browser playground adds a no-Atomics SharedArrayBuffer example that grows
the buffer, writes through TypedArray and DataView views, slices it, and
returns 42 through the real WASM engine. Reproduce the focused evidence and
the public-artifact smoke with:

```sh
./scripts/test-test262-shared-array-buffer-core.sh --check
./scripts/test-web-playground.sh
```

## R3dg implemented leaf built-in admission

R3dg globally admits the already implemented `Error.isError`, `RegExp.escape`,
and `TypedArray.prototype.at` metadata tags. This milestone changes no runtime
semantics: it only widens the authenticated global profile from 127 to 130
tags. The candidate and live profiles are byte-identical and hash to
`280264ae035da45cd0e2727b981e64380496ed75af3216208616dfee82d0459a`.

The exact manifest contains 47 paths / 94 sloppy-and-strict variants. Oxide's
candidate records 86 passes plus eight retained `unsupported-feature`
diagnostics: two variants still require `class`, and six still require
cross-realm support. Pinned QuickJS passes all 94 variants.

The complete 102,037-variant join changes exactly those 94 manifest rows: 86
outcomes become passes and eight rows change only diagnostic detail. All
101,943 rows outside the manifest are unchanged, with zero pass regressions.
At R3dg, the canonical vector became 65,610 passes / 65,662 runnable, with
`unsupported-feature=13,623`. Candidate full TSV/JSONL SHA-256 values are
`a3b097fe77a996bc1272a9576c39f509c60ee9c3644e667ab4f0d4c141f72e32`
and
`dc37ed90322630e81fa4295daa57b8f81093719541076f84d4da27ef0d3c5d23`.

Reproduce this admission with:

```sh
./scripts/test-test262-error-regexp-typedarray-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-error-regexp-typedarray-global.sh
TEST262_REUSE_FULL_REPORTS=true \
  TEST262_FULL_WORKERS=8 \
  ./scripts/test-test262-error-regexp-typedarray-global.sh --full
```

## R3df global `Atomics.pause` admission

R3df admits the complete pinned `Atomics.pause` metadata tag without widening
the shared-memory boundary. The runtime behavior was already implemented from
QuickJS's `js_atomics_pause`: only `undefined` and integral Number values are
accepted, objects are not coerced, the operation is a non-blocking CPU hint,
and the result is `undefined`. At R3df, SharedArrayBuffer, agents, waiters, and
`Atomics.waitAsync` were still unsupported.

The old 126-tag global profile is frozen at SHA-256
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`.
The candidate and live profiles are byte-identical, contain 127 tags, and hash
to
`00265570870a778f2fded16969311eac5707b9c6d4fcd4068640700d637e9ff0`;
their only feature delta is `Atomics.pause`. The runner permits the candidate
only with the exact six-path manifest or `--all`. The frozen parent additionally
permits the exact R3dc and R3de manifests so those historical gates remain
replayable, while arbitrary manifests and single-test selection stay rejected.

The manifest hashes to
`72252a61f2d3c97626b544a1ac1a2a31191149a535227b16a8fa798c91e0d69c`.
Its 12 sloppy/strict variants move from 12 `unsupported-feature` outcomes to
12 passes in Oxide; pinned QuickJS also passes all 12. The exact focused
transition hashes to
`d8a49b050c5dd0a665008576d34f6968654c28cc32af146e0993edac59e1fdee`.

The fresh release-mode, two-worker full run joins all 102,037 variants. Exactly
those 12 outcomes change to pass, the other 102,025 rows are byte-identical,
and there are no detail-only changes or regressions. The canonical vector is
now 65,524 passes / 65,576 runnable, with `unsupported-feature=13,709`;
the seven parse failures, 43 runtime failures, and two timeouts are unchanged.
Candidate TSV/JSONL SHA-256 values are
`205ec5ef4ec03dfea59a8ff424e776406a83c1bf0c4070e68f42127331f0e6aa`
and
`627f4ccdea5825f382d9d5500a4e578fa5b38cf5bd7422525d8fb19b48065e86`.
The R3dc → R3de → R3df successor chain authenticates the historical parent
receipts before delegating to the current gate.

The browser playground's `Atomics.pause + ArrayBuffer` example executes the
real WASM engine and still returns 42. Reproduce this milestone with:

```sh
./scripts/test-test262-atomics-pause-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-atomics-pause-global.sh
TEST262_REUSE_FULL_REPORTS=true \
  TEST262_FULL_WORKERS=2 ./scripts/test-test262-atomics-pause-global.sh --full
./scripts/test-web-playground.sh
```

## R3de authenticated non-shared Atomics gate

R3de turns R3dd's source probe into a fail-closed milestone gate. A complete
source audit of the pinned Atomics metadata universe identifies 96 paths that
neither evaluate `SharedArrayBuffer` nor carry its feature tag. The formal
non-shared cohort adds the one `SharedArrayBuffer`-tagged `isLockFree` path
whose source never evaluates that constructor, plus the metadata-less
SpiderMonkey detached-buffer path. The resulting 98 paths / 196 sloppy/strict
variants pass in both Oxide and pinned QuickJS.

The runner recognizes the dedicated selection-only profile only when it is
paired with the exact 98-line manifest. It rejects `--all`, `--test`, a
different manifest, and either file after checksum drift. The profile and
manifest SHA-256 values are
`c3db1670b6cd4e2b9b1e7bd812d2e580df4ea0d8f0ceee96c074378d14dc9a5b`
and
`5c8805da455cb66810646a709d847346c1c07b2710b46838da6006667f627aac`.
The authenticated parent records 196 `unsupported-feature` outcomes; the
candidate records 196 passes, with every transition confined to the manifest.
This scoped profile is evidence machinery, not a runtime capability claim.
The committed 382-row / 764-variant Atomics metadata ledger records each
path's category, includes, flags, features, and source SHA-256. The gate
regenerates the complete 53,125-record metadata inventory, proves that the ledger is its exact
`Atomics` / `Atomics.pause` projection, and rechecks every category and source.

The twelve already-frozen paths that really evaluate `SharedArrayBuffer`
remain a disjoint deferred receipt. Pinned QuickJS passes all 24 variants;
at R3de, Oxide had no shared backing-store implementation. This gate executes
the deferred partition only in pinned QuickJS, so it is a small explicit
frontier check rather than an Oxide pass receipt or the whole future
shared-memory corpus.

The exhaustive source audit classifies all 382 paths carrying `Atomics` or
`Atomics.pause` metadata into five mutually exclusive groups. It is not an
inventory of every `SharedArrayBuffer`-only path in Test262:

- 96 evaluate no SAB and carry no SAB tag;
- one carries SAB metadata but never evaluates SAB;
- 123 really evaluate SAB without an additional agent/host requirement;
- 61 really evaluate SAB and also require an agent or host facility (59 agent
  paths plus two `CanBlock`-false paths);
- 101 exercise `Atomics.waitAsync`.

Of the 184 non-`waitAsync` shared paths, 174 construct SAB directly and ten
call the pinned `testAtomics.js` non-view helper that constructs it.

The two metadata-less staging paths remain explicit outside those 382 rows:
the detached-buffer test belongs to the green non-shared gate, while the
cross-compartment test evaluates real SAB and also requires realm support.

R3df above admits the independent six-path / 12-variant `Atomics.pause` slice
without equating it with shared-memory completion. Broad `Atomics` is not ready
for the immediately following admission. Its raw metadata closure is 119
paths / 238 variants: 90 / 180 are green and 29 / 58 hide real SAB or waiter
dependencies. Runner precedence changes the members without changing the
total: host selection already preempts `wait/good-views.js`, while the
metadata-less detached-buffer path has a supplemental `Atomics` requirement.
The resulting transition planning set is 91 green paths / 182 variants plus
28 hidden-shared paths / 56 variants. This is a checksum-bound planning
projection, not a candidate transition report; a later broad gate must execute
and freeze it or first implement the missing runtime. SAB, agents, blocking
waiters, and `Atomics.waitAsync` remain separate implementation milestones.
At R3de, `compat/test262-oxide.conf` and the canonical 102,037-row vector did
not move. Its 65,512-pass / 65,564-runnable report is now the authenticated
parent of R3df rather than the current canonical vector.

Reproduce the scoped evidence with:

```sh
./scripts/test-test262-atomics-non-shared-core.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-atomics-non-shared-core.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-atomics-non-shared-core.sh --full
```

## R3dd non-shared ArrayBuffer Atomics core

R3dd publishes QuickJS's lazy, realm-local `%Atomics%` namespace after
`DataView` and before `Promise`. Its table order, descriptors, function
metadata, non-constructor behavior, and `Symbol.toStringTag` match pinned
QuickJS. The implementation lives in a dedicated intrinsic module rather than
growing the runtime coordinator.

All eight integer TypedArray classes now support `load`, `store`, the six
read-modify-write operations, and `compareExchange` over ordinary
ArrayBuffer-backed storage. Validation freezes the old view length before
`ToIndex`, then revalidates at the same post-index and post-value boundaries as
QuickJS. Number and BigInt truncation, narrow wrapping, old-value decoding, and
the full untruncated `store` return value are preserved. `wait` rejects a
non-shared view before later argument coercions; `notify` still performs the
observable index/count coercions and returns zero. `isLockFree` and `pause`
retain QuickJS's saturated and raw-number rules.

The backing-store transaction is callback-free and contains no `unsafe`, host
atomic primitive, thread, or lock. That is sufficient for an ordinary
ArrayBuffer in the current single-agent runtime while leaving shared backing
stores and waiter coordination as a distinct future architecture.

The pinned differential matrix passes in both engines; its stdout SHA-256 is
`d5a393c1534768aec2bb3f8512bc5b01170a18c85e817e02acfd56140b2931d6`.
The historical R3dd temporary probe corrected its focused Test262 boundary to
90 paths / 180 sloppy-and-strict variants with then-current manifest SHA-256
`e9ab48b9faa090e1bc2a58a1d62e2398bca0de88a28f34c53d3397442636a380`.
Oxide and pinned QuickJS both passed 180/180. R3de above supersedes that
snapshot with the current 98-path manifest. The historical candidate
scoped-probe TSV and JSONL hashes are
`0d5b99acb171c079d91b89ca010c9061b2b552d1a1dfe530efaa554caa2335d4`
and
`baaf530b6697390a82e2751411b6cbfd7fa84dbb2c890248af37f9b06836a05f`;
the pinned QuickJS log hashes to
`7a033067036e950e1dd60e7fa91a98d7b2ed51a0a6ce0c0eeec84895d531f6d9`.

Twelve paths from the broader audit really evaluate `SharedArrayBuffer` and
are frozen separately in `test262-atomics-shared-deferred.txt`, SHA-256
`00b82b9589391b350ee77ee736c7e7c4637c19466465b4dfa4e53270cdbc02ee`.
They are not hidden inside the green non-shared count. The scoped probe was
selection-only; the global Test262 profile still declares neither `Atomics`,
`Atomics.pause`, nor `SharedArrayBuffer`, so the canonical 102,037-row vector
does not move in this runtime milestone. Global admission remains a separate
checksum-bound evidence step, and shared-memory semantics remain unfinished.

The browser playground includes an `Atomics.pause + ArrayBuffer` example whose real
WASM build returns 42. Reproduce the direct differential with:

```sh
QJS_ORACLE=target/oracle/quickjs-2026-06-04/qjs \
  cargo test --locked --test oracle_atomics_non_shared -- --nocapture
./scripts/test-web-playground.sh
```

## R3dc Atomics metadata-gap classification

R3dc is an evidence correction, not an engine feature. Two SpiderMonkey
staging tests use Atomics without declaring Test262 feature metadata. The
cross-compartment fixture was already supplementally classified as requiring
`Atomics` and `SharedArrayBuffer`; the detached-buffer fixture instead reached
the runtime and reported two Oxide-only `ReferenceError` failures even though
Oxide does not publish an `Atomics` global. Both overrides are now bound to the
exact relative path and pinned source SHA-256. A checksum mismatch aborts the
coordinator rather than silently changing selection. The cross-compartment
rule additionally retains its `createRealm`, `Atomics`, and
`SharedArrayBuffer` source-shape checks.

The focused manifest contains two paths / four sloppy-and-strict variants and
hashes to
`4863dea8db26a20638b24f6a727a0a7f0a207585a4b966a855f10fa3ea1fcb18`.
Pinned QuickJS passes 4/4. The authenticated R3db parent records the two
cross-compartment variants as `unsupported-feature` and the two detached
variants as `fail-runtime`; the candidate records all four as
`unsupported-feature / selection / EngineCapability`, with no pass movement.
Candidate focused TSV/JSONL SHA-256 values are
`3eb9e15b57371dc9d8e6b6c89edc4bb62074ef893850b0d8a6c8b7d0da5d41c5`
and
`fbd94ab0292664901f42639050d14d4da273d4a1cab66588007f0de30ec224d4`.

The fresh two-worker 102,037-row join changes exactly the two detached-buffer
outcomes from `fail-runtime` to `unsupported-feature`. The other two cohort
rows and all 102,033 non-cohort rows are byte-identical, with no detail-only
movement or pass regression. Passes remain 65,512; runnable variants become
65,564, `fail-parse=7`, `fail-runtime=43`, and `unsupported-feature=13,721`;
the two JSON mega-array variants still time out. Full candidate TSV/JSONL
SHA-256 values are
`35c329c649ecb75ec473bdaa42b361ad1173025893588f47f41a0270112872f1`
and
`f2811b3b7724123d8cb4a1b81c470f6c0b1f5f4c74d8ee26c76856c0c065861f`.

After this correction, every non-timeout failure left in the runnable vector
is also present in pinned QuickJS. That says only that the current runnable
slice has no known Oxide-only failure: 13,721 feature-unsupported variants,
host/module exclusions, and unimplemented surfaces remain explicit, so this is
not a Feature Parity claim. The next runtime target is the bounded non-shared
ArrayBuffer Atomics core; shared backing stores, agents, and blocking waiters
remain a separate architectural milestone.

Reproduce the frozen inputs and focused evidence with:

```sh
./scripts/test-test262-atomics-metadata-gaps.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-atomics-metadata-gaps.sh
```

Reproduce the complete exact join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-atomics-metadata-gaps.sh --full`.

## R3db sloppy direct-eval var BindingPattern references

R3db fixes a scope-isolation error in sloppy direct eval. A novel `var` name
inside an object, array, nested, or rest BindingPattern was correctly declared
on the caller's eval-variable object, but its final value was written to the
realm global instead. The compiler kept the eval object as a late scope
candidate and a global write as fallback; the global-Reference shortcut
incorrectly ignored that pending candidate. The shortcut is now legal only
when `late_sources` is empty, so the final Set performs QuickJS's late
`HasDynamicBinding -> PutDynamicBinding -> global fallback` selection.

This is deliberately not an early snapshot of the eval object. Pinned QuickJS
re-resolves the target after iterator, computed-key, getter, default, and rest
callbacks. If one of those callbacks deletes the eval binding, the final write
falls through to the global. The differential matrix locks both ordinary
eval-object writes and these delete/retarget cases, as well as anonymous
function, generator, async function, arrow, async arrow, and class names,
repeated eval, existing caller bindings, IteratorClose, and evaluation order.
The Pages Node/WASM smoke executes the same direct-eval NamedEvaluation path
and requires the real Rust engine to return 42.

The exact Test262 manifest contains one path / two sloppy-and-strict variants
and hashes to
`cdaad046146fc09292816cd7638ab2b3e8e9f41778f2b459ec8a7fab93b338ed`.
Pinned QuickJS and the R3db candidate pass 2/2. The checksum-bound R3da parent
records one sloppy `fail-runtime` (`TypeError: cannot read property 'name' of
undefined`) while its strict variant already passes. Candidate focused
TSV/JSONL SHA-256 values are
`8f6e7e62dbf384d3da4d35b490ad637446c26e2a57488d4d41e05b155c128ccb`
and
`622cbe1eac81740d7cd71acdf2a589aae8f52b14a361ca2c48899c7532888965`.

The fresh two-worker 102,037-row join changes exactly the sloppy
`fail-runtime -> pass` outcome. The strict cohort row and all 102,035
non-cohort rows are byte-identical; there is no detail-only movement or
previous-pass regression. The canonical vector is now 65,512 passes / 65,566
runnable, `fail-parse=7`, and `fail-runtime=45`; the two JSON mega-array
variants still time out. Full candidate TSV/JSONL SHA-256 values are
`9cfd1c1f807b10581b2964e9a6d48a3fd4cbc92ebbecf15d359a9a21fc55680e`
and
`bf0755551c28dec28cc180a492512849faeb4aeae068202b185d041493d6c0c0`.

The standing pinned-oracle classification now leaves two known actionable
QuickJS deltas in the runnable vector, both from the unimplemented Atomics
surface; the other 50 non-timeout failures are also present in pinned QuickJS.
This is a progress measure, not a parity claim: excluded and unsupported
features remain tracked by the profile and parity contract.

Reproduce the frozen inputs and focused evidence with:

```sh
./scripts/test-test262-eval-var-destructuring.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-eval-var-destructuring.sh
```

Reproduce the complete exact join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-eval-var-destructuring.sh --full`.

## R3da synchronous generator delegation stack budget

R3da removes an Oxide-only early rejection from deep synchronous `yield*`
chains. QuickJS's generator `next`/`return`/`throw` path resumes heap-owned
generator state and checks the real C stack on call entry and resume. Oxide
also has a real address-based stack guard, but it previously charged every
`GeneratorPrototypeResume` the conservative eight-unit cost for an unknown
callback-capable native. Ten nested resumes could therefore exhaust the
80-unit logical budget before the real stack guard was reached. Generator
resume now costs one nonzero unit: the address guard remains authoritative and
mixed native recursion remains budgeted.

A two-MiB native-stack test locks delegated result identity, a catchable deep
overflow, complete active-frame unwinding, and reuse of the same Context after
the exception. A sloppy/strict differential against pinned QuickJS additionally
covers deep `next`, `return` through `finally`, and `throw` propagation. The
Pages job repeats the native floor on Ubuntu, while the actual Node/WASM build
checks a 20-level result and a catchable 1,000-level overflow without a WebAssembly
trap.

This milestone does not claim exact stack-threshold parity. The current
optimized native build reaches roughly 72 delegated levels before its real
host-stack guard, while pinned QuickJS reaches 509 under the same probe. Closing
that implementation-depth gap requires an explicit VM call/resume trampoline;
it remains part of the Feature Parity goal.

The exact Test262 manifest contains one path / two sloppy-and-strict variants
and hashes to
`3f4494005a5d8089fd9a9063aed01bed2b408bc9ae119043606a33aa82d400dc`.
Pinned QuickJS and the R3da candidate pass 2/2; the checksum-bound R3cz parent
records `fail-runtime=2` with catchable `InternalError: stack overflow`.
Candidate focused TSV/JSONL SHA-256 values are
`9c6c195196450e147231924d1ec548e6c2257c42ed062b26cd3fad5753a92f46`
and
`de4c08027b27b12aa0acd00a7bc5ab386dbe218dfb11ef59a48048da4dcc4718`.

The fresh two-worker 102,037-row join changes exactly those two
`fail-runtime -> pass` outcomes, leaves 102,035 non-cohort rows byte-identical,
has no detail-only movement, and records no previous-pass regression. The
canonical vector is now 65,511 passes / 65,566 runnable, `fail-parse=7`, and
`fail-runtime=46`; the two JSON mega-array variants still time out. Full
candidate TSV/JSONL SHA-256 values are
`b97744b88f1a46727b1073559d0640a09b61a9e0a32703dccc062f2d61387001`
and
`b28b8db0e45ba299ab2cc60e4b12f88856f864b8b3afd54c15d6f8c7e9f857d7`.

Reproduce the frozen inputs and focused evidence with:

```sh
./scripts/test-test262-generator-yield-star-stack-budget.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-generator-yield-star-stack-budget.sh
```

Reproduce the complete exact join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-generator-yield-star-stack-budget.sh --full`.

## R3cz class-field initializer await context

R3cz matches QuickJS 2026-06-04's
`js_parse_function_class_fields_init()` lexer-context boundary. Public/private,
instance/static field initializers are parsed in a synthetic normal-method
child: strict mode and the parent's module grammar parameter survive that
boundary, while the child must not inherit an enclosing async/generator
function's lexer flags. Computed keys remain in the enclosing
context because QuickJS parses them before entering the initializer child.
Oxide now switches both the compiler function and future lexer context at that
same boundary, then restores the parent without rescanning the already-read
field terminator. Compiler canaries and the QuickJS differential cover raw and
escaped `await`, synchronous arrows, all four field shapes, computed keys, and
neighboring static-block diagnostics.

The exact Test262 manifest contains one path / two sloppy-and-strict variants
and hashes to
`beea6c8fc86db377966dbe2454b23ef7c227bf07f66661d676b4cb1f323e7c3a`.
Pinned QuickJS and the R3cz candidate pass 2/2; the checksum-bound R3cy parent
records `fail-parse=2`. Candidate focused TSV/JSONL SHA-256 values are
`9312266f78a2734f1d83349c0d6d264b0eb1098ea8a1e921cf23ad49e895bafd`
and
`40a12b019d865c41f066d0c5f7330cabcd551604797e82bb6f2a3c15e5d00087`.
The global profile remains byte-identical at 126 feature tags and SHA-256
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`.

The fresh two-worker 102,037-row join changes exactly the two
`fail-parse -> pass` outcomes, leaves 102,035 non-cohort rows byte-identical,
has no detail-only movement, and records no previous-pass regression. The
canonical vector is now 65,509 passes / 65,566 runnable, `fail-parse=7`, and
`fail-runtime=48`; the two JSON mega-array variants still time out. Full
candidate TSV/JSONL SHA-256 values are
`e2c3127f1d07909579e0f9cab108b70ebdaf5555646bd47cd2c1d63768ec6c1e`
and
`c2d3379b16f6a39a99a1ba6f2d93d26b383dce1c287f8482517e2179546bdd1c`.

Reproduce the frozen inputs and focused evidence with:

```sh
./scripts/test-test262-class-field-await.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-class-field-await.sh
```

Reproduce the complete exact join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-class-field-await.sh --full`.

## R3cy Math.atanh numerical parity

R3cy replaces Rust's single-expression `f64::atanh` path with a
QuickJS-compatible, fdlibm-shaped evaluation. Inputs below `2^-28` return
directly, preserving tiny values and signed zero; the remaining finite domain
uses separate `|x| < 0.5` and near-one `log1p` forms on the positive magnitude
before restoring the input sign. Domain overflow, NaN, and the infinities at
`+/-1` retain the ECMAScript boundaries. This avoids the thousands-of-ULPs
loss that the previous negative near-one expression could introduce. Unit
tests lock the branch boundaries, domain behavior, signed zero, and selected
near-one values; the Math differential checks those values and special cases
against pinned QuickJS.

The exact Test262 manifest contains seven paths / 14 sloppy-and-strict
variants and hashes to
`ffd98f946fde17f8a0af13c9dd172c8aa2c476e96baaa9df86ae42ee5479b215`.
Pinned QuickJS and the R3cy candidate pass 14/14; the checksum-bound R3cx
parent records `pass=12 fail-runtime=2`. Candidate focused TSV/JSONL SHA-256
values are
`03129f451be73355a0b33d6d74930e63bea0a1a9f001a5a8c524b6654f761140`
and
`d7cd0e97acb5dcda64378dccff535c1cfae6271ed4cbd448d65737894b1d57c8`.
The global profile remains byte-identical at 126 feature tags and SHA-256
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`.

The fresh two-worker 102,037-row join changes exactly the two
`fail-runtime -> pass` outcomes, leaves 102,035 rows unchanged, has no
detail-only movement, and records no previous-pass regression. The canonical
vector is now 65,507 passes / 65,566 runnable, `fail-parse=9`, and
`fail-runtime=48`. Full candidate TSV/JSONL SHA-256 values are
`9009145c5b7033c4b4392022f97c73ab62efe4f78c4085e6b76a48f89a34ad76`
and
`edcd4d53c03e09c447eed001d0033a36ce85e0a2b510b63e0eedec9066c44e60`.

Reproduce the frozen inputs and focused evidence with:

```sh
./scripts/test-test262-math-atanh.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-math-atanh.sh
```

Reproduce the complete exact join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-math-atanh.sh --full`.

## R3cx for-of async member lookahead

R3cx matches QuickJS 2026-06-04's raw, non-committing
`simple_next_token(..., FALSE)` lookahead for `async of` in an ordinary for-of
head. The probe skips ordinary whitespace and JavaScript comments, but it is
deliberately not a normal lexer pass: Annex B HTML comments are left for the
real lexer and a backslash after raw `of` preserves QuickJS's scanner boundary.
Bare, unescaped `async` followed by that raw `of` probe remains a syntax error,
while complete member targets such as `async.x`, `async["x"]`, and `async.of`
parse normally. Compiler canaries cover block/line comments and newlines,
escaped and parenthesized `async`, the raw `of\u0061` continuation, both HTML
comment forms, and invalid call/optional-chain targets. The QuickJS
differential covers the accepted member and HTML-comment behavior.

The exact Test262 manifest contains one path / two sloppy-and-strict variants
and hashes to
`a4d8c570908bb500728aca7dad45b0e064d9f43394e5d8e9bece95be74bc40a5`.
Pinned QuickJS and the R3cx candidate pass 2/2; the checksum-bound R3cw parent
records `fail-parse=2`. Candidate focused TSV/JSONL SHA-256 values are
`1f1a5c30dde9ede5f58635ec2d3a15396dc988c9b2c378f5bd5db4fc6135a3e6`
and
`d343db38f5fdfc8ffc46b2a06acd04507e58692ab3879ea3451a6b5f3e9b5cc4`.
The global profile remains byte-identical at 126 feature tags and SHA-256
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`.

The fresh two-worker 102,037-row join changes exactly those two
`fail-parse -> pass` outcomes, leaves 102,035 non-cohort rows byte-identical,
has no detail-only movement, and records no previous-pass regression. The
canonical vector is now 65,505 passes / 65,566 runnable, `fail-parse=9`, and
`fail-runtime=50`. Full candidate TSV/JSONL SHA-256 values are
`687eec42e9611a377b37f68aa61cba263d2e8fe0dcf66d19b003f25b5a7746bb`
and
`9a8a8a645a890a3f56fb9f40001aa46f08b6b46009dd6a426873249a7611a46f`.

Reproduce the frozen inputs and focused evidence with:

```sh
./scripts/test-test262-for-of-async-member.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-for-of-async-member.sh
```

Reproduce the complete exact join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-for-of-async-member.sh --full`.

## R3cw RegExp exec recompilation ordering

R3cw matches QuickJS 2026-06-04's observable `RegExpBuiltinExec` order: first
validate the RegExp brand, then apply `ToString` to the input, then apply
`ToLength` to `lastIndex`, and only afterwards read the current compiled
bytecode and flags. Either coercion may reenter the legacy
`RegExp.prototype.compile()` method and replace the program. Oxide therefore
delays its program snapshot until both coercions have completed. A seven-vector
QuickJS differential covering replacement programs and flags, sticky removal,
captures, named groups, and match indices passes 7/7 in both engines.

The pinned Test262 cohort contains two paths / four sloppy-and-strict variants.
Pinned QuickJS and the R3cw candidate pass 4/4; the authenticated R3cv parent
records four `fail-runtime` outcomes. The manifest SHA-256 is
`2d272e6f86d0cb3f041e824008771750a833d30209971d6dbebc2c0598726aa3`;
focused candidate TSV/JSONL SHA-256 values are
`51bd65e7c991d8e371d263cf352cdd57dc2bb24e329b2032f4d28dd2eedafa10`
and
`86e7c93040d35347add9f1a209eb0de7d6dbf87f8d98ab9116f32a995319fb27`.
The global profile remains byte-identical at 126 feature tags and SHA-256
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`.

The fresh two-worker 102,037-row join changes those same four outcomes, leaves
102,033 rows byte-identical, has no detail-only movement, and records no
previous-pass regression. Its non-cohort TSV and JSON row streams are both
byte-identical. The canonical vector is 65,503 passes / 65,566 runnable;
`fail-runtime` falls from 54 to 50 while all 17,996 unsupported outcomes remain
unchanged. Full candidate TSV/JSONL SHA-256 values are
`cd5aa3df85c45b72a8939d9c5778c70192b1dc3699eb3330ff8f7aff0ef1159f`
and
`709f49e182e1cfb83353c46251d5eb0bbc24109c3690532f2f4e348d64f1664f`.

Reproduce the evidence with:

```sh
./scripts/test-test262-regexp-exec-recompilation.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-regexp-exec-recompilation.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-regexp-exec-recompilation.sh --full
```

## R3cv Array flat/flatMap global admission

R3cv globally admits the already-implemented `Array.prototype.flat` and
`Array.prototype.flatMap` surfaces. This is a profile and evidence milestone,
not a new runtime widening: the parent profile kept both metadata tags
fail-closed, while the candidate adds exactly those two tags and leaves the
1,197 audited negative paths and execution policy byte-identical. The profile
therefore grows from 124 to 126 feature tags. Parent and candidate profile
SHA-256 values are
`ff0a591164b267d06762bd5d5a41781d50cc8128377a3787e3c1ea13f7c30b1a`
and
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`.

The complete pinned metadata universe contains 35 paths / 69 variants. Pinned
QuickJS 2026-06-04 passes all 69. The authenticated R3cu parent records 69
`unsupported-feature` outcomes, while the R3cv candidate passes 69/69. The
manifest SHA-256 is
`867fe0a1303259a449e12d367c5c67d4409218c6ac0eb41a1a335326d89f1c6e`;
focused candidate TSV/JSONL SHA-256 values are
`02030ecd7daac3a3656d9bec6966145e2fd955d0e6c977bd4993faf38110aa7e`
and
`92507f9130e7bfb1b231d1ad40cbc622463858fb9707847540724257480ecefd`.

The exact 102,037-row R3cu-to-R3cv join changes those same 69 outcomes, leaves
101,968 rows byte-identical, has no detail-only movement, and records no
previous-pass regression. The canonical vector is 65,499 passes / 65,566
runnable; `unsupported-feature` falls from 13,788 to 13,719. Full candidate
TSV/JSONL SHA-256 values are
`4cec8ef8be4b432b6f754c07522e744af856bbd8c9ed32fb98fecfe41810c076`
and
`022ab0c11d55e70d2f08c7df7361a36b571bac91320f43d6edfe46e19dba4975`.
The residual classification is `fail-parse=11`, `fail-runtime=54`,
`skipped-config-exclude=6700`, `skipped-feature=11775`, `timeout=2`,
`unsupported-feature=13719`, `unsupported-host-agent=118`,
`unsupported-host-can-block-false=4`, `unsupported-host-is-html-dda=84`,
`unsupported-module=679`, and `unsupported-negative-provenance=3392`.

Reproduce the evidence with:

```sh
./scripts/test-test262-array-flatten-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-array-flatten-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-array-flatten-global.sh --full
```

## R3cu dynamic eval WTF-8 source preservation

R3cu makes String inputs to direct and indirect `eval` follow QuickJS
2026-06-04's reversible WTF-8 source-text semantics. Lone UTF-16 surrogates
cross the Rust `str` parser seam through a same-width marked carrier without
becoming U+FFFD or colliding with genuine private-use code points. String
literals, cooked and raw templates, RegExp literals, and saved debug source
recover the original UTF-16 units; debug source is retained as canonical
WTF-8, comments retain their tokenization, and identifier diagnostics retain
their locations. `Function.prototype.toString` therefore reproduces the exact
evaluated function source. Valid surrogate pairs retain their ordinary
canonical UTF-8 spelling. Dynamic `Function` and the Test262 host's
`$262.evalScript` remain separately typed frontiers.

The frozen pinned cohort contains 11 paths / 22 sloppy-and-strict variants.
Pinned QuickJS and the R3cu candidate both pass 22/22; the authenticated R3ct
parent records 22 `unsupported-runtime` outcomes. The manifest SHA-256 is
`3e4f73f980aae940fe3f81df608e5f32154d851c632535a58de89de728b31f2d`.
Focused candidate TSV/JSONL SHA-256 values are
`515e3b7056e86958fe3b7e265f717ce301e95245ed907f35cbeae7d5ff8c3859`
and
`3f36b9aa435cd8c29b58f6cb9f65a8a6b4a57fbb66ec588deacf13c6e1de6dca`.

The live 124-tag profile remains byte-identical at
`ff0a591164b267d06762bd5d5a41781d50cc8128377a3787e3c1ea13f7c30b1a`.
The exact 102,037-row R3ct-to-R3cu join changes those same 22 outcomes,
leaves 102,015 rows byte-identical, and records no detail-only change or
previous-pass regression. The canonical vector is 65,430 passes / 65,497
runnable; `unsupported-runtime` falls from 22 to zero. Full candidate
TSV/JSONL SHA-256 values are
`8cbb90ce01fcc2c887871d7de02cfb62a6588ff807e8604e27700823b99d5820`
and
`10cb9ef6db26da8150cf8f23222b0aad02ac7cee9326aab18ef56ca0ab272aa4`.
The residual classification is `fail-parse=11`, `fail-runtime=54`,
`skipped-config-exclude=6700`, `skipped-feature=11775`, `timeout=2`,
`unsupported-feature=13788`, `unsupported-host-agent=118`,
`unsupported-host-can-block-false=4`, `unsupported-host-is-html-dda=84`,
`unsupported-module=679`, and `unsupported-negative-provenance=3392`.

Reproduce the evidence with:

```sh
./scripts/test-test262-eval-wtf8-source.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-eval-wtf8-source.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-eval-wtf8-source.sh --full
```

## R3ct basic RegExp v CharacterClassEscape runtime

R3ct opens a deliberately narrow first `v`-flag runtime slice, following the
pinned QuickJS `unicode_sets` path through its basic class-atom construction.
The six `d`, `D`, `s`, `S`, `w`, and `W` escapes now work as atoms and inside
simple classes, including anchors, ordinary quantifiers, Unicode-width
matching, complements, and `iv` folding. Set operations, nested sets,
properties, strings, groups, disjunction, literals, and dot remain typed
`Unsupported`; malformed syntax inside the admitted slice remains a real
`SyntaxError`.

The complete pinned `CharacterClassEscapes` directory contains 12 paths / 24
sloppy-and-strict variants. Pinned QuickJS passes all 24. Oxide's authenticated
parent records 24 `unsupported-parser` outcomes, while the candidate passes
24/24. The manifest SHA-256 is
`45a7ee70a325e4f175c4cb3d021d9ba73180c2106058f694a0ff2ca40da36bc6`;
focused candidate TSV/JSONL hashes are
`b3db379e2fb33ac9a2042e35e81758c7dd76f6351cc944ec0660b79582922710`
and
`4acd63c554f26a10132139d21b45473b4e38646754545857258405e64436bbfa`.

The live Test262 profile remains byte-identical at
`ff0a591164b267d06762bd5d5a41781d50cc8128377a3787e3c1ea13f7c30b1a`.
In particular, `regexp-v-flag` is not admitted globally: these generated tests
omit that metadata tag, and broader `v` grammar remains fail-closed. The exact
102,037-row join changes only the same 24 outcomes, leaves 102,013 rows
byte-identical, and records no previous-pass regression. The canonical vector
is 65,408 passes / 65,497 runnable, and the residual `unsupported-parser`
category falls from 24 to zero. Full TSV/JSONL hashes are
`908f7e0a9dca5a0b7f7c4a154ecffce425a0998cf1c0e7c8830dbe35850599d7`
and
`9a128f5e3a901ddb50bb9e98a080dfe1355ec0d6ddad9fa9d6fc09c7501e7eb7`.

A release/2-worker recovery receipt additionally replays all 15
resource-sensitive paths / 30 variants exposed by a rejected high-contention
debug probe. All 30 pass and are byte-identical to their R3cs parent rows;
this keeps scheduler noise outside the canonical join. Reproduce the evidence
with:

```sh
./scripts/test-test262-regexp-v-character-class-escapes.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-regexp-v-character-class-escapes.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-regexp-v-character-class-escapes.sh --full
```

## R3cs future-reserved-word negative-test global admission

R3cs globally admits the 25 parse-negative paths / 32 variants authenticated
by R3cr's scoped receipt. This is a profile and evidence milestone, not a new
runtime change. Together with the activation and already-passing partitions,
the complete future-reserved-word universe remains 56 paths / 86 variants.
Oxide passes all 86 under the live global profile, and pinned QuickJS
2026-06-04 independently passes all 86.

The 124 feature tags and execution policy remain byte-identical. The audited
negative section grows from 1,172 to 1,197 paths, moving the profile SHA-256
from
`40e8669015c3ea00d2704b49e540947c0aa202fe22900b0dff84acb5da3b554e`
to
`ff0a591164b267d06762bd5d5a41781d50cc8128377a3787e3c1ea13f7c30b1a`.
The added-path and added-variant-key hashes are
`8bd18ff57c518d106de263d3b77ea56695fd6368e846afdabaaaab72033fd51f`
and
`d51615c929d874567d2a53789c0c671ebfc5c7792b55f51d170c6cbdcf16ff73`.
The focused parent records 54 passes and 32 fail-closed negative variants; the
candidate passes 86/86. Its exact join changes those 32 outcomes, leaves 54
rows unchanged, and has no detail-only movement.

Across the complete 102,037-row vector, the same 32 outcomes change, 102,005
rows remain byte-identical, and no previous pass regresses. The R3cs
scoreboard was 65,384 passes / 65,497 runnable, while
`unsupported-negative-provenance` fell from 3,424 to 3,392. Full candidate
TSV/JSONL hashes are
`1df77fd5d67b0ba585b3390cf0ce50a53f59226dfd57983edcc26d3c7a034dfe`
and
`257eef22e32ed8d5b1d6a837d07a82d7c1bf4263b996364000a1e98522f83138`.

Reproduce the evidence with:

```sh
./scripts/test-test262-future-reserved-words-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-future-reserved-words-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-future-reserved-words-global.sh --full
```

## R3cr future-reserved-word runtime parity

R3cr matches QuickJS 2026-06-04's Script/Eval treatment of the complete pinned
future-reserved-word cohort. Always-reserved `enum`, `export`, and `extends`
now produce real `SyntaxError` results in invalid statement and expression
positions while remaining valid IdentifierName property keys. `import` keeps
three boundaries distinct: malformed ImportCall grammar and Script/Eval
`import.meta` produce `SyntaxError`, but syntactically valid dynamic import
remains a typed `Unsupported` module-loading frontier.

That `Unsupported` result is deliberately deferred until the complete source
has parsed and identifier/private-name resolution has finished. A later syntax
or private-name early error therefore retains QuickJS priority instead of
being swallowed by the unimplemented dynamic-import runtime. Pinned QuickJS
passes the exhaustive 56-path / 86-variant cohort. Under the unchanged global
profile Oxide moves from 53 to 54 passes; the scoped profile audits all 26
negative paths and passes 86/86. The focused join changes one outcome, leaves
85 rows unchanged, and has no regression. The live 124-tag profile remains
`40e8669015c3ea00d2704b49e540947c0aa202fe22900b0dff84acb5da3b554e`.

Across the complete 102,037-row vector, the same one outcome changes, 102,036
rows remain byte-identical, and no previous pass regresses. The canonical
scoreboard is 65,352 passes / 65,465 runnable, and `unsupported-runtime` falls
from 23 to 22. Full candidate TSV/JSONL hashes are
`22203b1a0cdb51a76552ef4e999dde24c582f981f50fe85f9f8c12a0b17a6f7f`
and
`c009cbc3c65fdd617d33b488b47fd80c10cb703b269e034025facce1e5b1a470`.

Reproduce the evidence with:

```sh
./scripts/test-test262-future-reserved-words.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-future-reserved-words.sh
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3cq debugger negative-test global admission

R3cq globally admits the five negative paths / ten sloppy-and-strict variants
whose exact parse-phase `SyntaxError` provenance was authenticated by R3cp.
This is a profile and evidence milestone, not a new runtime change. The full
ten-path / 20-variant `debugger` cohort now passes in both Oxide and pinned
QuickJS 2026-06-04.

The live profile retains 124 feature tags and its execution policy while its
audited-negative section grows from 1,167 to 1,172 paths. The profile SHA-256
is
`40e8669015c3ea00d2704b49e540947c0aa202fe22900b0dff84acb5da3b554e`.
The focused join changes exactly those ten negative variants, leaves the other
ten cohort rows unchanged, and records no detail-only movement.

Across the complete 102,037-row vector, exactly the same ten outcomes change,
102,027 rows remain byte-identical, and no previous pass regresses. The
canonical scoreboard is 65,351 passes / 65,465 runnable; full candidate
TSV/JSONL hashes are
`91bad0c048a1d90a76346a41dd2676ae5a530b8ad787c30292bd2f7c956e573a`
and
`40c39453be1b9e7cbc912fd841442a0e81cbab650b568a44b765168424433583`.

Reproduce the evidence with:

```sh
./scripts/test-test262-debugger-statement-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-debugger-statement-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-debugger-statement-global.sh --full
```

## R3cp debugger statement runtime parity

R3cp implements QuickJS 2026-06-04's `debugger` statement semantics. QuickJS
has no debugger hook in this path: parsing advances past the keyword, applies
ordinary automatic semicolon insertion, and emits no bytecode. Oxide now does
the same in Script, Eval, function bodies, labels, and single-statement
contexts. Because the statement is a no-op, eval preserves the last non-empty
completion value. `debugger` remains reserved outside statement grammar, while
escaped IdentifierName property and method uses remain valid.

The exhaustive pinned cohort contains ten paths / 20 variants, and pinned
QuickJS passes all 20. Under the unchanged global profile Oxide passes ten
variants; the exact scoped profile authenticates the five negative paths and
passes all 20. The focused runtime transition repairs the two sloppy/strict
`debugger`-statement variants and leaves the other 18 rows unchanged.

Across the complete 102,037-row vector, exactly those two outcomes change,
102,035 rows remain byte-identical, and no previous pass regresses. The
canonical scoreboard is 65,341 passes / 65,455 runnable, while
`unsupported-parser` falls from 26 to 24. Full candidate TSV/JSONL hashes are
`362690ef82273724b8a5a24247e7529060051e63a5a43671d37e30909da0f779`
and
`b61846b93d222f52ded5dd28c1a849c566dceb7d855d49e3e2a8f899046cff13`.

Reproduce the evidence with:

```sh
./scripts/test-test262-debugger-statement.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-debugger-statement.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-debugger-statement.sh --full
```

## R3co Annex B HTML-like-comment global admission

R3co globally admits the ten negative Script paths / 17 variants whose parse
or runtime error provenance was authenticated in R3cn. This is a profile and
evidence milestone, with no new engine semantics: 13 variants exercise the
expected runtime error and four the expected parse error. The complete
HTML-like-comments universe remains 19 paths / 32 variants, and pinned QuickJS
passes all 32. Oxide now passes 29 globally, with only the three Module
variants still unsupported.

The live profile retains the same 124 feature tags and execution policy while
its audited-negative section grows from 1,157 to 1,167 paths. Its SHA-256 is
`1a85d1b9b43c54825c1a435011be737593ccc9754753daabdd255f9bd078bf7a`.
The focused transition changes exactly the 17 admitted outcomes and leaves the
other 15 rows unchanged.

Across the complete 102,037-row vector, the same 17 outcomes change, 102,020
rows remain byte-identical, and no previous pass regresses. The canonical
scoreboard is now 65,339 passes / 65,455 runnable. Full candidate TSV/JSONL
hashes are
`2502eda033dc3a91c64ddaab00093af254bead7c2dd15b13060b6b6088b5c1a7`
and
`062115b6363fb8ea49ed7240c80bfcb6fd035e94f34d2ff8365284cd75844302`.

Reproduce the evidence with:

```sh
./scripts/test-test262-html-comments-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-html-comments-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-html-comments-global.sh --full
```

## R3cn Annex B HTML-like comments

R3cn enables the existing typed lexer implementation of Annex B HTML-like
line comments for Script and Eval roots. `<!--` begins a line comment wherever
it is encountered; `-->` does so only at the start of the source or after a
LineTerminator. Strict scripts retain the extension. The option remains off by
default so a future Module root cannot accidentally inherit Script grammar.
Direct and indirect eval use the same compiler boundary, while the existing
QuickJS-shaped dynamic-Function wrapper supplies the line breaks that decide
parameter/body behavior. No production code was added to `runtime.rs`.

The exact pinned universe contains 19 paths / 32 variants: five runtime
activation paths / ten variants, one already-passing Function boundary / two
variants, ten negative Script paths / 17 variants, and three Module variants.
Pinned QuickJS passes all 32. Under the unchanged global profile Oxide now
passes 12, keeps 17 unaudited negatives fail-closed, and leaves all three
module rows unsupported. A separate manifest-bound scoped profile audits only
those ten negative paths and passes 29/32, with only modules excluded.

The focused parent TSV/JSONL hashes are
`241c5d403f78728a4c1caf5b11220f8c3d7224e6fef2ad56c91b9892df996224`
and
`31f272c60ee8261ee4e915715aa40474abd6fc3417c8e2ff3b3c9e31c57d3eb0`;
the global candidate hashes are
`5e434c45148a97ff8c94b68601e42c306eb50c57e28a32b450195b8e07261d67`
and
`32f41a3626ff7ed81209fa33b63457ff7634b81e9f17943a5e867ebab970bf03`.
The exact transition has ten outcome changes and 22 unchanged rows. The
scoped 29-pass TSV/JSONL hashes are
`d0ff8ffe6899c5006d2068c351f9bc1a36d72c37994febd515fce246c74c7389`
and
`966843c8af0a76e43d4dcdc68a645fa541171929a5ec5779ba6be983e5f5982d`.

The global profile remains byte-identical at 124 tags and SHA-256
`ef17b52324782431adc1ddbabc81530de3e24fb436545202f248d850a1043dbb`.
The actual 102,037-row join records the same ten repairs, 102,027 unchanged
rows, and zero prior-pass regressions. The canonical vector is now 65,322
passes / 65,438 runnable; `fail-runtime` falls from 64 to 54. Full parent
TSV/JSONL hashes are
`d404fdd6e1fa7e9f19703bbdbc49bd55fddb83b744d30254349087f0a26568d5`
and
`2196ac6f9ca0c6f251ae0ee8987ea5351c7be076188e5b81c82055f2b2d86188`;
candidate hashes are
`abd85c73e941a35a990069c619e1164d1a785f537057ff5f3e1b70ab434a0c07`
and
`691713498774972a6539dcd6506c66be0eb4aa397bc04141d55f86594c816e3f`.

Reproduce the evidence with:

```sh
./scripts/test-test262-html-comments-runtime.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-html-comments-runtime.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-html-comments-runtime.sh --full
```

## R3cm Promise proposal tag admission

R3cm globally admits the pinned Test262 `promise-try` and
`promise-with-resolvers` metadata tags. This milestone changes no runtime
code: `Promise.try` and `Promise.withResolvers` were already implemented and
covered by the earlier 112-path Promise static-method gate. Its frozen Oxide
receipt remains 224/224 pass, with TSV/JSONL SHA-256 values
`350e8f80d30a1942e44595c1e771b5e0008fd33aa2f93d6d2345e219d5bb6968`
and
`4058a876e0f05e0ff0b07d6ae6a5b4886ea9dca3ebbe178c758221aa371df6ca`.

The metadata-derived global universe contains 21 paths / 39 variants. It
partitions into 16 activation paths / 32 variants, two class-dependent paths /
four reason-only variants, and three top-level-await module variants. Pinned
QuickJS passes all 39. Under the candidate profile Oxide passes all 32
activation variants; the class variants retain only their independent `class`
reason, and the module variants remain byte-identical. The focused transition
therefore has 32 outcome changes, four detail-only changes, and three
unchanged rows. Parent TSV/JSONL hashes are
`f1aea0fffe03dde4746c2012e63e68c4113d8f6f89fa22dacb01feec7d4f1d0a`
and
`65f8aba6981fd95d1fafa85cb3b1908cd3f4290af7673b53cb5269db247df296`;
candidate hashes are
`9b3b2b58d6c7d064b0917c18aac7037a1849fe2339700d994ec6870b464d3f5e`
and
`f7163c2edc7d100e953d4b7d1c186fa6ee34d7c4d1bf3e32123349b9235d3885`.

The global profile grows from 122 to 124 sorted feature tags, with no change
to audited negative paths or execution policy. Its SHA-256 is
`ef17b52324782431adc1ddbabc81530de3e24fb436545202f248d850a1043dbb`.
The exact 102,037-row join records 65,312 passes and 65,438 runnable variants:
32 new passes, four reason refinements, 102,001 unchanged rows, and zero prior
pass regressions. Full parent TSV/JSONL hashes are
`ef3b88f82d4e65f55b584731f1cf78e7b734baf467639a6e18028f405c77ee56`
and
`81d1071fe7dc47e0e2a874641bea28bc5b707d17690c764194231a838de75d66`;
candidate hashes are
`d404fdd6e1fa7e9f19703bbdbc49bd55fddb83b744d30254349087f0a26568d5`
and
`2196ac6f9ca0c6f251ae0ee8987ea5351c7be076188e5b81c82055f2b2d86188`.

Reproduce the evidence with:

```sh
./scripts/test-r3n-promise-static-oracle.sh --oxide target/debug/qjs
./scripts/test-test262-promise-try-with-resolvers-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-promise-try-with-resolvers-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-promise-try-with-resolvers-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3cl Unicode locale comparison

R3cl completes QuickJS 2026-06-04's two-function Unicode String extension by
adding `String.prototype.localeCompare` immediately after `normalize`. This is
the pinned non-Intl implementation: it coerces the receiver and `that` in that
order, never observes `locales` or `options`, NFC-normalizes both values with
the shared Unicode 17 kernel, and compares UTF-32 code points. It returns the
raw first code-point difference rather than reducing every result to -1 or 1;
only a proper-prefix comparison returns +/-1. Valid surrogate pairs are
decoded and lone surrogates remain code points with their code-unit values.
The generic method has `length=1`, is not constructible, and preserves
defining-realm conversion and allocation errors. This does not claim Intl
collation parity; the pinned configuration excludes the ten `intl402`
`localeCompare` paths.

The checksum-bound gate contains all 13 direct paths plus two supplemental
descriptor/nullish-receiver paths, for 15 paths / 30 variants. Pinned QuickJS
and Oxide both pass 30/30. The R3ck parent had 26 `fail-runtime` outcomes and
four outcome-level false passes; the exact focused join changes only those 26
failures to passes. Parent TSV/JSONL SHA-256 values are
`95b594ce9d6219b51681b77bab86e4b82ae79e4e2b6f839b36af489d5ff0f43c`
and
`ac8bc91d74eb602e2789b88f62ab1fde2e19a3cda8eca3e64c32c7173c38db4d`;
candidate values are
`677848008880a63d0c7decd351d96afc9c1668d9ca8c952f814e05ba1853b937`
and
`a4303ea1d66064561d0192d1828e81b5f96bec12fab2628e12ebc269199b1dc6`.
The transition hashes to
`5abf0cf81924a88204791b35eb990b8a5d0930cee03aabf6e33da399ae941e84`.

The Test262 profile remains byte-identical at 122 tags and SHA-256
`1e39c157e444f60f0a44f4fd373ad63147d814986cde5f08c4f5b33d8f5839a2`.
The canonical 102,037-row join changes the same 26 outcomes, leaves 102,011
rows byte-identical, and has zero prior-pass regressions. It records 65,280
passes with 65,406 runnable variants; `fail-runtime` falls from 90 to 64. The
full parent TSV/JSONL hashes are
`f491512281647b752796da1abe8fcf559981b48a53270bf128e9b698ade60c3f`
and
`d65c1fbb9f17bc1666b2dbd0c228843a33147d4f762f7c18aa9491e883c3c59a`;
the candidate hashes are
`ef3b88f82d4e65f55b584731f1cf78e7b734baf467639a6e18028f405c77ee56`
and
`81d1071fe7dc47e0e2a874641bea28bc5b707d17690c764194231a838de75d66`.
Allocator-failure recovery is covered, while byte-for-byte temporary
allocation topology remains outside this semantic receipt.

Reproduce the evidence with:

```sh
./scripts/test-test262-string-locale-compare.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-string-locale-compare.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-string-locale-compare.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3ck Unicode string normalization

R3ck implements `String.prototype.normalize` as an unsafe-free Rust port of
QuickJS 2026-06-04's Unicode 17 normalization data and algorithm. NFC, NFD,
NFKC, and NFKD preserve QuickJS's UTF-16 behavior, including lone surrogates,
coercion order, error types, defining-realm errors, recursion limits, and
allocation-failure recovery. This is parity for the normalization intrinsic,
not yet for the rest of the String surface.

The checksum-bound gate combines all 18 direct normalize paths with one
supplemental receiver-error path, for 19 paths / 38 variants. Pinned QuickJS
and Oxide both pass 38/38. The historical parent had 20 `fail-runtime` rows
and 18 outcome-level false-passes caused by missing-method errors, guards, or
property enumeration. The exact focused join changes those 20 outcomes to
`pass` and deliberately reruns the other 18 unchanged rows under the
implemented intrinsic. The focused parent TSV/JSONL SHA-256 values are
`4ef7519798294a023d7cefa1af595945fcfab49060639d49a23271fb9e8b35ad`
and
`9533e4e935a9a77dc8444ea761e06daeca49684a143583c8c784dc040c7d4353`;
the candidate values are
`22a3aa4192be516cd5ca6eb0ce7c69325ab6ccf7cb7619892726015f8051d2a7`
and
`b613d9f29d75e67b41561ad2b7e29d8a6e89f933b02e70e5a73b07e9f82283fb`.

Test262 has no normalize feature tag, so the live 122-tag profile is unchanged
at SHA-256
`1e39c157e444f60f0a44f4fd373ad63147d814986cde5f08c4f5b33d8f5839a2`.
The exhaustive 102,037-row join has the same 20 outcome changes, 102,017
unchanged rows, and zero prior-pass regressions. The canonical vector is now
65,254 passes with 65,406 runnable variants. The full parent TSV/JSONL hashes
are
`acd43fe1eb9752246e9994c58c3f139ceff0c5e80416baea06757428e5ba6bba`
and
`c1a4bf7cc058a70b6b97475fccc92700403a19c63936c341ea3a6ebe79e4f34a`;
the candidate hashes are
`f491512281647b752796da1abe8fcf559981b48a53270bf128e9b698ade60c3f`
and
`d65c1fbb9f17bc1666b2dbd0c228843a33147d4f762f7c18aa9491e883c3c59a`.

Reproduce the evidence with:

```sh
oracle=$(./scripts/build-quickjs-oracle.sh)
./scripts/check-unicode-normalize-fingerprint.sh "$(dirname "$oracle")"
./scripts/test-test262-string-normalize.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-string-normalize.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-string-normalize.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3cj binary-data Test262 admission

R3cj globally admits the 18 residual metadata names for the implemented
numeric binary-data surface: eight concrete `DataView.prototype` operations
and ten concrete numeric TypedArray constructors. The profile grows from 104
to 122 sorted features without changing the 1,157 audited negative paths or
the async execution policy. Its SHA-256 is
`1e39c157e444f60f0a44f4fd373ad63147d814986cde5f08c4f5b33d8f5839a2`.

The metadata-derived inventory contains exactly 200 paths / 400 variants. Its
193-path activation combines 141 paths already authenticated by the earlier
DataView and TypedArray gates with 52 supplemental paths audited here. Pinned
QuickJS and Oxide both pass all 386 activation variants, with no activation
exclusions. Five paths / ten variants retain independent missing features and
two paths / four variants remain pinned configuration skips.

The complete parent/candidate join produces 386 new passes and ten detail-only
reason refinements, leaves 101,641 rows byte-identical, and has zero prior-pass
regressions. The R3cj candidate vector was 65,234 passes with 65,406 runnable
variants out of 102,037. Its TSV/JSONL SHA-256 values are
`acd43fe1eb9752246e9994c58c3f139ceff0c5e80416baea06757428e5ba6bba`
and
`c1a4bf7cc058a70b6b97475fccc92700403a19c63936c341ea3a6ebe79e4f34a`.

Reproduce the evidence with:

```sh
./scripts/test-test262-binary-data-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-binary-data-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-binary-data-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3ci recursive Test262 realm hosts

R3ci implements QuickJS 2026-06-04's Test262-only `$262.createRealm` and
`$262.evalScript` hooks. A created realm is a new context in the same runtime;
its returned `$262` object keeps that context alive and recursively exposes
`createRealm`, `evalScript`, `detachArrayBuffer`, `gc`, `codePointRange`, and
`global`. Host functions are non-constructible and defining-realm bound.
`evalScript` preserves its realm's global and lexical state, returns the raw
completion value, and does not implicitly drain pending jobs. The runtime gate
also covers nested realms, exception prototypes, realm-specific intrinsics,
job provenance, and collection after the last exported realm object dies.

The source audit finds 281 direct `createRealm` paths / 545 variants. Pinned
QuickJS passes the 80-path / 152-variant oracle envelope. Oxide passes the
formally admitted 79 paths / 150 variants; the two excluded staging variants
also require the still-unimplemented Atomics/SharedArrayBuffer surface. The
remaining direct paths are kept visible as 340 reason-only variants, 22 config
exclusions, and 33 config feature skips. The independent `evalScript` audit
contains 31 paths / 44 variants, all of which pass in pinned QuickJS and Oxide.

Global admission adds exactly `host-create-realm-required` and
`host-eval-script-required` to the prior 102-feature profile. The 104-feature
candidate retains all 1,157 audited negative paths and the same async policy;
its SHA-256 is
`01f936b9f5e0b920f10119a73f7e8ea52450863f113fff6542f3f241ed914d75`.
The exhaustive source union has 312 paths / 589 variants. It includes the
entire direct host universe, not just the 110 activation paths, so the gate
also authenticates every residual-feature diagnostic.

Three exact joins separate runtime installation from profile admission:

- historical R3ch to the current-runtime parent reclassifies 534 old host
  selections as fail-closed feature gaps without changing pass or runnable
  counts;
- runtime parent to candidate adds 194 passes and changes only the remaining
  feature detail on 340 variants; 55 config-selected variants are unchanged;
- historical R3ch to candidate has 534 outcome changes and no other changed
  row.

The complete join changes exactly those 534 rows, leaves the other 101,503
byte-identical, and has zero prior-pass regression. The R3ci candidate vector
was 64,848 passes with 65,020 runnable variants out of 102,037. Its TSV/JSONL
SHA-256 values are
`2f40849011fae4f96455225e467c817c6aeeaf3cc90722d357a1d8bdddbbf3bc`
and
`e6c18b7d9f6ef3f42bbf86ab396b91fb64773640e932581940f43cb9754509a1`.

Reproduce the evidence with:

```sh
./scripts/test-test262-create-realm.sh --check
./scripts/test-test262-create-realm.sh
./scripts/test-test262-eval-script.sh --check
./scripts/test-test262-eval-script.sh
./scripts/test-test262-realm-hosts-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-realm-hosts-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-realm-hosts-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

SharedArrayBuffer/Atomics, agent coordination, HTMLDDA, modules, and the broader
engine surface remain explicit parity frontiers.

## R3ch reentrant Test262 host GC

R3ch implements QuickJS 2026-06-04's Test262-only `js_gc` callback as a real
native `$262.gc` function. It is realm-bound, non-constructible, has
`name="gc"` and `length=0`, runs collection synchronously, returns `undefined`,
and does not drain the pending-job queue. Active ordinary, generator, and job
frames keep their receiver, arguments, locals, closures, and callable roots
while collection re-enters the runtime.

An exact QuickJS/Oxide lifecycle transcript covers WeakRef death, WeakMap
ephemeron cleanup, FinalizationRegistry's two-pass cycle handling, and Promise
and finalizer FIFO order. The checksum-bound Test262 inventory contains 15
paths / 28 variants. Pinned QuickJS passes all 28; Oxide now executes and
passes the 14 paths / 26 variants whose only missing dependency was host GC.
The remaining DataView path / two variants still requires `createRealm` and
keeps that independent host classification.

The global profile adds only `host-gc-required`, growing from 101 to 102
features with SHA-256
`c671ae022251a9a0f7d17cc851db7506d825c34854c69adedc6475d3da0f389f`.
Against the preceding canonical 102,037-row vector, exactly 26
`unsupported-host-gc` outcomes become passes, the two `createRealm` rows only
drop `gc` from their residual diagnostic, and the other 102,009 rows are
byte-identical. There is no previous-pass regression. The canonical vector is
now 64,654 passes with 64,826 runnable variants; its TSV/JSONL SHA-256 values
are `8e5c370f57e8d7dcd813df7199c79d210bf82316e802219c6d8a982dab72ac58`
and `f5270e02f19cfb1ab5fc7a5ba5020e15a1ee0cea947914d7656766af0e8a721e`.

Reproduce the evidence with:

```sh
./scripts/test-host-gc-reentrant-oracle.sh --oxide
./scripts/test-test262-host-gc.sh
./scripts/test-test262-host-gc-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-host-gc-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-host-gc-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

`createRealm`, the broader cross-realm host surface, the independent `for-of`
tag, and the rest of the engine remain explicit parity frontiers.

## R3cg global WeakRef and FinalizationRegistry admission

R3cg promotes exactly `WeakRef` and `FinalizationRegistry` from the focused
R3cf candidate into the live Test262 profile. The profile grows from 99 to 101
sorted feature tags while retaining all 1,157 audited negative paths and the
same execution policy; its SHA-256 is
`8be6c2a3892a62d89ed17df3f3d3b54e9e84fda8ef6be2bcdaa7d49044593990`.
The pinned runner permits both frozen profiles only with the checksum-bound
82-path manifest or an explicit full-suite `--all`, so an unreviewed manifest
or unrelated profile cannot reproduce the admission.

Pinned QuickJS 2026-06-04 passes all 164 universe variants. The exact focused
join changes 158 `unsupported-feature` outcomes to passes, changes only the
remaining-feature detail on the two `for-of` variants, and leaves four
`createRealm` host-capability rows byte-identical. The full 102,037-row join
contains those same 160 changes, keeps the other 101,877 rows byte-identical,
and has no previous-pass regression. The checked-in focused transition receipt
has SHA-256
`dd7080494f0d628aec4ab45bb793228cca52bebd208e4177ff308dd682b7c5af`.

The new canonical vector is 64,628 passes with 64,800 runnable variants and
13,866 `unsupported-feature` outcomes. Its TSV/JSONL SHA-256 values are
`c919dd56fc37f2946d729ee9a9a6958fc91c3f95366843ffae258953145e5a4f`
and
`342c22edd7cfdc4edf2b5085455c8586095bb4abc5b59d55cc4657c5ff954459`.
The parent full reports are byte-identical to the preceding R3ce canonical
receipts
(`e0b0be534f07a34bc7a9e18f4c3bae8c9360dd62c89176f96bf3234c5895b6ec` /
`8227cb6d19fc2f814bdb016308cf1003be6c91ebe01145ccc3c719f6e38ac6bf`),
proving that the runtime milestone did not alter any result hidden by the
former global feature boundary.

Reproduce the evidence with:

```sh
./scripts/test-test262-weak-ref-finalization-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-weak-ref-finalization-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-weak-ref-finalization-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

The independent `for-of` tag, Test262 `createRealm`/GC host hooks, broader heap
teardown OOM topology, and the rest of the engine remain explicit parity
frontiers. This is a stronger Feature Parity receipt, not a completion claim.

## R3cf WeakRef and FinalizationRegistry runtime

R3cf implements genuine `WeakRef` and `FinalizationRegistry` objects and keeps
the public surface in a dedicated intrinsic module. Object and non-registered
Symbol targets use non-owning generational identities. FinalizationRegistry
holds callback, creation realm, and held values strongly while targets and
unregister tokens remain weak; `unregister` removes every matching cell without
introducing an allocation boundary.

The heap now keeps WeakMap, WeakSet, WeakRef, and FinalizationRegistry in one
construction-order list. A single forward weak pass therefore reproduces
pinned QuickJS's observable one-versus-two-GC behavior. Dead registrations
transfer pre-owned roots into the runtime-wide Promise/finalization FIFO only
after queue capacity is reserved. Reservation failure silently drops the
cleanup job, and runtime teardown skips weak removal from the start, matching
the upstream no-exception and teardown paths.

Pending-job root retention and release now use a fixed, allocation-free root
set. `PendingJobOutcome` also distinguishes an empty queue from a job that ran
after its realm's last non-job owner disappeared; the latter reports
`Executed { context: None }`, matching QuickJS's successful return with a null
`pctx`. Broader heap teardown GC still uses allocation-backed worklists, so
fault-injected OOM topology there remains an explicit whole-engine parity gap.

The checksum-bound Test262 universe contains 82 paths / 164 variants. Pinned
QuickJS 2026-06-04 passes all 164. A candidate profile whose only feature delta
is `WeakRef` plus `FinalizationRegistry` activates 79 paths / 158 variants, all
of which pass in Oxide. One path / two variants remains behind the independent
`for-of` feature, and two paths / four variants remain behind the Test262
host's `createRealm` capability. The focused report SHA-256 is
`5ff2b92a694f71b63ab5b883e6c9416e2810c7230e26d36fcaec5f5815b20fe6`.

Reproduce the evidence with:

```sh
./scripts/test-test262-weak-ref-finalization.sh --check
./scripts/test-test262-weak-ref-finalization.sh
```

The cohort intentionally contains no host-GC tests. Rust heap/runtime tests,
checked against the pinned QuickJS source and manual oracle probes, therefore
cover dereference clearing, mixed-list pass order, registration transfer,
Promise/finalizer FIFO order, callback exceptions, Symbol ownership, silent
reservation failure, and teardown. The global 99-feature profile remains
unchanged in this runtime milestone; admission is a separate audited change,
and neither milestone claims whole-engine parity.

## R3ce global WeakMap and WeakSet admission

R3ce promotes exactly `WeakMap`, `WeakSet`, `symbols-as-weakmap-keys`, and
`upsert` into the global Test262 profile after the R3cd runtime landed. The
profile now contains 99 reviewed feature tags and the same 1,157 audited
negative paths; its SHA-256 is
`3b6c3316992b60644867d76799995ea7005c6c586438064072b017f7c3bd44ef`.
This is a capability-boundary and evidence change, not a runtime shortcut.

Pinned metadata derives a 154-path / 306-variant four-tag universe. Exactly
147 paths / 292 variants become runnable and pass, while seven paths / 14
variants remain fail-closed behind `WeakRef` and/or `FinalizationRegistry`.
The established Weak collections certificate covers 117 activation paths / 233
variants, the Map certificate covers another 26 / 52, and a disjoint four-path
/ seven-variant supplement closes the activation set. Pinned QuickJS
2026-06-04 independently passes all 306 variants.

The exact 102,037-row join changes 292 outcomes from `unsupported-feature` to
`pass`, changes only the remaining-feature detail on 14 rows, keeps the other
101,731 rows byte-identical, and records zero previous-pass regression. The
canonical vector is now 64,470 passes with 64,642 runnable variants and 14,024
`unsupported-feature` outcomes. Its TSV/JSONL SHA-256 values are
`e0b0be534f07a34bc7a9e18f4c3bae8c9360dd62c89176f96bf3234c5895b6ec`
and
`8227cb6d19fc2f814bdb016308cf1003be6c91ebe01145ccc3c719f6e38ac6bf`.

Reproduce the evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-weak-collections-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-weak-collections-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

The later R3cf milestone implements the WeakRef and FinalizationRegistry
surface; host-GC observation, `createRealm`, and global-profile admission remain
separate frontiers. This advances the Feature Parity evidence; it is not a
completion claim.

## R3cd WeakMap and WeakSet runtime

R3cd implements genuine QuickJS-shaped `WeakMap` and `WeakSet` objects rather
than admitting their names through the capability profile. Object keys and
non-registered Symbol keys use non-owning generational identities; WeakMap
values remain strong edges. The heap uses expected-O(1) hash storage, prunes
weak records in QuickJS construction order, and preserves QuickJS's explicit
GC lifetime boundaries. Constructors cache their adder and iterator `next`,
perform IteratorClose at the same abrupt-completion boundaries, select the
`new.target` realm fallback, and expose `getOrInsert` /
`getOrInsertComputed` with QuickJS-compatible reentrancy.

The checksum-bound source universe contains 264 paths: the 231-path built-in
core plus every metadata-tagged WeakMap/WeakSet consumer, including 33 paths
outside that core. Seven paths retain explicit unmet dependencies (two
`cross-realm`, two `host-gc-required`, and three WeakRef/FinalizationRegistry
paths), leaving 257 paths / 513 variants. Oxide and pinned QuickJS 2026-06-04
both pass all 513. Separate Rust/QuickJS vectors cover the constructor graph
and descriptors, object and Symbol keys, iterator close, upsert reentrancy,
brand errors, stale-key mutation, and cross-realm fallback. A 100,000-key heap
test protects the deep-WeakMap complexity boundary.

Without changing the 95-tag global profile, the complete 102,037-variant run
moves from 63,831 to 64,178 passes. `fail-runtime` falls from 400 to 110 and
all 57 former `harness-error` rows become passes; those include every variant
from the separately frozen 29-path TypedArray harness audit. All other summary
categories and the 64,350 runnable count are unchanged. The canonical
TSV/JSONL SHA-256 values are
`a7dbb819f224c1710843dab51033c4c32e7eb5c47cbad272e53b77031eb9babd`
and
`73249b49ff9f4081c8de1f9f3ca802de8eac6506c2b2c4dd8152f939832b5eaa`.
Of the expanded focused manifest, 280 variants already pass under the
conservative global profile and 233 remain classified as
`unsupported-feature`; promoting the weak-collection tags is a separate
admission milestone, not hidden inside this runtime change.

Reproduce the evidence with:

```sh
./scripts/test-test262-weak-collections.sh
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

This historical milestone advanced the Feature Parity implementation and raw
full-suite behavior. R3cf later implements WeakRef and FinalizationRegistry;
host-GC and cross-realm Test262 hooks remain separate work.

## R3cc global object-rest admission

R3cc promotes the complete pinned `object-rest` metadata tag while separately
auditing nine tag-external syntax companions. The live profile now contains 95
reviewed features and 1,157 audited negative paths, preserves async execution,
and has SHA-256
`f229cd652dd5b38ed3a0387a089eab974148d404bd166e8b4c0eb2cb0fa7a2c1`.
It adds exactly `object-rest` and three non-module parse-negative paths; the
tagged `import.meta` negative remains honestly `unsupported-module`.

The tag contains 355 paths / 707 variants: 282 / 562 become passes, 72 / 144
retain other feature dependencies, and one module row is unchanged. Existing
binding and assignment gates cover a disjoint 53 paths / 105 activation
variants; a 229-path / 457-variant supplement closes the activation set.
Oxide passes all 562 activated variants and pinned QuickJS 2026-06-04 passes
all 707 tag variants. The nine companions contribute 18 unchanged global
rows: eight passes, two residual private-class-field outcomes, and eight
QuickJS-config exclusions. Pinned QuickJS independently passes all ten
non-config companion variants.

The complete 102,037-key join records 562 outcome changes, 144 diagnostic-only
changes, 101,331 unchanged rows, and zero previous-pass regressions. All
101,330 rows outside the tag, including the explicitly extracted companion
rows, are byte-identical. The canonical vector is 63,831 passes, 64,350
runnable variants, 14,316 `unsupported-feature` outcomes, 3,451
`unsupported-negative-provenance` outcomes, and 19,261 total unsupported. Its
TSV/JSONL SHA-256 values are
`2cf5a7da27e028c4b3d5d91e8f1df43b25fb133714f0cd1ac2bfe64bc2726ac2`
and
`665f8c066abb3e894a4c80e86ed0f25dffff14c46b651e2e89e63faecf2cf473`.

Reproduce the evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-object-rest-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-object-rest-global.sh
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

This is a checksum-bound admission for already implemented object-rest
semantics, not a claim that Feature Parity is complete.

## R3cb global DataView admission

R3cb promotes the complete pinned `DataView` tag into the checksum-bound live
profile. The profile now contains 94 reviewed features and 1,154 audited
negative paths, preserves the async-execution policy, and has SHA-256
`b51eee39825e3325effab1c326df30b999e636f67c8ce7bb800f0afdc2d8eab4`.
This is an evidence admission for the QuickJS-shaped DataView implementation
landed in R3ao, not a new runtime shortcut.

The exhaustive metadata universe is 190 paths / 380 variants: 98 paths / 196
variants activate, 79 / 158 retain other unsupported dependencies, and 13 /
26 remain QuickJS-config skips. The all-green 492-path DataView gate directly
covers 174 activation variants; a checksum-bound 11-path supplement covers the
other 22. Oxide and pinned QuickJS 2026-06-04 pass all 196 activation variants.

The complete tag transition has 196 `unsupported-feature` to `pass` changes,
158 diagnostic-only changes, and 26 unchanged skips. The full 102,037-key join
keeps every one of the 101,657 non-universe rows byte-identical and records no
previous-pass regression. The canonical vector is now 63,269 passes, 63,788
runnable variants, 14,878 `unsupported-feature` outcomes, 3,451
`unsupported-negative-provenance` outcomes, and 19,823 total unsupported.
Its TSV/JSONL SHA-256 values are
`324e9d64423494796a9403a7f799f29075a2a98be9d705f7d8310cfb1707bff4`
and
`6b68da27cf87198da2c4f2db4e99d1af54b54df2bb936e7d33320f27acee147b`.

Reproduce the evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-data-view.sh
TEST262_WORKERS=8 ./scripts/test-test262-data-view-global.sh
TEST262_FULL_WORKERS=8 ./scripts/test-test262-data-view-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3ca global default parameters admission

R3ca promotes the complete R3bz `default-parameters` certificate into the
live checksum-pinned profile and closes its untagged early-error collateral.
The profile now contains 93 reviewed feature tags and 1,154 audited negative
paths, preserves the async-execution policy, and has SHA-256
`63f139b1a74da9a6114180593770dbcc86bb84fbafab5731f59e1387175c5a6a`.
Relative to R3by it adds exactly `default-parameters`, the 219 tagged
parse-negative paths, and 11 previously unaudited paths from the complete
14-path non-simple-parameter strict-body cohort.

The admission scope is a disjoint union of the 2,269-path / 4,516-variant tag
universe and 14 companion paths / 28 variants. The tag transition contains
3,352 `unsupported-feature` to `pass` changes, 1,162 diagnostic-only changes,
and two unchanged `IsHTMLDDA` host rows. The companion transition contains
22 `unsupported-negative-provenance` to `pass` changes and six unchanged
passes. All 28 companion variants are forced through both the raw Oxide worker
and pinned QuickJS 2026-06-04 as parse-phase `SyntaxError` tests; both engines
pass every variant.

The exact 102,037-key full-suite join records 3,374 outcome changes, 1,162
diagnostic-only changes, 97,501 unchanged rows, and zero previous-pass
regressions. The other 97,493 rows outside the combined admission scope are
byte-identical in both TSV and JSONL. The new canonical vector contains
63,073 passes, 63,592 runnable variants, 15,074 `unsupported-feature`
outcomes, 3,451 `unsupported-negative-provenance` outcomes, and 20,019 total
unsupported outcomes. Its TSV/JSONL SHA-256 values are
`2db7d8772074f90de6525cd51ffcd43ea3bf906d78e7c938d452cd6cac21a216`
and
`5c201991551f3bb3f03f5a5b232cff0b2470969ae440bc942c324ba4fc5d57a3`.

Reproduce the focused, admission, and canonical evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-default-parameters.sh
TEST262_WORKERS=8 ./scripts/test-test262-default-parameters-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-default-parameters-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

The known staging case for implicit `this` inside a parameter-expression
direct eval remains explicit shared QuickJS/Test262 spec debt: pinned QuickJS
2026-06-04 and Oxide fail it identically. It is not hidden by this admission
and is not an Oxide-versus-QuickJS parity blocker.

## R3bz default parameters certification

R3bz freezes the complete pinned Test262 `default-parameters` tag without yet
promoting it into the live profile. The candidate differs from the 92-feature
R3by parent by exactly that feature and all 219 parse-phase `SyntaxError`
paths selected by the tag. It therefore contains 93 reviewed features and
1,143 audited negative paths. The parent and candidate profile SHA-256 values
are
`d55e0625b1f6878b7afa6885d82cf332909271ce1c2222100fe3a403a8455969`
and
`9c345c1e2d79911eec5d6c8750a730f3b3ed0dbefdcd483e0f9c92fcf66aeca0`.

The exhaustive metadata universe is 2,269 paths / 4,516 variants. The
candidate activates 1,687 paths / 3,352 variants: 1,516 positive paths plus
171 newly authenticated negative paths. Oxide passes all 3,352, as does
pinned QuickJS 2026-06-04. Another 581 paths / 1,162 variants retain explicit
dependencies on private class methods or object rest, while one path / two
variants remains blocked by the unsupported `IsHTMLDDA` host capability.
There are no module, configuration-skip, failure, timeout, harness, or
negative-provenance outcomes in the runnable partition.
All 219 tagged negative paths / 435 variants are also forced through the raw
Oxide worker and pinned QuickJS, including the 48 paths whose normal candidate
rows remain blocked by another feature; both engines pass every variant.

Five- and eight-worker candidate reports are byte-identical. Their TSV/JSONL
SHA-256 values are
`a8047ac4a92d9d482eace99eec54bb361de70b8787c1c55f41a0c98bef89400f`
and
`4eb248df0b35c4ce6aa0e207de3c035d3d6792dabad90a383460b3246f8cb146`.
The exact keyed parent/candidate join contains 3,352 outcome changes, 1,162
diagnostic-only changes, and two unchanged host rows.

Reproduce the certificate with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-default-parameters.sh
```

This milestone authenticates the exact feature-tag universe; it does not
claim that every untagged parameter-expression interaction is finished. The
R3ca admission above separately retains all 14 non-simple-parameter
strict-body companion paths, including 11 newly audited negatives, while the
existing staging failure for implicit `this` inside a parameter-expression
direct eval remains visible.
Pinned QuickJS 2026-06-04 fails that staging case in the same way, so it is
shared Test262/spec debt rather than an Oxide parity blocker. The exact tag
projection was 3,352 new passes, and the complete R3ca join confirms 22 more
passes outside the tag universe.

## R3by global rest parameters admission

R3by promotes the R3bx `rest-parameters` certificate into the live
checksum-pinned Test262 profile. The profile now contains 92 reviewed feature
tags and 924 audited negative paths, retains the same async-execution entry,
and has SHA-256
`d55e0625b1f6878b7afa6885d82cf332909271ce1c2222100fe3a403a8455969`.
This is a capability/evidence promotion of semantics already implemented and
authenticated against QuickJS 2026-06-04; it adds no runtime code.

The complete tag transition changes all 192 sloppy/strict variants from
`unsupported-feature` to `pass`. Its receipt/data SHA-256 values are
`0aa8ac11097f5f81f138c7782b992312003f7ffca6bfad1f92dbb89f6fa8f8ce`
and
`602f57fb32774acc3fbfafa473b339fabca07581ebf882c31b602fa7d698a64b`.
The exact 102,037-key full-suite join has those same 192 outcome changes,
keeps the other 101,845 rows byte-identical, and records zero previous-pass
regressions.

Two independent two-worker full runs reproduce the new canonical vector:
59,699 passes, 60,218 runnable variants, 18,426 `unsupported-feature`
outcomes, and 23,393 total unsupported outcomes. Its TSV/JSONL SHA-256 values
are
`3268581d1be88057cd4953d8b91401cb6068bff95aa4830d49c77cd902baa9a5`
and
`7d1595d9aff6d04c022e688d5e82f32e09a6cfe7adc1f5ea1c0cb21d412933a6`.

Reproduce the focused, admission, and canonical evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-rest-parameters.sh
TEST262_WORKERS=8 ./scripts/test-test262-rest-parameters-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-rest-parameters-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3bx rest parameters certification

R3bx freezes the complete pinned Test262 `rest-parameters` tag as a scoped
candidate. The parent is the 91-feature R3bw profile; the candidate adds
exactly `rest-parameters` and the 96 negative-test paths selected by that tag,
growing to 92 reviewed features and 924 audited negatives. Their SHA-256
values are
`fc2716ff2ef12fda73c33db0603525f100713ff3b6df0ac8205977a20717ea3a`
and
`d55e0625b1f6878b7afa6885d82cf332909271ce1c2222100fe3a403a8455969`.

The tag is narrower than the full rest-parameter runtime surface: all 96
paths are generated parse-negative tests for the early error that forbids a
function body's own `"use strict"` directive when its parameter list is not
simple. They expand to 192 sloppy/strict variants with no module, config, or
residual-feature partition. The parent classifies all 192 as
`unsupported-feature`; the candidate and pinned QuickJS 2026-06-04 both pass
all 192. Five- and eight-worker Oxide reports are byte-identical, with TSV and
JSONL SHA-256 values
`9db05360e6b8d8199caea374321bdf3808fbd4d06218693212c3f1aeb6669c3d`
and
`4127e8c0b024f7039070352c99232656028b6f2a85e8aa35369e26fd7649fe5f`.

This certifies existing implementation rather than adding runtime code.
QuickJS clears `has_simple_parameter_list` while parsing rest and destructured
formals, then rejects `has_use_strict` in `js_parse_function_check_names`.
Oxide carries the same state through ordinary, arrow, async, generator, and
method parsing and already has dedicated strict-body early-error tests. The
broader rest surface remains independently covered by the 65-variant
identifier-rest gate, the now-green six formerly deferred pattern/new-target
variants, the 298- and 936-variant parameter BindingPattern gates, the
71-variant direct-eval parameter-environment gate, and 43 pinned-QuickJS
differential probes.

Reproduce the certificate with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-rest-parameters.sh
```

The projected global transition is exactly 192 new passes and 101,845
unchanged non-universe rows, with no previous-pass regression. R3by above
executes and freezes that complete 102,037-row join and promotes the candidate
to the canonical profile.

## R3bw global computed property names admission

R3bw promotes the R3bv `computed-property-names` candidate into the live
checksum-pinned Test262 profile. The profile now contains 91 reviewed feature
tags, preserves the same 828 audited negative paths and async execution entry,
and has SHA-256
`fc2716ff2ef12fda73c33db0603525f100713ff3b6df0ac8205977a20717ea3a`.
This is a capability/evidence promotion of semantics already authenticated
against QuickJS 2026-06-04, not a new runtime implementation.

The complete 478-path / 946-variant tagged join changes exactly the expected
895 rows: 439 dependency-clean variants move from `unsupported-feature` to
`pass`, while 456 variants retain `unsupported-feature` with only the admitted
tag removed from their residual dependency detail. All 42 configuration skips
and nine module rows remain byte-identical. The other 101,091 full-suite rows
also remain byte-identical, and no previous pass regresses.

Two independent full-suite constructions agree byte-for-byte. The canonical
102,037-row vector now contains 59,507 passes, 60,026 runnable variants,
18,618 `unsupported-feature` outcomes, and 23,585 total unsupported outcomes.
Its TSV/JSONL SHA-256 values are
`574d90530b5815329e65ab55d94bce4dd684233f1b296a888c87eced9077ba69`
and
`6d7ec82af17368ebea46213633efcec331198cf904db457434b7493b003e9616`.

Reproduce the focused, admission, and canonical evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-computed-property-names.sh
TEST262_WORKERS=8 ./scripts/test-test262-computed-property-names-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-computed-property-names-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3bv computed property names certification

R3bv freezes a scoped candidate for the core `computed-property-names`
grammar and runtime semantics. The candidate adds exactly that tag to the
R3bu profile, growing from 90 to 91 reviewed features while preserving all
828 audited negative paths and the async-execution entry byte-for-byte. The
parent and candidate profile SHA-256 values are
`e9c1ca295ca9270391f128c3f58484be3ac03a2a649b0170b551d41ab542f898`
and
`fc2716ff2ef12fda73c33db0603525f100713ff3b6df0ac8205977a20717ea3a`.

The complete pinned tag universe contains 478 paths / 946 variants. Its
disjoint partition is 220 paths / 439 dependency-clean activation variants,
228 / 456 reason-only variants, 21 / 42 `Atomics.waitAsync` config skips, and
nine module paths / variants. The frozen parent has zero runnable rows and
records 895 `unsupported-feature`, 42 `skipped-feature`, and nine
`unsupported-module` outcomes. The candidate passes all 439 activation rows,
leaves the 456 residual dependencies explicit, and preserves the 51 config
and module rows. Five- and eight-worker candidate reports are byte-identical;
their TSV/JSONL SHA-256 values are
`f29e969d8ce120fbbeba909265515a35219c68a621cc8892488137fc8fb55b56`
and
`ff01f50adcdc58253e55df2d13f01d960694c3a654548c3b7bc8a60148a5f3ba`.
Pinned QuickJS 2026-06-04 independently passes the same 439 activation
variants.

The implementation follows QuickJS's corresponding parser/lowering shape:
`js_parse_property_name` evaluates bracketed assignment expressions,
object literals apply `OP_to_propkey` before their values, and class fields
retain each computed key in a hidden lexical binding for later instance or
static initialization. R3bv adds no runtime semantics; it authenticates the
already implemented object, method, accessor, class, conversion-order, and
function-name behavior against both engines.

Reproduce the scoped evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-computed-property-names.sh
```

The projected global join is 439 outcome changes, 456 detail-only changes,
51 unchanged in-universe rows, and 101,091 unchanged non-universe rows. Those
whole-suite numbers remain a projection until a later global-admission
milestone reruns and freezes the complete 102,037-key vector. R3bw above is
that admission and confirms the projection exactly.

## R3bu global resizable ArrayBuffer admission

R3bu adds exactly `resizable-arraybuffer` to the checksum-pinned global
Test262 profile after the R3bt focused spillover gate. The live profile grows
from 89 to 90 reviewed feature tags, retains the same 828 audited negative
paths and async execution entry, and has SHA-256
`e9c1ca295ca9270391f128c3f58484be3ac03a2a649b0170b551d41ab542f898`.
Its ordered feature stream has SHA-256
`2c02df29f05b4d3303da0c26784f7f6eab7a83d4f20caf7b3e5747bcaed7de42`.

The exhaustive metadata universe contains 463 paths / 926 sloppy and strict
variants. It partitions into 381 paths / 762 activation variants, 80 paths /
160 reason-only variants that retain other unsupported dependencies, and two
paths / four QuickJS-config skips. The earlier ArrayBuffer, DataView, and
TypedArray gates already cover 312 of those paths; R3bt freezes the remaining
151-path spillover as 95 activation, 54 reason-only, and two config-skip
paths. Oxide and pinned QuickJS both pass all 762 activation variants, and
the four-vector ArrayBuffer differential also passes. The focused TSV/JSONL
SHA-256 values are
`79baa1c1e323cb1256f3e0f7bdfbc403f3732100f40be807f63dfed6d84ab70c`
and
`8a0ed16786ae3ecec118e1fd84392cb6857fb3c9ecb57d6977e7b962ed8bb0da`.

The global parent records 922 `unsupported-feature` rows and four unchanged
config skips. The candidate changes all 762 activation rows to `pass`,
narrows only the diagnostic detail of the 160 reason-only rows, and preserves
the four skips. The 926-row transition receipt and data-row SHA-256 values are
`f51b57c077fcbea258c39907ed95cffa50280d4af11ef355076bcad675959c0e`
and
`2dab3bfcf31b227e34920dbdbf5e3cd47c7a16fb32abda95c14d88d10965d335`.
The candidate tag TSV/JSONL SHA-256 values are
`dbaa0982f04607a08819b73e2505f20c83e62ed9670b779043764f0ce2a8053b`
and
`54882a02e660938116bc4acb7a9fa5d9efeb611bb8e5bd0ac3780d3e4a8ecd37`.

The exact 102,037-key full join has 762 outcome changes, 160 detail-only
changes, four unchanged config rows, and 101,111 byte-identical non-universe
rows. No previous pass regresses. The R3bu vector contained 59,068
passes, 59,587 runnable variants, 19,057 `unsupported-feature` outcomes, and
24,024 total unsupported outcomes. Its rates are 57.89% raw, a 70.69%
conservative target-scope lower bound after the 18,475 pinned QuickJS target
exclusions, and 99.21% among 59,538 variants with a non-unsupported observed
outcome. The full TSV/JSONL SHA-256 values are
`a21d195a1a6209c5df6b7080a9a941d773c87abeed7ec63961b5896b1b294045`
and
`834754d9d6ab62606c3463b351932dedade8e9f78ba6ea835a87aa743cf9fb41`;
independent worker-count reproductions are byte-identical.

Reproduce the focused and global evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-resizable-arraybuffer.sh
TEST262_WORKERS=8 ./scripts/test-test262-resizable-arraybuffer-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-resizable-arraybuffer-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

This admission adds no runtime semantics beyond the already-certified buffer
and view implementation. It is a capability-profile and evidence milestone,
not a Feature Parity completion claim.

## R3bs global Uint8Array base64/hex codec admission

R3bs admits exactly `uint8array-base64` to the checksum-pinned global Test262
profile after the R3br implementation and focused differential gate. The R3bs
candidate contains 89 reviewed feature tags, retains the same 828 audited
negative paths and async execution entry, and has SHA-256
`ed80ab5aed86c606a1d7b5c1854b78ab1bb3c517cf0c6898a89e9f8d19135000`.
Its ordered feature stream has SHA-256
`593a376a65171a87d8c12df6834570322657ab42d6d48560a7ca14df5c6e7e96`.
The frozen parent is exactly the historical 88-tag R3bq profile with SHA-256
`5d3543018b022f968e4d7bb1725cef1c0e101e3c61a4d2d35f2c77df5ec975e9`;
the negative and execution sections are byte-identical across the transition.

The exhaustive tagged universe is the same 69 paths / 138 sloppy/strict
variants authenticated by R3br. Under the parent, all 138 fail closed as
`unsupported-feature` for exactly `uint8array-base64`; under the candidate,
all 138 are runnable and pass. The transition receipt and its data-row
SHA-256 values are
`d3c7b72f7dfaea4523c7378deedbd5f9b2f3a8aca26dcbdf3f86727b1f1fb2c5`
and
`ce4172f23d0e5986b85171c2b85201f20b96e3f772d684d5ffd050c0f88010ad`.
The candidate tag TSV/JSONL SHA-256 values are
`2a2c523a9d02087a72eca78a94cbe785fac269e81815a3065f8daa0b3ca87fe2`
and
`fbc2b21194d9fece3e2b9d7afc1d906d8576eaafa0c6608fa3cca7020a69a127`;
independent eight- and five-worker tagged runs are byte-identical.

The exact full-corpus candidate changes only those 138 outcomes from
`unsupported-feature` to `pass`: all 101,899 non-universe rows are unchanged,
there are no detail-only changes, and no previous pass regresses. It contains
58,306 passes, 58,825 runnable variants, 19,819 `unsupported-feature`
outcomes, and 24,786 total unsupported outcomes. Its rates are 57.14% raw, a
69.78% conservative target-scope lower bound after the 18,475 pinned QuickJS
target exclusions, and 99.20% among 58,776 variants with a non-unsupported
observed outcome. The candidate full TSV/JSONL SHA-256 values are
`789b1d116e10dbeb7607faf4bbbcb5df818a6e588799d156579b5047238b0379`
and
`d1476490e0f53bb1397ce432c813c781e51130cfd97da22e1fdc8edc10f95a8f`.

Reproduce the focused dependency gate and global transition with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-uint8array-codecs-global.sh
TEST262_WORKERS=5 ./scripts/test-test262-uint8array-codecs-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-uint8array-codecs-global.sh --full
```

This is a global capability-profile and evidence milestone. It adds no new
runtime semantics beyond R3br and is not a Feature Parity completion claim.

## R3br focused Uint8Array base64/hex codec gate

R3br implements the complete six-function codec surface from QuickJS
2026-06-04 `quickjs.c:58741-59597`: static `Uint8Array.fromBase64` and
`Uint8Array.fromHex`, plus prototype `toBase64`, `toHex`, `setFromBase64`,
and `setFromHex`. The implementation preserves the pinned upstream behavior
for function placement and descriptors, exact Uint8Array branding, base64
alphabets and final-chunk modes, option getter order, capacity-limited and
partial writes, detached/out-of-bounds revalidation, accepted whitespace,
WTF-8 rejection, realm ownership, and diagnostics. A ten-vector differential
oracle passes unchanged in both Oxide and pinned QuickJS.

The checksum-bound scoped profile admits exactly `Reflect.construct`,
`TypedArray`, and `uint8array-base64`. Its SHA-256 is
`2e8f870a5c6d1c05adc37c759098d2412943beff8b8de3c1593ba74df7761ac9`;
the ordered feature stream has SHA-256
`41acf42eb5acbf12874115c7cbc757d7cb3e2ddd26603a55b55fbf95bb90532e`.
The exhaustive sorted manifest contains 69 paths / 138 sloppy/strict variants.
Its path-stream and complete-file SHA-256 values are
`cbde75ee5038f3c24abfbf8f6e2734494281163bbe36370d0c81443da02a660c`
and
`2a52c3f54ef83a8df736e823d76e17927b670045f42d338d42a64f0e48681bb2`;
the 138-key stream has SHA-256
`e55870b3ba3591f83a43fb3e58c0beb6be7de35916aa6efdbde1844f4f9ba628`.

All 138 variants are runnable and pass in Oxide, with zero failures,
unsupported outcomes, or skips; pinned QuickJS independently passes the same
138 variants. The classified TSV/JSONL SHA-256 values are
`4862f2570cf27fed439f3bd4c731b520b2ebac1643a5b257aaa21d112592742b`
and
`04395a486012a649f6cba508791ebd83367a4e0db2cb7d418ec0bcc302b46663`;
eight- and five-worker Oxide runs are byte-identical.

Reproduce the implementation differential and scoped Test262 evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-uint8array-codecs.sh
TEST262_WORKERS=5 ./scripts/test-test262-uint8array-codecs.sh
```

This was the focused implementation milestone. At R3br it did not yet add
`uint8array-base64` to the then-live 88-tag global capability profile or
change the then-current 58,168/102,037 canonical vector. R3bs now performs
that separate global admission above; neither milestone claims Feature
Parity.

## R3bq global Promise capability closure

R3bq admits exactly `Promise`, `Promise.allSettled`, `Promise.any`, and
`Promise.prototype.finally` to the checksum-pinned global Test262 profile.
The live profile now contains 88 reviewed feature tags, retains the same 828
audited negative paths and async execution entry, and has SHA-256
`5d3543018b022f968e4d7bb1725cef1c0e101e3c61a4d2d35f2c77df5ec975e9`.
The admission reuses the pinned-QuickJS Promise jobs,
`Promise.prototype.finally`, and aggregate differential gates; all three pass.

The exhaustive metadata universe is 226 paths / 452 variants. Its exact
partition is 208 paths / 416 activation variants, all passing after admission,
plus 18 paths / 36 reason-only variants that remain fail-closed: 12 for
`class` and 24 for `computed-property-names`. The historical parent tag
TSV/JSONL SHA-256 values are
`623a2e0fecca4a2746b667ea0552b9621a89bc8f1448a1c3b1aa7f557e487b1a`
and
`fff4000cdd7f160f12e7495f09f6f995e0be2d96452ffbecdce54822a50c2ed5`;
the candidate values are
`500d94a18e8872bdd9df1bf87cb535cee41a3632a922575f6a11699170662c2d`
and
`04f9e7b06d26709b507a9809e8f757a075811f4069345f070513c86b60ee29b3`.
The 452-row transition receipt and data SHA-256 values are
`955e77db96a429533b946fac4de9f9c0808f793a1506fddec2d2ab29eb1e91d8`
and
`0831ea9577c8ae2c9ddf7a84903ffaaa49882e1f9fd889570740d1d3da3a91b4`.

The complete 102,037-key join contains exactly 416 outcome changes, 36
diagnostic-detail-only changes, and 101,585 byte-identical rows, with zero
previous-pass regressions. The canonical vector now has 58,168 passes, 58,687
runnable variants, 19,957 `unsupported-feature` outcomes, and 24,924 total
unsupported outcomes. Its rates are 57.01% raw, a 69.61% conservative
target-scope lower bound after the 18,475 pinned QuickJS target exclusions,
and 99.20% among 58,638 variants with a non-unsupported observed outcome. The
full TSV/JSONL SHA-256 values are
`4a529df1318a233d16de1e3563de3e987a4a51f200bb6d37e73281142e51e19a`
and
`80006172f384144bb3f169ba56d587bb2f48f5e21cdaadde0308e0fcde386df9`.
Eight- and five-worker tag repeats are byte-identical, as are two- and
one-worker full repeats; an independent two-worker canonical repeat matches
the frozen candidate.

Reproduce the admission and canonical evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-promise-global.sh
TEST262_WORKERS=5 ./scripts/test-test262-promise-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-promise-global.sh --full
TEST262_FULL_WORKERS=1 ./scripts/test-test262-promise-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

This closes the global profile admission for the already-implemented Promise
surface. It is not a Feature Parity completion claim; modules, host hooks, and
the broader remaining engine surface stay explicit.

## R3bp global `globalThis` admission

At R3bp, exactly `globalThis` was added to the then-live checksum-pinned
Test262 profile after the R3bo focused gate closed its source, compiler, and
harness evidence.
The resulting historical profile contained 84 reviewed feature tags, the same
828 audited negative paths, and the same async execution entry. Its complete
SHA-256 is
`caa287cbf8188ea1c0519daa7d77fc5adb63d98c523299377eec14730b54cd15`;
the feature section alone has SHA-256
`e928613f44d53e2d3690a5305ae29a707b30fc66ec0a797016b46d2460b39423`.

The exhaustive tagged population remains 148 paths / 165 variants. Exactly
135 paths / 150 variants move from the historical profile's
`unsupported-feature` classification to `pass`. The other 13 paths / 15
variants are byte-identical before and after admission: 11 module variants
remain `unsupported-module`, and four `explicit-resource-management`
variants remain `skipped-feature`. The candidate tag TSV/JSONL SHA-256 values
are
`fe95410a26b918c8aeb2aab5218fc653aeee7bc7cba1b8d1bc44b67deebe11d2`
and
`917e99fb2cb41ae7698d376f4a93e078bede865c749f59e0be1a06f8503c947a`.
The 165-row transition receipt and its data SHA-256 values are
`46c161ade8b302c99167d0837c18e0991cab40b9ae9129fd2ec45719ba418507`
and
`d4351933687b1ee1a284c84868af09f158584806a3a23545bf53b1d373491466`.
Independent eight- and five-worker tagged runs are byte-identical.

The exact historical full join retains all 102,037 keys. Its only changes are
the 150 focused `unsupported-feature -> pass` rows: all 15 deferred rows, all
101,872 non-`globalThis` rows, and therefore all 101,887 unchanged rows are
byte-identical in both TSV and JSONL. There are zero detail-only changes and
zero previous-pass regressions. The R3bp canonical vector had 57,752 passes,
58,271 runnable variants, 20,373 `unsupported-feature` outcomes, and 25,340
total unsupported outcomes. Its rates are 56.60% raw, a 69.11% conservative
target-scope lower bound after the 18,475 pinned QuickJS target exclusions,
and 99.19% among 58,222 variants with a non-unsupported observed outcome.
The canonical full TSV/JSONL SHA-256 values are
`1dfbd54d69e3ebace9edfb1ba3502d402edbd1919f34a353c8996eec63522a0d`
and
`f255a6852b17479e0d699195e2b50477e5094113861672587852b04bb3ed9668`.
The candidate report from the two-worker admission run, the independent
one-worker frozen-vector reproduction, and the independent canonical
two-worker live run are byte-identical.

Reproduce the focused, tagged, and historical full-join gates with:

```sh
./scripts/test-test262-global-this.sh
TEST262_WORKERS=8 ./scripts/test-test262-global-this-global.sh
TEST262_WORKERS=5 ./scripts/test-test262-global-this-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-global-this-global.sh --full
TEST262_FULL_WORKERS=1 ./scripts/test-test262-global-this-global.sh --full
```

This is a profile and evidence milestone with no production runtime-semantics
change, not a Feature Parity completion claim. Module execution, the two
config-excluded paths, and the broader remaining engine surface stay explicit.

## R3bo focused `globalThis` gate

R3bo freezes the complete `globalThis` metadata population at 148 paths / 165
variants. Its partition is disjoint and exhaustive: 135 paths / 150 variants
form the activation, while 13 paths / 15 variants remain deferred. The
deferred ledger is exactly 11 module paths / 11 variants plus two
`explicit-resource-management` paths / four variants excluded by the pinned
QuickJS config. All eight negative tests are deferred module-resolution
`SyntaxError` cases. Pinned QuickJS passes all 150/150 activation variants.

The immutable historical parent is the then-current 83-tag global profile. It
admits 0 activation variants and classifies all 150 as `unsupported-feature`
for exactly `globalThis`. A frozen candidate adds only `globalThis`; it admits
and passes all 150/150 variants. The exact transition receipt and data
SHA-256 values are
`33cc8a8ffd153694a0f0d331c75f777e859a0de39bf227e1ff441ba1e1e73193`
and
`f43cb0f5682c394eeacffdee49dc1353f9fd92cf792efbd04831272a6779eb97`.

Independent eight- and five-worker reports are byte-identical. The parent
TSV/JSONL SHA-256 values are
`46850bdc3e24aeda34b5dfb26fec33cae85b9bdce2fc8c75e43e26bcb4d035c5`
and
`b2db5df01d15118155f20453adaaefeba0bffe6b54759177d1ba11c15d181736`;
the candidate values are
`21b125444add1d6e114670e69e9510e305b659608f41b59a4a6a46ab5a419c2e`
and
`73b47ebf51b0cdb70112654eb0791e1a01a8729e952192dd02ddb23112dbd75d`.

A pinned-source audit also matched QuickJS's global-object installation,
parameter-scope direct-eval declaration checks, and
`with`/`Symbol.unscopables` lookup against Oxide's corresponding runtime and
compiler paths. Ten representative real Test262 probes spanning those three
families passed in both engines before the exhaustive gate ran.

This was a focused evidence milestone with no production runtime-semantics
change. At R3bo, `globalThis` had not yet been admitted to the 83-tag live
global profile, and no complete-vector run or new global score was claimed.
R3bp later performs that global admission above.

## R3bn global Iterator Helpers admission

R3bn adds exactly `iterator-helpers` to the checksum-pinned global Test262
profile after the R3bm focused gate closed its source-and-harness
dependencies. The resulting profile contains 83 reviewed feature tags and the
same 828 audited negative paths; its SHA-256 is
`8a3b253f6d2a24b18f9bec66628ba5aec3fb337d677c60bfde37c4c3a33d3910`.

The exhaustive tagged population contains 567 paths / 1,134 variants. Its
partition is exact: 538 paths / 1,076 variants are activated, 13 paths / 26
variants remain `unsupported-feature` only because they also require
`globalThis`, and 16 paths / 32 variants remain behind the audited host/config
ledger. All 1,076 activated variants pass. The tagged candidate TSV/JSONL
SHA-256 values are
`21e6b0be9aa662c485176690d5665bc6f79687fc7bf4ae4ddf6335ee419a8f5d`
and
`d62917441f6a7c6a5163316d1c09ccf4540ff54b4a6599a58285d0f34f01a66b`.
The exact transition receipt and data SHA-256 values are
`97d980227d3f9913d6fedb6e97deec7ae0b1db3df3fefe01b74d96320c775d4f`
and
`84cdc0c565ddff0257ff1aff6c29e0297f8d3c3afca72015cfd17d6192e5b108`.

The complete 102,037-key join records exactly 1,076
`unsupported-feature -> pass` transitions and 26 diagnostic-detail-only
changes; all 32 host/config variants and all 100,903 non-Iterator-Helper
variants are unchanged. No previous pass regresses. The R3bn canonical vector
has 57,602 passes and 58,121 runnable variants: 56.45% raw, a 68.93% lower
bound after the 18,475 pinned QuickJS target exclusions, and 99.19% among
58,072 variants with a non-unsupported observed outcome. Its canonical full
TSV/JSONL SHA-256 values are
`7b5bb9d188473f7f7298e131da405f7e77e66c6eddbf10d14949722bf275c6fc`
and
`869d9150a532a72c02e37eae9d1d3ead2c88c8384be23e5222efe055e99a18a2`.
An independent canonical two-worker repeat is byte-identical.

This is a profile and evidence milestone with no production runtime-semantics
change, not a Feature Parity completion claim. `globalThis` and the audited
host/config capabilities remain explicit follow-up frontiers.

## R3bm historical Iterator Helper Proxy-closure refresh

R3bm kept the complete 567-path `iterator-helpers` population and its raw
44-path dependency union explicit. That union partitioned into the complete
28-path source-and-harness Proxy closure and 16 host/config paths. R3bl had
already promoted the exact 14-path optional-chaining adjacency; R3bm promoted
the remaining 11 source-Proxy and three harness-Proxy paths. The refreshed
manifest therefore contained 551 paths / 1,102 sloppy-strict variants. Pinned
QuickJS passed all 551 paths in each mode, Oxide passed all 1,102 variants,
and independent 8/8/5-worker Oxide reports were byte-identical.

The 16-path / 32-variant deferred ledger was exactly 11 `$262.createRealm`
paths, four `$262.IsHTMLDDA` paths, and one pinned QuickJS-config exclusion.
Its variant-key SHA-256 was
`5f9105c90732493741b8b652f0a5ad74f775740706d847171c96617fdd23b760`.

The immutable scoped profile contained 76 feature tags and 802 audited
negative paths. Its complete-file SHA-256 was
`a0ed7fa1a5cd46c5c47895d671c0078434635ae41f0a420e66573dcb86d18a7f`.
The manifest path-stream, complete-file, variant-key, TSV, and JSONL hashes
were
`32b3a539828fe72e32cb28bed6b6942749ac1aa6402a04bb809126da0a2cea4c`,
`6db8a38003ba95245dde0e0559b64a75c1a0215e610408811174f482363b729c`,
`cc432f145a9f12ad959f0b856c5b91c73a1b9ce0ebb3fd0c9cc5a18ac0f2f841`,
`47b725903172118e8fbde4ba8f6d87343d44fa280889630e1ee5d620634154e5`,
and
`9c55978a8b8200be94617eb5c80ea97abac7172b93599fdd31769df6a7679d08`.

The independent Iterator sequencing gate remained 64/64 in both engines. Both
Iterator scripts authenticated immutable historical profile sections instead
of comparing against the growing live global profile. R3bm was a focused
evidence refresh, not global `iterator-helpers` admission: its complete vector
remained at R3bj's 56,526/102,037 passes and 57,045 runnable variants, with
canonical TSV/JSONL hashes
`84c15d4a25343e1d306e17f431e515993abe09db76590920539eefe93d6fb3eb`
and
`96ebd4a8f51001b403e88d19c128bebb92b74bb9abf1e45c832b187924c635fd`.

## R3bk refreshed `for await` focused gate

R3bk removes exactly
`test/language/expressions/optional-chaining/iteration-statement-for-await-of.js`
from the `for await` exclusion ledger and adds `optional-chaining` to the
gate-owned scoped profile. Candidate discovery remains fixed at 1,297 paths /
2,531 variants. The ledger now excludes 32 paths / 39 variants, leaving 1,265
paths / 2,492 variants admitted. Pinned QuickJS passes 1,265/1,265 paths and
Oxide passes 2,492/2,492 variants; independent 8/8/5-worker reports are
byte-identical.

The scoped-profile, exclusion-ledger, manifest, key-set, TSV, and JSONL
SHA-256 values are
`d5d30d77eaabebeea1a9fa3cb18f555e3c5d69d263d1b82ca624c339f6262a2e`,
`cf172c4d38c6fee27f20ccc6775251284e328255f1a416b9ff22f5760e2a1e47`,
`f87858a6c22df8c689d15f081075cba2758feb63eacb4be9ee310e72e9d17a0a`,
`8669fd1b353cf24a52297a6680a4b43041a7c03ac5c33cd93abf8afbe82535cd`,
`6b102a66ca2c71be3f9999efd027bda49f65b3a3465d555c7775a59b999ed823`,
and
`ca3703f6fb7296af390979df9f60a6049d3d8703cc6929cf2937586afd972832`.

The R3al global-profile hash is retained only as historical provenance. The
gate pins its own focused profile and checks the live global profile for async
execution, so unrelated future global feature admissions cannot drift this
receipt. This evidence-only refresh does not move the complete vector: it
remains at 56,526/102,037 passes and 57,045 runnable variants with R3bj's
canonical full TSV/JSONL hashes
`84c15d4a25343e1d306e17f431e515993abe09db76590920539eefe93d6fb3eb`
and
`96ebd4a8f51001b403e88d19c128bebb92b74bb9abf1e45c832b187924c635fd`.

## R3bj optional chaining global admission

R3bj admits exactly the `optional-chaining` tag and its 26 audited
parse-negative paths into the checksum-pinned global Test262 profile. The
profile now contains 82 reviewed feature tags and 828 reviewed negative paths;
its SHA-256 is
`205554c5686ef2ec77420984ce038d321411a11acabefd2c37d9b63b67fcba62`.

The dependency-clean activation contains 52 paths / 104 variants. All 104 move
from `unsupported-feature` to `pass`. Four class/private paths / eight variants
remain `unsupported-feature` behind another dependency and change diagnostic
detail only. The provenance canary consequently records 10 intended parse
passes and nine fail-closed variants.

The exact complete vector reaches 56,526/102,037 passes with 57,045 runnable
variants and 21,599 `unsupported-feature` outcomes. That is 55.40% raw, a
67.65% lower bound after the 18,475 pinned QuickJS target exclusions, and
99.18% among the 56,996 variants with a non-unsupported observed outcome.
Canonical full TSV/JSONL SHA-256 values are
`84c15d4a25343e1d306e17f431e515993abe09db76590920539eefe93d6fb3eb`
and
`96ebd4a8f51001b403e88d19c128bebb92b74bb9abf1e45c832b187924c635fd`.
No previous pass regresses.

R3bj also closes the remaining historical coupling in the R3be TypedArray
receipt. Its reconstructed parent now takes the frozen 80-tag inventory and
the immutable 802-path negative section from the checksum-bound Iterator
sequencing profile, rather than reading either section from the growing global
profile. Later feature or negative-provenance admissions therefore cannot move
the R3be activation or reason-only partitions.

This is a profile and evidence milestone, not a Feature Parity completion
claim. At R3bj, the Iterator Helper adjacency cohort and the for-await-of
ledger remained separate follow-up gates; R3bk above refreshes the latter.

## R3bi optional chaining focused implementation

R3bi ports QuickJS-shaped optional chaining in the compiler without adding a
runtime or VM opcode. Each `?.` edge lowers to the existing nullish test,
branch, drop, fallback, and shared-chain-end instructions. Parser-owned
Reference metadata preserves method receivers across public grouped calls and
deliberately reproduces pinned QuickJS behavior for indirect `eval?.()`,
authored-`with` optional calls, grouped private methods, and optional private
`delete`.

The pinned QuickJS oracle contains 38 reviewed vectors covering fixed and
computed members, fixed/spread calls, multi-edge short circuiting, grouping,
receiver identity, `super`, private fields and methods, `new.target`, async and
generator effects, `delete`, and assignment/update/destructuring/iteration/
`new`/template early errors. Oxide and QuickJS 2026-06-04 agree on all 38.

The checksum-bound focused Test262 gate derives 56 tagged paths, keeps four
class/private paths in an explicit reason-only ledger, authenticates all 26
parse-negative paths, and runs the remaining 52 paths / 104 variants. Oxide
and pinned QuickJS both pass 104/104. A separate 14-path / 28-variant ledger
records the hidden optional-chain dependency in Iterator Helpers without
claiming that family here. The global `optional-chaining` tag is intentionally
not admitted by this milestone.

Five already-runnable staging paths improve by exactly nine variants, with no
other full-vector movement: seven parse failures and two runtime failures
become passes. The canonical complete vector therefore reaches
56,422/102,037 passes while remaining at 56,941 runnable variants. Its
TSV/JSONL SHA-256 values are
`5c388e568e6ee9e09799bc0f471a5926f0b680bd8f4d781e84130fce1a968e8a`
and
`19f076e99f56f22374a533e1f9c8fead0775bf81d2d1940641ae322901c1cc88`.
Iterator spillover, the now-obsolete for-await-of exclusion, negative
provenance, and global admission remain separately auditable next steps.

R3bi is an implementation and focused-evidence milestone, not a Feature
Parity completion claim.

## R3bh Proxy global admission

R3bh admits exactly the `Proxy` tag into the checksum-pinned global Test262
profile, bringing it to 81 reviewed feature tags with SHA-256
`2bfad693206dd09934a4c95ca241c49c4997ad795b8f0016571aada9c2cf1804`.
The Proxy-only activation partition contains 405 paths / 787 variants, all
passing. A disjoint 21-path / 42-variant reason-only partition remains
`unsupported-feature` because every row has another unsupported dependency.

The exact complete vector now reaches 56,413/102,037 passes with 56,941
runnable variants and 21,703 `unsupported-feature` outcomes. That is 55.29%
raw, a 67.51% lower bound after the 18,475 pinned QuickJS target exclusions,
and 99.16% among the 56,892 variants with a non-unsupported observed outcome.
Canonical full TSV/JSONL SHA-256 values are
`b634753cd21d2ed2194ee6170bfaf530767ffbc591b04d16e21ca30021b96623`
and
`94ffbb29cbac96a3b1237ce3b4521b56f336f75020ff256ba79fb1875a5e63bb`.
All 787 newly activated variants move from `unsupported-feature` to `pass`;
the 42 reason-only rows change only diagnostic detail, and no previous pass
regresses. R3bh froze the feature side of the older R3be TypedArray parent with
an immutable checked-in 80-tag inventory, but its 802 negative-provenance
paths still came from the growing global profile. R3bj above closes that
remaining historical coupling.

Four already-exposed Test262 variants still fail in both Oxide and pinned
QuickJS 2026-06-04: sloppy/strict
`test/staging/sm/object/defineProperties-order.js` and sloppy/strict
`test/staging/sm/regress/regress-1383630.js`. They respectively pin QuickJS's
batch descriptor-enumerability snapshot order and its incomplete Proxy
fixed-descriptor compatibility check. Differential regressions now preserve
those target deviations, so the runtime is not changed to satisfy conflicting
Test262 expectations at the expense of QuickJS feature parity.

R3bh is a profile and evidence milestone; it does not claim Feature Parity.
Modules, host hooks, SharedArrayBuffer/Atomics, and broad built-in coverage
remain explicit frontiers.

## R3bg exotic object oracle activation

R3bg removes seven obsolete capability sentinels which still expected
`Proxy`, `ArrayBuffer`, and concrete TypedArrays to be absent after those
families had already landed. It replaces them with 30 active pinned-QuickJS
differential vectors across Object descriptors, extensibility, integrity,
`fromEntries`, `hasOwn`, the String intrinsic, and the String includes family.

The new coverage observes Proxy trap order and invariants, TypedArray
integer-index descriptors, resizable-buffer extensibility rejection, partial
integrity mutations, IteratorClose after a Proxy entry throw, and
TypedArray/Proxy string conversion. The existing Rust release CLI and pinned
QuickJS 2026-06-04 produce byte-for-byte identical observations for all 30
vectors. This is a test-publication milestone: it changes no product runtime
code and does not by itself justify global Proxy admission.

The checksum-bound scoped Proxy vector remains the broader baseline: its 904
variants contain 823 passes and 81 unsupported outcomes, with 829 classified
as runnable and zero failures. The unsupported outcomes are 74 missing
`create-realm` host cases, one module case, and six parser cases.
Module namespace objects and SharedArrayBuffer/Atomics remain independent real
frontiers rather than being hidden behind already-published Proxy/TypedArray
capability checks.

## R3bf browser playground milestone

R3bf adds the public
[browser playground](https://pocket-stack.github.io/quickjs-oxide/) as a
pre-parity presentation milestone. The page executes the project's real Rust
engine compiled to WebAssembly; it does not route source through browser or
Node `eval`/`Function`.

- The new `HostServices` seam supplies time, timezone offset, and random seed
  data. Native runtimes keep their system-backed services, while the WebAssembly
  wrapper supplies browser-backed services without changing JavaScript
  semantics in the product layer.
- Every evaluation creates a fresh `Runtime` and `Context`. A dedicated Worker
  owns the engine and is terminated and recreated after the two-second limit,
  so a non-terminating example does not strand the page.
- The build smoke loads the generated WebAssembly in Node and evaluates the
  demo function through the Rust compiler/VM, returning `42`. The current web
  profile's first CI deployment is 3,140,771 bytes raw and 1,037,571 bytes
  gzip-compressed.

This milestone improves access and demonstration only. Its examples and smoke
test are not parity evidence; the pinned QuickJS differential gates and full
Test262 vector remain the conformance baseline and the main Feature Parity
workstream. Build and architecture details live in
[`playground.md`](playground.md).

## Implemented on the final architecture path

- QuickJS 2026-06-04 release metadata, archive checksum, bytecode version,
  Unicode version, and Test262 commit are pinned in `compat/upstream.toml`.
- The process-isolated Rust Test262 runner now saves a complete conservative
  outcome vector for all 102,037 sloppy/strict variants. A checksum-pinned
  capability profile now admits 130 reviewed feature tags and 1,197 exact
  audited negative-test paths. Those fail-closed canaries and the source/metadata host
  requirements keep unsupported grammar,
  features, modes, and `$262` hooks from becoming false passes. Bounded workers
  preserve canonical byte-for-byte TSV and JSONL ordering. R3al promotes the
  fully authenticated async-function and async-iteration stack into that
  global profile. R3am then adds the scoped Proxy internal-method gate without
  globally admitting the feature tag. R3an adds the branded ArrayBuffer
  backing-store/intrinsic milestone, R3ao adds the independently owned DataView
  intrinsic, and R3ap adds the shared kernel for all 12 concrete TypedArray
  classes. R3aq promotes the in-place mutation cohort without globally
  admitting the still-incomplete broad TypedArray feature tag. R3ar adds the
  dedicated indexed lookup/search kernel and promotes `at`, `includes`,
  `indexOf`, and `lastIndexOf` under the same conservative boundary. R3as adds
  the callback-driven `find`, `findIndex`, `findLast`, and `findLastIndex`
  kernel, and R3at adds QuickJS-shaped `every`/`some` short-circuit traversal,
  R3au adds QuickJS-shaped `forEach`, and R3av adds QuickJS-shaped
  `reduce`/`reduceRight` accumulation. R3aw adds species-aware `map`/`filter`,
  R3ax adds QuickJS-shaped `slice`/`subarray`, and R3ay adds non-species
  change-by-copy `with`/`toReversed`. R3az adds dedicated
  `join`/`toLocaleString` stringification and the inherited `toString` surface,
  R3ba adds QuickJS-shaped `sort`/`toSorted`, and R3bb authenticates the
  existing shared `entries`/`keys` iterator path without a production-code
  change. R3bc authenticates static `TypedArray.of` and fixes the shared
  static-`from`/`of` primitive-receiver constructor diagnostic seam. R3bd
  authenticates static `TypedArray.from`, including QuickJS's nullish-source
  diagnostics, materialize-before-construct ordering, and hidden-list value
  lifetime. R3be then admits the global `TypedArray` feature after freezing its
  exact activation and spillover partitions. R3bh admits the globally audited
  `Proxy` feature after its 405-path / 787-variant activation passes in full;
  21 paths / 42 variants remain reason-only rows behind other unsupported
  dependencies. R3bi then implements QuickJS-shaped optional chaining and
  authenticates its 104-variant dependency-clean focused cohort. R3bj admits
  the global tag together with its 26 audited negative paths; all 104
  dependency-clean variants pass, while eight class/private variants remain
  reason-only. It also freezes the historical R3be parent's 802-path negative
  source. R3bk then refreshes the `for await` gate after optional chaining
  admission: its unchanged 1,297-path / 2,531-variant candidate now admits and
  passes 1,265 paths / 2,492 variants in both engines. R3bl first promotes the
  exact 14-path optional-chaining adjacency into the scoped Iterator Helper
  gate. R3bm then promotes the remaining 14 source-and-harness Proxy paths,
  completing the 28-path Proxy closure. The gate now passes 551 paths / 1,102
  variants in both engines while retaining exactly 16 host/config deferrals.
  R3bn then admits exactly `iterator-helpers` into the global profile: 1,076
  variants activate and pass, 26 remain fail-closed behind `globalThis`, and
  all 32 host/config variants remain unchanged. R3bp then admits exactly
  `globalThis`: its 150 dependency-clean variants activate and pass while all
  15 module/config deferrals remain unchanged. R3bq closes the implemented
  global Promise surface: 416 variants activate and pass, while 36 variants
  remain fail-closed behind `class` or `computed-property-names`.
  R3br then implements the complete six-entry Uint8Array base64/hex codec
  surface and authenticates all 138 variants against pinned QuickJS. R3bs
  admits exactly `uint8array-base64` globally: those 138 variants move from
  `unsupported-feature` to `pass`, while every non-universe row remains
  unchanged. R3bt then authenticates the complete 762-variant
  `resizable-arraybuffer` activation and its previously uncovered spillover;
  R3bu admits that tag globally while keeping all residual dependencies and
  config skips explicit. R3bv then authenticates the complete
  `computed-property-names` universe against pinned QuickJS, and R3bw admits
  that tag globally while preserving its residual class-field, config, and
  module boundaries. Those R3bw counts are historical. Subsequent globally
  admitted milestones through R3dl advance the current measurement to 66,476
  passes and 66,528 runnable variants: 65.15% raw, a 79.55% lower bound after
  the 18,475 pinned QuickJS target exclusions, or 99.92% among the 66,528
  variants with a non-unsupported observed outcome. R3dl globally admits the
  previously scoped SharedArrayBuffer and Atomics implementation, so the
  current vector records 12,761 `unsupported-feature` and 17,034 total
  unsupported outcomes, seven parse failures, 43 runtime failures, no harness
  failures, and two timeouts. Two independent full runs are byte-identical; the
  canonical TSV/JSONL SHA-256 values are
  `501b64ed5c8367f33408225d956a262619163adf52baadf28f02811d14f3eae9`
  and
  `610e16ba65a0239556842efec7a745ba2885c72dfb3b8447c2578b8767ef7d40`.
  The cumulative TypedArray scoped gate still passes 2,254 paths / 4,463
  variants in both engines.
  Modules, the 59-path / 118-variant Test262 `$262.agent` host and its
  agent-backed waiter conformance, and broad built-in coverage remain explicit
  frontiers. `Atomics.waitAsync` is outside the pinned QuickJS parity target.
  The fixed smoke now
  passes all 193 variants with no unsupported result. See
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
  Those counts and report hashes preserve the historical R2b landing.
  Subsequent synchronous-arrow, generator, ordinary-async, and R3ab
  async-arrow work closes each adjacent frontier without changing this
  203-path / 205-variant evidence boundary. The same focused gate now passes
  205/205; its unchanged manifest/key hashes are
  `8f43b8f924d127814ea157637acebbb4e37fc89f97e6a76789e5e329d10250d6`
  and
  `1c04aebebd7c6e575113ca1466832c92096fef90af088aa1f3d317561aed0d4e`,
  while its current TSV/JSONL hashes are
  `f2f211cb3cc6619fda2c051d890f5994633d8962f1e98c58d2e9829e6289ee21`
  and
  `c3868df36a65922cac3f961ae82840fc90151f9f9312bc592e661d7c07ffca75`.
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
  direct-eval inheritance. At the R2h landing, parameter initializers,
  classes/derived construction, and async/generator methods remained separate
  frontiers; the later R3e slice now covers base classes.

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
  At the R2j landing, `super()` remained disabled pending class heritage and
  derived constructors; R3f later closes that boundary.

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

  The focused gate freezes 48 paths and 89 variants. It executes 85 and all 85
  pass; the later private-name work closed its two original staging
  frontiers. Two `create-realm` variants remain host-unsupported and two TCO
  variants remain excluded by the pinned configuration. Its
  manifest/key-set/non-pass hashes
  are
  `d3a7e597a049e9a78830ee089a90db27c6b6b0b8b2d049cd76b30f5515e6d23a`,
  `91852cd5c970debac2ef05af2715198736757b1276a34e6a73722df86bd80356`,
  and
  `cebe904ead643233ee754510a90cf53967525c4db1163281188b47aa56c80b50`;
  focused TSV/JSONL hashes are
  `a132ee39e73f44d77348b544427045069bb112ece353009ac7d5b2651fe51089`
  and
  `c32ef91f30cb4646228aee7cb2cd8a2445f4d6afa04c0173e4673f68acbb36b0`.

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
  At the R2m landing, the 22-path/44-variant raw gate recorded 36 passes, four
  unrelated rest/spread parse failures, two unrelated arrow-destructuring
  parser frontiers, and two pinned staging exclusions. Refreshed through R3d,
  all 42 runnable variants pass; the two staging exclusions remain. Stringify
  manifest/key-set/non-pass hashes are
  `001d8337407a2689dc181120160bc6d45d6b03765ec5ca0c2c7f3421f9705f11`,
  `ab8b0bdfa3895693115c79579f936d2559806dbc95f2588537267a73d6039892`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`;
  its R2m-landing TSV/JSONL hashes are
  `38ebfa11ff63d080072eb93845711ff4f90bd6753a70fa793edc0c128f89bd82`
  and
  `1ff4e957792cf2f1702f21df30bd7656d5448a71f5cf9fcc6f37c9cd48fa445b`.
  R2m-landing Raw JSON manifest/key-set/non-pass hashes were
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
  Derived/subclass forms, generator/object-generator, rest-parameter,
  lexical-destructuring, WeakSet, and `$262.createRealm` dependencies remain
  separate frontiers.
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
  At the R2p landing, all 806 Symbol-ready variants passed. The other 204
  exposed only independent class, rest/spread, Promise, buffer/TypedArray,
  Proxy, and weak-collection frontiers: 60 parse failures, 98 runtime failures,
  18 harness failures, and 28 typed parser frontiers. R3e brought the gate to
  864 passes while refining the old generic class diagnostics; R3f resolves
  all 28 derived-class parser frontiers, so the current gate passes 892 / 1,010
  variants. Its other 118 outcomes are the independent two parse, 98 runtime,
  and 18 harness failures. R2p-landing
  normalized-manifest/manifest-file/key-set/non-pass
  hashes were
  `eaf2a48408b6b1f5673389335cda73cb66bed062636a669c655460d9fef99a4b`,
  `6147636f7950b899f7c0eea25078e2f4c9c4c7fda2977181dd7c9671aa0bcde2`,
  `e87d58ad7a8be3e60b5545129a70a1abd70ee350654092a4aa066d17dc69e450`,
  and
  `4783b1a8bb909a6e4706138265c477cfa3979bb6821f09f590e4c8c66a0dd5d2`;
  R2p-landing TSV/JSONL hashes were
  `ed0363676e7efdfc6bb24ee396739cf67d49a4ce685c3bd37d98569a60a96267`
  and
  `75c40ff9adf28f0b9120c23af44268b4660189ff815e3f4c2ba0b74786ede048`.
  Current non-pass/TSV/JSONL SHA-256 values are
  `831fea4c50b0ffcf14e073a75fa75a4c6855bbadc5c7ed58fbc988c8b33cdf73`,
  `310560aa182de2df22b3a261157e92e6f94810a51adda918bea6e9f45fba5209`,
  and
  `d2fc654e57792e6670d21383e2cbc2c71d7638684ede17db28813dc126e9a409`.

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

  R2r extends that shared lowering to recursively nested ArrayBindingPatterns
  across direct `var`/`let`/`const`, classic `for`, and synchronous
  `for-in`/`for-of` declarations. Nested defaults, terminal rest patterns,
  elisions, and abrupt completion share the existing iterator-region path,
  including `IteratorClose` for every active iterator. Dynamic `with`
  References, whole-pattern AllowIn in classic-for initializers, and QuickJS
  malformed-pattern error priority remain pinned by differential tests.

  The dependency-audited R2r gate freezes 72 paths / 144 variants and passes
  all 144. Its exact one-feature scoped profile has SHA-256
  `c770387473b6ba2e273ab635182b5f07ae80ad902f48057ba5e2fb4f036c723e`.
  Normalized-manifest/manifest-file/key-set/TSV/JSONL hashes are
  `84d3c39bb9dcc81f16d92e8b30045a7b5c5d8c2fa6b24151a849633ae087d269`,
  `f7c7c181cdde65c84dfcb677cbe45f77884990666a774f952bc165df89f5e8a5`,
  `a95c253cbdaf997e9b6d4ed38a48c63e4ffc7400204137c5f4fdd693a815ca7f`,
  `39abfe594755acdeb26375bce7c173544bc9404ad5e96b7c6c4b0dd3f48b1c89`,
  and
  `d4f25a4495c080fd36c237077f323e9686a99b7b9dfdf192c93c18643467f187`.
  The exact R2q/R2r full join keeps all 102,037 keys and changes only the two
  sloppy/strict variants of
  `staging/sm/regress/regress-469625-03.js` from
  `unsupported-parser` to pass. There are zero previous-pass regressions or
  other outcome changes. Passes reach 34,880, runnable variants remain 38,421,
  and typed parser frontiers fall to 1,202. Full TSV/JSONL hashes are
  `10704652e6a0f24369203c0830bf8e70c7cf3ecd6e158823ee70dc5130d91214`
  and
  `53590c254bbb591279dc86b4bb8c668dd5f84098fb8eaa0410318e6f42e924d8`.
  Object rest remains separate because it additionally needs exclusion-aware
  `CopyDataProperties`.

  R2s ports fixed and computed recursive ObjectBindingPattern declarations
  from QuickJS 2026-06-04 onto the same shared lowering. Direct
  `var`/`let`/`const`, classic `for`, and synchronous `for-in`/`for-of`
  declaration heads accept identifier, String, numeric, keyword, computed
  String, and computed Symbol property keys. Defaults retain
  undefined-only selection and NamedEvaluation; object and array patterns can
  recurse into each other. Property-key conversion, sloppy `var` References,
  getters, initializers, and writes preserve QuickJS's observable order under
  `with`, while abrupt nested patterns retain the existing inner-to-outer
  iterator unwind and pending-error priority.

  The dependency-audited R2s Test262 gate freezes 324 paths / 648
  sloppy/strict variants across direct, classic-for, and synchronous for-of
  declarations; all 648 are runnable and pass. Its exact one-feature scoped
  profile has SHA-256
  `aa6cdca241b5f0be7eb202461ba80e44132f917a66480f1c04225cedc410d0d7`.
  Normalized-manifest/manifest-file/key-set/empty-non-pass hashes are
  `f6d9bda32460f3d16bd8084186c05b163e0d44a8788515fe20bf58a0f32d5c2d`,
  `ab9974676a1f15442875d6b9de607a27a94a76896a949c8b9cf86b05dbac18dc`,
  `bf712cfc7a3c455a2c8188baf82032876ba0321d3bf70d4c4281e00f4b945731`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`;
  TSV/JSONL hashes are
  `70d85400fb852c831a1088a8a53e52f8a693eea660f14fc2429983f499858d09`
  and
  `27218697cb5950df31ae2ef0610ca57d39ee531f4e33ab757a3145c72fafae52`.

  The exact R2r/R2s full join retains all 102,037 keys. Forty-nine outcomes
  change across 25 paths, another 71 rows change detail only, and no previous
  pass regresses. The outcome transitions are nine
  `fail-parse -> fail-runtime`, two `fail-parse -> pass`, two
  `fail-runtime -> pass`, two `unsupported-parser -> fail-parse`, two
  `unsupported-parser -> fail-runtime`, 30 `unsupported-parser -> pass`, and two
  `unsupported-runtime -> pass`. Passes therefore rise by 36 to 34,916 while
  runnable variants remain 38,421. Full TSV/JSONL hashes are
  `616026d35b7b86f6b4e6c24d22456db9ca50b64fcc00e787472e75aeebc3e3c2`
  and
  `a3f633ac23d0fe6d22dcec563ec7f2296f46b2be00738176b543079b7da283e6`.
  Object rest remains a typed frontier, but its `Unsupported` result is now
  deferred until the complete source has finished syntax and declaration
  scanning, so later syntax errors and declaration conflicts retain QuickJS
  priority. The next binding slice is exclusion-aware object-rest
  `CopyDataProperties`.

  R2t ports exclusion-aware ObjectBindingPattern rest declarations from
  QuickJS 2026-06-04. The bytecode carries typed target/source/exclusion stack
  depths and leaves those operands in place, matching upstream's
  `OP_copy_data_properties` lowering without encoding parser-local scope state.
  After source `ToObject`, a fresh exclusion object is created before any
  computed-key conversion or getter. Fixed and computed String/Symbol keys
  enter it before the rest copy; computed keys receive exactly one
  `ToPropertyKey`, excluded
  accessors are not read twice, and ordinary own enumerable String/Symbol keys
  are copied in QuickJS order with fresh writable, enumerable, configurable
  data properties. Direct `var`/`let`/`const`, classic `for`, and synchronous
  `for-in`/`for-of` declarations share the existing recursive binding owner.
  Sloppy `with` References are prepared before source enumeration and abrupt
  copy/Put failures retain the surrounding iterator-close priority.

  The dependency-audited R2t gate freezes 27 Test262 paths / 54 sloppy/strict
  variants; all 54 are runnable and pass. Its exact two-feature scoped profile
  has SHA-256
  `122a2b055aaf40672a0540441861ecd1e6c09b65e88d45b947bc27a691afc45e`.
  Normalized-manifest/manifest-file/key-set/empty-non-pass hashes are
  `381dc052af426d6d73e498600660d479c843dee1333896958b73176e23b705d7`,
  `fc75564488d2ae45a015fa8b07989f3a178f08978221d87ffdeeca0a9359fe57`,
  `4b1f4177d308124eb74c0eff3a8028c4bf09b5cf713392467f635e05b03f7e7e`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`;
  TSV/JSONL hashes are
  `9a1a364218204b9d6aede93dadd52cb97256b1504a0f016e8d41d46cca3b26be`
  and
  `53d8920bf0b160e0899a56af3a64fa50be354a899d78a8ec6864be96b3c79694`.

  The exact R2s/R2t full join retains all 102,037 keys and changes only the two
  sloppy/strict variants of
  `test/staging/sm/expressions/destructuring-object-__proto__-1.js` from
  `unsupported-parser` to pass. There are zero previous-pass regressions,
  missing/extra keys, or other outcome changes. Passes rise to 34,918 while
  runnable variants remain 38,421 and typed parser frontiers fall to 1,166.
  Full TSV/JSONL hashes are
  `0c4e7a6e1939aaee3926e8cd2b91e05af0f61a4bfb0cf0c932827e49ea7bb95c`
  and
  `512e97b82df170c24e262968c6ebf73fa450be92fb1f0db14aaa58d50c17d7f6`.
  Destructuring assignment, parameter patterns, and catch patterns remain
  separate compiler surfaces; assignment lowering is the next high-yield
  binding slice.

  R2u ports ArrayAssignmentPattern lowering from QuickJS 2026-06-04 for direct
  AssignmentExpression and synchronous `for-in`/`for-of` assignment heads.
  The direct control-inverted path keeps an independent RHS copy, so the
  expression returns the original iterable while the pattern consumes its
  working value. Identifier, fixed, computed, and `super` targets prepare their
  complete Reference before `IteratorStep`; depth-addressed steps then preserve
  computed-key conversion, Put, `with`, and abrupt-completion order. Empty
  patterns, elisions, undefined-only defaults with NamedEvaluation, terminal
  rest, and recursively nested arrays share the existing iterator-region
  machinery. Matching-closer lookahead keeps leading Array/Object literal
  member targets on the ordinary for-head path. Valid object assignment remains
  a typed frontier, but a destructive syntax pass lets malformed object targets
  retain QuickJS's earlier `SyntaxError` priority.

  The focused pinned-QuickJS differential passes 12/12 Rust tests covering 31
  semantic sources, 23 exact parser CLI diagnostics, eleven exact
  iterator-origin stack traces, the object-assignment frontier, and a Rust-only
  smoke. The dependency-audited Test262 gate freezes 70 direct
  flat-array paths / 131 sloppy/strict variants; all 131 are runnable and pass.
  Its exact five-feature profile has SHA-256
  `b2133d90974566c72ab788525254de68d260b44756a8c5981111873fb38727af`.
  Normalized-manifest/manifest-file/key-set/empty-non-pass hashes are
  `ee0b310ee20a89e3cff58469a4a7020a4a73980f5086fe189964a2c6c10c120f`,
  `046679bd745132066b4982770f13236bfecdbd953b70bdba98afa60424c599c8`,
  `093abb8f2b240a97cd1bcf5728cbd720203e91b5ed9df00d22f0394cd86ef4cb`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`;
  TSV/JSONL hashes are
  `e3b579aacafa0f63e1e17857b242311ca2512481e86f8ddbe55fcbf28267df51`
  and
  `832eebb660ad3f50771c60348d203cb5eaef7055098d2a07098f86d04a1b5fc8`.

  The exact R2t/R2u full join retains all 102,037 unique keys and changes 33
  outcomes: 14 `fail-parse -> pass`, one `unsupported-parser -> pass`, 14
  `fail-parse -> unsupported-parser`, and four `fail-parse -> fail-runtime`.
  No previous pass regresses, and there are no missing, extra, duplicate, or
  detail-only rows. Passes rise by 15 to 34,933 while runnable variants remain
  38,421. The newly exposed non-pass cases stop honestly at object assignment,
  Proxy, or an existing staging semantic frontier. Full TSV/JSONL hashes are
  `17c3c36e73ad8d098ae9d3bd3fc5c5d372187830d5e11f8532bc28471fbb4da3`
  and
  `e9cb57c7616c27e01e156e7754b9cbc606c40100ea632bcc651c411d10c6c8e9`.
  ObjectAssignmentPattern is the next assignment slice; parameters and catch
  patterns remain independent compiler surfaces. A synchronous for-head nested
  iterator-acquisition fault also still inherits the existing for-of control
  marker rather than QuickJS's RHS value site; its behavior is correct, but
  that debug-frame provenance remains a separate for-of source-map follow-up.

  R2v replaces the ObjectAssignmentPattern frontier with QuickJS-shaped
  lowering for direct AssignmentExpression and synchronous `for-in`/`for-of`
  assignment heads. Direct assignment shares the array path's control
  inversion and therefore returns the untouched RHS while consuming a separate
  working value. Each object first performs `ToObject`; ordinary leaves then
  run PropertyName/`ToPropertyKey`, prepare the full depth-0-to-3 target
  Reference, read the source property, apply an undefined-only default and
  NamedEvaluation, and perform a NOKEEP Put. Nested patterns intentionally read
  the outer property before preparing inner References. Rest prepares its
  target before copying own enumerable String/Symbol properties through the
  existing exclusion object and `CopyDataPropertiesExcluded` opcode. Array and
  object patterns now recurse through each other; no new VM instruction or
  runtime path is required.

  The focused pinned-QuickJS target passes 9/9 Rust tests: 35 eval
  differentials, five exact CLI stack traces, 14 exact parser diagnostics, and
  one Rust-only smoke. Three independently runner-bound Test262 cohorts freeze
  flat, nested, and rest behavior at 67/14/26 paths and 118/24/51 variants;
  all 193 variants pass. Their profile SHA-256 values are
  `989f5617484d5c12a15fb26a447121fa3436b19f05cd998cf400b5d3d7179a51`,
  `18411f3d674a9493806bbf6a601bda903e859395aeec572e466c4a59470ceb12`,
  and
  `4b9f50b982dc5c3af1466d425a1665448c4a00165d465a74fd4057ef6e414206`.
  Focused TSV/JSONL hashes are respectively
  `f0cd537e2349ce952828c6c61c073636b8631ca27750c7decbc4a8cd634087c6` /
  `27456fb05f0015a01c37f2d6c35a0d2b44e49a20578b9e0eabe5c57d53c546d9`,
  `430391c59cb61029ecdb1b7f2d81b0ec7054cba76f6bbfdab8b0840baf438669` /
  `cad849b67be5b15bbe7fd63b1fa635c5f74f4d2e05c8b65941fe076bb762a37a`,
  and
  `14d7dba398df75de6aa4583fe126ffc3aca871890121a7f6d53df71d8da4e4de` /
  `b6cb010459de59ffaab193fb7ad5fddc9fb73b1f8e437f8041fd2a56ba358964`.
  The broad `destructuring-binding` and `object-rest` tags remain scoped: they
  also cover unsupported parameter, generator, async, and class surfaces.

  The exact R2u/R2v full join retains all 102,037 keys and moves all 14 former
  object-assignment `unsupported-parser` variants to pass, with zero previous
  pass regression, missing/extra key, or detail-only change. Passes rise to
  34,947 among 38,421 runnable variants and typed parser frontiers fall to
  1,165. The same measured run also moves both modes of the unrelated
  `staging/sm/Proxy/ownkeys-linear.js` from its eventual missing-Proxy
  `ReferenceError` to the 30-second timeout because its 15,000-property setup
  now crosses that host limit; this is recorded as performance noise, not an
  ObjectAssignmentPattern result. Full TSV/JSONL SHA-256 values are
  `bbc5babdb70a470ff6d937dde2771cb7de270bc6971bfc7597e1f5bf0b24e5da`
  and
  `2839c0d58d8661b6cec4f6e606d297625343756dbbd656224013c17f992743fe`.
  Nested object reads keep stable Rust source sites but do not yet reproduce
  QuickJS's inherited source marker exactly; that joins the existing nested
  for-head marker as source-map debt. Destructuring parameters and catch
  patterns remain independent compiler surfaces.

  R2w ports recursive catch ArrayBindingPattern and ObjectBindingPattern
  lowering from QuickJS 2026-06-04 onto the shared declaration binding owner.
  Identifier leaves, elisions, defaults with NamedEvaluation, terminal array
  rest, fixed and computed object keys, object rest, and arbitrary array/object
  recursion initialize inside the catch lexical scope. Iterator/property abrupt
  completions reuse the verified exception and iterator-unwind paths. Pattern
  leaves are ordinary mutable catch-scope lexicals; only a simple catch
  identifier retains the private catch-parameter marker needed by direct-eval
  `var` redeclaration rules. The handler target lands on explicit catch-scope
  preparation rather than executing a normal `EnterScope`, matching the pinned
  QuickJS control-flow shape while initializing every pattern leaf before the
  binding operations run.

  The dependency-audited R2w Test262 gate freezes 97 paths / 177 variants and
  passes all 177. It covers the implemented synchronous try-destructuring
  corpus, six audited parse-negative rest cases, Annex B catch-body early-error
  integrations, and four untagged catch-scope paths. Generator-valued and
  unsupported derived/class-element defaults remain separate frontiers. Its
  exact four-feature scoped profile has
  SHA-256
  `a654327057a974e0feab6799f3c99a3104884a403cbc41bbc85f3fc226328718`.
  Normalized-manifest/manifest-file/key-set/empty-non-pass hashes are
  `50c326ca60fdfa0cd5d3683df265e730c1947801db6e0892645b9bcfcd450927`,
  `e3fb469169b069c185a7d9ea6b8cdce2fdb54d49181b7e87e33cff59a27c212e`,
  `1f66a5b898cf1f0cb4a3dc333ee3bb4e7d5dc1361dd5a06b7c1c4be2b0573784`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`;
  TSV/JSONL hashes are
  `c1a01134926200028f476ca165ed8127566725bab5faa1a174e77b9f4f460557`
  and
  `4215e94bb7c8435345542d80ebfcad56ff91567cb4c45582c3cf8426f66dc3da`.
  The broad `destructuring-binding` and `object-rest` tags remain scoped rather
  than entering the global profile.

  The exact R2v/R2w full join retains all 102,037 keys and adds 49 passes: 24
  `unsupported-runtime -> pass`, 23 `unsupported-parser -> pass`, and two
  `fail-runtime -> pass`, with zero previous-pass regression. Both modes of the
  unrelated `staging/sm/Proxy/ownkeys-linear.js` move from timeout back to the
  eventual missing-Proxy runtime failure; this is performance noise rather than
  a catch-binding regression. Passes reach 34,996 among 38,421 runnable
  variants; typed parser frontiers fall to 1,142, typed runtime frontiers fall
  to 50, and timeouts fall to six. Full TSV/JSONL SHA-256 values are
  `e00e85d148fcc5d03ff7830b0e730af0a64b478c498eaad8d018d0bf1c96898a`
  and
  `ace137cda9b5f55762b2e729a172adbed3715659c981c53bd809f9099fcf20ae`.
  Destructuring parameters remain the next independent synchronous binding
  surface.

  R2x adds the identifier-only synchronous rest-parameter slice shared by
  ordinary function declarations and expressions, synchronous object methods,
  arrows, and the `Function` constructor. Rest collects only actual trailing
  arguments into a fresh callee-realm Array, formal `length` stops before the
  rest slot, and sloppy non-simple functions receive an unmapped `arguments`
  object which snapshots raw arguments before rest initialization. The entry
  prefix initializes `arguments` and rest before body function hoists, so body
  `var` reuse and function replacement follow the pinned QuickJS order.
  Duplicate-name, `"use strict"`, position, trailing-comma, initializer, and
  accessor-arity diagnostics are exact across the admitted forms. Publication
  authenticates the rest operand, formal metadata, and prologue shape before
  the VM may slice an active frame.

  The runner-bound R2x Test262 gate freezes 34 paths / 65 variants and passes
  all 65. Its six-feature scoped profile includes 11 audited negative paths;
  the broad `rest-parameters` tag remains absent from the global profile. The
  profile SHA-256 is
  `da6a76cb6338019f5c233e252bf6d40b7f3eb5c4235a6967cf78f9a74917dced`.
  Normalized-manifest/manifest-file/key-set/empty-non-pass hashes are
  `5cfb4770e35f128a3481a15dcff70dc4733657072fe9cf7a185c91624c355b43`,
  `cc326a73c13d2cd90726150e77ad5f5a247074f12a233fe9efa382b3ec6c420e`,
  `5a3751688f145e0eda20738258675c1ee27f86fc7808a8a2654dae88d3917c1a`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`;
  focused TSV/JSONL hashes are
  `7b28768f2bb46974d563728cda36e025bc5123f8d3749a32bf83a490e0ac691f`
  and
  `0a2d3aa3518bc8ab10c5f2bbf768bbd94bc88e809202837416849c63dfa14065`.

  The exact R2w/R2x full join retains all 102,037 keys and adds 88 passes: 31
  `fail-parse -> pass`, nine `unsupported-parser -> pass`, three
  `unsupported-runtime -> pass`, and 45 `harness-error -> pass`, with zero
  previous-pass regression. Sixty-one other rows change outcome without
  becoming passes, and ten rows retain their outcome while changing detail.
  Passes reach 35,084 among 38,421 runnable variants. Full TSV/JSONL SHA-256
  values are
  `1ff253545ba69824b686e23d40998645a57330d83fa01a8bf9a39fa2994e4959`
  and
  `6a1971269b694b9c5e344884714f9f2234619a3200b6ff2e25a69e2b45e26fb9`.
  This is not complete rest or FormalParameters support: Parameter
  Environments and defaults land in R2y, while parameter destructuring, rest
  BindingPatterns, and async/generator forms remained explicit frontiers at
  that landing; R3e now covers synchronous base-class constructors.

  R2y adds synchronous `BindingIdentifier = Initializer` parameters to ordinary
  functions, object methods, arrows, and the `Function` constructor. Parsing
  now establishes the child function before consuming its formals and creates
  a real, parentless Parameter scope at the first default. One mutable lexical
  cell per parameter is reset to TDZ and initialized left-to-right; initializer
  closures retain those cells, later/self references throw, outer bindings and
  function pseudo-bindings remain visible, and body declarations stay hidden
  until the body scope is entered. The body deliberately continues to read and
  write raw argument slots, matching pinned QuickJS's observable `2|1` split
  between a body assignment and an initializer closure rather than silently
  adopting the different Node/spec result.

  Default substitution writes the selected value back to the physical argument
  slot before initializing the lexical cell, non-simple `arguments` is
  unmapped, function `length` stops before the first default, NamedEvaluation
  supplies anonymous function/arrow names, and defaults compose with terminal
  identifier rest. Body function hoists are installed only after leaving the
  Parameter scope and entering the body scope. Immutable bytecode metadata now
  publishes the leading parameter-local count; both publication boundaries
  authenticate the structural ABI: reverse-order TDZ reset, left-to-right
  single initialization, default+rest's exact
  `Rest/Dup/PutArg/InitializeLocal` shape, and any preceding pseudo-binding
  pair. The unlinked boundary additionally authenticates mutable lexical
  definitions and the HomeObject, `new.target`, and `this` binding names. It
  also binds every referenced child closure to the segment which instantiates
  it: initializer closures cannot capture raw argument slots, and body
  closures cannot recapture Parameter cells after that environment closes.

  The pinned QuickJS oracle freezes 15 semantic vectors, including the target's
  raw-argument/parameter-cell split. The runner-bound Test262 gate freezes 76
  paths / 143 variants and passes all 143. Its scoped profile admits only
  `default-parameters` and the required `super` cases plus 19 exact negative
  paths; `default-parameters` remains absent from the global profile. Profile,
  manifest-file, focused TSV, and focused JSONL SHA-256 values are
  `5c98d19ccb72c7e2c577ddc98ee4ac83d43a0ba7d49175a8ebe271866d0feab6`,
  `264bb2b25e7502eed86f8a5df1b3fe8c0ccdeecd43171af390764b5e053a6472`,
  `f1775881f89d5b76f7a46f1a89391a60b213508becec9df244e2fb0d9a937bc7`,
  and
  `dc1edd9121ce27142df0e499a8e4ccdca1e6ff43ca178a35ea40981d45538a23`.

  The exact R2x/R2y full join retains all 102,037 unique keys and every
  previous pass. It adds 60 passes: 35 `fail-parse -> pass` and 25
  `unsupported-parser -> pass`. Thirty-eight former parse failures now stop at
  the explicit direct-eval/destructuring/class parser frontier and 16 reach
  already-known runtime frontiers; 64 rows keep their outcome while exposing a
  deeper diagnostic. There are no missing, extra, or duplicate keys. Passes
  reach 35,144 among 38,421 runnable variants. Full TSV/JSONL SHA-256 values are
  `e02a1e768065e63af6908932dc7ba8e5ff9ec552c3dc6adbce55db91a74eb866`
  and
  `b762e44abbca482419b5e24ed4479a1726a8c7d25232907538c71780829d4def`.
  Direct eval in or below a Parameter Environment remains an explicit
  `Unsupported` boundary because QuickJS parity requires independent `<var>`
  and `<arg_var>` variable objects and exact function-segment topology; it is
  intentionally not approximated here.

  R2z adds synchronous FormalParameters BindingPatterns on the QuickJS
  `SKIP_HAS_ASSIGNMENT == 0` path. Ordinary declarations/expressions, object
  methods, arrows, the `Function` constructor, and one-argument setters share
  recursive array/object/rest lowering. Ordinary patterns reserve anonymous
  physical argument slots; terminal rest patterns reserve no slot and retain
  QuickJS's observable `length` quirks, including its zero-initialized
  bytecode-record behavior when a function has no arguments or locals.
  Pattern initialization runs in FunctionRoot before body lexical entry and
  before body function hoists, with unmapped `arguments`, direct eval, computed
  keys, HomeObject/`super`, iterator closing, and closure visibility matched to
  the pinned source. Both publication boundaries authenticate the anonymous
  reads, rest start, initialization marker, control-flow boundary, arguments
  prologue, and the prohibition on direct body-lexical access. The complete
  tree publisher additionally authenticates child instantiation and rejects
  pattern-phase closure capture of a body lexical cell.

  The runner-bound R2z gate derives 37 dependency-clean generated paths from
  each of four synchronous surfaces and adds one direct unmapped-arguments
  consumer: 149 paths / 298 variants, all passing. Its three-feature scoped
  profile contains 12 audited negatives. Profile/manifest/key-set/focused
  TSV/JSONL hashes are
  `1f25a0648044b6cb3027e23bc58032b2b2fc3517cd0a29b35d5e4d0844fc6e5e`,
  `9cb9662c3c5860e05ba2199be6d3818091e64780ccf7ef61c6d63276a6747f60`,
  `3dbed4631c1c6670bae9256f82773b62ad7a82facda80dac0fb72187fd546e92`,
  `9ef03e119426a2f65dadf3898e63fa48af05469e2f194f1d6c3ab20a3d8cc9db`,
  and
  `0a23a3e1252ddfa2cf0d8fd708b1c0646f13a8d5ccf45098b4ed102c0f3814c1`.
  The exact full join retains all 102,037 keys and every previous pass. It adds
  22 passes, moves 11 old failures to the explicit Parameter-Environment
  frontier, and changes only 14 same-frontier diagnostics. Passes reach 35,166
  among 38,421 runnable variants; full TSV/JSONL hashes are
  `5d85f32719d07937a0e352cc665911c94014ae1f910292100821692c9cbe4546`
  and
  `2818623121c2991151fdb0c055090283fd5f131e5dcfdd135b97fcdb77df708c`.
  BindingPatterns whose FormalParameters contain a standalone `=` require the
  independent Parameter Environment and are the next R3a boundary; async,
  generator and class forms remained later callable milestones at that
  landing; R3e now covers synchronous base-class callables.

  R3a completes synchronous parameter-expression BindingPatterns on that
  independent Parameter Environment. A standalone `=` anywhere in the whole
  FormalParameters list creates the parentless argument scope before the first
  parameter is parsed, and every identifier/pattern BoundName receives a TDZ
  cell in source order. Named parameters preserve raw physical argument reads;
  pattern names are initialized in the argument scope and copied to fresh body
  locals only after all defaults have run. Initializer and body closures thus
  retain QuickJS's distinct capture targets. The zero-cell environment,
  whole-pattern versus leaf-default `length`, rest-pattern initializer quirk,
  getter/setter arity, duplicate-name priority, and hidden `arguments` object
  cases are pinned by focused QuickJS differentials.

  The immutable `ParameterEnvironmentLayout` crosses both publication
  boundaries into the heap and records initialization, named argument cells,
  pattern copies, raw default sources, and reserved future eval/arguments
  slots. The publishers and Heap independently authenticate TDZ order, exact
  initialization/default/copy skeletons, cross-phase jumps, body reads, and
  child closure captures. This makes direct eval in or below the Parameter
  Environment an explicit later `<arg_var>` ABI extension rather than an
  approximation.

  The dependency-audited R3a gate derives 117 paths from each of four
  synchronous surfaces: 468 paths / 936 variants, all passing. Its four-feature
  scoped profile contains 36 audited negatives. Profile/manifest/key-set/
  focused TSV/JSONL hashes are
  `0addc7345b6576e1944afc3d5d84cffe16e299e44af09245e78c08cb29207f7b`,
  `1db4662456a3ea231c7ce3f629d5224a8cb19d38d13d69c83e43f6407aac21c0`,
  `5d4d801025b940f11608d4110169daf6f15427a063e26ca0b1770587a11f464b`,
  `e7292d11cc347daf9016b28a987626ee648fc64e4740161ce843058a6fe7265c`,
  and
  `e6ad140b2e960920c4586455ee9905b4c982ba63e4aa7a9cfc102542c0de8827`.
  The 20-vector QuickJS oracle passes all four integration tests. Pinned
  Test262 has no exact BindingPattern + standalone `=` + terminal
  identifier-rest path, so three custom vectors freeze that otherwise invisible
  cross-feature entry shape across functions, arrows, and object methods.

  The exact R2z/R3a full join retains all 102,037 keys and every prior pass.
  Twelve `unsupported-parser` variants become passes, two untagged staging
  variants advance from the typed runtime frontier to existing adjacent
  failures, and 15 same-outcome rows expose deeper diagnostics. There are no
  missing, extra, duplicate, or regressed-pass keys. Passes reach 35,178 among
  38,421 runnable variants; full TSV/JSONL hashes are
  `a529e8bc7556be32188fa20dd9a2db121e7feba4cc0dede5d4a1882b4ba363ec`
  and
  `78839d051f03908350eded05b8ea99c6d9843f4668ec4aa3673b50ca60e710da`.
  The canonical complete gate now uses two workers: its timeout is wall-clock,
  and higher contention can make the pre-existing 15,000-property
  `Proxy/ownkeys-linear.js` setup cross 30 seconds before reaching the missing
  `Proxy` frontier. Focused gates retain their existing parallel defaults.
  R3b implements sloppy direct eval in and below that non-simple Parameter
  Environment. Following pinned QuickJS, each activation now owns separate
  body `<var>` and parameter `<arg_var>` variable objects. Body lookup is
  static body -> `<var>` -> `<arg_var>` -> outer, while parameter lookup is
  static parameter -> `<arg_var>` -> outer; strict eval keeps its declarations
  local. Compiler, unlinked publication, complete-tree publication, Heap, and
  VM exchange typed `FunctionRoot`/`Parameter` target metadata instead of
  inferring either object from an untyped closure slot.

  QuickJS's parameter-time synthetic `arguments` cell is now distinct from a
  named `arguments` parameter and from the ordinary body binding. Descendant
  arrows append a late body-arguments closure only when authored code really
  captures it; an eval-only descendant does not manufacture one. BindingPattern
  initializers may read or close over the synthetic cell, body closures may
  retain `<arg_var>`, and the publication validators authenticate each exact
  role, sentinel, capture segment, and lifecycle operation.

  The entry composer now emits and verifies one upstream-ordered prefix:
  HomeObject, `new.target`, `this`, arguments, `<var>`, `<arg_var>`, then the
  reverse TDZ prologue. Global function hoists follow that prefix. This closes
  the composition debt recorded by R3a while keeping ordinary script
  `PushThis; PutLocal(eval_completion)` bytecode distinct from a hidden
  `<this>` pseudo binding.

  The R3b QuickJS oracle contains 42 reviewed vectors: 13 parameter-target, 7
  environment-split, 4 dynamic-object, 13 arguments, 1 entry-order, 2
  scope-switch, and 2 strict-eval cases. All four Rust integration tests pass.
  The focused Test262 gate freezes 71 `noStrict` paths / 71 sloppy variants:
  48 arguments/direct-eval matrix cases, 16 scope open/close cases, 4
  redeclaration negatives, 2 computed/default cases, and 1 staging composite.
  All 71 pass in Oxide and in pinned QuickJS `run-test262 -a -m`. Profile,
  manifest, key-set, focused TSV, and focused JSONL SHA-256 values are
  `98b5e323db1b4be493c1e05b8937a1060b71f7a1cc126087d05e88e7c2a2b335`,
  `3df66805796888dd41acbc007b2a958aba5751e9694c0deffa5f0efba19c61a1`,
  `08aeb2a3e23a3a3e1bb6e03262d730cd0bbaec1d9aff0f9cc744ebc3ce003938`,
  `e2759eb05400218abb31e257fe60bedfcb321e05bbffc0018d9042b60c87ec12`,
  and
  `a25aaf9087fc356b4b5b3d8437a52cf19166c76ec09aeefc5569f4297a93844d`.

  The exact R3a/R3b full join retains all 102,037 unique keys and every prior
  pass. All 66 focused `unsupported-parser` variants become passes. Outside the
  manifest, one untagged staging case reaches its known implicit-`this`
  runtime mismatch and two variants reach the generator-method typed runtime
  frontier. Those are the only three other outcome or row changes: the join
  has no missing, extra, duplicate, same-outcome-detail, or regressed-pass
  keys. Passes reach 35,244 among 38,421 runnable variants; full TSV/JSONL
  SHA-256 values are
  `41ef0f16cbae0aa05cdc0bfb13e38130b9b87b1ac958fe6e807541140cda918a`
  and
  `ecd12b154863534e80f5ac0f40ee6615f1a8743856e9e4f9ca98b44e00a793a0`.

  R3c publishes QuickJS-shaped `AggregateError` and audits the already-shared
  Error `cause` path. Construction now follows the pinned order: resolve
  `newTarget` and allocate the branded Error object, convert `message`, install
  `cause`, consume `errors`, define the own `errors` Array, then capture the
  stack. Iterator acquisition, cached `next`, abrupt `done`/`value`,
  IteratorClose, and original-throw priority are covered by 19 QuickJS oracle
  vectors; all three oracle integration tests pass. Rust cross-realm tests
  additionally pin defining-realm Array allocation and newTarget-realm
  prototype fallback.

  The complete 28-path focused feature cohort expands to 56 variants. Fifty
  pass. The remaining six are the sloppy/strict modes of three upstream tests
  whose metadata omits their actual `Proxy` dependency; they are pinned as
  `ReferenceError: 'Proxy' is not defined`, not counted as AggregateError
  semantic failures. Profile, manifest, key-set, focused TSV, and JSONL hashes
  are
  `ad9e38f7b1b42445a848ee01437e925fc23f5525276bc45dd15c5ae7a1454d7a`,
  `f54979cc3881fd7d361dda7ffbbe75a5bf846e233512c7428711c1091b8474c5`,
  `81e86c6e47fcc63ab2063814e34125de57fbc2ed14a8802186db5caa1be6bf5d`,
  `40ee7c2976c4319b09457e311ed103bd3851a5a82ae11587794aa3dbc457b537`,
  and
  `019abe8aedfd1c82ee283aeb976a2364b1e124f91cb401c67407bb17556bd01b`.

  The exact R3b/R3c full join retains all 102,037 unique keys and every prior
  pass. It records 52 `unsupported-feature -> pass`, including the two modes
  of `Object/seal/seal-aggregateerror.js` outside the focused manifest; six
  `unsupported-feature -> fail-runtime` transitions at the undeclared Proxy
  dependency; and four `unsupported-feature -> unsupported-parser`
  transitions at the existing class frontier. There are no missing, extra,
  duplicate, or previous-pass-regressed keys. Passes reach 35,296 among 38,483
  runnable variants. Full TSV/JSONL SHA-256 values are
  `8579dc70c2b02843b3b0e7680be35d48807bf24f17e3a6b3b2d7daabe6cfb71e`
  and
  `72296c8615ac07f1de8305445ff7fd9b170eb00b37e616e35679051a90536525`.

  R3d adds argument spread to ordinary, method, constructor, and direct-eval
  calls through typed `Apply(Call)`, `Apply(Construct)`, and `ApplyEval`
  bytecode. The shared dense argument-list path preserves method receivers,
  authenticated eval environments, source/value rooting, and QuickJS's
  callable/list/constructor and eval-identity error order. Append performs the
  two observable `@@iterator` Gets used by the target and reproduces its
  fast-Array quirk: after a genuine dense Array is classified by the first Get,
  a direct built-in cached Array iterator-next method causes values to be
  copied from the original Array without advancing or brand-checking that
  second iterator.

  The focused gate freezes 67 paths / 134 variants: 122 pass, while twelve
  runtime failures form the exact adjacent-feature frontier. Fifteen
  automated Oxide/QuickJS semantic differentials all pass. At the R3d
  checkpoint, three dense 65K Oxide stress vectors were ignored in routine
  automation because the then-current immutable-shape model made construction
  O(n²); their pinned QuickJS expectations were self-checked, while the shared
  65,534/65,535 argument limit was checked quickly by
  `oracle_function_apply`. R3am removes that shape-growth bottleneck; the cases
  remain explicitly marked as stress tests.

  The exact R3c/R3d full join retains all 102,037 unique keys and every prior
  pass. It records 122 `fail-parse -> pass`, ten `fail-parse -> fail-runtime`,
  two `fail-runtime -> pass`, and 13 `fail-parse` detail-only refinements, for
  147 changed complete rows. Passes reach 35,420 among 38,483 runnable
  variants; fail-parse falls to 259 and fail-runtime reaches 1,553. Full
  TSV/JSONL SHA-256 values are
  `8fe66b2478571da55c1061a56ca521fbc8f3926591eb6093d3ac537f4cdccf60`
  and
  `e6ae2522eb1790119f95537d946c90fb529222e9d649710ea8e1c07fd715a89b`.
  The refreshed Symbol protocol gate passes 864 / 1,010 variants, and all 42
  runnable Raw JSON variants pass.

  R3e ports the base-class path from QuickJS `js_parse_class`,
  `js_op_define_class`, and `OP_define_class`. Declarations and expressions
  now carry separate outer and immutable inner lexical bindings with TDZ;
  explicit/default base constructors are construct-only and preserve
  parameter/default/rest order, length, `new.target`, return validation, and
  exact descriptor shapes. Instance/static synchronous methods and accessors
  support fixed/computed names, inferred names, strict bodies,
  non-constructability, and HomeObject-backed `super` property access.

  The focused Test262 gate freezes 157 paths / 294 variants and passes all
  294; pinned QuickJS passes all 157 paths, and all five Rust oracle/frontier
  tests pass. The gate's scoped profile admits `class` only for the frozen
  manifest; the global profile deliberately does not claim the whole feature.
  Profile/manifest/key-set/TSV/JSONL SHA-256 values are
  `df73a1ac299cce6ade0b0638f0a4c3322310aa2db8e15a28039f483328e69f00`,
  `0894fc15cf840a8897ad1b9243324c6312f28fd90e78cdafa377170d15b79f5f`,
  `bb0c150613a6e85b4699f612b1c4755f04cd55a60384e8e3ac5b21e543e8de8b`,
  `6049119789bd02e1d7848ec661a693c4161b769592b6567e567b21a17122703c`,
  and
  `7a10a6964629fdb96ed239be78587d9d1ebfdb6fd856549fbe813e5d28352521`.

  The exact R3d/R3e full join retains all 102,037 unique keys with no missing,
  extra, duplicate, or previous-pass-regressed key. It records 324
  `unsupported-parser -> pass`, four `unsupported-runtime -> pass`, 50
  transitions to deeper honest failures/frontiers, and 719 same-outcome
  diagnostic refinements: 1,097 complete rows change. Passes reach 35,748
  among the same 38,483 runnable variants; fail-parse is 273 and fail-runtime
  is 1,587. Full TSV/JSONL SHA-256 values are
  `10e3fee1e93b3491b4c97041990cd17a7f1051dbcd2d0d13c6514961934200ae`
  and
  `b863a62f5e7dbfcff8975fae28251731b80103f63b3c039d62f1f98271720ada`.
  The full corpus exposed and now locks a named class captured inside a
  Parameter BindingPattern initializer; explicit initializer-scope provenance
  is authenticated at both publication layers without weakening body-lexical
  isolation.

  R3f ports class heritage, derived constructors, and `super()` from the pinned
  QuickJS parser/runtime path. Heritage is evaluated as a
  LeftHandSideExpression before the class body; publication preserves
  IsConstructor-before-`prototype` ordering, `extends null`, constructor and
  instance-prototype reparenting, and abrupt-completion ordering. Derived
  constructors keep `this` in TDZ until a successful `super()`, forward raw
  actual arguments in the synthesized default constructor without iterator
  lookup, snapshot the active constructor's live `[[Prototype]]` before
  explicit arguments, preserve `new.target`, and apply the distinct
  object/undefined/primitive return protocol. The same authenticated cells are
  relayed through arrows, parameter initializers, and nested direct eval.

  Constructor authority is typed rather than inferred from ordinary stack
  values. `MarkSuperCall`, `ConstructSuper`, and `ApplySuper` protect the
  authenticated active-function/new-target pair through argument control flow;
  publication traces active function, new target, and derived `this` through
  ParentLocal, ParentClosure, and EvalEnvironment origins. Generic construction
  cannot initialize derived `this`, repeated initialization remains one-shot,
  and the heap boundary pins the synthesized default constructor to its exact
  shape.

  The final synchronous dependency closure is 386 paths / 767 variants. Oxide
  passes all 767 variants with no failure, unsupported result, engine fault, or
  runner fault; pinned QuickJS passes all 386 paths. The focused
  profile/manifest/key-set/TSV/JSONL SHA-256 values are
  `1aa167fef279273185060224bd8a65765283d95fe1e08986c5c4ea197657e160`,
  `c9c477104d7f538c4b3fa58a108171be866273bedf19825bedf682afc9d00366`,
  `366f33fe39e2980a2a7e6c94e4e20896cd415b8e93b0118f69bc33c39c07e1e5`,
  `69467d4d2f8c76ec299e97ce9c88bf74cee35e5cdae42e029377761aa25e4b8a`,
  and
  `abbe6c64c2fe250f477cf95085c9201a9b9654a2ef01deaa826dff1fea9b1193`.
  The dedicated Rust/QuickJS oracle also passes both fixed and differential
  observations.

  Two overlapping existing scoreboards independently record the same class
  progress: the named-groups gate moves four derived-RegExp variants to pass
  and reaches 198/202, while the Symbol-protocol gate moves 28 derived-class
  and spread-`super()` variants to pass and reaches 892/1,010. Neither loses a
  prior pass.

  The exact R3e/R3f full join retains all 102,037 unique keys and every prior
  pass. It records 545 `unsupported-parser -> pass`, 37
  `unsupported-parser -> fail-runtime`, two `unsupported-parser -> fail-parse`,
  and 49 `unsupported-harness-parser -> harness-error` transitions, plus six
  detail-only refinements: 639 complete rows change. Passes reach 36,293 among
  the same 38,483 runnable variants; full TSV/JSONL SHA-256 values are
  `018c55de6e745b35eae7bb8f7d1c3b7680579a58d8bbb241641d860c723a0e34`
  and
  `995cce2dc58694f8728e1ad12602b2ec5c65169f650cff5047e45d84bc4b407a`.
  At the R3f checkpoint, global `class` remains disabled: fields/private
  elements, static blocks, async/generator methods, unsupported intrinsics, and
  host hooks stay explicit later frontiers.

  R3g ports QuickJS's public class-initialization path: public instance fields,
  public static fields, and static blocks. Computed keys are evaluated exactly
  once during class definition; static fields and blocks execute in source
  order, while instance fields become own writable/enumerable/configurable data
  properties. Base constructors initialize fields before parameter
  initialization and the body, and derived constructors initialize them after
  each successful one-shot `super()` result establishes `this`. Anonymous
  function/class names, HomeObject-backed `super`, direct eval, abrupt
  completion, static-block lexical isolation, and the pinned `arguments` /
  `await` / `yield` early-error boundaries follow the QuickJS 2026-06-04
  behavior.

  Hidden typed bytecode children separate instance fields, aggregate static
  elements, and individual static blocks. Dedicated VM bridges are responsible
  for installing or invoking those children, keeping constructor authority and
  GC ownership explicit rather than treating initializer functions as ordinary
  source-visible callables.

  The R3g dependency-audited gate is a distinct cohort that also contains 386
  paths / 767 sloppy-or-strict variants. Oxide passes all 767 variants with no
  failure, unsupported result, timeout, or infrastructure fault; pinned
  QuickJS passes all 386 paths. Its admission profile is scoped to the frozen
  manifest, so this result does not claim all of Test262 or enable whole-feature
  `class` globally. Private elements, async/generator class forms, Proxy and
  other excluded adjacent dependencies remain fail-closed. The dedicated
  QuickJS transcript additionally freezes computed/static/instance order,
  inferred names, descriptors, HomeObject, lexical scope, and abrupt
  completion.

  R3h ports the field-only private-element path: private instance and static
  data fields, private reads/writes/updates, and `#name in value`. Each class
  evaluation creates fresh typed private-name cells; field storage is own-only,
  hidden from public reflection, and independent of ordinary extensibility.
  Nested functions, arrows, and direct eval retain the same authenticated name
  identity without exposing it as an ECMAScript value. The implementation is
  anchored to QuickJS 2026-06-04's private-field operations at `quickjs.c`
  8365-8460, private-`in` operator at 15964-15999, class-field parsing and
  initialization at 24314-24330 and 25049-25629, and private-reference
  resolution at 33281-33466.

  The hash-authenticated R3h profile freezes 630 paths / 1,260 variants. Oxide
  passes 1,260/1,260 and pinned QuickJS passes 630/630, with zero failure,
  unsupported, skip, timeout, crash, or infrastructure result. The focused
  profile and manifest-path-stream SHA-256 values are
  `c03c22a7ea0d767536c77f1720b5c87766b06759d8a42a6e7b9ec3069633ffa2`
  and
  `8ae21223239ac757bad085913f11f0d86f0b371d66131843932824eb69744f78`.
  Admission remains manifest-scoped. Private methods, private accessors, and
  their QuickJS brand path (`quickjs.c` 8462-8550) were the next explicit
  frontier at this field-only checkpoint.

  R3i ports ordinary synchronous private instance and static methods plus the
  QuickJS brand path. Each class evaluation creates independent instance-side
  and static-side brands, publishes one non-constructible method callable per
  declaration, and installs hidden own brand markers without exposing them to
  reflection or ordinary extensibility checks. Methods are shared by branded
  receivers, retain `#name`, `length`, no `prototype`, and the correct
  HomeObject for `super`; extracted calls work, while wrong-side or foreign
  receivers, primitive operands, duplicate initialization, and assignment to a
  private method retain QuickJS's exact error ordering and read-only behavior.
  In particular, an initialized method whose class-side brand has not yet been
  published reports `expecting <brand> private field` before a primitive
  receiver can report `not an object`, matching QuickJS's `JS_CheckBrand`
  ordering. A forward `#name in object` also preserves QuickJS's internal-tag
  quirk: before either a field or method cell is initialized, the hidden value
  becomes the own-property atom `[unsupported type]` instead of being
  normalized to unconditional `false`.
  `#method in value`, nested functions/arrows/direct eval, nested classes,
  forward private-name references, computed-key timing, public/private field
  initializer ordering, inheritance, non-extensible replacement receivers,
  and fresh class reevaluation all run through the same typed private-name and
  brand cells.

  The differential also freezes QuickJS's abrupt computed-key reentry detail:
  when a computed key throws before the class scope closes, an escaped closure
  keeps the captured private VarRef and the next reentry reuses and resets that
  same cell. Normal scope closure still freshens the VarRef on the next class
  evaluation. This is implemented deliberately rather than normalized into a
  more intuitive fresh-cell rule.

  The dependency-audited R3i cohort at Test262 commit
  `5c8206929d81b2d3d727ca6aac56c18358c8d790` contains 267 paths / 534
  variants: 219 positive paths and 48 parse-negative paths. Oxide passes all
  534 variants and pinned QuickJS passes all 267 paths. Profile, manifest-file,
  manifest-path-stream, TSV, JSONL, and empty non-pass SHA-256 values are
  `76b0fcc5610e2ceee386469344fd727a8c359abe884befccec1ab435fed93315`,
  `af3047bf66c6477f34d4229b03493a2c4247cc3f6f2b5dc4bf26e40c3ed4c7b6`,
  `7ea0bbef5d3b5b27aa5e661574fbb0f53cc65fa785874bd1baabb1d83339b375`,
  `89dacb36c99d9266e65dd7b0614d93d593007bac3cf0398b1ed0cb1a2258b357`,
  `a7a32da2995f30bb21646817d21a2389da92e5b2b17e0c3922179d4e52dd637a`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
  The QuickJS differential source and expected-transcript SHA-256 values are
  `23053aea3d41c9ee72a61007c713a17d7082dd418c9b06433a03800173b77567`
  and
  `7e87481d5b8a4202554d7c50264bb8063547512468f8c2df22bf05d06965e452`.
  Admission remains manifest-scoped; private accessors and async/generator
  class forms stay fail-closed.

  R3j ports synchronous private instance/static getters and setters,
  paired-accessor bindings, and
  their shared class-side brands. The dependency audit partitions the same
  572-path minimum synchronous private-element inventory into the admitted
  267-path R3i method slice and a disjoint 305-path accessor slice. The latter
  expands to 610 sloppy/strict variants: 229 positive paths and 76 audited
  parse-negative `SyntaxError` paths. Oxide passes all 610 variants and pinned
  QuickJS 2026-06-04 passes all 305 paths, with zero failure, unsupported,
  skip, timeout, crash, or runner fault.

  The R3j path-stream, manifest-file, profile, positive-stream, and
  negative-stream SHA-256 values are
  `ca77913172666cbe4e74a6476f7f4d87383e801260b2c5b80932dc15e8e98cd6`,
  `f8d7b7cb065cf15bae4066ec0790d1c7f0da513b83c8166aef20b3ad7e024cf4`,
  `1040d156877d88f6aae651f90b8fae472a8a4054d21f49bbbf2162d280afd884`,
  `8ef30d5843d48aaee66a55834c79d710ed8f8d0afa89ea368dee89fef75d897c`,
  and
  `9d0e56fa4e6fd1ac21a075733fdd327d41f3107500506fbff5987960be1a5901`.
  The TSV, JSONL, variant-key-stream, and empty non-pass SHA-256 values are
  `aa54c8da45ac9a32aaeb9202ee5aae375a1b42dca0ac59928d78fd11042a02f0`,
  `655a02032e50f63b281dce8cc5364d3c6aeff210a1bd3f69adae27c4c053c491`,
  `6c72f931034ee9e2e4b13910c5d88f4d06b527ff49cf6fa6211c751ad28b40a1`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
  The source filter excludes 18 accessor paths / 28 variants before this
  partition: 14 eval paths and four Function-constructor paths. The dedicated
  QuickJS transcript freezes getter/setter partial initialization, the pinned
  setter-only `#name in value` internal-tag behavior, brand/error ordering,
  initializer and `super()` timing, HomeObject `super`, nested/direct-eval
  capture, fresh evaluation, duplicate brands, abrupt class-scope reentry, and
  duplicate-name parser rules. Its JavaScript and transcript hashes are
  `0ee124bbd77f45ae9cd81bc6203cedd03e03b5e78640460abc9670ca77ffca12`
  and
  `c2656658102e7bfd9ee8da51848e18519afccb9a9ec02cc094d27cb6646d834a`.
  Admission remains manifest-scoped; async/generator class forms and the
  other explicitly excluded source frontiers stay fail-closed.

  R3k ports synchronous generator declarations/expressions, public object and
  class generator methods, `yield`, and the complete synchronous `yield*`
  delegation protocol. Generator activations are heap-visible resumable VM
  snapshots rather than host-side iterators: arguments, locals, operand stack,
  private environments, callable/VarRef edges, atoms, current realm, and saved
  bytecode PC survive suspension and participate in GC. `next`, `return`, and
  `throw` share the QuickJS state machine across suspended-start,
  suspended-yield, executing, and completed states. The implementation also
  publishes the reciprocal GeneratorFunction/Generator prototypes with pinned
  descriptor flags and keeps generator callables non-constructible.
  The direct QuickJS 2026-06-04 anchors are `quickjs.c` 20478-20491
  (VM suspension), 20929-21075 (state/GC/call), 27888-28024 (`yield`/`yield*`
  lowering), 36757-36768 (initial yield), 53094-53103 and 56463-56488
  (prototype/intrinsic graph), plus `quickjs-opcode.h` 210-215 (stack ABI).

  The authenticated bootstrap gate freezes 82 public synchronous class-
  generator paths / 160 variants: 44 positive paths and 38 audited parse-
  negative `SyntaxError` paths. Oxide passes 160/160 and pinned QuickJS
  2026-06-04 passes 82/82, with no failure, unsupported, skip, timeout, crash,
  or runner fault. Manifest, profile, variant-key, TSV, JSONL, and empty
  non-pass SHA-256 values are
  `30857ac44aa29bf86925b72b14da28c9215fb3bc29f81fc6b950694fa0d70b0f`,
  `eab79cc5f8ba041e93b7ea04bc391bed8fa249eaf5cbb11857d533fe27028c52`,
  `184f80aeb39690da69a802db371fe30cd1678726797181b4a660bf25a9996256`,
  `018401955c96b0909e2a56e76be443556e790f4a06dd067bd2d70414afa8e94f`,
  `6d005f8570ef7bb45b36b50a65cb6672e1e6863a67bf825eee0ccc25a2438f99`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
  R3t later refined ten passing negative-row `yield` diagnostics without
  changing an outcome; the hashes above are the refreshed current vectors.
  The global capability profile remained fail-closed for `generators` at this
  checkpoint; R3u later admits the authenticated synchronous cohort.

  A supplemental fresh-tree dependency audit widens the selection to 1,203
  paths / 2,378 variants. It passes 2,376, has zero engine faults or skips, and
  leaves only the sloppy/strict variants of
  `test/language/statements/class/static-init-arguments-methods.js` at the
  explicit async-class-method parser frontier; every generator-dependent row
  in that selection passes. Path, variant-key, profile, TSV, and JSONL SHA-256
  values are
  `8aaa256a04dd6b8b4d0ebfb6c49f70fa21efe0abdff9f8dfc591858539891c80`,
  `cdf4ec0a992ec3d034111871945f14f0c488c2d114610d48174565a0d890a360`,
  `d3cc7178cf10be7166ec3dcb8d690ce487fa85dd697c74ad0b7cecfa5663f0fa`,
  `42d06dde909a48d6f961697c68d32a4809a01778075be79a4a15bde599412d93`,
  and
  `50108d91e551c71c9659487aaec997324099e13f8c6422e8302b549c588a5378`.
  This supplemental audit measures breadth; the smaller checked-in gate remains
  the reproducible acceptance vector.

  R3l adds synchronous private instance/static class generator methods by
  composing the R3i private-method authority with the R3k generator execution
  kind. The compiler retains `BindingKind::PrivateMethod`, forces HomeObject,
  and emits the existing `InitializePrivateMethod`; no new opcode, private cell,
  callable class, or GC edge is introduced. The unlinked publisher, linked heap
  verifier, and runtime typed-cell reader independently admit only
  `(Normal, no prototype)` or `(Generator, own prototype)` for a private method,
  while private accessors remain ordinary-only. Suspended private generators
  preserve their callable/HomeObject/brand/private-field cycle and defining
  realm across GC without retaining the realm that invoked them.

  The direct pinned QuickJS anchors are `quickjs.c` 638-650 and 678-693
  (private-method and generator metadata), 24485-24615 and 25309-25519 (class
  element parsing/publication), 36517-36547 and 36759-36762 (method HomeObject
  and initial yield), 8462-8550 and 33368-33464 (brands/private access),
  17388-17433 and 21042-21070 (generator callable/prototype/call startup), plus
  `quickjs-opcode.h` 70, 114-115, 147-150, and 212-215.

  The authenticated R3l gate starts from 90 candidates, excludes eight
  object-spread-dependent paths, and freezes 82 paths / 160 variants: 16
  positive and 66 parse-negative paths. Oxide passes 160/160 and pinned
  QuickJS 2026-06-04 passes 82/82. Manifest, profile, variant-key, TSV, JSONL,
  and empty non-pass SHA-256 values are
  `b7b2c71cab374f9bcc6754bd9a80506d273d2e135e3f66eb373f325c94d33685`,
  `e3732db0b47608265f4f950c1c72929e782eb507597c5f0b336896e51874133e`,
  `74f827bf644507c0f0101d6597a8c5560de82b8d2303ef236beef1f3ac9de22d`,
  `24f51f0526a7c950b229ae789be58ccc42eb167f0d0f80c8c788fca832619654`,
  `2f54d423f00a410b57c6dbd4c1e3fe1c82fd8bf965f07dcf6d6bb07f69192486`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
  R3t later refined eight passing negative-row `yield` diagnostics without
  changing an outcome; the hashes above are the refreshed current vectors.
  Its pinned differential covers parameter timing, brand/error ordering,
  extracted calls, reflection/source, dynamic `super`, private capture,
  `yield*`, reevaluation, static subclass separation, and outer-generator class
  evaluation. Source and transcript SHA-256 values are
  `5af87d8181536da15ba5458ab97698e40d5df953955751bb74656a95a5dd382f`
  and
  `ff79f3ed6798a77b04e1baec6a6e022a46538f0e463707298cee894487c1a2dc`.
  A supplemental 714-path / 1,420-variant primary inventory passes all 1,388
  runnable synchronous variants. Its remaining 32 variants are 16
  async-adjacency paths rejected at selection as `unsupported-async`; there is
  no engine fault, crash, timeout, or skip. Pinned QuickJS passes all 714
  paths. Inventory, variant-key, normalized-report, and non-pass-stream
  SHA-256 values are
  `84434292de9506822d95c5afef5590d78db2cbb4d0bddeeb3acb9e9e7d1399b1`,
  `5fbee112b9ea46b5ba4002b0398e5b7045e97c9d2120a23e524f971a907b0c6c`,
  `f48961f1d6223eccabaa2a17726898f8abd76081bf91769a8f9503e4851d3355`,
  and
  `867ef271b2a97d5de723276b22ce7ec50f36c01f2cddc05aeab19eb515ec6658`.
  This supplemental audit is breadth evidence rather than a second acceptance
  gate.
  The global profile remains fail-closed for `generators`; this focused result
  does not change the complete-vector percentage.

  R3m adds QuickJS-shaped Promise heap state, paired first-call resolving
  functions, the constructor, `then`, generic `catch`, static `resolve` and
  `reject`, species construction, thenable assimilation, and a runtime-wide
  FIFO job queue. Evaluation remains non-draining; embedders execute one job at
  a time and can recover its originating `ContextId`, while the qjs and scoped
  Test262 hosts explicitly drain to empty. Job records and pending reactions
  retain only QuickJS's handler/resolving-function/value/context edges across
  GC. The host rejection tracker reports initial unhandled rejection and later
  handled transitions.

  The authenticated R3m gate freezes the 58 files directly under
  `built-ins/Promise`, excluding only the `$262.createRealm` path. Oxide passes
  all 112 sloppy/strict variants across the remaining 57 paths; pinned QuickJS
  2026-06-04 passes 57/57. The manifest, scoped profile, variant-key, TSV,
  JSONL, and empty non-pass SHA-256 values are
  `6cd3564883d5c0e459872b835e19ee7bb8c7f13716824fa2617ca1e698d5ed25`,
  `f3a07d4c1c839b4d252ed65f8fb9cadc1862cd31280002caa4656d581007eb71`,
  `0290f32ed1fe1968adf0e039748011f30588f4c1ac4b99719c5ce95d1ed9623c`,
  `ae6c2454e0aba85f1ce89e1216007c863bcefbf3ce092b2f231549e544b689cf`,
  `0d0c92b15448bf8ef94f040ff36c970e1c1d795bfdc99a720e1dff45d1071c18`,
  and
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
  A pinned transcript additionally checks synchronous executor order, nested
  FIFO tails, late reactions, propagation, thenable timing, self-resolution,
  species, identities, and forced GC. The global profile remains async
  fail-closed; `Promise.all`, `allSettled`, `any`, `race`, `try`,
  `withResolvers`, and `Promise.prototype.finally` remain explicit next
  frontiers at the R3m checkpoint rather than false parity claims.

  At its landing checkpoint, R3n adds `Promise.try`,
  `Promise.withResolvers`, and `Promise.race` without claiming the then-absent
  `all`/`allSettled`/`any`/`finally` surfaces.
  `try` creates the custom capability before synchronously calling its callback
  with `undefined` receiver and all trailing arguments, then routes return or
  throw through the captured resolve/reject callable. `withResolvers` returns
  a realm-local ordinary object whose enumerable `promise`, `resolve`, and
  `reject` fields preserve that order. `race` creates the capability first,
  reads and validates the constructor's `resolve` once, then consumes the
  cached iterator-next method synchronously and dynamically invokes each
  resolved value's `then`. Empty input remains pending. Pinned QuickJS's actual
  close boundary is retained: iterator-next/value failures reject without
  `IteratorClose`, while abrupt constructor-resolve or dynamic-then paths close
  the acquired iterator and preserve the original exception. Result,
  iterator, thenable, and queued-job edges remain rooted across forced GC.
  Direct upstream anchors are `quickjs.c` 53592-53655, 53895-53962, and
  16512-16675.

  At R3n landing, the complete authenticated inventory contains 112 paths / 224
  sloppy/strict variants: 94 race paths, 12 try paths, six withResolvers
  paths, 66 async paths, and 46 synchronous paths. Oxide records 214 passes and
  ten `fail-runtime` results, with zero unsupported or skipped variants;
  pinned QuickJS 2026-06-04 passes 112/112 paths. The ten failures are exactly
  both variants of these five adjacent consumers:

  - `test/built-ins/Promise/race/resolved-sequence-extra-ticks.js`
  - `test/built-ins/Promise/race/resolved-sequence-mixed.js`
  - `test/built-ins/Promise/race/resolved-sequence-with-rejections.js`
  - `test/built-ins/Promise/race/resolved-sequence.js`
  - `test/built-ins/Promise/race/resolved-then-catch-finally.js`

  They require the next `Promise.all`/`Promise.prototype.finally` slice rather
  than exposing a race/try/withResolvers defect. Manifest, scoped-profile,
  variant-key, adjacency-inventory, non-pass, TSV, and JSONL SHA-256 values are
  `be545aefd5f2029faae9745d859a43de176ec9865599a916f15ec465bf84d340`,
  `8548d12a4d7f3141583b986c8e3ffcae4e1afb93476ae8a444f64b940bb44654`,
  `bfe113d1c47283c84f5fc5f97e30cc74e3fea8d5975a3b87129e5b51eb05d7db`,
  `9383382995694ab1f7356f23541c00e5f99910dfd6d80ab6f38662117043e7ae`,
  `2fb9eb8c655158ba09dffcad4c9e50f96584cb218ad5e2e5d43a4216b90d3790`,
  `faf0b4f680edab60b560e54a62ad0b9ba242c7b85abe92c9714b4152c87324cf`,
  and
  `fc10101195f430cd4c382c84a4a1a7bd84bb05daff24cd3e7d62351e7dda0968`.
  The pinned static-method differential's source/transcript SHA-256 values are
  `2bc2a52869d42f314614905f4ac750b87064d6e44cbcfdcb20b3703522bdd0b2`
  and
  `0da636dbcf08f6d6ec112b439a54ec3d6b0816fff34f1381516a5cad3789f16d`.
  Its scoped profile alone opts into async execution and the two new feature
  tags; the unchanged global profile
  (`1860224ce1e828406f4869b66b3f1964f96fad85e4eab6ba7fecb256b4b6c2f2`)
  remains fail-closed with no `[execution]` section.

  At the R3o checkpoint, the same R3n inventory recorded 216/224 passes and
  eight `fail-runtime` results, with zero unsupported or skipped variants.
  `resolved-then-catch-finally.js` passed in both modes; the R3o-checkpoint
  non-passes were exactly the sloppy and strict variants of the four
  `Promise.all` consumers:

  - `test/built-ins/Promise/race/resolved-sequence-extra-ticks.js`
  - `test/built-ins/Promise/race/resolved-sequence-mixed.js`
  - `test/built-ins/Promise/race/resolved-sequence-with-rejections.js`
  - `test/built-ins/Promise/race/resolved-sequence.js`

  The R3o-checkpoint R3n non-pass, TSV, and JSONL SHA-256 values are
  `0865a76b4a9760298b3725c3b1e46559dabeb69e097b07cd9098882f595e64ba`,
  `b37787f5024f9132fb4148e6b87a247c05e9439302dd19069c18e44dd1858469`,
  and
  `21dd45dcc42d79af81e1ff9c979690cbacca86fe1e24e2728edffc104bc300a0`.
  This R3o-checkpoint result does not rewrite the authenticated 214/224 R3n
  landing checkpoint above.

  R3o completes the `Promise.prototype.finally` algorithm against pinned
  QuickJS `quickjs.c` 54057-54135. The receiver is first required to be an
  object, then `SpeciesConstructor` runs before `onFinally` callability is
  tested. QuickJS's `undefined` default-constructor sentinel remains
  `undefined`, rather than being eagerly replaced by the intrinsic Promise;
  this preserves the later observable TypeError from
  `PromiseResolve(undefined, result)`. A non-callable `onFinally` is forwarded
  twice to the receiver's dynamic `then`.

  Callable cleanup uses typed
  `PromiseFinallyHandler(Fulfill|Reject)` functions with
  `InternalCallableData::PromiseFinallyHandler { constructor, on_finally }`
  captures. Each calls `onFinally` with an `undefined` receiver and no
  arguments, propagates a cleanup throw, runs `PromiseResolve` with the
  captured constructor and cleanup result, then dynamically calls `then` with
  a typed `PromiseFinallyThunk(Fulfill|Reject)`. The thunk's
  `InternalCallableData::PromiseFinallyThunk { value }` returns the original
  fulfillment or throws the original rejection. This locks the QuickJS order
  of species lookup, callback, resolve, and dynamic `then`: cleanup failure
  overrides the original settlement, while successful cleanup preserves it.
  Heap validation pairs each native ID with its typed payload and verifies
  constructor/callable/storable captures.

  QuickJS invokes Promise resolving class callbacks and its
  `JS_NewCFunctionData` capability/finally callbacks with the calling Context,
  unlike ordinary C built-ins that switch to their defining realm. Oxide
  records that distinction as a typed native-dispatch policy for the resolving
  pair, capability executor, finally handlers, and finally thunks. A two-Context
  regression verifies that an internal finally handler materializes its
  observable TypeError from the caller's `TypeError.prototype`. Direct pinned
  anchors are `quickjs.c` 6025-6044, 17588-17612, 17742-17750,
  53352-53357, 53508-53515, and 54070-54121.

  The handler traces its constructor and callback object edges; the thunk
  traces raw settlement object/value edges. Symbol settlements additionally
  retain their atom at internal-function allocation, expose it through heap
  atom enumeration, and release the shape on allocation failure. The pinned
  forced-GC transcript records
  `symbol-thunk-thrower-gc=value:true|thrower:true` and `finally-gc=42`, so both
  typed thunks and the complete finally graph are covered.

  The complete R3o inventory contains 29 paths / 58 sloppy and strict variants:
  12 async paths / 24 variants, 17 synchronous paths / 34 variants, and one
  Proxy path / two variants. Oxide passes 56; the only two `fail-runtime`
  results are `test/built-ins/Promise/prototype/finally/this-value-proxy.js`
  in both modes because `Proxy` is not yet defined. There are zero unsupported
  results and zero skips; pinned QuickJS passes all 29/29 paths. The scoped
  profile admits `Promise`, `Promise.prototype.finally`, `Reflect.construct`,
  `Symbol`, `arrow-function`, and `class`, plus `[execution] async=true`.

  The R3o manifest, scoped-profile, variant-key, async, synchronous, Proxy,
  feature, include, non-pass, TSV, and JSONL SHA-256 values are, respectively,
  `9c24a81143fc4d3dcaa8251a2ed98e381f07cb7969f30427a60e9ca931941464`,
  `fa10d45a7ddd3924e9124cfc42239e296847223c6c9686beb2a8435e9c83bf04`,
  `d468c957b3132cb0dcfb0f9ab2d76237cbefc2b5b86a8ba387c072345be70a9f`,
  `72cf44a63ba76996ec5950307c6d79cbac4eeb917389399cdece903bc96f028b`,
  `e4a96c0de4f8bda904c8c84868d3f4c51227526290f88cf8ff26961f9a8df6c3`,
  `115c53865f31eb747b22e877e8e41154b0e1276618467c595250cf42d730ac8d`,
  `38ad367b90ca8661fef8c0ba91e8dd308ddb8aa9afca2301ed6e7e22e9212fed`,
  `0df478d04b840824e8f175d0e7fbb2e4a29afecce716f6ca7728163d406b0ea2`,
  `f8155380318e12c8fcf6fef09db3b7628f8934c761279a066a772f6c675a9400`,
  `80beabb219bb0a04830f7c2b40e47549234e20b458bd04e27998df7b64cb335d`,
  and
  `0375fb338a4fe87345f0406c5ce2ff05cb27c2779d2a7260989521cf44444cf8`.
  The pinned Test262 patch/config/metadata hashes are
  `f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3`,
  `79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b`,
  and
  `a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a`.
  The differential fixture/transcript hashes are
  `720b53338045bd65c70337c3d43678b52e8c7d3e0ce0b0ef1210f512b7d7a53a`
  and
  `9b30fc689ebac8bb116d18a87460fb9bd987f5c7b40dfabe508f787c249c10fe`.
  The unchanged global profile hash remains
  `1860224ce1e828406f4869b66b3f1964f96fad85e4eab6ba7fecb256b4b6c2f2`.

  R3n and R3o leave the facade bounded at 9,803 lines in `runtime.rs`; the new
  capability/convenience/race algorithms live in the dedicated 327-line
  `runtime/intrinsics/promise/convenience.rs` module and the finally algorithm
  lives in the dedicated 203-line `runtime/intrinsics/promise/finally.rs`
  module rather than extending the monolith. At the R3o checkpoint, the
  remaining explicit Promise frontiers are `Promise.all`,
  `Promise.allSettled`, and `Promise.any`.

  R3p implements `Promise.all` against pinned QuickJS `quickjs.c`
  53656-53893 and the constructor table at 54137. It creates the custom
  capability before reading `C.resolve`, caches that callable once, acquires
  the iterator and its `next` method, then creates a realm-local empty values
  Array and a shared remaining-elements counter initialized with the sentinel
  value one. Every yielded item calls the cached resolve with `C` as receiver,
  allocates a fresh typed element callback, increments the counter before the
  dynamic `then`, and passes the same captured reject function.

  Iterator step/done/value failures reject without closing. Abrupt
  constructor-resolve or dynamic-then paths perform preserving-throw
  `IteratorClose` before rejection. A synchronous custom `then` can call its
  handler immediately, but the sentinel prevents final resolution before the
  iterator reports done. Each element callback has an independent first-call
  bit and input index; it uses CreateDataProperty semantics to write the
  values Array without invoking inherited setters, decrements the shared
  counter, and calls the final resolve only when the last reference is gone.
  Synchronous rejection does not stop the main iteration when `then` itself
  returns normally.

  The heap representation is
  `InternalCallableData::PromiseAllResolveElement { values, resolve,
  remaining, already_called, index }`. GC traces the values Array and outer
  resolve from every escaped callback; the shared/per-element counters contain
  only scalars, and ordinary Array slots own stored object and Symbol edges.
  The values Array and callbacks use the `Promise.all` builtin's defining
  realm, while the CFunctionData-shaped element callback body executes in its
  calling Context. Heap validation pairs the typed native target with an
  Array, callable resolve, non-constructor callback, and valid index.

  The complete authenticated R3p inventory contains 98 paths / 196 sloppy and
  strict variants: 57 async paths / 114 variants and 41 synchronous paths /
  82 variants. It contains no negative, Proxy, or `$262` host test. Oxide
  passes 196/196 with zero failures, unsupported results, or skips; pinned
  QuickJS 2026-06-04 passes 98/98. Manifest, profile, variant-key, empty
  non-pass, TSV, and JSONL SHA-256 values are
  `293639a6d0e3f1937535997a4f61613fd40b2b10267d1d27cc5faa231865c1e5`,
  `83b69f80efbe0aa1c1273c646595424d4e3cda01f65ccc1e7400495a6779bb21`,
  `be2fbe56f4e095c9ebc5ad7a2dc611ec3ca0fcf3878cac552b9b08c3bb0442c7`,
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
  `a71f0e04b81bed11d3760296a40753ed18f0572d25145857b5bcee434f6fa2c9`,
  and
  `3c895f2876be7ceabb12e6e85af5f1bc9d9b1eab2f5cb3a884f5f340d871c22a`.
  The scoped profile admits its six observed feature tags and async execution;
  the global profile remains byte-identical and fail-closed.

  The pinned differential covers descriptors, generic/custom capabilities,
  fresh handlers and shared reject identity, empty/out-of-order completion,
  synchronous sentinel ordering, duplicate callbacks, resolve lookup,
  IteratorClose boundaries, thenable/identity behavior, forced GC, and
  cross-Context realm routing. Its fixture and transcript SHA-256 values are
  `e43406b9de7de5a88034ec5321486d7b352f2c6f43986fddba1b36fe79074835`
  and
  `efb2fd9cfdd1db42291295e0b313dbf271b0007d30f3823e0377cb7196ab6b54`.
  The unchanged R3n inventory now passes 224/224; its current empty non-pass,
  TSV, and JSONL hashes are
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
  `350e8f80d30a1942e44595c1e771b5e0008fd33aa2f93d6d2345e219d5bb6968`,
  and
  `4058a876e0f05e0ff0b07d6ae6a5b4886ea9dca3ebbe178c758221aa371df6ca`.

  R3p leaves `runtime.rs` at 9,803 lines. The complete algorithm lives in the
  dedicated 240-line `runtime/intrinsics/promise/all.rs` module. Ordinary
  JavaScript-observable behavior is locked; exact allocation-failure routing
  remains narrower than QuickJS because values-Array or element-handler OOM
  currently returns a runtime error instead of capability rejection (and, for
  the latter, close-then-reject). The checked `u32` overflow path is likewise
  a theoretical multi-billion-element boundary. The remaining explicit
  aggregate Promise frontiers are `Promise.allSettled` and `Promise.any`.

  R3q closes those two aggregate frontiers against the same pinned QuickJS
  `js_promise_all` loop. `Promise.all`, `Promise.allSettled`, and `Promise.any`
  now enter one shared typed Rust algorithm while retaining three distinct
  heap callback captures. `allSettled` creates fulfillment records with
  `{ status, value }` and rejection records with `{ status, reason }` in the
  callback's calling Context. Deliberately matching QuickJS's copied
  CFunctionData cells, its fulfillment and rejection callbacks do not share
  an already-called bit: a hostile synchronous `then` may invoke both and the
  later callback can overwrite the same output slot. The signed shared counter
  preserves QuickJS's corresponding below-zero state after this edge case.
  Internal indexed writes also use QuickJS's no-throw define mode: if an early
  custom capability exposes and freezes the output Array, a later rejected
  definition is ignored and iteration continues instead of closing and
  rejecting with a synthesized `TypeError`.

  `Promise.any` passes the outer capability resolve itself as every fulfillment
  handler, allocates one fresh typed rejection handler per element, and
  pre-fills the errors Array before invoking `then`. Final rejection constructs
  the intrinsic branded `AggregateError` internally, retaining the exact
  errors Array without consulting the public constructor or iterating/copying
  it. The errors Array belongs to the aggregate call Context; a non-empty final
  `AggregateError` belongs to the last rejection callback's calling Context,
  while the empty-input error belongs to the builtin's defining Context. GC
  traces each output Array and final settlement callable through the typed
  callback captures.

  The complete authenticated `allSettled` inventory is 104 paths / 208
  variants (57 async paths / 114 variants and 47 synchronous paths / 94
  variants). The `any` inventory is 94 paths / 188 variants (65 async paths /
  130 variants and 29 synchronous paths / 58 variants). Neither cohort has a
  negative, Proxy, or `$262` host test. Oxide passes 208/208 and 188/188 with
  zero failures, unsupported results, or skips; pinned QuickJS passes 104/104
  and 94/94 paths. Their manifest, scoped-profile, variant-key, TSV, and JSONL
  SHA-256 values are, respectively:

  - `allSettled`:
    `5ac6c5f7e21194ee432a6480fc8e8b5ae7fff2c3c859aa61da98f7605261eb52`,
    `755439ed09621a0460802bfda11ed27983364d572b13baaf93a2e00c5b481947`,
    `9b27ccbbdc3e2d8f3eae0f76b783625cc0aefebc52a2802446e21a6f5dbb083c`,
    `69f7dffcd523a759ea7518708d02a74e56349000c86058574c0dc10bc6313b62`,
    and
    `d3173fdd5c6d7d2b6b2523c1e9c05b19b3524a6411d383f529c09877a687cc55`;
  - `any`:
    `331a3d6f0b19a9353904afa5c5d740f844f97c89fcbc99b58cd11275d3b67eaf`,
    `8059eea59f179846a4739ddb280b4d16518286261d1cdb361a2d383474f27826`,
    `4f2cd9023246ba0631d27846c942f9e227425717208ef0454da1178c105872a5`,
    `6b984703c5f155cfd5300314f0f32a98801ad058294aa8b60125f56d478f83a3`,
    and
    `856e0679a8425f1a1a403d2577d39547fbeb6053c88dcca4bd9778bf67e6b0f8`.

  The independent combined differential locks descriptors, custom
  capabilities and handler identities, QuickJS's two-callback overwrite,
  result/error property order and descriptors, empty/sentinel ordering,
  resolve and iterator-close boundaries, forced-GC capture lifetime, and
  cross-Context realm routing. Its fixture and pinned transcript hashes are
  `e053bb7944943607b9a29ef15fd34d44a58c44792afaf5193e6b757f4231d8c4`
  and
  `992d7e26fa681747b67c49a6cfd296c22ae54a558f1d8a86d70ce9eeea3a71e9`.
  `runtime.rs` remains 9,803 lines; the shared aggregate owner is the dedicated
  496-line `runtime/intrinsics/promise/all.rs` module. Exact allocation-failure
  routing and the theoretical multi-billion-element index boundary remain
  explicit hardening debt in `docs/deviations.md`, not approved differences.

  R3r removes the complete vector's two engine faults by porting QuickJS's
  transient iterator control around array BindingPatterns and
  AssignmentPatterns. A generator `.return(value)` injected at a suspended
  default initializer now closes nested pattern iterators inside-out before
  returning or entering an enclosing `finally`. The compiler owns the control
  shape across `src/compiler.rs` and `src/compiler/destructuring.rs`; no logic
  moves into `runtime.rs`.

  For a yield in a for-of head assignment fragment, pinned QuickJS abandons
  the active outer loop record without calling its `return` method. The typed
  `IteratorDropPreserve` bytecode operation reproduces that parser-ordering
  behavior while explicitly discharging the verifier region rather than
  weakening the `Return` invariant. An inner close throw instead follows the
  normal pending-exception path and closes the outer iterator without replacing
  the original throw. The frozen QuickJS differential covers ordinary,
  nested, `finally`, `yield*`, for-of-head, and close-throw ordering; its
  fixture/transcript hashes are
  `05d8e677e984df2a9accb0c56ddb6f2e06ba6d3b2d2d08a51d4ba48811463398`
  and
  `4e39206df0f8213845227839ad1986759f12566e570a4820265a40e239add715`.

  The complete 102,037-key join has zero previous-pass regressions. Relative
  to the immediately preceding R3q rerun, exactly two `engine-fault -> pass`
  transitions remain; relative to the last checked baseline, 630 variants
  become passes. The vector now records 36,923 passes and no engine fault,
  with TSV/JSONL hashes
  `87b1adf3234e6625dd95c96c11357e347447438d412b4007ec2236cb0fd18c7c`
  and
  `90726c1feee169bf923c857101d73c4f95ffc002de378dfe1f637451ce4fa906`.

  Async functions/generators, destructuring eval declarations, unsupported
  class elements, and ill-formed UTF-16 source stay explicit frontiers.
  QuickJS also allocates the callable and VarRef
  array before capturing caller cells, while this Rust slice materializes the
  roots first and then allocates the callable; only successful-compilation
  OOM/GC priority is affected, but exact allocation-order parity remains a
  later generic closure-instantiation task.

  R3s publishes pinned QuickJS's strict static `RegExp.escape` and completes
  Annex B legacy control escapes. The two-pass UTF-16 implementation linearizes
  ropes, preserves narrow/wide storage, checks the expanded length before an
  exact fallible reservation, and reproduces QuickJS's character classifier,
  lone-surrogate handling, lowercase hexadecimal output, property order, and
  non-constructability. The RegExp parser now accepts ASCII control letters,
  permits Annex B digits and `_` only inside non-Unicode character classes, and
  rewinds every other non-Unicode `\c` form to the original backslash exactly as
  `libregexp.c` does.

  The new static RegExp built-ins gate starts from all 1,879 pinned
  `test/built-ins/RegExp` paths. It excludes the union of 182 metadata-`v`
  paths (one also uses `createRealm`), 12 source-audited literal-`v` paths whose
  metadata omits that feature, and 12 `createRealm` paths: 205 exclusions leave
  1,674 paths and 3,346 sloppy/strict variants. Oxide and pinned QuickJS both
  pass 3,346/3,346. The manifest/profile hashes are
  `db6201093f57412de0d0cf16d4ff06f74512af3bc76d6f83c337474c7b982ab3`
  and
  `0214f6789a3276c4755fadde19477b70620184a6137d29eefef0975cfb379c15`;
  the canonical focused TSV/JSONL hashes are
  `c2bf334ddcc255048c778095db5bc85e7bacde63ec66049feead47478e66742d`
  and
  `9a3ec4c6e5d2c894d22c9e930a74c793dcbf5a691d5e85da34aa024585fac8d0`.

  `RegExp.escape` remains admitted only by that checksum-bound scoped profile,
  so the global profile is still fail-closed. The complete 102,037-key join
  therefore records only the untagged control-escape movement: two
  `unsupported-parser -> pass` and two `unsupported-runtime -> pass`
  transitions, plus eight detail-only rows that proceed to the existing
  ill-formed UTF-16 eval frontier. There are no missing/extra keys, duplicate
  keys, or previous-pass regressions. The vector reaches 36,927 passes; its
  TSV/JSONL hashes are
  `8f6401e033c8a58d0886ee6453015ca5f289022b90f3f32471e43f7022b2307b`
  and
  `80055a2278a54aa97f5d0dc8e07bcaefa641cc15ef26ddcc53f35f4095d704e5`.
  Additional exhaustive QuickJS differential probes cover every BMP initial
  unit, every non-initial BMP unit, all ASCII `\c` followers and class-range
  endpoint pairs, surrogate boundaries, and long ropes. The checked-in
  representative fixture/transcript hashes are
  `babb9f0e94a7f4e3cf62ad25faf923dc86adb9248db36f081b4b2e7667c6f784`
  and
  `c6226637ca00cfcef2c436cb64442d8264ba18553aba31baffe70a34d48f480f`.

  R3t closes the pinned synchronous 11-feature generator/destructuring cohort.
  Mapped `arguments` now follows QuickJS by sharing frame VarRefs only
  for actual arguments backed by formal parameters and allocating detached
  VarRefs for extras. Generator `.caller`/`.arguments` use poison accessors;
  sloppy contextual `function yield(){}` is accepted only for an ordinary
  FunctionExpression; and scoped generator declarations now preserve the
  QuickJS lexical/Annex B duplicate-declaration boundary. Active-`yield` and
  related `for-in` negatives retain genuine `SyntaxError` provenance.

  The static metadata universe contains 3,418 paths/6,624 variants. Removing
  25 module paths/variants and three source-audited async-callable paths (six
  variants) whose feature metadata is non-exhaustive leaves 3,390 synchronous
  paths and 6,593 variants:
  3,011 positive paths/5,906 variants plus 379 parse-negative paths/687
  variants. Oxide and pinned QuickJS both pass 6,593/6,593. The manifest,
  scoped-profile, key, TSV, and JSONL hashes are
  `07ad2748c65763366ebdcb8c01893a13aa4fbbcca3e900a31042fc670593f3c5`,
  `8057ef347c07ffc80a66c5c83ff73873148a8813af49bcca1ced9863cfb9ac9e`,
  `f5e729f4b439733ee900ce1d7d98163b9969aab6998b4a288cb4a6eea5c35f81`,
  `f81c2f7b946360f44c1b2d5bdc40782d2e13f989af372329fb6582cb8ded8978`,
  and
  `eb1d82ad4d156880bc539d2bfc73e8203cd9dd8f70289e80560388ea07c11083`.

  R3t intentionally keeps this as a checksum-bound scoped admission. The
  global profile remains byte-identical and fail-closed for `generators` and
  `destructuring-binding`, so none of the 6,593 scoped passes is counted as a
  global admission. One untagged Annex B generator-declaration case moves from
  `fail-runtime` to `pass`; one untagged staging case moves from `fail-parse`
  to the deeper QuickJS-matching `fail-runtime`. The exact full join therefore
  has one new pass, no previous-pass regression, and no engine fault:
  36,928/102,037 with TSV/JSONL hashes
  `6b2fb9219bad5f25bfcebc297ce9373798cd210140ebab0566a18e8dd83d052b`
  and
  `d2cf352f98f7d12b1ff734d7ff001c443c896be3c8adddd54951dd0a47f78eb2`.
  A separate admission milestone will classify the three async adjacencies and
  migrate the globally profile-bound baselines.

  R3u performs that global admission without changing engine semantics. The
  global profile is the bytewise-sorted union of the previous global profile
  and R3t's reviewed surface: 73 feature tags and 802 exact negative paths,
  SHA-256
  `d01f4f49fbd14b2cad610983624142b468587b2e0bd10ae6264641c39cffa05f`.
  A cohort-scoped source guard catches the three tests whose source contains
  async function/arrow grammar not represented by their non-exhaustive feature
  metadata; all six variants remain `unsupported-async`. Metadata modes, host
  hooks, ordinary feature gaps, and negative provenance retain priority, and
  the `$262` scanner keeps its conservative non-RegExp-skipping path.

  The exact 102,037-key R3t/R3u join records 6,593
  `unsupported-feature -> pass` and six
  `unsupported-feature -> unsupported-async` transitions, with no other
  outcome movement, key drift, duplicate, engine fault, or previous-pass
  regression. The vector reaches 43,521 passes and 45,076 runnable variants.
  Its TSV/JSONL SHA-256 values are
  `202ab3480b39a6c7a68443bf9faba7bf9eb139b7c15baf2fde25c55c40c5d023`
  and
  `25df14d037d181bc82b70855a44e782cfbff3118603666dca6ec908cfd659387`.
  The parser-provenance canary now records eight intended passes and eleven
  fail-closed variants. Re-running the public/private class-generator gates
  also refreshes two stale R3k/R3l report hashes for R3t's ten/eight
  detail-only `yield` diagnostic refinements; both gates remain 160/160.

  R3v adds the realm-local synchronous `Iterator` intrinsic and the core
  Iterator Helpers surface: `Iterator.from`; lazy `drop`, `filter`, `flatMap`,
  `map`, and `take`; and eager `every`, `find`, `forEach`, `reduce`, `some`,
  and `toArray`. QuickJS-shaped heap payloads own helper and wrapper state,
  traced sources/callbacks/inner iterators, completion and reentry flags, and
  close behavior. Pinned-QuickJS differentials cover huge finite limit
  conversion, primitive String fallback, dynamic wrapper `return`, and nested
  `flatMap` close-error priority; a two-context Rust probe separately locks
  cross-realm constructor identity.

  At R3v, the dependency-audited Test262 gate started from all 567
  `iterator-helpers` paths and removed the exact 44-path union of 25 direct
  Proxy dependencies, three harness-level Proxy dependencies, 11
  `$262.createRealm` paths, four `$262.IsHTMLDDA` paths, and one pinned-config
  exclusion. Oxide and pinned QuickJS both passed all 1,046 sloppy/strict
  variants from the remaining 523 paths. The historical scoped-profile, key,
  TSV, and JSONL hashes were
  `a6ce2d6be97d7826cf20aeba7ab8946ad28ce134b0ad7165a8e591a986e6d22e`,
  `43be68340124e844c5e456899a084460ad87edd2c279c3ac1ca4057726b3697a`,
  `4746567453ed198096fd270e70f7c2c51975de837df0a1181645ceffd3cdefc9`,
  and
  `a25b115582160d38acb534c0192f93db65f3c8473d3c9211adb39c8f40a1a02a`.
  The safe `--bless` path required every frozen variant to pass.

  At that checkpoint the milestone remained scoped because Proxy and the
  required host hooks were separate frontiers, so the conservative global
  vector stayed byte-identical at 43,521/102,037 passes. R3bl later promoted
  the 14-path optional adjacency, and R3bm completed the 28-path Proxy closure
  while passing 551 paths / 1,102 variants. R3bn later admitted exactly
  `iterator-helpers` globally; the 13 `globalThis` paths and 16 host/config
  paths remain fail-closed.
  `Iterator.concat` remains in the separate R3w `iterator-sequencing`
  milestone immediately below.

  R3w adds QuickJS's independent `Iterator.concat` class and sequencing state
  machine rather than folding it into Iterator Helpers. Construction eagerly
  validates and captures each iterable's exact `@@iterator` method; `next`
  lazily opens one iterator at a time, caches its `next`, skips empty inputs,
  preserves retryable abrupt state without closing, and guards every
  observable step against reentry. `return` preserves all state when its
  getter throws, but drains every captured edge after a successful getter
  whether the subsequent call returns or throws, forwarding that result
  without IteratorClose-style normalization. The heap releases consumed and
  drained edges at the pinned QuickJS boundaries.

  The complete pinned `iterator-sequencing` inventory is 32 paths / 64
  sloppy-and-strict variants with no Proxy, host-hook, config, mode, or
  negative-test exclusions. Oxide and QuickJS 2026-06-04 both pass 64/64. The
  manifest-file, scoped-profile, key-stream, TSV, and JSONL SHA-256 values are
  `74eebb8c63a2606e54e1d0023c5244b8a0538ac51d1ca0a105fe56a04fa74af2`,
  `8284db009a398fb88b2d357d7d8255479943d963574392f7b718610ee12cb16a`,
  `eab38e1c6d7f22397e7c8521ec934476b2472406db5d83cfea23d0fbe7b17d5b`,
  `716d98068f7f2b28ff142abca546e71ff7eee9224bad1cea52ac0830240b8560`,
  and
  `a184e7e80444282cc23015c5846052430c593eab93da358d4679859422f2e029`.
  Focused QuickJS differentials cover topology, evaluation order, retry
  boundaries, return drain priority, and reentry beyond the Test262 cohort.
  A two-context Rust regression plus a pinned same-runtime libquickjs C probe
  separately lock the cross-realm behavior.

  R3w also corrects the direct native IteratorNext fast path to retain the
  outer iterator operation's current realm, matching `JS_IteratorNext2`
  bypassing ordinary C-function realm switching. The final workspace audit
  also moves the public `Iterator` global after `Function` and before
  `parseInt`, matching pinned QuickJS's observable global own-key order.
  R3x then promotes the already-authenticated `iterator-sequencing` tag into
  the global profile. The exact full-vector join changes only the same 64
  frozen keys from `unsupported-feature` to `pass`: no other outcome, detail,
  key, or failure moves. The conservative vector reaches 43,585 passes and
  45,140 runnable variants. Its TSV/JSONL SHA-256 values are
  `0f43b6e164c0954a02f911774c34871ea67e6255f28ffa65419ea15d3f4b73fd`
  and
  `f24e92ad54c4c59651206db66bfd7a4ed9dea4f3543311a990def0fc16e66be8`.
  At R3x, core `iterator-helpers` remained scoped behind its 44
  Proxy/host/config adjacencies. R3bl later promoted 14 Proxy paths, and R3bm
  completed the 28-path Proxy closure while leaving the current 16-path
  host/config deferred ledger explicit. Re-running the
  profile-bound focused gates also refreshed two stale tagged-template
  PrivateName staging rows that later private-name work had already moved to
  pass.

  R3y authenticates the existing synchronous class implementation as one
  generated-matrix closure rather than widening the global `class` feature.
  Exact metadata allowlists derive 3,890 paths / 7,763 variants from the four
  class `dstr`/`elements` roots. A frontmatter-stripped source audit assigns
  eight async private method-name paths / 16 variants and six Proxy-dependent
  paths / 12 variants to their separate frontiers; optional chaining adds no
  hidden dependency. The resulting clean manifest contains 3,876 paths /
  7,735 variants, including 680 audited parse/SyntaxError paths. Oxide and
  pinned QuickJS both pass all 7,735.

  The manifest, scoped-profile, key-stream, TSV, and JSONL SHA-256 values are
  `40f038bdc52c762baf7f16ea885c98fc3d0afd033e56059717e8627086e14c78`,
  `de71fc1d3c675ed25dc54d43222a10c4f3d607c14cb4d43628d7a4587827a7ef`,
  `1095d6e01eb78c11ed9ff23f195ac909cd99381cb646973095b7cac9ad4676bc`,
  `61e9a260c91e886bd65b2b148564ce861324b8a5b5343f85688d603bd3217b1e`,
  and
  `a258e37e13d99f3491e79db321172f3202800b526f8059ef5c8f3b1a77d9fee2`.
  This is scoped semantic evidence, not a scoreboard increase: the global
  profile and conservative 43,585/102,037 vector remain byte-identical.

  R3z ports ordinary async function declarations/expressions and `await`
  through the final bytecode/VM path. `Instruction::Await` has an authenticated
  1-to-1 stack effect; fulfill resumes the parked slot, while rejection enters
  the ordinary VM unwind machinery used by catch/finally. A hidden,
  GC-visible `AsyncFunctionState` owns dormant activations and the outer
  resolving pair. Its driver Promise and continuation jobs belong to the
  caller realm, while the resumed body, globals, and constructed errors retain
  the bytecode's defining realm. Await uses the cached intrinsic
  PromiseResolve and capability-free internal reactions, so replacing global
  `Promise`, `Promise.resolve`, or `Promise.prototype.then` cannot intercept
  it. The same activation path preserves direct-eval variable objects before
  and after suspension.

  The original R3z dependency-audited scoped cohort started from 207 paths in
  the pinned AsyncFunction/async-function/await roots. At that landing,
  sixty-five explicit exclusions kept complex parameters, eval/with
  adjacencies, async arrows, async generators/for-await, and host/cross-realm
  dependencies outside the first core. Its clean manifest contained 142 paths /
  259 variants: 95 positive and
  47 exact parse/SyntaxError paths, with 65 async-harness and 77 synchronous
  paths. Oxide passed 259/259 and pinned QuickJS passed all 142 paths. The
  manifest, scoped-profile, variant-key, TSV, and JSONL SHA-256 values were
  `fdd1679242195cb32508b7976a1b0b3508fe96a2e77483808d3bf5c9c554ff52`,
  `05634144cdc2e64874ffda721b429181ac8b7a8f82b1ba253f2b8d8a29a4332e`,
  `a5249ce3625e80f41ea2464e00fcf19804913d49556e680ad6624fd6bf71d391`,
  `d0d3933d5cc4114b60a55bd6040d4350cba890b7d8a29a4e41e372eb4291cfaa`,
  and
  `9259b27b167856e5e3a2428530d1943d74fc967a659759568b5068ce2a74c4c3`.
  Those values preserve the historical R3z landing snapshot.
  The complete 102,037-key R3y/R3z join has no missing, extra, duplicate, or
  previous-pass row. It records 54 `fail-parse -> pass`, four
  `fail-runtime -> pass`, four `fail-parse -> fail-runtime`, 19
  `fail-parse -> unsupported-parser`, two
  `fail-parse -> unsupported-runtime`, and seven
  `fail-runtime -> unsupported-runtime` transitions, plus two detail-only
  diagnostic refinements. Passes rise from 43,585 to 43,643. Final full
  TSV/JSONL SHA-256 values are
  `8d47c7d70de9d1049cded9b4fe4aec3459313e374421ab99e1c36eb5730531f6`
  and
  `14295f172893540d703e02aa4c9ba3e5bdee02d866131479680b5c33b2ddfabd`.
  Thirteen Rust integration tests lock job checkpoints, abrupt thenables,
  return assimilation, mutable-Promise resistance, dynamic construction,
  direct eval, GC, contextual `await` names, host-stack preflight, and
  caller/callee realm separation. A checksum-pinned transcript separately
  compares the identical fixture in both engines.
  At the R3z landing, async arrows, object/class async methods, async
  generators, for-await, and modules remained explicit later frontiers.

  R3aa expanded only the authenticated bookkeeping around that implementation.
  It admitted all 40 complex-parameter paths and nine of the 11 eval/with
  adjacencies from the original R3z exclusion ledger. The two remaining
  eval/with paths contained async arrows, so the R3aa 16-path ledger consisted
  of ten async-arrow paths, those two async-arrow-dependent eval paths, two
  async-generator/for-await paths, and two host/cross-realm paths. At the R3aa
  landing, the clean manifest contained 191 paths / 348 variants: 126 positive
  and 65 exact parse/SyntaxError paths, with 96 async-harness and 95
  synchronous paths; 157 paths ran in both modes, 26 were `noStrict`, and
  eight were `onlyStrict`. Oxide passed 348/348, and pinned QuickJS passed all
  191 paths.

  The R3aa exclusion-ledger, manifest, scoped-profile, variant-key, TSV, and
  JSONL SHA-256 values are
  `7c29c59cc107d74da4a5fcfba4571947195003a2f551bb82f9fc2dd8b3fb42ac`,
  `a0fa7acd444257ca7cbfffc40c61eb3b85867c81df04f1d1691100a72c97b0dc`,
  `7fb94b8e350b5a270ab5f685f0a223e32c7d12fedf0ac3e0c1e157b03f4f0b33`,
  `25e87df8047ce67fb30a570f9e211540b689dc00c9a4b7e29de20b528f77a077`,
  `ba690597d3ca1d9f6604106b0d54d37a7d1215b4a832c0a72a4ccdde8c28e913`,
  and
  `fe4be77b96c8af7b8bda137d8377818ab04450f340beaa2e172f290eadcb264f`.
  This is a scoped-evidence expansion, not a new semantic or scoreboard
  landing: the global profile, 43,643/102,037 vector, and full-vector hashes
  remain unchanged.

  R3ab ports async-arrow functions with the same QuickJS-shaped split between
  grammar and execution: compiled functions retain `FunctionKind::Arrow`,
  while `BytecodeFunctionKind::Async` selects the existing async Promise,
  suspension, and `await` machinery. This preserves arrow
  non-constructibility, lack of an own `prototype`, AsyncFunction branding,
  authored source, inferred name, and length. Lexical `this`, `arguments`,
  `new.target`, and `super` remain captured from the enclosing function across
  suspension.

  The parser also reproduces the pinned target's token-timing asymmetry. The
  token immediately after `async` is committed in the parent lexical context
  before the async-arrow child is created, while later parameter tokens use
  the child's async context. Every nested arrow creates a new formal-parameter
  boundary: future `await`/`yield` classification is recomputed from that arrow
  and its immediate parent execution/static-block role, so an ancestor
  async/generator/static context cannot leak through transitively.
  Consequently QuickJS 2026-06-04 and Oxide accept the single-binding
  `async await => 1` and escaped-`await` forms at top level, but reject
  parenthesized `async (await) => 1`, an `await` expression in a parameter
  default, and the corresponding single-binding forms when the enclosing
  async/generator context has already classified `await`/`yield`.

  The canonical R3ab language cohort is the complete 60-path pinned
  `language/expressions/async-arrow-function` tree. All complex-parameter,
  eval/with, and five forbidden-extension paths are admitted, leaving zero
  exclusions. Its 31 positive and 29 audited parse/SyntaxError paths expand to
  110 variants. Oxide passes 110/110, and pinned QuickJS 2026-06-04 passes
  60/60 paths.

  The frozen focused gate adds the exact
  `test/built-ins/Function/prototype/toString/async-arrow-function.js`
  adjacency.
  Its full 61-path / 112-variant manifest still has zero exclusions: 32
  positive and 29 negative paths, 27 async-harness and 34 synchronous paths,
  51 double-mode, eight `noStrict`, and two `onlyStrict`, yielding 59 sloppy
  and 53 strict variants. Oxide passes 112/112 with no unsupported, skipped,
  failed, timed-out, crashed, or infrastructure outcome; pinned QuickJS passes
  61/61 paths. The pre-existing 203-path / 205-variant `with` gate now also
  passes 205/205 after R3ab closes its final async-arrow adjacency.

  R3ab also returns the ten direct async-arrow paths and two
  async-arrow-dependent eval/with paths from the historical R3aa
  ordinary-async exclusion ledger. That current gate now admits 203 of its 207
  candidates and passes 366/366 variants in Oxide; pinned QuickJS passes
  203/203 paths. No complex-parameter, eval/with, or async-arrow exclusion
  remains. Its four explicit exclusions are exactly two
  async-generator/for-await paths and two host/cross-realm paths. The current
  exclusion-ledger, manifest, scoped-profile, variant-key, TSV, and JSONL
  SHA-256 values are
  `7e60ccc3b07d5539d3c55958ee8889df3de899525688d346e8d5763d9a1d4f41`,
  `97930e30959d8bdbdd1b030e4f4e94fe9657791951f48e58a6790e73a7191390`,
  `7fb94b8e350b5a270ab5f685f0a223e32c7d12fedf0ac3e0c1e157b03f4f0b33`,
  `109e78ccd538a5ce8376140b50c624a9ccdcb929b8d4819ab25acd9610e8e995`,
  `2f22a49938c079c0133f372f1e5b8f757b5aace881385a185c4b775f6186fd39`,
  and
  `c750dba4c8a45f4cc18c658810774b4919771d89052ed5a8423b92a636922eaf`.

  One SpiderMonkey staging test,
  `test/staging/sm/async-functions/async-contains-unicode-escape.js`, expects a
  SyntaxError for the single-binding token case that the pinned QuickJS target
  accepts. R3ab checksum-pins and differentially audits that target quirk, but
  keeps the staging path outside the 61-path focused candidate universe. It is
  audit-only, not a hidden exclusion.

  The R3ab manifest, empty exclusion ledger, scoped profile, variant-key, TSV,
  and JSONL SHA-256 values are
  `d4bc4b286b2da1b19949d56b614e1d1af110437285827fa4f4c6cb00dae1d969`,
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
  `f6634c6298e3d3fb740c0f55e8932ddc402ca8e120d8f0d2d9326f552186af2c`,
  `b3407b31ee0df08990b09aa13b77f7f6ff7028ab0ad4f1eb3c1f083a36a6cd03`,
  `9f110385b1695b6eaaabafb0984c091d35e6cc83878e10b1450f1730db2636f1`,
  and
  `68b5efe50c71eb24a75a53fd72200d883fa966e77813a22014a046b6f09f2f58`.
  At the R3ab landing this remained scoped evidence: the global
  `async-functions` feature and async host stayed fail-closed pending async
  methods and generators.
  Nevertheless, 12 already-admitted, untagged consumers now pass. The exact
  R3z/R3ab full-vector join retains all 102,037 keys with no missing, extra,
  duplicate, or previous-pass row. Its 16 outcome transitions are eight
  `unsupported-parser -> pass`, four `unsupported-runtime -> pass`, two
  `unsupported-parser -> unsupported-runtime`, and two
  `unsupported-runtime -> fail-runtime`. The last pair is the pinned
  `await-in-arrow-parameters` target difference and also fails under QuickJS
  2026-06-04. Two additional `toString` rows keep their unsupported-parser
  outcome but advance their detail to the async-object-method frontier. The
  runnable count remains 45,140; passes rise from 43,643 to 43,655. Final full
  TSV/JSONL SHA-256 values are
  `f9b0827706c24cc97f1792e92aa1d275d7c5c7bd14d3e2b47f16d27dc543c8f0`
  and
  `9026be710ff432002357a236b4ebd81abc8fd6ea9039e04b3af8944968d83d70`.

  R3ac ports ordinary async object-literal methods without adding a second
  execution path. Their grammar identity remains `FunctionKind::Method`,
  `BytecodeFunctionKind::Async` selects the existing Promise/await driver, and
  ordinary `DefineMethod` publication retains the method's HomeObject. This
  preserves `super` in parameters and across suspension, AsyncFunction
  branding, non-constructibility, inferred names, length, and authored source.
  No runtime change was required.

  The checksum-bound candidate universe contains 49 paths / 90 variants,
  including Function `toString`, complex-parameter, eval,
  forbidden-extension, private-name, Proxy, and async-generator adjacencies.
  Pinned QuickJS 2026-06-04 passes all 49 paths. Six async-generator neighbors
  and one Proxy path remain explicit exclusions, leaving a clean 42-path /
  76-variant manifest. Oxide passes 76/76 with no unsupported, skipped,
  failed, timed-out, crashed, or infrastructure outcome. At the R3ac landing,
  broad async feature/host admission remained fail-closed pending async class
  methods and generators.

  The scoped-profile, candidate, manifest, TSV, and JSONL SHA-256 values are
  `ec8be515bb6f68cb3226f1770b4ac73b66c013d5c27a74bcda974770546b9e9f`,
  `535880772cfeff4e3c7cf31d956a80ea70fa67b2fc1fd043825d43eab6c6536a`,
  `38b1fd3cc785923d4e98a28b8e8daf19777bf02630634753715abf7160c9d796`,
  `511ebd130275568679425664893888b56da1280d93731bbab7c4003dadd2ad64`,
  and
  `06a5557df83f1ff795d73a89ae60d83547976b372b8a87d1f3098126f4b9dc95`.

  The object-method lookahead separately freezes a pinned QuickJS quirk:
  U+2028/U+2029 directly after `async` are line terminators, but the same code
  points inside the intervening block comment are ignored by QuickJS's small
  contextual lookahead scanner. Normal lexer metadata remains
  ECMAScript-correct; Annex B HTML comments remain an existing independent
  parser frontier.

  The exact R3ab/R3ac full-vector join retains all 102,037 unique keys and
  every previous pass. Two `unsupported-parser` and two
  `unsupported-runtime` variants become passes. Two more
  `unsupported-parser` variants reach the typed async-generator runtime
  frontier. The two variants of
  `staging/sm/async-functions/async-contains-unicode-escape.js` move from a
  typed runtime frontier to `fail-runtime`; their Test262 expectation also
  fails under pinned QuickJS, so they are target disagreements rather than
  Oxide regressions. Six additional rows change only their downstream detail.
  Runnable remains 45,140 and passes rise from 43,655 to 43,659, with 18
  parse failures, 1,281 runtime failures, 45 typed parser frontiers, and 34
  typed runtime frontiers. The R3ac full TSV/JSONL SHA-256 values are
  `627e6e8dc2aa44e9ef6869db54c3a9059528d33eb7b24c55658db36d84a250b0`
  and
  `5879cef785efe0a855e3abb74d820dd9bc2274d20fdba9ba8c557641d0fa5dbe`.
  At the R3ac landing, public async class methods were the next recommended
  milestone, followed by private async class methods and then async generators.

  R3ad ports public ordinary async instance and static class methods through
  the shape of pinned QuickJS `js_parse_class`. QuickJS classifies an async
  property name, parses it as the `JS_PARSE_FUNC_METHOD` /
  `JS_FUNC_ASYNC` pair, then publishes the fixed or computed method on the
  prototype or constructor. Oxide mirrors that split with
  `FunctionKind::Method`, `BytecodeFunctionKind::Async`, and the existing
  non-enumerable `DefineMethod`/HomeObject publication path. This preserves
  strict class bodies, AsyncFunction branding, source text, inferred names,
  non-constructibility, rejected parameter/body completion, and instance or
  static `super` across `await` without a new runtime path. As in QuickJS, the
  authored function range starts at `async` after an optional `static`, so
  fixed/computed key spelling and intervening trivia round-trip while
  `Function.prototype.toString` excludes the class element's `static` marker.
  The pinned source anchors are `quickjs.c` 24485-24565 and 25157-25520.

  The checksum-bound candidate universe contains 313 paths, all passing in
  pinned QuickJS 2026-06-04. The explicit ledger excludes eight private async
  methods, eight private async generators, and three async-generator
  adjacencies. The resulting 294-path manifest expands to 568 sloppy/strict
  variants, and Oxide passes 568/568 with no unsupported, skipped, failed,
  timed-out, crashed, or infrastructure outcome. The scoped-profile,
  candidate, manifest, variant-key, TSV, and JSONL SHA-256 values are
  `9dbf8b47dafbc6df98ae38a1c24c489fc530bf93bc5be7cd8d9efa0d86a3bd4c`,
  `59f3e239b96257ac169ad20b4df664c463e4b29f823423c833a667118b8aec8d`,
  `220fd2dd88cef8efb4ff92616f01bd28cfbf6c0e0527cd20cd14a0dbb15db524`,
  `36f9c5af110ae8d5623a528db3c8462fe2a02d57d580a948b5b146d6387a682e`,
  `d63549c1597784d0624320e14f91dc0a67bc39fe41673370edc6e3e018724b43`,
  and
  `774df1782c75f240b2163f91600745d7245980f0cdd265a56938f8c404fe2ff5`.

  The class lookahead intentionally retains two pinned target details.
  `async;` is committed as an async-method prefix and rejected instead of
  becoming an `async` field. Direct line terminators after `async` split a
  field from the following synchronous element, while U+2028/U+2029 inside an
  intervening block comment are ignored by QuickJS's small contextual scanner.
  The normal lexer continues to record those separators for ECMAScript ASI and
  restricted productions.

  Three rejection probes have a known non-blocking column mismatch:
  instance `async constructor`, static `async prototype`, and `super()` in an
  async method body. Their `ErrorKind` and message match QuickJS. Equivalent
  column offsets already occur for synchronous getter/generator constructors,
  static `prototype`, and synchronous object/class method `super()` errors, so
  this is the pre-existing shared class/method diagnostic-span debt rather
  than an R3ad semantic regression.

  The exact R3ac/R3ad full-vector join retains all 102,037 unique keys with no
  missing, extra, duplicate, detail-only, or previous-pass-regressed row. Its
  only two outcome transitions are the sloppy and strict variants of
  `staging/sm/Function/function-name-method.js`, both moving from the former
  typed public-async-class runtime frontier to pass. Runnable remains 45,140;
  passes rise from 43,659 to 43,661 and typed runtime frontiers fall from 34
  to 32. The R3ad full TSV/JSONL SHA-256 values are
  `a7bf54d0dda0b341fc4e84b7ba0edfb3af36e21ed3f5c93cbaae6cd510ef1aee`
  and
  `ab5e5385fa073939aef78864d97710fa05da0c331f001f6ffbabb85abc01f777`.
  At the R3ad landing, private async class methods were the next recommended
  milestone, followed by async generators.

  R3ae adds ordinary private async instance and static class methods by
  composing two already-authenticated paths rather than introducing a third
  callable model. Pinned QuickJS `js_parse_class` keeps the grammar role at
  `JS_PARSE_FUNC_METHOD`, selects `JS_FUNC_ASYNC`, marks the relevant class side
  as branded, forces the private callable to retain its HomeObject, then applies
  the ordinary private duplicate check, inferred `#name`, and lexical-cell
  initialization. Oxide likewise reuses R3ad's
  `parse_async_method_definition` and R3i's `BindingKind::PrivateMethod`,
  `InitializePrivateMethod`, HomeObject-derived brand, and typed callable cell.
  The unlinked publisher, linked heap validator, and live private-cell boundary
  now admit the exact async-method shape `(FunctionKind::Async, false)` while
  continuing to reject async generators and private accessors whose execution
  kind is not Normal. No new opcode, private-reference representation, brand
  store, Promise driver, or suspension format is required.

  The checksum-bound R3ae candidate universe contains 233 paths, all passing in
  pinned QuickJS 2026-06-04. Its 77-path exclusion ledger keeps 68 private async
  generators, eight public async-generator adjacencies, and one mixed staging
  path fail-closed. The admitted manifest contains 156 paths / 312
  sloppy/strict variants: 92 positive and 64 audited negative paths, 86 async
  and 70 synchronous paths, all in both modes. Oxide passes 312/312 with no
  unsupported, skipped, failed, timed-out, crashed, or infrastructure outcome.
  The scoped-profile, candidate, manifest, variant-key, canonical TSV, and
  JSONL SHA-256 values are
  `668acc7b6b7de1345a1baa90d4f60fb67a2fa8beb018ab12a9bcd4cfba928b8e`,
  `a9a2aa2e48f83d2a4beb86704923827223bef6b77b83324f8fc0a319645b93f5`,
  `baa888fd5d5bea134123d563f8cc23a2ab483d6b0644c319c8dbc210b1a8d5bf`,
  `2ecb1effac625bd14a932929fecfe4f721f264a5cfeafaed9f0717d245716231`,
  `712c9dc36155bb8337d28e30ec2ee48fd69027f3c4145fb9ce93f4e32af726c0`,
  and
  `37a22c4ea13d16a0403c73d2ac6988a566665a0c78a7ac1716bc973f5ddd9c3a`.

  The dedicated pinned-QuickJS differential covers instance/static function
  shape and authored source, inferred private names and length,
  non-constructibility, Promise settlement and rejection boundaries,
  independent brands, private-`in`, read-only assignment, extracted dynamic
  receivers, `super` in parameters and across `await`, and the synchronous
  wrong-brand-before-argument ordering. The class lookahead continues to use
  the R3ad boundary: a direct line terminator after `async` ends a public field
  before a following synchronous private method, while same-line `async #name`
  selects the private async method path.

  The exact R3ad/R3ae full-vector join is byte-identical. It retains all 102,037
  unique keys and 45,140 runnable variants, with zero outcome transitions,
  detail-only change, missing, extra, duplicate, or previous-pass regression.
  Passes remain 43,661, and the canonical full TSV/JSONL SHA-256 values remain
  `a7bf54d0dda0b341fc4e84b7ba0edfb3af36e21ed3f5c93cbaae6cd510ef1aee`
  and
  `ab5e5385fa073939aef78864d97710fa05da0c331f001f6ffbabb85abc01f777`.
  This zero-delta result is expected because R3ae remains a checksum-bound
  scoped admission; async generators are the next class-method frontier.

  R3af adds ordinary async-generator declarations and expressions on a
  dedicated QuickJS-shaped runtime driver. It reuses the resumable VM
  activation and Promise job machinery while keeping a distinct branded
  AsyncGenerator object, FIFO request queue, and suspended-start,
  suspended-yield, awaiting, executing, awaiting-return, and completed state
  transitions. `%AsyncIteratorPrototype%`, `%AsyncGeneratorPrototype%`, and
  `%AsyncGeneratorFunction.prototype%` are published as a separate intrinsic
  graph. Calls still perform parameter initialization synchronously, while
  `next`, `return`, and `throw` always return per-request Promises and settle
  them in queue order.

  The driver follows four less-obvious pinned behaviors explicitly. A genuine
  Promise whose own `constructor` getter throws is injected back into an
  authored `await` immediately, without manufacturing an extra rejected
  Promise job; repeated abrupt awaits stay in one bounded native loop. Promise
  reactions resume using the realm attached to the actual settlement job while
  decoded bytecode continues in the function's defining realm. Once completed,
  QuickJS services one already-queued request per driver entry; a later
  protocol call re-enters the driver and advances the next parked request.
  Resolution of a yielded iterator result can itself synchronously re-enter
  through an inherited `then` getter; the still-active outer QuickJS driver
  then advances a newly parked await with its untouched `undefined` slot, and
  a later stale reaction is ignored. Focused oracles lock these boundaries,
  including the distinct asynchronous rejected-Promise path for a poisoned
  completed `.return()`.

  The checksum-bound candidate universe contains 1,008 paths / 1,970 variants,
  all passing in pinned QuickJS 2026-06-04. The explicit 765-path /
  1,530-variant ledger keeps 564 destructuring, 185 `yield*`, six `for await`,
  five method-syntax, two Proxy, and three realm/host paths fail-closed. The
  admitted manifest contains 243 paths / 440 variants: 167 positive and 76
  audited parse-negative paths, 117 async and 126 synchronous paths, with 197
  dual-mode, 33 sloppy-only, and 13 strict-only paths. Oxide passes 440/440
  with no unsupported, skipped, failed, timed-out, crashed, or infrastructure
  outcome; the canonical report is byte-identical at five and eight workers.
  The scoped-profile, candidate, exclusion-ledger, manifest, variant-key, TSV,
  and JSONL SHA-256 values are
  `edb34a6dd924e3b01535b94e24495ba69a4a195b7492fed670f17714d5e543d7`,
  `695b6ebd1518df08b47ee946f5a9dcbaf10396cebf2dadf27f797dea2e91a07d`,
  `f795112b63fe9909c1cd6aa8dbb882ab5cd8c2db035aa7b69d416350f12d3d62`,
  `bfc4244e45d22fd2d98c06f6d413cc7e58b58b004dfc3eebcc7d964834108e9f`,
  `1de03a01c7a295fc8cf92c79ef8df77c4af3e641df1bd7e53249efad6b5a113c`,
  `ab0974936b304c5789a44d6298821ce885ec39d655ad6a549f4301871c81f1bb`,
  and
  `0b348ca0165431dd152c44c862adaf9e52bca64045c5e99079035196824d38e8`.

  The exact R3ae/R3af full join retains all 102,037 unique TSV and JSONL keys,
  with zero missing, extra, duplicate, metadata-drifted, or
  previous-pass-regressed row. It records 18 outcome transitions: ten
  `unsupported-parser -> pass`, three `unsupported-runtime -> pass`, two
  `fail-runtime -> pass`, one `unsupported-parser -> unsupported-runtime` at
  the explicit `for await` frontier, and two
  `unsupported-parser -> fail-runtime` variants which cross the
  async-generator parser and expose the pre-existing missing `Int8Array`
  intrinsic before Proxy behavior. Six additional same-outcome detail rows
  refine the remaining method/class-method frontier. The 15 new passes raise
  the complete vector from 43,661 to 43,676 while runnable remains 45,140;
  `unsupported-parser` falls from 45 to 32 and `unsupported-runtime` from 32
  to 30. The R3af full TSV/JSONL SHA-256 values are
  `6b34f59397a351c833b1d79803b4aafd9d93256177f59d8044361123f01391b1`
  and
  `c4f8ec2a11d5d84601c2250f25570f015952a7c10723ad92d52c649e606792ba`.
  `docs/test262.md` lists every transitioned path and variant.

  The first R3af boundary deliberately excludes async iterator closing.
  Every destructuring candidate is in the ledger. The sole admitted ordinary
  `for-of` source occurrence is a harness loop outside the async-generator
  body, so no admitted path exercises `.return()` through an active ordinary
  `for-of` or destructuring iterator. Async-generator methods, `yield*`,
  `for await`, async iterator close, Proxy, and realm/host behavior remained
  independent later milestones. The global async profile was not widened.

  R3ag adds ordinary object-literal async-generator methods as a compiler-only
  composition of QuickJS's Method grammar role and AsyncGenerator execution
  kind. Fixed and computed property names continue through the established
  enumerable `DefineMethod` path, which installs inferred names and the
  HomeObject; the R3af callable prototype graph, own generator prototype,
  Promise request queue, and resume driver are reused without a runtime or
  heap change. Differential coverage locks exact authored source, descriptors,
  nonconstructibility, fixed/computed/string/numeric/Symbol names, computed-key
  order, `__proto__` method spelling, synchronous parameter initialization,
  delayed body entry, borrowed receivers, and `super` across `await`, `yield`,
  and GC.

  The checksum-bound R3ag focused core candidate universe contains 113 paths /
  216 variants from the object method-definition and Function `toString`
  families, all passing in pinned QuickJS 2026-06-04. Its 67-path /
  134-variant exclusion ledger records 58 `yield*`, two `for await`, four
  destructuring, two private-name, and one Proxy path. The resulting manifest
  contains 46 paths / 82 variants: 23 positive and 23 audited parse-negative
  paths, 18 async and 28 synchronous paths, with 36 dual-mode, eight
  sloppy-only, and two strict-only paths. Oxide passes 82/82 with no
  unsupported, skipped, failed, timed-out, crashed, or infrastructure outcome;
  its default 8/8/5-worker reports are byte-identical. Other suite consumers
  remain visible in the complete vector. The scoped-profile, candidate,
  exclusion-ledger, manifest, variant-key, TSV, and JSONL SHA-256 values are
  `7c21b92bc769a6de2812f2c953bc7fe567e5df528255b4a85bfa429eb3d56ad9`,
  `d6fd96dcc29e4b3b87b64cfe3d8692f99bd1852762ebb7673467e9b85f6d49f9`,
  `97a7fd213d823a1c43eb650daef69c6153eb56c17db43ac54f38b1a288d97f00`,
  `d4e3923053e589ec699880a946f5e1b9f00180c0b017a98377ed1a85643f3798`,
  `f8cca2f8b154bef5aaa37d9dbc53c6a4faaec1e2048ff0f9cc8ceadee2c6e0dd`,
  `e5798193ae60299f94099b8f4b8cedc72a656051d165471c535074b3c097d93c`,
  and
  `84b86f1fac5b6e8b9e3ed6761576202221c8dbf558a3783e646aedf0a2db96b3`.

  The exact R3af/R3ag full-vector audit retains all 102,037 keys and 45,140
  runnable variants with no previous-pass regression. Both modes of
  `staging/sm/PrivateName/illegal-in-object-context.js` and
  `staging/sm/extensions/newer-type-functions-caller-arguments.js` move from
  `unsupported-runtime` to `pass`; no other outcome changes. Both modes of
  `staging/sm/BigInt/property-name.js` remain `unsupported-parser`, but their
  detail advances from the object-method frontier to the still-explicit class
  async-generator method frontier. All six changed rows are already-admitted
  consumers outside the 113-path focused candidate partition; neither the
  manifest nor its exclusions drift. Passes rise by four to 43,680 and
  `unsupported-runtime` falls from 30 to 26. The R3ag full TSV/JSONL SHA-256
  values are
  `37f72b038cdfa81ba1704bef05578e273e70a612e3daf8c23a54d22a984a5b88`
  and
  `8e7a70940a97f97232fc4fccc8b05bf57f1135896944399b9d96a8bc76fb3d2f`.
  Public/private class async-generator methods, `yield*`, `for await`, and
  active iterator closing remain explicit later frontiers.

  R3ah adds public instance/static class async-generator methods as the next
  compiler-only QuickJS composition. The class parser now distinguishes
  contextual `async *` from ordinary async and synchronous generator methods,
  then invokes the existing Method+AsyncGenerator function parser. Fixed and
  computed names still publish through the non-enumerable class
  `DefineMethod` path, so inferred names, HomeObject, descriptors, the
  AsyncGenerator intrinsic graph, and the Promise request driver require no
  runtime or heap branch. Direct private `async *#name` remains a separately
  typed Unsupported frontier instead of being miscompiled as an ordinary
  private async method.

  Differential coverage locks instance/static fixed, computed, string,
  numeric, and Symbol names; authored source beginning at `async` even after
  `static`; descriptors, prototype relationships, and nonconstructibility;
  computed `constructor` publication and the runtime TypeError for a computed
  static `prototype`; synchronous parameter initialization and abrupt
  completion; delayed body entry; `arguments`, `new.target`, `await`, and
  `yield`; and base/derived `super` with borrowed receivers across suspension
  and GC. The focused core candidate universe contains 573 paths / 1,118
  variants: 396 direct method paths, four Function `toString` paths, one
  contextual-token path, 160 class-element composition paths, and 12 syntax
  paths. Pinned QuickJS 2026-06-04 passes all 573.

  The 256-path / 512-variant exclusion ledger records 232 `yield*`, eight `for
  await`, eight destructuring-scope, and eight private-composition paths. The
  resulting manifest contains 317 paths / 606 variants: 236 positive and 81
  audited parse-negative paths, 216 async and 101 synchronous paths, with 289
  dual-mode, 20 sloppy-only, and eight strict-only paths. Oxide passes 606/606
  with no non-pass outcome; default 8/8/5-worker and override 3/3/5-worker
  reports are byte-identical. This is a focused core partition rather than an
  exhaustive async-generator class feature inventory.

  The scoped-profile, candidate, exclusion-ledger, manifest, variant-key, TSV,
  and JSONL SHA-256 values are
  `4c088b7e15be3bc1de099abf6560917c5677aa229fdc1799d0ff31367166ca63`,
  `69ad11be927670c4578b0ac5ee80e2862a9c2f2c881a5282af39fd660b5bace5`,
  `7b2a630ec520d90a973f9e7c1cd3af03938adc871afbed44f5a0893b8032e2c5`,
  `f7620c23730693b2b8b46ef85b2f373d9c5d0fd5c7da19b4af356ede77bcdc43`,
  `75e07a55c503357ead33c8782ccdb416d2a238a90757500d593b305d5d3c4d53`,
  `1e1e8bdfc2101862e835db7eda9e6ae304cdaa6457035cd2c8dd6c7fff1940e0`,
  and
  `d7d9bbd90e09f2f02d23b2533a5076887ea0dcf7f4c114ff13b472af24d5e18b`.

  The exact R3ag/R3ah full-vector audit retains all 102,037 unique keys and
  45,140 runnable variants with no duplicate, missing, extra, or previous-pass
  regression. Sloppy and strict variants of
  `staging/sm/BigInt/property-name.js`,
  `staging/sm/Function/function-name-computed-01.js`, and
  `staging/sm/Function/function-name-computed-02.js` move from the former
  public class async-generator `unsupported-parser` frontier to `pass`.
  Those six rows are already-admitted consumers outside the 573-path focused
  candidate partition; its manifest and exclusion ledger have zero drift.
  There are no other outcome or same-outcome detail changes. Passes rise from
  43,680 to 43,686 while `unsupported-parser` falls from 32 to 26; every other
  summary count is unchanged. The R3ah full TSV/JSONL SHA-256 values are
  `2932f9d54df006def9ac2e9b01a8f9b7a5228bb58a42309d2f27b5fb26d81c18`
  and
  `7e7121200f385829a3676514ad091d26c39ee9780c46ed5f54c41dadff1ad193`.
  Private class async-generator methods, `yield*`, `for await`, and active
  iterator closing remain explicit later frontiers.

  R3ai adds private instance/static class async-generator methods by following
  the same QuickJS composition. The parser retains Method grammar with
  AsyncGenerator execution, while private publication reuses the typed method
  cell, HomeObject, instance/static side brand, and
  `InitializePrivateMethod` path. The callable's own generator prototype,
  initial-yield shape, Promise request queue, and resume driver come from R3af;
  no class-specific `runtime.rs` branch is introduced. Differential and
  publication-verifier coverage lock extraction, names/source/prototypes,
  nonconstructibility, synchronous parameters, delayed body entry, FIFO
  `next`, private-name `in`, access- and resume-time brand checks, borrowed
  receivers, `await`, and `yield`.

  The checksum-bound candidate universe contains 433 paths / 858 variants:
  322 direct private-method paths (162 instance and 160 static), 68
  class-element composition paths, 40 syntax paths, two object-negative paths,
  and one staging path. Pinned QuickJS 2026-06-04 passes all 433. The
  308-path / 616-variant exclusion ledger contains 300 `yield*` and eight `for
  await` paths. Sixty-eight generated class-element composition filenames do
  not advertise delegation, but their private async-generator bodies use
  `yield * await value`; they therefore belong to that ledger rather than this
  milestone. The resulting manifest contains 125 paths / 242 variants: 29
  positive and 96 audited parse-negative paths, 22 async and 103 synchronous
  paths, with 117 dual-mode and eight strict-only paths. Oxide passes 242/242
  with no non-pass outcome.

  The scoped-profile, candidate, exclusion-ledger, manifest, variant-key, TSV,
  and JSONL SHA-256 values are
  `1b9d03b352d8e221cae6d0cc6c6c685776f16e0ca39c97c5fafc7b8bdca00f38`,
  `3b54cf73426d746a18563c75b4b827b7c4d25d3ee98e8908ca312b7db43dd909`,
  `3508dcaff42bb06de45f8b6678170a290fdf52bc932a7a6b8c4d5bd662e7839c`,
  `82bae49d063b9691d245f1a08d0e37583fc27282ceb878cca7c4e1129e6fcad6`,
  `e0f31c9d25a89ec4b6d8ca5b2a7ba13ab223d219d65c56e84f478a34f50b9bbb`,
  `d4b22c03825eeb1d0a6e6214a69eec9dbea3c81f2571b4f0d6aa7dd84c55c0ec`,
  and
  `c3ebc03b435d2ca8f534cd48970da8d703c4edd6dc8b02a4600a514030ae0d6f`.

  The exact R3ah/R3ai full-vector audit is byte-identical. It retains all
  102,037 unique keys and 45,140 runnable variants, with zero outcome
  transition, detail-only change, missing, extra, duplicate, or previous-pass
  regression. Passes remain 43,686 and the full TSV/JSONL SHA-256 values remain
  `2932f9d54df006def9ac2e9b01a8f9b7a5228bb58a42309d2f27b5fb26d81c18`
  and
  `7e7121200f385829a3676514ad091d26c39ee9780c46ed5f54c41dadff1ad193`.
  This zero-delta landing keeps broad async admission fail-closed.
  At that checkpoint, async-generator `yield*` was the next semantic priority.

  R3aj adds async-generator `yield*` across ordinary
  declarations/expressions, object-literal methods, and public/private
  instance/static class methods. The implementation follows pinned QuickJS
  2026-06-04: it prefers `Symbol.asyncIterator`, falls back through
  Async-from-Sync to `Symbol.iterator`, caches delegate methods, and keeps the
  async and synchronous value-assimilation paths distinct. Ten differential
  transcripts lock iterator acquisition, `next`/`throw`/`return`, FIFO,
  missing-method, close/error-priority, and abrupt-completion behavior; a
  separate GC test retains suspended async and synchronous delegates.

  The authenticated cohort is the duplicate-free union of the four prior
  `yield_star` ledgers: 775 paths / 1,550 sloppy/strict variants. Pinned
  QuickJS passes 775/775 and Oxide passes 1,550/1,550 with no non-pass outcome.
  Independent 8/8/5-worker TSV and JSONL reports are byte-identical. The
  focused profile, manifest, key-set, TSV, and JSONL SHA-256 values are
  `80bd7d1c042473a76ba15d85b3e5bbd6ebf175f0543c57e2908fd99a6b7b5256`,
  `bb31f01a982136b336f9267701ef8b2874bc0596e226f6e9ca5b59e7b9af09fb`,
  `d3beb98f2b199c3a66acf4c58d44f65c06f2edf6ef2a52fe4d7caf045105dec5`,
  `b819f6fe3443cfd2f3baefdde489d397ea405115f5692f943172e010df08dc40`,
  and
  `53ebba1f2d8fb80ab82aff4869b99646230f16013fd9fef8a6660d48ef36a915`.
  The complete R3aj regression retains all 102,037 variants, 45,140 runnable
  variants, and 43,686 passes. Its TSV and JSONL are byte-identical to R3ai at
  `2932f9d54df006def9ac2e9b01a8f9b7a5228bb58a42309d2f27b5fb26d81c18`
  and
  `7e7121200f385829a3676514ad091d26c39ee9780c46ed5f54c41dadff1ad193`.
  This result still does not claim complete async iteration. `for await` is
  next; closing an independently active outer iterator when `.return()`
  crosses delegation remains a separate semantic frontier.

  R3ak implements `for await ... of` in ordinary async functions and async
  generators across function, object-method, and public/private class-method
  forms. The compiler follows QuickJS's three-slot iterator record and emits
  `for_await_of_next -> await -> iterator_get_value_done`; the VM disables
  automatic close before calling cached `next`, carries that state across the
  suspension, reads `done` then `value` even on completion, and re-enables
  close only after both reads succeed. Async-from-Sync fallback reuses the R3aj
  adapter. Async-generator return across an active iterator uses QuickJS's
  hand-lowered `return` call, pre-Await Object check, and Await; ordinary async
  function exits retain QuickJS's synchronous close behavior. The differential
  fixture also freezes upstream's observable quirks: natural exhaustion calls
  `return`, ordinary close does not await its Promise, and next/result failures
  do not close.

  The authenticated input candidate intersects the exhaustive pinned metadata
  inventory with every tracked `.js` path whose name or source contains
  `for-await`: 1,297 paths / 2,531 sloppy-strict variants. The explicit
  33-path / 41-variant dependency ledger removes three
  `explicit-resource-management` paths skipped by upstream QuickJS, 28
  module/dynamic-import paths, one optional-chaining path, and one
  `$262.IsHTMLDDA` host path. The resulting executable milestone manifest is
  1,264 paths / 2,490 variants.

  This is intentionally broader than the 24 `for_await` rows inherited from
  the four async-generator exclusion ledgers. It contains 1,232
  baseline-enabled paths from `language/statements/for-await-of` (including
  1,215 destructuring paths), those 24 ordinary/object/public/private
  async-generator shapes, five `AsyncFromSyncIteratorPrototype` consumers,
  one async/Promise interleaving path, and two staging grammar paths. Pinned
  QuickJS passes 1,264/1,264 admitted paths; it also executes all 1,294
  baseline-enabled paths in the wider candidate, with exactly the three
  upstream-configured ERM skips.

  At R3ak, the gate reproduced the candidate derivation, all metadata and
  variant partitions, source-ledger provenance, profile and inventory hashes,
  both pinned-QuickJS runs, and the Oxide result. Oxide passed all 2,490
  variants with no failure, unsupported, or skipped outcome; independent
  8/8/5-worker TSV and JSONL reports were byte-identical. The
  profile, manifest, key-set, TSV, and JSONL SHA-256 values are
  `20b369af5ce33890a6c480835baf3801392c26e6d7432da9d55fba1c4c1ad823`,
  `45afa1e6f8f61d44e733aeea8bde5dae562a7ec919ea40d9d1e18551d6f2881f`,
  `756ea05ac92fed9281a84f8e7f40b1992c640258ca41790158c41dfbe720bf57`,
  `7eafa4725fbb6f70954c5bdb52a823caeaa89497eb01d6c80d446925d01361d0`,
  and
  `ecba171afdc2272de5b0e40b824f28159bfad04c9f485527b64ad6b533dd00fd`.
  A pinned transcript and repeated-GC test separately cover close ordering and
  the pending-next activation root not represented by the Test262 cohort.
  The exact R3aj/R3ak full-vector join changes only three already-admitted
  SpiderMonkey staging variants from `unsupported-runtime` to `pass`: the
  sloppy `for-await-bad-syntax.js` variant and both variants of
  `for-await-of-error.js`. All 102,037 keys match with no other outcome,
  detail, key-set, or previous-pass drift. The complete vector retains 45,140
  runnable variants and reaches 43,689 passes; its TSV and JSONL SHA-256 values
  are
  `36e2a11f4eaba4ffd92fdd561b18b27337b90b14a564cab9da6385f1aa0f79a3`
  and
  `1dd6c356c678568b51794d253959a58a644dbdd2871187f67516ad8d78e649af`.
  These remain the historical R3ak receipt; R3bk above records the refreshed
  current focused gate without changing the complete vector.

  R3al promotes `async-functions`, `async-iteration`, and the async Test262
  host into the global capability profile after the R3z-R3ak implementation
  stack closed ordinary async functions, every async method/generator shape,
  `yield*`, and `for await`. The exact admission cohort is 3,589 paths / 7,076
  canonical variants. Every newly executed variant passes; none becomes a
  parser, runtime, harness, timeout, crash, or engine fault.
  `./scripts/test-test262-global-async.sh` authenticates the frozen manifest
  against the exhaustive metadata inventory, runs all 3,589 paths in pinned
  QuickJS, and proves Oxide's 7,076 variants byte-identical at 8/8/5 workers.
  The manifest, metadata selection, key-set, TSV, and JSONL SHA-256 values are
  `7e83bef89f3deaf151275877fd3baeab1891ed66cdc423af8e52c45a858acd97`,
  `b94d52b85bc1faa296bada8b0dd7f09e70e3fe3e2575c6cfcdccbd66138f3a29`,
  `8029a961f158f0b649532cd13ff18d85a07a133ed1e3b37a0494fd3e624908db`,
  `136b179ed6ab8d4b17c56e0ed6e214753c5700fcbc448a4d10d5d95bf648be40`,
  and
  `14ec16dd95ff9953b58d2be537f71b21611d5419f0f904af73e8ae0e7960997f`.
  A frozen R3ak before-outcome table independently covers all 12,647 variants
  whose old selection detail mentioned the promoted async capabilities.
  Reclassifying that 6,496-path universe under R3al proves that exactly the
  manifest's 7,076 keys become runnable passes; the other 5,571 remain
  explicitly unsupported. The before-table SHA-256 is
  `173d61580131172206cb476a4239395a5a258d539723587d924d161eb12d461f`.

  The exact R3ak/R3al join retains all 102,037 keys and every previous pass.
  It records 6,122 `unsupported-async -> pass` and 954
  `unsupported-feature -> pass` transitions. The remaining selection changes
  are honest dependency refinement: 3,866 async rows expose another missing
  feature, 952 feature rows expose unaudited negative provenance, 75 async
  module rows become the existing module frontier, and four async rows expose
  `createRealm` or `IsHTMLDDA`. Another 674 unsupported-feature rows keep their
  outcome while dropping the two newly admitted tags from their detail.

  The global profile now has 76 features, 802 audited negative paths, and
  async execution enabled; its SHA-256 is
  `fc6e8010c982bd6324b146e5f8e3ea0592aac7c03a323a8dbc8d778b4b670b23`.
  Runnable variants rise from 45,140 to 52,216 and passes from 43,689 to
  50,765. The full TSV and JSONL SHA-256 values are
  `93456e63a780ac6b02253853a5711464d01944f6df30a22d8b1a6fcde6a66366`
  and
  `40417ac19f60988a3257e4d577ea1f485ef61637f1c444820ebe5662638fa13e`.
  This is a global combination gate over implemented semantics rather than a
  claim that the remaining async-tagged tests are complete: adjacent class,
  default-parameter, module, Promise-method, host, and negative-provenance
  dependencies remain fail-closed.
  R3am delivers the completion-aware internal-method seam modeled on QuickJS's
  generic object-operation dispatch. Heap-backed Proxy objects retain target
  and handler edges, preserve callable/constructable identity, publish `Proxy`
  and `Proxy.revocable`, and implement revocation plus all 13 traps:
  `getPrototypeOf`, `setPrototypeOf`, `isExtensible`, `preventExtensions`,
  `getOwnPropertyDescriptor`, `defineProperty`, `has`, `get`, `set`,
  `deleteProperty`, `ownKeys`, `apply`, and `construct`.

  Object/Reflect operations, `for-in`, class fields, Array and JSON consumers,
  callable RegExp checks, dynamic-eval/global-binding lookup, and new-target
  realm selection now use the same completion-aware seam. The pinned QuickJS
  differential locks fallback order, invariants, nested proxies, revocation,
  reentrancy, error realms, GC edges, and target-specific quirks. The
  checksum-bound 464-path / 904-variant Test262 gate records 811 passes, 81
  explicit unsupported outcomes, and 12 TypedArray-adjacent harness failures;
  pinned QuickJS passes 904/904. The exact full-vector join adds 208
  `fail-runtime -> pass` and four `timeout -> pass` transitions with no
  previous-pass or key-set regression, bringing the complete vector to 50,977
  passes.

  `ownKeys` duplicate and invariant checks use an atom-identity set rather than
  repeated linear scans. Large uniquely owned shapes append in place after
  eight canonical entries, while shared and common small shapes retain the
  weak-cache immutable-transition path. The 15,000-key SpiderMonkey
  `ownkeys-linear` test now passes both modes in under five seconds together.
  This keeps the main `runtime.rs` facade at 9,904 lines, below R3al's 9,912;
  the 1,706-line completion-aware dispatcher and 226-line Proxy intrinsic live
  in `runtime/internal_methods.rs` and `runtime/intrinsics/proxy.rs`. QuickJS's
  hashed-successor reuse for two independently built large equal layouts
  remains a non-observable memory/performance optimization opportunity, not a
  semantic deviation. Complete hashes and frontier breakdowns are recorded in
  `docs/test262.md`.

  R3an adds a genuinely branded `ObjectKind::ArrayBuffer` /
  `ObjectPayload::ArrayBuffer` whose backing store is an owned `Vec<u8>`,
  rather than an emulated property bag. The payload keeps fixed-versus-
  resizable identity, `maxByteLength`, and detached state independently, so an
  attached empty store remains distinct from a detached buffer and detachment
  releases the bytes without erasing the surviving metadata. The independent
  `runtime/intrinsics/array_buffer.rs` owner installs and dispatches the
  constructor, species, accessors, `slice`, `resize`, `transfer`, and
  `transferToFixedLength`; the same backing-store boundary implements the
  Test262 host detach operation. Allocation, conversion, subclass/species,
  reentrancy, error ordering, and detach/transfer behavior are locked against
  the pinned QuickJS 2026-06-04 source and executable oracle.

  The checksum-bound pure-core gate starts from 168 ArrayBuffer paths. It
  freezes 24 latent transfer paths whose sources directly instantiate
  `Uint8Array` without declaring TypedArray metadata; those paths stay
  fail-closed until a coherent DataView/TypedArray view kernel lands instead
  of growing a test-only partial view. The resulting 144-path / 288-variant
  manifest passes 288/288 in both Oxide and pinned QuickJS. Its scoped profile,
  manifest, key-set, TSV, and JSONL SHA-256 values are
  `0803a027b2e9c238f80189993968816adfdda983ef3b23114a06f07b26c2d598`,
  `d5720cc22c785d3757eb4e30aa3de53a664d58133a2323c6afe6233788014d01`,
  `bb2d3b0e3728e4aae955569ba0ffefc54ad215a02cfe5204fc3d483daf6e3bad`,
  `254ae11ac69e0d2b13f9949f498224af8770cdf16c120c8a24fe5faaa9d97716`,
  and
  `43bb5e266e7558dd0b425831caefe7fb11d8fa8601194dac7c3f4042ec1ee642`;
  the 24-path exclusion stream is pinned at
  `5118e3de12f8d432856c99112ff9ec093da3e83f40c52a8c19c3b39b3d05b610`.

  The global profile promotes only `ArrayBuffer`, `arraybuffer-transfer`, and
  `align-detached-buffer-semantics-with-web-reality`;
  `resizable-arraybuffer` remains deliberately unpromoted until its wider view
  dependencies are coherent. The profile now has 79 reviewed features and 802
  audited negative paths, with SHA-256
  `9b155f41c9c7541423c45b57da1bb805d6e7cf350ec7d6442d6700424afdbafc`.
  The exact R3am/R3an full-vector join retains all 102,037 keys, adds 216
  passes and 252 runnable variants, and has zero previous-pass regression.
  The resulting vector has 51,193 passes, 52,468 runnable variants, and 52,419
  non-unsupported observed outcomes: 50.17% raw, 61.26% against the
  QuickJS-exclusion lower-bound denominator, and 97.66% observed. Full
  TSV/JSONL SHA-256 values are
  `12a60e9d1cd3e30b8b33e095ef226f50f56706bed942cdc465c15cc3463d45fe`
  and
  `814f8e1e6e99dba7778c3ba8bc4b26f4015ebe0130c1e5cc5f1e1c55653a8fb2`.

  R3ao builds a branded DataView on that backing store as an ordinary object
  with a traced ArrayBuffer edge, byte offset, and fixed-versus-tracking length
  metadata. Its dedicated intrinsic owns the constructor, `buffer`,
  `byteLength`, `byteOffset`, `ArrayBuffer.isView` integration, and all 11
  signed, unsigned, BigInt, Float16, Float32, and Float64 getter/setter pairs:
  `Int8`, `Uint8`, `Int16`, `Uint16`, `Int32`, `Uint32`, `BigInt64`,
  `BigUint64`, `Float16`, `Float32`, and `Float64`. Pinned QuickJS conversion,
  endian, detach, range, coercion, reentrancy, and error-ordering behavior are
  locked without introducing a TypedArray or SharedArrayBuffer dependency.
  Fixed and length-tracking views derive their bounds from the current
  resizable ArrayBuffer state, including shrink-induced out-of-bounds state
  and recovery after a later grow.

  The checksum-bound DataView cohort starts with 578 candidate paths. Its
  86-path exclusion ledger keeps TypedArray, SharedArrayBuffer,
  immutable-buffer, and cross-realm dependencies outside this milestone,
  leaving 492 paths / 984 sloppy-strict variants. Oxide and pinned QuickJS both
  pass 984/984. The candidate stream, exclusion path stream, and exclusion
  ledger file SHA-256 values are
  `1df8f075f57cbcc2cf72f88835bbd08449fe2093bf8f5d33badc0148249db3ed`,
  `feade99c881ad6763b2241d988ab4c95ff3a8b79ae51f6c3ddf0501b62fd9354`,
  and
  `9cdc8a031c926dd59dc152b0cfb76bd97758d63d79703df86d162b3a7eec4f44`.
  The manifest, scoped profile, key stream, TSV, and JSONL SHA-256 values are
  `3475b4a32f0a5f0ab50d5cd4e4843a7c7a59365298ecabcc5986b3fdd3f697e2`,
  `485ea3baf6695767108fb9f7f346c3a82d5a3db000af4510d6d002b313990cc8`,
  `07d60a25d9dcb8316d4602456931cedff7668df634a92ab11c6efe4798c3f90c`,
  `6a73330ca5a7114d60946cf276d7b2601fdd023b260789cea1b5c911380d1206`,
  and
  `3a4b68f28084b0dc76773fe7255e090da73981afbab5388766fe6a149beb542b`.
  The independent `oracle_data_view` target passes all 3/3 Rust, frozen-vector,
  and pinned-QuickJS checks.

  The exact R3an/R3ao full-vector join retains all 102,037 unique keys, with
  zero missing, extra, duplicate, or previous-pass-regression rows. Its only
  transition is 514 `fail-runtime -> pass` outcomes across 257 paths: 502
  variants under `built-ins/DataView` and 12 under `staging/sm/DataView`.
  The changed-key stream has SHA-256
  `e3483d6bfb005a92ad9f5515d2fe8e7745c3e8a003be6f7291fa376ff8b9487c`.
  The resulting classified vector has 51,707 passes, 52,468 runnable variants,
  587 runtime failures, and 52,419 non-unsupported observed outcomes: 50.67%
  raw, 61.88% against the QuickJS-exclusion lower-bound denominator, and
  98.64% observed. Full TSV/JSONL SHA-256 values are
  `3d79ecd1349488f03e8288a9a0f41b4bc5e8b70573e8d41121438aa893940990`
  and
  `b233a6fe08dc14d0bd428f537cd9693f37a3d1d2a4f5d2b49881f9607ca60996`.

  R3ap adds one branded payload and one behavioral kernel for all 12 concrete
  TypedArray classes in pinned QuickJS class-id order: Uint8Clamped, signed and
  unsigned integer widths, BigInt64/BigUint64, and Float16/32/64. Concrete
  constructors and prototypes share the hidden `%TypedArray%` graph. Length,
  buffer, byte length, byte offset, tag, values iteration, integer-indexed
  internal methods, ArrayBuffer view detection, detach, and fixed or
  length-tracking resizable-buffer state all derive from the live backing
  store. Constructor and `set` coercion/reentrancy order, same-kind memmove,
  receiver-aware assignment, dynamic own keys, `for-in` refresh, GC edges, and
  host property definition are locked by Rust tests. The different-kind
  overlapping `set` path deliberately reproduces pinned QuickJS's observable
  `[2, 2]` result rather than substituting the specification's temporary-copy
  result.

  The audited candidate contains 2,361 paths / 4,669 variants. Its
  checksum-bound exclusion ledger assigns 1,626 paths to later prototype
  method families or explicit cross-realm, SharedArrayBuffer, WeakMap, Math,
  and IsHTMLDDA dependencies. The resulting shared-core manifest contains 735
  paths / 1,447 variants; Oxide and pinned QuickJS both pass 1,447/1,447 with
  zero failure, unsupported, skip, or timeout. Profile, manifest, exclusion
  ledger, key-stream, TSV, and JSONL SHA-256 values are
  `046200aa1abd9afa11a63602d5a8ea073ba6dd1ccee2e910775731c175378402`,
  `9ebae7adb9e1c033a71c0abf42aa003e0e03121da24ef98ca939e1f360a03777`,
  `2b18c745fe886709f578ba9cd927cea21c98dca9c02a6664c94f6fce3385e400`,
  `2d1e474a52971496b669d5f3d650dece8c21069944a463356954442dbbf75362`,
  `816005701f3d6d5273860454dcde466bd7bfe64d24c44834ffea5d5363af71d2`,
  and
  `fb86a625a7bc9eddf043db9be4b736d65e4d023972219d7569ce082826cfd92c`.
  Static `from`/`of`, `set`, and iterator entries/keys are present and have
  directed coverage, but remain outside this all-green claim until their
  complete method-family cohorts land.

  The exact R3ao/R3ap full-vector join retains all 102,037 unique keys with no
  missing, extra, duplicate, or previous-pass-regression row. It records 149
  `fail-runtime -> pass`, 46 `harness-error -> pass`, and two
  `harness-error -> fail-runtime` transitions; the latter are both modes of
  `staging/sm/Math/atanh-approx.js`, which now pass the TypedArray harness and
  expose the independent Math-accuracy frontier. Another 44 rows retain their
  outcome while advancing from missing constructors to honest later-method or
  external-dependency details. The 197-transition stream has SHA-256
  `2b94d55d59acaf0daa969cbd7c3af8d0ada968f70713c304dbbbe83f48620304`.
  The classified vector reaches 51,902 passes with unchanged runnable and
  observed denominators; full TSV/JSONL hashes are
  `8a1b83df5e28641fb57d5d4a6fe29ed8c5b1f962e82c98f6acbce0cf595e85e5`
  and
  `a3f7a5952f67ab7e1c8055d8ef29f2645700c8aa6124411644c8cb6058684052`.

  R3aq publishes the in-place TypedArray mutation family on that shared
  kernel: `set`, `copyWithin`, `fill`, and `reverse`. The new algorithms mirror
  pinned QuickJS's initial length snapshot, target/start/end or value/start/end
  coercion order, final detach/out-of-bounds revalidation, and live
  resizable-buffer clipping. Backing-store mutation remains allocation-free:
  same-buffer copies use memmove semantics, while fill and reverse operate on
  raw 1/2/4/8-byte words so NaN payloads and negative zero survive. Directed
  tests cover overlap, BigInt, byte-offset views, shrink/detach, temporary
  fixed-view out-of-bounds recovery, partial tracking elements, heap bounds,
  and raw-word invariants.

  The unchanged 2,361-path / 4,669-variant TypedArray candidate contains a
  254-path / 508-variant mutation cohort. Two `set` paths depend on the
  not-yet-published TypedArray `join`; one SpiderMonkey `set` path depends on
  its unavailable WeakMap harness. Keeping those three paths / six variants in
  the ledger leaves 251 paths / 502 variants to promote. The cumulative gate
  is now 986 paths / 1,949 variants with 1,375 exclusions, and both Oxide and
  pinned QuickJS pass 1,949/1,949. Profile, manifest, exclusion-ledger,
  key-stream, TSV, and JSONL SHA-256 values are
  `663ac07f1fe379125eec29aec0c7b8b8215c08f40b93e9c39056ff40c6331036`,
  `8542757a466917d9841cdc25317b78abad5db64aceda07ab78c8f38ced08bd3f`,
  `fe441699f63debd30e3c5e2ed66d2c9b21732280afc03807be8a2268dbe56c3a`,
  `1b983b9b5c97314449c54ec0da387f393964a758db02836e6bd2b9aa0af39f7b`,
  `159c4b02f25fe4430c970891141acda807336933382bd7363d4ed1d2a77dc618`,
  and
  `0d5d6917134fc7087a301e23be7d24c3544fc739af158a6eaa270dd0615ac25c`.

  The global profile remains conservative until the rest of the broad
  TypedArray family lands. It therefore keeps 284 newly authenticated variants
  classified as `unsupported-feature`; only the two untagged
  `fill-detached.js` modes move from runtime failure to pass. The complete
  summary reaches 51,904 passes and 438 runtime failures, with every other
  outcome count and both runnable/observed denominators unchanged. Full
  TSV/JSONL hashes are
  `ab641b72ef2c2bc4615d493e03cf1538c308daa2edd4c8b7e752c0da3416e586`
  and
  `7eae1d679bfe748a6ea7123c534e60c0ba8d8fe5edfa29ff6a0a16ffb3e15e5f`.

  R3ar publishes `%TypedArray%.prototype.at`, `includes`, `indexOf`, and
  `lastIndexOf` through a dedicated indexed lookup/search kernel. It mirrors
  pinned QuickJS's initial brand/out-of-bounds validation, length snapshot,
  observable index coercion, live resizable-buffer length cap, direct
  integer-index reads, Strict Equality versus SameValueZero split, and the
  target-specific `includes(undefined)` rule for a tail that disappears during
  `fromIndex` coercion. Directed tests cover Number/BigInt separation, NaN,
  signed zero, positive grow, negative indexing against the old length,
  shrink, detach, fixed-view OOB, descriptor identity, and the filtered
  QuickJS own-key order.

  The independently audited atomic inventory has 152 paths / 304 variants.
  One SpiderMonkey staging path loads a harness with an unavailable WeakMap
  dependency and remains attributed as such; the other 151 paths / 302
  variants join the cumulative 1,137-path / 2,251-variant gate. Oxide and
  pinned QuickJS both pass 2,251/2,251, while the expanded 2,361-path /
  4,669-variant candidate also remains all-green in the oracle. The scoped
  profile, manifest, exclusion-ledger file, and key-stream SHA-256 values are
  `c5d1a75871d567f892a982a1c549390c0f79aa3cefbd057dd88f713e98aafed7`,
  `85f8c692cdd7ae1715f19006da3b11f6f34e4b598f18f701ebc9fd911c9e9714`,
  `6eb2500c8befaaee380d1bed1e94f03450592f5d3da86c2cd523b6f7c2f9da62`,
  and
  `8489275bb065e249286a3f113f26a90b9483b5030f2809e8575ec3148f419067`.
  Its canonical TSV/JSONL report hashes are
  `cd4e54e8444178f8828b26615b983d90e3791346def1eec0e3d570e1c3204197`
  and
  `8a8d3f884bc2b22a2112a8d44ecb2cbf6091866235692239252b4352cedb4c28`.

  The broad global TypedArray tag stays withheld. Exactly four untagged
  staging modes move from runtime failure to pass; the other newly
  authenticated variants remain fail-closed globally. The exact 102,037-key
  join has four `fail-runtime -> pass` transitions and zero previous-pass
  regression, missing, extra, or duplicate rows. Its transition stream
  SHA-256 is
  `2b87010242ba56dcf9ca6bf1b49c733db36b3b4e558cd945b12ce22aa4acb2f7`;
  the complete summary reaches 51,908 passes and 434 runtime failures. Full
  TSV/JSONL hashes are
  `3e5f9fd57b7a19a51843db7585e2b4aebed0fc1b93b75856f482dec962805fe3`
  and
  `f75fd46059efcaade454d125b7643eb7a067b856f30570396663cf472443da37`.

  R3as publishes `%TypedArray%.prototype.find`, `findIndex`, `findLast`, and
  `findLastIndex` through a dedicated callback traversal kernel. It follows
  pinned QuickJS's initial brand/out-of-bounds validation and one-time length
  snapshot, then performs live integer-index reads across that original range.
  Callback shrink or detach therefore supplies `undefined` for disappeared
  slots without skipping them, growth does not extend traversal, callback
  writes are visible to later iterations, and a truthy callback returns the
  value captured before the callback. Forward and reverse methods share the
  same kernel without falling through generic Array property traversal.

  The atomic inventory contains 158 paths / 300 variants. Two SpiderMonkey
  staging paths require `sm/non262-TypedArray-shell.js` and its unavailable
  WeakMap dependency; the remaining 156 paths / 296 variants join the
  cumulative 1,293-path / 2,547-variant gate. Oxide and pinned QuickJS both
  pass 2,547/2,547, and pinned QuickJS also passes the unchanged expanded
  2,361-path / 4,669-variant candidate. The scoped profile, manifest,
  exclusion-ledger file, and variant-key SHA-256 values are
  `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`,
  `38fe4dd01e098bee2c646865039c49e989b079f66c88913fbf644b438279b8ac`,
  `a8e2e74492138119133cabf6dd7d5fd1133cb06ce259f88f8c777d857154c2ef`,
  and
  `f689489da433d110e4fe32be1940d141751d4112341a0319a43a0df5a815eeca`.
  Its canonical TSV/JSONL hashes are
  `7b0d8183176cdc53a1e5502dba684e80fe40549758e0e44bd875a0258253a4ae`
  and
  `1ec975c7f5b60a81a9363dffea10faaa993ade9f385b14621062cb06d78e2538`.

  Broad global TypedArray admission remains withheld, so all 296 promoted
  variants were already fail-closed as `unsupported-feature`; the two
  deferred staging paths remain harness failures. The complete 102,037-key
  vector is therefore byte-identical to R3ar at 51,908 passes, with zero
  previous-pass regression. Its TSV/JSONL hashes remain
  `3e5f9fd57b7a19a51843db7585e2b4aebed0fc1b93b75856f482dec962805fe3`
  and
  `f75fd46059efcaade454d125b7643eb7a067b856f30570396663cf472443da37`.

  R3at publishes `%TypedArray%.prototype.every` and `some` through a
  TypedArray-specific forward callback kernel corresponding to pinned
  QuickJS's `js_array_every` TypedArray branch. Receiver branding and initial
  detached/out-of-bounds validation precede callback-callability checking,
  and the shared initial OOB diagnostic is calibrated to QuickJS's
  `ArrayBuffer is detached or resized` message. The kernel snapshots the
  internal length once, bypasses `HasProperty` and numeric prototype getters,
  and reads each indexed value live. Shrink or detach therefore supplies
  `undefined` across the remaining original range, growth does not extend the
  range, and callback writes or a fixed view that regrows are observed by
  later iterations. Both methods preserve `(value, index, receiver)`,
  `thisArg`, abrupt completion, and their inverse Boolean short-circuit rules.

  The independently audited atomic candidate contains 93 paths / 185
  variants. The single
  `test/staging/sm/TypedArray/every-and-some.js` path is deferred as
  `external:cross-realm`; its harness also has a hard WeakMap dependency, so
  that one path / one variant remains explicit. The other 92 paths / 184
  variants join the cumulative 1,385-path / 2,731-variant gate. Oxide and
  pinned QuickJS both pass 2,731/2,731, pinned QuickJS passes all 4,669
  variants in the unchanged 2,361-path expanded candidate, and the exclusion
  ledger falls to 976 paths.

  The candidate path and variant-key SHA-256 values are
  `dbbd4a7e6f601888070c0f56de9771942e4d2354d75a29ab70439df3517d61cd`
  and
  `213e8b79b6447d17e562139b268ab87d7394ee6edebc755f4c4bbb31b9fe3ec4`.
  The deferred path/key hashes are
  `6189caae9a943a1fa5d65308b4bba02c25bba4af5d9e7e791da8820bd851b99f`
  and
  `2b728d9962391b75d27de09d05010642a9919f826719497c55e40e3f03a3e2f2`;
  the promoted path/key hashes are
  `8ad580d2a9cb33a091e714f7f309fd6c814503bfcb251ccdfd3bbbf5f87bae88`
  and
  `9144eaf7e8b0c6664fd082d639aa35c176ee34d3d1947452fad6523dabe22604`.
  The scoped profile, cumulative manifest, exclusion-ledger file, and
  cumulative key-stream hashes are
  `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`,
  `e96748da96cf70a08e0e678e46db24de4bf724d4d9b1bdd2012bc733596fb117`,
  `14dbdcf4d3eda7f9f0c26dade127cfca2a7cea415c732770216bd7acb6d13939`,
  and
  `17b39adb34d9ed0502713acea7e1e75228043d7462de366ffd67747f8677ddff`.
  Its canonical scoped TSV/JSONL hashes are
  `830cd524c30d68581aa7a22052f7d25ff8580c3cecf66723a9ebf031ebc36be2`
  and
  `f328ef2fcb4462ca5468cecaf1e5cfc3e170e347b483d360cf849f3073d35ea1`.

  Broad TypedArray admission remains withheld, so the complete 102,037-key
  vector is byte-identical to R3as at 51,908 passes. A fresh canonical
  two-worker run confirms the same full TSV/JSONL hashes
  `3e5f9fd57b7a19a51843db7585e2b4aebed0fc1b93b75856f482dec962805fe3`
  and
  `f75fd46059efcaade454d125b7643eb7a067b856f30570396663cf472443da37`.

  R3au publishes `%TypedArray%.prototype.forEach` through the same
  TypedArray-specific forward callback kernel corresponding to pinned
  QuickJS's `js_array_every` TypedArray branch. It preserves the existing
  receiver branding, initial detached/out-of-bounds validation, one-time
  internal-length snapshot, live indexed reads, numeric-prototype suppression,
  callback arguments, `thisArg`, and abrupt-completion behavior. Unlike
  `every` and `some`, `forEach` discards every normal callback result without
  `ToBoolean`, never short-circuits, and returns `undefined` after the entire
  snapshotted range. Focused differentials lock the exact `not a TypedArray`,
  `not a function`, and `ArrayBuffer is detached or resized` diagnostics and
  their priority.

  The independently audited atomic candidate contains 45 paths / 89 variants.
  The single `test/staging/sm/TypedArray/forEach.js` path is deferred as
  `external:cross-realm`; its harness also has a hard WeakMap dependency, so
  that one path / one variant remains explicit. The other 44 paths / 88
  variants join the cumulative 1,429-path / 2,819-variant gate. Oxide and
  pinned QuickJS both pass 2,819/2,819, pinned QuickJS passes all 4,669
  variants in the unchanged 2,361-path expanded candidate, and the exclusion
  ledger falls to 932 paths.

  The candidate path and variant-key SHA-256 values are
  `ee8af85d761e4da707fc72afc992e8c0e0b314782d0f879cff69845e66cc2bf6`
  and
  `67f42550bd10879a86d2401c4048e30a833a6ccda375b0d41ed44287b575c2a5`.
  The deferred path/key hashes are
  `26efea2e4065acf3a5bf1d8dab6ed0a78df866e1d956f9e08c44644635a5239f`
  and
  `e3ce2a05f163af4827c1fdad2c7535a2dfe7f46bbe27c3c0ed76a803650bf661`;
  the promoted path/key hashes are
  `dba18b09bd2a2bc35a9f716e9a371547757d6225d2433c524a45cd5b92ba7177`
  and
  `e3c038e152bb843d9dd55e9d16f89ca6227ac690a1e6d378c78d26757a211c4f`.
  The scoped profile, cumulative manifest, exclusion-ledger file, and
  cumulative key-stream hashes are
  `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`,
  `cb837c070ca771c4c9b29a60a7dab0f3d83866f2b7508a82b57a846a9253d1f9`,
  `58c132e168bbaea25271c4d3dd7c6161b031d5fd883054e4aaf720eab999810d`,
  and
  `446625e6284b989b8a18fb54064778ebbf471172cb0ed6caf0c3950f4e2f19a5`.
  Its canonical scoped TSV/JSONL hashes are
  `50765aa252be5e634181d870dadafe8a7971f812a492f2c58d7878d1425ca3c8`
  and
  `8ded861e362fe5cc5b276d843aee0c4d8cc93e47db657593da0008e9289afb0d`.

  Because broad TypedArray admission remains withheld, a fresh canonical
  two-worker rerun confirms that the complete vector remains byte-identical to
  R3at at 51,908/102,037, with the same full TSV/JSONL hashes
  `3e5f9fd57b7a19a51843db7585e2b4aebed0fc1b93b75856f482dec962805fe3`
  and
  `f75fd46059efcaade454d125b7643eb7a067b856f30570396663cf472443da37`.
  This is the confirmed no-transition join.

  R3av publishes `%TypedArray%.prototype.reduce` and `reduceRight` through a
  TypedArray-specific accumulator kernel corresponding to the TypedArray
  branch of pinned QuickJS's shared `js_array_reduce`. Receiver branding and
  initial detached/out-of-bounds validation precede callback-callability
  checking; callback validation in turn precedes the explicit-initial-value
  decision and the `empty array` error. An explicitly supplied `undefined`
  therefore remains a real accumulator, while an omitted accumulator seeds
  `reduce` from the first element and `reduceRight` from the last without
  calling the callback for that seed.

  Both directions snapshot the internal length once, bypass `HasProperty` and
  numeric prototype lookup, and read every remaining original-range index
  live. Callback shrink or detach supplies `undefined` for disappeared slots,
  growth does not extend traversal, and later writes are observed. Each call
  receives `(accumulator, value, index, receiver)` with `this = undefined`;
  normal callback results become the next accumulator, and arbitrary
  accumulator values, callback throws, and cross-realm object/error identity
  are preserved.

  The independently audited atomic candidate contains 105 paths / 209
  variants, all of which pass in pinned QuickJS. The single
  `test/staging/sm/TypedArray/reduce-and-reduceRight.js` path is deferred as
  `external:cross-realm`, leaving one path / one variant explicit. The other
  104 paths / 208 variants join the cumulative 1,533-path / 3,027-variant
  gate. Oxide and pinned QuickJS both pass 3,027/3,027, and the exclusion
  ledger falls to 828 paths.

  The candidate path and variant-key SHA-256 values are
  `f40c52a2edb4635d7ca1ec1a2b0abfa4c978c51a73ae567b8efffd8ab5d87ad5`
  and
  `6cc0b62d9fe01cdaacf629a3152ca09b975ada81b4169bad7ffb05714662fe72`.
  The deferred path/key hashes are
  `b99151319be2a66b2d78111bff0ea5e73a308313670a1b4e9488a3afefd6f909`
  and
  `97e3f4dbb189808dc1dd6cb9f8be100c74edbbb333e4c890c165cb7409fdf6cb`;
  the promoted path/key hashes are
  `79f2ce5172ba5afc48a87a3417ce99010762ba9de2cc3c49dd4db7696d6ba7b6`
  and
  `79522bed3692d0c21ac44370796b6c37861dca2fab511d38d8872605e78d9fff`.
  The unchanged scoped profile, cumulative manifest, cumulative variant-key
  stream, exclusion path stream, and exclusion-ledger file SHA-256 values are
  `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`,
  `b12b213d5b0d279bf3fdb328cba831a404fd0f4bc2bc105b1da6aa077c5508c7`,
  `06eceaa517e89f94217d85698d1618f1f297f9e8789f8bc42d7034753dff1e95`,
  `b5f0caf421df10d9958b1d6de4e8d10462a6e89d51b3492a707c0f5a5a83a2a0`,
  and
  `4c6158d8cdb8fbde441e30f9820403912cbbb6f7b57f2af27b5f6c99bfaecca2`.
  Its canonical scoped TSV/JSONL hashes are
  `089be9fab5e932b0003c99df8d70064591e35abe2f184ce0a01a575f7ee2c5e8`
  and
  `5ff2a426b2df285afa4eda8e9abb62dc192b52621a89f2234de475a242f99392`.

  Broad TypedArray admission remains withheld, and a fresh canonical
  two-worker rerun confirms that the complete vector is byte-identical to
  R3au at 51,908/102,037 with no transition. Its full TSV/JSONL hashes remain
  `3e5f9fd57b7a19a51843db7585e2b4aebed0fc1b93b75856f482dec962805fe3`
  and
  `f75fd46059efcaade454d125b7643eb7a067b856f30570396663cf472443da37`.

  R3aw publishes `%TypedArray%.prototype.map` and `filter` through the
  existing callback dispatch and a new isolated species-construction seam.
  Both methods validate and snapshot the source range before traversal, then
  keep per-index reads live without generic property lookup. `map` constructs
  its species target before callbacks and converts/writes every callback
  result immediately. `filter` creates a hidden ordinary Array in the method
  realm before callbacks, defers constructor/`@@species` access until they
  finish, and invokes the result's observable public `.set` even for an empty
  selection. Cross-content custom species therefore follow pinned QuickJS's
  write-time conversion behavior rather than an up-front type-family check.

  The independently audited atomic candidate contains 175 paths / 349
  variants, all passing in pinned QuickJS. The single raw
  `test/staging/sm/TypedArray/map-and-filter.js` path remains deferred as
  `external:cross-realm`; its SpiderMonkey shell also requires WeakMap. The
  other 174 paths / 348 variants join the cumulative 1,707-path /
  3,375-variant gate. Oxide and pinned QuickJS both pass 3,375/3,375, and the
  exclusion ledger falls to 654 paths.

  The candidate path/key hashes are
  `2a4d0d92c7a4b3aec6e559770bd3baa5780b2c3780f408333526619dfbfef9fc`
  and
  `9e51d82281ea14f0568b2116054927aca5187708584e68b8cf551426f7529743`.
  The deferred path/key hashes are
  `198ede24f4c8a6e1dbb4135a14906c9f8a513178a42f23545711651eeaf26e31`
  and
  `c7140d02e8e9d00feedd33ff35c98afa0a1bf365db3dd6ede640f1a8b34c6bd3`;
  the promoted path/key hashes are
  `57a0d825fa96ae56a44dd64be290d6368838d90fcd5cdd739c9735573b8d2a02`
  and
  `b92f4b302934a05ca68f39bde019ef71f2353a664f3e304f2092ccf1eb8cf78b`.

  The scoped profile remains unchanged. Its hash, followed by the current
  cumulative manifest, cumulative variant-key stream, exclusion path stream,
  and exclusion-ledger file hashes, is
  `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`,
  `e6a3af181bf643b70558661802544681ac92356f06c4c27c9b1504b31379b42f`,
  `6bf48fc08165d42f32ff8ed7cf08ad94249b23daaf111cc3700df248c667b075`,
  `b2406a45aab98366342205bf4fb5149091b802500dc09b5a6afb8a1ef784c774`,
  and
  `1c3d6f79c99f423c77c11256d65993143b4fced944f700f64b16975ffb730298`.
  Its canonical scoped TSV/JSONL hashes are
  `05080ac47b8b5be9cc0d8ab70ed7f2233c843c54e42bac54ea8eb7f92a7d206c`
  and
  `439fdf6994613b1f945e7bbd5a02ccd9326dd474c28fa54db45e82d5e208322d`.

  Broad TypedArray admission remains withheld. The exact full join retains all
  102,037 keys and every previous pass: only the sloppy/strict
  `filter-species.js` and `map-species.js` rows move from `fail-runtime` to
  pass, with zero other outcome or detail drift. The complete vector reaches
  51,912 passes and 430 runtime failures; full TSV/JSONL hashes are
  `432394a9db53afd584a532b969382af167f0b17e42f77c8effd930a50389dfeb`
  and
  `d4a7540e05ba0cbcea9b7d94a8c2a6c7c7dea51613b7dcafd90c71e0983ba356`.

  R3ax publishes `%TypedArray%.prototype.slice` and `subarray` through a
  separate copying/view module and extends the shared species seam without
  growing `runtime.rs` or `heap.rs`. `slice` initially validates and snapshots
  the receiver, resolves species after bound coercion, and only revalidates the
  source and target when the original copy count is nonzero. Same-class copies
  preserve raw bits; overlapping custom-species views use QuickJS's mandated
  forward byte-copy semantics rather than memmove. Cross-class copies stay
  live and convert element by element. `subarray` intentionally performs only
  a brand check before coercion, retains the durable raw byte offset of an
  OOB/detached source, and passes either `(buffer, offset)` for an automatic
  length-tracking result or `(buffer, offset, count)` for a fixed result.
  Default species uses the method realm's intrinsic prototype directly;
  custom species may return any live TypedArray and receives no minimum-length
  check.

  The atomic candidate contains 178 paths / 356 variants and passes 356/356 in
  pinned QuickJS. Five SpiderMonkey staging paths / ten variants remain
  explicit: three require `createRealm` plus the shell WeakMap, and two require
  WeakMap. The other 173 paths / 346 variants join the cumulative 1,880-path /
  3,721-variant gate. Oxide and pinned QuickJS both pass 3,721/3,721, while the
  exclusion ledger falls to 481 paths.

  Candidate path/key, deferred path/key, and promoted path/key SHA-256 pairs
  are respectively
  `b47079faf02e6e29ab9b1d1da45d35d79f30f1498fff96ea47c3d0fdf4057417` /
  `d149931f862e672317077644ffae6ccc6e319442a97dbb2a951bb1cdaeed8769`,
  `9f1d0a737704df4c1503cecd69ec953faae2496fa6da4bff07d36b35b377c328` /
  `c991213141a15cd3e647dd9b1c40553c5dc0a709f5ebfbd10e30769683e7eb37`,
  and
  `a6f25c6d1af227a6f656284a2f3c833e4320caea80e7029fc376eb066e01584e` /
  `103222ebda62afb2a76d6b9efc6fefa0c086707509607f58a24b6a73a5f1cb1b`.
  The unchanged scoped profile, cumulative manifest, cumulative variant-key
  stream, exclusion path stream, and exclusion-ledger file hashes are
  `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`,
  `3894d40cf21ca00f0b641b729c7562c65c5cb41d31bb4616b6d1ca8c3871b092`,
  `ba80d9ddfb13f4c8ff20098b267b592a4c0682a806f0b9ce3633f7f61a8c05d4`,
  `16ccf5fac0c47daa0626d26e25aa3d49e305e193f80e8148448d9d444addcf27`,
  and
  `11616f23d68983bb517dff1d4563f060d0ae3955941e66a681d0a9ab4be5b565`.
  Canonical scoped TSV/JSONL hashes are
  `88d9061e2d31b2869f7d71b0cda7a0cd059c8d7cf346de967eeabc572fe24aff`
  and
  `e36ef63eac28058534553577595b947a044ebd61d177e4a1704eab415bcb3ba0`.

  Broad TypedArray admission remains withheld. Two independent canonical full
  runs are byte-identical and retain every one of 102,037 keys and every
  previous pass. Ten sloppy/strict staging rows move from `fail-runtime` to
  pass: the three frozen slice/subarray paths plus two untagged subarray
  consumers. No other outcome or detail moves. The vector reaches 51,922
  passes and 420 runtime failures; full TSV/JSONL hashes are
  `796783147bae745b1cbb21eb2cf211feefcb98e80008f760eed8f18eb84f7641`
  and
  `e912ed7dc3f9a9f0141f9c96168fb8bb5e4be4661d6d47030295427a21baf4aa`.

  R3ay publishes `%TypedArray%.prototype.with` and `toReversed` as
  non-species change-by-copy methods. `with` snapshots the old length, derives
  a relative index from that snapshot, performs index conversion and then the
  replacement's number-hint `ToPrimitive` before revalidating the live view. A
  resizable-buffer shrink checks the index against the current length but
  retains the old result length: missing numeric tail elements are converted
  from `undefined`, while a BigInt tail throws. `toReversed` clones the
  same-class raw element words before reversing word-sized slots, preserving
  NaN payloads and negative zero. Both methods ignore the source's public
  constructor and species and allocate with the builtin defining realm's
  default TypedArray prototype.

  The shared constructor-clone helper now owns the common QuickJS
  `js_typed_array_constructor_ta` validation, allocation, raw-word copy, and
  element-conversion path instead of duplicating it in each copying method.
  Adjacent `at`, `reverse`, and same-class TypedArray-constructor OOB failures
  also use the canonical pinned QuickJS error text, so the differential
  contract covers message text as well as exception type and ordering.

  The dependency-clean atomic candidate is 34 paths / 68 variants with no
  deferred path; Oxide and pinned QuickJS pass every variant. The cumulative
  gate therefore reaches 1,914 paths / 3,789 variants, and the exclusion ledger
  falls to 447 paths. Candidate and promoted path/key SHA-256 pairs are both
  `e212ba0d3d9c819403d3d226f23a735ff2bb9b746618fff779e2654a39f5fddb` /
  `6d341ea9896a878f9beea36e477e96227642812a1cded595620a6de0f76e7723`;
  the empty deferred path and key streams both hash to
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

  The scoped profile, cumulative manifest, cumulative variant-key stream,
  exclusion path stream, and exclusion-ledger file hashes are
  `07837fd2bdb1cf5f300163c483b611d0862955c7976de5f385faebe1b4dd7ac1`,
  `1237074662d16674a5ea23f6a2bed26ee3126358f7fb80949846f2329f2ce318`,
  `c6d46821eae8f1affec571a38c5dfd074aa1774ef36df2a78e47db554e151e02`,
  `d8842f1aeedb8d42ce551c72c15a433c6d776c44f2abe39e789dfea82b24c348`,
  and
  `aaca7878d12694635eb5f65d9ae53f9000aafba5e647eb88365663683fdc07fc`.
  Canonical scoped TSV/JSONL hashes are
  `19ab4f7385457ea72e47c7e3b5ba7031d0a0cdffbbd2db8825d1685230b92ce1`
  and
  `09d1a226a84e10f39cc5228037eddb5a1af5c2eee64664b45bf9f2407e27dd96`.

  Broad TypedArray admission remains withheld. Two independent canonical full
  runs are byte-identical and retain all 102,037 keys and every previous pass.
  Only the sloppy and strict `test/staging/sm/TypedArray/with.js` rows move from
  `fail-runtime` to pass; there are no other outcome or detail changes.
  Replacing those two rows with their R3ax records reconstructs the R3ax
  canonical TSV/JSONL hashes exactly. The vector reaches 51,924 passes and 418
  runtime failures while runnable remains 52,468; full TSV/JSONL hashes are
  `73141c5f26f9e3f132b0046c1066a7d5965497c27754e1b4ec89b5649e8ba7a9`
  and
  `b69db1a2c29dfdb7e0196fc2e452591a1d25316fd9ec449ef24cdbdd7d2f5481`.

  R3az publishes `%TypedArray%.prototype.join` and `toLocaleString` through a
  dedicated stringification kernel while retaining the inherited `toString`
  alias. Both methods brand-check and validate the source before snapshotting
  its old length. An explicit `join` separator is converted before the live
  length is re-read; shrink or detach clips element reads but preserves the
  old-length separator shape, growth is ignored, and string-limit overflow
  stops before another element conversion. `toLocaleString` ignores its
  arguments, uses a comma separator, and invokes each live primitive
  element's builtin defining-realm `toLocaleString` with zero arguments before
  stringifying the result. These paths follow the pinned QuickJS TypedArray
  kernel rather than routing through generic Array traversal.

  The exact atomic candidate is 88 paths / 175 variants. Five paths / nine
  variants stay deferred: two cross-realm paths / three variants require
  `$262.createRealm` plus the SpiderMonkey WeakMap shell, and three paths / six
  variants require that WeakMap shell alone. The remaining 83 paths / 166
  variants join the cumulative 1,997-path / 3,955-variant gate, which Oxide and
  pinned QuickJS both pass completely; the exclusion ledger falls to 364
  paths.

  Candidate path/key, deferred path/key, and promoted path/key SHA-256 pairs
  are respectively
  `d968b61ff553acb2654f2904a9afff46660f43d6848ad7496ff28f18a81b8d4b` /
  `81131955a7d4ef4b2358965cd0691498bb78abfac7c48d0f60b8aafcdbbe81f1`,
  `0254c5edb9969e43038d03dd42f9d43fd29c10c647673cd63cb4230bc8c53151` /
  `092d6f18a34c2dd23f7add4d9a73a5c1c14e63f99c6fd91f70c8a2c050edc44c`,
  and
  `ae64162fb7742828d9dc45d5f54e4666887c4ac95499bbfbe8622ae6fc875b89` /
  `0fe599bb568d384f84657000208d47df7b7ffa1d3133b6d2795abafa06bf00f6`.
  The scoped profile, cumulative manifest, cumulative variant-key stream,
  exclusion path stream, and exclusion-ledger file hashes are
  `173f0f6f33966a97c8ef65d55f261e5cf1b9c2ee68d1acf2adca92a48d16eb4b`,
  `00f63843eda645f8701e678663f505ae3004574110f3ccb5fb78e12a94ee98cb`,
  `b6b16404066ac2e03815b38fd55bbc62d70066ee50e5696687e15a3e8d4a0bfe`,
  `e11790d0921680b55ba8f5c47a1bd4d7f1254107ea2c05c5f75f51319b578c17`,
  and
  `432e55cc4bccbdad68f90b7556f89aaf704141e0f4b64242964fcd0ad2853575`.
  Canonical scoped TSV/JSONL hashes are
  `623401f1ee46bb26a6313d26dd71a408e19f58a64bef95bb537428ad19f018bd`
  and
  `4983527238f6d436e8c01d40ca707514f3847a879b4c2fcda64aedaa1f986552`.

  Broad TypedArray admission remains withheld. Two independent canonical full
  runs are byte-identical and retain all 102,037 keys and every previous pass.
  Only the sloppy and strict
  `test/staging/sm/TypedArray/detached-array-buffer-checks.js` rows move from
  `fail-runtime` to pass; there is no other outcome or detail movement. The
  vector reaches 51,926 passes and 416 runtime failures while runnable remains
  52,468; full TSV/JSONL hashes are
  `bd1119fe3ea8e4eaaad2e21bf3d0991b58200bacef91e695c2c2a4c11e6538c3`
  and
  `d78dfbd84ebab70441362d6bd535fab9fcfc433419b09fe8668309a749e7c759`.

  R3ba publishes `%TypedArray%.prototype.sort` and `toSorted` through the
  pinned QuickJS `rqsort` choreography shared with Array, while keeping
  TypedArray storage and comparison rules in a dedicated kernel. Default
  comparison sorts raw backing words in place with O(1) auxiliary storage,
  preserving numeric order, NaN placement and signed-zero order without
  converting elements. A custom comparator instead snapshots exact raw bytes
  plus a `u32` original-index vector, decodes callback arguments from that
  immutable snapshot, uses original position as the stable tie-break, and
  writes raw words back only after successful comparison.

  `sort` validates its comparator before branding and validating the receiver;
  `toSorted` brands and copies first, then validates the comparator. The latter
  allocates a fixed same-class result in the builtin's defining realm without
  consulting `constructor` or `@@species`. Comparator-driven detach or final
  out-of-bounds state suppresses writeback, shrink clips the old snapshot,
  growth does not add elements, and callback throws preserve identity. Array
  and TypedArray sorting share one catchable 16-entry native recursion family
  so alternating comparator reentry cannot bypass the host-stack budget.

  The exact atomic candidate is 64 paths / 128 variants. Six staging paths /
  12 variants remain deferred: four cross-realm paths and two SpiderMonkey
  WeakMap-shell paths. The remaining 58 paths / 116 variants join the
  cumulative 2,055-path / 4,071-variant gate; Oxide and pinned QuickJS both
  pass all 4,071 variants, and the exclusion ledger falls to 306 paths.

  Candidate path/key, deferred path/key, and promoted path/key SHA-256 pairs
  are respectively
  `d06f1655781895a7f77a5ae378e25920e4cf62c87134a1cabaaa0418bfb8a0b8` /
  `53e35176074fdfdd0c414d30b9365995b0d420f43a2e45c420955cc0fc1d6de9`,
  `0067268a56e709b6be94b51b1a7472b961a27f9a99e623a6cce6d04ed4cf1b96` /
  `f242add5304bef7ba11b82181cc1646b5a1ea970f06ee38d857d4c65f144ecfd`,
  and
  `1efa5ed5b57d0638963f183b0294e5dc90b711b754c63aa50b79cd34f3e0d3d4` /
  `b76f083344a23bdb330cdec16aa22f07175fb151f374858a77bbf3cc48e624c1`.
  The scoped profile, cumulative manifest, cumulative variant-key stream,
  exclusion path stream, and exclusion-ledger file hashes are
  `8261eff7f79ebc2b724cf42c0853d8f74336ac23eccfa862172bcbca2f918a3e`,
  `fa6f12f165793a00c4fc987ebaa043e9090c694dc2d77fc3b7ba670a3639e0cd`,
  `6ecf7cb35ecb89cb831b43db6d778f4f2b8a4432c83c8d7a08d396c36fb7e65b`,
  `8bb730391734446ade26ae9835772a7bd4493d4cb6fa9f97a8b6a2e5dbd30000`,
  and
  `fad925fb491f4a1c5e55ab1ca54ce6dd46e189e655c5f2d7c145981d1d2d1178`.
  Canonical scoped TSV/JSONL hashes are
  `5db3782454d4556687a918c946a676b8708b5e4f0be7e9edd84e25700a258629`
  and
  `8948ee41244d744c6099868b86bbca8dfc88d7cea9865d5e58b6eb86492cb8f9`.

  Broad TypedArray admission remained withheld at R3ba. That milestone's
  canonical full measurement retains all 102,037 keys and the 52,468 runnable
  count while moving 14 runtime failures to pass; every other summary category is
  unchanged. The vector reaches 51,940 passes and 402 runtime failures. Two
  independent formal two-worker repeats reproduce the measured full
  TSV/JSONL hashes:
  `f9944fe74a9eee0330a9f4681e3064cba5fc70e00b4fc7eef73fcbce6f709b07`
  and
  `8cc3f8420e290d3094a21bee23a10e26c2cb2e860228d3f98a2bda80c5eb1390`.
  At the R3ba landing, the next milestone was a residual TypedArray audit
  rather than one inferred from the sorting result.

  R3bb authenticates `%TypedArray%.prototype.entries` and `keys` through the
  existing shared Array-iterator implementation. No production code changes:
  its per-`next` length recheck, integer-indexed element read, detach/OOB
  behavior, and iterator completion match differential observations; the
  manual-next/outer-operation realm split is separately source-audited against
  pinned QuickJS. Three frozen/self-check/differential observation tests cover
  all 12 concrete TypedArray classes, resizable-buffer shrink/grow, detach, and
  transient-OOB recovery. A fourth Rust cross-realm structural regression
  locks the audited realm split.

  The exact candidate is 46 paths / 92 variants. Three paths / six variants
  remain deferred: the SpiderMonkey staging `entries.js` and `keys.js` paths
  require both the unavailable `createRealm` and WeakMap shell, while
  `prototype-constructor-identity.js` required WeakMap and the then-missing
  Uint8Array codec surface at R3bb. The other 43 paths / 86 variants—42
  `entries`/`keys` paths / 84 variants plus the two-variant
  `detached-array-buffer-checks.js` canary—join the cumulative 2,098-path /
  4,157-variant gate. Oxide and pinned QuickJS both pass all 4,157 variants,
  and the exclusion ledger falls to 263 paths. The checksum-bound candidate,
  deferral, promotion, and cumulative evidence is recorded in
  `docs/test262.md`.

  The complete vector is unchanged by construction: the global capability
  profile does not change, all 84 newly authenticated `entries`/`keys` rows
  remain classified `unsupported-feature`, and the two detached-buffer rows
  already pass in the R3ba baseline. It therefore remains at 51,940/102,037
  with canonical TSV/JSONL hashes
  `f9944fe74a9eee0330a9f4681e3064cba5fc70e00b4fc7eef73fcbce6f709b07`
  and
  `8cc3f8420e290d3094a21bee23a10e26c2cb2e860228d3f98a2bda80c5eb1390`.
  One non-blocking resource-parity caveat remains: Rust and QuickJS allocator
  bookkeeping have different OOM topology, and the weighted native-stack
  budget is still an approximation, so this admission does not claim identical
  injected-OOM or extreme native-interleave failure thresholds. At the R3bb
  landing, the next audited slice was static TypedArray `of`.

  R3bc authenticates the inherited static `%TypedArray%.of` implementation.
  The existing algorithm constructs through the receiver with one length
  argument and the same `newTarget`, validates the returned live TypedArray and
  minimum length, then converts and writes source arguments from left to right.
  The focused vectors cover descriptors and call-only behavior, all 12 concrete
  classes, Number/BigInt conversion, custom/bound/Proxy constructors, returned
  class mismatches, abrupt conversion and partial writes, RAB shrink/grow,
  detach, and zero/512-argument calls.

  One real diagnostic mismatch was fixed. Static `from` and `of` now share a
  narrow direct-constructor seam: a primitive receiver reaches QuickJS's
  defining-realm `TypeError: not a function`, while an object that is not a
  constructor continues through the ordinary constructor check and reports
  `not a constructor`. Static `from` retains its observable order—map-function
  validation first, then iterator collection or array-like length access, and
  receiver validation only when creating the target. TypedArray species
  construction deliberately does not use this seam, so a primitive
  `@@species` value keeps its distinct `not a constructor` diagnostic.

  `tests/oracle_typed_array_of.rs` has four test entry points. Only the first
  three form the pinned-QuickJS observation layer: frozen observations, oracle
  self-check, and direct Oxide/QuickJS differential. The fourth entry is a
  Rust-only cross-realm structural test for result prototypes, defining-realm
  native errors, and caller-thrown identity; it does not execute QuickJS and is
  not claimed as a differential.

  The atomic candidate is 35 paths / 70 variants. Only
  `test/staging/sm/TypedArray/of.js` remains deferred—one path / two variants—
  because it requires both `$262.createRealm` and the SpiderMonkey
  TypedArray-shell WeakMap. The other 34 paths / 68 variants join the cumulative
  2,132-path / 4,225-variant scoped gate. Oxide and pinned QuickJS both pass all
  4,225 admitted variants, and the exclusion ledger falls to 229 paths.

  Candidate path/key, deferred path/key, and promoted path/key SHA-256 pairs
  are respectively
  `6fdec16ab63ca0b1081a90f7a5f12fa6c87b6c73fdb209079d24bf793d2787b8` /
  `3bfcf9a16f2c28c819d121a819f7c52882e34fb3a3443ebb6c66db0bdbcc25a7`,
  `2b66ebd26cc79b9df0d5e5771e665d164311633010ea66eb33a22e85d6d62a0e` /
  `07a640bcebe1fc380bde8bd0ab1a3b80779d4e45b085a744018a50858c016140`,
  and
  `01095b2e0348fb1328026684c7422975cf8396a08fa73719955c9350ee15f13f` /
  `8318904a86586b2bc771200348972ffd59c6f84b61219d84b262668517c363df`.
  The scoped profile, cumulative manifest, cumulative variant-key stream,
  exclusion path stream, and exclusion-ledger file hashes are
  `c7118e34b64929bd57678ac490fb5793a3e6974fb4272e09633614d424fe4ef7`,
  `3334625f2df7a60c7541884f14f5b001e2f0eadbafdb85529eb5018b9eb0f4d8`,
  `1fb72c0d146a365b8ff7eee5eeca291d0aa1af97b786f02f05011a89cd694ec7`,
  `db842baa3b677f2e2312540bfb279e72fb56e6acaa390bd5ee602e0fc40bd371`,
  and
  `be473162b0c73865415bf26bcfab36041139bb0f1684b8ecea5fe2065b995267`.
  Canonical scoped TSV/JSONL hashes are
  `a0f5531d24e57b3da8af70ba865b2aa9764f64973489da07d812a80d92dbecab`
  and
  `3f4f16ca175e057f063cfb4d917bdadd31c66e421edb60c97e7900cbca41cf50`.

  Broad TypedArray admission remains withheld. The global profile is unchanged,
  and an exact full-row audit shows that all 68 newly authenticated rows remain
  `unsupported-feature`; the canonical complete vector therefore stays at
  51,940/102,037 with TSV/JSONL hashes
  `f9944fe74a9eee0330a9f4681e3064cba5fc70e00b4fc7eef73fcbce6f709b07`
  and
  `8cc3f8420e290d3094a21bee23a10e26c2cb2e860228d3f98a2bda80c5eb1390`.
  Resource parity remains deliberately narrower. The Oxide native bridge keeps
  an extra O(argc) cloned argument vector where QuickJS reuses VM `argv`;
  extreme Rust allocation failure can abort instead of becoming QuickJS's
  catchable `InternalError`. Direct TypedArray allocation also has a different
  object/backing-store topology, and BigInt writes currently use
  `as_int_n(64)` temporaries where QuickJS reads the low limb directly. The
  safe-large oracle stops at 512 arguments, so this milestone does not claim
  identical injected-OOM ordering or thresholds.
  At the R3bc landing, the next static `from` inventory contained 90 paths /
  175 variants in total: 81 paths / 158 variants were dependency-clean
  promotion candidates and nine paths / 17 variants were already attributed
  deferrals. This total/promoted/deferred terminology replaces the ambiguous
  earlier shorthand that called only the 81 promoted paths the “candidate”.

  R3bd authenticates inherited static `%TypedArray%.from` against pinned
  QuickJS. The implementation validates a supplied map function before any
  source access, selects and completely materializes an iterator before target
  construction, or reads an array-like length before construction, then maps,
  converts, and writes from left to right. The target is created directly from
  the receiver rather than through species and is validated as a live,
  sufficiently large TypedArray; conversion follows the actual returned
  element class, including Number/BigInt mismatches, partial writes, detach,
  and resizable-buffer shrink/grow behavior.

  Two production details were tightened. `undefined` and null sources now
  reproduce QuickJS's exact defining-realm
  `cannot read property 'Symbol.iterator' of ...` TypeErrors, while an invalid
  map function still wins before that source diagnostic. Iterable
  materialization keeps its `Vec<Value>` alive and traverses
  `iter().cloned()`, retaining every original yielded object through the whole
  map/write phase just as QuickJS's hidden Array does.

  `tests/oracle_typed_array_from.rs` is a 914-line focused oracle with eight
  frozen vectors and four Rust test entry points. The first three are the
  QuickJS observation, oracle self-check, and direct differential layers. The
  fourth is deliberately Rust-only cross-realm structure: it locks result and
  native-error realm ownership, sloppy versus strict mapper `this`, and abrupt
  value identity without claiming to execute QuickJS.

  The exact atomic universe is 90 paths / 175 variants. Pinned QuickJS passes
  all of it. Eighty-one paths / 158 variants are promoted; nine paths / 17
  variants remain deferred: seven SpiderMonkey staging paths depend on the
  absent WeakMap shell, `from_realms.js` additionally requires
  `$262.createRealm`, and the Annex B path requires IsHTMLDDA. Total candidate
  path/key, promoted path/key, and deferred path/key SHA-256 pairs are
  respectively
  `87e7cfd69fbac9265f7e4a28ceaea8f21f053b7a587a95494becc7bbab61b20c` /
  `041fc07db938e2bf21fd1135fdbb3be648e2e5f3bdbf5688dfdf78784ed505a4`,
  `a75d6ebea395327340d498c6f4d5e2b2c4224c039f6c1a58e42b19d070e94e41` /
  `5ea8a30f1578a6160441c068c91384ea635e179a90c6804af23730cfec7f6f34`,
  and
  `7e466133fdeb876268cf10e629701daa332922d484d16ad76b58679aee3e47b6` /
  `df334b586f8ab8494ab8ec1d9a06d4492ae76b0fe0d73479637001f18ab3dd24`.

  The cumulative scoped gate reaches 2,213 paths / 4,383 variants, all passing
  in Oxide and pinned QuickJS. Its profile, manifest, and variant-key SHA-256
  values are
  `dd106c074751866ce667352d3449cc0ec7d9b9072034a4f0a97050da7b7bad13`,
  `d71be16dfcd42b58e3371c47d35d8f6cc9fbe29a11135ebd39ea447cb84d0c56`,
  and
  `ac56a6047ecb71616e098b5cb6a0c449d11af21141f8f18af5ebe4dccefb9a84`.
  It admits 27 feature tags with hash
  `de5b9c5c6a66566a6b1481fc0b014a6ef00a95ebecc90c37da4508aa85a8d830`
  and 11 includes with hash
  `b1b60b5e1f7635615ff31eb139d1803608e5743c5f46ca53fadc3797e0abe012`.
  The remaining 148-path exclusion stream and complete ledger hash to
  `0d425a326fc950257410849ada4c2435b410e84f4c9651f9393c39f6d5c3032a`
  and
  `4c79c3c86364a5c0aa6d2ea5bf3cba6da47261d0b4847fbfeaa5cd368749b783`;
  their reasons are 71 SharedArrayBuffer, 54 cross-realm, 21 WeakMap, one
  IsHTMLDDA, and one Math path. Canonical scoped TSV/JSONL hashes are
  `de22c434d3ac28ed823a6c20c1bbc01a7e44e43e86e1a1b368696196b2399c1b`
  and
  `6f1904f5001deb1f96cd06d697def75999991350e582c3b69486246b1a68b460`.

  Broad TypedArray admission remains withheld. An exact read-only join of the
  158 promoted rows against the conservative full vector finds four existing
  passes and 154 `unsupported-feature` rows: 142 need only TypedArray, six
  additionally need `Array.prototype.values`, and six additionally need
  resizable ArrayBuffer support. The normalized row stream hashes to
  `fecefca50dcb3d97f321ba81fe8af1490bd74520b3d7327be142a882085023b7`.
  The complete measurement and canonical artifacts therefore remain
  51,940/102,037 with TSV/JSONL hashes
  `f9944fe74a9eee0330a9f4681e3064cba5fc70e00b4fc7eef73fcbce6f709b07`
  and
  `8cc3f8420e290d3094a21bee23a10e26c2cb2e860228d3f98a2bda80c5eb1390`.

  One resource-parity caveat remains explicit. Retained value lifetime now
  matches, but QuickJS allocates a hidden realm-local Array while Oxide stores
  the materialized values in a Rust Vec, so allocation, GC pressure, and
  injected-OOM topology are not certified as identical. At the R3bd landing,
  the next audit was broad TypedArray global admission: enabling only
  `TypedArray` exposed 3,686 variants, 3,606 already covered by the scoped
  certification, leaving an 80-variant spillover across 41 paths for review.

  R3be admits that single global `TypedArray` tag, bringing the checksum-pinned
  global profile to 80 tags. Its frozen activation manifest contains 1,865
  paths / 3,686 variants: 1,824 paths / 3,606 variants were already
  authenticated, and the disjoint spillover adds 41 paths / 80 variants. The
  activation manifest and key stream hash to
  `44a9b901eb59f9dc41dde71e0595d2777f52814a864632e7e27bdd739654bdee`
  and
  `68b01ca00423a3e62a090ee8cac24d54b5866276de306b0c846e74d3663218e5`;
  its all-pass TSV/JSONL hashes are
  `e663c9b957e7e061573cc42e092ddd7b06a4508cd2e67ba74919ad243239ab54`
  and
  `9db88feb1d2d79dd3f0abce8a818c1bffff67d79f1a77f671c3a5fdb8a1078fc`.

  The audited TypedArray candidate is now 2,402 paths / 4,749 variants, and the
  cumulative scoped gate admits and passes 2,254 paths / 4,463 variants in
  Oxide and pinned QuickJS. Its manifest, key stream, and TSV/JSONL hashes are
  `91ac9a132c8099ecd15d3cfcfe160b21a1f7e9a083a5210a33406606270ad378`,
  `e8e3c0d8f19343bbf0160c5af3239caa98fb7e01d006ff6b53f0d946a500e7cc`,
  `388d8f32ef0d7d0a8f2c86ac0931178d2d850335b80cf13fe81888930be5f38c`,
  and
  `e32b0abdcab0409491132690a4b22441791016ac57c83c1bcbdfd26c0a0b3c9d`.
  A separate 471-path / 938-variant reason-only ledger remains
  `unsupported-feature`: removing `TypedArray` from its diagnostic exposes
  another still-unsupported dependency rather than admitting execution.

  The checked-in 4,624-data-row R3bd-to-R3be transition receipt,
  `tests/test262-typed-array-global-r3bd-r3be-transitions.tsv`, partitions
  exactly into 3,686 `unsupported-feature -> pass` transitions and 938
  `unsupported-feature -> unsupported-feature` reason-only changes, with zero
  other row changes or previous-pass regressions. Its complete-file and
  header-free data-row SHA-256 values are
  `851ef0961a28532081f7b9dc281c305ea8839dd3b8ceed750d182da90b69eafd`
  and
  `26babcba92c23bb699f8fd3a2db7cce376fa868f5b3ca4081abc4148a90a4a57`.
  The complete vector reaches 55,626/102,037 passes with 56,154 runnable
  variants; canonical full TSV/JSONL hashes are
  `bdeb287ea6f74baefa0eb034773aa57f7c87f9ecaa6d2af20f27a6ea94b53693`
  and
  `916fbebcb964be779138ca6ad588d14b9cf3e55c0f22b4aaeb474739bdb74ece`.
  R3be changes only profiles, manifests, gates, baselines, and focused tests:
  no production runtime code changes, `runtime.rs` remains 9,950 lines, and
  `heap.rs` remains 23,026 lines. Four focused `with` tests include pinned
  QuickJS differentials for the `with`-statement spillover. At R3be,
  Uint8Array codecs, modules, SharedArrayBuffer/Atomics, and broad built-ins
  remained the next frontiers; R3br/R3bs later close the codec item.

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
  including line and column. Module contextual words stay with the
  unimplemented module surface.
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
  pinned QuickJS, including cross-realm construction/results/errors. At R1x the
  frozen 225-path/450-variant Test262 RegExp-core vector had 448 passes after
  executing its five eval consumers; R3s resolves both remaining typed
  legacy-control frontiers, bringing that older gate to 450/450.
  `RegExp.escape` is now published; Unicode-sets (`v`) grammar and cross-realm
  Test262 host execution remain explicit boundaries rather than stubs. The
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
  252 variants; two require the IsHTMLDDA host hook. R1p resolves the two Annex
  B `\k` separator variants,
  R1x executes the two eval consumers, R2c resolves the Arrow consumers, and
  R2f resolves the six concise-method consumers. Its
  current TSV/JSONL SHA-256 values are
  `13f8c26ce2c9cd93904ce420cc00010e06e60f1eedccd7e22cc2f1e98fdb1303`
  and
  `eb88da8a2773b80e436c9311ba39f0868c623555e6679aeff4761ef631e5f26d`.

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
  typed frontier outcomes. Later RegExp slices and R1x lead to the 448-pass
  R3r vector; R3s completes the current 450/450 vector summarized above.

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
  passes. The current vector admits and passes 362 variants; eight retain
  independently undeclared features, two require create-realm, and four
  require IsHTMLDDA. The current focused TSV/JSONL SHA-256 values are
  `0dccee6d3228b5c665a9f2c42890e46345d865bb0905020224e04e1b35589a94`
  and
  `facaadcafe19ae3444b8aa0ae2b7467519037f9c4ee4dc0bfa6f1bd07e8c98a2`.
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
  match/reference cases initially reached the lexical-destructuring parser
  frontier. Later object binding support resolves those final four, so the
  current gate passes all 98 variants. Its TSV/JSONL SHA-256 values are
  `fc91f2bc073844d86dc5b4c4b739da40e41a21267fde6f61d8fc6792d2b6c9a4`
  and
  `7ab11b9287f97ea7faf73331501b7fff2624a7892467b8f68879da2e155a1d8c`.

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
  Later object-binding and rest-parameter milestones move the frozen gate to
  194 passes; R3f derived construction resolves its remaining four class
  frontiers, so the current gate has 198 passes, two feature-gated variants,
  and two at an unrelated runtime frontier.
  Current TSV/JSONL hashes are
  `37d54ae152bd48b0fc35625d4776e082c3baa2b4024382bd274f0633ea2323e3`
  and
  `b96318614cf6bd6a9d0d8b1c360cccd0a2f12131f59988baba24002201aff846`.

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
  Later binding and rest-parameter milestones move the current gate to 58
  passes; two variants remain feature-gated and two require
  `$262.createRealm`. Current TSV/JSONL hashes are
  `da103588eaf15c8864b2aff5966f5e7a60fe533ca85be14607956695cf193b1d`
  and
  `ec5f84df5135174cbe78b91218a13879a53e758d97c583120c32b7a8026b5f7a`.

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

  Advanced grammar still fails closed: Unicode set/string properties, set
  operations, and unported Annex-B control escapes return typed unsupported
  errors. R3ct opens only the basic `d`, `D`, `s`, `S`, `w`, and `W`
  CharacterClassEscape slice in `v` mode; adjacent `v` grammar remains
  fail-closed. Pattern group nesting is temporarily capped at 256 with
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

  Source lexical population now covers simple-name and recursive
  array/object/rest `let` and `const` lists in the direct Program global
  lexical environment plus four
  local authored environments: an ordinary function body (including a normal
  `%Function%` constructor body), every non-empty nested brace block, the one
  CaseBlock scope shared by every clause of a `switch`, and the initializer
  scope of a classic `for (;;)` loop. Block, switch, and classic-for locals also
  work in scripts.
  A simple-name `let` without an initializer performs explicit `undefined`
  initialization, while `const` and binding patterns require an initializer.
  Array patterns accept identifier leaves, empty/elided/trailing elements,
  undefined-only defaults, and terminal rest; anonymous function initializers
  retain contextual NamedEvaluation. Object patterns accept fixed and computed
  String/Symbol keys, defaults, and recursive object/array nesting while object
  rest remains typed unsupported. Registration occurs before each
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

  Script `var` names from simple declarations and recursive array/object/rest
  patterns use the same production global-declaration path
  in Program bodies, blocks,
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
  declarations are covered by a pinned differential matrix; async declarations
  remain an explicit boundary, while R3k adds synchronous generator
  declarations and the ordinary Annex B statement forms are described below.

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
  `for-in`. Identifier-rest lists now use the forced-unmapped path and are
  covered across ordinary functions, synchronous methods, arrows, and
  `%Function%` by R2x. Identifier defaults use the independent Parameter
  Environment and forced-unmapped path in R2y. R2z/R3a add synchronous
  recursive parameter BindingPatterns, including terminal rest patterns and
  standalone parameter expressions; R3b adds direct eval in and below that
  Parameter Environment. Async function forms remain a separate slice; R3k
  later adds synchronous generator forms over the same parameter substrate.

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
  Full/StripSource/StripDebug stacks and parser diagnostics. The companion
  `oracle_catch_destructuring` target locks recursive array/object/rest catch
  patterns, defaults and computed keys, iterator close, abrupt initialization,
  lexical closures, direct eval, and early errors. It also distinguishes the
  private marker on a simple catch identifier from ordinary lexical pattern
  leaves.
  The R2x Rust/compiler and `oracle_rest_parameters` matrix locks the
  identifier-only rest slice: actual trailing-argument collection, including a
  dedicated cross-realm compiler test, formal `length`, raw and unmapped
  `arguments`, entry/hoist order, closures and receiver behavior,
  `call`/`apply`, direct eval, the `Function` constructor, and exact
  diagnostics.
  The R2y `oracle_identifier_default_parameters` matrix then locks child-first
  initializer parsing, Parameter-scope TDZ and closure cells, raw/unmapped
  `arguments`, `length`, NamedEvaluation, body-hoist ordering, `this`/`super`,
  default-plus-rest composition, and the pinned QuickJS-only `2|1` body/raw-
  argument split. That R2y oracle deliberately excluded direct eval; the R3b
  oracle now locks its independent `<var>` / `<arg_var>` Parameter-Environment
  ABI.

  The upstream anchors for the catch/finally slice are `quickjs.c` 21775-21785
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
  in QuickJS 2026-06-04. Async eval declarations, single-statement lexical
  declarations, and the remaining private/static-block/class-element scopes
  remain explicit boundaries rather than falling back to local or ordinary
  global storage.
  R3e adds the distinct outer declaration and immutable inner-name scopes for
  base classes, including TDZ and direct-eval visibility; R3f extends those
  authenticated environments through heritage, derived construction, and
  `super()`.

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
  Script-wide simple-name or recursive array/object/rest `var`, direct Program
  simple-name or recursive array/object/rest `let`/`const`, and
  the body/block/switch/classic-for-head lexical slice above,
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
  function-local `var` declaration path. Its simple-name and recursive
  array/object/rest lexical declarations use the same NoIn
  initializer grammar, conflict registration,
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
  slice covers simple names plus recursive array/object/rest patterns, not
  complete SwitchStatement parity.
  Synchronous `for-of` follows QuickJS `js_parse_for_in_of` for
  `var`/`let`/`const` declarations plus identifier, fixed-member,
  computed-member, and recursive array/object/rest assignment targets.
  The assignment fragment is emitted before the iterable expression and
  skipped on first entry; the head lexical environment is therefore already
  in TDZ while evaluating the iterable. Captured lexical head cells close at
  the pinned per-iteration boundary. Local and labelled continue retain the
  active iterator, while edges crossing an iterator control close it in
  inner-to-outer order and interleave correctly with switch cleanup and
  try/finally subroutines. Recursive array declaration patterns reuse nested
  iterator records with QuickJS close semantics; recursive object declarations
  share their fixed/computed property and exclusion-aware rest lowering, then
  unwind nested arrays in inner-to-outer order. At the synchronous `for-of`
  checkpoint, parameter patterns and `for-await-of` were explicit frontiers;
  R3ak later closed the implemented async-function and async-generator
  `for await` surface.
  The classic
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

  Synchronous `for-in` now uses the same upstream parser path for simple and
  recursive array/object/rest `var`/`let`/`const` declaration heads, plus
  identifier, fixed-member, computed-member, and recursive array/object/rest
  assignment heads. Its
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
  ordinary shadowing, ordering, lexical cells, labels and finally cleanup. The
  VM host outcomes preserve arbitrary JavaScript throws. Proxy enumeration now
  follows the pinned duplicate prototype pre-scan and per-level
  `getPrototypeOf`, `ownKeys`, `getOwnPropertyDescriptor`, and live-`has` trap
  order, including revocation and prototype mutation. `oracle_proxy` locks this
  path against QuickJS 2026-06-04.
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
  conversion, inferred names, and non-constructability. Public synchronous
  generator methods reuse the define-method, HomeObject, and shared parameter
  machinery. Ordinary async object methods now reuse that same publication
  path with Async execution. Public and private ordinary async class methods
  now reuse the same method/Async split through class publication. Async
  generator methods, `yield*` delegation, and `for await` reuse those
  publication paths.
  Synchronous
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
  indirect eval cut that capability off. Base-class methods now reuse this
  HomeObject/SuperProperty path, and R3f extends the authenticated capability
  through heritage, derived construction, and `super()`. R3ad extends that
  HomeObject path through public instance/static async class methods, and R3ae
  composes it with the authenticated private-method callable cell and
  HomeObject-derived brand. Other exotic-source spread variants remain
  explicit frontiers. The pinned
  anchors are
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
  Its runtime entry returns a Completion so Proxy `has` trap throws cross the
  same VM opcode boundary. `instanceof` performs
  the full `JS_IsInstanceOf` sequence: RHS Object validation, observable
  `@@hasInstance` lookup, callable method invocation and ToBoolean, followed by
  the callable OrdinaryHasInstance fallback only for a nullish method. The
  existing standard native method supplies its defining-realm frame, while
  bound functions delegate through the complete path without recursing on the
  Rust host stack. Pinned differentials lock precedence, classic-for NoIn,
  evaluation and key-conversion order, custom/inherited/accessor methods,
  arbitrary throws, exact errors and source sites; host tests additionally
  lock deep bound chains and cross-realm error ownership. Proxy
  `[[HasProperty]]` and `[[GetPrototypeOf]]` are routed through the shared
  completion-aware internal-method seam.
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
  the Rust host stack. Release keeps QuickJS's one-MiB byte budget; debug uses a
  1.25-MiB frame-size calibration so the pinned 20-level acyclic oracle succeeds
  on a two-MiB test thread while a true cycle still throws and the runtime
  recovers. Its current 64-stringification-frame limit can nevertheless reject
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
  The pinned implementation deliberately uses HasProperty followed by a
  conditional Get rather than the specification's unconditional Get; Proxy
  receivers preserve that visible `has` trap. It also inherits
  `js_allocate_fast_array`'s signed 31-bit length ceiling,
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
  and side-effect parity. The pinned conditional-Get Proxy behavior is covered
  by the shared internal-method seam.
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
  behavior uses the shared internal-method seam.
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
  the native-call trampoline. Proxy trap order and invariants are covered by
  the R3am internal-method gate.
  `isExtensible` and `preventExtensions` preserve QuickJS's deliberate
  non-boxing branch: every primitive, including nullish, Symbol and BigInt,
  reports non-extensible, while `preventExtensions` returns that exact
  primitive unchanged. Ordinary objects use their existing extensibility bit;
  prevention is irreversible and idempotent, returns the original object, and
  leaves existing property descriptors untouched. Proxy trap forwarding and
  invariants use the shared completion-aware methods; the resizable TypedArray
  rejection branch participates in the pinned differential.
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
  the native-call trampoline. Proxy descriptor traps/invariants are active;
  integer-indexed TypedArray details participate in the same differential,
  while module-namespace exotic descriptors remain an explicit object-model
  boundary. The differential preserves two
  pinned QuickJS target quirks: incomplete identity checks for some frozen
  descriptors, and the nested-Proxy undefined-trap path which bypasses target
  `[[IsExtensible]]`.
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
  recursion is covered by the shared weighted budget. Proxy descriptor
  rechecks, invariant quirks, and TypedArray index copying are active; module
  namespace sources remain an explicit object-model boundary.
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
  Proxy trap order, partial failures, and non-empty TypedArray rejection are
  active, while module namespace behavior remains an explicit object-model
  boundary.
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
  entry iteration, Set values which are themselves entry objects, and generator
  `finally` during IteratorClose now run in the pinned differential. Proxy and
  TypedArray entry reads are active; module namespace entries remain an
  explicit boundary.
  `hasOwn` converts and boxes its target in the defining realm before converting
  its property key, deliberately reversing the legacy prototype method's
  observable conversion order. It probes only the resulting object's own
  descriptor, so inherited properties are absent, stored accessors are not
  called, String UTF-16 indices and `length` are present, Symbols retain
  identity, and lazy AutoInit slots remain unmaterialized. A measured
  nine-active-call family guard turns recursive `@@toPrimitive` reentry into a
  catchable `InternalError` before the Rust host stack is exhausted; exact
  QuickJS platform-stack depth still awaits the general native-call trampoline.
  Proxy `getOwnPropertyDescriptor` traps, invariants, and integer-indexed
  TypedArrays are active; module namespaces remain the corresponding explicit
  object-model boundary.
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
  resolves the six concise-method consumers; R2p well-known Symbol admission
  executes the final four feature-gated variants. The current vector admits
  and passes 252 variants with no parser frontier. Declaring
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
  all 53/53 own keys. This fifty-three-key list is the complete pinned QuickJS
  own-key table, not a claim that every allocation boundary has parity. The
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
  The remaining String parity gaps are Context-level observable
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
  Script-wide simple-name or recursive array/object/rest `var` and direct Program
  simple-name or recursive array/object/rest `let`/`const` now
  drive this substrate through production declaration instantiation rather
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
  trial-deletion GC behavior remain coherent. Upstream initializes the hidden
  generator intrinsics after Symbol and before defining this property; the
  current bootstrap does the same. Because no generator constructor is exposed
  as a global binding, the observable string-key order remains Math, Reflect,
  Symbol, `globalThis`, then BigInt.
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
- The Error intrinsic graph now includes `Error` plus all eight native Error
  constructors, their constructor/prototype/global relationships,
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
  pinned continue behavior, Program-global declaration descriptors, recursive
  array/object/rest binding declarations and assignments, catch BindingPatterns,
  the remaining destructuring-parameter boundary, and strip-debug name removal
  without losing read-only atoms.
  The companion `oracle_function_body_lexicals` target compares ordinary and
  normal-`Function` body/block/switch values, nested script block/switch locals,
  direct and transitive closure cells, repeated-entry and break/continue scope
  exits, cross-case TDZ/conflicts, TDZ/read-only CLI stacks, recursive
  array/object/rest binding declarations, and the remaining parameter-pattern
  boundary with the pinned release.
  The `oracle_for_lexicals` target separately locks classic-head initialization
  and NoIn parsing, ordinary and normal-`Function` values, script-local and
  cross-eval captures, initializer/body/update cell identity, the pinned
  shared-head-cell continue quirk, labeled jumps through a nested switch,
  conflicts, exact full/StripDebug TDZ and read-only stacks, and recursive
  array/object/rest binding declarations.
  The `oracle_program_lexicals` target locks direct Program values, declaration
  source order, repeated eval persistence, globalThis separation and VarRef
  splitting, preflight atomicity, failed-initializer behavior, exact
  full/StripDebug stacks and parser errors, recursive array/object/rest binding
  declarations, and the still-explicit parameter-pattern boundary.
  The `oracle_program_vars` target locks duplicate declaration records,
  no-initializer and unreachable-statement instantiation, classic-for shared
  cells, NamedEvaluation, cross-eval persistence and hidden-cell reconnection,
  exact global property attributes, data/accessor/AutoInit/inherited and
  non-extensible paths, mixed-declaration preflight atomicity, full/StripDebug
  stacks, parser conflicts, recursive array/object/rest binding declarations,
  and explicit nonclassic-loop boundaries.
  The `oracle_program_functions` target locks direct hoisting, duplicate and
  var/function source ordering, the pinned lexical-first asymmetry, global
  property normalization and rejection, two-pass atomicity, cross-eval cell
  identity, compile/execute realm splitting, and exact full/StripDebug parser
  and runtime stacks.
  The `oracle_function_body_declarations` target locks direct local/argument
  hoisting, duplicate and `var` ordering (including the `arguments` name),
  captured later lexicals and failed initializers, normal `%Function%` bodies,
  synchronous generator declarations, exact full/StripDebug stacks and parser
  errors, the explicit async-declaration boundary, and the pinned cross-realm
  regression.
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
  `%Function%` bodies, synchronous generator declarations, exact
  full/StripDebug stacks, realm splitting, and the remaining explicit async
  boundary.
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
  quirk, realm splitting, three debug modes, and exact diagnostics. The
  companion `oracle_catch_destructuring` target covers recursive catch
  BindingPatterns, lexical/eval behavior, iterator unwind, and early errors.
  The `oracle_rest_parameters` target covers identifier-only rest parameters in
  ordinary functions, synchronous object methods, arrows, and the `Function`
  constructor, including realm, length, `arguments`, entry-order, and
  diagnostic behavior.
  The `oracle_identifier_default_parameters` target covers synchronous
  identifier defaults on the same four surfaces, including TDZ, initializer
  closures, body hoists, `arguments`, `length`, anonymous names, `this`,
  `super`, default-plus-rest, and the pinned raw-argument/parameter-cell split.
  The `oracle_object_bindings` target locks direct, classic-for, and synchronous
  for-in/of declaration surfaces; fixed and computed String/Symbol property
  keys; object/array/rest recursion; defaults and NamedEvaluation; observable
  `with` Reference timing; exclusion and copy order; iterator unwind; and
  malformed-pattern diagnostics.
  The `oracle_object_rest` target separately locks fresh rest objects, fixed and
  computed String/Symbol exclusions, one-shot `ToPropertyKey`, own-key and
  enumerability snapshots followed by live `Get`, property definition order and
  attributes, primitive boxing, nested patterns, sloppy `with` Reference
  timing, parser skip-scanning, and copy/Put faults under iterator unwind.
  The `oracle_for_of` target locks simple binding/reference heads, recursive
  array/object declaration patterns, the generic iterator protocol and
  accessor order,
  Unicode String iteration, natural and abrupt close behavior, completion
  precedence, nested labels/switch/finally, raw
  native-next dispatch, realm splitting, exact diagnostics, and all three
  debug modes. Generic Array iteration is now covered by `oracle_array`;
  `for-await-of` and Iterator Helpers were covered by the later R3ak and R3v
  milestones, while
  synchronous parameter BindingPatterns are covered independently by R2z/R3a.
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
parse/link/evaluate, the ES5.1 suite, and a separate QuickJS-runner-quirk
profile remain future milestones.
Unsupported and host-missing outcomes are failures, not additional feature
skips.

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

The language slice remains incomplete. Ordinary async function
declarations/expressions, `await`, simple parameters, and the audited
default/rest/destructuring parameter plus eval/with forms are implemented.
Async arrows are implemented with lexical `this`, `arguments`, `new.target`,
and `super` capture across `await`. Ordinary async object-literal methods share
the established DefineMethod/HomeObject path and preserve `super` across
suspension. Public ordinary async instance/static class methods reuse the
corresponding class publication and HomeObject path; ordinary private async
methods compose that execution path with authenticated private callable cells
and HomeObject-derived brands. Ordinary async-generator declarations,
expressions, object-literal methods, and public/private class methods now
include the intrinsic graph and FIFO Promise driver. Object and public class
methods reuse DefineMethod/HomeObject; private class methods compose the typed
private callable cell and side-brand path. Async-generator `yield*` delegation
and `for await` now cover both async iterators and Async-from-Sync adaptation,
including active outer-iterator close across `.return()`. Other general
assignment targets, module resolution, remaining non-simple parameter
combinations, non-simple
ObjectLiteral accessor forms outside the covered synchronous setter slice, and
remaining exotic-source combinations are not yet implemented.
Unsupported declaration contexts are rejected instead of being
faked as Program functions or ordinary vars. Source `let`/`const` supports
simple identifiers and recursive array/object/rest patterns in direct Program
code, authored
ordinary-function bodies, non-empty nested brace blocks, shared switch scopes,
classic `for (;;)` heads, and synchronous `for-in`/`for-of` heads. Patterns
cover fixed and computed properties, undefined-only defaults, NamedEvaluation,
array terminal rest, object rest, and object/array recursion. These forms also
work in scripts, and ordinary
bodies including classic heads are available through the normal `%Function%`
constructor. Single-statement lexical declarations remain a later compiler
slice. Base
class declaration/expression lexical environments and TDZ behavior land in
R3e; heritage, derived construction, and `super()` land in R3f. Direct
Program lexicals now use the production global VarRef path with two-phase
instantiation; simple-name and recursive array/object/rest Program vars plus
direct ordinary function declarations use ordered, kind-specific global
declaration records.
One internal resource-failure
hardening gap is tracked here: the Rust path currently allocates the callable
after creating accepted global bindings, whereas QuickJS reserves the
callable object first. Ordinary JavaScript cannot trigger the intervening heap
failure today; matching the allocation order safely requires a provisional
two-phase bytecode-function reservation plus failure-injection coverage, rather
than attempting to roll back migrated VarRefs after the fact.

The dynamic AsyncFunction and AsyncGeneratorFunction constructors are
implemented; other native builtin constructor families remain. The hidden
dynamic GeneratorFunction constructor, base/derived class construction,
construct-only guards, constructor return validation, `super()`, `new.target`,
`Reflect.construct`, and Proxy call/construct dispatch are active. Typed
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
share the implemented completion path. Synchronous generator suspension and
resumption, ordinary async `await` rejection, and ordinary async-generator
resume/await/queue/delegation/for-await transitions and active outer-iterator
close now use that completion path. Recoverable OOM and backtrace-allocation
fallback, interrupt/termination, and the remaining abrupt-completion surfaces
are still open. The `JS_STRIP_DEBUG` /
`JS_STRIP_SOURCE` debug/source-stripping decision is implemented as a
runtime-wide three-state policy sampled by subsequent compilation: strip-source
retains filename/PC metadata but removes authored source, while strip-debug
removes the represented function source/location payload. The `qjs`
`--strip-source` and `-s` options select the same states in upstream order,
including combined short options and their effect on `toString`, function debug
accessors and Error backtraces. Strip-debug compilation also removes ordinary
lexical vardef and captured-relay names while retaining atoms needed by
read-only execution; bytecode debug serialization remains pending. The qjs
host now supplies `print` for runnable demos and Promise-job transcripts,
but non-String values still use JavaScript `ToString`; QuickJS's host-side,
side-effect-free structured `JS_PrintValue` formatting remains a separate CLI
parity frontier. The normal `%Function%` graph is present; dynamic formal
parameters support identifiers,
identifier defaults, one terminal identifier-rest parameter, and recursive
array/object/rest BindingPatterns, including mixed standalone parameter
expressions and the same Parameter Environment semantics as authored ordinary
functions. Bodies remain limited to the current statement, expression, and
simple body/block/switch/classic-for and for-in/of-head lexical-declaration
grammar.
Compiler input is still UTF-8,
so dynamic source containing an unpaired UTF-16 surrogate throws an explicit
implementation-gap `InternalError` instead of being silently rewritten. The
parser now requests tokens through fallible advances, and directive probes
seek back before strict-context rescanning, so current-token grammar errors no
longer lose to untouched later lexical failures. Contextual word reparsing for
modules stays with that unimplemented surface.
The parser now produces synchronous generator, ordinary async-function,
async-arrow, async-object-method, and public/private ordinary
async-class-method bytecode. It also produces ordinary async-generator
declaration/expression, object-literal method, and public/private class-method
bytecode, including `yield*` and `for await`; function-kind metadata and
`toString` fallback distinguish all four QuickJS kinds. Bound
dispatch is iterative and therefore does not
consume the Rust host
stack, but exact QuickJS runtime-stack accounting and its deep-bound-chain
overflow threshold are not yet reproduced. VM object coercion is wired through
the implemented unary, arithmetic, exponentiation, bitwise, shift, relational,
addition and abstract-equality operators and now reaches the implemented
callable classes through `Function.prototype.toString`. Proxy hooks share the
completion-aware internal-method seam; Date's special default hint behavior,
OOM/interrupt edges and operators outside the current bytecode slice remain
pending.

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
for the current static scope tree and defining-realm global object. Sloppy
direct-eval object-environment lookup/deletion is implemented for the current
synchronous script/function and Parameter-Environment surfaces;
`with`-introduced dynamic object environments, the remaining two entries of
String's 53-key prototype surface, Unicode-sets RegExp grammar,
the full `function_accessors.js` fixture, and other exotic internal methods are
still pending. Uncatchable termination state is also pending. Other iterator
classes and helpers, the remaining RegExp
grammar and cross-realm host surface, Unicode-backed String methods, non-simple
ObjectLiteral setter forms outside the covered synchronous slice,
exotic-source spread, and the rest of the builtin table build on those layers.

The remaining parity surface also includes the full grammar/opcode set, the
Unicode 17 normalization/script/property tables beyond the implemented
identifier, case-conversion, `Cased` and `Case_Ignorable` data, the advanced
RegExp grammar, modules, remaining jobs/Promise/async and generator surfaces,
the Test262 agent host and agent-backed waiter conformance, remaining
WeakRef/finalization edge cases, bytecode version 5 and BJSON interoperability,
`std`/`os`, workers, REPL/qjsc, and the complete Rust and C embedding APIs.
`Atomics.waitAsync` is not part of the pinned QuickJS target.

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
moves the shared flat-array binding lowering into the new
`compiler/destructuring.rs`; `compiler.rs` is 11,838 lines. R2r keeps both
facades unchanged and grows that bounded owner from 418 to 568 lines for the
recursive array path. Ordinary declarations and synchronous iteration heads
use the same owner, so the feature does not duplicate binding logic or resume
runtime-monolith growth.
R2s again keeps `runtime.rs` unchanged at 9,822 lines. Fixed/computed recursive
object binding lowering grows the bounded `compiler/destructuring.rs` owner
from 568 to 1,122 lines; the `compiler.rs` facade reaches 11,849 lines for its
declaration-head wiring and deferred object-rest diagnostic frontier. The
runtime monolith therefore does not grow with this compiler-only slice.
R2t also keeps `runtime.rs` unchanged at 9,822 lines. Exclusion/rest lowering
grows `compiler/destructuring.rs` from 1,122 to 1,247 lines while the compiler
facade shrinks from 11,849 to 11,840 lines after removing the deferred
whole-source diagnostic path. The shared CopyDataProperties kernel stays in
`runtime/intrinsics/object.rs`, which grows from 2,246 to 2,306 lines; its
VM-facing host bridge grows from 3,210 to 3,427 lines in
`runtime/vm_host.rs`, while typed stack-depth metadata and dispatch remain in
bytecode/VM owners. This adds the runtime behavior without resuming growth of
the runtime facade.
R2u again leaves `runtime.rs` unchanged at 9,822 lines. Array assignment and
the syntax-preserving object-assignment frontier grow the bounded
`compiler/destructuring.rs` owner from 1,247 to 1,913 lines; exact direct and
for-head dispatch adds only 20 net lines to the `compiler.rs` facade, now
11,860 lines. The object-frontier validator is intentionally colocated with
the lowering it will replace when ObjectAssignmentPattern lands, rather than
adding another runtime or compiler-facade path.
R2v replaces that temporary validator with complete direct and synchronous
for-head ObjectAssignmentPattern lowering. Removing the frontier while adding
fixed/computed leaves, recursive array/object joins, rest copying, and shared
assignment control inversion reduces `compiler/destructuring.rs` slightly from
1,913 to 1,904 lines. `compiler.rs` reaches 11,865 lines after the shared
Array/Object invalid-target diagnostic path, while `runtime.rs` remains
unchanged at 9,822 lines. No VM opcode or runtime-facade branch was added; the
existing typed Reference, property, and CopyDataProperties operations carry
the semantics.
R2w again keeps `runtime.rs` unchanged at 9,822 lines. Catch BindingPattern
wiring grows `compiler.rs` from 11,865 to 11,930 lines and the shared
`compiler/destructuring.rs` owner from 1,904 to 1,945 lines. Oxide expands
`PrepareCatchScope` to `Undefined; InitializeLocal` for every pattern leaf,
reproducing the frame-default `undefined` state after QuickJS's exception
handler skips `OP_enter_scope`. Ordinary semantics are oracle-locked, but the
temporary one-slot stack use and two instructions per leaf do not yet reproduce
QuickJS's extreme max-stack/code-size boundary exactly; a future zero-stack
scope-preparation opcode should remove that resource-parity debt.
R3b keeps the runtime facade bounded: `runtime.rs` moves only from the R3a
baseline of 9,826 lines to 9,835. The complete synchronous parameter-direct-
eval path raises `compiler.rs` from 13,164 to 13,760 lines, while cross-layer
authentication and its adversarial tests raise `heap.rs` from 11,874 to 12,397,
`runtime/bytecode_publish.rs` from 6,811 to 7,737, and
`runtime/vm_host.rs` from 3,443 to 3,697. Keeping the execution facade at a
nine-line delta is intentional; the compiler, heap, and publisher totals are
explicit phase-split debt, not a precedent for continuing to grow those
owners.
R3c removes the complete Error intrinsic family from the runtime facade:
`runtime.rs` falls by 243 lines, from 9,835 to 9,592, while the new
`runtime/intrinsics/error.rs` owner is 385 lines including AggregateError's
iterator-to-Array kernel. The combined owner surface grows by 142 lines for the
new behavior, but dispatch, construction, Error formatting, branding, and
IteratorClose logic no longer add to the monolith. This is the intended answer
to the earlier runtime-size warning: feature work must leave the facade smaller
when a coherent intrinsic boundary is available.
R3d keeps `runtime.rs` unchanged at 9,592 lines. Argument-list construction and
the QuickJS iterator/fast-Array behavior stay behind the existing VM-host and
intrinsic owners rather than returning feature logic to the facade.
R3e moves the class parser and constructor/prototype publication into the new
400-line `compiler/class.rs` and 300-line `runtime/class.rs` owners.
`runtime.rs` grows only from 9,592 to 9,595 lines: one module declaration and
two parameter-initializer provenance relays used by dynamic eval publication.
`compiler.rs` is 13,976 lines; the new class grammar does not return to that
facade, while later compiler decomposition remains necessary.
R3f keeps the runtime facade at 9,610 lines; class-definition parsing and
publication remain in the 463-line `compiler/class.rs` and 575-line
`runtime/class.rs` owners. The authenticated derived-constructor boundary does
raise `compiler.rs`, `heap.rs`, `runtime/bytecode_publish.rs`, and
`runtime/vm_host.rs` to 14,225 / 13,839 / 8,928 / 4,296 lines respectively.
Those are explicit extraction debt: the next class-element work must split the
derived-construction verifier/executor seams instead of adding field logic to
`runtime.rs` or enlarging the same trust-boundary blocks inline.
R3g follows that boundary: `runtime.rs` is 9,611 lines at the milestone
checkpoint, while public-field and static-block lowering/runtime behavior live
under `compiler/class/` and `runtime/class_fields.rs`. The facade gains only
the module seam; initializer execution and validation do not return to the
monolith.
R3h keeps private-name storage and operations in
`runtime/private_elements.rs`, VM bridging in
`runtime/vm_host/private_elements.rs`, and lowering under `compiler/class/`.
The public runtime facade again gains only the module seam.
R3i extends those same owners for ordinary synchronous private methods and
per-class-side brands, with publication checks isolated in
`runtime/bytecode_publish/private_elements.rs`. `runtime.rs` remains a facade
at 9,658 lines; the method and brand implementation lives in the dedicated
private-element/compiler/VM-host modules rather than returning to the
monolith. R3j extends those same typed private-element cells and publication/
VM-host seams for getter/setter pairs; `runtime.rs` remains the 9,674-line
facade rather than absorbing the accessor implementation.
R3k keeps the same boundary: `runtime.rs` is 9,748 lines, while the generator
state machine and intrinsics live in the dedicated 875-line
`runtime/generator.rs`; VM snapshot encode/decode stays in `runtime/vm_host.rs`,
heap ownership and tracing in `heap.rs`, and resumable execution in `vm.rs`.
The facade gained only dispatch/publication seams instead of absorbing the
generator kernel.
R3l composes those existing generator and private-element seams: lowering stays
under `compiler/class/`, private callable/brand storage stays in
`runtime/private_elements.rs`, VM access stays in
`runtime/vm_host/private_elements.rs`, and both publication defenses retain
their dedicated owners. `runtime.rs` remains the 9,748-line facade; this slice
adds no product code to that monolith or a second generator state machine.
R3m-R3q preserve that boundary for Promise work: the Promise object/reaction
owner remains `runtime/intrinsics/promise.rs`, the runtime FIFO remains
`runtime/jobs.rs`, and R3n places its new static-method and race algorithms in
the 327-line `runtime/intrinsics/promise/convenience.rs`; R3o and R3p place
their algorithms in the 203-line `runtime/intrinsics/promise/finally.rs` and
the initially 240-line `runtime/intrinsics/promise/all.rs` modules. R3q extends
the latter shared aggregate owner to 496 lines for `allSettled` and `any`. The
current `runtime.rs` facade is still 9,803 lines rather than absorbing those
algorithms.
R3s keeps that boundary: `runtime.rs` remains 9,803 lines, while the
QuickJS-shaped `RegExp.escape` algorithm and its focused allocation/UTF-16 tests
live in the dedicated 281-line `runtime/intrinsics/regexp/escape.rs` module.
The RegExp kernel itself is isolated in
`src/regexp/` as flags, typed opcodes, compiler and executor modules rather than
growing the runtime facade. Realm-aware property completion wrappers and storage
helpers, bytecode publication linking and call dispatch, runtime/root lifecycle,
and the remaining intrinsic families still share the file; `compiler.rs`
similarly combines several compiler phases.
R3t moves `runtime.rs` only from 9,803 to 9,808 lines. Mapped-arguments logic
remains in `runtime/vm_host.rs`, generator regressions in
`runtime/generator.rs`, and declaration/parser work in compiler modules. It
does not introduce a second generator state machine.
R3v moves Iterator algorithms into
`runtime/intrinsics/iterator.rs`; `runtime.rs` is 9,764 lines after the new
realm hooks and extraction of adjacent Iterator plumbing. The 1,871-line
module is still substantial, but it is a cohesive intrinsic owner rather than
another expansion of the runtime facade.
R3w keeps the intrinsic graph in that owner but splits the sequencing state
machine into the adjacent `runtime/intrinsics/iterator/concat.rs` module. It
adds only the raw-fast-path realm plumbing plus ordinary payload classification
to the facade. `runtime.rs` moves from 9,764 to 9,824 lines; the main Iterator
owner moves from 1,871 to 1,902 lines, with `Iterator.concat` isolated in a
375-line submodule. Generated Test262 bookkeeping remains outside all three
product modules.
R3z keeps the async driver in the new 395-line
`runtime/async_function.rs`; `runtime.rs` reaches 9,856 lines, a 32-line
module/initialization/dispatch seam rather than another inline state machine.
The generator-specific VM snapshot bridge is generalized once for generator
and await activations in `runtime/vm_host.rs`. `heap.rs` reaches 19,676 lines
because the hidden async state, GC edge/atom ownership, transactional
phase checks, and adversarial lifecycle tests live at the arena trust
boundary. That file remains extraction debt, but async behavior does not
return to the runtime facade.
R3am moves completion-aware object dispatch into the new 1,706-line
`runtime/internal_methods.rs` owner and Proxy lifecycle into the 226-line
`runtime/intrinsics/proxy.rs` owner. The main facade is 9,904 lines, eight
below its 9,912-line R3al baseline; the arena trust boundary grows to 21,034
lines for Proxy edges, validation, and unique-shape mutation.
R3an keeps ArrayBuffer installation, dispatch, and all observable algorithms
in the independent `runtime/intrinsics/array_buffer.rs` owner; `runtime.rs`
remains below 10,000 lines rather than absorbing that intrinsic family. The
branded `Vec<u8>` payload, realm roots, mutation boundary, and backing-store
validation still make `heap.rs` explicit extraction debt, but that arena debt
does not obscure or replace the dedicated intrinsic owner.
R3ao keeps the facade bounded at 9,934 lines in `runtime.rs`. The complete
DataView constructor, accessor, conversion, bounds, and read/write behavior
lives in the independent 894-line
`runtime/intrinsics/array_buffer/data_view.rs` module rather than returning to
the facade. `heap.rs` reaches 22,334 lines because the branded view payload,
realm roots, traced ArrayBuffer edge, mutation validation, and adversarial
lifecycle coverage remain at the arena trust boundary; that file is still
explicit extraction debt.
R3ap keeps `runtime.rs` bounded at 9,950 lines. The complete shared TypedArray
kernel lives in the independent 1,872-line
`runtime/intrinsics/array_buffer/typed_array.rs` module, with 1,243 lines of
directed adversarial tests in its nested test module. `heap.rs` reaches 22,884
lines because the branded payload, 12 realm prototype roots, traced backing
edge, validation, and allocation-free backing-store memmove remain at the
arena trust boundary; that file remains explicit extraction debt.
R3aq keeps `runtime.rs` unchanged at 9,950 lines. The shared TypedArray owner is
1,889 lines, while the new observable mutation algorithms live in the adjacent
247-line `typed_array/mutation.rs` module rather than growing the facade.
Directed tests are split between the 1,246-line owner test module and the
273-line mutation test module. `heap.rs` reaches 23,026 lines after adding the
raw-word fill/reverse mutation boundary and direct invariant coverage; it
remains explicit extraction debt rather than hidden runtime-facade growth.
R3ar again keeps `runtime.rs` unchanged at 9,950 lines and `heap.rs` unchanged
at 23,026 lines. Prototype installation and typed dispatch add only the
filtered QuickJS surface seam to the 1,914-line shared owner; the observable
algorithms live in the adjacent 196-line `typed_array/search.rs`, with a
246-line directed test module. This milestone therefore expands semantics
without growing either monolith or mixing indexed-search rules into generic
Array property traversal.
R3as keeps both monoliths unchanged again. The shared owner is 1,940 lines
after publishing and dispatching the four methods; their complete observable
algorithm lives in the adjacent 92-line `typed_array/find.rs`, with a
308-line directed test module. Callback mutation, resizable-buffer and detach
semantics stay isolated from both the facade and generic Array traversal.
R3at again leaves `runtime.rs` at 9,950 lines and `heap.rs` at 23,026 lines.
The shared TypedArray owner reaches 1,959 lines after publishing and
dispatching `every`/`some`; their complete algorithm stays in the adjacent
96-line `typed_array/iteration.rs`, with a 323-line directed test module.
Predicate short-circuit, error ordering, live RAB/detach reads, and numeric
prototype suppression therefore remain outside both monoliths and generic
Array traversal.
R3au likewise leaves `runtime.rs` at 9,950 lines and `heap.rs` at 23,026
lines. Extending the same isolated owner and callback kernel for `forEach`
brings the shared owner to 1,960 lines, `typed_array/iteration.rs` to 105
lines, and its directed test module to 382 lines. Callback-result disposal,
non-short-circuit traversal, and the final `undefined` result therefore add no
new facade or heap seam.
R3av again leaves `runtime.rs` at 9,950 lines and `heap.rs` at 23,026 lines.
Publishing and dispatching `reduce`/`reduceRight` brings the shared TypedArray
owner to 1,976 lines; the complete accumulator algorithm lives in the adjacent
94-line `typed_array/reduce.rs`, with a 470-line directed test module.
Direction, accumulator identity, live RAB/detach reads, and cross-realm
behavior therefore remain outside both monoliths and generic Array traversal.
R3aw again leaves `runtime.rs` at 9,950 lines and `heap.rs` at 23,026 lines.
Publishing `map`/`filter` brings the shared TypedArray owner to 1,979 lines.
Their callback flow stays in the adjacent 172-line
`typed_array/iteration.rs`; the 79-line `typed_array/species.rs` owns species
construction and the observable filter `.set` handoff. The transform-specific
adversarial tests live in a separate 626-line module. This keeps construction,
realm, conversion, RAB, and detach rules out of both monoliths.
R3ax again leaves `runtime.rs` at 9,950 lines and `heap.rs` at 23,026 lines.
Moving exact-argv species validation and the post-coercion buffer-view
constructor into `typed_array/species.rs` reduces the shared owner to 1,895
lines even after publishing and dispatching `slice`/`subarray`. The complete
copy/view algorithms live in the adjacent 263-line `typed_array/slice.rs`;
the expanded species/construction seam is 299 lines, and 455 lines of directed
tests cover descriptor, realm, raw-bit, overlap, RAB, detach, species, and
constructor-error ordering. No new heap primitive is required.
R3ay again leaves `runtime.rs` at 9,950 lines and `heap.rs` at 23,026 lines.
Consolidating the same-class constructor clone and publishing
`with`/`toReversed` reduces the shared TypedArray owner to 1,863 lines. The
complete change-by-copy algorithms live in the adjacent 224-line
`typed_array/copying.rs`; its 391-line directed test module covers old-length
indexing, coercion order, RAB shrink tails, raw-word reversal, defining-realm
allocation, and canonical QuickJS errors. No new heap primitive or facade seam
is required.
R3az again leaves `runtime.rs` at 9,950 lines and `heap.rs` at 23,026 lines.
Publishing the two stringification entries brings the shared TypedArray owner
from 1,863 to 1,878 lines. The complete dedicated algorithm lives in the
adjacent 120-line `typed_array/stringification.rs`; its 563-line directed test
module covers surface metadata, primitive locale dispatch, error ordering,
string limits, RAB growth/shrink, detach, fixed-view OOB behavior, and
defining-realm selection. Generic Array traversal remains independently owned.
R3ba again leaves `runtime.rs` at 9,950 lines and `heap.rs` at 23,026 lines.
Publishing and dispatching `sort`/`toSorted` brings the shared TypedArray owner
to 1,893 lines. The complete TypedArray algorithm lives in the adjacent
504-line `typed_array/sort.rs`, with a separate 791-line directed test module.
Default sorting uses an O(1)-auxiliary raw-word backing-store accessor; custom
sorting owns only its exact raw-byte snapshot and `u32` index vector. The
shared QuickJS `rqsort` storage-accessor seam leaves
`runtime/intrinsics/array.rs` at 3,851 lines, roughly 45 net lines above the
previous owner, while TypedArray realm, RAB, raw-bit, error-ordering and
writeback rules remain outside generic Array code. No new heap primitive or
runtime facade seam is required.
R3bb makes no production-code change, so those owner and facade sizes remain
fixed. Three QuickJS observation tests authenticate the existing shared
Array-iterator path across all 12 TypedArray classes, resizable-buffer
behavior, detach, and transient-OOB recovery. A fourth Rust structural test
locks the source-audited manual-next/outer-operation realm split.
R3bc again leaves `runtime.rs` at 9,950 lines and `heap.rs` at 23,026 lines.
Routing static `from` and `of` through their diagnostic-specific constructor
seam brings the shared TypedArray owner to 1,895 lines and
`typed_array/species.rs` to 321 lines; species construction itself remains on
the distinct generic constructor path. The owner test module reaches 1,296
lines, and the separate 685-line `oracle_typed_array_of.rs` keeps its three
QuickJS-observation entry points distinct from the Rust-only cross-realm
structural test.
R3bd again leaves `runtime.rs` at 9,950 lines and `heap.rs` at 23,026 lines.
The shared TypedArray owner reaches 1,910 lines after the exact nullish
diagnostic and retained-materialization lifetime fix, while its directed test
module reaches 1,316 lines. The separate 914-line
`oracle_typed_array_from.rs` owns eight focused vectors and keeps its three
QuickJS observation/self-check/differential entries distinct from the fourth
Rust-only cross-realm structural entry.
R3be admits the global `TypedArray` profile without production runtime changes:
`runtime.rs` remains 9,950 lines and `heap.rs` remains 23,026 lines. The
activation, spillover, reason-only, scoped, and full-vector evidence stays in
checksum-bound manifests and reports, while four focused `with` tests include
pinned QuickJS differentials.
Dedicated structural milestones must keep splitting those seams under the same
differential and Rust-only gates, and future feature work must not resume
extending either monolith indefinitely.

`README.md` remains the concise public entry point; milestone bookkeeping stays
in these dedicated status and Test262 documents.

## Reproduce current evidence

```sh
cargo test --locked --workspace --all-targets

QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_iterator_helpers -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_iterator_concat -- --nocapture
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
  cargo test --test oracle_catch_destructuring -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_rest_parameters -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_identifier_default_parameters -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_for_of -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_bindings -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_rest -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_assignment -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_object_assignment -- --nocapture
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
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_aggregate_error -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_argument_spread -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_class_base -- --nocapture

./scripts/test-parity-slice.sh
./scripts/test-test262-smoke.sh
./scripts/test-test262-provenance.sh
./scripts/test-test262-reflect.sh
./scripts/test-test262-date.sh
./scripts/test-test262-string-split.sh
./scripts/test-test262-regexp-core.sh
./scripts/test-test262-regexp-builtins.sh
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
./scripts/test-test262-array-binding-nested.sh
./scripts/test-test262-array-assignment-flat.sh
./scripts/test-test262-object-assignment-flat.sh
./scripts/test-test262-object-assignment-nested.sh
./scripts/test-test262-object-assignment-rest.sh
./scripts/test-test262-object-binding.sh
./scripts/test-test262-object-rest-binding.sh
./scripts/test-test262-object-rest-global.sh
./scripts/test-test262-catch-binding.sh
./scripts/test-test262-identifier-rest.sh
./scripts/test-test262-identifier-defaults.sh
./scripts/test-test262-parameter-binding-patterns.sh
./scripts/test-test262-parameter-expression-binding-patterns.sh
./scripts/test-test262-parameter-direct-eval.sh
./scripts/test-test262-aggregate-error.sh
./scripts/test-test262-argument-spread.sh
./scripts/test-test262-class-base.sh
./scripts/test-test262-class-derived.sh
./scripts/test-test262-class-public-init.sh
./scripts/test-test262-class-private-fields.sh
./scripts/test-test262-class-private-methods.sh
./scripts/test-test262-class-private-accessors.sh
./scripts/test-test262-class-generator-methods.sh
./scripts/test-test262-class-private-generator-methods.sh
./scripts/test-test262-class-sync-matrix.sh
./scripts/test-test262-generator-destructuring.sh
./scripts/test-test262-iterator-helpers.sh
./scripts/test-test262-iterator-sequencing.sh
./scripts/test-test262-async-function-core.sh
./scripts/test-test262-async-arrow-core.sh
./scripts/test-test262-async-object-method-core.sh
./scripts/test-test262-async-class-method-core.sh
./scripts/test-test262-async-private-class-method-core.sh
./scripts/test-test262-async-generator-core.sh
./scripts/test-test262-async-generator-object-method-core.sh
./scripts/test-test262-async-generator-class-method-core.sh
./scripts/test-test262-async-generator-private-class-method-core.sh
./scripts/test-test262-async-generator-yield-star.sh
./scripts/test-test262-for-await-of.sh
./scripts/test-test262-global-async.sh
cargo build --bin qjs
./scripts/test-r3l-class-private-generators-oracle.sh --oxide ./target/debug/qjs
./scripts/test-r3s-regexp-escape-control-oracle.sh --oxide ./target/debug/qjs
./scripts/test-r3z-async-function-core-oracle.sh --oxide ./target/debug/qjs
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_async_class_method -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_async_private_class_method -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_async_generator -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_async_generator_object_method -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_async_generator_class_method -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_async_generator_private_class_method -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_async_generator_yield_star -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_for_await_of -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_proxy -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_array_buffer -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_data_view -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_typed_array_of -- --nocapture
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_typed_array_from -- --nocapture
./scripts/test-test262-proxy.sh
./scripts/test-test262-array-buffer.sh
./scripts/test-test262-data-view.sh
./scripts/test-test262-data-view-global.sh
./scripts/test-test262-typed-array-core.sh
./scripts/test-test262-uint8array-codecs.sh
./scripts/test-test262-uint8array-codecs-global.sh
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
single/labelled Annex B, synchronous try/catch/finally with recursive
array/object/rest catch BindingPatterns, synchronous identifier rest/default
and recursive BindingPattern parameters, and direct eval in their Parameter
Environments across ordinary functions, object methods, arrows, and the
`Function` constructor, synchronous
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
The ArrayBuffer target locks its branded backing store, fixed/resizable and
detached metadata, constructor/species/accessor graph, resize/slice/transfer
semantics, host detachment, allocation failure, realm ownership, and GC
lifetime against pinned QuickJS.
The DataView target locks its complete constructor/prototype graph, all 11
getter/setter families, endian and numeric conversion behavior,
`ArrayBuffer.isView`, detach and error ordering, and fixed-versus-tracking
resizable-buffer shrink/grow behavior against the same pinned oracle.
The TypedArray shared-kernel target locks the 12-class graph and backing
payload, constructor/coercion order, integer-indexed exotic internal methods,
live resizable-buffer bounds, detach, iteration, in-place mutation, `for-in`,
overlap, raw words, indexed lookup/search, callback find traversal, realm,
host-property, and GC seams against the same pinned oracle. Its exclusion
ledger keeps later method families and external dependencies visible.
The full gate discovers every `tests/oracle_*.rs`
integration target, reuses an executable `QJS_ORACLE` or checksum-verifies and
builds the pinned test-only oracle, obtains and checksum-verifies the matching
Unicode table source, then runs both generated-table drift checks, formatting,
unit/integration/oracle tests, Clippy, and the Rust-only product gate. The oracle
is never part of the product dependency graph or runtime.
