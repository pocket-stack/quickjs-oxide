#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const generator = join(root, "scripts/generate-test262-module-json-a.mjs");
const oracleArguments = process.argv.slice(2);
const protectedFiles = [
  "dev-support/test262/current.conf",
  "dev-support/test262/admissions.tsv",
  "dev-support/test262/negative-diagnostics.tsv",
  "dev-support/test262/negative-diagnostic-rules.tsv",
  "compat/test262-oxide.conf",
  "tests/test262-class-private-callables-b.txt",
].map((relativePath) => join(root, relativePath));

function run(arguments_, expectedStatus = 0) {
  const result = spawnSync(process.execPath, [generator, ...oracleArguments, ...arguments_], {
    cwd: root,
    encoding: "utf8",
  });
  assert.equal(result.signal, null);
  assert.equal(result.status, expectedStatus, result.stderr);
  return result;
}

const before = new Map(protectedFiles.map((path) => [path, readFileSync(path)]));
const output = mkdtempSync(join(tmpdir(), "quickjs-oxide-module-json-a-"));
try {
  const generated = run(["--output", output]);
  assert.match(generated.stdout, /generated 13 candidate files/u);

  const roots = readFileSync(join(output, "test262-module-json-a.txt"), "utf8")
    .trimEnd()
    .split("\n");
  assert.equal(roots.length, 11);
  assert(roots.every((path) => path.includes("/json-") && path.endsWith(".js")));
  assert(!roots.some((path) => path.endsWith("/json-idempotency.js")));

  const sources = readFileSync(join(output, "test262-module-json-a-sources.txt"), "utf8")
    .trimEnd()
    .split("\n");
  assert.equal(sources.length, 20);
  assert.equal(sources.filter((path) => path.endsWith(".json")).length, 9);

  const admissions = run(["--admissions"]).stdout;
  const admissionCandidate = readFileSync(
    join(output, "test262-module-json-a-admission-rows.tsv"),
    "utf8",
  );
  assert.equal(admissionCandidate.split("\n").slice(1).join("\n"), admissions);
  assert.equal(admissions.trimEnd().split("\n").length, 42);
  for (const line of admissions.split("\n").filter(Boolean)) {
    const fields = line.split("\t");
    if (!fields[2].endsWith(".json")) continue;
    assert.equal(fields[0], "graph-file");
    assert(fields.slice(4, 9).every((field) => field === "-"));
  }

  const diagnosticCandidates = run(["--diagnostic-candidates"]).stdout;
  assert.equal(
    readFileSync(
      join(output, "test262-module-json-a-negative-diagnostic-candidates.tsv"),
      "utf8",
    ),
    diagnosticCandidates,
  );
  assert.equal(diagnosticCandidates.trimEnd().split("\n").length, 3);

  const diagnostics = run(["--negative-diagnostics"]).stdout;
  assert.equal(
    readFileSync(join(output, "test262-module-json-a-negative-diagnostics.tsv"), "utf8"),
    diagnostics,
  );
  assert.equal(diagnostics.trimEnd().split("\n").length, 3);
  assert.equal(
    diagnostics.split("\n").filter((line) => line.endsWith("\texact")).length,
    1,
  );
  assert.equal(
    diagnostics.split("\n").filter((line) => line.endsWith("\tabsent")).length,
    1,
  );
  assert.match(diagnostics, /json-invalid\.js.*\texpecting property name\t2\t3\texact/u);
  assert.match(diagnostics, /json-named-bindings\.js.*\tmodule\.missing-export\t/u);

  const exclusions = readFileSync(
    join(output, "test262-module-json-a-exclusions.tsv"),
    "utf8",
  );
  assert.match(exclusions, /dynamic-json-module.*json-idempotency\.js/u);
  assert.match(exclusions, /dynamic-import-json-attributes/u);
  assert.match(exclusions, /text-module.*text-javascript\.js/u);

  const quickjs = readFileSync(join(output, "test262-module-json-a-quickjs.tsv"), "utf8");
  assert.equal(quickjs.trimEnd().split("\n").length, 12);
  assert.equal(quickjs.split("\n").filter((line) => line.includes("\tpass\t")).length, 9);
  assert.equal(
    quickjs.split("\n").filter((line) => line.includes("\texpected-error\t")).length,
    2,
  );
  assert.match(quickjs, /json-invalid\.js.*\texpecting property name\t2\t3\texact/u);

  const unknown = run(["--not-an-option"], 1);
  assert.match(unknown.stderr, /unknown option/u);

  for (const [path, contents] of before) {
    assert.deepEqual(readFileSync(path), contents, `${path} was modified by candidate generation`);
  }
} finally {
  rmSync(output, { recursive: true, force: true });
}

console.log("module-json-a candidate generator tests passed");
