#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const checkedSuite = join(root, "target/oracle/quickjs-2026-06-04/test262");
const args = process.argv.slice(2);
const suiteIndex = args.indexOf("--suite");
const outputIndex = args.indexOf("--output");
const suite = resolve(suiteIndex === -1 ? checkedSuite : args[suiteIndex + 1]);
const mode = args.includes("--rust")
  ? "rust"
  : outputIndex === -1
    ? "check"
    : "output";
const output = outputIndex === -1 ? null : resolve(args[outputIndex + 1]);

assert(existsSync(join(suite, "test")), `missing Test262 suite: ${suite}`);

const cohort = "test/language/module-code";
const expected = {
  roots: 86,
  exportRoots: 43,
  importRoots: 43,
  generatorRoots: 12,
  rustSha256: "9ebc2b3d9f8d237efa6245fd9aa3a26d33f274155ad84d47d4c0e9ab2f968896",
  evidenceSha256: {
    "tests/test262-module-decl-position-a.txt":
      "5e70969f0a3f4ed428f69e868fdd69fe2b6821d42733e97cc13c1e24837ef182",
    "tests/test262-module-decl-position-a-ledger.tsv":
      "640f63f2a82ec0055315fc40062f5801fde707a4076e56dcf18aaf10a1fec908",
    "tests/test262-module-decl-position-a-variants.tsv":
      "b3c079495d0161773a15cd5f039d840e7959fe6c039b416a74203c084f7186db",
    "tests/test262-module-decl-position-a-negatives.txt":
      "5e70969f0a3f4ed428f69e868fdd69fe2b6821d42733e97cc13c1e24837ef182",
    "tests/test262-module-decl-position-a-exclusions.tsv":
      "593c026b26d72c7ee5511fdd7ab526c25f8a08719e43ecdac9e7ce6bcb6c7b36",
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

const roots = readdirSync(join(suite, cohort), { withFileTypes: true })
  .filter(
    (entry) =>
      entry.isFile() && /^parse-err-decl-pos-(export|import)-.*\.js$/.test(entry.name),
  )
  .map((entry) => `${cohort}/${entry.name}`)
  .sort();
const exportRoots = roots.filter((relativePath) => relativePath.includes("-export-"));
const importRoots = roots.filter((relativePath) => relativePath.includes("-import-"));
const generatorRoots = roots.filter((relativePath) =>
  metadata(relativePath).features.includes("generators"),
);
const exclusionCanaries = [
  ["adjacent-parse-negative", `${cohort}/parse-err-export-dflt-const.js`],
  ["import-attributes", `${cohort}/import-attributes/import-attribute-empty.js`],
  ["top-level-await", `${cohort}/top-level-await/await-expr-resolution.js`],
];

const lines = (...values) => `${values.join("\n")}\n`;
const manifest = lines(...roots);
const ledger = lines(
  "path\tdeclaration_keyword\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256\tfrontmatter_sha256",
  ...roots.map((relativePath) => {
    const shape = metadata(relativePath);
    return [
      relativePath,
      relativePath.includes("-export-") ? "export" : "import",
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
const variants = lines(
  "path\tvariant\tflags\tfeatures\tnegative_phase\tnegative_type\tsource_sha256",
  ...roots.map((relativePath) => {
    const shape = metadata(relativePath);
    return [
      relativePath,
      "sloppy",
      shape.flags.join(","),
      shape.features.join(","),
      shape.negativePhase,
      shape.negativeType,
      sha256(source(relativePath)),
    ].join("\t");
  }),
);
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

const evidence = new Map([
  ["tests/test262-module-decl-position-a.txt", manifest],
  ["tests/test262-module-decl-position-a-ledger.tsv", ledger],
  ["tests/test262-module-decl-position-a-variants.tsv", variants],
  ["tests/test262-module-decl-position-a-negatives.txt", manifest],
  ["tests/test262-module-decl-position-a-exclusions.tsv", exclusions],
]);

assert.equal(roots.length, expected.roots);
assert.equal(exportRoots.length, expected.exportRoots);
assert.equal(importRoots.length, expected.importRoots);
assert.equal(generatorRoots.length, expected.generatorRoots);
assert(
  roots.every((relativePath) => {
    const shape = metadata(relativePath);
    return (
      shape.includes.length === 0 &&
      shape.flags.join(",") === "module" &&
      (shape.features.length === 0 || shape.features.join(",") === "generators") &&
      shape.negativePhase === "parse" &&
      shape.negativeType === "SyntaxError"
    );
  }),
  "declaration-position metadata shape drifted",
);
for (const [, relativePath] of exclusionCanaries) {
  assert(existsSync(join(suite, relativePath)), `missing exclusion canary: ${relativePath}`);
  assert(!roots.includes(relativePath), `excluded surface entered cohort: ${relativePath}`);
}
for (const [relativePath, contents] of evidence) {
  assert.equal(sha256(contents), expected.evidenceSha256[relativePath], `${relativePath} changed`);
}

function rustAdmissions() {
  const outputLines = [];
  outputLines.push(
    `const DECL_POSITION_MODULE_ADMISSIONS: [DependencyFreeModuleAdmission; ${roots.length}] = [`,
  );
  for (const relativePath of roots) {
    outputLines.push("    DependencyFreeModuleAdmission {");
    outputLines.push(`        path: ${JSON.stringify(relativePath)},`);
    outputLines.push(`        source_sha256: ${JSON.stringify(sha256(source(relativePath)))},`);
    outputLines.push(
      `        metadata: ${metadata(relativePath).features.includes("generators") ? "MODULE_GENERATORS_PARSE_SYNTAX_ERROR_METADATA" : "MODULE_PARSE_SYNTAX_ERROR_METADATA"},`,
    );
    outputLines.push("    },");
  }
  outputLines.push("];", "");
  return outputLines.join("\n");
}

if (mode === "rust") {
  const contents = rustAdmissions();
  assert.equal(sha256(contents), expected.rustSha256, "Rust admissions changed");
  process.stdout.write(contents);
} else if (mode === "output") {
  assert(output, "--output requires a directory");
  for (const [relativePath, contents] of evidence) {
    writeFileSync(join(output, relativePath.split("/").at(-1)), contents);
  }
  console.log(`generated ${evidence.size} authenticated evidence files in ${output}`);
} else {
  for (const [relativePath, contents] of evidence) {
    if (relativePath === "tests/test262-module-decl-position-a-ledger.tsv") {
      assert.equal(
        readFileSync(join(root, relativePath), "utf8"),
        contents,
        `${relativePath} drifted`,
      );
    }
  }
  console.log(
    `module-decl-position-a: roots=${roots.length} export=${exportRoots.length} import=${importRoots.length} generators=${generatorRoots.length} variants=${roots.length} canaries=${exclusionCanaries.length}`,
  );
}
