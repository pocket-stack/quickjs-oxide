#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join, posix, resolve } from "node:path";

import {
  admissionRecord,
  assertAdmissionGroup,
  renderAdmissionRows,
} from "./test262-admission-data.mjs";

const root = resolve(import.meta.dirname, "../..");
const checkedSuite = join(root, "target/oracle/quickjs-2026-06-04/test262");
const checkedAdmissions = join(root, "dev-support/test262/admissions.tsv");
const checkedProfile = join(root, "compat/test262-oxide.conf");
const checkedFocused = join(root, "dev-support/test262/generated/test262-class-private-callables-b.txt");
const checkedDiagnostics = join(root, "dev-support/test262/negative-diagnostics.tsv");
const checkedRules = join(root, "dev-support/test262/negative-diagnostic-rules.tsv");
const checkedExemptions = join(root, "dev-support/test262/negative-diagnostic-exemptions.tsv");
const args = process.argv.slice(2);

function option(name, fallback) {
  const indexes = args.flatMap((value, index) => (value === name ? [index] : []));
  assert(indexes.length <= 1, `duplicate ${name}`);
  if (indexes.length === 0) return fallback;
  const value = args[indexes[0] + 1];
  assert(value && !value.startsWith("--"), `${name} requires a value`);
  return resolve(value);
}

const suite = option("--suite", checkedSuite);
const selectedModes = [
  "--admissions",
  "--diagnostic-candidates",
  "--check-current",
].filter((mode) => args.includes(mode));
assert(selectedModes.length <= 1, "select at most one output/check mode");
const mode = selectedModes[0] ?? "--check-current";

const valueOptions = new Set(["--suite"]);
const flagOptions = new Set([
  "--admissions",
  "--diagnostic-candidates",
  "--check-current",
]);
for (let index = 0; index < args.length; index += 1) {
  const argument = args[index];
  if (valueOptions.has(argument)) {
    index += 1;
  } else {
    assert(flagOptions.has(argument), `unknown option: ${argument}`);
  }
}

assert(existsSync(join(suite, "test")), `missing Test262 suite: ${suite}`);

const cohort = "test/language/expressions/dynamic-import";
const admissionGroup = "dynamic-import-a";
const fixture = `${cohort}/dynamic-import-module_FIXTURE.js`;
const usageFixture = `${cohort}/usage/dynamic-import-module_FIXTURE.js`;
const roots = [
  `${cohort}/always-create-new-promise.js`,
  `${cohort}/assign-expr-get-value-abrupt-throws.js`,
  `${cohort}/reuse-namespace-object.js`,
  `${cohort}/usage/top-level-import-then-returns-thenable.js`,
].sort();
const expectedSourceSha256 = new Map([
  [
    `${cohort}/always-create-new-promise.js`,
    "e11f060801c828fbd99052aeb90eb4fa94420eed9ddf5530983decd121420cc1",
  ],
  [
    `${cohort}/assign-expr-get-value-abrupt-throws.js`,
    "e856975950b76ebdd1447bd273f4a7e07485333a6fe1d82e356582577f732b23",
  ],
  [
    `${cohort}/reuse-namespace-object.js`,
    "aa09ffc882f21eabd879596e11e83655c2146b20f5a9031d5202a2960cdfa68b",
  ],
  [
    `${cohort}/usage/top-level-import-then-returns-thenable.js`,
    "3ffdd7ebe4968cdd6abebafb718b855e818a041735120dec4611c045f98ecf3f",
  ],
  [fixture, "08bd41ffb3fbbfe3a33b50b66d0ca03f6266e5a4468cafc266eb109c15d7a261"],
  [usageFixture, "08bd41ffb3fbbfe3a33b50b66d0ca03f6266e5a4468cafc266eb109c15d7a261"],
]);

const sha256 = (value) => createHash("sha256").update(value).digest("hex");
const bytewise = (left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right));
const source = (relativePath) => readFileSync(join(suite, relativePath), "utf8");
const frontmatter = (text) => text.match(/\/\*---[\s\S]*?---\*\/(?:\r?\n)?/)?.[0] ?? "";
const arrayField = (text, name) => {
  const match = text.match(new RegExp(`^${name}:\\s*\\[([^\\]]*)\\]`, "m"));
  return (
    match?.[1]
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean) ?? []
  );
};

function metadata(relativePath) {
  const text = frontmatter(source(relativePath));
  if (!text) {
    return { includes: [], flags: [], features: [], negativePhase: "", negativeType: "" };
  }
  const negative = text.match(
    /^negative:\s*\n\s*phase:\s*([^\s]+)\s*\n\s*type:\s*([^\s]+)\s*$/m,
  );
  return {
    includes: arrayField(text, "includes"),
    // The Rust metadata parser stores flags in a BTreeSet. Keep the admission
    // contract in the same canonical order rather than frontmatter order.
    flags: arrayField(text, "flags").sort(),
    features: arrayField(text, "features"),
    negativePhase: negative?.[1] ?? "",
    negativeType: negative?.[2] ?? "",
  };
}

const invalidDirectory = `${cohort}/syntax/invalid`;
const invalidFiles = readdirSync(join(suite, invalidDirectory), { withFileTypes: true })
  .filter((entry) => entry.isFile() && entry.name.endsWith(".js"))
  .map((entry) => `${invalidDirectory}/${entry.name}`)
  .sort(bytewise);
const noNewFiles = invalidFiles.filter((relativePath) =>
  relativePath.endsWith("-no-new-call-expression.js"),
);
const newTargetRoots = noNewFiles.filter((relativePath) =>
  source(relativePath).includes("// - src/dynamic-import/no-new-call-expression.case\n"),
);
const sourcePhaseCanaries = noNewFiles.filter((relativePath) =>
  source(relativePath).includes(
    "// - src/dynamic-import/import-source-no-new-call-expression.case\n",
  ),
);
const importDeferCanaries = noNewFiles.filter((relativePath) =>
  source(relativePath).includes(
    "// - src/dynamic-import/import-defer-no-new-call-expression.case\n",
  ),
);
const pathManifest = (paths) => `${paths.join("\n")}\n`;
assert.equal(noNewFiles.length, 63, "dynamic-import no-new surface matrix changed");
assert.equal(newTargetRoots.length, 21, "plain dynamic-import new-target cohort changed");
assert.equal(sourcePhaseCanaries.length, 21, "import.source new-target canaries changed");
assert.equal(importDeferCanaries.length, 21, "import.defer new-target canaries changed");
const discoveredNoNewFiles = [...newTargetRoots, ...sourcePhaseCanaries, ...importDeferCanaries]
  .sort(bytewise);
assert.equal(
  new Set(discoveredNoNewFiles).size,
  63,
  "dynamic-import no-new provenance cohorts overlap",
);
assert.deepEqual(discoveredNoNewFiles, noNewFiles, "dynamic-import no-new provenance coverage changed");
assert.equal(
  sha256(pathManifest(newTargetRoots)),
  "cded3adc5d0a858964a71bb3cbda146925c5ce221efc99ef4e6fb7cf2815473b",
  "plain dynamic-import new-target path manifest changed",
);
assert.equal(
  sha256(pathManifest(sourcePhaseCanaries)),
  "278258e827ba7b990e0c6981ee9d1cf5b10aa5a05187ac0c0479d009e4675142",
  "import.source new-target path manifest changed",
);
assert.equal(
  sha256(pathManifest(importDeferCanaries)),
  "254e1a06734073963a5c109509f6551aad1dfcd977bc5aad8bb47ec1744a53c6",
  "import.defer new-target path manifest changed",
);
const normalizeNewTargetSurface = (relativePath) =>
  relativePath.replace(/-import-(?:source|defer)-no-new-call-expression\.js$/u, "-no-new-call-expression.js");
assert.deepEqual(
  sourcePhaseCanaries.map(normalizeNewTargetSurface).sort(bytewise),
  newTargetRoots.map(normalizeNewTargetSurface).sort(bytewise),
  "import.source new-target stems changed",
);
assert.deepEqual(
  importDeferCanaries.map(normalizeNewTargetSurface).sort(bytewise),
  newTargetRoots.map(normalizeNewTargetSurface).sort(bytewise),
  "import.defer new-target stems changed",
);

const assignmentTargetRoots = invalidFiles.filter((relativePath) =>
  /\/invalid-assignmenttargettype-syntax-error-[0-9]+-[^/]+\.js$/u.test(relativePath),
);
assert.equal(
  assignmentTargetRoots.length,
  17,
  "dynamic-import assignment-target negative cohort changed",
);
assert.equal(
  sha256(pathManifest(assignmentTargetRoots)),
  "79db48d835028190823247d0dab8a0587e82da01dd7d20a766c1c04476981803",
  "dynamic-import assignment-target path manifest changed",
);

function parseNegativeShape(relativePath) {
  const shape = metadata(relativePath);
  assert.deepEqual(shape.includes, [], `${relativePath}: includes changed`);
  assert.equal(shape.negativePhase, "parse", `${relativePath}: negative phase changed`);
  assert.equal(shape.negativeType, "SyntaxError", `${relativePath}: negative type changed`);
  return JSON.stringify({ flags: shape.flags, features: shape.features });
}

function assertShapeCounts(paths, expected, label) {
  const actual = new Map();
  for (const relativePath of paths) {
    const shape = parseNegativeShape(relativePath);
    actual.set(shape, (actual.get(shape) ?? 0) + 1);
  }
  assert.deepEqual(
    [...actual].sort(([left], [right]) => bytewise(left, right)),
    [...expected].sort(([left], [right]) => bytewise(left, right)),
    `${label} metadata shapes changed`,
  );
}

const shape = (flags, features) => JSON.stringify({ flags, features });
const generatedDynamicImportShapes = new Map([
  [shape(["generated"], ["dynamic-import"]), 18],
  [shape(["generated"], ["dynamic-import", "async-iteration"]), 1],
  [shape(["generated", "noStrict"], ["dynamic-import"]), 2],
]);
assertShapeCounts(
  newTargetRoots,
  generatedDynamicImportShapes,
  "plain dynamic-import new-target",
);
assertShapeCounts(
  sourcePhaseCanaries,
  new Map([
    [shape(["generated"], ["source-phase-imports", "source-phase-imports-module-source", "dynamic-import"]), 18],
    [shape(["generated"], ["source-phase-imports", "source-phase-imports-module-source", "dynamic-import", "async-iteration"]), 1],
    [shape(["generated", "noStrict"], ["source-phase-imports", "source-phase-imports-module-source", "dynamic-import"]), 2],
  ]),
  "import.source new-target canary",
);
assertShapeCounts(
  importDeferCanaries,
  new Map([
    [shape(["generated"], ["import-defer", "dynamic-import"]), 18],
    [shape(["generated"], ["import-defer", "dynamic-import", "async-iteration"]), 1],
    [shape(["generated", "noStrict"], ["import-defer", "dynamic-import"]), 2],
  ]),
  "import.defer new-target canary",
);
assertShapeCounts(
  assignmentTargetRoots,
  new Map([
    [shape([], ["dynamic-import"]), 16],
    [shape([], ["dynamic-import", "exponentiation"]), 1],
  ]),
  "dynamic-import assignment-target negative",
);

function variants(relativePath) {
  const flags = metadata(relativePath).flags;
  assert(
    !(flags.includes("noStrict") && flags.includes("onlyStrict")),
    `${relativePath}: mutually exclusive strictness flags`,
  );
  if (flags.includes("noStrict")) return ["sloppy"];
  if (flags.includes("onlyStrict")) return ["strict"];
  return ["sloppy", "strict"];
}

const variantCount = (paths) =>
  paths.reduce((count, relativePath) => count + variants(relativePath).length, 0);
assert.equal(variantCount(newTargetRoots), 40, "plain new-target variant count changed");
assert.equal(variantCount(sourcePhaseCanaries), 40, "import.source canary variant count changed");
assert.equal(variantCount(importDeferCanaries), 40, "import.defer canary variant count changed");

const parseNegativeFamilies = [
  {
    marker: "assignment-expr-not-optional.case",
    label: "missing-specifier",
    manifestSha256: "507096fea67dbbd387d5bdbe1621259cc0a335d185d457c00cc531cfe81fcf67",
    rule: "dynamic-import.missing-specifier",
    message: "unexpected token in expression: ')'",
  },
  {
    marker: "no-rest-param.case",
    label: "spread-argument",
    manifestSha256: "605898c553a1bc4488167a924addc84b22458f66498a781344f50f123bbea11c",
    rule: "dynamic-import.spread-argument",
    message: "unexpected token in expression: '...'",
  },
  {
    marker: "not-extensible-args.case",
    label: "excess-argument",
    manifestSha256: "656bb173d8aca2c7d6bea5e869f8324e7a7ef76da851dcb59cd07e59a34c3d58",
    rule: "dynamic-import.excess-argument",
    message: "expecting ')'",
  },
  {
    marker: "typeof-import.case",
    label: "typeof-keyword",
    manifestSha256: "3c499a164b8963d74c3c10facbd675d2ba13780c24a0787674907e21a619b328",
    rule: "dynamic-import.typeof-keyword",
    message: "expecting '('",
  },
].map((family) => ({
  ...family,
  paths: invalidFiles.filter((relativePath) =>
    source(relativePath).includes(`// - src/dynamic-import/${family.marker}\n`),
  ),
}));

for (const family of parseNegativeFamilies) {
  assert.equal(family.paths.length, 21, `${family.label} path count changed`);
  assert.equal(
    sha256(pathManifest(family.paths)),
    family.manifestSha256,
    `${family.label} path manifest changed`,
  );
  assertShapeCounts(family.paths, generatedDynamicImportShapes, family.label);
  assert.equal(variantCount(family.paths), 40, `${family.label} variant count changed`);
}

const escapedKeywordPath = `${cohort}/escape-sequence-import.js`;
const secondArgumentYieldPath =
  `${cohort}/import-attributes/2nd-param-yield-ident-invalid.js`;
assert.equal(
  sha256(source(escapedKeywordPath)),
  "144e19cd58f4843588295c4ba911a46cd3bce330f7c2204f5d27db6168a45b8c",
  "escaped import keyword source changed",
);
assert.equal(
  sha256(source(secondArgumentYieldPath)),
  "9a59d601e13bcff394660be1e46bb9ff567780d6a3c404b837c43990a1f6e3c2",
  "second-argument yield source changed",
);
assert.deepEqual(metadata(escapedKeywordPath), {
  includes: [],
  flags: [],
  features: ["dynamic-import"],
  negativePhase: "parse",
  negativeType: "SyntaxError",
});
assert.deepEqual(metadata(secondArgumentYieldPath), {
  includes: [],
  flags: ["onlyStrict"],
  features: ["dynamic-import", "import-attributes"],
  negativePhase: "parse",
  negativeType: "SyntaxError",
});

const parseNegativeRoots = [
  ...parseNegativeFamilies.flatMap((family) => family.paths),
  escapedKeywordPath,
  secondArgumentYieldPath,
].sort(bytewise);
assert.equal(parseNegativeRoots.length, 86, "dynamic-import parse-negative path count changed");
assert.equal(
  new Set(parseNegativeRoots).size,
  parseNegativeRoots.length,
  "dynamic-import parse-negative paths overlap",
);
assert.equal(
  sha256(pathManifest(parseNegativeRoots)),
  "591730673c502fc1a3d23222a3479fe2c3edc8196cb40e0fe03f6f3bd3179359",
  "dynamic-import parse-negative path manifest changed",
);
assert.equal(
  variantCount(parseNegativeRoots),
  163,
  "dynamic-import parse-negative variant count changed",
);

const futureSyntaxCanaries = invalidFiles.filter((relativePath) => {
  const features = metadata(relativePath).features;
  return features.includes("source-phase-imports") || features.includes("import-defer");
});
assert.equal(futureSyntaxCanaries.length, 189, "future dynamic-import canary count changed");
assert.equal(
  sha256(pathManifest(futureSyntaxCanaries)),
  "fb816445fc1057f7d8a2a2d4fc80f3ccb496d67efcfab66dad357d3cb44cdf73",
  "future dynamic-import canary path manifest changed",
);
assert(
  parseNegativeRoots.every((relativePath) => !futureSyntaxCanaries.includes(relativePath)),
  "future dynamic-import syntax entered the parse-negative cohort",
);

function javascriptFiles(relativeDirectory) {
  return readdirSync(join(suite, relativeDirectory), { withFileTypes: true })
    .flatMap((entry) => {
      const relativePath = `${relativeDirectory}/${entry.name}`;
      if (entry.isDirectory()) return javascriptFiles(relativePath);
      return entry.isFile() && entry.name.endsWith(".js") ? [relativePath] : [];
    })
    .sort(bytewise);
}

const moduleAdmissionGroup = "dynamic-import-module-a";
const moduleDynamicImportSurface = javascriptFiles("test").filter((relativePath) => {
  const shape = metadata(relativePath);
  return shape.flags.includes("module") && shape.features.includes("dynamic-import");
});
const moduleGoalRoots = moduleDynamicImportSurface.filter((relativePath) => {
  const features = metadata(relativePath).features;
  return features.length === 1 && features[0] === "dynamic-import";
});
const moduleGoalCanaries = moduleDynamicImportSurface.filter(
  (relativePath) => !moduleGoalRoots.includes(relativePath),
);
assert.equal(moduleGoalRoots.length, 27, "dynamic-import Module-goal root count changed");
assert.equal(moduleGoalCanaries.length, 15, "dynamic-import Module-goal canary count changed");
assert.equal(
  sha256(pathManifest(moduleGoalRoots)),
  "f29c503a2b0e20da8fa72683443cd884cdb425d9904479afb69820b5e17914e0",
  "dynamic-import Module-goal root path manifest changed",
);
for (const relativePath of moduleGoalRoots) {
  const shape = metadata(relativePath);
  assert(shape.flags.includes("async"), `${relativePath}: Module-goal root lost async flag`);
  assert(shape.flags.includes("module"), `${relativePath}: Module-goal root lost module flag`);
  assert.deepEqual(shape.features, ["dynamic-import"], `${relativePath}: feature shape changed`);
  assert.equal(shape.negativePhase, "", `${relativePath}: negative phase changed`);
  assert.equal(shape.negativeType, "", `${relativePath}: negative type changed`);
}

function requestSpecifiers(relativePath) {
  const body = source(relativePath).replace(/\/\*---[\s\S]*?---\*\//, "");
  const requests = [];
  const importCall = /\bimport\s*\(\s*(["'])([^"']+)\1/g;
  for (const match of body.matchAll(importCall)) {
    if (!requests.includes(match[2])) requests.push(match[2]);
  }
  for (const request of requests) {
    assert(request.startsWith("./"), `${relativePath}: non-child request ${request}`);
    assert(!request.includes("/../"), `${relativePath}: escaping request ${request}`);
  }
  return requests;
}

function moduleRequestSpecifiers(relativePath) {
  const body = source(relativePath).replace(/\/\*---[\s\S]*?---\*\//, "");
  const matches = [];
  const literalRequestPatterns = [
    /\bimport\s*\(\s*(["'])([^"']+)\1/g,
    /\bimport\s+(?!\()(?:(?:[\w*{},\s]+)\s+from\s+)?(["'])([^"']+)\1/g,
    /\bexport\s+(?:\*|\{[^}]*\})[^'";]*\bfrom\s*(["'])([^"']+)\1/g,
  ];
  for (const pattern of literalRequestPatterns) {
    for (const match of body.matchAll(pattern)) {
      matches.push({ offset: match.index, specifier: match[2] });
    }
  }
  matches.sort((left, right) => left.offset - right.offset);
  const requests = [];
  for (const { specifier } of matches) {
    if (!requests.includes(specifier)) requests.push(specifier);
  }
  for (const request of requests) {
    assert(request.startsWith("./"), `${relativePath}: non-child request ${request}`);
    assert(!request.includes("/../"), `${relativePath}: escaping request ${request}`);
  }
  return requests;
}

const normalize = (base, request) => posix.join(posix.dirname(base), request);
const sources = [...roots, fixture, usageFixture].sort();
const fileEdges = new Map(
  sources.map((base) => [
    base,
    requestSpecifiers(base).map((specifier) => ({
      specifier,
      normalized: normalize(base, specifier),
    })),
  ]),
);

for (const relativePath of sources) {
  assert.equal(
    sha256(source(relativePath)),
    expectedSourceSha256.get(relativePath),
    `${relativePath}: pinned source changed`,
  );
}
for (const rootPath of roots) {
  const shape = metadata(rootPath);
  assert.deepEqual(shape.features, ["dynamic-import"], `${rootPath}: feature shape changed`);
  assert(!shape.flags.includes("module"), `${rootPath}: Script goal changed`);
  assert.equal(shape.negativePhase, "", `${rootPath}: negative phase changed`);
  assert.equal(shape.negativeType, "", `${rootPath}: negative type changed`);
}
assert.deepEqual(metadata(fixture), {
  includes: [],
  flags: [],
  features: [],
  negativePhase: "",
  negativeType: "",
});
assert.deepEqual(fileEdges.get(`${cohort}/assign-expr-get-value-abrupt-throws.js`), []);
for (const rootPath of roots.filter((path) => !path.endsWith("assign-expr-get-value-abrupt-throws.js"))) {
  const normalized = rootPath.includes("/usage/") ? usageFixture : fixture;
  assert.deepEqual(fileEdges.get(rootPath), [
    { specifier: "./dynamic-import-module_FIXTURE.js", normalized },
  ]);
}
assert.deepEqual(fileEdges.get(fixture), []);
assert.deepEqual(fileEdges.get(usageFixture), []);

const closureSize = (rootPath) => (fileEdges.get(rootPath).length === 0 ? 1 : 2);
const admissionRecords = [
  ...sources.map((relativePath) => {
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
  ...sources.flatMap((relativePath) =>
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
  ...roots.map((rootPath) =>
    admissionRecord({
      kind: "dynamic-import-root",
      group: admissionGroup,
      path: rootPath,
      closure_file_count: closureSize(rootPath),
      priority: 0,
      policy: "initial-import-tree",
    }),
  ),
];

assert.equal(roots.length, 4);
assert.equal(sources.length, 6);
assert.equal(
  [...fileEdges.values()].reduce((count, requests) => count + requests.length, 0),
  3,
);
assert.equal(admissionRecords.length, 13);

// dynamic-import/usage graph closure cohort (S1 C1a). The whole directory is a
// generated matrix of 6 cases x 18 syntax templates: 108 Script-goal async
// roots that each dynamically import exactly one of four dependency-free
// fixtures. 12 roots (the async-gen templates x two template families) also
// carry async-iteration. 18 roots pull in fnGlobalObject.js. The
// specifier-tostring case names its request through the standard generated
// `// import('...')` annotation rather than a literal call, which the shared
// request scanner already records. Two variants (sloppy + strict) per root
// give 216 variants.
const usageAdmissionGroup = "dynamic-import-usage-a";
const usageDirectory = `${cohort}/usage`;
const usageFiles = readdirSync(join(suite, usageDirectory), { withFileTypes: true })
  .filter((entry) => entry.isFile() && entry.name.endsWith(".js"))
  .map((entry) => `${usageDirectory}/${entry.name}`)
  .sort(bytewise);
const usageFixturePaths = usageFiles.filter((relativePath) =>
  relativePath.endsWith("_FIXTURE.js"),
);
const usageRootPaths = usageFiles.filter(
  (relativePath) => !usageFixturePaths.includes(relativePath),
);
assert.equal(usageFixturePaths.length, 4, "dynamic-import usage fixture count changed");
assert.equal(usageRootPaths.length, 108, "dynamic-import usage root count changed");
assert.equal(
  new Set(usageFiles).size,
  usageFiles.length,
  "dynamic-import usage files overlap",
);
assert.equal(
  sha256(pathManifest(usageRootPaths)),
  "778d3e7805823de523987fa84776fdd5732c163ea14ad3c73b68f4438e2d8212",
  "dynamic-import usage root path manifest changed",
);
assert.equal(
  sha256(pathManifest(usageFixturePaths)),
  "1f9465e41fbc1ac213bcd36081ed310e39f914a15874c6d1ced1506cf87f1a32",
  "dynamic-import usage fixture path manifest changed",
);
const expectedUsageFixtureHashes = new Map([
  [`${usageDirectory}/dynamic-import-module_FIXTURE.js`,
    "08bd41ffb3fbbfe3a33b50b66d0ca03f6266e5a4468cafc266eb109c15d7a261"],
  [`${usageDirectory}/eval-gtbndng-indirect-update-dflt_FIXTURE.js`,
    "12a289fc58e54afefbf54b974ac7db0432b3dca1815d3c7f0fa7d4f639b0fe1f"],
  [`${usageDirectory}/eval-gtbndng-indirect-update_FIXTURE.js`,
    "86f9d73e4f721d046412952d46a9fdeb2864fb6bdc2917d995170945d6f7800b"],
  [`${usageDirectory}/module-code_FIXTURE.js`,
    "ca4d734fe7ffe820cb22917ddee0d077afada2cdc2e7ad18784e623888fe2b9b"],
]);
for (const relativePath of usageFixturePaths) {
  assert.equal(
    sha256(source(relativePath)),
    expectedUsageFixtureHashes.get(relativePath),
    `${relativePath}: pinned usage fixture source changed`,
  );
  assert.deepEqual(
    metadata(relativePath),
    { includes: [], flags: [], features: [], negativePhase: "", negativeType: "" },
    `${relativePath}: usage fixture metadata changed`,
  );
}

const usageShapeCounts = new Map();
for (const rootPath of usageRootPaths) {
  const shape = metadata(rootPath);
  assert.deepEqual(shape.flags, ["async", "generated"], `${rootPath}: usage flags changed`);
  assert(
    shape.features.length === 1
      ? shape.features[0] === "dynamic-import"
      : shape.features.length === 2
        && shape.features[0] === "dynamic-import"
        && shape.features[1] === "async-iteration",
    `${rootPath}: usage features changed`,
  );
  assert(
    shape.includes.length === 0 || shape.includes[0] === "fnGlobalObject.js",
    `${rootPath}: unexpected usage include`,
  );
  assert(!shape.flags.includes("module"), `${rootPath}: usage root became a Module`);
  assert.equal(shape.negativePhase, "", `${rootPath}: usage root gained a negative phase`);
  assert.equal(shape.negativeType, "", `${rootPath}: usage root gained a negative type`);
  const shapeKey = JSON.stringify({
    features: shape.features,
    includes: shape.includes,
  });
  usageShapeCounts.set(shapeKey, (usageShapeCounts.get(shapeKey) ?? 0) + 1);
}
assert.deepEqual(
  [...usageShapeCounts].sort(([left], [right]) => bytewise(left, right)),
  [
    [
      JSON.stringify({
        features: ["dynamic-import", "async-iteration"],
        includes: ["fnGlobalObject.js"],
      }),
      2,
    ],
    [
      JSON.stringify({ features: ["dynamic-import", "async-iteration"], includes: [] }),
      10,
    ],
    [JSON.stringify({ features: ["dynamic-import"], includes: ["fnGlobalObject.js"] }), 16],
    [JSON.stringify({ features: ["dynamic-import"], includes: [] }), 80],
  ],
  "dynamic-import usage metadata shape matrix changed",
);

const usageFileEdges = new Map(
  usageFiles.map((relativePath) => [
    relativePath,
    requestSpecifiers(relativePath).map((specifier) => ({
      specifier,
      normalized: normalize(relativePath, specifier),
    })),
  ]),
);
for (const fixturePath of usageFixturePaths) {
  assert.deepEqual(
    usageFileEdges.get(fixturePath),
    [],
    `${fixturePath}: usage fixture gained an import`,
  );
}
const usageFixtureBySpecifier = new Map(
  usageFixturePaths.map((relativePath) => [`./${relativePath.split("/").at(-1)}`, relativePath]),
);
for (const rootPath of usageRootPaths) {
  const requests = usageFileEdges.get(rootPath);
  assert.equal(requests.length, 1, `${rootPath}: usage root must request exactly one fixture`);
  const [request] = requests;
  assert.equal(
    request.normalized,
    usageFixtureBySpecifier.get(request.specifier),
    `${rootPath}: usage request does not resolve to its pinned fixture`,
  );
}
const usageRequestCount = [...usageFileEdges.values()].reduce(
  (count, requests) => count + requests.length,
  0,
);
assert.equal(usageRequestCount, 108, "dynamic-import usage edge count changed");
assert.equal(variantCount(usageRootPaths), 216, "dynamic-import usage variant count changed");

// Priority 1 keeps the earlier dynamic-import-a admission authoritative for
// the one usage root that group already owns; the catalog keys roots by
// (group, path), so the duplicate root row here is an independent audit.
const usageAdmissionRecords = [
  ...usageFiles.map((relativePath) => {
    const shape = metadata(relativePath);
    return admissionRecord({
      kind: "graph-file",
      group: usageAdmissionGroup,
      path: relativePath,
      source_sha256: sha256(source(relativePath)),
      includes: shape.includes,
      flags: shape.flags,
      features: shape.features,
      negative_phase: shape.negativePhase,
      negative_type: shape.negativeType,
    });
  }),
  ...usageFiles.flatMap((relativePath) =>
    usageFileEdges.get(relativePath).map((request, requestIndex) =>
      admissionRecord({
        kind: "graph-request",
        group: usageAdmissionGroup,
        path: relativePath,
        request_index: requestIndex,
        specifier: request.specifier,
        normalized_path: request.normalized,
      }),
    ),
  ),
  ...usageRootPaths.map((rootPath) =>
    admissionRecord({
      kind: "dynamic-import-root",
      group: usageAdmissionGroup,
      path: rootPath,
      closure_file_count: 2,
      priority: 1,
      policy: "initial-import-tree",
    }),
  ),
];
assert.equal(usageAdmissionRecords.length, 328);

const moduleFileEdges = new Map();
const pendingModuleFiles = [...moduleGoalRoots].reverse();
while (pendingModuleFiles.length !== 0) {
  const relativePath = pendingModuleFiles.pop();
  if (moduleFileEdges.has(relativePath)) continue;
  const requests = moduleRequestSpecifiers(relativePath).map((specifier) => ({
    specifier,
    normalized: normalize(relativePath, specifier),
  }));
  moduleFileEdges.set(relativePath, requests);
  for (const request of requests.toReversed()) {
    assert(
      existsSync(join(suite, request.normalized)),
      `${relativePath}: missing requested module ${request.normalized}`,
    );
    if (!moduleFileEdges.has(request.normalized)) pendingModuleFiles.push(request.normalized);
  }
}
const moduleSources = [...moduleFileEdges.keys()].sort(bytewise);
const moduleEdgeLines = moduleSources.flatMap((relativePath) =>
  moduleFileEdges
    .get(relativePath)
    .map((request) => `${relativePath}\t${request.specifier}\t${request.normalized}`),
).sort(bytewise);
assert.equal(moduleSources.length, 31, "dynamic-import Module-goal file count changed");
assert.equal(moduleEdgeLines.length, 32, "dynamic-import Module-goal edge count changed");
assert.equal(
  sha256(pathManifest(moduleSources)),
  "e75c150f36519ec500e65f25cbe7670b5115f98a3845955379eb98589b06242c",
  "dynamic-import Module-goal file path manifest changed",
);
assert.equal(
  sha256(pathManifest(moduleEdgeLines)),
  "ff3a0ce8abb2962cbe255cce10320d5e66cc48a27a373815eaf052c7629fe130",
  "dynamic-import Module-goal edge manifest changed",
);

function moduleClosureSize(rootPath) {
  const visited = new Set();
  const pending = [rootPath];
  while (pending.length !== 0) {
    const relativePath = pending.pop();
    if (visited.has(relativePath)) continue;
    visited.add(relativePath);
    for (const request of moduleFileEdges.get(relativePath).toReversed()) {
      pending.push(request.normalized);
    }
  }
  return visited.size;
}

const moduleClosureDistribution = new Map();
for (const rootPath of moduleGoalRoots) {
  const size = moduleClosureSize(rootPath);
  moduleClosureDistribution.set(size, (moduleClosureDistribution.get(size) ?? 0) + 1);
}
assert.deepEqual(
  [...moduleClosureDistribution].sort(([left], [right]) => left - right),
  [
    [1, 11],
    [2, 15],
    [3, 1],
  ],
  "dynamic-import Module-goal closure distribution changed",
);

const moduleAdmissionRecords = [
  ...moduleSources.map((relativePath) => {
    const shape = metadata(relativePath);
    return admissionRecord({
      kind: "graph-file",
      group: moduleAdmissionGroup,
      path: relativePath,
      source_sha256: sha256(source(relativePath)),
      includes: shape.includes,
      flags: shape.flags,
      features: shape.features,
      negative_phase: shape.negativePhase,
      negative_type: shape.negativeType,
    });
  }),
  ...moduleSources.flatMap((relativePath) =>
    moduleFileEdges.get(relativePath).map((request, requestIndex) =>
      admissionRecord({
        kind: "graph-request",
        group: moduleAdmissionGroup,
        path: relativePath,
        request_index: requestIndex,
        specifier: request.specifier,
        normalized_path: request.normalized,
      }),
    ),
  ),
  ...moduleGoalRoots.map((rootPath) =>
    admissionRecord({
      kind: "graph-root",
      group: moduleAdmissionGroup,
      path: rootPath,
      closure_file_count: moduleClosureSize(rootPath),
      priority: 0,
      policy: "initial-import-tree",
    }),
  ),
];
assert.equal(moduleAdmissionRecords.length, 90);

function parseRejectedAdmissionRecords(paths, group) {
  return [
    ...paths.map((relativePath) => {
      const shape = metadata(relativePath);
      return admissionRecord({
        kind: "graph-file",
        group,
        path: relativePath,
        source_sha256: sha256(source(relativePath)),
        includes: shape.includes,
        flags: shape.flags,
        features: shape.features,
        negative_phase: shape.negativePhase,
        negative_type: shape.negativeType,
      });
    }),
    ...paths.map((relativePath) =>
      admissionRecord({
        kind: "dynamic-import-root",
        group,
        path: relativePath,
        closure_file_count: 1,
        priority: 0,
        policy: "parse-rejected",
      }),
    ),
  ];
}

const assignmentTargetGroup = "dynamic-import-invalid-assignment-target-a";
const assignmentTargetAdmissionRecords = parseRejectedAdmissionRecords(
  assignmentTargetRoots,
  assignmentTargetGroup,
);
const newTargetGroup = "dynamic-import-new-target-a";
const newTargetAdmissionRecords = parseRejectedAdmissionRecords(
  newTargetRoots,
  newTargetGroup,
);
const parseNegativeGroup = "dynamic-import-parse-negative-b";
const parseNegativeAdmissionRecords = parseRejectedAdmissionRecords(
  parseNegativeRoots,
  parseNegativeGroup,
);
assert.equal(assignmentTargetAdmissionRecords.length, 34);
assert.equal(newTargetAdmissionRecords.length, 42);
assert.equal(parseNegativeAdmissionRecords.length, 172);

// C1b: dynamic-import/syntax/valid positive roots. Every authored test is a
// Script-goal syntax probe whose only literal request is the dependency-free
// empty_FIXTURE; the module is never meaningfully instantiated before the
// assertion observes a TypeError or the expression is only compiled (nested
// closures, import(''), new-cover). They therefore admit under a closed
// one/two-file initial-import-tree policy with no transitive graph. Files
// carrying an upstream-=skip future token (import-defer / import.source) stay
// canaries and must not enter the catalog.
const validSyntaxGroup = "dynamic-import-syntax-valid-a";
const validSyntaxDirectory = `${cohort}/syntax/valid`;
const closedFixture = `${validSyntaxDirectory}/empty_FIXTURE.js`;
const validSyntaxFutureTokens = new Set([
  "import-defer",
  "source-phase-imports",
  "source-phase-imports-module-source",
]);
const validSyntaxAllowedTokens = new Set([
  "dynamic-import",
  "async-iteration",
  "import-attributes",
]);
const validSyntaxFiles = readdirSync(join(suite, validSyntaxDirectory), { withFileTypes: true })
  .filter((entry) => entry.isFile() && entry.name.endsWith(".js"))
  .map((entry) => `${validSyntaxDirectory}/${entry.name}`)
  .sort(bytewise);
assert.equal(validSyntaxFiles.length, 193, "dynamic-import syntax/valid surface changed");
const validSyntaxCanaries = [];
const closedSyntaxRoots = [];
for (const relativePath of validSyntaxFiles) {
  if (relativePath === closedFixture) continue;
  const shape = metadata(relativePath);
  assert(
    shape.features.includes("dynamic-import"),
    `${relativePath}: syntax/valid file lost the dynamic-import feature`,
  );
  if (shape.features.some((feature) => validSyntaxFutureTokens.has(feature))) {
    validSyntaxCanaries.push(relativePath);
  } else {
    assert.equal(shape.negativePhase, "", `${relativePath}: positive root gained a negative phase`);
    assert(
      shape.features.every((feature) => validSyntaxAllowedTokens.has(feature)),
      `${relativePath}: unexpected cohort feature ${shape.features.join(",")}`,
    );
    closedSyntaxRoots.push(relativePath);
  }
}
assert.equal(closedSyntaxRoots.length, 108, "dynamic-import closed-root cohort changed");
assert.equal(validSyntaxCanaries.length, 84, "dynamic-import syntax/valid canary cohort changed");
assert.equal(
  new Set([...closedSyntaxRoots, ...validSyntaxCanaries, closedFixture]).size,
  193,
  "dynamic-import syntax/valid cohorts overlap",
);
assert.equal(
  sha256(pathManifest(closedSyntaxRoots)),
  "f04254868b28e7110efe903505e69f731dc871111f72637bc77d635b650fcccf",
  "dynamic-import closed-root path manifest changed",
);
assert.equal(
  sha256(pathManifest(validSyntaxCanaries)),
  "62c4339db029d22dd55fb8e3f20e0d4af47aa54b77eb89b4af47ed3e7486f3b2",
  "dynamic-import syntax/valid canary path manifest changed",
);
const closedFeatureShape = (features) => JSON.stringify({ flags: [], features });
const closedFeatureShapes = new Map();
for (const relativePath of closedSyntaxRoots) {
  const key = closedFeatureShape(metadata(relativePath).features);
  closedFeatureShapes.set(key, (closedFeatureShapes.get(key) ?? 0) + 1);
}
assert.deepEqual(
  [...closedFeatureShapes].sort(([left], [right]) => bytewise(left, right)),
  [
    [closedFeatureShape(["dynamic-import", "async-iteration"]), 3],
    [closedFeatureShape(["dynamic-import"]), 63],
    [
      closedFeatureShape(["import-attributes", "dynamic-import", "async-iteration"]),
      2,
    ],
    [closedFeatureShape(["import-attributes", "dynamic-import"]), 40],
  ],
  "dynamic-import closed-root feature shapes changed",
);
const closedFlagShapes = new Map();
for (const relativePath of closedSyntaxRoots) {
  const key = JSON.stringify(metadata(relativePath).flags);
  closedFlagShapes.set(key, (closedFlagShapes.get(key) ?? 0) + 1);
}
assert.deepEqual(
  [...closedFlagShapes].sort(([left], [right]) => bytewise(left, right)),
  [
    [JSON.stringify(["generated", "noStrict"]), 10],
    [JSON.stringify(["generated"]), 95],
    [JSON.stringify([]), 3],
  ],
  "dynamic-import closed-root flag shapes changed",
);
assert.equal(variantCount(closedSyntaxRoots), 206, "dynamic-import closed-root variant count changed");

assert.equal(
  sha256(source(closedFixture)),
  "a2094af42316bee0dfd33e1d4e5dd95cb42c51df911929ee01afe9f36714f52d",
  "dynamic-import closed-root fixture source changed",
);
assert.deepEqual(metadata(closedFixture), {
  includes: [],
  flags: [],
  features: [],
  negativePhase: "",
  negativeType: "",
});
const closedSources = [closedFixture, ...closedSyntaxRoots].sort(bytewise);
assert.equal(closedSources.length, 109, "dynamic-import closed-root source count changed");
assert.equal(
  sha256(pathManifest(closedSources)),
  "9469673eafc703d38fb066fa75ed9469da8229252bbdc52e6f68db78841a81e8",
  "dynamic-import closed-root source path manifest changed",
);

const closedEdges = new Map(
  closedSyntaxRoots.map((relativePath) => [
    relativePath,
    requestSpecifiers(relativePath).map((specifier) => ({
      specifier,
      normalized: normalize(relativePath, specifier),
    })),
  ]),
);
const closedEdgeLines = closedSyntaxRoots.flatMap((relativePath) =>
  closedEdges
    .get(relativePath)
    .flatMap((request, requestIndex) => [
      `${relativePath}\t${requestIndex}\t${request.specifier}\t${request.normalized}`,
    ]),
);
const closedZeroEdgeRoots = closedSyntaxRoots
  .filter((relativePath) => closedEdges.get(relativePath).length === 0)
  .sort(bytewise);
const closedEdgeRoots = closedSyntaxRoots.filter(
  (relativePath) => closedEdges.get(relativePath).length === 1,
);
assert.equal(
  closedZeroEdgeRoots.length + closedEdgeRoots.length,
  closedSyntaxRoots.length,
  "dynamic-import closed root has an unexpected request count",
);
assert.equal(closedZeroEdgeRoots.length, 22, "dynamic-import import('') cohort changed");
assert.equal(closedEdgeRoots.length, 86, "dynamic-import fixture-import cohort changed");
assert.equal(closedEdgeLines.length, 86, "dynamic-import closed-root edge count changed");
for (const relativePath of closedEdgeRoots) {
  assert.deepEqual(
    closedEdges.get(relativePath),
    [{ specifier: "./empty_FIXTURE.js", normalized: closedFixture }],
    `${relativePath}: closed-root request shape changed`,
  );
}
assert.equal(
  sha256(pathManifest(closedZeroEdgeRoots)),
  "1e6bfb0bb475c63b7e50122cfb8c23ae6771ba0b753f45ba0c48a67fd675c2d3",
  "dynamic-import import('') path manifest changed",
);
assert.equal(
  sha256(pathManifest(closedEdgeLines)),
  "db75a9cc8fc31c8bc197db041097a3edfd6e733da5f610eb399bd4ce7216346d",
  "dynamic-import closed-root edge manifest changed",
);

const closedSyntaxAdmissionRecords = [
  ...closedSources.map((relativePath) => {
    const shape = metadata(relativePath);
    return admissionRecord({
      kind: "graph-file",
      group: validSyntaxGroup,
      path: relativePath,
      source_sha256: sha256(source(relativePath)),
      includes: shape.includes,
      flags: shape.flags,
      features: shape.features,
      negative_phase: shape.negativePhase,
      negative_type: shape.negativeType,
    });
  }),
  ...closedSyntaxRoots.flatMap((relativePath) =>
    closedEdges.get(relativePath).map((request, requestIndex) =>
      admissionRecord({
        kind: "graph-request",
        group: validSyntaxGroup,
        path: relativePath,
        request_index: requestIndex,
        specifier: request.specifier,
        normalized_path: request.normalized,
      }),
    ),
  ),
  ...closedSyntaxRoots.map((rootPath) =>
    admissionRecord({
      kind: "dynamic-import-root",
      group: validSyntaxGroup,
      path: rootPath,
      closure_file_count: closedEdges.get(rootPath).length === 0 ? 1 : 2,
      priority: 0,
      policy: "initial-import-tree",
    }),
  ),
];
assert.equal(closedSyntaxAdmissionRecords.length, 303);

const diagnosticRule = "dynamic-import.invalid-new-target";
const diagnosticRuleLine =
  `${diagnosticRule}\tjs_parse_postfix_expr\tnew cannot directly target ImportCall`;
const newTargetDiagnosticCandidates = newTargetRoots.flatMap((relativePath) =>
  variants(relativePath).map((variant) => `${relativePath}\t${variant}\t${diagnosticRule}`),
);
newTargetDiagnosticCandidates.sort(bytewise);
assert.equal(newTargetDiagnosticCandidates.length, 40);

const parseNegativeSingletons = [
  {
    path: escapedKeywordPath,
    rule: "dynamic-import.escaped-keyword",
    message: "'import' is a reserved identifier",
  },
  {
    path: secondArgumentYieldPath,
    rule: "dynamic-import.second-argument-yield-context",
    message: "unexpected 'yield' keyword",
  },
];
const parseNegativeContractByPath = new Map([
  ...parseNegativeFamilies.flatMap((family) =>
    family.paths.map((relativePath) => [
      relativePath,
      { rule: family.rule, message: family.message },
    ]),
  ),
  ...parseNegativeSingletons.map(({ path, rule, message }) => [path, { rule, message }]),
]);
assert.equal(parseNegativeContractByPath.size, 86);
assert.deepEqual(
  [...parseNegativeContractByPath.keys()].sort(bytewise),
  parseNegativeRoots,
  "dynamic-import parse-negative diagnostic ownership changed",
);

const parseNegativeDiagnosticRuleLines = [
  "dynamic-import.missing-specifier\tjs_parse_postfix_expr\tImportCall requires a module specifier assignment expression",
  "dynamic-import.spread-argument\tjs_parse_postfix_expr\tImportCall does not accept spread arguments",
  "dynamic-import.excess-argument\tjs_parse_postfix_expr\tImportCall accepts at most a specifier and options argument",
  "dynamic-import.typeof-keyword\tjs_parse_postfix_expr\tthe bare import keyword cannot be a unary typeof operand",
  "dynamic-import.escaped-keyword\tjs_parse_error_reserved_identifier\tthe import terminal cannot contain Unicode escapes",
  "dynamic-import.second-argument-yield-context\tjs_parse_assign_expr2\tImportCall options expressions preserve the surrounding Yield grammar parameter",
];
assert.equal(new Set(parseNegativeDiagnosticRuleLines).size, 6);
const parseNegativeDiagnosticCandidates = parseNegativeRoots.flatMap((relativePath) => {
  const contract = parseNegativeContractByPath.get(relativePath);
  assert(contract, `${relativePath}: missing diagnostic ownership`);
  return variants(relativePath).map(
    (variant) => `${relativePath}\t${variant}\t${contract.rule}`,
  );
});
parseNegativeDiagnosticCandidates.sort(bytewise);
assert.equal(parseNegativeDiagnosticCandidates.length, 163);

const diagnosticCandidates = [
  ...newTargetDiagnosticCandidates,
  ...parseNegativeDiagnosticCandidates,
];
diagnosticCandidates.sort(bytewise);
assert.equal(diagnosticCandidates.length, 203);

function checkedLineSet(path) {
  return new Set(readFileSync(path, "utf8").split("\n"));
}

function profileSection(name) {
  const lines = readFileSync(checkedProfile, "utf8").trimEnd().split("\n");
  const start = lines.indexOf(`[${name}]`);
  assert.notEqual(start, -1, `missing profile section [${name}]`);
  const next = lines.findIndex((line, index) => index > start && /^\[[^\]]+\]$/u.test(line));
  return new Set(lines.slice(start + 1, next === -1 ? lines.length : next).filter(Boolean));
}

function assertPromoted() {
  const profile = profileSection("audited-negative-tests");
  const focused = checkedLineSet(checkedFocused);
  const canaryPaths = new Set(futureSyntaxCanaries);
  for (const relativePath of moduleGoalRoots) {
    assert(focused.has(relativePath), `${relativePath}: Module-goal focused path not promoted`);
  }
  for (const relativePath of assignmentTargetRoots) {
    assert(profile.has(relativePath), `${relativePath}: assignment profile path not promoted`);
  }
  for (const relativePath of newTargetRoots) {
    assert(profile.has(relativePath), `${relativePath}: profile path not promoted`);
    assert(focused.has(relativePath), `${relativePath}: focused path not promoted`);
  }
  for (const relativePath of parseNegativeRoots) {
    assert(profile.has(relativePath), `${relativePath}: parse-negative profile path not promoted`);
    assert(focused.has(relativePath), `${relativePath}: parse-negative focused path not promoted`);
  }
  for (const relativePath of futureSyntaxCanaries) {
    assert(!profile.has(relativePath), `${relativePath}: canary escaped into the profile`);
    assert(!focused.has(relativePath), `${relativePath}: canary escaped into the focused manifest`);
  }
  for (const relativePath of validSyntaxCanaries) {
    assert(
      !focused.has(relativePath),
      `${relativePath}: future-syntax canary escaped into the focused manifest`,
    );
  }
  const admittedPaths = readFileSync(checkedAdmissions, "utf8")
    .trimEnd()
    .split("\n")
    .slice(1)
    .map((line) => line.split("\t")[2]);
  assert(
    admittedPaths.every((relativePath) => !canaryPaths.has(relativePath)),
    "future dynamic-import syntax canary escaped into admissions",
  );
  assert(
    admittedPaths.every((relativePath) => !validSyntaxCanaries.includes(relativePath)),
    "dynamic-import syntax/valid future-syntax canary escaped into admissions",
  );
  // The closed positive cohort is full-corpus admissions data, not part of the
  // frozen focused manifest (which refuses a stale source); neither its roots
  // nor its empty fixture may silently appear there.
  for (const relativePath of closedSources) {
    assert(
      !focused.has(relativePath),
      `${relativePath}: closed-root source escaped into the focused manifest`,
    );
  }
  const rules = checkedLineSet(checkedRules);
  assert(rules.has(diagnosticRuleLine), `${diagnosticRule}: rule not promoted`);
  for (const ruleLine of parseNegativeDiagnosticRuleLines) {
    assert(rules.has(ruleLine), `${ruleLine.split("\t")[0]}: rule not promoted exactly`);
  }

  const contractLines = readFileSync(checkedDiagnostics, "utf8")
    .trimEnd()
    .split("\n")
    .slice(1);
  const assignmentPaths = new Set(assignmentTargetRoots);
  const assignmentContracts = contractLines
    .map((line) => line.split("\t"))
    .filter((fields) => assignmentPaths.has(fields[0]));
  assert.equal(
    assignmentContracts.length,
    34,
    "assignment-target diagnostic contract count drifted",
  );
  const expectedAssignmentKeys = assignmentTargetRoots
    .flatMap((relativePath) =>
      variants(relativePath).map((variant) => `${relativePath}\t${variant}`),
    )
    .sort(bytewise);
  const actualAssignmentKeys = assignmentContracts
    .map((fields) => `${fields[0]}\t${fields[1]}`)
    .sort(bytewise);
  assert.deepEqual(
    actualAssignmentKeys,
    expectedAssignmentKeys,
    "assignment-target diagnostic variants drifted",
  );
  for (const fields of assignmentContracts) {
    assert.equal(fields.length, 10, `${fields[0]} ${fields[1]}: assignment diagnostic schema drifted`);
    assert.equal(
      fields[2],
      sha256(source(fields[0])),
      `${fields[0]}: assignment diagnostic source hash drifted`,
    );
    assert.equal(fields[3], "parse", `${fields[0]}: assignment diagnostic phase drifted`);
    assert.equal(fields[4], "SyntaxError", `${fields[0]}: assignment diagnostic type drifted`);
    assert(fields[5], `${fields[0]}: assignment diagnostic rule is empty`);
    assert(fields[6], `${fields[0]}: assignment diagnostic message is empty`);
    assert.match(fields[7], /^[1-9][0-9]*$/u, `${fields[0]}: assignment diagnostic line drifted`);
    assert.match(fields[8], /^[1-9][0-9]*$/u, `${fields[0]}: assignment diagnostic column drifted`);
    assert.equal(fields[9], "exact", `${fields[0]}: assignment location policy drifted`);
  }
  const ownedPaths = new Set(newTargetRoots);
  const ownedContracts = contractLines
    .map((line) => line.split("\t"))
    .filter((fields) => ownedPaths.has(fields[0]));
  assert(
    contractLines.every((line) => !canaryPaths.has(line.split("\t")[0])),
    "new-target surface canary escaped into diagnostics",
  );
  assert.equal(ownedContracts.length, 40, "new-target diagnostic contract count drifted");
  for (const fields of ownedContracts) {
    assert.equal(fields.length, 10, `${fields[0]} ${fields[1]}: diagnostic schema drifted`);
    assert.equal(fields[2], sha256(source(fields[0])), `${fields[0]}: diagnostic source hash drifted`);
    assert.equal(fields[3], "parse", `${fields[0]}: diagnostic phase drifted`);
    assert.equal(fields[4], "SyntaxError", `${fields[0]}: diagnostic type drifted`);
    assert.equal(fields[5], diagnosticRule, `${fields[0]}: diagnostic rule drifted`);
    assert.equal(fields[6], "invalid use of 'import()'", `${fields[0]}: diagnostic message drifted`);
    assert.match(fields[7], /^[1-9][0-9]*$/u, `${fields[0]}: diagnostic line drifted`);
    assert.match(fields[8], /^[1-9][0-9]*$/u, `${fields[0]}: diagnostic column drifted`);
    assert.equal(fields[9], "exact", `${fields[0]}: diagnostic location policy drifted`);
  }
  const actualCandidates = ownedContracts
    .map((fields) => `${fields[0]}\t${fields[1]}\t${fields[5]}`)
    .sort(bytewise);
  assert.deepEqual(
    actualCandidates,
    newTargetDiagnosticCandidates,
    "new-target diagnostic contracts drifted",
  );

  const parseNegativePaths = new Set(parseNegativeRoots);
  const parseNegativeContracts = contractLines
    .map((line) => line.split("\t"))
    .filter((fields) => parseNegativePaths.has(fields[0]));
  assert.equal(
    parseNegativeContracts.length,
    163,
    "dynamic-import parse-negative diagnostic contract count drifted",
  );
  const expectedParseNegativeKeys = parseNegativeRoots
    .flatMap((relativePath) =>
      variants(relativePath).map((variant) => `${relativePath}\t${variant}`),
    )
    .sort(bytewise);
  const actualParseNegativeKeys = parseNegativeContracts
    .map((fields) => `${fields[0]}\t${fields[1]}`)
    .sort(bytewise);
  assert.deepEqual(
    actualParseNegativeKeys,
    expectedParseNegativeKeys,
    "dynamic-import parse-negative diagnostic variants drifted",
  );
  for (const fields of parseNegativeContracts) {
    const contract = parseNegativeContractByPath.get(fields[0]);
    assert(contract, `${fields[0]}: diagnostic ownership disappeared`);
    assert.equal(fields.length, 10, `${fields[0]} ${fields[1]}: diagnostic schema drifted`);
    assert.equal(fields[2], sha256(source(fields[0])), `${fields[0]}: source hash drifted`);
    assert.equal(fields[3], "parse", `${fields[0]}: diagnostic phase drifted`);
    assert.equal(fields[4], "SyntaxError", `${fields[0]}: diagnostic type drifted`);
    assert.equal(fields[5], contract.rule, `${fields[0]}: diagnostic rule drifted`);
    assert.equal(fields[6], contract.message, `${fields[0]}: diagnostic message drifted`);
    assert.match(fields[7], /^[1-9][0-9]*$/u, `${fields[0]}: diagnostic line drifted`);
    assert.match(fields[8], /^[1-9][0-9]*$/u, `${fields[0]}: diagnostic column drifted`);
    assert.equal(fields[9], "exact", `${fields[0]}: location policy drifted`);
  }
  const actualParseNegativeCandidates = parseNegativeContracts
    .map((fields) => `${fields[0]}\t${fields[1]}\t${fields[5]}`)
    .sort(bytewise);
  assert.deepEqual(
    actualParseNegativeCandidates,
    parseNegativeDiagnosticCandidates,
    "dynamic-import parse-negative diagnostic contracts drifted",
  );

  const exemptions = readFileSync(checkedExemptions, "utf8")
    .trimEnd()
    .split("\n")
    .slice(1)
    .map((line) => line.split("\t")[0]);
  assert(
    exemptions.every((relativePath) => !ownedPaths.has(relativePath)),
    "new-target diagnostics must not use exemptions",
  );
  assert(
    exemptions.every((relativePath) => !assignmentPaths.has(relativePath)),
    "assignment-target diagnostics must not use exemptions",
  );
  assert(
    exemptions.every((relativePath) => !parseNegativePaths.has(relativePath)),
    "dynamic-import parse-negative diagnostics must not use exemptions",
  );
  assert(
    exemptions.every((relativePath) => !canaryPaths.has(relativePath)),
    "future dynamic-import syntax canary escaped into exemptions",
  );
}

if (mode === "--admissions") {
  process.stdout.write(
    renderAdmissionRows([
      ...admissionRecords,
      ...moduleAdmissionRecords,
      ...assignmentTargetAdmissionRecords,
      ...newTargetAdmissionRecords,
      ...parseNegativeAdmissionRecords,
      ...closedSyntaxAdmissionRecords,
      ...usageAdmissionRecords,
    ]),
  );
} else if (mode === "--diagnostic-candidates") {
  process.stdout.write(`path\tvariant\trule\n${diagnosticCandidates.join("\n")}\n`);
} else {
  assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
  assertAdmissionGroup(checkedAdmissions, moduleAdmissionGroup, moduleAdmissionRecords);
  assertAdmissionGroup(
    checkedAdmissions,
    assignmentTargetGroup,
    assignmentTargetAdmissionRecords,
  );
  assertAdmissionGroup(checkedAdmissions, newTargetGroup, newTargetAdmissionRecords);
  assertAdmissionGroup(
    checkedAdmissions,
    parseNegativeGroup,
    parseNegativeAdmissionRecords,
  );
  assertAdmissionGroup(
    checkedAdmissions,
    validSyntaxGroup,
    closedSyntaxAdmissionRecords,
  );
  assertAdmissionGroup(
    checkedAdmissions,
    usageAdmissionGroup,
    usageAdmissionRecords,
  );
  assertPromoted();
  console.log(
    "dynamic-import admissions authenticated: runtime roots=4/variants=8; " +
      "module roots=27/files=31/edges=32; " +
      "assignment negatives=17/34; new-target negatives=21/40; " +
      "parse negatives=86/163; " +
      "syntax/valid closed roots=108/variants=206/sources=109/edges=86; " +
      "usage roots=108/variants=216/edges=108; " +
      "sources=351; edges=197",
  );
}
