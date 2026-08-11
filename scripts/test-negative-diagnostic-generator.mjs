#!/usr/bin/env node

import assert from "node:assert/strict";
import {
  access,
  chmod,
  copyFile,
  link,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { constants as fsConstants } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";

const root = path.resolve(import.meta.dirname, "..");
const audit = path.join(root, "scripts/audit-negative-diagnostics.mjs");
const rules = path.join(root, "dev-support/test262/negative-diagnostic-rules.tsv");
const contracts = path.join(root, "dev-support/test262/negative-diagnostics.tsv");
const smoke = path.join(
  root,
  "dev-support/test262/negative-diagnostic-generator-smoke.tsv",
);

function argumentsFrom(commandLine) {
  const values = new Map();
  for (let index = 0; index < commandLine.length; index += 2) {
    const name = commandLine[index];
    const value = commandLine[index + 1];
    if (!/^(?:--suite|--qjs|--oxide)$/.test(name) || !value || values.has(name)) {
      throw new Error(
        "usage: test-negative-diagnostic-generator.mjs " +
          "--suite DIR --qjs FILE --oxide FILE",
      );
    }
    values.set(name, path.resolve(value));
  }
  for (const required of ["--suite", "--qjs", "--oxide"]) {
    if (!values.has(required)) throw new Error(`missing ${required}`);
  }
  return {
    oxide: values.get("--oxide"),
    qjs: values.get("--qjs"),
    suite: values.get("--suite"),
  };
}

function run(arguments_) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [audit, ...arguments_], {
      cwd: root,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stderr.on("data", (chunk) => stderr.push(chunk));
    child.once("error", reject);
    child.once("close", (status, signal) => {
      resolve({
        signal,
        status,
        stderr: Buffer.concat(stderr).toString("utf8"),
        stdout: Buffer.concat(stdout).toString("utf8"),
      });
    });
  });
}

function generationArguments({ candidates, output, oxide, qjs, suite }) {
  return [
    "--generate",
    candidates,
    "--output",
    output,
    "--rules",
    rules,
    "--suite",
    suite,
    "--qjs",
    qjs,
    "--oxide",
    oxide,
    "--workers",
    "2",
  ];
}

async function expectFailure(arguments_, message) {
  const result = await run(arguments_);
  assert.equal(result.signal, null);
  assert.notEqual(result.status, 0, result.stdout);
  assert.match(result.stderr, message);
}

async function main() {
  const engines = argumentsFrom(process.argv.slice(2));
  await Promise.all([
    access(engines.suite, fsConstants.R_OK),
    access(engines.qjs, fsConstants.X_OK),
    access(engines.oxide, fsConstants.X_OK),
  ]);
  const temporary = await mkdtemp(path.join(tmpdir(), "quickjs-oxide-negative-generator-"));
  try {
    const output = path.join(temporary, "smoke.tsv");
    const success = await run(
      generationArguments({ candidates: smoke, output, ...engines }),
    );
    assert.equal(success.signal, null);
    assert.equal(success.status, 0, success.stderr);

    const candidateKeys = new Set(
      (await readFile(smoke, "utf8"))
        .trimEnd()
        .split("\n")
        .slice(1)
        .map((line) => line.split("\t").slice(0, 2).join("\t")),
    );
    const canonicalLines = (await readFile(contracts, "utf8")).trimEnd().split("\n");
    const expected = `${[
      canonicalLines[0],
      ...canonicalLines.slice(1).filter((line) => {
        const key = line.split("\t").slice(0, 2).join("\t");
        return candidateKeys.has(key);
      }),
    ].join("\n")}\n`;
    assert.equal(await readFile(output, "utf8"), expected);

    const bomCandidates = path.join(temporary, "bom-candidates.tsv");
    await writeFile(
      bomCandidates,
      Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), await readFile(smoke)]),
    );
    await expectFailure(
      generationArguments({
        candidates: bomCandidates,
        output: path.join(temporary, "bom.tsv"),
        ...engines,
      }),
      /diagnostic candidates header drifted/u,
    );

    const syntheticSuite = path.join(temporary, "suite");
    await mkdir(path.join(syntheticSuite, "test"), { recursive: true });
    await writeFile(
      path.join(syntheticSuite, "test/runtime.js"),
      "/*---\nnegative:\n  type: SyntaxError\n  phase: parse\n---*/\n" +
        'throw new SyntaxError("runtime-deception");\n',
    );
    const runtimeCandidates = path.join(temporary, "runtime-candidates.tsv");
    await writeFile(
      runtimeCandidates,
      "path\tvariant\trule\n" +
        "test/runtime.js\tsloppy\tassignment-target.non-simple\n",
    );
    const deceptiveOutput = path.join(temporary, "runtime.tsv");
    await expectFailure(
      generationArguments({
        candidates: runtimeCandidates,
        output: deceptiveOutput,
        suite: syntheticSuite,
        qjs: engines.qjs,
        oxide: engines.oxide,
      }),
      /parse probe emitted no native error/u,
    );
    await assert.rejects(access(deceptiveOutput), { code: "ENOENT" });

    await expectFailure(
      generationArguments({
        candidates: smoke,
        output: path.join(temporary, "a-a.tsv"),
        suite: engines.suite,
        qjs: engines.qjs,
        oxide: engines.qjs,
      }),
      /QuickJS and Oxide must be distinct executables/u,
    );

    const hardlinkOutput = path.join(temporary, "candidate-hardlink.tsv");
    await link(smoke, hardlinkOutput);
    await expectFailure(
      generationArguments({
        candidates: smoke,
        output: hardlinkOutput,
        ...engines,
      }),
      /generated output must not alias an input/u,
    );
    const symlinkOutput = path.join(temporary, "candidate-symlink.tsv");
    await symlink(smoke, symlinkOutput);
    await expectFailure(
      generationArguments({
        candidates: smoke,
        output: symlinkOutput,
        ...engines,
      }),
      /generated output must be a regular non-symlink file/u,
    );

    const copiedQuickJs = path.join(temporary, "copied-qjs");
    await copyFile(engines.qjs, copiedQuickJs);
    await chmod(copiedQuickJs, 0o700);
    await expectFailure(
      generationArguments({
        candidates: smoke,
        output: path.join(temporary, "copied-a-a.tsv"),
        suite: engines.suite,
        qjs: engines.qjs,
        oxide: copiedQuickJs,
      }),
      /QuickJS and Oxide must be distinct executables/u,
    );

    await writeFile(
      path.join(syntheticSuite, "test/module.js"),
      "/*---\nflags:\n  - module\nnegative:\n  type: SyntaxError\n  phase: parse\n---*/\n" +
        "import.meta = 1;\n",
    );
    const moduleCandidates = path.join(temporary, "module-candidates.tsv");
    await writeFile(
      moduleCandidates,
      "path\tvariant\trule\n" +
        "test/module.js\tsloppy\tassignment-target.import-meta-script-goal\n",
    );
    await expectFailure(
      generationArguments({
        candidates: moduleCandidates,
        output: path.join(temporary, "module.tsv"),
        suite: syntheticSuite,
        qjs: engines.qjs,
        oxide: engines.oxide,
      }),
      /module; Oxide CLI generation is script-only/u,
    );

    await writeFile(
      path.join(syntheticSuite, "test/raw.js"),
      "/*---\nflags: [raw]\nnegative:\n  type: SyntaxError\n  phase: parse\n---*/\n" +
        "0 = 1;\n",
    );
    const rawCandidates = path.join(temporary, "raw-candidates.tsv");
    await writeFile(
      rawCandidates,
      "path\tvariant\trule\n" +
        "test/raw.js\tstrict\tassignment-target.non-simple\n",
    );
    await expectFailure(
      generationArguments({
        candidates: rawCandidates,
        output: path.join(temporary, "raw.tsv"),
        suite: syntheticSuite,
        qjs: engines.qjs,
        oxide: engines.oxide,
      }),
      /strict is not selected by Test262 metadata/u,
    );

    await symlink("runtime.js", path.join(syntheticSuite, "test/source-link.js"));
    const symlinkCandidates = path.join(temporary, "symlink-candidates.tsv");
    await writeFile(
      symlinkCandidates,
      "path\tvariant\trule\n" +
        "test/source-link.js\tsloppy\tassignment-target.non-simple\n",
    );
    await expectFailure(
      generationArguments({
        candidates: symlinkCandidates,
        output: path.join(temporary, "source-link.tsv"),
        suite: syntheticSuite,
        qjs: engines.qjs,
        oxide: engines.oxide,
      }),
      /not a regular non-symlink suite source/u,
    );
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
  console.log("Negative diagnostic generator regression checks passed.");
}

main().catch((error) => {
  console.error(`test-negative-diagnostic-generator: ${error.stack || error}`);
  process.exitCode = 1;
});
