#!/usr/bin/env node

// Cohort "module-depfree-bindings-a" (S1 C2a, first slice):
//
// The remaining dependency-free instantiation/evaluation module semantics in
// `test/language/module-code`: the `instn-*` static instantiation family, the
// `eval-gtbndng-*` / `eval-export-*` evaluation family, the four
// `export-default-*-declaration-binding` roots, and the two self-referential
// `dynamic-import/eval-export-dflt-expr-gen-*` module roots. Every graph closes
// over the root itself or same-directory `_FIXTURE.js` files; there are no
// cross-directory requests and no harness `includes`.
//
// Two sibling root-throw *runtime* tests
// (`eval-export-dflt-expr-err-eval.js` and `...-err-get-value.js`) are excluded
// by canary: their expected rejection is a root throw (one a harness-defined
// `Test262Error` plain object with an empty message), which the frozen exact
// runtime-contract model does not represent yet. The twelve admitted negatives
// are all resolution-phase SyntaxErrors replayed through
// `run-test262 -N --module`.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, posix, relative, resolve, sep } from "node:path";

import {
  admissionHeader,
  admissionRecord,
  assertAdmissionGroup,
  renderAdmissionRows,
} from "./test262-admission-data.mjs";

const root = resolve(import.meta.dirname, "../..");
const checkedSuite = join(root, "target/oracle/quickjs-2026-06-04/test262");
const checkedProfile = join(root, "compat/test262-oxide.conf");
const checkedAdmissions = join(root, "dev-support/test262/admissions.tsv");
const checkedDiagnostics = join(root, "dev-support/test262/negative-diagnostics.tsv");
const checkedDiagnosticRules = join(root, "dev-support/test262/negative-diagnostic-rules.tsv");
const checkedConfig = join(root, "target/oracle/quickjs-2026-06-04/test262.conf");
const checkedRunner = join(root, "target/oracle/quickjs-2026-06-04/run-test262");

const args = process.argv.slice(2);
const suiteIndex = args.indexOf("--suite");
const outputIndex = args.indexOf("--output");
const quickjsRunnerIndex = args.indexOf("--quickjs-runner");
const quickjsConfigIndex = args.indexOf("--quickjs-config");
const suite = resolve(suiteIndex === -1 ? checkedSuite : args[suiteIndex + 1]);
const output = outputIndex === -1 ? null : resolve(args[outputIndex + 1]);
const quickjsRunner =
  quickjsRunnerIndex === -1 ? checkedRunner : resolve(args[quickjsRunnerIndex + 1]);
const quickjsConfig =
  quickjsConfigIndex === -1 ? checkedConfig : resolve(args[quickjsConfigIndex + 1]);
const selectableModes = new Set([
  "--admissions",
  "--negative-diagnostics",
  "--diagnostic-candidates",
  "--diagnostic-rules",
  "--check-current",
]);
const selectedModes = args.filter((argument) => selectableModes.has(argument));
assert(selectedModes.length <= 1, "select at most one output/check mode");
const mode = output ? "output" : selectedModes[0] ?? "--check-current";
for (let index = 0; index < args.length; index += 1) {
  const argument = args[index];
  if (["--suite", "--output", "--quickjs-runner", "--quickjs-config"].includes(argument)) {
    index += 1;
  } else {
    assert(selectableModes.has(argument), `unknown option: ${argument}`);
  }
}

assert(existsSync(join(suite, "test")), `missing Test262 suite: ${suite}`);
assert(
  mode !== "--check-current" || existsSync(checkedProfile),
  `missing Oxide profile: ${checkedProfile}`,
);

const cohort = "test/language/module-code";
const dynamicImportCohort = "test/language/expressions/dynamic-import";
const admissionGroup = "module-depfree-bindings-a";

// Expected cohort geometry, pinned against test262 @5c8206929.
const expected = {
  familySurface: 110,
  roots: 57,
  depFreeRoots: 11,
  graphRoots: 46,
  graphSources: 95,
  graphFixtures: 49,
  graphRequests: 85,
  maxClosure: 5,
  resolutionNegatives: 12,
  positiveRoots: 45,
  dynamicPolicyRoots: 2,
  rootsSha256: "PENDING",
};

const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const source = (relativePath) => readFileSync(join(suite, relativePath), "utf8");
const frontmatter = (text) => text.match(/\/\*---[\s\S]*?---\*\/(?:\r?\n)?/)?.[0] ?? "";
const arrayField = (text, name) => {
  const match = text.match(new RegExp(`^${name}:\\s*\\[([^\\]]*)\\]`, "mu"));
  return (
    match?.[1]
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean) ?? []
  );
};
function metadata(relativePath) {
  const text = frontmatter(source(relativePath));
  const negative = text.match(
    /^negative:\s*\n\s*phase:\s*([^\s]+)\s*\n\s*type:\s*([^\s]+)\s*$/m,
  );
  return {
    includes: arrayField(text, "includes"),
    flags: arrayField(text, "flags"),
    features: arrayField(text, "features"),
    negativePhase: negative?.[1] ?? "",
    negativeType: negative?.[2] ?? "",
  };
}

// All literal module requests in source order: dynamic `import("x")`, static
// `import ... from "x"`/`import "x"`, and re-export `export ... from "x"`.
function requestSpecifiers(relativePath) {
  const body = source(relativePath).replace(/\/\*---[\s\S]*?---\*\//, "");
  const matches = [];
  const patterns = [
    { re: /\bimport\s*\(\s*(["'])([^'"]+)\1/gu, dynamic: true },
    {
      re: /\bimport\s+(?!\()(?:(?:[\w*{},\s]+)\s+from\s+)?(["'])([^'"]+)\1/gu,
      dynamic: false,
    },
    {
      re: /\bexport\s+(?:\*|\{[^}]*\})[^'";]*\bfrom\s*(["'])([^'"]+)\1/gu,
      dynamic: false,
    },
  ];
  for (const { re, dynamic } of patterns) {
    for (const match of body.matchAll(re)) {
      matches.push({ offset: match.index, specifier: match[2], dynamic });
    }
  }
  matches.sort((left, right) => left.offset - right.offset);
  const requests = [];
  for (const match of matches) {
    if (!requests.some((request) => request.specifier === match.specifier)) {
      requests.push({ specifier: match.specifier, dynamic: match.dynamic });
    }
  }
  for (const { specifier } of requests) {
    assert(specifier.startsWith("./"), `${relativePath}: non-child request ${specifier}`);
    assert(!specifier.includes("/../"), `${relativePath}: escaping request ${specifier}`);
  }
  return requests;
}

// Static requests drive transitive closure; dynamic self-imports are recorded
// as edges (for the loader ledger) but do not add closure files.
function staticRequests(relativePath) {
  const all = requestSpecifiers(relativePath);
  // requestSpecifiers mergges duplicates; a static-only request is one that
  // also appears outside a dynamic `import(...)` position. Recompute static
  // set explicitly to avoid classifying a dynamic-only self edge as static.
  const body = source(relativePath).replace(/\/\*---[\s\S]*?---\*\//, "");
  const staticSpecifiers = [];
  for (const raw of body.split(/\r?\n/)) {
    const line = raw.trimStart();
    if (!line.startsWith("import") && !line.startsWith("export")) continue;
    let request = null;
    const from = line.indexOf(" from ");
    if (from !== -1) {
      request = line.slice(from + " from ".length).trimStart();
    } else if (line.startsWith("import")) {
      request = line.slice("import".length).trimStart();
      if (!request.startsWith("'") && !request.startsWith('"')) request = null;
    }
    if (!request || (!request.startsWith("'") && !request.startsWith('"'))) continue;
    const quote = request[0];
    const specifier = request.slice(1, request.indexOf(quote, 1));
    if (!staticSpecifiers.includes(specifier)) staticSpecifiers.push(specifier);
  }
  return all
    .map((request) => request.specifier)
    .filter((specifier) => staticSpecifiers.includes(specifier));
}

const normalize = (base, request) => posix.join(posix.dirname(base), request);

function closure(rootPath) {
  const reached = new Set();
  const pending = [rootPath];
  while (pending.length > 0) {
    const base = pending.pop();
    if (reached.has(base)) continue;
    reached.add(base);
    for (const specifier of staticRequests(base)) pending.push(normalize(base, specifier));
  }
  return [...reached].sort();
}

// ---- cohort selection ------------------------------------------------------

const bindingRootNames = new Set([
  "export-default-asyncfunction-declaration-binding.js",
  "export-default-asyncgenerator-declaration-binding.js",
  "export-default-function-declaration-binding.js",
  "export-default-generator-declaration-binding.js",
]);
function isFamilyName(name) {
  return (
    (!name.endsWith("_FIXTURE.js") &&
      (/^instn-/.test(name) || /^eval-gtbndng-/.test(name) || /^eval-export-/.test(name))) ||
    bindingRootNames.has(name)
  );
}
const familySurface = readdirSync(cohortDir(), { withFileTypes: true })
  .filter((entry) => entry.isFile() && entry.name.endsWith(".js") && isFamilyName(entry.name))
  .map((entry) => `${cohort}/${entry.name}`);
function cohortDir() {
  return join(suite, cohort);
}
familySurface.push(
  `${dynamicImportCohort}/eval-export-dflt-expr-gen-anon.js`,
  `${dynamicImportCohort}/eval-export-dflt-expr-gen-named.js`,
);
familySurface.sort();
for (const relativePath of familySurface) {
  assert(existsSync(join(suite, relativePath)), `missing cohort surface path: ${relativePath}`);
}

const foreignAdmittedRoots = new Set(
  readFileSync(checkedAdmissions, "utf8")
    .trimEnd()
    .split("\n")
    .slice(1)
    .map((line) => line.split("\t"))
    .filter((fields) => (fields[0] === "module" || fields[0] === "graph-root"))
    // Roots already owned by this cohort remain eligible so the generator is
    // idempotent after promotion; only other groups' roots are excluded.
    .filter((fields) => fields[1] !== admissionGroup)
    .map((fields) => fields[2]),
);

// Root-throw runtime negatives await the audit tool's root-throw runtime
// contract path; they stay in the unsupported-module bucket until then.
const rootThrowRuntimeCanaries = new Set([
  `${cohort}/eval-export-dflt-expr-err-eval.js`,
  `${cohort}/eval-export-dflt-expr-err-get-value.js`,
]);

const roots = familySurface
  .filter((relativePath) => !foreignAdmittedRoots.has(relativePath))
  .filter((relativePath) => !rootThrowRuntimeCanaries.has(relativePath));
assert.equal(roots.length, expected.roots, "dependency-free binding root count changed");
assert.equal(
  roots.filter((p) => closure(p).length === 1 && requestSpecifiers(p).length === 0).length,
  expected.depFreeRoots,
  "dependency-free singleton root count changed",
);

const depFreeRoots = roots.filter(
  (relativePath) => closure(relativePath).length === 1 && requestSpecifiers(relativePath).length === 0,
);
const graphRootList = roots.filter((relativePath) => !depFreeRoots.includes(relativePath));
assert.equal(graphRootList.length, expected.graphRoots, "fixture/self graph root count changed");

const graphSources = [...new Set(graphRootList.flatMap((rootPath) => closure(rootPath)))].sort();
assert.equal(graphSources.length, expected.graphSources, "graph source count changed");
assert.equal(
  graphSources.length - graphRootList.length,
  expected.graphFixtures,
  "graph fixture count changed",
);

const fileEdges = new Map(
  graphSources.map((relativePath) => [
    relativePath,
    requestSpecifiers(relativePath).map(({ specifier }) => ({
      specifier,
      normalized: normalize(relativePath, specifier),
    })),
  ]),
);
const graphRequestRows = graphSources.flatMap((relativePath) => fileEdges.get(relativePath));
assert.equal(graphRequestRows.length, expected.graphRequests, "graph request edge count changed");
assert.equal(
  Math.max(...graphRootList.map((rootPath) => closure(rootPath).length)),
  expected.maxClosure,
  "max graph closure changed",
);

const resolutionNegativeRoots = roots.filter(
  (relativePath) => metadata(relativePath).negativePhase === "resolution",
);
assert.equal(
  resolutionNegativeRoots.length,
  expected.resolutionNegatives,
  "resolution negative root count changed",
);
assert.equal(
  roots.length - resolutionNegativeRoots.length,
  expected.positiveRoots,
  "positive root count changed",
);
const dynamicPolicyRoots = graphRootList.filter(
  (rootPath) =>
    rootPath.startsWith(dynamicImportCohort) &&
    requestSpecifiers(rootPath).some(({ dynamic }) => dynamic),
);
assert.equal(
  dynamicPolicyRoots.length,
  expected.dynamicPolicyRoots,
  "dynamic-import policy root count changed",
);

// Every root closes within its own directory, imports no harness, and carries
// only the supported feature set.
const supportedFeatures = new Set([
  "",
  "generators",
  "export-star-as-namespace-from-module",
  "dynamic-import,generators",
]);
for (const relativePath of roots) {
  const shape = metadata(relativePath);
  assert.deepEqual(shape.includes, [], `${relativePath}: harness include added`);
  assert(shape.flags.includes("module"), `${relativePath}: lost module flag`);
  assert(
    supportedFeatures.has(shape.features.join(",")),
    `${relativePath}: unsupported feature shape ${shape.features.join(",")}`,
  );
  assert(
    !shape.negativePhase || shape.negativePhase === "resolution",
    `${relativePath}: unexpected negative phase ${shape.negativePhase}`,
  );
  assert(
    !shape.negativePhase || shape.negativeType === "SyntaxError",
    `${relativePath}: unexpected negative type`,
  );
  for (const request of requestSpecifiers(relativePath)) {
    const normalized = normalize(relativePath, request.specifier);
    assert(
      posix.dirname(normalized) === posix.dirname(relativePath),
      `${relativePath}: request escapes its directory: ${request.specifier}`,
    );
  }
}

// ---- canaries: the deferred surfaces must remain outside the cohort --------

const exclusionCanaries = [
  ...[...rootThrowRuntimeCanaries].map((p) => ["root-throw-runtime", p]),
  ["private-name", `${cohort}/privatename-not-valid-earlyerr-module-1.js`],
  ["top-level-await", `${cohort}/top-level-await/await-expr-resolution.js`],
  ["ambiguous-export", `${cohort}/ambiguous-export-bindings/error-export-from-named-as.js`],
  ["cross-family-fixture", `${cohort}/instn-resolve-empty-import.js`],
];
for (const [surface, relativePath] of exclusionCanaries) {
  assert(existsSync(join(suite, relativePath)), `missing ${surface} canary: ${relativePath}`);
  assert(!roots.includes(relativePath), `${surface} canary entered cohort: ${relativePath}`);
}

// ---- evidence renderers ----------------------------------------------------

const lines = (...values) => `${values.join("\n")}\n`;
const manifest = lines(...roots);
const sourceManifest = lines(...graphSources);
const rootedEdges = lines(
  "root_path\tbase_path\trequest_index\tspecifier\tnormalized_path",
  ...graphRootList.flatMap((rootPath) =>
    closure(rootPath).flatMap((base) =>
      fileEdges.get(base).map((request, requestIndex) =>
        [rootPath, base, requestIndex, request.specifier, request.normalized].join("\t"),
      ),
    ),
  ),
);
const closures = lines(
  "root_path\tclosure_files\trequest_edges",
  ...graphRootList.map((rootPath) => {
    const files = closure(rootPath);
    return [
      rootPath,
      files.length,
      files.reduce((count, relativePath) => count + fileEdges.get(relativePath).length, 0),
    ].join("\t");
  }),
);
const ledger = lines(
  "path\trole\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256",
  ...[...depFreeRoots, ...graphSources].sort().map((relativePath) => {
    const shape = metadata(relativePath);
    return [
      relativePath,
      graphRootList.includes(relativePath) || depFreeRoots.includes(relativePath)
        ? "root"
        : "fixture",
      shape.includes.join(","),
      shape.flags.join(","),
      shape.features.join(","),
      shape.negativePhase,
      shape.negativeType,
      sha256(source(relativePath)),
    ].join("\t");
  }),
);
const negatives = lines(...resolutionNegativeRoots);

// ---- admission rows --------------------------------------------------------

const admissionRecords = [
  ...depFreeRoots.map((relativePath) => {
    const shape = metadata(relativePath);
    return admissionRecord({
      kind: "module",
      group: admissionGroup,
      path: relativePath,
      source_sha256: sha256(source(relativePath)),
      includes: shape.includes,
      flags: shape.flags,
      features: shape.features,
      negative_phase: shape.negativePhase,
      negative_type: shape.negativeType,
    });
  }),
  ...graphSources.map((relativePath) => {
    const shape = metadata(relativePath);
    return admissionRecord({
      kind: "graph-file",
      group: admissionGroup,
      path: relativePath,
      source_sha256: sha256(source(relativePath)),
      includes: shape.includes,
      flags: shape.flags,
      features: shape.features,
      negative_phase: shape.negativePhase,
      negative_type: shape.negativeType,
    });
  }),
  ...graphSources.flatMap((relativePath) =>
    fileEdges.get(relativePath).map((request, requestIndex) =>
      admissionRecord({
        kind: "graph-request",
        group: admissionGroup,
        path: relativePath,
        request_index: requestIndex,
        specifier: request.specifier,
        normalized_path: request.normalized,
      }),
    ),
  ),
  ...graphRootList.map((rootPath) =>
    admissionRecord({
      kind: "graph-root",
      group: admissionGroup,
      path: rootPath,
      closure_file_count: closure(rootPath).length,
      priority: 0,
      ...(dynamicPolicyRoots.includes(rootPath) ? { policy: "initial-import-tree" } : {}),
    }),
  ),
];
const admissionRows = renderAdmissionRows(admissionRecords);
const admissionCandidate = `${admissionHeader}\n${admissionRows}`;

// ---- exact resolution diagnostic contracts ---------------------------------

const diagnosticRules = [
  [
    "module.circular-export",
    "js_resolve_export_throw_error",
    "module resolution reports a cycle while looking up an indirectly exported binding",
  ],
  [
    "module.dependency-parse-break",
    "emit_break",
    "module resolution surfaces a dependency parse error for a break outside a loop or switch",
  ],
  [
    "module.dependency-parse-lvalue",
    "get_lvalue",
    "module resolution surfaces a dependency parse error for an invalid assignment target",
  ],
];
const diagnosticRulesText = lines("rule\tquickjs_anchor\tdescription", ...diagnosticRules.map((r) => r.join("\t")));

// Probe the pinned runner exactly like the audit replay does: cwd = suite,
// `run-test262 -N --module test/...`, then strip nothing — the runner already
// emits suite-relative `test/...` module names in that configuration.
assert(existsSync(quickjsRunner), `missing pinned runner: ${quickjsRunner}`);
assert(existsSync(quickjsConfig) || mode !== "--check-current", `missing pinned config: ${quickjsConfig}`);
function probePinnedResolution(relativePath) {
  const result = spawnSync(quickjsRunner, ["-N", "--module", relativePath], {
    cwd: suite,
    encoding: "utf8",
  });
  assert.equal(result.signal, null, `${relativePath}: runner terminated by signal`);
  assert.notEqual(result.status, 0, `${relativePath}: pinned runner unexpectedly succeeded`);
  const transcript = `${result.stdout}${result.stderr}`.replaceAll("\r\n", "\n");
  const lines_ = transcript.split("\n");
  const errorLine = lines_.find((line) => /^[A-Za-z_$][\w$]*Error: /.test(line));
  assert(errorLine, `${relativePath}: pinned runner emitted no Error line:\n${transcript}`);
  const separator = errorLine.indexOf(": ");
  const type = errorLine.slice(0, separator);
  const message = errorLine.slice(separator + 2);
  const frame = lines_.find((line) => /^\s*at test\/.+:\d+:\d+\s*$/.test(line));
  return { type, message, frame };
}

function classifyResolutionRule(message) {
  if (message.startsWith("Could not find export")) return "module.missing-export";
  if (message.startsWith("circular reference")) return "module.circular-export";
  if (message === "break must be inside loop or switch") {
    return "module.dependency-parse-break";
  }
  if (message === "invalid increment/decrement operand") {
    return "module.dependency-parse-lvalue";
  }
  throw new Error(`unclassified resolution message: ${message}`);
}

const probedDiagnostics = new Map();
for (const relativePath of resolutionNegativeRoots) {
  const probed = probePinnedResolution(relativePath);
  assert.equal(probed.type, "SyntaxError", `${relativePath}: expected SyntaxError`);
  const rule = classifyResolutionRule(probed.message);
  let line = "";
  let column = "";
  let locationPolicy = "absent";
  if (probed.frame) {
    const match = probed.frame.match(/:(\d+):(\d+)\s*$/);
    assert(match, `${relativePath}: malformed location frame ${probed.frame}`);
    line = match[1];
    column = match[2];
    locationPolicy = "exact";
  }
  probedDiagnostics.set(relativePath, { rule, ...probed, line, column, locationPolicy });
}

// Recompute candidate table now that rules are known.
const candidateLines = lines(
  "path\tvariant\trule",
  ...resolutionNegativeRoots.map((relativePath) =>
    [relativePath, "sloppy", probedDiagnostics.get(relativePath).rule].join("\t"),
  ),
);

const negativeDiagnosticRecords = resolutionNegativeRoots.map((relativePath) => {
  const shape = metadata(relativePath);
  const probed = probedDiagnostics.get(relativePath);
  assert.equal(shape.negativeType, "SyntaxError");
  return [
    relativePath,
    "sloppy",
    sha256(source(relativePath)),
    "resolution",
    "SyntaxError",
    probed.rule,
    probed.message,
    probed.line,
    probed.column,
    probed.locationPolicy,
  ].join("\t");
});
const negativeDiagnostics = lines(
  "path\tvariant\tsource_sha256\tphase\ttype\trule\tmessage\tline\tcolumn\tlocation_policy",
  ...negativeDiagnosticRecords,
);

const evidence = new Map([
  ["dev-support/test262/generated/test262-module-depfree-bindings-a.txt", manifest],
  ["dev-support/test262/generated/test262-module-depfree-bindings-a-sources.txt", sourceManifest],
  ["dev-support/test262/generated/test262-module-depfree-bindings-a-edges.tsv", rootedEdges],
  ["dev-support/test262/generated/test262-module-depfree-bindings-a-closures.tsv", closures],
  ["dev-support/test262/generated/test262-module-depfree-bindings-a-ledger.tsv", ledger],
  ["dev-support/test262/generated/test262-module-depfree-bindings-a-negatives.txt", negatives],
  ["dev-support/test262/generated/test262-module-depfree-bindings-a-exclusions.tsv",
    lines("surface\tcanary_path", ...exclusionCanaries.map((record) => record.join("\t")))],
  ["dev-support/test262/generated/test262-module-depfree-bindings-a-admission-rows.tsv", admissionCandidate],
  ["dev-support/test262/generated/test262-module-depfree-bindings-a-negative-diagnostic-candidates.tsv", candidateLines],
  ["dev-support/test262/generated/test262-module-depfree-bindings-a-negative-diagnostic-rules.tsv", diagnosticRulesText],
  ["dev-support/test262/generated/test262-module-depfree-bindings-a-negative-diagnostics.tsv", negativeDiagnostics],
]);

// ---- modes -----------------------------------------------------------------

if (mode === "--admissions") {
  process.stdout.write(admissionRows);
} else if (mode === "--negative-diagnostics") {
  process.stdout.write(negativeDiagnostics);
} else if (mode === "--diagnostic-candidates") {
  process.stdout.write(candidateLines);
} else if (mode === "--diagnostic-rules") {
  process.stdout.write(diagnosticRulesText);
} else if (mode === "output") {
  assert(output, "--output requires a directory");
  for (const [relativePath, contents] of evidence) {
    writeFileSync(join(output, basename(relativePath)), contents);
  }
  console.log(`generated ${evidence.size} authenticated evidence files in ${output}`);
} else {
  assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
  for (const [relativePath, contents] of evidence) {
    assert.equal(
      readFileSync(join(root, relativePath), "utf8"),
      contents,
      `${relativePath} drifted`,
    );
  }
  const checkedDiagnosticText = readFileSync(checkedDiagnostics, "utf8");
  for (const record of negativeDiagnosticRecords) {
    assert(
      checkedDiagnosticText.includes(`\n${record}\n`),
      `${record.split("\t")[0]} diagnostic not promoted`,
    );
  }
  const checkedRulesText = readFileSync(checkedDiagnosticRules, "utf8");
  for (const rule of diagnosticRules) {
    assert(
      checkedRulesText.includes(`\n${rule.join("\t")}\n`),
      `${rule[0]} rule not promoted`,
    );
  }
  // The 12 audited resolution negatives must be in the live profile.
  const profileText = readFileSync(checkedProfile, "utf8");
  for (const relativePath of resolutionNegativeRoots) {
    assert(
      profileText.includes(`\n${relativePath}\n`) ||
        profileText.endsWith(`\n${relativePath}`),
      `${relativePath}: missing from [audited-negative-tests]`,
    );
  }
  console.log(
    `module-depfree-bindings-a: roots=${roots.length} depfree=${depFreeRoots.length} ` +
      `graph=${graphRootList.length} sources=${graphSources.length} ` +
      `requests=${graphRequestRows.length} resolution_negatives=${resolutionNegativeRoots.length}`,
  );
}
