#!/usr/bin/env node

import { createHash, randomUUID } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import {
  access,
  lstat,
  open,
  readFile,
  realpath,
  rename,
  stat,
  unlink,
} from "node:fs/promises";
import { availableParallelism } from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";

const CONTRACT_HEADER =
  "path\tvariant\tsource_sha256\tphase\ttype\trule\tmessage\tline\tcolumn\tlocation_policy";
const CANDIDATE_HEADER = "path\tvariant\trule";
const RULE_HEADER = "rule\tquickjs_anchor\tdescription";
const DEFAULT_CONTRACTS = "dev-support/test262/negative-diagnostics.tsv";
const DEFAULT_RULES = "dev-support/test262/negative-diagnostic-rules.tsv";
const DEFAULT_TIMEOUT_MS = 10_000;
const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
const QUICKJS_NON_JS_DIAGNOSTIC_ANCHORS = new Set([
  "get_class_atom",
  "get_lvalue",
  "json_parse_value",
  "lre_compile",
  "parse_unicode_property",
  "re_parse_nested_class",
]);

function usage() {
  console.error(
    "usage: audit-negative-diagnostics.mjs [--check] [--contracts FILE] " +
      "[--rules FILE] [--suite DIR --qjs FILE] [--workers N]\n" +
      "       audit-negative-diagnostics.mjs --generate FILE --output FILE " +
      "--suite DIR --qjs FILE --oxide FILE [--rules FILE] [--workers N]",
  );
}

function parseArguments(arguments_) {
  const options = {
    check: false,
    contracts: DEFAULT_CONTRACTS,
    contractsSpecified: false,
    generate: undefined,
    output: undefined,
    oxide: undefined,
    qjs: undefined,
    rules: DEFAULT_RULES,
    suite: undefined,
    workers: Math.max(1, Math.min(availableParallelism(), 12)),
  };
  for (let index = 0; index < arguments_.length; index += 1) {
    const argument = arguments_[index];
    const takeValue = () => {
      index += 1;
      if (index >= arguments_.length) throw new Error(`${argument} requires a value`);
      return arguments_[index];
    };
    if (argument === "--check") options.check = true;
    else if (argument === "--contracts") {
      options.contracts = takeValue();
      options.contractsSpecified = true;
    }
    else if (argument === "--generate") options.generate = takeValue();
    else if (argument === "--output") options.output = takeValue();
    else if (argument === "--oxide") options.oxide = takeValue();
    else if (argument === "--rules") options.rules = takeValue();
    else if (argument === "--suite") options.suite = takeValue();
    else if (argument === "--qjs") options.qjs = takeValue();
    else if (argument === "--workers") {
      const value = takeValue();
      if (!/^[1-9][0-9]*$/.test(value) || Number(value) > 64) {
        throw new Error("--workers must be between 1 and 64");
      }
      options.workers = Number(value);
    } else if (argument === "--help" || argument === "-h") {
      usage();
      process.exit(0);
    } else {
      throw new Error(`unknown option: ${argument}`);
    }
  }
  if (options.check && options.generate) {
    throw new Error("--check and --generate are mutually exclusive");
  }
  if (options.generate && (!options.output || !options.oxide)) {
    throw new Error("--generate requires both --output and --oxide");
  }
  if (options.generate && (!options.suite || !options.qjs)) {
    throw new Error("--generate requires both --suite and --qjs");
  }
  if (options.generate && options.contractsSpecified) {
    throw new Error("--contracts is not valid with --generate");
  }
  if (!options.generate && (options.output || options.oxide)) {
    throw new Error("--output and --oxide are valid only with --generate");
  }
  if (!options.check && !options.generate && (!options.suite || !options.qjs)) {
    throw new Error("replay requires both --suite and --qjs");
  }
  return options;
}

function isCanonicalTestPath(relative) {
  return (
    relative.startsWith("test/") &&
    relative.endsWith(".js") &&
    !relative.endsWith("_FIXTURE.js") &&
    !relative.includes("\\") &&
    !/\p{Cc}/u.test(relative) &&
    relative.split("/").every((part) => part && part !== "." && part !== "..")
  );
}

function bytewiseCompare(left, right) {
  return Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8"));
}

function hasControlCharacters(value) {
  return /\p{Cc}/u.test(value);
}

async function readCanonicalLines(file, expectedHeader, label) {
  let source;
  try {
    source = UTF8_DECODER.decode(await readFile(file));
  } catch (error) {
    throw new Error(`${label} is not valid UTF-8`, { cause: error });
  }
  if (source.includes("\r")) throw new Error(`${label} must use LF line endings`);
  if (!source.endsWith("\n")) throw new Error(`${label} must end with a newline`);
  const lines = source.slice(0, -1).split("\n");
  if (lines.shift() !== expectedHeader) throw new Error(`${label} header drifted`);
  if (lines.length === 0 || lines.some((line) => line.length === 0)) {
    throw new Error(`${label} must contain non-empty rows`);
  }
  return lines;
}

async function loadCandidates(file, rules) {
  const lines = await readCanonicalLines(file, CANDIDATE_HEADER, "diagnostic candidates");
  const candidates = [];
  let previous = "";
  for (const [index, line] of lines.entries()) {
    const fields = line.split("\t");
    if (
      fields.length !== 3 ||
      fields.some((field) => field.trim() !== field || hasControlCharacters(field))
    ) {
      throw new Error(`diagnostic candidate line ${index + 2} is not canonical`);
    }
    const [relative, variant, rule] = fields;
    const key = `${relative}\t${variant}`;
    if (!isCanonicalTestPath(relative) || bytewiseCompare(previous, key) >= 0) {
      throw new Error(`diagnostic candidates are duplicate or unsorted at ${key}`);
    }
    if (!/^(sloppy|strict)$/.test(variant) || !rules.has(rule)) {
      throw new Error(`diagnostic candidate ${key} has invalid identity data`);
    }
    previous = key;
    candidates.push({ relative, rule, variant });
  }
  return candidates;
}

async function loadRules(file) {
  const lines = await readCanonicalLines(file, RULE_HEADER, "diagnostic rule registry");
  const rules = new Map();
  let previous = "";
  for (const [index, line] of lines.entries()) {
    const fields = line.split("\t");
    if (
      fields.length !== 3 ||
      fields.some((field) => field.trim() !== field || hasControlCharacters(field))
    ) {
      throw new Error(`diagnostic rule line ${index + 2} is not canonical`);
    }
    const [rule, anchor, description] = fields;
    if (!/^[a-z0-9]+(?:[.-][a-z0-9]+)*$/.test(rule)) {
      throw new Error(`diagnostic rule line ${index + 2} has an invalid rule`);
    }
    if (
      !/^js_[a-z0-9_]+$/.test(anchor) &&
      !QUICKJS_NON_JS_DIAGNOSTIC_ANCHORS.has(anchor)
    ) {
      throw new Error(`diagnostic rule ${rule} has an invalid QuickJS anchor`);
    }
    if (!description || bytewiseCompare(previous, rule) >= 0 || rules.has(rule)) {
      throw new Error(`diagnostic rule registry is duplicate or unsorted at ${rule}`);
    }
    previous = rule;
    rules.set(rule, { anchor, description });
  }
  return rules;
}

async function loadContracts(file, rules, { requireAllRules = true } = {}) {
  const lines = await readCanonicalLines(file, CONTRACT_HEADER, "negative diagnostics");
  const contracts = [];
  const usedRules = new Set();
  let previous = "";
  for (const [index, line] of lines.entries()) {
    const fields = line.split("\t");
    if (
      fields.length !== 10 ||
      fields.some((field) => field.trim() !== field || hasControlCharacters(field))
    ) {
      throw new Error(`negative diagnostic line ${index + 2} is not canonical`);
    }
    const [
      relative,
      variant,
      sourceSha256,
      phase,
      errorType,
      rule,
      message,
      lineText,
      columnText,
      locationPolicy,
    ] = fields;
    const key = `${relative}\t${variant}`;
    if (!isCanonicalTestPath(relative) || bytewiseCompare(previous, key) >= 0) {
      throw new Error(`negative diagnostics are duplicate or unsorted at ${key}`);
    }
    if (!/^(sloppy|strict)$/.test(variant) || !/^[0-9a-f]{64}$/.test(sourceSha256)) {
      throw new Error(`negative diagnostic ${key} has invalid identity data`);
    }
    if (!/^(?:parse|resolution)$/.test(phase) || errorType !== "SyntaxError") {
      throw new Error(`QuickJS audit does not yet support ${phase}/${errorType}: ${key}`);
    }
    if (!rules.has(rule)) throw new Error(`negative diagnostic ${key} uses unknown rule ${rule}`);
    if (!message) throw new Error(`negative diagnostic ${key} has an empty message`);
    const exact = locationPolicy === "exact";
    const absent = locationPolicy === "absent";
    const lineCoordinate = exact ? Number(lineText) : undefined;
    const column = exact ? Number(columnText) : undefined;
    if (
      (!exact && !absent) ||
      (exact &&
        (!/^[1-9][0-9]*$/.test(lineText) ||
          !/^[1-9][0-9]*$/.test(columnText) ||
          lineCoordinate > 0xffff_ffff ||
          column > 0xffff_ffff)) ||
      (absent && (lineText || columnText))
    ) {
      throw new Error(`negative diagnostic ${key} has an invalid location policy`);
    }
    previous = key;
    usedRules.add(rule);
    contracts.push({
      column,
      errorType,
      line: lineCoordinate,
      locationPolicy,
      message,
      phase,
      relative,
      rule,
      sourceSha256,
      variant,
    });
  }
  const unused = [...rules.keys()].filter((rule) => !usedRules.has(rule));
  if (requireAllRules && unused.length) {
    throw new Error(`diagnostic rule registry contains unused rules: ${unused.join(", ")}`);
  }
  return contracts;
}

function frontmatter(source) {
  const start = source.indexOf("/*---");
  if (start < 0) return "";
  const end = source.indexOf("---*/", start + 5);
  if (end < 0) throw new Error("unterminated Test262 frontmatter");
  return source.slice(start + 5, end);
}

function cleanScalar(value) {
  return value.trim().replace(/^(['"])(.*)\1$/u, "$2");
}

function splitListItems(value) {
  return value
    .split(",")
    .flatMap((item) => item.trim().split(/\s+/u))
    .map(cleanScalar)
    .filter(Boolean);
}

function frontmatterList(source, key) {
  const lines = source.split(/\r\n|\n|\r/u);
  const index = lines.findIndex((line) => {
    if (/^\s/u.test(line)) return false;
    const colon = line.indexOf(":");
    return colon !== -1 && line.slice(0, colon).trim() === key;
  });
  if (index < 0) return [];
  const raw = lines[index].slice(lines[index].indexOf(":") + 1).trim();
  if (raw.startsWith("[")) {
    let joined = raw;
    for (let next = index + 1; !joined.includes("]") && next < lines.length; next += 1) {
      joined += ` ${lines[next].trim()}`;
    }
    const end = joined.indexOf("]");
    if (end < 0) throw new Error(`unterminated ${key} list`);
    return splitListItems(joined.slice(1, end));
  }
  if (raw) return [cleanScalar(raw)];
  const values = [];
  for (const line of lines.slice(index + 1)) {
    if (!/^\s/u.test(line)) break;
    const nested = line.trim();
    if (nested.startsWith("-")) values.push(cleanScalar(nested.slice(1)));
  }
  return values;
}

function hasTopLevelKey(source, key) {
  return source.split(/\r\n|\n|\r/u).some((line) => {
    if (/^\s/u.test(line)) return false;
    const colon = line.indexOf(":");
    return colon !== -1 && line.slice(0, colon).trim() === key;
  });
}

function nestedScalar(source, parent, key) {
  const lines = source.split(/\r\n|\n|\r/u);
  const index = lines.findIndex((line) => {
    if (/^\s/u.test(line)) return false;
    const colon = line.indexOf(":");
    return colon !== -1 && line.slice(0, colon).trim() === parent;
  });
  if (index < 0) return undefined;
  for (const line of lines.slice(index + 1)) {
    if (!/^\s/u.test(line)) break;
    const colon = line.indexOf(":");
    if (colon !== -1 && line.slice(0, colon).trim() === key) {
      return cleanScalar(line.slice(colon + 1));
    }
  }
  return undefined;
}

function test262Metadata(source) {
  const text = frontmatter(source);
  const flags = new Set(frontmatterList(text, "flags"));
  const negative = hasTopLevelKey(text, "negative")
    ? {
        phase: nestedScalar(text, "negative", "phase"),
        errorType: nestedScalar(text, "negative", "type"),
      }
    : undefined;
  return { flags, negative };
}

function selectedVariants(flags) {
  if (flags.has("module") || flags.has("noStrict") || flags.has("raw")) {
    return ["sloppy"];
  }
  if (flags.has("onlyStrict")) return ["strict"];
  return ["sloppy", "strict"];
}

function assertParseNegativeMetadata(metadata, candidate) {
  if (
    metadata.negative?.phase !== "parse" ||
    metadata.negative.errorType !== "SyntaxError"
  ) {
    throw new Error(`${candidate.relative} is not a parse/SyntaxError Test262 negative`);
  }
  if (!selectedVariants(metadata.flags).includes(candidate.variant)) {
    throw new Error(
      `${candidate.relative} ${candidate.variant} is not selected by Test262 metadata`,
    );
  }
}

function assertContractMetadata(metadata, contract) {
  if (
    metadata.negative?.phase !== contract.phase ||
    metadata.negative.errorType !== contract.errorType
  ) {
    throw new Error(
      `${contract.relative} ${contract.variant} diagnostic metadata does not match the contract`,
    );
  }
  if (!selectedVariants(metadata.flags).includes(contract.variant)) {
    throw new Error(
      `${contract.relative} ${contract.variant} is not selected by Test262 metadata`,
    );
  }
  if (contract.phase === "resolution" && !metadata.flags.has("module")) {
    throw new Error(`${contract.relative} resolution contract is not a Module test`);
  }
}

function parsePhaseProbe(source, variant, module) {
  if (source.startsWith("#!") || source.startsWith("\ufeff#!")) {
    throw new Error("parse-phase generation does not support hashbang Script sources");
  }
  if (variant === "sloppy" && /(?:"use strict"|'use strict')/u.test(source)) {
    throw new Error(
      "parse-phase generation rejects sloppy candidates with a possible authored use strict directive",
    );
  }
  return authoredSource(
    `throw "__quickjs_oxide_parse_phase_probe__";\n${source}`,
    variant,
    module,
  );
}

function authoredSource(source, variant, module) {
  return variant === "strict" && !module ? `"use strict";\n${source}` : source;
}

function runEngine(executable, arguments_, timeoutMs, label, { cwd } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, arguments_, {
      cwd,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    let stdoutBytes = 0;
    let stderrBytes = 0;
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, timeoutMs);
    child.stdout.on("data", (chunk) => {
      stdoutBytes += chunk.length;
      if (stdoutBytes <= 1_048_576) stdout.push(chunk);
    });
    child.stderr.on("data", (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes <= 1_048_576) stderr.push(chunk);
    });
    child.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on("close", (status, signal) => {
      clearTimeout(timer);
      if (timedOut) return reject(new Error(`${label} timed out after ${timeoutMs}ms`));
      if (stdoutBytes > 1_048_576 || stderrBytes > 1_048_576) {
        return reject(new Error(`${label} diagnostic output exceeded 1 MiB`));
      }
      resolve({
        signal,
        status,
        stderr: Buffer.concat(stderr).toString("utf8"),
        stdout: Buffer.concat(stdout).toString("utf8"),
      });
    });
  });
}

function parseEngineDiagnostic(result, label) {
  if (result.status === 0 || result.signal) {
    throw new Error(`${label} did not return a normal failing status: ${JSON.stringify(result)}`);
  }
  const lines = result.stderr.replaceAll("\r\n", "\n").split("\n");
  const errorLine = lines.find((line) => /^[A-Za-z_$][A-Za-z0-9_$]*Error: /.test(line));
  if (!errorLine) throw new Error(`${label} emitted no native error: ${result.stderr}`);
  const separator = errorLine.indexOf(": ");
  const errorType = errorLine.slice(0, separator);
  const message = errorLine.slice(separator + 2);
  const locationLine = lines.find((line) => /^\s*at <cmdline>:[1-9][0-9]*:[1-9][0-9]*\s*$/.test(line));
  const location = locationLine?.match(/<cmdline>:([1-9][0-9]*):([1-9][0-9]*)/);
  return {
    column: location ? Number(location[2]) : undefined,
    errorType,
    line: location ? Number(location[1]) : undefined,
    message,
  };
}

function parseQuickJsTest262Diagnostic(result, label) {
  if (result.status === 0 || result.signal) {
    throw new Error(`${label} did not return a normal failing status: ${JSON.stringify(result)}`);
  }
  const transcript = `${result.stdout}${result.stderr}`.replaceAll("\r\n", "\n");
  const lines = transcript.split("\n");
  const errorLine = lines.find((line) => /^[A-Za-z_$][A-Za-z0-9_$]*Error: /.test(line));
  if (!errorLine) throw new Error(`${label} emitted no native error: ${transcript}`);
  const separator = errorLine.indexOf(": ");
  const locationLine = lines.find((line) =>
    /^\s*at .+:[1-9][0-9]*:[1-9][0-9]*\s*$/.test(line),
  );
  const location = locationLine?.match(/:([1-9][0-9]*):([1-9][0-9]*)\s*$/);
  return {
    column: location ? Number(location[2]) : undefined,
    errorType: errorLine.slice(0, separator),
    line: location ? Number(location[1]) : undefined,
    message: errorLine.slice(separator + 2),
  };
}

async function readSuiteSource(options, relative) {
  const sourcePath = path.join(options.realSuite, relative);
  const sourceInfo = await lstat(sourcePath);
  if (!sourceInfo.isFile() || sourceInfo.isSymbolicLink()) {
    throw new Error(`${relative} is not a regular non-symlink suite source`);
  }
  const resolved = await realpath(sourcePath);
  const inside = path.relative(options.realSuite, resolved);
  if (inside === "" || inside.startsWith("..") || path.isAbsolute(inside)) {
    throw new Error(`${relative} escapes the pinned suite`);
  }
  return readFile(resolved);
}

async function replayContract(options, contract) {
  const sourceBytes = await readSuiteSource(options, contract.relative);
  const actualSha256 = createHash("sha256").update(sourceBytes).digest("hex");
  if (actualSha256 !== contract.sourceSha256) {
    throw new Error(`${contract.relative} source hash drifted`);
  }
  let source;
  try {
    source = UTF8_DECODER.decode(sourceBytes);
  } catch (error) {
    throw new Error(`${contract.relative} is not valid UTF-8`, { cause: error });
  }
  const metadata = test262Metadata(source);
  assertContractMetadata(metadata, contract);
  const module = metadata.flags.has("module");
  const resolution = contract.phase === "resolution";
  const authored = authoredSource(source, contract.variant, module);
  const result = resolution
    ? await runEngine(
        options.quickJsRunner,
        ["-N", "--module", contract.relative],
        DEFAULT_TIMEOUT_MS,
        "QuickJS Test262 module resolution",
        { cwd: options.quickJsRoot },
      )
    : await runEngine(
        options.qjs,
        [module ? "--module" : "--script", "-e", authored],
        DEFAULT_TIMEOUT_MS,
        "QuickJS",
      );
  const actual = resolution
    ? parseQuickJsTest262Diagnostic(result, "QuickJS Test262 module resolution")
    : parseEngineDiagnostic(result, "QuickJS");
  const expectedLocation = contract.locationPolicy === "exact";
  if (
    actual.errorType !== contract.errorType ||
    actual.message !== contract.message ||
    (expectedLocation && (actual.line !== contract.line || actual.column !== contract.column)) ||
    (!expectedLocation && (actual.line !== undefined || actual.column !== undefined))
  ) {
    throw new Error(
      `${contract.relative} ${contract.variant} ${contract.rule} mismatch:\n` +
        `expected ${JSON.stringify({ type: contract.errorType, message: contract.message, line: contract.line, column: contract.column })}\n` +
        `actual   ${JSON.stringify(actual)}\n${result.stdout}${result.stderr}`,
    );
  }
}

function assertParsePhaseProbe(candidate, original, probe, label) {
  const originalHasLocation = original.line !== undefined && original.column !== undefined;
  const probeHasLocation = probe.line !== undefined && probe.column !== undefined;
  if (
    original.errorType !== probe.errorType ||
    original.message !== probe.message ||
    originalHasLocation !== probeHasLocation ||
    (originalHasLocation &&
      (probe.line !== original.line + 1 || probe.column !== original.column))
  ) {
    throw new Error(
      `${candidate.relative} ${candidate.variant} did not prove ${label} parse phase:\n` +
        `original ${JSON.stringify(original)}\nprobe    ${JSON.stringify(probe)}`,
    );
  }
}

async function generateContract(options, candidate) {
  const sourceBytes = await readSuiteSource(options, candidate.relative);
  let source;
  try {
    source = UTF8_DECODER.decode(sourceBytes);
  } catch (error) {
    throw new Error(`${candidate.relative} is not valid UTF-8`, { cause: error });
  }
  const metadata = test262Metadata(source);
  assertParseNegativeMetadata(metadata, candidate);
  const module = metadata.flags.has("module");
  if (module) {
    throw new Error(`${candidate.relative} is a module; Oxide CLI generation is script-only`);
  }
  const authored = authoredSource(source, candidate.variant, module);
  const probed = parsePhaseProbe(source, candidate.variant, module);
  const [quickJsProbeResult, oxideProbeResult] = await Promise.all([
    runEngine(
      options.qjs,
      ["--script", "-e", probed],
      DEFAULT_TIMEOUT_MS,
      "QuickJS parse probe",
    ),
    runEngine(options.oxide, ["-e", probed], DEFAULT_TIMEOUT_MS, "Oxide parse probe"),
  ]);
  const quickJsProbe = parseEngineDiagnostic(quickJsProbeResult, "QuickJS parse probe");
  const oxideProbe = parseEngineDiagnostic(oxideProbeResult, "Oxide parse probe");
  const [quickJsResult, oxideResult] = await Promise.all([
    runEngine(options.qjs, ["--script", "-e", authored], DEFAULT_TIMEOUT_MS, "QuickJS"),
    runEngine(options.oxide, ["-e", authored], DEFAULT_TIMEOUT_MS, "Oxide"),
  ]);
  const quickJs = parseEngineDiagnostic(quickJsResult, "QuickJS");
  const oxide = parseEngineDiagnostic(oxideResult, "Oxide");
  assertParsePhaseProbe(candidate, quickJs, quickJsProbe, "QuickJS");
  assertParsePhaseProbe(candidate, oxide, oxideProbe, "Oxide");
  if (
    quickJs.errorType !== metadata.negative.errorType ||
    quickJs.errorType !== oxide.errorType ||
    quickJs.message !== oxide.message ||
    quickJs.line !== oxide.line ||
    quickJs.column !== oxide.column
  ) {
    throw new Error(
      `${candidate.relative} ${candidate.variant} ${candidate.rule} differential mismatch:\n` +
        `QuickJS ${JSON.stringify(quickJs)}\nOxide   ${JSON.stringify(oxide)}`,
    );
  }
  const hasLocation = quickJs.line !== undefined && quickJs.column !== undefined;
  if (hasLocation !== (quickJs.line !== undefined || quickJs.column !== undefined)) {
    throw new Error(`${candidate.relative} emitted a partial diagnostic location`);
  }
  return [
    candidate.relative,
    candidate.variant,
    createHash("sha256").update(sourceBytes).digest("hex"),
    metadata.negative.phase,
    metadata.negative.errorType,
    candidate.rule,
    quickJs.message,
    hasLocation ? String(quickJs.line) : "",
    hasLocation ? String(quickJs.column) : "",
    hasLocation ? "exact" : "absent",
  ].join("\t");
}

function sameFile(left, right) {
  return left.dev === right.dev && left.ino === right.ino;
}

async function executableIdentity(file, label) {
  const resolved = await realpath(file);
  const info = await stat(resolved);
  if (!info.isFile()) throw new Error(`${label} is not a regular file`);
  await access(resolved, fsConstants.X_OK);
  const sha256 = createHash("sha256").update(await readFile(resolved)).digest("hex");
  return { info, resolved, sha256 };
}

async function generationOutput(options, protectedIdentities) {
  const requested = path.resolve(options.output);
  const parent = await realpath(path.dirname(requested));
  const parentInfo = await stat(parent);
  if (!parentInfo.isDirectory()) throw new Error("generated output parent is not a directory");
  const output = path.join(parent, path.basename(requested));
  const insideSuite = path.relative(options.realSuite, output);
  if (insideSuite === "" || (!insideSuite.startsWith("..") && !path.isAbsolute(insideSuite))) {
    throw new Error("generated output must be outside the pinned suite");
  }
  if (protectedIdentities.some(({ resolved }) => resolved === output)) {
    throw new Error("generated output must not overwrite an input");
  }
  try {
    const outputLink = await lstat(output);
    if (!outputLink.isFile() || outputLink.isSymbolicLink()) {
      throw new Error("generated output must be a regular non-symlink file when it exists");
    }
    const outputInfo = await stat(output);
    if (protectedIdentities.some(({ info }) => sameFile(info, outputInfo))) {
      throw new Error("generated output must not alias an input");
    }
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
  return output;
}

async function atomicWriteContracts(output, rows, rules) {
  const temporary = path.join(
    path.dirname(output),
    `.${path.basename(output)}.${process.pid}.${randomUUID()}.tmp`,
  );
  let temporaryExists = false;
  try {
    const handle = await open(temporary, "wx", 0o600);
    temporaryExists = true;
    try {
      await handle.writeFile(`${CONTRACT_HEADER}\n${rows.join("\n")}\n`);
      await handle.sync();
    } finally {
      await handle.close();
    }
    const roundTripped = await loadContracts(temporary, rules, { requireAllRules: false });
    if (roundTripped.length !== rows.length) {
      throw new Error("generated contract schema round-trip changed the row count");
    }
    await rename(temporary, output);
    temporaryExists = false;
  } finally {
    if (temporaryExists) {
      try {
        await unlink(temporary);
      } catch (error) {
        if (error?.code !== "ENOENT") throw error;
      }
    }
  }
}

async function generateAll(options, candidates, rules) {
  options.realSuite = await realpath(options.suite);
  const suite = await stat(options.realSuite);
  if (!suite.isDirectory()) throw new Error("suite is not a directory");
  const [qjs, oxide, candidateFile, ruleFile, contractFile] = await Promise.all([
    executableIdentity(options.qjs, "QuickJS"),
    executableIdentity(options.oxide, "Oxide"),
    realpath(options.generate).then(async (resolved) => ({
      info: await stat(resolved),
      resolved,
    })),
    realpath(options.rules).then(async (resolved) => ({
      info: await stat(resolved),
      resolved,
    })),
    realpath(options.contracts).then(async (resolved) => ({
      info: await stat(resolved),
      resolved,
    })),
  ]);
  if (sameFile(qjs.info, oxide.info) || qjs.sha256 === oxide.sha256) {
    throw new Error("QuickJS and Oxide must be distinct executables");
  }
  options.qjs = qjs.resolved;
  options.oxide = oxide.resolved;
  const output = await generationOutput(options, [
    qjs,
    oxide,
    candidateFile,
    ruleFile,
    contractFile,
  ]);
  const rows = new Array(candidates.length);
  let next = 0;
  const failures = [];
  let stopped = false;
  const worker = async () => {
    while (!stopped) {
      const index = next;
      next += 1;
      if (index >= candidates.length) return;
      try {
        rows[index] = await generateContract(options, candidates[index]);
      } catch (error) {
        failures.push({ error, index });
        stopped = true;
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(options.workers, candidates.length) }, worker));
  if (failures.length) {
    failures.sort((left, right) => left.index - right.index);
    throw failures[0].error;
  }
  await atomicWriteContracts(output, rows, rules);
}

async function replayAll(options, contracts) {
  options.realSuite = await realpath(options.suite);
  const suite = await stat(options.realSuite);
  const qjs = await executableIdentity(options.qjs, "QuickJS");
  if (!suite.isDirectory()) throw new Error("suite is not a directory");
  options.qjs = qjs.resolved;
  if (contracts.some((contract) => contract.phase === "resolution")) {
    const quickJsRoot = path.dirname(qjs.resolved);
    const runner = await executableIdentity(
      path.join(quickJsRoot, "run-test262"),
      "QuickJS run-test262",
    );
    options.quickJsRoot = options.realSuite;
    options.quickJsRunner = runner.resolved;
  }
  let next = 0;
  const failures = [];
  const worker = async () => {
    while (failures.length === 0) {
      const index = next;
      next += 1;
      if (index >= contracts.length) return;
      try {
        await replayContract(options, contracts[index]);
      } catch (error) {
        failures.push(error);
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(options.workers, contracts.length) }, worker));
  if (failures.length) throw failures[0];
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  const rules = await loadRules(options.rules);
  if (options.generate) {
    const candidates = await loadCandidates(options.generate, rules);
      await generateAll(options, candidates, rules);
    console.log(
      `Generated ${candidates.length} exact contracts after QuickJS/Oxide differential validation.`,
    );
    return;
  }
  const contracts = await loadContracts(options.contracts, rules);
  if (options.check) {
    console.log(
      `Negative diagnostic registry passed: ${contracts.length} exact contracts / ${rules.size} rules.`,
    );
    return;
  }
  await replayAll(options, contracts);
  console.log(
    `QuickJS negative diagnostic audit passed: ${contracts.length} exact contracts / ${rules.size} rules.`,
  );
}

main().catch((error) => {
  console.error(`audit-negative-diagnostics: ${error.stack || error}`);
  process.exitCode = 1;
});
