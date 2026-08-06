# Test262 progress baseline

Test262 is now a pinned progress instrument, not yet a completion claim. The
authoritative compatibility target remains QuickJS 2026-06-04; focused QuickJS
differentials still decide exact behavior inside each implemented slice.

Last audited: 2026-08-07.

## R3dw global destructuring-assignment admission

R3dw freezes the complete Test262 `destructuring-assignment` universe at 141
paths / 217 variants. The gate binds the raw universe, ordered activation and
retained partitions, complete metadata and source ledgers, and the exact
profile transition. The candidate adds one feature name plus exactly 90
audited parse-negative paths; runtime, parser, VM, host hooks, and execution
policy remain byte-identical.

The activation contains 137 paths / 213 variants: 47 normal paths / 49
variants and 90 negative paths / 164 variants. Each negative is authenticated
against the pinned parse phase and expected `SyntaxError`. The other four raw
variants carry `flags: [module]` and exercise `import.meta`, so they remain
`unsupported-module` instead of being counted as destructuring passes.

Pinned QuickJS 2026-06-04 passes all 217 raw variants and all 213 activation
variants. Oxide gains all 213 activation passes with zero regressions and
leaves the four module outcomes unchanged. The full-vector acceptance join is
required to change exactly those 213 rows and preserve the other 101,824 of
102,037 variants. Independent candidates A and B both report a byte-identical
68,091 passes / 68,143 runnable, and the final gate requires both full-vector
replays to match before the receipt is promoted. This remains a progress
instrument, not a Feature Parity claim.

```sh
./scripts/test-test262-destructuring-assignment-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-destructuring-assignment-global.sh
TEST262_FULL_WORKERS=8 ./scripts/test-test262-destructuring-assignment-global.sh --full
```

## R3dv global implemented-leaf admission

R3dv freezes the complete 46-path / 69-variant union carrying any of these
eight already-implemented Test262 features: `Array.prototype.values`,
`Object.is`, `caller`, `for-of`, `json-superset`, `proxy-missing-checks`,
`stable-array-sort`, and `stable-typedarray-sort`. The exact feature
partitions, ordered variants, metadata, source ledger, host-hook inventory,
and profile transition are checksum-bound by the milestone gate.

Pinned QuickJS 2026-06-04 runs and passes all 69 variants. The candidate
profile adds exactly the eight feature names; runtime, parser, VM, host hooks,
audited negatives, and execution policy remain unchanged. Oxide gains 65
global passes. Four variants also require the independent
`destructuring-assignment` feature, so they remain gated instead of being
counted as implementation passes.

The complete 102,037-variant join changes only the 69 expected rows: 65 gain
passes, four lose only the newly admitted leaf reason, and the other 101,968
are unchanged. The canonical baseline is now 67,878 passes / 67,930 runnable.
This remains a progress instrument, not a Feature Parity claim.

```sh
./scripts/test-test262-implemented-leaves-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-implemented-leaves-global.sh
TEST262_FULL_WORKERS=8 ./scripts/test-test262-implemented-leaves-global.sh --full
```

## R3du global cross-realm admission

R3du freezes the complete Test262 universe carrying `cross-realm`: 201 paths /
394 variants. The path manifest, ordered variant keys, metadata projection,
and source-ledger rows hash to
`dbff48284d8659486931a76fac06705efd94e118e6531b4f3f6a5df052654986`,
`f794b9260cc1314d534c1faa47b45364c4fa8d29f89197ebc341ade164c34897`,
`520b2382a56ec16ab37ed5d6fc15c5be051646e27815552b39af30667c8432ec`,
and
`12cf853edfca03bd3ee809f6bc2569084e4d09e56e29edd31903c96676c47ecc`.

Pinned QuickJS 2026-06-04 runs and passes 173 paths / 338 variants twice; its
runnable manifest and variant keys hash to
`a9a29ffbf8da06bd44004857f0dcc177e5fe9739726df61ad6f57a16b397277a`
and
`237b433e19843e4f9b224939d9662a1137d5d0904b99e4e2eceda9e9838e115c`.
The other 56 variants form a disjoint canonical skip partition: 18 paths / 36
variants are skipped by feature and 10 paths / 20 variants by configuration.
Six Intl paths match both upstream rules, with config exclusion taking
precedence; no skipped variant is counted as an implementation pass.

The candidate profile adds exactly `cross-realm`. The runtime, host-hook
implementation and allowlists, audited negatives, and execution policy remain
unchanged. The parent and candidate profile SHA-256 values are
`a2e139f4c7523fd29d7f06441b4d04816bac8a074972afb9866d889588158db8`
and
`4fc0f253f5146025732b7b89b8c0547fa4a268f671373078980b4d07de15860d`.
The focused parent has 338 `unsupported-feature` outcomes; the candidate gains
323 passes and retains 15 independently gated variants—13 requiring private
class features and two requiring `regexp-v`. The 15 retained rows change only
their unsupported-feature detail, and all 56 QuickJS skips are unchanged.

Two full candidate replays are byte-identical. The complete join records 323
pass gains, 15 detail-only changes, 101,699 unchanged rows, and zero
regressions across all 102,037 variants. The canonical baseline is now 67,813
passes / 67,865 runnable. Its TSV and JSONL SHA-256 values are
`dbc3eb642f2e26e91cd811abae0aad185714ba3c9a08fcf6cbb505217923d169`
and
`350a2c9293d6d559b197dc530f2d32af2c43f1ebc9ef09e8b0d1efcea7fc7394`.
The admission remains a progress instrument, not a Feature Parity claim.

```sh
./scripts/test-test262-cross-realm-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-cross-realm-global.sh
TEST262_FULL_WORKERS=8 ./scripts/test-test262-cross-realm-global.sh --full
```

## R3dt global base-class admission

R3dt freezes the complete Test262 universe carrying the base `class` feature:
4,768 paths / 9,374 variants. The path manifest, ordered variant keys, and
source ledger hash to
`a92e0bd5ab869839868a734308cb43d1fed369deef5d08d015b025d4b6acde17`,
`d75e8faeb7ec9c076d4d565f26319298d1c3f9d28c5d8c50b9c59c6424610c7e`,
and
`9d0286bcffe71b2296871e28c2c6ac18faa4cbb27f6cd71172b80cbb89c21216`.
Pinned QuickJS 2026-06-04 passes every one of its 9,311 runnable variants and
config-skips the other 63. The skip partition is independently frozen rather
than being counted as implementation success.

The promoted profile adds exactly `class` plus 54 audited parse-negative paths
/ 108 variants. The focused parent reports 9,268 unsupported-feature rows, 63
config skips, and 43 unsupported modules. The candidate converts 816 rows to
passes, retains 8,452 behind independent feature tags, and leaves all 106
config/module rows unchanged. Exact TSV and JSONL projections agree on 816
outcome gains, 8,452 reason-only changes, and zero regressions. This admits the
base feature without treating private methods, decorators, or other finer
class tags as implemented.

The parent and candidate profile hashes are
`02dd4c59f0103d8bce2296646e7d9031051634c37e5b693336d752c11aa647d4`
and
`a2e139f4c7523fd29d7f06441b4d04816bac8a074972afb9866d889588158db8`.
Two independent Linux ARM64 full candidate reports are byte-identical and
match the native macOS ARM64 hashes. Across all 102,037 variants, only the
9,268 expected class-universe rows change: 816 gain passes and 8,452 lose only
the admitted `class` reason. The remaining 92,769 rows are unchanged and there
are zero pass regressions.

The canonical baseline is now 67,490 passes / 67,542 runnable. Its TSV and
JSONL SHA-256 values are
`de1f16b5ae92cba92d04ccf5c582d45516625abddf06af7827e97bc6e76175cc`
and
`270dd718813a652fb99846c83238bb97cf1275fa6a3e29292ba21d30d8209db1`.
The gate binds those reports to the parent commit, upstream/profile hashes,
full metadata and source ledger, QuickJS receipt, four scoped class
predecessors, and independent candidate files. This remains a progress
instrument, not a Feature Parity claim.

```sh
./scripts/test-test262-class-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-class-global.sh
TEST262_FULL_WORKERS=8 ./scripts/test-test262-class-global.sh --full
```

## R3ds `IsHTMLDDA` host semantics and global admission

R3ds freezes the complete pinned Test262 `$262.IsHTMLDDA` universe: 42 paths /
84 sloppy-and-strict variants. The path manifest hashes to
`36adfbe3ebab8b0ba9d5a109ba7f5175cafff1971dfbc5fc762ad168dbfdb0a5`,
the ordered variant keys hash to
`ab6197861f36270b6610882285b111b09963ae28d03592df399ef0c25bf0b83e`,
and the 42-line source ledger hashes to
`72e10df46f93d5cdf0a16e8366d9c358c8101810f62219ee8a0943ceb3b418bd`.
Pinned QuickJS 2026-06-04 passes all 84 variants.

The gate records four independently authenticated states. The historical
R3dr runtime and profile report 84 `unsupported-host-is-html-dda` outcomes.
The R3ds runtime under that same profile reports 84 `unsupported-feature`
outcomes, proving that host implementation does not bypass feature selection.
The global candidate adds only `IsHTMLDDA` and passes 80 variants, retaining
four `unsupported-feature` outcomes that also require `class`. The exact
scoped candidate adds `IsHTMLDDA` plus `class` only for this manifest and
passes 84/84. Its 40-path activation and two-path class-deferred manifests
hash to
`12a27c2af023d4679c45f4248111e840b29337ead0e45b17e5c89d357f84ce55`
and
`e1cdd7f226bcffd0710704c5cbedb3bc1cc01e7cc99016e0563b49ac88141d07`.

The parent, global candidate, and scoped candidate profile hashes are
`a903c4c7850dbf676477d5ef9038a9ce7c9d581eb70e1ac1f17cf30adc3f21fe`,
`02dd4c59f0103d8bce2296646e7d9031051634c37e5b693336d752c11aa647d4`,
and
`0cc6bb596188cf3b244f8f223663a2bd881bae9a90f73d456f1d9ded4295f118`.
Selection is fail-closed on the exact manifest, pinned source hashes, flags,
features, includes, negative metadata, and profile identity at both the
coordinator and isolated worker boundaries.

Two independent native Linux ARM64 full candidate runs are byte-identical.
The canonical result is 66,674 passes / 66,726 runnable / 102,037 total
variants. An exact historical-to-candidate join changes only the 84-member
cohort and preserves all 101,953 outside rows byte-for-byte, with 80 pass gains
and zero pass regressions. The canonical TSV and JSONL SHA-256 values are
`0a70f2e3e1deb5d6e410367522451ac7a88f3a05a2a479d378933085a93fe05d`
and
`ae793ae44b4ad65b8ee87238501aa2d02ba453277168c212ec0397b494f41adc`.
The four deferred class-dependent variants are not silently counted as
implemented, and the result remains pre-parity evidence.

```sh
./scripts/test-test262-is-html-dda-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-is-html-dda-global.sh
TEST262_FULL_WORKERS=8 ./scripts/test-test262-is-html-dda-global.sh --full
```

## R3dr global agent wake/FIFO admission

R3dr globally admits the exact 21-path union previously proven by the R3dp and
R3dq scoped gates. The 17 wake/count/location paths hash to
`8502e6fa50a94a7e9eef34310535f29906c2d9b1eaa49e8fe0d9388fa0e4c4f4`;
the four FIFO paths hash to
`8e0fc31a034e1b76aff14e15bc1582ed820e8efb93bd633c173b3ccbf33ba5e8`;
their exact union hashes to
`76dc724e39d9eab3c707150ac5811712c543b71ab650339ba559e9a5429c7ea4`.
No runtime, feature, negative-test, or execution-policy bytes change. The
parent and candidate profile hashes are
`8c80eee8846d3eaf08f1aa0622e0edc9a8290aa03c492eb25003f9c2dc8f4052`
and
`a903c4c7850dbf676477d5ef9038a9ce7c9d581eb70e1ac1f17cf30adc3f21fe`.

The focused transition has 42 unsupported-to-pass gains and zero regressions;
pinned QuickJS passes the same 42 variants. Two independent native Linux ARM64
release-mode candidate full runs are byte-identical and match the native macOS
hashes. Exact TSV and JSON joins change only the 42 admitted variants and
preserve all 101,995 outside rows byte-for-byte. The canonical vector is 66,594
passes / 66,646 runnable / 102,037 total variants, with zero
`unsupported-host-agent` outcomes. Its TSV/JSONL SHA-256 values are
`ba1d7e6612b4750d4cecb71ab947bba5d4a47e0603d5987aef64b4187ba25cf3`
and
`72bb3268e0b0a7e8297772a5aaa59487a6b6b8181f2eb812c379ce2351222f34`.
The checksum-bound gate also authenticates both scoped predecessor gates, the
pinned source ledger, the QuickJS receipt, the prior global baseline, and the
canonical full-vector bridge. This is progress evidence, not a parity claim.

```sh
./scripts/test-test262-agent-wake-fifo-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-agent-wake-fifo-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-agent-wake-fifo-global.sh --full
```

## R3dq scoped agent FIFO wake-order admission

R3dq admits only the exact four FIFO wake-order paths / eight sloppy-and-strict
variants into scoped runner profiles. It changes neither runtime semantics nor
the live global profile. The canonical universe hashes to
`8e0fc31a034e1b76aff14e15bc1582ed820e8efb93bd633c173b3ccbf33ba5e8`.
The parent is byte-identical to the R3dp candidate and hashes to
`3e378f7260dac9b5a70155cfbad411f282f7584300f96ca4e0be887f4e6254a0`;
the candidate adds only these four authenticated paths and hashes to
`196325f0899f6d570f9974bdb0428e444f29f5a93d377173d392f33f18dc99b9`.

Admission is bound independently at coordinator and worker boundaries to the
canonical path, pinned source SHA-256, flags, ordered features, includes, and
negative metadata. Only the canonical four-path universe or `--all` is
accepted. `--test`, the R3dp 17- and 21-path manifests, an older byte-identical
four-path manifest, and unrelated manifests remain fail-closed.

The focused parent has eight `unsupported-host-agent` rows; the candidate has
eight passes. Exact TSV and JSON joins therefore record eight
unsupported-to-pass gains, no unchanged rows, and zero regressions. One
hundred one-worker Oxide runs produce 800/800 passes, while 32 eight-worker
runs produce 256/256 with byte-identical reports. Pinned QuickJS 2026-06-04
passes the same eight variants across 100 runs, totaling 800/800 oracle
passes. The gate replays the complete R3dp scoped gate as its predecessor.
These results are scoped admission evidence, not a global promotion or a
Feature Parity completion claim.

```sh
./scripts/test-test262-agent-fifo-wake-order.sh --check
```

## R3dp scoped agent wake/count/location admission

R3dp keeps the live global profile and runtime byte-for-byte outside this
runner-only admission. Its universe is the exact 21-path R3do retained
manifest, SHA-256
`76dc724e39d9eab3c707150ac5811712c543b71ab650339ba559e9a5429c7ea4`.
The disjoint activation and retained partitions contain 17 and four paths and
hash to `8502e6fa50a94a7e9eef34310535f29906c2d9b1eaa49e8fe0d9388fa0e4c4f4`
and `8e0fc31a034e1b76aff14e15bc1582ed820e8efb93bd633c173b3ccbf33ba5e8`.
The four retained paths are the FIFO-order cohort reserved for R3dq.

The parent profile is byte-identical to the R3do scoped candidate and hashes
to `9e20cfcb8b4b6f23116079b9ad12b823e1845688efbc1b81de97f9c28e2f5fb9`.
The candidate adds only the 17 authenticated host-agent paths and hashes to
`3e378f7260dac9b5a70155cfbad411f282f7584300f96ca4e0be887f4e6254a0`.
Both profiles accept only `--all` or the canonical 21-path universe. Selection
by `--test`, either 17/4 submanifest, an unrelated manifest, or the same bytes
under the R3do retained path is rejected. Coordinator and worker validation
bind admission to exact path, source SHA-256, flags, ordered features,
includes, and negative metadata.

The parent contains 42 `unsupported-host-agent` rows. The candidate contains
34 passes and eight byte-identical retained outcomes. Exact TSV and JSON joins
therefore record 34 unsupported-to-pass gains, eight unchanged rows, and zero
regressions. Candidate TSV/JSONL hashes are
`1b11c5f99c59c5e498eaf715f0bfa8d3c136b002a9856abe480d6867da145263`
and `9746ff7dacf2a65badd40518f1e4ee58dcf6a2b5acde0334306c1cd7b0517b53`.

Fifty one-worker replays are byte-identical, producing 1,700 activation passes
and 400 retained fail-closed outcomes. Twenty additional eight-worker pressure
runs produce 680 passes and 160 retained outcomes with the same report hashes.
Pinned QuickJS 2026-06-04 passes all 34 activation variants. The gate also
replays the complete R3do scoped receipt independently.

```sh
./scripts/test-test262-agent-wake-count-location.sh --check
```

## R3do global bounded agent-wait cohort A admission

R3do promotes exactly the scoped 22-path activation into the global
`[host-agent-tests]` allowlist. The R3dn parent profile hashes to
`f48a059f97fb7fdb1e2b883221756fa47343de3b4b06f85923eef81c3d98a955`;
the candidate/live profile preserves all feature, negative-test, and execution
sections, adds only those 22 paths, and hashes to
`8c80eee8846d3eaf08f1aa0622e0edc9a8290aa03c492eb25003f9c2dc8f4052`.

The global focused parent contains 86 `unsupported-host-agent` rows. The
candidate contains 44 passes and 42 unchanged unsupported rows. Its TSV/JSONL
hashes are
`230100c55ae8f957514d9a1ce238716b3707becbbd75276cd6cd541481bfc593`
and
`1953046aeeea710ba8de5f3f30293208e48306d1d88073d32f2fb495a13fa00b`;
the exhaustive transition hashes to
`7cf767db98f54af56f7bd50ced4634f3e9dcda9117805aab89ff367c597c12e1`.

Two independent native Linux release-mode full runs are byte-identical. Each
exact join changes only the 44 activation variants from
`unsupported-host-agent` to `pass`; all 101,993 outside-cohort TSV rows and JSON
result rows remain byte-identical, with zero detail-only movement and zero
previous-pass regression. The canonical vector is 66,552 passes / 66,604
runnable / 102,037 total variants, with 42 `unsupported-host-agent` outcomes.
Its TSV/JSONL SHA-256 values are
`9c90b6e0eef96583834b824e628238faa3f10e7304e68797bd55b62a63f3bcb5`
and
`a6560c2ad8060dc7ce9a7468c49277efba6be51a04bd769baf223d0ef72cc959`.

```sh
./scripts/test-test262-agent-wait-bounded-a.sh --check
./scripts/test-test262-agent-wait-bounded-a-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-agent-wait-bounded-a-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-agent-wait-bounded-a-global.sh --full
```

## R3do scoped bounded agent-wait cohort A

R3do admits no new global capability. It creates a scoped runner receipt for
the bounded timeout/no-spurious subset of R3dn's 43 retained agent paths. The
43-path universe hashes to
`6c723dcea7ff0f92b79b5d1f8218e74d209d0206a3e6c111f129ac4321a1497f`
and is byte-identical to the R3dn retained manifest. Its exact disjoint
partition is 22 activation paths hashing to
`239bcf25fa58c9081c7e4bc9bfd831862225691490ede631d585a99bd8995eb0`
and 21 retained paths hashing to
`76dc724e39d9eab3c707150ac5811712c543b71ab650339ba559e9a5429c7ea4`.
The activation source ledger hashes to
`79105013edd054a045fe16f3de55fe1b5fb233e373ac9052c1213f1c4bcea04d`.

The activation inventory consists of three zero-count notification tests,
nine Int32 and nine BigInt64 no-spurious-wakeup tests, and one true-timeout
coercion test. Every path has exactly `atomicsHelper.js`, empty flags, no
negative expectation, and its pinned ordered feature list. Admissions are
bound to exact path, source SHA-256, and that complete metadata shape. The
coordinator performs this validation while planning, and the isolated worker
rereads and revalidates the source and metadata before installing the agent
host.

The scoped parent is byte-identical to the R3dn broadcast candidate and hashes
to
`4f2a285a77e31815a94266ddcdafac7df9c8a148c0236be4e60968590999e706`.
The scoped candidate adds only the 22 activation paths and hashes to
`9e20cfcb8b4b6f23116079b9ad12b823e1845688efbc1b81de97f9c28e2f5fb9`.
Both profiles require either `--all` or the canonical 43-path manifest.
Selection by `--test`, either 22/21 submanifest, an unrelated manifest, or the
same universe bytes under the R3dn predecessor path is rejected.

The parent vector contains 86 `unsupported-host-agent` rows. The candidate
contains 44 passes and 42 retained unsupported rows. TSV and JSON result rows
join exactly on all 86 path/variant keys; activation changes are exactly 44
unsupported-to-pass gains, retained rows are byte-identical, and there are no
regressions. The parent TSV/JSONL hashes are
`50f600ce2bf37ca20e76e390a79d176ee819568517abf9bea577f7bb5aae19ab`
and
`ca343931edeb266a06dec6dba31dda24b1f97843a66cef2e998a470a94554f3d`;
the candidate hashes are
`31a867b732145c44e351e2691ae3ba0dc20e2efc5f3f1d0ce042d97cf83aaf12`
and
`904192d569373232dafddc3a7e1935ffb35c7d01b169b1baa933517ca6ce8eb3`.
The exhaustive transition hashes to
`37590a163b6a4d6acd0279d3faa05f37d3ff5bc194df1fb05d76b07220f6c33b`.

Twenty byte-identical single-worker candidate replays cover 880 runnable
variants with 880 passes and keep all 840 retained outcomes fail-closed.
Pinned QuickJS 2026-06-04 passes the same activation cohort 44/44. This scoped
milestone changes no runtime waiter semantics and initially left the live
profile at R3dn; the distinct global admission above subsequently advances
README, `compat/test262-oxide.conf`, and current-global receipts.

```sh
./scripts/test-test262-agent-wait-bounded-a.sh --check
```

## R3dn global agent broadcast cohort A admission

After the scoped implementation receipt below was frozen, R3dn promoted only
its 15 source- and metadata-authenticated paths into the global
`[host-agent-tests]` allowlist. The R3dm parent profile hashes to
`37cb029eda8e3abe97a17c93c1c3fe95e6aaed330de09d41b5941e9a6c3784f8`;
the candidate/live profile preserves all 132 feature tags, 1,197 audited
negative paths, and the execution policy, adds exactly those 15 agent paths,
and hashes to
`f48a059f97fb7fdb1e2b883221756fa47343de3b4b06f85923eef81c3d98a955`.

The global focused parent reports 116 `unsupported-host-agent` outcomes. The
candidate reports 30 passes and 86 unchanged unsupported outcomes. Its
TSV/JSONL hashes are
`18343ae78609b2787b9f977977a9c4258a2b74652dcaeba1466377bcc74ab173`
and
`2e951bf8c4bb8ad773bbeb244cb3cec940ebf4f0b7231d2b8c5da8d95f0d57e0`;
the exact transition hashes to
`8d43e059c9d4fe23ed30705bedcb823316bef9e816c1a6fdc4b3ca02eb789e3d`.

Two independent release-mode, two-worker complete runs are byte-identical.
Each exact join changes only the 30 activated variants from
`unsupported-host-agent` to `pass`: all 102,007 outside-cohort TSV rows and
JSON result rows are byte-identical, with no detail-only movement and no
previous-pass regression. The canonical vector is now 66,508 passes / 66,560
runnable / 102,037 total variants, with 86 `unsupported-host-agent` outcomes.
Its TSV/JSONL SHA-256 values are
`de511db69ffd4a3912487251a6d1a7b2327b649464dde53ae590a04ab0212f86`
and
`6ad3f1e044d105fadad35be0626f004041d0566f7dd3eb021d7e434d37ef2363`.

```sh
./scripts/test-test262-agent-broadcast-a.sh --check
./scripts/test-test262-agent-broadcast-a-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-agent-broadcast-a-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-agent-broadcast-a-global.sh --full
```

## R3dn scoped agent broadcast cohort A

R3dn stages the first authenticated `broadcast` / `receiveBroadcast` slice
without changing the live global profile. Its 58-path universe hashes to
`bb59fba98ce4d41426a47de67630940f4eae29927421a2bfe6e1ea70c8f56d55`
and is exactly partitioned into a 15-path activation manifest hashing to
`8ca17690ee6e1fd5b4deb6e41047925e5b1d5a3cd0dfcd4f5ccdaeb04b336f23`
and a 43-path retained manifest hashing to
`6c723dcea7ff0f92b79b5d1f8218e74d209d0206a3e6c111f129ac4321a1497f`.
The activation path/source ledger hashes to
`b467b2cdca29ad877981b7894e5b28bdf966385034aa5e722d9d81b86b19c0cf`.

The scoped parent and candidate profiles hash to
`7254bdc5a52a30f70f270bdabc9337231c94799d636f993ba54b8ae082915dea`
and
`4f2a285a77e31815a94266ddcdafac7df9c8a148c0236be4e60968590999e706`.
They differ only by the 15 authenticated host-agent paths. The runner accepts
the profiles only with the exact 58-path manifest or `--all`, and both the
coordinator and isolated worker revalidate exact path, pinned source SHA-256,
and complete metadata before installing the agent host.

The parent scoped vector contains 116 `unsupported-host-agent` outcomes. The
candidate contains 30 passes and 86 unchanged unsupported outcomes, yielding
30 pass gains and zero regressions. Its TSV/JSONL SHA-256 values are
`bf3524122b78ed31d931f3e28df2d930ba0308cfb6ed33a144a51caa2ff7d457`
and
`8acfbeb71c2c3ae70b8e1677601eb2e5b718c877fd8d0c0fffd38778907d4a5b`.
Twenty deterministic single-worker replays cover 600 activated variants with
600 passes; pinned QuickJS passes the same 15 paths in both modes, 30/30.

This admission is deliberately limited to fixed `SharedArrayBuffer` backing.
Ordinary `ArrayBuffer`, growable shared buffers, and the 43 wake-order,
timeout, and FIFO paths remain fail-closed pending their own audits. The global
admission above preserves that boundary and promotes only this exact cohort.

```sh
./scripts/test-test262-agent-broadcast-a.sh --check
```

## R3dm exact Test262 agent Stage A admission

R3dm turns the authenticated `$262.agent` frontier into an exact, fail-closed
global admission. The 132-feature R3dl parent profile hashes to
`47cf8351f7844340bbbff3ba9bb781faf552f8f27d0dd6cca2e35dbf9ad48232`.
The candidate/live profile changes no feature, audited-negative, or execution
entry; it adds only a `[host-agent-tests]` allowlist containing
`test/built-ins/Atomics/wait/good-views.js` and hashes to
`37cb029eda8e3abe97a17c93c1c3fe95e6aaed330de09d41b5941e9a6c3784f8`.
The runner accepts those profiles only with the exact 59-path universe
manifest or `--all`, and it independently binds the activation path to its
pinned source SHA-256 and metadata shape.

The universe, activation, and retained manifests hash to
`39774992fb157df3676b53c4c001c7f9cb60ca546309b0e8be18ff3ac9737151`,
`cc8da184af01572cb83fc743f062fb54c17124663358532ccb2216587e62bb58`,
and
`bb59fba98ce4d41426a47de67630940f4eae29927421a2bfe6e1ea70c8f56d55`.
Their exact partition is one activation path / two variants and 58 retained
paths / 116 variants. The parent focused receipt reports 118
`unsupported-host-agent` outcomes; the candidate reports two passes and 116
unchanged unsupported outcomes. The global focused TSV/JSONL hashes move from
`dfc09335a5a485e74844b5e4d701f601e4b5b73ed0dd1b4f863f5320a411029c`
and
`067c21dd84b54b4f4462d93261383dba0247490e311e552182e9f2c6a4b844ea`
to
`f2d934c0a46e17f9c4c5bc79fae91822df2c7dc619df96123b99180ee0ceba11`
and
`a0cf03718c3e869bdc96b1821642316b15083cdee98eb52b8efaf49c5031d70e`.
The exact transition records two pass gains, 116 unchanged rows, and zero
regressions. A separate 20-run activation receipt is byte-identical on every
run, and pinned QuickJS passes both activation variants.

The complete join changes only those two rows. It reaches 66,478 passes /
66,530 runnable / 102,037 total variants and reduces
`unsupported-host-agent` from 118 to 116; every other outcome count is
unchanged. Two independent release-mode full runs are byte-identical. The new
canonical TSV/JSONL SHA-256 values are
`a05aed38d47216ca485334ad50656cbe3ddf8d5c9922a6eaf28e0ee9ff0863dc`
and
`4f0ff98da92582ae37571d754e6608b20aa707c0c1e456232b527e778b87e9c0`.

The implemented host uses a fresh `Runtime` and `Context` per native agent,
2 MiB stacks, FIFO reports, and start-order cleanup joins. It intentionally
keeps `broadcast` and `receiveBroadcast` fail-closed, so the retained 58 paths
remain the next agent-parity frontier rather than being folded into the green
result. Native agent threads are unavailable on WebAssembly, and
`Atomics.waitAsync` remains outside the pinned QuickJS target.

```sh
./scripts/test-test262-agent-stage-a.sh --check
./scripts/test-test262-agent-stage-a-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-agent-stage-a-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-agent-stage-a-global.sh --full
```

## R3dl global SharedArrayBuffer and Atomics admission

R3dl globally admits the implemented `SharedArrayBuffer` and `Atomics` tags.
The exact parent has 130 reviewed feature tags and hashes to
`280264ae035da45cd0e2727b981e64380496ed75af3216208616dfee82d0459a`;
the 132-tag candidate/live profile adds only those two tags and hashes to
`47cf8351f7844340bbbff3ba9bb781faf552f8f27d0dd6cca2e35dbf9ad48232`.

The authenticated universe contains 445 paths / 886 sloppy-and-strict
variants. Its exact partition is 435 activation paths / 866 variants, six
already admitted `Atomics.pause` paths / 12 variants, and four retained
cross-realm paths / eight variants. The parent records 12 passes and 874
`unsupported-feature` outcomes; the candidate records 878 passes and eight
cross-realm `unsupported-feature` outcomes. The join therefore has 866 pass
gains, eight diagnostic-only changes, 12 unchanged rows, and zero regressions.
Pinned QuickJS 2026-06-04 passes all 886 variants.

The complete join changes only those 874 rows. It reaches 66,476 passes /
66,528 runnable / 102,037 total variants and reduces `unsupported-feature`
from 13,627 to 12,761; every other outcome count is unchanged. Two independent
release-mode full runs are byte-identical. The canonical TSV/JSONL SHA-256
values are
`501b64ed5c8367f33408225d956a262619163adf52baadf28f02811d14f3eae9`
and
`610e16ba65a0239556842efec7a745ba2885c72dfb3b8447c2578b8767ef7d40`.

This admission leaves the Test262 `$262.agent` host as the next shared-memory
frontier: its exact 59-path / 118-variant cohort remains classified as
`unsupported-host-agent`, while pinned QuickJS passes all 118. The pinned
QuickJS release has no `Atomics.waitAsync`, so that API remains outside the
parity target.

```sh
./scripts/test-test262-shared-atomics-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-shared-atomics-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-shared-atomics-global.sh --full
```

## R3dj exact bounded non-agent Atomics.wait implementation

R3dj derives the synchronous non-agent boundary from the complete pinned suite,
not from the `SharedArrayBuffer` tag or the `Atomics/wait` directory. Across
53,393 JavaScript files, 93 paths contain a raw `Atomics.wait` member reference.
The R3di-equivalent code-token scanner finds 35 executable member-reference
paths: 33 direct-call paths plus the non-call references in `length.js` and
`name.js`. The full raw partition is 33 selected paths, 57 agent paths, and the
three descriptor/length/name metadata paths. No selected path overlaps the 101
raw `Atomics.waitAsync` paths, and every raw path maps back to the authenticated
Atomics ledger.

The exact 33-path / 66-variant manifest consists of the R3dh-tagged 20 / 40 and
a disjoint source spillover of 13 / 26. The spillover includes two upstream
files under `Atomics/notify` that actually invoke `Atomics.wait`. The combined
manifest, source projection, and sloppy/strict key hashes are respectively
`38f69242c52bfda864397a6413dedad9eb3a60ca2c07683f857791300948348d`,
`42d1a6f2f80512985a3d893c306bbe0914da07869ad9270391c1a3b7be2b2033`,
and
`274d406bae7a821f3e48a8ac2d8d49a8eae98dbfc04633127c66bc05ae546558`.
Pinned QuickJS passes all 66 variants.

Oxide now passes all 66 variants. Its deterministic 77-line TSV and 68-line
JSONL hash to
`b90662c8814a1e3db00338aadb84731d0721e349c9b2a76ddeb0b583cb0d667a`
and
`4845a5629ecdc6b26b3e9ea2724cee1215a07903dc9b5139f76406627ee5bf6d`.
The scoped profile accepts only the exact combined manifest and rejects
`--all`, `--test`, tagged-only, spillover-only, and other manifests. The gate
also runs the native waiter and host-policy tests before rebuilding the
current-worktree release runner. The canonical global vector remains 65,610
passes / 65,662 runnable / 102,037 total. Implementing the false host policy
reclassifies four still-feature-blocked rows from
`unsupported-host-can-block-false` to `unsupported-feature`; the current full
TSV/JSONL hashes are
`17370398c6a211d4657ad763a6e40f0cd198d72faa14b2995f7937ad52a0c6db`
and
`6e12d86318b2f1d7e5f684962a02585b1a91a4d7830d6e05ed38f80c766cc9a1`.

This remains a bounded receipt. Seven paths exercise zero- or one-millisecond
timeouts and 26 finish before blocking; none requires an infinite wait,
`not-equal`, notification, or agent wakeup. Native tests separately cover
those internal waiter branches, including a cross-runtime wakeup, but they do
not replace the 57 excluded `$262.agent` paths.

Pinned QuickJS 2026-06-04 has no `Atomics.waitAsync`; Test262 marks that feature
as skipped in its QuickJS configuration. Accordingly waitAsync is outside the
parity target, while the Test262 agent host and agent-backed waiter behavior
remain a real future parity frontier.

```sh
TEST262_WORKERS=8 ./scripts/test-test262-atomics-wait-nonagent-bounded.sh --check
```

## R3di exact non-blocking shared Atomics selection

R3di authenticates the complete pinned non-blocking shared Atomics closure
without admitting a broad feature tag. The `SharedArrayBuffer`-tagged
projection contains 78 paths / 156 variants. A source audit finds another 22
paths / 44 variants which exercise the same implemented operations with real
shared backing even though their metadata omits that tag. Their disjoint union
is the exact 100-path / 200-variant selection.

The source closure itself has 99 paths and overlaps the tagged projection in
77. The tagged-only path is a metadata-only `isLockFree` check. That scoped
audit found one test filed under `Atomics/notify` which actually calls
`Atomics.wait`; R3dj's full-suite closure above finds both such misfiled paths.

The selection covers shared `load`, `store`, `add`, `sub`, `and`, `or`, `xor`,
`exchange`, and `compareExchange`, plus adjacent `isLockFree` table checks and
`notify` validation when there are no registered waiters. Pinned QuickJS
2026-06-04 independently passes all 200 variants with no failure or feature
skip. Synchronous `wait`, `waitAsync`, agent cases, and host-blocking behavior
remain outside the selection.

The R3di profile is selection-only and is accepted only with the exact
combined manifest; the runner rejects `--all`, `--test`, and a different
manifest. It does not add `SharedArrayBuffer` or broad `Atomics` to the global
capability profile. The canonical complete vector therefore stays at 65,610
passes / 65,662 runnable / 102,037 total variants.

Oxide passes all 200 selected variants with no other outcome. Two focused runs
are byte-identical. The authenticated 211-line TSV has SHA-256
`e265924c5773626f73f5396803a8b3e19e5650bad49efe04a390dfa77b86548a`;
the 202-line JSONL has SHA-256
`9a012e1be03b8f752efc15a1250548388c1c0c41f6443e44ec8be4f98842fb34`.
The 100-path combined manifest hashes to
`a9072513df3c730b87a84218a88755c229a8090e0100bc44bbd7d2550ac72dc0`,
and the scoped profile hashes to
`ec33455551c3601859241870624b5017551aa04c8edbf8c9e899d4ef9b5332cc`.

Reproduce the checksum-bound inventory, fail-closed selection checks, focused
Oxide run, and pinned QuickJS differential with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-shared-atomics-nonblocking.sh --check
```

Because this is an implementation receipt, the gate rejects
`TEST262_RUNNER` overrides and builds the release runner from the current
worktree before producing the authenticated reports.

## R3dh authenticated SharedArrayBuffer core

The complete pinned metadata universe contains 463 paths / 922 variants with
the `SharedArrayBuffer` feature. Its checksum-bound ledger records each path's
category, variants, includes, flags, features, host requirements, pinned
QuickJS disposition, and source SHA-256. The exact no-Atomics core is 221 paths
/ 438 variants and its manifest hashes to
`160a70bf9ebd5695f582a9100d09db7df930e9001b592edd0f269fe434c4893c`.

The R3dh profile is selection-only and is accepted only with that manifest;
the runner rejects `--all`, `--test`, and a wider manifest. Oxide passes all
438 exact-core variants. The authenticated focused TSV has SHA-256
`03f445aa2978b001a7737bbd482e9b36d35182471b71961fa273d916d24450d8`;
the JSONL has SHA-256
`b4d7ff88f0f9480eb81b068cdcaedcf3667aca70ab82e830d5ef7e5aafc01ad1`.
Pinned QuickJS 2026-06-04 independently passes the same 438 variants with no
failure or feature skip.

The other 242 paths / 484 variants remain visible rather than being folded
into the green result:

- 78 paths / 156 variants cover non-blocking shared Atomics;
- 20 / 40 cover synchronous `Atomics.wait` without agents;
- 58 / 116 require agents, excluding `waitAsync`;
- 86 / 172 cover `Atomics.waitAsync`, including its agent cases.

R3dh did not globally admit the `SharedArrayBuffer` tag. The live global
profile therefore remains at 130 tags and the canonical complete vector stays
65,610 passes / 65,662 runnable / 102,037 total variants. This focused pass is
evidence for the implemented constructor, grow/slice/species, and shared-view
slice; it is not evidence for the R3di shared Atomics slice, agents, waiters,
or `waitAsync`, and it is not a Feature Parity claim.

Reproduce the authenticated inventory, fail-closed selection checks, Oxide
receipt, and pinned QuickJS differential with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-shared-array-buffer-core.sh --check
```

## R3dg implemented leaf built-in admission

R3dg adds `Error.isError`, `RegExp.escape`, and
`TypedArray.prototype.at` to the global capability profile without changing
runtime semantics. The resulting 130-tag candidate/live profile hashes to
`280264ae035da45cd0e2727b981e64380496ed75af3216208616dfee82d0459a`.

The exact cohort is 47 paths / 94 sloppy-and-strict variants. Oxide's
candidate produces 86 passes and retains eight `unsupported-feature`
diagnostics: two for the missing `class` prerequisite and six for cross-realm
dependencies. Pinned QuickJS passes all 94 variants.

The full transition joins all 102,037 variants. Exactly 94 manifest rows
change: 86 outcome changes to pass and eight diagnostic-detail-only changes;
zero rows outside the manifest change and there are zero pass regressions. At
R3dg, the canonical result became 65,610 passes / 65,662 runnable, with
`unsupported-feature=13,623`. Candidate full TSV/JSONL SHA-256 values are
`a3b097fe77a996bc1272a9576c39f509c60ee9c3644e667ab4f0d4c141f72e32`
and
`dc37ed90322630e81fa4295daa57b8f81093719541076f84d4da27ef0d3c5d23`.

Reproduce the authenticated transition with:

```sh
./scripts/test-test262-error-regexp-typedarray-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-error-regexp-typedarray-global.sh
TEST262_REUSE_FULL_REPORTS=true \
  TEST262_FULL_WORKERS=8 \
  ./scripts/test-test262-error-regexp-typedarray-global.sh --full
```

## R3df global `Atomics.pause` admission

R3df moves the complete pinned `Atomics.pause` tag into the live capability
profile. The 126-tag parent profile hashes to
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`;
the 127-tag candidate/live profile adds only `Atomics.pause` and hashes to
`00265570870a778f2fded16969311eac5707b9c6d4fcd4068640700d637e9ff0`.
The runner binds both identities to an exact transition contract rather than
accepting arbitrary files.

The six-path manifest hashes to
`72252a61f2d3c97626b544a1ac1a2a31191149a535227b16a8fa798c91e0d69c`.
All 12 sloppy/strict parent rows are `unsupported-feature`; all 12 candidate
rows pass, as do all 12 variants in pinned QuickJS. The focused transition has
12 outcome changes, zero detail-only changes, and zero regressions; its whole
TSV hashes to
`d8a49b050c5dd0a665008576d34f6968654c28cc32af146e0993edac59e1fdee`.

A fresh release-mode run with two workers authenticates the complete 102,037
variant join. The 12 manifest rows become passes and all 102,025 other rows
remain byte-identical. The canonical result is 65,524 passes / 65,576 runnable,
with `unsupported-feature` reduced from 13,721 to 13,709. Candidate full
TSV/JSONL SHA-256 values are
`205ec5ef4ec03dfea59a8ff424e776406a83c1bf0c4070e68f42127331f0e6aa`
and
`627f4ccdea5825f382d9d5500a4e578fa5b38cf5bd7422525d8fb19b48065e86`.
No other outcome count changes. This admission does not claim SharedArrayBuffer,
agent, waiter, or `Atomics.waitAsync` support.

Reproduce the authenticated transition with:

```sh
./scripts/test-test262-atomics-pause-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-atomics-pause-global.sh
TEST262_REUSE_FULL_REPORTS=true \
  TEST262_FULL_WORKERS=2 ./scripts/test-test262-atomics-pause-global.sh --full
```

## R3de exhaustive non-shared Atomics gate

R3de replaces the temporary source probe with a runner-authenticated scoped
profile and exact manifest. The source audit now exhausts the non-shared
portion of the pinned metadata universe: 96 metadata paths neither evaluate
`SharedArrayBuffer` nor carry its tag, one safe `isLockFree` path carries the
tag without evaluating SAB, and one metadata-less SpiderMonkey detached-
buffer path supplements them. Oxide and pinned QuickJS both pass all 98 paths
/ 196 sloppy/strict variants.

The manifest SHA-256 is
`5c8805da455cb66810646a709d847346c1c07b2710b46838da6006667f627aac`;
the selection-only profile SHA-256 is
`c3db1670b6cd4e2b9b1e7bd812d2e580df4ea0d8f0ceee96c074378d14dc9a5b`.
The runner binds those two exact files and rejects broad, single-test, or
alternate-manifest selection. Against the unchanged global parent profile, all
196 focused rows move from `unsupported-feature` to `pass`; no row outside the
manifest can enter this gate. Candidate TSV/JSONL SHA-256 values are
`0213b1e6484568dc2d350c8f551a62ad621fb4a9925b59134a80f9d77ff1d05a`
and
`d0c37816b7b0a8426eb9242e0df5ea27a92e9d877b0f345c21f18d6e52fb5c1d`.
The 382-row / 764-variant ledger also freezes each metadata path's category,
includes, flags, features, and source SHA-256. The gate regenerates all 53,125
pinned metadata records, derives the exact `Atomics` / `Atomics.pause`
projection, and verifies the ledger plus its source and category projections.

The disjoint 12-path shared-deferred manifest remains unchanged at SHA-256
`00b82b9589391b350ee77ee736c7e7c4637c19466465b4dfa4e53270cdbc02ee`.
Pinned QuickJS passes its 24 variants. At R3de, Oxide had no shared
backing-store implementation, and that gate did not execute this deferred
partition in Oxide. It prevents the frontier from being folded into the green
non-shared cohort, but is not an Oxide result receipt or an exhaustive SAB
inventory.

The exhaustive audit covers the 382 mutually exclusive paths carrying
`Atomics` or `Atomics.pause` metadata. It is not the larger universe of every
`SharedArrayBuffer`-only path:

- 96: no SAB evaluation and no SAB feature tag;
- 1: SAB metadata only, with no SAB evaluation;
- 123: real SAB evaluation and no additional missing host requirement;
- 61: real SAB evaluation plus a host requirement (59 agent paths and two
  `CanBlock`-false paths);
- 101: `Atomics.waitAsync`.

The 184 non-`waitAsync` shared paths comprise 174 direct SAB constructions and
ten calls through the pinned `testAtomics.js` non-view helper.

The metadata-less detached-buffer and cross-compartment staging fixtures sit
outside that count. The former is the 98th green path; the latter evaluates
real SAB and also needs realm support.

R3df above admits the independent six-path / 12-variant `Atomics.pause` slice
through its own exact global transition. Broad `Atomics` remains deferred. Its raw metadata
closure is 119 paths / 238 variants: 90 / 180 are green and 29 / 58 hide real
SAB or waiter dependencies. The runner's host-before-feature precedence swaps
one member: `wait/good-views.js` is already host-preempted, while the
metadata-less detached-buffer fixture has a supplemental `Atomics`
requirement. The resulting precedence-aware transition planning set remains
119 / 238, partitioned into 91 green paths / 182 variants and 28 hidden-shared
paths / 56 variants. This is not yet a candidate transition report. A later
broad gate must execute and freeze that set or implement the missing runtime,
then account for reason-only rows. Neither namespace admission implies
`SharedArrayBuffer`, agent, waiter, or `Atomics.waitAsync` support.

At R3de, `compat/test262-oxide.conf` did not move and the complete report stayed
byte-identical to R3dc at 65,512 passes, 65,564 runnable variants, seven parse
failures, 43 runtime failures, and two timeouts. That report is now R3df's
authenticated parent, not the current canonical vector.

Reproduce the authenticated focused receipt with:

```sh
./scripts/test-test262-atomics-non-shared-core.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-atomics-non-shared-core.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-atomics-non-shared-core.sh --full
```

## R3dd source-audited non-shared Atomics cohort

R3dd adds the runtime namespace but deliberately does not enable broad
`Atomics` metadata in the global profile. A first 102-path audit exposed a
selection mistake: twelve tests really evaluate `SharedArrayBuffer`, including
eleven method `not-a-constructor` paths whose argument expressions are
evaluated before the constructor check. Those paths are now frozen in the
12-line shared-memory deferred manifest instead of being mislabeled green.

The historical R3dd temporary-probe manifest contained 90 paths / 180 variants
and then hashed to
`e9ab48b9faa090e1bc2a58a1d62e2398bca0de88a28f34c53d3397442636a380`.
That snapshot comprised 43 namespace/method metadata paths, 41 explicitly
named non-shared paths, five remaining `pause`/`isLockFree` semantic paths,
and the staging detached-buffer path. Oxide and pinned QuickJS passed 180/180.
R3de above supersedes it with the current 98-path manifest; the historical
candidate scoped-probe TSV/JSONL SHA-256 values were
`0d5b99acb171c079d91b89ca010c9061b2b552d1a1dfe530efaa554caa2335d4`
and
`baaf530b6697390a82e2751411b6cbfd7fa84dbb2c890248af37f9b06836a05f`;
the pinned QuickJS log SHA-256 is
`7a033067036e950e1dd60e7fa91a98d7b2ed51a0a6ce0c0eeec84895d531f6d9`.

The deferred manifest hashes to
`00b82b9589391b350ee77ee736c7e7c4637c19466465b4dfa4e53270cdbc02ee`.
At R3dd, the 90-path snapshot and deferred manifest reconstructed the original
102-path audit without erasing the SAB frontier. One green `isLockFree` path
conservatively carried SAB metadata without evaluating SAB, so the temporary
scoped profile existed only to select that audited cohort. It was not a global
capability declaration. R3de superseded that scoped probe, and R3df now
authenticates both as historical predecessors of the live global profile.

## R3dc Atomics metadata-gap classification

R3dc corrects the classified vector without adding runtime functionality. Two
SpiderMonkey Atomics staging fixtures omit feature metadata. Both supplemental
rules are now exact-path and exact-source-SHA bound: the cross-compartment test
requires `Atomics`, `SharedArrayBuffer`, and the already implemented realm
host, while the detached-buffer test requires only `Atomics`. Source drift is
a hard coordinator error. This prevents an absent `%Atomics%` intrinsic from
being counted as an actionable runtime failure merely because the detach host
is available.

The exact manifest contains two paths / four variants and hashes to
`4863dea8db26a20638b24f6a727a0a7f0a207585a4b966a855f10fa3ea1fcb18`.
Pinned QuickJS passes all four. The authenticated R3db parent records
`fail-runtime=2 unsupported-feature=2`; the candidate records
`unsupported-feature=4`. Candidate focused TSV/JSONL SHA-256 values are
`3eb9e15b57371dc9d8e6b6c89edc4bb62074ef893850b0d8a6c8b7d0da5d41c5`
and
`fbd94ab0292664901f42639050d14d4da273d4a1cab66588007f0de30ec224d4`.
There are no pass changes.

The fresh two-worker full transition keeps 65,512 passes, moves runnable from
65,566 to 65,564, reduces `fail-runtime` from 45 to 43, and increases
`unsupported-feature` from 13,719 to 13,721. Exactly two outcomes change; the
other two cohort rows and all 102,033 non-cohort rows are byte-identical, with
no detail-only change or pass regression. Candidate whole-corpus TSV/JSONL
SHA-256 values are
`35c329c649ecb75ec473bdaa42b361ad1173025893588f47f41a0270112872f1`
and
`f2811b3b7724123d8cb4a1b81c470f6c0b1f5f4c74d8ee26c76856c0c065861f`.
This was the R3dc/R3de canonical vector and is now R3df's authenticated parent.

Reproduce the frozen inputs and focused receipts with:

```sh
./scripts/test-test262-atomics-metadata-gaps.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-atomics-metadata-gaps.sh
```

Reproduce the complete join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-atomics-metadata-gaps.sh --full`.

## R3db sloppy direct-eval var BindingPattern references

R3db freezes QuickJS's late scope selection for sloppy direct-eval `var`
BindingPatterns. The eval declaration prelude already creates a novel name on
the caller's eval-variable object. Its Reference must nevertheless retain that
object as a late candidate until the final Set: getters, iterator steps,
defaults, and rest-copy callbacks may delete the binding and force the write to
the global fallback. The old global-Reference shortcut bypassed the late
candidate even when it still existed, leaking ordinary destructured values to
the realm global. The shared shortcut now requires an empty `late_sources`
list. A pinned QuickJS matrix covers the normal and delete/retarget branches,
NamedEvaluation, repeated eval, and exact property/iterator ordering.

The exact manifest contains one path / two sloppy-and-strict variants and
hashes to
`cdaad046146fc09292816cd7638ab2b3e8e9f41778f2b459ec8a7fab93b338ed`.
Pinned QuickJS and the candidate pass both. The authenticated R3da parent
records `fail-runtime=1 pass=1`; the candidate records `pass=2`. Candidate
focused TSV/JSONL SHA-256 values are
`8f6e7e62dbf384d3da4d35b490ad637446c26e2a57488d4d41e05b155c128ccb`
and
`622cbe1eac81740d7cd71acdf2a589aae8f52b14a361ca2c48899c7532888965`.

The fresh two-worker full transition moves the complete vector to 65,512
passes / 65,566 runnable, retains `fail-parse=7`, reduces `fail-runtime` from
46 to 45, and leaves the strict cohort row plus all 102,035 non-cohort rows
byte-identical. It has no detail-only movement or previous-pass regression;
the two JSON mega-array variants still time out. Candidate whole-corpus
TSV/JSONL SHA-256 values are
`9cfd1c1f807b10581b2964e9a6d48a3fd4cbc92ebbecf15d359a9a21fc55680e`
and
`bf0755551c28dec28cc180a492512849faeb4aeae068202b185d041493d6c0c0`;
this is now the canonical full vector.

Reproduce the frozen inputs and focused receipts with:

```sh
./scripts/test-test262-eval-var-destructuring.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-eval-var-destructuring.sh
```

Reproduce the complete join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-eval-var-destructuring.sh --full`.

## R3da synchronous generator delegation stack budget

R3da freezes the first globally admitted repair for deep synchronous `yield*`.
Oxide already checked the real host-stack address before native calls, but its
secondary weighted guard classified every generator resume as an unknown
callback-capable native. That default cost rejected a ten-level chain before
the address guard. Synchronous generator resumes now retain a one-unit mixed
recursion charge while the real stack guard remains authoritative. Native and
Node/WASM tests prove successful finite delegation, catchable overflow,
complete frame unwinding, and recovery; a pinned QuickJS differential covers
result identity and `next`/`return`/`throw` propagation.

The exact manifest contains one path / two sloppy-and-strict variants and
hashes to
`3f4494005a5d8089fd9a9063aed01bed2b408bc9ae119043606a33aa82d400dc`.
Pinned QuickJS passes both. The authenticated R3cz parent records
`fail-runtime=2`, while the candidate records `pass=2`. Candidate focused
TSV/JSONL SHA-256 values are
`9c6c195196450e147231924d1ec548e6c2257c42ed062b26cd3fad5753a92f46`
and
`de4c08027b27b12aa0acd00a7bc5ab386dbe218dfb11ef59a48048da4dcc4718`.

The fresh two-worker full transition moves the complete vector to 65,511
passes / 65,566 runnable, retains `fail-parse=7`, reduces `fail-runtime` from
48 to 46, and leaves the other 102,035 rows byte-identical. It has no
detail-only movement or previous-pass regression; the two JSON mega-array
variants still time out. Candidate whole-corpus TSV/JSONL SHA-256 values are
`b97744b88f1a46727b1073559d0640a09b61a9e0a32703dccc062f2d61387001`
and
`b28b8db0e45ba299ab2cc60e4b12f88856f864b8b3afd54c15d6f8c7e9f857d7`;
this is now the canonical full vector.

Reproduce the frozen inputs and focused receipts with:

```sh
./scripts/test-test262-generator-yield-star-stack-budget.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-generator-yield-star-stack-budget.sh
```

Reproduce the complete join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-generator-yield-star-stack-budget.sh --full`.

The optimized native stack threshold is not yet QuickJS-equivalent (about 72
delegated levels versus 509 in the pinned oracle probe). That requires a future
VM resume trampoline and is not claimed by this receipt.

## R3cz class-field initializer await context

R3cz freezes QuickJS 2026-06-04's lexer-context boundary for synthetic class
field initializer functions. Public/private, instance/static initializers are
strict normal-method children which preserve the parent's module grammar
parameter but do not inherit an enclosing function's async/generator flags.
Computed keys are parsed before that child boundary and therefore retain the
enclosing context. This lets raw `await` remain an IdentifierReference in a
Script field initializer nested in an async function without weakening the
computed-key, static-block, or Module early errors. Compiler tests cover the
neighboring diagnostic boundaries, and a sloppy/strict runtime matrix matches
pinned QuickJS across all four field shapes, escaped `await`, a synchronous
arrow, and an async computed key.

The exact manifest contains one path / two sloppy-and-strict variants and
hashes to
`beea6c8fc86db377966dbe2454b23ef7c227bf07f66661d676b4cb1f323e7c3a`.
Pinned QuickJS passes both. The authenticated R3cy parent records
`fail-parse=2`, while the candidate records `pass=2`. Candidate focused
TSV/JSONL SHA-256 values are
`9312266f78a2734f1d83349c0d6d264b0eb1098ea8a1e921cf23ad49e895bafd`
and
`40a12b019d865c41f066d0c5f7330cabcd551604797e82bb6f2a3c15e5d00087`.
The focused transition changes two outcomes, has no detail-only movement, and
records no previous-pass regression. The live profile remains byte-identical
at SHA-256
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`.

The fresh two-worker full transition moves the complete vector to 65,509
passes / 65,566 runnable, reduces `fail-parse` from 9 to 7, retains
`fail-runtime=48`, and leaves the other 102,035 rows byte-identical. It has no
detail-only movement or previous-pass regression; the two JSON mega-array
variants still time out. Candidate whole-corpus TSV/JSONL SHA-256 values are
`e2c3127f1d07909579e0f9cab108b70ebdaf5555646bd47cd2c1d63768ec6c1e`
and
`c2d3379b16f6a39a99a1ba6f2d93d26b383dce1c287f8482517e2179546bdd1c`;
this is now the canonical full vector.

Reproduce the frozen inputs and focused receipts with:

```sh
./scripts/test-test262-class-field-await.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-class-field-await.sh
```

Reproduce the complete join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-class-field-await.sh --full`.

## R3cy Math.atanh numerical parity

R3cy replaces the previous `f64::atanh` call with a QuickJS-compatible,
fdlibm-shaped evaluation. Tiny inputs below `2^-28` return directly; other
finite inputs use separate `|x| < 0.5` and near-one `log1p` formulas on the
positive magnitude before the sign is restored. This preserves signed zero,
the `+/-1` infinities, and the NaN domain while avoiding the large error of the
previous expression for negative inputs close to -1. Rust unit tests cover the
branch and domain boundaries, and the focused Math differential checks the
accuracy and special-value behavior against pinned QuickJS.

The exact manifest contains seven paths / 14 sloppy-and-strict variants and
hashes to
`ffd98f946fde17f8a0af13c9dd172c8aa2c476e96baaa9df86ae42ee5479b215`.
Pinned QuickJS passes all 14. The authenticated R3cx parent records
`pass=12 fail-runtime=2`, while the candidate records `pass=14`. Candidate
focused TSV/JSONL SHA-256 values are
`03129f451be73355a0b33d6d74930e63bea0a1a9f001a5a8c524b6654f761140`
and
`d7cd0e97acb5dcda64378dccff535c1cfae6271ed4cbd448d65737894b1d57c8`.
The focused transition changes two outcomes, has no detail-only movement, and
records no previous-pass regression. The live profile remains byte-identical
at SHA-256
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`.

The fresh two-worker full transition moves the complete vector to 65,507
passes / 65,566 runnable, retains `fail-parse=9`, reduces `fail-runtime` from
50 to 48, and leaves the other 102,035 rows unchanged. It has no detail-only
movement or previous-pass regression. Candidate whole-corpus TSV/JSONL
SHA-256 values are
`9009145c5b7033c4b4392022f97c73ab62efe4f78c4085e6b76a48f89a34ad76`
and
`edcd4d53c03e09c447eed001d0033a36ce85e0a2b510b63e0eedec9066c44e60`;
this is now the canonical full vector.

Reproduce the frozen inputs and focused receipts with:

```sh
./scripts/test-test262-math-atanh.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-math-atanh.sh
```

Reproduce the complete join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-math-atanh.sh --full`.

## R3cx for-of async member lookahead

R3cx freezes QuickJS 2026-06-04's raw, non-committing
`simple_next_token(..., FALSE)` lookahead for an ordinary for-of assignment
target. The probe skips ordinary whitespace and JavaScript comments but does
not perform a normal lexer pass: Annex B HTML comments remain visible to the
real lexer, and a backslash after raw `of` retains QuickJS's scanner boundary.
Only bare, unescaped `async` followed by that probe is rejected. Legal member
targets such as `async.x`, `async["x"]`, and `async.of` continue through normal
left-hand-side parsing. Compiler canaries cover block/line comments and
newlines, escaped and parenthesized `async`, raw `of\u0061`, both HTML-comment
forms, and invalid call/optional-chain targets; the QuickJS differential covers
accepted member and HTML-comment behavior. For-await retains its pinned
behavior.

The exact manifest contains one path / two sloppy-and-strict variants and
hashes to
`a4d8c570908bb500728aca7dad45b0e064d9f43394e5d8e9bece95be74bc40a5`.
Pinned QuickJS passes both. The authenticated R3cw parent records
`fail-parse=2`, while the candidate records `pass=2`. Candidate focused
TSV/JSONL SHA-256 values are
`1f1a5c30dde9ede5f58635ec2d3a15396dc988c9b2c378f5bd5db4fc6135a3e6`
and
`d343db38f5fdfc8ffc46b2a06acd04507e58692ab3879ea3451a6b5f3e9b5cc4`.
The focused transition changes two outcomes, has no detail-only movement, and
records no previous-pass regression. The live profile remains byte-identical
at SHA-256
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`.

The fresh two-worker full transition moves the complete vector to 65,505
passes / 65,566 runnable, reduces `fail-parse` from 11 to 9, retains
`fail-runtime=50`, and leaves the other 102,035 rows byte-identical. It has no
detail-only movement or previous-pass regression. Candidate whole-corpus
TSV/JSONL SHA-256 values are
`687eec42e9611a377b37f68aa61cba263d2e8fe0dcf66d19b003f25b5a7746bb`
and
`9a8a8a645a890a3f56fb9f40001aa46f08b6b46009dd6a426873249a7611a46f`;
this is now the canonical full vector.

Reproduce the frozen inputs and focused receipts with:

```sh
./scripts/test-test262-for-of-async-member.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-for-of-async-member.sh
```

Reproduce the complete join with
`TEST262_FULL_WORKERS=2 ./scripts/test-test262-for-of-async-member.sh --full`.

## R3cw RegExp exec recompilation ordering

R3cw freezes QuickJS 2026-06-04's observable `RegExpBuiltinExec` order: RegExp
brand validation precedes input `ToString`, which precedes `lastIndex`
`ToLength`; only after those calls does the engine read the current bytecode
and flags. Because either coercion may call the legacy
`RegExp.prototype.compile()`, Oxide now delays its program snapshot until both
coercions have completed. The independent seven-vector QuickJS oracle covers
replacement programs and global/sticky flags plus capture, named-group, and
indices allocation from the replacement program; both engines pass 7/7.

The exact manifest contains two paths / four sloppy-and-strict variants and
hashes to
`2d272e6f86d0cb3f041e824008771750a833d30209971d6dbebc2c0598726aa3`.
Pinned QuickJS passes all four. The authenticated R3cv parent records
`fail-runtime=4`, while the candidate records `pass=4`. Candidate focused
TSV/JSONL SHA-256 values are
`51bd65e7c991d8e371d263cf352cdd57dc2bb24e329b2032f4d28dd2eedafa10`
and
`86e7c93040d35347add9f1a209eb0de7d6dbf87f8d98ab9116f32a995319fb27`.
The profile remains byte-identical at 126 tags and SHA-256
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`.

A fresh two-worker full replay changes the same four outcomes, leaves 102,033
rows unchanged, has no detail-only movement, and records no previous-pass
regression. Both non-cohort row streams are byte-identical: their TSV/JSON
SHA-256 values are
`6c2e3be458283298c2bc512c1c0b2f5ecf24a5ca1377abc6554d65286a951311`
and
`dfb69468f4eb79aacd032b88ec75f03f8d22558e2ea52ce7bba08cbc57b3e3df`.
The canonical result is 65,503 passes / 65,566 runnable; `fail-runtime` falls
from 54 to 50 and total unsupported remains 17,996. Full candidate TSV/JSONL
SHA-256 values are
`cd5aa3df85c45b72a8939d9c5778c70192b1dc3699eb3330ff8f7aff0ef1159f`
and
`709f49e182e1cfb83353c46251d5eb0bbc24109c3690532f2f4e348d64f1664f`.
The complete residual candidate summary is `fail-parse=11`,
`fail-runtime=50`, `skipped-config-exclude=6700`, `skipped-feature=11775`,
`timeout=2`, `unsupported-feature=13719`, `unsupported-host-agent=118`,
`unsupported-host-can-block-false=4`, `unsupported-host-is-html-dda=84`,
`unsupported-module=679`, and `unsupported-negative-provenance=3392`.

Reproduce the receipts with:

```sh
./scripts/test-test262-regexp-exec-recompilation.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-regexp-exec-recompilation.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-regexp-exec-recompilation.sh --full
```

## R3cv Array flat/flatMap global admission

R3cv promotes the implemented `Array.prototype.flat` and
`Array.prototype.flatMap` surfaces into the live global profile. The candidate
adds exactly those two feature tags to the authenticated R3cu parent, growing
the profile from 124 to 126 tags while preserving all 1,197 audited negative
paths and the execution policy byte-for-byte. Parent and candidate profile
SHA-256 values are
`ff0a591164b267d06762bd5d5a41781d50cc8128377a3787e3c1ea13f7c30b1a`
and
`7c186f132e1228136085fe37322c9baf821741b10af3378d5a16217c98896775`.

The exhaustive pinned metadata universe contains 35 paths / 69 variants and
hashes to
`867fe0a1303259a449e12d367c5c67d4409218c6ac0eb41a1a335326d89f1c6e`.
Pinned QuickJS passes all 69. The authenticated parent reports
`unsupported-feature=69`; the R3cv candidate reports `pass=69`. Candidate
focused TSV/JSONL SHA-256 values are
`02030ecd7daac3a3656d9bec6966145e2fd955d0e6c977bd4993faf38110aa7e`
and
`92507f9130e7bfb1b231d1ad40cbc622463858fb9707847540724257480ecefd`.
The focused join changes all 69 outcomes, has no detail-only movement, and
records no regression.

Across the complete 102,037-row vector, the same 69 outcomes change and the
other 101,968 rows remain byte-identical. The canonical result is 65,499
passes / 65,566 runnable; `unsupported-feature` falls from 13,788 to 13,719,
and no previous pass regresses. Full candidate TSV/JSONL SHA-256 values are
`4cec8ef8be4b432b6f754c07522e744af856bbd8c9ed32fb98fecfe41810c076`
and
`022ab0c11d55e70d2f08c7df7361a36b571bac91320f43d6edfe46e19dba4975`.
An independent fresh two-worker candidate replay is byte-identical to that
canonical full report. An eight-worker trial that transiently timed out two
unrelated `String/fromCodePoint` variants was rejected and is not frozen.
The complete residual candidate summary is `fail-parse=11`,
`fail-runtime=54`, `skipped-config-exclude=6700`, `skipped-feature=11775`,
`timeout=2`, `unsupported-feature=13719`, `unsupported-host-agent=118`,
`unsupported-host-can-block-false=4`, `unsupported-host-is-html-dda=84`,
`unsupported-module=679`, and `unsupported-negative-provenance=3392`.

Reproduce the receipts with:

```sh
./scripts/test-test262-array-flatten-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-array-flatten-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-array-flatten-global.sh --full
```

## R3cu dynamic eval WTF-8 source preservation

R3cu freezes the pinned Test262 surface that sends raw lone UTF-16 surrogates
through direct or indirect String `eval`. The engine now carries that source
through parsing and compilation with a reversible same-width carrier, then
retains saved debug metadata as QuickJS-compatible canonical WTF-8. String and
template values, RegExp patterns, and `Function.prototype.toString` preserve
the original UTF-16 units, while comment tokenization and identifier error
locations remain stable. A real U+E000 remains distinct from the internal
carrier, and valid surrogate pairs retain canonical UTF-8. Dynamic `Function`
and the Test262 host's `$262.evalScript` are outside this milestone and remain
typed frontiers.

The exact manifest contains 11 paths / 22 sloppy-and-strict variants and
hashes to
`3e4f73f980aae940fe3f81df608e5f32154d851c632535a58de89de728b31f2d`.
Pinned QuickJS passes all 22. The authenticated R3ct parent records
`unsupported-runtime=22`, while the R3cu candidate records `pass=22`.
Candidate focused TSV/JSONL SHA-256 values are
`515e3b7056e86958fe3b7e265f717ce301e95245ed907f35cbeae7d5ff8c3859`
and
`3f36b9aa435cd8c29b58f6cb9f65a8a6b4a57fbb66ec588deacf13c6e1de6dca`.
The focused join changes all 22 outcomes, has no detail-only movement, and
records no regression.

The global profile remains byte-identical: 124 admitted feature tags, 1,197
audited negative paths, and SHA-256
`ff0a591164b267d06762bd5d5a41781d50cc8128377a3787e3c1ea13f7c30b1a`.
Across the complete 102,037-row vector, the same 22 outcomes change and the
other 102,015 rows remain byte-identical. The canonical result is 65,430
passes / 65,497 runnable; `unsupported-runtime` falls from 22 to zero. No
previous pass regresses. Full candidate TSV/JSONL SHA-256 values are
`8cbb90ce01fcc2c887871d7de02cfb62a6588ff807e8604e27700823b99d5820`
and
`10cb9ef6db26da8150cf8f23222b0aad02ac7cee9326aab18ef56ca0ab272aa4`.
The complete residual candidate summary is `fail-parse=11`,
`fail-runtime=54`, `skipped-config-exclude=6700`, `skipped-feature=11775`,
`timeout=2`, `unsupported-feature=13788`, `unsupported-host-agent=118`,
`unsupported-host-can-block-false=4`, `unsupported-host-is-html-dda=84`,
`unsupported-module=679`, and `unsupported-negative-provenance=3392`.

Reproduce the receipts with:

```sh
./scripts/test-test262-eval-wtf8-source.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-eval-wtf8-source.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-eval-wtf8-source.sh --full
```

## R3ct basic RegExp v CharacterClassEscape runtime

R3ct implements the first deliberately bounded `v`-flag slice by following
QuickJS 2026-06-04's `unicode_sets` class-atom path for `d`, `D`, `s`, `S`,
`w`, and `W`. Those six escapes work as consuming atoms and inside simple
classes, with anchors, ordinary quantifiers, complements, Unicode code-point
width, and `iv` case folding. Adjacent `v` grammar remains fail-closed: set
operations, nested sets, properties, strings, groups, disjunction, literals,
and dot return typed `Unsupported`. Malformed syntax within the narrow slice
continues to return `SyntaxError` with QuickJS-compatible priority.

The frozen universe is the complete pinned
`test/built-ins/RegExp/CharacterClassEscapes` directory: 12 paths / 24
sloppy-and-strict variants. Its path manifest and exact variant keys hash to
`45a7ee70a325e4f175c4cb3d021d9ba73180c2106058f694a0ff2ca40da36bc6`
and
`e14024023a6e7ab1f266b464a31bdd835c1aaeab3ef8b1306ad508d37c1fd34c`.
Pinned QuickJS passes 24/24. The authenticated Oxide parent records
`unsupported-parser=24`; its TSV/JSONL hashes are
`cfadcf29b6c4000f67d0949c627d7c3130e7b31772a1463e3e1f330b9e76873d`
and
`77261b872e8addbd97363556a60f2a3721571010875c2d77b56df63481776684`.
The actual candidate passes 24/24, with focused TSV/JSONL hashes
`b3db379e2fb33ac9a2042e35e81758c7dd76f6351cc944ec0660b79582922710`
and
`4acd63c554f26a10132139d21b45473b4e38646754545857258405e64436bbfa`.

No capability admission is hidden in this result. The live profile remains
byte-identical at
`ff0a591164b267d06762bd5d5a41781d50cc8128377a3787e3c1ea13f7c30b1a`:
its 124 features still omit `regexp-v-flag`, while the upstream-generated
cohort requests only the already-admitted `String.fromCodePoint`. The gate
derives all 12 paths from the pinned directory and asserts that metadata
boundary explicitly.

The exact full join changes those same 24 outcomes, leaves the other 102,013
rows byte-identical, and has zero detail-only changes or previous-pass
regressions. The canonical vector is 65,408 passes / 65,497 runnable;
`unsupported-parser` falls from 24 to zero. Full candidate TSV/JSONL hashes
are
`908f7e0a9dca5a0b7f7c4a154ecffce425a0998cf1c0e7c8830dbe35850599d7`
and
`9a128f5e3a901ddb50bb9e98a080dfe1355ec0d6ddad9fa9d6fc09c7501e7eb7`.

The landing audit rejected an eight-worker debug probe because scheduler
contention produced 28 extra timeouts and two stack-overflow failures. Its
complete diff isolated 15 resource-sensitive paths / 30 variants. A release
runner replayed that exact manifest with the canonical two-worker policy;
all 30 passed and their row stream was byte-identical to the R3cs parent. The
recovery manifest, TSV, and JSONL hashes are
`be6fbae575fd0d759269ed8805870fa26d8e58eaf14029d9805f2e8454fcc476`,
`e9e9e0e0959df5ccd4a6e58ee45de93c5befc828dffd657ca628d16e9ed4b575`,
and
`a430d6a04b85af752e95fc194500bf9aea36a36ba8a7a865638796ddd3b3b054`.
The normal full gate still performs a complete release/two-worker replay; the
partitioned recovery is a frozen landing receipt, not a weaker default.

Reproduce the receipts with:

```sh
./scripts/test-test262-regexp-v-character-class-escapes.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-regexp-v-character-class-escapes.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-regexp-v-character-class-escapes.sh --full
```

## R3cs future-reserved-word negative provenance admission

R3cs admits the 25 parse-negative paths / 32 variants authenticated by R3cr's
scoped receipt into the live profile. The added-path manifest hashes to
`8bd18ff57c518d106de263d3b77ea56695fd6368e846afdabaaaab72033fd51f`;
its exact variant keys hash to
`d51615c929d874567d2a53789c0c671ebfc5c7792b55f51d170c6cbdcf16ff73`.
Together with the activation and already-passing partitions, the frozen
future-reserved-word universe remains 56 paths / 86 variants. Oxide passes all
86 under the live global profile, and pinned QuickJS 2026-06-04 independently
passes all 86.

The 124 feature tags and execution-policy entry remain byte-identical. Only
the audited-negative section grows from 1,172 to 1,197 paths, moving the
profile SHA-256 from
`40e8669015c3ea00d2704b49e540947c0aa202fe22900b0dff84acb5da3b554e`
to
`ff0a591164b267d06762bd5d5a41781d50cc8128377a3787e3c1ea13f7c30b1a`.
The focused parent records 54 passes and 32
`unsupported-negative-provenance` outcomes; its TSV/JSONL hashes are
`82b70fb5bba526bcd86122b024f17d1a71ceab29a9579096ee1e6ea70b086b4d`
and
`326265ea2e062576c658c6bb28a40cd004f7bbb5f4de5440773d0df0798e6396`.
The candidate records `pass=86`, with hashes
`dd15a12ae62c5a4a1dd2466ac1934bb8093f9cfaf9149d45e8a7cde3b9de72ef`
and
`20dc3e8a6139f86b37e725ed359a3da70132a3b3262effad61f13ab0055115ee`.
The exact focused join changes 32 outcomes and leaves 54 rows unchanged.

Across all 102,037 variants, those same 32 outcomes change, 102,005 rows stay
byte-identical, and no previous pass regresses. At R3cs, the canonical vector
was 65,384 passes / 65,497 runnable; `unsupported-negative-provenance` fell from
3,424 to 3,392. Full candidate TSV/JSONL hashes are
`1df77fd5d67b0ba585b3390cf0ce50a53f59226dfd57983edcc26d3c7a034dfe`
and
`257eef22e32ed8d5b1d6a837d07a82d7c1bf4263b996364000a1e98522f83138`.

Reproduce the receipts with:

```sh
./scripts/test-test262-future-reserved-words-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-future-reserved-words-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-future-reserved-words-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3cr future-reserved-word runtime and scoped receipt

R3cr freezes the complete future-reserved-word universe at 56 paths / 86
variants; pinned QuickJS 2026-06-04 passes all 86. Oxide now rejects invalid
`enum`, `export`, and `extends` statement/expression uses with `SyntaxError`
while preserving their IdentifierName property uses. It also distinguishes
malformed ImportCall and Script/Eval `import.meta`, which are real
`SyntaxError` results, from syntactically valid dynamic import, which remains
a typed `Unsupported` module-loading frontier.

The valid-import diagnostic is deferred until the whole Script/Eval source has
parsed and identifier/private-name resolution has completed. Consequently a
later grammar or private-name early error wins instead of being hidden by the
unimplemented import runtime. The unchanged global profile SHA-256 is
`40e8669015c3ea00d2704b49e540947c0aa202fe22900b0dff84acb5da3b554e`.
Its focused parent records 53 passes, one `unsupported-runtime`, and 32
`unsupported-negative-provenance`; the candidate records 54 passes and the
same 32 fail-closed negatives. The scoped profile audits all 26 negative paths
and passes 86/86. The exact focused join changes one outcome, leaves 85 rows
unchanged, and has no regression.

Across all 102,037 variants, the same one outcome changes, 102,036 rows stay
byte-identical, and no previous pass regresses. The canonical vector is now
65,352 passes / 65,465 runnable; `unsupported-runtime` falls from 23 to 22.
Full candidate TSV/JSONL hashes are
`22203b1a0cdb51a76552ef4e999dde24c582f981f50fe85f9f8c12a0b17a6f7f`
and
`c009cbc3c65fdd617d33b488b47fd80c10cb703b269e034025facce1e5b1a470`.

Reproduce the receipts with:

```sh
./scripts/test-test262-future-reserved-words.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-future-reserved-words.sh
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3cq debugger negative-test global admission

R3cq admits the five negative paths / ten sloppy-and-strict variants from the
R3cp scoped receipt into the live global profile. All ten require the declared
parse-phase `SyntaxError`. Together with the activation and escaped-property
canaries, the complete frozen `debugger` universe remains ten paths / 20
variants. Oxide passes all 20 under the candidate global profile, and pinned
QuickJS 2026-06-04 independently passes all 20.

The parent and candidate profile SHA-256 values are
`1a85d1b9b43c54825c1a435011be737593ccc9754753daabdd255f9bd078bf7a`
and
`40e8669015c3ea00d2704b49e540947c0aa202fe22900b0dff84acb5da3b554e`.
The 124 feature tags and single execution-policy entry are byte-identical;
only the audited-negative section grows from 1,167 to 1,172 paths. The exact
focused join changes the ten admitted outcomes and leaves the other ten rows
unchanged.

Across all 102,037 variants, the same ten outcomes change, 102,027 rows stay
byte-identical, and no previous pass regresses. The canonical vector is now
65,351 passes / 65,465 runnable; `unsupported-negative-provenance` falls from
3,434 to 3,424. Full candidate TSV/JSONL hashes are
`91bad0c048a1d90a76346a41dd2676ae5a530b8ad787c30292bd2f7c956e573a`
and
`40c39453be1b9e7cbc912fd841442a0e81cbab650b568a44b765168424433583`.

Reproduce the receipts with:

```sh
./scripts/test-test262-debugger-statement-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-debugger-statement-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-debugger-statement-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3cp debugger statement runtime parity

R3cp freezes the complete pinned `debugger` statement cohort at ten paths / 20
variants. Pinned QuickJS passes all 20. Its parser treats the statement as an
ASI-terminated no-op: it consumes `debugger`, emits no bytecode, and therefore
preserves a prior eval completion value. Oxide now matches that behavior while
continuing to reject reserved-identifier uses and accept escaped
IdentifierName property and method uses.

The unchanged global profile records ten passes out of the 20 variants; the
exact scoped profile audits the five negative paths and records 20/20 passes.
The focused join repairs the two sloppy/strict statement variants and leaves
the remaining 18 rows unchanged.

Across all 102,037 variants, the same two outcomes change, 102,035 rows stay
byte-identical, and no previous pass regresses. The canonical vector is now
65,341 passes / 65,455 runnable; `unsupported-parser` falls from 26 to 24.
Full candidate TSV/JSONL hashes are
`362690ef82273724b8a5a24247e7529060051e63a5a43671d37e30909da0f779`
and
`b61846b93d222f52ded5dd28c1a849c566dceb7d855d49e3e2a8f899046cff13`.

Reproduce the receipts with:

```sh
./scripts/test-test262-debugger-statement.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-debugger-statement.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-debugger-statement.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3co HTML-like-comments negative provenance admission

R3co admits the ten negative Script paths authenticated by the R3cn scoped
receipt. Their path manifest hashes to
`e301116b8ea4220bc054d7228b338f6982d4001c4ef74560fd1af2b44f5bb8fd`;
the 17 variant keys hash to
`e6d4e9b8750fc22295c5c02041917a98c9f941180d4f3a885d54b7db9f0ec5a1`.
Thirteen variants require their declared runtime error and four require their
declared parse error. Together with the R3cn activation, already-pass, and
Module partitions, the complete universe remains 19 paths / 32 variants.
Pinned QuickJS passes all 32.

The live profile keeps 124 feature tags and an unchanged execution policy,
but grows from 1,157 to 1,167 audited negative paths. The parent and candidate
profile SHA-256 values are
`ef17b52324782431adc1ddbabc81530de3e24fb436545202f248d850a1043dbb`
and
`1a85d1b9b43c54825c1a435011be737593ccc9754753daabdd255f9bd078bf7a`.
The focused parent records `pass=12 unsupported-negative-provenance=17
unsupported-module=3`; the candidate records `pass=29
unsupported-module=3`. The exact join has 17 outcome changes and 15 unchanged
rows.

Across all 102,037 variants, those same 17 outcomes change, 102,020 rows stay
byte-identical, and no previous pass regresses. The canonical vector is now
65,339 passes / 65,455 runnable; `unsupported-negative-provenance` falls from
3,451 to 3,434. Full candidate TSV/JSONL hashes are
`2502eda033dc3a91c64ddaab00093af254bead7c2dd15b13060b6b6088b5c1a7`
and
`062115b6363fb8ea49ed7240c80bfcb6fd035e94f34d2ff8365284cd75844302`.

Reproduce the receipts with:

```sh
./scripts/test-test262-html-comments-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-html-comments-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-html-comments-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3cn HTML-like-comments runtime parity

R3cn freezes the complete pinned HTML-like-comments universe: 19 paths / 32
variants, with path and variant-key SHA-256 values
`cefee3d124372362a146cff066bd3da2609d66db3c50a60956bc4a63351948e6`
and
`25bb0e022c21bf9d84663198ef27c47a1315f6cdc9a2302e4f3c56c2965c7ed7`.
It partitions without overlap into five runtime activation paths / ten
variants, one already-pass path / two variants, ten negative-pending Script
paths / 17 variants, and three unsupported Module variants. Pinned QuickJS
passes all 32.

The unchanged global profile moves from `fail-runtime=10 pass=2` to
`pass=12`, while retaining 17 `unsupported-negative-provenance` and three
`unsupported-module` outcomes. The exact focused join has ten outcome changes
and 22 unchanged rows. Parent TSV/JSONL hashes are
`241c5d403f78728a4c1caf5b11220f8c3d7224e6fef2ad56c91b9892df996224`
and
`31f272c60ee8261ee4e915715aa40474abd6fc3417c8e2ff3b3c9e31c57d3eb0`;
candidate hashes are
`5e434c45148a97ff8c94b68601e42c306eb50c57e28a32b450195b8e07261d67`
and
`32f41a3626ff7ed81209fa33b63457ff7634b81e9f17943a5e867ebab970bf03`.
A separate exact-manifest scoped profile audits only the ten negative paths
and records `pass=29 unsupported-module=3`; its TSV/JSONL hashes are
`d0ff8ffe6899c5006d2068c351f9bc1a36d72c37994febd515fce246c74c7389`
and
`966843c8af0a76e43d4dcdc68a645fa541171929a5ec5779ba6be983e5f5982d`.

The live profile remains at 124 feature tags and SHA-256
`ef17b52324782431adc1ddbabc81530de3e24fb436545202f248d850a1043dbb`;
the 17 negative variants are deliberately not admitted globally in this
runtime milestone. Across all 102,037 variants, exactly the same ten outcomes
change, 102,027 rows remain byte-identical, and no previous pass regresses.
The canonical vector is now 65,322 passes / 65,438 runnable, with
`fail-runtime=54`. Full parent TSV/JSONL hashes are
`d404fdd6e1fa7e9f19703bbdbc49bd55fddb83b744d30254349087f0a26568d5`
and
`2196ac6f9ca0c6f251ae0ee8987ea5351c7be076188e5b81c82055f2b2d86188`;
candidate hashes are
`abd85c73e941a35a990069c619e1164d1a785f537057ff5f3e1b70ab434a0c07`
and
`691713498774972a6539dcd6506c66be0eb4aa397bc04141d55f86594c816e3f`.

Reproduce the receipts with:

```sh
./scripts/test-test262-html-comments-runtime.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-html-comments-runtime.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-html-comments-runtime.sh --full
```

## R3cm Promise.try and Promise.withResolvers admission

R3cm adds exactly `promise-try` and `promise-with-resolvers` to the global
profile, growing it from 122 to 124 feature tags. The parent and candidate
profile SHA-256 values are
`1e39c157e444f60f0a44f4fd373ad63147d814986cde5f08c4f5b33d8f5839a2`
and
`ef17b52324782431adc1ddbabc81530de3e24fb436545202f248d850a1043dbb`.
Their audited-negative and execution sections are byte-identical.

The complete metadata universe is 21 paths / 39 variants. Its path and key
hashes are
`5a6ee02c250ba64bc4869702634f6f858442d4543dec4aaccf9b8766f66b2dab`
and
`6bbc93692001799e4acdf3142397346b798359726a7cf67c1eea66e976b1bbb0`.
It partitions without overlap into 16 activation paths / 32 variants, two
class-dependent paths / four reason-only variants, and three top-level-await
module paths / three variants. Pinned QuickJS passes all 39. Oxide passes the
32 activation variants, keeps only `class` on the four reason-only rows, and
leaves the module rows unchanged. The pre-existing broader Promise gate also
remains 224/224 pass.

The focused parent has `unsupported-feature=36 unsupported-module=3`; the
candidate has `pass=32 unsupported-feature=4 unsupported-module=3`. Its exact
transition contains 32 outcome changes, four detail-only changes, and three
unchanged rows. The transition TSV SHA-256 is
`73e918eec307ecc9294f0ac62201954413cc0f8532c010ab4d5dca12371f3e18`.

Across all 102,037 variants, the same 36 rows change, 102,001 remain
byte-identical, and no previous pass regresses. The canonical vector is now
65,312 passes and 65,438 runnable variants; `unsupported-feature` falls to
13,788. Full parent TSV/JSONL hashes are
`ef3b88f82d4e65f55b584731f1cf78e7b734baf467639a6e18028f405c77ee56`
and
`81d1071fe7dc47e0e2a874641bea28bc5b707d17690c764194231a838de75d66`;
candidate hashes are
`d404fdd6e1fa7e9f19703bbdbc49bd55fddb83b744d30254349087f0a26568d5`
and
`2196ac6f9ca0c6f251ae0ee8987ea5351c7be076188e5b81c82055f2b2d86188`.

Reproduce the receipts with:

```sh
./scripts/test-test262-promise-try-with-resolvers-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-promise-try-with-resolvers-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-promise-try-with-resolvers-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3cl String.prototype.localeCompare runtime parity

R3cl freezes every direct non-Intl `String.prototype.localeCompare` path in
the pinned Test262 tree plus two supplemental descriptor/nullish-receiver
paths: 15 paths / 30 variants. The 13 direct paths and 26 variant keys hash to
`75762419fa0a204aed1fc697d20a12e7403c3db673540a41f29aedca6ad70825`
and
`c568684da620fa39a72387aca971d14b5ddc9a4dafc47dfb9de2ff8d271c7c0b`;
the complete gate's path and key hashes are
`6fc68bb701d04bf9dce2d8c3d4ee5d52b433b2a9ad1ab53b853decfda39b8105`
and
`605c6c4e9e80e8c4a12e2dc5d42a8477c0c2559b744dfda30ea85ea909a03d18`.
Pinned QuickJS and Oxide both pass all 30 variants. The ten Intl402 paths stay
configuration-excluded and are not part of this parity claim.

The R3ck parent contains 26 `fail-runtime` outcomes and four outcome-level
false passes. R3cl changes exactly those 26 failures to passes, with no detail
changes and four unchanged rows. The focused parent TSV/JSONL hashes are
`95b594ce9d6219b51681b77bab86e4b82ae79e4e2b6f839b36af489d5ff0f43c`
and
`ac8bc91d74eb602e2789b88f62ab1fde2e19a3cda8eca3e64c32c7173c38db4d`;
candidate hashes are
`677848008880a63d0c7decd351d96afc9c1668d9ca8c952f814e05ba1853b937`
and
`a4303ea1d66064561d0192d1828e81b5f96bec12fab2628e12ebc269199b1dc6`.
The transition TSV hashes to
`5abf0cf81924a88204791b35eb990b8a5d0930cee03aabf6e33da399ae941e84`.

There is no localeCompare feature tag, so the 122-tag profile remains
byte-identical at SHA-256
`1e39c157e444f60f0a44f4fd373ad63147d814986cde5f08c4f5b33d8f5839a2`.
The exact full-suite join changes the same 26 outcomes, leaves 102,011 rows
unchanged, and has zero previous-pass regressions. The canonical vector is now
65,280 passes and 65,406 runnable variants out of 102,037; `fail-runtime` is
64. Full parent TSV/JSONL hashes are
`f491512281647b752796da1abe8fcf559981b48a53270bf128e9b698ade60c3f`
and
`d65c1fbb9f17bc1666b2dbd0c228843a33147d4f762f7c18aa9491e883c3c59a`;
candidate hashes are
`ef3b88f82d4e65f55b584731f1cf78e7b734baf467639a6e18028f405c77ee56`
and
`81d1071fe7dc47e0e2a874641bea28bc5b707d17690c764194231a838de75d66`.

Reproduce the receipts with:

```sh
./scripts/test-test262-string-locale-compare.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-string-locale-compare.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-string-locale-compare.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3ck String.prototype.normalize runtime parity

R3ck freezes every direct normalize test in the pinned Test262 tree plus the
supplemental SpiderMonkey receiver-error test: 19 paths / 38 variants. The 18
direct paths and 36 variant keys have SHA-256 values
`fbc367395cfe02aff55fdc162f7bac0e1dd9218d6416c50359809d03058486ff`
and
`ba68067e07e79dd0e498d1b7666c0783db0f5063544711bb558fa7428f721ad1`;
the complete gate's path and key hashes are
`6f70d7c7adbce4cae537c05e6e35338baee407a813d01c60b1b1f4e187dcca4c`
and
`dbcbd112f6d4b3c08580b9ee40f9e7e111a57a9f2995b428083f66f0356ae44f`.
Pinned QuickJS 2026-06-04 and Oxide both pass all 38 variants.

The historical parent report contains 20 `fail-runtime` outcomes and 18
outcome-level false-passes caused by missing-method errors, guards, or property
enumeration. The candidate changes exactly those 20 failures to passes; the
other 18 rows remain outcomes-equivalent but are rerun with the intrinsic
present. The focused transition therefore has 20 outcome changes, no
detail-only changes, and 18 unchanged rows. Its TSV SHA-256 is
`b72ee5fccd647d74275c756f1bdd44f95c2976fa2bd5c1eab99cc24a2ab7ed8d`.
The focused parent TSV/JSONL hashes are
`4ef7519798294a023d7cefa1af595945fcfab49060639d49a23271fb9e8b35ad`
and
`9533e4e935a9a77dc8444ea761e06daeca49684a143583c8c784dc040c7d4353`;
the candidate hashes are
`22a3aa4192be516cd5ca6eb0ce7c69325ab6ccf7cb7619892726015f8051d2a7`
and
`b613d9f29d75e67b41561ad2b7e29d8a6e89f933b02e70e5a73b07e9f82283fb`.

Because Test262 assigns no feature tag to `String.prototype.normalize`, this
runtime milestone leaves the 122-tag profile byte-identical at SHA-256
`1e39c157e444f60f0a44f4fd373ad63147d814986cde5f08c4f5b33d8f5839a2`.
The exact full-suite join changes the same 20 outcomes, leaves 102,017 rows
unchanged, and has zero previous-pass regressions. The canonical vector is
65,254 passes and 65,406 runnable variants out of 102,037; `fail-runtime`
drops from 110 to 90. The full parent TSV/JSONL hashes are
`acd43fe1eb9752246e9994c58c3f139ceff0c5e80416baea06757428e5ba6bba`
and
`c1a4bf7cc058a70b6b97475fccc92700403a19c63936c341ea3a6ebe79e4f34a`;
the candidate hashes are
`f491512281647b752796da1abe8fcf559981b48a53270bf128e9b698ade60c3f`
and
`d65c1fbb9f17bc1666b2dbd0c228843a33147d4f762f7c18aa9491e883c3c59a`.

Reproduce the receipts with:

```sh
./scripts/test-test262-string-normalize.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-string-normalize.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-string-normalize.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3cj binary-data metadata admission

R3cj admits the remaining Test262 metadata names for the implemented numeric
binary-data surface: eight `DataView.prototype` read/write tags plus
`Float16Array`, `Float32Array`, `Float64Array`, `Int8Array`, `Int16Array`,
`Int32Array`, `Uint8Array`, `Uint8ClampedArray`, `Uint16Array`, and
`Uint32Array`. The candidate adds exactly these 18 sorted features to the R3ci
parent, growing the global profile from 104 to 122 entries. The parent and
candidate SHA-256 values are
`01f936b9f5e0b920f10119a73f7e8ea52450863f113fff6542f3f241ed914d75`
and
`1e39c157e444f60f0a44f4fd373ad63147d814986cde5f08c4f5b33d8f5839a2`;
their audited-negative and execution sections are byte-identical.

The metadata-derived universe is exactly 200 paths / 400 variants. Its path
and variant-key SHA-256 values are
`180891c61576e604beec526e36928735380c31d431a3035cb343c9985ebc4c99`
and
`f0e8838e2b9aeba652199da01105eebd10518d4b29b5263fd07fcdfea3173582`.
It partitions without overlap into:

- 193 activation paths / 386 variants, comprising 141 paths authenticated by
  prior DataView/TypedArray gates and 52 independently audited supplemental
  paths;
- 5 paths / 10 variants which retain other unsupported feature tags;
- 2 paths / 4 variants skipped by the pinned Test262 configuration.

There are no exclusions inside activation: pinned QuickJS and Oxide both pass
all 386 variants. Across the complete focused universe, the parent has 396
`unsupported-feature` selections and four config skips. The candidate turns
386 selections into passes, narrows the other ten to their independent feature
reasons, and leaves the four config skips unchanged.

The exact 102,037-row parent/candidate join changes those same 396 rows: 386
outcome changes, ten detail-only changes, 101,641 byte-identical rows, and zero
previous-pass regressions. The R3cj candidate vector was 65,234 passes and
65,406 runnable variants, with 13,820 `unsupported-feature` selections. Its
TSV/JSONL SHA-256 values are
`acd43fe1eb9752246e9994c58c3f139ceff0c5e80416baea06757428e5ba6bba`
and
`c1a4bf7cc058a70b6b97475fccc92700403a19c63936c341ea3a6ebe79e4f34a`.

Reproduce the receipts with:

```sh
./scripts/test-test262-binary-data-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-binary-data-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-binary-data-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3ci createRealm and evalScript admission

R3ci source-audits every direct `$262.createRealm` and `$262.evalScript` use in
the pinned Test262 tree. The two direct universes are disjoint and combine to
312 paths / 589 variants. Their path and variant-key SHA-256 values are
`8262c45e99d6af8cd6cba3f883a91a8031ad94478bf847202b7081420a5ee371`
and
`0432024eb336744a11b7de5fc3a960eccae9263e7dfbf3af51d2fba5103f15cd`.

The `createRealm` universe partitions exactly into:

- 79 activation paths / 150 variants, all passing in Oxide;
- 174 paths / 340 variants which retain independent missing features;
- 11 config-excluded paths / 22 variants;
- 17 config-skipped paths / 33 variants.

Pinned QuickJS passes the wider 80-path / 152-variant oracle envelope. The two
variants outside formal activation are the source-audited Atomics
cross-compartment staging test; both remain `unsupported-feature` because the
global profile does not claim Atomics or SharedArrayBuffer. The separate
`evalScript` universe is featureless apart from its synthetic host admission
tag: all 31 paths / 44 variants pass in both engines.

The global candidate adds only `host-create-realm-required` and
`host-eval-script-required`, growing from 102 to 104 sorted features. Parent
and candidate keep byte-identical audited-negative and execution sections.
Their SHA-256 values are
`c671ae022251a9a0f7d17cc851db7506d825c34854c69adedc6475d3da0f389f`
and
`01f936b9f5e0b920f10119a73f7e8ea52450863f113fff6542f3f241ed914d75`.

The runtime-only parent preserves the R3ch totals of 64,654 passes and 64,826
runnable variants while moving 490 createRealm and 44 evalScript selections
from host-capability outcomes to `unsupported-feature`. Admitting the two tags
then creates 194 passes and leaves 340 createRealm variants behind their exact
independent feature reasons. The full join reports 534 changed rows, 101,503
unchanged rows, and zero previous-pass regressions.

The R3ci candidate full vector was 64,848 passes and 65,020 runnable variants
out of 102,037. It contains 14,206 `unsupported-feature` outcomes and no remaining
`unsupported-host-create-realm` or `unsupported-host-eval-script` outcomes.
The TSV/JSONL SHA-256 values are
`2f40849011fae4f96455225e467c817c6aeeaf3cc90722d357a1d8bdddbbf3bc`
and
`e6c18b7d9f6ef3f42bbf86ab396b91fb64773640e932581940f43cb9754509a1`.

Reproduce the receipts with:

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

## R3ch host-GC admission

R3ch freezes every path in the pinned Test262 tree which declares
`host-gc-required`: 15 paths / 28 variants. Source-token and metadata
projections authenticate the universe. Fourteen paths / 26 variants require
only `$262.gc`; one DataView path / two variants also calls
`$262.createRealm`. Pinned QuickJS passes all 28 variants.

The worker now publishes a genuine QuickJS-shaped GC callback. With the scoped
profile, Oxide passes all 26 activation variants while the other two remain
`unsupported-host-create-realm`. Repeated focused runs at different worker
counts are byte-identical. The scoped TSV/JSONL SHA-256 values are
`78ba543fd816f68a82ce264353478bc94dc4dfa067663d88d80a44fc51699211`
and `a3ae18bfc094bb7957cf45adaa3efa42830163e2eed912f9b754f6fe60ee770e`.

The global admission adds exactly `host-gc-required` to the prior 101-feature
profile. The candidate has 102 features, preserves all 1,157 audited negative
paths and the execution policy, and has SHA-256
`c671ae022251a9a0f7d17cc851db7506d825c34854c69adedc6475d3da0f389f`.
Three authenticated joins separate the runtime capability from the profile
change:

- historical canonical to runtime-only parent: 26 host-GC rows become
  `unsupported-feature`, while two `createRealm` diagnostics lose only their
  GC residual;
- runtime-only parent to admitted candidate: those 26 rows become passes and
  the two `createRealm` rows are unchanged;
- historical canonical to admitted candidate: 26 outcome changes, two
  detail-only changes, 102,009 unchanged rows, and zero pass regressions.

The canonical full vector is 64,654 passes and 64,826 runnable variants out of
102,037. Its TSV/JSONL SHA-256 values are
`8e5c370f57e8d7dcd813df7199c79d210bf82316e802219c6d8a982dab72ac58`
and `f5270e02f19cfb1ab5fc7a5ba5020e15a1ee0cea947914d7656766af0e8a721e`.
The remaining two cohort variants stay attributed to `createRealm`; this
admission does not claim cross-realm host parity.

Reproduce the receipts with:

```sh
./scripts/test-host-gc-reentrant-oracle.sh --oxide
./scripts/test-test262-host-gc.sh --check
./scripts/test-test262-host-gc.sh
./scripts/test-test262-host-gc-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-host-gc-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-host-gc-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3cg global WeakRef and FinalizationRegistry admission

R3cg admits the two implemented metadata tags `WeakRef` and
`FinalizationRegistry`. The frozen parent and candidate profile SHA-256 values
are `3b6c3316992b60644867d76799995ea7005c6c586438064072b017f7c3bd44ef`
and `8be6c2a3892a62d89ed17df3f3d3b54e9e84fda8ef6be2bcdaa7d49044593990`.
Their only difference is the two sorted feature entries: the candidate keeps
the same 1,157 audited negative paths and async-execution policy. The live
profile now contains 101 feature tags and is checksum-bound by both the runner
and upstream manifest.

The gate derives the complete 82-path / 164-variant universe from all 53,125
pinned metadata records. Its path/key hashes are
`0325512882ba3d93d225423b62b76b9d8bebc7266a427ed6e05be3b70559c060`
and `f4beb592d73342a4d694430d8b13a04122b03f61e7c9a79d2e24476e002910a9`.
Pinned QuickJS passes every variant. The exact partition is:

- 79 paths / 158 variants changing from `unsupported-feature` to `pass`;
- one path / two variants retaining `unsupported-feature`, with only the
  diagnostic changing to the independent `for-of` dependency;
- two paths / four variants retaining the identical
  `unsupported-host-create-realm` classification.

The focused parent TSV/JSONL receipts are
`ff55fff3c7a4d2d8dd1bebdb352f0971c3f28c44ff6819cfd72a0acbce8021cc`
and `e5dea54cb9cb809c27bda931e38a876e416c84bc7523bb9455b9bde097ae2deb`;
the candidate receipts are
`b36215a9c1c3863ecafb324ebe67bf4f89d90e7ac05df86369193be584ddfb72`
and `12ef67d72c0dca233599ed041fa6243bc62d376fa5db96932fe082e0e5998745`.
The 169-line transition receipt has SHA-256
`dd7080494f0d628aec4ab45bb793228cca52bebd208e4177ff308dd682b7c5af`.

Before admitting the candidate, the gate proves that its 102,037-row parent
TSV/JSONL is byte-identical to the preceding live canonical baseline:
`e0b0be534f07a34bc7a9e18f4c3bae8c9360dd62c89176f96bf3234c5895b6ec`
and `8227cb6d19fc2f814bdb016308cf1003be6c91ebe01145ccc3c719f6e38ac6bf`.
The full parent/candidate join has 158 outcome changes, two detail-only
changes, 101,877 unchanged rows, and zero previous-pass regressions. The new
canonical vector is:

- 64,628 pass and 64,800 runnable out of 102,037 variants;
- 110 `fail-runtime`, 13,866 `unsupported-feature`, and 3,451
  `unsupported-negative-provenance` outcomes;
- TSV SHA-256
  `c919dd56fc37f2946d729ee9a9a6958fc91c3f95366843ffae258953145e5a4f`;
- JSONL SHA-256
  `342c22edd7cfdc4edf2b5085455c8586095bb4abc5b59d55cc4657c5ff954459`.

Reproduce the receipts with:

```sh
./scripts/test-test262-weak-ref-finalization-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-weak-ref-finalization-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-weak-ref-finalization-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

The two `for-of` variants and four `createRealm` variants remain classified at
their independent boundaries. Admission therefore widens only the capability
surface justified by R3cf and does not hide host or parser work.

## R3cf WeakRef and FinalizationRegistry focused gate

R3cf freezes every pinned metadata path carrying `WeakRef` or
`FinalizationRegistry`: 82 paths / 164 sloppy-strict variants. Pinned QuickJS
2026-06-04 passes the entire universe. The scoped candidate profile is derived
from the authenticated 99-feature global profile and adds exactly those two
feature tags; it changes no audited-negative or execution-policy row.

The universe partitions exactly into:

- 79 paths / 158 activated variants, all passing in isolated Oxide workers;
- one path / two variants still requiring the independent `for-of` feature;
- two paths / four variants requiring the Test262 host's `createRealm` hook.

The universe path/key hashes are
`0325512882ba3d93d225423b62b76b9d8bebc7266a427ed6e05be3b70559c060`
and `f4beb592d73342a4d694430d8b13a04122b03f61e7c9a79d2e24476e002910a9`.
The activation path/key hashes are
`de660ae31e700129f9668760e92cd0e712fcbbe753d4f31d321790645428b848`
and `f04acfd7dcc3c8aaf9e06f4734089eb61bf1cf0ffc99d47cf80c5f98ab35e5de`.
The generated focused report has SHA-256
`5ff2b92a694f71b63ab5b883e6c9416e2810c7230e26d36fcaec5f5815b20fe6`.

Reproduce the receipt with:

```sh
./scripts/test-test262-weak-ref-finalization.sh --check
./scripts/test-test262-weak-ref-finalization.sh
```

This cohort has no `$262.gc` tests, so it proves the public API surface rather
than collection timing. Runtime/heap tests checked against the pinned QuickJS
source and manual oracle probes separately bind the lifecycle semantics. The
live global profile remains at 99 features in this implementation milestone;
global admission is intentionally separate.

## R3ce global Weak collections admission

R3ce admits the four implemented metadata tags `WeakMap`, `WeakSet`,
`symbols-as-weakmap-keys`, and `upsert`. The parent and candidate profiles have
SHA-256 values `f229cd652dd5b38ed3a0387a089eab974148d404bd166e8b4c0eb2cb0fa7a2c1`
and `3b6c3316992b60644867d76799995ea7005c6c586438064072b017f7c3bd44ef`;
their only difference is those four sorted feature entries. The live profile
now has 99 feature tags, retains all 1,157 audited negative paths and the same
async-execution policy, and is checksum-bound by both the runner and upstream
manifest.

The universe is regenerated from all 53,125 pinned metadata records rather
than trusted from its checked-in list alone. Its 154 paths / 306 variants
partition exactly into:

- 147 paths / 292 activation variants, all passing in Oxide;
- seven paths / 14 reason-only variants, still withheld by `WeakRef`,
  `FinalizationRegistry`, or both.

The universe path/key hashes are
`d0bd5c21db1165cd72618168ce5592b78a6909be5f2cd0813fa15ed6a3c17cc1`
and `2bf72c55541b84e9a4f0dac4a6eba4c6b073d5154801ae0cbce9d94a7472e319`.
The activation key hash is
`920d30c0e48f75ae77c39b89b32bf1b23d89cfce88ccb05a09ab51ffa430f184`;
the residual key hash is
`63086bdb2ec2f1beefff2d5473f660ef3e4595f9d38884f478158d83da79ac85`.
Existing Weak collections and Map gates cover 233 and 52 activation variants;
a frozen four-path / seven-variant supplement covers the rest. Pinned QuickJS
2026-06-04 passes all 306 universe variants.

The focused transition is exactly 306 `unsupported-feature` outcomes becoming
292 passes plus 14 remaining `unsupported-feature` outcomes; its receipt
SHA-256 is
`7d18cef62b857b175c34529b9147da6404b95114b12440cfa1e36212ffa6cf31`.
The complete join has 292 outcome changes, 14 diagnostic-only changes, 101,731
unchanged rows, and no prior-pass regression. The new canonical vector is:

- 64,470 pass and 64,642 runnable out of 102,037 variants;
- 110 `fail-runtime`, 14,024 `unsupported-feature`, and 3,451
  `unsupported-negative-provenance` outcomes;
- TSV SHA-256
  `e0b0be534f07a34bc7a9e18f4c3bae8c9360dd62c89176f96bf3234c5895b6ec`;
- JSONL SHA-256
  `8227cb6d19fc2f814bdb016308cf1003be6c91ebe01145ccc3c719f6e38ac6bf`.

Reproduce the receipts with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-weak-collections-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-weak-collections-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

The 14 residual variants made the later R3cf WeakRef / FinalizationRegistry
frontier visible. This remains a progress certificate, not a whole-project
Feature Parity claim.

## R3cd WeakMap and WeakSet runtime

R3cd freezes a 264-path WeakMap/WeakSet source universe: a 231-path core from
both built-in directories, the adjacent Object.seal tests, and three named
SpiderMonkey staging paths, joined with all 110 metadata-tagged paths. The
union includes 33 tagged consumers outside the core. Metadata excludes exactly
seven paths from the focused execution boundary: two `cross-realm`, two
`host-gc-required`, and three which also require WeakRef or
FinalizationRegistry. The pinned manifest therefore contains 257 paths / 513
sloppy-strict variants, with one `onlyStrict` path and four checksum-bound
`generated` paths. Its scoped profile admits exactly 11 dependency tags and no
negative-test or host-execution exceptions.

Pinned QuickJS 2026-06-04 and Oxide both pass all 513 variants with zero
failure, unsupported, or skipped outcomes. The focused TSV/JSONL SHA-256
values are
`6fef7950676c1578300a52d6bdd6935892163428b39b1424f3d97f3db0275872`
and
`855047a97f9626c8b3ddad5a72e16e19dd60c7f038c2f5568c933c7b44e757d3`.
The profile and manifest-file hashes are
`a23cfb3270eb40eb3839413f3dacaf75fee2cecaca9d1b0ecc40d2c6c3c804c1`
and
`6189cde88a7fcb15222d536d19f3e8172be66e35de24f47107e0c67910b92b7a`.

The independent canonical run deliberately keeps the existing 95-tag global
profile unchanged. Even at that conservative boundary, implemented weak
collections change the raw vector by 347 passes: `fail-runtime` drops from
400 to 110, and `harness-error` drops from 57 to zero. The separately
checksum-bound 29-path TypedArray audit accounts for all 57 of its variants,
which now pass after the SpiderMonkey harness can construct `WeakMap`.
The canonical vector is now:

- 64,178 pass and 64,350 runnable out of 102,037 variants;
- 110 `fail-runtime`, 14,316 `unsupported-feature`, 3,451
  `unsupported-negative-provenance`, and 19,261 total unsupported;
- TSV SHA-256
  `a7dbb819f224c1710843dab51033c4c32e7eb5c47cbad272e53b77031eb9babd`;
- JSONL SHA-256
  `73249b49ff9f4081c8de1f9f3ca802de8eac6506c2b2c4dd8152f939832b5eaa`.

Of the focused 513 variants, 280 already run and pass under the global profile;
233 remain conservatively `unsupported-feature` until a separate global
tag-admission audit. Reproduce both receipts with:

```sh
./scripts/test-test262-weak-collections.sh
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

The later R3ce/R3cf milestones close or reclassify these dependency rows and
implement the WeakRef/FinalizationRegistry surface. This historical receipt is
not a full Feature Parity claim.

## R3cc global object-rest admission

R3cc admits `object-rest` into the live profile and binds the exact delta to
the pinned metadata inventory. The profile grows from 94 to 95 feature tags
and from 1,154 to 1,157 audited negative paths. Its SHA-256 is
`f229cd652dd5b38ed3a0387a089eab974148d404bd166e8b4c0eb2cb0fa7a2c1`.
The three added negatives are the assignment, `for-in`, and `for-of`
`obj-rest-not-last-element-invalid.js` paths; the separate module negative is
not misclassified as an audited runnable test.

The 355-path / 707-variant tag partitions exactly into:

- 282 paths / 562 variants which change from `unsupported-feature` to pass;
- 72 paths / 144 variants which retain another unsupported feature but lose
  only `object-rest` from their diagnostic;
- one parse-negative row which remains `unsupported-module`.

The established binding (27 paths / 54 variants) and assignment-rest (26 / 51)
certificates are disjoint. Their 53-path / 105-variant union plus a frozen
229-path / 457-variant supplement exactly covers the activation partition.
Five- and eight-worker tag reports are byte-identical. Pinned QuickJS
2026-06-04 passes all 707 tag variants. The candidate tag TSV/JSONL SHA-256
values are
`35fd4c36a2cd8f20b0b13862730e8abde0097017d1671564ef0f97c0481e5af9`
and
`05a1044dc221d3e12d3c7495b4a731d5c07d6bc8415a8a603738ec827091f311`.
The 707-row transition receipt/data hashes are
`38ad31cab30e4a4bbe90ff4fdece5fe907f39d586668cf6cfb505d63d786e003`
and
`07df02d020f8e0e5f7943206f352c56f8c674fc432ae4e4961a84dd32b683e93`.

Nine tag-external object-rest syntax companions are audited separately. Their
18 global rows are byte-identical between parent and candidate: eight pass,
two remain blocked by `class,class-fields-private`, and eight are excluded by
the pinned QuickJS config because they require `Temporal`. Pinned QuickJS
passes the ten non-config companion variants; the shared TSV/JSONL data hash
is `bb11a1c81d6ff7d634a755862970ea2dab820288699f3f3919a5680d33ca40c8`.

The exact full join has 562 outcome changes, 144 detail-only changes, 101,331
unchanged rows, and no previous-pass regression. Every one of the 101,330
non-tag rows is byte-identical, with the companion rows also extracted and
verified explicitly. The new canonical progress vector is:

- 63,831 pass and 64,350 runnable;
- 14,316 `unsupported-feature`, 3,451
  `unsupported-negative-provenance`, and 19,261 total unsupported;
- TSV SHA-256
  `2cf5a7da27e028c4b3d5d91e8f1df43b25fb133714f0cd1ac2bfe64bc2726ac2`;
- JSONL SHA-256
  `665f8c066abb3e894a4c80e86ed0f25dffff14c46b651e2e89e63faecf2cf473`.

Reproduce it with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-object-rest-global.sh --check
TEST262_WORKERS=8 ./scripts/test-test262-object-rest-global.sh
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

This milestone changes only the authenticated capability boundary and
evidence; it does not add runtime shortcuts or claim complete Feature Parity.

## R3cb global DataView admission

R3cb admits the complete pinned `DataView` metadata tag into the live global
profile. The profile grows from 93 to 94 reviewed feature tags while retaining
all 1,154 audited negative paths and the async-execution policy byte-for-byte.
Its SHA-256 is
`b51eee39825e3325effab1c326df30b999e636f67c8ce7bb800f0afdc2d8eab4`.

The exact tag universe contains 190 paths / 380 sloppy-strict variants. It
partitions without overlap into 98 paths / 196 dependency-clean activation
variants, 79 / 158 variants which retain other feature dependencies, and 13 /
26 QuickJS-configuration skips. The established 492-path DataView gate covers
87 activation paths / 174 variants; an explicit 11-path / 22-variant
supplement closes the remainder. Oxide and pinned QuickJS 2026-06-04 pass the
entire 196-variant activation.

The candidate tag report is exactly `pass=196 skipped-feature=26
unsupported-feature=158`. Its TSV/JSONL SHA-256 values are
`c198565eebea7459a7ed76b1d83d28cf3a2fd320c1afcdb519ccc2d6ec339baa`
and
`2793cbdc7ff38beb93a90ffc9f882210b03c9d4852293e5445f63d285ed73f4d`.
The 380-row transition receipt has 196 outcome changes, 158 detail-only
changes, and 26 unchanged skips; its receipt/data SHA-256 values are
`ee4b95f69ef34e41cd69bf10062b0cda302640933432a69b3abe1e4e6c9e38dd`
and
`9136cff39561f0a5c4b28f1ff78731748f7127d3db6a8c77146d93e5bfeca371`.

The exact 102,037-key join keeps all 101,657 non-universe rows byte-identical,
records zero previous-pass regressions, and produces the new canonical vector:

- 63,269 pass and 63,788 runnable;
- 14,878 `unsupported-feature`, 3,451
  `unsupported-negative-provenance`, and 19,823 total unsupported;
- TSV SHA-256
  `324e9d64423494796a9403a7f799f29075a2a98be9d705f7d8310cfb1707bff4`;
- JSONL SHA-256
  `6b68da27cf87198da2c4f2db4e99d1af54b54df2bb936e7d33320f27acee147b`.

Reproduce the focused, admission, and canonical evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-data-view.sh
TEST262_WORKERS=8 ./scripts/test-test262-data-view-global.sh
TEST262_FULL_WORKERS=8 ./scripts/test-test262-data-view-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

This milestone promotes already implemented QuickJS-shaped DataView semantics;
it does not broaden the Feature Parity completion claim.

## R3ca global default parameters admission

R3ca promotes `default-parameters` into the live profile together with the
full untagged non-simple-parameter strict-body cohort. The resulting profile
has 93 feature tags, 1,154 audited negative paths, and SHA-256
`63f139b1a74da9a6114180593770dbcc86bb84fbafab5731f59e1387175c5a6a`.
Its exact delta from R3by is one feature and 230 negative paths: all 219
tagged negatives plus the 11 not already audited among 14 companion paths.

The two disjoint admission partitions behave as follows:

- tag universe: 4,516 variants, with 3,352 new passes, 1,162 residual-feature
  detail changes, and two unchanged `IsHTMLDDA` rows;
- strict-body companions: 28 variants, with 22 new passes and six unchanged
  passes.

The gate independently executes every companion variant through the raw
Oxide worker and pinned QuickJS 2026-06-04, authenticates the exact profile
delta and both manifests, and joins the combined 4,544-row scope against the
complete suite. The full 102,037-row transition has 3,374 outcome changes,
1,162 detail-only changes, 97,501 unchanged rows, and no previous-pass
regression. All 97,493 rows outside the combined scope are byte-identical.

The new canonical progress vector is exactly:

- 63,073 pass and 63,592 runnable;
- 15,074 `unsupported-feature`, 3,451
  `unsupported-negative-provenance`, and 20,019 total unsupported;
- TSV SHA-256
  `2db7d8772074f90de6525cd51ffcd43ea3bf906d78e7c938d452cd6cac21a216`;
- JSONL SHA-256
  `5c201991551f3bb3f03f5a5b232cff0b2470969ae440bc942c324ba4fc5d57a3`.

Reproduce it with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-default-parameters-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-default-parameters-global.sh --full
```

The staging implicit-`this` parameter-eval failure remains in the full vector.
Pinned QuickJS fails the same test with the same behavior, so it is tracked as
shared upstream/spec debt rather than being erased or misreported as an Oxide
parity regression.

## R3bz default parameters certification

R3bz creates a checksum-bound candidate for the exact pinned
`default-parameters` metadata universe. It does not yet change the global
capability profile. The candidate adds exactly that feature plus all 219
tagged parse-negative paths to the R3by parent, growing from 92 to 93 feature
tags and from 924 to 1,143 audited negatives. Its SHA-256 is
`9c345c1e2d79911eec5d6c8750a730f3b3ed0dbefdcd483e0f9c92fcf66aeca0`.

The universe contains 2,269 paths / 4,516 variants and partitions exactly as
follows:

- 1,516 positive paths / 3,013 runnable variants;
- 171 newly audited negative paths / 339 runnable variants;
- 581 paths / 1,162 variants with other unsupported feature dependencies;
- one path / two variants requiring `IsHTMLDDA`.

The parent records 4,514 `unsupported-feature` and two host outcomes. The
candidate passes all 3,352 dependency-clean variants, retains 1,162
`unsupported-feature` and the same two host outcomes, and has zero failure or
unaudited-negative outcomes. Pinned QuickJS 2026-06-04 independently passes
the same 3,352 activation variants. Five- and eight-worker Oxide reports are
byte-identical; their TSV/JSONL SHA-256 values are
`a8047ac4a92d9d482eace99eec54bb361de70b8787c1c55f41a0c98bef89400f`
and
`4eb248df0b35c4ce6aa0e207de3c035d3d6792dabad90a383460b3246f8cb146`.

The gate rebuilds the complete metadata inventory, verifies the exact
feature/audit delta and all four partitions, cross-checks TSV and JSONL, and
authenticates the keyed transition and both engines. It also forces all 219
tagged negative paths / 435 variants through Oxide and pinned QuickJS, so the
48 residual-feature negative paths do not enter the audit list on metadata
alone:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-default-parameters.sh
```

The focused scope deliberately remains the exact feature tag. The R3ca
milestone above performs the separate collateral check for all 14 untagged
non-simple-parameter strict-body paths and admits the 11 new negative paths.
An existing staging failure involving implicit `this` in parameter-expression
direct eval remains visible. Pinned QuickJS 2026-06-04 produces the same
failure, so it is shared Test262/spec debt rather than an Oxide parity blocker.

## R3by global rest parameters admission

R3by adds exactly `rest-parameters` and its 96 audited parse-negative paths to
the live capability profile. The 92-tag / 924-negative-path profile SHA-256 is
`d55e0625b1f6878b7afa6885d82cf332909271ce1c2222100fe3a403a8455969`;
its async-execution policy is unchanged.

The complete 192-row tag join changes every parent `unsupported-feature`
outcome to `pass`, with no residual dependency, module, or configuration
partition. The transition receipt/data SHA-256 values are
`0aa8ac11097f5f81f138c7782b992312003f7ffca6bfad1f92dbb89f6fa8f8ce`
and
`602f57fb32774acc3fbfafa473b339fabca07581ebf882c31b602fa7d698a64b`.
The candidate tag TSV/JSONL SHA-256 values remain
`9db05360e6b8d8199caea374321bdf3808fbd4d06218693212c3f1aeb6669c3d`
and
`4127e8c0b024f7039070352c99232656028b6f2a85e8aa35369e26fd7649fe5f`.

The exact 102,037-key global join preserves all 101,845 non-universe rows and
records zero previous-pass regressions. The new canonical vector is exactly:

- 59,699 pass and 60,218 runnable;
- 18,426 `unsupported-feature` and 23,393 total unsupported;
- 11 parse failures, 400 runtime failures, 57 harness failures, and two
  timeouts;
- TSV SHA-256
  `3268581d1be88057cd4953d8b91401cb6068bff95aa4830d49c77cd902baa9a5`;
- JSONL SHA-256
  `7d1595d9aff6d04c022e688d5e82f32e09a6cfe7adc1f5ea1c0cb21d412933a6`.

The gate authenticates the focused QuickJS oracle, both profile sections, the
tag reports and transition receipt, then performs an exact TSV/JSONL
parent/candidate join over the whole suite:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-rest-parameters-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-rest-parameters-global.sh --full
```

## R3bx rest parameters certification

R3bx freezes the exhaustive `rest-parameters` metadata universe without yet
promoting it into the live capability profile. The candidate differs from the
91-feature R3bw parent by exactly one tag and by all 96 negative paths selected
by that tag; no existing feature, audit, or execution policy is removed.

All 96 paths are generated parse-phase `SyntaxError` cases and expand to 192
sloppy/strict variants. There are no module, QuickJS-config, or residual
dependency rows. The exact transition is therefore 192
`unsupported-feature` outcomes in the parent to 192 passes in the candidate.
Pinned QuickJS 2026-06-04 independently passes the same 192 variants. The
candidate TSV/JSONL SHA-256 values are
`9db05360e6b8d8199caea374321bdf3808fbd4d06218693212c3f1aeb6669c3d`
and
`4127e8c0b024f7039070352c99232656028b6f2a85e8aa35369e26fd7649fe5f`.

The checksum-bound gate rebuilds the complete 53,125-record metadata index,
proves the tag universe equals the 96 audited negatives, authenticates the
one-feature profile delta, checks TSV/JSONL projections and the keyed
parent/candidate transition, and runs both engines:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-rest-parameters.sh
```

Because this Test262 tag is exclusively an early-error cohort, it is evidence
for that complete tag rather than a standalone claim that every rest-parameter
runtime interaction is covered. Existing rest, BindingPattern, direct-eval,
and pinned-QuickJS differential gates remain part of the parity evidence.
R3by above performs the full-suite admission and makes this candidate the live
canonical profile.

## R3bw global computed property names admission

R3bw adds exactly `computed-property-names` to the live capability profile.
The 91-tag profile SHA-256 is
`fc2716ff2ef12fda73c33db0603525f100713ff3b6df0ac8205977a20717ea3a`;
its 828 audited negative paths and single async-execution entry are unchanged
from R3bu.

The complete 946-row tag join contains 439 outcome changes, 456 detail-only
changes, 42 unchanged config skips, and nine unchanged module rows. The full
102,037-key join preserves all 101,091 non-universe rows and records zero
previous-pass regressions. The new canonical vector is exactly:

- 59,507 pass and 60,026 runnable;
- 18,618 `unsupported-feature` and 23,585 total unsupported;
- 11 parse failures, 400 runtime failures, 57 harness failures, and two
  timeouts;
- TSV SHA-256
  `574d90530b5815329e65ab55d94bce4dd684233f1b296a888c87eced9077ba69`;
- JSONL SHA-256
  `6d7ec82af17368ebea46213633efcec331198cf904db457434b7493b003e9616`.

The gate cross-checks TSV and JSONL projections, the complete parent/candidate
tag transition, all four tagged partitions, the byte-identical non-universe,
the canonical 102,037-key join, and the focused QuickJS oracle:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-computed-property-names-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-computed-property-names-global.sh --full
```

## R3bv computed property names certification

R3bv creates a checksum-bound scoped candidate for
`computed-property-names`; it does not yet add the tag to the live global
profile. The 91-tag candidate differs from the R3bu parent by exactly that one
feature, while the 828 audited negatives and async-execution entry remain
byte-identical. Its SHA-256 is
`fc2716ff2ef12fda73c33db0603525f100713ff3b6df0ac8205977a20717ea3a`.

Pinned metadata yields 478 paths / 946 variants, partitioned without overlap
into 220 / 439 activation, 228 / 456 residual-capability, 21 / 42
QuickJS-config-skip, and nine / nine module rows. The candidate report is
exactly `pass=439 skipped-feature=42 unsupported-feature=456
unsupported-module=9`; its TSV/JSONL SHA-256 values are
`f29e969d8ce120fbbeba909265515a35219c68a621cc8892488137fc8fb55b56`
and
`ff01f50adcdc58253e55df2d13f01d960694c3a654548c3b7bc8a60148a5f3ba`.
Pinned QuickJS 2026-06-04 also passes all 439 activation variants. The gate
verifies the exhaustive metadata-derived universe, both profile sections,
all path and variant-key fingerprints, the parent/candidate keyed transition,
both report formats, and the QuickJS differential:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-computed-property-names.sh
```

The eventual global admission is expected to expose 439 new passes and only
narrow 456 residual diagnostics, but the canonical 102,037-row baseline stays
at R3bu until that full join is rerun and independently checked. R3bw above
performs that admission and confirms the projection exactly.

## R3bu global resizable ArrayBuffer admission

R3bu adds exactly `resizable-arraybuffer` to the live capability profile. The
profile grows from 89 to 90 reviewed feature tags while its 828 audited
negative paths and async execution entry remain unchanged. Its SHA-256 is
`e9c1ca295ca9270391f128c3f58484be3ac03a2a649b0170b551d41ab542f898`;
the ordered 90-feature stream has SHA-256
`2c02df29f05b4d3303da0c26784f7f6eab7a83d4f20caf7b3e5747bcaed7de42`.

The complete pinned metadata universe for this tag contains 463 paths / 926
sloppy and strict variants. Exactly 381 paths / 762 variants depend only on
the newly admitted tag, 80 paths / 160 variants retain other feature gaps,
and two paths / four variants remain QuickJS-config skips. The focused R3bt
gate authenticates the full 762-variant activation in both Oxide and pinned
QuickJS; it also proves that the 312 paths covered by earlier ArrayBuffer,
DataView, and TypedArray gates plus the 151-path spillover exhaust the
universe. Its Oxide TSV/JSONL SHA-256 values are
`79baa1c1e323cb1256f3e0f7bdfbc403f3732100f40be807f63dfed6d84ab70c`
and
`8a0ed16786ae3ecec118e1fd84392cb6857fb3c9ecb57d6977e7b962ed8bb0da`.

The parent tag report has zero runnable rows, 922 `unsupported-feature`
outcomes, and four skips. The candidate has 762 passes, 160 residual
`unsupported-feature` outcomes, and the same four skips. Its TSV/JSONL
SHA-256 values are
`dbaa0982f04607a08819b73e2505f20c83e62ed9670b779043764f0ce2a8053b`
and
`54882a02e660938116bc4acb7a9fa5d9efeb611bb8e5bd0ac3780d3e4a8ecd37`.
The exact 926-row transition receipt and data SHA-256 values are
`f51b57c077fcbea258c39907ed95cffa50280d4af11ef355076bcad675959c0e`
and
`2dab3bfcf31b227e34920dbdbf5e3cd47c7a16fb32abda95c14d88d10965d335`.

The complete 102,037-key join contains exactly 762
`unsupported-feature -> pass` transitions and 160 diagnostic-only changes.
All four config skips and all 101,111 non-universe rows are unchanged, and no
previous pass regresses. The R3bu vector reached 59,068 passes with
59,587 runnable variants, 19,057 `unsupported-feature` outcomes, and 24,024
total unsupported outcomes. That is 57.89% raw, a 70.69% conservative
target-scope lower bound after the 18,475 pinned QuickJS target exclusions,
and 99.21% among 59,538 variants with a non-unsupported observed outcome. The
candidate full TSV/JSONL SHA-256 values are
`a21d195a1a6209c5df6b7080a9a941d773c87abeed7ec63961b5896b1b294045`
and
`834754d9d6ab62606c3463b351932dedade8e9f78ba6ea835a87aa743cf9fb41`.
Independent eight-, four-, and two-worker candidate runs are byte-identical.

Reproduce the evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-resizable-arraybuffer.sh
TEST262_WORKERS=8 ./scripts/test-test262-resizable-arraybuffer-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-resizable-arraybuffer-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

This is a global profile and evidence milestone, not a Feature Parity
completion claim.

## R3bs global Uint8Array base64/hex codec admission

R3bs adds exactly `uint8array-base64` to its checksum-pinned capability
candidate after the R3br focused gate authenticates all six codec entry points.
That transition grows from 88 to 89 reviewed feature tags while its 828 audited
negative paths and async execution entry remain byte-identical. The parent and
candidate profile SHA-256 values are
`5d3543018b022f968e4d7bb1725cef1c0e101e3c61a4d2d35f2c77df5ec975e9`
and
`ed80ab5aed86c606a1d7b5c1854b78ab1bb3c517cf0c6898a89e9f8d19135000`;
the candidate's ordered feature stream has SHA-256
`593a376a65171a87d8c12df6834570322657ab42d6d48560a7ca14df5c6e7e96`.

The tagged universe is exactly R3br's exhaustive 69-path / 138-variant
manifest. The parent admits none of it and records 138
`unsupported-feature` outcomes for exactly `uint8array-base64`; the candidate
admits and passes all 138. The transition receipt and data SHA-256 values are
`d3c7b72f7dfaea4523c7378deedbd5f9b2f3a8aca26dcbdf3f86727b1f1fb2c5`
and
`ce4172f23d0e5986b85171c2b85201f20b96e3f772d684d5ffd050c0f88010ad`.
The parent tag TSV/JSONL SHA-256 values are
`b2363e1c5a942b21565c3b756f94b566ce84a93e4e39e0eed2ac0671f0ee773c`
and
`1c0f0ab3a4442c3cf3d0648f50b68cfbbb8af25fd8ad4d2e7545ef758525594c`;
the candidate values are
`2a2c523a9d02087a72eca78a94cbe785fac269e81815a3065f8daa0b3ca87fe2`
and
`fbc2b21194d9fece3e2b9d7afc1d906d8576eaafa0c6608fa3cca7020a69a127`.
Independent eight- and five-worker tagged reports are byte-identical.

The complete 102,037-key candidate join has exactly 138
`unsupported-feature -> pass` transitions. All 101,899 non-universe rows are
unchanged, there are zero detail-only changes, and no previous pass regresses.
The candidate vector contains 58,306 passes, 58,825 runnable variants, 19,819
`unsupported-feature` outcomes, and 24,786 total unsupported outcomes. That is
57.14% raw, a 69.78% conservative target-scope lower bound after the 18,475
pinned QuickJS target exclusions, and 99.20% among 58,776 variants with a
non-unsupported observed outcome. The candidate full TSV/JSONL SHA-256 values
are
`789b1d116e10dbeb7607faf4bbbcb5df818a6e588799d156579b5047238b0379`
and
`d1476490e0f53bb1397ce432c813c781e51130cfd97da22e1fdc8edc10f95a8f`.

Reproduce the admission with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-uint8array-codecs-global.sh
TEST262_WORKERS=5 ./scripts/test-test262-uint8array-codecs-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-uint8array-codecs-global.sh --full
```

This is a global profile and evidence milestone, not a new implementation
slice or a Feature Parity completion claim.

## R3br focused Uint8Array base64/hex codec gate

R3br freezes the complete Test262 cohort for
`Uint8Array.fromBase64`/`fromHex` and
`Uint8Array.prototype.toBase64`/`toHex`/`setFromBase64`/`setFromHex`.
The scoped profile contains exactly three feature tags:
`Reflect.construct`, `TypedArray`, and `uint8array-base64`. Its complete
SHA-256 is
`2e8f870a5c6d1c05adc37c759098d2412943beff8b8de3c1593ba74df7761ac9`,
and its ordered feature stream has SHA-256
`41acf42eb5acbf12874115c7cbc757d7cb3e2ddd26603a55b55fbf95bb90532e`.

The exhaustive sorted manifest contains 69 paths / 138 sloppy/strict
variants. Its path-stream SHA-256 is
`cbde75ee5038f3c24abfbf8f6e2734494281163bbe36370d0c81443da02a660c`,
its complete-file SHA-256 is
`2a52c3f54ef83a8df736e823d76e17927b670045f42d338d42a64f0e48681bb2`,
and its 138-key stream SHA-256 is
`e55870b3ba3591f83a43fb3e58c0beb6be7de35916aa6efdbde1844f4f9ba628`.
All 138 variants are runnable and pass in Oxide, with zero failures,
unsupported outcomes, or skips. Pinned QuickJS independently executes and
passes the same 138 variants.

The classified Oxide TSV/JSONL SHA-256 values are
`4862f2570cf27fed439f3bd4c731b520b2ebac1643a5b257aaa21d112592742b`
and
`04395a486012a649f6cba508791ebd83367a4e0db2cb7d418ec0bcc302b46663`.
The empty non-pass stream has SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`;
independent eight- and five-worker Oxide reports are byte-identical. The same
gate first runs a ten-vector Oxide-versus-pinned-QuickJS differential covering
descriptors, options, brands, capacity, partial writes, buffer invalidation,
WTF-8, realms, and exact errors.

Reproduce the complete focused evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-uint8array-codecs.sh
TEST262_WORKERS=5 ./scripts/test-test262-uint8array-codecs.sh
```

At R3br, `uint8array-base64` had not yet been added to the then-live 88-tag
global profile, so that milestone did not publish a new full-corpus vector.
R3bs now performs the separate global admission above.

## R3bq global Promise capability closure

R3bq adds exactly `Promise`, `Promise.allSettled`, `Promise.any`, and
`Promise.prototype.finally` to the checksum-pinned live capability profile.
It now contains 88 reviewed feature tags, the same 828 audited negative paths,
and the same async execution entry. Its SHA-256 is
`5d3543018b022f968e4d7bb1725cef1c0e101e3c61a4d2d35f2c77df5ec975e9`.
The pinned QuickJS Promise jobs, `Promise.prototype.finally`, and aggregate
differential gates all pass before the Test262 admission runs.

The exhaustive tagged universe contains 226 paths / 452 variants. The exact
activation is 208 paths / 416 variants, all passing after admission. The other
18 paths / 36 variants remain `unsupported-feature` only for their residual
dependencies: 12 variants require `class`, and 24 require
`computed-property-names`. The historical parent tag TSV/JSONL SHA-256 values
are
`623a2e0fecca4a2746b667ea0552b9621a89bc8f1448a1c3b1aa7f557e487b1a`
and
`fff4000cdd7f160f12e7495f09f6f995e0be2d96452ffbecdce54822a50c2ed5`;
the candidate values are
`500d94a18e8872bdd9df1bf87cb535cee41a3632a922575f6a11699170662c2d`
and
`04f9e7b06d26709b507a9809e8f757a075811f4069345f070513c86b60ee29b3`.
The exact 452-row transition receipt and data SHA-256 values are
`955e77db96a429533b946fac4de9f9c0808f793a1506fddec2d2ab29eb1e91d8`
and
`0831ea9577c8ae2c9ddf7a84903ffaaa49882e1f9fd889570740d1d3da3a91b4`.

The full join retains all 102,037 keys: 416 outcomes change from
`unsupported-feature` to `pass`, 36 reason-only rows change diagnostic detail,
and all 101,585 non-universe rows are byte-identical. No previous pass
regresses. The canonical vector reaches 58,168 passes with 58,687 runnable
variants, 19,957 `unsupported-feature` outcomes, and 24,924 total unsupported
outcomes. That is 57.01% raw, a 69.61% conservative target-scope lower bound
after the 18,475 pinned QuickJS target exclusions, and 99.20% among 58,638
variants with a non-unsupported observed outcome. Its full TSV/JSONL SHA-256
values are
`4a529df1318a233d16de1e3563de3e987a4a51f200bb6d37e73281142e51e19a`
and
`80006172f384144bb3f169ba56d587bb2f48f5e21cdaadde0308e0fcde386df9`.
Eight- and five-worker tagged runs are byte-identical; two- and one-worker
full runs are also byte-identical, and an independent two-worker canonical
repeat matches the candidate.

Reproduce the evidence with:

```sh
TEST262_WORKERS=8 ./scripts/test-test262-promise-global.sh
TEST262_WORKERS=5 ./scripts/test-test262-promise-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-promise-global.sh --full
TEST262_FULL_WORKERS=1 ./scripts/test-test262-promise-global.sh --full
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

This is a global profile and evidence milestone, not a Feature Parity
completion claim.

## R3bp global `globalThis` admission

At R3bp, `globalThis` was added to the capability profile frozen by the R3bo
focused gate. The resulting historical profile contained 84 reviewed feature
tags, the same 828 audited negative paths, and the same async execution entry.
Its complete SHA-256 is
`caa287cbf8188ea1c0519daa7d77fc5adb63d98c523299377eec14730b54cd15`;
the feature-section SHA-256 is
`e928613f44d53e2d3690a5305ae29a707b30fc66ec0a797016b46d2460b39423`.

The complete tagged universe remains 148 paths / 165 variants. The admission
changes exactly the 135-path / 150-variant activation from
`unsupported-feature` for only `globalThis` to `pass`. The remaining 13 paths
/ 15 variants are byte-identical: 11 module variants remain
`unsupported-module`, and the four variants requiring
`explicit-resource-management` remain `skipped-feature`. The candidate tag
TSV/JSONL SHA-256 values are
`fe95410a26b918c8aeb2aab5218fc653aeee7bc7cba1b8d1bc44b67deebe11d2`
and
`917e99fb2cb41ae7698d376f4a93e078bede865c749f59e0be1a06f8503c947a`.
The 165-row transition receipt and data SHA-256 values are
`46c161ade8b302c99167d0837c18e0991cab40b9ae9129fd2ec45719ba418507`
and
`d4351933687b1ee1a284c84868af09f158584806a3a23545bf53b1d373491466`.
Independent eight- and five-worker tagged reports are byte-identical.

The exact full-corpus join retains all 102,037 keys. Exactly 150 rows move
from `unsupported-feature` to `pass`; all 15 deferred rows, all 101,872
non-`globalThis` rows, and all 101,887 unchanged rows remain byte-identical in
both report formats. There are zero detail-only changes and zero previous-pass
regressions. The R3bp canonical vector reached 57,752/102,037 passes with
58,271 runnable variants, 20,373 `unsupported-feature` outcomes, and 25,340
total unsupported outcomes. That is 56.60% raw, a 69.11% conservative
target-scope lower bound after the 18,475 pinned QuickJS target exclusions,
and 99.19% among 58,222 variants with a non-unsupported observed outcome. Its
full TSV/JSONL SHA-256 values are
`1dfbd54d69e3ebace9edfb1ba3502d402edbd1919f34a353c8996eec63522a0d`
and
`f255a6852b17479e0d699195e2b50477e5094113861672587852b04bb3ed9668`.
The candidate reports from the two-worker admission run and independent
one-worker frozen-vector reproduction are byte-identical to the independent
canonical two-worker live repeat.

Reproduce the evidence with:

```sh
./scripts/test-test262-global-this.sh
TEST262_WORKERS=8 ./scripts/test-test262-global-this-global.sh
TEST262_WORKERS=5 ./scripts/test-test262-global-this-global.sh
TEST262_FULL_WORKERS=2 ./scripts/test-test262-global-this-global.sh --full
TEST262_FULL_WORKERS=1 ./scripts/test-test262-global-this-global.sh --full
```

This changed only the global capability profile and its evidence. It is not a
Feature Parity completion claim.

## R3bo focused `globalThis` gate

R3bo freezes the complete `globalThis` metadata universe at 148 paths / 165
variants. The exact activation contains 135 paths / 150 variants; the
remaining 13 paths / 15 variants are a disjoint deferred ledger of 11 module
paths / 11 variants and two `explicit-resource-management` paths / four
variants excluded by the pinned QuickJS config. The universe, activation, and
deferred manifest SHA-256 values are
`aecc6d30cc47676fd20541c509c1016b3cd8d238e96afa6178d3f0c2bd62abc4`,
`4d8be634488c72eafbbd350f0d75829f4d3f71fb4b141db192e5f69ace41ea23`,
and
`989c02dd93d888cad5116edb9e00a047b4843fbffa3a0ac86145907e593dd75c`.
Reproduce the gate with `scripts/test-test262-global-this.sh`.
All eight negative paths are deferred module-resolution `SyntaxError` tests;
the activation has no negative or `$262` host-requirement path. Pinned
QuickJS passes all 150/150 activation variants.

The frozen parent profile is the historical 83-tag global profile with
SHA-256
`8a3b253f6d2a24b18f9bec66628ba5aec3fb337d677c60bfde37c4c3a33d3910`.
It has 0 runnable activation variants and all 150 rows are the exact
`globalThis`-only `unsupported-feature` vector. The candidate profile adds
only `globalThis`, has SHA-256
`caa287cbf8188ea1c0519daa7d77fc5adb63d98c523299377eec14730b54cd15`,
and admits and passes all 150/150 variants. The 150-key transition receipt
and data SHA-256 values are
`33cc8a8ffd153694a0f0d331c75f777e859a0de39bf227e1ff441ba1e1e73193`
and
`f43cb0f5682c394eeacffdee49dc1353f9fd92cf792efbd04831272a6779eb97`.

Independent eight- and five-worker runs are byte-identical. The parent
TSV/JSONL SHA-256 values are
`46850bdc3e24aeda34b5dfb26fec33cae85b9bdce2fc8c75e43e26bcb4d035c5`
and
`b2db5df01d15118155f20453adaaefeba0bffe6b54759177d1ba11c15d181736`;
the candidate values are
`21b125444add1d6e114670e69e9510e305b659608f41b59a4a6a46ab5a419c2e`
and
`73b47ebf51b0cdb70112654eb0791e1a01a8729e952192dd02ddb23112dbd75d`.

R3bo changes no production runtime semantics. At that milestone it did not
add `globalThis` to the 83-tag live global profile or publish a new complete
classified vector. R3bp later performs that global admission above.

## Pinned inputs

- Test262 commit: `5c8206929d81b2d3d727ca6aac56c18358c8d790`
- QuickJS patch SHA-256: `f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3`
- QuickJS config SHA-256: `79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b`
- quickjs-oxide 132-tag capability profile SHA-256:
  `47cf8351f7844340bbbff3ba9bb781faf552f8f27d0dd6cca2e35dbf9ad48232`
- 53,125 non-fixture metadata records SHA-256:
  `a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a`

`scripts/prepare-test262.sh` prepares and verifies that exact checkout and the
two harness changes carried by the QuickJS release. No Test262 source is
vendored into the product.

## Smoke baseline

`tests/test262-smoke.txt` fixes 100 synchronous script tests. They expand to
193 independent sloppy/strict variants:

- all 193 pass;
- 0 unsupported, failed, skipped, timed-out, crashed, or infrastructure-faulted
  variants.

The final two former frontiers are parse-negative tests which put class
declarations in a single-statement `if` position. They now pass for the
intended early `SyntaxError`, rather than because class parsing is absent.

This 193/193 result is a runner smoke baseline, not a project-wide 100%
estimate. The sample was selected from already implemented synchronous
surfaces. Modules, most `$262` host hooks, the Test262 agent host and
agent-backed waiter cases, and many other broad built-ins remain absent.
`Atomics.waitAsync` is outside the pinned QuickJS target. Proxy is
measured by the checksum-bound
R3am scoped gate and admitted globally by R3bh below.
The pure ArrayBuffer core is measured by the checksum-bound R3an gate, its
DataView layer by R3ao, and the shared 12-class TypedArray kernel by R3ap.
R3aq promotes the TypedArray mutation cohort, and R3ar promotes the indexed
`at`/search cohort. R3as promotes the callback-driven
`find`/`findIndex`/`findLast`/`findLastIndex` cohort, and R3at promotes
`every`/`some`; R3au promotes `forEach`, and R3av promotes
`reduce`/`reduceRight`; R3aw promotes species-aware `map`/`filter`, and R3ax
promotes `slice`/`subarray` copying and view creation. R3ay promotes
change-by-copy `with`/`toReversed`, and R3az promotes dedicated
`join`/`toLocaleString` stringification plus inherited `toString`. R3ba
promotes QuickJS-shaped `sort`/`toSorted`, and R3bb authenticates the existing
shared `entries`/`keys` iterators. R3bc authenticates static `TypedArray.of`
and its shared static-`from`/`of` constructor diagnostic seam; R3bd
authenticates static `TypedArray.from`, and R3be admits the global
`TypedArray` tag after freezing its exact activation and spillover partitions.
The cumulative scoped TypedArray gate reaches 2,254 paths / 4,463 variants.
R3br separately implements and authenticates the Uint8Array codec surface, and
R3bs admits its feature tag globally. R3bt authenticates the complete
resizable ArrayBuffer activation and spillover, and R3bu admits its feature
tag globally. R3bv authenticates computed property names, and R3bw admits that
feature globally. Modules, the Test262 agent host and agent-backed waiter
cases, and broad built-ins remain explicit frontiers. `Atomics.waitAsync` is
outside the pinned QuickJS target.
Ordinary async functions/jobs are measured by the scoped R3ab-refreshed R3z
gate, async arrows by the R3ab gate, and ordinary async object-literal methods
by the R3ac gate. Public ordinary async class methods are measured by R3ad and
ordinary private async class methods by R3ae; ordinary async-generator
functions are measured by R3af and ordinary object-literal async-generator
methods by R3ag; public instance/static class async-generator methods are
measured by R3ah and private instance/static class async-generator methods by
R3ai. Async-generator `yield*` delegation is measured across all four shapes
by R3aj. R3ak first measured `for await ... of` across ordinary async functions
and all four async-generator shapes; R3bk refreshes that focused gate after
optional-chaining admission. The active outer-iterator `.return()` path is
locked separately by the pinned QuickJS differential. Public fields, static
blocks, private elements, and
public/private synchronous generator methods are measured by the scoped
R3g/R3h/R3i/R3j/R3k/R3l gates below.

Synchronous Iterator Helpers were first authenticated by R3v. The historical
R3bm focused gate completed the 28-path source-and-harness Proxy closure and
passed 551 paths / 1,102 variants in both engines. R3bn admitted exactly
`iterator-helpers` globally: 1,076 variants activated and passed, 26 remained
fail-closed behind `globalThis`, and 32 host/config variants remained
unchanged. R3bp subsequently admitted `globalThis`: those 26 Iterator Helper
variants and the rest of the 150-row activation pass, while its 15
module/config variants remained unchanged.
R3bq subsequently admitted the four implemented Promise tags: 416 activation
variants pass, and 36 reason-only variants remain fail-closed behind `class`
or `computed-property-names`.
R3br subsequently authenticated all 138 Uint8Array codec variants, and R3bs
admits their `uint8array-base64` tag globally. All 138 now pass under the live
profile without changing any non-universe outcome.

Nineteen additional provenance variants guard the result: 10 audited negative
variants pass for the intended parse error, while nine variants fail closed
behind unsupported features or unaudited negative provenance instead of
passing because they happened to throw a `SyntaxError`.

## Complete classified vector

The pinned suite expands to 102,037 sloppy/strict variants. The runner emits
every outcome in canonical order. The current R3dl canonical summary is:

- 66,476 pass;
- 18,475 are outside the pinned QuickJS target configuration;
- 17,034 are classified as unsupported because of a feature, mode, host
  capability, parser/runtime/harness frontier, or unaudited negative-test
  provenance, including 12,761 `unsupported-feature` variants;
- seven fail to parse, 43 fail at runtime, none fail in the harness, and two
  time out; there are no crashes or runner/engine infrastructure faults.

The runner admits 66,528 variants to execution. All 66,528 produce a
non-unsupported observed outcome; the runnable count can otherwise include
typed parser/runtime frontiers or harness failures.

Three rates answer different questions:

- raw suite pass rate: 65.15% (`66,476 / 102,037`);
- conservative target-scope lower bound: 79.55%
  (`66,476 / (102,037 - 18,475)`);
- pass rate among variants with a non-unsupported observed outcome: 99.92%
  (`66,476 / 66,528`).

The 79.55% figure is the useful whole-project progress floor, not a claim that
the engine is 79.55% conformant. The 99.92% conditional rate measures quality
only on the currently exposed frontier and must not be read as overall
completion. It can move in either direction as classification improves: R2p
lowers it slightly by admitting 204 real, independent non-Symbol frontiers that
had previously failed closed as unsupported features; R2q then raises it
slightly as 31 untagged binding variants become real passes, R2t resolves two
more typed parser frontiers, R2u adds 15 array-assignment passes without
admitting additional jobs, R2v resolves 14 untagged object-assignment
frontiers, and R2w resolves 23 parser frontiers, 24 runtime frontiers, and two
ordinary runtime failures on the synchronous catch-binding surface. R2x then
adds 88 passes from the synchronous identifier-rest surface and its untagged
harness consumers without admitting additional jobs. R2y adds another 60
passes from synchronous identifier defaults and moves direct-eval,
destructuring, class, and missing-intrinsic consumers to their deeper explicit
frontiers, again without changing the runnable count. R2z then adds 22 passes
from synchronous no-default parameter BindingPatterns, while moving 11 old
failures to the deeper Parameter-Environment frontier and keeping the runnable
count fixed. R3a adds 12 passes from the combined parameter-expression and
BindingPattern path, moves two typed runtime frontiers to their already-known
adjacent failures, and again keeps the runnable count fixed. R3b adds 66 passes
from direct eval in non-simple Parameter Environments; one untagged staging
variant reaches its known implicit-`this` runtime mismatch and two reach the
generator-method typed runtime frontier, while the runnable count remains
fixed. R3e then adds 328 passes from the dependency-audited synchronous base
class slice, exposes adjacent derived/class-element and missing-intrinsic
frontiers, and again keeps the runnable count fixed. R3f adds 545 passes by
opening synchronous heritage/derived construction, while 88 adjacent variants
move from parser/harness frontiers to honest missing-intrinsic, optional-chain,
or pinned-target-error outcomes. The capability profile currently admits 132
reviewed Test262 feature tags and 1,197 reviewed
negative-test paths; all other feature-tagged or
negative-provenance cases fail closed. Expanding that profile as implementation
lands can only make the measurement more representative. Focused QuickJS
differential tests remain the semantic judge.

R3s originally admitted `RegExp.escape` only in its checksum-bound complete
RegExp built-ins profile; R3dg later admitted that implemented tag globally.
R3t likewise authenticates synchronous `generators` plus
`destructuring-binding` in a checksum-bound scoped profile. R3u promotes that
authenticated synchronous cohort into the global profile while keeping its
three async-function/arrow adjacencies fail-closed.

The complete TSV/JSONL reports are generated under `target/` rather than
committed (together they are tens of megabytes). Their complete hashes and
outcome summary are pinned in `tests/test262-full-baseline.txt` after
reproduction. Runner ordering was cross-checked at five and eight workers
through the scoped RegExp modifier milestone. The canonical full gate now uses
two workers because its 30-second budget is wall-clock based and CPU-heavy
generated cases become scheduler-sensitive under higher worker contention.
R3am makes the untagged 15,000-key `Proxy/ownkeys-linear.js` path linear enough
to pass in both modes; only the two `JSON/parse-mega-huge-array.js` modes still
time out. Focused gates and the generic runner retain their existing parallel
defaults. The current byte expectations use a fixed
`TZ=America/Los_Angeles`; the hash gate therefore requires a Unix-like zoneinfo
installation, and Windows still lacks the corresponding IANA-zone backend.
Two independent R3dl full runs are byte-identical. Their canonical TSV/JSONL
SHA-256 values are
`501b64ed5c8367f33408225d956a262619163adf52baadf28f02811d14f3eae9`
and
`610e16ba65a0239556842efec7a745ba2885c72dfb3b8447c2578b8767ef7d40`.
They supersede the R3dj receipt as the current complete vector.

## Milestone policy

Test262 is now the project-wide milestone scoreboard, while the pinned QuickJS
source and focused differential probes remain the semantic specification for
each feature slice. A substantial slice lands only after its Rust/unit and
QuickJS differential gates pass; the full Test262 vector then records pass
movement, regressions, newly exposed failures, and unsupported-frontier
movement. Small implementation commits do not need an independent full-suite
run.

The preceding simple-parameter `arguments` milestone moved 17,365 to 18,011
passes and exposed `Math.pow` as a common harness blocker. The Math milestone
moved the complete vector from 18,011 to 21,429 passes with no previous-pass
regression. Its exact old/new join matched all 102,037 keys: all 4,435 outcome
changes are inside the 4,589-variant reviewed set, with zero outcome drift
among the other 97,448 variants.

The reviewed set now has 3,420 passes and 1,169 non-pass outcomes. Every one of
the 568 runnable `built-ins/Math` variants passes; 86 more remain explicitly
unsupported because they also require other unimplemented feature tags. The
3,770 `propertyHelper.js` variants now split into 2,755 passes, 897 runtime
failures, four harness errors, 52 parse failures, and 62 explicit parser
frontiers.

The keyed transition audit records 2,763 `harness-error -> pass`, 897
`harness-error -> fail-runtime`, 639 `fail-runtime -> pass`, 62
`harness-error -> unsupported-parser`, 56 `harness-error -> fail-parse`, 16
`unsupported-feature -> pass`, and two `fail-runtime -> timeout`. Those two
timeouts are the sloppy and strict variants of
`staging/sm/String/fromCodePoint.js`: implementing `Math.pow` lets them reach
their 49,152-argument `apply` stress path, so they record a performance
frontier rather than a Math semantic regression.

The Reflect milestone moves the complete vector from 21,429 to 21,740 passes
and from 31,873 to 32,227 runnable variants. An exact keyed join again matched
all 102,037 variants: precisely 371 outcomes changed, every one inside the
427-variant reviewed Reflect manifest, with no previous-pass regression and no
outside-manifest drift. The transitions are 294 `unsupported-feature -> pass`,
38 `unsupported-feature -> fail-runtime`, ten `unsupported-feature ->
fail-parse`, six `unsupported-feature -> harness-error`, six
`unsupported-feature -> unsupported-parser`, and 17 `fail-runtime -> pass`.

The current 427-variant focused Reflect vector admits 405 variants: 387 pass,
18 fail at runtime, and 22 remain gated by adjacent features. R2f moved four
concise-method parser frontiers to runtime assertions; R2g then made four
independent getter consumers pass. Later aggregate refreshes exposed the
already-landed downstream fixes, including both variants of the Reflect.apply
rest-parameter case. R3al reconciles ten more such passes that were already
present in the R3ak complete vector, so this focused-baseline catch-up is not a
new whole-suite transition. The other non-pass results continue to expose
ArrayBuffer, async/generator, JSON, TypedArray, parser, or adjacent-feature
frontiers rather than being hidden from the scoreboard. Current focused
TSV/JSONL SHA-256 values are
`a7f21f90b63f4067b217d3730676b6ddb6797f9b13af30fadc166630c854398e`
and
`a408f81f5910ac44d23fe62fd54bef4c7bfaeb5c6f7f308c9b254ef0081b0a3b`.

The observable Date milestone moves the complete vector from 21,740 to 23,016
passes without changing its 32,227 admitted jobs. An exact keyed join across
all 102,037 variants records exactly 1,276 `fail-runtime -> pass` transitions,
no previous-pass regression, and no outcome change outside the reviewed Date
manifest. Five- and eight-worker full reports are byte-identical.

The Date-focused review corpus contains 799 paths and 1,598 sloppy/strict
variants. Its Date-owned subset contains all 646 paths and 1,292 variants from
`built-ins/Date`, `annexB/built-ins/Date`, and `staging/sm/Date`; 153 adjacent
paths expose Date through globals, reflection, constructors, or indirect
dependencies. The current focused outcome vector has 1,552 passes, two runtime
failures, 34 configured/feature skips, and ten explicit `create-realm` host
frontiers. The runner admits 1,554 jobs, all of which now have an observed
non-unsupported outcome, for a 99.87% pass rate on that frontier (97.12% of the
complete focused vector). R2f resolved 62 former concise-method parser
frontiers; R2g then resolved the final ten accessor variants. Later milestones
lifted additional downstream blockers without changing the Date-owned surface.
The remaining runtime and host frontiers stay explicit.
Current focused TSV/JSONL SHA-256 values are
`751cdacad364af8f324df0ddaa5aa446a28963e0933a836f90c23b5a0600364e`
and
`e9d2e46bc6cccde539ae8b5950837f469862f3e1462cb966b346ca37034745ea`.
The six grouped QuickJS differentials, one oracle vector self-check, two
cross-realm/GC integration tests, and 44 Date unit tests pass. Reproduce the
hash-pinned focused vector with `scripts/test-test262-date.sh`; both it and the
full-vector command fix `TZ=America/Los_Angeles`. The Date-landing focused and
full reports were byte-identical at five and eight workers on the required
Unix-like zoneinfo host.

The generic `String.prototype.split` milestone moves the complete vector from
23,016 to 23,190 passes and from 32,227 to 32,289 admitted jobs. Its exact keyed
join matches all 102,037 variants and records 220 changes with no previous-pass
regression: 158 `fail-runtime -> pass`, 16 `unsupported-feature -> pass`, 30
`unsupported-feature -> fail-parse`, 12 `unsupported-feature -> fail-runtime`,
and four `unsupported-feature -> unsupported-parser`. Of those changes, 172
are inside the focused manifest and become passes; outside it, the existing
`Symbol.split` descriptor test contributes two more passing variants and 46
`RegExp.prototype[Symbol.split]` variants move from feature-gated to explicit
RegExp parser/runtime/parser-frontier outcomes.

The focused split corpus contains 127 paths and 254 sloppy/strict variants:
all 120 `built-ins/String/prototype/split` paths, one Annex-B IsHTMLDDA path,
and six direct consumers selected from the previous full vector. At the
generic-split landing it had 186 passes, 52 runtime failures, eight
feature-gated outcomes, six typed parser frontiers, and two host-capability
outcomes. Declaring `Symbol.split` meant that the well-known symbol and
generic/custom-splitter delegation were audited, not that the then-unpublished
RegExp protocol was complete; exposing those outcomes made the next semantic
frontier visible.

R1e activates that existing delegation through
`RegExp.prototype[Symbol.split]`. At that landing, the same frozen vector
admitted 244 variants and recorded 234 passes, four runtime failures at the
independent missing-global-`eval` frontier, eight adjacent feature outcomes,
two IsHTMLDDA host outcomes, and six typed parser frontiers. Its TSV and JSONL
SHA-256 values are
`ad66315d9b6d285240d9f0628a899ab71b64496ea451f153bcf4916d7ffeccdb`
and
`c0182c6f56c9df1cb4b1e991f60aa94aa5c8173e01f7882e7fa4031e966eaebc`.
The capability profile remains unchanged at 18 reviewed tags.
R1p's Annex B named-backreference parser resolves the two
`separator-regexp.js` variants. R1x then executes the two eval consumers. The
R2c Arrow and R2f concise-method slices resolve the remaining parser consumers.
The current gate admits and passes 252 variants; two require IsHTMLDDA.
Current TSV/JSONL hashes are
`13f8c26ce2c9cd93904ce420cc00010e06e60f1eedccd7e22cc2f1e98fdb1303`
and
`eb88da8a2773b80e436c9311ba39f0868c623555e6679aeff4761ef631e5f26d`.

The RegExp R0 foundation deliberately did not increase the pass count. It
added the internal UTF-16 compiler/executor and heap brand while `%RegExp%`
remained unavailable. A static RegExp-core manifest froze 225 untagged
`built-ins/RegExp` paths and 450 sloppy/strict variants as a zero-pass named
implementation queue rather than a feature claim.

The parser now selects the RegExp lexical goal when `/` or `/=` begins a
primary expression. An exact join across all 102,037 full-vector keys records
1,312 classification-only changes and no pass regression: 1,209
`fail-parse -> unsupported-parser` transitions plus 103
`harness-error -> unsupported-harness-parser` transitions. Every old result
was `SyntaxError: unexpected '/'`; every new result is the typed RegExp-literal
frontier, including 73 harness users of `nativeFunctionMatcher.js` and 30 of
`sm/non262-Math-shell.js`. No flags, feature metadata, expected phase or
actual phase changed. The same reclassification moves 2 Reflect, 8 Date and 38
String-split focused variants without changing their 321, 1,282 and 174 pass
counts. Five- and eight-worker R0 full reports were byte-identical at 23,190
passes and 32,289 admitted jobs.

The RegExp R1a observable shell publishes the constructor, ordinary prototype,
species, source/flag accessors, `exec`, abstract RegExpExec/`test`, `toString`,
`lastIndex`, captures and `d` indices while continuing to reject advanced
grammar explicitly. At the R1a landing, the 450-variant core vector recorded
430 passes, ten `fail-runtime` outcomes caused only by the separate
missing-`eval` frontier, and ten `unsupported-runtime` outcomes. The later R1f
Unicode decimal-escape classification refinement moves both variants of
`unicode_restricted_octal_escape.js` to pass, so the core vector at R1f had
432 passes and eight typed advanced-pattern outcomes. The R1a full vector moves
from 23,190 to 23,859 passes, reduces `fail-runtime` from 4,540 to 3,861, and
adds ten typed `unsupported-runtime` outcomes. RegExp literals, legacy
`compile`, `RegExp.escape`, and Symbol protocols were not claimed by that
slice.
An exact join matches all 102,037 `(path, variant)` keys with no duplicates or
missing rows and zero previous-pass regressions. Its only 679 transitions are
669 `fail-runtime -> pass` and ten
`fail-runtime -> unsupported-runtime`. The new passes span 462 RegExp, 132
Object, 42 Array, 12 String, nine language-expression and 12 adjacent global,
literal or staging variants; those collateral groups construct or consume
regular expressions rather than representing unrelated feature work.
Subsequent RegExp grammar slices moved the same core gate to 436 passes; R1p's
Unicode bare-`\k` diagnostic resolves two more, and R1x executes the five eval
consumers. R3s resolves the final two typed legacy-control frontiers, so the
current core vector passes all 450 variants. Its TSV/JSONL hashes are
`ec6298bec9cd1f268a5e36ef725ea196d44a13a6d7ed0e3b53791edb853c1021`
and
`7702a505d3ad53624cd6f7975bb55973c89eac5b1b6edcc9fdb0d6dc1fd693e9`.
The frozen core vector is reproduced by
`scripts/test-test262-regexp-core.sh`.

The RegExp R1b literal slice follows QuickJS's compile-once/instantiate-many
model: a typed compiled-pattern constant is linked into bytecode, and
`Instruction::RegExp` creates a fresh RegExp with the execution realm's
canonical shape and prototype on every evaluation. Pattern diagnostics remain
at compile time. `tests/test262-regexp-literals.txt` freezes 48 paths and 96
sloppy/strict variants; `tests/test262-regexp-literals-baseline.txt` pins the
classified TSV/JSONL hashes plus the R1a selection provenance, and
`scripts/run-test262-regexp-literals.sh` reproduces both checks. At the R1b
landing, the focused vector had 88 passes, two `fail-runtime` outcomes and six
typed `unsupported-parser` outcomes: two lookaround and four backreference
variants. Relative to R1a, all 88 passes move from the typed RegExp-literal
parser frontier. The two runtime variants still stop at an earlier
`String.prototype.match` call; R1d later makes both pass, moving the vector
from 88 to 90 passes. R1k resolves four linked backreference variants and R1l
the final two lookahead variants, so the current focused gate passes all 96.
The complete R1b vector moves
from 23,859 to 24,699 passes while the 18,475 target exclusions and 32,289
admitted jobs stay unchanged. Its exact 102,037-key join has 1,193 transitions:
840 `unsupported-parser -> pass`, 226 `unsupported-parser -> fail-runtime`, 24
`unsupported-parser -> fail-parse`, and 103
`unsupported-harness-parser -> harness-error`. There are no previous-pass
regressions. The focused vector remains an independent, faster reproduction
gate rather than a substitute for that full baseline.

The RegExp R1c search slice publishes `String.prototype.search` and
`RegExp.prototype[Symbol.search]` with the QuickJS conversion, delegation,
abstract-RegExpExec, `lastIndex` SameValue reset/restore, result-index and realm
boundaries locked by eight Rust tests, including nine QuickJS differential
vectors and one cross-realm runtime test. `tests/test262-regexp-search.txt`
freezes all 66 search paths and their
132 sloppy/strict variants from the R1b report;
`tests/test262-regexp-search-baseline.txt` pins both the R1b selection
provenance and current outcome hashes, and
`scripts/run-test262-regexp-search.sh` reproduces the gate. It now admits and
passes 128 variants, while four outcomes remain gated by adjacent feature
requirements. R2g resolves the final 12 accessor consumers. At R1b the same
keys were 2 passes, 60
runtime failures and 70 feature-gated outcomes, so the focused slice contributes
110 new passes. Eight more search-enabled variants outside the frozen manifest
pass, for 118 new full-vector passes in total.

The complete R1c vector moves from 24,699 to 24,817 passes and from 32,289 to
32,353 admitted jobs. Its exact old/new join matches all 102,037 keys, has zero
previous-pass regressions, and records only 66 `fail-runtime -> pass`, 52
`unsupported-feature -> pass`, and 12 `unsupported-feature ->
unsupported-parser` transitions. The parser transitions are the explicitly
bounded object-literal grammar frontier, not search algorithm drift.

The RegExp R1d match slice publishes `String.prototype.match` and
`RegExp.prototype[Symbol.match]` with QuickJS 2026-06-04 delegation,
conversion, abstract-RegExpExec, global-loop, empty-match UTF-16 advance,
mutation and realm boundaries locked by 11 passing Rust
oracle/differential/cross-realm/recursion tests. The String entry shares the
isolated generic protocol helper with search; the 155-line RegExp algorithm
lives in `runtime/intrinsics/regexp/match_protocol.rs` rather than the runtime
facade. The shared four-active-frame native recursion guard remains an explicit
non-parity frontier: the fifth mixed match/search frame throws `InternalError`,
where pinned QuickJS continues.

`tests/test262-regexp-match.txt` freezes all 104 match paths and their 208
sloppy/strict variants from the R1c report;
`tests/test262-regexp-match-baseline.txt` pins both the R1c selection provenance
and current outcome hashes, and `scripts/run-test262-regexp-match.sh`
reproduces the gate. It now admits and passes 206 variants, while two outcomes
remain gated by `regexp-v-flag`. R1x executes the
legacy eval consumer. At R1c the
same keys were two passes, 76 runtime failures and 130 feature-gated outcomes.
The focused TSV and JSONL SHA-256 values are
`5aa6b8b6c61a48acf72417d583f3439b8fbfc5dde9020b8c8341e31759a790a6`
and
`5f3e63c0d709819e47a57e4bfbb3929a565b615d74a6a95966b3dc19c90948e2`.

The complete R1d vector moves from 24,817 to 25,029 passes and from 32,353 to
32,497 admitted jobs. Its exact old/new join matches all 102,037 keys with no
missing, extra or duplicate rows and zero previous-pass regressions. The only
230 transitions are 86 `fail-runtime -> pass`, 126 `unsupported-feature ->
pass`, 16 `unsupported-feature -> unsupported-parser`, and two
`unsupported-feature -> fail-runtime`. Those two are the sloppy/strict variants
of one Annex-B path that at R1d reaches the then-unimplemented
`RegExp.prototype[Symbol.split]`. The two literal-focused variants noted at R1b
now pass, independently moving that gate from 88 to 90 passes. The full
eight-worker TSV/JSONL hashes are
`a695d6299b44e4298b553c28c12983b6b12fc9d8522f1216e18e16a6bad28012`
and
`fb305cd709b2af1bf28de5fc82b440f836a0567ff8ed3e36af967723e3beb64b`.

The RegExp R1e split slice publishes
`RegExp.prototype[Symbol.split]` and activates the existing generic
`String.prototype.split` delegation for RegExp separators. Its dedicated
237-line algorithm and reusable SpeciesConstructor helper follow QuickJS
2026-06-04 construction, flags/sticky handling, abstract RegExpExec, UTF-16
advance, capture insertion, limit, mutation, abrupt-completion and realm
boundaries. Only four facade lines are added to `runtime.rs`. Eight Rust tests
cover 19 QuickJS differential vectors.

`tests/test262-regexp-split.txt` freezes 46 direct paths and their 92
sloppy/strict variants from the R1d report;
`tests/test262-regexp-split-baseline.txt` pins the R1d selection provenance and
current outcome hashes, and `scripts/run-test262-regexp-split.sh` reproduces the
gate. It now admits and passes 50 variants. Forty core variants
remain conservatively gated by the undeclared `Symbol.species` profile tag, two
require the create-realm host hook, and R2g resolves the four former accessor
parser frontiers. The QuickJS differential suite separately locks
SpeciesConstructor semantics
without widening the full-suite capability profile. Its current TSV and JSONL
SHA-256 values are
`377746133482618291d3948d5a2da8a30f2cd7c6a7ca9cf3fce3589f426b8be5`
and
`853e1dcd3353307b0c6e2b71f4acfa3df3014f9c1dd516caad6d3f62a3f51629`.
The independent 127-path String split gate now records 252 passes. Its current
TSV/JSONL hashes are
`13f8c26ce2c9cd93904ce420cc00010e06e60f1eedccd7e22cc2f1e98fdb1303`
and
`eb88da8a2773b80e436c9311ba39f0868c623555e6679aeff4761ef631e5f26d`.

The complete R1e vector moves from 25,029 to 25,119 passes while admitted jobs
remain at 32,497. Its exact R1d/R1e join matches all 102,037 keys with no
missing, extra or duplicate rows and zero previous-pass regressions. The only
outcome transitions are 90 `fail-runtime -> pass`; five- and eight-worker TSV
and JSONL reports are byte-identical. The full hashes are
`5673ac15896bab5b1665bf8930db517447012c3d63d69bfbb1da9b8e7f9574c1`
and
`fe98f9fdb5f4c21c25cd045d8b1824fe34e3481e26c8661376d7afe78596fa64`.
The summary now has 3,847 runtime failures, four timeouts and 2,251 typed parser
frontiers; all other outcome counts are unchanged. The two variants of
`staging/sm/RegExp/split.js` retain their `fail-runtime` classification but now
reach the independent missing-JSON-global frontier, so that detail change is
not an outcome transition. The capability profile remains at 18 tags with
SHA-256
`cc10293aa847f5a449ac2b039709dff98d264b672dddc8828b8e17d8b7e12d9a`.

The RegExp R1f slice publishes the pinned legacy
`RegExp.prototype.compile` mutation. A dedicated 35-path/70-variant vector
freezes the complete Annex-B compile directory and every pinned-suite source
which directly invokes the method. It records 44 passes: all executable core
compile variants plus the four linked RegExp split variants. The sloppy/strict
variants of one staging replace path still stop first at the independent
missing `@@replace` protocol; feature, host, arrow and object-method parser
frontiers remain explicitly classified.
The focused TSV/JSONL SHA-256 values are
`1f1fb2ff6dfe5cd5dde0445e60daa310fa5b8056dfeeddac83bf3a81f0d74874`
and
`60fbf6017a8302242f5d8fa9de929e7fe39d59d7a7993631d69cc05030c56f43`.

R1f also refines Unicode decimal-escape classification at the pure RegExp
compiler boundary. The two variants of
`unicode_restricted_octal_escape.js` move from typed Unsupported to pass, so
the 450-variant RegExp-core gate now records 432 passes, ten missing-`eval`
runtime failures and eight advanced-pattern frontiers. Its TSV/JSONL SHA-256
values are
`a650f0855a4585f81c3b4c3d8df2da8ab2b9f4771ad1f94f90be0390db5c6b2b`
and
`123eae124abc4ff59475df4a028f1aafef5bb16b10c12e88d0b2a5bb2ce10e90`.

The exact R1e/R1f full join covers all 102,037 keys with no missing, extra, or
duplicate rows and zero previous-pass regressions. Its only transitions are 44
`fail-runtime -> pass` and two `unsupported-runtime -> pass`. The full vector
therefore moves to 25,165 passes while admitted jobs remain 32,497;
`fail-runtime` falls to 3,803 and `unsupported-runtime` to eight. Five- and
eight-worker reports are byte-identical. The full TSV/JSONL hashes are
`57caefa97b579fafeb6b56ba45da7daf9cbe5e168849e4ab0459b87452d4745e`
and
`613a396d850698fff9472991e547946eac6bc9bc4f3b95cf90ce57d85953dee0`.

The RegExp R1g slice ports the pinned scoped modifier grammar
`(?ims-ims:...)`. The focused manifest freezes every Test262 path whose sole
feature tag is `regexp-modifiers`: 230 paths and 460 sloppy/strict variants.
All 460 are admitted, 448 pass, and the remaining 12 stop at existing typed
frontiers: four backreference variants and eight Unicode property-escape
variants. There are no modifier-owned parse or runtime failures. The focused
TSV/JSONL SHA-256 values are
`b9baafd9e3d49b1cda6a6a5b99bbddc5ae938aa494c35bd31e1a1ceccb545c68`
and
`cf2e6a818da59c66735d46f429b885c916454cf4a2b160f6b2d10dd2b40b8e86`.

Publishing this feature also audits exactly 83 modifier-owned literal
parse-negative paths. The capability profile therefore moves to 19 feature
tags and 101 exact negative paths, with SHA-256
`0d26aedd5b5d7fa00b6c2551a93c7d776f22e2934b790615d6dc58c454156d5f`.
Because that hash is part of every report header, all earlier focused report
hashes change mechanically. Their manifests have zero overlap with the new
feature, and replacing only the R1g profile hash with the R1f value reconstructs
every previous TSV/JSONL hash exactly; their outcome rows, summaries, and
historical provenance are unchanged.

The exact R1f/R1g full join covers all 102,037 keys with no missing, extra, or
duplicate rows, no change outside the `regexp-modifiers` feature, and zero
previous-pass regressions. Its only transitions are 448
`unsupported-feature -> pass` and 12
`unsupported-feature -> unsupported-parser`. The complete vector moves to
25,613 passes and 32,957 admitted jobs. Five- and eight-worker reports are
byte-identical. The full TSV/JSONL hashes are
`5ece50a681fcb4fe97779002b179174930d2cdbdb4bd2120e0679678bd96b161`
and
`83539d1bcea789f87853cdc6d9862dd2741d61a5b6696e8513e551318c9e5df8`.

The R1h replacement slice publishes `String.prototype.replace`,
`String.prototype.replaceAll`, and the generic
`RegExp.prototype[Symbol.replace]` path. Its frozen manifest contains 191 paths
and 376 variants. At the R1h landing the profile admitted 332 variants and
recorded 286 passes with zero runtime failures. Six variants failed to parse,
40 stopped at typed parser frontiers, 38 at other undeclared features, and six
at host capabilities. The R1h focused TSV/JSONL hashes are
`055d52219998a0863a4241b3c5b374b917c1503d93b0715048ee2e171db3d012`
and
`dffcdbd8260a3d6e1c277d76797ba7187e40a971860ff802efaf8b3c6e65c0ad`.
R1i's direct standard-RegExp route preserved that outcome vector. At R1p the
gate admitted 348 variants and recorded 300 passes. The current vector admits
and passes 362 variants; eight retain adjacent feature requirements, two
require create-realm, and four require IsHTMLDDA. Current focused TSV/JSONL
hashes are
`0dccee6d3228b5c665a9f2c42890e46345d865bb0905020224e04e1b35589a94`
and
`facaadcafe19ae3444b8aa0ae2b7467519037f9c4ee4dc0bfa6f1bd07e8c98a2`.

Publishing `String.prototype.replaceAll` and `Symbol.replace` moves the
capability profile to 21 reviewed feature tags, with SHA-256
`921df0ef452f4d1286162093ebdf81a74d0805eb7c04601c86abd6ec7347ed7f`.
The Test262 worker also installs the pinned qjs-compatible `print` host surface
before raw or harness scripts, while raw tests still receive no Test262
harness.

The exact R1g/R1h full join covers all 102,037 keys with no missing, extra, or
duplicate rows and zero previous-pass regressions. Its transitions are 110
`fail-runtime -> pass`, 170 `unsupported-feature -> pass`, four
`unsupported-feature -> fail-parse`, and 38
`unsupported-feature -> unsupported-parser`. The complete vector moves to
25,893 passes and 33,169 admitted jobs. The full TSV/JSONL hashes are
`2895a8d2ddbe5857e83b573827e46b4a60a97d89b5882727c85ff75d2ff9d368`
and
`64fed7fd3bb722d470bbd420e42995e138aed5d6f3588b7d2657973cb3968419`.
Earlier focused vectors retain their outcome rows and update only their profile
metadata hashes, except the compile vector, whose two linked staging replace
variants now pass and move that focused result from 44 to 46 passes.

R1i implements the branded standard-RegExp direct replacement matcher and its
raw, AutoInit-sensitive predicate. This changes observable getter traffic on
already-passing programs but does not add a Test262 capability, manifest path,
or runnable variant. The focused replacement gate remains byte-identical at
286/376, with TSV/JSONL hashes
`055d52219998a0863a4241b3c5b374b917c1503d93b0715048ee2e171db3d012`
and
`dffcdbd8260a3d6e1c277d76797ba7187e40a971860ff802efaf8b3c6e65c0ad`.
The complete gate likewise remains byte-identical at 25,893/102,037, with
TSV/JSONL hashes
`2895a8d2ddbe5857e83b573827e46b4a60a97d89b5882727c85ff75d2ff9d368`
and
`64fed7fd3bb722d470bbd420e42995e138aed5d6f3588b7d2657973cb3968419`.
The exact R1h/R1i join therefore has zero transitions and zero previous-pass
regressions; focused QuickJS differentials, rather than pass-count movement,
are the acceptance evidence for this semantic-path milestone.

R1j publishes `Symbol.matchAll` and `String.prototype.matchAll` together with a
QuickJS-shaped RegExp String Iterator. Its static manifest is the complete
68-path union of the RegExp protocol, iterator prototype, and String entry
directories, expanding to 136 variants. The post-implementation vector admits
112 variants and records 64 passes; the other 72 remain explicitly classified
at unrelated feature, parser, or harness frontiers. The focused TSV/JSONL
hashes are
`03def26414f02bf5056ebb1421a28d28178c29946b07fc8d0e085fdbb9bfe72b`
and
`b020aa4bd8cd878a8b96aa66b1736eee991df4fc87b6adda3510101a0a911fd8`.

The exact R1i/R1j full join covers all 102,037 keys with no previous-pass
regression. Sixty-six variants move from `unsupported-feature` to pass; 20
reach an existing harness-parser frontier, 28 reach an existing parser
frontier. The complete vector moves to 25,959 passes and 33,283 admitted jobs.
Its
TSV/JSONL hashes are
`5f0e4601ce6b0212dacdd5c98fc1ba4cb2c8c217e3f0eb6c91411ad6e3f243fa`
and
`a829007d38ffe4bd84b7420200b0fef505671808e1a003326c2fccb6383edcd6`.
At R1j the capability profile contained 23 reviewed feature tags and had
SHA-256
`5aaca9f98ddca05a2bcb3bb6dfdc297f3f27a8314cb6efde61b25c2944548fd9`.
Earlier focused outcome rows remain unchanged; their whole-report hashes move
mechanically because this profile hash is part of every report header.

R1k ports numeric RegExp backreferences together with QuickJS's inseparable
non-Unicode Annex B decimal/octal fallback. The static focused manifest covers
49 paths and 98 variants, including syntax-priority canaries and linked
lookaround/named-group frontiers. At R1k, 74 variants were admitted; R1l
resolved four linked lookahead variants, R1o resolved 14 linked lookbehind
variants, and R1p admits the final six named-group variants. Later object
binding support resolves the last four parser frontiers, so the current gate
passes all 98 variants. Current focused TSV/JSONL hashes are
`fc91f2bc073844d86dc5b4c4b739da40e41a21267fde6f61d8fc6792d2b6c9a4`
and
`7ab11b9287f97ea7faf73331501b7fff2624a7892467b8f68879da2e155a1d8c`.

The exact R1j/R1k full join adds 68 passes with no previous-pass regression:
62 variants move from `unsupported-parser`, two from `unsupported-runtime`,
and four audited Unicode parse-negative variants from
`unsupported-negative-provenance`. The complete vector reaches 26,027 passes
and 33,287 admitted jobs. Its TSV/JSONL hashes are
`0bdf4955b2a9060279d0ad4232f653adb2018e9864654148f068caf22c0aabd6`
and
`7fcfbcd8157fa1d21d52af7df7e3b2226db7be08bfe42254994a28d56a5b9857`.
The profile still has 23 feature tags, now with 103 exact audited negative
paths and SHA-256
`6f27d9fcfa5a13423796ad48fe8ccbf8d5edcd49118ad7f0f64cc5a936090645`.

R1l ports forward positive and negative lookahead using QuickJS-shaped paired
assertion instructions and typed control frames on the existing non-recursive
executor stack. Positive success discards internal alternatives while
retaining captures and compacting their undo records into any surviving outer
transaction; negative completion always rolls assertion-local state back.
Non-Unicode assertions retain Annex B quantification, while `u` mode preserves
the distinct `*`/`+`/`?` versus brace-quantifier syntax priorities.

The static focused manifest covers 26 paths and 52 variants. All 52 are
admitted and pass. Its TSV/JSONL hashes are
`f4087df9d8fb3a91b9f92e733ba4568c62c6c083a340a27b449ecec54deb025b`
and
`18551f6e79bc933a9337b5709011657b9c94e46be7f77120049a63e9753761fb`.
The exact R1k/R1l full join converts 50 `unsupported-parser` and two
`unsupported-runtime` variants to pass, with no other category movement and no
previous-pass regression. The complete vector reaches 26,079 passes while
admitted jobs remain 33,287. Its TSV/JSONL hashes are
`9a60ea477bb8d383b316b9418683865031b43b3609400d7bcacb448cb535a85b`
and
`b69f3de1d2e61d3cb7667e6de1ffe2f5a811569df83b1cf34929008aaf8e393a`.

R1m ports `u`-mode Unicode property escapes from pinned QuickJS. The generated
Rust catalog contains 38 General_Category sets, 176 Script sets, 176
Script_Extensions sets, and 55 accepted binary properties. Exact aliases,
errors, non-`u` identity behavior, `\P` inversion-before-folding under `iu`,
scoped modifiers, astral code points, lone surrogates, and class-range
priorities are locked by 37 match and 28 compile/error oracle vectors.

The focused Test262 manifest contains the 144 direct property-escape paths
which do not require the generated helper corpus, plus four scoped-modifier
canaries. All 296 variants pass. Its TSV/JSONL hashes are
`66a129065346b23b454c6275b15301508bc8a4afaf6dacd8a473d6a948b7c392`
and
`87b704d71d7d8e33403abd81445cfd302c136fc2de30308c7f7caf9ceed9d869`.
The profile now contains 24 feature tags and 245 exact audited negative paths,
with SHA-256
`6d5bb9a92d00babb6a4a0bcb19334fbcfcd532bb5382ce278ce85a960d40d781`.

The exact R1l/R1m full join adds 298 passes and admits 1,170 more jobs:
288 variants move from `unsupported-feature` to pass, ten move from
`unsupported-parser` to pass, and 882 generated Unicode-property variants move
from `unsupported-feature` to the existing harness-parser frontier. There are
no previous-pass regressions or other category changes. The complete vector
reaches 26,377 passes and 34,457 admitted jobs. Its TSV/JSONL hashes are
`275fd8b3f6b1e5f078b6aad58bfc33797abaf6637179f47cc52228bc8f52feda`
and
`c2e14d42cfbb933946d9ce738d27c371e15fa3b9865131c2a6160cfe70b480f9`.

R1n completes that generated Unicode-property data tranche without claiming
general destructuring support. The compiler lowers identifier-only
`const`/`let`/`var` array BindingPatterns in synchronous `for-in`/`for-of`
through nested QuickJS-shaped iterator records; holes, empty patterns, trailing
commas, early exhaustion, abrupt close order, and fresh lexical cells are
covered. Assignment patterns, object patterns, defaults, rest, and nested
patterns remain typed frontiers.

The Test262 worker now publishes only the QuickJS
`js_string_codePointRange` native helper needed by the pinned harness, with
exact `ToUint32`, UTF-16, realm, function-metadata, and non-constructor
behavior. Other `$262` hooks remain absent. RegExp normalized range lookup uses
binary search, matching the upstream data-plane shape instead of scanning up
to 1,372 intervals for every input code point.

The cumulative Unicode-property gate is now 589 paths and 1,178 variants: the
original 148 paths plus all 441 generated code-point property files. Every
variant passes. Its TSV/JSONL hashes are
`1cc6e3fec21a989c4a916a5dcfd069c9600efaa03883611a7dc5888ead73dd48`
and
`8b0dd3a9e76c7795f945631987f4dbd1ab3c5596dfda921993ea4594cb2f072e`.
The 28 generated properties-of-strings files remain behind the unimplemented
RegExp `v`-mode frontier.

The same harness change advances 20 already-tracked MatchAll variants:
14 pass and six reach the object-literal method/accessor parser frontier. The
complete R1m/R1n join matches all 102,037 keys. Its 930 outcome changes are
896 `unsupported-harness-parser -> pass`, six
`unsupported-harness-parser -> unsupported-parser`, 20
`unsupported-parser -> pass`, six `unsupported-parser -> fail-runtime`, and
two `unsupported-parser -> fail-parse`. All 935 changed complete rows are
inside the pre-audited 475-path set; there are no previous-pass regressions or
outside-set changes. Admitted jobs remain 34,457, while the complete vector
reaches 27,293 passes. Its TSV/JSONL hashes are
`6035ae86888c4db9e99b73be65e706bf7b90ee83c108082a3e7931f2000edc61`
and
`fb37235d0d651a2d424cb4f63c16b6662813183f25fd2126e970bacb3506c50d`.

R1o ports positive and negative variable-length lookbehind through the same
non-recursive assertion controls used by R1l. Code generation retains
alternative priority while reversing each alternative's terms, emits
QuickJS-shaped `Prev` instructions around ordinary consuming atoms, swaps
capture boundaries, and compares participating numeric backreferences
right-to-left without crossing the capture start. Nested lookahead/lookbehind,
greedy and lazy captures, assertion atomicity and rollback, anchors, word
boundaries, scoped case folding, and UTF-16/Unicode reverse movement are
covered by 42 match and ten compile/error vectors against pinned QuickJS.

The frozen focused gate contains 27 paths and 54 variants. At R1o, its 17 pure
lookbehind paths and eight audited parse-negative paths contributed 50 passing
variants while four co-tagged named-group variants stayed gated. R1p resolves
those four, so the current gate passes all 54. Focused TSV/JSONL hashes are
`590b466885fe087bc30cb02e1adc1b1076af0322e229a998af8cda3a680131dd`
and
`5aca0c7d11afea0d6c1facd893663ad2000f7a95860703112c641dd8a8fa914c`.

The exact R1n/R1o full join matches all 102,037 keys. It records 34
`unsupported-feature -> pass` and 16
`unsupported-negative-provenance -> pass` transitions; all 50 outcome changes
and 54 complete-row changes are inside the frozen set, with no previous-pass
regression or outside-set drift. The complete vector reaches 27,343 passes and
34,507 admitted jobs. Its TSV/JSONL hashes are
`50fe24e393c2532e2c25fc2113e6bbb48c163678a6bc8a0991f8c6ad0d8273c1`
and
`c997357b861109bfd17c46ad0c8059004f2b797cf9254394b90892dca078810b`.

R1p ports ordinary named captures and named backreferences from pinned
QuickJS. The runtime-independent compiled program stores normalized
`JsString` names aligned to captures 1..N, while the matcher reuses the
existing multi-capture forward/backward backreference instructions. The
parser preserves QuickJS's fixed group-name buffer, Unicode 17 identifier
rules, Annex B `\k` fallback, wrapping global alternative scope, and
forward-name scan cursor quirk. Match `groups` and `indices.groups` are
null-prototype C/W/E objects with QuickJS duplicate-name value and insertion
order. Named replacement deliberately leaves the direct helper before any
state mutation and uses the generic `$<name>` path.

Fifty-nine execution, grammar, construction, result, replacement, and
QuickJS-quirk vectors match the pinned oracle; a separate Rust test locks
defining-realm ownership. At the R1p landing, the frozen focused gate is 101
paths and 202 variants: 184 are admitted and 158 pass. Six reach pre-existing
arrow-function parse failures, 20 reach typed
class/object-method/destructuring parser frontiers, and 18 stay behind
`regexp-match-indices`, `Symbol.iterator`, or class syntax. At that landing,
the 19 paths tagged with
`regexp-duplicate-named-groups` remained outside that declaration even though
the lower-level QuickJS duplicate selection behavior was implemented; R1q
audits them below. R1p focused TSV/JSONL hashes are
`505845ba54ec78ae1a636f91f7285e447444d3ffca8b66a03592591573a15d26`
and
`5daec58cf49af34cdf2ad8e70d5a945513e6490180ab4c74e9e996f39d4fa234`.
Later object-binding and rest-parameter milestones move the frozen gate to 194
passes; R3f derived construction resolves its remaining four class frontiers,
so the current gate has 198 passes, two feature-gated variants, and two at an
unrelated runtime frontier.
Current TSV/JSONL hashes are
`37d54ae152bd48b0fc35625d4776e082c3baa2b4024382bd274f0633ea2323e3`
and
`b96318614cf6bd6a9d0d8b1c360cccd0a2f12131f59988baba24002201aff846`.

The exact R1o/R1p full join again matches all 102,037 keys. It records 158
`unsupported-feature -> pass`, six `unsupported-feature -> fail-parse`, 20
`unsupported-feature -> unsupported-parser`, two `unsupported-parser -> pass`,
and two `unsupported-runtime -> pass` transitions. There are 188 outcome and
204 complete-row changes with no previous-pass regression. Four changes lie
outside the frozen manifest and are explicit linked `\k` canaries: the
Unicode restricted-identity-escape test now receives the required
`SyntaxError`, and the String split separator test now receives Annex B
identity-escape behavior when no named capture exists. The vector reaches
27,505 passes and 34,691 admitted jobs. Full TSV/JSONL hashes are
`ff31a5f63b2b9e27f5650dd99c301cbff9c863314cce48e592f97b6ca1df2704`
and
`e1766ea22ab3e33ef610310a6d83ce101eb66dcfa598d581ebaed257295e9402`.

R1q declares the duplicate-named-capture feature after a separate pinned
QuickJS source and adversarial-probe audit confirmed that R1p already mirrors
the target's global wrapping 8-bit scope, nested-alternative leakage,
multi-capture backreference selection, reset behavior, result order, indices,
and replacement semantics. No production engine change is needed.

The frozen focused gate is the complete 19-path/38-variant feature set. It
admits 32 variants and passes 26 at the R1q landing. Six variants in the
constructor,
`RegExp.prototype.compile`, and matchAll syntax tests reach the existing arrow
parser frontier; the six match-indices co-tagged variants remain gated in that
historical report and are admitted by R1r.
Focused TSV/JSONL hashes are
`bd55aacd10c14cf1f0f7a38e11a610ad3763bce8c4f326c9a6ae3ad548a8ef30`
and
`1b9dc971d9c965910b7e0bd88573e80553d17b74651c0ef4762dd34d998cc666`.

The exact R1p/R1q full join matches all 102,037 keys. It records 26
`unsupported-feature -> pass` and six
`unsupported-feature -> fail-parse` transitions. All 32 outcome changes and
38 complete-row changes stay inside the focused manifest, with no
previous-pass regression. The complete vector reaches 27,531 passes and
34,723 admitted jobs. Full TSV/JSONL hashes are
`16759de6e768905a3feae8dc96889936668838f42b64217bd70776cb6e56db96`
and
`36b947828eda57d0216d84e623b6af51143d26586860db3639cc3875765fc7e0`.

R1r declares `regexp-match-indices` after pinned QuickJS source review and
focused probes confirmed that the existing production engine already matches
the target's `d` flag and canonical flag order, `hasIndices`, UTF-16 match
ranges, unmatched-capture `undefined` values, null-prototype named
`indices.groups`, duplicate-name selection, construction/legacy-compile
behavior, and observable descriptors. No production engine change is needed.
Seven dedicated differential tests lock result/pair descriptors,
low-surrogate `lastIndex`, protocol propagation, replacement non-observation,
and nested defining realms against the pinned oracle.

The frozen focused gate contains 31 paths and 62 variants. At the R1r landing,
it admits 50 and passes 38; two variants fail at the existing arrow-function
parser frontier, four stop in the existing `deepEqual.js` harness frontier,
and six reach the typed object-setter parser frontier. Ten remain behind the
independently gated `regexp-dotall` feature in that historical report and are
admitted by R1s, while two retain the missing `$262.createRealm` host
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

The exact R1q/R1r full join matches all 102,037 keys. It records 38
`unsupported-feature -> pass`, two `unsupported-feature -> fail-parse`, four
`unsupported-feature -> harness-error`, and six `unsupported-feature ->
unsupported-parser` transitions. All 50 outcome changes and ten detail-only
changes stay inside the focused manifest, for 60 complete-row changes and no
previous-pass regression. The complete vector reaches 27,569 passes and
34,773 admitted jobs. Full TSV/JSONL hashes are
`e09478accaf05c27e39555c5a4c1889617c97ce5c1454ddf945c7f675ea3d2ef`
and
`95ea74491558035ac02af4f60c3a2d202120798fc2ab08c41c7050a6031e950b`.
The capability profile now contains 28 reviewed feature tags and 307 audited
negative paths, with SHA-256
`b39bee15a2aaa88e00c8f7ca6cb0736313456d43a77e176a8c5cf7844e9ea718`.

R1s declares `regexp-dotall` after a pinned QuickJS source review and focused
probes confirmed that the existing Rust implementation already follows the
target end to end. The `s` flag uses QuickJS's bit, the compiler selects the
all-character instruction instead of ordinary dot, UTF-16 and Unicode width
come from the shared executor, scoped modifiers restore their enclosing state,
and the constructor, legacy `compile`, accessors, canonical flags, protocols,
species paths, and defining-realm brand checks retain the flag exactly. No
production engine change is needed. Six dedicated differential tests cover the
oracle-vector self-check, line-terminator and UTF-16 matching, the public and
construction surface, nested scoped modifiers, matchAll/split species flags,
and cross-realm getter brands and error realms.

The frozen focused gate contains all 17 paths and 34 variants tagged with
`regexp-dotall`. At R1s it admitted 26 and passed 18, with linked Arrow,
accessor, `u180e`, `regexp-v-flag`, and create-realm frontiers kept visible.
Later slices resolve Arrow and `u180e`; R2g resolves the final four accessor
consumers. The current gate admits and passes 30 variants, while two remain
behind `regexp-v-flag` and two retain the missing `$262.createRealm` host
requirement. Its exact outcome summary is
`pass=30 unsupported-feature=2 unsupported-host-create-realm=2`. Focused
TSV/JSONL hashes are
`3d5bda20dece92150f0398cb6f2d70a4114ff46fea69c7326ef056e439c7e246`
and
`a584c2db7b136338cb5ea9ca5116572f17ce2121740b5670889ab035e979bd23`.

The exact R1r/R1s join matches all 102,037 keys. It records 18
`unsupported-feature -> pass`, four `unsupported-feature -> fail-parse`, and
four `unsupported-feature -> unsupported-parser` transitions. All 26 outcome
changes and six detail-only changes stay inside the frozen manifest, for 32
complete-row changes and no previous-pass regression. The complete vector
reaches 27,587 passes and 34,799 admitted jobs. Full TSV/JSONL hashes are
`44f7ee3d6de6c97962c4b372da2f492882b8834d76663b334dd46265fae9e69f`
and
`fa263cbcd0483000f0645f017d486e4a4403d5227b97ce3bf5e812bf8a6857ce`.
The capability profile now contains 29 reviewed feature tags and 307 audited
negative paths, with SHA-256
`84fe6615092829a107e66beb49ac54b00a1910616424494f47e5f75c8ccc7880`.
The admission and differential locks add no production code; `runtime.rs`
remains 9,677 lines.

R1t declares `u180e` after pinned QuickJS source review and focused probes
confirmed that the existing Rust implementation already matches the target
where that semantics is implemented. U+180E is not ECMAScript whitespace or a
line terminator; it is preserved in comments and literals, rejected between
tokens, rejected by Number conversion, honored as a prefix-parser stopping
point, retained by trim, skipped as Case_Ignorable for Final Sigma, excluded
from RegExp `\s`, and matched by dot and `\S`. Seven dedicated differential
tests lock these lexer, conversion, string, casing, and RegExp boundaries.
Global `eval` and JSON are recorded as independent subsystem frontiers rather
than receiving U+180E-specific production code.

At the R1t landing, the frozen focused gate contained all 25 paths and 50
variants tagged with `u180e`. All 50 were admitted and 40 passed. Ten variants
failed at runtime because the five `*-eval.js` paths required the then-missing
JavaScript global `eval`; the exact parse-negative path was separately
provenance-audited and passed as a real SyntaxError. Its historical summary was
`fail-runtime=10 pass=40`. R1t focused TSV/JSONL hashes are
`3e42dd0c0e7272d51f02a03f95c1d907218b9f3ee5e29a20c0c6760565fbaf0c`
and
`4d6e6d514c9a4e6108f828b57b53507e24564df2d0a670a31132a878dbbc8d5c`.

The exact R1s/R1t join matches all 102,037 keys. It records 40
`unsupported-feature -> pass` and ten `unsupported-feature -> fail-runtime`
transitions. All 50 outcome and complete-row changes stay inside the frozen
manifest, with no detail-only changes or previous-pass regression. The
complete vector reaches 27,627 passes and 34,849 admitted jobs. Full
TSV/JSONL hashes are
`7ea006b596e26f56712c9618f74cd8a5af9aada88702d08f855e6bc8eb313424`
and
`6d1d42c46ff6ff145dd72890c90abf6047d11910545599186e5f285028a21fc4`.
The capability profile now contains 30 reviewed feature tags and 308 audited
negative paths, with SHA-256
`3c5dee6fa18c428a45556488873ab216dd99e9f8859875ce2e4d1475d307aca6`.
The admission and differential locks add no production code; `runtime.rs`
remains 9,677 lines.

R1u installs the global eval intrinsic shell while keeping primitive String
source execution fail-closed. Pinned QuickJS differentials cover the callable
metadata and descriptors, lack of a prototype and constructor protocol,
no-argument behavior, exact non-String identity with no coercion, global
delete/replacement with a held alias, and cross-realm calls. The original
callable is retained as a realm-local root independently of the mutable global
property, matching the identity model that QuickJS's direct-eval opcode uses.
Primitive String input returns the uncatchable engine-level `Unsupported`
frontier rather than being run through the host Script evaluator.

Before R1u, a source-and-diagnostic inventory identified 1,085 eval-bearing
paths / 1,517 fail-runtime variants as the audit ceiling. Of those, 1,056 paths
/ 1,465 variants execute String source through direct, indirect, or mixed eval;
the remaining 29 / 52 only depend on the callable surface. The exact frozen
join moves 1,503 of those variants, while 14 remain unchanged fail-runtime
behind earlier or secondary independent failures. This is an architectural
work queue, not a predicted pass delta, because String execution will expose
further parser and runtime gaps. The independent `$262.evalScript` host hook
accounts for another 31 paths / 44 variants and is not global eval.

The complete positive focused gate contains 31 paths and 55 variants, and all
55 pass. Its manifest, TSV, and JSONL SHA-256 values are
`ae398ca6148d5babf468e7ba1cdcf956f454d35cdb6f612a3c4444d2b3c97cea`,
`9d364c24169423efa49ecfa384c86280f94011b430fa787f72a8214fe867a6f6`,
and
`63d5717d85f57c19705196aee0333c18cc270242b37e431622a035a8c34cf2fd`.
The R1u versions of the U+180E, RegExp-core, RegExp-match, and String-split
focused gates now classify their String eval consumers as
`unsupported-runtime`; the Date gate gains two linked passes. No other existing
focused manifest changes.

The exact R1t/R1u full join matches all 102,037 keys. It records 55
`fail-runtime -> pass`, 1,448 `fail-runtime -> unsupported-runtime`, and 41
`pass -> unsupported-runtime` transitions, with 1,544 outcome and complete-row
changes and no detail-only changes. The 41 former passes were all audited as
missing-eval false positives: 31 accepted the wrong outer `ReferenceError`,
while ten swallowed it and asserted state left untouched because the eval
source never ran. This correction makes the scoreboard more truthful even
though the net gain is only 14 passes. The full vector reaches 27,641 passes
and 34,849 admitted jobs. Full TSV/JSONL hashes are
`59736a4a4f63122a458a33374d2afd873a706aeb7ff271b52f9fa4aa2aa71fbe`
and
`c4849aecc54afcc7c73bb182cd240bc9cf35634bc74bc4d5558d6951898af2f2`.
The capability profile remains byte-identical at
`3c5dee6fa18c428a45556488873ab216dd99e9f8859875ce2e4d1475d307aca6`.

R1v adds the QuickJS-shaped direct-eval opcode path but intentionally changes
no Test262 classification. The compiler recognizes only a syntactic
IdentifierReference named `eval`, retaining the call-site scope in parser IR;
the VM then compares the resolved callee with the current realm's cached
original `%eval%`. Identity mismatch remains an ordinary call with an
undefined receiver and all evaluated arguments. Identity match bypasses the
native callable frame and forwards only the first argument (or `undefined`) to
the existing non-String/typed-Unsupported shell. This is the execution shape
required before String source can receive a linked caller environment.

The 31-path/55-variant focused report is byte-identical to R1u: 55 pass, zero
fail, unsupported, or skipped outcomes, with TSV/JSONL SHA-256
`9d364c24169423efa49ecfa384c86280f94011b430fa787f72a8214fe867a6f6`
and
`63d5717d85f57c19705196aee0333c18cc270242b37e431622a035a8c34cf2fd`.
The complete 102,037-key report is also byte-identical, with zero outcome,
complete-row, detail-only, missing, extra, or duplicate changes. It remains at
27,641 passes and 34,849 admitted jobs; full TSV/JSONL SHA-256 are
`59736a4a4f63122a458a33374d2afd873a706aeb7ff271b52f9fa4aa2aa71fbe`
and
`c4849aecc54afcc7c73bb182cd240bc9cf35634bc74bc4d5558d6951898af2f2`.
This zero movement is the acceptance result for R1v, not a claim that direct
String eval is complete. Spread arguments (`OP_apply_eval`), optional calls,
and the immutable eval-environment descriptor table remain later milestones.

R1w adds that immutable direct-eval caller-environment descriptor table without
opening String source execution. Descriptors walk the exact inner-to-outer
scope chain, divide it into current and ancestor function segments, retain
authoritative names on Local/Argument/Closure sources, force eval-visible
`arguments` and private function-name bindings, and reuse existing closure
slots. Publication checks the segment count against function-tree depth,
Body/Root topology, source partition, bounds, flags, parent-relay names, global
exclusion, and atom ownership. For primitive String input the VM validates the
complete descriptor and materializes live caller VarRefs before returning the
existing typed Unsupported error; non-String input still returns before scope
inspection or `this` normalization.

The R1w focused run remains 55/55 and is byte-identical to R1v, with TSV/JSONL
SHA-256
`9d364c24169423efa49ecfa384c86280f94011b430fa787f72a8214fe867a6f6`
and
`63d5717d85f57c19705196aee0333c18cc270242b37e431622a035a8c34cf2fd`.
The complete report also remains byte-identical: 27,641 passes among 102,037
variants, 34,849 runnable jobs, and TSV/JSONL SHA-256
`59736a4a4f63122a458a33374d2afd873a706aeb7ff271b52f9fa4aa2aa71fbe`
and
`c4849aecc54afcc7c73bb182cd240bc9cf35634bc74bc4d5558d6951898af2f2`.
That zero movement is the required result: the next compiler/runtime milestone
must add QuickJS-shaped eval bytecode publication and explicit defining-realm
ownership before any bounded String-execution slice can be enabled. Persistent
sloppy dynamic variables remain a separate declaration-environment milestone.

R1x opens that bounded primitive-String slice with a dedicated Eval root rather
than reusing the Script root. Direct eval imports the caller descriptor as an
ordered authenticated external closure prefix, while indirect eval has no
caller bindings and executes in the original `%eval%` callable's defining
realm. Eval-local lexical declarations, expression/statement completion,
strict inheritance, caller-cell writes, returned closures and catchable parser
errors now execute. Dynamic `var`, FunctionDeclaration instantiation, nested
syntactic direct eval, direct `new.target` and ill-formed UTF-16 source remain
typed frontiers rather than being approximated.

The focused eval gate grows from 31 paths / 55 variants to 74 paths / 138
variants, all passing. The 43 added paths account for 83 added passing
variants. Manifest, TSV and JSONL SHA-256 values are
`99aa8af497946369babf6f639f5ccfb4c8da5bffb7587f75825ead076556c314`,
`2b3f87db4ae4333cee6ff896c3d0ead2e061fd98000b0673a6fa32ff4acd7ad4`
and
`29e965a24abdd74d70ea0970a8c2afd6ce20f5b52153239f1b15bb7ec651b34e`.
The capability profile remains byte-identical; eight Test262 eval-lexical
paths are therefore covered by focused Rust/QuickJS tests but not added to the
gate because globally declaring the suite's `let`/`const` feature tags would
reclassify a much broader surface. One runtime-negative indirect parse case is
likewise left for a coordinated negative-provenance profile update.
Opening String eval also moves already-frozen collateral gates without changing
their manifests: RegExp core rises from 438 to 448 passes, RegExp match from
184 to 186, generic String split from 236 to 240, and U+180E from 40 to 50.

The exact R1w/R1x full join matches all 102,037 keys with no additions,
removals or previous-pass regressions. It records 575
`unsupported-runtime -> pass` and 13 `unsupported-runtime -> fail-runtime`
transitions. Ten exposed failures stop at existing arrow, async, generator or
non-simple-parameter parser frontiers. The remaining three are pinned QuickJS
behaviors already recorded as SpiderMonkey staging failures: the two
`try-completion.js` variants and `regress-602621.js`. Changing them here would
move away from the declared QuickJS target. The full vector reaches 28,216
passes while runnable remains 34,849; TSV/JSONL SHA-256 values are
`c62f104a2a3801c9b3eca38362fa5075f1fc21564395c58f45dfb23153ef1530`
and
`526c00942821ff5f153e08d3056627bbe35e7e12e4cde3702a55c220351bbd09`.

R1y opens QuickJS-shaped eval `var`, ordinary FunctionDeclaration, and Annex B
declaration environments without broadening the Test262 capability profile.
The new bytewise-sorted manifest freezes 497 paths: 54 core eval-declaration
paths and 443 Annex B consumers. They expand to 519 runnable variants, all of
which pass. Nested direct eval, `with`, generator/async declarations, and the
shared-profile lexical-feature surface remain outside this focused vector.
The manifest, TSV and JSONL SHA-256 values are
`ecc3cb3b50f8b59cae548fa9c1017dfd1d71878644bf204146d4002015c2bd70`,
`1b9cfacfe80671d5e2579865b7efb1478b5d7c1da70b240b71a1cccc3cf1c80a`
and
`0a0e7db1f1c80431302b14b66148f34efa998f38811e965f126c2d548ab6dd6d`.
The gate also pins a separate 15-path hash for collateral Test262 failures
which reproduce on QuickJS 2026-06-04, so target behavior is not mislabeled as
an Oxide regression.

The exact R1x/R1y join has the same 102,037 unique keys, with no missing,
extra, duplicate, or previous-pass rows. Outcome movement is:

- 752 `unsupported-runtime -> pass`;
- 16 `fail-runtime -> pass`;
- 16 `unsupported-runtime -> fail-runtime`.

Fifteen of the newly exposed failures are the pinned QuickJS collateral set;
the remaining test reaches the existing generator/async declaration frontier.
One additional row remains `unsupported-runtime` but now stops at the narrower
nested-direct-eval frontier after its preceding labelled FunctionDeclarations
execute. Net growth is 768 passes. The complete report reaches 28,984 passes,
keeps 34,849 runnable jobs, and contains no engine or runner fault. Full
TSV/JSONL SHA-256 values are
`cca9eadc35c3c5f9acdf24b00cb9d65b0a2ca20a65860e137185f4f7fa48c4e4`
and
`348e25af619fcf81ef534b82f57571889c1d2ab7f06cad3d5233e7d49fae240f`.

R1z removes the recursive direct-eval environment frontier without broadening
the capability profile or runnable set. Its bytewise-sorted manifest freezes
all 25 formerly blocked paths / 30 variants. Twenty-nine pass; the remaining
`staging/sm/global/eval-in-strict-eval-in-normal-function.js` sloppy variant
reaches the independent `with statements are not implemented yet` frontier.
The manifest, focused TSV and JSONL SHA-256 values are
`0b5e9ab5d51376e66a3b5b28614803fc32843649bbf6494747892de20c9032fc`,
`3a6dd32c7f3d0154b36946c6894f9cdba79a12d7086bf5602a210360b90f5248`
and
`23f4e2115b5a1ed322eac39faa51517912825562e71965a73261b3f4ad86a1fb`.

The exact R1y/R1z full join retains all 102,037 unique keys. It records 29
`unsupported-runtime -> pass` transitions and one detail-only refinement,
strictly inside the frozen manifest, with no missing, extra, duplicate, or
previous-pass row. Passes rise from 28,984 to 29,013; runnable remains 34,849
and `unsupported-runtime` falls from 135 to 106. Full TSV/JSONL SHA-256 values
are
`2ba53703827155be4ce36f11a52b48c3ac1bb4efc8f61da9cc31b6b1ca8e125a`
and
`c9369e14acb1469b20aea4caab2c0a880cb7f040a72718d629f38e1301582650`.

R2a fixes QuickJS-specific precedence between a named function expression's
private self binding and same-named direct/nested eval declarations. Authored
caller code keeps the private FunctionName binding, while eval's ordered
external scan still sees the nearest `<var>` property first. The accompanying
pinned-QuickJS differential also freezes the target's `add_eval_variables`
metadata-loss quirk, including entry-before-children ordering with source-keyed
first-flags/kind-wins closure slots, plain-leaf FunctionName restoration, and
the contrasting Eval-root relay behavior.

The pinned Test262 tree contains no exact instance of this declaration shape:
that declaration-shape cohort is 0 paths / 0 variants. R2a therefore adds no
empty manifest and records no Test262 progress increase. The full gate remains
byte-identical across all 102,037 keys: 29,013 pass, 34,849 are runnable, and
the TSV/JSONL hashes remain
`2ba53703827155be4ce36f11a52b48c3ac1bb4efc8f61da9cc31b6b1ca8e125a`
and
`c9369e14acb1469b20aea4caab2c0a880cb7f040a72718d629f38e1301582650`.

## R2b `with` statement gate

The dynamic-environment cohort remains reproducible independently of the full
vector. `tests/test262-with.txt` preserves every R2a path whose execution
stopped at the exact typed frontier
`with statements are not implemented yet`: 203 bytewise-sorted paths expand
to 205 positive, synchronous script variants. R2b removes that parser/runtime
frontier completely. The focused result is 198 passes, five parse failures and
two runtime failures. The five parse failures all reach the existing arrow
function grammar gap; one runtime failure reaches generator syntax through
String-source eval and the other reaches arrow syntax through direct eval.
They remain in the stable cohort so later adjacent milestones can turn them
into passes without rewriting this evidence boundary.

The manifest and `(path, variant)` key-set SHA-256 values are
`8f43b8f924d127814ea157637acebbb4e37fc89f97e6a76789e5e329d10250d6`
and
`1c04aebebd7c6e575113ca1466832c92096fef90af088aa1f3d317561aed0d4e`.
The R2b focused TSV/JSONL SHA-256 values are
`e22e130dfd23e5509aab68cf4ac244ecb6f5a827067c3622dc34014f9cf9d65d`
and
`cfc1aeeaf7fd6cc8ab1a3741cdbfe17db50b8a2817a054bf182838108cf22129`.
Those outcomes and report hashes preserve the historical R2b landing. The
same 203-path / 205-variant evidence boundary is intentionally stable:
subsequent synchronous-arrow, generator, ordinary-async, and R3ab async-arrow
work closes each adjacent frontier. The current focused report therefore
passes 205/205 with no non-pass outcome. Its unchanged manifest/key hashes are
`8f43b8f924d127814ea157637acebbb4e37fc89f97e6a76789e5e329d10250d6`
and
`1c04aebebd7c6e575113ca1466832c92096fef90af088aa1f3d317561aed0d4e`;
the refreshed TSV/JSONL hashes are
`f2f211cb3cc6619fda2c051d890f5994633d8962f1e98c58d2e9829e6289ee21`
and
`c3868df36a65922cac3f961ae82840fc90151f9f9312bc592e661d7c07ffca75`.
Reproduce and validate the complete vector with
`scripts/run-test262-with.sh`; the entry point derives the repository root and
pinned suite location at runtime and does not encode a workstation path.

This is a focused progress gate, not a full-parity claim. The implementation
uses the hidden with-object scope binding and its closure/eval relay,
`ToObject`, prototype-aware `HasProperty`, `Symbol.unscopables`, and the
get/put/delete/make-reference/get-reference paths which preserve the implicit
receiver of a call. The relevant upstream anchors are
`quickjs.c::resolve_scope_var`, `var_object_test`, the `TOK_WITH` statement
case, `JS_GetGlobalVarRef`, and `OP_with_*`. The eval-variable object remains a
distinct environment source with different ownership and Reference timing.

The exact R2a/R2b full join retains all 102,037 unique keys and changes only
the 205 frozen rows. It records 173 `unsupported-parser -> pass`, five
`unsupported-parser -> fail-parse`, one `unsupported-parser -> fail-runtime`,
25 `unsupported-runtime -> pass`, and one `unsupported-runtime ->
fail-runtime` transition. There are no missing, extra, duplicate,
detail-only, outside-manifest, or previous-pass changes. The complete vector
therefore rises by 198 passes to 29,211 while runnable remains 34,849. Full
TSV/JSONL SHA-256 values are
`8eba52564839d3a11a92ac28c883494cfc51d1f49785b07e7d3ac62ec867965c`
and
`54122f8b86f8cdbea6f3de6aa9532f770b72df1f6bf28bdc7cd62ec665b32ca1`.

## R2c synchronous ArrowFunction gate

R2c implements the synchronous, simple-parameter ArrowFunction slice on the
QuickJS compiler path. The frozen differential covers 34 cases: identifier and
parenthesized heads, line-terminator lookahead, reserved-word errors,
expression and block bodies, strictness, `name`/`length`/source metadata,
lexical `this`/`arguments`/`new.target`, nested closures, `with`, direct and
nested eval, `typeof this`, missing `prototype`, and non-constructability.
At the R2c landing, async Arrow, default/rest/destructuring parameters,
class/`super`, and method/accessor adjacency remained typed independent
frontiers.

The focused manifest fixes 40 paths / 66 positive synchronous variants. All 66
are admitted and pass. Its manifest and key-set SHA-256 values are
`75c1e7e8c12a493eb1b2f38b662ca51c2a20bbe68434900b2a890573ad8d4360`
and
`52684eee5c0df05893b6f6d00376669f2b845ec35a7f01ac0c4bea96cc324384`;
focused TSV/JSONL SHA-256 values are
`fd5b76fb8cb81bcebe786abc6c7992e318b0b7bf8ce9e5b7b58c2a75111b5108`
and
`d363b03a69f71bf760d8366e4b565b743d85a7f3127ea401e45aeb51b0aa50e4`.
Reproduce it with `scripts/run-test262-arrow.sh`.

Declaring `arrow-function` changes the shared capability-profile SHA-256 to
`5c3c11f7c7c81fd54b706d6d50b5f28f6dddbd915c7b3543af9e5e6b5fb08aae`
and admits 575 more full-suite jobs. Of those, 534 pass, two fail to parse, 28
reach runtime failures, and 11 stop at typed parser frontiers. The explicit
feature-tag cohort contains 1,800 variants in total: another 522 remain gated
by other feature tags, 496 are excluded by the pinned QuickJS configuration,
195 are async, and 12 require detach-array-buffer host support. The profile
declaration therefore exposes the broad queue; it does not claim the remaining
Arrow-adjacent grammar or dependencies are implemented.

Arrow syntax also appears in untagged tests. The exact R2b/R2c join retains all
102,037 keys and every previous pass while adding 1,043 passes:

- 474 `fail-parse -> pass`;
- 5 `fail-runtime -> pass`;
- 30 `harness-error -> pass`;
- 534 `unsupported-feature -> pass`.

The first full run caught one old-pass regression in strict direct eval:
`typeof this` had promoted the authenticated pseudo read to
`GetOrUndefined`, which the new resolver initially rejected as a non-read.
The dedicated QuickJS differential and the original Test262 path now pin that
case. The final join has zero previous-pass regressions, 30,254 passes and
35,424 runnable variants. Full TSV/JSONL SHA-256 values are
`c28acb10ae63e46e8aad1372f679c3be3b283322c2f690e0296bf0a77e243345`
and
`e82fbff1bdd49b300ea561d7ad21b9c3d62ed4d640f7080c3375bc9044bf32f9`.

## R2e capability-profile truth-up

R2e first audits already-implemented surface area before adding another grammar
slice. Direct single-variant runs in quickjs-oxide and the pinned QuickJS
2026-06-04 oracle prove 22 feature cohorts that the conservative profile had
continued to reject:

- `Array.prototype.at`, `Array.prototype.includes`, `array-find-from-last`;
- `Object.fromEntries`, `Object.hasOwn`;
- `String.fromCodePoint`, `String.prototype.includes`,
  `String.prototype.isWellFormed`, `String.prototype.toWellFormed`,
  `String.prototype.trimEnd`, `String.prototype.trimStart`, `string-trimming`;
- `__getter__`, `__setter__`;
- `coalesce-expression`, `logical-assignment-operators`, `new.target`,
  `numeric-separator-literal`, `object-spread`;
- `const`, `let`, `optional-catch-binding`.

The same audit admits 95 exact negative-test paths only after both engines
produce the expected phase and error type. The runner also re-reads those paths
from the pinned suite and rejects the profile if any no longer carries negative
metadata. Together these additions move the profile from 31/308 to 53 reviewed
feature tags and 403 audited negative paths, with SHA-256
`e2043efeaa2d8b4420d0c82550f7ba42d53588897ec14ac87f6f03c4358a8218`.
No engine semantic code changes in this step.

All 28 focused, non-full Test262 gates preserve their existing key sets,
runnable counts, pass counts and outcome summaries. The 26 metadata baselines
and four direct TSV/JSONL baselines are regenerated only because the canonical
report header embeds the capability-profile hash.

The complete 102,037-key join admits 1,342 more variants and reaches 31,459
passes: 1,205 rows move from `unsupported-feature` to pass and 137 move to an
existing typed parser frontier. Another 507 rows retain their outcome while
their unsupported-feature detail loses one or more newly reviewed tags. The
join has no missing, extra, duplicate, or previous-pass rows, and all 1,849
complete-row changes carry at least one of the 22 new tags. Runnable jobs rise
from 35,424 to 36,766. Full TSV/JSONL SHA-256 values are
`7e05dd58a0387d8639d09b3896917ad38fd8fd8fdecef85a3f0bcd26f730a22a`
and
`c9faabfd53bd125b3f7e4f3f6cbce884e0ce3172de320a1056398de60aa73ab6`.

## R2f synchronous ObjectLiteral concise-method gate

R2f implements synchronous, simple-parameter ObjectLiteral concise methods on
the QuickJS-shaped compiler/VM path. Fixed identifier/keyword/String/numeric
keys and computed String/numeric/Symbol keys, contextual `get`/`set`/`async`
identifiers before `(`, inferred names, source/name/length metadata, C/W/E
property descriptors, dynamic `this`,
owned `arguments`/`new.target`/direct-eval environments, strictness inheritance,
trailing commas, duplicate-parameter early errors, non-constructability,
missing `prototype`, and ordinary `__proto__()` data-property behavior are
pinned against QuickJS 2026-06-04. Accessors, async/generator methods,
non-simple parameters, and home-object/`super` semantics remain typed
independent frontiers.

The focused manifest freezes 74 paths and 144 sloppy/strict variants. All 144
are admitted and pass. Its manifest and key-set SHA-256 values are
`e9f877f938d52a5f5ccbe13af35822b0cb94a9486bb0857156f254a4b532ae75`
and
`ebba13cb8173521639bc12b78f2d5acb498893984f8e42e744a57f6c82f08b9a`;
R2f-landing TSV/JSONL SHA-256 values are
`41a1812b56f74b21967c155f33f93261c767aed6338562535faaded4227e7c4c`
and
`5dbf57993c5c4c1dd47f31769e20bbde16c31bc41d486edd8f1999c19d91e16b`.
`scripts/run-test262-object-methods.sh` reproduces the same 144-pass manifest
against the current profile; its regenerated report hashes are pinned in the
checked-in baseline.

Ten exact parse-negative paths are admitted only after quickjs-oxide and the
pinned oracle both produce the expected phase and error type. The capability
profile therefore keeps 53 reviewed feature tags, moves from 403 to 413 audited
negative paths, and has SHA-256
`1a5258a57285ff43149d8377692b5f1a3939ed19c790cbee81abab6912d21e51`.

The same grammar slice advances existing frozen focused gates without widening
their manifests: Date reaches 1,478 passes (+62), String split 248 (+6), RegExp
match 192 (+2), compile 58 (+2), replacement 326 (+18), matchAll 108 (+26),
named groups 172 (+4), and match indices 48 (+4). Reflect keeps 365 passes while
four parser frontiers advance to runtime assertions; dotAll keeps 26 passes.
These focused manifests overlap, so their movements are not a full-suite pass
delta. The checked-in focused baselines pin each resulting outcome vector
independently.

The exact R2e/R2f full-vector join matches all 102,037 unique keys with no
missing, extra, or duplicate keys and no previous-pass regression. It adds 492
passes: 472
`unsupported-parser -> pass` transitions plus 20
`unsupported-negative-provenance -> pass` transitions from the ten newly
audited negative paths. The remaining exposed parser consumers split into 38
`unsupported-parser -> fail-parse`, 89 `unsupported-parser -> fail-runtime`,
and six `unsupported-parser -> unsupported-runtime` transitions; every other
outcome is unchanged. The join records 625 outcome changes and 631 detail-only
changes. Runnable jobs rise from 36,766 to 36,786 and the complete vector reaches
31,951 passes. Full TSV/JSONL SHA-256 values are
`b63cd00601ea67854cd837a023d1ee14d0b7bdcd02b5e337c0f3eb14f4aa9a67`
and
`4196b714970aae9710d76d07e169c1f96ce80afe65cf37d4677ec2da20e3fe2d`.
The conditional observed rate falls from 91.79% to 91.57% because the new
grammar honestly exposes 127 ordinary parse/runtime failures that were
previously typed parser frontiers; no formerly passing variant regresses.

## R2g synchronous ObjectLiteral accessor gate

R2g ports synchronous ObjectLiteral getters and simple-parameter setters on
the same QuickJS-shaped define-method path. Fixed and computed String, numeric,
keyword, and Symbol keys; one-time `ToPropertyKey`; getter/setter half merging
and replacement; data/accessor conversion; inferred names; descriptors;
dynamic `this`, `arguments`, `new.target`, and direct eval; strictness;
non-constructability; source spans; and ordinary accessor-named `__proto__`
properties are pinned against QuickJS 2026-06-04. QuickJS error priority is
also preserved for accessor arity and strict reserved-word diagnostics.
Non-simple setter parameters, HomeObject/`super`, and async/generator methods
remain independent typed frontiers.

The focused manifest freezes 70 paths and 128 sloppy/strict variants. All 128
are admitted and pass. Its manifest and key-set SHA-256 values are
`02e2810fd012d7f2191cfd2a14d0ae54425c82717c9b8aacd5460e65f9d72175`
and
`2b70d0e1d0054705fe4da193374a67ad664c5f5027d17fb21e1873bb3f8fc1e3`;
the empty non-pass projection has SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL SHA-256 values are
`fec46a88e750f33f59085a09386a0f05bd563a5c11ed1310bbd19f8de18cb70a`
and
`51f232d679e7045da9634cc0d417cf74815d0f9a1af6064eb1385e6aafa260bd`.
Reproduce it with `scripts/run-test262-object-accessors.sh`.

Nine independently audited parse-negative paths move the capability profile
to 53 reviewed feature tags and 422 exact negative paths, with SHA-256
`73da0ef92820d81935e2f784a2f0e9ce565ccd10c302d8905c4bd4353c3a81ef`.
All 23 existing script-focused gates remain green after regeneration. Nine of
them gain 76 overlapping passes: dotAll +4, compile +2, match indices +4,
RegExp split +4, replacement +24, match +14, matchAll +8, named groups +4,
and search +12. The separately frozen Reflect and Date vectors add four and
eight passes respectively; the latter also exposes two existing missing-JSON
runtime failures. String split and the RegExp-core vector retain their outcome
summaries and change only because the report header embeds the profile hash.

The exact R2f/R2g full-vector join matches all 102,037 unique keys with no
missing, extra, or duplicate keys and no previous-pass regressions. It adds
447 passes across
267 paths: 436 accessor consumers, two strict reserved-word consumers, and
nine newly audited negative variants. The outcome transitions are two
`fail-runtime -> pass`, nine
`unsupported-negative-provenance -> pass`, 414
`unsupported-parser -> pass`, and 22 `unsupported-runtime -> pass`.
Ten former parser frontiers now report ordinary parse failures and 42 reach
ordinary runtime failures at downstream Proxy, JSON, TypedArray, and other
unimplemented surfaces. There are 499 outcome changes and 42 detail-only
changes. Runnable jobs rise from 36,786 to 36,795 and the complete vector
reaches 32,398 passes. Full TSV/JSONL SHA-256 values are
`8510e4117dd3854cd3c428548e36e0bba13a31abd66a875decf5f774850302d3`
and
`71cba68a097d685638b4f77f5e77676ea161e4212410724937ab9804d3c43cb8`.

## R2h direct ObjectLiteral `super` gate

R2h adds QuickJS-shaped HomeObject state and direct SuperProperty Reference
semantics to synchronous ObjectLiteral methods, getters, and setters. The
HomeObject is installed after inferred naming and before property definition;
the super base is the HomeObject's current prototype, while ordinary reads and
writes use the current method receiver. When `super.x()` resolves through an
accessor, pinned QuickJS invokes the getter with the frozen super base and then
calls the returned function with the current method receiver. Fixed and
computed reads,
calls, simple/compound/logical assignments, prefix/postfix updates,
`for-in`/`for-of` assignment targets, strict-versus-sloppy rejected writes,
key-coercion ordering, null-base diagnostics, and `delete super.x` are pinned to
QuickJS 2026-06-04. `super()` remains an early error in ObjectLiteral methods.

The focused manifest freezes 26 paths and 48 sloppy/strict variants. All 48 are
admitted and pass. Its manifest/key-set SHA-256 values are
`75a8d27edff0f6add47f2538a1d44b07509353c1352e759427d4ef93dffd0210`
and
`e25ea45b40345ed6e368d2010f3a48b46364f822845094546a658526b530d41a`;
the non-pass projection is empty. Focused TSV/JSONL SHA-256 values are
`f9d39c6ecbbd768899ad6d9a0962a87271c35a3af8fef16f7a375d82139bb28d`
and
`501107f4cb1dd6f8db6a5e7a43b127a244abce810626fde34c2342e89fe1309e`.
Reproduce it with `scripts/run-test262-object-super.sh`.

Declaring the reviewed `super` feature and one independently audited negative
path moves the capability profile to 54 feature tags and 423 exact negative
paths, with SHA-256
`85cec5c2713df52c631ed38b96621e253baf9e1fafc06eceeea19e9eba64c6f9`.
All existing focused gates remain green after regeneration. The smoke vector
also advances two early-error variants, from 189 to 191 passes, because a
top-level function-body `super` now produces the intended `SyntaxError` rather
than a typed parser frontier.

The exact R2g/R2h full-vector join matches all 102,037 unique keys with no
missing, extra, duplicate, or previous-pass rows. It adds 82 passes: 52
`unsupported-parser -> pass`, 24 `unsupported-feature -> pass`, four
`unsupported-runtime -> pass`, and two
`unsupported-negative-provenance -> pass`. Eighteen other rows expose honest
downstream frontiers or failures, and nine retain their outcome with a more
specific detail, for 100 outcome changes and nine detail-only changes. Runnable
jobs rise from 36,795 to 36,825. The complete vector reaches 32,480 passes;
full TSV/JSONL SHA-256 values are
`44f6f555cc8f72a6d0ff5ed392468a315b44d8c2cd289f7b72a65adde8c58a78`
and
`4d220f27199ee71757e368eb863a535264cc9914a85efaa90d69d54813dd575c`.

R2i below resolves ArrowFunction inheritance and R2j resolves direct-eval
inheritance. Parameter initializers, classes and derived constructors, and
async/generator methods remain explicit follow-up slices rather than being
inferred from the direct ObjectLiteral result.

## R2i ObjectLiteral arrow `super` gate

R2i extends SuperProperty Reference semantics through synchronous arrows nested
in ObjectLiteral methods and accessors. Arrows capture neither a fresh `this`
nor a fresh HomeObject: the enclosing method lazily owns both authenticated
pseudo locals and nested or escaped arrows relay those cells through ordinary
closure slots. An 11-case pinned QuickJS differential covers live HomeObject
prototype changes, lexical receivers, accessor and nested-arrow inheritance,
computed writes, updates, strictness, getter-call receiver behavior, deletion,
and grammar boundaries.

The focused manifest freezes four paths and eight sloppy/strict variants. All
eight are runnable and pass. Its manifest/key-set SHA-256 values are
`d29f77c5920b21a92f61b0022eb186b5ba24e100f6ffa52b4d952347c9aaad90`
and
`4ac13c25ee6b84ee9019b53f5119fb2d7dc3154eb9785eda8800f725bbf32eba`;
the empty non-pass projection has SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL SHA-256 values are
`afa0f32205ef75af6aae165a3b2e74023d4408cef423333cad63454f9c402872`
and
`0c35ca795fc6b8329bcc6a3af0bbe7878d9819e22bf8b590f2634c79fbba4cbc`.
Reproduce it with `scripts/run-test262-object-super-arrow.sh`. The capability
profile remains unchanged at 54 feature tags and 423 audited negative paths.

The exact R2h/R2i full-vector join matches all 102,037 unique keys with no
missing, extra, duplicate, or detail-only rows and no previous-pass
regressions. Exactly four `unsupported-parser -> pass` transitions occur: the
sloppy/strict variants of
`prop-dot-obj-val-from-arrow.js` and `prop-expr-obj-val-from-arrow.js`.
Runnable jobs remain 36,825 and the complete vector reaches 32,484 passes.
Full TSV/JSONL SHA-256 values are
`dcc079d5c819b066703046136bfe2bdb17a6f02723796c6a8020680db0bb3acb`
and
`c82f264111cd4d0526f2f607ead97aab0e2776b49410b58d25425b8491df2664`.

R2j below resolves direct-eval inheritance of HomeObject. Parameter
initializers, classes and derived constructors, and async/generator methods
remain explicit follow-up slices.

## R2j ObjectLiteral direct-eval `super` gate

R2j extends ObjectLiteral SuperProperty Reference semantics through syntactic
Direct Eval inside synchronous methods, getters, setters, and their synchronous
arrows. Following QuickJS 2026-06-04, the bytecode and eval descriptors persist
independent `super_call_allowed` and `super_allowed` capabilities. ObjectLiteral
methods, getters, and setters carry `false/true`; ordinary functions, scripts,
and indirect eval carry `false/false`; arrows inherit both flags exactly; and
Direct Eval inherits the exact authenticated caller descriptor. HomeObject
pseudo locals and closure cells provide storage, not authority, so merely
finding a captured HomeObject cannot enable `super` across an ordinary-function
boundary.

A 16-case pinned QuickJS differential covers live HomeObject prototype changes,
method/getter/setter receivers, reads, calls, writes, updates, deletion,
strictness, authored and eval-created arrows, nested eval, ordinary/global/
indirect cutoffs, and `super()` argument-order boundaries. An unconditional
Rust expectation test runs the same vector without `QJS_ORACLE`; oracle-enabled
runs independently verify both the pinned expected vector and the Rust/QuickJS
differential. ObjectLiteral descriptors keep `super_call_allowed=false`, so
their `super()` forms remain early errors before argument evaluation. Execution
with an authenticated call capability remains a typed Unsupported boundary for
the future derived-constructor slice.

The focused manifest freezes 12 paths and 24 sloppy/strict variants. All 24 are
runnable and pass. Its manifest and key-set SHA-256 values are
`8643870c3932da98f7ba60cb4e7d4499b02783853f4154f096122796bd998b0f`
and
`6f193e1ebf25a09717fe1c9bbd032d3f1b9cc38eb602870e551f50d5e82277fa`;
the empty non-pass projection has SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL SHA-256 values are
`5fa67acef400c5525df9eace328219a30539a1661776ebc964e9ac6c4d38a470`
and
`5274231bdedc8c3d99f159626cdeef92fe4cf1fe6a9427d70b6f81f9928fbf0a`.
Reproduce it with `scripts/run-test262-object-super-eval.sh`. The capability
profile remains unchanged at 54 feature tags and 423 audited negative paths.

The exact R2i/R2j full-vector join matches all 102,037 unique keys with no
missing, extra, or duplicate keys, no metadata drift or detail-only rows, and
no previous-pass regressions.
Its only outcome changes are six `fail-runtime -> pass` transitions: the
sloppy/strict variants of `super-prop-method.js`,
`prop-dot-obj-val-from-eval.js`, and `prop-expr-obj-val-from-eval.js`. Runnable
jobs remain 36,825, runtime failures fall from 2,431 to 2,425, and the complete
vector reaches 32,490 passes. Full TSV/JSONL SHA-256 values are
`8a1633a0d527bc77926124f3a6e1fa5ef340e6e79626a22ed171f37dafb8c6e0`
and
`b904278dd9c8cc5d3cf54babd037723ec7e52d015a636fe0d19ef5a4b0f36cfb`.

## R2k tagged-template gate

R2k ports tagged-template parsing and runtime publication from QuickJS
2026-06-04. Each bytecode site owns one frozen cooked Array and one frozen
`raw` Array in its compilation realm; the cooked constant retains that identity
through repeated closure calls and GC. Invalid escapes become `undefined` only
in the cooked Array, raw UTF-16 text is preserved, substitutions remain full
comma expressions, and dot/computed/`with`/`super` tags keep the same Reference
receiver as ordinary calls. Tagged `eval` is deliberately an ordinary call.
Constructor precedence, chained tags, direct-eval HomeObject relay, dynamic
eval/Function site separation, newline continuation, descriptors, abrupt order,
and receiver behavior are pinned by 16 QuickJS differential vectors. A
separate Rust lifecycle test locks site identity across StripDebug publication
and cycle collection.

The focused manifest freezes 48 paths and 89 variants. All 85 executed
variants pass; the later private-name work closed its two original staging
frontiers. Two `create-realm` variants remain host-unsupported and two TCO
variants remain excluded by the pinned QuickJS configuration. Manifest,
key-set, and non-pass SHA-256 values are
`d3a7e597a049e9a78830ee089a90db27c6b6b0b8b2d049cd76b30f5515e6d23a`,
`91852cd5c970debac2ef05af2715198736757b1276a34e6a73722df86bd80356`,
and
`cebe904ead643233ee754510a90cf53967525c4db1163281188b47aa56c80b50`.
Focused TSV/JSONL SHA-256 values are
`a132ee39e73f44d77348b544427045069bb112ece353009ac7d5b2651fe51089`
and
`c32ef91f30cb4646228aee7cb2cd8a2445f4d6afa04c0173e4673f68acbb36b0`.
Reproduce it with `scripts/test-test262-tagged-template.sh`.

Declaring `template` moves the capability profile to 55 feature tags and 423
audited negative paths, with SHA-256
`d146a337c9bab8b171aaddfe31d404073a9d3cbb65fd7ac7d6ab46fdefe69ef7`.
The exact R2j/R2k full join retains all 102,037 unique keys with no missing,
extra, duplicate, or detail-only rows and no previous-pass regressions. It
records 79 `unsupported-parser -> pass`, two `unsupported-runtime -> pass`,
and two `unsupported-feature -> pass` transitions. Two PrivateName staging variants
advance from the parser frontier to the existing typed runtime frontier. The
complete vector reaches 32,573 passes and 36,827 runnable variants. Full
TSV/JSONL SHA-256 values are
`96dfb48f8887e525ff2813e4f8ac9ab7cf191f9e0fedd0d8724ee52943ce60e9`
and
`799be95a11b86d2b1efdfa694cd88971a600c64992fd07b03d61d913377f2e23`.

## R2l strict JSON parse and reviver gate

R2l ports the pinned QuickJS JSON grammar and post-order reviver walk instead
of reusing the JavaScript lexer or an external serializer. Parsing preserves
arbitrary UTF-16 code units, allocates Arrays and ordinary objects in the
method's defining realm, defines `__proto__` as data, and retains exact
primitive source spans only when a callable reviver needs them. The walk
snapshots own keys, keeps QuickJS's duplicate-key parse-record behavior, and
observes mutations through ordinary property operations. It passes the third
reviver context argument with `source` only when the parsed primitive is still
unchanged.

The focused manifest freezes 84 paths and 168 variants. All 168 run: 166 pass,
and the sloppy/strict forms of the 2,097,153-element dense-array stress case
time out at the existing object-model performance frontier. Nothing is skipped
or reported unsupported. Manifest, key-set, and non-pass SHA-256 values are
`16b919d34d9eebcc60a92e038e0a6fd565e9306c1ba17cffc6f62ce0f05f23c4`,
`36e19d071bb8ad9e4982ae85a5f32a3205925b6bf68fe335cfd1cbdfb429cff9`,
and
`2436785b58ef14db6e47d65537af5a9edf58e33bec81837eaf2f3b36f1eee4d0`.
Landing TSV/JSONL hashes under the R2k profile were
`31d01dbc119767d5eb9e2be69c9054f97ca78a3b4ca5e5ae60faf9ed1f29b8e9`
and
`7ed6c23a8b94dfb2854f9be793c4aba388d64a432e0a931d6d8d81dbb7c38dbf`.
Under R2m's profile metadata, the gate hashes were
`22377dfabe093c798ec712be77ab06ca600e11725666945e523b68410d6927cb`
and
`2fa563ffd36405eee7433e0aada0abe1a1474e64b31228949f5a0dc04af2da04`.
Reproduce the current outcome vector and its checked-in
`tests/test262-json-parse-baseline.txt` byte hashes with
`scripts/test-test262-json-parse.sh`.

## R2m JSON stringify and Raw JSON gate

R2m completes the pinned JSON intrinsic family. `JSON.stringify` normalizes
the replacer before `space`, creates the root holder afterward in the defining
realm, invokes `toJSON` then the replacer, unwraps supported primitive wrappers,
snapshots object keys and Array length at the corresponding QuickJS points,
uses a path-only ancestor stack for cycle detection, quotes exact UTF-16
including lone surrogates, rejects unspecialized BigInts, and preserves
QuickJS's indentation and omission/null substitution rules. An explicit task
stack preserves the observable recursive traversal order without imposing the
old 256-level Rust cutoff; differential cases lock 257 and 4,096 nested Arrays.

Its focused manifest deliberately selects the direct stringify semantic
surface, excluding cases whose formatter usage is incidental. All 160 variants
across 80 paths pass. Manifest, key-set, and empty non-pass SHA-256 values are
`001d8337407a2689dc181120160bc6d45d6b03765ec5ca0c2c7f3421f9705f11`,
`ab8b0bdfa3895693115c79579f936d2559806dbc95f2588537267a73d6039892`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
R2m-landing focused TSV/JSONL hashes are
`38ebfa11ff63d080072eb93845711ff4f90bd6753a70fa793edc0c128f89bd82`
and
`1ff4e957792cf2f1702f21df30bd7656d5448a71f5cf9fcc6f37c9cd48fa445b`.
Reproduce the current outcome vector and its checked-in
`tests/test262-json-stringify-baseline.txt` byte hashes with
`scripts/test-test262-json-stringify.sh`.

`JSON.rawJSON` first converts and validates the exact source text through the
same strict parser, then creates a null-prototype, non-extensible object with a
runtime-wide unforgeable heap brand and one frozen enumerable `rawJSON` data
property. `JSON.isRawJSON` tests that brand directly without traps or coercion;
stringify recognizes it after `toJSON`/replacer processing and splices its
validated lexeme before cycle handling. The raw manifest freezes 22 paths and
44 variants. At the R2m landing, 42 were runnable: 36 passed, four parse
failures required unrelated rest/spread syntax, and two typed parser frontiers
required unrelated arrow destructuring. Refreshed through R3d, the current gate
passes all 42 runnable variants. The pinned staging path remains config-excluded
in both modes. R2m-landing manifest, key-set, and non-pass hashes were
`8e4d1fa6f59eae77cf1a35668ea02002de4d4f4cae146bb9ea6bde1c849b1df4`,
`c5be0b3a9dd6c106d9e1c19cd15726b7a6756ac5ee464d4279fd835d520ddee7`,
and
`2c8fb7640ded74e86d6e5b8990dcaf8650ec0eccbc855cb2dcbef808e8caae8a`.
R2m-landing focused TSV/JSONL hashes are
`bb3792c4b565855a533a56db306f9fb465b6f899ca739db3a0ceb92979a0cf34`
and
`4d76fd54f0d4878a816f452170f1b7436fec0c86a0c601d925f86aca1ae16264`.
Reproduce the current outcome vector and its checked-in
`tests/test262-json-raw-baseline.txt` byte hashes with
`scripts/test-test262-json-raw.sh`.

Declaring `json-parse-with-source` and `well-formed-json-stringify` moves the
capability profile to 57 feature tags and 423 audited negative paths, with
SHA-256
`0c6b9ef80d683bd69a97f87bbee10e7029432deb25d23695a96c251e9dfc9f66`.
Because every profile-aware report pins that hash in its header, R2m-era
baselines for older focused gates were re-emitted with metadata-only byte/hash
changes; their outcomes and key sets remained unchanged, while the historical
sections retained each gate's landing hashes.
The exact R2k/R2m full join keeps all 102,037 unique keys with no missing,
extra, duplicate, or previous-pass-regression rows. Of 518 outcome changes,
472 are `fail-runtime -> pass`, 38 are `unsupported-feature -> pass`, two are
`unsupported-feature -> unsupported-parser`, four are
`unsupported-feature -> fail-parse`, and the dense-array pair is
`fail-runtime -> timeout`; nine more rows change detail only. Runnable variants
reach 36,871 and passes reach 33,083, a net gain of 510. Full TSV/JSONL hashes
are
`63d5a44dd8d057e220882d02abebb1b221fdb1a419ce1fc691e1ed084d2b0a3e`
and
`0b8eedcae7d427a6bf7fbbcefb412d9f2691c0bdf00c4bc2229bbfd1a8212fb2`.

## R2n strong Map gate

R2n ports the pinned strong `Map` family through realm-local constructor,
prototype, and iterator graphs. Ordered records use `SameValueZero`, normalize
negative zero, and preserve live mutation behavior for iterators and
`forEach`. Construction follows QuickJS's cached-adder and `IteratorClose`
ordering; the implemented surface includes `set`, `get`, `has`, `delete`,
`clear`, `size`, `forEach`, `keys`, `values`, `entries`, `getOrInsert`,
`getOrInsertComputed`, species, tags, and `Map.groupBy`.

The dependency-audited focused gate freezes 186 paths and 370 variants; all
370 pass. `Symbol.iterator` and `upsert` are admitted only by its runner-bound
scoped profile, whose SHA-256 is
`16ab6bfe18540aae398c847905f492491e81500045b45a6bfb21f447fd537ea2`;
this is not a global feature claim for Set, WeakMap, or other consumers.
Manifest, key-set, and empty non-pass SHA-256 values are
`50387c488c3ade2aafbbe2cd4cecc387bc0c97a76808831d74b634407b990cd1`,
`2704f0c3407fa65dec9297df89f3643eba808f72347b530c71f091be15b14d81`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`10e2e4ca4f285eaaf345c1231b7707951e72882e1d603dc144cdde50eb8ed645`
and
`e8645afd72aec2e917fbc11ae4c9502bbb4473897414cc9882027d79082cda69`.
Reproduce the gate with `scripts/test-test262-map.sh`.

Declaring only `Map` and `array-grouping` globally moves the capability profile
to 59 feature tags and 423 audited negative paths, with SHA-256
`0f4617ff1678710c97620aa1257c4868b2a4daf0f4f917f9d7393566ee549c45`.
The exact R2m/R2n full join retains all 102,037 unique keys and records 234
`fail-runtime -> pass`, 80 `unsupported-feature -> pass`, eight
`unsupported-feature -> fail-runtime`, and four
`unsupported-feature -> unsupported-parser` transitions. The eight runtime
failures expose four WeakMap receiver-brand paths in both modes; the four
parser frontiers are two subclass-Map class paths in both modes. They are newly
admitted gaps, not regressions of previously runnable tests. Eighteen further
rows change detail only. There is no previous-pass regression or outcome drift
outside the reviewed admission set: the focused Map manifest plus rows gated
by the newly global `Map` or `array-grouping` tags. Runnable variants reach
36,963 and passes reach 33,397, a net gain of 314. Full TSV/JSONL hashes are
`5a0502380cb281bb089fe229cb1ec806228dd70e75987f852476984cb4d30271`
and
`2370d923625dc76d0a89c8314ed16875a402bccde665b6e45e30948e7526a2f8`.
All global-profile focused reports are re-emitted because the profile header
changed; their key sets remain stable. Older aggregate gates may also change
outcomes or details when the newly installed Map surface removes a downstream
blocker.

Parameter initializers, classes and derived constructors, and async/generator
methods remain explicit follow-up slices.

## R2o strong Set gate

R2o ports the pinned observable strong `Set` family through realm-local
constructor, prototype, and independent Set-iterator graphs. Ordered records
use `SameValueZero`, normalize negative zero, and preserve live mutation for
iterators and `forEach`. Construction follows QuickJS's cached-adder and
`IteratorClose` ordering. The implemented surface includes `add`, `has`,
`delete`, `clear`, `size`, `forEach`, the exact keys/values alias, `entries`,
species, tags, `Set.groupBy`, and `isDisjointFrom`, `isSubsetOf`,
`isSupersetOf`, `intersection`, `difference`, `symmetricDifference`, and
`union`. Set-producing methods follow the pinned set-like protocol and allocate
a base Set in their defining realm without consulting subclass species or an
overridden `add`.

The dependency-audited focused gate freezes 322 paths and 642 variants; all
642 pass. The global profile already admits `Set` and `set-methods`; the
runner-bound scoped profile adds only the exact well-known-Symbol dependencies
required by the frozen manifest. Its SHA-256 is
`6869e9d28fff1d5bd4e5b698dcdf6ee677b9134a91781ad7abe226200d669455`.
Class, generator/object-generator, rest-parameter, lexical-destructuring,
WeakSet, and `$262.createRealm` dependencies remain separate frontiers.
Manifest, key-set, and empty non-pass SHA-256 values are
`44c6b6b599e7fe48324aaa693fa684649469c35209bc5c1edb34f0eebe2085b9`,
`5b4959128a9fb34b72b83950fd329f8a98bbbb2b08f256d5ff8bc3f7bc73a0ac`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`b45345b024a33560f2244b69bcdd181e2c5f07add1a04d9fe474169117cb222b`
and
`de7d718b67a1bae7d8031345ce55ba7f32aa8a5d6bcefd745ac2c4401ae65e3f`.
Reproduce the gate with `scripts/test-test262-set.sh`.

Declaring only `Set` and `set-methods` globally moves the capability profile to
61 feature tags and 423 audited negative paths, with SHA-256
`086b4964eebc8dd8960b33aaa333b0adaeefb1447cbf63f893042ab269a5a17b`.
The exact R2n/R2o full join retains all 102,037 unique keys and records 342
`fail-runtime -> pass`, 302 `unsupported-feature -> pass`, 82
`unsupported-feature -> unsupported-parser`, 50
`unsupported-feature -> fail-parse`, and 14
`unsupported-feature -> fail-runtime` transitions. Of the 644 new passes, 602
are inside the focused manifest and 42 are linked Map-brand, for-of, and
staging variants outside it. The focused gate's other 40 variants remain
fail-closed under the global profile because their well-known-Symbol tags are
deliberately scoped. The 14 newly exposed runtime failures are WeakMap/WeakSet
receiver-brand cases; the parser and parse failures expose existing class,
generator/object-method, and parameter-syntax frontiers. There is no
previous-pass regression or outcome drift outside the focused manifest and
rows selected by the newly global tags. Runnable variants reach 37,411 and
passes reach 34,041, a net gain of 644. Full TSV/JSONL SHA-256 values are
`14f8412069dc7ba2a648c2facead1cbcd79ccf2cc5116832602f50decd5f95ab`
and
`c29229ceeee55db836e701d8a2984ef0ba9eb9396d6deca8a5166026b58bb71b`.
All global-profile focused reports are re-emitted because the profile header
changed; their key sets remain stable.

The stable-vector storage shared with Map deliberately retains tombstones and
uses linear lookup. That preserves the tested observable semantics but does
not yet match QuickJS's hash lookup and reclaimable zombie records. WeakMap and
WeakSet additionally require genuine weak-reference/GC infrastructure rather
than another strong-record wrapper. Both remain explicit resource-parity or
feature frontiers rather than part of this milestone.

## R2p well-known Symbol protocol admission

R2p audits the already-implemented realm-local well-known Symbol graph and
admits its eight remaining protocol tags globally: `Symbol.asyncIterator`,
`Symbol.hasInstance`, `Symbol.iterator`, `Symbol.prototype.description`,
`Symbol.species`, `Symbol.toPrimitive`, `Symbol.toStringTag`, and
`Symbol.unscopables`. The focused QuickJS differential suite continues to pin
intrinsic identity, descriptors, descriptions, coercion, iteration, species,
instance checks, tags, and unscopables behavior; this milestone changes the
runner's audited capability boundary rather than production semantics.

The dependency-audited focused gate freezes 517 paths and 1,010 variants under
an exact 30-feature scoped profile. At the R2p landing, all 806 protocol-ready
variants passed. The remaining 204 outcomes were 60 parse failures, 98 runtime
failures, 18 harness failures, and 28 typed parser frontiers caused by
independent class, rest/spread, Promise, buffer/TypedArray, Proxy, and
weak-collection dependencies; the source/result audit found no Symbol protocol
mismatch. R3e brought the gate to 864 passes while refining the remaining
class diagnostics; R3f resolves all 28 derived-class parser frontiers, so the
current gate passes 892 of 1,010 variants. Its other 118 outcomes are the
independent two parse, 98 runtime, and 18 harness failures. The
scoped profile SHA-256 is
`ff674aafc4b1b61b0c40042f831b44c600b1f741e06b8c8c35863b876919aa7b`.
R2p-landing normalized-manifest, manifest-file, key-set, and non-pass SHA-256
values were
`eaf2a48408b6b1f5673389335cda73cb66bed062636a669c655460d9fef99a4b`,
`6147636f7950b899f7c0eea25078e2f4c9c4c7fda2977181dd7c9671aa0bcde2`,
`e87d58ad7a8be3e60b5545129a70a1abd70ee350654092a4aa066d17dc69e450`,
and
`4783b1a8bb909a6e4706138265c477cfa3979bb6821f09f590e4c8c66a0dd5d2`.
R2p-landing focused TSV/JSONL hashes were
`ed0363676e7efdfc6bb24ee396739cf67d49a4ce685c3bd37d98569a60a96267`
and
`75c40ff9adf28f0b9120c23af44268b4660189ff815e3f4c2ba0b74786ede048`.
The current non-pass/TSV/JSONL SHA-256 values are
`831fea4c50b0ffcf14e073a75fa75a4c6855bbadc5c7ed58fbc988c8b33cdf73`,
`310560aa182de2df22b3a261157e92e6f94810a51adda918bea6e9f45fba5209`,
and
`d2fc654e57792e6670d21383e2cbc2c71d7638684ede17db28813dc126e9a409`.
Reproduce the gate with `scripts/test-test262-symbol-protocols.sh`.

The global profile now contains 69 reviewed feature tags and 423 audited
negative paths, with SHA-256
`a1a347d2d74c946a50f1e26fca6c1756c0e9948f087de3aed2339b3a4c7d6677`.
The exact R2o/R2p full join retains all 102,037 unique keys. Its 1,010 outcome
changes exactly equal the focused key set: 806 move from
`unsupported-feature` to pass, 98 to runtime failure, 60 to parse failure, 28
to a typed parser frontier, and 18 to harness failure. Another 1,954 rows change
feature-detail only. Every changed row carries at least one newly admitted tag;
there are no missing/extra keys, previous-pass regressions, or unrelated
outcome changes. Runnable variants reach 38,421 and passes reach 34,847, an
exact net gain of 806. Full TSV/JSONL SHA-256 values are
`a56285e53591df1d2026da4d6334d42e374a107cbcc7744e87f1d8b4c49d865d`
and
`0f1b3899b73d990575b8ee1f4cb11e308847c5fd3fb728b13b3e3e583e08f15e`.

The next high-yield semantic line is binding/destructuring rather than weak
collections: it unlocks several thousand immediately classifiable variants,
while WeakMap and WeakSet first require genuine weak heap edges and collection
semantics.

## R2q flat array binding declarations

R2q implements flat ArrayBindingPattern declarations for `var`, `let`, and
`const` in Program code, ordinary-function bodies, nested blocks, shared
switch scopes, classic `for` heads, and synchronous `for-in`/`for-of` heads.
The shared lowering accepts identifier leaves, empty patterns, elisions,
trailing commas, undefined-only defaults with NamedEvaluation, and a terminal
rest binding. Direct declarations use QuickJS-shaped control-flow inversion:
the binding fragment is emitted first, execution jumps to the right-hand side,
and then returns to the iterator-driven assignment fragment. Iterator records,
abrupt unwind, and `IteratorClose` therefore stay on the same VM path as
synchronous `for-of`. For `var` under `with`, the destination Reference is now
prepared before `IteratorStep`, preserving the binding target even when
observable iterator side effects mutate the object environment.

The dependency-audited gate freezes the clean identifier-leaf projection
across direct declarations, classic `for`, and synchronous `for-of`: 90 paths
and 180 sloppy/strict variants, all runnable and all passing. Its runner-bound
profile admits only `destructuring-binding` and the already-implemented
`Symbol.iterator`; it is deliberately not a global claim for nested or object
patterns, destructuring assignment, parameters, catch bindings, or
async/generator contexts. Normalized-manifest, manifest-file, key-set, and
empty non-pass SHA-256 values are
`257af4e4f08f01ed33c0d88a7c64b44dd29adee6bbc64d87cb0213402e72c048`,
`db17670a1f7715a325a07087b766f6e64cf2bb24cec727278db05db3f79ee679`,
`fdceb7f320989a25165bd37ec41b2b3d2cdd616695979a1a0db92a5415537325`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
The scoped profile SHA-256 is
`8232e2c11e908f7cbf5a9e0f34fbd5223a9551b49ae64647f2a72b2314bcaf84`;
focused TSV/JSONL hashes are
`f0a66030c0a650874b003639775cb87149a4fcd221a1cfd80f603ab8d86f0dde`
and
`ca54eb7e1763501e130fff72dd67ec90469ab8fbc580e12809b6e6cda88e2f35`.
Reproduce the gate with `scripts/test-test262-array-binding-flat.sh`.

The broad `destructuring-binding` tag remains absent from the global profile,
but the full vector is not byte-identical: several Test262 and staging paths do
not carry that metadata tag, so the newly implemented syntax is reached
naturally. The exact R2p/R2q join retains all 102,037 keys and changes 37
outcomes: 23 `unsupported-parser -> pass`, eight `fail-parse -> pass`, two
`unsupported-parser -> fail-parse`, and four
`fail-parse -> unsupported-parser`. The two new parse failures are both modes
of one still-unsupported destructuring-assignment staging path now reaching its
generic syntax frontier; the four typed parser outcomes are nested patterns.
Two further rows keep `fail-parse` but move from the old declaration diagnostic
to that same assignment diagnostic, so 39 data rows change bytes in total.
There are zero previous-pass regressions. Passes rise by 31 to 34,878 while
runnable variants remain 38,421; the full summary now contains 552 parse
failures and 1,204 typed parser frontiers, with every other category unchanged.
Full TSV/JSONL hashes are
`bc9e6f71acbad459fabfcd2838c691cf318a781dea3dc2239161eced7c065c2f`
and
`b0b99d49bec652fa0b686a8d9af4296a5b156db6fec849c56168fb1dc41e6b7e`.
Wider declaration contexts and destructuring consumers must still land behind
their own audited projections before the global capability boundary can move.

## R2r recursive nested array binding declarations

R2r extends the shared declaration lowering from flat identifier leaves to
recursively nested ArrayBindingPatterns. The same path now handles direct
`var`/`let`/`const` declarations, classic `for` declarations, and synchronous
`for-in`/`for-of` declaration heads. Nested defaults, terminal rest patterns,
elisions, and abrupt completion use the existing iterator-region machinery, so
each active iterator receives QuickJS-compatible `IteratorClose` treatment.
The lowering also preserves dynamic `with` References, restores AllowIn for a
whole-pattern initializer in a classic-for NoIn head, and pins QuickJS error
priority for malformed nested and rest patterns.

The dependency-audited R2r gate freezes 72 paths / 144 sloppy/strict variants;
all 144 are runnable and pass. Its runner-bound profile admits only
`destructuring-binding`, so object patterns, destructuring assignment,
parameters, catch bindings, async/generator contexts, and modules remain
outside this claim. The scoped profile SHA-256 is
`c770387473b6ba2e273ab635182b5f07ae80ad902f48057ba5e2fb4f036c723e`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`84d3c39bb9dcc81f16d92e8b30045a7b5c5d8c2fa6b24151a849633ae087d269`,
`f7c7c181cdde65c84dfcb677cbe45f77884990666a774f952bc165df89f5e8a5`,
`a95c253cbdaf997e9b6d4ed38a48c63e4ffc7400204137c5f4fdd693a815ca7f`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`39abfe594755acdeb26375bce7c173544bc9404ad5e96b7c6c4b0dd3f48b1c89`
and
`d4f25a4495c080fd36c237077f323e9686a99b7b9dfdf192c93c18643467f187`.
Reproduce the gate with `scripts/test-test262-array-binding-nested.sh`.

The exact R2q/R2r full join retains all 102,037 unique keys and records only
the sloppy and strict variants of `staging/sm/regress/regress-469625-03.js`
moving from `unsupported-parser` to pass. There are no previous-pass
regressions or other outcome changes. Passes therefore rise by two to 34,880,
runnable variants remain 38,421, and typed parser frontiers fall from 1,204 to
1,202. Full TSV/JSONL SHA-256 values are
`10704652e6a0f24369203c0830bf8e70c7cf3ecd6e158823ee70dc5130d91214`
and
`53590c254bbb591279dc86b4bb8c668dd5f84098fb8eaa0410318e6f42e924d8`.

## R2s fixed/computed recursive object binding declarations

R2s extends the shared declaration lowering to fixed and computed recursive
ObjectBindingPatterns, following QuickJS 2026-06-04. Direct
`var`/`let`/`const`, classic `for`, and synchronous `for-in`/`for-of`
declaration heads accept identifier, String, numeric, keyword, computed String,
and computed Symbol property keys. Defaults use undefined-only selection and
NamedEvaluation, and object and array patterns recurse into each other.
Property-key conversion, sloppy `var` Reference preparation, getters,
initializers, and writes preserve QuickJS's observable `with` ordering.
Abrupt nested patterns retain inner-to-outer iterator unwind and the pending
original-error priority.

The dependency-audited R2s gate freezes 36 generated positive templates across
nine direct, classic-for, and synchronous for-of declaration surfaces: 324
paths / 648 sloppy/strict variants. All 648 are runnable and pass. The global
`destructuring-binding` capability remains closed; the gate's exact
one-feature scoped profile has SHA-256
`aa6cdca241b5f0be7eb202461ba80e44132f917a66480f1c04225cedc410d0d7`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`f6d9bda32460f3d16bd8084186c05b163e0d44a8788515fe20bf58a0f32d5c2d`,
`ab9974676a1f15442875d6b9de607a27a94a76896a949c8b9cf86b05dbac18dc`,
`bf712cfc7a3c455a2c8188baf82032876ba0321d3bf70d4c4281e00f4b945731`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`70d85400fb852c831a1088a8a53e52f8a693eea660f14fc2429983f499858d09`
and
`27218697cb5950df31ae2ef0610ca57d39ee531f4e33ab757a3145c72fafae52`.
Reproduce the gate with `scripts/test-test262-object-binding.sh`.

The exact R2r/R2s full join retains all 102,037 unique keys. Forty-nine
outcomes change across 25 paths, another 71 rows change detail only, and no
previous pass regresses. The transitions are nine `fail-parse -> fail-runtime`,
two `fail-parse -> pass`, two `fail-runtime -> pass`, two
`unsupported-parser -> fail-parse`, two `unsupported-parser -> fail-runtime`,
30 `unsupported-parser -> pass`, and two `unsupported-runtime -> pass`.
Passes rise by 36 to 34,916 while runnable variants remain 38,421. Parse
failures become 543, runtime failures 1,504, typed parser frontiers 1,168, and
typed runtime frontiers 74. Full TSV/JSONL SHA-256 values are
`616026d35b7b86f6b4e6c24d22456db9ca50b64fcc00e787472e75aeebc3e3c2`
and
`a3f633ac23d0fe6d22dcec563ec7f2296f46b2be00738176b543079b7da283e6`.

Object rest remains a typed frontier because it still needs exclusion-aware
`CopyDataProperties`. Its `Unsupported` result is now deferred until the whole
source has completed syntax and declaration scanning, preserving the priority
of later syntax errors and declaration conflicts. Exclusion-aware object rest
is the next binding slice.

## R2t object-rest binding declarations

R2t implements exclusion-aware ObjectBindingPattern rest declarations against
QuickJS 2026-06-04. Direct `var`/`let`/`const`, classic `for`, and synchronous
`for-in`/`for-of` declarations share the recursive object/array lowering. The
new depth-addressed `CopyDataPropertiesExcluded` bytecode preserves its stack
operands. After source `ToObject`, a fresh exclusion object is created before
any computed-key conversion or getter and records fixed and computed
String/Symbol keys before the copy. Computed keys receive exactly one
`ToPropertyKey`; excluded accessors are not read again; ordinary own enumerable
keys are snapshotted in String/Symbol order and then read live into fresh
writable, enumerable, configurable own data properties. Differential tests also
pin primitive boxing, sloppy `with` Reference preparation, nested patterns,
parser skip-scanning, and copy/Put failures under iterator unwind.

The dependency-audited Test262 cohort selects the three available object-rest
semantic templates across direct, classic-for, and synchronous for-of
`var`/`let`/`const` declarations: 27 paths / 54 sloppy/strict variants. All 54
are runnable and pass. Synchronous for-in rest is covered by the focused
QuickJS differential rather than this Test262 cohort. The scoped profile admits
only `destructuring-binding` and `object-rest`; its SHA-256 is
`122a2b055aaf40672a0540441861ecd1e6c09b65e88d45b947bc27a691afc45e`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`381dc052af426d6d73e498600660d479c843dee1333896958b73176e23b705d7`,
`fc75564488d2ae45a015fa8b07989f3a178f08978221d87ffdeeca0a9359fe57`,
`4b1f4177d308124eb74c0eff3a8028c4bf09b5cf713392467f635e05b03f7e7e`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`9a1a364218204b9d6aede93dadd52cb97256b1504a0f016e8d41d46cca3b26be`
and
`53d8920bf0b160e0899a56af3a64fa50be354a899d78a8ec6864be96b3c79694`.
Reproduce the gate with `scripts/test-test262-object-rest-binding.sh`.

The exact R2s/R2t full join retains all 102,037 unique keys and changes only
the sloppy and strict variants of
`test/staging/sm/expressions/destructuring-object-__proto__-1.js` from
`unsupported-parser` to pass. There are no previous-pass regressions,
missing/extra keys, detail-only changes, or other outcome changes. Passes rise
by two to 34,918, runnable variants remain 38,421, and typed parser frontiers
fall from 1,168 to 1,166. Full TSV/JSONL SHA-256 values are
`0c4e7a6e1939aaee3926e8cd2b91e05af0f61a4bfb0cf0c932827e49ea7bb95c`
and
`512e97b82df170c24e262968c6ebf73fa450be92fb1f0db14aaa58d50c17d7f6`.

Destructuring assignment, parameter patterns, and catch patterns remain
separate compiler surfaces. Destructuring assignment is the next high-yield
binding slice.

## R2u array destructuring assignment

R2u implements ArrayAssignmentPattern for direct AssignmentExpression and
synchronous `for-in`/`for-of` assignment heads against QuickJS 2026-06-04.
Direct assignments retain the original RHS as their expression result while a
separate copy feeds the pattern. Identifier, fixed, computed, and `super`
targets prepare their References before `IteratorStep`; defaults, terminal
rest, elisions, empty patterns, recursive arrays, and abrupt completion reuse
the existing iterator-region path. Matching-closer lookahead distinguishes a
real for-head pattern from valid leading literal member targets such as
`for ([].x of values)`. ObjectAssignmentPattern remains typed Unsupported, but
its frontier validates the pattern first so malformed targets retain QuickJS's
SyntaxError and source location.

The dependency-audited Test262 projection selects direct, non-nested,
non-rest `array-*` paths under `expressions/assignment/dstr`: 70 paths / 131
sloppy/strict variants, all runnable and all passing. Its runner-bound profile
admits exactly `Symbol`, `Symbol.iterator`, `const`, `destructuring-binding`,
and `let`, plus exactly three audited parse-negative paths. This is deliberately
not a global `destructuring-binding` admission; Test262 labels much of this
assignment corpus with that broader binding tag. The scoped profile SHA-256 is
`b2133d90974566c72ab788525254de68d260b44756a8c5981111873fb38727af`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`ee0b310ee20a89e3cff58469a4a7020a4a73980f5086fe189964a2c6c10c120f`,
`046679bd745132066b4982770f13236bfecdbd953b70bdba98afa60424c599c8`,
`093abb8f2b240a97cd1bcf5728cbd720203e91b5ed9df00d22f0394cd86ef4cb`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`e3b579aacafa0f63e1e17857b242311ca2512481e86f8ddbe55fcbf28267df51`
and
`832eebb660ad3f50771c60348d203cb5eaef7055098d2a07098f86d04a1b5fc8`.
Reproduce the gate with `scripts/test-test262-array-assignment-flat.sh`.

Nested, rest, synchronous loop, `super`, `with`, IteratorClose, and exact
diagnostic behavior remain covered by the separate 12/12 pinned-QuickJS
differential: 31 semantic sources, 23 exact parser CLI diagnostics, eleven exact
iterator-origin stack traces, plus Rust-only frontier and smoke checks. Object
assignment, generator/async forms, optional
chaining, parameters, and catch patterns remain separate surfaces. Nested
iterator acquisition in a synchronous for-head also retains the existing
for-of control marker instead of QuickJS's RHS value site; behavior matches,
but that debug-frame provenance remains a separate source-map follow-up.

The exact R2t/R2u full join retains all 102,037 unique keys and changes exactly
33 outcomes: 14 `fail-parse -> pass`, one `unsupported-parser -> pass`, 14
`fail-parse -> unsupported-parser`, and four `fail-parse -> fail-runtime`.
There are no previous-pass regressions, missing/extra/duplicate keys, or
detail-only changes. The newly exposed non-pass variants stop at the explicit
object-assignment frontier, missing Proxy, or an already-known staging semantic
frontier. Passes rise by 15 to 34,933 while runnable variants remain 38,421;
parse failures fall to 511, runtime failures become 1,508, and typed parser
frontiers become 1,179. Full TSV/JSONL SHA-256 values are
`17c3c36e73ad8d098ae9d3bd3fc5c5d372187830d5e11f8532bc28471fbb4da3`
and
`e9cb57c7616c27e01e156e7754b9cbc606c40100ea632bcc651c411d10c6c8e9`.

## R2v object destructuring assignment

R2v implements ObjectAssignmentPattern for direct AssignmentExpression and
synchronous `for-in`/`for-of` assignment heads. It shares the direct array
path's control inversion so the original RHS remains the expression result,
then follows QuickJS's object-specific order: `ToObject`, source key
canonicalization, complete target Reference, source Get, undefined-only
default/NamedEvaluation, and NOKEEP Put. Nested patterns read the outer
property before preparing inner References. Object rest prepares its arbitrary
identifier/member/`super` target before exclusion-aware CopyDataProperties;
computed exclusions are canonicalized once and shared by Get and copy. Arrays
and objects recurse through each other without adding a VM opcode.

The pinned-QuickJS gate passes all nine Rust tests: 35 eval differentials, five
exact CLI stack traces, 14 exact parser diagnostics, and a Rust-only smoke.
Nested source-marker inheritance remains a documented non-exact source-map
surface rather than a false stack-parity assertion.

The Test262 projection is split by semantic owner:

- flat: 67 paths / 118 variants, all pass;
- nested object/array recursion: 14 paths / 24 variants, all pass;
- rest: 26 paths / 51 variants, all pass.

The three runner-bound profiles admit only their audited features and 6/4/1
negative paths. Profile SHA-256 values are
`989f5617484d5c12a15fb26a447121fa3436b19f05cd998cf400b5d3d7179a51`,
`18411f3d674a9493806bbf6a601bda903e859395aeec572e466c4a59470ceb12`,
and
`4b9f50b982dc5c3af1466d425a1665448c4a00165d465a74fd4057ef6e414206`.
Normalized-manifest/manifest-file/key-set hashes are respectively
`51eda576685e7a42d734c789f83a3a39efd9614f59e583afb179da4aec8b053a` /
`92089af97dcc157d557061120dfdb68c868f2a8823288290a227a22bfadb285b` /
`f4f62e06502ac316a37ad3b9a55c80a48be6c12fa61b51701b04fbc510994808`,
`925359ce13f9f03e82c2357e5b8ccf1d4024a712445455237fa78f4bba328be6` /
`0e5a594cee6e1c021f310c8e9d88e8b253d789171c97511aec4adcfd346d7d27` /
`ffd426c04c9d96bcae249d576811d2eae1d9a68c455b396769db145212113010`,
and
`014a3e85c43f1ceabdc49379bd502444bc1ca93da163ad25a7ed1ad9f32f899f` /
`931d743e7e2f46d78e66baf7c7c83fcf33208fd8ced6f6c72619ec5948971226` /
`6e574b6e8c3450e0ddb29aaa3d51fe892ad086d718f062858c48f2d115e91595`.
Every non-pass hash is the empty SHA-256. Focused TSV/JSONL hashes are
`f0cd537e2349ce952828c6c61c073636b8631ca27750c7decbc4a8cd634087c6` /
`27456fb05f0015a01c37f2d6c35a0d2b44e49a20578b9e0eabe5c57d53c546d9`,
`430391c59cb61029ecdb1b7f2d81b0ec7054cba76f6bbfdab8b0840baf438669` /
`cad849b67be5b15bbe7fd63b1fa635c5f74f4d2e05c8b65941fe076bb762a37a`,
and
`14d7dba398df75de6aa4583fe126ffc3aca871890121a7f6d53df71d8da4e4de` /
`b6cb010459de59ffaab193fb7ad5fddc9fb73b1f8e437f8041fd2a56ba358964`.
Reproduce them with the three
`scripts/test-test262-object-assignment-{flat,nested,rest}.sh` entry points.

The exact R2u/R2v full join retains all 102,037 keys. All 14 former
ObjectAssignmentPattern `unsupported-parser` variants move to pass; no prior
pass regresses and there are no missing/extra keys or detail-only changes.
Passes reach 34,947 among 38,421 runnable variants and typed parser frontiers
fall from 1,179 to 1,165. Both modes of the unrelated
`staging/sm/Proxy/ownkeys-linear.js` also move from their eventual missing-Proxy
runtime failure to the 30-second timeout while constructing 15,000 properties;
that performance-only movement is kept explicit in the vector. Full TSV/JSONL
SHA-256 values are
`bbc5babdb70a470ff6d937dde2771cb7de270bc6971bfc7597e1f5bf0b24e5da`
and
`2839c0d58d8661b6cec4f6e606d297625343756dbbd656224013c17f992743fe`.

## R2w synchronous catch binding patterns

R2w implements recursive ArrayBindingPattern and ObjectBindingPattern catch
parameters against QuickJS 2026-06-04. Identifier leaves, elisions, defaults
with NamedEvaluation, terminal array rest, fixed and computed object keys,
object rest, and arbitrary array/object recursion reuse the declaration
binding owner. The thrown value is initialized inside the catch lexical scope;
iterator and property abrupt completions therefore reach the surrounding
handler/finally machinery through the same verified paths as other binding
contexts. Pattern leaves are ordinary mutable catch-scope lexicals. Only a
simple catch identifier carries the private catch-parameter marker used by
direct-eval `var` redeclaration rules.

The dependency-audited gate freezes 97 paths / 177 variants, all runnable and
all passing. It covers the synchronous `language/statements/try/dstr` corpus
whose dependencies are implemented, six audited parse-negative rest cases,
the Annex B catch-body early-error integrations, and four untagged catch-scope
paths. Class- and generator-valued defaults remain independent frontiers. The
runner-bound profile admits only `Symbol.iterator`, `destructuring-binding`,
`let`, and `object-rest`; the broad binding/rest tags remain absent from the
global profile.

The scoped profile SHA-256 is
`a654327057a974e0feab6799f3c99a3104884a403cbc41bbc85f3fc226328718`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`50c326ca60fdfa0cd5d3683df265e730c1947801db6e0892645b9bcfcd450927`,
`e3fb469169b069c185a7d9ea6b8cdce2fdb54d49181b7e87e33cff59a27c212e`,
`1f66a5b898cf1f0cb4a3dc333ee3bb4e7d5dc1361dd5a06b7c1c4be2b0573784`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`c1a01134926200028f476ca165ed8127566725bab5faa1a174e77b9f4f460557`
and
`4215e94bb7c8435345542d80ebfcad56ff91567cb4c45582c3cf8426f66dc3da`.
Reproduce the gate with `scripts/test-test262-catch-binding.sh`.

The exact R2v/R2w full join retains all 102,037 keys and adds 49 passes: 24
`unsupported-runtime -> pass`, 23 `unsupported-parser -> pass`, and two
`fail-runtime -> pass`. No previous pass regresses. The two modes of the
unrelated `staging/sm/Proxy/ownkeys-linear.js` move from timeout back to their
eventual missing-Proxy runtime failure; this is recorded performance noise, not
a catch-binding regression. Passes reach 34,996 among 38,421 runnable variants;
typed parser frontiers fall from 1,165 to 1,142, typed runtime frontiers fall
from 74 to 50, and timeouts fall from eight to six. Full TSV/JSONL SHA-256
values are
`e00e85d148fcc5d03ff7830b0e730af0a64b478c498eaad8d018d0bf1c96898a`
and
`ace137cda9b5f55762b2e729a172adbed3715659c981c53bd809f9099fcf20ae`.

## R2x synchronous identifier-rest parameters

R2x implements the identifier-only synchronous rest-parameter slice against
QuickJS 2026-06-04. Ordinary function declarations and expressions,
synchronous object methods, arrows, and the `Function` constructor share the
same formal metadata and entry initialization. Rest collects only actual
trailing arguments into a fresh Array in the callee realm; formal padding does
not leak into that Array. The first rest position also becomes the public
`length`, and a sloppy function with rest receives an unmapped `arguments`
object which snapshots the raw arguments before the rest slot is initialized.

The entry prefix creates `arguments`, initializes rest, and only then installs
body function hoists. This preserves rest under a body `var` declaration while
allowing a body function declaration to replace it at the QuickJS-compatible
point. The bytecode publication boundary authenticates the rest operand,
formal metadata, and prologue shape before the VM may slice the active frame.
The parser also pins duplicate names, non-simple-body `"use strict"`, rest
position, trailing comma, initializer, and getter/setter diagnostics across the
four admitted function forms.

This is not complete rest-parameter or FormalParameters support. Parameter
Environment creation and its direct-eval interactions, default parameters,
parameter destructuring, rest BindingPatterns, and async, generator, and class
forms remain explicit later frontiers.

The runner-bound Test262 gate freezes 34 paths / 65 variants. All 65 are
runnable and pass. Its six-feature profile admits only `Reflect`,
`String.prototype.replaceAll`, `Symbol`, `arrow-function`, `rest-parameters`,
and `set-methods` for this exact manifest, together with 11 audited negative
paths; `rest-parameters` remains absent from the global profile.

The scoped profile SHA-256 is
`da6a76cb6338019f5c233e252bf6d40b7f3eb5c4235a6967cf78f9a74917dced`.
Normalized-manifest, manifest-file, key-set, and empty non-pass SHA-256 values
are
`5cfb4770e35f128a3481a15dcff70dc4733657072fe9cf7a185c91624c355b43`,
`cc326a73c13d2cd90726150e77ad5f5a247074f12a233fe9efa382b3ec6c420e`,
`5a3751688f145e0eda20738258675c1ee27f86fc7808a8a2654dae88d3917c1a`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`7b28768f2bb46974d563728cda36e025bc5123f8d3749a32bf83a490e0ac691f`
and
`0a2d3aa3518bc8ab10c5f2bbf768bbd94bc88e809202837416849c63dfa14065`.

Reproduce both gates with:

```sh
./scripts/test-test262-identifier-rest.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_rest_parameters -- --nocapture
```

The exact R2w/R2x full join retains all 102,037 keys and every previous pass.
It adds 88 passes: 31 `fail-parse -> pass`, nine `unsupported-parser -> pass`,
three `unsupported-runtime -> pass`, and 45 `harness-error -> pass`. There are
61 other outcome changes and ten same-outcome detail changes, with no missing,
extra, or duplicate keys. Passes reach 35,084 among 38,421 runnable variants.
Full TSV/JSONL SHA-256 values are
`1ff253545ba69824b686e23d40998645a57330d83fa01a8bf9a39fa2994e4959`
and
`6a1971269b694b9c5e344884714f9f2234619a3200b6ff2e25a69e2b45e26fb9`.

## R2y synchronous identifier-default parameters

R2y implements `BindingIdentifier = Initializer` for synchronous ordinary
function declarations and expressions, object methods, arrows, and the
`Function` constructor against QuickJS 2026-06-04. The parser establishes the
callee before parsing its formals and creates a parentless Parameter
Environment at the first default. All parameter lexical cells begin in TDZ and
initialize left-to-right; earlier cells, outer bindings, `arguments`, `this`,
`new.target`, `super`, and the private function name retain their applicable
visibility while body declarations do not leak into initializers.

The implementation intentionally preserves a pinned QuickJS behavior which
differs from current Node/spec behavior: initializer closures retain the
lexical parameter cell, while the authored function body reads and writes the
raw argument slot. Thus assigning `a = 2` in the body after an initializer
captured default `a = 1` produces the differential result `2|1`. Default
substitution also updates the raw slot before lexical initialization,
`arguments` is unmapped, `length` stops before the first default,
NamedEvaluation names anonymous functions/arrows, body hoists run after the
Parameter Environment closes, and a default composes with terminal identifier
rest.

The immutable function metadata carries the leading Parameter-local count.
Unlinked publication and final heap allocation share one structural validator
for the exact reverse TDZ reset, left-to-right single initialization,
default-plus-rest ABI, and fixed-order pseudo-binding prologue. The unlinked
boundary additionally authenticates lexical definitions and pseudo-binding
names, and binds each `FClosure` capture source to its bytecode segment:
initializer closures may capture Parameter cells but not raw argument slots,
while body closures use raw argument slots and cannot recover a closed
Parameter cell. Direct eval remains deliberately unsupported in or below a
Parameter Environment: matching the target requires independent `<arg_var>`
and body `<var>` objects plus function-segment topology, so this milestone does
not substitute a one-environment approximation.

The runner-bound scoped gate freezes 76 paths / 143 sloppy/strict variants.
All 143 are runnable and pass. Its profile admits only `default-parameters`
and the required `super` surface, together with 19 audited negative paths;
`default-parameters` remains absent from the repository-wide profile. The
15-case pinned QuickJS oracle separately covers undefined/supplied values,
initializer skipping, all four parser surfaces, later/self TDZ, unmapped
`arguments`, `length`, body hoists and initializer closures, NamedEvaluation,
default-plus-rest, the target-specific raw-argument split, and private named
function bindings across direct/captured reads and strict/sloppy writes.

Profile, normalized-manifest, manifest-file, key-set, and empty non-pass
SHA-256 values are
`5c98d19ccb72c7e2c577ddc98ee4ac83d43a0ba7d49175a8ebe271866d0feab6`,
`8427bc44409269c8edbcef0c1615c7c0c37c6fbbe270c2beb119a9deb3a85bf7`,
`264bb2b25e7502eed86f8a5df1b3fe8c0ccdeecd43171af390764b5e053a6472`,
`26c1a2ac0ab8da8cfa6aca04b724cd4dece1205dfb65b093cd7888343c7c0174`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`f1775881f89d5b76f7a46f1a89391a60b213508becec9df244e2fb0d9a937bc7`
and
`dc1edd9121ce27142df0e499a8e4ccdca1e6ff43ca178a35ea40981d45538a23`.

Reproduce the focused gates with:

```sh
./scripts/test-test262-identifier-defaults.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_identifier_default_parameters -- --nocapture
```

The exact R2x/R2y full join retains all 102,037 unique keys and every previous
pass. It adds 60 passes: 35 `fail-parse -> pass` and 25
`unsupported-parser -> pass`. The 54 other outcome changes are 38
`fail-parse -> unsupported-parser` transitions at the explicit direct-eval,
destructuring, and class boundaries plus 16 `fail-parse -> fail-runtime`
transitions at already-known runtime frontiers. Sixty-four same-outcome rows
now expose a deeper diagnostic; there are no missing, extra, or duplicate
keys. Passes reach 35,144 among 38,421 runnable variants. Full TSV/JSONL
SHA-256 values are
`e02a1e768065e63af6908932dc7ba8e5ff9ec552c3dc6adbce55db91a74eb866`
and
`b762e44abbca482419b5e24ed4479a1726a8c7d25232907538c71780829d4def`.

## R2z synchronous no-default parameter BindingPatterns

R2z implements synchronous FormalParameters BindingPatterns on QuickJS's
`SKIP_HAS_ASSIGNMENT == 0` path. Ordinary function declarations and
expressions, object methods, arrows, the `Function` constructor, and
one-argument setters share recursive array/object/rest lowering. A standalone
`=` anywhere in FormalParameters deliberately stays on the later Parameter
Environment path, including nested defaults and computed-key expressions.

Ordinary patterns reserve anonymous physical argument slots. A terminal rest
pattern reserves no slot and preserves QuickJS's observable `length` behavior,
including the zero-initialized bytecode-record result for an otherwise empty
function. Pattern initialization runs in FunctionRoot before body lexical
entry and before body function hoists. Unmapped `arguments`, direct eval,
computed keys, HomeObject/`super`, iterator closing, and closures created by
the pattern follow the pinned QuickJS ordering and visibility rules.

Both bytecode publication boundaries authenticate anonymous argument reads,
the rest start, the initialization marker, its control-flow boundary, the
arguments prologue, and the absence of direct body-lexical access during the
pattern phase. The complete-tree publisher additionally authenticates child
closure instantiation and rejects a pattern-phase closure which captures a
body lexical cell.

The runner-bound gate derives 37 dependency-clean generated paths from each of
four synchronous surfaces and adds one direct unmapped-arguments consumer: 149
paths / 298 sloppy/strict variants, all runnable and passing. Its scoped
profile admits only `Symbol.iterator`, `destructuring-binding`, and
`object-rest`, together with 12 audited negative paths; these scoped
admissions do not widen the repository-wide profile.

Profile, normalized-manifest, manifest-file, key-set, and empty non-pass
SHA-256 values are
`1f25a0648044b6cb3027e23bc58032b2b2fc3517cd0a29b35d5e4d0844fc6e5e`,
`9cb9662c3c5860e05ba2199be6d3818091e64780ccf7ef61c6d63276a6747f60`,
`9cb9662c3c5860e05ba2199be6d3818091e64780ccf7ef61c6d63276a6747f60`,
`3dbed4631c1c6670bae9256f82773b62ad7a82facda80dac0fb72187fd546e92`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
Focused TSV/JSONL hashes are
`9ef03e119426a2f65dadf3898e63fa48af05469e2f194f1d6c3ab20a3d8cc9db`
and
`0a23a3e1252ddfa2cf0d8fd708b1c0646f13a8d5ccf45098b4ed102c0f3814c1`.

Reproduce both gates with:

```sh
./scripts/test-test262-parameter-binding-patterns.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_parameter_binding_patterns -- --nocapture
```

The exact R2y/R2z full join retains all 102,037 unique keys and every previous
pass. It adds 22 passes: 12 `fail-parse -> pass`, four
`fail-runtime -> pass`, four `unsupported-parser -> pass`, and two
`unsupported-runtime -> pass`. Nine former parse failures and two former
runtime failures move to the explicit Parameter-Environment frontier; 14
other rows keep their unsupported-parser outcome while exposing a deeper
diagnostic. There are no missing, extra, or duplicate keys. Passes reach
35,166 among 38,421 runnable variants. Full TSV/JSONL SHA-256 values are
`5d85f32719d07937a0e352cc665911c94014ae1f910292100821692c9cbe4546`
and
`2818623121c2991151fdb0c055090283fd5f131e5dcfdd135b97fcdb77df708c`.
BindingPatterns whose FormalParameters contain a standalone `=` are the next
R3a milestone; async, generator, and class forms remain later callable slices.

## R3a synchronous parameter-expression BindingPatterns

R3a completes synchronous BindingPatterns on QuickJS's Parameter Environment
path. A standalone `=` token anywhere in FormalParameters now pre-creates the
parentless argument scope before parsing the first parameter. The bounded
QuickJS-style lookahead retains an assignment already observed if its
256-delimiter safety limit is reached. Every identifier and pattern BoundName
is allocated in that lexical scope in source order, including the meaningful
zero-cell Parameter Environment.

Named parameters still keep their physical argument slot for body reads.
BindingPatterns use anonymous physical sources, then copy their initialized
cells into fresh ordinary body locals only after every parameter initializer
has run. Initializer closures therefore capture the Parameter Environment,
while body closures and body bytecode observe the copied locals. Whole-pattern
defaults, leaf defaults, mixed named/pattern parameters, terminal rest
patterns, function `length`, duplicate-name diagnostics, implicit `arguments`,
getter/setter arity quirks, and QuickJS's accepted but unreachable rest-pattern
initializer are all covered by focused differentials.

An immutable `ParameterEnvironmentLayout` crosses unlinked publication,
complete-tree publication, and heap installation. It records the initialization
boundary, named argument cells, pattern-copy map, raw default sources, future
synthetic-arguments and eval variable-object slots, and authenticates the exact
TDZ, initialization, default-branch, reverse-copy, body-access, closure-capture,
and control-flow topology. R3a deliberately made future direct eval use a typed
extension point instead of silently reusing this ABI. R3b below fills that
`<arg_var>` extension.

The runner-bound R3a gate derives 117 dependency-clean generated paths from
each of four synchronous surfaces: 468 paths / 936 sloppy/strict variants, all
runnable and passing. Its scoped profile admits only `Symbol.iterator`,
`default-parameters`, `destructuring-binding`, and `object-rest`, together with
36 audited negative paths. Profile, normalized-manifest/manifest-file, key-set,
focused TSV, and focused JSONL SHA-256 values are
`0addc7345b6576e1944afc3d5d84cffe16e299e44af09245e78c08cb29207f7b`,
`1db4662456a3ea231c7ce3f629d5224a8cb19d38d13d69c83e43f6407aac21c0`,
`5d4d801025b940f11608d4110169daf6f15427a063e26ca0b1770587a11f464b`,
`e7292d11cc347daf9016b28a987626ee648fc64e4740161ce843058a6fe7265c`,
and
`e6ad140b2e960920c4586455ee9905b4c982ba63e4aa7a9cfc102542c0de8827`.
The QuickJS oracle target contains 20 semantic/early-error vectors and passes
all four Rust integration tests. The pinned Test262 tree contains no exact
BindingPattern + standalone `=` + terminal identifier-rest combination, so
three of those oracle vectors explicitly freeze that cross-feature entry ABI
across ordinary functions, arrows, and object methods.

Reproduce the focused gates with:

```sh
./scripts/test-test262-parameter-expression-binding-patterns.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_parameter_expression_binding_patterns -- --nocapture
```

The exact R2z/R3a full join retains all 102,037 unique keys and every previous
pass. Twelve `unsupported-parser` variants become passes, while two untagged
staging variants advance from the old typed runtime frontier to already-known
generator/async/class runtime failures. Fifteen same-outcome rows expose deeper
diagnostics. There are no missing, extra, duplicate, or previous-pass-regressed
keys. Passes reach 35,178 among 38,421 runnable variants; full TSV/JSONL hashes
are
`a529e8bc7556be32188fa20dd9a2db121e7feba4cc0dede5d4a1882b4ba363ec`
and
`78839d051f03908350eded05b8ea99c6d9843f4668ec4aa3673b50ca60e710da`.
At the R3a landing, async, generator, and class callables remained later
callable milestones; R3e now covers synchronous base classes.

## R3b direct eval in Parameter Environments

R3b implements sloppy direct eval in and below a synchronous non-simple
Parameter Environment with the two hidden variable objects used by pinned
QuickJS. The body environment owns `<var>` and resolves static body bindings,
then `<var>`, then `<arg_var>`, then outer scopes. Parameter initializers own
`<arg_var>` and resolve static parameter cells, then `<arg_var>`, then outer
scopes. Strict eval still uses a local declaration target.

The cross-layer ABI is explicit: `ClosureVariableKind` distinguishes the
parameter variable object, `EvalScopeKind` distinguishes the Parameter scope,
and `EvalVariableEnvironment` carries the exact scope and source selected for
declarations. Compiler, both publication boundaries, Heap, and VM authenticate
the target role and sentinel rather than guessing from closure order.

The implementation also reproduces QuickJS's synthetic parameter `arguments`
cell. It remains separate from a named `arguments` formal and from the body
binding, is initialized before either variable object, and is available to
BindingPattern expressions and closures. A descendant arrow receives a late
body-arguments closure suffix only for a real authored capture; eval alone does
not synthesize one. Body closures may retain the authenticated `<arg_var>`
object after the outer activation returns.

The QuickJS oracle freezes 42 cases across parameter declaration targets,
body/parameter object separation, deletion and lifetime, `arguments`, entry
ordering, computed/default scope selection, and strict eval. All four oracle
integration tests pass. The dependency-audited Test262 gate contains 71
`noStrict` paths / 71 sloppy variants: 48 arguments/direct-eval cases, 16
scope-open/close cases, 4 redeclaration negatives, 2 computed/default cases,
and 1 staging composite. Oxide and pinned QuickJS `run-test262 -a -m` both run
and pass all 71.

Profile, manifest, key-set, focused TSV, and focused JSONL SHA-256 values are
`98b5e323db1b4be493c1e05b8937a1060b71f7a1cc126087d05e88e7c2a2b335`,
`3df66805796888dd41acbc007b2a958aba5751e9694c0deffa5f0efba19c61a1`,
`08aeb2a3e23a3a3e1bb6e03262d730cd0bbaec1d9aff0f9cc744ebc3ce003938`,
`e2759eb05400218abb31e257fe60bedfcb321e05bbffc0018d9042b60c87ec12`,
and
`a25aaf9087fc356b4b5b3d8437a52cf19166c76ec09aeefc5569f4297a93844d`.

The exact R3a/R3b full join matches all 102,037 unique keys with no duplicates,
missing or extra rows, previous-pass regressions, or same-outcome detail drift.
It records 69 outcome changes: all 66 focused `unsupported-parser` variants
become passes; outside the manifest,
`staging/sm/Function/implicit-this-in-parameter-expression.js` advances to its
known runtime mismatch and the sloppy/strict variants of
`staging/sm/Function/function-name-method.js` advance to the generator-method
typed runtime frontier. Passes reach 35,244 among 38,421 runnable variants.
Full TSV/JSONL SHA-256 values are
`41ef0f16cbae0aa05cdc0bfb13e38130b9b87b1ac958fe6e807541140cda918a`
and
`ecd12b154863534e80f5ac0f40ee6615f1a8743856e9e4f9ca98b44e00a793a0`.

Reproduce the focused gates with:

```sh
./scripts/test-test262-parameter-direct-eval.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_parameter_direct_eval -- --nocapture
```

At the R3b landing, async, generator, and class callables remained later
callable milestones; R3e now covers synchronous base classes.

## R3c AggregateError and Error cause

R3c publishes `%AggregateError%` on the existing NativeError substrate and
moves the Error intrinsic family into `runtime/intrinsics/error.rs`. The
constructor follows pinned QuickJS order: resolve `newTarget` and allocate the
branded object, convert `message`, perform the completion-aware `cause`
HasProperty/Get sequence, consume `errors`, define the own `errors` Array, and
only then snapshot `stack`. The iterable path caches `next`, allocates the
Array in the constructor's defining realm, closes after abrupt step/done/value
or indexed definition, and preserves the original throw when IteratorClose is
itself abrupt. Primitive `newTarget.prototype` falls back to the AggregateError
prototype belonging to the newTarget realm.

The QuickJS oracle freezes 19 vectors covering the intrinsic graph and
descriptors, call/construct behavior, custom and fallback newTarget prototypes,
message/cause/iterator ordering, genuine Array materialization, normal and
abrupt iterator completion, stack capture, and Error branding. Its expected,
pinned-QuickJS self-check, and Oxide/QuickJS differential tests all pass.

The complete focused feature cohort contains 28 paths / 56 variants. Fifty
pass. Six variants stop at the independent missing-Proxy frontier: the
sloppy/strict modes of
`AggregateError/newtarget-proto-custom.js`,
`AggregateError/newtarget-proto-fallback.js`, and `Error/cause_abrupt.js` use
`Proxy` in their bodies without declaring that dependency in Test262 metadata.
The gate pins those exact `ReferenceError: 'Proxy' is not defined` results so
they cannot masquerade as AggregateError failures or passes. Pinned QuickJS
passes all 28 source paths.

Profile, manifest, path/variant key-set, focused TSV, and focused JSONL SHA-256
values are
`ad9e38f7b1b42445a848ee01437e925fc23f5525276bc45dd15c5ae7a1454d7a`,
`f54979cc3881fd7d361dda7ffbbe75a5bf846e233512c7428711c1091b8474c5`,
`81e86c6e47fcc63ab2063814e34125de57fbc2ed14a8802186db5caa1be6bf5d`,
`40ee7c2976c4319b09457e311ed103bd3851a5a82ae11587794aa3dbc457b537`,
and
`019abe8aedfd1c82ee283aeb976a2364b1e124f91cb401c67407bb17556bd01b`.

The exact R3b/R3c full join matches all 102,037 unique keys with no missing,
extra, duplicate, or previous-pass-regressed rows. It records 62 outcome
changes: 52 `unsupported-feature -> pass`, six `unsupported-feature ->
fail-runtime` at the undeclared Proxy dependency, and four
`unsupported-feature -> unsupported-parser` at the existing class frontier.
The 52 passes include both modes of
`Object/seal/seal-aggregateerror.js`, which correctly consume the new feature
outside the focused intrinsic directory. Passes reach 35,296 among 38,483
runnable variants. Full TSV/JSONL SHA-256 values are
`8579dc70c2b02843b3b0e7680be35d48807bf24f17e3a6b3b2d7daabe6cfb71e`
and
`72296c8615ac07f1de8305445ff7fd9b170eb00b37e616e35679051a90536525`.

Reproduce the focused gates with:

```sh
./scripts/test-test262-aggregate-error.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_aggregate_error -- --nocapture
```

The six Proxy-dependent variants, cross-realm host fixture, class subclasses,
and Promise consumers remain assigned to their independent milestones.

## R3d argument spread calls

R3d lowers spread calls through typed `Apply(Call)`, `Apply(Construct)`, and
`ApplyEval` bytecode instead of widening the fixed-argument call ABI. Ordinary,
method, constructor, and direct-eval calls share the QuickJS-shaped temporary
dense argument-list path while retaining method receivers and authenticated
eval environments. The VM preserves QuickJS's callable/list/constructor and
eval-identity error order, and keeps the spread source and materialized values
rooted across every observable iterator and call step.

The append path reproduces QuickJS's two observable `@@iterator` Gets. It also
pins the target's fast-Array quirk: when the first Get classifies a genuine
dense Array and the iterator record's cached `next` is the direct built-in
Array iterator-next function, values are copied from the original Array
without advancing or brand-checking that second iterator.

The dependency-audited focused gate freezes 67 paths / 134 variants. It records
122 passes and an exact adjacent-feature frontier of twelve runtime failures.
Fifteen automated Oxide/QuickJS semantic differentials all pass. At the R3d
checkpoint, three dense 65K Oxide stress vectors were kept manual because the
then-current immutable shape growth was O(n²); their pinned QuickJS
expectations were self-checked, while the shared 65,534/65,535 argument limit
was checked quickly by `oracle_function_apply`. R3am removes that shape-growth
bottleneck; the three cases remain explicitly marked as stress tests rather
than routine unit coverage.

The exact R3c/R3d full join retains all 102,037 unique keys and every prior
pass. It records 122 `fail-parse -> pass`, ten `fail-parse -> fail-runtime`,
and two `fail-runtime -> pass` transitions, plus 13 `fail-parse` detail-only
refinements: 147 complete rows change. Passes reach 35,420 among 38,483
runnable variants; full TSV/JSONL SHA-256 values are
`8fe66b2478571da55c1061a56ca521fbc8f3926591eb6093d3ac537f4cdccf60`
and
`e6ae2522eb1790119f95537d946c90fb529222e9d649710ea8e1c07fd715a89b`.
The refreshed Symbol protocol gate now passes 864 / 1,010 variants, and all 42
runnable Raw JSON variants pass.

Reproduce the focused gates with:

```sh
./scripts/test-test262-aggregate-error.sh
./scripts/test-test262-argument-spread.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_argument_spread -- --nocapture
```

## R3e base classes

R3e ports the base-only path through QuickJS `js_parse_class`,
`js_op_define_class`, and `OP_define_class`. Class declarations and
expressions now have distinct outer declaration and immutable inner-name
bindings with TDZ behavior. Explicit and synthesized base constructors are
construct-only, preserve parameter/default/rest ordering and constructor
return validation, and publish the exact constructor/prototype descriptor
cycle. Synchronous instance/static methods and accessors support fixed and
computed names, inferred names, strict bodies, non-constructability, and
HomeObject-backed `super` property access.

The pinned QuickJS differential covers constructor errors, descriptors,
computed-key ordering, lexical/direct-eval behavior, HomeObject, source text,
and return handling; all five Rust integration tests pass. The
dependency-audited Test262 gate freezes 157 paths / 294 variants and
passes all 294, while pinned QuickJS passes all 157 paths. Its scoped profile
admits `class` only for that frozen manifest: the global capability profile
deliberately does not claim the whole feature. Profile, manifest, key-set,
TSV, and JSONL SHA-256 values are
`df73a1ac299cce6ade0b0638f0a4c3322310aa2db8e15a28039f483328e69f00`,
`0894fc15cf840a8897ad1b9243324c6312f28fd90e78cdafa377170d15b79f5f`,
`bb0c150613a6e85b4699f612b1c4755f04cd55a60384e8e3ac5b21e543e8de8b`,
`6049119789bd02e1d7848ec661a693c4161b769592b6567e567b21a17122703c`,
and
`7a10a6964629fdb96ed239be78587d9d1ebfdb6fd856549fbe813e5d28352521`.

The exact R3d/R3e full join retains all 102,037 unique keys with no missing,
extra, duplicate, or previous-pass-regressed key. It records 324
`unsupported-parser -> pass` and four `unsupported-runtime -> pass`
transitions. Another 50 outcomes move to deeper honest failures/frontiers, and
719 rows retain their outcome while refining the diagnostic; 1,097 complete
rows change. Passes reach 35,748 among the same 38,483 runnable variants. Full
TSV/JSONL SHA-256 values are
`10e3fee1e93b3491b4c97041990cd17a7f1051dbcd2d0d13c6514961934200ae`
and
`b863a62f5e7dbfcff8975fae28251731b80103f63b3c039d62f1f98271720ada`.

The full run also exposed a named class captured from a BindingPattern default
inside parameter initialization. Its local is now carried as explicit
initializer-scope provenance and authenticated at both publication layers;
forged TDZ lifecycles and body-side access/capture remain rejected. Class
heritage/derived constructors and `super()`, fields/private elements, static
blocks, and generator/async class methods remained typed frontiers at the R3e
landing.

Reproduce the focused gates with:

```sh
./scripts/test-test262-class-base.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_class_base -- --nocapture
```

## R3f derived classes and `super()`

R3f was frozen before implementation so the class milestone started from an
authenticated dependency closure rather than a path-name sample. At the R3e
full-vector baseline, 294 paths / 588 variants stopped at the exact typed parser
detail `class heritage and derived constructors are not implemented yet`. The
focused cohort also carries already-passing `super` regressions, class-tagged
paths which the global profile intentionally keeps closed, and exact
parse-negative provenance checks.

The implementation ports the pinned QuickJS heritage and derived-constructor
path: LeftHandSideExpression heritage evaluation, constructor validation before
one observable `prototype` read, `extends null`, constructor/prototype wiring,
raw-argument default forwarding, and explicit fixed/spread `super()` with the
live superclass snapshot taken before argument evaluation. Derived `this`
remains a one-shot TDZ cell; object, undefined, and primitive returns follow the
distinct constructor protocol; `new.target` is preserved through
`Reflect.construct`; and arrows, parameter initializers, and nested direct eval
relay the same authenticated cells.

The bytecode boundary makes that authority explicit. `MarkSuperCall` protects
the authenticated active-function/new-target pair through argument control
flow, only `ConstructSuper`/`ApplySuper` results may initialize derived `this`,
and publication traces all three pseudo bindings through ParentLocal,
ParentClosure, and EvalEnvironment origins. The synthesized default constructor
also has an exact fail-closed shape rather than a name-based privilege.

The first audit draft reported 376 paths. That number omitted the 18 class
declaration paths under
`test/language/statements/class/subclass-builtins/` while retaining their
class-expression mirrors; all 18 depend only on already-present intrinsics and
pass pinned QuickJS, so the omission was corrected to an intermediate 394
paths. A subsequent source-body audit removed three paths whose metadata does
not advertise async support but whose programs contain async methods:

- `test/language/expressions/object/method-definition/early-errors-object-method-formals-contains-super-call.js`;
- `test/language/statements/class/definition/early-errors-class-method-body-contains-super-call.js`;
- `test/language/statements/class/definition/early-errors-class-method-formals-contains-super-call.js`.

Those are async-grammar frontiers, not evidence for synchronous derived-class
support, leaving a provisional 391 paths / 777 variants. A second,
execution-backed source audit then removed five more paths whose metadata does
not declare their adjacent intrinsic dependency: the statement-side
`subclass/builtins.js` directly extends `Uint8Array`, while
`superCallBadNewTargetPrototype.js`, `superCallBaseInvoked.js`,
`superPropDelete.js`, and `destructuring/order-super.js` directly require
`Proxy`. Whole Test262 files cannot be partially admitted, so their otherwise
useful derived-class assertions remain outside this gate until those globals
exist.

The final dependency-audited cohort is therefore 386 paths / 767 variants.
Its R3e global-profile outcomes are 95 pass, 544
`unsupported-parser`, 104 `unsupported-feature`, and 24
`unsupported-negative-provenance`. The focused profile contains exactly the
17 metadata feature tags used by those paths and all 29 parse-negative paths;
it adds `class` only inside this frozen gate. The global profile must continue
to omit whole-feature `class` until fields, private elements, static blocks,
and async/generator class forms are complete.

The 19 immediate heritage-frontier paths intentionally excluded from this
gate require ArrayBuffer/DataView/TypedArray (seven), Promise (two), Proxy
(six), WeakMap/WeakSet (one), private elements (two), or optional chaining
(one). Broader source-linked adjacent populations remain feature-gated: 212
public-field paths / 421 variants, 175 private-element paths / 346 variants,
two static-block paths / four variants, 95 async paths / 139 variants, 40
generator paths / 56 variants, and 15 host-dependent paths / 30 variants.
Those counts overlap where a test combines features and are an adjacency
inventory, not a proposed manifest.

Three otherwise in-scope staging paths are also excluded from the all-pass
oracle gate because pinned QuickJS 2026-06-04 itself records them in
`test262_errors.txt`: `boundFunctionSubclassing.js`, `strictExecution.js`, and
`superPropOrdering.js`. They remain separate target-known-error evidence rather
than being hidden or misreported as derived-class dependencies.

Pinned QuickJS passes all 386 selected paths, and Oxide passes all 767 variants
with no failure, unsupported result, timeout, crash, or infrastructure fault.
The manifest, focused profile, variant-key, TSV, and JSONL SHA-256 values are
`c9c477104d7f538c4b3fa58a108171be866273bedf19825bedf682afc9d00366`,
`1aa167fef279273185060224bd8a65765283d95fe1e08986c5c4ea197657e160`,
`366f33fe39e2980a2a7e6c94e4e20896cd415b8e93b0118f69bc33c39c07e1e5`,
`69467d4d2f8c76ec299e97ce9c88bf74cee35e5cdae42e029377761aa25e4b8a`,
and
`abbe6c64c2fe250f477cf95085c9201a9b9654a2ef01deaa826dff1fea9b1193`.

Two overlapping existing scoreboards independently record the same class
progress: the named-groups gate moves four derived-RegExp variants to pass and
reaches 198/202, while the Symbol-protocol gate moves 28 derived-class and
spread-`super()` variants to pass and reaches 892/1,010. Neither loses a prior
pass.

The exact R3e/R3f full join retains all 102,037 unique keys, has no duplicate,
missing, extra, or previous-pass-regressed row, and records 633 outcome changes:
545 `unsupported-parser -> pass`, 37 `unsupported-parser -> fail-runtime`, two
`unsupported-parser -> fail-parse`, and 49
`unsupported-harness-parser -> harness-error`. Six more rows refine only their
diagnostic. The 544 pass transitions inside the focused manifest are exact; one
excluded pinned-known-error strict variant also passes, while the 88 other
outside-manifest transitions expose missing ArrayBuffer/TypedArray/Promise/
Proxy/WeakMap support, optional chaining, or pinned QuickJS staging
differences. Passes reach 36,293 among the unchanged 38,483 runnable variants.
Full TSV/JSONL SHA-256 values are
`018c55de6e745b35eae7bb8f7d1c3b7680579a58d8bbb241641d860c723a0e34`
and
`995cce2dc58694f8728e1ad12602b2ec5c65169f650cff5047e45d84bc4b407a`.

Reproduce the complete focused vector and the dedicated QuickJS differential
with:

```sh
./scripts/test-test262-class-derived.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_class_derived -- --nocapture
```

Use `--check` to authenticate only the frozen inputs and pinned QuickJS oracle.
Global `class` remains disabled; host realms, Proxy, fields/private
initialization, static blocks, and async/generator methods remain separate
probes.

## R3g public fields and static blocks

R3g freezes public instance fields, public static fields, and static blocks as
a separate dependency-audited cohort. It contains 386 paths / 767 variants;
the matching R3f path and variant counts are coincidental, not a reused
manifest. Its overlapping inventory includes 305 public-instance-field paths,
51 public-static-field paths, 54 static-block paths, and 119 parse-negative
paths. Ten source-audited adjacent cases are excluded because they require
Proxy or generator/async grammar not declared reliably by their metadata.

Before implementation, all 767 variants failed closed under the global
profile. With the manifest-scoped admission profile, Oxide now passes all 767
variants with no failure, unsupported result, skip, timeout, crash, or runner
fault; pinned QuickJS 2026-06-04 passes all 386 paths. This is a focused
scoreboard, not a full-Test262 percentage or a whole-feature `class` claim.
The global profile remains closed while private elements, async/generator class
forms, host hooks, Proxy, and other adjacent dependencies are incomplete.

The cohort checks computed-key and initialization order, base-versus-derived
constructor timing, public-field descriptor creation, inferred names,
HomeObject-backed `super`, direct eval, static-block scope, and abrupt
completion. A pinned transcript separately compares representative combined
observations byte-for-byte with QuickJS.

Reproduce the focused gate and differential with:

```sh
./scripts/test-test262-class-public-init.sh
cargo build --bin qjs
./scripts/test-r3g-class-public-init-oracle.sh --oxide ./target/debug/qjs
```

Use `--check` on either script to authenticate its frozen inputs without
claiming a new full-vector baseline.

## R3h private data fields

R3h is deliberately field-only: private instance data fields, private static
data fields, their read/write/update References, and `#name in value`. Private
methods, private accessors, and their shared brand operations are not included
in this checkpoint.
The semantic anchors are QuickJS 2026-06-04's own-field operations at
`quickjs.c` 8365-8460, private-`in` operator at 15964-15999, class-field parser
and initialization path at 24314-24330 and 25049-25629, and private-reference
resolution at 33281-33466. The adjacent `JS_AddBrand`/`JS_CheckBrand` path at
8462-8550 was the next methods/accessors milestone at the R3h checkpoint.

The dependency-audited cohort contains 630 paths / 1,260 sloppy-and-strict
variants: 405 positive paths and 225 parse-negative `SyntaxError` paths. Oxide
passes 1,260/1,260 and pinned QuickJS passes 630/630: 100% of this focused
cohort, with zero failure, unsupported, skip, timeout, crash, or runner fault.
This is not a full-Test262 percentage or a claim for all private elements.

The focused profile is hash-authenticated to the exact manifest. Its profile,
manifest path stream, variant-key stream, TSV, and JSONL SHA-256 values are
`c03c22a7ea0d767536c77f1720b5c87766b06759d8a42a6e7b9ec3069633ffa2`,
`8ae21223239ac757bad085913f11f0d86f0b371d66131843932824eb69744f78`,
`dc8a4cd362471eb05abc94b29a5c0ffcb967e5224ab0a75eb50446083015c6ac`,
`755120cd0d3222bf2ec26d43813470dcab31a0ecb6a9f25b904d121df4e35b78`,
and
`f391809104b47e5e05609e625321df4a2759339a080a17d98a34f7be2f181ec4`.
The global profile continues to reject the three private-field tags; ordinary
public observer methods and function-valued data fields do not widen this gate
to private method syntax.

Reproduce the frozen inventory and pinned QuickJS oracle with:

```sh
./scripts/test-test262-class-private-fields.sh --check
```

Run the same command without `--check` for the authenticated Oxide vector.

## R3i ordinary synchronous private methods

R3i adds ordinary synchronous private instance and static methods and their
per-class-side QuickJS brand semantics. It deliberately does not admit private
accessors or async/generator private forms. Each class evaluation creates fresh
instance-side and static-side brands; method callables are shared within that
evaluation, non-constructible, named `#method`, HomeObject-backed for `super`,
and read-only. Hidden own receiver markers remain outside public reflection and
ordinary extensibility, while `#method in value` and wrong-brand diagnostics
follow the same typed brand path. An initialized method with no published brand
also preserves QuickJS's priority: `expecting <brand> private field` is thrown
before a primitive receiver can produce `not an object`. For forward
`#name in object` before a field or method cell initializes, the fixture also
locks QuickJS's internal `[unsupported type]` own-property atom behavior.

The differential fixture also authenticates nested arrows/functions/direct
eval, nested classes, forward names, computed-key and initializer order,
inheritance, non-extensible replacement receivers, reevaluation, exact error
priority, and QuickJS's abrupt computed-key reentry behavior. If the key throws
before class-scope closure, escaped closures keep the captured private VarRef
and the next reentry reuses and resets that cell; normal closure still creates
a fresh identity for the next evaluation.

The dependency-audited manifest at Test262 commit
`5c8206929d81b2d3d727ca6aac56c18358c8d790` contains 267 paths / 534
sloppy-and-strict variants: 219 positive paths and 48 parse-negative paths.
Oxide passes 534/534, pinned QuickJS passes 267/267, and the non-pass report is
empty. This remains a focused manifest-scoped result, not a whole-feature or
full-Test262 percentage.

The profile, manifest file, manifest path stream, TSV, JSONL, and non-pass
SHA-256 values are
`76b0fcc5610e2ceee386469344fd727a8c359abe884befccec1ab435fed93315`,
`af3047bf66c6477f34d4229b03493a2c4247cc3f6f2b5dc4bf26e40c3ed4c7b6`,
`7ea0bbef5d3b5b27aa5e661574fbb0f53cc65fa785874bd1baabb1d83339b375`,
`89dacb36c99d9266e65dd7b0614d93d593007bac3cf0398b1ed0cb1a2258b357`,
`a7a32da2995f30bb21646817d21a2389da92e5b2b17e0c3922179d4e52dd637a`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
The differential JavaScript and expected transcript hashes are
`23053aea3d41c9ee72a61007c713a17d7082dd418c9b06433a03800173b77567`
and
`7e87481d5b8a4202554d7c50264bb8063547512468f8c2df22bf05d06965e452`.

Reproduce the authenticated gate and pinned differential with:

```sh
./scripts/test-test262-class-private-methods.sh
cargo build --bin qjs
./scripts/test-r3i-class-private-methods-oracle.sh --oxide ./target/debug/qjs
```

Use `--check` on either script to authenticate only the frozen inputs.

## R3j synchronous private accessors

R3j implements the synchronous private getter/setter admission target
separately from R3i. Starting from the 651-path
metadata-minimal private-method inventory, the shared source audit removes 79
adjacent paths and leaves 572 minimum synchronous paths. The accessor selector
then partitions those paths exactly into 305 private-accessor paths and the 267
ordinary-private-method paths already admitted by R3i.

All 305 accessor paths have both sloppy and strict variants, producing 610
variants: 229 positive paths and 76 parse-negative `SyntaxError` paths. Oxide
passes 610/610 and pinned QuickJS 2026-06-04 passes 305/305. The non-pass
vector is empty. This remains a focused manifest-scoped result, not a
whole-feature or full-Test262 percentage.

The accessor manifest includes only the same audited class/private-field
dependency tags as R3i. It excludes module/raw/async flags, other feature tags,
and the shared eval, Function-constructor, Proxy, await/yield, async,
generator, static-block, and optional-chaining source frontiers. Within the
pre-filter accessor inventory, 18 paths / 28 variants are excluded: 14 eval
paths and four Function-constructor paths; the other source categories have no
accessor overlap.

The profile, manifest file, manifest path stream, positive stream, and negative
stream SHA-256 values are
`1040d156877d88f6aae651f90b8fae472a8a4054d21f49bbbf2162d280afd884`,
`f8d7b7cb065cf15bae4066ec0790d1c7f0da513b83c8166aef20b3ad7e024cf4`,
`ca77913172666cbe4e74a6476f7f4d87383e801260b2c5b80932dc15e8e98cd6`,
`8ef30d5843d48aaee66a55834c79d710ed8f8d0afa89ea368dee89fef75d897c`,
and
`9d0e56fa4e6fd1ac21a075733fdd327d41f3107500506fbff5987960be1a5901`.
The variant-key stream, TSV, JSONL, and empty non-pass SHA-256 values are
`6c72f931034ee9e2e4b13910c5d88f4d06b527ff49cf6fa6211c751ad28b40a1`,
`aa54c8da45ac9a32aaeb9202ee5aae375a1b42dca0ac59928d78fd11042a02f0`,
`655a02032e50f63b281dce8cc5364d3c6aeff210a1bd3f69adae27c4c053c491`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

The pinned differential covers getter/setter pairs and one-sided accessors,
including QuickJS's setter-only `#name in value` internal-tag quirk; partial
getter/setter initialization; instance/static brands and diagnostics;
initializer and `super()` ordering; HomeObject `super`; nested function,
arrow, class, and direct-eval capture; fresh class reevaluation; duplicate
brand insertion; abrupt computed-key VarRef reentry; and duplicate-name parser
rules. Its JavaScript and expected transcript SHA-256 values are
`0ee124bbd77f45ae9cd81bc6203cedd03e03b5e78640460abc9670ca77ffca12`
and
`c2656658102e7bfd9ee8da51848e18519afccb9a9ec02cc094d27cb6646d834a`.

Reproduce the authenticated gate and differential with:

```sh
./scripts/test-test262-class-private-accessors.sh
cargo build --bin qjs
./scripts/test-r3j-class-private-accessors-oracle.sh --oxide ./target/debug/qjs
```

Use `--check` on either script to authenticate only the frozen inputs.

## R3k synchronous generators

R3k implements synchronous generator declarations and expressions, public
object/class generator methods, `yield`, and synchronous `yield*`. The first
authenticated Test262 gate deliberately freezes the smallest directly audited
public class-generator inventory before widening feature admission: 82 paths /
160 sloppy/strict variants, comprising 44 positive paths and 38 parse-negative
`SyntaxError` paths. Four `onlyStrict` paths account for the two-variant
difference from a full 164-row expansion.

Oxide passes 160/160 variants and pinned QuickJS 2026-06-04 passes all 82
paths. There are no failures, unsupported results, skips, timeouts, crashes, or
runner faults, and the non-pass vector is empty. The gate includes instance and
static class generator methods, definition/name behavior, parameter/body var
scope, early errors, and two direct `yield*` paths. Runtime Rust tests and the
pinned differential separately cover function and object forms, every
`next`/`return`/`throw` state, catch/finally and reentry, delegation forwarding
and close behavior, first-`next` arguments, closures/direct eval,
`this`/`arguments`/`super`, multiple instances, GC, dynamic GeneratorFunction
calls, prototype selection, descriptors, binding, and non-constructibility.

The manifest, profile, variant-key stream, TSV, JSONL, and empty non-pass
SHA-256 values are
`30857ac44aa29bf86925b72b14da28c9215fb3bc29f81fc6b950694fa0d70b0f`,
`eab79cc5f8ba041e93b7ea04bc391bed8fa249eaf5cbb11857d533fe27028c52`,
`184f80aeb39690da69a802db371fe30cd1678726797181b4a660bf25a9996256`,
`018401955c96b0909e2a56e76be443556e790f4a06dd067bd2d70414afa8e94f`,
`6d005f8570ef7bb45b36b50a65cb6672e1e6863a67bf825eee0ccc25a2438f99`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
The checked-in baseline also authenticates the pinned Test262 patch/config/
metadata and current global-profile provenance. R3t refined ten passing
negative-row `yield` diagnostics without moving an outcome; the hashes above
are the refreshed vectors.

The global profile did not admit `generators` at the R3k checkpoint. R3k kept
private class generator methods separate; R3l measures that slice below. R3u
now admits the authenticated synchronous cohort globally. Async
functions/generators, Proxy-dependent paths, and unrelated unsupported
dependencies remain separate frontiers, so this focused gate is not a
whole-Test262 percentage or a full generator-parity claim.

A supplemental fresh-tree dependency audit at the R3k checkpoint broadened the
selection to 1,203 paths / 2,378 variants. Oxide passed 2,376 with zero engine
faults or skips.
The only two non-passes are the sloppy/strict variants of
`test/language/statements/class/static-init-arguments-methods.js`: that file's
unrelated ordinary async method reaches the explicit
`async class methods are not implemented yet` parser frontier. Every
generator-dependent row in the expanded selection passes. Its path stream,
variant-key stream, profile, TSV, and JSONL SHA-256 values are
`8aaa256a04dd6b8b4d0ebfb6c49f70fa21efe0abdff9f8dfc591858539891c80`,
`cdf4ec0a992ec3d034111871945f14f0c488c2d114610d48174565a0d890a360`,
`d3cc7178cf10be7166ec3dcb8d690ce487fa85dd697c74ad0b7cecfa5663f0fa`,
`42d06dde909a48d6f961697c68d32a4809a01778075be79a4a15bde599412d93`,
and
`50108d91e551c71c9659487aaec997324099e13f8c6422e8302b549c588a5378`.
This is breadth evidence rather than a second checked-in acceptance gate.

Reproduce the authenticated gate with:

```sh
./scripts/test-test262-class-generator-methods.sh
```

Use `--check` to authenticate only the frozen manifest/profile and pinned
QuickJS oracle.

## R3l private synchronous class generators

R3l closes the private instance/static class-generator slice without adding a
parallel callable or private-element representation. Private generator methods
retain the existing authenticated `PrivateMethod` cell and class-side brand;
their child bytecode carries the orthogonal generator execution kind. The
unlinked publisher, linked heap verifier, and runtime cell reader accept only
the two legal method shapes—ordinary without an own prototype or generator
with one—while private accessors remain ordinary-only.

The authenticated bootstrap gate starts from a 90-path candidate
universe, excludes eight instance/static expression/statement paths whose
declared `object-spread` dependency is outside this slice, and freezes the
remaining 82 paths / 160 sloppy/strict variants. The manifest contains 16
positive paths and 66 parse-negative `SyntaxError` paths: 36 direct private
generator paths, eight core name/production/valid paths, and 38 early-error
paths. Four `onlyStrict` paths account for the two-variant difference from a
full 164-row expansion.

Oxide passes 160/160 variants and pinned QuickJS 2026-06-04 passes all 82
paths. There are no failures, unsupported results, skips, timeouts, crashes, or
runner faults. The manifest, profile, variant-key stream, TSV, JSONL, and empty
non-pass SHA-256 values are
`b7b2c71cab374f9bcc6754bd9a80506d273d2e135e3f66eb373f325c94d33685`,
`e3732db0b47608265f4f950c1c72929e782eb507597c5f0b336896e51874133e`,
`74f827bf644507c0f0101d6597a8c5560de82b8d2303ef236beef1f3ac9de22d`,
`24f51f0526a7c950b229ae789be58ccc42eb167f0d0f80c8c788fca832619654`,
`2f54d423f00a410b57c6dbd4c1e3fe1c82fd8bf965f07dcf6d6bb07f69192486`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
R3t refined eight passing negative-row `yield` diagnostics without moving an
outcome; the hashes above are the refreshed vectors.

The pinned differential independently locks parameter evaluation before the
first `next`, resumable instance/static bodies, brand-check ordering, callable
identity and extraction, reflection and source text, dynamic `super`, direct
eval/private capture, `yield*`, fresh class evaluations, static subclass
separation, and class evaluation resumed from an outer generator. Its source
and expected-transcript SHA-256 values are
`5af87d8181536da15ba5458ab97698e40d5df953955751bb74656a95a5dd382f`
and
`ff79f3ed6798a77b04e1baec6a6e022a46538f0e463707298cee894487c1a2dc`.
Rust white-box tests additionally lock all three callable-shape defenses and
the GC/realm lifetime of a suspended private generator.

A broader dependency inventory contains 714 primary private-generator paths /
1,420 variants. Oxide passes all 1,388 runnable synchronous variants; the
remaining 32 variants are 16 async-adjacency paths rejected at selection as
`unsupported-async`, with zero engine fault, crash, timeout, or skip. Pinned
QuickJS passes all 714 paths. The groups are 160 frozen-gate passes, 1,072
destructuring passes, 40 arguments-object passes, 12 object-spread passes, and
104 adjacency/name passes plus the 32 async selections. Inventory, variant-key,
normalized-report, and non-pass-stream SHA-256 values are
`84434292de9506822d95c5afef5590d78db2cbb4d0bddeeb3acb9e9e7d1399b1`,
`5fbee112b9ea46b5ba4002b0398e5b7045e97c9d2120a23e524f971a907b0c6c`,
`f48961f1d6223eccabaa2a17726898f8abd76081bf91769a8f9503e4851d3355`,
and
`867ef271b2a97d5de723276b22ce7ec50f36c01f2cddc05aeab19eb515ec6658`.
This is breadth evidence, not a second acceptance gate. The global capability
profile still deliberately keeps `generators` fail-closed, so neither focused
gate changes the whole-suite percentage.

Reproduce both gates with:

```sh
./scripts/test-test262-class-private-generator-methods.sh
cargo build --bin qjs
./scripts/test-r3l-class-private-generators-oracle.sh --oxide ./target/debug/qjs
```

Use `--check` on either script to authenticate only the frozen inputs and
pinned QuickJS oracle.

## R3m Promise constructor and jobs

R3m establishes the first Promise/microtask acceptance boundary without
claiming the rest of the 652-path Promise tree. The frozen candidate universe
is the 58 JavaScript files directly under `test/built-ins/Promise/`; the single
`proto-from-ctor-realm.js` path remains excluded because it requires the
separate `$262.createRealm` host capability. The resulting gate contains 57
paths / 112 sloppy/strict variants: 26 async paths, 31 synchronous paths, and
one each of `noStrict` and `onlyStrict`.

Oxide passes 112/112 variants and pinned QuickJS 2026-06-04 passes all 57
paths. There are no failures, unsupported results, skips, timeouts, crashes, or
runner faults. The manifest, scoped profile, variant-key stream, TSV, JSONL,
and empty non-pass SHA-256 values are
`6cd3564883d5c0e459872b835e19ee7bb8c7f13716824fa2617ca1e698d5ed25`,
`f3a07d4c1c839b4d252ed65f8fb9cadc1862cd31280002caa4656d581007eb71`,
`0290f32ed1fe1968adf0e039748011f30588f4c1ac4b99719c5ce95d1ed9623c`,
`ae6c2454e0aba85f1ce89e1216007c863bcefbf3ce092b2f231549e544b689cf`,
`0d0c92b15448bf8ef94f040ff36c970e1c1d795bfdc99a720e1dff45d1071c18`,
and
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

The scoped profile declares only the metadata features actually present
(`Reflect`, `Reflect.construct`, and `arrow-function`) and opts into the
Test262 async host through `[execution] async=true`. That opt-in loads
`doneprintHandle.js`, captures every string argument passed to `print` behind a
read-only snapshot boundary, drains the FIFO runtime job queue, and requires
exactly one `$DONE` report. The global profile has no execution section, so
async tests remain fail-closed outside this pinned manifest.

Reproduce the gate with:

```sh
./scripts/test-test262-promise-constructor-jobs.sh
```

Use `--check` to authenticate only the frozen manifest/profile and pinned
QuickJS oracle.

## R3n Promise.try, Promise.withResolvers, and Promise.race

At its landing checkpoint, R3n freezes every file directly under the pinned
Test262 `built-ins/Promise/race`, `Promise/try`, and
`Promise/withResolvers` directories: 112 complete paths / 224 sloppy and
strict variants. The inventory contains 94 race paths, 12 try paths, six
withResolvers paths, 66 async paths, and 46 synchronous paths. No negative test
or unrelated Promise directory is admitted.

At R3n landing, Oxide passes 214 variants. The remaining ten are
`fail-runtime`; there are zero
unsupported results and zero skips. Pinned QuickJS 2026-06-04 passes all
112/112 paths. Every failure is the sloppy and strict mode of one of these five
explicit `Promise.all`/`Promise.prototype.finally` adjacency consumers:

- `test/built-ins/Promise/race/resolved-sequence-extra-ticks.js`
- `test/built-ins/Promise/race/resolved-sequence-mixed.js`
- `test/built-ins/Promise/race/resolved-sequence-with-rejections.js`
- `test/built-ins/Promise/race/resolved-sequence.js`
- `test/built-ins/Promise/race/resolved-then-catch-finally.js`

The passing rows cover method descriptors and generic/custom constructors;
`Promise.try` argument forwarding, synchronous callback invocation, return and
throw routing; `withResolvers` result shape and first-call settlement; and
`race` empty-input pending state, FIFO resolution, one-time resolve lookup,
pinned iterator-next no-close behavior, abrupt resolve/then IteratorClose, and
job-graph lifetime across GC. Thus the ten adjacent failures do not widen the
implemented R3n semantic frontier.

Manifest, scoped-profile, variant-key, adjacency-inventory, non-pass, TSV, and
JSONL SHA-256 values are
`be545aefd5f2029faae9745d859a43de176ec9865599a916f15ec465bf84d340`,
`8548d12a4d7f3141583b986c8e3ffcae4e1afb93476ae8a444f64b940bb44654`,
`bfe113d1c47283c84f5fc5f97e30cc74e3fea8d5975a3b87129e5b51eb05d7db`,
`9383382995694ab1f7356f23541c00e5f99910dfd6d80ab6f38662117043e7ae`,
`2fb9eb8c655158ba09dffcad4c9e50f96584cb218ad5e2e5d43a4216b90d3790`,
`faf0b4f680edab60b560e54a62ad0b9ba242c7b85abe92c9714b4152c87324cf`,
and
`fc10101195f430cd4c382c84a4a1a7bd84bb05daff24cd3e7d62351e7dda0968`.
The independent pinned QuickJS static-method fixture/transcript hashes are
`2bc2a52869d42f314614905f4ac750b87064d6e44cbcfdcb20b3703522bdd0b2`
and
`0da636dbcf08f6d6ec112b439a54ec3d6b0816fff34f1381516a5cad3789f16d`.

The scoped profile declares exactly its eight observed feature tags and
`[execution] async=true`. The global profile remains byte-identical at
`1860224ce1e828406f4869b66b3f1964f96fad85e4eab6ba7fecb256b4b6c2f2`;
it has no execution section, so async execution and the two new Promise feature
tags continue to fail closed outside this authenticated manifest.

Reproduce both locks with:

```sh
./scripts/test-r3n-promise-static-oracle.sh --check
./scripts/test-test262-promise-race-try-with-resolvers.sh
```

Use `--check` on the Test262 command to authenticate only its frozen
manifest/profile and pinned QuickJS result.

At the R3o checkpoint, this same frozen R3n inventory passed 216/224 variants.
Its eight `fail-runtime` rows, with zero unsupported results and zero skips,
were the sloppy and strict variants of these four `Promise.all`
consumers:

- `test/built-ins/Promise/race/resolved-sequence-extra-ticks.js`
- `test/built-ins/Promise/race/resolved-sequence-mixed.js`
- `test/built-ins/Promise/race/resolved-sequence-with-rejections.js`
- `test/built-ins/Promise/race/resolved-sequence.js`

Both variants of `resolved-then-catch-finally.js` passed at that checkpoint.
The R3o-checkpoint non-pass, TSV, and JSONL SHA-256 values are
`0865a76b4a9760298b3725c3b1e46559dabeb69e097b07cd9098882f595e64ba`,
`b37787f5024f9132fb4148e6b87a247c05e9439302dd19069c18e44dd1858469`,
and
`21dd45dcc42d79af81e1ff9c979690cbacca86fe1e24e2728edffc104bc300a0`.
The manifest, scoped profile, variant keys, adjacency inventory, and
static-method fixture/transcript remained byte-identical. This was an R3o
cross-milestone result, not a rewrite of the 214/224 authenticated R3n landing
checkpoint and its hashes above. R3p's current result is recorded below.

## R3o Promise.prototype.finally

R3o freezes all 29 files directly under the pinned Test262
`built-ins/Promise/prototype/finally` directory, producing 58 sloppy and strict
variants. The complete cohort contains 12 async paths / 24 variants and 17
synchronous paths / 34 variants. It has no negative tests or unrelated Promise
directories; its sole Proxy path contributes two variants.

Oxide passes 56/58 variants. The only failures are the sloppy and strict modes
of `test/built-ins/Promise/prototype/finally/this-value-proxy.js`, both
classified `fail-runtime` because `Proxy` is not yet defined. There are zero
unsupported results and zero skips. Pinned QuickJS 2026-06-04 passes all 29/29
paths. The scoped profile admits exactly the observed feature tags `Promise`,
`Promise.prototype.finally`, `Reflect.construct`, `Symbol`, `arrow-function`,
and `class`, plus `[execution] async=true`; the global profile remains
fail-closed.

The implementation follows pinned QuickJS `quickjs.c` 54057-54135. It requires
an object receiver, performs `SpeciesConstructor` before testing whether
`onFinally` is callable, and preserves QuickJS's `undefined`
default-constructor sentinel instead of eagerly substituting the intrinsic
Promise. That sentinel makes the later
`PromiseResolve(undefined, cleanupResult)` TypeError observable. A
non-callable argument is forwarded twice to the receiver's dynamic `then`.

Callable cleanup is represented by typed
`PromiseFinallyHandler(Fulfill|Reject)` native functions carrying
`InternalCallableData::PromiseFinallyHandler { constructor, on_finally }`.
Each handler invokes `onFinally` with an `undefined` receiver and no arguments,
routes a thrown cleanup error directly, performs `PromiseResolve` with the
captured constructor and cleanup result, then attaches a typed
`PromiseFinallyThunk(Fulfill|Reject)` through a dynamic `then`. Its
`InternalCallableData::PromiseFinallyThunk { value }` returns the original
fulfillment value or throws the original rejection reason. This locks the
QuickJS sequence of species lookup, callback, resolve, and dynamic `then`, as
well as the rule that failed cleanup overrides the original settlement while
successful cleanup preserves it. Heap validation checks that each native ID
has the matching typed capture, a constructible constructor when present, a
callable cleanup function, and a storable thunk value.

Pinned QuickJS runs Promise resolving class callbacks and its
`JS_NewCFunctionData` capability/finally callbacks in the calling Context,
while ordinary C built-ins switch to their defining realm. Oxide models this
as a typed dispatch policy covering the resolving pair, capability executor,
finally handlers, and finally thunks. A two-Context regression exposes a
finally handler from one Context, calls it from another, and verifies that its
TypeError uses the caller's `TypeError.prototype`. The pinned context-routing
anchors are `quickjs.c` 6025-6044, 17588-17612, 17742-17750,
53352-53357, 53508-53515, and 54070-54121.

GC tracing follows those typed captures: the handler owns constructor and
callback object edges, while the thunk owns raw settlement edges. A Symbol
settlement additionally goes through `retain_raw_value_atoms` during internal
function allocation and through the heap's raw-value atom enumerator;
allocation failure releases the acquired shape. The differential transcript's
`symbol-thunk-thrower-gc=value:true|thrower:true` and `finally-gc=42` rows lock
both value/thrower thunks and the complete finally graph across forced GC.

The manifest and manifest-file hash are both
`9c24a81143fc4d3dcaa8251a2ed98e381f07cb7969f30427a60e9ca931941464`.
The scoped-profile and global-profile SHA-256 values are
`fa10d45a7ddd3924e9124cfc42239e296847223c6c9686beb2a8435e9c83bf04`
and
`1860224ce1e828406f4869b66b3f1964f96fad85e4eab6ba7fecb256b4b6c2f2`.
The variant-key, async, synchronous, Proxy, feature, and include inventory
hashes are
`d468c957b3132cb0dcfb0f9ab2d76237cbefc2b5b86a8ba387c072345be70a9f`,
`72cf44a63ba76996ec5950307c6d79cbac4eeb917389399cdece903bc96f028b`,
`e4a96c0de4f8bda904c8c84868d3f4c51227526290f88cf8ff26961f9a8df6c3`,
`115c53865f31eb747b22e877e8e41154b0e1276618467c595250cf42d730ac8d`,
`38ad367b90ca8661fef8c0ba91e8dd308ddb8aa9afca2301ed6e7e22e9212fed`,
and
`0df478d04b840824e8f175d0e7fbb2e4a29afecce716f6ca7728163d406b0ea2`.
The non-pass, TSV, and JSONL hashes are
`f8155380318e12c8fcf6fef09db3b7628f8934c761279a066a772f6c675a9400`,
`80beabb219bb0a04830f7c2b40e47549234e20b458bd04e27998df7b64cb335d`,
and
`0375fb338a4fe87345f0406c5ce2ff05cb27c2779d2a7260989521cf44444cf8`.
The pinned Test262 patch, config, and metadata hashes are
`f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3`,
`79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b`,
and
`a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a`.
The independent pinned QuickJS differential fixture/transcript hashes are
`720b53338045bd65c70337c3d43678b52e8c7d3e0ce0b0ef1210f512b7d7a53a`
and
`9b30fc689ebac8bb116d18a87460fb9bd987f5c7b40dfabe508f787c249c10fe`.

The Promise facade remains 9,803 lines in `runtime.rs`; the finally algorithm
lives in the dedicated 203-line
`runtime/intrinsics/promise/finally.rs` module. At the R3o checkpoint, the
remaining explicit Promise frontiers were `Promise.all`,
`Promise.allSettled`, and `Promise.any`.

Reproduce both locks with:

```sh
./scripts/test-r3o-promise-finally-oracle.sh --check
./scripts/test-test262-promise-finally.sh
```

Pass `--oxide target/debug/qjs` to the oracle script for the byte-for-byte
QuickJS/Oxide transcript comparison. Use `--check` on the Test262 command to
authenticate only its frozen manifest/profile and pinned QuickJS result.

## R3p Promise.all

R3p freezes all 98 files directly under the pinned Test262
`built-ins/Promise/all` directory, producing 196 sloppy and strict variants.
The complete cohort contains 57 async paths / 114 variants and 41 synchronous
paths / 82 variants. It has no negative, Proxy, or `$262` host tests. Oxide
passes 196/196 with zero failures, unsupported results, or skips; pinned
QuickJS 2026-06-04 passes all 98/98 paths.

The scoped profile admits exactly `Reflect.construct`, `Symbol`,
`Symbol.iterator`, `Symbol.species`, `arrow-function`, and `class`, plus
`[execution] async=true`. The global profile remains byte-identical and has no
execution section. The manifest and manifest-file SHA-256 values are both
`293639a6d0e3f1937535997a4f61613fd40b2b10267d1d27cc5faa231865c1e5`;
the scoped profile and global profile hashes are
`83b69f80efbe0aa1c1273c646595424d4e3cda01f65ccc1e7400495a6779bb21`
and
`1860224ce1e828406f4869b66b3f1964f96fad85e4eab6ba7fecb256b4b6c2f2`.

The variant-key, async, synchronous, feature, and include inventory hashes are
`be2fbe56f4e095c9ebc5ad7a2dc611ec3ca0fcf3878cac552b9b08c3bb0442c7`,
`291bd0ed5b12d2e857bbbfcae3ff967cdb885d1863c10f1a611ac91f68833bf4`,
`160a6566ad05a90da034c1e0be2bafbbc341dd38743dd271592a83443521b81a`,
`ae2f5435de250ebddbc91135bf5847caa09e5150e199aa79061898380c8d180c`,
and
`0df478d04b840824e8f175d0e7fbb2e4a29afecce716f6ca7728163d406b0ea2`.
Both empty Proxy/host inventories and the empty non-pass vector hash to
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
The canonical TSV and JSONL hashes are
`a71f0e04b81bed11d3760296a40753ed18f0572d25145857b5bcee434f6fa2c9`
and
`3c895f2876be7ceabb12e6e85af5f1bc9d9b1eab2f5cb3a884f5f340d871c22a`.

The independent differential locks descriptor/generic behavior, custom
capabilities, fresh element callbacks and shared reject identity, empty and
out-of-order fulfillment, the synchronous-thenable sentinel, first-call
guards, one-time constructor resolve lookup, pinned IteratorClose boundaries,
thenable and identity behavior, forced-GC capture lifetime, and cross-Context
realm routing. Its fixture and pinned transcript hashes are
`e43406b9de7de5a88034ec5321486d7b352f2c6f43986fddba1b36fe79074835`
and
`efb2fd9cfdd1db42291295e0b313dbf271b0007d30f3823e0377cb7196ab6b54`.

R3p also moves the unchanged R3n inventory from its R3o checkpoint of 216/224
to the current 224/224. The empty non-pass, TSV, and JSONL hashes are
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
`350e8f80d30a1942e44595c1e771b5e0008fd33aa2f93d6d2345e219d5bb6968`,
and
`4058a876e0f05e0ff0b07d6ae6a5b4886ea9dca3ebbe178c758221aa371df6ca`.
The authenticated 214/224 R3n landing result and 216/224 R3o checkpoint above
remain historical records; their inventory identities did not change.

Reproduce both R3p locks with:

```sh
./scripts/test-r3p-promise-all-oracle.sh --oxide target/debug/qjs
./scripts/test-test262-promise-all.sh
```

Use `--check` on either script to authenticate pinned inputs without comparing
an Oxide oracle transcript or executing the Oxide Test262 cohort.

## R3q Promise.allSettled and Promise.any

R3q freezes both remaining aggregate Promise directories from the pinned
Test262 checkout. `Promise.allSettled` contains 104 paths / 208 sloppy and
strict variants: 57 async paths / 114 variants and 47 synchronous paths / 94
variants. `Promise.any` contains 94 paths / 188 variants: 65 async paths / 130
variants and 29 synchronous paths / 58 variants. Neither cohort contains a
negative, Proxy, or `$262` host test.

Oxide passes the complete cohorts at 208/208 and 188/188, with zero failures,
unsupported results, or skips. Pinned QuickJS 2026-06-04 passes all 104/104
`allSettled` paths and all 94/94 `any` paths. The scoped profiles admit only
the metadata features observed in their respective manifests plus
`[execution] async=true`; the global profile remains byte-identical and
fail-closed for async execution.

For `allSettled`, the manifest, scoped-profile, variant-key, empty non-pass,
TSV, and JSONL hashes are:

```text
5ac6c5f7e21194ee432a6480fc8e8b5ae7fff2c3c859aa61da98f7605261eb52
755439ed09621a0460802bfda11ed27983364d572b13baaf93a2e00c5b481947
9b27ccbbdc3e2d8f3eae0f76b783625cc0aefebc52a2802446e21a6f5dbb083c
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
69f7dffcd523a759ea7518708d02a74e56349000c86058574c0dc10bc6313b62
d3173fdd5c6d7d2b6b2523c1e9c05b19b3524a6411d383f529c09877a687cc55
```

For `any`, the corresponding hashes are:

```text
331a3d6f0b19a9353904afa5c5d740f844f97c89fcbc99b58cd11275d3b67eaf
8059eea59f179846a4739ddb280b4d16518286261d1cdb361a2d383474f27826
4f2cd9023246ba0631d27846c942f9e227425717208ef0454da1178c105872a5
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
6b984703c5f155cfd5300314f0f32a98801ad058294aa8b60125f56d478f83a3
856e0679a8425f1a1a403d2577d39547fbeb6053c88dcca4bd9778bf67e6b0f8
```

The combined independent differential fixes QuickJS-specific callback
identity, duplicate-call, property descriptor/order, sentinel, IteratorClose,
forced-GC, and cross-Context realm behavior. Its fixture and pinned transcript
hashes are
`e053bb7944943607b9a29ef15fd34d44a58c44792afaf5193e6b757f4231d8c4`
and
`992d7e26fa681747b67c49a6cfd296c22ae54a558f1d8a86d70ce9eeea3a71e9`.

Reproduce R3q with:

```sh
./scripts/test-r3q-promise-aggregates-oracle.sh --oxide target/debug/qjs
./scripts/test-test262-promise-all-settled.sh
./scripts/test-test262-promise-any.sh
```

Use `--check` on any script to authenticate its frozen inputs without running
the Oxide differential or Test262 cohort.

## R3r generator destructuring return unwind

R3r fixes the only engine faults in the current complete vector. A generator
return injected at a `yield` inside an active array binding or assignment
pattern now follows QuickJS's transient `BlockEnv.has_iterator` path: nested
destructuring iterators close from inner to outer before the return completes,
and an enclosing `finally` runs afterward. The compiler models that region
explicitly; the bytecode verifier remains strict.

QuickJS precompiles a for-of head's assignment fragment before marking the
outer loop iterator in its parser control stack. Its observable behavior is
therefore unusual but pinned: returning from a yield in that fragment closes
an inner destructuring iterator but abandons the outer loop iterator without
calling its `return` method. Oxide reproduces that behavior with the typed
`IteratorDropPreserve` opcode. If the inner close throws, the drop is not
reached and ordinary exception unwinding closes the still-active outer
iterator with the original throw pending, preserving QuickJS's exception
priority.

The independent differential covers binding, assignment, nested patterns,
`finally`, `yield*`, the for-of-head abandon path, and the inner-close throw
path. Its fixture and pinned transcript SHA-256 values are
`05d8e677e984df2a9accb0c56ddb6f2e06ba6d3b2d2d08a51d4ba48811463398`
and
`4e39206df0f8213845227839ad1986759f12566e570a4820265a40e239add715`.

The complete 102,037-key join has no missing or extra key and no previous-pass
regression. Relative to the last checked full baseline it adds 630 passes:
371 `fail-runtime -> pass`, 166 `fail-parse -> pass`, 81
`unsupported-parser -> pass`, and 12 `unsupported-runtime -> pass`; the
remaining changed outcomes are 14 `unsupported-harness-parser ->
fail-runtime`, 11 `fail-parse -> fail-runtime`, and six
`unsupported-parser -> fail-runtime`. Relative to the immediately preceding
R3q implementation rerun, the only changes are both variants of
`staging/sm/expressions/destructuring-array-default-yield.js` moving from
`engine-fault` to `pass`. The complete vector reaches 36,923 passes with no
engine fault; its TSV and JSONL hashes are
`87b1adf3234e6625dd95c96c11357e347447438d412b4007ec2236cb0fd18c7c`
and
`90726c1feee169bf923c857101d73c4f95ffc002de378dfe1f637451ce4fa906`.

R3r also refreshes the flat array-assignment gate's report hashes. All 131
variants still pass; only two successful strict parse-negative detail fields
changed when R3k introduced QuickJS's dedicated `unexpected 'yield' keyword`
diagnostic. A detached R3q checkout produced byte-identical reports to R3r,
proving this bookkeeping refresh is not caused by the unwind fix.

Reproduce R3r with:

```sh
./scripts/test-r3r-generator-destructuring-return-oracle.sh --oxide target/debug/qjs
./scripts/test-test262-array-binding-flat.sh
./scripts/test-test262-array-binding-nested.sh
./scripts/test-test262-array-assignment-flat.sh
./scripts/test-test262-full.sh
```

## R3s complete non-v RegExp built-ins

R3s publishes QuickJS's strict static `RegExp.escape` and completes Annex B
legacy control escapes. The static gate derives all 1,879 pinned
`test/built-ins/RegExp` paths, then excludes a 205-path union: 182
`regexp-v-flag` paths (one also uses `createRealm`), 12 source-audited literal
`/v` paths missing that metadata tag, and 12 `createRealm` paths. The remaining
1,674 paths expand to 3,346 variants; Oxide and pinned QuickJS both pass
3,346/3,346.

The manifest and scoped profile SHA-256 values are
`db6201093f57412de0d0cf16d4ff06f74512af3bc76d6f83c337474c7b982ab3`
and
`0214f6789a3276c4755fadde19477b70620184a6137d29eefef0975cfb379c15`.
The variant-key, TSV, and JSONL hashes are
`98daa9a51c3c4a3067ce293351a4ac9c4cdf0530f67d5bc6ea193c3eb5cbcb26`,
`c2bf334ddcc255048c778095db5bc85e7bacde63ec66049feead47478e66742d`,
and
`9a3ec4c6e5d2c894d22c9e930a74c793dcbf5a691d5e85da34aa024585fac8d0`.
That scoped admission does not widen the global profile.

The complete 102,037-key join has no missing, extra, or duplicate key and no
previous-pass regression. Exactly two `unsupported-parser -> pass` and two
`unsupported-runtime -> pass` transitions raise the total to 36,927. Eight
same-outcome rows proceed past legacy `\c` classification to the existing
ill-formed UTF-16 eval-source frontier. The full TSV/JSONL hashes are
`8f6401e033c8a58d0886ee6453015ca5f289022b90f3f32471e43f7022b2307b`
and
`80055a2278a54aa97f5d0dc8e07bcaefa641cc15ef26ddcc53f35f4095d704e5`.

The independent fixture exercises every `RegExp.escape` classifier category,
strict input behavior, property order and descriptors, lone and paired
surrogates, long ropes, and Annex B `\c` rollback/class behavior. Its fixture
and pinned transcript hashes are
`babb9f0e94a7f4e3cf62ad25faf923dc86adb9248db36f081b4b2e7667c6f784`
and
`c6226637ca00cfcef2c436cb64442d8264ba18553aba31baffe70a34d48f480f`.

Reproduce R3s with:

```sh
./scripts/test-r3s-regexp-escape-control-oracle.sh --oxide target/debug/qjs
./scripts/test-test262-regexp-builtins.sh
./scripts/test-test262-regexp-core.sh
./scripts/test-test262-full.sh
```

## R3t synchronous generators + destructuring binding

R3t derives its boundary from pinned metadata rather than current Oxide
outcomes. A path enters the raw universe when it carries `generators` or
`destructuring-binding` and all of its remaining feature tags belong to the
exact 11-tag scoped profile. That yields 3,418 paths and 6,624 variants.
Removing 25 module paths/variants leaves 3,393 paths and 6,599 variants. Three
source-audited paths contain async-callable grammar not represented by their
non-exhaustive feature metadata; excluding their six variants freezes the
synchronous gate at 3,390 paths and 6,593 variants. The Test262 `async` flag
governs `$DONE` completion and would be incorrect for these tests.

The final inventory contains 3,011 positive paths/5,906 variants and 379
parse-negative paths/687 variants. Its mode split is 3,313 sloppy and 3,280
strict variants. Oxide and pinned QuickJS both pass all 6,593 variants, with an
empty non-pass vector.

The semantic fixes follow the pinned QuickJS implementation:

- mapped `arguments` shares frame VarRefs only through
  `min(actual_count, formal_count)`; extra actual arguments receive detached
  VarRefs (`quickjs.c:16228-16275`);
- generator `.caller` and `.arguments` keep the poison accessors used by
  non-ordinary functions (`quickjs.c:16110-16117`, `17388-17434`, and
  `36513-36516`);
- contextual `yield` is accepted only as the name of a sloppy ordinary
  FunctionExpression (`quickjs.c:36430-36444`);
- scoped generator declarations use the lexical/Annex B duplicate-declaration
  distinction from `quickjs.c:24186-24223` and `36487-36493`; active `yield`
  and the associated `for-in` negatives now produce genuine `SyntaxError`
  results instead of generic unsupported-parser classifications.

The scoped profile, manifest, variant-key, TSV, and JSONL SHA-256 values are
`8057ef347c07ffc80a66c5c83ff73873148a8813af49bcca1ced9863cfb9ac9e`,
`07ad2748c65763366ebdcb8c01893a13aa4fbbcca3e900a31042fc670593f3c5`,
`f5e729f4b439733ee900ce1d7d98163b9969aab6998b4a288cb4a6eea5c35f81`,
`f81c2f7b946360f44c1b2d5bdc40782d2e13f989af372329fb6582cb8ded8978`,
and
`eb1d82ad4d156880bc539d2bfc73e8203cd9dd8f70289e80560388ea07c11083`.
The complete derivation and remaining inventory hashes are frozen in
`tests/test262-generator-destructuring-baseline.txt`.

This is a checksum-bound scoped admission, not a global profile migration.
The global profile remains byte-identical and fail-closed for `generators` and
`destructuring-binding`, so the 6,593 scoped passes are not claimed as a global
uplift. One untagged Annex B generator-declaration test does move from
`fail-runtime` to `pass`. A second untagged staging test moves from
`fail-parse` to the deeper `fail-runtime` expected from the pinned QuickJS
behavior: QuickJS itself rejects that old SpiderMonkey assertion after
accepting contextual `TOK_YIELD`. The exact 102,037-key join therefore has one
new pass, no previous-pass regression, 97 parse failures, and 1,284 runtime
failures. Its score is 36,928/102,037 and its TSV/JSONL hashes are
`6b2fb9219bad5f25bfcebc297ce9373798cd210140ebab0566a18e8dd83d052b`
and
`d2cf352f98f7d12b1ff734d7ff001c443c896be3c8adddd54951dd0a47f78eb2`.
A later, separate admission milestone can classify the three async
adjacencies and refresh every globally profile-bound baseline without mixing
that bookkeeping into the semantic implementation commit.

Reproduce R3t with:

```sh
./scripts/test-test262-generator-destructuring.sh
./scripts/test-test262-full.sh
```

Use `./scripts/test-test262-generator-destructuring.sh --check` to authenticate
the static inventory, scoped profile, QuickJS oracle, and six-row Oxide async
admission guard without running the 6,593-row Oxide synchronous gate.

## R3u global generator/destructuring admission

R3u changes the conservative scoreboard, not the engine semantics already
authenticated by R3t. The global profile becomes the bytewise-sorted union of
the previous global profile and R3t's reviewed capability set: 73 feature tags
and 802 exact audited negative paths. Its SHA-256 is
`d01f4f49fbd14b2cad610983624142b468587b2e0bd10ae6264641c39cffa05f`;
it still has no `[execution]` section.

Test262 feature metadata is non-exhaustive for three paths in this cohort.
Their source contains async function or async-arrow grammar even though the
tests complete synchronously and correctly omit the `$DONE`-oriented `async`
flag. A final, feature-scoped source guard therefore keeps all six variants
`unsupported-async`. Config exclusions, declared module/async modes, host
requirements, feature gaps, and negative provenance retain priority. The
general `$262` host-hook scan stays independently conservative and never skips
possible RegExp text, so the scoped lexical audit cannot hide a host
requirement.

The exact R3t/R3u join contains the same 102,037 unique `(path, variant)` keys
in the same order. Its only outcome transitions are:

- 6,593 `unsupported-feature -> pass`;
- six `unsupported-feature -> unsupported-async`.

Another 6,583 `unsupported-feature` rows keep the same outcome and change only
their detail after the newly admitted tags are removed from the remaining
feature-gap list. The complete row-level diff is therefore 13,182 rows.

There are no previous-pass regressions, missing or extra keys, duplicates,
crashes, or runner/engine infrastructure faults. Passes rise from 36,928 to
43,521 and runnable variants from 38,483 to 45,076. `unsupported-feature`
falls from 30,523 to 23,924 while `unsupported-async` rises from 9,986 to
9,992; every other outcome count is unchanged. The final TSV/JSONL SHA-256
values are
`202ab3480b39a6c7a68443bf9faba7bf9eb139b7c15baf2fde25c55c40c5d023`
and
`25df14d037d181bc82b70855a44e782cfbff3118603666dca6ec908cfd659387`.

The migration also re-pins profile metadata for 31 direct-global focused
vectors and 14 scoped vectors. Their test rows do not intersect this admission
and the admission itself does not change their outcomes. Re-running the two
class-generator gates exposed stale R3k/R3l report hashes: R3t had refined ten
public and eight private passing `yield` diagnostic rows without refreshing
those older vectors. Their current hashes now record those detail-only changes.
The provenance canary does change semantically: four destructuring variants
become intended parse-negative passes, moving it to eight pass and eleven
fail-closed variants.

Reproduce R3u with:

```sh
./scripts/test-test262-generator-destructuring.sh
./scripts/test-test262-provenance.sh
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3v synchronous Iterator helpers

R3v ports the QuickJS 2026-06-04 synchronous `Iterator` intrinsic and the core
Iterator Helpers algorithms. The realm-local graph includes the abstract
constructor, `Iterator.from`, `%Iterator.prototype%`, lazy `drop`, `filter`,
`flatMap`, `map`, and `take`, and eager `every`, `find`, `forEach`, `reduce`,
`some`, and `toArray`. Dedicated heap payloads preserve lazy helper and wrapper
state, GC edges, completion, reentry, dynamic `return` lookup, and iterator
close ordering. The implementation also follows QuickJS's low-64-bit
`JS_ToInt64Free` behavior for huge finite limits, primitive String fallback,
cross-realm native-constructor identity, and nested `flatMap` close-error
priority.

The pinned Test262 metadata names `iterator-helpers` on 567 paths. At R3v, the
dependency audit removed 25 paths that executed `Proxy` directly, three whose
included harness files executed `Proxy`, 11 that required
`$262.createRealm`, four that required `$262.IsHTMLDDA`, and one excluded by
the pinned QuickJS configuration. That exact union contained 44 paths, leaving
523 paths and 1,046 sloppy/strict variants. Oxide and pinned QuickJS both
passed 1,046/1,046 with no failure, unsupported result, skip, duplicate key, or
engine fault.

The historical R3v manifest path-stream and complete-file SHA-256 values were
`9d01f0a6846feac8b6c9b555d95fd1eb4942262f51d4602e4c395f5f45b76443`
and
`ce8dd5bfebd79924090ff4a628607009d11ff116ffeb38720808b585335a91e5`.
The scoped profile and `(path, variant)` key hashes were
`a6ce2d6be97d7826cf20aeba7ab8946ad28ce134b0ad7165a8e591a986e6d22e`
and
`43be68340124e844c5e456899a084460ad87edd2c279c3ac1ca4057726b3697a`.
The canonical focused TSV/JSONL hashes were
`4746567453ed198096fd270e70f7c2c51975de837df0a1181645ceffd3cdefc9`
and
`a25b115582160d38acb534c0192f93db65f3c8473d3c9211adb39c8f40a1a02a`.

At R3v this was a checksum-bound scoped admission and the global capability
profile stayed fail-closed for `iterator-helpers`; the complete vector
therefore remained at 43,521/102,037 passes and byte-identical to R3u. Those
numbers are historical. The focused script reproduces the historical R3bm
receipt below. `Iterator.concat` belongs to the separate R3w
`iterator-sequencing` cohort.

The implementation differential remains:

```sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_iterator_helpers -- --nocapture
```

The focused script now reproduces R3bm below rather than this historical R3v
receipt.

## R3bl Iterator Helper optional-adjacency refresh

At R3bl, the gate re-derived the complete 567-path `iterator-helpers` metadata
population and its raw 44-path dependency union. It promoted exactly the 14
source-Proxy paths frozen by the optional-chaining adjacency ledger and left
30 paths deferred: 11 other source-Proxy paths, three harness-Proxy paths, 11
`$262.createRealm` paths, four `$262.IsHTMLDDA` paths, and one path excluded by
the pinned QuickJS configuration. The selected manifest therefore grew from
R3v's historical 523 paths / 1,046 variants to 537 paths / 1,074 variants.

Pinned QuickJS passed all 537 selected paths in sloppy mode and all 537 in
strict mode. Oxide passed all 1,074 variants with no failure, unsupported,
skipped, timeout, crash, or infrastructure outcome. The scoped profile
retained exactly 76 feature tags and 802 audited negative paths; its
complete-file SHA-256 was
`a0ed7fa1a5cd46c5c47895d671c0078434635ae41f0a420e66573dcb86d18a7f`.
The manifest path-stream, complete-file, and variant-key SHA-256 values were
`563b7040eb391512a5118d7102f2a58e0fe88629c9069b1019bfb9bc4ed07e75`,
`09bbd1dd78d226ab2cdd9131072647aaaa87c4f98859f73db29559f924439da8`,
and
`b4c06bdd75fe4ef062b04eefe2b5e21ccbf4bc5130afc7c5ba24b9e9295364fa`.
The canonical focused TSV/JSONL hashes were
`136c5e8b9520a1b61ee3981a32ec8009e0e023d8b2737debe76157f2f7615b59`
and
`b8635ff35f5699cffd698f4d569960d505422b1b723e27661e13d88aece4a87c`.

The helper profile authenticated the immutable synchronous
`iterator-sequencing` profile as its historical parent rather than comparing
with the growing live global profile. The sequencing gate itself remained 32
paths / 64 variants, all passing in both engines, and its script likewise
treated the R3al global hash only as provenance. Later global feature growth
therefore could not drift either focused receipt.

R3bl changed no runtime semantics and did not admit `iterator-helpers` into
the global capability profile. The complete vector remained at R3bj's
56,526/102,037 passes and 57,045 runnable variants, with canonical full
TSV/JSONL SHA-256 values
`84c15d4a25343e1d306e17f431e515993abe09db76590920539eefe93d6fb3eb`
and
`96ebd4a8f51001b403e88d19c128bebb92b74bb9abf1e45c832b187924c635fd`.

R3bl's receipt remains recorded as historical evidence. The focused script now
reproduces R3bm below.

## R3bn global Iterator Helpers admission

R3bn adds exactly `iterator-helpers` to R3bj's immutable 82-tag feature
inventory and retains the same 828 audited negative paths and one async
execution entry. The resulting 83-tag profile has SHA-256
`8a3b253f6d2a24b18f9bec66628ba5aec3fb337d677c60bfde37c4c3a33d3910`;
the feature section alone has SHA-256
`4f33e9167adc040023ef9df3d5e8995b22877da83e327d7162632a1d4fc13198`.
No negative path is present in the tagged Iterator Helper population.

The exhaustive metadata population is 567 paths / 1,134 sloppy-strict
variants. Its path-stream, complete-manifest, and variant-key SHA-256 values
are
`70eca80ab1c3e1f45dfc4383ec08a9bcf0c0ef1d656fa345c356c4d9610f795c`,
`c4700fe6efcfa05d4e00c3d7cfc9d4a4aa062db7ac58cd8318a51bf41c1bbcf4`,
and
`b8794b55c01b6e185bcb8a15275bef51aaf8eda8fe274eb8eda9824748fcaa02`.
The partition is disjoint and exhaustive:

- 538 paths / 1,076 variants activate. Their path-stream, complete-manifest,
  and variant-key hashes are
  `d3190f82ceaa1a3f16b32ac824b26a4fe1c689aa6ede51a43188d861899441d2`,
  `4bbb1d7788c177bbfb924ffaddccec39084eaf482de9a4b6ea120a7a079aac5d`,
  and
  `de1acf288cbce9fed7ea4f5dfc81ce53c4795b5122a3e9e485e5b27356d68bfb`.
- 13 paths / 26 variants are reason-only rows that also require `globalThis`.
  Their corresponding hashes are
  `0cd2175342155365d92fa29bedf5bd12193e4ed8c95426f1ad2f80a40cc9825b`,
  `23a26e5f007ee4fac04486ad9c816d01e78802fd1c0974ca154aec3cd2ee2447`,
  and
  `cec177031ee2f3dab6596e0ab337ec411a42bf7551a57120e5a770c0fe11b56e`.
- 16 paths / 32 variants remain behind host/config requirements: 11
  `$262.createRealm`, four `$262.IsHTMLDDA`, and one pinned-config exclusion.
  Their corresponding hashes are
  `de0e1e40da2120f5149aa07a8a3f62588e61761aadf659b26c2a80706b2086c2`,
  `1f346639b0b941fec7b6411c63057ab01a096196be3ac47906b1e2c4a643c49c`,
  and
  `5f9105c90732493741b8b652f0a5ad74f775740706d847171c96617fdd23b760`.

The candidate tagged run passes every activated variant: 1,076 pass, 26
remain `unsupported-feature` behind `globalThis`, 22 remain behind
`$262.createRealm`, eight remain behind `$262.IsHTMLDDA`, and the two variants
of the excluded path remain skipped. Its TSV/JSONL SHA-256 values are
`21e6b0be9aa662c485176690d5665bc6f79687fc7bf4ae4ddf6335ee419a8f5d`
and
`d62917441f6a7c6a5163316d1c09ccf4540ff54b4a6599a58285d0f34f01a66b`.
The 1,134-row transition receipt has no missing or duplicate key; its receipt
and data SHA-256 values are
`97d980227d3f9913d6fedb6e97deec7ae0b1db3df3fefe01b74d96320c775d4f`
and
`84cdc0c565ddff0257ff1aff6c29e0297f8d3c3afca72015cfd17d6192e5b108`.

The exact complete-vector join retains all 102,037 keys. Exactly 1,076 rows
move from `unsupported-feature` to `pass`; 26 rows remain
`unsupported-feature` with only `iterator-helpers` removed from their
diagnostic detail; the 32 host/config rows and all 100,903 non-Iterator-Helper
rows are unchanged. No previous pass regresses. The canonical vector reaches
57,602/102,037 passes with 58,121 runnable variants and 20,523
`unsupported-feature` outcomes. That is 56.45% raw, a 68.93% lower bound
after the 18,475 pinned QuickJS target exclusions, and 99.19% among 58,072
variants with a non-unsupported observed outcome. Its full TSV/JSONL SHA-256
values are
`7b5bb9d188473f7f7298e131da405f7e77e66c6eddbf10d14949722bf275c6fc`
and
`869d9150a532a72c02e37eae9d1d3ead2c88c8384be23e5222efe055e99a18a2`.
An independent canonical two-worker repeat is byte-identical.

R3bn changes only the global capability profile and evidence. It adds no
runtime semantics and is not a Feature Parity completion claim. The 13
`globalThis` paths and 16 host/config paths remain deliberately fail-closed.

## R3bm historical Iterator Helper Proxy-closure refresh

At R3bm, the gate retained the exhaustive 567-path `iterator-helpers`
inventory and its raw 44-path dependency union. It froze the 25 source-Proxy
paths and three harness-Proxy paths as one exhaustive 28-path closure. R3bl
had already promoted the exact 14-path optional-chaining adjacency; R3bm
promoted the remaining 11 source-Proxy and three harness-Proxy paths. The
selected manifest therefore grew to 551 paths / 1,102 sloppy-strict variants.

Pinned QuickJS passed all 551 selected paths in sloppy mode and all 551 in
strict mode. Oxide passed all 1,102 variants with no failure, unsupported,
skipped, timeout, crash, or infrastructure outcome. Independent 8/8/5-worker
Oxide reports were byte-identical. The scoped profile remained unchanged at
76 feature tags and 802 audited negative paths; its complete-file SHA-256 was
`a0ed7fa1a5cd46c5c47895d671c0078434635ae41f0a420e66573dcb86d18a7f`.

The manifest path-stream, complete-file, and `(path, variant)` key SHA-256
values were
`32b3a539828fe72e32cb28bed6b6942749ac1aa6402a04bb809126da0a2cea4c`,
`6db8a38003ba95245dde0e0559b64a75c1a0215e610408811174f482363b729c`,
and
`cc432f145a9f12ad959f0b856c5b91c73a1b9ce0ebb3fd0c9cc5a18ac0f2f841`.
The canonical focused TSV/JSONL SHA-256 values were
`47b725903172118e8fbde4ba8f6d87343d44fa280889630e1ee5d620634154e5`
and
`9c55978a8b8200be94617eb5c80ea97abac7172b93599fdd31769df6a7679d08`.

The remaining deferred ledger was exactly 16 paths / 32 variants: 11
`$262.createRealm` paths, four `$262.IsHTMLDDA` paths, and one pinned
QuickJS-config exclusion. Its variant-key SHA-256 was
`5f9105c90732493741b8b652f0a5ad74f775740706d847171c96617fdd23b760`.
The full Proxy closure and this host/config ledger were disjoint and exhaustive
within the original raw dependency union.

R3bm changed no runtime semantics and did not admit `iterator-helpers` into
the global capability profile. Its complete vector therefore remained at
R3bj's 56,526/102,037 passes and 57,045 runnable variants, with canonical full
TSV/JSONL SHA-256 values
`84c15d4a25343e1d306e17f431e515993abe09db76590920539eefe93d6fb3eb`
and
`96ebd4a8f51001b403e88d19c128bebb92b74bb9abf1e45c832b187924c635fd`.
The independently authenticated Iterator sequencing gate remained 64/64 in
both engines.

The historical focused gates remain reproducible with:

```sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_iterator_helpers -- --nocapture
./scripts/test-test262-iterator-helpers.sh
./scripts/test-test262-iterator-sequencing.sh
```

## R3w Iterator.concat sequencing

R3w ports pinned QuickJS's independent `JS_CLASS_ITERATOR_CONCAT` payload,
hidden prototype, `Iterator.concat`, `next`, and `return` algorithms. It keeps
the upstream eager/lazy boundary: input objects and their `@@iterator` methods
are validated and captured left-to-right at construction, while each iterator
and cached `next` method is created only when reached. Abrupt open, step,
`done`, or `value` operations retain retryable state and never close the inner
iterator. A throwing `return` getter also preserves state; once that getter
succeeds, the call result or error is forwarded exactly and all remaining
state is drained.

The pinned Test262 feature inventory is exact and unusually clean: all 32
`iterator-sequencing` paths live under `test/built-ins/Iterator/concat`, every
path has both ordinary variants, and none uses Proxy, a host hook, a negative
phase, a special flag, or a pinned-config exclusion. Oxide and QuickJS
2026-06-04 both pass all 64 sloppy/strict variants with zero failure,
unsupported result, skip, duplicate key, timeout, crash, or engine fault.

The manifest path-stream and complete-file hashes are
`4a2613c71099c481cf16a9ca087c2c6db92112341008f785ce1363cd6794e18d`
and
`74eebb8c63a2606e54e1d0023c5244b8a0538ac51d1ca0a105fe56a04fa74af2`.
The scoped profile and `(path, variant)` key hashes are
`8284db009a398fb88b2d357d7d8255479943d963574392f7b718610ee12cb16a`
and
`eab38e1c6d7f22397e7c8521ec934476b2472406db5d83cfea23d0fbe7b17d5b`.
The canonical TSV/JSONL hashes are
`716d98068f7f2b28ff142abca546e71ff7eee9224bad1cea52ac0830240b8560`
and
`a184e7e80444282cc23015c5846052430c593eab93da358d4679859422f2e029`;
the non-pass stream is empty.

R3bl later decouples this historical gate from the growing live global
profile. The script still authenticates the immutable 74-feature,
802-negative scoped profile and the unchanged 64/64 result.

Test262 has no cross-realm rows here and only shallow coverage of getter
caching, retry state, return error priority, and reentry. The separate pinned
QuickJS differential and Rust heap/runtime tests lock those boundaries. A
pinned same-runtime libquickjs C probe plus the two-context Rust regression
lock the `JS_IteratorNext2` native fast path retaining the outer operation's
current realm. R3w originally kept this as a scoped admission; R3x performs
the separately audited global promotion below.

Reproduce R3w with:

```sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_iterator_concat -- --nocapture
./scripts/test-test262-iterator-sequencing.sh
./scripts/test-test262-full.sh
```

## R3x global Iterator sequencing admission

R3x changes only the conservative scoreboard. The global profile now contains
74 bytewise-sorted feature tags, including `iterator-sequencing`, plus the
unchanged 802 audited negative paths; it still has no execution opt-in. Its
SHA-256 is
`6a4d3dc37da05f6e63d7b8564483159c383ed66c665a2b5530624e628f73b908`.

The 32-path R3w inventory is the tag's complete pinned Test262 surface. Joining
the before/after full reports preserves all 102,037 keys and changes exactly
64 variants from `unsupported-feature` to `pass`. No prior pass regresses and
no failure, skip, timeout, host classification, or report detail outside those
keys changes. Passes rise from 43,521 to 43,585, runnable variants from 45,076
to 45,140, and `unsupported-feature` falls from 23,924 to 23,860.

The final TSV/JSONL SHA-256 values are
`0f43b6e164c0954a02f911774c34871ea67e6255f28ffa65419ea15d3f4b73fd`
and
`f24e92ad54c4c59651206db66bfd7a4ed9dea4f3543311a990def0fc16e66be8`.
Thirty direct-global vectors, 14 unrelated scoped vectors, smoke/provenance
canaries, and two Iterator gates are re-pinned only for authenticated profile
metadata; their outcomes remain unchanged. Re-running the tagged-template gate
also refreshes its two stale PrivateName staging rows, which later private-name
work had already moved from `unsupported-runtime` to `pass`.

Reproduce R3x with:

```sh
./scripts/test-test262-iterator-helpers.sh
./scripts/test-test262-iterator-sequencing.sh
./scripts/test-test262-smoke.sh
./scripts/test-test262-provenance.sh
TEST262_WORKERS=2 ./scripts/test-test262-full.sh
```

## R3y synchronous class matrix closure

R3y authenticates the already-implemented synchronous class surface as one
generated-matrix closure; it does not add engine semantics or widen the global
profile. The metadata-only inventory starts from the `dstr` and `elements`
subtrees under class expressions and declarations. Requiring every feature to
fit the exact 19-feature scoped profile, and at least one of its ten
class/default/rest additions, derives 3,890 paths / 7,763 variants.

A frontmatter-stripped source audit exposes 14 dependencies missing from that
metadata: eight async private method-name paths / 16 variants and six
Proxy-dependent paths / 12 variants. Optional chaining contributes no hidden
path. Pinned QuickJS passes the full 7,763-variant upper closure; Oxide passes
the remaining clean 3,876 paths / 7,735 variants while the 28 adjacencies stay
assigned to the separate async and Proxy frontiers.

The clean gate contains 3,196 positive paths / 6,383 variants and 680 exact
`parse` / `SyntaxError` paths / 1,352 variants. Its 3,867 sloppy plus 3,868
strict variants pass in both Oxide and QuickJS 2026-06-04. The manifest,
scoped-profile, variant-key, TSV, and JSONL SHA-256 values are
`40f038bdc52c762baf7f16ea885c98fc3d0afd033e56059717e8627086e14c78`,
`de71fc1d3c675ed25dc54d43222a10c4f3d607c14cb4d43628d7a4587827a7ef`,
`1095d6e01eb78c11ed9ff23f195ac909cd99381cb646973095b7cac9ad4676bc`,
`61e9a260c91e886bd65b2b148564ce861324b8a5b5343f85688d603bd3217b1e`,
and
`a258e37e13d99f3491e79db321172f3202800b526f8059ef5c8f3b1a77d9fee2`.

The conservative full vector therefore remains 43,585/102,037. Broad `class`,
computed-name, default-parameter, and object-rest tags remain scoped because
global admission would also expose unreviewed async and Proxy combinations.

Reproduce R3y with:

```sh
./scripts/test-test262-class-sync-matrix.sh
```

## R3z ordinary async function core

R3z opened the first checksum-bound async execution profile without promoting
the broad `async-functions` feature or `[execution] async=true` into the global
scoreboard. The static universe was the 207 pinned paths directly under
`built-ins/AsyncFunction` or recursively within the expression/declaration
`async-function` and expression `await` trees. At the R3z landing, a checked
exclusion ledger assigned 65 paths to five independent frontiers: 40
complex-parameter cases, 11 eval/with adjacencies, ten async-arrow cases, two
async-generator/for-await cases, and two host/cross-realm cases.

The clean cohort contained 142 paths / 259 variants. It had 95 positive paths,
47 audited parse/SyntaxError paths, 65 async-harness paths, and 77 synchronous
paths; 117 paths run in both modes, 17 are `noStrict`, and eight are
`onlyStrict`. Pinned QuickJS 2026-06-04 passed all 142 paths. The canonical
Oxide report had 259/259 passes with no unsupported, skipped, failed, timed
out, crashed, or infrastructure outcomes.

The manifest, scoped profile, variant keys, canonical TSV, and JSONL SHA-256
values were
`fdd1679242195cb32508b7976a1b0b3508fe96a2e77483808d3bf5c9c554ff52`,
`05634144cdc2e64874ffda721b429181ac8b7a8f82b1ba253f2b8d8a29a4332e`,
`a5249ce3625e80f41ea2464e00fcf19804913d49556e680ad6624fd6bf71d391`,
`d0d3933d5cc4114b60a55bd6040d4350cba890b7d8a29a4e41e372eb4291cfaa`,
and
`9259b27b167856e5e3a2428530d1943d74fc967a659759568b5068ce2a74c4c3`.
The profile admitted only four exact feature tags, the 47 frozen negative
paths, and the async host. The global profile remained fail-closed because opening it
would expose thousands of unrelated async/module/host combinations.
These counts and hashes preserve the historical R3z landing snapshot.

The complete R3y/R3z join retains all 102,037 keys and every previous pass.
Fifty-four parse failures and four runtime failures become passes; 32 other
variants advance to explicit downstream runtime or typed async
arrow/generator frontiers, and two rows refine only their diagnostic detail.
The conservative score therefore rises from 43,585 to 43,643. Full TSV/JSONL
SHA-256 values are
`8d47c7d70de9d1049cded9b4fe4aec3459313e374421ab99e1c36eb5730531f6`
and
`14295f172893540d703e02aa4c9ba3e5bdee02d866131479680b5c33b2ddfabd`.

Reproduce the R3z runtime oracle with:

```sh
cargo build --bin qjs
./scripts/test-r3z-async-function-core-oracle.sh --oxide ./target/debug/qjs
```

## R3aa ordinary async gate expansion

R3aa changed only the authenticated scoped evidence. It admitted all 40
complex-parameter paths and nine of the 11 eval/with adjacencies from the
original ledger. The two remaining eval/with paths contained async arrows, so
the R3aa 16 exclusions consisted of ten async-arrow paths, those two
async-arrow-dependent eval paths, two async-generator/for-await paths, and two
host/cross-realm paths.

At the R3aa landing, the clean cohort contained 191 paths / 348 variants: 126
positive and 65 audited parse/SyntaxError paths, 96 async-harness and 95
synchronous paths. There were 157 double-mode paths, 26 `noStrict` paths, and
eight `onlyStrict` paths, producing 183 sloppy and 165 strict variants. Pinned
QuickJS 2026-06-04 passed all 191 paths; the canonical Oxide report passed
348/348 with no unsupported, skipped, failed, timed-out, crashed, or
infrastructure outcomes.

The R3aa scoped profile authenticated eight exact feature tags:
`Symbol.toStringTag`, `Symbol.unscopables`, `arrow-function`,
`async-functions`, `default-parameters`, `generators`, `globalThis`, and
`rest-parameters`. It also froze all 65 negative paths and opted into only the
async host. The global profile remained fail-closed.

The R3aa exclusion-ledger, manifest, scoped-profile, variant-key, canonical TSV,
and JSONL SHA-256 values are
`7c29c59cc107d74da4a5fcfba4571947195003a2f551bb82f9fc2dd8b3fb42ac`,
`a0fa7acd444257ca7cbfffc40c61eb3b85867c81df04f1d1691100a72c97b0dc`,
`7fb94b8e350b5a270ab5f685f0a223e32c7d12fedf0ac3e0c1e157b03f4f0b33`,
`25e87df8047ce67fb30a570f9e211540b689dc00c9a4b7e29de20b528f77a077`,
`ba690597d3ca1d9f6604106b0d54d37a7d1215b4a832c0a72a4ccdde8c28e913`,
and
`fe4be77b96c8af7b8bda137d8377818ab04450f340beaa2e172f290eadcb264f`.
This expansion adds no global keys or passes: the conservative
43,643/102,037 vector and its R3z full-vector hashes remain unchanged.
Those values preserve the historical R3aa landing; the checked-in
ordinary-async gate now reproduces the R3ab refresh described below.

## R3ab async-arrow core

R3ab ports async arrows by preserving the QuickJS split between syntactic
identity and execution kind. The compiler publishes `FunctionKind::Arrow` for
arrow grammar and `BytecodeFunctionKind::Async` for execution, so the existing
Promise/continuation driver handles parameter rejection, return assimilation,
and `await` without turning the function into an ordinary-function grammar
node. Async arrows remain non-constructible, have no own `prototype`, expose
the AsyncFunction brand and authored source, and retain inferred name/length.
Their lexical `this`, `arguments`, `new.target`, and `super` bindings survive
across suspension.

The parser also matches a token-timing quirk in QuickJS 2026-06-04: the first
token after `async` is committed in the parent lexical context, while later
parameter tokens use the async-arrow child's context. Every nested arrow
creates a new formal-parameter boundary: future `await`/`yield` classification
is recomputed from that arrow and its immediate parent
execution/static-block role, so an ancestor async/generator/static context
cannot leak through transitively. At top level this admits both
`async await => 1` and the escaped-`await` spelling. Parenthesized
`async (await) => 1`, `await` in a parameter default, and equivalent
single-binding forms whose enclosing async/generator context already
classified `await`/`yield` remain syntax errors in both engines.

The canonical language-tree universe is the complete 60-path pinned
`language/expressions/async-arrow-function` tree. It admits all three
complex-parameter paths, all three eval/with paths, and all five
forbidden-extension paths; no language-tree path is excluded. The tree has 31
positive and 29 audited parse/SyntaxError paths, expanding to 110 variants.
Pinned QuickJS 2026-06-04 passes 60/60 paths, and Oxide passes 110/110
variants.

The frozen focused gate additionally admits
`test/built-ins/Function/prototype/toString/async-arrow-function.js`. Its full
61-path / 112-variant manifest has zero exclusions: 32 positive and 29
negative paths, 27 async-harness and 34 synchronous paths, 51 double-mode,
eight `noStrict`, and two `onlyStrict`, producing 59 sloppy plus 53 strict
variants. Pinned QuickJS passes 61/61 paths. Oxide passes 112/112 with no
unsupported, skipped, failed, timed-out, crashed, or infrastructure outcome.
The stable, pre-existing `with` gate also reaches 205/205 because R3ab closes
its last async-arrow adjacency.

R3ab also refreshes the ordinary-async evidence boundary by returning the ten
direct async-arrow paths and two async-arrow-dependent eval/with paths from the
historical R3aa exclusion ledger. The 207-candidate universe now has four
explicit exclusions and a 203-path manifest. Its 138 positive plus 65 audited
parse/SyntaxError paths expand to 366 variants: 108 async-harness and 95
synchronous paths, 163 double-mode, 29 `noStrict`, and 11 `onlyStrict`, for
192 sloppy plus 174 strict variants. Oxide passes 366/366, and pinned QuickJS
2026-06-04 passes 203/203 paths. There are no remaining complex-parameter,
eval/with, or async-arrow exclusions; the four remaining paths are exactly two
async-generator/for-await and two host/cross-realm dependencies.

The current ordinary-async exclusion-ledger, manifest, scoped-profile,
variant-key, canonical TSV, and JSONL SHA-256 values are
`7e60ccc3b07d5539d3c55958ee8889df3de899525688d346e8d5763d9a1d4f41`,
`97930e30959d8bdbdd1b030e4f4e94fe9657791951f48e58a6790e73a7191390`,
`7fb94b8e350b5a270ab5f685f0a223e32c7d12fedf0ac3e0c1e157b03f4f0b33`,
`109e78ccd538a5ce8376140b50c624a9ccdcb929b8d4819ab25acd9610e8e995`,
`2f22a49938c079c0133f372f1e5b8f757b5aace881385a185c4b775f6186fd39`,
and
`c750dba4c8a45f4cc18c658810774b4919771d89052ed5a8423b92a636922eaf`.

The SpiderMonkey staging path
`test/staging/sm/async-functions/async-contains-unicode-escape.js` expects a
SyntaxError for the single-binding token case accepted by the pinned QuickJS
target. The R3ab gate pins that file's checksum and checks the differential
directly, but deliberately keeps it outside the 61-path focused candidate
universe. It is an audit-only target-quirk canary, not one of the zero
exclusions.

The manifest, empty exclusion ledger, scoped profile, variant-key, canonical
TSV, and JSONL SHA-256 values are
`d4bc4b286b2da1b19949d56b614e1d1af110437285827fa4f4c6cb00dae1d969`,
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
`f6634c6298e3d3fb740c0f55e8932ddc402ca8e120d8f0d2d9326f552186af2c`,
`b3407b31ee0df08990b09aa13b77f7f6ff7028ab0ad4f1eb3c1f083a36a6cd03`,
`9f110385b1695b6eaaabafb0984c091d35e6cc83878e10b1450f1730db2636f1`,
and
`68b5efe50c71eb24a75a53fd72200d883fa966e77813a22014a046b6f09f2f58`.

At the R3ab landing this remained a scoped semantic gate: the global
`async-functions` feature and async host stayed fail-closed pending async
methods and generators. Twelve already-admitted, untagged consumers
nevertheless advanced to pass. The exact R3z/R3ab join retains all 102,037
keys with no missing, extra, duplicate, or previous-pass row. Its 16 outcome
transitions are eight
`unsupported-parser -> pass`, four `unsupported-runtime -> pass`, two
`unsupported-parser -> unsupported-runtime`, and two
`unsupported-runtime -> fail-runtime`. The last pair comes from
`test/staging/sm/async-functions/await-in-arrow-parameters.js`, whose
expectation also fails under pinned QuickJS 2026-06-04. Two additional
`toString` variants keep their unsupported-parser outcome but expose the
downstream async-object-method detail. Runnable remains 45,140 and passes rise
from 43,643 to 43,655.

The R3ab full TSV/JSONL SHA-256 values are
`f9b0827706c24cc97f1792e92aa1d275d7c5c7bd14d3e2b47f16d27dc543c8f0`
and
`9026be710ff432002357a236b4ebd81abc8fd6ea9039e04b3af8944968d83d70`.

Reproduce the R3ab ordinary-async and async-arrow gates with:

```sh
./scripts/test-test262-async-function-core.sh
./scripts/test-test262-async-arrow-core.sh
```

## R3ac ordinary async object methods

R3ac ports ordinary async object-literal methods onto the established async
function machinery rather than adding a runtime fork. The compiler retains
`FunctionKind::Method`, selects `BytecodeFunctionKind::Async`, and publishes
through the existing `DefineMethod`/HomeObject path. `super` therefore keeps
the current receiver in parameter initializers and across `await`; branding,
non-constructibility, inferred name/length, authored source, rejected
parameter completion, and Promise settlement follow the same QuickJS-shaped
paths already used by R3z/R3ab. No runtime change was required.

The frozen candidate universe contains 49 paths / 90 variants. It includes
Function `toString`, complex-parameter, eval, forbidden-extension,
private-name, Proxy, and async-generator adjacencies, and pinned QuickJS
2026-06-04 passes all 49 paths. Six async-generator neighbors and one Proxy
path remain in the explicit exclusion ledger. The admitted manifest therefore
contains 42 paths / 76 variants: 25 positive and 17 audited
parse/SyntaxError paths, 23 async-harness and 19 synchronous paths, 34
double-mode, six `noStrict`, and two `onlyStrict`, producing 40 sloppy and 36
strict variants. Oxide passes 76/76 with no unsupported, skipped, failed,
timed-out, crashed, or infrastructure outcome. At the R3ac landing, broad
async feature/host admission remained fail-closed pending async class methods
and generators.

The scoped-profile, candidate, manifest, variant-key, canonical TSV, and JSONL
SHA-256 values are
`ec8be515bb6f68cb3226f1770b4ac73b66c013d5c27a74bcda974770546b9e9f`,
`535880772cfeff4e3c7cf31d956a80ea70fa67b2fc1fd043825d43eab6c6536a`,
`38b1fd3cc785923d4e98a28b8e8daf19777bf02630634753715abf7160c9d796`,
`dc5b96263b7f54cae137ec6e2c935da0f8186387fa59baf43630b0be7474e5db`,
`511ebd130275568679425664893888b56da1280d93731bbab7c4003dadd2ad64`,
and
`06a5557df83f1ff795d73a89ae60d83547976b372b8a87d1f3098126f4b9dc95`.

The differential oracle also freezes QuickJS's contextual-lookahead quirk.
Direct U+2028/U+2029 trivia after `async` prevents an async method, but the
same code points inside an intervening block comment are ignored by the
pinned target's small lookahead scanner. The ordinary lexer still records
them correctly for ASI and restricted productions. Annex B HTML comments
remain an existing independent parser frontier.

The exact R3ab/R3ac full join retains all 102,037 unique keys and every prior
pass. Two `unsupported-parser -> pass` and two
`unsupported-runtime -> pass` transitions account for the four new passes.
Two `unsupported-parser -> unsupported-runtime` variants now stop at the
typed async-generator frontier. The two
`unsupported-runtime -> fail-runtime` variants are
`test/staging/sm/async-functions/async-contains-unicode-escape.js`; pinned
QuickJS also fails that Test262 expectation, making them recorded target
disagreements rather than semantic regressions. Six more rows retain their
outcome while exposing a deeper async class/generator detail.

The runnable count remains 45,140. Passes rise from 43,655 to 43,659,
`fail-runtime` rises to 1,281, `unsupported-parser` falls to 45, and
`unsupported-runtime` falls to 34. The R3ac full TSV/JSONL SHA-256 values are
`627e6e8dc2aa44e9ef6869db54c3a9059528d33eb7b24c55658db36d84a250b0`
and
`5879cef785efe0a855e3abb74d820dd9bc2274d20fdba9ba8c557641d0fa5dbe`.

Reproduce the focused gate with:

```sh
./scripts/test-test262-async-object-method-core.sh
```

At the R3ac landing, public async class methods were the next recommended
milestone, followed by private async class methods and then async generators.

## R3ad public ordinary async class methods

R3ad ports public ordinary async instance and static class methods through the
same parser/runtime split as the pinned target. QuickJS `js_parse_class`
classifies the property as async, calls its function parser with
`JS_PARSE_FUNC_METHOD` plus `JS_FUNC_ASYNC`, and publishes a fixed or computed
method on the prototype or constructor. Oxide retains
`FunctionKind::Method`, selects `BytecodeFunctionKind::Async`, and reuses the
existing non-enumerable `DefineMethod`/HomeObject path. Strict class bodies,
AsyncFunction branding, non-constructibility, inferred name/length, authored
source, rejected parameter/body completion, and instance/static `super`
across `await` therefore stay on the already-authenticated machinery. The
authored range begins at `async` after an optional `static`; fixed/computed key
spelling and trivia round-trip, while `Function.prototype.toString` excludes
the class element's `static` marker just as it does in QuickJS. The pinned
source anchors are `quickjs.c` 24485-24565 and 25157-25520.

The frozen candidate universe contains 313 paths, all passing in pinned
QuickJS 2026-06-04. Its 19-path exclusion ledger records eight private async
methods, eight private async generators, and three async-generator
adjacencies. The admitted manifest contains 294 paths / 568 variants: 236
positive and 58 audited parse/SyntaxError paths, 226 async-harness and 68
synchronous paths, and 274 double-mode plus 20 `noStrict` paths, producing 294
sloppy and 274 strict variants. Oxide passes 568/568 with no unsupported,
skipped, failed, timed-out, crashed, or infrastructure outcome. The global
async profile remains fail-closed; this is an authenticated scoped claim.

The scoped-profile, candidate, manifest, variant-key, canonical TSV, and JSONL
SHA-256 values are
`9dbf8b47dafbc6df98ae38a1c24c489fc530bf93bc5be7cd8d9efa0d86a3bd4c`,
`59f3e239b96257ac169ad20b4df664c463e4b29f823423c833a667118b8aec8d`,
`220fd2dd88cef8efb4ff92616f01bd28cfbf6c0e0527cd20cd14a0dbb15db524`,
`36f9c5af110ae8d5623a528db3c8462fe2a02d57d580a948b5b146d6387a682e`,
`d63549c1597784d0624320e14f91dc0a67bc39fe41673370edc6e3e018724b43`,
and
`774df1782c75f240b2163f91600745d7245980f0cdd265a56938f8c404fe2ff5`.

The contextual differential keeps two QuickJS-specific boundaries explicit.
`async;` is committed as the start of an async method and rejected, rather
than parsed as an empty `async` field. A direct line terminator after `async`
splits that field from the following synchronous element, but U+2028/U+2029
inside an intervening block comment are ignored by QuickJS's small lookahead
scanner. The ordinary lexer still records those code points correctly for ASI
and restricted productions.

The rejection oracle also records a non-blocking diagnostic-span debt.
Instance `async constructor`, static `async prototype`, and method-body
`super()` match QuickJS in `ErrorKind` and message but not column. The same
offset already exists for synchronous getter/generator constructors, static
`prototype`, and synchronous object/class method `super()` errors, so R3ad
does not widen the known semantic frontier.

The exact R3ac/R3ad full join retains all 102,037 unique keys with zero
missing, extra, duplicate, detail-only, or previous-pass-regressed rows. Its
only outcome transitions are the sloppy and strict variants of
`test/staging/sm/Function/function-name-method.js`, both
`unsupported-runtime -> pass`. Runnable remains 45,140; passes rise from
43,659 to 43,661 and `unsupported-runtime` falls from 34 to 32. The R3ad full
TSV/JSONL SHA-256 values are
`a7bf54d0dda0b341fc4e84b7ba0edfb3af36e21ed3f5c93cbaae6cd510ef1aee`
and
`ab5e5385fa073939aef78864d97710fa05da0c331f001f6ffbabb85abc01f777`.

Reproduce the focused gate and semantic differential with:

```sh
./scripts/test-test262-async-class-method-core.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_async_class_method -- --nocapture
```

At the R3ad landing, private async class methods and async generators remained
explicit typed frontiers.

## R3ae ordinary private async class methods

R3ae composes the existing async-method execution path with the authenticated
private-method publication path. Pinned QuickJS keeps the grammar role at
`JS_PARSE_FUNC_METHOD`, selects `JS_FUNC_ASYNC`, marks the relevant class side
as branded, forces the private callable to retain its HomeObject, then performs
the ordinary private duplicate check, inferred `#name`, and lexical
initialization. Oxide likewise reuses `parse_async_method_definition`,
`BindingKind::PrivateMethod`, `InitializePrivateMethod`, the
HomeObject-derived brand, and the existing Promise/await driver. Its unlinked
publisher, linked heap validator, and live private-cell boundary admit the
exact method shape `(FunctionKind::Async, false)` while continuing to reject
async generators and private accessors whose execution kind is not Normal. No
new opcode, cell type, brand store, or suspension format is introduced.

The frozen candidate universe contains 233 paths, all passing in pinned
QuickJS 2026-06-04. Its 77-path exclusion ledger records 68 private async
generators, eight public async-generator adjacencies, and one mixed staging
path. The admitted manifest contains 156 paths / 312 variants: 92 positive and
64 audited negative paths, 86 async-harness and 70 synchronous paths, all in
both sloppy and strict mode. Oxide passes 312/312 with no unsupported, skipped,
failed, timed-out, crashed, or infrastructure outcome. The global async profile
remains fail-closed; this is an authenticated scoped claim.

The scoped-profile, candidate, manifest, variant-key, canonical TSV, and JSONL
SHA-256 values are
`668acc7b6b7de1345a1baa90d4f60fb67a2fa8beb018ab12a9bcd4cfba928b8e`,
`a9a2aa2e48f83d2a4beb86704923827223bef6b77b83324f8fc0a319645b93f5`,
`baa888fd5d5bea134123d563f8cc23a2ab483d6b0644c319c8dbc210b1a8d5bf`,
`2ecb1effac625bd14a932929fecfe4f721f264a5cfeafaed9f0717d245716231`,
`712c9dc36155bb8337d28e30ec2ee48fd69027f3c4145fb9ce93f4e32af726c0`,
and
`37a22c4ea13d16a0403c73d2ac6988a566665a0c78a7ac1716bc973f5ddd9c3a`.

The Test262 candidate tree has no private-async
`Function.prototype.toString` path, so the dedicated pinned-QuickJS
differential closes that semantic gap. It covers instance/static shape and
authored source, private inferred names and length, non-constructibility,
Promise settlement and rejection, independent brands, private-`in`, read-only
assignment, extracted dynamic receivers, `super` in parameters and across
`await`, and synchronous wrong-brand checks before argument side effects. The
parser differential also keeps the R3ad contextual boundary: same-line
`async #name` selects an async private method, while a direct line terminator
ends an `async` public field before the following synchronous private method.

The exact R3ad/R3ae full join is byte-identical. All 102,037 unique keys and
45,140 runnable variants remain present, with zero outcome transitions,
detail-only change, missing, extra, duplicate, or previous-pass regression.
Passes remain 43,661. The full TSV/JSONL SHA-256 values therefore remain
`a7bf54d0dda0b341fc4e84b7ba0edfb3af36e21ed3f5c93cbaae6cd510ef1aee`
and
`ab5e5385fa073939aef78864d97710fa05da0c331f001f6ffbabb85abc01f777`.
This zero-delta result is expected because R3ae remains scoped rather than
widening the global async profile.

Reproduce the focused gate and semantic differential with:

```sh
./scripts/test-test262-async-private-class-method-core.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_async_private_class_method -- --nocapture
```

Ordinary private async class methods are now measured independently; async
generators remain the next class-method frontier.

## R3af ordinary async generators

R3af adds ordinary async-generator declarations and expressions, the
AsyncGeneratorFunction/AsyncGenerator/AsyncIterator intrinsic graph, and the
Promise-backed FIFO `next`/`return`/`throw` state machine. The scoped claim
covers direct `yield`, `await`, awaited return values, suspended-start,
suspended-yield and completed states, queued requests, descriptors, dynamic
construction, authored source, parameter behavior, and parse-negative
provenance. Async-generator object/class/private methods remain separate
grammar/publication milestones.

The pinned semantic oracle also covers behavior beyond the admitted Test262
rows: immediate injection of an abrupt intrinsic PromiseResolve into authored
`await` without recursive native driving, actual settlement-job realm versus
defining-body realm, the asynchronous rejected-Promise path for a poisoned
completed `.return()`, iterator-result Promise-resolution reentry through an
inherited `then` getter, and QuickJS's one-trailing-request-per-completed-
driver-entry behavior. A later explicit protocol call is required to advance
the next already-queued completed request.

The frozen candidate universe contains 1,008 paths and 1,970 metadata-selected
variants, all passing in pinned QuickJS 2026-06-04. Its explicit 765-path /
1,530-variant exclusion ledger records 564 destructuring paths, 185 `yield*`
paths, six `for await` paths, five async-generator method-syntax paths, two
Proxy paths, and three realm/host paths. The resulting manifest contains 243
paths / 440 variants: 167 positive and 76 audited parse-negative paths, 117
async-harness and 126 synchronous paths, with 197 dual-mode, 33
sloppy-only, and 13 strict-only paths. Oxide passes 440/440 with no
unsupported, skipped, failed, timed-out, crashed, or infrastructure outcome.
The canonical report is byte-identical at five and eight workers.

The scoped-profile, candidate, exclusion-ledger, manifest, variant-key,
canonical TSV, and JSONL SHA-256 values are
`edb34a6dd924e3b01535b94e24495ba69a4a195b7492fed670f17714d5e543d7`,
`695b6ebd1518df08b47ee946f5a9dcbaf10396cebf2dadf27f797dea2e91a07d`,
`f795112b63fe9909c1cd6aa8dbb882ab5cd8c2db035aa7b69d416350f12d3d62`,
`bfc4244e45d22fd2d98c06f6d413cc7e58b58b004dfc3eebcc7d964834108e9f`,
`1de03a01c7a295fc8cf92c79ef8df77c4af3e641df1bd7e53249efad6b5a113c`,
`ab0974936b304c5789a44d6298821ce885ec39d655ad6a549f4301871c81f1bb`,
and
`0b348ca0165431dd152c44c862adaf9e52bca64045c5e99079035196824d38e8`.

Iterator-close semantics stay deliberately fail-closed. Every destructuring
path in the candidate universe is in the explicit exclusion ledger. The
admitted manifest has one ordinary `for-of` source occurrence:
`test/built-ins/AsyncIteratorPrototype/Symbol.asyncIterator/return-val.js`;
that loop is outside the async-generator body and only iterates a harness value
list. Thus the admitted cohort contains zero tests where `.return()` crosses an
active ordinary `for-of` or destructuring iterator. `yield*`, `for await`, and
their async iterator-close behavior remain later milestones rather than
accidental partial admissions.

The exact R3ae/R3af full-vector join authenticates the global movement. Both
TSV and JSONL contain the same 102,037 unique `(path, variant)` keys, with zero
missing, extra, duplicate, or previous-pass-regressed row. There are exactly 18
outcome transitions:

- ten `unsupported-parser -> pass` variants across
  `Object/seal/seal-asyncgeneratorfunction.js`,
  `comments/hashbang/function-constructor.js`,
  `expressions/async-generator/name.js`, and the two staging
  `AsyncGenerators/async-generator-declaration-in-modules.js` and
  `create-function-parse-before-getprototype.js` paths;
- three `unsupported-runtime -> pass` variants: the sloppy Annex B direct-eval
  catch redeclaration path and both modes of
  `async-functions/await-in-parameters-of-async-func.js`;
- two `fail-runtime -> pass` variants of
  `AsyncGeneratorFunction/is-a-constructor.js`;
- one `unsupported-parser -> unsupported-runtime` sloppy
  `AsyncGenerators/for-await-bad-syntax.js` variant, which now crosses the
  ordinary async-generator parser and stops at the explicit `for await`
  frontier;
- two `unsupported-parser -> fail-runtime` variants of
  `Proxy/revoked-get-function-realm-typeerror.js`, which now cross the
  async-generator parser and expose the pre-existing missing `Int8Array`
  intrinsic before reaching Proxy behavior.

Six same-outcome detail-only rows also become more precise: both modes of the
two `Function/function-name-computed-0{1,2}.js` paths now identify the remaining
async-generator class-method parser frontier, and both modes of
`extensions/newer-type-functions-caller-arguments.js` identify the remaining
async-generator method runtime frontier. No other row changes. The 15
previous-nonpass variants that become passes raise the complete vector to
43,676/102,037; runnable stays 45,140, `unsupported-parser` falls from 45 to 32,
and `unsupported-runtime` falls from 32 to 30. The R3af full TSV/JSONL SHA-256
values are
`6b34f59397a351c833b1d79803b4aafd9d93256177f59d8044361123f01391b1`
and
`c4f8ec2a11d5d84601c2250f25570f015952a7c10723ad92d52c649e606792ba`.

Reproduce the focused gate and semantic differential with:

```sh
./scripts/test-test262-async-generator-core.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --locked --test oracle_async_generator -- --nocapture
```

The global Test262 profile remains fail-closed for broad async execution, so
this authenticated scoped result is not a whole-suite parity claim.

## R3ag object-literal async-generator methods

R3ag adds ordinary object-literal `async *method(){}` syntax without adding a
second runtime path. This follows pinned QuickJS directly: the parser combines
the Method grammar role with AsyncGenerator execution, then publishes fixed or
computed keys through the existing enumerable `DefineMethod` operation. Oxide
therefore reuses the R3af intrinsic graph and Promise-backed request driver
while the established method path supplies inferred names and HomeObject.

The semantic differential locks fixed, computed, string, numeric, and Symbol
names; computed-key ordering; the `__proto__` method spelling; property and
callable descriptors; own/intrinsic prototype relationships;
nonconstructibility; exact authored `Function.prototype.toString`; synchronous
parameter initialization and abrupt completion; delayed body entry; borrowed
receivers; and `super` across `await`, `yield`, and a GC cycle. Contextual-token
tests cover escaped `async`, line terminators, duplicate parameters,
parameter-position `await`/`yield`, strict directives, and `super()` early
errors. Class/private methods, `yield*`, `for await`, and return across an
active iterator remain typed fail-closed frontiers.

The frozen focused core candidate universe contains 113 paths / 216
metadata-selected variants from the object method-definition and Function
`toString` families, and pinned QuickJS 2026-06-04 passes all 113 paths. The
explicit 67-path / 134-variant ledger excludes 58 `yield*` paths, two `for
await` paths, four destructuring paths, two private-name paths, and one Proxy
path. The resulting manifest contains 46 paths / 82 variants: 23 positive and
23 audited parse-negative paths, 18 async-harness and 28 synchronous paths,
with 36 dual-mode, eight sloppy-only, and two strict-only paths. Oxide passes
82/82 with no unsupported, skipped, failed, timed-out, crashed, or
infrastructure outcome. Independent default 8/8/5-worker runs produce
byte-identical TSV and JSONL; other suite consumers remain visible in the
complete vector.

The scoped-profile, candidate, exclusion-ledger, manifest, variant-key,
canonical TSV, and JSONL SHA-256 values are
`7c21b92bc769a6de2812f2c953bc7fe567e5df528255b4a85bfa429eb3d56ad9`,
`d6fd96dcc29e4b3b87b64cfe3d8692f99bd1852762ebb7673467e9b85f6d49f9`,
`97a7fd213d823a1c43eb650daef69c6153eb56c17db43ac54f38b1a288d97f00`,
`d4e3923053e589ec699880a946f5e1b9f00180c0b017a98377ed1a85643f3798`,
`f8cca2f8b154bef5aaa37d9dbc53c6a4faaec1e2048ff0f9cc8ceadee2c6e0dd`,
`e5798193ae60299f94099b8f4b8cedc72a656051d165471c535074b3c097d93c`,
and
`84b86f1fac5b6e8b9e3ed6761576202221c8dbf558a3783e646aedf0a2db96b3`.

The exact R3af/R3ag full-vector audit retains all 102,037 unique keys and all
45,140 runnable variants. Exactly four outcomes change: sloppy and strict
variants of `staging/sm/PrivateName/illegal-in-object-context.js` and
`staging/sm/extensions/newer-type-functions-caller-arguments.js` move from
`unsupported-runtime` to `pass`. There is no other outcome drift and no
previous-pass regression. Two same-outcome detail rows also advance: sloppy
and strict `staging/sm/BigInt/property-name.js` remain
`unsupported-parser`, but now identify the class async-generator method
frontier instead of the completed object-method frontier. All six changed rows
are already-admitted consumers outside the 113-path candidate partition; its
46-path manifest and 67 exclusions have zero drift. Passes rise from 43,676 to
43,680 while `unsupported-runtime` falls from 30 to 26. The R3ag full
TSV/JSONL SHA-256 values are
`37f72b038cdfa81ba1704bef05578e273e70a612e3daf8c23a54d22a984a5b88`
and
`8e7a70940a97f97232fc4fccc8b05bf57f1135896944399b9d96a8bc76fb3d2f`.

Reproduce the focused gate and semantic differential with:

```sh
./scripts/test-test262-async-generator-object-method-core.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --locked --test oracle_async_generator_object_method -- --nocapture
```

## R3ah public class async-generator methods

R3ah adds public instance/static class `async *method(){}` syntax without a
class-specific runtime path. This matches pinned QuickJS: the parser retains
the Method grammar role while selecting AsyncGenerator execution, and public
fixed/computed names continue through the existing non-enumerable class
`DefineMethod` operation. The callable prototype graph, own generator
prototype, FIFO Promise driver, inferred names, and HomeObject therefore reuse
the R3af/R3ag machinery. Private `async *#name`, delegation, `for await`, and
active iterator closing remain independently fail-closed.

The semantic differential covers instance/static fixed, computed, string,
numeric, and Symbol names; computed-key ordering; exact authored
`Function.prototype.toString` with `static` excluded; method and callable
descriptors; own/intrinsic prototype relationships; nonconstructibility;
computed `constructor` publication; the runtime TypeError from attempting a
computed static `prototype` definition; synchronous parameter initialization
and abrupt completion; delayed body entry; `arguments`, `new.target`, `await`,
and `yield`; and base/derived `super` with borrowed receivers across suspension
and GC. Contextual and early-error probes cover escaped `async`, line
terminators, comment separators, duplicate parameters, parameter-position
`await`/`yield`, strict directives, constructor/prototype restrictions, and
forbidden `super()`.

The frozen focused core candidate universe contains 573 paths / 1,118
metadata-selected variants: 396 direct class method paths, four Function
`toString` paths, one contextual-token path, 160 class-element composition
paths, and 12 syntax paths. Pinned QuickJS 2026-06-04 passes all 573. The
explicit 256-path / 512-variant ledger excludes 232 `yield*` paths, eight `for
await` paths, eight destructuring-scope paths, and eight private-composition
paths. The resulting manifest contains 317 paths / 606 variants: 236 positive
and 81 audited parse-negative paths, 216 async-harness and 101 synchronous
paths, with 289 dual-mode, 20 sloppy-only, and eight strict-only paths. Oxide
passes 606/606 with no non-pass outcome. Independent default 8/8/5-worker and
override 3/3/5-worker runs produce byte-identical reports. This is a focused
core partition, not an exhaustive inventory of every Test262 consumer.

The scoped-profile, candidate, exclusion-ledger, manifest, variant-key,
canonical TSV, and JSONL SHA-256 values are
`4c088b7e15be3bc1de099abf6560917c5677aa229fdc1799d0ff31367166ca63`,
`69ad11be927670c4578b0ac5ee80e2862a9c2f2c881a5282af39fd660b5bace5`,
`7b2a630ec520d90a973f9e7c1cd3af03938adc871afbed44f5a0893b8032e2c5`,
`f7620c23730693b2b8b46ef85b2f373d9c5d0fd5c7da19b4af356ede77bcdc43`,
`75e07a55c503357ead33c8782ccdb416d2a238a90757500d593b305d5d3c4d53`,
`1e1e8bdfc2101862e835db7eda9e6ae304cdaa6457035cd2c8dd6c7fff1940e0`,
and
`d7d9bbd90e09f2f02d23b2533a5076887ea0dcf7f4c114ff13b472af24d5e18b`.

The exact R3ag/R3ah full-vector audit retains all 102,037 unique keys and all
45,140 runnable variants with no duplicate, missing, extra, or previous-pass
regression. Exactly six outcomes change: sloppy and strict variants of
`staging/sm/BigInt/property-name.js`,
`staging/sm/Function/function-name-computed-01.js`, and
`staging/sm/Function/function-name-computed-02.js` move from
`unsupported-parser` at the former public class async-generator frontier to
`pass`. All six are already-admitted consumers outside the 573-path focused
candidate partition; its 317-path manifest and 256-path exclusions have zero
drift. There are no other outcome or same-outcome detail changes. Passes rise
from 43,680 to 43,686 while `unsupported-parser` falls from 32 to 26; every
other summary count is byte-identical. The R3ah full TSV/JSONL SHA-256 values
are
`2932f9d54df006def9ac2e9b01a8f9b7a5228bb58a42309d2f27b5fb26d81c18`
and
`7e7121200f385829a3676514ad091d26c39ee9780c46ed5f54c41dadff1ad193`.

Reproduce the focused gate and semantic differential with:

```sh
./scripts/test-test262-async-generator-class-method-core.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --locked --test oracle_async_generator_class_method -- --nocapture
```

## R3ai private class async-generator methods

R3ai adds private instance/static class `async *#method(){}` syntax by
composing paths already used by pinned QuickJS. The parser retains the Method
grammar role with AsyncGenerator execution; private publication keeps the
existing typed method cell, HomeObject, side brand, and
`InitializePrivateMethod` path. The callable's own generator prototype and
Promise-backed FIFO request driver are the R3af implementation. No
class-specific branch is added to `runtime.rs`.

The semantic differential covers private instance/static names and extraction;
function length, exact authored source, intrinsic and own-prototype shape,
nonconstructibility, synchronous parameters and parameter failures, delayed
body entry, queued `next` settlement, private-name `in`, access-time and
resume-time brand checks, borrowed receivers, `await`, and `yield`. The
publication verifier independently authenticates Method+AsyncGenerator
metadata, the required own prototype and initial-yield shape, HomeObject, and
instance/static brand initializers. `yield*`, `for await`, and `.return()`
while a nested iterator is active remain fail-closed.

The frozen focused core candidate universe contains 433 paths / 858
metadata-selected variants: 322 direct private-method paths (162 instance and
160 static), 68 class-element composition paths, 40 syntax paths, two
object-literal negative paths, and one staging path. Pinned QuickJS 2026-06-04
passes all 433. The explicit 308-path / 616-variant ledger excludes 300
`yield*` paths and eight `for await` paths. Sixty-eight generated
class-element composition filenames do not advertise delegation, but their
private async-generator bodies use `yield * await value`; they therefore
belong to the delegation ledger rather than this milestone. The resulting
manifest contains 125 paths / 242 variants: 29 positive and 96 audited
parse-negative paths, 22 async-harness and 103 synchronous paths, with 117
dual-mode and eight strict-only paths. Oxide passes 242/242 with no non-pass
outcome.

The scoped-profile, candidate, exclusion-ledger, manifest, variant-key,
canonical TSV, and JSONL SHA-256 values are
`1b9d03b352d8e221cae6d0cc6c6c685776f16e0ca39c97c5fafc7b8bdca00f38`,
`3b54cf73426d746a18563c75b4b827b7c4d25d3ee98e8908ca312b7db43dd909`,
`3508dcaff42bb06de45f8b6678170a290fdf52bc932a7a6b8c4d5bd662e7839c`,
`82bae49d063b9691d245f1a08d0e37583fc27282ceb878cca7c4e1129e6fcad6`,
`e0f31c9d25a89ec4b6d8ca5b2a7ba13ab223d219d65c56e84f478a34f50b9bbb`,
`d4b22c03825eeb1d0a6e6214a69eec9dbea3c81f2571b4f0d6aa7dd84c55c0ec`,
and
`c3ebc03b435d2ca8f534cd48970da8d703c4edd6dc8b02a4600a514030ae0d6f`.

The exact R3ah/R3ai complete-vector join retains all 102,037 unique keys and
all 45,140 runnable variants, with no outcome transition, same-outcome detail
change, duplicate, missing, extra, or previous-pass regression. Passes remain
43,686 and every summary count is unchanged; the full TSV/JSONL SHA-256 values
therefore remain
`2932f9d54df006def9ac2e9b01a8f9b7a5228bb58a42309d2f27b5fb26d81c18`
and
`7e7121200f385829a3676514ad091d26c39ee9780c46ed5f54c41dadff1ad193`.
This zero-delta result is expected from a scoped private-method admission rather
than a global async-profile widening. The next implementation priority is
async-generator `yield*`, followed by `for await` and its iterator-close
semantics.

Reproduce the focused gate and semantic differential with:

```sh
./scripts/test-test262-async-generator-private-class-method-core.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --locked --test oracle_async_generator_private_class_method -- --nocapture
```

## R3aj async-generator `yield*`

R3aj adds async-generator delegation without claiming complete async
iteration. It covers `yield*` in ordinary declarations/expressions,
object-literal methods, and public/private instance/static class methods. The
runtime selects `Symbol.asyncIterator` first and otherwise installs the
QuickJS-shaped Async-from-Sync adapter over `Symbol.iterator`; delegation
preserves the distinct async-iterator and synchronous-iterator value
assimilation rules.

The frozen cohort is the exact, duplicate-free union of the `yield_star`
partitions from the four R3af-R3ai exclusion ledgers: 185 ordinary-function
paths, 58 object-method paths, 232 public-class paths, and 300 private-class
paths. That yields 775 Test262 paths and 1,550 sloppy/strict variants. There
are 774 positive paths and one audited parse-negative path; 774 paths use the
async harness and one is synchronous. Pinned QuickJS 2026-06-04 passes
775/775. Oxide runs and passes 1,550/1,550 with zero failure, unsupported,
skipped, timeout, crash, or infrastructure outcome. A canonical eight-worker
run, an independent eight-worker repeat, and a five-worker run produce
byte-identical TSV and JSONL reports (8/8/5).

The focused QuickJS differential authenticates ten observable transcripts:
iterator selection and cached `next`, async yielded-Promise identity,
Async-from-Sync value assimilation, delegated `next`/`throw`/`return`, FIFO
requests, missing-method behavior, IteratorClose/error precedence, rejection
paths, and abrupt iterator acquisition/result completion. A separate GC test
proves that suspended delegation retains both async and synchronous delegates.

The scoped profile, manifest, variant-key, canonical TSV, and JSONL SHA-256
values are
`80bd7d1c042473a76ba15d85b3e5bbd6ebf175f0543c57e2908fd99a6b7b5256`,
`bb31f01a982136b336f9267701ef8b2874bc0596e226f6e9ca5b59e7b9af09fb`,
`d3beb98f2b199c3a66acf4c58d44f65c06f2edf6ef2a52fe4d7caf045105dec5`,
`b819f6fe3443cfd2f3baefdde489d397ea405115f5692f943172e010df08dc40`,
and
`53ebba1f2d8fb80ab82aff4869b99646230f16013fd9fef8a6660d48ef36a915`.
The complete-vector R3aj regression retains all 102,037 variants, 45,140
runnable variants, and 43,686 passes, with zero outcome, detail, key-set, or
previous-pass drift. It is byte-identical to R3ai: the TSV and JSONL SHA-256
values remain
`2932f9d54df006def9ac2e9b01a8f9b7a5228bb58a42309d2f27b5fb26d81c18`
and
`7e7121200f385829a3676514ad091d26c39ee9780c46ed5f54c41dadff1ad193`.
At R3aj, `for await` was the next async-iteration frontier, and closing an
independently active outer iterator when `.return()` crossed delegation was a
separate semantic slice. R3ak closes both boundaries.

Reproduce the focused gate and semantic differential with:

```sh
./scripts/test-test262-async-generator-yield-star.sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --locked --test oracle_async_generator_yield_star -- --nocapture
```

## R3ak authenticated `for await` gate

R3ak freezes and executes the complete pinned `for await` neighborhood.
Candidate discovery unions tracked `.js` paths
whose filename matches `for[-_]await|forawait` with paths whose source contains
`for await`, then intersects that set with the exhaustive 53,125-row metadata
inventory. This yields 1,297 candidate paths and 2,531 canonical variants. The
candidate path and key-set SHA-256 values are
`4bdfca76ce452b54f7de1a877cf3ac63a845f89d905fe6fdd712d18abf968bcf`
and
`a52d5b419f81529f05a76f5fa1895f0f1c31169f46f09be552e8ee59045db54f`.

The fail-closed ledger contains 33 paths / 41 variants:

- three `explicit-resource-management` paths that pinned
  `test262.conf` marks `skip`;
- 28 module, top-level-await, import-meta, or dynamic-import paths;
- one optional-chaining consumer;
- one `$262.IsHTMLDDA` host consumer.

The ledger, excluded-path, and excluded-key SHA-256 values are
`cfcad1d88bd4e39de7f24763dc8e221c304f19ecbf8b314732171ca5aeb48eb1`,
`2d2aaa8448ec87e5f7b825c6d72898be1f1f814aa1e8b0f93c94cbe958c8acdd`,
and
`ef68b63e1d68784d45801f0236894119a6498ab057b1e641fa29b858f68727c1`.
Subtracting it leaves 1,264 admitted paths / 2,490 variants; the derived
manifest and variant-key SHA-256 values are
`45afa1e6f8f61d44e733aeea8bde5dae562a7ec919ea40d9d1e18551d6f2881f`
and
`756ea05ac92fed9281a84f8e7f40b1992c640258ca41790158c41dfbe720bf57`.

The admitted shape is:

- 1,232 paths / 2,427 variants from
  `test/language/statements/for-await-of`, of which 1,215 paths / 2,396
  variants exercise assignment or binding destructuring;
- 24 paths / 48 variants inherited from the four async-generator ledgers:
  six ordinary-function, two object-method, eight public-class, and eight
  private-class paths, evenly split between async iterators and synchronous
  fallback;
- five `AsyncFromSyncIteratorPrototype` paths / ten variants;
- one async interleaving path / two variants;
- two staging grammar paths / three variants.

Metadata classifies the manifest as 1,174 positive and 90 audited
parse-negative paths, 1,170 async and 94 synchronous paths, and 1,226
double-mode, 31 `noStrict`, and seven `onlyStrict` paths. The scoped profile
admits exactly 12 features, three includes, four flags, and the async host.
Pinned QuickJS 2026-06-04 passes 1,264/1,264 admitted paths. On the wider
candidate it executes 1,294/1,294 baseline-enabled paths and skips exactly the
three upstream-configured ERM paths.

At the R3ak checkpoint, Oxide passed all 2,490 variants: 2,490 pass, zero
fail, zero unsupported, and zero skipped. Independent 8/8/5-worker runs
produced byte-identical TSV and JSONL reports. The empty non-pass, TSV, and
JSONL SHA-256 values were
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
`7eafa4725fbb6f70954c5bdb52a823caeaa89497eb01d6c80d446925d01361d0`,
and
`ecba171afdc2272de5b0e40b824f28159bfad04c9f485527b64ad6b533dd00fd`.
At the R3ak checkpoint the global profile remained fail-closed for
`async-iteration`, so that scoped admission did not silently widen the
complete-vector denominator. A dedicated pinned-QuickJS transcript covers
active async-generator `.return()` crossing the outer iterator, including the
suspension and close-Promise ordering which is not represented by this Test262
cohort.

The exact R3aj/R3ak full-vector join matches all 102,037 keys. Exactly three
already-admitted SpiderMonkey staging variants move from
`unsupported-runtime` to `pass`: the sloppy
`for-await-bad-syntax.js` variant and both sloppy/strict
`for-await-of-error.js` variants. There is no other outcome, detail, key-set,
or previous-pass drift. The complete vector therefore retains 45,140 runnable
variants and reaches 43,689 passes. Its TSV and JSONL SHA-256 values are
`36e2a11f4eaba4ffd92fdd561b18b27337b90b14a564cab9da6385f1aa0f79a3`
and
`1dd6c356c678568b51794d253959a58a644dbdd2871187f67516ad8d78e649af`.
These values remain the historical R3ak receipt. R3bk below records the
refreshed current focused gate without changing the complete vector.

## R3bk refreshed `for await` gate

R3bk keeps the exhaustive R3ak candidate discovery unchanged at 1,297 paths /
2,531 canonical variants. Now that optional chaining is implemented and
globally admitted, the refreshed dependency ledger removes exactly
`test/language/expressions/optional-chaining/iteration-statement-for-await-of.js`
from its exclusions and the scoped profile adds `optional-chaining`. The
remaining 32 exclusions account for 39 variants: three upstream-skipped
explicit-resource-management paths, 28 module/dynamic-import paths, and one
`$262.IsHTMLDDA` host path. The exclusion-ledger SHA-256 is
`cf172c4d38c6fee27f20ccc6775251284e328255f1a416b9ff22f5760e2a1e47`.

The refreshed manifest therefore admits 1,265 paths / 2,492 variants. Pinned
QuickJS 2026-06-04 passes 1,265/1,265 admitted paths; Oxide passes all
2,492/2,492 variants with zero non-pass outcome. Independent 8/8/5-worker runs
again produce byte-identical TSV and JSONL reports. The scoped-profile,
manifest, variant-key, TSV, and JSONL SHA-256 values are
`d5d30d77eaabebeea1a9fa3cb18f555e3c5d69d263d1b82ca624c339f6262a2e`,
`f87858a6c22df8c689d15f081075cba2758feb63eacb4be9ee310e72e9d17a0a`,
`8669fd1b353cf24a52297a6680a4b43041a7c03ac5c33cd93abf8afbe82535cd`,
`6b102a66ca2c71be3f9999efd027bda49f65b3a3465d555c7775a59b999ed823`,
and
`ca3703f6fb7296af390979df9f60a6049d3d8703cc6929cf2937586afd972832`.

Reproduce the current refreshed gate with:

```sh
./scripts/test-test262-for-await-of.sh
```

The checked-in R3al global-profile hash now serves only as provenance for the
historical async admission point. The focused gate no longer compares that
hash with the growing global profile: it pins its own capability profile and
checks that the live profile still enables async execution. This preserves the
real async execution precondition while preventing unrelated future global
feature admissions from drifting the focused receipt.

This is a scoped evidence refresh, not a new global admission or runtime
change. The complete vector remains at 56,526/102,037 passes and 57,045
runnable variants. Its canonical R3bj TSV/JSONL SHA-256 values remain
`84c15d4a25343e1d306e17f431e515993abe09db76590920539eefe93d6fb3eb`
and
`96ebd4a8f51001b403e88d19c128bebb92b74bb9abf1e45c832b187924c635fd`.

## R3al global async admission

R3al promotes `async-functions`, `async-iteration`, and the async execution
host into the global capability profile. This is the combination gate for the
R3z-R3ak implementation stack: ordinary async functions and arrows, ordinary
and private object/class methods, all four async-generator shapes, `yield*`,
and `for await ... of`.

The frozen newly executable cohort contains 3,589 paths / 7,076 canonical
variants. Pinned QuickJS passes all 3,589 paths. Oxide passes all 7,076
variants, and independent 8/8/5-worker reports are byte-identical. The
manifest, metadata selection, key-set, TSV, and JSONL SHA-256 values are
`7e83bef89f3deaf151275877fd3baeab1891ed66cdc423af8e52c45a858acd97`,
`b94d52b85bc1faa296bada8b0dd7f09e70e3fe3e2575c6cfcdccbd66138f3a29`,
`8029a961f158f0b649532cd13ff18d85a07a133ed1e3b37a0494fd3e624908db`,
`136b179ed6ab8d4b17c56e0ed6e214753c5700fcbc448a4d10d5d95bf648be40`,
and
`14ec16dd95ff9953b58d2be537f71b21611d5419f0f904af73e8ae0e7960997f`.

The gate also proves that this manifest is exhaustive rather than merely
self-consistent. A checked-in provenance table extracts all 12,647 R3ak
variants whose old selection result mentioned the async host,
`async-functions`, or `async-iteration`, across 6,496 paths, from the
checksum-pinned R3ak full vector. Reclassifying that whole candidate universe
under R3al produces exactly 7,076 runnable passes and 5,571 still-unsupported
variants; the runnable key set is byte-for-byte identical to the focused
manifest key set. The before-table and its key-set SHA-256 values are
`173d61580131172206cb476a4239395a5a258d539723587d924d161eb12d461f`
and
`6d888787cb21790babb173d93d3a73df58ebaf323b87dcc8ec35cb4041e84bfc`.
The exact 12,647-row before/after transition vector has SHA-256
`eae7dd348199be707bdd914e1d8be2eb5bf63a17ee7c93ef96548e915e57b1d8`.

Reproduce that focused combination gate with:

```sh
./scripts/test-test262-global-async.sh
```

The exact R3ak/R3al full-vector join retains all 102,037 keys and every
previous pass:

- 6,122 variants move from `unsupported-async` to `pass`;
- 954 move from `unsupported-feature` to `pass`;
- 3,866 move from `unsupported-async` to another explicit feature frontier;
- 952 move from an async feature frontier to unaudited negative provenance;
- 75 move from `unsupported-module-async` to `unsupported-module`;
- two each move from the async frontier to `createRealm` and `IsHTMLDDA`;
- 674 remain `unsupported-feature` while their detail drops the newly admitted
  async tags.

No newly executed variant fails to parse or run, fails the harness, times out,
crashes, or produces an engine/runner fault. The global profile now contains
76 feature tags and 802 audited negative paths with async execution enabled.
Its SHA-256 is
`fc6e8010c982bd6324b146e5f8e3ea0592aac7c03a323a8dbc8d778b4b670b23`.

The complete vector reaches 50,765 passes and 52,216 runnable variants. Its
TSV and JSONL SHA-256 values are
`93456e63a780ac6b02253853a5711464d01944f6df30a22d8b1a6fcde6a66366`
and
`40417ac19f60988a3257e4d577ea1f485ef61637f1c444820ebe5662638fa13e`.
The remaining async-tagged tests stay behind their actual independent class,
default-parameter, module, Promise-method, host, or negative-provenance
dependencies.

## R3am Proxy internal-method gate

R3am freezes 464 Proxy and Proxy-consumer paths / 904 canonical variants in a
scoped profile. Pinned QuickJS passes all 904 variants. Oxide records 811
passes, 81 explicit unsupported outcomes, and 12 harness failures among 829
runnable variants:

- 74 variants require the unavailable `$262.createRealm` host hook;
- one is a module variant and six stop at independent parser frontiers;
- the 12 harness failures are six TypedArray-adjacent paths in both modes,
  where `testTypedArray.js` requires the still-unimplemented `Float64Array`.

There is no Proxy runtime failure, timeout, crash, or engine/runner fault in
the scoped vector. The profile, manifest, key-set, non-pass, TSV, and JSONL
SHA-256 values are
`0c151608ed8cd580238e27188f5e63382ee11e1dc91f7c480db2c366e1232d12`,
`ef2395cd3bd268d7ba1010773651408826452feaed121f8f2d4c0e6afeed66f3`,
`0ec3bfa0ffaec0ddf6e20512be35ab866d3e07658cddaf3e88f29f7d64b97bfd`,
`323c76b74e94c0f585d92e371ee69aa753c505ca683dd6a0ae42afaa53a76fda`,
`2e6f22c51fa30db9a3507aa60371c71afb3669840b624d09382119415962662c`,
and
`83e064472fdfd7154be4d1b57a21ec7628ee4b6d785f9b0f6c9af8e82848d1cf`.

Reproduce the scoped gate with:

```sh
./scripts/test-test262-proxy.sh
```

Proxy remains scoped in the global capability profile, so R3am does not
silently admit every feature-tagged combination. The exact R3al/R3am full
join nevertheless retains all 102,037 keys and every previous pass. It records
208 `fail-runtime -> pass` transitions from untagged or already-admitted Proxy
consumers and four `timeout -> pass` transitions after large unique-shape
property growth became amortized linear. The latter are both modes of
`String/fromCodePoint.js` and `String/string-upper-lower-mapping.js`;
`Proxy/ownkeys-linear.js` is included among the 208 runtime transitions because
its old run stopped at the missing global before R3am.

The complete vector reaches 50,977 passes with the same 52,216 runnable
variants. It has zero previous-pass regression, missing key, extra key, or
duplicate key. The full TSV and JSONL SHA-256 values are
`19209666492462edb063b24af6fd1278abcffa10178da0d1da1218fb49140b43`
and
`f4ee2c790693817bfe122127db7612ab7df5c2daf73b40514bce7b574f32061c`.

## R3an ArrayBuffer core gate

R3an freezes the pure ArrayBuffer core before the TypedArray/DataView view
kernel. Its audited pre-view candidate contains 168 paths. Twenty-four
transfer paths are held in a checksum-bound exclusion ledger because their
sources directly instantiate `Uint8Array` without declaring `TypedArray`
metadata. The resulting scoped manifest contains 144 paths / 288 canonical
sloppy/strict variants. Oxide passes 288/288, and pinned QuickJS passes the
same 288/288 variants.

The scoped profile, manifest, path/variant key stream, TSV, and JSONL SHA-256
values are
`0803a027b2e9c238f80189993968816adfdda983ef3b23114a06f07b26c2d598`,
`d5720cc22c785d3757eb4e30aa3de53a664d58133a2323c6afe6233788014d01`,
`bb2d3b0e3728e4aae955569ba0ffefc54ad215a02cfe5204fc3d483daf6e3bad`,
`254ae11ac69e0d2b13f9949f498224af8770cdf16c120c8a24fe5faaa9d97716`,
and
`43bb5e266e7558dd0b425831caefe7fb11d8fa8601194dac7c3f4042ec1ee642`.

Reproduce the scoped gate and its pinned QuickJS oracle with:

```sh
./scripts/test-test262-array-buffer.sh
```

The global profile admits only the authenticated `ArrayBuffer`,
`arraybuffer-transfer`, and
`align-detached-buffer-semantics-with-web-reality` tags from this slice.
`resizable-arraybuffer` remains scoped because that metadata tag also admits
large TypedArray/DataView-adjacent cohorts which are not implemented yet. The
global profile now has SHA-256
`9b155f41c9c7541423c45b57da1bb805d6e7cf350ec7d6442d6700424afdbafc`.

The exact R3am/R3an full-vector join retains all 102,037 keys and every
previous pass. Its pass-producing transitions are:

- 150 `fail-runtime -> pass`;
- 58 `unsupported-feature -> pass`;
- eight `unsupported-host-detach-array-buffer -> pass`.

Installing the real `$262.detachArrayBuffer` host hook also removes an old
selection barrier and therefore exposes deeper, still-open frontiers rather
than hiding them:

- 162 `unsupported-host-detach-array-buffer -> fail-runtime`;
- eight `unsupported-host-detach-array-buffer -> harness-error`;
- eight `unsupported-host-detach-array-buffer -> unsupported-host-gc`;
- 430 `unsupported-host-detach-array-buffer -> unsupported-feature`;
- 16 `unsupported-feature -> fail-runtime`.

The new runtime and harness failures are chiefly the still-unimplemented
DataView/TypedArray stack; the final 16 transitions are the latent transfer
variants which now reach their direct `Uint8Array` dependency. They are
recorded as honest frontier exposure, not as ArrayBuffer passes. This milestone
therefore establishes the pure backing-store/constructor/detach/transfer core,
not complete binary-data feature parity.

The complete vector reaches 51,193 passes, 52,468 runnable variants, and
52,419 variants with a non-unsupported observed outcome. Its raw pass rate is
50.17%, the conservative pinned-target lower bound is 61.26%, and the
conditional observed rate is 97.66%. The full TSV and JSONL SHA-256 values are
`12a60e9d1cd3e30b8b33e095ef226f50f56706bed942cdc465c15cc3463d45fe`
and
`814f8e1e6e99dba7778c3ba8bc4b26f4015ebe0130c1e5cc5f1e1c55653a8fb2`.

## R3ao DataView gate

R3ao adds the pure DataView layer on the R3an ArrayBuffer backing store. The
constructor and three prototype accessors, `ArrayBuffer.isView` integration,
and all 11 getter/setter families are implemented: `Int8`, `Uint8`, `Int16`,
`Uint16`, `Int32`, `Uint32`, `BigInt64`, `BigUint64`, `Float16`, `Float32`,
and `Float64`. The gate covers big- and little-endian access, pinned QuickJS
numeric conversion, detach, range and coercion order, constructor and method
reentrancy, and fixed-versus-length-tracking views across resizable-buffer
shrink and grow. TypedArray, SharedArrayBuffer, immutable-buffer, and
cross-realm dependencies are not claimed by this slice.

The audited candidate contains 578 paths. A checksum-bound 86-path exclusion
ledger leaves 492 paths / 984 canonical sloppy/strict variants. Oxide passes
984/984, and pinned QuickJS passes the same 984/984 variants. The cohort
fingerprints are:

- candidate stream SHA-256:
  `1df8f075f57cbcc2cf72f88835bbd08449fe2093bf8f5d33badc0148249db3ed`;
- exclusion path stream SHA-256:
  `feade99c881ad6763b2241d988ab4c95ff3a8b79ae51f6c3ddf0501b62fd9354`;
- exclusion ledger file SHA-256:
  `9cdc8a031c926dd59dc152b0cfb76bd97758d63d79703df86d162b3a7eec4f44`;
- manifest SHA-256:
  `3475b4a32f0a5f0ab50d5cd4e4843a7c7a59365298ecabcc5986b3fdd3f697e2`;
- scoped profile SHA-256:
  `485ea3baf6695767108fb9f7f346c3a82d5a3db000af4510d6d002b313990cc8`;
- path/variant key-stream SHA-256:
  `07d60a25d9dcb8316d4602456931cedff7668df634a92ab11c6efe4798c3f90c`;
- TSV SHA-256:
  `6a73330ca5a7114d60946cf276d7b2601fdd023b260789cea1b5c911380d1206`;
- JSONL SHA-256:
  `3a4b68f28084b0dc76773fe7255e090da73981afbab5388766fe6a149beb542b`.

The independent `oracle_data_view` target passes 3/3. Reproduce the Rust,
frozen-vector, pinned-QuickJS, and scoped Test262 evidence with:

```sh
QJS_ORACLE=/path/to/quickjs-2026-06-04/qjs \
  cargo test --test oracle_data_view -- --nocapture
./scripts/test-test262-data-view.sh
```

The exact R3an/R3ao join matches all 102,037 unique `(path, variant)` keys,
with zero missing, extra, duplicate, or previous-pass-regression rows. Its
only transition is 514 `fail-runtime -> pass` outcomes across 257 paths: 502
variants under `built-ins/DataView` and 12 under `staging/sm/DataView`. The
changed-key stream has SHA-256
`e3483d6bfb005a92ad9f5515d2fe8e7745c3e8a003be6f7291fa376ff8b9487c`.
The classified vector now has 51,707 passes, 52,468 runnable variants, 587
runtime failures, and 52,419 variants with a non-unsupported observed outcome.
The raw rate is 50.67%, the conservative pinned-target lower bound is 61.88%,
and the conditional observed rate is 98.64%. Full TSV and JSONL SHA-256 values are
`3d79ecd1349488f03e8288a9a0f41b4bc5e8b70573e8d41121438aa893940990`
and
`b233a6fe08dc14d0bd428f537cd9693f37a3d1d2a4f5d2b49881f9607ca60996`.

## R3ap TypedArray shared-core gate

R3ap adds a single branded backing-view payload and shared behavioral kernel
for all 12 concrete TypedArray classes in pinned QuickJS order. The scoped
surface covers the hidden `%TypedArray%` graph, concrete constructors,
accessors and tags, integer-indexed exotic internal methods, values iteration,
ArrayBuffer view detection, detach, and fixed or length-tracking views across
resizable-buffer shrink/grow and out-of-bounds recovery. Directed Rust probes
also lock `set`, static `from`/`of`, entries/keys, reentrant constructor
coercions, same-kind memmove, pinned QuickJS's different-kind overlapping
`set` result, `for-in`, host property definitions, realm ownership, and GC.
Those directed extras are not promoted into the scoped Test262 claim before
their complete method-family cohorts land.

The frozen candidate has 2,361 paths / 4,669 variants. A checksum-bound
1,626-path exclusion ledger classifies later method families and cross-realm,
SharedArrayBuffer, WeakMap, Math, and IsHTMLDDA dependencies, leaving 735 paths
/ 1,447 canonical variants. Oxide and pinned QuickJS both pass 1,447/1,447.
The relevant fingerprints are:

- scoped profile:
  `046200aa1abd9afa11a63602d5a8ea073ba6dd1ccee2e910775731c175378402`;
- manifest:
  `9ebae7adb9e1c033a71c0abf42aa003e0e03121da24ef98ca939e1f360a03777`;
- exclusion ledger:
  `2b18c745fe886709f578ba9cd927cea21c98dca9c02a6664c94f6fce3385e400`;
- path/variant keys:
  `2d1e474a52971496b669d5f3d650dece8c21069944a463356954442dbbf75362`;
- TSV:
  `816005701f3d6d5273860454dcde466bd7bfe64d24c44834ffea5d5363af71d2`;
- JSONL:
  `fb86a625a7bc9eddf043db9be4b736d65e4d023972219d7569ce082826cfd92c`.

Reproduce the candidate derivation, pinned oracle, and Oxide gate with:

```sh
./scripts/test-test262-typed-array-core.sh
```

The exact R3ao/R3ap full join retains all 102,037 unique keys and every
previous pass. Its 197 outcome transitions are 149 `fail-runtime -> pass`, 46
`harness-error -> pass`, and two `harness-error -> fail-runtime`; the two
deeper failures are the independent `Math/atanh-approx.js` accuracy frontier.
Another 44 rows retain their outcome while reaching more precise missing-method
or external-dependency details. There are no missing, extra, duplicate, or
previous-pass-regression rows. The transition stream SHA-256 is
`2b94d55d59acaf0daa969cbd7c3af8d0ada968f70713c304dbbbe83f48620304`.
The complete vector reaches 51,902 passes with 52,468 runnable and 52,419
non-unsupported observed variants. Full TSV/JSONL SHA-256 values are
`8a1b83df5e28641fb57d5d4a6fe29ed8c5b1f962e82c98f6acbce0cf595e85e5`
and
`a3f7a5952f67ab7e1c8055d8ef29f2645700c8aa6124411644c8cb6058684052`.

## R3aq TypedArray mutation promotion

R3aq publishes `%TypedArray%.prototype.copyWithin`, `fill`, and `reverse`,
and promotes those methods together with the previously directed-only `set`
cohort. The implementation follows pinned QuickJS's initial length snapshot,
observable coercion order, final detach/out-of-bounds revalidation, and live
resizable-buffer clipping. Same-buffer copy uses an allocation-free memmove;
fill converts one raw machine word before the bound coercions; reverse swaps
raw 1/2/4/8-byte words. Directed tests retain NaN payloads and negative zero
and cover temporary fixed-view out-of-bounds recovery, partial tracking
elements, detach, shrink, overlap, byte offsets, and BigInt conversion.

The frozen 2,361-path / 4,669-variant candidate inventory contains 254 paths /
508 variants classified as mutation. Two `set` paths use the not-yet-published
TypedArray `join` as their observation helper, and
`test/staging/sm/TypedArray/set.js` requires the unavailable WeakMap harness.
Those three paths / six variants remain explicit `dependency:join` or
`external:WeakMap` rows. The other 251 paths / 502 variants are promoted,
expanding the admitted manifest to 986 paths / 1,949 variants and reducing the
exclusion ledger to 1,375 paths. Oxide and pinned QuickJS both pass
1,949/1,949.

The focused profile, manifest, exclusion-ledger file, path/variant key stream,
TSV, and JSONL SHA-256 values are:

- `663ac07f1fe379125eec29aec0c7b8b8215c08f40b93e9c39056ff40c6331036`;
- `8542757a466917d9841cdc25317b78abad5db64aceda07ab78c8f38ced08bd3f`;
- `fe441699f63debd30e3c5e2ed66d2c9b21732280afc03807be8a2268dbe56c3a`;
- `1b983b9b5c97314449c54ec0da387f393964a758db02836e6bd2b9aa0af39f7b`;
- `159c4b02f25fe4430c970891141acda807336933382bd7363d4ed1d2a77dc618`;
- `0d5d6917134fc7087a301e23be7d24c3544fc739af158a6eaa270dd0615ac25c`.

The conservative global profile still does not declare the broad `TypedArray`
feature while later method families are absent. Consequently, 284 variants
from the three newly published methods remain honestly classified as
`unsupported-feature` in the complete vector; the two untagged
`staging/sm/TypedArray/fill-detached.js` modes move from runtime failure to
pass. The complete summary therefore advances from 51,902 to 51,904 passes,
with runtime failures falling from 440 to 438 and every other outcome count
unchanged. Full TSV/JSONL SHA-256 values are
`ab641b72ef2c2bc4615d493e03cf1538c308daa2edd4c8b7e752c0da3416e586`
and
`7eae1d679bfe748a6ea7123c534e60c0ba8d8fe5edfa29ff6a0a16ffb3e15e5f`.

## R3ar TypedArray indexed lookup/search promotion

R3ar publishes `%TypedArray%.prototype.at`, `includes`, `indexOf`, and
`lastIndexOf` as the first non-callback traversal slice. The implementation is
kept separate from generic Array search because pinned QuickJS validates the
TypedArray receiver up front, snapshots its internal length, performs
observable argument coercion, then caps direct integer-index reads against the
live backing view. It also preserves QuickJS's special
`includes(undefined)` result when `fromIndex` coercion shrinks or detaches the
buffer, while the two strict-equality index methods return `-1`.

The exact atomic candidate is 152 paths / 304 variants. One
`staging/sm/TypedArray/indexOf-and-lastIndexOf.js` path loads
`sm/non262-TypedArray-shell.js`, whose required WeakMap support is still
unavailable, so its two variants remain explicitly deferred. The other 151
paths / 302 variants join the manifest. This expands the cumulative scoped
gate to 1,137 paths / 2,251 variants and reduces the exclusion ledger to 1,224
paths. Oxide and pinned QuickJS both pass 2,251/2,251; pinned QuickJS also
passes all 4,669 variants in the unchanged expanded candidate.

The candidate, deferred, and promoted path-stream SHA-256 values are:

- candidate:
  `8e68d86281c54b4b2a6a35422a55b348969d43fa11622c142cc31507aaae371f`;
- deferred:
  `de7e9738d5d1934ea4d23809c52acc9c11598d51f7f8dc321cae940d054a0d46`;
- promoted:
  `061efff451e31693b84f61bf8072651ef366c1feb5ac880b2a47bba24203aeab`.

The scoped profile, manifest, exclusion-ledger file, and cumulative
path/variant key-stream SHA-256 values are:

- `c5d1a75871d567f892a982a1c549390c0f79aa3cefbd057dd88f713e98aafed7`;
- `85f8c692cdd7ae1715f19006da3b11f6f34e4b598f18f701ebc9fd911c9e9714`;
- `6eb2500c8befaaee380d1bed1e94f03450592f5d3da86c2cd523b6f7c2f9da62`;
- `8489275bb065e249286a3f113f26a90b9483b5030f2809e8575ec3148f419067`.

The canonical scoped TSV/JSONL SHA-256 values are
`cd4e54e8444178f8828b26615b983d90e3791346def1eec0e3d570e1c3204197`
and
`8a8d3f884bc2b22a2112a8d44ecb2cbf6091866235692239252b4352cedb4c28`.

The conservative global profile still withholds broad TypedArray admission.
Only four untagged variants—the sloppy/strict modes of the two SpiderMonkey
negative-zero index tests—move from runtime failure to pass. The exact full
join retains all 102,037 keys and every previous pass, with no missing, extra,
or duplicate rows. Its four-row transition stream SHA-256 is
`2b87010242ba56dcf9ca6bf1b49c733db36b3b4e558cd945b12ce22aa4acb2f7`.
The complete vector reaches 51,908 passes and 434 runtime failures; full
TSV/JSONL SHA-256 values are
`3e5f9fd57b7a19a51843db7585e2b4aebed0fc1b93b75856f482dec962805fe3`
and
`f75fd46059efcaade454d125b7643eb7a067b856f30570396663cf472443da37`.

## R3as TypedArray callback find promotion

R3as publishes `%TypedArray%.prototype.find`, `findIndex`, `findLast`, and
`findLastIndex` through one TypedArray-specific callback kernel. Pinned
QuickJS validates the receiver and snapshots its length once, but reads every
value live over that original forward or reverse range. Shrink and detach
therefore produce `undefined` for disappeared slots without skipping callback
calls, growth cannot extend the iteration, later writes are visible, and
`find` returns the value captured before the matching callback. The
implementation deliberately does not reuse generic Array property traversal.

The exact atomic candidate is 158 paths / 300 variants. The two SpiderMonkey
staging paths load `sm/non262-TypedArray-shell.js`, whose unconditional
WeakMap dependency is unavailable; their four variants stay explicitly
deferred. The remaining 156 paths / 296 variants expand the cumulative gate
to 1,293 paths / 2,547 variants and reduce the exclusion ledger to 1,068
paths. Oxide and pinned QuickJS both pass 2,547/2,547, while pinned QuickJS
also passes all 4,669 variants in the unchanged expanded candidate.

The candidate, deferred, and promoted path-stream SHA-256 values are:

- candidate:
  `88049528555f5f985395612fcd92e90f447f147d5ea63efb9449a840c259933f`;
- deferred:
  `4faf20dabff85cc8ffdee8c8d0d8212d290c8f41b4ef38ea4fc7bf9c36e0f6cc`;
- promoted:
  `86de1d6f7e44e6d148bef24f86e24256df53b97ab90f3ad4a4be543f22d0ed4b`.

Their corresponding variant-key hashes are
`622062fc24a78be0b21f77cd9e0ede4fecd5f93cac8858b0db9f75220dbdb990`,
`29de30037c833b16b08d51c5e1f9ed476d2b57c29c30d0924854b270d765c7d1`,
and
`1304d6a4cee8a78cef45653c1b8247aa0400e8fe4fbdb34abac53c5bcd1e623f`.

The scoped profile, cumulative manifest, exclusion-ledger file, and
cumulative variant-key SHA-256 values are:

- `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`;
- `38fe4dd01e098bee2c646865039c49e989b079f66c88913fbf644b438279b8ac`;
- `a8e2e74492138119133cabf6dd7d5fd1133cb06ce259f88f8c777d857154c2ef`;
- `f689489da433d110e4fe32be1940d141751d4112341a0319a43a0df5a815eeca`.

The canonical scoped TSV/JSONL SHA-256 values are
`7b0d8183176cdc53a1e5502dba684e80fe40549758e0e44bd875a0258253a4ae`
and
`1ec975c7f5b60a81a9363dffea10faaa993ade9f385b14621062cb06d78e2538`.

The global profile still withholds broad TypedArray admission. All 296 newly
promoted variants remain fail-closed there as `unsupported-feature`, while
the two staging paths remain harness failures. A complete two-worker run
therefore reproduces R3ar byte for byte: 51,908 passes, no previous-pass
regression, and unchanged full TSV/JSONL hashes
`3e5f9fd57b7a19a51843db7585e2b4aebed0fc1b93b75856f482dec962805fe3`
and
`f75fd46059efcaade454d125b7643eb7a067b856f30570396663cf472443da37`.

## R3at TypedArray every/some promotion

R3at publishes `%TypedArray%.prototype.every` and `some` through a
TypedArray-specific forward callback kernel corresponding to pinned
QuickJS's `js_array_every` TypedArray branch. Receiver branding and initial
detached/out-of-bounds validation happen before callback-callability
checking; the shared initial OOB diagnostic is now calibrated to QuickJS's
`ArrayBuffer is detached or resized` message. The internal length is
snapshotted once, but each original-range integer index is read live without
`HasProperty` or numeric prototype lookup. Mid-callback shrink or detach
therefore passes `undefined` for disappeared slots, growth cannot extend the
range, and later writes or a fixed view that regrows are observed. Callback
arguments, `thisArg`, abrupt completion, and `every`'s falsy versus `some`'s
truthy short-circuit behavior follow the pinned implementation.

The exact atomic candidate is 93 paths / 185 variants. The single
`test/staging/sm/TypedArray/every-and-some.js` path is deferred as
`external:cross-realm`; `sm/non262-TypedArray-shell.js` also carries a hard
WeakMap dependency. That leaves one path / one variant explicitly deferred
and promotes 92 paths / 184 variants. The cumulative scoped gate expands to
1,385 paths / 2,731 variants, the exclusion ledger falls to 976 paths, and
Oxide and pinned QuickJS both pass 2,731/2,731. Pinned QuickJS also passes all
4,669 variants across the unchanged 2,361-path expanded candidate.

The candidate path-stream and variant-key SHA-256 values are:

- path:
  `dbbd4a7e6f601888070c0f56de9771942e4d2354d75a29ab70439df3517d61cd`;
- keys:
  `213e8b79b6447d17e562139b268ab87d7394ee6edebc755f4c4bbb31b9fe3ec4`.

The deferred path/key hashes are:

- path:
  `6189caae9a943a1fa5d65308b4bba02c25bba4af5d9e7e791da8820bd851b99f`;
- keys:
  `2b728d9962391b75d27de09d05010642a9919f826719497c55e40e3f03a3e2f2`.

The promoted path/key hashes are:

- path:
  `8ad580d2a9cb33a091e714f7f309fd6c814503bfcb251ccdfd3bbbf5f87bae88`;
- keys:
  `9144eaf7e8b0c6664fd082d639aa35c176ee34d3d1947452fad6523dabe22604`.

The scoped profile, cumulative manifest, exclusion-ledger file, and
cumulative variant-key SHA-256 values are:

- `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`;
- `e96748da96cf70a08e0e678e46db24de4bf724d4d9b1bdd2012bc733596fb117`;
- `14dbdcf4d3eda7f9f0c26dade127cfca2a7cea415c732770216bd7acb6d13939`;
- `17b39adb34d9ed0502713acea7e1e75228043d7462de366ffd67747f8677ddff`.

The canonical scoped TSV/JSONL SHA-256 values are
`830cd524c30d68581aa7a22052f7d25ff8580c3cecf66723a9ebf031ebc36be2`
and
`f328ef2fcb4462ca5468cecaf1e5cfc3e170e347b483d360cf849f3073d35ea1`.

Broad TypedArray admission remains withheld, so the complete vector is
byte-identical to R3as at 51,908/102,037. A fresh canonical two-worker run
confirms that its full TSV/JSONL hashes remain
`3e5f9fd57b7a19a51843db7585e2b4aebed0fc1b93b75856f482dec962805fe3`
and
`f75fd46059efcaade454d125b7643eb7a067b856f30570396663cf472443da37`.

## R3au TypedArray forEach promotion

R3au publishes `%TypedArray%.prototype.forEach` through the same
TypedArray-specific forward callback kernel corresponding to pinned
QuickJS's `js_array_every` TypedArray branch. Receiver branding and initial
detached/out-of-bounds validation still precede callback-callability
checking, the internal length is snapshotted once, and each original-range
integer index is read live without `HasProperty` or numeric prototype lookup.
Callback arguments, `thisArg`, and abrupt completion retain the shared
behavior. The `forEach` specialization instead discards each normal callback
result without `ToBoolean`, never short-circuits, and returns `undefined` only
after visiting the entire snapshotted range. Focused differentials lock the
exact `not a TypedArray`, `not a function`, and
`ArrayBuffer is detached or resized` diagnostics and their priority.

The exact atomic candidate is 45 paths / 89 variants. The single
`test/staging/sm/TypedArray/forEach.js` path is deferred as
`external:cross-realm`; its harness also has a hard WeakMap dependency. That
leaves one path / one variant explicitly deferred and promotes 44 paths / 88
variants. The cumulative scoped gate expands to 1,429 paths / 2,819 variants,
the exclusion ledger falls to 932 paths, and Oxide and pinned QuickJS both
pass 2,819/2,819. Pinned QuickJS also passes all 4,669 variants across the
unchanged 2,361-path expanded candidate.

The candidate path-stream and variant-key SHA-256 values are:

- path:
  `ee8af85d761e4da707fc72afc992e8c0e0b314782d0f879cff69845e66cc2bf6`;
- keys:
  `67f42550bd10879a86d2401c4048e30a833a6ccda375b0d41ed44287b575c2a5`.

The deferred path/key hashes are:

- path:
  `26efea2e4065acf3a5bf1d8dab6ed0a78df866e1d956f9e08c44644635a5239f`;
- keys:
  `e3ce2a05f163af4827c1fdad2c7535a2dfe7f46bbe27c3c0ed76a803650bf661`.

The promoted path/key hashes are:

- path:
  `dba18b09bd2a2bc35a9f716e9a371547757d6225d2433c524a45cd5b92ba7177`;
- keys:
  `e3c038e152bb843d9dd55e9d16f89ca6227ac690a1e6d378c78d26757a211c4f`.

The scoped profile, cumulative manifest, exclusion-ledger file, and
cumulative variant-key SHA-256 values are:

- `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`;
- `cb837c070ca771c4c9b29a60a7dab0f3d83866f2b7508a82b57a846a9253d1f9`;
- `58c132e168bbaea25271c4d3dd7c6161b031d5fd883054e4aaf720eab999810d`;
- `446625e6284b989b8a18fb54064778ebbf471172cb0ed6caf0c3950f4e2f19a5`.

The canonical scoped TSV/JSONL SHA-256 values are
`50765aa252be5e634181d870dadafe8a7971f812a492f2c58d7878d1425ca3c8`
and
`8ded861e362fe5cc5b276d843aee0c4d8cc93e47db657593da0008e9289afb0d`.

Because broad TypedArray admission remains withheld, a fresh canonical
two-worker rerun confirms that the complete vector remains byte-identical to
R3at at 51,908/102,037, with full TSV/JSONL hashes
`3e5f9fd57b7a19a51843db7585e2b4aebed0fc1b93b75856f482dec962805fe3`
and
`f75fd46059efcaade454d125b7643eb7a067b856f30570396663cf472443da37`.
This is the confirmed no-transition join.

## R3av TypedArray reduce/reduceRight promotion

R3av publishes `%TypedArray%.prototype.reduce` and `reduceRight` through a
TypedArray-specific accumulator kernel corresponding to the TypedArray branch
of pinned QuickJS's shared `js_array_reduce`. Receiver branding and initial
detached/out-of-bounds validation happen before callback-callability
checking; callback validation happens before distinguishing an explicit
initial value from an omitted one and before the omitted-empty `empty array`
error. Explicit `undefined` is therefore retained as an accumulator. Without
an initial value, `reduce` seeds from the first element and `reduceRight` from
the last, then traverses in their respective directions.

The internal length is snapshotted once, but each remaining original-range
index is read live without `HasProperty` or numeric prototype lookup. Shrink
or detach therefore supplies `undefined` without skipping an index, growth
does not extend the range, and callback writes are visible later. The callback
receives `(accumulator, value, index, receiver)` with `this = undefined`;
normal results become the next accumulator, while arbitrary accumulator
values, callback throws, and cross-realm object/error identity are preserved.

The exact atomic candidate is 105 paths / 209 variants, all passing in pinned
QuickJS. The single
`test/staging/sm/TypedArray/reduce-and-reduceRight.js` path is deferred as
`external:cross-realm`. That leaves one path / one variant explicit and
promotes 104 paths / 208 variants. The cumulative scoped gate expands to
1,533 paths / 3,027 variants, the exclusion ledger falls to 828 paths, and
Oxide and pinned QuickJS both pass 3,027/3,027.

The candidate path-stream and variant-key SHA-256 values are:

- path:
  `f40c52a2edb4635d7ca1ec1a2b0abfa4c978c51a73ae567b8efffd8ab5d87ad5`;
- keys:
  `6cc0b62d9fe01cdaacf629a3152ca09b975ada81b4169bad7ffb05714662fe72`.

The deferred path/key hashes are:

- path:
  `b99151319be2a66b2d78111bff0ea5e73a308313670a1b4e9488a3afefd6f909`;
- keys:
  `97e3f4dbb189808dc1dd6cb9f8be100c74edbbb333e4c890c165cb7409fdf6cb`.

The promoted path/key hashes are:

- path:
  `79f2ce5172ba5afc48a87a3417ce99010762ba9de2cc3c49dd4db7696d6ba7b6`;
- keys:
  `79522bed3692d0c21ac44370796b6c37861dca2fab511d38d8872605e78d9fff`.

The scoped profile, cumulative manifest, cumulative variant-key stream,
exclusion path stream, and exclusion-ledger file SHA-256 values are:

- `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`;
- `b12b213d5b0d279bf3fdb328cba831a404fd0f4bc2bc105b1da6aa077c5508c7`;
- `06eceaa517e89f94217d85698d1618f1f297f9e8789f8bc42d7034753dff1e95`;
- `b5f0caf421df10d9958b1d6de4e8d10462a6e89d51b3492a707c0f5a5a83a2a0`;
- `4c6158d8cdb8fbde441e30f9820403912cbbb6f7b57f2af27b5f6c99bfaecca2`.

The canonical scoped TSV/JSONL SHA-256 values are
`089be9fab5e932b0003c99df8d70064591e35abe2f184ce0a01a575f7ee2c5e8`
and
`5ff2a426b2df285afa4eda8e9abb62dc192b52621a89f2234de475a242f99392`.

Broad TypedArray admission remains withheld, and a fresh canonical two-worker
rerun confirms that the complete vector is byte-identical to R3au at
51,908/102,037. Its full TSV/JSONL hashes remain
`3e5f9fd57b7a19a51843db7585e2b4aebed0fc1b93b75856f482dec962805fe3`
and
`f75fd46059efcaade454d125b7643eb7a067b856f30570396663cf472443da37`.

## R3aw TypedArray map/filter promotion

R3aw publishes `%TypedArray%.prototype.map` and `filter` through the existing
TypedArray callback dispatch while isolating species construction in a small
shared seam. Both methods validate the branded receiver and its initial
detached/out-of-bounds state before checking callback callability, then
snapshot the original internal length and read each original-range element
live without `HasProperty` or numeric prototype lookup.

`map` resolves and constructs the target species before the first callback.
Each callback result is converted and written immediately, so destination
detach or resize still observes conversion before an out-of-bounds write is
dropped. `filter` instead creates an ordinary hidden Array in the method's
defining realm before callbacks, records selected source values there, and
does not consult `constructor` or `@@species` until traversal completes. It
then invokes the result's public `.set(hiddenArray)` method even for an empty
selection. Pinned QuickJS performs no up-front Number/BigInt content-type
check: custom cross-content species succeed or throw only through their
observable write path.

The exact atomic candidate is 175 paths / 349 variants, all passing in pinned
QuickJS. The single
`test/staging/sm/TypedArray/map-and-filter.js` raw path is deferred as
`external:cross-realm`; it also depends on the SpiderMonkey shell's WeakMap
state. That leaves one path / one variant explicit and promotes 174 paths /
348 variants. The cumulative scoped gate expands to 1,707 paths / 3,375
variants, the exclusion ledger falls to 654 paths, and Oxide and pinned
QuickJS both pass 3,375/3,375.

The candidate path-stream and variant-key SHA-256 values are:

- path:
  `2a4d0d92c7a4b3aec6e559770bd3baa5780b2c3780f408333526619dfbfef9fc`;
- keys:
  `9e51d82281ea14f0568b2116054927aca5187708584e68b8cf551426f7529743`.

The deferred path/key hashes are:

- path:
  `198ede24f4c8a6e1dbb4135a14906c9f8a513178a42f23545711651eeaf26e31`;
- keys:
  `c7140d02e8e9d00feedd33ff35c98afa0a1bf365db3dd6ede640f1a8b34c6bd3`.

The promoted path/key hashes are:

- path:
  `57a0d825fa96ae56a44dd64be290d6368838d90fcd5cdd739c9735573b8d2a02`;
- keys:
  `b92f4b302934a05ca68f39bde019ef71f2353a664f3e304f2092ccf1eb8cf78b`.

The scoped profile remains unchanged. Its hash, followed by the current
cumulative manifest, cumulative variant-key stream, exclusion path stream,
and exclusion-ledger file SHA-256 values, is:

- `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`;
- `e6a3af181bf643b70558661802544681ac92356f06c4c27c9b1504b31379b42f`;
- `6bf48fc08165d42f32ff8ed7cf08ad94249b23daaf111cc3700df248c667b075`;
- `b2406a45aab98366342205bf4fb5149091b802500dc09b5a6afb8a1ef784c774`;
- `1c3d6f79c99f423c77c11256d65993143b4fced944f700f64b16975ffb730298`.

The canonical scoped TSV/JSONL SHA-256 values are
`05080ac47b8b5be9cc0d8ab70ed7f2233c843c54e42bac54ea8eb7f92a7d206c`
and
`439fdf6994613b1f945e7bbd5a02ccd9326dd474c28fa54db45e82d5e208322d`.

Broad TypedArray admission remains withheld. The exact canonical full-vector
join retains all 102,037 keys and every previous pass. Only the sloppy/strict
rows for `filter-species.js` and `map-species.js` change, producing four
`fail-runtime -> pass` transitions with zero missing, extra, or detail-only
rows. The vector reaches 51,912 passes and 430 runtime failures; its full
TSV/JSONL hashes are
`432394a9db53afd584a532b969382af167f0b17e42f77c8effd930a50389dfeb`
and
`d4a7540e05ba0cbcea9b7d94a8c2a6c7c7dea51613b7dcafd90c71e0983ba356`.

## R3ax TypedArray slice/subarray promotion

R3ax publishes `%TypedArray%.prototype.slice` and `subarray` from a dedicated
copy/view module. The implementation follows the pinned QuickJS algorithms
rather than generic Array copying:

- `slice` validates and snapshots before bound coercion, creates the species
  result with one length argument, and performs its post-species source/target
  validations only when the original count is nonzero;
- a length-tracking source that shrinks during species construction clips only
  the live copy count, leaving the originally sized result tail untouched;
- same-class copying preserves raw NaN payloads and negative zero, including
  QuickJS's forward byte-copy behavior for overlapping same-buffer species
  views; cross-class copying reads and converts each element live;
- `subarray` performs an initial brand check without rejecting an OOB/detached
  source, retains its durable raw byte offset, and passes two constructor
  arguments for an automatic length-tracking view or three for a fixed view;
- default species allocates the source element class with the method defining
  realm's intrinsic prototype, while custom subarray species may return any
  live TypedArray without a minimum-length or content-type check.

The exact atomic candidate is 178 paths / 356 variants, all passing in pinned
QuickJS. Five raw SpiderMonkey staging paths / ten variants remain deferred:
three are `external:cross-realm` and also depend on the shell WeakMap, while
two are `external:WeakMap`. The promoted set is therefore 173 paths / 346
variants. The cumulative gate expands to 1,880 paths / 3,721 variants, the
exclusion ledger falls to 481 paths, and Oxide and pinned QuickJS both pass
3,721/3,721.

The candidate path/key hashes are:

- path:
  `b47079faf02e6e29ab9b1d1da45d35d79f30f1498fff96ea47c3d0fdf4057417`;
- keys:
  `d149931f862e672317077644ffae6ccc6e319442a97dbb2a951bb1cdaeed8769`.

The deferred path/key hashes are:

- path:
  `9f1d0a737704df4c1503cecd69ec953faae2496fa6da4bff07d36b35b377c328`;
- keys:
  `c991213141a15cd3e647dd9b1c40553c5dc0a709f5ebfbd10e30769683e7eb37`.

The promoted path/key hashes are:

- path:
  `a6f25c6d1af227a6f656284a2f3c833e4320caea80e7029fc376eb066e01584e`;
- keys:
  `103222ebda62afb2a76d6b9efc6fefa0c086707509607f58a24b6a73a5f1cb1b`.

The scoped profile, cumulative manifest, cumulative variant-key stream,
exclusion path stream, and exclusion-ledger file SHA-256 values are:

- `08dda435c36df9b647ee575421d7d725df2d405fed9653b89d217231307167fc`;
- `3894d40cf21ca00f0b641b729c7562c65c5cb41d31bb4616b6d1ca8c3871b092`;
- `ba80d9ddfb13f4c8ff20098b267b592a4c0682a806f0b9ce3633f7f61a8c05d4`;
- `16ccf5fac0c47daa0626d26e25aa3d49e305e193f80e8148448d9d444addcf27`;
- `11616f23d68983bb517dff1d4563f060d0ae3955941e66a681d0a9ab4be5b565`.

The canonical scoped TSV/JSONL hashes are
`88d9061e2d31b2869f7d71b0cda7a0cd059c8d7cf346de967eeabc572fe24aff`
and
`e36ef63eac28058534553577595b947a044ebd61d177e4a1704eab415bcb3ba0`.

Broad TypedArray admission remains withheld. Two independent canonical
two-worker full runs are byte-identical, retain all 102,037 keys, and preserve
every previous pass. Ten sloppy/strict staging rows move from `fail-runtime`
to pass: `slice-conversion.js`, `slice-detached.js`,
`subarray-species.js`, `TypedArray-subarray-arguments-detaching.js`, and
`typedarray-subarray-of-subarray.js`. No other outcome or detail changes. The
complete vector reaches 51,922 passes and 420 runtime failures; its full
TSV/JSONL hashes are
`796783147bae745b1cbb21eb2cf211feefcb98e80008f760eed8f18eb84f7641`
and
`e912ed7dc3f9a9f0141f9c96168fb8bb5e4be4661d6d47030295427a21baf4aa`.

## R3ay TypedArray with/toReversed promotion

R3ay publishes the non-species change-by-copy methods
`%TypedArray%.prototype.with` and `toReversed`:

- `with` snapshots the old length, computes its relative index from that
  snapshot, and performs index conversion followed by the replacement's
  number-hint `ToPrimitive` before checking the live view;
- after a resizable-buffer shrink, index validity uses the current length while
  result allocation retains the old length; missing numeric tail values convert
  from `undefined`, while the corresponding BigInt path throws;
- `toReversed` clones same-class element words and reverses those raw words,
  preserving NaN payloads and negative zero;
- both methods ignore public constructor/species overrides and allocate through
  the builtin defining realm's default TypedArray prototype;
- the shared constructor-clone helper owns QuickJS's common
  `js_typed_array_constructor_ta` validation/copy seam, while adjacent `at`,
  `reverse`, and same-class constructor OOB errors use canonical QuickJS text.

The exact candidate and promoted set is 34 paths / 68 variants, with no
deferred path. Oxide and pinned QuickJS both pass 68/68. The cumulative gate
therefore expands to 1,914 paths / 3,789 variants and the exclusion ledger
falls to 447 paths.

Candidate and promoted path/key hashes are both:

- path:
  `e212ba0d3d9c819403d3d226f23a735ff2bb9b746618fff779e2654a39f5fddb`;
- keys:
  `6d341ea9896a878f9beea36e477e96227642812a1cded595620a6de0f76e7723`.

The empty deferred path and key streams both hash to
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

The scoped profile, cumulative manifest, cumulative variant-key stream,
exclusion path stream, and exclusion-ledger file SHA-256 values are:

- `07837fd2bdb1cf5f300163c483b611d0862955c7976de5f385faebe1b4dd7ac1`;
- `1237074662d16674a5ea23f6a2bed26ee3126358f7fb80949846f2329f2ce318`;
- `c6d46821eae8f1affec571a38c5dfd074aa1774ef36df2a78e47db554e151e02`;
- `d8842f1aeedb8d42ce551c72c15a433c6d776c44f2abe39e789dfea82b24c348`;
- `aaca7878d12694635eb5f65d9ae53f9000aafba5e647eb88365663683fdc07fc`.

The canonical scoped TSV/JSONL hashes are
`19ab4f7385457ea72e47c7e3b5ba7031d0a0cdffbbd2db8825d1685230b92ce1`
and
`09d1a226a84e10f39cc5228037eddb5a1af5c2eee64664b45bf9f2407e27dd96`.

Broad TypedArray admission remains withheld. Two independent canonical full
runs are byte-identical and retain all 102,037 keys and every previous pass.
Only the sloppy and strict `test/staging/sm/TypedArray/with.js` rows move from
`fail-runtime` to pass, with no other outcome or detail movement. Replacing
those two rows with their R3ax records reconstructs the R3ax canonical
TSV/JSONL hashes exactly. The complete vector reaches 51,924 passes and 418
runtime failures while runnable remains 52,468; its full TSV/JSONL hashes are
`73141c5f26f9e3f132b0046c1066a7d5965497c27754e1b4ec89b5649e8ba7a9`
and
`b69db1a2c29dfdb7e0196fc2e452591a1d25316fd9ec449ef24cdbdd7d2f5481`.

## R3az TypedArray stringification promotion

R3az publishes `%TypedArray%.prototype.join` and `toLocaleString` through the
pinned QuickJS dedicated TypedArray stringification path, while retaining the
inherited `toString` alias:

- both methods validate the branded source and snapshot its old length before
  traversal;
- an explicit `join` separator is stringified before the live length is
  re-read; a shrink or detach leaves blank old-length tail slots, growth is
  ignored, and separator overflow stops before a later element conversion;
- `toLocaleString` ignores every argument, uses a comma separator, invokes
  each live primitive element's builtin defining-realm method with zero arguments, and
  then stringifies the returned value;
- neither method consults ordinary indexed properties or the public `length`
  property, and the implementation does not reuse generic Array traversal.

The exact atomic candidate contains 88 paths / 175 variants. Five paths / nine
variants remain deferred:

- `test/staging/sm/TypedArray/join.js` and
  `test/staging/sm/TypedArray/toLocaleString.js` account for two paths / three
  variants requiring both `$262.createRealm` and the SpiderMonkey WeakMap
  shell;
- `test/staging/sm/TypedArray/toLocaleString-detached.js`,
  `test/staging/sm/TypedArray/toLocaleString-nointl.js`, and
  `test/staging/sm/TypedArray/toString.js` account for three paths / six
  variants requiring the WeakMap shell.

The remaining 83 paths / 166 variants are promoted. The cumulative scoped gate
therefore expands to 1,997 paths / 3,955 variants and the exclusion ledger
falls to 364 paths. Oxide and pinned QuickJS both pass 3,955/3,955.

Candidate, deferred, and promoted path/key SHA-256 pairs are respectively:

- candidate path:
  `d968b61ff553acb2654f2904a9afff46660f43d6848ad7496ff28f18a81b8d4b`;
- candidate keys:
  `81131955a7d4ef4b2358965cd0691498bb78abfac7c48d0f60b8aafcdbbe81f1`;
- deferred path:
  `0254c5edb9969e43038d03dd42f9d43fd29c10c647673cd63cb4230bc8c53151`;
- deferred keys:
  `092d6f18a34c2dd23f7add4d9a73a5c1c14e63f99c6fd91f70c8a2c050edc44c`;
- promoted path:
  `ae64162fb7742828d9dc45d5f54e4666887c4ac95499bbfbe8622ae6fc875b89`;
- promoted keys:
  `0fe599bb568d384f84657000208d47df7b7ffa1d3133b6d2795abafa06bf00f6`.

The scoped profile, cumulative manifest, cumulative variant-key stream,
exclusion path stream, and exclusion-ledger file SHA-256 values are:

- `173f0f6f33966a97c8ef65d55f261e5cf1b9c2ee68d1acf2adca92a48d16eb4b`;
- `00f63843eda645f8701e678663f505ae3004574110f3ccb5fb78e12a94ee98cb`;
- `b6b16404066ac2e03815b38fd55bbc62d70066ee50e5696687e15a3e8d4a0bfe`;
- `e11790d0921680b55ba8f5c47a1bd4d7f1254107ea2c05c5f75f51319b578c17`;
- `432e55cc4bccbdad68f90b7556f89aaf704141e0f4b64242964fcd0ad2853575`.

Canonical scoped TSV/JSONL hashes are
`623401f1ee46bb26a6313d26dd71a408e19f58a64bef95bb537428ad19f018bd`
and
`4983527238f6d436e8c01d40ca707514f3847a879b4c2fcda64aedaa1f986552`.

Broad TypedArray admission remains withheld. Two independent canonical full
runs are byte-identical and retain all 102,037 keys and every previous pass.
Only the sloppy and strict
`test/staging/sm/TypedArray/detached-array-buffer-checks.js` rows move from
`fail-runtime` to pass; there is no other outcome or detail movement. The
complete vector reaches 51,926 passes and 416 runtime failures while runnable
remains 52,468. Its full TSV/JSONL hashes are
`bd1119fe3ea8e4eaaad2e21bf3d0991b58200bacef91e695c2c2a4c11e6538c3`
and
`d78dfbd84ebab70441362d6bd535fab9fcfc433419b09fe8668309a749e7c759`.

## R3ba TypedArray sort/toSorted promotion

R3ba publishes `%TypedArray%.prototype.sort` and `toSorted` through the pinned
QuickJS `rqsort` comparison choreography. Default comparison operates directly
on backing-store words and sorts in place with O(1) auxiliary storage; numeric
ordering, NaN placement, signed zero, BigInt values, and raw payload bits do
not pass through ordinary property or string conversion. Custom comparison
instead snapshots exact raw bytes and a `u32` original-index vector, decodes
each callback pair from that immutable snapshot, uses original position as the
stable tie-break, and writes the selected raw words back only after the sort
finishes successfully.

The two entry points deliberately retain QuickJS's different error order.
`sort` validates a supplied comparator before receiver branding and
detached/out-of-bounds validation. `toSorted` brands and copies first, then
validates its comparator; it creates fixed same-class storage in the builtin's
defining realm and never consults `constructor` or `@@species`. During custom
`sort`, comparator-driven detach or final out-of-bounds state suppresses
writeback, shrink clips it to the final live prefix, growth does not extend the
old snapshot, and callback throws retain identity. Array and TypedArray sorts
share a catchable 16-entry native recursion family, including alternating
comparator reentry.

The exact atomic candidate contains 64 paths / 128 variants. Six paths / 12
variants remain deferred:

- `sort-negative-nan.js`, `sort_byteoffset.js`, `sort_errors.js`, and
  `sort_globals.js` require the unavailable cross-realm shell;
- `sort_large_countingsort.js` and `sorting_buffer_access.js` require the
  SpiderMonkey WeakMap shell.

The remaining 58 paths / 116 variants are promoted. The cumulative scoped gate
therefore expands to 2,055 paths / 4,071 variants, the exclusion ledger falls
to 306 paths, and Oxide and pinned QuickJS both pass 4,071/4,071.

Candidate, deferred, and promoted path/key SHA-256 pairs are respectively:

- candidate path:
  `d06f1655781895a7f77a5ae378e25920e4cf62c87134a1cabaaa0418bfb8a0b8`;
- candidate keys:
  `53e35176074fdfdd0c414d30b9365995b0d420f43a2e45c420955cc0fc1d6de9`;
- deferred path:
  `0067268a56e709b6be94b51b1a7472b961a27f9a99e623a6cce6d04ed4cf1b96`;
- deferred keys:
  `f242add5304bef7ba11b82181cc1646b5a1ea970f06ee38d857d4c65f144ecfd`;
- promoted path:
  `1efa5ed5b57d0638963f183b0294e5dc90b711b754c63aa50b79cd34f3e0d3d4`;
- promoted keys:
  `b76f083344a23bdb330cdec16aa22f07175fb151f374858a77bbf3cc48e624c1`.

The scoped profile, cumulative manifest, cumulative variant-key stream,
exclusion path stream, and exclusion-ledger file SHA-256 values are:

- `8261eff7f79ebc2b724cf42c0853d8f74336ac23eccfa862172bcbca2f918a3e`;
- `fa6f12f165793a00c4fc987ebaa043e9090c694dc2d77fc3b7ba670a3639e0cd`;
- `6ecf7cb35ecb89cb831b43db6d778f4f2b8a4432c83c8d7a08d396c36fb7e65b`;
- `8bb730391734446ade26ae9835772a7bd4493d4cb6fa9f97a8b6a2e5dbd30000`;
- `fad925fb491f4a1c5e55ab1ca54ce6dd46e189e655c5f2d7c145981d1d2d1178`.

Canonical scoped TSV/JSONL hashes are
`5db3782454d4556687a918c946a676b8708b5e4f0be7e9edd84e25700a258629`
and
`8948ee41244d744c6099868b86bbca8dfc88d7cea9865d5e58b6eb86492cb8f9`.

Broad TypedArray admission remained withheld at R3ba. That milestone's
canonical full measurement retains all 102,037 keys and the 52,468 runnable
count, moves 14 runtime failures to pass, and leaves every other summary
category unchanged.
It reaches 51,940 passes and 402 runtime failures. Two independent formal
two-worker runs reproduce the measured full TSV/JSONL hashes:
`f9944fe74a9eee0330a9f4681e3064cba5fc70e00b4fc7eef73fcbce6f709b07`
and
`8cc3f8420e290d3094a21bee23a10e26c2cb2e860228d3f98a2bda80c5eb1390`.
At the R3ba landing, the next milestone was a residual TypedArray audit.

## R3bb TypedArray entries/keys authentication

R3bb authenticates `%TypedArray%.prototype.entries` and `keys` without a
production-code change. Both entry points already use the shared Array
iterator, whose per-`next` length recheck and integer-indexed read match pinned
QuickJS. Three frozen/self-check/differential observation tests cover all 12
concrete TypedArray classes, resizable-buffer shrink/grow, detach,
transient-OOB recovery, and iterator completion. A fourth Rust cross-realm
structural regression locks the separately source-audited
manual-next/outer-operation realm split.

The exact candidate contains 46 paths / 92 variants. Three paths / six
variants remain deferred:

- `test/staging/sm/TypedArray/entries.js` and
  `test/staging/sm/TypedArray/keys.js` require both the unavailable
  `createRealm` and WeakMap shell;
- `test/staging/sm/TypedArray/prototype-constructor-identity.js`
  required WeakMap and the then-missing Uint8Array codec surface at R3bb.

The remaining 43 paths / 86 variants are promoted: 42 `entries`/`keys` paths /
84 variants plus the two-variant
`test/staging/sm/TypedArray/detached-array-buffer-checks.js` canary. The
cumulative scoped gate therefore expands to 2,098 paths / 4,157 variants, the
exclusion ledger falls to 263 paths, and Oxide and pinned QuickJS both pass
4,157/4,157.

Candidate, deferred, and promoted path/key SHA-256 pairs are respectively:

- candidate path:
  `45cfe102015cb7c25b3b2b064853c16c3e30d2f5c655bd3983a686689ca2540e`;
- candidate keys:
  `239f0a0f477d2d26f59b4247714e9dc2785bf5afac3adc8ea8a619067f299b4d`;
- deferred path:
  `bc0552a01cb1a8561461fe3bc6e82b3ed7a432599f16889a5c1e324552456a2d`;
- deferred keys:
  `4eb2eaecfec843385d2cb7562f278b17dae9d7cf20b61f8365f3fa734bc3b1c6`;
- promoted path:
  `029c249f88eb6a61f988495ea00e3455ca9878611e2c26e4c6b768faf0867d22`;
- promoted keys:
  `92e7b6ed05c315c3a6bc83e791e49a9880543247cf08bc5a329c6ffe0c2777ac`.

The scoped profile, cumulative manifest, cumulative variant-key stream,
exclusion path stream, and exclusion-ledger file SHA-256 values are:

- `67c4d0804fb606d052a9f62b10e069952538398f0f22ebc911083bc5bd5a8a5f`;
- `3fc805b6745d0bb464f0ad9831ee7e21e536eabe3d5a84006d458bb6ef30d85e`;
- `c85de1ea3355fb3bc31fbc93e2ea862d7f2d0ce882f7e7a00c011519f51f4516`;
- `156725665a6c47fe5b0b85bd0cb0ba1a7bc380b7212689ca4497b5123cf40dd0`;
- `107032e49012a49664cfef27730e5497e986b6090e150afc5d26fc1774b3a1aa`.

Canonical scoped TSV/JSONL hashes are
`60ab18160baec6cdb57fecb56e6900685ea66de54610cab339008d1a1b562d5d`
and
`b00eba7643be86cf1ca0bb7ccfa94feed1f860d4db8d8756d0551c91ae07a8aa`.

The complete vector is unchanged by construction. The global capability
profile does not change, all 84 newly authenticated `entries`/`keys` rows
remain `unsupported-feature`, and the two
`detached-array-buffer-checks.js` rows already pass in the R3ba baseline. The
canonical full TSV/JSONL hashes therefore remain
`f9944fe74a9eee0330a9f4681e3064cba5fc70e00b4fc7eef73fcbce6f709b07`
and
`8cc3f8420e290d3094a21bee23a10e26c2cb2e860228d3f98a2bda80c5eb1390`.
At the R3bb landing, the next audited slice was static TypedArray `of`.

## R3bc TypedArray static of authentication

R3bc authenticates the inherited `%TypedArray%.of` surface against pinned
QuickJS. The implementation constructs through its receiver with exactly one
length argument and the receiver as `newTarget`, validates the live TypedArray
result and minimum length, then performs left-to-right element conversion and
integer-indexed writes. The focused matrix covers all 12 concrete classes,
Number and BigInt conversion, custom/bound/Proxy constructors, returned class
mismatches, partial mutation before abrupt conversion, RAB shrink/grow,
detach, zero arguments, and a safe 512-argument call.

This milestone includes a production semantic fix rather than bookkeeping
alone. Primitive receivers (`undefined`, null, Boolean, Number, String, BigInt,
and Symbol) now take the direct static-constructor seam and produce pinned
QuickJS's defining-realm `TypeError: not a function`. Object receivers still
take the ordinary constructor check and produce `not a constructor` when
appropriate. Static `TypedArray.from` shares this seam in both its iterable and
array-like branches while retaining its earlier observable work: map-function
validation precedes source access, and receiver validation occurs only after
iterator materialization or the array-like length read. Species construction
remains intentionally separate, preserving `not a constructor` for a primitive
`@@species`.

The oracle boundary is explicit. `tests/oracle_typed_array_of.rs` contains
seven frozen vectors and four Rust test entry points:

- the first freezes Oxide against observations taken from QuickJS;
- the second self-checks those observations by executing pinned QuickJS;
- the third directly differentials Oxide against pinned QuickJS;
- the fourth is a Rust-only cross-realm structural test covering result
  prototype ownership, defining-realm native errors, and caller-thrown value
  identity.

Only the first three entries belong to the QuickJS observation/oracle/
differential layer. The fourth does not execute QuickJS and is not counted as
a QuickJS differential.

The exact atomic candidate contains 35 paths / 70 variants. The sole deferred
path is `test/staging/sm/TypedArray/of.js`, whose two variants require both
`$262.createRealm` and the SpiderMonkey TypedArray-shell WeakMap. Pinned
QuickJS passes all 70 candidate variants. The remaining 34 paths / 68 variants
are promoted, expanding the cumulative scoped gate to 2,132 paths / 4,225
variants and reducing the exclusion ledger to 229 paths. Oxide and pinned
QuickJS both pass 4,225/4,225 admitted variants.

Candidate, deferred, and promoted path/key SHA-256 pairs are respectively:

- candidate path:
  `6fdec16ab63ca0b1081a90f7a5f12fa6c87b6c73fdb209079d24bf793d2787b8`;
- candidate keys:
  `3bfcf9a16f2c28c819d121a819f7c52882e34fb3a3443ebb6c66db0bdbcc25a7`;
- deferred path:
  `2b66ebd26cc79b9df0d5e5771e665d164311633010ea66eb33a22e85d6d62a0e`;
- deferred keys:
  `07a640bcebe1fc380bde8bd0ab1a3b80779d4e45b085a744018a50858c016140`;
- promoted path:
  `01095b2e0348fb1328026684c7422975cf8396a08fa73719955c9350ee15f13f`;
- promoted keys:
  `8318904a86586b2bc771200348972ffd59c6f84b61219d84b262668517c363df`.

The scoped profile, cumulative manifest, cumulative variant-key stream,
exclusion path stream, and exclusion-ledger file SHA-256 values are:

- `c7118e34b64929bd57678ac490fb5793a3e6974fb4272e09633614d424fe4ef7`;
- `3334625f2df7a60c7541884f14f5b001e2f0eadbafdb85529eb5018b9eb0f4d8`;
- `1fb72c0d146a365b8ff7eee5eeca291d0aa1af97b786f02f05011a89cd694ec7`;
- `db842baa3b677f2e2312540bfb279e72fb56e6acaa390bd5ee602e0fc40bd371`;
- `be473162b0c73865415bf26bcfab36041139bb0f1684b8ecea5fe2065b995267`.

Canonical scoped TSV/JSONL hashes are
`a0f5531d24e57b3da8af70ba865b2aa9764f64973489da07d812a80d92dbecab`
and
`3f4f16ca175e057f063cfb4d917bdadd31c66e421edb60c97e7900cbca41cf50`.

The conservative full vector remains 51,940/102,037. The global profile is
unchanged, and an exact current-row audit shows that all 68 promoted variants
remain `unsupported-feature`, so the checked-in full TSV/JSONL hashes remain
`f9944fe74a9eee0330a9f4681e3064cba5fc70e00b4fc7eef73fcbce6f709b07`
and
`8cc3f8420e290d3094a21bee23a10e26c2cb2e860228d3f98a2bda80c5eb1390`.

Resource parity remains a declared caveat. Oxide keeps an extra O(argc) cloned
native argument vector where QuickJS reuses VM `argv`; direct TypedArray
allocation topology and the BigInt write path also differ internally. The
focused safe-large case stops at 512 arguments and does not inject allocator
failure, so this slice does not certify identical OOM ordering or thresholds.

At the R3bc landing, the next static `from` inventory contained 90 paths / 175
variants in total: 81 paths / 158 variants were promotable and nine paths / 17
variants were dependency deferrals. This wording corrects the earlier
ambiguous “81 candidate” shorthand.

## R3bd TypedArray static from authentication

R3bd authenticates inherited `%TypedArray%.from` against pinned QuickJS's
`js_typed_array_from`, `js_typed_array_create`, and
`js_array_from_iterator` paths. Map-function callability is checked before
source access. The iterable branch caches and drains the iterator before
constructing the target; the array-like branch reads and converts length before
construction and leaves indexed property reads live. Both then map, convert
using the actual returned TypedArray element class, and write left to right.
Static construction bypasses species and validates a live result with at least
the requested length.

This milestone fixes two real parity details. `undefined` and null sources now
produce QuickJS's exact defining-realm
`cannot read property 'Symbol.iterator' of ...` TypeErrors without moving the
earlier invalid-map-function check. Oxide also traverses its fully materialized
`Vec<Value>` with `iter().cloned()` so every original yielded object remains
rooted throughout mapping and writing, matching the lifetime provided by
QuickJS's hidden Array.

`tests/oracle_typed_array_from.rs` is 914 lines and contains eight frozen
vectors with four test entry points. The first three freeze QuickJS
observations, self-check those observations against the pinned executable, and
directly differential Oxide against QuickJS. The fourth is a Rust-only
cross-realm structural test: it covers result and error realm ownership,
sloppy/strict mapper `this`, and abrupt value identity, but does not count as a
QuickJS differential.

The exact atomic universe contains 90 paths / 175 variants. Pinned QuickJS
passes all 175. The promoted partition contains 81 paths / 158 variants:
all 79 standard built-in paths plus the independent SpiderMonkey
`from_string.js` and `from_typedarray_fastpath_detached.js` paths. The deferred
partition contains nine paths / 17 variants:

- seven staging paths have a hard dependency on the SpiderMonkey shell's
  missing WeakMap;
- `from_realms.js` has that shell dependency and independently needs
  `$262.createRealm`;
- the Annex B `iterator-method-emulates-undefined.js` path requires IsHTMLDDA.

SharedArrayBuffer in the shell is guarded by `typeof` and is not an additional
blocker for this partition. Candidate, promoted, and deferred path/key SHA-256
pairs are respectively:

- total candidate path:
  `87e7cfd69fbac9265f7e4a28ceaea8f21f053b7a587a95494becc7bbab61b20c`;
- total candidate keys:
  `041fc07db938e2bf21fd1135fdbb3be648e2e5f3bdbf5688dfdf78784ed505a4`;
- promoted path:
  `a75d6ebea395327340d498c6f4d5e2b2c4224c039f6c1a58e42b19d070e94e41`;
- promoted keys:
  `5ea8a30f1578a6160441c068c91384ea635e179a90c6804af23730cfec7f6f34`;
- deferred path:
  `7e466133fdeb876268cf10e629701daa332922d484d16ad76b58679aee3e47b6`;
- deferred keys:
  `df334b586f8ab8494ab8ec1d9a06d4492ae76b0fe0d73479637001f18ab3dd24`.

Promotion expands the cumulative gate to 2,213 paths / 4,383 variants, all
passing in Oxide and pinned QuickJS. The scoped profile, cumulative manifest,
and cumulative key-stream hashes are:

- `dd106c074751866ce667352d3449cc0ec7d9b9072034a4f0a97050da7b7bad13`;
- `d71be16dfcd42b58e3371c47d35d8f6cc9fbe29a11135ebd39ea447cb84d0c56`;
- `ac56a6047ecb71616e098b5cb6a0c449d11af21141f8f18af5ebe4dccefb9a84`.

The profile admits 27 feature tags with SHA-256
`de5b9c5c6a66566a6b1481fc0b014a6ef00a95ebecc90c37da4508aa85a8d830`
and 11 includes with SHA-256
`b1b60b5e1f7635615ff31eb139d1803608e5743c5f46ca53fadc3797e0abe012`.
The remaining exclusion ledger contains 148 paths: 71
SharedArrayBuffer, 54 cross-realm, 21 WeakMap, one IsHTMLDDA, and one Math.
Its path stream and complete file hashes are
`0d425a326fc950257410849ada4c2435b410e84f4c9651f9393c39f6d5c3032a`
and
`4c79c3c86364a5c0aa6d2ea5bf3cba6da47261d0b4847fbfeaa5cd368749b783`.
Canonical scoped TSV/JSONL hashes are
`de22c434d3ac28ed823a6c20c1bbc01a7e44e43e86e1a1b368696196b2399c1b`
and
`6f1904f5001deb1f96cd06d697def75999991350e582c3b69486246b1a68b460`.

Broad TypedArray admission remains withheld. A read-only exact join of the 158
promoted variants against the canonical full report records four existing
passes and 154 `unsupported-feature` outcomes: 142 are blocked only by
TypedArray, six by `Array.prototype.values` plus TypedArray, and six by
TypedArray plus resizable ArrayBuffer. The normalized full-row stream SHA-256
is
`fecefca50dcb3d97f321ba81fe8af1490bd74520b3d7327be142a882085023b7`.
The complete vector therefore remains 51,940/102,037, and its canonical
TSV/JSONL hashes remain
`f9944fe74a9eee0330a9f4681e3064cba5fc70e00b4fc7eef73fcbce6f709b07`
and
`8cc3f8420e290d3094a21bee23a10e26c2cb2e860228d3f98a2bda80c5eb1390`.

QuickJS's materialized values live in a hidden realm-local Array while Oxide
uses a Rust Vec. Their ordinary value lifetime now agrees, but hidden-Array
allocation, GC pressure, and injected-OOM topology remain an explicit
uncertified resource seam.

At the R3bd landing, the next step was a broad TypedArray global-admission
audit. Enabling only the `TypedArray` feature exposed 3,686 variants; 3,606
were already certified by the scoped manifest, leaving 80 spillover variants
across 41 paths to review.

## R3be TypedArray global admission

R3be adds exactly one tag, `TypedArray`, to the checksum-pinned global profile,
which now contains 80 reviewed feature tags. No adjacent concrete-constructor,
codec, SharedArrayBuffer, Atomics, module, or broad built-in tag is admitted.
The global profile SHA-256 is
`99ad7997a6328ab24f87af9575f9e8ddda76db81092c008d5a84e06a84a0c5ee`.

The frozen activation manifest contains 1,865 paths / 3,686 variants. Its
already-authenticated partition contains 1,824 paths / 3,606 variants, and its
disjoint spillover partition contains 41 paths / 80 variants. The activation
manifest and exact variant-key stream hash to
`44a9b901eb59f9dc41dde71e0595d2777f52814a864632e7e27bdd739654bdee`
and
`68b01ca00423a3e62a090ee8cac24d54b5866276de306b0c846e74d3663218e5`.
All 3,686 activation variants pass; canonical activation TSV/JSONL hashes are
`e663c9b957e7e061573cc42e092ddd7b06a4508cd2e67ba74919ad243239ab54`
and
`9db88feb1d2d79dd3f0abce8a818c1bffff67d79f1a77f671c3a5fdb8a1078fc`.

The complete TypedArray candidate is now 2,402 paths / 4,749 variants. After
the existing dependency exclusions, the cumulative scoped manifest admits
2,254 paths / 4,463 variants, all passing in Oxide and pinned QuickJS. Its
manifest, variant-key stream, and canonical TSV/JSONL hashes are:

- manifest:
  `91ac9a132c8099ecd15d3cfcfe160b21a1f7e9a083a5210a33406606270ad378`;
- keys:
  `e8e3c0d8f19343bbf0160c5af3239caa98fb7e01d006ff6b53f0d946a500e7cc`;
- TSV:
  `388d8f32ef0d7d0a8f2c86ac0931178d2d850335b80cf13fe81888930be5f38c`;
- JSONL:
  `e32b0abdcab0409491132690a4b22441791016ac57c83c1bcbdfd26c0a0b3c9d`.

A separate reason-only ledger contains 471 paths / 938 variants. Those rows
remain `unsupported-feature`: admission removes `TypedArray` from their
diagnostic, but each still declares at least one unsupported dependency. They
are therefore bookkeeping-only detail changes, not newly executed variants.

The checked-in 4,624-data-row R3bd-to-R3be transition receipt,
`tests/test262-typed-array-global-r3bd-r3be-transitions.tsv`, partitions
exactly into 3,686 `unsupported-feature -> pass` transitions and 938
`unsupported-feature -> unsupported-feature` reason-only changes. Its
complete-file and header-free data-row SHA-256 values are
`851ef0961a28532081f7b9dc281c305ea8839dd3b8ceed750d182da90b69eafd`
and
`26babcba92c23bb699f8fd3a2db7cce376fa868f5b3ca4081abc4148a90a4a57`.
The exact 102,037-key full join has zero other row changes and zero
previous-pass regressions. The complete vector reaches 55,626/102,037 passes
with 56,154 runnable variants; canonical full TSV/JSONL hashes are
`bdeb287ea6f74baefa0eb034773aa57f7c87f9ecaa6d2af20f27a6ea94b53693`
and
`916fbebcb964be779138ca6ad588d14b9cf3e55c0f22b4aaeb474739bdb74ece`.

R3be changes no production runtime code. `runtime.rs` remains 9,950 lines and
`heap.rs` remains 23,026 lines. Four focused `with` tests include pinned
QuickJS differentials for the `with`-statement spillover.
At R3be, Uint8Array codecs, modules, SharedArrayBuffer/Atomics, and broad
built-ins remained explicit frontiers; R3br/R3bs later close the codec item.

## R3bh Proxy global admission

R3bh adds exactly one tag, `Proxy`, to the checksum-pinned global profile,
which now contains 81 reviewed feature tags. The profile SHA-256 is
`2bfad693206dd09934a4c95ca241c49c4997ad795b8f0016571aada9c2cf1804`.
No adjacent module, host, SharedArrayBuffer/Atomics, or other broad built-in
capability is admitted.

The Proxy-only activation partition contains 405 paths / 787 variants. Every
variant passes. A disjoint reason-only partition contains 21 paths / 42
variants which remain `unsupported-feature`: removing `Proxy` from their
diagnostic exposes another unsupported dependency, so these rows change only
bookkeeping detail and do not become runnable.

At R3bh, the historical R3be TypedArray gate began reconstructing its 80-tag
feature side from a checked-in inventory. Its 802 audited negative paths still
came from the growing global profile, so feature admission was decoupled but
negative-provenance admission was not. R3bj below closes that remaining
historical coupling.

Four already-exposed Test262 variants remain deliberate pinned-target
deviations:

- `test/staging/sm/object/defineProperties-order.js` in sloppy and strict mode;
- `test/staging/sm/regress/regress-1383630.js` in sloppy and strict mode.

QuickJS 2026-06-04 fails the same four variants. The first observes QuickJS's
batch descriptor-enumerability snapshot order in `Object.defineProperties`;
the second observes its incomplete fixed-descriptor compatibility check in a
Proxy `getOwnPropertyDescriptor` trap. Focused differential regressions freeze
those target behaviors. R3bh therefore does not change the runtime to satisfy
the conflicting Test262 expectations at the cost of QuickJS feature parity.

The exact complete vector reaches 56,413/102,037 passes with 56,941 runnable
variants and 21,703 `unsupported-feature` outcomes. Its canonical full
TSV/JSONL SHA-256 values are
`b634753cd21d2ed2194ee6170bfaf530767ffbc591b04d16e21ca30021b96623`
and
`94ffbb29cbac96a3b1237ce3b4521b56f336f75020ff256ba79fb1875a5e63bb`.
All 787 newly activated variants move from `unsupported-feature` to `pass`;
the 42 reason-only variants retain their outcome, and no previous pass
regresses. This is a profile and evidence milestone, not a Feature Parity
completion claim.

## R3bi optional chaining focused implementation

R3bi implements optional chaining in the parser/compiler using QuickJS's
shared-chain-end lowering. No optional-chain VM opcode or runtime intrinsic is
introduced. The compiler retains only the Reference metadata needed for
method-call receivers and `delete`, including pinned QuickJS's observable
grouped-public, grouped-private, `with`, and indirect-eval behavior.

The focused inventory is derived from the pinned Test262 metadata:

- 56 paths / 112 variants carry `features: [optional-chaining]`;
- four class/private paths / eight variants remain reason-only behind the
  separately unsupported class-private dependency;
- all 26 parse-negative paths / 52 variants have explicit provenance;
- the remaining 52 paths / 104 variants are runnable in the scoped profile;
- pinned QuickJS and Oxide both pass all 104 runnable variants.

`scripts/test-test262-optional-chaining.sh` re-derives those partitions, binds
the immutable scoped profile and manifest hashes in the runner, verifies the
pinned QuickJS input, and reproduces the all-pass Oxide TSV/JSONL receipt. A
separate ledger freezes 14 Iterator Helper paths / 28 variants whose source
uses optional chaining without declaring the metadata tag. They remain outside
this focused claim and outside the unchanged global profile.

The implementation also changes five untagged, already-runnable staging paths:
nine variants move to pass, comprising seven prior parse failures and two
prior runtime failures. A fresh canonical two-worker run keeps all 102,037
keys and 56,941 runnable variants fixed, advances the pass count from 56,413
to 56,422, and produces complete TSV/JSONL SHA-256 values
`5c388e568e6ee9e09799bc0f471a5926f0b680bd8f4d781e84130fce1a968e8a`
and
`19f076e99f56f22374a533e1f9c8fead0775bf81d2d1940641ae322901c1cc88`.
No previous pass regresses.

Global `optional-chaining` admission is intentionally separate: it must admit
the tag and negative provenance together, refresh the complete receipt, and
retain the four reason-only rows. The Iterator adjacency cohort and the
for-await-of exclusion are likewise left visible for their own cohesive gate
refreshes. This focused implementation is not a Feature Parity completion
claim.

## R3bj optional chaining global admission

R3bj adds exactly `optional-chaining` and the focused gate's 26 audited
parse-negative paths to the global profile. The resulting 82-feature,
828-negative profile has SHA-256
`205554c5686ef2ec77420984ce038d321411a11acabefd2c37d9b63b67fcba62`.
No adjacent class-private, Iterator Helper, module, host, or other broad
built-in feature is admitted.

The dependency-clean activation contains 52 paths / 104 variants, all of which
move from `unsupported-feature` to `pass`. Four class/private paths / eight
variants remain `unsupported-feature` behind another dependency and change
diagnostic detail only. The provenance canary now records 10 intended parse
passes and nine fail-closed variants.

The exact R3bi/R3bj full join retains all 102,037 keys and every previous pass.
The complete vector reaches 56,526 passes with 57,045 runnable variants and
21,599 `unsupported-feature` outcomes. Its raw, pinned-target lower-bound, and
observed-frontier rates are 55.40%, 67.65%, and 99.18% respectively. Canonical
full TSV/JSONL SHA-256 values are
`84c15d4a25343e1d306e17f431e515993abe09db76590920539eefe93d6fb3eb`
and
`96ebd4a8f51001b403e88d19c128bebb92b74bb9abf1e45c832b187924c635fd`.

R3bj also makes the historical R3be TypedArray receipt independent of both
kinds of later profile growth. Its reconstructed parent uses the checked-in
80-tag inventory plus the immutable 802-path negative section of
`tests/test262-iterator-sequencing.conf`; it no longer reads either historical
section from the current global profile. At R3bj, the Iterator adjacency cohort
and for-await-of ledger remained separate follow-up gates; R3bk above refreshes
the latter. This is not a Feature Parity completion claim.

## Runner contract

`run-test262` provides a conservative, process-isolated progress measurement:

- fresh Rust process, `Runtime`, and `Context` for every runnable variant;
- hard parent-process timeout and crash classification;
- canonical Test262 `raw` behavior (no harness or strict prefix);
- separate harness compilation/evaluation, then test compile and execute;
- exact parse-versus-runtime negative phase and constructor-name checks;
- explicitly typed implementation-frontier errors kept distinct from
  JavaScript `SyntaxError`;
- parse-negative tests execute only after compilation succeeds, so
  `$DONOTEVALUATE` cannot turn a missing parse error into a pass;
- unsupported features and unaudited negative tests fail closed through the
  checksum-pinned quickjs-oxide capability profile;
- metadata and source requirements classify module, `CanBlockIsFalse`, and the
  `$262` host hooks used by the pinned suite before execution; async execution
  requires an authenticated profile opt-in and is now enabled globally;
- bounded parallel workers with deterministic result ordering and full child
  cleanup after errors;
- deterministic TSV outcome vector plus a JSONL sidecar;
- module variants and profile-rejected feature/host variants are reported as
  unsupported and treated as failures unless a caller is explicitly recording
  a baseline.

The host scan is deliberately conservative and the pinned inventory has no
unknown `$262` hook. Native `$262` objects and an out-of-band host sentinel are
still required before those host-dependent tests can be admitted for execution.

This deliberately fixes three known limitations in the pinned QuickJS
`run-test262.c`: it does not discard negative phase, does not load harness code
for `raw`, and does not let a stable known-error ledger hide the raw failure
count. A future QuickJS-runner compatibility profile may reproduce those quirks
for outcome-vector differential work, but it must remain separate from the
canonical progress report.

## Reproduce

```sh
./scripts/test-test262-smoke.sh
./scripts/test-test262-provenance.sh
./scripts/test-test262-reflect.sh
./scripts/test-test262-date.sh
./scripts/test-test262-string-split.sh
./scripts/test-test262-regexp-core.sh
./scripts/test-test262-regexp-builtins.sh
./scripts/test-r3s-regexp-escape-control-oracle.sh --oxide target/debug/qjs
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
./scripts/test-r3r-generator-destructuring-return-oracle.sh --oxide target/debug/qjs
./scripts/test-test262-async-generator-yield-star.sh
./scripts/test-test262-global-async.sh
./scripts/test-test262-array-buffer.sh
./scripts/test-test262-data-view.sh
./scripts/test-test262-data-view-global.sh
./scripts/test-test262-typed-array-core.sh
./scripts/test-test262-proxy.sh
./scripts/test-test262-uint8array-codecs.sh
./scripts/test-test262-uint8array-codecs-global.sh
./scripts/test-test262-full.sh
```

The smoke command also exhaustively validates pinned metadata against its
independent fingerprint. The provenance command guards known false-positive
boundaries. The full command uses the release runner, defaults to two workers,
and compares the complete outcome vector and sidecar by SHA-256. Set
`TEST262_WORKERS` to change concurrency without changing the expected bytes.

Math, Reflect, Date, and generic `String.prototype.split` are no longer common
blockers in their reviewed sets.
The Date transition also resolves the four otherwise-ready Reflect variants
which had stopped at `Date.now`; generic split resolves six more linked Reflect
variants. Basic RegExp literal execution, the search/match/split protocols,
legacy compile, scoped modifiers, generic replacement, matchAll, and numeric
backreferences, forward lookahead, lookbehind, Unicode property escapes,
ordinary named captures, duplicate named captures, match indices, and dotAll
and U+180E are now measured separately in
R1b/R1c/R1d/R1e/R1f/R1g/R1h/R1i/R1j/R1k/R1l/R1m/R1n/R1o/R1p/R1q/R1r/R1s/R1t;
R1u separately measures the eval intrinsic shell and its typed String-source
frontier; R1v establishes its syntactic opcode and realm-identity path with a
byte-identical scoreboard; R1w adds the immutable caller-environment table and
live-cell materialization with the same zero-movement result; R1x opens the
bounded independent String-eval root and adds 575 full-vector passes; R1y adds
QuickJS-shaped eval declaration environments and another 768 passes; R1z adds
recursive direct-eval caller-environment relay and another 29 passes; R2a fixes
private FunctionName/eval declaration precedence with a byte-identical full
vector; R2b adds the `with` environment and 198 passes; R2c adds synchronous
simple-parameter ArrowFunctions, declares their shared feature tag, and adds
1,043 passes with zero previous-pass regressions; R2e audits the capability
profile to 53 feature tags and 403 negative paths without changing engine
semantics; R2f adds synchronous simple-parameter ObjectLiteral concise methods,
moves the profile to 413 audited negative paths, and passes its 144-variant
focused gate while adding 492 full-vector passes with zero previous-pass
regressions; R2g adds synchronous simple-parameter ObjectLiteral accessors,
moves the profile to 422 audited negative paths, passes its 128-variant focused
gate, and adds 447 full-vector passes with zero previous-pass regressions; R2h
adds direct ObjectLiteral SuperProperty References, moves the profile to 54
feature tags and 423 audited negative paths, passes its 48-variant focused gate,
and adds 82 full-vector passes with zero previous-pass regressions; R2i relays
ObjectLiteral HomeObject and lexical `this` through synchronous arrows, passes
its eight-variant focused gate, and adds four full-vector passes without
changing the profile or runnable count; R2j authenticates the independent
SuperCall and SuperProperty capabilities through ObjectLiteral direct eval,
passes its 24-variant focused gate, and adds six full-vector passes with no
previous-pass regression or runnable-count change; R2k adds QuickJS-shaped
tagged-template site objects and calls, declares `template`, passes all 83
runnable non-frontier variants in its focused gate, and adds 83 full-vector
passes with zero previous-pass regressions; R2l adds the strict JSON parser,
reviver walk, and exact source contexts, passing 166/168 focused variants with
only the dense-array timeout pair; R2m adds stringify and branded Raw JSON,
passes 160/160 direct stringify variants and 36/42 runnable Raw JSON variants,
declares the two reviewed JSON feature tags, and brings the complete vector to
33,083 passes with zero previous-pass regressions; R2n adds the complete strong
Map surface, passes its 370/370 focused gate, declares only `Map` and
`array-grouping` globally, and adds 314 full-vector passes with zero
previous-pass regression, bringing the complete vector to 33,397 passes.
R2o adds the observable strong Set family and all seven set-composition
methods, passes its 642/642 focused gate, declares only `Set` and
`set-methods`, and adds 644 full-vector passes with zero previous-pass
regression, bringing the complete vector to 34,041 passes.
R2p audits and globally admits the eight remaining well-known Symbol protocol
tags, passes all 806 protocol-ready variants in its frozen 1,010-variant gate,
and adds exactly 806 full-vector passes with zero previous-pass regression,
bringing the complete vector to 34,847 passes.
R2q implements flat array binding declarations, passes all 180 variants in its
90-path scoped gate, and deliberately keeps `destructuring-binding` scoped.
Untagged binding variants nevertheless add 31 full-vector passes with zero
previous-pass regressions, bringing the complete vector to 34,878 passes.
R2r adds recursive nested array declaration patterns across direct
declarations, classic `for`, and synchronous `for-in`/`for-of`, passing all 144
variants in its 72-path scoped gate. The two variants of
`staging/sm/regress/regress-469625-03.js` move to pass with no other
full-vector outcome change, bringing the complete vector to 34,880 passes.
R2s adds fixed and computed recursive object declaration patterns on the same
surfaces, including object/array recursion, observable `with` Reference timing,
and iterator unwind. All 648 variants in its 324-path scoped gate pass. The
full vector gains 36 passes with zero previous-pass regression, reaching
34,916 passes among 38,421 runnable variants; exclusion-aware object rest is
the next binding slice.
R2t adds exclusion-aware object-rest declarations on those direct, loop, and
recursive binding surfaces. All 54 variants in its 27-path scoped gate pass;
the full vector changes only the two modes of one staging path from typed
parser frontier to pass, with zero previous-pass regression, reaching 34,918
passes among 38,421 runnable variants.
R2u adds direct and synchronous for-in/of array assignment patterns, including
member/computed/super References, defaults, rest, recursion, and iterator
unwind. Its direct flat gate passes all 131 variants across 70 paths; the exact
full join adds 15 passes with zero previous-pass regression, reaching 34,933
passes among 38,421 runnable variants. Object assignment is the next
assignment slice.
R2v adds direct and synchronous for-in/of object assignment patterns, including
depth-0-to-3 References, defaults, object/array recursion, exclusion-aware
rest, and iterator unwind. Its three scoped gates pass all 193 variants across
107 paths; the exact full join adds 14 passes with zero previous-pass
regression, reaching 34,947 passes among 38,421 runnable variants.
R2w adds recursive array/object/rest catch BindingPatterns while preserving
catch lexical scope, iterator unwind, direct-eval redeclaration metadata, and
Annex B integration. Its 97-path scoped gate passes all 177 variants; the exact
full join adds 49 passes with zero previous-pass regression, reaching 34,996
passes among 38,421 runnable variants.
R2x adds synchronous identifier-rest parameters to ordinary functions, object
methods, arrows, and the `Function` constructor. Its exact 34-path scoped gate
passes all 65 variants; the full join adds 88 passes with zero previous-pass
regression, reaching 35,084 passes among 38,421 runnable variants. Parameter
Environments, defaults, parameter destructuring, rest BindingPatterns, and
async/generator/class forms remain later FormalParameters milestones.
R2y adds synchronous identifier defaults and a real Parameter Environment to
the same four surfaces. Its exact 76-path scoped gate passes all 143 variants;
the full join adds 60 passes with zero previous-pass regression, reaching
35,144 passes among 38,421 runnable variants.
R2z adds synchronous no-default parameter BindingPatterns across ordinary
functions, object methods, arrows, the `Function` constructor, and setters.
Its exact 149-path scoped gate passes all 298 variants; the full join adds 22
passes with zero previous-pass regression, reaching 35,166 passes among 38,421
runnable variants. BindingPatterns combined with standalone `=` parameter
expressions are the next R3a milestone; async/generator/class forms remain
later callable milestones.
R3a combines standalone `=` parameter expressions with synchronous
BindingPatterns on those surfaces. Its dependency-audited 468-path scoped gate
passes all 936 variants; the full join adds 12 passes with zero previous-pass
regression, reaching 35,178 passes among 38,421 runnable variants.
R3b adds the separate `<var>` / `<arg_var>` direct-eval environment model. Its
71-path scoped gate passes all 71 sloppy variants and its 42-case QuickJS
differential passes all four integration tests. The full join adds 66 passes
with zero previous-pass regression, reaching 35,244 passes among 38,421
runnable variants. Async, generator, and class forms remain later callable
milestones.
R3c adds AggregateError and globally audits Error cause. Its 19-case QuickJS
oracle passes all three integration tests; the complete 56-variant feature
cohort has 50 passes and six exact missing-Proxy dependency results. The full
join adds 52 passes with zero previous-pass regression, reaching 35,296 passes
among 38,483 runnable variants. Proxy, cross-realm host fixtures, class
subclasses, and Promise consumers remain independent milestones.
R3d adds typed ordinary/construct/direct-eval argument spread and the pinned
double-iterator-Get/fast-Array behavior. Its 134-variant focused gate passes
122 with 12 exact adjacent frontiers; the full join adds 124 net passes with no
previous-pass regression, reaching 35,420 among the same 38,483 runnable
variants. The current Symbol protocol and Raw JSON gates pass 892/1,010 and
42/42 runnable variants respectively.
R3e adds synchronous base class declarations/expressions, constructors,
methods/accessors, lexical/TDZ environments, computed names, and HomeObject
semantics. Its 157-path/294-variant focused gate and five-test QuickJS oracle
both pass completely; the full join adds 328 passes with no previous-pass
regression, reaching 35,748 among the same 38,483 runnable variants. Class
heritage/derived constructors, fields/private elements, static blocks, and
generator/async class methods remain separate milestones.
R3f adds synchronous heritage, derived constructors, and `super()` across
direct, arrow, parameter-initializer, and nested direct-eval paths. Its
386-path/767-variant focused gate and dedicated QuickJS differential pass
completely; the exact full join adds 545 passes with no previous-pass
regression, reaching 36,293 among the same 38,483 runnable variants. The global
profile still keeps whole-feature `class` disabled until fields/private
elements, static blocks, and generator/async class methods land.
R3g adds public instance/static fields and static blocks. Its distinct
386-path/767-variant focused gate passes completely against a pinned-QuickJS
oracle. The result remains manifest-scoped; no new whole-suite percentage or
global `class` admission is claimed until the full join and the remaining
private/async/generator class surfaces are complete.
R3h adds private instance/static data fields and private-`in` References. Its
hash-authenticated 630-path/1,260-variant focused gate passes 1,260/1,260;
pinned QuickJS passes 630/630.
R3i adds ordinary synchronous private instance/static methods with independent
class-side brands. Its hash-authenticated 267-path/534-variant focused gate
passes 534/534; pinned QuickJS passes 267/267. R3j adds the disjoint
305-path/610-variant synchronous private-accessor target: Oxide passes 610/610
and pinned QuickJS passes 305/305. R3k adds synchronous generator
declarations/expressions and public object/class generator methods. Its first
authenticated 82-path/160-variant class-generator gate passes 160/160; pinned
QuickJS passes 82/82. Async forms and private generator methods remain later
frontiers at the R3k checkpoint. R3l adds the private instance/static class-
generator slice: its authenticated 82-path/160-variant gate passes 160/160,
and pinned QuickJS passes 82/82. Async forms remain later frontiers, and the
global profile stays closed for `generators`. R3m establishes the Promise
constructor/reaction/job boundary with 112/112 focused variants and QuickJS
57/57 paths. R3n adds `try`, `withResolvers`, and `race`; its complete
112-path/224-variant landing inventory records 214 passes and the ten
explicitly listed `all`/`finally` adjacency failures, while pinned QuickJS
passes 112/112. R3o implements `Promise.prototype.finally`; its complete
29-path/58-variant gate passes 56 variants, with only the two Proxy-dependent
variants failing, while pinned QuickJS passes 29/29. R3p implements
`Promise.all`; its complete 98-path/196-variant gate passes 196/196, and the
unchanged R3n inventory now passes 224/224.
R3q adds `Promise.allSettled` and `Promise.any`; their complete gates pass
208/208 and 188/188. R3r removes the complete vector's final two engine faults
through generator-destructuring iterator unwind. R3s then completes the pinned
non-`v`, non-`createRealm` RegExp built-ins cohort at 3,346/3,346 and raises the
full vector to 36,927 passes without a previous-pass regression.
R3t closes the authenticated synchronous generator/destructuring cohort at
6,593/6,593. Its untagged Annex B fix raises the conservative full vector by
one to 36,928 passes; the scoped cohort itself remains outside that global
score. R3u then admits that synchronous cohort globally: 6,593 variants move
to pass, six async adjacencies remain fail-closed, and the vector reaches
43,521 passes. The global profile remains fail-closed for async execution and
Promise features.
R3v adds the synchronous `Iterator` intrinsic and core Iterator Helpers. Its
dependency-audited 523-path/1,046-variant scoped gate passes 1,046/1,046 in
both Oxide and pinned QuickJS. The global score remains 43,521 because Proxy
and host-dependent adjacencies remain fail-closed. R3w adds the independent
`Iterator.concat` sequencing state machine; its clean 32-path/64-variant scoped
gate passes 64/64 in both engines. R3x globally admits that exact clean cohort,
moving only those 64 variants to pass and bringing the full vector to
43,585/102,037. R3y then freezes the synchronous class generated matrix at
7,735/7,735 in both engines, leaving the 28 async/Proxy adjacencies and the
global scoreboard unchanged. R3z adds ordinary async functions and `await`
through a separately authenticated 142-path/259-variant profile; Oxide passes
259/259 and pinned QuickJS passes every path, while async arrows/methods,
async generators, for-await, modules, and broad async execution remained
fail-closed. Untagged and intrinsic consumers add 58 conservative full-vector
passes without regressing a previous pass, bringing the R3z landing score to
43,643/102,037.
R3aa subsequently expanded the authenticated ordinary-async gate to 191 paths /
348 variants by admitting all 40 complex-parameter paths and nine eval/with
adjacencies. Oxide passed 348/348 and pinned QuickJS passed 191/191; at that
landing, the 16 exclusions were async-arrow, async-generator/for-await, or
host/cross-realm dependencies. Because this was a scoped bookkeeping
expansion, the conservative full-vector score and hashes did not change.
R3ab then implements async arrows as Arrow grammar with Async execution and
closes their complete canonical 60-path/110-variant language tree with zero
exclusions: Oxide passes 110/110 and pinned QuickJS passes 60/60. The focused
gate adds one exact Function `toString` adjacency and passes 112/112 across 61
paths in Oxide, with QuickJS at 61/61. The stable `with` gate also reaches
205/205, while the ordinary-async gate expands to 203 paths / 366 passing
variants and leaves only four async-generator/host dependencies excluded. A
SpiderMonkey staging case that contradicts the pinned target's single-binding
`await` token timing is checksum-pinned as audit-only, outside that universe.
At the R3ab landing, broad async feature/host admission remained fail-closed
pending async methods and generators. Twelve already-admitted consumers still
advanced without a previous-pass regression, bringing the full vector to
43,655/102,037.
R3ac then adds ordinary async object-literal methods through Method+Async
metadata and the existing DefineMethod/HomeObject path, without a runtime
change. Its 49-path/90-variant candidate universe passes 49/49 in pinned
QuickJS; after six async-generator and one Proxy exclusion, Oxide passes the
42-path/76-variant gate in full. Four already-admitted consumers advance, no
previous pass regresses, and the conservative vector reaches
43,659/102,037.
R3ad ports public instance/static async class methods through the same
Method+Async and DefineMethod/HomeObject path. Pinned QuickJS passes all 313
candidate paths; after 19 private-async/async-generator exclusions, Oxide
passes all 568 variants across the 294 admitted paths. At that landing, private
async class methods and async generators remained the recommended next
milestones. The two already-admitted staging consumers advance without a
previous-pass regression, bringing the conservative vector to 43,661/102,037.
R3ae then composes ordinary private async instance/static methods from the
existing Async execution and authenticated private-method HomeObject/brand
paths. Pinned QuickJS passes all 233 candidates; after 77 async-generator or
mixed-staging exclusions, Oxide passes all 312 variants across 156 admitted
paths. The exact full vector remains byte-identical at 43,661/102,037, so async
generators become the next class-method frontier without a global-profile
widening.
R3af adds ordinary async-generator declarations/expressions, their intrinsic
graph, and the Promise-backed FIFO resume state machine. Pinned QuickJS passes
all 1,008 candidates; after 765 explicit destructuring, delegation, for-await,
method, Proxy, and realm/host exclusions, Oxide passes all 440 variants across
243 admitted paths. The scoped profile keeps active iterator-close semantics
and broad async execution fail-closed. Fifteen already-admitted consumers
advance with no previous-pass regression, bringing the conservative full
vector to 43,676/102,037.
R3ag then composes ordinary object-literal async-generator methods from Method
grammar, enumerable DefineMethod/HomeObject publication, and the R3af driver.
Pinned QuickJS passes all 113 candidates; after 67 explicit delegation,
for-await, destructuring, private-name, and Proxy exclusions, Oxide passes all
82 variants across 46 admitted paths. Four already-admitted staging variants
advance from `unsupported-runtime` to `pass` with no other outcome change or
previous-pass regression, bringing the conservative full vector to
43,680/102,037.
R3ah composes public instance/static class async-generator methods from the
same Method+AsyncGenerator parser, non-enumerable class publication, and
shared driver. Pinned QuickJS passes all 573 focused candidates; after 256
explicit delegation, for-await, destructuring-scope, and private-composition
exclusions, Oxide passes all 606 variants across 317 admitted paths. Six
already-admitted consumers advance from `unsupported-parser` to `pass` with no
other summary change or previous-pass regression, bringing the conservative
full vector to 43,686/102,037.
R3ai composes private instance/static class async-generator methods from the
same Method+AsyncGenerator execution with typed private callable cells,
HomeObject, side brands, and the shared driver. Pinned QuickJS passes all 433
focused candidates; after 300 `yield*` and eight `for await` exclusions, the
scoped manifest passes all 242 variants across 125 paths. The exact complete
vector remains byte-identical at 43,686/102,037.
R3aj then closes the four-shape async-generator `yield*` cohort: pinned
QuickJS passes all 775 frozen paths and Oxide passes all 1,550 sloppy/strict
variants. Independent 8/8/5-worker reports are byte-identical, while ten
QuickJS transcripts and a GC-retention test lock the protocol behavior. The
complete R3aj vector retains 102,037 variants, 45,140 runnable variants, and
43,686 passes, with zero drift and byte-identical TSV/JSONL reports relative
to R3ai. `for await` is next; closing an independently active outer iterator
across `.return()` remains a separate frontier.
R3ak implements that `for await ... of` frontier in ordinary async functions
and all four async-generator shapes. Pinned QuickJS passes all 1,264
authenticated paths; Oxide passes all 2,490 variants, and independent
8/8/5-worker reports are byte-identical. The dedicated transcript and
repeated-GC test cover active async-generator `.return()` while `next` is
pending. The exact complete-vector join changes only three already-admitted
SpiderMonkey staging variants from `unsupported-runtime` to `pass`, with no
previous-pass or other drift; the vector reaches 43,689/102,037.
R3al then promotes the authenticated async-function and async-iteration stack
into the global profile. Its 3,589-path / 7,076-variant newly executable
cohort passes in full. The exact 102,037-key join retains every previous pass,
raises the complete vector to 50,765 passes and 52,216 runnable variants, and
leaves adjacent feature, module, host, and negative-provenance dependencies
fail-closed.
R3am adds the QuickJS-shaped Proxy internal-method seam and a
464-path/904-variant scoped gate. Oxide passes 811 variants; the remaining 93
are exact host/module/parser or TypedArray-harness frontiers, with no Proxy
runtime failure. The complete join adds 212 passes with zero previous-pass
regression and reaches 50,977/102,037 while global Proxy admission remains
fail-closed.
R3an adds the pure ArrayBuffer backing-store, constructor, detach, resize,
slice/species, and transfer core. After 24 latent `Uint8Array` exclusions,
Oxide and pinned QuickJS both pass the authenticated 144-path/288-variant
gate. The exact full join retains every previous pass and reaches
51,193/102,037; installing the real detach host also exposes the still-missing
DataView/TypedArray stack and 16 latent transfer variants as deeper failures
rather than overstating binary-data feature parity.
R3ao adds the branded DataView layer with all 11 getter/setter families and
fixed-versus-tracking resizable-buffer views. After 86 adjacent-dependency
exclusions, Oxide and pinned QuickJS both pass the authenticated
492-path/984-variant gate. The complete vector reaches 51,707/102,037; its
exact join records only 514 `fail-runtime -> pass` transitions, with zero
missing, extra, duplicate, or previous-pass-regression rows.
R3ap adds the shared 12-class TypedArray kernel. Its candidate/exclusion audit
leaves a 735-path/1,447-variant core which Oxide and pinned QuickJS both pass
completely. The full join adds 195 net passes, retains all 102,037 keys and
every previous pass, and reaches 51,902/102,037; later method families and
SharedArrayBuffer stay explicit.
R3aq publishes the in-place TypedArray mutation family. Of its audited
254-path/508-variant candidate, 251 paths/502 variants are dependency-clean and
join the cumulative 986-path/1,949-variant gate; Oxide and pinned QuickJS both
pass every admitted variant. Three paths remain attributed to `join` or
WeakMap dependencies. Because the conservative global profile still withholds
the broad TypedArray tag, only two untagged variants change there, bringing the
complete vector to 51,904/102,037 without overstating whole-family support.
R3ar publishes TypedArray `at`, `includes`, `indexOf`, and `lastIndexOf`.
After one explicitly attributed WeakMap-harness deferral, 151 paths / 302
variants join the cumulative 1,137-path / 2,251-variant gate; Oxide and pinned
QuickJS both pass it completely. Four untagged staging modes advance the
complete vector to 51,908/102,037 with zero previous-pass regression, while
the global broad TypedArray tag remains withheld.
R3as publishes TypedArray `find`, `findIndex`, `findLast`, and
`findLastIndex`. After two explicitly attributed WeakMap-harness deferrals,
156 paths / 296 variants join the cumulative 1,293-path / 2,547-variant gate;
Oxide and pinned QuickJS both pass it completely. The full vector stays
byte-identical at 51,908/102,037 because broad TypedArray admission remains
withheld.
R3at publishes TypedArray `every` and `some`. One cross-realm staging path
with a hard WeakMap harness dependency remains deferred; the other 92 paths /
184 variants join the cumulative 1,385-path / 2,731-variant gate, which Oxide
and pinned QuickJS both pass completely. Broad TypedArray admission remains
withheld, so a fresh complete run confirms that the full vector stays
byte-identical at 51,908/102,037.
R3au publishes TypedArray `forEach`. One cross-realm staging path with a hard
WeakMap harness dependency remains deferred; the other 44 paths / 88 variants
join the cumulative 1,429-path / 2,819-variant gate, which Oxide and pinned
QuickJS both pass completely. Broad TypedArray admission remains withheld, so
a fresh canonical full rerun confirms that the vector stays byte-identical at
51,908/102,037.
R3av publishes TypedArray `reduce` and `reduceRight`. One cross-realm staging
path remains deferred; the other 104 paths / 208 variants join the cumulative
1,533-path / 3,027-variant gate, which Oxide and pinned QuickJS both pass
completely. Broad TypedArray admission remains withheld; a fresh canonical
full rerun confirms that the vector stays byte-identical to R3au at
51,908/102,037.
R3aw publishes species-aware TypedArray `map` and `filter`. One raw
cross-realm/WeakMap staging path remains deferred; the other 174 paths / 348
variants join the cumulative 1,707-path / 3,375-variant gate, which Oxide and
pinned QuickJS both pass completely. The exact full join changes only the
sloppy/strict `filter-species.js` and `map-species.js` rows from runtime failure
to pass, advancing the complete vector to 51,912/102,037 with no other row
drift.
R3ax publishes QuickJS-shaped TypedArray `slice` and `subarray`. Five raw
cross-realm/WeakMap staging paths remain deferred; the other 173 paths / 346
variants join the cumulative 1,880-path / 3,721-variant gate, which Oxide and
pinned QuickJS both pass completely. Two byte-identical full runs change only
ten staging rows from runtime failure to pass, advancing the complete vector
to 51,922/102,037 with no previous-pass regression or other row drift.
R3ay publishes non-species TypedArray `with` and `toReversed`. Its complete
34-path / 68-variant dependency-clean cohort joins the cumulative 1,914-path /
3,789-variant gate with no deferred path, and Oxide and pinned QuickJS both
pass completely. Two independent canonical full runs are byte-identical and
change only the sloppy/strict `with.js` rows from runtime failure to pass,
advancing the complete vector to 51,924/102,037 with no previous-pass
regression or other row drift.
R3az publishes dedicated TypedArray `join` and `toLocaleString` while retaining
inherited `toString`. Its atomic 88-path / 175-variant candidate defers five
paths / nine variants for cross-realm and SpiderMonkey WeakMap-shell
dependencies; the other 83 paths / 166 variants join the cumulative
1,997-path / 3,955-variant gate. Oxide and pinned QuickJS both pass it
completely. Two byte-identical full runs change only the sloppy and strict
`detached-array-buffer-checks.js` rows from runtime failure to pass, advancing
the complete vector to 51,926/102,037 with no previous-pass regression or
other row drift.
R3ba publishes TypedArray `sort` and `toSorted`. Four cross-realm and two
WeakMap-shell staging paths remain deferred; the other 58 paths / 116 variants
join the cumulative 2,055-path / 4,071-variant gate, which Oxide and pinned
QuickJS both pass completely. The current complete measurement moves 14
runtime failures to pass and advances the vector to 51,940/102,037 while
runnable and every other summary category remain unchanged. Two independent
formal two-worker repeats reproduce the canonical report; the next milestone
at that landing was a residual TypedArray audit.
R3bb authenticates the existing shared TypedArray `entries` and `keys`
iterators without a production-code change. After deferring two
`createRealm`/WeakMap staging paths and one WeakMap/Uint8Array-codec identity
path, 43 paths / 86 variants join the cumulative 2,098-path /
4,157-variant gate, which Oxide and pinned QuickJS both pass completely. The
global profile remains unchanged, so the canonical complete vector remains
51,940/102,037. At that landing, static TypedArray `of` was the next audited
slice.
R3bc authenticates static TypedArray `of` and fixes the shared static
`from`/`of` primitive-receiver diagnostic seam. Its 35-path / 70-variant
candidate defers only the two cross-realm/WeakMap staging variants; the other
34 paths / 68 variants join the cumulative 2,132-path / 4,225-variant gate,
which Oxide and pinned QuickJS both pass completely. All 68 promoted full rows
remain `unsupported-feature`, so the conservative vector stays
51,940/102,037. Its next audited static `from` universe contained 90 paths /
175 variants: 81 paths / 158 promotable and nine paths / 17 deferred.
R3bd authenticates static TypedArray `from`, including exact nullish-source
diagnostics, observable map/iterator/construction ordering, retained iterable
value lifetime, Number/BigInt conversion, and RAB/detach behavior. The 81
promoted paths expand the cumulative gate to 2,213 paths / 4,383 variants,
which both engines pass completely; the nine external-dependency paths remain
explicitly deferred. The broad feature tag remains withheld, so the
conservative full vector stays 51,940/102,037. At that landing, the next audit
reviewed 80 spillover variants / 41 paths exposed by TypedArray-only global
admission.
R3be admits that global `TypedArray` tag after freezing the complete
1,865-path / 3,686-variant activation and its 471-path / 938-variant
reason-only ledger.
The spillover expands the scoped gate to 2,254 paths / 4,463 all-pass variants.
The exact full join changes 3,686 unsupported rows to pass, changes only 938
other diagnostic details, and has no other row movement or previous-pass
regression. The complete vector reaches 55,626/102,037 with 56,154 runnable
variants; no production runtime code changes.
R3bh then admits the global `Proxy` tag. Its Proxy-only activation contains
405 paths / 787 variants, all passing, while 21 paths / 42 variants remain
reason-only rows behind other unsupported dependencies. The complete vector
reaches 56,413/102,037 with 56,941 runnable variants and no previous-pass
regression. Four already-exposed staging variants still fail in both engines
and are frozen as pinned QuickJS target deviations rather than “fixed” away
from the parity target.
R3bi implements optional chaining through the QuickJS-shaped compiler path.
Its independent gate authenticates 52 dependency-clean paths / 104 variants,
all passing in both engines, while four class/private paths and 14 hidden
Iterator-adjacency paths remain explicit ledgers. Nine already-runnable
staging variants turn green, advancing the unchanged conservative global
profile to 56,422/102,037 passes without admitting the broad feature tag.
R3bj admits that `optional-chaining` tag together with all 26 audited
parse-negative paths. The 104 dependency-clean variants move from
`unsupported-feature` to `pass`, eight class/private variants remain
reason-only, and no previous pass regresses. The complete vector reaches
56,526/102,037 with 57,045 runnable variants. R3bj also freezes the historical
R3be TypedArray parent's 802-path negative-provenance source, completing the
decoupling begun by R3bh's 80-tag feature inventory.
R3bk then refreshes the `for await` focused gate after that admission. Its
unchanged 1,297-path / 2,531-variant candidate now excludes 32 paths / 39
variants and admits 1,265 paths / 2,492 variants, all passing in both engines.
The scoped receipt is independent of later global-profile feature growth, and
the complete R3bj vector remains unchanged.
R3bl then promotes the exact 14-path optional-chaining adjacency into the
scoped Iterator Helper gate. It passes 537 paths / 1,074 variants in both
engines while retaining a 30-path deferred ledger.
R3bm then promotes the remaining 11 source-Proxy and three harness-Proxy
paths, completing the 28-path Proxy closure. That historical scoped gate passed
551 paths / 1,102 variants in both engines and retained only the 16
host/config paths. The independently authenticated 64-variant sequencing gate
was unchanged. Neither R3bl nor R3bm admitted global `iterator-helpers` or
moved the R3bj complete vector.
R3bn then admits exactly `iterator-helpers` into the global profile. Its
exhaustive 567-path / 1,134-variant join activates and passes 1,076 variants,
changes only unsupported-reason detail for the 26 `globalThis` variants, and
leaves the 32 host/config variants and all 100,903 non-Iterator-Helper variants
unchanged. R3bp then admits exactly `globalThis`: 150 variants move from
`unsupported-feature` to `pass`, its 15 module/config deferrals and all
101,872 non-`globalThis` variants remain byte-identical, and no detail-only
change or previous-pass regression occurs.
R3bq then admits the four implemented global Promise tags. Its 416 activation
variants pass, 36 reason-only variants retain their `class` or
`computed-property-names` frontier, and all 101,585 non-universe rows remain
byte-identical. R3br then authenticates the complete 138-variant Uint8Array
codec cohort, and R3bs admits exactly `uint8array-base64`. Those 138 outcomes
move from `unsupported-feature` to `pass`; the other 101,899 rows remain
unchanged and no previous pass regresses. R3bt then authenticates all 762
dependency-clean `resizable-arraybuffer` variants, and R3bu admits that tag:
762 outcomes change to `pass`, 160 residual-capability rows change only their
diagnostic detail, and the other 101,115 rows remain unchanged. The R3bu
vector reached 59,068/102,037 passes with 59,587 runnable variants, 19,057
`unsupported-feature` outcomes, and 24,024 total unsupported outcomes.
R3bv then certifies all 439 dependency-clean `computed-property-names`
variants against both engines and freezes the remaining 507 tagged rows; it
is still scoped, so the R3bu canonical vector remains unchanged.
R3bw then admits that tag globally: 439 outcomes change to `pass`, 456 rows
change only their residual-capability detail, and 101,142 rows remain
unchanged. That historical R3bw vector reached 59,507/102,037 passes with 60,026
runnable variants, 18,618 `unsupported-feature` outcomes, and 23,585 total
unsupported outcomes. Subsequent milestones through R3cz advanced the
then-current vector to 65,509/102,037 passes with 65,566 runnable variants, 13,719
`unsupported-feature` outcomes, and 17,996 total unsupported outcomes.
The generated Unicode code-point property corpus now passes; properties of
strings remain coupled to `v` mode.
Test262 remains the project scoreboard, while focused QuickJS
differentials decide exact target semantics for each slice. None of these
progress figures is a feature-parity completion claim.
