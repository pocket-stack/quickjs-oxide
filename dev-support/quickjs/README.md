# Dynamic-import trace oracle

`dynamic-import-trace-2026-06-04.patch` is a test-only instrumentation patch
for the authenticated QuickJS 2026-06-04 archive. It is not product code and
must never be applied to `target/oracle/quickjs-2026-06-04`.

`scripts/build-quickjs-dynamic-import-trace.sh` verifies the archive, patch,
and patched-source fingerprints, extracts into a fresh temporary directory,
applies the patch with zero fuzz, builds `run-test262`, and prints the temporary
trace source directory. The same isolated extraction also produces
`run-test262.stock` for behavior A/B checks. Set
`QJS_DYNAMIC_IMPORT_TRACE_BUILD_DIR` to choose the temporary build root;
callers should remove the build root when done.

The patched runner is fail-closed. It enables tracing only when
`QJS_OXIDE_DYNAMIC_IMPORT_TRACE=1` is present at process startup and descriptor
3 is already a regular file. It verifies and duplicates descriptor 3 before
opening config or test inputs, requires an explicit `-T 1`, refuses
`$262.agent`, and exits with status 74 if a trace write fails. These constraints
make the deliberately simple multi-write encoder deterministic:

```sh
build=$(scripts/build-quickjs-dynamic-import-trace.sh)
QJS_OXIDE_DYNAMIC_IMPORT_TRACE=1 \
  "$build/run-test262" -T 1 -N --module test.js 3>trace.tsv
```

Each record begins with `QJODI1` and contains only ASCII. Arbitrary byte strings
use `<byte-length>:<lowercase-hex>`; `-` represents null. Every record includes
the root test identity (`JSRuntime.rt_info` or the loader opaque) before its
event-specific fields. Record types are:

- `N`: root, base name, requested name, normalized name
- `L`: root, loader request, effective filesystem path, outcome, saved load
  `errno`
- `T`: root, compiled module name, parser-derived `has_tla` bit

`scripts/parse-quickjs-dynamic-import-trace.mjs` is the strict parser. The
regression test covers computed and bare requests, missing-file `errno`, and
top-level await in blocks/templates versus await inside a nested function. It
also compares the stock and instrumented runners' exit status, stdout, and
stderr to detect instrumentation-induced behavior drift. As a separate opt-in
gate, the test passes an otherwise-valid descriptor 3 without the environment
variable and requires the trace to remain empty.
