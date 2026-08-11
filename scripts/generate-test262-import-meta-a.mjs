#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, posix, relative, resolve } from "node:path";

import {
  admissionRecord,
  assertAdmissionGroup,
  renderAdmissionRows,
} from "./test262-admission-data.mjs";

const root = resolve(import.meta.dirname, "..");
const checkedSuite = join(root, "target/oracle/quickjs-2026-06-04/test262");
const checkedAdmissions = join(root, "dev-support/test262/admissions.tsv");
const args = process.argv.slice(2);
const suiteIndex = args.indexOf("--suite");
const outputIndex = args.indexOf("--output");
const suite = resolve(suiteIndex === -1 ? checkedSuite : args[suiteIndex + 1]);
const mode = args.includes("--admissions")
  ? "admissions"
  : outputIndex === -1
    ? "check"
    : "output";
const output = outputIndex === -1 ? null : resolve(args[outputIndex + 1]);

assert(existsSync(join(suite, "test")), `missing Test262 suite: ${suite}`);

const cohort = "test/language/expressions/import.meta";
const expected = {
  roots: 22,
  sources: 23,
  fixtures: 1,
  moduleRoots: 17,
  scriptRoots: 5,
  moduleSources: 18,
  rootedEdges: 1,
  variants: 27,
  negatives: 12,
  evidenceSha256: {
    "tests/test262-import-meta-a.txt":
      "e74868cc1620f70e5e1cb4528bd2af6915cf1ada5895406869004de7d857def6",
    "tests/test262-import-meta-a-sources.txt":
      "6adb4674b5fd39fa55c3727e937539362426b5e94598bc17b6e49deca0f1e0b5",
    "tests/test262-import-meta-a-module-roots.txt":
      "13e6fe16e2861bedab398511180006c1b3660e1265b5c350809579c83267e8d3",
    "tests/test262-import-meta-a-script-roots.txt":
      "7a7f50e4def2ed2bbc83b944a4f28287b8cad1f5aa1eb960bc0a7ab6428047b3",
    "tests/test262-import-meta-a-edges.tsv":
      "24adcf9f82ac269972dde05bc445534bcbb658fc6fab364e7caf1ad42f971a37",
    "tests/test262-import-meta-a-closures.tsv":
      "7ed199bbfd603004f11cc0130651cc11c48b0697bccd688bf3a1db2378e21310",
    "tests/test262-import-meta-a-ledger.tsv":
      "ad2282a942e54a6da43dfc092f99a2bf89cc42519d9d1f89f4a78e632e9edfba",
    "tests/test262-import-meta-a-variants.tsv":
      "403284c9137dde8db3f7d0ea149e383fc169735f41123182f57636834cf8a336",
    "tests/test262-import-meta-a-negatives.txt":
      "eebc250f1fac5b3ff153bf76ff32c5a31d88c74bb3ea8249597513c61fd771fc",
    "tests/test262-import-meta-a-exclusions.tsv":
      "c4b24a9d180fa8cf4ebc5f10b43bf6e47adb07b89bcd979954cc545a033241bf",
  },
};

const sha256 = (value) => createHash("sha256").update(value).digest("hex");
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
    flags: arrayField(text, "flags"),
    features: arrayField(text, "features"),
    negativePhase: negative?.[1] ?? "",
    negativeType: negative?.[2] ?? "",
  };
}

function walk(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const absolute = join(directory, entry.name);
    return entry.isDirectory() ? walk(absolute) : [absolute];
  });
}

function requestSpecifiers(relativePath) {
  const body = source(relativePath).replace(/\/\*---[\s\S]*?---\*\//, "");
  const requests = [];
  for (const line of body.split(/\r?\n/)) {
    const match =
      line.match(/\sfrom\s*['"]([^'"]+)['"]/) ??
      line.match(/^\s*import\s*['"]([^'"]+)['"]/);
    if (match && !requests.includes(match[1])) requests.push(match[1]);
  }
  for (const request of requests) {
    assert(request.startsWith("./"), `${relativePath}: non-child request ${request}`);
    assert(!request.includes("/../"), `${relativePath}: escaping request ${request}`);
  }
  return requests;
}

const normalize = (base, request) => posix.join(posix.dirname(base), request);
const roots = walk(join(suite, cohort))
  .filter((absolute) => absolute.endsWith(".js") && !absolute.endsWith("_FIXTURE.js"))
  .map((absolute) => relative(suite, absolute).split(posix.sep).join(posix.sep))
  .sort();
const moduleRoots = roots.filter((relativePath) => metadata(relativePath).flags.includes("module"));
const scriptRoots = roots.filter((relativePath) => !metadata(relativePath).flags.includes("module"));

function closure(rootPath) {
  const reached = new Set();
  const pending = [rootPath];
  while (pending.length > 0) {
    const base = pending.pop();
    if (reached.has(base)) continue;
    reached.add(base);
    for (const request of requestSpecifiers(base)) pending.push(normalize(base, request));
  }
  return [...reached].sort();
}

const sources = [...new Set(roots.flatMap(closure))].sort();
const moduleSources = [...new Set(moduleRoots.flatMap(closure))].sort();
const rootedEdges = roots.flatMap((rootPath) =>
  closure(rootPath).flatMap((base) =>
    requestSpecifiers(base).map((specifier, requestIndex) => ({
      rootPath,
      base,
      requestIndex,
      specifier,
      normalized: normalize(base, specifier),
    })),
  ),
);
const fileEdges = new Map(
  sources.map((base) => [
    base,
    requestSpecifiers(base).map((specifier) => ({
      specifier,
      normalized: normalize(base, specifier),
    })),
  ]),
);

function variants(relativePath) {
  const flags = metadata(relativePath).flags;
  if (flags.includes("module") || flags.includes("noStrict") || flags.includes("raw")) {
    return ["sloppy"];
  }
  if (flags.includes("onlyStrict")) return ["strict"];
  return ["sloppy", "strict"];
}

const variantRecords = roots.flatMap((relativePath) =>
  variants(relativePath).map((variant) => ({ relativePath, variant })),
);
const negativeRoots = roots.filter((relativePath) => metadata(relativePath).negativePhase);
const exclusionCanaries = [
  [
    "dynamic-import",
    "test/language/expressions/dynamic-import/assignment-expression/import-meta.js",
  ],
  [
    "assignmenttargettype-direct",
    "test/language/expressions/assignmenttargettype/direct-import.meta.js",
  ],
  [
    "assignmenttargettype-parenthesized",
    "test/language/expressions/assignmenttargettype/parenthesized-import.meta.js",
  ],
];

const lines = (...values) => `${values.join("\n")}\n`;
const manifest = lines(...roots);
const sourceManifest = lines(...sources);
const moduleRootManifest = lines(...moduleRoots);
const scriptRootManifest = lines(...scriptRoots);
const edges = lines(
  "root_path\tbase_path\trequest_index\tspecifier\tnormalized_path",
  ...rootedEdges.map(({ rootPath, base, requestIndex, specifier, normalized }) =>
    [rootPath, base, requestIndex, specifier, normalized].join("\t"),
  ),
);
const closures = lines(
  "root_path\texecution_goal\tclosure_files\trequest_edges",
  ...roots.map((rootPath) => {
    const files = closure(rootPath);
    return [
      rootPath,
      moduleRoots.includes(rootPath) ? "module" : "script",
      files.length,
      files.reduce((count, relativePath) => count + fileEdges.get(relativePath).length, 0),
    ].join("\t");
  }),
);
const ledger = lines(
  "path\trole\texecution_goal\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256\tfrontmatter_sha256",
  ...sources.map((relativePath) => {
    const shape = metadata(relativePath);
    const isRoot = roots.includes(relativePath);
    return [
      relativePath,
      isRoot ? "root" : "fixture",
      isRoot ? (shape.flags.includes("module") ? "module" : "script") : "module-fixture",
      shape.includes.join(","),
      shape.flags.join(","),
      shape.features.join(","),
      shape.negativePhase,
      shape.negativeType,
      sha256(source(relativePath)),
      sha256(frontmatter(source(relativePath))),
    ].join("\t");
  }),
);
const variantsLedger = lines(
  "path\tvariant\texecution_goal\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256",
  ...variantRecords.map(({ relativePath, variant }) => {
    const shape = metadata(relativePath);
    return [
      relativePath,
      variant,
      shape.flags.includes("module") ? "module" : "script",
      shape.flags.join(","),
      shape.features.join(","),
      shape.negativePhase,
      shape.negativeType,
      sha256(source(relativePath)),
    ].join("\t");
  }),
);
const negatives = lines(...negativeRoots);
const exclusions = lines(
  "surface\tcanary_path\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256\tfrontmatter_sha256",
  ...exclusionCanaries.map(([surface, relativePath]) => {
    const shape = metadata(relativePath);
    return [
      surface,
      relativePath,
      shape.includes.join(","),
      shape.flags.join(","),
      shape.features.join(","),
      shape.negativePhase,
      shape.negativeType,
      sha256(source(relativePath)),
      sha256(frontmatter(source(relativePath))),
    ].join("\t");
  }),
);

assert.equal(roots.length, expected.roots);
assert.equal(sources.length, expected.sources);
assert.equal(sources.length - roots.length, expected.fixtures);
assert.equal(moduleRoots.length, expected.moduleRoots);
assert.equal(scriptRoots.length, expected.scriptRoots);
assert.equal(moduleSources.length, expected.moduleSources);
assert.equal(rootedEdges.length, expected.rootedEdges);
assert.equal(variantRecords.length, expected.variants);
assert.equal(negativeRoots.length, expected.negatives);
assert(moduleRoots.every((relativePath) => metadata(relativePath).features.includes("import.meta")));
assert(scriptRoots.every((relativePath) => metadata(relativePath).features.includes("import.meta")));
assert.deepEqual(
  rootedEdges.map(({ specifier, normalized }) => [specifier, normalized]),
  [["./distinct-for-each-module_FIXTURE.js", `${cohort}/distinct-for-each-module_FIXTURE.js`]],
);
for (const [surface, relativePath] of exclusionCanaries) {
  assert(existsSync(join(suite, relativePath)), `missing exclusion canary: ${relativePath}`);
  assert(!roots.includes(relativePath), `excluded surface entered cohort: ${relativePath}`);
  const shape = metadata(relativePath);
  if (surface === "dynamic-import") {
    assert.deepEqual(shape.features, ["dynamic-import", "import.meta"]);
    assert.deepEqual(shape.flags, ["module", "async"]);
  } else {
    assert.deepEqual(shape.flags, ["generated"]);
    assert.equal(shape.negativePhase, "parse");
    assert.equal(shape.negativeType, "SyntaxError");
  }
}

const evidence = new Map([
  ["tests/test262-import-meta-a.txt", manifest],
  ["tests/test262-import-meta-a-sources.txt", sourceManifest],
  ["tests/test262-import-meta-a-module-roots.txt", moduleRootManifest],
  ["tests/test262-import-meta-a-script-roots.txt", scriptRootManifest],
  ["tests/test262-import-meta-a-edges.tsv", edges],
  ["tests/test262-import-meta-a-closures.tsv", closures],
  ["tests/test262-import-meta-a-ledger.tsv", ledger],
  ["tests/test262-import-meta-a-variants.tsv", variantsLedger],
  ["tests/test262-import-meta-a-negatives.txt", negatives],
  ["tests/test262-import-meta-a-exclusions.tsv", exclusions],
]);
for (const [relativePath, contents] of evidence) {
  assert.equal(sha256(contents), expected.evidenceSha256[relativePath], `${relativePath} changed`);
}

const admissionGroup = "import-meta-a";
const admissionRecords = [
  ...moduleSources.map((relativePath) => {
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
  ...moduleSources.flatMap((relativePath) =>
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
  ...moduleRoots.map((rootPath) =>
    admissionRecord({
      kind: "graph-root",
      group: admissionGroup,
      path: rootPath,
      closure_file_count: closure(rootPath).length,
      priority: 1,
    }),
  ),
];

if (mode === "admissions") {
  process.stdout.write(renderAdmissionRows(admissionRecords));
} else if (mode === "output") {
  assert(output, "--output requires a directory");
  for (const [relativePath, contents] of evidence) {
    const destination = join(output, relativePath.split("/").at(-1));
    writeFileSync(destination, contents);
  }
  console.log(`generated ${evidence.size} authenticated evidence files in ${output}`);
} else {
  assertAdmissionGroup(checkedAdmissions, admissionGroup, admissionRecords);
  console.log(
    `import-meta-a generated evidence authenticated: roots=${roots.length} sources=${sources.length} module_roots=${moduleRoots.length} script_roots=${scriptRoots.length} rooted_edges=${rootedEdges.length} variants=${variantRecords.length} canaries=${exclusionCanaries.length}`,
  );
}
